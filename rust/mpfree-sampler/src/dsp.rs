//! Small DSP building blocks for the sampler voice: interpolation, the
//! AHDSR envelope, the voice's filter ([`VoiceFilter`]) and gain tables.
//! Everything here is allocation-free and safe to call from the audio
//! thread.

use std::f32::consts::{FRAC_PI_2, PI};

use mpfree_filter::ladder::{self, LadderCore};
use mpfree_filter::multi::Multi;
use mpfree_filter::svf::SvfMode;

/// 4-point, 3rd-order Hermite interpolation between `x1` and `x2`
/// (`t` in 0..1), with `x0` and `x3` as the outer neighbours.
#[inline]
pub fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

/// A sample as a sound holds it ([`crate::model::Pcm`]), read as a float:
/// 32-bit floats as they are, 16-bit integers over 32768, which gives
/// each the float a 16-bit WAV decodes to, exactly.
pub trait Sample: Copy + Send + Sync + 'static {
    /// What [`Sample::raw`] is multiplied by to give [`Sample::f`]: a
    /// power of two, so a sum or product of raw values scaled once at the
    /// end is exactly the sum or product of the scaled ones (short of the
    /// denormals). Hot loops scale once instead of once a sample.
    const SCALE: f32;
    /// The sample unscaled: for 16 bits, the integer as a float.
    fn raw(self) -> f32;
    fn f(self) -> f32;
}

impl Sample for f32 {
    const SCALE: f32 = 1.0;
    #[inline(always)]
    fn raw(self) -> f32 {
        self
    }
    #[inline(always)]
    fn f(self) -> f32 {
        self
    }
}

impl Sample for i16 {
    const SCALE: f32 = 1.0 / 32_768.0;
    #[inline(always)]
    fn raw(self) -> f32 {
        f32::from(self)
    }
    #[inline(always)]
    fn f(self) -> f32 {
        f32::from(self) * Self::SCALE
    }
}

/// `x` as a 16-bit sample, if it is one exactly: `k / 32768` for a whole
/// `k`, sign of zero included (-0.0 isn't).
#[inline]
pub fn as_i16(x: f32) -> Option<i16> {
    let y = x * 32_768.0;
    if y != y.trunc() || !(-32_768.0..=32_767.0).contains(&y) {
        return None;
    }
    let k = y as i16;
    (k.f().to_bits() == x.to_bits()).then_some(k)
}

/// Linear gain from decibels.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -120.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Equal-power pan: `pan` from -1 (left) to +1 (right), returns (left, right) gains.
/// At centre both are about 0.707 (-3 dB).
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * PI;
    (p.cos(), p.sin())
}

/// Equal-power crossfade weights for `x` in 0..1: (fading out, fading in).
pub fn equal_power(x: f32) -> (f32, f32) {
    let a = x.clamp(0.0, 1.0) * FRAC_PI_2;
    (a.cos(), a.sin())
}

/// Envelope times in milliseconds and a sustain level (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ahdsr {
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub decay_ms: f32,
    pub sustain: f32,
    pub release_ms: f32,
}

impl Ahdsr {
    /// Open: instant attack, full sustain, a short release so nothing clicks.
    pub const OPEN: Ahdsr = Ahdsr {
        attack_ms: 0.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 5.0,
    };
}

impl Default for Ahdsr {
    fn default() -> Self {
        Ahdsr::OPEN
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Done,
}

/// Envelope level below which a release counts as finished (about -80 dB).
const FLOOR: f32 = 1.0e-4;

/// A level far below hearing (-400 dB) but well above the denormals
/// (under 1.2e-38): state that decays toward zero is set to zero once it
/// gets here, as the effects do (`mpfree-fx`'s `flush`).
pub const TINY: f32 = 1.0e-20;

/// A multimode filter's channel is zeroed once both its states are under
/// this, 300 dB down ([`VoiceFilter::flush`]): over the slow tail the
/// shared stage's own flush leaves at a low cutoff
/// ([`mpfree_filter::svf::Svf::flush_below`]), so a filter ringing out on
/// silence reaches exact zeros as soon as its sound is gone.
pub const QUIET: f32 = 1.0e-15;

/// A running AHDSR. Attack is linear; decay and release are exponential
/// (one multiply per sample), reaching about -80 dB of their distance in
/// the given time.
#[derive(Clone, Copy, Debug)]
pub struct Envelope {
    stage: Stage,
    level: f32,
    attack_step: f32,
    hold_left: u32,
    decay_coef: f32,
    sustain: f32,
    release_coef: f32,
}

fn exp_coef(ms: f32, rate: f32) -> f32 {
    let n = (ms.max(0.0) * 0.001 * rate).max(1.0);
    // After n samples the remaining distance is FLOOR of the start.
    FLOOR.powf(1.0 / n)
}

impl Envelope {
    pub fn idle() -> Envelope {
        Envelope {
            stage: Stage::Done,
            level: 0.0,
            attack_step: 1.0,
            hold_left: 0,
            decay_coef: 0.0,
            sustain: 1.0,
            release_coef: 0.0,
        }
    }

    pub fn start(p: &Ahdsr, rate: f32) -> Envelope {
        let attack_n = (p.attack_ms.max(0.0) * 0.001 * rate).max(0.0);
        let (stage, level, attack_step) = if attack_n < 1.0 {
            (Stage::Hold, 1.0, 1.0)
        } else {
            (Stage::Attack, 0.0, 1.0 / attack_n)
        };
        Envelope {
            stage,
            level,
            attack_step,
            hold_left: (p.hold_ms.max(0.0) * 0.001 * rate) as u32,
            decay_coef: exp_coef(p.decay_ms, rate),
            sustain: p.sustain.clamp(0.0, 1.0),
            release_coef: exp_coef(p.release_ms, rate),
        }
    }

    /// [`Envelope::start`], its attack rising from `level` (0 to 1) instead
    /// of from zero, in what is left of its time: a mono pad's next note
    /// with Continue (the filter gap plan's F3). From 0 it is `start`.
    pub fn start_from(p: &Ahdsr, rate: f32, level: f32) -> Envelope {
        let mut e = Envelope::start(p, rate);
        if e.stage == Stage::Attack && level > 0.0 {
            e.level = level.min(1.0);
        }
        e
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Done {
            self.stage = Stage::Release;
        }
    }

