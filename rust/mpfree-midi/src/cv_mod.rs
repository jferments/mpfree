//! CV outputs that move on their own: LFOs and envelopes (powerhouse 3.9).
//!
//! An [`Lfo`] output sweeps between two voltages in one of seven
//! [`Shape`]s, at a rate in hertz or in beats of the song's tempo
//! ([`Rate`]). One that follows the tempo sits on the song: while the song
//! plays its cycle starts on the beat, so a one-bar LFO starts with each
//! bar; while it is stopped it keeps the tempo's pace. An LFO can restart
//! on its track's notes instead. Its voltage can snap to notes ([`Notes`]:
//! semitones at 1 V/oct, or a scale's), so a Random or Drift LFO plays a
//! melody on an oscillator.
//!
//! An [`Env`] output is an ADSR envelope of a track's notes: up in
//! `attack_ms` (a straight line), down to the sustain in `decay_ms`, and to
//! 0 V in `release_ms` once the track's last key lets go (both curves, as
//! an analog envelope's, are 99% there at their time). A note while keys
//! are held starts the attack again from where the envelope is.
//!
//! Their voltage is worked out every [`MOD_STEP_MS`] on a grid of frames
//! from the engine's start, and at each note that moves it, so the same
//! notes give the same voltages for any period size. Only a change is
//! sent. Nothing allocates.

/// How often an LFO's or envelope's voltage is worked out and sent: 250
/// times a second. A choice, not measured on the MPC: each change is two
/// control changes to the controller (see [`crate::cv_wire`]), so four
/// moving outputs send 2000 a second.
pub const MOD_STEP_MS: f32 = 4.0;
/// The slowest and fastest free LFO.
pub const LFO_MIN_HZ: f32 = 0.01;
pub const LFO_MAX_HZ: f32 = 20.0;
/// The shortest and longest LFO cycle that follows the tempo, in quarter
/// notes (a 1/32 note to 16 bars of 4/4).
pub const LFO_MIN_BEATS: f32 = 0.125;
pub const LFO_MAX_BEATS: f32 = 64.0;
/// The longest attack, decay or release.
pub const ENV_MAX_MS: f32 = 10_000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Starts at the low voltage, high halfway.
    #[default]
    Sine,
    Triangle,
    /// Low to high, then back at once.
    RampUp,
    RampDown,
    /// High for the first half of each cycle.
    Square,
    /// A new level each cycle, held (sample and hold).
    Random,
    /// From one random level to the next, smoothly.
    Drift,
}

impl Shape {
    pub const ALL: [Shape; 7] = [
        Shape::Sine,
        Shape::Triangle,
        Shape::RampUp,
        Shape::RampDown,
        Shape::Square,
        Shape::Random,
        Shape::Drift,
    ];

    /// The saved word and the screen's name.
    pub fn word(self) -> &'static str {
        match self {
            Shape::Sine => "sine",
            Shape::Triangle => "triangle",
            Shape::RampUp => "ramp-up",
            Shape::RampDown => "ramp-down",
            Shape::Square => "square",
            Shape::Random => "random",
            Shape::Drift => "drift",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shape::Sine => "Sine",
            Shape::Triangle => "Triangle",
            Shape::RampUp => "Ramp up",
            Shape::RampDown => "Ramp down",
            Shape::Square => "Square",
            Shape::Random => "Random",
            Shape::Drift => "Drift",
        }
    }

    pub fn from_word(w: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|s| s.word() == w)
    }

    /// 0 to 1 at `phase` (0 to 1) of cycle `cycle`; `seed` picks the
    /// output's own random levels.
    pub fn at(self, phase: f64, cycle: i64, seed: u64) -> f32 {
        let p = phase as f32;
        match self {
            Shape::Sine => (1.0 - (std::f32::consts::TAU * p).cos()) / 2.0,
            Shape::Triangle if p < 0.5 => 2.0 * p,
            Shape::Triangle => 2.0 - 2.0 * p,
            Shape::RampUp => p,
            Shape::RampDown => 1.0 - p,
            Shape::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Shape::Random => level(cycle, seed),
            Shape::Drift => {
                let (a, b) = (level(cycle, seed), level(cycle + 1, seed));
                a + (b - a) * (1.0 - (std::f32::consts::PI * p).cos()) / 2.0
            }
        }
    }
}

