//! Load shedding: when the periods fall behind, give up the voices nobody
//! hears rather than let the sound card's queue run dry and click, as the
//! Deluge does.
//!
//! - **When.** Right after each write the audio loop hands [`Shed::decide`]
//!   the period just made: how long its work took (from the last write to
//!   this one, the write's own wait left out) and how far the card's queue
//!   is below full. A period that took longer than it plays is falling
//!   behind; while the queue is still within a quarter of its spare
//!   periods of full, the queue takes it and nothing is given up, so a
//!   lone slow period costs nothing.
//! - **How many.** Enough voices to bring the period back to 90 % of its
//!   length, at the period's own cost a voice (the engines' time over their
//!   voices), at most [`MAX_AT_ONCE`] at once. When giving up every voice
//!   would still not bring it back (the time went to the mixer or the
//!   effects), none is: the music is not thinned for nothing. Those periods
//!   are counted and logged too.
//! - **Which.** The least audible voice across the sampler, the synth and
//!   the tracks' own instruments ([`Cull::audibility`]: the quietest, a
//!   releasing voice counted 12 dB quieter), one at a time, each with the
//!   short fade a stolen voice gets. A plugin's voices are never given up
//!   (its process doesn't say what it plays).
//! - **Said.** Every voice given up is counted, logged within a second
//!   ([`Shed::note`]), and shown on the Debug page; the 30 s status line
//!   has the totals.
//!
//! Nothing here allocates but the log lines, which are made at most once
//! a second and only after something happened.

use mpfree_node::Cull;
use std::time::{Duration, Instant};

/// The most voices given up after one period.
pub const MAX_AT_ONCE: usize = 8;
/// What a period behind is brought back to, as a share of its length.
const TARGET: f64 = 0.9;
/// The share of the queue's spare periods it must be down before anything
/// is given up (a lone slow period is the queue's to take).
const QUEUE_SHARE: i64 = 4;
/// The shortest time between two log lines.
const NOTE_EVERY: Duration = Duration::from_secs(1);

/// The period just made, as [`Shed::decide`] reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    /// Its work, from the last write to this one.
    pub work: Duration,
    /// Frames the card's queue is below full right after this write (None
    /// when the card can't say).
    pub short: Option<i64>,
    /// Voices sounding in the engines that can give one up.
    pub voices: usize,
    /// What those engines took this period.
    pub voice_time: Duration,
}

/// What to do after a period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// In time, or a lone slow period the queue takes.
    Keep,
    /// Behind: give up this many voices.
    GiveUp(usize),
    /// Behind, but giving up every voice would not catch up.
    Futile,
}

/// The rule, on its own: what to do after period `p`, for periods `period`
/// long and a queue with `spare` frames beyond the period playing.
pub fn decide(p: &Period, period: Duration, spare: i64) -> Decision {
    if p.work <= period {
        return Decision::Keep;
    }
    if p.short.is_some_and(|short| short < spare / QUEUE_SHARE) {
        return Decision::Keep;
    }
    let need = p.work.as_secs_f64() - period.as_secs_f64() * TARGET;
    let have = p.voice_time.as_secs_f64();
    if p.voices == 0 || have < need {
        return Decision::Futile;
    }
    let each = have / p.voices as f64;
    let n = (need / each).ceil() as usize;
    Decision::GiveUp(n.clamp(1, MAX_AT_ONCE).min(p.voices))
}

/// An engine that can give up voices.
pub trait Voices {
    /// The voice it would give up first.
    fn candidate(&self) -> Option<Cull>;
    /// Gives that voice up. True if it did.
    fn give_up(&mut self) -> bool;
}

impl Voices for crate::sampler::Player {
    fn candidate(&self) -> Option<Cull> {
        self.cull_candidate()
    }
    fn give_up(&mut self) -> bool {
        self.cull()
    }
}

impl Voices for crate::synth::Synth {
    fn candidate(&self) -> Option<Cull> {
        self.cull_candidate().map(|(_, c)| c)
    }
    fn give_up(&mut self) -> bool {
        self.cull_candidate().is_some_and(|(e, _)| self.cull(e))
    }
}

impl Voices for crate::instruments::Instruments {
    fn candidate(&self) -> Option<Cull> {
        self.cull_candidate().map(|(_, c)| c)
    }
    fn give_up(&mut self) -> bool {
        self.cull_candidate().is_some_and(|(s, _)| self.cull(s))
    }
}

/// Gives up the least audible voice across `engines` and says what it
/// was. None if none had one to give up. No allocation.
pub fn give_up_one(engines: &mut [&mut dyn Voices]) -> Option<Cull> {
    let best = engines.iter().enumerate().fold(None, |best, (i, e)| {
        Cull::first(best, e.candidate().map(|c| (i, c)))
    });
    let (i, c) = best?;
    engines[i].give_up().then_some(c)
}

