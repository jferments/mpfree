//! Moving filters: the [`AutoWah`] and the [`TalkBox`].
//!
//! - The auto wah sweeps one resonant filter between a low and a high
//!   frequency, moved by the sound's own level (an envelope follower with
//!   its attack and release, as the MPC's Auto Wah), by an LFO (free or in
//!   time with the tempo), or by hand (a pedal on a Q-Link). The sweep is
//!   even in pitch, and the band-pass is normalised so a sharper peak
//!   doesn't jump in level.
//! - The talk box is the MPC's AIR Talk Box idea: three formant filters
//!   tuned to a vowel (average adult formants from Peterson and Barney,
//!   1952), so the sound seems to say "ah" or "ee". The envelope and the
//!   LFO move it along the row of vowels (back to front: OO, OU, AW, AH,
//!   UH, AE, EH, IH, EE), so it talks as the level moves or in time; the
//!   formants glide between vowels in pitch, and Formant shifts them all
//!   for a smaller or bigger mouth.
//!
//! Both are trapezoidal state-variable filters (Simper), retuned every
//! [`CTRL`] frames from a smoothed position, so a sweep costs a few
//! tangents a block rather than one a frame.

use std::f32::consts::PI;

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth, Unit};

use crate::kit::{
    self, node_impl, param, Choice, Follower, Glide, Hz, Lfo, Linear, Log, Ms, Percent, NONE,
};
use crate::{defaults, fast_db_to_gain, fast_exp2, fast_log2, flush, lerp, Preset};

/// Frames between retunes.
pub(crate) const CTRL: usize = 16;

/// One stereo trapezoidal state-variable stage, tuned by [`Sv::tune`].
#[derive(Clone, Copy, Debug, Default)]
struct Sv {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
}

impl Sv {
    fn tune(&mut self, hz: f32, q: f32, rate: u32) {
        let fs = rate as f32;
        let g = (PI * hz.clamp(10.0, 0.45 * fs) / fs).tan();
        self.k = 1.0 / q.max(0.1);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    /// `(low, band)` for one sample of channel `ch`.
    #[inline]
    fn run(&mut self, ch: usize, v0: f32) -> (f32, f32) {
        let (ic1, ic2) = (self.ic1[ch], self.ic2[ch]);
        let v3 = v0 - ic2;
        let v1 = self.a1 * ic1 + self.a2 * v3;
        let v2 = ic2 + self.a2 * ic1 + self.a3 * v3;
        self.ic1[ch] = flush(2.0 * v1 - ic1);
        self.ic2[ch] = flush(2.0 * v2 - ic2);
        (v2, v1)
    }
    fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
    }
}

/// How much louder the envelope reads at full Sensitivity, in dB.
const SENS_DB: f32 = 42.0;

/// The envelope follower's level (0 up) as a sweep amount, 0 to 1: soft
/// at the top, so loud notes don't all hit the end stop.
#[inline]
fn env_amount(e: f32) -> f32 {
    e / (1.0 + e)
}

// ---------------------------------------------------------------- auto wah

/// [`AutoWah`] parameter ids.
pub mod autowah {
    pub const SOURCE: u16 = 0;
    pub const CENTER: u16 = 1;
    pub const DEPTH: u16 = 2;
    pub const SENSITIVITY: u16 = 3;
    pub const ATTACK: u16 = 4;
    pub const RELEASE: u16 = 5;
    pub const RATE: u16 = 6;
    pub const SYNC: u16 = 7;
    pub const LOW: u16 = 8;
    pub const HIGH: u16 = 9;
    pub const RESONANCE: u16 = 10;
    pub const FILTER: u16 = 11;
    pub const MIX: u16 = 12;
}

const SOURCES: &[&str] = &["Envelope", "LFO", "Pedal"];
const FILTERS: &[&str] = &["Band-pass", "Low-pass", "High-pass"];

