//! M13: the live touchscreen reader for MPFree's GUI.
//!
//! Reads `/dev/input/event0` (the MPC One's ILI2117) read-only and
//! non-blocking, **without `EVIOCGRAB`**, so Akai's app keeps receiving every
//! event when it runs alongside. It closes two gaps the hand-rolled reader in
//! `mpfree-app` had:
//!
//! 1. **Seeding at open.** The kernel sends `ABS_MT_SLOT` only when the slot
//!    changes, so a fresh reader that assumes slot 0 can file the first
//!    contact under the wrong slot. At open we read the current slot and every
//!    slot's state (`EVIOCGABS` + `EVIOCGMTSLOTS`) and seed the decoder.
//!    Fingers already resting on the screen are ignored until they lift.
//!    If the snapshot can't be read, the reader still works, unseeded (the
//!    old behaviour), and says so in [`ReaderStats::seeded`] and
//!    [`TouchReader::seed_error`], so touch never disappears because of it.
//! 2. **Resync after overflow.** On `SYN_DROPPED` we read the same snapshot
//!    and resync, so fingers that stayed down keep going instead of all being
//!    cancelled. Only if the snapshot fails do we fall back to cancelling.
//!
//! Before each snapshot the reader drains what is already queued and drops
//! it, because those events are older than the snapshot and replaying them on
//! top of it could end a contact that is still down.
//!
//! The only unsafe code is the two read-only ioctls in [`sys`].
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod sys;

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use mpfree_touch::mt::{InputEvent, MtDecoder, MtSnapshot, SlotSnapshot, Step};
use mpfree_touch::pointer::{AxisMap, PointerEvent};
use mpfree_touch::profile::TouchProfile;
use mpfree_touch::{ABS_MT_POSITION_X, ABS_MT_POSITION_Y, ABS_MT_SLOT, ABS_MT_TRACKING_ID};

/// `struct input_event`: 16 bytes on the MPC's 32-bit ARM, 24 on a 64-bit host.
pub const EVENT_SIZE: usize = if cfg!(target_pointer_width = "32") {
    16
} else {
    24
};

/// The MPC One's touchscreen node (M10).
pub const MPC_ONE_TOUCH: &str = "/dev/input/event0";

/// Events read per `read(2)`.
const BATCH: usize = 64;
/// Upper bound on reads while draining before a snapshot, so a flood can
/// never hold the caller. At ~113 Hz one read nearly always suffices.
const MAX_DRAIN_READS: usize = 32;

/// Decode one native `struct input_event` (exactly [`EVENT_SIZE`] bytes).
pub fn parse_input_event(b: &[u8]) -> InputEvent {
    let word = EVENT_SIZE / 2 - 4; // bytes per timeval field: 4 or 8
    let field = |at: usize| -> u64 {
        let mut v = [0u8; 8];
        v[..word].copy_from_slice(&b[at..at + word]);
        u64::from_ne_bytes(v)
    };
    let (sec, usec) = (field(0), field(word));
    let base = EVENT_SIZE - 8;
    InputEvent::new(
        sec.wrapping_mul(1_000_000).wrapping_add(usec),
        u16::from_ne_bytes([b[base], b[base + 1]]),
        u16::from_ne_bytes([b[base + 2], b[base + 3]]),
        i32::from_ne_bytes([b[base + 4], b[base + 5], b[base + 6], b[base + 7]]),
    )
}

/// Where events and multitouch state come from: the real device, or a fake in
/// tests.
pub trait TouchSource {
    /// Non-blocking read of whole events. `Ok(0)` or `WouldBlock` mean none.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// The kernel's current slot and every slot's tracking id and position.
    fn snapshot(&mut self, slots: usize) -> io::Result<MtSnapshot>;
}

impl<T: TouchSource + ?Sized> TouchSource for Box<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (**self).read(buf)
    }

    fn snapshot(&mut self, slots: usize) -> io::Result<MtSnapshot> {
        (**self).snapshot(slots)
    }
}

/// The real device, opened read-only and non-blocking. Never grabbed.
pub struct EvdevFile {
    file: File,
}

