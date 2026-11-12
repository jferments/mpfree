//! Direct ALSA, no alsa-lib: the PCM playback calls the click needs, the
//! PCM capture calls the line input needs, and the RawMIDI substream choice
//! the pad test needs.
//!
//! The parameter blocks are kept as byte arrays in the 32-bit ARM layout the
//! MPC's kernel expects (sizes and offsets from <sound/asound.h> compiled for
//! armv7 hard-float: snd_pcm_hw_params 604 bytes, snd_pcm_sw_params 104,
//! snd_xferi 12, snd_rawmidi_info 268). Building them is plain code that the
//! host tests check; only the ioctl itself is device-only.

use crate::sys;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

// Request numbers, 32-bit ARM: _IOC(dir, type, nr, size) = dir<<30 | size<<16 | type<<8 | nr.
pub const PCM_HW_REFINE: u32 = 0xC25C_4110;
pub const PCM_HW_PARAMS: u32 = 0xC25C_4111;
pub const PCM_SW_PARAMS: u32 = 0xC068_4113;
pub const PCM_PREPARE: u32 = 0x0000_4140;
pub const PCM_DROP: u32 = 0x0000_4143;
pub const PCM_DRAIN: u32 = 0x0000_4144;
pub const PCM_WRITEI_FRAMES: u32 = 0x400C_4150;
/// _IOR('A', 0x51, struct snd_xferi): read interleaved frames (capture).
pub const PCM_READI_FRAMES: u32 = 0x800C_4151;
/// _IO('A', 0x42): start a prepared stream now (capture starts here).
pub const PCM_START: u32 = 0x0000_4142;
/// _IOR('A', 0x21, snd_pcm_sframes_t): frames queued ahead of the next one written.
pub const PCM_DELAY: u32 = 0x8004_4121;
pub const CTL_RAWMIDI_PREFER_SUBDEVICE: u32 = 0x4004_5542;
pub const RAWMIDI_INFO: u32 = 0x810C_5701;

/// Errors the stream calls report.
pub const EAGAIN: i32 = 11;
pub const EPIPE: i32 = 32;

pub const RAWMIDI_STREAM_OUTPUT: u32 = 0;
pub const RAWMIDI_STREAM_INPUT: u32 = 1;

pub const HW_PARAMS_SIZE: usize = 604;
pub const SW_PARAMS_SIZE: usize = 104;
pub const RAWMIDI_INFO_SIZE: usize = 268;

// snd_pcm_hw_params, 32-bit ARM.
const HW_MASKS: usize = 4; // 3 masks of 32 bytes: ACCESS, FORMAT, SUBFORMAT
const HW_INTERVALS: usize = 260; // 12 intervals of 12 bytes, SAMPLE_BITS (8) .. TICK_TIME (19)
const HW_RMASK: usize = 512;
const HW_CMASK: usize = 516;
const HW_INFO: usize = 520;
const HW_MSBITS: usize = 524;
const HW_RATE_NUM: usize = 528;
const HW_RATE_DEN: usize = 532;
const HW_FIFO: usize = 536;

pub const PARAM_ACCESS: usize = 0;
pub const PARAM_FORMAT: usize = 1;
pub const PARAM_SUBFORMAT: usize = 2;
pub const PARAM_SAMPLE_BITS: usize = 8;
pub const PARAM_FRAME_BITS: usize = 9;
pub const PARAM_CHANNELS: usize = 10;
pub const PARAM_RATE: usize = 11;
pub const PARAM_PERIOD_SIZE: usize = 13;
pub const PARAM_PERIODS: usize = 15;
pub const PARAM_BUFFER_SIZE: usize = 17;
const LAST_INTERVAL: usize = 19;

pub const ACCESS_RW_INTERLEAVED: u32 = 3;
pub const FORMAT_S32_LE: u32 = 10;
pub const SUBFORMAT_STD: u32 = 0;

