//! The capture programme: every stimulus fxlab analyses, laid end to end
//! with room for tails, so a reference device (an AIR effect on the
//! factory MPC) can be recorded playing all of it in one pass and the
//! recording cut back into the same pieces.
//!
//! A programme starts with a sync chirp. A recording is lined up on it
//! (cross-correlation), then cut by the [`Schedule`], which lists every
//! segment's id, name, start and length in frames. The schedule is plain
//! tab-separated text so the on-device capture plugin can write it and a
//! person can read it. Each cut keeps the gap after its segment, so tails
//! are kept.

use crate::harmonics;
use crate::null::align;
use crate::response;
use crate::signals::{db_to_amp, impulse, level_steps, pink, Sweep};
use crate::wav::Stereo;

/// What a segment is, which says which analysis reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum Stim {
    /// The sync chirp at the start.
    Sync,
    /// An exponential sweep at a level (dBFS peak): frequency response.
    Sweep { level_db: f64 },
    /// A coherent sine (Hz, dBFS) whose last `harmonics::N` frames are
    /// analysed: distortion and aliasing.
    Sine { hz: f64, level_db: f64 },
    /// Pink noise at an RMS level (dBFS): loudness and level.
    Pink { rms_db: f64 },
    /// A unit impulse: decay, density, width.
    Impulse,
    /// A 1 kHz sine stepping low, high, low (dBFS), `secs` each: dynamics
    /// timing.
    Steps {
        low_db: f64,
        high_db: f64,
        secs: f64,
    },
    /// A music clip from the corpus, by file name.
    Clip { file: String },
    /// Silence: the device's own noise.
    Silence,
}

impl Stim {
    /// A short text form for the schedule: `sweep:-12`, `sine:1000:-18`,
    /// `pink:-18`, `impulse`, `steps:-40:0:1`, `clip:<file>`, `silence`,
    /// `sync`.
    pub fn code(&self) -> String {
        match self {
            Stim::Sync => "sync".into(),
            Stim::Sweep { level_db } => format!("sweep:{level_db}"),
            Stim::Sine { hz, level_db } => format!("sine:{hz}:{level_db}"),
            Stim::Pink { rms_db } => format!("pink:{rms_db}"),
            Stim::Impulse => "impulse".into(),
            Stim::Steps {
                low_db,
                high_db,
                secs,
            } => format!("steps:{low_db}:{high_db}:{secs}"),
            Stim::Clip { file } => format!("clip:{file}"),
            Stim::Silence => "silence".into(),
        }
    }

    pub fn from_code(code: &str) -> Option<Stim> {
        let f: Vec<&str> = code.splitn(4, ':').collect();
        let num = |i: usize| -> Option<f64> { f.get(i)?.parse().ok() };
        Some(match f[0] {
            "sync" => Stim::Sync,
            "sweep" => Stim::Sweep { level_db: num(1)? },
            "sine" => Stim::Sine {
                hz: num(1)?,
                level_db: num(2)?,
            },
            "pink" => Stim::Pink { rms_db: num(1)? },
            "impulse" => Stim::Impulse,
            "steps" => Stim::Steps {
                low_db: num(1)?,
                high_db: num(2)?,
                secs: num(3)?,
            },
            "clip" => Stim::Clip {
                file: code.strip_prefix("clip:")?.to_string(),
            },
            "silence" => Stim::Silence,
            _ => return None,
        })
    }
}

/// One piece of the programme.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub id: u32,
    pub name: String,
    pub stim: Stim,
    pub audio: Stereo,
}

/// Where each segment sits in a rendered programme.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: u32,
    pub name: String,
    pub start: usize,
    pub len: usize,
    /// Frames of silence after it, kept with it when cut.
    pub gap: usize,
    /// What it is ([`Stim::code`]); None in schedules from before the
    /// sixth column.
    pub stim: Option<Stim>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Schedule {
    pub rate: u32,
    pub entries: Vec<Entry>,
}