    pub fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }
    /// Holding a level that [`Envelope::next`] leaves as it is until a
    /// release: its sustain, above zero. Its level is then the gain every
    /// frame gets.
    pub fn is_steady(&self) -> bool {
        self.stage == Stage::Sustain && self.sustain > 0.0
    }
    pub fn level(&self) -> f32 {
        self.level
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.level += self.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Hold;
                }
            }
            Stage::Hold => {
                if self.hold_left == 0 {
                    self.stage = Stage::Decay;
                } else {
                    self.hold_left -= 1;
                }
            }
            Stage::Decay => {
                self.level = self.sustain + (self.level - self.sustain) * self.decay_coef;
                if (self.level - self.sustain).abs() < FLOOR {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                if self.sustain <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Done;
                }
            }
            Stage::Release => {
                self.level *= self.release_coef;
                if self.level < FLOOR {
                    self.level = 0.0;
                    self.stage = Stage::Done;
                }
            }
            Stage::Done => self.level = 0.0,
        }
        self.level
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Off,
    LowPass,
    HighPass,
    BandPass,
    /// Everything but a band around the cutoff (sampler R5.6).
    Notch,
    /// A 4-pole ladder low-pass with a resonance that sings and a Drive
    /// into it ([`Ladder`], sampler R5.6).
    Ladder,
    /// Two formant band-passes: the cutoff moves the vowel from "oo"
    /// through "oh", "ah" and "eh" to "ee" ([`vowel_formants`], sampler
    /// R5.6).
    Vowel,
    /// A comb with feedback: the sound plus itself a short delay later,
    /// over and over, so it rings at the cutoff's pitch and its overtones.
    /// Resonance is the feedback ([`Comb`], sampler R5.6).
    Comb,
    /// A boost around the cutoff, the rest left as it is (the MPC's
    /// band-boost): Resonance raises it from nothing to 35 dB, ringing
    /// as a 2-pole filter's resonance does (the filter gap plan's F4).
    Peak,
}

/// A filter setting. `poles` is the slope, 6 dB an octave each: a
/// low-pass or high-pass 1, 2, 3, 4, 6 or 8, a band-pass 2, 4, 6 or 8, a
/// notch 2 or 4 (the filter gap plan's F4; [`mpfree_filter::multi::slope`]
/// takes any other count to one of these). The Ladder is 4, the Vowel,
/// Comb and Peak 2. `env_octaves` is how far the filter envelope moves the
/// cutoff at full level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    pub kind: FilterKind,
    pub poles: u8,
    pub cutoff_hz: f32,
    /// 0 = no resonance, 1 = close to self-oscillation.
    pub resonance: f32,
    pub env_octaves: f32,
    /// How hard the ladder is driven, in dB (0 to [`MAX_DRIVE_DB`]); the
    /// other kinds leave it be (sampler R5.6).
    pub drive_db: f32,
}

/// The ladder's strongest drive, in dB.
pub const MAX_DRIVE_DB: f32 = 24.0;

/// The filter envelope's deepest move of the cutoff either way, in
/// octaves ([`Filter::env_octaves`]).
pub const MAX_ENV_OCTAVES: f32 = 4.0;

/// The Peak's boost at its cutoff for Resonance `res` (0 to 1), in dB: 0
/// at 0, 35 at full ([`FilterKind::Peak`]).
pub fn peak_boost_db(res: f32) -> f32 {
    20.0 * (mpfree_filter::svf::res_to_q(res) / 0.707).log10()
}

impl Filter {
    /// The slope it plays, in poles: a multimode type's
    /// ([`mpfree_filter::multi::slope`]), the Ladder's 4, any other 2.
    pub fn slope(&self) -> u8 {
        match self.kind {
            FilterKind::LowPass
            | FilterKind::HighPass
            | FilterKind::BandPass
            | FilterKind::Notch => mpfree_filter::multi::slope(svf_mode(self.kind), self.poles),
            FilterKind::Ladder => 4,
            _ => 2,
        }
    }
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            kind: FilterKind::Off,
            poles: 2,
            cutoff_hz: 20_000.0,
            resonance: 0.0,
            env_octaves: 0.0,
            drive_db: 0.0,
        }
    }
}

/// A smooth clip that is `tanh` to within 2.5 % (a rational fit, cheap
/// enough for every frame of every voice): straight through 0 with a slope
/// of 1, flat at ±1 from ±3 (where its slope has come down to 0, so the
/// join has no corner).
#[inline(always)]
pub fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// The Drive's curve, in units of [`DRIVE_KNEE`] (`u = x / knee`): the
/// quintic smoothstep `(15u - 10u³ + 3u⁵) / 8`, flat at ±1 from ±1 with its
/// first two derivatives 0 there, so its harmonics fall away fast. Slope 1
/// through 0 in `x`.
#[inline(always)]
fn drive_curve_u(u: f32) -> f32 {
    let u = u.clamp(-1.0, 1.0);
    let u2 = u * u;
    u * (15.0 + u2 * (3.0 * u2 - 10.0)) * 0.125
}

/// Where the Drive's curve flattens: its slope at 0 is `15 / 8` in `u`.
const DRIVE_KNEE: f32 = 15.0 / 8.0;

/// The Drive's curve at `x`.
#[inline(always)]
fn drive_curve(x: f32) -> f32 {
    drive_curve_u(x / DRIVE_KNEE)
}

/// The curve's average from `w` to `u` (first-order antiderivative
/// antialiasing): its antiderivative `(15u² - 5u⁴ + u⁶) / 16` (then `|u|`
/// plus a constant from ±1), divided by the step. Between ±1 the quotient
/// is worked out as a polynomial in `u` and `w`, exact at any step with no
/// division; across ±1, the quotient itself, or the curve at the midpoint
/// where the step is too small for it.
#[inline(always)]
fn drive_average_u(u: f32, w: f32) -> f32 {
    if u.abs() <= 1.0 && w.abs() <= 1.0 {
        let (s1, p2, uw) = (u + w, u * u + w * w, u * w);
        s1 * (15.0 - 5.0 * p2 + p2 * p2 - uw * uw) * 0.0625
    } else if u >= 1.0 && w >= 1.0 {
        1.0
    } else if u <= -1.0 && w <= -1.0 {
        -1.0
    } else if (u - w).abs() < 1.0e-4 {
        drive_curve_u(0.5 * (u + w))
    } else {
        let big = |u: f32| {
            let a = u.abs();
            if a >= 1.0 {
                a - 1.0 + 11.0 / 16.0
            } else {
                let u2 = u * u;
                u2 * (15.0 + u2 * (u2 - 5.0)) * 0.0625
            }
        };
        (big(u) - big(w)) / (u - w)
    }
}