const INTERVAL_OPENMIN: u32 = 1;
const INTERVAL_OPENMAX: u32 = 2;
const INTERVAL_INTEGER: u32 = 4;
const INTERVAL_EMPTY: u32 = 8;

fn put(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn get(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interval {
    pub min: u32,
    pub max: u32,
    pub openmin: bool,
    pub openmax: bool,
    pub integer: bool,
    pub empty: bool,
}

impl Interval {
    pub fn single(&self) -> Option<u32> {
        (!self.empty && self.min == self.max && !self.openmin && !self.openmax).then_some(self.min)
    }

    pub fn describe(&self) -> String {
        if self.empty {
            return "none".to_string();
        }
        match self.single() {
            Some(v) => v.to_string(),
            None => format!(
                "{}{}..{}{}",
                if self.openmin { "(" } else { "[" },
                self.min,
                self.max,
                if self.openmax { ")" } else { "]" }
            ),
        }
    }
}

/// struct snd_pcm_hw_params, as bytes.
#[derive(Clone)]
pub struct HwParams(pub [u8; HW_PARAMS_SIZE]);

impl HwParams {
    /// Everything allowed: what alsa-lib and tinyalsa start from before narrowing.
    pub fn any() -> HwParams {
        let mut b = [0u8; HW_PARAMS_SIZE];
        for m in 0..3 {
            for w in 0..8 {
                put(&mut b, HW_MASKS + m * 32 + w * 4, 0xFFFF_FFFF);
            }
        }
        for p in PARAM_SAMPLE_BITS..=LAST_INTERVAL {
            let at = HW_INTERVALS + (p - PARAM_SAMPLE_BITS) * 12;
            put(&mut b, at, 0);
            put(&mut b, at + 4, u32::MAX);
            put(&mut b, at + 8, 0);
        }
        put(&mut b, HW_RMASK, 0xFFFF_FFFF);
        put(&mut b, HW_CMASK, 0);
        put(&mut b, HW_INFO, 0xFFFF_FFFF);
        HwParams(b)
    }

    /// Allow only `value` for a mask parameter (access, format, subformat).
    pub fn set_mask(&mut self, param: usize, value: u32) {
        assert!(param <= PARAM_SUBFORMAT && value < 256);
        let base = HW_MASKS + param * 32;
        for w in 0..8 {
            put(&mut self.0, base + w * 4, 0);
        }
        let at = base + (value as usize / 32) * 4;
        put(&mut self.0, at, 1 << (value % 32));
    }

    pub fn mask_has(&self, param: usize, value: u32) -> bool {
        let at = HW_MASKS + param * 32 + (value as usize / 32) * 4;
        get(&self.0, at) & (1 << (value % 32)) != 0
    }

    /// Exactly `value` for an interval parameter.
    pub fn set_int(&mut self, param: usize, value: u32) {
        let at = Self::interval_at(param);
        put(&mut self.0, at, value);
        put(&mut self.0, at + 4, value);
        put(&mut self.0, at + 8, INTERVAL_INTEGER);
    }

    pub fn interval(&self, param: usize) -> Interval {
        let at = Self::interval_at(param);
        let flags = get(&self.0, at + 8);
        Interval {
            min: get(&self.0, at),
            max: get(&self.0, at + 4),
            openmin: flags & INTERVAL_OPENMIN != 0,
            openmax: flags & INTERVAL_OPENMAX != 0,
            integer: flags & INTERVAL_INTEGER != 0,
            empty: flags & INTERVAL_EMPTY != 0,
        }
    }

    fn interval_at(param: usize) -> usize {
        assert!((PARAM_SAMPLE_BITS..=LAST_INTERVAL).contains(&param));
        HW_INTERVALS + (param - PARAM_SAMPLE_BITS) * 12
    }

    pub fn msbits(&self) -> u32 {
        get(&self.0, HW_MSBITS)
    }

    pub fn rate_fraction(&self) -> (u32, u32) {
        (get(&self.0, HW_RATE_NUM), get(&self.0, HW_RATE_DEN))
    }

    pub fn fifo_size(&self) -> u32 {
        get(&self.0, HW_FIFO)
    }

    /// The stock app's playback stream (docs/dev/hardware/audio-runtime.md): stereo S32_LE at
    /// 44.1 kHz, interleaved, 128-frame periods, 3 periods (384 frames).
    pub fn stock_playback(periods: u32) -> HwParams {
        let mut p = HwParams::any();
        p.set_mask(PARAM_ACCESS, ACCESS_RW_INTERLEAVED);
        p.set_mask(PARAM_FORMAT, FORMAT_S32_LE);
        p.set_mask(PARAM_SUBFORMAT, SUBFORMAT_STD);
        p.set_int(PARAM_SAMPLE_BITS, 32);
        p.set_int(PARAM_FRAME_BITS, 64);
        p.set_int(PARAM_CHANNELS, 2);
        p.set_int(PARAM_RATE, 44_100);
        p.set_int(PARAM_PERIOD_SIZE, 128);
        p.set_int(PARAM_PERIODS, periods);
        p
    }

    /// The line input's stream: the same format as playback, 128-frame
    /// periods, 2 periods (256 frames), the capture buffer seen while Akai's
    /// own service ran (docs/dev/hardware/audio-runtime.md).
    pub fn stock_capture() -> HwParams {
        HwParams::stock_playback(2)
    }

    /// Only the format, channels and rate: what the card allows for the rest.
    pub fn format_only() -> HwParams {
        let mut p = HwParams::any();
        p.set_mask(PARAM_ACCESS, ACCESS_RW_INTERLEAVED);
        p.set_mask(PARAM_FORMAT, FORMAT_S32_LE);
        p.set_int(PARAM_CHANNELS, 2);
        p.set_int(PARAM_RATE, 44_100);
        p
    }
}

/// struct snd_pcm_sw_params, as bytes.
pub struct SwParams(pub [u8; SW_PARAMS_SIZE]);

impl SwParams {
    /// Wake every period, start when `start` frames are queued, and stop (an
    /// underrun, reported as EPIPE) when the queue runs dry.
    pub fn new(avail_min: u32, start: u32, stop: u32) -> SwParams {
        let mut b = [0u8; SW_PARAMS_SIZE];
        put(&mut b, 0, 0); // tstamp_mode: none
        put(&mut b, 4, 1); // period_step
        put(&mut b, 8, 0); // sleep_min
        put(&mut b, 12, avail_min);
        put(&mut b, 16, 1); // xfer_align (ignored by current kernels)
        put(&mut b, 20, start);
        put(&mut b, 24, stop);
        put(&mut b, 28, 0); // silence_threshold
        put(&mut b, 32, 0); // silence_size
        put(&mut b, 36, 0); // boundary: the kernel writes it back
        SwParams(b)
    }
}

pub struct Pcm {
    file: File,
}

impl Pcm {
    /// Open a playback device without waiting if something else holds it, then
    /// switch to blocking writes.
    pub fn open_playback(path: &str) -> io::Result<Pcm> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(sys::O_NONBLOCK)
            .open(path)?;
        sys::clear_nonblock(file.as_raw_fd())?;
        Ok(Pcm { file })
    }

    /// Open a capture device without waiting if something else holds it.
    /// It stays non-blocking: reads return what has arrived (or nothing),
    /// so the playback loop never waits on the input.
    pub fn open_capture(path: &str) -> io::Result<Pcm> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(sys::O_NONBLOCK)
            .open(path)?;
        Ok(Pcm { file })
    }

    pub fn refine(&self, p: &mut HwParams) -> io::Result<()> {
        sys::ioctl_buf(self.file.as_raw_fd(), PCM_HW_REFINE, &mut p.0).map(|_| ())
    }

    pub fn hw_params(&self, p: &mut HwParams) -> io::Result<()> {
        sys::ioctl_buf(self.file.as_raw_fd(), PCM_HW_PARAMS, &mut p.0).map(|_| ())
    }

    pub fn sw_params(&self, p: &mut SwParams) -> io::Result<()> {
        sys::ioctl_buf(self.file.as_raw_fd(), PCM_SW_PARAMS, &mut p.0).map(|_| ())
    }

    pub fn prepare(&self) -> io::Result<()> {
        sys::ioctl_none(self.file.as_raw_fd(), PCM_PREPARE).map(|_| ())
    }

    pub fn start(&self) -> io::Result<()> {
        sys::ioctl_none(self.file.as_raw_fd(), PCM_START).map(|_| ())
    }

    pub fn drain(&self) -> io::Result<()> {
        sys::ioctl_none(self.file.as_raw_fd(), PCM_DRAIN).map(|_| ())
    }

    pub fn drop_stream(&self) -> io::Result<()> {
        sys::ioctl_none(self.file.as_raw_fd(), PCM_DROP).map(|_| ())
    }

    /// Write interleaved stereo frames; blocks until there is room. An underrun
    /// shows as an error with raw_os_error() == Some(32) (EPIPE).
    pub fn write_frames(&self, interleaved: &[i32]) -> io::Result<u32> {
        if !sys::IS_DEVICE_ABI {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "device calls are for 32-bit ARM only",
            ));
        }
        let frames = (interleaved.len() / 2) as u32;
        // struct snd_xferi { snd_pcm_sframes_t result; void *buf; snd_pcm_uframes_t frames; } (32-bit)
        let mut x = [0u8; 12];
        put(&mut x, 4, interleaved.as_ptr() as usize as u32);
        put(&mut x, 8, frames);
        sys::ioctl_buf(self.file.as_raw_fd(), PCM_WRITEI_FRAMES, &mut x)?;
        Ok(get(&x, 0))
    }

    /// Read up to `interleaved.len() / 2` stereo frames that have arrived,
    /// without waiting: Ok(0) if none have. An overrun (the input was not
    /// read in time) shows as an error with raw_os_error() == Some(32)
    /// (EPIPE); prepare and start again.
    pub fn read_frames(&self, interleaved: &mut [i32]) -> io::Result<u32> {
        if !sys::IS_DEVICE_ABI {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "device calls are for 32-bit ARM only",
            ));
        }
        let frames = (interleaved.len() / 2) as u32;
        let mut x = [0u8; 12];
        put(&mut x, 4, interleaved.as_mut_ptr() as usize as u32);
        put(&mut x, 8, frames);
        match sys::ioctl_buf(self.file.as_raw_fd(), PCM_READI_FRAMES, &mut x) {
            Ok(_) => Ok(get(&x, 0)),
            Err(e) if e.raw_os_error() == Some(EAGAIN) => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Frames between what the card plays now and the end of what was
    /// written: how long a frame written now waits before it leaves the
    /// card's buffer (the codec and anything after it not included).
    pub fn delay(&self) -> io::Result<i32> {
        let mut d = [0u8; 4];
        sys::ioctl_buf(self.file.as_raw_fd(), PCM_DELAY, &mut d)?;
        Ok(i32::from_ne_bytes(d))
    }
}

