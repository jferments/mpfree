//! Opens one ALSA RawMIDI subdevice for reading, without alsa-lib.
//!
//! The MPC One's internal controller is one RawMIDI device with three
//! subdevices (USB-MIDI cables); every physical control arrives on
//! subdevice 1, "Private". Opening `/dev/snd/midiC<card>D<device>` alone
//! gives whichever subdevice is free first, so, as alsa-lib does, we first
//! tell the card's control device which subdevice this thread prefers
//! (SNDRV_CTL_IOCTL_RAWMIDI_PREFER_SUBDEVICE, which only sets a field on our
//! own open control file), then open the RawMIDI device read-only and ask
//! it which subdevice we got (SNDRV_RAWMIDI_IOCTL_INFO, a query).
//!
//! Nothing here writes to the controller or changes any device setting.
//! The ioctl numbers and the 268-byte info struct were checked against
//! <sound/asound.h> for both x86-64 and 32-bit ARM.
#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

/// _IOW('U', 0x42, int)
const SNDRV_CTL_IOCTL_RAWMIDI_PREFER_SUBDEVICE: u32 = 0x4004_5542;
/// _IOR('W', 0x01, struct snd_rawmidi_info), 268 bytes
const SNDRV_RAWMIDI_IOCTL_INFO: u32 = 0x810c_5701;

#[repr(C)]
struct RawmidiInfo {
    device: u32,
    subdevice: u32,
    stream: i32,
    card: i32,
    flags: u32,
    id: [u8; 64],
    name: [u8; 80],
    subname: [u8; 32],
    subdevices_count: u32,
    subdevices_avail: u32,
    reserved: [u8; 64],
}

const _: () = assert!(std::mem::size_of::<RawmidiInfo>() == 268);

fn c_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Opens subdevice `subdevice` of RawMIDI device `device` on `card` for reading.
pub fn open_rawmidi(card: u32, device: u32, subdevice: u32) -> Result<File, String> {
    let control_path = format!("/dev/snd/controlC{card}");
    let control = OpenOptions::new()
        .read(true)
        .open(&control_path)
        .map_err(|e| format!("{control_path}: {e}"))?;
    let prefer: libc::c_int = subdevice as libc::c_int;
    // SAFETY: the request takes a pointer to one int, which outlives the call.
    let rc = unsafe {
        libc::ioctl(
            control.as_raw_fd(),
            SNDRV_CTL_IOCTL_RAWMIDI_PREFER_SUBDEVICE as _,
            &prefer,
        )
    };
    if rc != 0 {
        return Err(format!(
            "{control_path}: prefer subdevice {subdevice}: {}",
            std::io::Error::last_os_error()
        ));
    }
    // The preference applies to opens by this thread while `control` is open.
    let midi_path = format!("/dev/snd/midiC{card}D{device}");
    let midi = OpenOptions::new()
        .read(true)
        .open(&midi_path)
        .map_err(|e| format!("{midi_path}: {e}"))?;
    drop(control);

    // SAFETY: an all-zero RawmidiInfo is valid (plain integers and byte arrays).
    let mut info: RawmidiInfo = unsafe { std::mem::zeroed() };
    info.stream = 1; // SNDRV_RAWMIDI_STREAM_INPUT
    // SAFETY: the kernel fills at most size_of::<RawmidiInfo>() bytes of `info`.
    let rc = unsafe { libc::ioctl(midi.as_raw_fd(), SNDRV_RAWMIDI_IOCTL_INFO as _, &mut info) };
    if rc != 0 {
        return Err(format!(
            "{midi_path}: info: {}",
            std::io::Error::last_os_error()
        ));
    }
    eprintln!(
        "mpfree-splash: reading {midi_path} subdevice {} of {} ({:?} / {:?})",
        info.subdevice,
        info.subdevices_count,
        c_text(&info.name),
        c_text(&info.subname)
    );
    if info.subdevice != subdevice {
        return Err(format!(
            "{midi_path}: asked for subdevice {subdevice} but got {}",
            info.subdevice
        ));
    }
    Ok(midi)
}