/// The level a driven sound keeps: a sine peaking here (-12 dBFS) into
/// the drive comes out at the same loudness (RMS), whatever the Drive.
const DRIVE_REF: f32 = 0.25;

/// The scale after the curve at gain `drive` that keeps a sine at
/// [`DRIVE_REF`] at its RMS (a quarter cycle at 32 points; on a change of
/// Drive only).
fn drive_norm(drive: f32) -> f32 {
    const N: usize = 32;
    let ms = (0..N)
        .map(|i| {
            let s = ((i as f32 + 0.5) / N as f32 * FRAC_PI_2).sin();
            drive_curve(drive * DRIVE_REF * s).powi(2)
        })
        .sum::<f32>()
        / N as f32;
    (0.5 * DRIVE_REF * DRIVE_REF / ms).sqrt()
}

/// The half-band's all-pass coefficients, the even sections' and the odd
/// ones' (6 sections, transition 0.2 to 0.3 of the higher rate:
/// Valenzuela and Constantinides' elliptic design, as mpfree-synth's
/// Squelch and mpfree-fx's oversampler have it).
const HB_EVEN: [f32; 3] = [0.060_297_39, 0.412_590_7, 0.772_715_6];
const HB_ODD: [f32; 3] = [0.215_971_44, 0.604_358_6, 0.923_886_1];

/// One polyphase all-pass section, `y = a (x - y1) + x1`, state `[x1, y1]`.
#[inline(always)]
fn ap(a: f32, s: &mut [f32; 2], x: f32) -> f32 {
    let y = a * (x - s[1]) + s[0];
    *s = [x, y];
    y
}

/// The Ladder's twice-the-rate path for one channel: an IIR half-band up,
/// the Drive's curve when Drive is on ([`drive_curve`]; each sample the
/// curve's average over the straight line from the last, first-order
/// antiderivative antialiasing, Parker, Zavalishin and Le Bihan, DAFx
/// 2016, as mpfree-fx's Drive does it), the ladder twice, and the
/// half-band down. A 5 kHz sine at 0 dBFS driven 12 dB aliases about 84
/// dB under the tone through the curve; the bare curve at the base rate
/// gives -18 dB. The average is half a step late and a little dull at the
/// very top (about -2.4 dB at 20 kHz), under a low-pass anyway.
#[derive(Clone, Copy, Debug, Default)]
struct LadderOs {
    /// The up-sampler's sections (even then odd) and the down-sampler's.
    up: [[f32; 2]; 6],
    down: [[f32; 2]; 6],
    /// The curve's last input, in units of [`DRIVE_KNEE`].
    w: f32,
    /// The curve has run since Drive came on: it starts from its first
    /// sample as if that had been there all along.
    live: bool,
}

impl LadderOs {
    /// The curve's average from the last input to `x`.
    #[inline(always)]
    fn curve(&mut self, x: f32) -> f32 {
        let u = x * (1.0 / DRIVE_KNEE);
        if !self.live {
            self.w = u;
            self.live = true;
        }
        let y = drive_average_u(u, self.w);
        self.w = u;
        y
    }

    /// `x` up to twice the rate, through the curve if `driven` (`x`
    /// already times the drive's gain), each of the two samples through
    /// `then` (the ladder), the earlier first, and back down.
    #[inline(always)]
    fn run(&mut self, x: f32, driven: bool, mut then: impl FnMut(f32) -> f32) -> f32 {
        let [u0, u1, u2, u3, u4, u5] = &mut self.up;
        let e = ap(HB_EVEN[2], u2, ap(HB_EVEN[1], u1, ap(HB_EVEN[0], u0, x)));
        let o = ap(HB_ODD[2], u5, ap(HB_ODD[1], u4, ap(HB_ODD[0], u3, x)));
        let (a, b) = if driven {
            (self.curve(e), self.curve(o))
        } else {
            (e, o)
        };
        let a = then(a);
        let b = then(b);
        // Down: the later sample through the even sections, the earlier
        // through the odd ones.
        let [d0, d1, d2, d3, d4, d5] = &mut self.down;
        let e = ap(HB_EVEN[2], d2, ap(HB_EVEN[1], d1, ap(HB_EVEN[0], d0, b)));
        let o = ap(HB_ODD[2], d5, ap(HB_ODD[1], d4, ap(HB_ODD[0], d3, a)));
        0.5 * (e + o)
    }

    fn flush(&mut self) {
        for x in self.up.iter_mut().chain(self.down.iter_mut()).flatten() {
            if x.abs() < TINY {
                *x = 0.0;
            }
        }
    }
}

/// A 4-pole transistor ladder low-pass, Moog style (sampler R5.6, rebuilt
/// in the filter gap plan's step F6): four one-pole stages in a loop with
/// a curve in every stage, as the circuit has ([`LadderCore`], its design
/// and measures in mpfree-filter's `ladder`), at twice the rate between
/// IIR half-bands. Its unit is full scale: a full-scale sound bends a
/// little at the input pair, as a hot signal into a real ladder does, and
/// at full Resonance it sings at the level its stages allow (with the
/// make-up, about 3 dB louder than the Ladder before F6). Drive goes
/// first, its curve at twice the rate too ([`LadderOs`]), and its scale
/// before the ladder (after it, the ladder heard the curve's full level
/// and aliased about 12 dB more), so a driven sound reaches the ladder
/// about as loud as a full-scale one and keeps its loudness. A singing
/// ladder falls flat as it gets louder (20 cents at full Resonance); the
/// cutoff is raised to make up for it ([`mpfree_filter::ladder::tuned`])
/// unless [`Ladder::authentic`] is set. Both channels; one per voice.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ladder {
    core: [LadderCore; 2],
    /// One stage's prewarped gain at twice the rate, and the feedback (0
    /// to [`LADDER_K`]; past 4 it sings) and the gain that gives back part
    /// of the bass a resonant ladder loses.
    g: f32,
    k: f32,
    makeup: f32,
    /// A glide to a new setting ([`Ladder::set`]): where `g` and `k` are
    /// heading, each frame's step, and the frames left.
    to: [f32; 2],
    step: [f32; 2],
    left: u32,
    /// Set at least once (the first setting lands at once).
    set: bool,
    /// Drive's gain into the curve and its scale after the ladder; gain 0:
    /// none. The Drive (dB) they are for.
    drive: f32,
    norm: f32,
    drive_db: f32,
    os: [LadderOs; 2],
    /// The DC blocker after it, each channel: its last input and output.
    dc: [[f32; 2]; 2],
    /// Sing where the circuit does, flat by up to 20 cents at full
    /// Resonance, instead of at the cutoff.
    pub authentic: bool,
}