static AUTOWAH_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.autowah",
    name: "Auto wah",
    kind: Kind::Effect,
};
static AUTOWAH_PARAMS: [ParamInfo; 13] = [
    param(0, "Source", "Wah", 0.0, 2.0, 0.0, NONE, Choice(SOURCES)),
    param(1, "Center", "Wah", 0.0, 100.0, 0.0, Percent, Linear),
    param(2, "Depth", "Wah", -100.0, 100.0, 100.0, Percent, Linear),
    param(
        3,
        "Sensitivity",
        "Envelope",
        0.0,
        100.0,
        50.0,
        Percent,
        Linear,
    ),
    param(4, "Attack", "Envelope", 1.0, 200.0, 8.0, Ms, Log),
    param(5, "Release", "Envelope", 10.0, 2000.0, 150.0, Ms, Log),
    param(6, "Rate", "LFO", 0.05, 10.0, 1.0, Hz, Log),
    param(7, "Sync", "LFO", 0.0, 13.0, 0.0, NONE, Choice(kit::SYNCS)),
    param(8, "Low", "Filter", 100.0, 1500.0, 300.0, Hz, Log),
    param(9, "High", "Filter", 500.0, 6000.0, 2500.0, Hz, Log),
    param(10, "Resonance", "Filter", 0.0, 100.0, 60.0, Percent, Linear),
    param(11, "Filter", "Filter", 0.0, 2.0, 0.0, NONE, Choice(FILTERS)),
    param(12, "Mix", "Wah", 0.0, 100.0, 100.0, Percent, Linear),
];

/// Auto wah factory presets.
pub static AUTOWAH_PRESETS: &[Preset] = &[
    Preset {
        name: "Funk quack",
        values: &[
            (autowah::SENSITIVITY, 65.0),
            (autowah::RELEASE, 90.0),
            (autowah::RESONANCE, 75.0),
        ],
    },
    Preset {
        name: "Slow bloom",
        values: &[
            (autowah::ATTACK, 60.0),
            (autowah::RELEASE, 600.0),
            (autowah::LOW, 200.0),
            (autowah::HIGH, 1800.0),
            (autowah::FILTER, 1.0),
        ],
    },
    Preset {
        name: "Down wah",
        values: &[
            (autowah::CENTER, 100.0),
            (autowah::DEPTH, -100.0),
            (autowah::SENSITIVITY, 60.0),
        ],
    },
    Preset {
        name: "Eighth sweep",
        values: &[
            (autowah::SOURCE, 1.0),
            (autowah::SYNC, 5.0),
            (autowah::CENTER, 50.0),
            (autowah::DEPTH, 50.0),
        ],
    },
    Preset {
        name: "Pedal",
        values: &[
            (autowah::SOURCE, 2.0),
            (autowah::CENTER, 50.0),
            (autowah::RESONANCE, 80.0),
        ],
    },
];

/// A wah moved by the sound's level, an LFO or by hand.
pub struct AutoWah {
    v: [f32; 13],
    rate: u32,
    lfo: Lfo,
    env: Follower,
    filter: Sv,
    /// The sweep position, 0 to 1 (glides between retunes).
    pos: Smooth,
    mix: Smooth,
    /// Frames until the next retune.
    ctrl: usize,
    sens: f32,
    lfo_now: f32,
}

