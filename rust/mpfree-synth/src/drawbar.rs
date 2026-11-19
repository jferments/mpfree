//! Drawbar: an organ and its rotary speaker, in two models. **Tonewheel**
//! is the electromechanical organ, built from the sources in
//! `docs/dev/research/tonewheel-organ.md`; **Combo** is the transistor
//! combo organ of the 1960s, from `docs/dev/research/organ-wind-choir.md`.
//! Everything below is the tonewheel's unless it says Combo; the keys, the
//! drawbars, the click, the percussion, drive, tone and the speaker serve
//! both.
//!
//! - **The wheels.** 91 wheels turn all the time, tuned from the organ's
//!   twelve gear ratios ([`gear_hz`]), whose small offsets from equal
//!   temperament ride on the project's tuning (**Wheels** Exact drops
//!   them). A key never starts an
//!   oscillator: it sets how much of each running wheel is heard, so no
//!   two notes start in phase and a wheel two keys share (one key's 4', the
//!   next octave's 8') sums once instead of beating. The lowest octave of
//!   wheels is nearly square (1×, 3× and 5×).
//! - **Drawbars.** Nine, at -12, +7, 0, +12, +19, +24, +28, +31 and +36
//!   semitones, 0 to 8 at 3 dB a step. The 16' folds back up in the
//!   lowest manual octave and anything past the top wheel folds down an
//!   octave ([`wheel_for`]).
//! - **Click.** Each drawbar has its own key contact. They close at
//!   slightly different moments and may bounce, switching running wheels
//!   in well under a millisecond ([`Contact`]); **Click** sets how uneven
//!   and how quick. Letting go opens them the same way, more gently.
//! - **Leakage.** Each wheel is heard faintly in its compartment partner,
//!   48 wheels (four octaves) away.
//! - **Percussion.** A decaying second or third harmonic, single trigger
//!   (only when a key goes down with none held) but on every key of a
//!   chord struck together. It takes the 1' drawbar's contact, so the 1'
//!   is silent while it is on, and Normal lowers the drawbars by 3 dB.
//! - **Vibrato and chorus.** The scanner sweeps back and forth across the
//!   taps of a 19-tap delay line, crossfading between neighbouring taps
//!   as its plates do; chorus mixes the dry sound back in.
//! - **Drive** (an asymmetric valve-like curve), **Tone** (a tilt), then
//!   the **rotary speaker**: an 800 Hz split into a horn and a drum, each
//!   a rotating source heard by two microphones through a modulated delay
//!   (Doppler), its level (and the horn's treble) following where it
//!   faces, one cabinet reflection, the horn's unmodulated leakage, and
//!   each rotor's own speeds and inertia (**Speed** Stop, Slow, Fast).
//! - **Combo.** Every note divided down from twelve top-octave
//!   oscillators: the shared bank ([`Bank`]) that Strings plays, octaves
//!   locked, each note a sawtooth as each divider's own filter makes it.
//!   Each drawbar takes its note from the bank (anything above B7 folds
//!   down by octaves) and sends it to two voices, as the combo's own two
//!   drawbars do: **Flute**, the saw integrated twice into a near sine of
//!   the same shape at every pitch (the designer's aim, "a good sine shape
//!   across the whole keyboard"), and **Reed**, the saw (or the dividers'
//!   square, **Reed wave**) through a fixed voicing filter that thins and
//!   brightens it. All three shapes start in step with the saw, so a
//!   note's flute and reed add. Each voice follows the keyboard a little
//!   (`combo_follow`) so one note sounds about as loud anywhere. The
//!   voices' filters run on two buses, so their cost does not grow with
//!   keys. Its vibrato is a smooth one, the whole bank
//!   swinging in pitch as when the master oscillators are swept: **Vibrato**
//!   V1 to V3 swing it 10, 20 or 30 cents either way at **Scanner** Hz, C1
//!   to C3 mix the dry sound back. A change of model crosses over in 10 ms.
//!
//! Velocity, pressure, bend and slides are ignored, as on the organ.
//! Contacts, percussion, the rotors' speeds and retuning run at a control
//! rate of one step every [`CTRL`] frames aligned to the absolute frame,
//! with each wheel's level moving smoothly across the step, so the output
//! does not depend on how the host cuts blocks. Nothing allocates, not
//! even `prepare`.
//!
//! Cost: a wheel that sounds is a phasor turned by a fixed rotation each
//! frame (four multiplies), started from the wheel's own phase when it
//! comes in and pulled back to unit length each step; a silent wheel only
//! counts its phase. Each key keeps the wheels its drawbars reach, and a
//! settled key (every contact closed) skips its contacts.

use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Tuning};

use crate::bank::{Bank, Shape};
use crate::drawbar_params::*;
use crate::dsp::Rng;
use crate::params::Preset;

/// Frames per control step: the bank's.
pub const CTRL: u64 = crate::bank::STEP as u64;
/// Tonewheels.
pub const N_WHEELS: usize = 91;
/// Wheel 1's note (C, 32.7 Hz): wheel `w` (from 0) is MIDI note 24 + `w`.
pub const FIRST_NOTE: i32 = 24;
/// The lowest manual key (C, 65.4 Hz): the 16' folds back above it.
pub const MANUAL_LOW: i32 = 36;
/// The drive shaft's turns per second (1200 rpm).
const MOTOR_HZ: f64 = 20.0;
/// Driving and driven gear teeth for C, C#, ... B.
pub const GEARS: [(u32, u32); 12] = [
    (85, 104),
    (71, 82),
    (67, 73),
    (105, 108),
    (103, 100),
    (84, 77),
    (74, 64),
    (98, 80),
    (96, 74),
    (88, 64),
    (67, 46),
    (108, 70),
];
/// Keys sounding or letting go at once.
pub const MAX_KEYS: usize = 64;
/// The sine table's size (a power of two), read with linear interpolation.
const TABLE_BITS: u32 = 11;
const TABLE: usize = 1 << TABLE_BITS;
/// The wheels' sum into the output: one key at the defaults peaks near
/// -15 dBFS, so a six-note chord of three drawbars stays under -1 dBFS.
const OUT_GAIN: f32 = 0.056;
/// Percussion's level against a drawbar at 8, normal and soft (-6 dB).
const PERC_NORMAL: f32 = 1.0;
const PERC_SOFT: f32 = 0.5012;
/// Percussion's time to fall 60 dB, fast and slow.
const PERC_FAST_S: f32 = 1.0;
const PERC_SLOW_S: f32 = 4.0;
/// Normal percussion lowers the drawbars this much.
const PERC_NORMAL_DROP: f32 = 0.7079;
/// Leakage at 100 %: -26 dB.
const LEAK_MAX: f32 = 0.05;
/// One section of the scanner's delay line, in seconds.
const TAP_S: f32 = 44.7e-6;
/// The taps (1 to 19) each scanner terminal reaches, for V1/C1, V2/C2, V3/C3.
const SCAN_TAPS: [[u8; 9]; 3] = [
    [1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 2, 3, 5, 7, 9, 11, 12, 13],
    [1, 2, 4, 7, 10, 13, 16, 18, 19],
];
const VIB_LEN: usize = 256;
const ROT_LEN: usize = 1024;
/// The rotary speaker's split.
const CROSSOVER_HZ: f32 = 800.0;
/// Rotor speeds in turns per second, slow and fast, and the time
/// constants of speeding up and slowing down, in seconds.
const HORN_SLOW: f32 = 40.32 / 60.0;
const HORN_FAST: f32 = 423.36 / 60.0;
const HORN_UP_S: f32 = 0.161;
const HORN_DOWN_S: f32 = 0.321;
const DRUM_SLOW: f32 = 36.0 / 60.0;
const DRUM_FAST: f32 = 357.3 / 60.0;
const DRUM_UP_S: f32 = 4.127;
const DRUM_DOWN_S: f32 = 1.371;
/// Rotor radii in metres, and the speed of sound.
const HORN_R: f32 = 0.192;
const DRUM_R: f32 = 0.22;
const SOUND_MS: f32 = 343.0;
/// How much quieter each rotor is facing away from a microphone.
const HORN_AM: f32 = 0.35;
const DRUM_AM: f32 = 0.3;
/// The horn's treble facing a microphone and facing away (a horn driver
/// gives little above 8 kHz or so, and less off its axis).
const HORN_BRIGHT_HZ: f32 = 9000.0;
const HORN_DARK_HZ: f32 = 3000.0;
/// The horn's unmodulated share, and its cabinet reflection's level and delay.
const HORN_LEAK: f32 = 0.15;
const REFLECT: f32 = 0.3;
const REFLECT_S: f32 = 0.0011;

