//! Strings: a string machine, after the divide-down string ensembles of
//! the 1970s, built from `docs/dev/research/string-machine.md`.
//!
//! - **The bank.** One sawtooth for every note, 0 to 127. In a tuning
//!   whose octaves are pure (12-TET and every 12-note temperament) each
//!   note's phase is exactly twice the phase of the note an octave below,
//!   as when the top octave is divided down: the octaves are locked, so
//!   an 8' and a 4' make one steady wave and two keys an octave apart never
//!   beat. In any other tuning (24-TET, a stretched octave) each note
//!   runs free at its own pitch, so every key is in tune whatever the
//!   tuning; only the locking is lost, and the cost is the same. Locked
//!   octaves sit half a cycle apart, so their steps never meet and a
//!   chord's peaks stay low. A note's phase follows from the absolute
//!   frame, so a silent oscillator costs nothing and starts where it would
//!   have been. Band-limited with the four-point step correction
//!   ([`poly_blep4`]).
//! - **Keys.** A key never starts an oscillator: it sets how much of the
//!   bank's notes at 16', 8' and 4' (an octave down, the note, an octave
//!   up) reach three footage buses. Every key can sound, and two keys that
//!   share a note (one key's 4', the next octave's 8') add on one
//!   oscillator.
//!   A footage above B7 folds back an octave, 6 dB down.
//! - **Registers.** Violin, Viola, Cello, Contrabass, Horn and Trumpet:
//!   each a mix of the three footages through its own fixed filter (a
//!   high-pass, a body resonance and a low-pass), mixed at any level and
//!   together at constant power, so a mix of several is about as loud as
//!   one. The filters run on the buses, so their cost does not grow with
//!   keys.
//!   **Brightness** moves every low-pass an octave either way (to 10 kHz
//!   at most), **Body** scales the resonances. The voicings are our reading of how these
//!   machines' registers sound, not measured from one.
//! - **Swell.** Crescendo (silence to full level) and Sustain (60 dB
//!   down), per key or shared: shared, a chord swells in together and the
//!   last chord fades out, while a key let go with others still held stops
//!   at once, as on the old machines. **Velocity** sets how much quieter
//!   a soft key is, and a soft key is darker too: each footage bus comes
//!   in a dark and a bright half, the dark one through low-passes an
//!   octave lower, and a key's velocity shares its level between them.
//!   Keyboard level scaling keeps single notes within 3 dB of each other
//!   from C2 to C6: up 2.5 dB an octave below C3 and 1.5 dB from C3 to
//!   C5, against what the registers' fixed filters and the ear take from
//!   low notes, and down 2 dB an octave above F#5.
//! - **Bow noise**, band-passed, following how loud the keys are.
//! - **The Ensemble** (`mpfree_fx::Ensemble`) on the output: three or four
//!   swept lines, the sound of these machines.
//! - **Playing it.** The sustain and sostenuto pedals are heard here
//!   ([`Pedals`]); the soft pedal lowers the level 6 dB and darkens it by
//!   half an octave; the mod wheel adds vibrato (the ensemble's fast
//!   sweep); a breath or foot controller sets the volume. Bend moves the
//!   whole bank. Pressure and per-note expression are ignored.
//!
//! The swells, the bank's levels, bend and the filters' settings move at a
//! control rate of one step every [`CTRL`] frames aligned to the absolute
//! frame, with each oscillator's level ramping across the step, so the
//! output does not depend on how the host cuts blocks. A tenth of a second
//! after the last note is gone the bank and the ensemble stop running, so
//! a Strings track at rest costs next to nothing. Nothing allocates in
//! `process`.

use mpfree_fx::ids::ensemble as ens;
use mpfree_fx::Ensemble;
use mpfree_node::pedal::Pedals;
use mpfree_node::{Ctl, Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Tuning};

use crate::bank::{Bank, Shape};
use crate::dsp::Rng;
use crate::params::Preset;
use crate::strings_params::*;
use crate::voice::{velocity_gain, Adsr, Stage};

/// Frames per control step: the bank's.
pub const CTRL: u64 = crate::bank::STEP as u64;
const STEP: usize = CTRL as usize;
/// Notes in the bank, and keys.
pub const NOTES: usize = crate::bank::NOTES;
/// The footages' offsets in semitones: 16', 8', 4'.
pub const FOOTAGES: [i32; 3] = [-12, 0, 12];
/// The buses into the output: one key of the default registers peaks
/// near -16 dBFS, so an eight-note chord stays under full scale.
const OUT_GAIN: f32 = 0.045;
/// The bank's top for a footage: a footage note above it folds back down
/// an octave, as the top-octave dividers end there, so the top keys'
/// 4' never climbs past B7.
pub const FOLD_ABOVE: i32 = 107;
/// A folded footage lands on a note the 8' often plays too, in phase, so
/// it comes in 6 dB down: the top keys stay about as loud as the rest.
const FOLD_GAIN: f32 = 0.5;
/// Keyboard level scaling, dB an octave of the note that sounds: up below
/// C3 and a little less up to C5, against what the registers' fixed
/// filters and the ear take from low notes, and down above F#5, where a
/// bright register's notes reach the ear's most sensitive octaves. Single
/// notes then stay within 3 dB of each other from C2 to C6 (L11).
const KEY_FOLLOW: KeyFollow = KeyFollow {
    low_below: 48,
    low_db: 2.5,
    mid_below: 72,
    mid_db: 1.5,
    top_above: 78,
    top_db: -2.0,
};

