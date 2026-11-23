//! The hit track: where hits come in on the line input or a mix, for
//! smart loop end (the smart loop end plan, step 3).
//!
//! Pads and MIDI tell the looper the frame of every note
//! ([`crate::NoteStart`]). The line input and a mix don't, so the hit
//! track listens for them. It sums the input's two channels and splits
//! them into three bands (low, mid and high; [`LOW_HZ`], [`HIGH_HZ`]),
//! with filters worked out when the looper is made, and follows each
//! band's peaks: up at once, down slowly ([`RELEASE_MS`]), so a low note's
//! own cycles don't read as hits. Every [`HOP`] frames (689 a second at
//! 44.1 kHz) it takes each band's level, and marks a hit
//! where the levels rise clearly: by more than [`RISE_DB`] on average over
//! the bands within two steps, by more than [`ADAPT`] times the recent
//! rises (so a busy, wobbly sound needs a bigger step), at the top of the
//! rise, and at least [`crate::ends::TOGETHER_MS`] after the hit before.
//!
//! Each hit ([`Hit`]) has a frame, how loud it was, and its tone: each
//! band's level against the whole, over the [`PEAK_HOPS`] steps from it.
//! The frame is the middle of the step where the whole input's level
//! first jumped by [`JUMP_DB`] within the rise, then a latency earlier:
//! where the recording puts that sound. Two hits are alike when their
//! tones are ([`Hit::alike`]), so the loop's opening kick played again
//! matches it and a hat doesn't, and a decision weighs hits as it weighs
//! notes ([`crate::ends::decide`]).
//!
//! Each sample is used once and nothing allocates: the looper runs it on
//! the audio thread only while a free first loop may need it. The hits go
//! into a log like the notes ([`crate::ends::EventLog`]), whose head keeps
//! the opening's, so a long loop never needs the heard audio to find its
//! start again. A hit's frame is coarse (within a step); the finder places
//! the ends on the audio itself.

use crate::ends::{Event, EventLog, TOGETHER_MS};

/// Frames in a step of the hit track: one level per band each.
pub const HOP: usize = 64;
/// The low band is below this, the high band above [`HIGH_HZ`], and the
/// mid band between them.
pub const LOW_HZ: f32 = 200.0;
pub const HIGH_HZ: f32 = 2500.0;
/// How fast each band's level falls back after a peak (the time to fall
/// to about a third), low, mid and high, then the whole input's: longer
/// than a cycle of the band's lowest notes.
pub const RELEASE_MS: [f32; 4] = [50.0, 15.0, 5.0, 20.0];
/// A band's level is never taken as lower than this (dB under full
/// scale): a rise out of the noise floor counts from here.
pub const FLOOR_DB: f32 = -70.0;
/// Steps a rise is measured over (2: an attack up to 2.9 ms long at
/// 44.1 kHz rises in one go).
pub const RISE_STEPS: usize = 2;
/// The smallest rise that is a hit: dB, on average over the three bands.
pub const RISE_DB: f32 = 6.0;
/// A hit also rises by more than this many times the average rise of the
/// last [`RECENT`] steps.
pub const ADAPT: f32 = 2.0;
/// Steps of recent rises the threshold follows.
pub const RECENT: usize = 16;
/// Steps after a hit's start that its level and tone are taken over
/// (5.8 ms at 44.1 kHz).
pub const PEAK_HOPS: usize = 4;
/// Where in its rise a hit is placed: the first step whose whole level
/// jumped by this much (dB) over the step before.
pub const JUMP_DB: f32 = 3.0;
/// A hit's level is 0 this far under full scale (dB), 1 at full scale.
pub const LEVEL_RANGE_DB: f32 = 48.0;
/// Two tones this far apart (dB, over the three bands) are about a third
/// alike ([`Hit::alike`]).
pub const TONE_DB: f32 = 6.0;

/// The steps kept: the rise, the top a step later, and the hit's level
/// and tone after it.
const STEPS: usize = 8;