/// The ladder's feedback at Resonance 1: past 4, where it sings.
pub const LADDER_K: f32 = 4.2;

/// The pole of the DC blocker after the Ladder: about 3 Hz at 44.1 kHz
/// (`1 - 2π × 3 / 44,100`; at 48 kHz, 2.8 Hz).
const LADDER_DC_R: f32 = 0.999_573;

/// The ladder's make-up gain at feedback `k`: half the bass it loses
/// (a ladder's gain at 0 Hz is `1 / (1 + k)`).
pub fn ladder_makeup(k: f32) -> f32 {
    1.0 + 0.5 * k
}

impl Ladder {
    /// Resonance 0 to 1 (from about 0.95 it sings), Drive in dB. The
    /// cutoff and resonance glide there over `frames` frames (the first
    /// setting, or 0 frames: at once); Drive changes at once.
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, drive_db: f32, rate: f32, frames: u32) {
        let k = LADDER_K * resonance.clamp(0.0, 1.0);
        let fc = if self.authentic {
            cutoff_hz
        } else {
            cutoff_hz * ladder::tuned(k)
        };
        let g = ladder::stage_gain(fc.clamp(10.0, rate * 0.45), 2.0 * rate);
        let drive_db = drive_db.clamp(0.0, MAX_DRIVE_DB);
        if drive_db > 0.0 {
            if drive_db != self.drive_db || self.drive == 0.0 {
                self.drive = db_to_gain(drive_db);
                self.norm = drive_norm(self.drive);
                self.drive_db = drive_db;
            }
        } else {
            (self.drive, self.norm, self.drive_db) = (0.0, 1.0, 0.0);
            // Off: the curve starts afresh when it next comes on.
            self.os[0].live = false;
            self.os[1].live = false;
        }
        if !self.set || frames == 0 {
            (self.g, self.k) = (g, k);
            self.left = 0;
        } else {
            let n = frames as f32;
            self.to = [g, k];
            self.step = [(g - self.g) / n, (k - self.k) / n];
            self.left = frames;
        }
        self.makeup = ladder_makeup(self.k);
        self.set = true;
    }

    /// One frame of the glide; the last lands on the setting itself.
    #[inline(always)]
    fn tick(&mut self) {
        if self.left == 0 {
            return;
        }
        self.left -= 1;
        if self.left == 0 {
            [self.g, self.k] = self.to;
        } else {
            self.g += self.step[0];
            self.k += self.step[1];
        }
        self.makeup = ladder_makeup(self.k);
    }

    /// Zeros a state too small to hear (see [`Svf::flush`]).
    #[inline]
    pub fn flush(&mut self) {
        for ch in 0..2 {
            self.core[ch].flush();
            self.os[ch].flush();
            for x in &mut self.dc[ch] {
                if x.abs() < TINY {
                    *x = 0.0;
                }
            }
        }
    }

    /// One frame, both channels.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.tick();
        (self.channel(0, l), self.channel(1, r))
    }

    #[inline(always)]
    fn channel(&mut self, ch: usize, x: f32) -> f32 {
        let (g, k, norm) = (self.g, self.k, self.norm);
        let driven = self.drive > 0.0;
        let gain = if driven { self.drive } else { 1.0 };
        let core = &mut self.core[ch];
        let y = self.os[ch].run(x * gain, driven, |u| core.step(u * norm, g, k));
        // A loud, lopsided wave through the bending stages leaves a little
        // DC (a full-scale sawtooth about -40 dB, where the exact circuit
        // gives -83 at Resonance 0 and -47 at 0.5), as the Drive's curve
        // does: a one-pole high-pass at 3 Hz takes it out, as a real
        // ladder's output coupling does (-0.1 dB at 20 Hz).
        let [x1, y1] = self.dc[ch];
        let out = y - x1 + LADDER_DC_R * y1;
        self.dc[ch] = [y, out];
        out * self.makeup
    }
}

/// The comb's lowest pitch: its delay line holds one cycle of it.
pub const COMB_MIN_HZ: f32 = 20.0;

/// The comb's feedback at Resonance 1: it rings for about a second at
/// 100 Hz (-60 dB in `6.9 / (-ln 0.97)` cycles, about 230), and never runs
/// away.
pub const COMB_FEEDBACK: f32 = 0.97;

/// The comb's gain after the loop at `resonance` (0 to 1): `sqrt(1 - g²)`
/// for feedback `g`, so noise or a full sound comes out at its own
/// loudness (RMS) at any feedback; a tone right on a peak comes out up to
/// `sqrt((1 + g) / (1 - g))` louder (18 dB at full feedback).
pub fn comb_gain(resonance: f32) -> f32 {
    let g = COMB_FEEDBACK * resonance.clamp(0.0, 1.0);
    (1.0 - g * g).sqrt()
}

/// A Comb filter's gain at `hz` (dB): the loop `1 / (1 - g e^(-jwD))` at
/// the cutoff's delay `D`, times [`comb_gain`]. For the Play page's
/// picture.
pub fn comb_db(cutoff_hz: f32, resonance: f32, hz: f32) -> f32 {
    let g = COMB_FEEDBACK * resonance.clamp(0.0, 1.0);
    let phase = 2.0 * PI * hz / cutoff_hz.max(COMB_MIN_HZ);
    let (re, im) = (1.0 - g * phase.cos(), g * phase.sin());
    let k = comb_gain(resonance);
    // The DC blocker, as an analog one-pole high-pass.
    let x = hz / COMB_DC_HZ;
    let hp = x * x / (1.0 + x * x);
    10.0 * (hp * k * k / (re * re + im * im)).max(1e-12).log10()
}