/// What snd_rawmidi_info reports about an opened substream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMidiInfo {
    pub subdevice: u32,
    pub stream: u32,
    pub name: String,
    pub subname: String,
    pub subdevices: u32,
}

pub fn parse_rawmidi_info(b: &[u8; RAWMIDI_INFO_SIZE]) -> RawMidiInfo {
    let text = |from: usize, len: usize| {
        let s = &b[from..from + len];
        let end = s.iter().position(|&c| c == 0).unwrap_or(len);
        String::from_utf8_lossy(&s[..end]).into_owned()
    };
    // device 0, subdevice 4, stream 8, card 12, flags 16, id 20 (64), name 84 (80), subname 164 (32),
    // subdevices_count 196, subdevices_avail 200, reserved 204 (64).
    RawMidiInfo {
        subdevice: get(b, 4),
        stream: get(b, 8),
        name: text(84, 80),
        subname: text(164, 32),
        subdevices: get(b, 196),
    }
}

pub enum Direction {
    Input,
    Output,
}

/// Open one substream of a RawMIDI device. ALSA picks the substream from the
/// "preferred subdevice" set on the card's control device by the same thread,
/// as alsa-lib does for hw:CARD,0,SUB. Non-blocking. The caller checks the
/// returned info before using it.
pub fn open_rawmidi(card: u32, subdevice: u32, dir: Direction) -> io::Result<(File, RawMidiInfo)> {
    let ctl = OpenOptions::new()
        .read(true)
        .open(format!("/dev/snd/controlC{card}"))?;
    sys::ioctl_int(
        ctl.as_raw_fd(),
        CTL_RAWMIDI_PREFER_SUBDEVICE,
        subdevice as i32,
    )?;
    let mut opts = OpenOptions::new();
    match dir {
        Direction::Input => opts.read(true),
        Direction::Output => opts.write(true),
    };
    let midi = opts
        .custom_flags(sys::O_NONBLOCK)
        .open(format!("/dev/snd/midiC{card}D0"))?;
    drop(ctl);
    let mut info = info_request(&dir);
    sys::ioctl_buf(midi.as_raw_fd(), RAWMIDI_INFO, &mut info)?;
    Ok((midi, parse_rawmidi_info(&info)))
}