/// A hit heard on the line input or a mix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The frame it sounded, on the song's frame clock (as recorded: the
    /// input's latency taken off).
    pub frame: u64,
    /// How loud, 0 to 1 ([`LEVEL_RANGE_DB`]).
    pub level: f32,
    /// Its tone: the low, mid and high bands' levels, each less the
    /// whole's (dB, 0 or under).
    pub tone: [f32; 3],
}

impl Hit {
    /// How far apart two tones are, in dB over the three bands.
    pub fn tone_gap(&self, o: &Hit) -> f32 {
        let mut d = 0.0;
        for b in 0..3 {
            let x = self.tone[b] - o.tone[b];
            d += x * x;
        }
        d.sqrt()
    }
}

impl Event for Hit {
    const NONE: Hit = Hit {
        frame: 0,
        level: 0.0,
        tone: [0.0; 3],
    };

    fn frame(&self) -> u64 {
        self.frame
    }

    fn level(&self) -> f64 {
        f64::from(self.level)
    }

    /// 1 for the same tone, falling off with how far apart the tones are
    /// ([`TONE_DB`]): a kick and a snare are nothing alike.
    fn alike(&self, o: &Hit) -> f64 {
        let d = self.tone_gap(o) / TONE_DB;
        f64::from((-d * d).exp())
    }
}

/// The hits of a pass, as heard.
pub type HitLog = EventLog<Hit>;

/// A second-order filter (the audio cookbook's), in transposed direct
/// form II.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// A low-pass (`high` false) or high-pass at `hz`, Q 1/sqrt(2).
    fn new(rate: u32, hz: f32, high: bool) -> Biquad {
        let w = std::f64::consts::TAU * f64::from(hz) / f64::from(rate.max(1));
        let (sin, cos) = w.sin_cos();
        let alpha = sin / std::f64::consts::SQRT_2;
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high {
            ((1.0 + cos) / 2.0, -(1.0 + cos))
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos)
        };
        Biquad {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b0 / a0) as f32,
            a1: (-2.0 * cos / a0) as f32,
            a2: ((1.0 - alpha) / a0) as f32,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// One step's levels, in dB.
#[derive(Clone, Copy, Debug)]
struct Step {
    /// The arrival frame of its first sample.
    from: u64,
    /// Each band's peak level as followed, and in dB.
    energy: [f32; 3],
    band: [f32; 3],
    /// The whole input's level.
    whole: f32,
    /// The average rise over the bands from [`RISE_STEPS`] before.
    rise: f32,
}

impl Step {
    const NONE: Step = Step {
        from: 0,
        energy: [0.0; 3],
        band: [FLOOR_DB; 3],
        whole: FLOOR_DB,
        rise: 0.0,
    };
}

/// A hit found, waiting for its level and tone.
#[derive(Clone, Copy, Debug)]
struct Waiting {
    /// The step it starts on (a count of steps), and its frame.
    step: u64,
    frame: u64,
}

/// The hit track (see the module notes).
#[derive(Clone, Debug)]
pub struct HitTrack {
    rate: u32,
    low: Biquad,
    high: Biquad,
    mid_high: Biquad,
    mid_low: Biquad,
    /// Each band's peak as followed (low, mid, high, then the whole
    /// input's), and what it keeps of it each frame.
    peak: [f32; 4],
    keep: [f32; 4],
    filled: usize,
    /// The arrival frame of this step's first sample.
    from: u64,
    /// Steps taken since the track started, and the last ones.
    count: u64,
    steps: [Step; STEPS],
    /// Recent rises, for the threshold, and their sum.
    rises: [f32; RECENT],
    rise_sum: f32,
    /// The last hit's step, and one waiting for its level.
    last: Option<u64>,
    waiting: Option<Waiting>,
    /// Steps between two hits, at least.
    apart: u64,
    /// Hits found since the looper was made.
    pub found: u64,
}