/// The combo's top note for a footage (B7): a footage above it folds back
/// down by octaves, as the top-octave dividers end there (as in Strings).
pub const COMBO_TOP: i32 = 107;
/// The combo's buses in the bank: the flute and, the bus after it, the
/// reed (a key feeds them as a pair).
const FLUTE_BUS: usize = 0;
const REED_BUS: usize = 1;
/// Control steps a change of model takes to cross over (10 ms at 48 kHz).
const MODEL_STEPS: f32 = 30.0;
/// The combo's voicing, our reading of "sine-like" and "bright, sawtooth-
/// like" (no circuit values are published): the flute through a gentle
/// top cut (it also keeps the near sine's highest harmonics from folding
/// back), the reed thinned below, given a nasal peak and cut above as a
/// small amplifier does.
const FLUTE_TOP_HZ: f32 = 9_000.0;
const REED_LOW_HZ: f32 = 200.0;
const REED_PEAK_HZ: f32 = 1_600.0;
const REED_PEAK_Q: f32 = 0.9;
const REED_PEAK_DB: f32 = 5.0;
const REED_TOP_HZ: f32 = 6_500.0;
const REED_TOP_Q: f32 = 0.8;
/// Each voice's level for a drawbar at 8, into the mono sum.
const COMBO_FLUTE: f32 = 0.063;
const COMBO_REED: f32 = 0.063;
/// The square reed against the saw: a square of the same swing carries
/// about 4 dB more, so it is trimmed to sit level with the saw at C4.
const SQUARE_TRIM: f32 = 0.63;
/// The combo's keyboard level scaling, dB a semitone from C4 (MIDI 60):
/// the flute lifted below C4 and eased above C5, the reed (thinned below
/// and lifted by its peak above) tilted the other way, so one note sounds
/// about as loud from C2 to C6. Held beyond `FOLLOW_LO` and `FOLLOW_HI`.
const FLUTE_LIFT_DB: f32 = 0.1;
const FLUTE_EASE_DB: f32 = 0.1;
const REED_TILT_DB: f32 = 0.2;
const FOLLOW_LO: f32 = 24.0;
const FOLLOW_HI: f32 = 96.0;
/// The combo vibrato's swing for V1, V2 and V3 (and C1 to C3), cents
/// either way (our estimate: the originals' depth trims are not
/// documented).
pub const COMBO_VIB_CENTS: [f32; 3] = [10.0, 20.0, 30.0];
/// The combo vibrato's delay line: it holds the widest swing at the
/// slowest rate (30 cents at the Scanner's 4 Hz, 134 frames either way)
/// up to 192 kHz.
const CVIB_LEN: usize = 512;
/// The combo vibrato's shortest delay, frames: the four-point read needs
/// two written frames ahead of it.
const CVIB_MIN: f32 = 2.0;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.drawbar",
    name: "Drawbar",
    kind: Kind::Instrument,
};

fn sine_table() -> &'static [f32; TABLE + 1] {
    static T: OnceLock<[f32; TABLE + 1]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0.0; TABLE + 1];
        for (i, v) in t.iter_mut().enumerate() {
            *v = (i as f64 / TABLE as f64 * std::f64::consts::TAU).sin() as f32;
        }
        t
    })
}

#[inline]
fn sine(t: &[f32; TABLE + 1], phase: u32) -> f32 {
    let i = (phase >> (32 - TABLE_BITS)) as usize;
    let f = ((phase >> (16 - TABLE_BITS)) & 0xFFFF) as f32 * (1.0 / 65536.0);
    let a = t[i];
    a + (t[i + 1] - a) * f
}

/// Wheel `w`'s pitch (from 0) as the organ's gears make it: the motor's
/// turns × the note's gear ratio × the wheel's teeth (2 to 128, then 192
/// for the top seven wheels, which take the gears of the note a fourth up).
pub fn gear_hz(w: usize) -> f64 {
    let note = w % 12;
    let (gear, teeth) = if w >= 84 {
        (GEARS[(note + 5) % 12], 192.0)
    } else {
        (GEARS[note], f64::from(2u32 << (w / 12)))
    };
    MOTOR_HZ * f64::from(gear.0) / f64::from(gear.1) * teeth
}

/// The wheel (from 0) drawbar `bar` sounds for `note`, with the organ's
/// foldback: the 16' in the lowest manual octave folds up an octave, and
/// anything below the first wheel or above the last folds by octaves.
pub fn wheel_for(note: i32, bar: usize) -> usize {
    let mut m = note + DRAWBARS[bar].2;
    if bar == 0 && note >= MANUAL_LOW && m < MANUAL_LOW {
        m += 12;
    }
    while m < FIRST_NOTE {
        m += 12;
    }
    while m > FIRST_NOTE + N_WHEELS as i32 - 1 {
        m -= 12;
    }
    (m - FIRST_NOTE) as usize
}

/// A drawbar position's level: 3 dB a step, silent at 0.
pub fn bar_level(pos: f32) -> f32 {
    if pos < 0.5 {
        0.0
    } else {
        10f32.powf(-3.0 * (8.0 - pos) / 20.0)
    }
}

/// Percussion's level against a drawbar at 8, `t` seconds after it fires,
/// for the values `v` (0 while it is off). For the Synth tab's picture.
pub fn perc_level(v: impl Fn(u16) -> f32, t: f32) -> f32 {
    if v(PERC) < 0.5 {
        return 0.0;
    }
    let level = if v(PERC_VOL) >= 0.5 {
        PERC_SOFT
    } else {
        PERC_NORMAL
    };
    let t60 = if v(PERC_DECAY) >= 0.5 {
        PERC_SLOW_S
    } else {
        PERC_FAST_S
    };
    level * (-6.9078 * t.max(0.0) / t60).exp()
}

/// The fastest a rotor turns, in turns per second (the horn on Fast).
pub const ROTOR_MAX: f32 = HORN_FAST;

/// The horn's and the drum's speeds in turns per second, `t` seconds
/// after **Speed** is switched to `speed` (0 Stop, 1 Slow, 2 Fast): from
/// Slow to Fast, or from Fast to Slow or Stop. For the Synth tab's
/// picture.
pub fn rotor_ramp(speed: f32, t: f32) -> (f32, f32) {
    let ((h0, d0), (h1, d1)) = match speed.round() as i32 {
        0 => ((HORN_FAST, DRUM_FAST), (0.0, 0.0)),
        1 => ((HORN_FAST, DRUM_FAST), (HORN_SLOW, DRUM_SLOW)),
        _ => ((HORN_SLOW, DRUM_SLOW), (HORN_FAST, DRUM_FAST)),
    };
    let up = h1 > h0;
    let go = |from: f32, to: f32, tau: f32| to + (from - to) * (-t.max(0.0) / tau).exp();
    (
        go(h0, h1, if up { HORN_UP_S } else { HORN_DOWN_S }),
        go(d0, d1, if up { DRUM_UP_S } else { DRUM_DOWN_S }),
    )
}

/// A contact that never bounces.
const NEVER: u16 = u16::MAX;

/// One key contact on its way to `to`: it waits `wait` control steps where
/// it is, then moves there over `left` steps; a closing contact may bounce
/// open for one step, `bounce` steps from now.
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    value: f32,
    to: f32,
    step: f32,
    wait: u16,
    left: u16,
    bounce: u16,
}

impl Default for Contact {
    fn default() -> Contact {
        Contact {
            value: 0.0,
            to: 0.0,
            step: 0.0,
            wait: 0,
            left: 0,
            bounce: NEVER,
        }
    }
}

impl Contact {
    /// This step's level, then on to the next step.
    #[inline]
    fn tick(&mut self) -> f32 {
        if self.wait > 0 {
            self.wait -= 1;
        } else if self.left > 0 {
            self.left -= 1;
            self.value = if self.left == 0 {
                self.to
            } else {
                self.value + self.step
            };
        }
        let g = if self.bounce == 0 { 0.0 } else { self.value };
        if self.bounce != NEVER {
            self.bounce = self.bounce.checked_sub(1).unwrap_or(NEVER);
        }
        g
    }

