//! Memory by engine, on Dev > CPU by engine under the audio loop's rows:
//! what the sampler's sounds, the looper, the synth's wavetables and the
//! rest of the app hold, beside what each engine costs in CPU (Jay asked
//! for RAM beside CPU; performance handoff 13 Nov, item 8).
//!
//! The numbers come from what each part already knows about its own
//! buffers, read on the screen thread, plus the process's resident memory
//! from the developer tools' /proc sample. "Everything else" is the
//! resident memory less what the rows count, so a leak shows there. All
//! megabytes are 1 000 000 bytes, as Prefs' Loop memory says them.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mpfree_ui::Tone;

use crate::cores::Row;
use crate::sample_page::SampleMemory;

/// The memory MPFree keeps free: a load (a sound, a project's kits, a
/// stretched copy, an auto-sample) that would leave less is refused, and
/// says why. The MPC has 2 GB and no swap, so when memory runs out the
/// kernel kills the app mid-song (performance review, 14 Nov, item 6);
/// this leaves room for the kernel, the screen and what a load allocates
/// beyond its estimate.
pub const FLOOR: u64 = 256_000_000;

/// How every refusal for want of memory starts.
pub const SHORT: &str = "not enough memory";

/// The memory free for new loads now, in bytes: /proc/meminfo's
/// MemAvailable (what can be had without swapping). None where it can't be
/// read: off Linux every load is then let through, on Linux only small ones
/// ([`BLIND_MOST`]).
pub fn free_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    free_in(&text)
}

/// MemAvailable from /proc/meminfo's text, in bytes.
pub fn free_in(meminfo: &str) -> Option<u64> {
    crate::perf::kb_field(meminfo, "MemAvailable").map(|kb| kb * 1024)
}

/// The most a load may need while free memory can't be read on Linux,
/// where /proc/meminfo is always there: a load past it is refused rather
/// than let through blind (the second streaming review, 22 Nov). Off Linux
/// (a desktop build) every load goes through.
pub const BLIND_MOST: u64 = 64_000_000;

/// Whether taking `need` bytes more leaves [`FLOOR`] free beside the memory
/// promised to loads still under way ([`reserve`]); `free` is the memory
/// free now, None when unknown. Err says why, in a musician's words. Two
/// checks at once can both pass: a load that must not race another holds a
/// [`Ticket`] instead.
pub fn room_in(free: Option<u64>, need: u64) -> Result<(), String> {
    promises().room(free, need)
}

/// Promise `need` bytes to a load about to allocate them, if they fit
/// beside [`FLOOR`] and every promise still standing. Free memory is read
/// (`free`) while no other promise can be made, so two loads at once can't
/// both count on the same memory. Hold the ticket until the memory is
/// allocated (it then shows in MemAvailable) or the load has failed.
pub fn reserve(free: fn() -> Option<u64>, need: u64) -> Result<Ticket, String> {
    promises().reserve(free, need)
}

/// The memory promised to loads under way now, in bytes.
pub fn promised() -> u64 {
    promises().held()
}

/// The promises loads have made (disk streaming's P3 step 1, the shared
/// memory check). Each engine keeps its own memory; this only stops two big
/// loads from counting on the same free memory. Nothing here allocates for
/// anyone, and the sound thread never calls it.
///
/// Memory already allocated isn't subtracted again: it is in MemAvailable
/// already, because MPFree locks its memory (`mlockall`, so an allocation
/// is real at once).
#[derive(Debug, Default)]
pub struct Promises {
    held: Mutex<u64>,
}

/// One load's promise. Dropping it ends the promise.
#[must_use = "the promise ends when the ticket is dropped"]
#[derive(Debug)]
pub struct Ticket {
    from: Arc<Promises>,
    bytes: u64,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut held = self.from.lock();
        *held = held.saturating_sub(self.bytes);
    }
}