/// The INFO request: the kernel reads `stream` (offset 8) to pick which of the
/// file's substreams to describe, and fails with ENODEV if that one is not open.
pub fn info_request(dir: &Direction) -> [u8; RAWMIDI_INFO_SIZE] {
    let mut info = [0u8; RAWMIDI_INFO_SIZE];
    put(
        &mut info,
        8,
        match dir {
            Direction::Output => RAWMIDI_STREAM_OUTPUT,
            Direction::Input => RAWMIDI_STREAM_INPUT,
        },
    );
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
        dir << 30 | (size as u32) << 16 | (ty as u32) << 8 | nr as u32
    }

    #[test]
    fn request_numbers_match_the_32_bit_arm_layout() {
        const W: u32 = 1;
        const R: u32 = 2;
        assert_eq!(PCM_HW_REFINE, ioc(R | W, b'A', 0x10, HW_PARAMS_SIZE));
        assert_eq!(PCM_HW_PARAMS, ioc(R | W, b'A', 0x11, HW_PARAMS_SIZE));
        assert_eq!(PCM_SW_PARAMS, ioc(R | W, b'A', 0x13, SW_PARAMS_SIZE));
        assert_eq!(PCM_PREPARE, ioc(0, b'A', 0x40, 0));
        assert_eq!(PCM_DROP, ioc(0, b'A', 0x43, 0));
        assert_eq!(PCM_DRAIN, ioc(0, b'A', 0x44, 0));
        assert_eq!(PCM_WRITEI_FRAMES, ioc(W, b'A', 0x50, 12));
        assert_eq!(PCM_READI_FRAMES, ioc(R, b'A', 0x51, 12));
        assert_eq!(PCM_START, ioc(0, b'A', 0x42, 0));
        assert_eq!(PCM_DELAY, ioc(R, b'A', 0x21, 4));
        assert_eq!(CTL_RAWMIDI_PREFER_SUBDEVICE, ioc(W, b'U', 0x42, 4));
        assert_eq!(RAWMIDI_INFO, ioc(R, b'W', 0x01, RAWMIDI_INFO_SIZE));
    }

    #[test]
    fn stock_playback_narrows_only_what_it_should() {
        let p = HwParams::stock_playback(3);
        assert!(p.mask_has(PARAM_ACCESS, ACCESS_RW_INTERLEAVED));
        assert!(!p.mask_has(PARAM_ACCESS, 0));
        assert!(p.mask_has(PARAM_FORMAT, FORMAT_S32_LE));
        assert!(!p.mask_has(PARAM_FORMAT, 2)); // S16_LE
        for (param, v) in [
            (PARAM_CHANNELS, 2),
            (PARAM_RATE, 44_100),
            (PARAM_PERIOD_SIZE, 128),
            (PARAM_PERIODS, 3),
            (PARAM_SAMPLE_BITS, 32),
            (PARAM_FRAME_BITS, 64),
        ] {
            let i = p.interval(param);
            assert_eq!(
                (i.min, i.max, i.integer, i.empty),
                (v, v, true, false),
                "param {param}"
            );
            assert_eq!(i.single(), Some(v));
        }
        let b = p.interval(PARAM_BUFFER_SIZE);
        assert_eq!((b.min, b.max), (0, u32::MAX));
        assert_eq!(b.describe(), format!("[0..{}]", u32::MAX));
        assert_eq!(get(&p.0, HW_RMASK), u32::MAX);
        assert_eq!(get(&p.0, 0), 0); // flags
                                     // The FORMAT mask is the second 32-byte mask; S32_LE is bit 10 of its first word.
        assert_eq!(get(&p.0, HW_MASKS + 32), 1 << 10);
        // RATE is the fourth interval (8, 9, 10, 11).
        assert_eq!(get(&p.0, HW_INTERVALS + 3 * 12), 44_100);
        assert_eq!(get(&p.0, HW_INTERVALS + 3 * 12 + 8), INTERVAL_INTEGER);
    }

    #[test]
    fn stock_capture_is_stereo_s32_at_44k1_with_a_256_frame_buffer() {
        let p = HwParams::stock_capture();
        assert!(p.mask_has(PARAM_ACCESS, ACCESS_RW_INTERLEAVED));
        assert!(p.mask_has(PARAM_FORMAT, FORMAT_S32_LE));
        for (param, v) in [
            (PARAM_CHANNELS, 2),
            (PARAM_RATE, 44_100),
            (PARAM_PERIOD_SIZE, 128),
            (PARAM_PERIODS, 2),
        ] {
            assert_eq!(p.interval(param).single(), Some(v), "param {param}");
        }
    }

    #[test]
    fn capture_calls_are_device_only() {
        // On the workstation the stream calls refuse before any ioctl.
        if !sys::IS_DEVICE_ABI {
            let f = File::open("/dev/null").unwrap();
            let pcm = Pcm { file: f };
            let mut buf = [0i32; 16];
            assert_eq!(
                pcm.read_frames(&mut buf).unwrap_err().kind(),
                io::ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn sw_params_layout() {
        let s = SwParams::new(128, 384, 384);
        assert_eq!(get(&s.0, 4), 1);
        assert_eq!(get(&s.0, 12), 128);
        assert_eq!(get(&s.0, 20), 384);
        assert_eq!(get(&s.0, 24), 384);
        assert_eq!(s.0.len(), 104);
    }

    #[test]
    fn rawmidi_info_names() {
        let mut b = [0u8; RAWMIDI_INFO_SIZE];
        put(&mut b, 4, 1);
        put(&mut b, 8, RAWMIDI_STREAM_INPUT);
        b[84..96].copy_from_slice(b"MPC One MIDI");
        b[164..187].copy_from_slice(b"MPC Studio Live Private");
        put(&mut b, 196, 3);
        let i = parse_rawmidi_info(&b);
        assert_eq!(
            i,
            RawMidiInfo {
                subdevice: 1,
                stream: 1,
                name: "MPC One MIDI".into(),
                subname: "MPC Studio Live Private".into(),
                subdevices: 3
            }
        );
    }

    #[test]
    fn info_request_names_the_open_direction() {
        // The kernel describes rfile->input for stream 1 and rfile->output for stream 0.
        let input = info_request(&Direction::Input);
        let output = info_request(&Direction::Output);
        assert_eq!(get(&input, 8), 1);
        assert_eq!(get(&output, 8), 0);
        assert!(input.iter().enumerate().all(|(i, &b)| i == 8 || b == 0));
    }
}