    /// Arrived, and done bouncing.
    fn settled(&self) -> bool {
        self.wait == 0 && self.left == 0 && self.bounce == NEVER
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Key {
    note: u8,
    down: bool,
    /// Every contact has arrived and none will bounce: a key held is fully
    /// on, a key let go is gone.
    settled: bool,
    /// Control steps since it went down or up.
    t: u32,
    /// The wheel each drawbar reaches, for the note as transposed, and the
    /// same as a set (with both percussion harmonics).
    wheels: [u8; 9],
    reach: Bits,
    /// The bank note each drawbar takes on the combo.
    notes: [u8; 9],
    contacts: [Contact; 9],
}

/// The wheels a key's nine drawbars reach, the same as a set, and the
/// bank notes they take on the combo.
fn key_wheels(note: u8, transpose: i32) -> ([u8; 9], Bits, [u8; 9]) {
    let n = (i32::from(note) + transpose).clamp(0, 127);
    let wheels: [u8; 9] = std::array::from_fn(|b| wheel_for(n, b) as u8);
    let mut reach = Bits::default();
    for &w in &wheels {
        reach.set(usize::from(w));
    }
    let notes = std::array::from_fn(|b| combo_note(n, b) as u8);
    (wheels, reach, notes)
}

/// The bank note drawbar `bar` takes for `note` on the combo: its
/// footage, folded down by octaves above [`COMBO_TOP`] and up below the
/// bank.
pub fn combo_note(note: i32, bar: usize) -> usize {
    let mut m = note + DRAWBARS[bar].2;
    while m > COMBO_TOP {
        m -= 12;
    }
    while m < 0 {
        m += 12;
    }
    m as usize
}

/// The combo's wave `t` of the way through one cycle of the 16' (two of
/// the 8') for the values `v`, before the voicing: each drawbar's flute
/// and reed at their levels, the 16''s octaves half a cycle on as its
/// dividers leave them (the other footages run free on the organ; here
/// they start with the 16'). With percussion on, the 1' is silent, as on
/// the tonewheel. For the Synth tab's picture.
pub fn combo_wave(v: impl Fn(u16) -> f32, t: f32) -> f32 {
    let flute = bar_level(v(FLUTE)) * COMBO_FLUTE;
    let (shape, trim) = if v(REED_WAVE) >= 0.5 {
        (Shape::Square, SQUARE_TRIM)
    } else {
        (Shape::Saw, 1.0)
    };
    let reed = bar_level(v(REED)) * COMBO_REED * trim;
    let perc = v(PERC) >= 0.5;
    let mut y = 0.0;
    for (b, &(id, _, semis)) in DRAWBARS.iter().enumerate() {
        let l = bar_level(v(id));
        if l == 0.0 || (perc && b == 8) {
            continue;
        }
        let from16 = semis + 12;
        let lock = if from16 > 0 && from16 % 12 == 0 {
            0.5
        } else {
            0.0
        };
        let p = ((from16 as f32 / 12.0).exp2() * t + lock).fract();
        y +=
            l * (flute * crate::bank::naive(Shape::Cubic, p) + reed * crate::bank::naive(shape, p));
    }
    y
}

/// A set of wheels, one bit each, in 32-bit words (cheap on the MPC's
/// 32-bit core).
#[derive(Clone, Copy, Debug, Default)]
struct Bits([u32; 4]);

impl Bits {
    #[inline]
    fn set(&mut self, w: usize) {
        self.0[(w >> 5) & 3] |= 1 << (w & 31);
    }

    fn or(self, o: Bits) -> Bits {
        Bits(std::array::from_fn(|i| self.0[i] | o.0[i]))
    }

    /// Each pair of compartment partners (`w`, `w + 48`) with either in
    /// the set, by its lower wheel.
    fn partners(self) -> Bits {
        let [a, b, c, _] = self.0;
        // The set moved down 48 wheels, joined to itself, kept to the 43
        // wheels that have a partner.
        let (d0, d1) = ((b >> 16) | (c << 16), c >> 16);
        Bits([a | d0, (b | d1) & ((1 << 11) - 1), 0, 0])
    }

    /// Calls `f` with each wheel in the set, lowest first.
    #[inline]
    fn each(self, mut f: impl FnMut(usize)) {
        for (i, &word) in self.0.iter().enumerate() {
            let mut m = word;
            while m != 0 {
                f(i * 32 + m.trailing_zeros() as usize);
                m &= m - 1;
            }
        }
    }
}

/// A second-order section (transposed direct form II).
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    s1: f32,
    s2: f32,
}

impl Biquad {
    fn butterworth(hz: f32, rate: f32, high: bool) -> Biquad {
        let w = TAU * hz / rate;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / std::f32::consts::SQRT_2;
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high {
            ((1.0 + cs) / 2.0, -(1.0 + cs))
        } else {
            ((1.0 - cs) / 2.0, 1.0 - cs)
        };
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b0 / a0,
            a1: -2.0 * cs / a0,
            a2: (1.0 - alpha) / a0,
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// A cookbook section from its numerator and denominator.
    fn from(b: [f32; 3], a: [f32; 3]) -> Biquad {
        Biquad {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// The cookbook low-pass or high-pass with its Q.
    fn pass(hz: f32, q: f32, rate: f32, high: bool) -> Biquad {
        let (sn, cs) = (TAU * hz.min(rate * 0.45) / rate).sin_cos();
        let al = sn / (2.0 * q);
        let b = if high {
            [(1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0]
        } else {
            [(1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0]
        };
        Biquad::from(b, [1.0 + al, -2.0 * cs, 1.0 - al])
    }

    /// The cookbook peak, `db` at `hz`.
    fn peak(hz: f32, q: f32, db: f32, rate: f32) -> Biquad {
        let (sn, cs) = (TAU * hz.min(rate * 0.45) / rate).sin_cos();
        let a = 10f32.powf(db / 40.0);
        let al = sn / (2.0 * q);
        Biquad::from(
            [1.0 + al * a, -2.0 * cs, 1.0 - al * a],
            [1.0 + al / a, -2.0 * cs, 1.0 - al / a],
        )
    }

    fn clear(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.s1;
        self.s1 = self.b1 * x - self.a1 * y + self.s2;
        self.s2 = self.b2 * x - self.a2 * y;
        y
    }

    fn flush(&mut self) {
        if self.s1.abs() < 1e-12 && self.s2.abs() < 1e-12 {
            self.s1 = 0.0;
            self.s2 = 0.0;
        }
    }
}

/// `floor(x)` for 0 <= x < 2^22, without a float-to-int conversion: adding
/// 1.5 × 2^23 leaves the rounded integer in the low bits of the float, and
/// rounding `x - 0.5` is the floor (at an exact tie it may land one below,
/// which leaves the fraction at 1.0: still the right interpolation).
#[inline(always)]
fn floor_small(x: f32) -> u32 {
    ((x - 0.5) + 12_582_912.0)
        .to_bits()
        .wrapping_sub(0x4B40_0000)
}

/// Reads `buf` (a ring, `w` the next write) `d` frames back, 4-point Hermite.
#[inline]
fn read_hermite<const N: usize>(buf: &[f32; N], w: usize, d: f32) -> f32 {
    let p = w as f32 - d + N as f32;
    let i = floor_small(p);
    let f = p - i as f32;
    let i = i as usize;
    let m = N - 1;
    let x0 = buf[(i + m) & m];
    let x1 = buf[i & m];
    let x2 = buf[(i + 1) & m];
    let x3 = buf[(i + 2) & m];
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * f + c2) * f + c1) * f + x1
}

/// Reads `buf` `d` frames back, linearly.
#[inline]
fn read_linear<const N: usize>(buf: &[f32; N], w: usize, d: f32) -> f32 {
    let p = w as f32 - d + N as f32;
    let i = floor_small(p);
    let f = p - i as f32;
    let i = i as usize;
    let m = N - 1;
    let a = buf[i & m];
    a + (buf[(i + 1) & m] - a) * f
}

/// A rotor: its angle as a unit vector, its turning per frame and per
/// control step, its speed (turns per second) and the speed it is heading
/// for.
#[derive(Clone, Copy, Debug)]
struct Rotor {
    c: f32,
    s: f32,
    dc: f32,
    ds: f32,
    /// One control step's turn.
    sc: f32,
    ss: f32,
    speed: f32,
    target: f32,
}

impl Rotor {
    fn new(angle: f32) -> Rotor {
        Rotor {
            c: angle.cos(),
            s: angle.sin(),
            dc: 1.0,
            ds: 0.0,
            sc: 1.0,
            ss: 0.0,
            speed: 0.0,
            target: 0.0,
        }
    }

    /// Works out the turns for the speed now.
    fn set_turns(&mut self, rate: f32) {
        let (s, c) = (TAU * self.speed / rate).sin_cos();
        self.dc = c;
        self.ds = s;
        let (s, c) = (TAU * self.speed * CTRL as f32 / rate).sin_cos();
        self.sc = c;
        self.ss = s;
    }

    /// One control step: the speed moves toward its target by the rotor's
    /// inertia (`up` and `down`, the share of the gap closed in a step), and
    /// the angle vector is pulled back to unit length.
    fn step(&mut self, up: f32, down: f32, rate: f32) {
        if self.speed != self.target {
            let k = if self.target > self.speed { up } else { down };
            self.speed += (self.target - self.speed) * k;
            if (self.target - self.speed).abs() < 1e-4 {
                self.speed = self.target;
            }
            // Coasting to a stop: friction ends it once it is barely turning
            // (a third of a turn a minute).
            if self.speed.abs() < 0.005 && self.target == 0.0 {
                self.speed = 0.0;
            }
            self.set_turns(rate);
        }
        let k = 1.5 - 0.5 * (self.c * self.c + self.s * self.s);
        self.c *= k;
        self.s *= k;
    }

    #[inline]
    fn turn(&mut self) {
        let c = self.c * self.dc - self.s * self.ds;
        self.s = self.s * self.dc + self.c * self.ds;
        self.c = c;
    }
}

/// The tonewheel organ. See the module docs.
pub struct Drawbar {
    rate: u32,
    values: [f32; ID_SPACE],
    clock: u64,
    rng: Rng,
    table: &'static [f32; TABLE + 1],
    /// Each wheel's step per frame, and its phase at frame 0: at frame `f`
    /// the wheel is at `base + inc * f` (wrapping), whether it sounds or not.
    base: [u32; N_WHEELS],
    inc: [u32; N_WHEELS],
    /// Each sounding wheel's phasor (cos, sin), its turn per frame, and
    /// whether the phasor is running.
    pc: [f32; N_WHEELS],
    ps: [f32; N_WHEELS],
    rc: [f32; N_WHEELS],
    rs: [f32; N_WHEELS],
    on: [bool; N_WHEELS],
    /// Each wheel's level now and its change per frame across this step.
    amp: [f32; N_WHEELS],
    damp: [f32; N_WHEELS],
    /// The wheels heard in this step: above the lowest octave (`live`) and
    /// in it (`low`, nearly square), and how many of each.
    live: [u8; N_WHEELS],
    n_hi: usize,
    low: [u8; 12],
    n_low: usize,
    n_live: usize,
    /// The wheels heard in the last step.
    heard: Bits,
    /// Each drawbar's level, and the leakage's.
    bar_gain: [f32; 9],
    leak: f32,
    keys: [Key; MAX_KEYS],
    n_keys: usize,
    /// Percussion: its level, whether the next key fires it, and its fall per step.
    perc: f32,
    perc_armed: bool,
    perc_fall: f32,
    /// The scanner: its delay line, write point, phase, and the vibrato
    /// path's 7 kHz low-pass.
    vib: [f32; VIB_LEN],
    vib_w: usize,
    vib_phase: f32,
    vib_lp: f32,
    vib_lp_a: f32,
    /// Each terminal's delay (frames) and level, for the depth chosen.
    term_d: [f32; 9],
    term_g: [f32; 9],
    /// Drive: input gain and the scale that keeps a reference level where it was.
    drive_k: f32,
    drive_n: f32,
    /// The curve's output at its rest point.
    drive_rest: f32,
    dc_x: f32,
    dc_y: f32,
    dc_a: f32,
    /// Tone: the shelves' gains and the split's state.
    tone_lo: Smooth,
    tone_hi: Smooth,
    tone_a: f32,
    tone_lp: f32,
    /// The rotary speaker.
    lp1: Biquad,
    lp2: Biquad,
    hp1: Biquad,
    hp2: Biquad,
    horn_buf: [f32; ROT_LEN],
    drum_buf: [f32; ROT_LEN],
    rot_w: usize,
    horn: Rotor,
    drum: Rotor,
    /// The microphones' directions (cos, sin of the angle each stands off
    /// centre, one each side), for the horn and the drum.
    mic_h: (f32, f32),
    mic_d: (f32, f32),
    /// The horn's treble filter facing a microphone and facing away.
    horn_bright: f32,
    horn_dark: f32,
    /// Doppler depths, the reflection's delay and the leakage's, in frames.
    horn_depth: f32,
    drum_depth: f32,
    reflect_d: f32,
    leak_d: usize,
    /// Each rotor's share of the gap to its target speed closed in a step.
    horn_up: f32,
    horn_down: f32,
    drum_up: f32,
    drum_down: f32,
    /// The horn's treble per microphone: one-pole coefficient now, its
    /// change per frame, and the filter's state.
    horn_a: [f32; 2],
    horn_da: [f32; 2],
    horn_lp: [f32; 2],
    /// How much of the rotary speaker is heard: it fades in and out rather
    /// than switching.
    rot_mix: Smooth,
    out_l: Smooth,
    out_r: Smooth,
    tuning: Tuning,
    /// One control step's mono sum of the wheels.
    bus: [f32; CTRL as usize],
    /// Frames the speaker, delay lines and filters still ring after the
    /// last wheel goes quiet.
    ring: u32,
    /// The combo: the bank (its buses the flute and the reed), how much of
    /// the combo is heard against the wheels now and at the step's end,
    /// with its change per frame, and each voice's level per drawbar at 8.
    bank: Bank<2>,
    mix: f32,
    mix_end: f32,
    dmix: f32,
    flute_gain: f32,
    reed_gain: f32,
    /// Each bank note's level on the flute and on the reed: the keyboard's
    /// level scaling.
    follow: [[f32; crate::bank::NOTES]; 2],
    flute_lp: Biquad,
    reed_hp: Biquad,
    reed_peak: Biquad,
    reed_lp: Biquad,
    /// The combo vibrato: its delay line and write point, its LFO's phase
    /// and step, and its swing (frames either way, gliding; the delay
    /// swings from `CVIB_MIN` to `CVIB_MIN` plus twice it, so the centre
    /// moves with the depth and a chorus combs no lower than it must).
    cvib: [f32; CVIB_LEN],
    cvib_w: usize,
    cvib_ph: u32,
    cvib_inc: u32,
    cvib_amp: Smooth,
    /// Whether the combo was sounding (its share above 0) at the last
    /// render: its line is fed only then, and cleared when it comes back.
    cvib_live: bool,
}

impl Drawbar {
    pub fn new(rate: u32) -> Drawbar {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let mut s = Drawbar {
            rate,
            values,
            clock: 0,
            rng: Rng::new(0x0D2A),
            table: sine_table(),
            base: [0; N_WHEELS],
            inc: [0; N_WHEELS],
            pc: [1.0; N_WHEELS],
            ps: [0.0; N_WHEELS],
            rc: [1.0; N_WHEELS],
            rs: [0.0; N_WHEELS],
            on: [false; N_WHEELS],
            amp: [0.0; N_WHEELS],
            damp: [0.0; N_WHEELS],
            live: [0; N_WHEELS],
            n_hi: 0,
            low: [0; 12],
            n_low: 0,
            n_live: 0,
            heard: Bits::default(),
            bar_gain: [0.0; 9],
            leak: 0.0,
            keys: [Key::default(); MAX_KEYS],
            n_keys: 0,
            perc: 0.0,
            perc_armed: true,
            perc_fall: 1.0,
            vib: [0.0; VIB_LEN],
            vib_w: 0,
            vib_phase: 0.0,
            vib_lp: 0.0,
            vib_lp_a: 0.0,
            term_d: [0.0; 9],
            term_g: [1.0; 9],
            drive_k: 1.0,
            drive_n: 1.0,
            drive_rest: drive_curve(DRIVE_BIAS),
            dc_x: 0.0,
            dc_y: 0.0,
            dc_a: 0.0,
            tone_lo: Smooth::new(1.0),
            tone_hi: Smooth::new(1.0),
            tone_a: 0.0,
            tone_lp: 0.0,
            lp1: Biquad::default(),
            lp2: Biquad::default(),
            hp1: Biquad::default(),
            hp2: Biquad::default(),
            horn_buf: [0.0; ROT_LEN],
            drum_buf: [0.0; ROT_LEN],
            rot_w: 0,
            horn: Rotor::new(0.3),
            drum: Rotor::new(1.9),
            mic_h: (1.0, 0.0),
            mic_d: (1.0, 0.0),
            horn_bright: 0.0,
            horn_dark: 0.0,
            horn_depth: 0.0,
            drum_depth: 0.0,
            reflect_d: 0.0,
            leak_d: 0,
            horn_up: 0.0,
            horn_down: 0.0,
            drum_up: 0.0,
            drum_down: 0.0,
            horn_a: [0.0; 2],
            horn_da: [0.0; 2],
            horn_lp: [0.0; 2],
            rot_mix: Smooth::new(1.0),
            out_l: Smooth::new(0.0),
            out_r: Smooth::new(0.0),
            tuning: Tuning::standard(),
            bus: [0.0; CTRL as usize],
            ring: 0,
            bank: Bank::new(),
            mix: 0.0,
            mix_end: 0.0,
            dmix: 0.0,
            flute_gain: 0.0,
            reed_gain: 0.0,
            follow: combo_follow(),
            flute_lp: Biquad::default(),
            reed_hp: Biquad::default(),
            reed_peak: Biquad::default(),
            reed_lp: Biquad::default(),
            cvib: [0.0; CVIB_LEN],
            cvib_w: 0,
            cvib_ph: 0,
            cvib_inc: 0,
            cvib_amp: Smooth::new(0.0),
            cvib_live: false,
        };
        s.prepare(rate, mpfree_node::MAX_BLOCK);
        s
    }

    /// Loads a factory preset: defaults, then the preset's values.
    pub fn load_preset(&mut self, preset: &Preset) {
        for p in PARAMS {
            if !p.readonly {
                self.set_param(p.id, p.default);
            }
        }
        for &(id, v) in preset.values {
            self.set_param(id, v);
        }
    }

    pub fn presets() -> &'static [Preset] {
        crate::factory::presets!("drawbar.presets", &INFO, PARAMS)
    }

    /// Keys held down.
    pub fn keys_down(&self) -> usize {
        self.keys[..self.n_keys].iter().filter(|k| k.down).count()
    }

    /// Keys down or still letting go (their contacts opening).
    pub fn keys_sounding(&self) -> usize {
        self.n_keys
    }

    /// What a host counts as voices: the keys sounding, or one while the
    /// last of the sound still rings out of the speaker and filters.
    pub fn active_voices(&self) -> usize {
        if self.n_keys > 0 {
            self.n_keys
        } else {
            usize::from(self.n_live > 0 || self.bank.heard() > 0 || self.ring > 0)
        }
    }

    /// Notes of the combo's bank heard now.
    pub fn bank_notes_heard(&self) -> usize {
        self.bank.heard()
    }

    /// Note `n`'s frequency in the combo's bank, Hz.
    pub fn bank_hz(&self, n: usize) -> f64 {
        self.bank.hz(n)
    }

    /// Whether note `n` of the combo's bank is locked to the octave below.
    pub fn bank_locked(&self, n: usize) -> bool {
        self.bank.locked(n)
    }

    /// The horn's and the drum's speeds now, in turns per second.
    pub fn rotor_speeds(&self) -> (f32, f32) {
        (self.horn.speed, self.drum.speed)
    }

    #[inline]
    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn rotary_on(&self) -> bool {
        self.v(ROTARY) >= 0.5
    }

    /// Whether the rotary speaker is heard at all, or fading.
    fn rotary_heard(&self) -> bool {
        self.rot_mix.value > 0.0 || self.rot_mix.target > 0.0
    }

    /// Empties the rotary speaker's delay lines and filters, so that it
    /// starts clean when it fades back in.
    fn clear_rotary(&mut self) {
        for b in [&mut self.lp1, &mut self.lp2, &mut self.hp1, &mut self.hp2] {
            b.s1 = 0.0;
            b.s2 = 0.0;
        }
        self.horn_buf = [0.0; ROT_LEN];
        self.drum_buf = [0.0; ROT_LEN];
        self.horn_lp = [0.0; 2];
        self.horn_da = [0.0; 2];
    }

    fn update_rotary(&mut self) {
        let on = self.rotary_on();
        if on && !self.rotary_heard() {
            self.clear_rotary();
        }
        self.rot_mix.target = if on { 1.0 } else { 0.0 };
        if self.quiet() {
            self.rot_mix.snap(self.rot_mix.target);
        }
    }

    /// No key down or letting go, and no wheel heard: a change can jump
    /// rather than glide, so loading a preset never starts with a swell.
    fn quiet(&self) -> bool {
        self.n_keys == 0 && self.n_live == 0 && self.bank.heard() == 0
    }

    fn combo_on(&self) -> bool {
        self.v(MODEL) >= 0.5
    }

    /// The model: heard at once when nothing sounds, else crossed over
    /// in [`MODEL_STEPS`] control steps.
    fn update_model(&mut self) {
        if self.quiet() {
            let to = if self.combo_on() { 1.0 } else { 0.0 };
            self.mix = to;
            self.mix_end = to;
            self.dmix = 0.0;
        }
    }

    /// The combo vibrato's swing and rate for **Vibrato** and **Scanner**:
    /// the delay's swing, in frames, that bends the pitch by the depth's
    /// cents either way at that rate.
    fn update_combo_vib(&mut self) {
        let mode = self.v(VIBRATO) as usize;
        let cents = if mode == 0 {
            0.0
        } else {
            COMBO_VIB_CENTS[(mode - 1) % 3]
        };
        let hz = self.v(SCANNER);
        let r = self.rate as f32;
        let ratio = (cents / 1200.0).exp2() - 1.0;
        let widest = (CVIB_LEN as f32 - CVIB_MIN - 4.0) * 0.5;
        self.cvib_amp.target = (ratio * r / (TAU * hz)).min(widest);
        if self.quiet() {
            self.cvib_amp.snap(self.cvib_amp.target);
        }
        self.cvib_inc = (f64::from(hz) / f64::from(r) * 4_294_967_296.0) as u32;
    }

    fn update_out(&mut self) {
        let db = self.v(LEVEL);
        let lvl = if db <= -60.0 {
            0.0
        } else {
            10f32.powf(db / 20.0)
        };
        let pan = self.v(PAN) / 100.0;
        self.out_l.target = lvl * (1.0 - pan).sqrt();
        self.out_r.target = lvl * (1.0 + pan).sqrt();
        if self.quiet() {
            self.out_l.snap(self.out_l.target);
            self.out_r.snap(self.out_r.target);
        }
    }

    fn update_tone(&mut self) {
        let t = self.v(TONE) / 100.0;
        self.tone_hi.target = 10f32.powf(0.45 * t);
        self.tone_lo.target = 10f32.powf(-0.15 * t);
        if self.quiet() {
            self.tone_hi.snap(self.tone_hi.target);
            self.tone_lo.snap(self.tone_lo.target);
        }
    }

    fn update_drive(&mut self) {
        let d = self.v(DRIVE) / 100.0;
        // Nearly linear at 0 (the curve barely leaves its rest point).
        self.drive_k = 0.1 + 24.0 * d * d;
        // A signal at 0.4 comes out at 0.4 whatever the drive.
        let c = drive_curve;
        let r = 0.4;
        self.drive_n = r / (c(self.drive_k * r + DRIVE_BIAS) - c(DRIVE_BIAS));
    }

    fn update_perc(&mut self) {
        let t60 = if self.v(PERC_DECAY) >= 0.5 {
            PERC_SLOW_S
        } else {
            PERC_FAST_S
        };
        self.perc_fall = (-6.9078 * CTRL as f32 / (t60 * self.rate as f32)).exp();
    }

    fn update_scanner(&mut self) {
        let mode = self.v(VIBRATO) as usize;
        let depth = if mode == 0 { 0 } else { (mode - 1) % 3 };
        let r = self.rate as f32;
        for (t, &tap) in SCAN_TAPS[depth].iter().enumerate() {
            self.term_d[t] = f32::from(tap) * TAP_S * r;
            // The ladder's first taps are a little quieter (-2.9 dB at tap 1,
            // full from tap 7).
            let db = -2.9 * (1.0 - (f32::from(tap) - 1.0) / 6.0).max(0.0);
            self.term_g[t] = 10f32.powf(db / 20.0);
        }
    }

    fn update_mics(&mut self) {
        let a = self.v(SPREAD) / 100.0 * PI * 0.5;
        self.mic_h = (a.cos(), a.sin());
        let ad = a * 0.5;
        self.mic_d = (ad.cos(), ad.sin());
    }

    fn update_bars(&mut self) {
        for (b, g) in self.bar_gain.iter_mut().enumerate() {
            *g = bar_level(self.values[DRAWBARS[b].0 as usize]);
        }
        self.flute_gain = bar_level(self.v(FLUTE)) * COMBO_FLUTE;
        let trim = if self.v(REED_WAVE) >= 0.5 {
            SQUARE_TRIM
        } else {
            1.0
        };
        self.reed_gain = bar_level(self.v(REED)) * COMBO_REED * trim;
        // -40 dB at 40 %, -26 dB at 100 %.
        self.leak = (self.v(LEAKAGE) / 100.0).powf(1.75) * LEAK_MAX;
    }

    /// Works out every key's wheels again (after a transpose).
    fn rekey(&mut self) {
        let t = self.v(TRANSPOSE) as i32;
        for k in self.keys[..self.n_keys].iter_mut() {
            (k.wheels, k.reach, k.notes) = key_wheels(k.note, t);
        }
    }

    fn update_speed(&mut self) {
        let (h, d) = match self.v(SPEED) as usize {
            0 => (0.0, 0.0),
            1 => (HORN_SLOW, DRUM_SLOW),
            _ => (HORN_FAST, DRUM_FAST),
        };
        self.horn.target = h;
        self.drum.target = d;
    }

    /// The project's tuning, with the gears' own small offsets from equal
    /// temperament on top for **Wheels** Gears (so in 12-TET at 440 Hz the
    /// wheels are exactly the gears' pitches).
    fn retune(&mut self) {
        let fine = 2f64.powf(f64::from(self.v(FINE)) / 1200.0);
        let gears = self.v(WHEELS) < 0.5;
        let r = f64::from(self.rate);
        let now = self.clock as u32;
        for w in 0..N_WHEELS {
            // Where the wheel is now, kept as its speed changes.
            let at = self.base[w].wrapping_add(self.inc[w].wrapping_mul(now));
            let note = FIRST_NOTE + w as i32;
            let mut hz = f64::from(self.tuning.hz(note as f32)) * fine;
            if gears {
                let equal = 440.0 * (f64::from(note - 69) / 12.0).exp2();
                hz *= gear_hz(w) / equal;
            }
            let hz = hz.clamp(1.0, r * 0.49);
            self.inc[w] = (hz / r * 4_294_967_296.0) as u32;
            self.base[w] = at.wrapping_sub(self.inc[w].wrapping_mul(now));
            let turn = f64::from(self.inc[w]) / 4_294_967_296.0 * std::f64::consts::TAU;
            self.rc[w] = turn.cos() as f32;
            self.rs[w] = turn.sin() as f32;
        }
        // The combo's bank: the project's tuning as it is, and Fine.
        self.bank
            .retune(&self.tuning, self.v(FINE) / 100.0, self.rate, self.clock);
    }

    /// A contact from `from` to `to` timed for **Click**: how long it
    /// waits and moves, in control steps, and whether it bounces.
    fn contact(&mut self, from: f32, to: f32, closing: bool) -> Contact {
        let c = self.v(CLICK) / 100.0;
        let step_ms = CTRL as f32 * 1000.0 / self.rate as f32;
        // Quick and uneven when clicky (well under a millisecond, spread
        // over up to 3 ms), slow and together when not (6 ms, no spread).
        let move_ms = 0.3 + 6.0 * (1.0 - c).powi(3);
        let spread_ms = 3.0 * c;
        let (move_ms, spread_ms) = if closing {
            (move_ms, spread_ms)
        } else {
            (move_ms * 2.0, spread_ms * 0.5)
        };
        let len = (move_ms / step_ms).round().max(1.0) as u16;
        let wait = (self.rng.unit() * spread_ms / step_ms) as u16;
        let bounce = if closing && self.rng.unit() < 0.5 * c {
            wait + len + (self.rng.unit() * 3.0) as u16
        } else {
            NEVER
        };
        Contact {
            value: from,
            to,
            step: (to - from) / f32::from(len),
            wait,
            left: len,
            bounce,
        }
    }

    fn note_on(&mut self, note: u8) {
        if self.v(PERC) >= 0.5 && self.perc_armed {
            self.perc = 1.0;
        }
        self.perc_armed = false;
        let found = self.keys[..self.n_keys].iter().position(|k| k.note == note);
        let idx = match found {
            Some(i) => i,
            None if self.n_keys < MAX_KEYS => {
                self.n_keys += 1;
                self.n_keys - 1
            }
            None => {
                // Full: the key letting go longest is replaced.
                match (0..self.n_keys)
                    .filter(|&i| !self.keys[i].down)
                    .max_by_key(|&i| self.keys[i].t)
                {
                    Some(i) => i,
                    None => return,
                }
            }
        };
        let old = self.keys[idx];
        let (wheels, reach, notes) = key_wheels(note, self.v(TRANSPOSE) as i32);
        let mut k = Key {
            note,
            down: true,
            settled: false,
            t: 0,
            wheels,
            reach,
            notes,
            contacts: [Contact::default(); 9],
        };
        for b in 0..9 {
            // Struck again while letting go: each contact closes from where it is.
            let from = if found.is_some() {
                old.contacts[b].value
            } else {
                0.0
            };
            k.contacts[b] = self.contact(from, 1.0, true);
        }
        self.keys[idx] = k;
    }

    fn note_off(&mut self, note: u8) {
        let Some(i) = self.keys[..self.n_keys]
            .iter()
            .position(|k| k.note == note && k.down)
        else {
            return;
        };
        let old = self.keys[i];
        let mut k = old;
        k.down = false;
        k.settled = false;
        k.t = 0;
        for b in 0..9 {
            k.contacts[b] = self.contact(old.contacts[b].value, 0.0, false);
        }
        self.keys[i] = k;
        if self.keys_down() == 0 {
            self.perc_armed = true;
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, .. } => self.note_on(note),
            NodeEvent::NoteOff { note } => self.note_off(note),
            NodeEvent::AllOff => {
                for i in 0..self.n_keys {
                    if self.keys[i].down {
                        let n = self.keys[i].note;
                        self.note_off(n);
                    }
                }
            }
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Bend { .. } | NodeEvent::Pressure { .. } | NodeEvent::Expression { .. } => {}
            // The pedals are held by the host for this engine (`Node::hears`).
            NodeEvent::Control { .. } => {}
        }
    }

    /// One control step: contacts and percussion become each wheel's level
    /// for the step, leakage is added, the rotors and the horn's treble move.
    fn control(&mut self, frame: u64) {
        // The model's crossover: the combo's share at the step's end (the
        // wheels have the rest).
        self.mix = self.mix_end;
        let to = if self.combo_on() { 1.0 } else { 0.0 };
        if self.mix_end != to {
            let d = 1.0 / MODEL_STEPS;
            self.mix_end = if to > self.mix_end {
                (self.mix_end + d).min(1.0)
            } else {
                (self.mix_end - d).max(0.0)
            };
        }
        self.dmix = (self.mix_end - self.mix) / CTRL as f32;
        let (wg, cg) = (1.0 - self.mix_end, self.mix_end);
        self.bank.begin_step();
        let mut target = [0.0f32; N_WHEELS];
        // The wheels the keys reach.
        let mut touched = Bits::default();
        let perc_on = self.v(PERC) >= 0.5;
        let drop = if perc_on && self.v(PERC_VOL) < 0.5 {
            PERC_NORMAL_DROP
        } else {
            1.0
        };
        let mut levels = self.bar_gain;
        for l in levels.iter_mut() {
            *l *= drop;
        }
        if perc_on {
            levels[8] = 0.0;
        }
        let perc_bar = if self.v(PERC_HARM) >= 0.5 { 4 } else { 3 };
        let perc_level = if self.v(PERC_VOL) >= 0.5 {
            PERC_SOFT
        } else {
            PERC_NORMAL
        };
        let perc_amp = if perc_on { perc_level * self.perc } else { 0.0 };
        let mut i = 0;
        while i < self.n_keys {
            let k = &mut self.keys[i];
            let mut g = [1.0f32; 9];
            if k.settled {
                if !k.down {
                    // Fully let go: drop it.
                    self.n_keys -= 1;
                    self.keys[i] = self.keys[self.n_keys];
                    continue;
                }
            } else {
                let mut all = true;
                for (gb, c) in g.iter_mut().zip(k.contacts.iter_mut()) {
                    *gb = c.tick();
                    all &= c.settled();
                }
                k.settled = all;
            }
            for b in 0..9 {
                target[k.wheels[b] as usize] += levels[b] * g[b];
            }
            if perc_amp > 0.0 {
                target[k.wheels[perc_bar] as usize] += perc_amp * g[8];
            }
            touched = touched.or(k.reach);
            k.t = k.t.saturating_add(1);
            if cg > 0.0 {
                let notes = k.notes;
                self.combo_key(&notes, &levels, &g, perc_bar, perc_amp, cg, frame);
            }
            i += 1;
        }
        self.bank.end_step();
        self.perc *= self.perc_fall;
        if self.perc < 1e-4 {
            self.perc = 0.0;
        }
        // Compartment partners, four octaves apart, hear each other.
        let leak = self.leak;
        if leak > 0.0 {
            touched.partners().each(|w| {
                let (lo, hi) = (target[w], target[w + 48]);
                target[w] = lo + hi * leak;
                target[w + 48] = hi + lo * leak;
                touched.set(w);
                touched.set(w + 48);
            });
        }
        // On the combo alone the wheels only fade out.
        if wg == 0.0 {
            touched = Bits::default();
        }
        if self.n_keys > 0 || self.n_live > 0 || self.bank.heard() > 0 {
            self.ring = self.tail_frames();
        } else {
            self.ring = self.ring.saturating_sub(CTRL as u32);
        }
        // Each wheel moves from where it is to its target across the step;
        // a wheel silent through the step only turns.
        let n = CTRL as f32;
        let table = self.table;
        self.n_hi = 0;
        self.n_low = 0;
        // Only the wheels the keys reach or that were heard last step: a
        // silent wheel keeps turning on its own count (see `base`).
        let now = frame as u32;
        let mut heard = Bits::default();
        touched.or(self.heard).each(|w| {
            let t = target[w] * OUT_GAIN * wg;
            // Land on silence (a ramp down ends within rounding of it).
            if self.amp[w].abs() < 1e-7 {
                self.amp[w] = 0.0;
            }
            if t == 0.0 && self.amp[w] == 0.0 {
                self.on[w] = false;
                return;
            }
            heard.set(w);
            if self.on[w] {
                // Pulled back to unit length (one Newton step: it is never
                // far off).
                let (c, s) = (self.pc[w], self.ps[w]);
                let k = 1.5 - 0.5 * (c * c + s * s);
                self.pc[w] = c * k;
                self.ps[w] = s * k;
            } else {
                // Coming in: the phasor starts where the wheel is.
                let start = self.base[w].wrapping_add(self.inc[w].wrapping_mul(now));
                self.pc[w] = sine(table, start.wrapping_add(1 << 30));
                self.ps[w] = sine(table, start);
                self.on[w] = true;
            }
            self.damp[w] = (t - self.amp[w]) / n;
            if w < 12 {
                self.low[self.n_low] = w as u8;
                self.n_low += 1;
            } else {
                self.live[self.n_hi] = w as u8;
                self.n_hi += 1;
            }
        });
        self.n_live = self.n_hi + self.n_low;
        self.heard = heard;

        let r = self.rate as f32;
        self.horn.step(self.horn_up, self.horn_down, r);
        self.drum.step(self.drum_up, self.drum_down, r);
        if self.rotary_heard() {
            // Where the horn will face at the end of this step, from each
            // microphone: its treble filter heads there.
            let (hc, hs) = self.mic_h;
            let c = self.horn.c * self.horn.sc - self.horn.s * self.horn.ss;
            let s = self.horn.s * self.horn.sc + self.horn.c * self.horn.ss;
            for (m, side) in [1.0f32, -1.0].into_iter().enumerate() {
                let face = 0.5 * (1.0 + c * hc + side * s * hs);
                let a = self.horn_dark + (self.horn_bright - self.horn_dark) * face * face;
                self.horn_da[m] = (a - self.horn_a[m]) / n;
            }
        } else {
            self.horn_da = [0.0; 2];
        }
        for x in [
            &mut self.vib_lp,
            &mut self.dc_x,
            &mut self.dc_y,
            &mut self.tone_lp,
        ]
        .into_iter()
        .chain(self.horn_lp.iter_mut())
        {
            if x.abs() < 1e-12 {
                *x = 0.0;
            }
        }
        for b in [
            &mut self.lp1,
            &mut self.lp2,
            &mut self.hp1,
            &mut self.hp2,
            &mut self.flute_lp,
            &mut self.reed_hp,
            &mut self.reed_peak,
            &mut self.reed_lp,
        ] {
            b.flush();
        }
    }

    /// One key's notes onto the combo's bank for this step: each drawbar's
    /// level (`levels`, its contact `g`) on its note, through the flute
    /// and the reed, and the percussion through the flute; `cg` is the
    /// combo's share.
    #[allow(clippy::too_many_arguments)]
    fn combo_key(
        &mut self,
        notes: &[u8; 9],
        levels: &[f32; 9],
        g: &[f32; 9],
        perc_bar: usize,
        perc_amp: f32,
        cg: f32,
        frame: u64,
    ) {
        let (fl, rl) = (self.flute_gain * cg, self.reed_gain * cg);
        if fl > 0.0 || rl > 0.0 {
            for b in 0..9 {
                let l = levels[b] * g[b];
                let n = usize::from(notes[b]);
                if l == 0.0 || !self.bank.sounds(n) {
                    continue;
                }
                let (f, r) = (l * fl * self.follow[0][n], l * rl * self.follow[1][n]);
                self.bank.add_pair(FLUTE_BUS, n, f, r, frame);
            }
        }
        let l = perc_amp * g[8];
        let n = usize::from(notes[perc_bar]);
        if l > 0.0 && self.bank.sounds(n) {
            let p = l * COMBO_FLUTE * cg * self.follow[0][n];
            self.bank.add(FLUTE_BUS, n, p, frame);
        }
    }

    /// The combo's voices for the next `n` frames, added into the bus.
    #[inline(never)]
    fn combo(&mut self, n: usize) {
        let mut buses = [[0.0f32; CTRL as usize]; 2];
        let reed = if self.v(REED_WAVE) >= 0.5 {
            Shape::Square
        } else {
            Shape::Saw
        };
        self.bank.render(n, &mut buses, &[Shape::Cubic, reed]);
        for k in 0..n {
            let f = self.flute_lp.run(buses[FLUTE_BUS][k]);
            let r = self
                .reed_lp
                .run(self.reed_peak.run(self.reed_hp.run(buses[REED_BUS][k])));
            self.bus[k] += f + r;
        }
    }

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let n = to - from;
        let r = self.rate as f32;
        let scan = self.v(SCANNER) / r;
        // The combo's line is fed only while the combo has a share, so the
        // tonewheel alone pays nothing for it; coming back, it starts from
        // silence rather than replaying an old note.
        let live = self.mix > 0.0 || self.mix_end > 0.0;
        if live && !self.cvib_live {
            self.cvib.fill(0.0);
        }
        self.cvib_live = live;
        if self.n_live == 0 && self.ring == 0 {
            // Silent, and the tail has rung out: only the scanner and the
            // rotors keep turning.
            for _ in 0..n {
                self.vib_phase += scan;
                if self.vib_phase >= 1.0 {
                    self.vib_phase -= 1.0;
                }
                self.horn.turn();
                self.drum.turn();
            }
            self.vib_w = (self.vib_w + n) & (VIB_LEN - 1);
            self.rot_w = (self.rot_w + n) & (ROT_LEN - 1);
            self.cvib_w = (self.cvib_w + n) & (CVIB_LEN - 1);
            self.cvib_ph = self
                .cvib_ph
                .wrapping_add(self.cvib_inc.wrapping_mul(n as u32));
            self.mix += self.dmix * n as f32;
            self.clock += n as u64;
            return;
        }
        self.wheels(n);
        if self.mix > 0.0 || self.mix_end > 0.0 || self.bank.heard() > 0 {
            self.combo(n);
        }
        let mode = self.v(VIBRATO) as usize;
        let chorus = mode >= 4;
        // Settled smoothers are read once, not stepped every frame.
        let tone_fixed = self.tone_lo.settled() && self.tone_hi.settled();
        let out_fixed = self.out_l.settled() && self.out_r.settled();
        let mix_fixed = self.rot_mix.settled();
        let (mut lo, mut hi) = (self.tone_lo.value, self.tone_hi.value);
        let (mut gl, mut gr) = (self.out_l.value, self.out_r.value);
        let mut mix = self.rot_mix.value;
        let (hmc, hms) = self.mic_h;
        let (dmc, dms) = self.mic_d;
        let (hd, dd, rd) = (self.horn_depth, self.drum_depth, self.reflect_d);
        for k in 0..n {
            let dry = self.bus[k];
            // The scanner. The line is always fed and the scanner always
            // turns, so switching it on mid-note picks up cleanly.
            self.vib[self.vib_w] = dry;
            if live {
                self.cvib[self.cvib_w] = dry;
            }
            let mut x = dry;
            let m = self.mix;
            if mode > 0 {
                if m < 1.0 {
                    let tri = 1.0 - (2.0 * self.vib_phase - 1.0).abs();
                    let pos = tri * 7.999;
                    let ti = (floor_small(pos) as usize).min(7);
                    let f = pos - ti as f32;
                    let a = read_linear(&self.vib, self.vib_w, self.term_d[ti]) * self.term_g[ti];
                    let b = read_linear(&self.vib, self.vib_w, self.term_d[ti + 1])
                        * self.term_g[ti + 1];
                    let wet = a + (b - a) * f;
                    self.vib_lp += (wet - self.vib_lp) * self.vib_lp_a;
                    x = if chorus {
                        0.6 * (dry + self.vib_lp)
                    } else {
                        self.vib_lp
                    };
                }
                if m > 0.0 {
                    // The combo's vibrato: the line read through a delay
                    // swung by a sine, so the whole bank bends evenly.
                    let a = self.cvib_amp.next();
                    let d = CVIB_MIN + a + a * sine(self.table, self.cvib_ph);
                    let wet = read_hermite(&self.cvib, self.cvib_w, d);
                    let xc = if chorus { 0.6 * (dry + wet) } else { wet };
                    x = if m >= 1.0 { xc } else { x + (xc - x) * m };
                }
            }
            if live {
                self.mix += self.dmix;
                self.cvib_w = (self.cvib_w + 1) & (CVIB_LEN - 1);
                self.cvib_ph = self.cvib_ph.wrapping_add(self.cvib_inc);
            }
            self.vib_w = (self.vib_w + 1) & (VIB_LEN - 1);
            self.vib_phase += scan;
            if self.vib_phase >= 1.0 {
                self.vib_phase -= 1.0;
            }
            // Drive, then a DC blocker for what the asymmetry adds.
            let u = self.drive_k * x + DRIVE_BIAS;
            let y = (drive_curve(u) - self.drive_rest) * self.drive_n;
            let hp = y - self.dc_x + self.dc_a * self.dc_y;
            self.dc_x = y;
            self.dc_y = hp;
            // Tone.
            self.tone_lp += (hp - self.tone_lp) * self.tone_a;
            if !tone_fixed {
                lo = self.tone_lo.next();
                hi = self.tone_hi.next();
            }
            let x = hi * hp + (lo - hi) * self.tone_lp;

            if !mix_fixed {
                mix = self.rot_mix.next();
            }
            let (l, rr) = if mix > 0.0 {
                let low = self.lp2.run(self.lp1.run(x));
                let high = self.hp2.run(self.hp1.run(x));
                let w = self.rot_w;
                self.horn_buf[w] = high;
                self.drum_buf[w] = low;
                self.horn.turn();
                self.drum.turn();
                // The microphones stand either side of centre: one sees the
                // rotor's angle plus its offset, the other minus.
                let (hc, hs) = (self.horn.c * hmc, self.horn.s * hms);
                let (dc, ds) = (self.drum.c * dmc, self.drum.s * dms);
                let leak = HORN_LEAK * self.horn_buf[(w + ROT_LEN - self.leak_d) & (ROT_LEN - 1)];
                let h0 = horn_at(&self.horn_buf, w, hc + hs, hd, rd);
                let h1 = horn_at(&self.horn_buf, w, hc - hs, hd, rd);
                self.horn_a[0] += self.horn_da[0];
                self.horn_a[1] += self.horn_da[1];
                self.horn_lp[0] += (h0 - self.horn_lp[0]) * self.horn_a[0];
                self.horn_lp[1] += (h1 - self.horn_lp[1]) * self.horn_a[1];
                let o0 = self.horn_lp[0] + leak + drum_at(&self.drum_buf, w, dc + ds, dd);
                let o1 = self.horn_lp[1] + leak + drum_at(&self.drum_buf, w, dc - ds, dd);
                self.rot_w = (w + 1) & (ROT_LEN - 1);
                (x + (o0 - x) * mix, x + (o1 - x) * mix)
            } else {
                (x, x)
            };
            if !out_fixed {
                gl = self.out_l.next();
                gr = self.out_r.next();
            }
            left[from + k] += l * gl;
            right[from + k] += rr * gr;
        }
        if !live {
            self.cvib_w = (self.cvib_w + n) & (CVIB_LEN - 1);
            self.cvib_ph = self
                .cvib_ph
                .wrapping_add(self.cvib_inc.wrapping_mul(n as u32));
        }
        self.clock += n as u64;
    }

