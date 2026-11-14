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
/// read (not Linux), and then every load is let through.
pub fn free_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    free_in(&text)
}

/// MemAvailable from /proc/meminfo's text, in bytes.
pub fn free_in(meminfo: &str) -> Option<u64> {
    crate::perf::kb_field(meminfo, "MemAvailable").map(|kb| kb * 1024)
}

/// Whether taking `need` bytes more leaves [`FLOOR`] free (`free`: the
/// memory free now, None when unknown). Err says why, in a musician's words.
pub fn room_in(free: Option<u64>, need: u64) -> Result<(), String> {
    let Some(free) = free else { return Ok(()) };
    if free.saturating_sub(need) >= FLOOR {
        return Ok(());
    }
    Err(format!(
        "{SHORT}: needs about {}, {} free, {} kept spare",
        mb_round(need),
        mb_round(free),
        mb_round(FLOOR)
    ))
}

/// What reading and decoding a sound file of `len` bytes holds at once:
/// the file, and its sound as 32-bit floats. A 16-bit WAV or AIFF doubles;
/// a FLAC, about half the size of its WAV, quadruples.
pub fn decode_need(name: &str, len: u64) -> u64 {
    let flac = name
        .rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("flac"));
    len.saturating_mul(if flac { 5 } else { 3 })
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
}

const LOOP_FRAME: usize = 2 * std::mem::size_of::<f32>();

fn mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

/// The rows, under a "Memory" heading row.
pub fn rows(h: &Held) -> Vec<Row> {
    let s = h.samples;
    let (used, kept) = (h.loops.0 * LOOP_FRAME, h.loops.1 * LOOP_FRAME);
    let counted = s.sounds + s.copies + s.input + kept + h.synth;
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
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_load_that_would_leave_less_than_the_floor_is_refused_and_says_why() {
        // Unknown (not Linux): let through.
        assert_eq!(room_in(None, u64::MAX), Ok(()));
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
    fn decoding_a_file_holds_the_file_and_its_sound_as_floats() {
        // A 16-bit WAV doubles, plus the file itself.
        assert_eq!(decode_need("Break 92.wav", 1_000_000), 3_000_000);
        assert_eq!(decode_need("/x/Pad.AIF", 10), 30);
        // A FLAC is about half its WAV.
        assert_eq!(decode_need("Loop.flac", 1_000_000), 5_000_000);
        assert_eq!(decode_need("LOOP.FLAC", 2), 10);
        assert_eq!(decode_need("no extension", 7), 21);
        assert_eq!(decode_need("x.wav", u64::MAX), u64::MAX);
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
            },
            loops: (441_000, 13_230_000),
            synth: 3_000_000,
            rss_kb: Some(200_000),
            total_kb: Some(1_969_436),
            free_kb: Some(1_610_000),
        };
        let r = rows(&h);
        let pairs: Vec<(&str, &str)> = r.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("Memory", ""),
                ("Sampler: every kit's sounds", "24.5 MB · 81 sounds"),
                ("Sampler: input buffer (Rec)", "21.2 MB"),
                ("Looper: used · set aside", "3.5 · 105.8 MB"),
                ("Synth: wavetables", "3.0 MB"),
                // 204.8 MB resident less 154.5 MB counted.
                ("Everything else", "50.3 MB"),
                ("The app in all (resident)", "204.8 MB of 2016.7 MB"),
                ("Free for loads (keeps 256 MB spare)", "1648.6 MB"),
            ]
        );
        assert_eq!(r.last().unwrap().2, Tone::Normal);
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