/// A feedback comb filter, both channels (sampler R5.6): `y = x + g y(t -
/// D)` with `D` one cycle of the cutoff, read between samples with
/// [`hermite`], then [`comb_gain`] and a DC blocker ([`COMB_DC_HZ`]). The delay glides to a new cutoff over
/// the frames until the next [`Comb::set`], so an envelope or a knob
/// sweeping it doesn't step. Its line is made with the voice
/// ([`Comb::new`], before the sound starts) and only reused after:
/// [`Comb::reset`] forgets what it held without clearing it (a read past
/// what was written since gives silence). One per voice.
#[derive(Clone, Debug, Default)]
pub struct Comb {
    /// The line, a power of two long: each frame's two channels.
    line: Box<[[f32; 2]]>,
    /// The next frame's place in the line.
    at: usize,
    /// Frames written since the reset, up to the line's length.
    written: usize,
    /// The delay (frames) now, where it glides to and by how much a frame,
    /// and the frames left of the glide. 0: not set since the reset.
    delay: f32,
    delay_to: f32,
    delay_step: f32,
    glide_left: u32,
    feedback: f32,
    gain: f32,
    /// The DC blocker after it: its pole, and each channel's last input
    /// and output.
    pole: f32,
    dc: [[f32; 2]; 2],
    /// The loudest output since `cycle_left` last ran out, the loudest
    /// over the last whole cycle before that, and the frames left of this
    /// one.
    peak_now: f32,
    peak_cycle: f32,
    cycle_left: u32,
}

/// Under this (-80 dBFS) for a whole cycle, the comb's ringing is over
/// ([`Comb::rings`]).
pub const COMB_QUIET: f32 = 1.0e-4;

/// Where the comb's DC blocker turns (Hz): its loop's lowest peak is at
/// 0 Hz, which would lift rumble and any offset 18 dB.
pub const COMB_DC_HZ: f32 = 10.0;

impl Comb {
    /// A comb whose line holds one cycle of [`COMB_MIN_HZ`] at `rate`.
    /// Allocates.
    pub fn new(rate: f32) -> Comb {
        let n = ((rate / COMB_MIN_HZ).ceil() as usize + 8).next_power_of_two();
        Comb {
            line: vec![[0.0; 2]; n].into_boxed_slice(),
            gain: 1.0,
            pole: 1.0 - 2.0 * PI * COMB_DC_HZ / rate,
            ..Comb::default()
        }
    }

    /// For a new note: the line holds nothing (without touching it).
    pub fn reset(&mut self) {
        self.written = 0;
        self.delay = 0.0;
        self.glide_left = 0;
        self.dc = [[0.0; 2]; 2];
        self.peak_now = 0.0;
        self.peak_cycle = 0.0;
        self.cycle_left = 0;
    }

    /// Whether it still rings: something was written since the reset, and
    /// its output reached [`COMB_QUIET`] in the last whole cycle or the
    /// one under way.
    #[inline]
    pub fn rings(&self) -> bool {
        self.written > 0 && self.peak_cycle.max(self.peak_now) >= COMB_QUIET
    }

    /// Pitch `cutoff_hz` (its delay one cycle of it), feedback from
    /// `resonance` (0 to 1), gliding there over the next `frames` frames
    /// (at once after a reset).
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, rate: f32, frames: u32) {
        let longest = self.line.len().saturating_sub(4).max(2) as f32;
        let d = (rate / cutoff_hz.max(COMB_MIN_HZ)).clamp(2.0, longest);
        self.feedback = COMB_FEEDBACK * resonance.clamp(0.0, 1.0);
        self.gain = comb_gain(resonance);
        self.delay_to = d;
        if self.delay == 0.0 || frames == 0 {
            self.delay = d;
            self.glide_left = 0;
        } else {
            self.delay_step = (d - self.delay) / frames as f32;
            self.glide_left = frames;
        }
    }

    /// The frame `k` frames back (1: the last one written), or silence
    /// past what was written since the reset.
    #[inline(always)]
    fn back(&self, k: usize) -> [f32; 2] {
        if k > self.written {
            return [0.0; 2];
        }
        self.line[self.at.wrapping_sub(k) & (self.line.len() - 1)]
    }

    /// One frame of both channels.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let n = self.line.len();
        if n == 0 {
            return (l, r);
        }
        if self.glide_left > 0 {
            self.glide_left -= 1;
            self.delay = if self.glide_left == 0 {
                self.delay_to
            } else {
                self.delay + self.delay_step
            };
        }
        // `delay` frames back: between `i` and `i + 1` back.
        let i = self.delay as usize;
        let t = self.delay - i as f32;
        let (a, b, c, d) = (
            self.back(i - 1),
            self.back(i),
            self.back(i + 1),
            self.back(i + 2),
        );
        let fl = hermite(a[0], b[0], c[0], d[0], t);
        let fr = hermite(a[1], b[1], c[1], d[1], t);
        let tiny = |x: f32| if x.abs() < TINY { 0.0 } else { x };
        let y = [tiny(l + self.feedback * fl), tiny(r + self.feedback * fr)];
        self.line[self.at] = y;
        self.at = (self.at + 1) & (n - 1);
        if self.written < n {
            self.written += 1;
        }
        let mut out = [0.0; 2];
        for (c, o) in out.iter_mut().enumerate() {
            let [x1, y1] = self.dc[c];
            let x = y[c] * self.gain;
            *o = tiny(x - x1 + self.pole * y1);
            self.dc[c] = [x, *o];
        }
        self.peak_now = self.peak_now.max(out[0].abs()).max(out[1].abs());
        if self.cycle_left == 0 {
            self.peak_cycle = self.peak_now;
            self.peak_now = 0.0;
            self.cycle_left = self.delay as u32 + 1;
        }
        self.cycle_left -= 1;
        (out[0], out[1])
    }
}