impl Promises {
    fn lock(&self) -> MutexGuard<'_, u64> {
        // A thread that panicked holding the lock left a whole number.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The bytes promised now.
    pub fn held(&self) -> u64 {
        *self.lock()
    }

    /// As [`room_in`], against these promises.
    pub fn room(&self, free: Option<u64>, need: u64) -> Result<(), String> {
        fits(free, self.held(), need)
    }

    /// As [`reserve`], against these promises.
    pub fn reserve(
        self: &Arc<Self>,
        free: fn() -> Option<u64>,
        need: u64,
    ) -> Result<Ticket, String> {
        let mut held = self.lock();
        fits(free(), *held, need)?;
        *held = held.saturating_add(need);
        Ok(Ticket {
            from: Arc::clone(self),
            bytes: need,
        })
    }
}

/// The app's promises. Each test thread has its own, so tests that set the
/// free memory exactly (`SamplePage::set_mem_free`) don't see each other's
/// loads.
#[cfg(not(test))]
fn promises() -> Arc<Promises> {
    static ALL: std::sync::OnceLock<Arc<Promises>> = std::sync::OnceLock::new();
    Arc::clone(ALL.get_or_init(Arc::default))
}

#[cfg(test)]
fn promises() -> Arc<Promises> {
    thread_local! {
        static ALL: Arc<Promises> = Arc::default();
    }
    ALL.with(Arc::clone)
}

/// Whether `need` more fits beside the floor and `promised`.
fn fits(free: Option<u64>, promised: u64, need: u64) -> Result<(), String> {
    let Some(free) = free else {
        if cfg!(target_os = "linux") && need > BLIND_MOST {
            return Err(format!(
                "{SHORT}: needs about {}, and how much is free can't be read",
                mb_round(need)
            ));
        }
        return Ok(());
    };
    if free.saturating_sub(promised).saturating_sub(need) >= FLOOR {
        return Ok(());
    }
    let promised = if promised > 0 {
        format!(", {} set aside for loads under way", mb_round(promised))
    } else {
        String::new()
    };
    Err(format!(
        "{SHORT}: needs about {}, {} free{promised}, {} kept spare",
        mb_round(need),
        mb_round(free),
        mb_round(FLOOR)
    ))
}

/// The most samples (frames × channels) a sound decodes to: the sampler
/// refuses a longer one before decoding it.
const MOST_SAMPLES: u64 = mpfree_sampler::wav::MAX_SAMPLES as u64;

/// What reading and decoding a sound file of `len` bytes holds at its peak,
/// from the header in `file` (read from its start). At the peak the sampler
/// holds the file, its sound decoded as 32-bit floats (4 bytes a sample),
/// and the copy the sound is kept in (`model::Pcm`, made from the floats: 2
/// bytes a sample when the file has 16 bits or fewer, else 4). The header
/// says how many samples there are; the file's size doesn't, since a FLAC
/// of silence is tiny beside its sound. So a 16-bit WAV holds four times
/// its size at the peak, and a FLAC as much as its sound needs.
///
/// A FLAC whose header doesn't give its length could grow to the most a
/// sound can be, its floats doubling in place as they go. A file whose
/// header can't be read is counted as if each byte were a sample, kept as
/// floats.
pub fn decode_peak<R: Read + Seek>(file: &mut R, len: u64) -> u64 {
    let peak = match samples_in(file) {
        Some(Some((samples, bits))) => {
            let kept = if bits <= 16 { 2 } else { 4 };
            samples.min(MOST_SAMPLES).saturating_mul(4 + kept)
        }
        Some(None) => MOST_SAMPLES * 12,
        None => len.saturating_mul(8).min(MOST_SAMPLES * 12),
    };
    len.saturating_add(peak)
}

/// As [`decode_peak`], for the file at `path`.
pub fn decode_peak_at(path: &Path, len: u64) -> u64 {
    match std::fs::File::open(path) {
        Ok(mut f) => decode_peak(&mut f, len),
        // Reading it will fail too, and say why.
        Err(_) => len,
    }
}