impl EvdevFile {
    pub fn open(path: &str) -> io::Result<EvdevFile> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(sys::O_NONBLOCK)
            .open(path)?;
        Ok(EvdevFile { file })
    }

    /// Wrap a file the caller opened (read-only, non-blocking). The ioctls
    /// simply fail on anything that isn't an evdev node, and the reader then
    /// runs unseeded.
    pub fn from_file(file: File) -> EvdevFile {
        EvdevFile { file }
    }

    /// Range metadata of one axis (and its current value).
    pub fn abs(&self, code: u8) -> io::Result<sys::AbsInfo> {
        sys::get_abs(self.file.as_raw_fd(), code)
    }

    /// Check that the device reports the ranges and slot count the profile was
    /// measured with, so a different panel fails loudly instead of mapping
    /// touches to the wrong place.
    pub fn check_profile(&self, profile: &TouchProfile) -> Result<(), String> {
        let range = |code: u8| -> Result<(i32, i32), String> {
            let a = self
                .abs(code)
                .map_err(|e| format!("EVIOCGABS({code}): {e}"))?;
            Ok((a.minimum, a.maximum))
        };
        let x = range(ABS_MT_POSITION_X)?;
        let y = range(ABS_MT_POSITION_Y)?;
        let s = range(ABS_MT_SLOT)?;
        let want_x = (profile.raw_x.min, profile.raw_x.max);
        let want_y = (profile.raw_y.min, profile.raw_y.max);
        let want_s = (0, profile.slots as i32 - 1);
        if (x, y, s) != (want_x, want_y, want_s) {
            return Err(format!(
                "touch device differs from the profile: x {x:?} y {y:?} slots {s:?}, \
                 expected x {want_x:?} y {want_y:?} slots {want_s:?}"
            ));
        }
        Ok(())
    }
}

impl TouchSource for EvdevFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }

    fn snapshot(&mut self, slots: usize) -> io::Result<MtSnapshot> {
        let fd = self.file.as_raw_fd();
        let current_slot = sys::get_abs(fd, ABS_MT_SLOT)?.value;
        let ids = sys::get_mt_slots(fd, ABS_MT_TRACKING_ID, slots)?;
        let xs = sys::get_mt_slots(fd, ABS_MT_POSITION_X, slots)?;
        let ys = sys::get_mt_slots(fd, ABS_MT_POSITION_Y, slots)?;
        Ok(MtSnapshot {
            current_slot,
            slots: (0..slots)
                .map(|i| SlotSnapshot {
                    tracking_id: ids[i],
                    raw_x: xs[i],
                    raw_y: ys[i],
                })
                .collect(),
        })
    }
}

/// Counters for the GUI's Debug page and the workstation log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReaderStats {
    /// The decoder was seeded from a snapshot at open. False means the
    /// snapshot failed and the reader runs unseeded (see `seed_error`).
    pub seeded: bool,
    /// Kernel's current slot when we opened.
    pub slot_at_open: i32,
    /// Fingers already down at open (ignored until they lift).
    pub fingers_at_open: u32,
    /// Overflows recovered with a snapshot (fingers kept).
    pub resyncs: u64,
    /// Overflows where the snapshot failed and every finger was cancelled.
    pub resync_fallbacks: u64,
    /// Queued events dropped before a snapshot.
    pub drained_events: u64,
    /// Contacts that went down.
    pub contacts: u64,
    /// Raw input events read (all types, including SYN), after open.
    pub raw_events: u64,
}

/// Touch events in logical pixels from any [`TouchSource`].
pub struct TouchReader<S: TouchSource> {
    source: S,
    decoder: MtDecoder,
    map: AxisMap,
    buf: Vec<u8>,
    stats: ReaderStats,
    seed_error: Option<String>,
}

/// The reader on the real device.
pub type EvdevTouch = TouchReader<EvdevFile>;

impl EvdevTouch {
    /// Open the touchscreen at `path`, check it against `profile`, and seed
    /// the decoder. `width`×`height` is the logical screen.
    pub fn open(
        path: &str,
        profile: &TouchProfile,
        width: u32,
        height: u32,
    ) -> Result<EvdevTouch, String> {
        let dev = EvdevFile::open(path).map_err(|e| format!("{path}: {e}"))?;
        dev.check_profile(profile)
            .map_err(|e| format!("{path}: {e}"))?;
        TouchReader::new(dev, profile, width, height).map_err(|e| format!("{path}: {e}"))
    }
}