/// The two formants (Hz) of the vowel at `cutoff_hz` (sampler R5.6): 200 Hz
/// and below is "oo", then "oh", "ah", "eh" and, from 5 kHz, "ee", each
/// gliding into the next. Male voice formants (Peterson and Barney 1952).
pub fn vowel_formants(cutoff_hz: f32) -> (f32, f32) {
    const VOWELS: [(f32, f32); 5] = [
        (300.0, 870.0),  // oo
        (570.0, 840.0),  // oh
        (730.0, 1090.0), // ah
        (530.0, 1840.0), // eh
        (270.0, 2290.0), // ee
    ];
    let t = ((cutoff_hz.max(1.0) / 200.0).ln() / 25.0f32.ln() * 4.0).clamp(0.0, 4.0);
    let i = (t.floor() as usize).min(3);
    let u = t - i as f32;
    let (a, b) = (VOWELS[i], VOWELS[i + 1]);
    // Glide in pitch, not in Hz.
    let mix = |x: f32, y: f32| x * (y / x).powf(u);
    (mix(a.0, b.0), mix(a.1, b.1))
}

/// The vowel's name at `cutoff_hz`, for the screen.
pub fn vowel_name(cutoff_hz: f32) -> &'static str {
    let t = ((cutoff_hz.max(1.0) / 200.0).ln() / 25.0f32.ln() * 4.0).clamp(0.0, 4.0);
    ["oo", "oh", "ah", "eh", "ee"][t.round() as usize]
}

/// The formants' `k` (one over their Q) at the filter's `resonance` (0 to
/// 1): a Q of 5 at 0 to 12.5 at 1, the spread of a voice's formants.
fn vowel_k(resonance: f32) -> f32 {
    0.2 * 0.4f32.powf(resonance.clamp(0.0, 1.0))
}

/// The resonance each formant's [`Svf`] is set to for the filter's
/// `resonance` (0 to 1).
pub fn vowel_resonance(resonance: f32) -> f32 {
    (2.0 - vowel_k(resonance)) / 1.95
}

/// The second formant's level against the first's.
pub const FORMANT_2: f32 = 0.6;

/// The gain after the formants at `resonance`: the narrower they are, the
/// more of a full sound they take away, so a saw at 110 Hz comes out
/// about 4 dB under its level at any resonance, and a sine right on the
/// first formant 8 to 12 dB over it.
pub fn vowel_gain(resonance: f32) -> f32 {
    1.1 / vowel_k(resonance).sqrt()
}

/// A Vowel filter's gain at `hz` (dB): its two formants at `cutoff_hz`,
/// in parallel as [`Svf`] band-passes peaking at 1, the analog
/// prototype the engine follows. For the Play page's picture.
pub fn vowel_db(cutoff_hz: f32, resonance: f32, hz: f32) -> f32 {
    let (f1, f2) = vowel_formants(cutoff_hz);
    let k = vowel_k(resonance);
    // k jx / (1 - x² + k jx), as (re, im).
    let band = |f: f32| {
        let x = hz / f;
        let (re, im) = (1.0 - x * x, k * x);
        let d = re * re + im * im;
        (k * x * im / d, k * x * re / d)
    };
    let (a, b) = (band(f1), band(f2));
    let g = vowel_gain(resonance);
    let (re, im) = (g * (a.0 + FORMANT_2 * b.0), g * (a.1 + FORMANT_2 * b.1));
    10.0 * (re * re + im * im).max(1e-12).log10()
}

/// A Ladder's small-signal gain at `hz` (dB), no Drive: four one-pole
/// stages at `cutoff_hz` in a loop with feedback `k`, then the make-up,
/// `m / ((1 + s)^4 + k)`. For the Play page's picture.
pub fn ladder_db(cutoff_hz: f32, resonance: f32, hz: f32) -> f32 {
    let k = LADDER_K * resonance.clamp(0.0, 1.0);
    let x = hz / (cutoff_hz * ladder::tuned(k)).max(1.0);
    // (1 + jx)^4 = 1 - 6x² + x⁴ + j(4x - 4x³).
    let x2 = x * x;
    let (re, im) = (1.0 - 6.0 * x2 + x2 * x2 + k, 4.0 * x - 4.0 * x * x2);
    let m = ladder_makeup(k);
    10.0 * (m * m / (re * re + im * im)).max(1e-12).log10()
}

/// One 2-pole topology-preserving state-variable filter stage per channel
/// (after Andrew Simper's published "SVF linear trap" design), with the
/// pad's old resonance (Q 0.5 to 20): the Vowel's formants' arithmetic
/// ([`Formants`] runs four side by side). The low-pass, high-pass,
/// band-pass and notch run the Filter effect's multimode filter instead
/// ([`VoiceFilter`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: [f32; 2],
    ic2: [f32; 2],
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, rate: f32) {
        let fc = cutoff_hz.clamp(10.0, rate * 0.49);
        let g = (PI * fc / rate).tan();
        // resonance 0..1 maps to Q 0.5..~20 via k = 1/Q.
        self.k = 2.0 - 1.95 * resonance.clamp(0.0, 1.0);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Sets a state too small to hear (under [`TINY`]) to zero, so a
    /// filter left ringing on silence decays to exact zeros instead of
    /// running on denormals, which cost the CPU many times a normal
    /// number's arithmetic (about 40 times on the workstation, for a
    /// 4-pole low-pass ringing out). Callers run it every few frames.
    #[inline]
    pub fn flush(&mut self) {
        let f = |x: f32| if x.abs() < TINY { 0.0 } else { x };
        self.ic1 = self.ic1.map(f);
        self.ic2 = self.ic2.map(f);
    }

    #[inline]
    pub fn process(&mut self, ch: usize, x: f32, kind: FilterKind) -> f32 {
        let v3 = x - self.ic2[ch];
        let v1 = self.a1 * self.ic1[ch] + self.a2 * v3;
        let v2 = self.ic2[ch] + self.a2 * self.ic1[ch] + self.a3 * v3;
        self.ic1[ch] = 2.0 * v1 - self.ic1[ch];
        self.ic2[ch] = 2.0 * v2 - self.ic2[ch];
        match kind {
            FilterKind::Off => x,
            FilterKind::LowPass => v2,
            FilterKind::BandPass => v1,
            FilterKind::HighPass => x - self.k * v1 - v2,
            FilterKind::Notch => x - self.k * v1,
            // A formant: the band-pass, its peak at unity.
            FilterKind::Vowel => self.k * v1,
            // The ladder, the comb and the peak run on their own
            // ([`Ladder`], [`Comb`], [`Multi`]).
            FilterKind::Ladder | FilterKind::Comb | FilterKind::Peak => v2,
        }
    }
}