impl HitTrack {
    /// A hit track for input at `rate`. Works out its filters: call it off
    /// the audio thread.
    pub fn new(rate: u32) -> HitTrack {
        let apart = (TOGETHER_MS * f64::from(rate) / 1000.0 / HOP as f64).ceil() as u64;
        HitTrack {
            rate,
            low: Biquad::new(rate, LOW_HZ, false),
            high: Biquad::new(rate, HIGH_HZ, true),
            mid_high: Biquad::new(rate, LOW_HZ, true),
            mid_low: Biquad::new(rate, HIGH_HZ, false),
            peak: [0.0; 4],
            keep: RELEASE_MS.map(|ms| (-1000.0 / (ms * rate.max(1) as f32)).exp()),
            filled: 0,
            from: 0,
            count: 0,
            steps: [Step::NONE; STEPS],
            rises: [0.0; RECENT],
            rise_sum: 0.0,
            last: None,
            waiting: None,
            apart: apart.max(1),
            found: 0,
        }
    }

    /// The rate it was made for.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Start again: the input changed or the track was idle, so what it
    /// heard before says nothing about the next step. The first steps
    /// after it find no hit (a sound already playing isn't one).
    pub fn reset(&mut self) {
        for f in [
            &mut self.low,
            &mut self.high,
            &mut self.mid_high,
            &mut self.mid_low,
        ] {
            f.reset();
        }
        self.peak = [0.0; 4];
        self.filled = 0;
        self.count = 0;
        self.steps = [Step::NONE; STEPS];
        self.rises = [0.0; RECENT];
        self.rise_sum = 0.0;
        self.last = None;
        self.waiting = None;
    }

    /// Listen to `input` (interleaved stereo; frame `i` arrived on frame
    /// `from + i`), each hit found into `log` with its frame `latency`
    /// earlier (where the recording puts it). Input that doesn't follow on
    /// from the last starts the track again ([`HitTrack::reset`]).
    pub fn hear(&mut self, from: u64, input: &[f32], latency: u32, log: &mut HitLog) {
        if self.from + self.filled as u64 != from {
            self.reset();
            self.from = from;
        }
        for x in input.chunks_exact(2) {
            // Tiny DC keeps the filters out of denormals in silence; the
            // high and mid bands take it out again.
            let m = 0.5 * (x[0] + x[1]) + 1e-18;
            let lo = self.low.run(m);
            let hi = self.high.run(m);
            let mid = self.mid_low.run(self.mid_high.run(m));
            for (b, y) in [lo, mid, hi, m].into_iter().enumerate() {
                self.peak[b] = y.abs().max(self.peak[b] * self.keep[b]);
            }
            self.filled += 1;
            if self.filled == HOP {
                self.step(latency, log);
            }
        }
    }