/// Counts since the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// Voices given up, and how many of them were releasing.
    pub given_up: u64,
    pub releasing: u64,
    /// Periods that fell behind with the queue running down.
    pub behind: u64,
    /// Of those, the ones where giving up voices would not have caught up.
    pub futile: u64,
}

impl Totals {
    fn add(&mut self, o: &Totals) {
        self.given_up += o.given_up;
        self.releasing += o.releasing;
        self.behind += o.behind;
        self.futile += o.futile;
    }
}

/// Load shedding for the audio loop. See the module docs.
pub struct Shed {
    period: Duration,
    spare: i64,
    totals: Totals,
    /// Since the last log line, and the slowest period in it (its share of
    /// its length).
    window: Totals,
    worst: f64,
    noted: Option<Instant>,
}

impl Shed {
    /// For periods of `frames` frames at `rate`, with `queued` periods in
    /// the card's buffer.
    pub fn new(frames: usize, rate: u32, queued: u32) -> Shed {
        let mut s = Shed {
            period: Duration::from_secs_f64(frames as f64 / f64::from(rate.max(1))),
            spare: 0,
            totals: Totals::default(),
            window: Totals::default(),
            worst: 0.0,
            noted: None,
        };
        s.set_queued(frames, queued);
        s
    }

    /// The buffer changed size (Prefs' audio buffer).
    pub fn set_queued(&mut self, frames: usize, queued: u32) {
        self.spare = i64::from(queued.saturating_sub(1)) * frames as i64;
    }

    /// The frames the queue holds beyond the period playing.
    #[cfg(test)]
    pub fn spare(&self) -> i64 {
        self.spare
    }

    /// How many voices to give up after period `p` (0 when none), counted.
    pub fn decide(&mut self, p: &Period) -> usize {
        let d = decide(p, self.period, self.spare);
        if d != Decision::Keep {
            self.window.behind += 1;
            self.worst = self
                .worst
                .max(p.work.as_secs_f64() / self.period.as_secs_f64());
        }
        match d {
            Decision::Keep => 0,
            Decision::Futile => {
                self.window.futile += 1;
                0
            }
            Decision::GiveUp(n) => n,
        }
    }

    /// Counts a voice given up.
    pub fn gave_up(&mut self, c: &Cull) {
        self.window.given_up += 1;
        if c.releasing {
            self.window.releasing += 1;
        }
    }

    /// Gives up `n` voices across `engines`, one at a time, each the least
    /// audible left, and counts them. Returns how many went (fewer when
    /// the engines ran out). No allocation.
    pub fn give_up(&mut self, n: usize, engines: &mut [&mut dyn Voices]) -> usize {
        let mut done = 0;
        while done < n {
            let Some(c) = give_up_one(engines) else {
                break;
            };
            self.gave_up(&c);
            done += 1;
        }
        done
    }

    /// Counts since the start, the last second's included.
    pub fn totals(&self) -> Totals {
        let mut t = self.totals;
        t.add(&self.window);
        t
    }

    /// A line for the log when something happened since the last one, at
    /// most once a second ([`NOTE_EVERY`]). Allocates only then.
    pub fn note(&mut self, now: Instant) -> Option<String> {
        if self.window.behind == 0 {
            return None;
        }
        if self
            .noted
            .is_some_and(|t| now.saturating_duration_since(t) < NOTE_EVERY)
        {
            return None;
        }
        self.noted = Some(now);
        let w = std::mem::take(&mut self.window);
        self.totals.add(&w);
        let worst = std::mem::take(&mut self.worst) * 100.0;
        let mut parts = Vec::new();
        if w.given_up > 0 {
            parts.push(format!(
                "gave up {} to keep the sound going ({} of them releasing)",
                voices(w.given_up),
                w.releasing
            ));
        }
        if w.futile > 0 {
            parts.push(format!(
                "{} behind where giving up every voice would not have caught up (the time \
                 went to the mixer or the effects), so none was given up",
                periods(w.futile)
            ));
        }
        if parts.is_empty() {
            return None;
        }
        Some(format!(
            "load: {}; the slowest period took {worst:.0} % of its time",
            parts.join("; ")
        ))
    }

    /// The 30 s status line's part.
    pub fn describe(&self) -> String {
        let t = self.totals();
        format!(
            "load shedding: {} given up ({} releasing), {} behind, {} where voices would not \
             have helped",
            voices(t.given_up),
            t.releasing,
            periods(t.behind),
            t.futile
        )
    }
}