/// The Vowel's two formants on both channels: four [`Svf`] band-passes
/// side by side, a lane each (the first formant's left and right, then
/// the second's), with the same arithmetic, so a Vowel plays exactly as
/// two [`Svf`]s would. Kept as lanes so each line of a frame is one
/// vector operation whatever code shares a voice's loop with it: as two
/// [`Svf`]s, the compiler's packing of them depended on the rest of the
/// loop, and the 36 and 48 dB filters' code there cost 16 held Vowel
/// pads 15 % more (the filter gap plan's F4).
#[derive(Clone, Copy, Debug, Default)]
pub struct Formants {
    ic1: [f32; 4],
    ic2: [f32; 4],
    a1: [f32; 4],
    a2: [f32; 4],
    a3: [f32; 4],
    k: [f32; 4],
}

impl Formants {
    /// Formant `i` (0 or 1) at `cutoff_hz` and `resonance`, as
    /// [`Svf::set`].
    pub fn set(&mut self, i: usize, cutoff_hz: f32, resonance: f32, rate: f32) {
        let mut s = Svf::default();
        s.set(cutoff_hz, resonance, rate);
        for lane in 2 * i..2 * i + 2 {
            self.a1[lane] = s.a1;
            self.a2[lane] = s.a2;
            self.a3[lane] = s.a3;
            self.k[lane] = s.k;
        }
    }

    /// [`Svf::flush`] on every lane.
    #[inline]
    pub fn flush(&mut self) {
        let f = |x: f32| if x.abs() < TINY { 0.0 } else { x };
        self.ic1 = self.ic1.map(f);
        self.ic2 = self.ic2.map(f);
    }

    /// One frame of `l` and `r`: each formant's band-pass, its peak at
    /// unity, as `[first left, first right, second left, second right]`.
    #[inline(always)]
    pub fn process(&mut self, l: f32, r: f32) -> [f32; 4] {
        let x = [l, r, l, r];
        let mut y = [0.0; 4];
        for i in 0..4 {
            let v3 = x[i] - self.ic2[i];
            let v1 = self.a1[i] * self.ic1[i] + self.a2[i] * v3;
            let v2 = self.ic2[i] + self.a2[i] * self.ic1[i] + self.a3[i] * v3;
            self.ic1[i] = 2.0 * v1 - self.ic1[i];
            self.ic2[i] = 2.0 * v2 - self.ic2[i];
            y[i] = self.k[i] * v1;
        }
        y
    }
}

/// The [`SvfMode`] a state-variable [`FilterKind`] runs as.
#[inline]
pub fn svf_mode(kind: FilterKind) -> SvfMode {
    match kind {
        FilterKind::HighPass => SvfMode::HighPass,
        FilterKind::BandPass => SvfMode::BandPass,
        FilterKind::Notch => SvfMode::Notch,
        FilterKind::Peak => SvfMode::Bell,
        _ => SvfMode::LowPass,
    }
}

/// The rate a filter's curve is drawn at: the engine's (it runs at 44.1
/// kHz; DSP quality's plan keeps it there).
pub const DRAW_RATE: u32 = 44_100;

/// A state-variable type's curve at `hz` (dB), for drawing: its multimode
/// filter's exact response at [`DRAW_RATE`]
/// ([`mpfree_filter::multi::response_db`]), so the curve is what is heard,
/// short of a loud sound's ring that the resonance guard holds back
/// ([`Multi::guard`]).
pub fn svf_db(f: &Filter, hz: f32) -> f32 {
    mpfree_filter::multi::response_db(
        svf_mode(f.kind),
        f.poles,
        f.cutoff_hz,
        f.resonance,
        hz,
        DRAW_RATE,
    )
}

/// One voice's filter: the state of every type, set from the voice's
/// [`Filter`] and run a frame at a time. The low-pass, high-pass,
/// band-pass, notch and Peak are the Filter effect's multimode filter
/// ([`Multi`]: Butterworth at Resonance 0, so the cutoff is the -3 dB
/// point; Q up to 40; 6 to 48 dB an octave; linear, with a resonance
/// guard that damps a loud sound's ring as it passes a knee, so nothing
/// bends and no DC is made; each new cutoff glided to over the frames
/// until the next). The Ladder, Vowel and Comb are the pad's own. The
/// sampler's voices and the quality lab's pad filter both run this, so the
/// lab measures what plays.
#[derive(Clone, Debug)]
pub struct VoiceFilter {
    pub multi: Multi,
    pub vowel: Formants,
    /// The Vowel's gain after its formants.
    pub vowel_gain: f32,
    pub ladder: Ladder,
    /// Made with the pool and kept from note to note ([`Comb::reset`]), so
    /// nothing allocates.
    pub comb: Comb,
}

impl Default for VoiceFilter {
    fn default() -> VoiceFilter {
        VoiceFilter {
            multi: Multi::default(),
            vowel: Formants::default(),
            vowel_gain: 1.0,
            ladder: Ladder::default(),
            comb: Comb::default(),
        }
    }
}

impl VoiceFilter {
    /// Every `frames` frames, before any new setting: zeros the states too
    /// small to hear (see [`Svf::flush`]), the multimode filter's once both
    /// of a channel's are under [`QUIET`] ([`Multi::flush_below`]; the Comb
    /// flushes as it goes), and runs the multimode filter's resonance guard
    /// ([`Multi::guard`]): its window of a loud ring's peak, letting the
    /// damping go as the sound quietens (a ring rising past it is damped
    /// in its own frame).
    #[inline]
    pub fn tick(&mut self, kind: FilterKind, frames: u32) {
        match kind {
            FilterKind::Off | FilterKind::Comb => {}
            FilterKind::Ladder => self.ladder.flush(),
            FilterKind::Vowel => self.vowel.flush(),
            _ => {
                self.multi.flush_below(QUIET);
                self.multi.guard(frames);
            }
        }
    }