    /// A step is complete: its levels, its rise, and whether the step
    /// before was the top of a hit.
    fn step(&mut self, latency: u32, log: &mut HitLog) {
        let db = |p: f32| (20.0 * p.max(1e-9).log10()).max(FLOOR_DB);
        let p = self.peak;
        let energy = [p[0] * p[0], p[1] * p[1], p[2] * p[2]];
        let band = [db(p[0]), db(p[1]), db(p[2])];
        let whole = db(p[3]);
        let k = self.count;
        let back = self.at(k, RISE_STEPS);
        let rise = (0..3)
            .map(|b| (band[b] - back.band[b]).max(0.0))
            .sum::<f32>()
            / 3.0;
        self.steps[(k % STEPS as u64) as usize] = Step {
            from: self.from,
            energy,
            band,
            whole,
            rise,
        };
        self.from += HOP as u64;
        self.filled = 0;
        self.count += 1;
        // The step before is a hit's top: risen enough, more than the
        // recent rises, and no less than the steps either side.
        if k >= (RISE_STEPS + 1) as u64 {
            let top = self.at(k, 1);
            let before = self.at(k, 2);
            let recent = self.rise_sum / RECENT as f32;
            let clear = top.rise >= RISE_DB && top.rise > ADAPT * recent;
            let apart = self.last.is_none_or(|l| k > l + self.apart);
            if clear && apart && top.rise >= before.rise && top.rise > rise {
                // Placed on the first step in the rise whose whole level
                // jumped, else the top's.
                let mut start = k - 1;
                for j in (0..RISE_STEPS as u64).rev() {
                    let s = k - 1 - j;
                    let (a, b) = (self.at(k, (1 + j) as usize), self.at(k, (2 + j) as usize));
                    if a.whole - b.whole >= JUMP_DB {
                        start = s;
                        break;
                    }
                }
                let s = self.at(k, (k - start) as usize);
                let frame = (s.from + HOP as u64 / 2).saturating_sub(u64::from(latency));
                self.last = Some(start);
                self.waiting = Some(Waiting { step: start, frame });
            }
        }
        // The threshold follows the rises.
        let i = (k % RECENT as u64) as usize;
        self.rise_sum += rise - self.rises[i];
        self.rises[i] = rise;
        // A hit's level and tone, once its steps are in.
        if let Some(w) = self.waiting.filter(|w| k + 1 >= w.step + PEAK_HOPS as u64) {
            self.waiting = None;
            let mut e = [0.0f32; 3];
            let mut peak = FLOOR_DB;
            for s in w.step..w.step + PEAK_HOPS as u64 {
                let st = self.at(k, (k - s) as usize);
                for b in 0..3 {
                    e[b] += st.energy[b];
                }
                peak = peak.max(st.whole);
            }
            let all = (e[0] + e[1] + e[2]).max(1e-12);
            let tone = e.map(|x| (10.0 * (x.max(1e-12) / all).log10()).max(-40.0));
            let level = ((peak + LEVEL_RANGE_DB) / LEVEL_RANGE_DB).clamp(0.0, 1.0);
            log.push(Hit {
                frame: w.frame,
                level,
                tone,
            });
            self.found += 1;
        }
    }

    /// The step `back` steps before step `k` (`k` itself for 0), as kept;
    /// the floor before the track started.
    fn at(&self, k: u64, back: usize) -> Step {
        if back as u64 > k || back >= STEPS {
            return Step::NONE;
        }
        self.steps[((k - back as u64) % STEPS as u64) as usize]
    }
}