fn voices(n: u64) -> String {
    if n == 1 {
        "1 voice".to_string()
    } else {
        format!("{n} voices")
    }
}

fn periods(n: u64) -> String {
    if n == 1 {
        "1 period".to_string()
    } else {
        format!("{n} periods")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: usize = 128;
    const RATE: u32 = 44_100;

    fn us(x: f64) -> Duration {
        Duration::from_secs_f64(x * 1e-6)
    }

    fn period() -> Duration {
        Duration::from_secs_f64(PERIOD as f64 / f64::from(RATE))
    }

    #[test]
    fn a_period_in_time_or_a_lone_slow_one_gives_up_nothing() {
        let spare = 2 * PERIOD as i64;
        let p = Period {
            work: us(2500.0),
            short: Some(0),
            voices: 24,
            voice_time: us(2000.0),
        };
        assert_eq!(decide(&p, period(), spare), Decision::Keep);
        // Over its length, with the queue still near full.
        let p = Period {
            work: us(3500.0),
            short: Some(40),
            ..p
        };
        assert_eq!(decide(&p, period(), spare), Decision::Keep);
        // Nothing playing and in time.
        let p = Period {
            work: us(1000.0),
            short: Some(200),
            voices: 0,
            voice_time: Duration::ZERO,
        };
        assert_eq!(decide(&p, period(), spare), Decision::Keep);
    }

    #[test]
    fn falling_behind_gives_up_what_brings_the_period_back() {
        let spare = 2 * PERIOD as i64;
        // 3500 µs against 2902: back to 2612 needs 888 µs, at 100 µs a
        // voice: 9, at most 8 at once.
        let p = Period {
            work: us(3500.0),
            short: Some(100),
            voices: 20,
            voice_time: us(2000.0),
        };
        assert_eq!(decide(&p, period(), spare), Decision::GiveUp(8));
        // A little over: back to 2612 needs 338 µs, 4 voices.
        let p = Period {
            work: us(2950.0),
            short: Some(100),
            voices: 20,
            voice_time: us(2000.0),
        };
        assert_eq!(decide(&p, period(), spare), Decision::GiveUp(4));
        // Never more than are playing.
        let p = Period {
            work: us(3500.0),
            short: None,
            voices: 2,
            voice_time: us(1000.0),
        };
        assert_eq!(decide(&p, period(), spare), Decision::GiveUp(2));
    }

    #[test]
    fn voices_are_not_given_up_when_they_are_not_what_costs() {
        let spare = 2 * PERIOD as i64;
        let p = Period {
            work: us(4000.0),
            short: Some(200),
            voices: 8,
            voice_time: us(400.0),
        };
        assert_eq!(decide(&p, period(), spare), Decision::Futile);
        let p = Period {
            voices: 0,
            voice_time: Duration::ZERO,
            ..p
        };
        assert_eq!(decide(&p, period(), spare), Decision::Futile);
    }

    /// A voice in the simulated engine: its level and whether it is
    /// releasing.
    struct Sim {
        voices: Vec<Cull>,
    }

    impl Voices for Sim {
        fn candidate(&self) -> Option<Cull> {
            self.voices
                .iter()
                .copied()
                .fold(None, |b, c| Cull::first(b, Some(((), c))))
                .map(|(_, c)| c)
        }
        fn give_up(&mut self) -> bool {
            let Some(c) = self.candidate() else {
                return false;
            };
            let i = self.voices.iter().position(|v| *v == c).unwrap();
            self.voices.remove(i);
            true
        }
    }

    /// Runs `periods` periods of an engine whose voices cost `each` µs on
    /// top of `base` µs (plus `spike(i)`), through the queue of a card
    /// with 3 periods, giving up what [`Shed`] says. Returns the voices
    /// left, the underruns and what was given up.
    fn run(
        mut sim: Sim,
        base: f64,
        each: f64,
        periods: usize,
        spike: impl Fn(usize) -> f64,
    ) -> (Sim, usize, Totals) {
        let mut shed = Shed::new(PERIOD, RATE, 3);
        let frames_per_us = f64::from(RATE) * 1e-6;
        let mut short = 0.0f64;
        let mut underruns = 0;
        for i in 0..periods {
            let voice_us = each * sim.voices.len() as f64;
            let work = base + voice_us + spike(i);
            // The write waits when the period was quicker than it plays.
            short = (short + (work - 2902.494) * frames_per_us).max(0.0);
            if short > shed.spare() as f64 {
                underruns += 1;
                short = 0.0;
            }
            let p = Period {
                work: us(work),
                short: Some(short as i64),
                voices: sim.voices.len(),
                voice_time: us(voice_us),
            };
            let n = shed.decide(&p);
            shed.give_up(n, &mut [&mut sim]);
        }
        let t = shed.totals();
        (sim, underruns, t)
    }

    fn voices(n: usize) -> Sim {
        Sim {
            voices: (0..n)
                .map(|i| Cull {
                    level: 0.1 + 0.9 * i as f32 / n as f32,
                    releasing: i % 3 == 0,
                })
                .collect(),
        }
    }

    #[test]
    fn a_busy_bar_loses_a_few_quiet_voices_and_never_runs_dry() {
        // 24 voices at 110 µs each on 800 µs: 3440 µs, 119 % of a period.
        let (sim, underruns, t) = run(voices(24), 800.0, 110.0, 2000, |_| 0.0);
        assert_eq!(underruns, 0);
        // Down to what fits (at most 19 voices) and not much further.
        assert!(sim.voices.len() <= 19, "{}", sim.voices.len());
        assert!(sim.voices.len() >= 15, "{}", sim.voices.len());
        assert_eq!(t.given_up as usize, 24 - sim.voices.len());
        // The quietest went: everything left is louder than what went,
        // counting releasing voices 12 dB down.
        let kept = sim
            .voices
            .iter()
            .map(|c| c.audibility())
            .fold(f32::MAX, f32::min);
        let all = voices(24);
        let gone = all
            .voices
            .iter()
            .filter(|c| !sim.voices.contains(c))
            .map(|c| c.audibility())
            .fold(0.0, f32::max);
        assert!(gone <= kept, "gone {gone} kept {kept}");
    }

    #[test]
    fn a_sudden_jump_is_caught_before_the_queue_runs_dry() {
        // 40 voices at 60 µs on 600 µs: 3000 µs to start, 103 %; a second
        // later, an effect jumps the base by 1500 µs: 155 %.
        let (sim, underruns, t) = run(voices(40), 600.0, 60.0, 1000, |i| {
            if i >= 344 {
                1500.0
            } else {
                0.0
            }
        });
        assert_eq!(underruns, 0);
        // 600 + 1500 + 60 n under 2902: n at most 13.
        assert!(sim.voices.len() <= 13, "{}", sim.voices.len());
        assert!(t.given_up >= 27);
    }

    #[test]
    fn lone_slow_periods_give_up_nothing() {
        // 2400 µs with a 4000 µs period every 50: the queue takes each.
        let (sim, underruns, t) = run(voices(24), 1000.0, 58.0, 2000, |i| {
            if i % 50 == 0 {
                1600.0
            } else {
                0.0
            }
        });
        assert_eq!(underruns, 0);
        assert_eq!(sim.voices.len(), 24);
        assert_eq!(t, Totals::default());
    }

    #[test]
    fn an_overloaded_mixer_keeps_its_voices() {
        // The mixer alone is over the period: voices would not help.
        let (sim, underruns, t) = run(voices(8), 3200.0, 20.0, 500, |_| 0.0);
        assert!(underruns > 0);
        assert_eq!(sim.voices.len(), 8);
        assert_eq!(t.given_up, 0);
        assert!(t.futile > 0);
    }

    #[test]
    fn the_log_says_what_went_at_most_once_a_second() {
        let mut shed = Shed::new(PERIOD, RATE, 3);
        let now = Instant::now();
        assert_eq!(shed.note(now), None);
        let behind = Period {
            work: us(3500.0),
            short: Some(200),
            voices: 20,
            voice_time: us(2000.0),
        };
        let n = shed.decide(&behind);
        let mut sim = voices(20);
        assert_eq!(shed.give_up(n, &mut [&mut sim]), 8);
        let line = shed.note(now).expect("a line");
        assert!(
            line.starts_with("load: gave up 8 voices to keep the sound going ("),
            "{line}"
        );
        assert!(
            line.ends_with("the slowest period took 121 % of its time"),
            "{line}"
        );
        // Within the second: nothing more, but still counted.
        shed.decide(&behind);
        assert_eq!(shed.note(now + Duration::from_millis(500)), None);
        let futile = Period {
            voice_time: us(100.0),
            ..behind
        };
        shed.decide(&futile);
        let line = shed.note(now + NOTE_EVERY).expect("a line");
        assert!(
            line.starts_with("load: 1 period behind where giving up every voice"),
            "{line}"
        );
        assert_eq!(
            shed.totals(),
            Totals {
                given_up: 8,
                releasing: shed.totals().releasing,
                behind: 3,
                futile: 1,
            }
        );
        assert!(shed
            .describe()
            .starts_with("load shedding: 8 voices given up ("));
    }
}