/// The random level (0 to 1) of a cycle: the same for the same cycle, so
/// a Random LFO on the song plays the same levels each time the song
/// passes the same bars.
fn level(cycle: i64, seed: u64) -> f32 {
    let mut z = (cycle as u64)
        .wrapping_add(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rate {
    /// Cycles a second, [`LFO_MIN_HZ`] to [`LFO_MAX_HZ`].
    Hz(f32),
    /// Quarter notes a cycle at the song's tempo (4 is a bar of 4/4).
    Beats(f32),
}

/// The notes an LFO's voltage snaps to: at 1 V/oct, 0 V is C (or `root`
/// semitones up). Pitch classes from the root, bit 0 the root itself; 0
/// leaves the voltage smooth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub mask: u16,
    pub root: u8,
}

impl Notes {
    pub const OFF: Notes = Notes { mask: 0, root: 0 };

    pub fn is_off(self) -> bool {
        self.mask & 0xFFF == 0
    }

    /// `volts` on the nearest note allowed (the lower of two as near).
    pub fn snap(self, volts: f32) -> f32 {
        if self.is_off() {
            return volts;
        }
        let semis = volts * 12.0;
        let near = semis.round() as i32;
        let ok = |n: i32| (self.mask >> (n - i32::from(self.root % 12)).rem_euclid(12)) & 1 == 1;
        let mut best = near;
        let mut best_d = f32::MAX;
        for d in -12..=12 {
            let n = near + d;
            let dist = (n as f32 - semis).abs();
            if ok(n) && (dist < best_d || (dist == best_d && n < best)) {
                best = n;
                best_d = dist;
            }
        }
        best as f32 / 12.0
    }
}

/// An LFO output's settings (see [`crate::cv::CvAssign::Lfo`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lfo {
    pub shape: Shape,
    pub rate: Rate,
    pub low_volts: f32,
    pub high_volts: f32,
    /// Restarts on each note of this track; `None` runs free (or on the
    /// song, at a tempo rate).
    pub restart: Option<u8>,
    pub notes: Notes,
}

impl Default for Lfo {
    fn default() -> Lfo {
        Lfo {
            shape: Shape::Sine,
            rate: Rate::Beats(4.0),
            low_volts: 0.0,
            high_volts: 5.0,
            restart: None,
            notes: Notes::OFF,
        }
    }
}

/// An envelope output's settings (see [`crate::cv::CvAssign::Envelope`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Env {
    pub track: u8,
    pub attack_ms: f32,
    pub decay_ms: f32,
    /// 0 to 1 of the top.
    pub sustain: f32,
    pub release_ms: f32,
    /// The top, at velocity 127 (or at any velocity with `velocity` off).
    pub volts: f32,
    /// The top follows each note's velocity.
    pub velocity: bool,
}

impl Env {
    pub fn new(track: u8) -> Env {
        Env {
            track,
            attack_ms: 5.0,
            decay_ms: 200.0,
            sustain: 0.6,
            release_ms: 300.0,
            volts: 5.0,
            velocity: false,
        }
    }
}

/// An exponential segment is this many time constants long, so it is 99%
/// there at its time.
const TAUS: f32 = 4.6;
/// Closer than this (of the top) ends a segment.
const NEAR: f32 = 1e-4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Where an envelope is: its stage, from frame `t0` at level `from`
/// (0 to 1), up to `top`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct EnvState {
    stage: Stage,
    t0: u64,
    from: f32,
    top: f32,
}