/// A tiny drum machine and guitar for tests and the bench: the synthetic
/// set the hit track and the finder are measured on (the smart loop end
/// plan's yardstick: clean hits within a step of the sampler's finder and
/// ends within 1 ms). Deterministic noise; nothing here runs in the app.
pub mod kit {
    /// What to play.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Sound {
        /// A pitched kick (0.8 s).
        Kick,
        /// Noise and a tone (0.5 s).
        Snare,
        /// A closed hat (0.12 s).
        Hat,
        /// A plucked string at 110 Hz with its harmonics (1.5 s), a
        /// softer attack.
        Pluck,
    }

    /// The drum machine: its noise.
    pub struct Kit {
        seed: u32,
        rate: f32,
    }

    impl Kit {
        pub fn new(rate: u32) -> Kit {
            Kit {
                seed: 0x1234_5678,
                rate: rate as f32,
            }
        }

        fn noise(&mut self) -> f32 {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
        }

        /// Adds `sound` at frame `at` of `out` (interleaved stereo), at
        /// `gain`.
        pub fn hit(&mut self, out: &mut [f32], at: usize, sound: Sound, gain: f32) {
            let rate = self.rate;
            let len = match sound {
                Sound::Kick => 0.8,
                Sound::Snare => 0.5,
                Sound::Hat => 0.12,
                Sound::Pluck => 1.5,
            };
            let frames = out.len() / 2;
            let n = ((len * rate) as usize).min(frames.saturating_sub(at));
            let mut phase = 0.0f32;
            for i in 0..n {
                let t = i as f32 / rate;
                let tau = std::f32::consts::TAU;
                let x = match sound {
                    Sound::Kick => {
                        let f = 50.0 + 100.0 * (-t * 30.0).exp();
                        phase += tau * f / rate;
                        phase.sin() * (-t * 9.0).exp()
                    }
                    Sound::Snare => {
                        0.6 * self.noise() * (-t * 18.0).exp()
                            + 0.4 * (tau * 190.0 * t).sin() * (-t * 25.0).exp()
                    }
                    Sound::Hat => {
                        let a = self.noise();
                        let b = self.noise();
                        0.5 * (a - b) * (-t * 80.0).exp()
                    }
                    Sound::Pluck => {
                        let attack = (t / 0.004).min(1.0);
                        let mut s = 0.0;
                        for h in 1..=6 {
                            let h = h as f32;
                            s += (tau * 110.0 * h * t).sin() * (-t * (2.0 + h)).exp() / h;
                        }
                        0.6 * attack * s
                    }
                };
                out[2 * (at + i)] += gain * x;
                out[2 * (at + i) + 1] += gain * x;
            }
        }

        /// A 4/4 beat at `bpm` for `bars` bars from frame `from` of `out`,
        /// with the next downbeat: kick on 1 and 3, snare on 2 and 4, hats
        /// on the eighths. `late(k)` moves the k-th eighth (ms). The hits'
        /// frames and sounds.
        pub fn beat(
            &mut self,
            out: &mut [f32],
            from: usize,
            bpm: f64,
            bars: u32,
            late: &dyn Fn(usize) -> f64,
        ) -> Vec<(usize, Sound)> {
            let eighth = 30.0 / bpm * f64::from(self.rate);
            let mut hits = Vec::new();
            for k in 0..=(8 * bars as usize) {
                let at = from as f64 + k as f64 * eighth + late(k) * f64::from(self.rate) / 1000.0;
                let at = at.round().max(0.0) as usize;
                let sound = match k % 8 {
                    0 | 4 => Sound::Kick,
                    2 | 6 => Sound::Snare,
                    _ => Sound::Hat,
                };
                let gain = match sound {
                    Sound::Kick => 0.8,
                    Sound::Snare => 0.7,
                    _ => 0.25,
                };
                self.hit(out, at, sound, gain);
                hits.push((at, sound));
            }
            hits
        }
    }
}

#[cfg(test)]
mod tests {
    use super::kit::{Kit, Sound};
    use super::*;

    const RATE: u32 = 44_100;

    fn heard(audio: &[f32], period: usize, latency: u32) -> Vec<Hit> {
        let mut t = HitTrack::new(RATE);
        let mut log = HitLog::new();
        let mut from = 0;
        for chunk in audio.chunks(2 * period) {
            t.hear(from, chunk, latency, &mut log);
            from += (chunk.len() / 2) as u64;
        }
        let mut buf = [Hit::NONE; crate::ends::HEAD + crate::ends::TAIL];
        let n = log.sorted(&mut buf);
        buf[..n].to_vec()
    }

    /// The synthetic beat's every hit is found once, within a step of
    /// where it starts (the frame `latency` earlier), at any period; a
    /// kick is louder than a hat, the kicks are alike and a kick and a
    /// hat aren't.
    #[test]
    fn every_hit_of_a_beat_is_found_once_within_a_step() {
        let frames = 3 * RATE as usize;
        let mut audio = vec![0.0; 2 * frames];
        let played = Kit::new(RATE).beat(&mut audio, 4410, 100.0, 1, &|_| 0.0);
        for (period, latency) in [(64, 0), (128, 0), (4096, 0), (300, 256)] {
            let hits = heard(&audio, period, latency);
            assert_eq!(hits.len(), played.len(), "{period}: {hits:?}");
            for (h, &(at, _)) in hits.iter().zip(&played) {
                let want = at as i64 - i64::from(latency);
                assert!(
                    (h.frame as i64 - want).abs() <= HOP as i64,
                    "{period}: {} for {want}",
                    h.frame
                );
            }
            let (kick, hat) = (hits[0], hits[1]);
            assert_eq!(played[1].1, Sound::Hat);
            assert!(kick.level > hat.level, "{kick:?} {hat:?}");
            assert!(kick.alike(&hits[4]) > 0.8, "{:?} {:?}", kick, hits[4]);
            assert!(kick.alike(&hat) < 0.1, "{kick:?} {hat:?}");
            assert!(hits[2].alike(&hits[6]) > 0.8, "the snares");
        }
    }

