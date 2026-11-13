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

use mpfree_ui::Tone;

use crate::cores::Row;
use crate::sample_page::SampleMemory;

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
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

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
            ]
        );
        // No /proc sample yet (developer tools just turned on).
        let r = rows(&Held { rss_kb: None, ..h });
        assert_eq!(r.last().unwrap().1, "not read yet");
        assert!(!r.iter().any(|r| r.0 == "Everything else"));
    }
}