impl Schedule {
    /// `rate\t<rate>` then one `id\tname\tstart\tlen\tgap\tstim` line per
    /// entry.
    pub fn to_text(&self) -> String {
        let mut s = format!("rate\t{}\n", self.rate);
        for e in &self.entries {
            let stim = e.stim.as_ref().map(Stim::code).unwrap_or_default();
            s.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                e.id, e.name, e.start, e.len, e.gap, stim
            ));
        }
        s
    }

    pub fn from_text(text: &str) -> Result<Schedule, String> {
        let mut sched = Schedule::default();
        for (n, line) in text.lines().enumerate() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.first() == Some(&"rate") && f.len() == 2 {
                sched.rate = f[1]
                    .parse()
                    .map_err(|_| format!("line {}: bad rate", n + 1))?;
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            if f.len() != 5 && f.len() != 6 {
                return Err(format!("line {}: expected 5 or 6 fields", n + 1));
            }
            let num = |i: usize| -> Result<usize, String> {
                f[i].parse()
                    .map_err(|_| format!("line {}: bad number", n + 1))
            };
            sched.entries.push(Entry {
                id: num(0)? as u32,
                name: f[1].to_string(),
                start: num(2)?,
                len: num(3)?,
                gap: num(4)?,
                stim: f.get(5).and_then(|c| Stim::from_code(c)),
            });
        }
        Ok(sched)
    }
}

/// The 250 ms sync chirp (500 Hz to 8 kHz, -12 dBFS): bright and short, so
/// it lines up sharply.
pub fn sync_chirp(rate: u32) -> Vec<f64> {
    Sweep::new(500.0, 8000.0, 0.25, db_to_amp(-12.0), rate).sweep
}

/// A list of segments and the gap after each.
#[derive(Clone, Debug, Default)]
pub struct Programme {
    pub rate: u32,
    pub segments: Vec<(Segment, usize)>,
}

impl Programme {
    pub fn new(rate: u32) -> Programme {
        let mut p = Programme {
            rate,
            segments: Vec::new(),
        };
        p.push(
            "sync",
            Stim::Sync,
            Stereo::mono(sync_chirp(rate), rate),
            0.5,
        );
        p
    }

    /// Adds a segment followed by `gap_secs` of silence.
    pub fn push(&mut self, name: &str, stim: Stim, audio: Stereo, gap_secs: f64) {
        let id = self.segments.len() as u32;
        let gap = (gap_secs * self.rate as f64) as usize;
        self.segments.push((
            Segment {
                id,
                name: name.to_string(),
                stim,
                audio,
            },
            gap,
        ));
    }

    /// The standard measurement programme (about 50 seconds plus clips):
    /// sweeps at three levels, sines at four frequencies and two levels,
    /// pink noise, an impulse with a long tail, level steps and silence.
    pub fn standard(rate: u32) -> Programme {
        let mut p = Programme::new(rate);
        for level in [-24.0, -12.0, 0.0] {
            let s = response::sweep(level, rate);
            p.push(
                &format!("sweep {level:.0} dB"),
                Stim::Sweep { level_db: level },
                Stereo::mono(s.sweep, rate),
                1.0,
            );
        }
        let settle = rate as usize / 4;
        for hz in [100.0, 1000.0, 5000.0, 10_000.0] {
            for level in [-18.0, 0.0] {
                let (stim, _, _) = harmonics::stimulus(hz, level, settle, rate);
                p.push(
                    &format!("sine {hz:.0} Hz {level:.0} dB"),
                    Stim::Sine {
                        hz,
                        level_db: level,
                    },
                    stim,
                    0.5,
                );
            }
        }
        p.push(
            "pink -18 dB",
            Stim::Pink { rms_db: -18.0 },
            Stereo::mono(pink(4 * rate as usize, db_to_amp(-18.0), 11), rate),
            1.0,
        );
        p.push(
            "impulse",
            Stim::Impulse,
            Stereo::mono(impulse(1, 0.5), rate),
            8.0,
        );
        p.push(
            "steps -40/0 dB",
            Stim::Steps {
                low_db: -40.0,
                high_db: 0.0,
                secs: 1.0,
            },
            Stereo::mono(
                level_steps(1000.0, -40.0, 0.0, rate as usize, 3, rate),
                rate,
            ),
            1.0,
        );
        p.push(
            "silence",
            Stim::Silence,
            Stereo::mono(vec![0.0; 2 * rate as usize], rate),
            0.0,
        );
        p
    }

    /// The short programme for setting sweeps (about 14 seconds): one sweep
    /// at -12 dBFS, sines at 1 kHz -18 dBFS and 5 kHz 0 dBFS, pink noise, an
    /// impulse with two seconds of tail, and a short silence. Enough for
    /// response, distortion, aliasing, level and decay at every setting
    /// without the hour-long runs the standard programme would need.
    pub fn short(rate: u32) -> Programme {
        let mut p = Programme::new(rate);
        let s = response::sweep(-12.0, rate);
        p.push(
            "sweep -12 dB",
            Stim::Sweep { level_db: -12.0 },
            Stereo::mono(s.sweep, rate),
            0.5,
        );
        let settle = rate as usize / 4;
        for (hz, level) in [(1000.0, -18.0), (5000.0, 0.0)] {
            let (stim, _, _) = harmonics::stimulus(hz, level, settle, rate);
            p.push(
                &format!("sine {hz:.0} Hz {level:.0} dB"),
                Stim::Sine {
                    hz,
                    level_db: level,
                },
                stim,
                0.25,
            );
        }
        p.push(
            "pink -18 dB",
            Stim::Pink { rms_db: -18.0 },
            Stereo::mono(pink(2 * rate as usize, db_to_amp(-18.0), 11), rate),
            0.5,
        );
        p.push(
            "impulse",
            Stim::Impulse,
            Stereo::mono(impulse(1, 0.5), rate),
            2.0,
        );
        p.push(
            "silence",
            Stim::Silence,
            Stereo::mono(vec![0.0; rate as usize / 2], rate),
            0.0,
        );
        p
    }