impl EnvState {
    /// The level (0 to 1) at frame `f`, moving to the next stage when one
    /// ends before it.
    pub(crate) fn level(&mut self, f: u64, e: &Env, rate: f32) -> f32 {
        let frames = |ms: f32| (ms.max(0.0) * rate / 1000.0) as f64;
        loop {
            let dt = f.saturating_sub(self.t0) as f64;
            match self.stage {
                Stage::Idle => return 0.0,
                Stage::Attack => {
                    let len = frames(e.attack_ms);
                    let left = (self.top - self.from).max(0.0) as f64;
                    // The top in `len` frames from 0, so less from higher.
                    let end = if len < 1.0 || self.top <= 0.0 {
                        0.0
                    } else {
                        (left / self.top as f64 * len).ceil()
                    };
                    if dt < end {
                        return self.from + (self.top - self.from) * (dt / end) as f32;
                    }
                    self.t0 += end as u64;
                    self.from = self.top;
                    self.stage = Stage::Decay;
                }
                Stage::Decay => {
                    let s = e.sustain.clamp(0.0, 1.0) * self.top;
                    let tau = frames(e.decay_ms) / f64::from(TAUS);
                    let v = if tau < 1.0 {
                        s
                    } else {
                        s + (self.from - s) * (-dt / tau).exp() as f32
                    };
                    if (v - s).abs() > NEAR {
                        return v;
                    }
                    self.stage = Stage::Sustain;
                }
                // The sustain follows its setting while keys are held.
                Stage::Sustain => return e.sustain.clamp(0.0, 1.0) * self.top,
                Stage::Release => {
                    let tau = frames(e.release_ms) / f64::from(TAUS);
                    let v = if tau < 1.0 {
                        0.0
                    } else {
                        self.from * (-dt / tau).exp() as f32
                    };
                    if v > NEAR {
                        return v;
                    }
                    self.stage = Stage::Idle;
                }
            }
        }
    }

    /// A note at frame `f`: the attack starts from where the envelope is.
    pub(crate) fn on(&mut self, f: u64, vel: u8, e: &Env, rate: f32) {
        let now = self.level(f, e, rate);
        *self = EnvState {
            stage: Stage::Attack,
            t0: f,
            from: now,
            top: if e.velocity {
                f32::from(vel) / 127.0
            } else {
                1.0
            },
        };
    }

    /// The track's last key let go at frame `f`.
    pub(crate) fn off(&mut self, f: u64, e: &Env, rate: f32) {
        if self.stage == Stage::Idle || self.stage == Stage::Release {
            return;
        }
        let now = self.level(f, e, rate);
        *self = EnvState {
            stage: Stage::Release,
            t0: f,
            from: now,
            top: self.top,
        };
    }

    #[cfg(test)]
    fn held(&self) -> bool {
        matches!(self.stage, Stage::Attack | Stage::Decay | Stage::Sustain)
    }
}

/// Where an LFO is: `pos` cycles (whole cycles count up, the fraction is
/// the phase) at frame `at`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LfoState {
    pub(crate) pos: f64,
    pub(crate) at: u64,
}

/// Part of a period with the song at one tempo (see
/// [`crate::cv::CvEngine::song`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SongSpan {
    pub(crate) offset: u32,
    pub(crate) len: u32,
    pub(crate) beat: f64,
    pub(crate) per_frame: f64,
    pub(crate) rolling: bool,
}

impl SongSpan {
    pub(crate) fn has(&self, offset: u32) -> bool {
        offset >= self.offset && offset - self.offset < self.len
    }

    pub(crate) fn beat_at(&self, offset: u32) -> f64 {
        self.beat + f64::from(offset.saturating_sub(self.offset)) * self.per_frame
    }
}

impl LfoState {
    /// The LFO's 0 to 1 at frame `f` (moving it there). `per_frame` is the
    /// tempo (quarter notes a frame); `song` the song's beat at `f` while
    /// it plays.
    pub(crate) fn value(
        &mut self,
        f: u64,
        l: &Lfo,
        rate: f32,
        per_frame: f64,
        song: Option<f64>,
        seed: u64,
    ) -> f32 {
        let dt = f.saturating_sub(self.at) as f64;
        match l.rate {
            Rate::Hz(hz) => {
                let hz = hz.clamp(LFO_MIN_HZ, LFO_MAX_HZ);
                self.pos += dt * f64::from(hz) / f64::from(rate);
            }
            Rate::Beats(b) => {
                let b = f64::from(b.clamp(LFO_MIN_BEATS, LFO_MAX_BEATS));
                match song {
                    Some(beat) if l.restart.is_none() => self.pos = beat / b,
                    _ => self.pos += dt * per_frame / b,
                }
            }
        }
        self.at = f;
        let cycle = self.pos.floor();
        l.shape.at(self.pos - cycle, cycle as i64, seed)
    }

    /// Restarted on a note at frame `f`: phase 0, and the next cycle's
    /// random level.
    pub(crate) fn restart(&mut self, f: u64) {
        self.pos = self.pos.floor() + 1.0;
        self.at = f;
    }
}