struct KeyFollow {
    low_below: i32,
    low_db: f32,
    mid_below: i32,
    mid_db: f32,
    top_above: i32,
    top_db: f32,
}

impl KeyFollow {
    /// The level, as a gain, for a note sounding at `pitch`.
    fn gain(&self, pitch: i32) -> f32 {
        let oct = |n: i32| n.max(0) as f32 / 12.0;
        let low = oct(self.low_below - pitch) * self.low_db;
        let mid = oct((self.mid_below - pitch).min(self.mid_below - self.low_below)) * self.mid_db;
        let top = oct(pitch - self.top_above) * self.top_db;
        ((low + mid + top) / 20.0 * std::f32::consts::LN_10).exp()
    }
}
/// The footage buses, each a dark and a bright half: bus `2 * f + 1` is
/// footage `f`'s bright half.
const BUSES: usize = 6;
/// The dark half's low-passes against the bright half's, octaves.
const DARK_OCTAVES: f32 = -1.0;
/// The highest a register's low-pass opens, Hz: the machines' outputs
/// were dull above it, and it keeps a C7's aliases 60 dB down.
const LOW_PASS_TOP: f32 = 10_000.0;
/// The ensemble's own depths at Depth 100 %, cents.
const SLOW_CENTS: f32 = 12.0;
const FAST_CENTS: f32 = 5.0;
/// Vibrato the mod wheel adds at full, cents.
const WHEEL_CENTS: f32 = 8.0;
/// A key's contacts in the shared swell: how fast it comes in and stops.
const CONTACT_ON_MS: f32 = 4.0;
const CONTACT_OFF_MS: f32 = 30.0;
/// The soft pedal: level and brightness.
const SOFT_GAIN: f32 = 0.5012;
const SOFT_OCTAVES: f32 = -0.5;
/// Bow noise at 100 % against a key's level, and its band.
const NOISE_GAIN: f32 = 0.35;
const NOISE_HZ: f32 = 3_200.0;
/// How long the ensemble runs on after the last note is gone, ms: longer
/// than its longest line, so what is skipped after it is silence.
const QUIET_MS: f32 = 100.0;

/// The built-in Ensemble's settings for these values and a mod wheel at
/// `wheel` (0 to 1): ids and values of `mpfree_fx::ids::ensemble`.
pub fn ensemble_settings(v: impl Fn(u16) -> f32, wheel: f32) -> [(u16, f32); 7] {
    let depth = v(DEPTH) / 100.0;
    [
        (ens::MIX, v(ENSEMBLE)),
        (ens::LINES, v(LINES)),
        (ens::SLOW_RATE, v(SLOW_RATE)),
        (ens::FAST_RATE, v(FAST_RATE)),
        (ens::SLOW_DEPTH, (SLOW_CENTS * depth).min(30.0)),
        (
            ens::FAST_DEPTH,
            (FAST_CENTS * depth + WHEEL_CENTS * wheel).min(15.0),
        ),
        (ens::WIDTH, v(WIDTH)),
    ]
}

/// The registers' tone, for drawing: the gain at a frequency (Hz) of the
/// mix these values set, each register through its filters at its level,
/// Brightness and Body, for a key at full velocity.
pub fn tone_response(v: impl Fn(u16) -> f32) -> impl Fn(f32) -> f32 {
    const AT: f32 = 48_000.0;
    let mult = ((v(BRIGHTNESS) - 50.0) / 50.0).exp2();
    let body = v(BODY) / 50.0;
    let mix: f32 = REGISTERS
        .iter()
        .map(|(id, _)| (v(*id) / 100.0).powi(2))
        .sum();
    let norm = 1.0 / mix.sqrt().max(1.0);
    let parts: Vec<(f32, [Biquad; 3])> = REGISTERS
        .iter()
        .zip(VOICING.iter())
        .filter_map(|((id, _), r)| {
            let g = v(*id) / 100.0 * r.trim * norm;
            (g > 0.0).then(|| {
                (
                    g,
                    [
                        Biquad::high_pass(r.high_pass_hz, 0.707, AT),
                        Biquad::peak(r.body_hz, r.body_q, r.body_db * body, AT),
                        Biquad::low_pass(
                            (r.low_pass_hz * mult).min(LOW_PASS_TOP),
                            r.low_pass_q,
                            AT,
                        ),
                    ],
                )
            })
        })
        .collect();
    move |hz| {
        parts
            .iter()
            .map(|(g, f)| g * f.iter().map(|b| b.gain_at(hz, AT)).product::<f32>())
            .sum()
    }
}

/// A key's level, for drawing: swelled in over Crescendo, let go at
/// `held` seconds and faded over Sustain, at `points` even times from 0 to
/// `secs` seconds. The swell's own envelope, stepped finely.
pub fn swell_curve(v: impl Fn(u16) -> f32, held: f32, secs: f32, points: usize) -> Vec<f32> {
    const SPS: f32 = 4_000.0;
    let mut env = Adsr::default();
    env.set(v(CRESCENDO), 0.0, 1.0, v(SUSTAIN), SPS);
    env.gate_on();
    let off = (held * SPS) as usize;
    let mut step = 0usize;
    let mut level = 0.0;
    (0..points)
        .map(|i| {
            let at = (i as f32 / (points.max(2) - 1) as f32 * secs * SPS) as usize;
            while step < at {
                if step == off {
                    env.gate_off();
                }
                level = env.next();
                step += 1;
            }
            level
        })
        .collect()
}