    /// The sounding wheels' sum for the next `n` frames, into the bus.
    fn wheels(&mut self, n: usize) {
        let bus = &mut self.bus[..n];
        bus.fill(0.0);
        // Two wheels at a time, so the bus is read and written once for both.
        let mut j = 0;
        while j + 1 < self.n_hi {
            let (w1, w2) = (self.live[j] as usize, self.live[j + 1] as usize);
            let (mut c1, mut s1, rc1, rs1) = (self.pc[w1], self.ps[w1], self.rc[w1], self.rs[w1]);
            let (mut c2, mut s2, rc2, rs2) = (self.pc[w2], self.ps[w2], self.rc[w2], self.rs[w2]);
            let (mut a1, da1, mut a2, da2) =
                (self.amp[w1], self.damp[w1], self.amp[w2], self.damp[w2]);
            for x in bus.iter_mut() {
                let t1 = c1 * rc1 - s1 * rs1;
                s1 = s1 * rc1 + c1 * rs1;
                c1 = t1;
                let t2 = c2 * rc2 - s2 * rs2;
                s2 = s2 * rc2 + c2 * rs2;
                c2 = t2;
                a1 += da1;
                a2 += da2;
                *x += a1 * s1 + a2 * s2;
            }
            (self.pc[w1], self.ps[w1], self.amp[w1]) = (c1, s1, a1);
            (self.pc[w2], self.ps[w2], self.amp[w2]) = (c2, s2, a2);
            j += 2;
        }
        if j < self.n_hi {
            let w = self.live[j] as usize;
            let (mut c, mut s, rc, rs) = (self.pc[w], self.ps[w], self.rc[w], self.rs[w]);
            let (mut a, da) = (self.amp[w], self.damp[w]);
            for x in bus.iter_mut() {
                let t = c * rc - s * rs;
                s = s * rc + c * rs;
                c = t;
                a += da;
                *x += a * s;
            }
            (self.pc[w], self.ps[w], self.amp[w]) = (c, s, a);
        }
        // The lowest octave, nearly square: sin + sin 3x / 3 + sin 5x / 5
        // from the phasor's sine alone.
        for j in 0..self.n_low {
            let w = self.low[j] as usize;
            let (mut c, mut s, rc, rs) = (self.pc[w], self.ps[w], self.rc[w], self.rs[w]);
            let (mut a, da) = (self.amp[w], self.damp[w]);
            for x in bus.iter_mut() {
                let t = c * rc - s * rs;
                s = s * rc + c * rs;
                c = t;
                a += da;
                let q = s * s;
                *x += a * s * (3.0 - (16.0 / 3.0) * q + 3.2 * q * q) * 0.8;
            }
            (self.pc[w], self.ps[w], self.amp[w]) = (c, s, a);
        }
    }
}

/// The horn as one microphone hears it, facing it by `face` (1 straight at
/// it, -1 away): the direct sound through a delay that follows the horn's
/// distance (Doppler) and a level that follows where it points, and one
/// cabinet reflection moving the other way.
#[inline(always)]
fn horn_at(buf: &[f32; ROT_LEN], w: usize, face: f32, depth: f32, reflect: f32) -> f32 {
    let d = 2.0 + depth * (1.0 - face);
    let g = 1.0 - HORN_AM * 0.5 * (1.0 - face);
    let dr = 2.0 + reflect + depth * (1.0 + face);
    let gr = REFLECT * (1.0 - HORN_AM * 0.5 * (1.0 + face));
    g * read_hermite(buf, w, d) + gr * read_linear(buf, w, dr)
}

/// The drum as one microphone hears it.
#[inline(always)]
fn drum_at(buf: &[f32; ROT_LEN], w: usize, face: f32, depth: f32) -> f32 {
    let d = 2.0 + depth * (1.0 - face);
    (1.0 - DRUM_AM * 0.5 * (1.0 - face)) * read_linear(buf, w, d)
}

/// The drive curve's rest point: off centre, so it adds even harmonics.
const DRIVE_BIAS: f32 = 0.25;

/// The drive's curve: smooth everywhere (`u / sqrt(1 + u²)`), so pushing
/// it hard adds harmonics that fall away quickly rather than folding back
/// as grit, as a curve with a corner would.
#[inline(always)]
fn drive_curve(u: f32) -> f32 {
    u / (1.0 + u * u).sqrt()
}

/// The combo's keyboard level scaling: each bank note's level on the flute
/// and on the reed.
fn combo_follow() -> [[f32; crate::bank::NOTES]; 2] {
    let mut f = [[1.0; crate::bank::NOTES]; 2];
    for n in 0..crate::bank::NOTES {
        let k = (n as f32).clamp(FOLLOW_LO, FOLLOW_HI);
        let flute = FLUTE_LIFT_DB * (60.0 - k).max(0.0) - FLUTE_EASE_DB * (k - 72.0).max(0.0);
        let reed = REED_TILT_DB * (60.0 - k);
        f[FLUTE_BUS][n] = 10f32.powf(flute / 20.0);
        f[REED_BUS][n] = 10f32.powf(reed / 20.0);
    }
    f
}

impl Node for Drawbar {
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
        self.values[id as usize] = p.clamp(value);
        match id {
            LEVEL | PAN => self.update_out(),
            TONE => self.update_tone(),
            DRIVE => self.update_drive(),
            PERC_DECAY => self.update_perc(),
            VIBRATO => {
                self.update_scanner();
                self.update_combo_vib();
            }
            SPREAD => self.update_mics(),
            SPEED => self.update_speed(),
            ROTARY => self.update_rotary(),
            MODEL => self.update_model(),
            FLUTE | REED | REED_WAVE => self.update_bars(),
            SCANNER => self.update_combo_vib(),
            FINE | WHEELS => self.retune(),
            TRANSPOSE => self.rekey(),
            LEAKAGE | DB_16 | DB_5_13 | DB_8 | DB_4 | DB_2_23 | DB_2 | DB_1_35 | DB_1_13 | DB_1 => {
                self.update_bars()
            }
            PERC if value < 0.5 => self.perc = 0.0,
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

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate;
        let r = rate as f32;
        self.out_l.set_time(rate, 10.0);
        self.out_r.set_time(rate, 10.0);
        self.rot_mix.set_time(rate, 15.0);
        self.tone_lo.set_time(rate, 10.0);
        self.tone_hi.set_time(rate, 10.0);
        self.tone_a = 1.0 - (-TAU * 900.0 / r).exp();
        self.vib_lp_a = 1.0 - (-TAU * 7000.0_f32.min(r * 0.45) / r).exp();
        self.dc_a = (-TAU * 10.0 / r).exp();
        self.lp1 = Biquad::butterworth(CROSSOVER_HZ, r, false);
        self.lp2 = self.lp1;
        self.hp1 = Biquad::butterworth(CROSSOVER_HZ, r, true);
        self.hp2 = self.hp1;
        let one_pole = |hz: f32| 1.0 - (-TAU * hz.min(r * 0.45) / r).exp();
        self.horn_bright = one_pole(HORN_BRIGHT_HZ);
        self.horn_dark = one_pole(HORN_DARK_HZ);
        self.horn_depth = HORN_R / SOUND_MS * r;
        self.drum_depth = DRUM_R / SOUND_MS * r;
        self.reflect_d = REFLECT_S * r;
        self.leak_d = (2.0 + self.horn_depth).round() as usize;
        let q = std::f32::consts::FRAC_1_SQRT_2;
        self.flute_lp = Biquad::pass(FLUTE_TOP_HZ, q, r, false);
        self.reed_hp = Biquad::pass(REED_LOW_HZ, q, r, true);
        self.reed_peak = Biquad::peak(REED_PEAK_HZ, REED_PEAK_Q, REED_PEAK_DB, r);
        self.reed_lp = Biquad::pass(REED_TOP_HZ, REED_TOP_Q, r, false);
        self.cvib_amp.set_time(rate, 20.0);
        let dt = CTRL as f32 / r;
        let share = |tau: f32| 1.0 - (-dt / tau).exp();
        self.horn_up = share(HORN_UP_S);
        self.horn_down = share(HORN_DOWN_S);
        self.drum_up = share(DRUM_UP_S);
        self.drum_down = share(DRUM_DOWN_S);
        self.reset();
    }