/// The samples (frames × channels) a sound file's header says it holds,
/// and its bits a sample: Some(None) for a FLAC that doesn't say how long it
/// is, None for a header that can't be read (WAV, RF64 and AIFF through
/// `mpfree_stream::format`).
fn samples_in<R: Read + Seek>(r: &mut R) -> Option<Option<(u64, u32)>> {
    let mut head = [0u8; 10];
    r.seek(SeekFrom::Start(0)).ok()?;
    r.read_exact(&mut head).ok()?;
    if &head[..4] == b"fLaC" || &head[..3] == b"ID3" {
        return flac_samples(r, &head);
    }
    r.seek(SeekFrom::Start(0)).ok()?;
    let l = mpfree_stream::format::layout(r).ok()?;
    let bits = 8 * l.sample.bytes() as u32;
    Some(Some((l.frames.saturating_mul(u64::from(l.channels)), bits)))
}

/// A FLAC file's samples and bits from its STREAMINFO, which must be its
/// first metadata block (RFC 9639): channels in 3 bits, bits a sample in 5
/// and frames in 36. `head` is its first 10 bytes, enough to step over an
/// ID3v2 tag.
fn flac_samples<R: Read + Seek>(r: &mut R, head: &[u8; 10]) -> Option<Option<(u64, u32)>> {
    let mut at = 0u64;
    if &head[..3] == b"ID3" {
        // Its size: 4 bytes of 7 bits; a footer adds 10 more.
        let size = head[6..10]
            .iter()
            .fold(0u64, |s, &x| s << 7 | u64::from(x & 0x7F));
        at = 10 + size + if head[5] & 0x10 != 0 { 10 } else { 0 };
    }
    let mut b = [0u8; 42];
    r.seek(SeekFrom::Start(at)).ok()?;
    r.read_exact(&mut b).ok()?;
    let block_len = u32::from(b[5]) << 16 | u32::from(b[6]) << 8 | u32::from(b[7]);
    if &b[..4] != b"fLaC" || b[4] & 0x7F != 0 || block_len != 34 {
        return None;
    }
    let info = &b[8..];
    let channels = u64::from(info[12] >> 1 & 0x07) + 1;
    let bits = u32::from((info[12] & 1) << 4 | info[13] >> 4) + 1;
    let frames = u64::from(info[13] & 0x0F) << 32
        | u64::from(u32::from_be_bytes([info[14], info[15], info[16], info[17]]));
    Some((frames != 0).then(|| (frames.saturating_mul(channels), bits)))
}