impl<S: TouchSource> TouchReader<S> {
    /// Wrap `source` and seed the decoder from its snapshot. If the queue
    /// can't be drained or the snapshot can't be read, the reader runs
    /// unseeded rather than failing, so the GUI keeps touch; the error is in
    /// [`Self::seed_error`]. Fails only on a bad profile or screen size.
    pub fn new(mut source: S, profile: &TouchProfile, width: u32, height: u32) -> io::Result<Self> {
        let mut decoder = MtDecoder::new(profile.slots)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, format!("{e:?}")))?;
        let map = profile
            .axis_map(width, height)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, format!("{e:?}")))?;
        let mut buf = vec![0u8; EVENT_SIZE * BATCH];
        let mut stats = ReaderStats::default();
        let seed = drain(&mut source, &mut buf).and_then(|drained| {
            stats.drained_events = drained;
            source.snapshot(profile.slots)
        });
        let seed_error = match seed {
            Ok(snap) => {
                decoder.seed(&snap, 0);
                stats.seeded = true;
                stats.slot_at_open = snap.current_slot;
                stats.fingers_at_open =
                    snap.slots.iter().filter(|s| s.tracking_id >= 0).count() as u32;
                None
            }
            Err(e) => Some(format!(
                "touch snapshot at open failed, running unseeded: {e}"
            )),
        };
        Ok(Self {
            source,
            decoder,
            map,
            buf,
            stats,
            seed_error,
        })
    }

    pub fn stats(&self) -> ReaderStats {
        self.stats
    }

    /// Why seeding at open failed, if it did. For the Debug page and log.
    pub fn seed_error(&self) -> Option<&str> {
        self.seed_error.as_deref()
    }

    pub fn decoder(&self) -> &MtDecoder {
        &self.decoder
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// Append everything that happened since the last call to `out`.
    pub fn read_into(&mut self, out: &mut Vec<PointerEvent>) -> io::Result<()> {
        'reads: loop {
            let n = match self.source.read(&mut self.buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            };
            let whole = n - n % EVENT_SIZE;
            self.stats.raw_events += (whole / EVENT_SIZE) as u64;
            for at in (0..whole).step_by(EVENT_SIZE) {
                let ev = parse_input_event(&self.buf[at..at + EVENT_SIZE]);
                match self.decoder.push(ev) {
                    Step::Frame => self.publish(out),
                    Step::ResyncRequired => {
                        // Everything after this point, in our buffer and in
                        // the kernel's queue, predates the snapshot.
                        self.stats.drained_events += ((whole - at) / EVENT_SIZE - 1) as u64;
                        self.resync(ev.time_us, out);
                        continue 'reads;
                    }
                    Step::Pending => {}
                }
            }
            if n < self.buf.len() {
                break;
            }
        }
        Ok(())
    }

    /// Convenience over [`Self::read_into`].
    pub fn read(&mut self) -> io::Result<Vec<PointerEvent>> {
        let mut out = Vec::new();
        self.read_into(&mut out)?;
        Ok(out)
    }

    fn resync(&mut self, time_us: u64, out: &mut Vec<PointerEvent>) {
        let snapshot = drain(&mut self.source, &mut self.buf).and_then(|drained| {
            self.stats.drained_events += drained;
            self.source.snapshot(self.decoder.slot_count())
        });
        match snapshot {
            Ok(snap) => {
                self.stats.resyncs += 1;
                self.decoder.resync(&snap, time_us);
            }
            Err(_) => {
                self.stats.resync_fallbacks += 1;
                self.decoder.resync_without_snapshot(time_us);
            }
        }
        self.publish(out);
    }

    fn publish(&mut self, out: &mut Vec<PointerEvent>) {
        for ce in self.decoder.events() {
            if ce.phase == mpfree_touch::mt::Phase::Down {
                self.stats.contacts += 1;
            }
            out.push(self.map.pointer(ce));
        }
    }
}

/// Read and discard whatever is queued; returns how many events that was.
fn drain<S: TouchSource>(source: &mut S, buf: &mut [u8]) -> io::Result<u64> {
    let mut dropped = 0;
    for _ in 0..MAX_DRAIN_READS {
        match source.read(buf) {
            Ok(0) => break,
            Ok(n) => {
                dropped += (n / EVENT_SIZE) as u64;
                if n < buf.len() {
                    break;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    Ok(dropped)
}