    fn reset(&mut self) {
        self.keys = [Key::default(); MAX_KEYS];
        self.n_keys = 0;
        self.perc = 0.0;
        self.perc_armed = true;
        self.amp = [0.0; N_WHEELS];
        self.damp = [0.0; N_WHEELS];
        self.on = [false; N_WHEELS];
        self.heard = Bits::default();
        self.n_hi = 0;
        self.n_low = 0;
        self.n_live = 0;
        // The wheels' phases bear no relation to each other.
        let mut rng = Rng::new(0x7EE1);
        for p in self.base.iter_mut() {
            *p = rng.next_u64() as u32;
        }
        self.rng = Rng::new(0x0D2A);
        // The combo's bank: silent, its own phases.
        self.bank.clear();
        self.bank.new_phases(0xC0B0);
        self.retune();
        self.update_bars();
        self.update_out();
        self.update_tone();
        self.update_drive();
        self.update_perc();
        self.update_scanner();
        self.update_mics();
        self.update_speed();
        self.out_l.snap(self.out_l.target);
        self.out_r.snap(self.out_r.target);
        self.tone_lo.snap(self.tone_lo.target);
        self.tone_hi.snap(self.tone_hi.target);
        self.vib = [0.0; VIB_LEN];
        self.vib_w = 0;
        self.vib_phase = 0.0;
        self.vib_lp = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.tone_lp = 0.0;
        self.clear_rotary();
        self.rot_w = 0;
        let on = if self.rotary_on() { 1.0 } else { 0.0 };
        self.rot_mix.snap(on);
        // The rotors start at their set speed, at rest positions apart.
        self.horn = Rotor::new(0.3);
        self.drum = Rotor::new(1.9);
        self.update_speed();
        self.horn.speed = self.horn.target;
        self.drum.speed = self.drum.target;
        let r = self.rate as f32;
        self.horn.set_turns(r);
        self.drum.set_turns(r);
        self.horn_a = [self.horn_bright; 2];
        self.bus = [0.0; CTRL as usize];
        self.ring = 0;
        let to = if self.combo_on() { 1.0 } else { 0.0 };
        self.mix = to;
        self.mix_end = to;
        self.dmix = 0.0;
        for b in [
            &mut self.flute_lp,
            &mut self.reed_hp,
            &mut self.reed_peak,
            &mut self.reed_lp,
        ] {
            b.clear();
        }
        self.cvib = [0.0; CVIB_LEN];
        self.cvib_w = 0;
        self.cvib_ph = 0;
        self.update_combo_vib();
        self.cvib_amp.snap(self.cvib_amp.target);
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        if ctx.rate != self.rate {
            self.prepare(ctx.rate, left.len());
        }
        let len = left.len().min(right.len());
        self.clock = ctx.frame;
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            while i < events.len() && events[i].offset as usize <= at {
                let ev = events[i].ev;
                self.apply(&ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            while at < next {
                let abs = ctx.frame + at as u64;
                if abs.is_multiple_of(CTRL) {
                    self.control(abs);
                }
                let end = next.min(at + (CTRL - abs % CTRL) as usize);
                self.render(at, end, left, right);
                at = end;
            }
        }
        for e in &events[i..] {
            let ev = e.ev;
            self.apply(&ev);
        }
    }

    fn tail_frames(&self) -> u32 {
        // The slowest contact opening (13 ms), the delay lines, and the
        // DC blocker (16 ms time constant) settling.
        (0.2 * self.rate as f32) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
        self.retune();
    }
}