    /// Sets `f`'s type at cutoff `fc` and resonance `res` (0 to 1). The
    /// multimode filter, the Ladder and the Comb glide there over `frames`
    /// frames (the first setting lands at once); the Vowel lands at once.
    #[inline]
    pub fn set(&mut self, f: &Filter, fc: f32, res: f32, rate: f32, frames: u32) {
        match f.kind {
            FilterKind::Off => {}
            FilterKind::Ladder => self.ladder.set(fc, res, f.drive_db, rate, frames),
            // The cutoff is its pitch.
            FilterKind::Comb => self.comb.set(fc, res, rate, frames),
            // The cutoff picks the vowel: its two formants, in parallel.
            FilterKind::Vowel => {
                let (f1, f2) = vowel_formants(fc);
                let r = vowel_resonance(res);
                self.vowel.set(0, f1, r, rate);
                self.vowel.set(1, f2, r, rate);
                self.vowel_gain = vowel_gain(res);
            }
            kind => {
                let rate = rate.round() as u32;
                self.multi
                    .set(svf_mode(kind), f.poles, fc, res, rate, frames);
            }
        }
    }

    /// One frame of type `kind`.
    #[inline(always)]
    pub fn process(&mut self, kind: FilterKind, l: f32, r: f32) -> (f32, f32) {
        match kind {
            FilterKind::Off => (l, r),
            FilterKind::Comb => self.comb.process(l, r),
            kind => self.process_first(kind, l, r),
        }
    }

    /// One frame of a type that comes before a voice's gains: any but Off
    /// (a voice's filter is `None` for Off) and the Comb. Two tests, not a
    /// jump table: the voice's loop runs it every frame.
    #[inline(always)]
    pub fn process_first(&mut self, kind: FilterKind, l: f32, r: f32) -> (f32, f32) {
        debug_assert!(kind != FilterKind::Off && kind != FilterKind::Comb);
        if kind == FilterKind::Ladder {
            self.ladder.process(l, r)
        } else if kind == FilterKind::Vowel {
            let y = self.vowel.process(l, r);
            let g = self.vowel_gain;
            (g * (y[0] + FORMANT_2 * y[2]), g * (y[1] + FORMANT_2 * y[3]))
        } else {
            self.multi.process(l, r)
        }
    }

    /// Every state but the Comb's line from silence, as a filter that
    /// comes in on a sounding voice starts.
    pub fn clear(&mut self) {
        self.multi = Multi::default();
        self.vowel = Formants::default();
        self.ladder = Ladder::default();
    }
}

/// A small deterministic random generator (xorshift32) for layer choice
/// and, later, grains. Seeded, so offline renders repeat exactly.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(if seed == 0 { 0x9E37_79B9 } else { seed })
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// 0..n (n > 0).
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_hits_the_points_and_follows_a_line() {
        assert_eq!(hermite(0.0, 1.0, 2.0, 3.0, 0.0), 1.0);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 1.0) - 2.0).abs() < 1e-6);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 0.5) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn pan_and_crossfade_keep_power() {
        for p in [-1.0f32, -0.3, 0.0, 0.6, 1.0] {
            let (l, r) = pan_gains(p);
            assert!((l * l + r * r - 1.0).abs() < 1e-5);
        }
        let (l, r) = pan_gains(-1.0);
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
        for x in [0.0f32, 0.25, 0.5, 1.0] {
            let (a, b) = equal_power(x);
            assert!((a * a + b * b - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn envelope_runs_its_stages() {
        let rate = 1000.0; // 1 sample = 1 ms
        let p = Ahdsr {
            attack_ms: 10.0,
            hold_ms: 5.0,
            decay_ms: 50.0,
            sustain: 0.5,
            release_ms: 20.0,
        };
        let mut e = Envelope::start(&p, rate);
        let mut peak_at = 0;
        for i in 0..10 {
            if e.next() >= 1.0 {
                peak_at = i;
                break;
            }
        }
        assert_eq!(peak_at, 9, "linear attack reaches 1 after 10 samples");
        for _ in 0..200 {
            e.next();
        }
        assert!((e.level() - 0.5).abs() < 1e-3, "settles at sustain");
        e.release();
        let mut n = 0;
        while !e.is_done() {
            e.next();
            n += 1;
            assert!(n < 100);
        }
        assert!(
            (19..=21).contains(&n),
            "release takes about its time, took {n}"
        );
    }

    #[test]
    fn zero_attack_starts_at_full_level() {
        let mut e = Envelope::start(&Ahdsr::OPEN, 44_100.0);
        assert_eq!(e.next(), 1.0);
    }

    #[test]
    fn low_pass_passes_dc_and_cuts_highs() {
        let rate = 44_100.0;
        let mut f = Svf::default();
        f.set(1000.0, 0.0, rate);
        let mut y = 0.0;
        for _ in 0..4000 {
            y = f.process(0, 1.0, FilterKind::LowPass);
        }
        assert!((y - 1.0).abs() < 1e-3, "DC passes: {y}");
        let mut f = Svf::default();
        f.set(200.0, 0.0, rate);
        let mut peak = 0f32;
        for i in 0..4000 {
            let x = if i % 2 == 0 { 1.0 } else { -1.0 }; // Nyquist
            let y = f.process(0, x, FilterKind::LowPass);
            if i > 2000 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak < 1e-3, "Nyquist is cut: {peak}");
    }

    #[test]
    fn rng_is_repeatable() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..10 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn the_drives_average_is_the_curves_mean_over_the_step() {
        // Against the mean of the curve at many points along the step.
        let mean = |u: f32, w: f32| {
            let n = 4000;
            (0..n)
                .map(|i| drive_curve_u(w + (u - w) * (i as f32 + 0.5) / n as f32))
                .sum::<f32>()
                / n as f32
        };
        for (u, w) in [
            (0.3, -0.2),
            (0.9, 0.95),
            (-0.7, 0.1),
            (1.4, 0.5),
            (-2.0, 0.8),
            (1.5, 3.0),
            (-1.2, -4.0),
            (1.00005, 0.99995),
        ] {
            let got = drive_average_u(u, w);
            assert!(
                (got - mean(u, w)).abs() < 2e-4,
                "{u} {w}: {got} {}",
                mean(u, w)
            );
        }
        // A step of nothing: the curve itself.
        for u in [-1.5, -0.5, 0.0, 0.25, 0.99, 2.0] {
            assert!((drive_average_u(u, u) - drive_curve_u(u)).abs() < 1e-6);
        }
        // Slope 1 through 0, flat at 1 from the knee.
        assert!((drive_curve(1e-3) / 1e-3 - 1.0).abs() < 1e-3);
        assert_eq!(drive_curve(DRIVE_KNEE), 1.0);
        assert_eq!(drive_curve(10.0), 1.0);
    }
}