/// Megabytes, whole from 10 MB ("120 MB"), else to a tenth ("2.4 MB").
fn mb_round(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

/// The screen thread's watch on free memory (it reads /proc/meminfo every
/// 2 s for the header's RAM chip): one line when free memory falls under
/// [`FLOOR`], and one when it is back above it with some to spare, so a
/// level near the floor doesn't fill the log.
#[derive(Debug, Default)]
pub struct Watch {
    low: bool,
    /// The least free memory seen while low.
    least: u64,
    since: Option<Instant>,
}

/// Free memory must be this far above [`FLOOR`] before the watch says it
/// is back.
const BACK_ABOVE: u64 = 32_000_000;

impl Watch {
    /// The memory free now (`free`, bytes). Gives a line for the log when
    /// it crosses the floor.
    pub fn check(&mut self, free: u64, now: Instant) -> Option<String> {
        if !self.low && free < FLOOR {
            self.low = true;
            self.least = free;
            self.since = Some(now);
            return Some(format!(
                "memory: {} free, under the {} MPFree keeps spare; new sounds, stretches and \
                 auto-samples are refused until some are freed",
                mb_round(free),
                mb_round(FLOOR)
            ));
        }
        if self.low {
            self.least = self.least.min(free);
            if free >= FLOOR + BACK_ABOVE {
                self.low = false;
                let took = self
                    .since
                    .take()
                    .map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
                return Some(format!(
                    "memory: {} free again, after {:.0} s under {} (at least {})",
                    mb_round(free),
                    took.as_secs_f64(),
                    mb_round(FLOOR),
                    mb_round(self.least)
                ));
            }
        }
        None
    }
}

/// What the rows are made from.
#[derive(Clone, Copy, Debug, Default)]
pub struct Held {
    pub samples: SampleMemory,
    /// The looper's sound: frames used and frames set aside (stereo f32).
    pub loops: (usize, usize),
    /// The synth's wavetables, built at the start.
    pub synth: usize,
    /// The app's resident memory and the system's, in kB (from /proc,
    /// while the developer tools are on).
    pub rss_kb: Option<u64>,
    pub total_kb: Option<u64>,
    /// The system's memory free for new loads (MemAvailable), in kB.
    pub free_kb: Option<u64>,
    /// The memory promised to loads under way ([`promised`]), in bytes.
    pub promised: u64,
}

const LOOP_FRAME: usize = 2 * std::mem::size_of::<f32>();

fn mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

/// The rows, under a "Memory" heading row.
pub fn rows(h: &Held) -> Vec<Row> {
    let s = h.samples;
    let (used, kept) = (h.loops.0 * LOOP_FRAME, h.loops.1 * LOOP_FRAME);
    let counted = s.sounds + s.copies + s.input + s.recall + kept + h.synth;
    let mut rows = vec![
        ("Memory".to_string(), String::new(), Tone::Dim),
        (
            "Sampler: every kit's sounds".to_string(),
            format!("{} · {} sounds", mb(s.sounds), s.count),
            Tone::Normal,
        ),
    ];
    if s.copies > 0 {
        rows.push((
            "Sampler: stretched copies".to_string(),
            mb(s.copies),
            Tone::Normal,
        ));
    }
    if s.input > 0 {
        rows.push((
            "Sampler: input buffer (Rec)".to_string(),
            mb(s.input),
            Tone::Normal,
        ));
    }
    if s.recall > 0 {
        rows.push((
            "Sampler: Sample Recall (the master, 2 min)".to_string(),
            mb(s.recall),
            Tone::Normal,
        ));
    }
    rows.push((
        "Looper: used · set aside".to_string(),
        format!("{:.1} · {}", used as f64 / 1e6, mb(kept)),
        Tone::Normal,
    ));
    rows.push(("Synth: wavetables".to_string(), mb(h.synth), Tone::Normal));
    match h.rss_kb.map(|kb| kb as usize * 1024) {
        Some(rss) => {
            rows.push((
                "Everything else".to_string(),
                mb(rss.saturating_sub(counted)),
                Tone::Dim,
            ));
            let of = h
                .total_kb
                .map(|kb| format!(" of {}", mb(kb as usize * 1024)))
                .unwrap_or_default();
            rows.push((
                "The app in all (resident)".to_string(),
                format!("{}{of}", mb(rss)),
                Tone::Normal,
            ));
        }
        None => rows.push((
            "The app in all (resident)".to_string(),
            "not read yet".to_string(),
            Tone::Dim,
        )),
    }
    if let Some(free) = h.free_kb.map(|kb| kb * 1024) {
        // Under the floor, loads are refused.
        let tone = if free < FLOOR {
            Tone::Bad
        } else {
            Tone::Normal
        };
        rows.push((
            format!("Free for loads (keeps {} spare)", mb_round(FLOOR)),
            mb(free as usize),
            tone,
        ));
    }
    if h.promised > 0 {
        rows.push((
            "Set aside for loads under way".to_string(),
            mb(h.promised as usize),
            Tone::Normal,
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_load_that_would_leave_less_than_the_floor_is_refused_and_says_why() {
        // Unknown: off Linux, let through; on Linux, only small loads.
        assert_eq!(room_in(None, BLIND_MOST), Ok(()));
        if cfg!(target_os = "linux") {
            assert_eq!(
                room_in(None, 80_000_000),
                Err(
                    "not enough memory: needs about 80 MB, and how much is free can't be read"
                        .to_string()
                )
            );
        } else {
            assert_eq!(room_in(None, u64::MAX), Ok(()));
        }
        // The MPC on live test 3: 1706 MB free; a 64 MB sound is fine.
        assert_eq!(room_in(Some(1_706_000_000), 64_000_000), Ok(()));
        // Exactly the floor left: fine. A byte less: refused.
        assert_eq!(room_in(Some(FLOOR + 5_000_000), 5_000_000), Ok(()));
        assert_eq!(
            room_in(Some(FLOOR + 5_000_000), 5_000_001),
            Err(
                "not enough memory: needs about 5.0 MB, 261 MB free, 256 MB kept spare".to_string()
            )
        );
        // Already under the floor: nothing more loads.
        let e = room_in(Some(200_000_000), 1).unwrap_err();
        assert!(e.starts_with(SHORT), "{e}");
        assert!(e.contains("200 MB free"), "{e}");
    }

    #[test]
    fn two_loads_at_once_cant_both_count_on_the_same_free_memory() {
        // The second review's case: 650 MB above the floor, then a next
        // song (250 MB), an instrument (300 MB) and a project (250 MB).
        fn free() -> Option<u64> {
            Some(FLOOR + 650_000_000)
        }
        let p = Arc::new(Promises::default());
        let song = p.reserve(free, 250_000_000).unwrap();
        let synth = p.reserve(free, 300_000_000).unwrap();
        assert_eq!(p.held(), 550_000_000);
        assert_eq!(song.bytes, 250_000_000);
        // Alone it would fit; beside the others it doesn't.
        assert_eq!(Promises::default().room(free(), 250_000_000), Ok(()));
        assert_eq!(
            p.reserve(free, 250_000_000).unwrap_err(),
            "not enough memory: needs about 250 MB, 906 MB free, 550 MB set aside for loads \
             under way, 256 MB kept spare"
        );
        assert_eq!(p.held(), 550_000_000, "a refusal promises nothing");
        // The song is loaded (or failed): its promise ends and the project
        // fits. Exactly the floor left is fine.
        drop(song);
        assert_eq!(p.held(), 300_000_000);
        let project = p.reserve(free, 350_000_000).unwrap();
        assert!(p.room(free(), 1).is_err());
        drop((synth, project));
        assert_eq!(p.held(), 0);
        assert_eq!(p.room(free(), 650_000_000), Ok(()));
    }

    #[test]
    fn room_in_counts_the_promises_of_loads_under_way() {
        fn free() -> Option<u64> {
            Some(FLOOR + 100_000_000)
        }
        // This test thread's own promises (each test thread has its own).
        assert_eq!(promised(), 0);
        assert_eq!(room_in(free(), 100_000_000), Ok(()));
        let t = reserve(free, 60_000_000).unwrap();
        assert_eq!(promised(), 60_000_000);
        let e = room_in(free(), 50_000_000).unwrap_err();
        assert!(e.contains("60 MB set aside for loads under way"), "{e}");
        assert_eq!(room_in(free(), 40_000_000), Ok(()));
        drop(t);
        assert_eq!(promised(), 0);
        assert_eq!(room_in(free(), 100_000_000), Ok(()));
    }

    /// A WAV of `frames` frames: 16-bit PCM, or 32-bit float.
    fn wav(frames: u32, channels: u16, float: bool) -> Vec<u8> {
        let (tag, bits): (u16, u16) = if float { (3, 32) } else { (1, 16) };
        let align = channels * bits / 8;
        let data = frames * u32::from(align);
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&44_100u32.to_le_bytes());
        b.extend_from_slice(&(44_100 * u32::from(align)).to_le_bytes());
        b.extend_from_slice(&align.to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for i in 0..frames * u32::from(channels) {
            if float {
                b.extend_from_slice(&((i % 100) as f32 / 1000.0).to_le_bytes());
            } else {
                b.extend_from_slice(&((i % 100) as i16).to_le_bytes());
            }
        }
        b
    }

    /// What decoding `bytes` really holds at its peak: the file, the
    /// floats and the copy the sound is kept in.
    fn decoded_peak(name: &str, bytes: &[u8]) -> u64 {
        let (s, _) = mpfree_sampler::sound_file::read(name, bytes).unwrap();
        let floats = s.frames() * u64::from(s.channels()) * 4;
        bytes.len() as u64 + floats + s.data().bytes() as u64
    }

    fn need_of(bytes: &[u8]) -> u64 {
        decode_peak(&mut std::io::Cursor::new(bytes), bytes.len() as u64)
    }

    #[test]
    fn decoding_a_file_holds_the_file_its_floats_and_the_copy_kept() {
        for (name, b) in [
            ("Break.wav", wav(1000, 2, false)),
            ("Pad.wav", wav(777, 1, true)),
            (
                "Mono.flac",
                include_bytes!("../../mpfree-flac/tests/data/s16-mono-verbatim.flac").to_vec(),
            ),
            (
                "Stereo.flac",
                include_bytes!("../../mpfree-flac/tests/data/s16-stereo-auto.flac").to_vec(),
            ),
        ] {
            assert_eq!(need_of(&b), decoded_peak(name, &b), "{name}");
        }
        // A 24-bit file whose low bits are all empty is kept as 16 bits.
        // Its header can't say so, so it is counted as kept in 4 bytes a
        // sample: 2 more than it takes.
        let b = include_bytes!("../../mpfree-flac/tests/data/s24-mono-wasted.flac");
        let (s, _) = mpfree_sampler::sound_file::read("Deep.flac", b).unwrap();
        assert!(s.data().is_16_bit());
        assert_eq!(need_of(b), decoded_peak("Deep.flac", b) + 2 * s.frames());
        // A 16-bit WAV holds four times its sound at the peak, not three.
        let b = wav(10_000, 2, false);
        assert_eq!(need_of(&b), 44 + 40_000 * 4);
        // A float WAV, three.
        let b = wav(10_000, 2, true);
        assert_eq!(need_of(&b), 44 + 80_000 * 3);
    }

    #[test]
    fn a_flacs_size_says_nothing_its_header_says_how_long_it_is() {
        // STREAMINFO alone: 10 minutes of stereo at 44.1 kHz, in a file of
        // 42 bytes (silence compresses to almost nothing).
        let frames: u64 = 26_460_000;
        let mut b = b"fLaC".to_vec();
        b.extend_from_slice(&[0x80, 0, 0, 34]);
        b.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0]);
        // 44 100 Hz (20 bits), 2 channels (3 bits: 1), 16 bits (5 bits: 15),
        // then the frames in 36 bits.
        b.extend_from_slice(&[0x0A, 0xC4, 0x42, 0xF0 | (frames >> 32) as u8]);
        b.extend_from_slice(&(frames as u32).to_be_bytes());
        b.extend_from_slice(&[0; 16]);
        assert_eq!(b.len(), 42);
        // Floats, and the sound kept as 16 bits.
        assert_eq!(need_of(&b), 42 + frames * 2 * 6);
        // Behind an ID3 tag of 100 bytes.
        let mut tagged = b"ID3\x04\x00\x00\x00\x00\x00\x64".to_vec();
        tagged.extend_from_slice(&[0; 100]);
        tagged.extend_from_slice(&b);
        assert_eq!(need_of(&tagged), 152 + frames * 2 * 6);
        // Longer than the sampler takes: refused before decoding, so it
        // never holds more than the most a sound can be.
        let mut long = b.clone();
        long[21] |= 0x0F;
        assert_eq!(need_of(&long), 42 + MOST_SAMPLES * 6);
        // A length the header doesn't give: the most a sound can be, with
        // its floats doubling as they grow.
        let mut unsaid = b.clone();
        unsaid[21] &= 0xF0;
        unsaid[22..26].fill(0);
        assert_eq!(need_of(&unsaid), 42 + MOST_SAMPLES * 12);
        // Not a header we know: as if every byte were a sample.
        assert_eq!(need_of(&[7u8; 100]), 900);
        // Nothing there to read.
        assert_eq!(decode_peak_at(Path::new("/no/such/file.wav"), 5), 5);
    }

    #[test]
    fn free_memory_is_meminfos_available() {
        let m = "MemTotal:        2091008 kB\nMemFree:  100 kB\nMemAvailable:    1648640 kB\n";
        assert_eq!(free_in(m), Some(1_648_640 * 1024));
        assert_eq!(free_in("MemTotal: 5 kB\n"), None);
    }

    #[test]
    fn the_watch_says_once_when_memory_runs_low_and_once_when_it_is_back() {
        let mut w = Watch::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert_eq!(w.check(1_000_000_000, at(0)), None);
        assert_eq!(
            w.check(200_000_000, at(2)).as_deref(),
            Some(
                "memory: 200 MB free, under the 256 MB MPFree keeps spare; new sounds, stretches and \
                 auto-samples are refused until some are freed"
            )
        );
        // Still low, lower, or just over the floor: nothing more.
        assert_eq!(w.check(150_000_000, at(4)), None);
        assert_eq!(w.check(FLOOR + 1, at(6)), None);
        assert_eq!(w.check(FLOOR + BACK_ABOVE - 1, at(8)), None);
        assert_eq!(
            w.check(400_000_000, at(32)).as_deref(),
            Some("memory: 400 MB free again, after 30 s under 256 MB (at least 150 MB)")
        );
        assert_eq!(w.check(400_000_000, at(34)), None);
        // Low again: said again.
        assert!(w.check(100_000_000, at(36)).is_some());
    }

    #[test]
    fn the_rows_count_each_part_and_the_rest() {
        let h = Held {
            samples: SampleMemory {
                sounds: 24_500_000,
                count: 81,
                copies: 0,
                input: 21_168_000,
                recall: 21_168_000,
            },
            loops: (441_000, 13_230_000),
            synth: 3_000_000,
            rss_kb: Some(200_000),
            total_kb: Some(1_969_436),
            free_kb: Some(1_610_000),
            promised: 0,
        };
        let r = rows(&h);
        let pairs: Vec<(&str, &str)> = r.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("Memory", ""),
                ("Sampler: every kit's sounds", "24.5 MB · 81 sounds"),
                ("Sampler: input buffer (Rec)", "21.2 MB"),
                ("Sampler: Sample Recall (the master, 2 min)", "21.2 MB"),
                ("Looper: used · set aside", "3.5 · 105.8 MB"),
                ("Synth: wavetables", "3.0 MB"),
                // 204.8 MB resident less 175.7 MB counted.
                ("Everything else", "29.1 MB"),
                ("The app in all (resident)", "204.8 MB of 2016.7 MB"),
                ("Free for loads (keeps 256 MB spare)", "1648.6 MB"),
            ]
        );
        assert_eq!(r.last().unwrap().2, Tone::Normal);
        // A load under way: what it was promised, after the free memory.
        let r = rows(&Held {
            promised: 120_000_000,
            ..h
        });
        assert_eq!(
            (r.last().unwrap().0.as_str(), r.last().unwrap().1.as_str()),
            ("Set aside for loads under way", "120.0 MB")
        );
        // Under the floor: red.
        let r = rows(&Held {
            free_kb: Some(200_000),
            ..h
        });
        assert_eq!(r.last().unwrap().2, Tone::Bad);
        // No /proc sample yet (developer tools just turned on).
        let r = rows(&Held {
            rss_kb: None,
            free_kb: None,
            ..h
        });
        assert_eq!(r.last().unwrap().1, "not read yet");
        assert!(!r.iter().any(|r| r.0 == "Everything else"));
    }
}