/// The ensemble's lines, for drawing: each line's delay against the
/// lines' shared delay (-1 to 1 of the widest sweep) at `points` even
/// times across one slow cycle, the fast sweep riding on it. Empty with
/// the ensemble off.
pub fn ensemble_lines(v: impl Fn(u16) -> f32, points: usize) -> Vec<Vec<f32>> {
    if v(ENSEMBLE) <= 0.0 {
        return Vec::new();
    }
    let mut e = Ensemble::new();
    for (id, value) in ensemble_settings(&v, 0.0) {
        e.set_param(id, value);
    }
    let (slow, fast, base) = e.sweeps_ms();
    let span = (slow + fast).max(1e-3);
    let ratio = v(FAST_RATE) / v(SLOW_RATE).max(1e-3);
    (0..4)
        .map_while(|k| {
            (0..points)
                .map(|i| {
                    let x = i as f32 / (points.max(2) - 1) as f32;
                    e.delay_ms(k, x, (ratio * x).fract())
                        .map(|d| (d - base) / span)
                })
                .collect::<Option<Vec<f32>>>()
        })
        .collect()
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.strings",
    name: "Strings",
    kind: Kind::Instrument,
};

/// A register: its footages' levels (16', 8', 4'), its filters, and a
/// trim that makes each register alone about as loud as the others.
#[derive(Clone, Copy, Debug)]
pub struct Register {
    pub feet: [f32; 3],
    pub high_pass_hz: f32,
    pub body_hz: f32,
    pub body_q: f32,
    /// The body resonance at Body 50 %, dB.
    pub body_db: f32,
    pub low_pass_hz: f32,
    pub low_pass_q: f32,
    pub trim: f32,
}

/// The registers, in the order of [`REGISTERS`].
pub const VOICING: [Register; 6] = [
    // Violin: bright, the 4' over the 8', a presence peak.
    Register {
        feet: [0.0, 1.0, 0.35],
        high_pass_hz: 180.0,
        body_hz: 2_800.0,
        body_q: 1.0,
        body_db: 5.0,
        low_pass_hz: 6_500.0,
        low_pass_q: 0.707,
        trim: 0.85,
    },
    // Viola: the 8' alone, darker, a lower body.
    Register {
        feet: [0.0, 1.0, 0.0],
        high_pass_hz: 120.0,
        body_hz: 1_100.0,
        body_q: 1.0,
        body_db: 5.0,
        low_pass_hz: 4_200.0,
        low_pass_q: 0.707,
        trim: 1.0,
    },
    // Cello: 16' with some 8', a low body.
    Register {
        feet: [1.0, 0.5, 0.0],
        high_pass_hz: 50.0,
        body_hz: 450.0,
        body_q: 1.2,
        body_db: 5.0,
        low_pass_hz: 3_000.0,
        low_pass_q: 0.707,
        trim: 0.8,
    },
    // Contrabass: 16' only, dark.
    Register {
        feet: [1.0, 0.0, 0.0],
        high_pass_hz: 30.0,
        body_hz: 220.0,
        body_q: 1.2,
        body_db: 4.0,
        low_pass_hz: 1_500.0,
        low_pass_q: 0.707,
        trim: 1.2,
    },
    // Horn: mellow, a resonant low-pass.
    Register {
        feet: [0.3, 1.0, 0.0],
        high_pass_hz: 80.0,
        body_hz: 700.0,
        body_q: 1.5,
        body_db: 6.0,
        low_pass_hz: 2_200.0,
        low_pass_q: 1.4,
        trim: 0.9,
    },
    // Trumpet: 8' and 4', bright with a bite.
    Register {
        feet: [0.0, 1.0, 0.5],
        high_pass_hz: 250.0,
        body_hz: 1_500.0,
        body_q: 1.5,
        body_db: 8.0,
        low_pass_hz: 5_000.0,
        low_pass_q: 1.6,
        trim: 0.7,
    },
];