    /// Adds a corpus clip, which must be at the programme's rate.
    pub fn push_clip(&mut self, file: &str, audio: Stereo, gap_secs: f64) -> Result<(), String> {
        if audio.rate != self.rate {
            return Err(format!(
                "{file} is at {} Hz, the programme at {}",
                audio.rate, self.rate
            ));
        }
        self.push(
            file,
            Stim::Clip {
                file: file.to_string(),
            },
            audio,
            gap_secs,
        );
        Ok(())
    }

    /// The whole programme as one signal, and its schedule.
    pub fn render(&self) -> (Stereo, Schedule) {
        let mut left = Vec::new();
        let mut right = Vec::new();
        let mut entries = Vec::new();
        for (seg, gap) in &self.segments {
            entries.push(Entry {
                id: seg.id,
                name: seg.name.clone(),
                start: left.len(),
                len: seg.audio.len(),
                gap: *gap,
                stim: Some(seg.stim.clone()),
            });
            left.extend_from_slice(&seg.audio.left);
            right.extend_from_slice(&seg.audio.right);
            left.resize(left.len() + gap, 0.0);
            right.resize(right.len() + gap, 0.0);
        }
        (
            Stereo::new(left, right, self.rate),
            Schedule {
                rate: self.rate,
                entries,
            },
        )
    }
}

/// Finds where the programme starts in a recording: the frame of the
/// recording that lines up with the programme's first frame, searching the
/// first `search_secs` seconds.
pub fn locate(recording: &Stereo, search_secs: f64) -> usize {
    let chirp = sync_chirp(recording.rate);
    let n = ((search_secs * recording.rate as f64) as usize).min(recording.len());
    let head: Vec<f64> = recording.mid()[..n].to_vec();
    let al = align(&head, &chirp, n);
    al.lag.round().max(0.0) as usize
}

/// Cuts a recording into its segments (each with its gap), from the
/// programme's first frame at `offset`.
pub fn cut(recording: &Stereo, schedule: &Schedule, offset: usize) -> Vec<(Entry, Stereo)> {
    schedule
        .entries
        .iter()
        .map(|e| (e.clone(), recording.slice(offset + e.start, e.len + e.gap)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schedule_round_trips_as_text() {
        let (_, s) = Programme::standard(44_100).render();
        assert_eq!(Schedule::from_text(&s.to_text()).unwrap(), s);
    }

    #[test]
    fn stim_codes_round_trip() {
        for p in [Programme::standard(44_100), Programme::short(44_100)] {
            let (_, s) = p.render();
            for e in &s.entries {
                let st = e.stim.as_ref().unwrap();
                assert_eq!(Stim::from_code(&st.code()).as_ref(), Some(st));
            }
        }
        let clip = Stim::Clip {
            file: "a:b.wav".into(),
        };
        assert_eq!(Stim::from_code(&clip.code()), Some(clip));
    }

    #[test]
    fn five_column_schedules_still_read() {
        let s = Schedule::from_text("rate\t44100\n0\tsync\t0\t100\t50\n").unwrap();
        assert_eq!(s.entries[0].stim, None);
        assert_eq!(s.entries[0].gap, 50);
    }

    #[test]
    fn a_delayed_recording_is_found_and_cut_back_exactly() {
        let p = Programme::standard(44_100);
        let (audio, sched) = p.render();
        let pad = 12_345;
        let mut rec = Stereo::mono(vec![0.0; pad], 44_100);
        rec.left.extend_from_slice(&audio.left);
        rec.right.extend_from_slice(&audio.right);
        let at = locate(&rec, 2.0);
        assert_eq!(at, pad);
        let pieces = cut(&rec, &sched, at);
        for ((seg, _), (_, piece)) in p.segments.iter().zip(&pieces) {
            assert_eq!(&piece.left[..seg.audio.len()], &seg.audio.left[..]);
        }
    }
}