impl Default for AutoWah {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoWah {
    pub fn new() -> AutoWah {
        let v = defaults(&AUTOWAH_PARAMS);
        let mut w = AutoWah {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            env: Follower::default(),
            filter: Sv::default(),
            pos: Smooth::new(v[1] / 100.0),
            mix: Smooth::new(v[12] / 100.0),
            ctrl: 0,
            sens: 1.0,
            lfo_now: 0.0,
        };
        w.setup(48_000);
        w
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            autowah::SENSITIVITY => self.sens = fast_db_to_gain(v / 100.0 * SENS_DB),
            autowah::ATTACK | autowah::RELEASE => {
                self.env.set(self.v[4], self.v[5], self.rate);
            }
            autowah::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, ctx: &Ctx) {
        self.lfo.begin(ctx, self.v[7] as usize);
    }
    /// The position the controls and the modulation ask for, 0 to 1.
    fn target(&self) -> f32 {
        let m = match self.v[0] as usize {
            0 => env_amount(self.env.env),
            1 => 0.5 + 0.5 * self.lfo_now,
            _ => 0.0,
        };
        (self.v[1] / 100.0 + self.v[2] / 100.0 * m).clamp(0.0, 1.0)
    }
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.set(self.v[6], self.v[7] as usize, self.rate);
        let (lo, hi) = (self.v[8].log2(), self.v[9].log2());
        let res = self.v[10] / 100.0;
        let q = 0.7 + res * res * 11.3;
        let mode = self.v[11] as usize;
        // A sharper band-pass gets a little more level back.
        let bp_gain = 1.0 + res;
        for i in 0..l.len() {
            let p = self.lfo.step();
            self.lfo_now = kit::sine(p);
            self.env.run(0.5 * (l[i].abs() + r[i].abs()) * self.sens);
            if self.ctrl == 0 {
                self.ctrl = CTRL;
                self.pos.target = self.target();
                let hz = fast_exp2(lo + (hi - lo) * self.pos.glide());
                self.filter.tune(hz, q, self.rate);
            } else {
                self.pos.glide();
            }
            self.ctrl -= 1;
            let mix = self.mix.glide();
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let x = *s;
                let (low, band) = self.filter.run(ch, x);
                let y = match mode {
                    1 => low,
                    2 => x - self.filter.k * band - low,
                    _ => self.filter.k * band * bp_gain,
                };
                *s = lerp(x, y, mix);
            }
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        // The position glides per frame between retunes; the follower
        // sets the pace, this only smooths the steps.
        self.pos.set_time(rate, 4.0);
        self.mix.set_time(rate, 20.0);
        self.env.set(self.v[4], self.v[5], rate);
        self.sens = fast_db_to_gain(self.v[3] / 100.0 * SENS_DB);
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.env.reset();
        self.filter.reset();
        self.ctrl = 0;
        self.lfo_now = 0.0;
        self.pos.snap(self.target());
        self.mix.snap(self.mix.target);
    }
    fn tail(&self) -> u32 {
        // The filter rings for a few periods of its lowest note.
        self.rate / 20
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(AutoWah, AUTOWAH_INFO, AUTOWAH_PARAMS);

// ---------------------------------------------------------------- talk box

/// [`TalkBox`] parameter ids.
pub mod talkbox {
    pub const VOWEL: u16 = 0;
    pub const FORMANT: u16 = 1;
    pub const RESONANCE: u16 = 2;
    pub const ENV_DEPTH: u16 = 3;
    pub const SENSITIVITY: u16 = 4;
    pub const ATTACK: u16 = 5;
    pub const RELEASE: u16 = 6;
    pub const LFO_RATE: u16 = 7;
    pub const LFO_SYNC: u16 = 8;
    pub const LFO_SHAPE: u16 = 9;
    pub const LFO_DEPTH: u16 = 10;
    pub const MIX: u16 = 11;
}

/// The vowels, back to front, so a sweep passes through its neighbours.
pub const VOWELS: &[&str] = &["OO", "OU", "AW", "AH", "UH", "AE", "EH", "IH", "EE"];

/// Each vowel's three formants in Hz and the second and third formants'
/// levels in dB under the first (Peterson and Barney's adult averages;
/// the levels after Fant).
const FORMANTS: [([f32; 3], [f32; 2]); 9] = [
    ([300.0, 870.0, 2240.0], [-12.0, -22.0]),  // OO "who'd"
    ([440.0, 1020.0, 2240.0], [-9.0, -20.0]),  // OU "hood"
    ([570.0, 840.0, 2410.0], [-4.0, -22.0]),   // AW "hawed"
    ([730.0, 1090.0, 2440.0], [-5.0, -18.0]),  // AH "father"
    ([640.0, 1190.0, 2390.0], [-6.0, -16.0]),  // UH "hud"
    ([660.0, 1720.0, 2410.0], [-6.0, -14.0]),  // AE "had"
    ([530.0, 1840.0, 2480.0], [-8.0, -12.0]),  // EH "head"
    ([390.0, 1990.0, 2550.0], [-10.0, -10.0]), // IH "hid"
    ([270.0, 2290.0, 3010.0], [-14.0, -10.0]), // EE "heed"
];
/// The formants' bandwidths in Hz at Resonance 50 %.
const BANDWIDTH: [f32; 3] = [80.0, 100.0, 140.0];
/// How many vowels the envelope and the LFO move it at full depth.
const REACH: f32 = 4.0;
/// Level back for the formant bank (it passes a fraction of a full-band
/// sound).
const TALK_GAIN: f32 = 2.5;

static TALKBOX_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.talkbox",
    name: "Talk box",
    kind: Kind::Effect,
};
static TALKBOX_PARAMS: [ParamInfo; 12] = [
    param(0, "Vowel", "Voice", 0.0, 8.0, 3.0, NONE, Choice(VOWELS)),
    param(
        1,
        "Formant",
        "Voice",
        -12.0,
        12.0,
        0.0,
        Unit::Semitones,
        Linear,
    ),
    param(2, "Resonance", "Voice", 0.0, 100.0, 50.0, Percent, Linear),
    param(
        3,
        "Env depth",
        "Envelope",
        -100.0,
        100.0,
        0.0,
        Percent,
        Linear,
    ),
    param(
        4,
        "Sensitivity",
        "Envelope",
        0.0,
        100.0,
        50.0,
        Percent,
        Linear,
    ),
    param(5, "Attack", "Envelope", 1.0, 2000.0, 30.0, Ms, Log),
    param(6, "Release", "Envelope", 10.0, 5000.0, 300.0, Ms, Log),
    param(7, "LFO rate", "LFO", 0.05, 10.0, 1.0, Hz, Log),
    param(
        8,
        "LFO sync",
        "LFO",
        0.0,
        13.0,
        0.0,
        NONE,
        Choice(kit::SYNCS),
    ),
    param(
        9,
        "LFO shape",
        "LFO",
        0.0,
        5.0,
        0.0,
        NONE,
        Choice(kit::SHAPES),
    ),
    param(10, "LFO depth", "LFO", -100.0, 100.0, 0.0, Percent, Linear),
    param(11, "Mix", "Voice", 0.0, 100.0, 100.0, Percent, Linear),
];

/// Talk box factory presets.
pub static TALKBOX_PRESETS: &[Preset] = &[
    Preset {
        name: "Yeah",
        values: &[
            (talkbox::VOWEL, 8.0),
            (talkbox::ENV_DEPTH, -100.0),
            (talkbox::SENSITIVITY, 60.0),
        ],
    },
    Preset {
        name: "Wow wow",
        values: &[
            (talkbox::VOWEL, 0.0),
            (talkbox::LFO_SYNC, 8.0),
            (talkbox::LFO_SHAPE, 1.0),
            (talkbox::LFO_DEPTH, 100.0),
        ],
    },
    Preset {
        name: "Robot choir",
        values: &[
            (talkbox::VOWEL, 3.0),
            (talkbox::RESONANCE, 85.0),
            (talkbox::LFO_RATE, 0.2),
            (talkbox::LFO_DEPTH, 40.0),
        ],
    },
    Preset {
        name: "Small mouth",
        values: &[(talkbox::VOWEL, 6.0), (talkbox::FORMANT, 5.0)],
    },
];

/// The formants (log2 Hz) and their gains at vowel position `v` (0 to 8),
/// shifted by `shift` octaves.
fn vowel_at(v: f32, shift: f32) -> ([f32; 3], [f32; 3]) {
    let v = v.clamp(0.0, (FORMANTS.len() - 1) as f32);
    let i = (v as usize).min(FORMANTS.len() - 2);
    let t = v - i as f32;
    let (a, b) = (&FORMANTS[i], &FORMANTS[i + 1]);
    let mut f = [0.0; 3];
    for k in 0..3 {
        f[k] = lerp(fast_log2(a.0[k]), fast_log2(b.0[k]), t) + shift;
    }
    let g = [
        1.0,
        fast_db_to_gain(lerp(a.1[0], b.1[0], t)),
        fast_db_to_gain(lerp(a.1[1], b.1[1], t)),
    ];
    (f, g)
}

/// Vowel formants that talk, moved by the level or an LFO.
pub struct TalkBox {
    v: [f32; 12],
    rate: u32,
    lfo: Lfo,
    env: Follower,
    bank: [Sv; 3],
    gains: [f32; 3],
    /// The vowel position, 0 to 8 (glides between retunes).
    pos: Smooth,
    mix: Smooth,
    ctrl: usize,
    sens: f32,
    lfo_now: f32,
}

impl Default for TalkBox {
    fn default() -> Self {
        Self::new()
    }
}

impl TalkBox {
    pub fn new() -> TalkBox {
        let v = defaults(&TALKBOX_PARAMS);
        let mut t = TalkBox {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            env: Follower::default(),
            bank: [Sv::default(); 3],
            gains: [1.0; 3],
            pos: Smooth::new(v[0]),
            mix: Smooth::new(v[11] / 100.0),
            ctrl: 0,
            sens: 1.0,
            lfo_now: 0.0,
        };
        t.setup(48_000);
        t
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            talkbox::SENSITIVITY => self.sens = fast_db_to_gain(v / 100.0 * SENS_DB),
            talkbox::ATTACK | talkbox::RELEASE => {
                self.env.set(self.v[5], self.v[6], self.rate);
            }
            talkbox::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, ctx: &Ctx) {
        self.lfo.begin(ctx, self.v[8] as usize);
    }
    fn target(&self) -> f32 {
        let env = env_amount(self.env.env) * self.v[3] / 100.0;
        let lfo = self.lfo_now * self.v[10] / 100.0;
        self.v[0] + REACH * (env + lfo)
    }
    fn retune(&mut self) {
        let (f, g) = vowel_at(self.pos.value, self.v[1] / 12.0);
        // Resonance 0 to 100 % is half to twice as sharp as speech.
        let sharp = fast_exp2(self.v[2] / 50.0 - 1.0);
        for k in 0..3 {
            let hz = fast_exp2(f[k]);
            let q = hz / BANDWIDTH[k] * sharp;
            self.bank[k].tune(hz, q, self.rate);
        }
        self.gains = g;
    }
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.set(self.v[7], self.v[8] as usize, self.rate);
        let shape = self.v[9] as usize;
        for i in 0..l.len() {
            let p = self.lfo.step();
            self.lfo_now = self.lfo.shape(shape, p, 0.0);
            self.env.run(0.5 * (l[i].abs() + r[i].abs()) * self.sens);
            if self.ctrl == 0 {
                self.ctrl = CTRL;
                self.pos.target = self.target();
                self.pos.glide();
                self.retune();
            } else {
                self.pos.glide();
            }
            self.ctrl -= 1;
            let mix = self.mix.glide();
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let x = *s;
                let mut y = 0.0;
                for k in 0..3 {
                    let (_, band) = self.bank[k].run(ch, x);
                    y += self.bank[k].k * band * self.gains[k];
                }
                *s = lerp(x, y * TALK_GAIN, mix);
            }
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.pos.set_time(rate, 6.0);
        self.mix.set_time(rate, 20.0);
        self.env.set(self.v[5], self.v[6], rate);
        self.sens = fast_db_to_gain(self.v[4] / 100.0 * SENS_DB);
        self.retune();
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.env.reset();
        for b in &mut self.bank {
            b.reset();
        }
        self.ctrl = 0;
        self.lfo_now = 0.0;
        self.pos.snap(self.target());
        self.mix.snap(self.mix.target);
        self.retune();
    }
    fn tail(&self) -> u32 {
        self.rate / 20
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(TalkBox, TALKBOX_INFO, TALKBOX_PARAMS);

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;

    fn sine_level(fx: &mut dyn Node, hz: f32, amp: f32) -> f32 {
        fx.prepare(48_000, 128);
        fx.reset();
        let n = 48_000;
        let mut l: Vec<f32> = (0..n)
            .map(|i| amp * (i as f32 * hz / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut r = l.clone();
        let ctx = Ctx::new(48_000, 0);
        for k in (0..n).step_by(128) {
            fx.process(&ctx, &[], &mut l[k..k + 128], &mut r[k..k + 128]);
        }
        let tail = &l[n / 2..];
        (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt() / (amp / 2f32.sqrt())
    }

    #[test]
    fn the_pedal_puts_the_peak_where_it_points() {
        // Pedal down: the low end passes a low tone and cuts a high one;
        // pedal up the other way round.
        let mut w = AutoWah::new();
        w.set_param(autowah::SOURCE, 2.0);
        w.set_param(autowah::CENTER, 0.0);
        let low = sine_level(&mut w, 300.0, 0.1);
        let high = sine_level(&mut w, 2500.0, 0.1);
        assert!(low > 0.7 && high < 0.3, "down: {low} {high}");
        w.set_param(autowah::CENTER, 100.0);
        let low = sine_level(&mut w, 300.0, 0.1);
        let high = sine_level(&mut w, 2500.0, 0.1);
        assert!(high > 0.7 && low < 0.3, "up: {low} {high}");
    }

    #[test]
    fn a_louder_sound_opens_the_wah_further() {
        // On the envelope, a 2 kHz tone passes more when it is loud.
        let mut w = AutoWah::new();
        let quiet = sine_level(&mut w, 2000.0, 0.003);
        let loud = sine_level(&mut w, 2000.0, 0.5);
        assert!(loud > 2.0 * quiet, "{quiet} {loud}");
    }

    #[test]
    fn vowels_have_their_formants() {
        // "AH" passes 730 Hz (its first formant) far better than "EE"
        // does, and "EE" passes 2.29 kHz (its second) better than "AH".
        let mut t = TalkBox::new();
        t.set_param(talkbox::VOWEL, 3.0);
        let ah_low = sine_level(&mut t, 730.0, 0.1);
        let ah_high = sine_level(&mut t, 2290.0, 0.1);
        t.set_param(talkbox::VOWEL, 8.0);
        let ee_low = sine_level(&mut t, 730.0, 0.1);
        let ee_high = sine_level(&mut t, 2290.0, 0.1);
        assert!(ah_low > 4.0 * ee_low, "{ah_low} {ee_low}");
        assert!(ee_high > 2.0 * ah_high, "{ee_high} {ah_high}");
    }

    #[test]
    fn the_formant_shift_moves_the_vowel_up() {
        let (f0, _) = vowel_at(3.0, 0.0);
        let (f1, _) = vowel_at(3.0, 1.0);
        for k in 0..3 {
            assert!((f1[k] - f0[k] - 1.0).abs() < 1e-5);
        }
        // Halfway between two vowels is between them in pitch.
        let (a, _) = vowel_at(0.0, 0.0);
        let (m, _) = vowel_at(0.5, 0.0);
        let (b, _) = vowel_at(1.0, 0.0);
        assert!(m[1] > a[1] && m[1] < b[1]);
    }
}