/// A second-order section (transposed direct form II).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn from(b: [f64; 3], a: [f64; 3]) -> Biquad {
        let n = 1.0 / a[0];
        Biquad {
            b0: (b[0] * n) as f32,
            b1: (b[1] * n) as f32,
            b2: (b[2] * n) as f32,
            a1: (a[1] * n) as f32,
            a2: (a[2] * n) as f32,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn w(hz: f32, rate: f32) -> (f64, f64) {
        let w =
            2.0 * std::f64::consts::PI * f64::from(hz.clamp(10.0, rate * 0.45)) / f64::from(rate);
        (w.cos(), w.sin())
    }

    /// The cookbook low-pass.
    pub fn low_pass(hz: f32, q: f32, rate: f32) -> Biquad {
        let (c, s) = Self::w(hz, rate);
        let al = s / (2.0 * f64::from(q));
        Biquad::from(
            [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
            [1.0 + al, -2.0 * c, 1.0 - al],
        )
    }

    /// The cookbook high-pass.
    pub fn high_pass(hz: f32, q: f32, rate: f32) -> Biquad {
        let (c, s) = Self::w(hz, rate);
        let al = s / (2.0 * f64::from(q));
        Biquad::from(
            [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
            [1.0 + al, -2.0 * c, 1.0 - al],
        )
    }

    /// The cookbook peak, `db` at `hz`.
    pub fn peak(hz: f32, q: f32, db: f32, rate: f32) -> Biquad {
        let (c, s) = Self::w(hz, rate);
        let a = 10f64.powf(f64::from(db) / 40.0);
        let al = s / (2.0 * f64::from(q));
        Biquad::from(
            [1.0 + al * a, -2.0 * c, 1.0 - al * a],
            [1.0 + al / a, -2.0 * c, 1.0 - al / a],
        )
    }

    /// A band-pass with 0 dB at its peak.
    pub fn band_pass(hz: f32, q: f32, rate: f32) -> Biquad {
        let (c, s) = Self::w(hz, rate);
        let al = s / (2.0 * f64::from(q));
        Biquad::from([al, 0.0, -al], [1.0 + al, -2.0 * c, 1.0 - al])
    }

    /// Takes `other`'s coefficients and keeps this one's state.
    fn take(&mut self, other: Biquad) {
        let (z1, z2) = (self.z1, self.z2);
        *self = other;
        self.z1 = z1;
        self.z2 = z2;
    }

    fn clear(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Its gain at `hz` (for tests).
    pub fn gain_at(&self, hz: f32, rate: f32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * f64::from(hz) / f64::from(rate);
        let z1 = (w.cos(), -w.sin());
        let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num = (
            f64::from(self.b0) + f64::from(self.b1) * z1.0 + f64::from(self.b2) * z2.0,
            f64::from(self.b1) * z1.1 + f64::from(self.b2) * z2.1,
        );
        let den = (
            1.0 + f64::from(self.a1) * z1.0 + f64::from(self.a2) * z2.0,
            f64::from(self.a1) * z1.1 + f64::from(self.a2) * z2.1,
        );
        ((num.0 * num.0 + num.1 * num.1) / (den.0 * den.0 + den.1 * den.1)).sqrt() as f32
    }
}

/// A key: its swell, its velocity's gain and the bank note it plays.
#[derive(Clone, Copy, Debug, Default)]
struct Key {
    env: Adsr,
    gain: f32,
    /// The share of its level in the bright buses.
    bright: f32,
    pitch: i32,
    /// Down, or held by a pedal: not let go yet.
    gated: bool,
    /// Shared swell: let go as the last key, so it fades with the swell.
    lingering: bool,
    /// In the list of keys sounding.
    live: bool,
}

/// One register's filters and level.
#[derive(Clone, Copy, Debug)]
struct Section {
    high: Biquad,
    body: Biquad,
    low: Biquad,
    dark: Biquad,
    gain: f32,
    /// Gain step per frame across the control step.
    dgain: f32,
    /// Its level at the step's end.
    target: f32,
}

/// The string machine. See the module docs.
pub struct Strings {
    values: [f32; ID_SPACE],
    rate: u32,
    /// The absolute frame of the next frame to render.
    now: u64,
    /// Frames since the last step with a key or a note heard; past
    /// [`QUIET_MS`] the steps skip the bank and the ensemble.
    quiet: u32,
    tuning: Tuning,
    keys: [Key; NOTES],
    live: [u8; NOTES],
    n_live: usize,
    n_gated: usize,
    pedals: Pedals,
    shared: Adsr,
    /// The bank, its six buses the footages' dark and bright halves.
    bank: Bank<BUSES>,
    /// The bend applied to the bank and the one asked for, semitones.
    bend: f32,
    bend_to: f32,
    sections: [Section; 6],
    /// Brightness (octaves) and Body (a factor) as the filters have them.
    bright: f32,
    body: f32,
    rng: Rng,
    noise_band: Biquad,
    noise: f32,
    dnoise: f32,
    noise_to: f32,
    level: Smooth,
    soft: bool,
    expression: f32,
    wheel: f32,
    ensemble: Ensemble,
}

impl Strings {
    pub fn new(rate: u32) -> Strings {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let section = Section {
            high: Biquad::default(),
            body: Biquad::default(),
            low: Biquad::default(),
            dark: Biquad::default(),
            gain: 0.0,
            dgain: 0.0,
            target: 0.0,
        };
        let mut s = Strings {
            values,
            rate: 0,
            now: 0,
            quiet: u32::MAX,
            tuning: Tuning::standard(),
            keys: [Key::default(); NOTES],
            live: [0; NOTES],
            n_live: 0,
            n_gated: 0,
            pedals: Pedals::new(),
            shared: Adsr::default(),
            bank: Bank::new(),
            bend: 0.0,
            bend_to: 0.0,
            sections: [section; 6],
            bright: 0.0,
            body: 1.0,
            rng: Rng::new(0x5701),
            noise_band: Biquad::default(),
            noise: 0.0,
            dnoise: 0.0,
            noise_to: 0.0,
            level: Smooth::new(1.0),
            soft: false,
            expression: 1.0,
            wheel: 0.0,
            ensemble: Ensemble::new(),
        };
        s.prepare(rate, mpfree_node::MAX_BLOCK);
        s
    }

    pub fn presets() -> &'static [Preset] {
        crate::factory::presets!("strings.presets", &INFO, PARAMS)
    }

    /// Sets every parameter to its default, then the preset's values.
    pub fn load_preset(&mut self, preset: &Preset) {
        for p in PARAMS.iter().filter(|p| !p.readonly) {
            self.set_param(p.id, p.default);
        }
        for &(id, v) in preset.values {
            self.set_param(id, v);
        }
    }

    /// Keys down or held by a pedal.
    pub fn keys_down(&self) -> usize {
        self.n_gated
    }

    /// Keys sounding, fading out included.
    pub fn keys_sounding(&self) -> usize {
        self.n_live
    }

    /// Oscillators of the bank heard now.
    pub fn oscillators_heard(&self) -> usize {
        self.bank.heard()
    }

    /// Whether note `n`'s phase is locked to the note an octave below.
    pub fn locked(&self, n: usize) -> bool {
        self.bank.locked(n)
    }

    /// Note `n`'s frequency in the bank, Hz (0 when silent).
    pub fn bank_hz(&self, n: usize) -> f64 {
        self.bank.hz(n)
    }

    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn steps_per_sec(&self) -> f32 {
        self.rate as f32 / CTRL as f32
    }

    fn shared_mode(&self) -> bool {
        self.v(SWELL) >= 0.5
    }

    /// The swells' times, for every key and the shared swell.
    fn update_swell(&mut self) {
        let sps = self.steps_per_sec();
        let (cresc, sus) = (self.v(CRESCENDO), self.v(SUSTAIN));
        self.shared.set(cresc, 0.0, 1.0, sus, sps);
        let shared = self.shared_mode();
        for k in self.keys.iter_mut() {
            if shared {
                k.env.set(CONTACT_ON_MS, 0.0, 1.0, CONTACT_OFF_MS, sps);
            } else {
                k.env.set(cresc, 0.0, 1.0, sus, sps);
            }
        }
    }

    /// Swell switched between per key and shared while keys sound: no
    /// jump in level either way.
    fn switch_swell(&mut self, shared: bool) {
        if shared {
            // Every key held keeps its level; the shared swell takes over
            // from full.
            for k in self.keys.iter_mut().filter(|k| k.live) {
                if k.gated {
                    k.env.level = 1.0;
                    k.env.stage = Stage::Decay;
                }
            }
            if self.n_gated > 0 {
                self.shared.level = 1.0;
                self.shared.stage = Stage::Decay;
            } else {
                self.shared.clear();
            }
        } else {
            let level = self.shared.level;
            for k in self.keys.iter_mut().filter(|k| k.live) {
                k.env.level *= level;
                if !k.gated {
                    k.lingering = false;
                    k.env.gate_off();
                }
            }
            self.shared.clear();
        }
        self.update_swell();
    }

    fn update_ensemble(&mut self) {
        for (id, value) in ensemble_settings(|id| self.v(id), self.wheel) {
            self.ensemble.set_param(id, value);
        }
    }

    fn update_level(&mut self) {
        let soft = if self.soft { SOFT_GAIN } else { 1.0 };
        let db = self.v(LEVEL);
        self.level.target = 10f32.powf(db / 20.0) * soft * self.expression * OUT_GAIN;
    }

    /// Every note's step in the bank from the tuning, Fine and the bend,
    /// each oscillator going on from where it is now (`frame`). A note
    /// whose pitch is twice the one an octave below locks to it.
    fn retune(&mut self, frame: u64) {
        let shift = self.v(FINE) / 100.0 + self.bend;
        self.bank.retune(&self.tuning, shift, self.rate, frame);
    }

    /// Fresh random phases, one per pitch class: the notes above follow
    /// from them when the octaves lock.
    fn new_phases(&mut self) {
        self.bank.new_phases(0x51A7);
    }

    fn update_filters(&mut self, frame_rate: f32) {
        let soft = if self.soft { SOFT_OCTAVES } else { 0.0 };
        let octaves = (self.v(BRIGHTNESS) - 50.0) / 50.0 + soft;
        let body = self.v(BODY) / 50.0;
        // Move the filters at most a twelfth of an octave and a tenth of
        // the body a step, so a turned knob glides.
        let step = |from: f32, to: f32, max: f32| from + (to - from).clamp(-max, max);
        let bright = step(self.bright, octaves, 1.0 / 12.0);
        let body = step(self.body, body, 0.1);
        if bright == self.bright && body == self.body && self.sections[0].low.b0 != 0.0 {
            return;
        }
        self.bright = bright;
        self.body = body;
        let mult = bright.exp2();
        for (s, v) in self.sections.iter_mut().zip(VOICING.iter()) {
            s.high
                .take(Biquad::high_pass(v.high_pass_hz, 0.707, frame_rate));
            s.body.take(Biquad::peak(
                v.body_hz,
                v.body_q,
                v.body_db * body,
                frame_rate,
            ));
            let top = (v.low_pass_hz * mult).min(LOW_PASS_TOP);
            s.low.take(Biquad::low_pass(top, v.low_pass_q, frame_rate));
            let dark = top * DARK_OCTAVES.exp2();
            s.dark
                .take(Biquad::low_pass(dark, v.low_pass_q, frame_rate));
        }
    }

    fn add_live(&mut self, note: usize) {
        if !self.keys[note].live {
            self.keys[note].live = true;
            self.live[self.n_live] = note as u8;
            self.n_live += 1;
        }
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        let n = usize::from(note & 0x7F);
        self.pedals.press(note);
        let shared = self.shared_mode();
        if shared && self.n_gated == 0 {
            // A new swell: the keys still fading stop.
            for i in 0..self.n_live {
                let k = &mut self.keys[usize::from(self.live[i])];
                if k.lingering {
                    k.lingering = false;
                    k.env.gate_off();
                }
            }
            self.shared.gate_on();
        }
        let shift = self.v(TRANSPOSE) as i32 + 12 * self.v(OCTAVE) as i32;
        let range = self.v(VELOCITY);
        let k = &mut self.keys[n];
        k.gain = velocity_gain(velocity, range) * KEY_FOLLOW.gain(n as i32 + shift);
        k.bright = if range > 0.0 {
            f32::from(velocity.min(127)) / 127.0
        } else {
            1.0
        };
        k.pitch = n as i32 + shift;
        k.lingering = false;
        k.env.gate_on();
        if !k.gated {
            k.gated = true;
            self.n_gated += 1;
        }
        self.add_live(n);
    }

    /// The note ends: its key is up and no pedal holds it.
    fn release(&mut self, note: u8) {
        let n = usize::from(note & 0x7F);
        if !self.keys[n].gated {
            return;
        }
        self.keys[n].gated = false;
        self.n_gated -= 1;
        if self.shared_mode() && self.n_gated == 0 {
            self.keys[n].lingering = true;
            self.shared.gate_off();
        } else {
            self.keys[n].env.gate_off();
        }
    }

    fn release_all(&mut self) {
        for n in 0..NOTES {
            if self.keys[n].gated {
                self.release(n as u8);
            }
        }
        self.pedals.clear_notes();
    }

    fn control_event(&mut self, ctl: Ctl, value: f32) {
        let Some(value) = Ctl::clamp(value) else {
            return;
        };
        match ctl {
            Ctl::Sustain | Ctl::Sostenuto => {
                let mut ends = [0u8; NOTES];
                let mut n = 0;
                self.pedals.set(ctl, value, |note| {
                    ends[n] = note;
                    n += 1;
                });
                for &note in &ends[..n] {
                    self.release(note);
                }
            }
            Ctl::Soft => {
                self.soft = Ctl::down(value);
                self.update_level();
            }
            Ctl::ModWheel => {
                self.wheel = value;
                self.update_ensemble();
            }
            Ctl::Breath | Ctl::Foot => {
                self.expression = value * value;
                self.update_level();
            }
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => self.note_on(note, velocity),
            NodeEvent::NoteOn { note, .. } | NodeEvent::NoteOff { note } => {
                if self.pedals.lift(note) {
                    self.release(note);
                }
            }
            NodeEvent::AllOff => self.release_all(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Bend { value } => {
                if value.is_finite() {
                    self.bend_to = value.clamp(-1.0, 1.0) * self.v(BEND_RANGE);
                }
            }
            NodeEvent::Control { ctl, value } => self.control_event(ctl, value),
            NodeEvent::Pressure { .. } | NodeEvent::Expression { .. } => {}
        }
    }

    /// One control step at absolute frame `frame`: the swells move, each
    /// oscillator's level for the step is set, the filters glide.
    fn control(&mut self, frame: u64) {
        if self.bend_to != self.bend {
            self.bend = self.bend_to;
            self.retune(frame);
        }
        let shared_mode = self.shared_mode();
        let shared = if shared_mode { self.shared.next() } else { 1.0 };
        let shared_idle = shared_mode && self.shared.idle();
        // The footages any register uses.
        let mut used = [false; 3];
        for (i, (id, _)) in REGISTERS.iter().enumerate() {
            if self.v(*id) > 0.0 || self.sections[i].gain > 0.0 {
                for f in 0..3 {
                    used[f] |= VOICING[i].feet[f] > 0.0;
                }
            }
        }
        // Last step's targets are where the levels are now.
        self.bank.begin_step();
        // Each key's level onto its notes.
        let mut power = 0.0f32;
        let mut i = 0;
        while i < self.n_live {
            let n = usize::from(self.live[i]);
            let k = &mut self.keys[n];
            if k.lingering && shared_idle {
                k.lingering = false;
                k.env.clear();
            }
            let level = k.env.next();
            if k.env.idle() && !k.gated && !k.lingering {
                k.live = false;
                self.n_live -= 1;
                self.live[i] = self.live[self.n_live];
                continue;
            }
            let g = level * k.gain * shared;
            let (gd, gb) = (g * (1.0 - k.bright), g * k.bright);
            power += g * g;
            for f in 0..3 {
                let mut o = k.pitch + FOOTAGES[f];
                let mut fold = 1.0;
                if f > 0 && o > FOLD_ABOVE {
                    o -= 12;
                    fold = FOLD_GAIN;
                }
                if used[f] && (0..NOTES as i32).contains(&o) && self.bank.sounds(o as usize) {
                    let o = o as usize;
                    self.bank.add(2 * f, o, gd * fold, frame);
                    self.bank.add(2 * f + 1, o, gb * fold, frame);
                }
            }
            i += 1;
        }
        // Ramps for the step; an oscillator silent at both ends is dropped.
        self.bank.end_step();
        // The registers' levels and filters.
        let inv = 1.0 / CTRL as f32;
        let r = self.rate as f32;
        self.update_filters(r);
        let mix: f32 = REGISTERS
            .iter()
            .map(|(id, _)| (self.values[*id as usize] / 100.0).powi(2))
            .sum();
        let norm = 1.0 / mix.sqrt().max(1.0);
        for (i, (id, _)) in REGISTERS.iter().enumerate() {
            let s = &mut self.sections[i];
            let to = self.values[*id as usize] / 100.0 * VOICING[i].trim * norm;
            if s.gain == 0.0 && s.target == 0.0 && to > 0.0 {
                // Coming in from silence: no stale state.
                s.high.clear();
                s.body.clear();
                s.low.clear();
                s.dark.clear();
            }
            s.gain = s.target;
            s.target = to;
            s.dgain = (to - s.gain) * inv;
        }
        // Bow noise follows the keys' power.
        self.noise = self.noise_to;
        self.noise_to = self.v(NOISE) / 100.0 * NOISE_GAIN * power.sqrt();
        self.dnoise = (self.noise_to - self.noise) * inv;
    }

    /// Renders the first `n` frames of `mono` (one control step at most).
    fn render(&mut self, n: usize, mono: &mut [f32; STEP]) {
        let mut bus = [[0.0f32; STEP]; BUSES];
        self.bank.render(n, &mut bus, &[Shape::Saw; BUSES]);
        let mut out = [0.0f32; STEP];
        for (s, v) in self.sections.iter_mut().zip(VOICING.iter()) {
            if s.gain == 0.0 && s.dgain == 0.0 {
                continue;
            }
            let [f0, f1, f2] = v.feet;
            for j in 0..n {
                let dark = f0 * bus[0][j] + f1 * bus[2][j] + f2 * bus[4][j];
                let bright = f0 * bus[1][j] + f1 * bus[3][j] + f2 * bus[5][j];
                let x = s.dark.run(dark) + s.low.run(bright);
                let y = s.body.run(s.high.run(x));
                out[j] += y * s.gain;
                s.gain += s.dgain;
            }
        }
        if self.noise != 0.0 || self.dnoise != 0.0 {
            for o in out.iter_mut().take(n) {
                *o += self.noise_band.run(self.rng.bipolar()) * self.noise;
                self.noise += self.dnoise;
            }
        }
        for (o, x) in mono.iter_mut().zip(&out).take(n) {
            *o = x * self.level.next();
        }
    }

    /// Adds frames `at..next` into `left` and `right`: a control step at a
    /// time, rendered, through the ensemble, then added.
    fn segment(&mut self, frame: u64, at: usize, next: usize, left: &mut [f32], right: &mut [f32]) {
        let mut i = at;
        while i < next {
            let abs = frame + i as u64;
            if abs.is_multiple_of(CTRL) {
                self.control(abs);
            }
            let end = next.min(i + (CTRL - abs % CTRL) as usize);
            let n = end - i;
            if self.n_live == 0 && self.bank.heard() == 0 && self.noise == 0.0 && self.dnoise == 0.0
            {
                if self.quiet as f32 >= QUIET_MS * 0.001 * self.rate as f32 {
                    i = end;
                    continue;
                }
                self.quiet = self.quiet.saturating_add(n as u32);
            } else {
                self.quiet = 0;
            }
            let mut l = [0.0f32; STEP];
            let mut r = [0.0f32; STEP];
            self.render(n, &mut l);
            self.ensemble.process_mono(&mut l[..n], &mut r[..n]);
            for (o, x) in left[i..end].iter_mut().zip(&l) {
                *o += x;
            }
            for (o, x) in right[i..end].iter_mut().zip(&r) {
                *o += x;
            }
            i = end;
        }
    }
}

impl Node for Strings {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.iter().find(|p| p.id == id) else {
            return;
        };
        if p.readonly {
            return;
        }
        let was_shared = self.shared_mode();
        self.values[id as usize] = p.clamp(value);
        match id {
            LEVEL => self.update_level(),
            CRESCENDO | SUSTAIN => self.update_swell(),
            SWELL => {
                let shared = self.shared_mode();
                if shared != was_shared {
                    self.switch_swell(shared);
                }
            }
            ENSEMBLE | LINES | SLOW_RATE | FAST_RATE | DEPTH | WIDTH => self.update_ensemble(),
            FINE => self.retune(self.now),
            _ => {}
        }
    }

    fn param(&self, id: u16) -> f32 {
        if id == METER_KEYS {
            return self.keys_down() as f32;
        }
        match PARAMS.iter().find(|p| p.id == id) {
            Some(_) => self.values[id as usize],
            None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        self.rate = rate;
        self.ensemble.prepare(rate, max_block.max(1));
        self.level.set_time(rate, 10.0);
        self.noise_band = Biquad::band_pass(NOISE_HZ, 0.8, rate as f32);
        self.reset();
    }

    fn reset(&mut self) {
        self.keys = [Key::default(); NOTES];
        self.n_live = 0;
        self.n_gated = 0;
        self.pedals.reset();
        self.shared.clear();
        self.bank.clear();
        self.bend = 0.0;
        self.bend_to = 0.0;
        self.soft = false;
        self.expression = 1.0;
        self.wheel = 0.0;
        self.noise = 0.0;
        self.dnoise = 0.0;
        self.noise_to = 0.0;
        self.noise_band.clear();
        self.rng = Rng::new(0x5701);
        self.now = 0;
        self.quiet = u32::MAX;
        self.new_phases();
        self.retune(0);
        for s in self.sections.iter_mut() {
            s.high.clear();
            s.body.clear();
            s.low.clear();
            s.dark.clear();
            s.gain = 0.0;
            s.dgain = 0.0;
            s.target = 0.0;
        }
        self.bright = (self.v(BRIGHTNESS) - 50.0) / 50.0;
        self.body = self.v(BODY) / 50.0;
        // Force the coefficients.
        self.sections[0].low.b0 = 0.0;
        self.update_filters(self.rate as f32);
        self.update_swell();
        self.update_level();
        self.level.snap(self.level.target);
        self.update_ensemble();
        self.ensemble.reset();
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        if ctx.rate != self.rate {
            self.prepare(ctx.rate, left.len());
        }
        let len = left.len().min(right.len());
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            self.now = ctx.frame + at as u64;
            while i < events.len() && events[i].offset as usize <= at {
                let ev = events[i].ev;
                self.apply(&ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            if next > at {
                self.segment(ctx.frame, at, next, left, right);
            }
            at = next;
        }
        self.now = ctx.frame + len as u64;
        for e in &events[i..] {
            let ev = e.ev;
            self.apply(&ev);
        }
    }

    fn tail_frames(&self) -> u32 {
        // The longest fade reaches silence at about 1.35 times its time;
        // then the ensemble's lines.
        (self.rate as f32 * (self.v(SUSTAIN) * 0.0014 + 0.1)) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
        self.retune(self.now);
    }

    fn hears(&self, ctl: Ctl) -> bool {
        matches!(ctl, Ctl::Sustain | Ctl::Sostenuto)
    }

    fn sounding_voices(&self) -> usize {
        self.n_live
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn run(s: &mut Strings, frames: usize, events: &[(usize, NodeEvent)]) -> (Vec<f32>, Vec<f32>) {
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        let block = 128;
        let mut at = 0;
        while at < frames {
            let end = (at + block).min(frames);
            let ev: Vec<Timed> = events
                .iter()
                .filter(|(f, _)| *f >= at && *f < end)
                .map(|&(f, e)| Timed::new((f - at) as u32, e))
                .collect();
            let ctx = Ctx::new(RATE, s.now);
            s.process(&ctx, &ev, &mut l[at..end], &mut r[at..end]);
            at = end;
        }
        (l, r)
    }

    fn on(note: u8) -> NodeEvent {
        NodeEvent::NoteOn {
            note,
            velocity: 100,
        }
    }

    #[test]
    fn octaves_lock_in_twelve_tet_and_every_note_is_in_tune() {
        let s = Strings::new(RATE);
        for n in 12..100 {
            assert!(s.locked(n), "note {n}");
        }
        for n in 0..100 {
            let want = 440.0 * ((n as f64 - 69.0) / 12.0).exp2();
            let cents = 1200.0 * (s.bank_hz(n) / want).log2();
            assert!(cents.abs() < 0.01, "note {n}: {cents} cents");
        }
    }

    #[test]
    fn every_built_in_tuning_keeps_every_key_within_a_cent() {
        // S6: locked octaves where the tuning's are pure, each note
        // itself where they are not (24-TET), on every root.
        for b in mpfree_tuning::Builtin::ALL {
            for root in [0u8, 7] {
                let mut s = Strings::new(RATE);
                let t = b.tuning(root, 440.0);
                s.set_tuning(&t);
                for n in 0..NOTES {
                    let want = f64::from(t.hz(n as f32));
                    if s.bank_hz(n) == 0.0 || want * 2.0 >= f64::from(RATE) {
                        continue;
                    }
                    let cents = 1200.0 * (s.bank_hz(n) / want).log2();
                    assert!(
                        cents.abs() < 1.0,
                        "{}, root {root}, note {n}: {cents} cents",
                        b.name()
                    );
                }
            }
        }
    }

    #[test]
    fn a_tuning_without_pure_octaves_tunes_each_note_itself() {
        let mut s = Strings::new(RATE);
        let t = Tuning::equal(24, 440.0);
        s.set_tuning(&t);
        for n in 12..90 {
            assert!(!s.locked(n), "note {n}");
            let want = f64::from(t.hz(n as f32));
            let cents = 1200.0 * (s.bank_hz(n) / want).log2();
            assert!(cents.abs() < 0.01, "note {n}: {cents} cents");
        }
    }

    #[test]
    fn the_shared_swell_fades_the_last_chord_and_cuts_keys_let_go_early() {
        let mut s = Strings::new(RATE);
        s.set_param(SWELL, SHARED);
        s.set_param(CRESCENDO, 10.0);
        s.set_param(SUSTAIN, 1_000.0);
        let off = |n| NodeEvent::NoteOff { note: n };
        run(&mut s, 4_800, &[(0, on(60)), (0, on(64))]);
        assert_eq!(s.keys_sounding(), 2);
        // 64 let go with 60 still down: it stops within a few steps.
        run(&mut s, 4_800, &[(0, off(64))]);
        assert_eq!(s.keys_sounding(), 1);
        // The last key let go fades with the swell.
        run(&mut s, 4_800, &[(0, off(60))]);
        assert_eq!(s.keys_sounding(), 1);
        run(&mut s, 2 * RATE as usize, &[]);
        assert_eq!(s.keys_sounding(), 0);
    }

    #[test]
    fn the_sustain_pedal_holds_notes_here() {
        let mut s = Strings::new(RATE);
        s.set_param(SUSTAIN, 100.0);
        assert!(s.hears(Ctl::Sustain) && s.hears(Ctl::Sostenuto));
        let pedal = |v| NodeEvent::Control {
            ctl: Ctl::Sustain,
            value: v,
        };
        run(
            &mut s,
            9_600,
            &[
                (0, pedal(1.0)),
                (0, on(60)),
                (100, NodeEvent::NoteOff { note: 60 }),
            ],
        );
        assert_eq!(s.keys_down(), 1);
        run(&mut s, 960, &[(0, pedal(0.0))]);
        assert_eq!(s.keys_down(), 0);
        run(&mut s, RATE as usize, &[]);
        assert_eq!(s.keys_sounding(), 0);
    }

    #[test]
    fn the_registers_filters_sit_where_they_are_voiced() {
        let r = RATE as f32;
        for v in VOICING {
            let lp = Biquad::low_pass(v.low_pass_hz, 0.707, r);
            assert!((lp.gain_at(v.low_pass_hz, r) - 0.707).abs() < 0.01);
            let pk = Biquad::peak(v.body_hz, v.body_q, v.body_db, r);
            let db = 20.0 * pk.gain_at(v.body_hz, r).log10();
            assert!((db - v.body_db).abs() < 0.05, "{db}");
        }
    }

    #[test]
    fn silence_until_a_key_and_bounded_after() {
        let mut s = Strings::new(RATE);
        let (l, _) = run(&mut s, 4_800, &[]);
        assert!(l.iter().all(|x| *x == 0.0));
        let (l, r) = run(
            &mut s,
            RATE as usize,
            &[(0, on(48)), (0, on(55)), (0, on(64))],
        );
        let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.01 && peak < 1.0, "{peak}");
    }
}