    /// Silence, a steady tone and quiet noise have no hits; a soft pluck
    /// does.
    #[test]
    fn steady_sound_is_no_hit_and_a_soft_pluck_is() {
        let frames = 2 * RATE as usize;
        assert!(heard(&vec![0.0; 2 * frames], 256, 0).is_empty());
        let mut tone = vec![0.0; 2 * frames];
        for i in 0..frames {
            let x = 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / RATE as f32).sin();
            let wobble = 1.0 + 0.1 * (std::f32::consts::TAU * 5.0 * i as f32 / RATE as f32).sin();
            tone[2 * i] = x * wobble;
            tone[2 * i + 1] = x * wobble;
        }
        // (The tone starting is a hit: from the floor.)
        let hits = heard(&tone, 256, 0);
        assert!(hits.len() <= 1, "{hits:?}");
        assert!(hits.iter().all(|h| h.frame < 2 * HOP as u64), "{hits:?}");
        let mut seed = 7u32;
        let mut noise = vec![0.0; 2 * frames];
        for x in noise.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *x = 0.01 * ((seed as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        let hits = heard(&noise, 256, 0);
        assert!(hits.len() <= 1, "{hits:?}");
        let mut pluck = noise.clone();
        Kit::new(RATE).hit(&mut pluck, RATE as usize, Sound::Pluck, 0.3);
        let hits = heard(&pluck, 256, 0);
        let near: Vec<_> = hits
            .iter()
            .filter(|h| h.frame.abs_diff(u64::from(RATE)) <= 4 * HOP as u64)
            .collect();
        assert_eq!(near.len(), 1, "{hits:?}");
    }

    /// The smart loop end plan's yardstick: on the synthetic set (clean
    /// hits: each sound has died away before the next), the hit track
    /// agrees with the sampler's hit finder (mpfree-analysis's onsets) to
    /// within one step, and both are within a step of the truth.
    #[test]
    fn the_hit_track_agrees_with_the_samplers_finder_within_a_step() {
        let frames = 9 * RATE as usize;
        let mut audio = vec![0.0; 2 * frames];
        let mut kit = Kit::new(RATE);
        let sounds = [
            (Sound::Kick, 0.8),
            (Sound::Snare, 0.6),
            (Sound::Hat, 0.3),
            (Sound::Pluck, 0.5),
            (Sound::Kick, 0.2),
            (Sound::Hat, 0.1),
            (Sound::Snare, 0.9),
            (Sound::Pluck, 0.15),
        ];
        let mut truth = Vec::new();
        for (i, &(sound, gain)) in sounds.iter().enumerate() {
            let at = 4410 + i * RATE as usize + i * 37;
            kit.hit(&mut audio, at, sound, gain);
            truth.push(at as u64);
        }
        let ours = heard(&audio, 256, 0);
        let theirs = mpfree_analysis::OnsetAnalysis::analyse(&audio, 2, RATE).unwrap();
        assert_eq!(ours.len(), truth.len(), "{ours:?}");
        for (h, &t) in ours.iter().zip(&truth) {
            // The sampler's hit there (its strongest candidate near it).
            let f = theirs
                .candidates()
                .iter()
                .filter(|c| c.frame.abs_diff(t) <= 1024)
                .max_by(|a, b| a.score.total_cmp(&b.score))
                .map(|c| c.frame)
                .unwrap_or_else(|| panic!("the sampler has no hit near {t}"));
            assert!(
                h.frame.abs_diff(f) <= HOP as u64,
                "{} and the sampler's {f}",
                h.frame
            );
            assert!(h.frame.abs_diff(t) <= HOP as u64, "{} for {t}", h.frame);
        }
    }
}