/// The voltage an LFO's 0 to 1 gives.
pub(crate) fn lfo_volts(l: &Lfo, x: f32) -> f32 {
    l.notes
        .snap(l.low_volts + (l.high_volts - l.low_volts) * x.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_start_where_they_say() {
        assert_eq!(Shape::Sine.at(0.0, 0, 1), 0.0);
        assert!((Shape::Sine.at(0.5, 0, 1) - 1.0).abs() < 1e-6);
        assert_eq!(Shape::Triangle.at(0.25, 0, 1), 0.5);
        assert_eq!(Shape::RampDown.at(0.0, 0, 1), 1.0);
        assert_eq!(Shape::Square.at(0.49, 0, 1), 1.0);
        assert_eq!(Shape::Square.at(0.5, 0, 1), 0.0);
        // Random holds a cycle; Drift goes from it to the next.
        let a = Shape::Random.at(0.1, 7, 3);
        assert_eq!(a, Shape::Random.at(0.9, 7, 3));
        assert_ne!(a, Shape::Random.at(0.1, 8, 3));
        assert_eq!(Shape::Drift.at(0.0, 7, 3), a);
        assert!((Shape::Drift.at(0.999_999, 7, 3) - Shape::Random.at(0.0, 8, 3)).abs() < 1e-4);
        for s in Shape::ALL {
            assert_eq!(Shape::from_word(s.word()), Some(s));
            for i in 0..100 {
                let v = s.at(i as f64 / 100.0, i, 5);
                assert!((0.0..=1.0).contains(&v), "{s:?} {v}");
            }
        }
    }

    #[test]
    fn notes_snap_to_the_scale_from_its_root() {
        let major = Notes {
            mask: 0b1010_1011_0101,
            root: 0,
        };
        // 1.04 V is 12.5 semitones: C or C#, and C# is not in C major.
        assert_eq!(major.snap(1.04), 1.0);
        assert_eq!(major.snap(0.5), 6.0 / 12.0 - 1.0 / 12.0); // F, not F#
        let d_major = Notes { root: 2, ..major };
        assert_eq!(d_major.snap(0.5), 6.0 / 12.0); // F#
        assert_eq!(Notes::OFF.snap(0.123), 0.123);
        let chromatic = Notes {
            mask: 0xFFF,
            root: 0,
        };
        assert_eq!(chromatic.snap(0.123), 1.0 / 12.0);
    }

    #[test]
    fn an_envelope_climbs_falls_to_its_sustain_and_lets_go() {
        let e = Env {
            attack_ms: 10.0,
            decay_ms: 100.0,
            sustain: 0.5,
            release_ms: 100.0,
            ..Env::new(0)
        };
        let rate = 1000.0; // a frame a millisecond
        let mut s = EnvState::default();
        assert_eq!(s.level(5, &e, rate), 0.0);
        s.on(10, 100, &e, rate);
        assert_eq!(s.level(15, &e, rate), 0.5);
        assert_eq!(s.level(20, &e, rate), 1.0);
        let d = s.level(120, &e, rate);
        assert!(d > 0.5 && d < 0.51, "{d}");
        assert_eq!(s.level(400, &e, rate), 0.5);
        assert!(s.held());
        s.off(400, &e, rate);
        let r = s.level(500, &e, rate);
        assert!(r > 0.0 && r < 0.01, "{r}");
        assert_eq!(s.level(700, &e, rate), 0.0);
        assert!(!s.held());
        // Again while still up: from where it is, not from 0.
        s.on(700, 100, &e, rate);
        s.off(705, &e, rate);
        s.on(706, 100, &e, rate);
        let v = s.level(706, &e, rate);
        assert!(v > 0.4 && v < 0.5, "{v}");
        // With velocity on, the top follows it.
        let ev = Env {
            velocity: true,
            attack_ms: 0.0,
            ..e
        };
        let mut s = EnvState::default();
        s.on(0, 127, &ev, rate);
        assert_eq!(s.level(0, &ev, rate), 1.0);
        let mut s = EnvState::default();
        s.on(0, 0, &ev, rate);
        assert_eq!(s.level(1, &ev, rate), 0.0);
    }
}
