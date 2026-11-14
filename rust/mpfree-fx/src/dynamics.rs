//! More dynamics: [`Limiter`], [`Gate`], [`DeEsser`] and
//! [`MultibandCompressor`].
//!
//! The limiter and the gate look ahead by a fixed time, so their latency
//! never changes with their settings (the mixer compensates for it, and an
//! export fails if a latency moves mid-pass). Detectors work in dB through
//! the crate's fast log and exp, so nothing calls the C library per frame.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{
    self, meter, node_impl, param, Choice, Db, Glide, Hz, Linear, Log, Ms, Percent, Toggle, NONE,
};
use crate::svf::{Svf, SvfMode};
use crate::{
    db_to_gain, defaults, fast_db_to_gain, fast_level_db, flush, gain_to_db, lerp, Preset,
};

/// The soft-knee gain reduction in dB for `over` dB above the threshold at
/// `ratio`, with a knee `knee` dB wide.
#[inline]
fn knee_reduction(over: f32, ratio: f32, knee: f32) -> f32 {
    let slope = 1.0 - 1.0 / ratio.max(1.0);
    let h = 0.5 * knee;
    if over <= -h {
        0.0
    } else if over >= h {
        over * slope
    } else {
        let x = over + h;
        slope * x * x / (2.0 * knee)
    }
}

// ---------------------------------------------------------------- limiter

/// [`Limiter`] parameter ids.
pub mod limiter {
    pub const GAIN: u16 = 0;
    pub const CEILING: u16 = 1;
    pub const RELEASE: u16 = 2;
    pub const AUTO: u16 = 3;
    pub const TRUE_PEAK: u16 = 4;
    /// Meter: gain reduction now, in dB (0 or more).
    pub const REDUCTION: u16 = 5;
}

/// The look-ahead, fixed so the latency never moves.
const LIMIT_AHEAD_MS: f32 = 1.5;
/// Taps of each true-peak interpolation phase (BS.1770's 4x filter has 12
/// a phase too), and how far behind the newest input the detector runs.
const TP_TAPS: usize = 12;
const TP_LAG: usize = TP_TAPS / 2;

/// The true-peak filter: for the points a quarter, a half and three
/// quarters of the way between two samples, Hann-windowed sinc taps over
/// the 12 samples around them, each phase normalised to unity at DC.
fn tp_phases() -> [[f32; TP_TAPS]; 3] {
    let mut out = [[0.0f32; TP_TAPS]; 3];
    for (k, ph) in out.iter_mut().enumerate() {
        let frac = (k + 1) as f64 * 0.25;
        let mut sum = 0.0;
        for (j, tap) in ph.iter_mut().enumerate() {
            // Sample j sits at (j - (TP_LAG - 1)) from the left sample of
            // the gap; the point is at frac.
            let x = j as f64 - (TP_LAG as f64 - 1.0) - frac;
            let sinc = if x.abs() < 1e-9 {
                1.0
            } else {
                (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
            };
            let w = 0.5 + 0.5 * (std::f64::consts::PI * x / (TP_LAG as f64 + 0.5)).cos();
            let v = sinc * w;
            *tap = v as f32;
            sum += v;
        }
        for tap in ph.iter_mut() {
            *tap /= sum as f32;
        }
    }
    out
}

static LIMITER_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.limiter",
    name: "Limiter",
    kind: Kind::Effect,
};
static LIMITER_PARAMS: [ParamInfo; 6] = [
    param(0, "Gain", "Limiter", 0.0, 24.0, 0.0, Db, Linear),
    param(1, "Ceiling", "Limiter", -12.0, 0.0, -0.3, Db, Linear),
    param(2, "Release", "Limiter", 5.0, 1000.0, 80.0, Ms, Log),
    param(3, "Auto release", "Limiter", 0.0, 1.0, 1.0, NONE, Toggle),
    param(4, "True peak", "Limiter", 0.0, 1.0, 0.0, NONE, Toggle),
    meter(5, "Reduction", "Limiter", 0.0, 30.0, Db),
];

/// Limiter factory presets.
pub static LIMITER_PRESETS: &[Preset] = &[
    Preset {
        name: "Safe peaks",
        values: &[(limiter::CEILING, -1.0)],
    },
    Preset {
        name: "Loud master",
        values: &[
            (limiter::GAIN, 6.0),
            (limiter::CEILING, -1.0),
            (limiter::TRUE_PEAK, 1.0),
        ],
    },
    Preset {
        name: "Drum smash",
        values: &[
            (limiter::GAIN, 12.0),
            (limiter::RELEASE, 25.0),
            (limiter::AUTO, 0.0),
        ],
    },
];

/// A brick-wall look-ahead limiter: the gain is down before each peak
/// arrives, so nothing passes the ceiling and there is no overshoot. Auto
/// release lets go fast after a short peak and slowly under a sustained
/// push, which keeps loud material from pumping. True peak also catches
/// the peaks between samples that a converter would make.
pub struct Limiter {
    v: [f32; 6],
    rate: u32,
    /// Look-ahead window in frames.
    w: usize,
    gain: Smooth,
    ceiling: Smooth,
    rel_fast: f32,
    rel_slow: f32,
    att_slow: f32,
    env_fast: f32,
    env_slow: f32,
    /// Sliding minimum of the wanted gain: (frame, value), increasing.
    dq: Vec<(u64, f32)>,
    dq_head: usize,
    dq_len: usize,
    /// The minima of the last `w` frames, and their sum.
    box_buf: Vec<f32>,
    box_pos: usize,
    box_sum: f64,
    /// Input history for the detector, written twice so the last
    /// TP_TAPS frames always sit side by side from `hist_at` (oldest
    /// first), and the audio delay.
    hist: [[f32; 2 * TP_TAPS]; 2],
    hist_at: usize,
    tp: [[f32; TP_TAPS]; 3],
    delay: [kit::Line; 2],
    frame: u64,
    least: f32,
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Limiter {
    pub fn new() -> Limiter {
        let v = defaults(&LIMITER_PARAMS);
        let mut l = Limiter {
            v,
            rate: 48_000,
            w: 1,
            gain: Smooth::new(db_to_gain(v[0])),
            ceiling: Smooth::new(db_to_gain(v[1])),
            rel_fast: 0.0,
            rel_slow: 0.0,
            att_slow: 0.0,
            env_fast: 1.0,
            env_slow: 1.0,
            dq: Vec::new(),
            dq_head: 0,
            dq_len: 0,
            box_buf: Vec::new(),
            box_pos: 0,
            box_sum: 0.0,
            hist: [[0.0; 2 * TP_TAPS]; 2],
            hist_at: 0,
            tp: tp_phases(),
            delay: [kit::Line::default(), kit::Line::default()],
            frame: 0,
            least: 1.0,
        };
        l.setup(48_000);
        l
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            limiter::GAIN => self.gain.target = db_to_gain(v),
            limiter::CEILING => self.ceiling.target = db_to_gain(v),
            limiter::RELEASE | limiter::AUTO => self.set_release(),
            _ => {}
        }
    }
    fn set_release(&mut self) {
        let ms = self.v[limiter::RELEASE as usize];
        if self.v[limiter::AUTO as usize] >= 0.5 {
            // Short peaks recover at a third of Release; a push that lasts
            // pulls the slow envelope down, which lets go at three times
            // Release.
            self.rel_fast = kit::coef(ms / 3.0, self.rate);
            self.rel_slow = kit::coef(ms * 3.0, self.rate);
            self.att_slow = kit::coef(80.0, self.rate);
        } else {
            self.rel_fast = kit::coef(ms, self.rate);
            self.rel_slow = 1.0;
            self.att_slow = 0.0;
        }
    }

    /// The detector's level for the sample TP_LAG frames back (with the
    /// points between it and the next sample when True peak is on).
    #[inline]
    fn level(&self, ch: usize, tp: bool) -> f32 {
        let h = &self.hist[ch][self.hist_at..self.hist_at + TP_TAPS];
        let mut p = h[TP_LAG - 1].abs();
        if tp {
            for ph in &self.tp {
                let mut y = 0.0;
                for j in 0..TP_TAPS {
                    y += ph[j] * h[j];
                }
                p = p.max(y.abs());
            }
        }
        p
    }

    fn push_min(&mut self, g: f32) {
        let cap = self.dq.len();
        // Indices wrap by one subtraction: a `%` is a library call on
        // the MPC's ARM, which has no divide instruction.
        let wrap = |i: usize| if i >= cap { i - cap } else { i };
        // Drop values no smaller than the new one from the back.
        while self.dq_len > 0 {
            let back = wrap(self.dq_head + self.dq_len - 1);
            if self.dq[back].1 >= g {
                self.dq_len -= 1;
            } else {
                break;
            }
        }
        let at = wrap(self.dq_head + self.dq_len);
        self.dq[at] = (self.frame, g);
        self.dq_len += 1;
        // Drop the front once it leaves the window.
        let oldest = (self.frame + 1).saturating_sub(self.w as u64);
        while self.dq[self.dq_head].0 < oldest {
            self.dq_head = wrap(self.dq_head + 1);
            self.dq_len -= 1;
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let tp = self.v[limiter::TRUE_PEAK as usize] >= 0.5;
        let auto = self.v[limiter::AUTO as usize] >= 0.5;
        let inv_w = 1.0 / self.w as f64;
        for i in 0..l.len() {
            let gain = self.gain.glide();
            let ceiling = self.ceiling.glide();
            let x = [l[i] * gain, r[i] * gain];
            for ch in 0..2 {
                self.hist[ch][self.hist_at] = x[ch];
                self.hist[ch][self.hist_at + TP_TAPS] = x[ch];
                self.delay[ch].push(x[ch]);
            }
            self.hist_at += 1;
            if self.hist_at == TP_TAPS {
                self.hist_at = 0;
            }
            let peak = self.level(0, tp).max(self.level(1, tp));
            let want = if peak > ceiling { ceiling / peak } else { 1.0 };
            // Release: down at once, up at the release rate.
            self.env_fast = if want < self.env_fast {
                want
            } else {
                flush(self.env_fast + (want - self.env_fast) * self.rel_fast)
            };
            let env = if auto {
                let k = if want < self.env_slow {
                    self.att_slow
                } else {
                    self.rel_slow
                };
                self.env_slow = flush(self.env_slow + (want - self.env_slow) * k);
                // The slow envelope only holds the gain down (it never
                // goes under what a peak needs on its own).
                self.env_fast.min(self.env_slow.max(want))
            } else {
                self.env_fast
            };
            self.push_min(env);
            let m = self.dq[self.dq_head].1;
            self.box_sum += f64::from(m) - f64::from(self.box_buf[self.box_pos]);
            self.box_buf[self.box_pos] = m;
            self.box_pos += 1;
            if self.box_pos >= self.w {
                self.box_pos = 0;
            }
            self.frame += 1;
            let g = ((self.box_sum * inv_w) as f32).min(1.0);
            self.least = self.least.min(g);
            // The audio, latency frames late (the detector runs TP_LAG
            // frames behind the newest input, and the window adds w - 1);
            // a tap of 1 is the newest frame.
            let d = self.lat() as usize + 1;
            l[i] = (self.delay[0].tap(d) * g).clamp(-ceiling, ceiling);
            r[i] = (self.delay[1].tap(d) * g).clamp(-ceiling, ceiling);
        }
        self.v[limiter::REDUCTION as usize] = -gain_to_db(self.least).min(0.0);
        self.least = 1.0;
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.w = ((LIMIT_AHEAD_MS * 0.001 * rate as f32).round() as usize).max(1);
        self.dq = vec![(0, 1.0); self.w + 2];
        self.box_buf = vec![1.0; self.w];
        let d = self.w + TP_LAG + 4;
        self.delay = [kit::Line::new(d), kit::Line::new(d)];
        self.gain.set_time(rate, 20.0);
        self.ceiling.set_time(rate, 20.0);
        self.set_release();
        self.clear();
    }
    fn clear(&mut self) {
        self.env_fast = 1.0;
        self.env_slow = 1.0;
        self.dq_head = 0;
        self.dq_len = 0;
        self.box_buf.fill(1.0);
        self.box_pos = 0;
        self.box_sum = self.w as f64;
        self.hist = [[0.0; 2 * TP_TAPS]; 2];
        self.hist_at = 0;
        for d in &mut self.delay {
            d.clear();
        }
        self.frame = 0;
        self.least = 1.0;
        self.v[limiter::REDUCTION as usize] = 0.0;
        self.gain.snap(self.gain.target);
        self.ceiling.snap(self.ceiling.target);
    }
    fn tail(&self) -> u32 {
        self.lat()
    }
    fn lat(&self) -> u32 {
        (self.w + TP_LAG - 1) as u32
    }
}

node_impl!(Limiter, LIMITER_INFO, LIMITER_PARAMS);

// ---------------------------------------------------------------- gate

/// [`Gate`] parameter ids.
pub mod gate {
    pub const THRESHOLD: u16 = 0;
    pub const RANGE: u16 = 1;
    pub const ATTACK: u16 = 2;
    pub const HOLD: u16 = 3;
    pub const RELEASE: u16 = 4;
    pub const KEY_HP: u16 = 5;
    pub const KEY_LP: u16 = 6;
    pub const KEY: u16 = 7;
    /// Meter: the gate's gain now, in dB (0 open, Range shut).
    pub const GAIN: u16 = 8;
}

const KEYS: &[&str] = &["Self", "Sidechain"];
const GATE_AHEAD_MS: f32 = 1.0;
/// The gate shuts this far under the threshold it opens at, so a level
/// hovering at the threshold doesn't chatter.
const HYSTERESIS_DB: f32 = 4.0;

static GATE_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.gate",
    name: "Gate",
    kind: Kind::Effect,
};
static GATE_PARAMS: [ParamInfo; 9] = [
    param(0, "Threshold", "Gate", -80.0, 0.0, -40.0, Db, Linear),
    param(1, "Range", "Gate", -80.0, 0.0, -80.0, Db, Linear),
    param(2, "Attack", "Gate", 0.1, 50.0, 0.5, Ms, Log),
    param(3, "Hold", "Gate", 0.0, 500.0, 30.0, Ms, Linear),
    param(4, "Release", "Gate", 5.0, 2000.0, 120.0, Ms, Log),
    param(5, "Key HP", "Key", 20.0, 2000.0, 20.0, Hz, Log),
    param(6, "Key LP", "Key", 500.0, 20_000.0, 20_000.0, Hz, Log),
    param(7, "Key", "Key", 0.0, 1.0, 0.0, NONE, Choice(KEYS)),
    meter(8, "Gain", "Gate", -80.0, 0.0, Db),
];

/// Gate factory presets.
pub static GATE_PRESETS: &[Preset] = &[
    Preset {
        name: "Tight drums",
        values: &[
            (gate::THRESHOLD, -30.0),
            (gate::HOLD, 10.0),
            (gate::RELEASE, 60.0),
        ],
    },
    Preset {
        name: "Gated snare",
        values: &[
            (gate::THRESHOLD, -24.0),
            (gate::HOLD, 180.0),
            (gate::RELEASE, 15.0),
            (gate::KEY_HP, 150.0),
        ],
    },
    Preset {
        name: "Soft cleanup",
        values: &[
            (gate::THRESHOLD, -50.0),
            (gate::RANGE, -12.0),
            (gate::RELEASE, 300.0),
        ],
    },
    Preset {
        name: "Kick trigger",
        values: &[
            (gate::KEY, 1.0),
            (gate::THRESHOLD, -30.0),
            (gate::HOLD, 40.0),
            (gate::RELEASE, 80.0),
            (gate::KEY_LP, 2000.0),
        ],
    },
];

/// A gate: silences (or turns down by Range) everything under the
/// threshold. It opens a little before each hit (1 ms of look-ahead), so
/// the first edge survives, holds, then fades over Release. The key can be
/// filtered (so the kick's thump or the hi-hat's hiss doesn't open it) or
/// come from the sidechain, for rhythmic gating.
pub struct Gate {
    v: [f32; 9],
    rate: u32,
    ahead: usize,
    delay: [kit::Line; 2],
    key_hp: Svf,
    key_lp: Svf,
    env: f32,
    env_fall: f32,
    open_at: f32,
    shut_at: f32,
    open: bool,
    held: u32,
    hold_frames: u32,
    g: f32,
    floor: f32,
    att_step: f32,
    rel_mult: f32,
    least: f32,
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    pub fn new() -> Gate {
        let v = defaults(&GATE_PARAMS);
        let mut g = Gate {
            v,
            rate: 48_000,
            ahead: 48,
            delay: [kit::Line::default(), kit::Line::default()],
            key_hp: Svf::default(),
            key_lp: Svf::default(),
            env: 0.0,
            env_fall: 0.0,
            open_at: 0.0,
            shut_at: 0.0,
            open: false,
            held: 0,
            hold_frames: 0,
            g: 0.0,
            floor: 0.0,
            att_step: 1.0,
            rel_mult: 0.0,
            least: 1.0,
        };
        g.setup(48_000);
        g
    }
    fn apply(&mut self, _id: u16, _v: f32) {
        self.derive();
    }
    fn derive(&mut self) {
        let fs = self.rate as f32;
        let thr = self.v[gate::THRESHOLD as usize];
        self.open_at = db_to_gain(thr);
        self.shut_at = db_to_gain(thr - HYSTERESIS_DB);
        self.floor = db_to_gain(self.v[gate::RANGE as usize]).min(1.0);
        if self.v[gate::RANGE as usize] <= -80.0 {
            self.floor = 0.0;
        }
        // Attack: a straight ramp from shut to open.
        let na = (self.v[gate::ATTACK as usize] * 0.001 * fs).max(1.0);
        self.att_step = 1.0 / na;
        // Release: an even fade in dB from open to Range (to -80 dB when
        // Range is all the way down).
        let nr = (self.v[gate::RELEASE as usize] * 0.001 * fs).max(1.0);
        let depth = self.v[gate::RANGE as usize].max(-80.0).min(-0.01);
        self.rel_mult = db_to_gain(depth / nr);
        self.hold_frames = (self.v[gate::HOLD as usize] * 0.001 * fs) as u32;
        self.key_hp.set(
            SvfMode::HighPass,
            self.v[gate::KEY_HP as usize],
            0.707,
            0.0,
            self.rate,
        );
        self.key_lp.set(
            SvfMode::LowPass,
            self.v[gate::KEY_LP as usize],
            0.707,
            0.0,
            self.rate,
        );
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, ctx: &Ctx, from: usize, l: &mut [f32], r: &mut [f32]) {
        let side = if self.v[gate::KEY as usize] >= 0.5 {
            ctx.sidechain
        } else {
            None
        };
        let hp_on = self.v[gate::KEY_HP as usize] > 21.0;
        let lp_on = self.v[gate::KEY_LP as usize] < 19_900.0;
        for i in 0..l.len() {
            let (kl, kr) = match side {
                Some((sl, sr)) => (
                    sl.get(from + i).copied().unwrap_or(0.0),
                    sr.get(from + i).copied().unwrap_or(0.0),
                ),
                None => (l[i], r[i]),
            };
            let mut k = [kl, kr];
            for (ch, x) in k.iter_mut().enumerate() {
                if hp_on {
                    *x = self.key_hp.run(ch, *x);
                }
                if lp_on {
                    *x = self.key_lp.run(ch, *x);
                }
            }
            let level = k[0].abs().max(k[1].abs());
            self.env = if level > self.env {
                level
            } else {
                flush(self.env * self.env_fall)
            };
            if self.env >= self.open_at {
                self.open = true;
                self.held = self.hold_frames;
            } else if self.env < self.shut_at {
                if self.held > 0 {
                    self.held -= 1;
                } else {
                    self.open = false;
                }
            }
            if self.open {
                self.g = (self.g + self.att_step).min(1.0);
            } else if self.g > self.floor {
                self.g = (self.g * self.rel_mult).max(self.floor);
                if self.g < 1e-4 && self.floor == 0.0 {
                    self.g = 0.0;
                }
            }
            self.least = self.least.min(self.g);
            self.delay[0].push(l[i]);
            self.delay[1].push(r[i]);
            l[i] = self.delay[0].tap(self.ahead) * self.g;
            r[i] = self.delay[1].tap(self.ahead) * self.g;
        }
        self.v[gate::GAIN as usize] = gain_to_db(self.least).max(-80.0);
        self.least = 1.0;
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.ahead = ((GATE_AHEAD_MS * 0.001 * rate as f32).round() as usize).max(1);
        self.delay = [
            kit::Line::new(self.ahead + 4),
            kit::Line::new(self.ahead + 4),
        ];
        // The level detector falls 60 dB in about 20 ms.
        self.env_fall = db_to_gain(-60.0 / (0.02 * rate as f32));
        self.derive();
        self.clear();
    }
    fn clear(&mut self) {
        for d in &mut self.delay {
            d.clear();
        }
        self.key_hp.reset();
        self.key_lp.reset();
        self.env = 0.0;
        self.open = false;
        self.held = 0;
        self.g = self.floor;
        self.least = 1.0;
        self.v[gate::GAIN as usize] = 0.0;
    }
    fn tail(&self) -> u32 {
        self.ahead as u32
    }
    fn lat(&self) -> u32 {
        self.ahead as u32
    }
}

node_impl!(Gate, GATE_INFO, GATE_PARAMS);

// ---------------------------------------------------------------- de-esser

/// [`DeEsser`] parameter ids.
pub mod deesser {
    pub const FREQ: u16 = 0;
    pub const THRESHOLD: u16 = 1;
    pub const RANGE: u16 = 2;
    pub const MODE: u16 = 3;
    pub const DETECT: u16 = 4;
    pub const LISTEN: u16 = 5;
    /// Meter: reduction now, in dB (0 or more).
    pub const REDUCTION: u16 = 6;
}

const DEESS_MODES: &[&str] = &["Split", "Wide"];
const DETECTS: &[&str] = &["Relative", "Absolute"];
const DEESS_RATIO: f32 = 4.0;

static DEESSER_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.deesser",
    name: "De-esser",
    kind: Kind::Effect,
};
static DEESSER_PARAMS: [ParamInfo; 7] = [
    param(0, "Freq", "De-esser", 2000.0, 12_000.0, 6000.0, Hz, Log),
    param(1, "Threshold", "De-esser", -60.0, 0.0, -30.0, Db, Linear),
    param(2, "Range", "De-esser", 0.0, 24.0, 10.0, Db, Linear),
    param(
        3,
        "Mode",
        "De-esser",
        0.0,
        1.0,
        0.0,
        NONE,
        Choice(DEESS_MODES),
    ),
    param(
        4,
        "Detect",
        "De-esser",
        0.0,
        1.0,
        0.0,
        NONE,
        Choice(DETECTS),
    ),
    param(5, "Listen", "De-esser", 0.0, 1.0, 0.0, NONE, Toggle),
    meter(6, "Reduction", "De-esser", 0.0, 24.0, Db),
];

/// De-esser factory presets.
pub static DEESSER_PRESETS: &[Preset] = &[
    Preset {
        name: "Male vocal",
        values: &[(deesser::FREQ, 5000.0), (deesser::THRESHOLD, -26.0)],
    },
    Preset {
        name: "Female vocal",
        values: &[(deesser::FREQ, 7500.0), (deesser::THRESHOLD, -26.0)],
    },
    Preset {
        name: "Harsh hats",
        values: &[
            (deesser::FREQ, 9000.0),
            (deesser::THRESHOLD, -20.0),
            (deesser::RANGE, 6.0),
        ],
    },
];

/// A de-esser: turns the "s" and "t" sounds down when they jump out. It
/// listens to the band above Freq; in Relative detection it compares that
/// band with the whole signal, so the threshold works at any input level.
/// Split mode turns down only the band (the rest passes bit for bit
/// untouched in level), Wide the whole signal. Listen plays the band alone,
/// to find the frequency.
pub struct DeEsser {
    v: [f32; 7],
    rate: u32,
    detect: Svf,
    split: Svf,
    band_env: kit::Follower,
    full_env: kit::Follower,
    freq: Smooth,
    tick: u32,
    most: f32,
}

impl Default for DeEsser {
    fn default() -> Self {
        Self::new()
    }
}

impl DeEsser {
    pub fn new() -> DeEsser {
        let v = defaults(&DEESSER_PARAMS);
        let mut d = DeEsser {
            v,
            rate: 48_000,
            detect: Svf::default(),
            split: Svf::default(),
            band_env: kit::Follower::default(),
            full_env: kit::Follower::default(),
            freq: Smooth::new(v[0].log2()),
            tick: 0,
            most: 0.0,
        };
        d.setup(48_000);
        d
    }
    fn apply(&mut self, id: u16, v: f32) {
        if id == deesser::FREQ {
            self.freq.target = v.log2();
        }
    }
    fn set_filters(&mut self, hz: f32) {
        self.detect
            .set(SvfMode::HighPass, hz, 0.707, 0.0, self.rate);
        self.split.set(SvfMode::HighPass, hz, 0.707, 0.0, self.rate);
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let thr = self.v[deesser::THRESHOLD as usize];
        let range = self.v[deesser::RANGE as usize];
        let wide = self.v[deesser::MODE as usize] >= 0.5;
        let absolute = self.v[deesser::DETECT as usize] >= 0.5;
        let listen = self.v[deesser::LISTEN as usize] >= 0.5;
        for i in 0..l.len() {
            // The filters follow a gliding Freq every 16 frames.
            if self.tick == 0 && self.freq.value != self.freq.target {
                let hz = crate::fast_exp2(self.freq.value);
                self.set_filters(hz);
            }
            self.tick = (self.tick + 1) & 15;
            self.freq.glide();
            let key = 0.5 * (l[i] + r[i]);
            let band = self.detect.run(0, key);
            let b = fast_level_db(self.band_env.run(band));
            let over = if absolute {
                b - thr
            } else {
                // How far the band stands above the whole signal, against
                // the threshold taken as a level relative to it.
                let f = fast_level_db(self.full_env.run(key));
                (b - f) - (thr * 0.5)
            };
            let red = knee_reduction(over, DEESS_RATIO, 4.0).min(range);
            self.most = self.most.max(red);
            let g = fast_db_to_gain(-red);
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let hi = self.split.run(ch, *x);
                *x = if listen {
                    hi
                } else if wide {
                    *x * g
                } else {
                    (*x - hi) + hi * g
                };
            }
        }
        self.v[deesser::REDUCTION as usize] = self.most;
        self.most = 0.0;
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.band_env.set(0.5, 60.0, rate);
        self.full_env.set(0.5, 60.0, rate);
        self.freq.set_time(rate, 30.0);
        self.clear();
    }
    fn clear(&mut self) {
        self.freq.snap(self.freq.target);
        self.set_filters(self.v[deesser::FREQ as usize]);
        self.detect.reset();
        self.split.reset();
        self.band_env.reset();
        self.full_env.reset();
        self.tick = 0;
        self.most = 0.0;
        self.v[deesser::REDUCTION as usize] = 0.0;
    }
    fn tail(&self) -> u32 {
        0
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(DeEsser, DEESSER_INFO, DEESSER_PARAMS);

// ---------------------------------------------------------------- multiband

/// [`MultibandCompressor`] parameter ids.
pub mod multiband {
    pub const LOW_X: u16 = 0;
    pub const HIGH_X: u16 = 1;
    /// Band `b` (0 low, 1 mid, 2 high): threshold, ratio, gain.
    pub const fn threshold(b: usize) -> u16 {
        2 + 3 * b as u16
    }
    pub const fn ratio(b: usize) -> u16 {
        3 + 3 * b as u16
    }
    pub const fn gain(b: usize) -> u16 {
        4 + 3 * b as u16
    }
    pub const ATTACK: u16 = 11;
    pub const RELEASE: u16 = 12;
    pub const SOLO: u16 = 13;
    pub const MIX: u16 = 14;
    pub const OUTPUT: u16 = 15;
    /// Meters: band `b`'s reduction now, in dB.
    pub const fn reduction(b: usize) -> u16 {
        16 + b as u16
    }
    pub const BANDS: usize = 3;
}

const SOLOS: &[&str] = &["Off", "Low", "Mid", "High"];

static MULTIBAND_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.multiband",
    name: "Multiband comp",
    kind: Kind::Effect,
};
static MULTIBAND_PARAMS: [ParamInfo; 19] = [
    param(0, "Low/mid", "Bands", 40.0, 1000.0, 200.0, Hz, Log),
    param(1, "Mid/high", "Bands", 1000.0, 12_000.0, 2500.0, Hz, Log),
    param(2, "Low thresh", "Low", -60.0, 0.0, -18.0, Db, Linear),
    param(3, "Low ratio", "Low", 1.0, 20.0, 2.0, NONE, Log),
    param(4, "Low gain", "Low", -24.0, 24.0, 0.0, Db, Linear),
    param(5, "Mid thresh", "Mid", -60.0, 0.0, -18.0, Db, Linear),
    param(6, "Mid ratio", "Mid", 1.0, 20.0, 2.0, NONE, Log),
    param(7, "Mid gain", "Mid", -24.0, 24.0, 0.0, Db, Linear),
    param(8, "High thresh", "High", -60.0, 0.0, -18.0, Db, Linear),
    param(9, "High ratio", "High", 1.0, 20.0, 2.0, NONE, Log),
    param(10, "High gain", "High", -24.0, 24.0, 0.0, Db, Linear),
    param(11, "Attack", "Bands", 0.1, 100.0, 10.0, Ms, Log),
    param(12, "Release", "Bands", 10.0, 1000.0, 150.0, Ms, Log),
    param(13, "Solo", "Bands", 0.0, 3.0, 0.0, NONE, Choice(SOLOS)),
    param(14, "Mix", "Bands", 0.0, 100.0, 100.0, Percent, Linear),
    param(15, "Output", "Bands", -24.0, 24.0, 0.0, Db, Linear),
    meter(16, "Low red.", "Low", 0.0, 30.0, Db),
    meter(17, "Mid red.", "Mid", 0.0, 30.0, Db),
    meter(18, "High red.", "High", 0.0, 30.0, Db),
];

use multiband as mb;

/// Multiband compressor factory presets.
pub static MULTIBAND_PRESETS: &[Preset] = &[
    Preset {
        name: "Master glue",
        values: &[
            (mb::threshold(0), -20.0),
            (mb::ratio(0), 2.0),
            (mb::threshold(1), -18.0),
            (mb::ratio(1), 1.6),
            (mb::threshold(2), -20.0),
            (mb::ratio(2), 2.0),
            (mb::ATTACK, 20.0),
            (mb::RELEASE, 200.0),
        ],
    },
    Preset {
        name: "Tight low end",
        values: &[
            (mb::LOW_X, 120.0),
            (mb::threshold(0), -24.0),
            (mb::ratio(0), 4.0),
            (mb::gain(0), 2.0),
            (mb::ratio(1), 1.0),
            (mb::ratio(2), 1.0),
        ],
    },
    Preset {
        name: "Vocal smooth",
        values: &[
            (mb::LOW_X, 250.0),
            (mb::HIGH_X, 5000.0),
            (mb::threshold(1), -22.0),
            (mb::ratio(1), 3.0),
            (mb::threshold(2), -26.0),
            (mb::ratio(2), 4.0),
            (mb::ATTACK, 3.0),
        ],
    },
    Preset {
        name: "Drum crush",
        values: &[
            (mb::threshold(0), -30.0),
            (mb::ratio(0), 8.0),
            (mb::gain(0), 6.0),
            (mb::threshold(1), -30.0),
            (mb::ratio(1), 8.0),
            (mb::gain(1), 6.0),
            (mb::threshold(2), -30.0),
            (mb::ratio(2), 8.0),
            (mb::gain(2), 6.0),
            (mb::ATTACK, 1.0),
            (mb::RELEASE, 60.0),
        ],
    },
];

/// A three-band compressor: two Linkwitz-Riley crossovers split the sound,
/// each band has its own threshold, ratio and gain, and the bands sum back
/// flat (the low band passes an all-pass at the upper crossover, so all
/// three stay in phase). Attack and release are shared; Solo plays one
/// band alone while setting up.
pub struct MultibandCompressor {
    v: [f32; 19],
    rate: u32,
    lo_x: kit::Crossover,
    hi_x: kit::Crossover,
    /// The low band's all-pass (the upper crossover's sum).
    lo_ap: kit::CrossoverPhase,
    x_lo: Smooth,
    x_hi: Smooth,
    tick: u32,
    env: [kit::Follower; 3],
    makeup: [Smooth; 3],
    mix: Smooth,
    out: Smooth,
    most: [f32; 3],
}

impl Default for MultibandCompressor {
    fn default() -> Self {
        Self::new()
    }
}

impl MultibandCompressor {
    pub fn new() -> MultibandCompressor {
        let v = defaults(&MULTIBAND_PARAMS);
        let mut m = MultibandCompressor {
            v,
            rate: 48_000,
            lo_x: kit::Crossover::default(),
            hi_x: kit::Crossover::default(),
            lo_ap: kit::CrossoverPhase::default(),
            x_lo: Smooth::new(v[0].log2()),
            x_hi: Smooth::new(v[1].log2()),
            tick: 0,
            env: [kit::Follower::default(); 3],
            makeup: [Smooth::new(1.0), Smooth::new(1.0), Smooth::new(1.0)],
            mix: Smooth::new(1.0),
            out: Smooth::new(1.0),
            most: [0.0; 3],
        };
        m.setup(48_000);
        m
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            mb::LOW_X => self.x_lo.target = v.log2(),
            mb::HIGH_X => self.x_hi.target = v.log2(),
            mb::ATTACK | mb::RELEASE => self.set_times(),
            mb::MIX => self.mix.target = v / 100.0,
            mb::OUTPUT => self.out.target = db_to_gain(v),
            _ => {
                for b in 0..3 {
                    if id == mb::gain(b) {
                        self.makeup[b].target = db_to_gain(v);
                    }
                }
            }
        }
    }
    fn set_times(&mut self) {
        let (a, r) = (self.v[mb::ATTACK as usize], self.v[mb::RELEASE as usize]);
        for e in &mut self.env {
            e.set(a, r, self.rate);
        }
    }
    fn set_crossovers(&mut self) {
        // The upper crossover stays at least an octave over the lower.
        let lo = crate::fast_exp2(self.x_lo.value);
        let hi = crate::fast_exp2(self.x_hi.value).max(lo * 2.0);
        self.lo_x.set(lo, self.rate);
        self.hi_x.set(hi, self.rate);
        self.lo_ap.set(hi, self.rate);
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let solo = self.v[mb::SOLO as usize] as usize;
        let thr = [0, 1, 2].map(|b| self.v[mb::threshold(b) as usize]);
        let ratio = [0, 1, 2].map(|b| self.v[mb::ratio(b) as usize]);
        for i in 0..l.len() {
            if self.tick == 0
                && (self.x_lo.value != self.x_lo.target || self.x_hi.value != self.x_hi.target)
            {
                self.set_crossovers();
            }
            self.tick = (self.tick + 1) & 15;
            self.x_lo.glide();
            self.x_hi.glide();
            let x = [l[i], r[i]];
            let mut bands = [[0.0f32; 2]; 3];
            for ch in 0..2 {
                let (lo, rest) = self.lo_x.split(ch, x[ch]);
                let (mid, hi) = self.hi_x.split(ch, rest);
                bands[0][ch] = self.lo_ap.run(ch, lo);
                bands[1][ch] = mid;
                bands[2][ch] = hi;
            }
            let mut y = [0.0f32; 2];
            for b in 0..3 {
                let level = bands[b][0].abs().max(bands[b][1].abs());
                let db = fast_level_db(self.env[b].run(level));
                let red = knee_reduction(db - thr[b], ratio[b], 6.0);
                self.most[b] = self.most[b].max(red);
                let g = fast_db_to_gain(-red) * self.makeup[b].glide();
                if solo == 0 || solo == b + 1 {
                    y[0] += bands[b][0] * g;
                    y[1] += bands[b][1] * g;
                }
            }
            let mix = self.mix.glide();
            let out = self.out.glide();
            // Dry is the input itself: the mix is for parallel squash, so a
            // little phase difference between the two is part of the sound.
            l[i] = lerp(x[0], y[0], mix) * out;
            r[i] = lerp(x[1], y[1], mix) * out;
        }
        for b in 0..3 {
            self.v[mb::reduction(b) as usize] = self.most[b];
            self.most[b] = 0.0;
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.x_lo.set_time(rate, 30.0);
        self.x_hi.set_time(rate, 30.0);
        for s in self.makeup.iter_mut() {
            s.set_time(rate, 20.0);
        }
        self.mix.set_time(rate, 20.0);
        self.out.set_time(rate, 20.0);
        self.set_times();
        self.clear();
    }
    fn clear(&mut self) {
        self.x_lo.snap(self.x_lo.target);
        self.x_hi.snap(self.x_hi.target);
        self.set_crossovers();
        self.lo_x.reset();
        self.hi_x.reset();
        self.lo_ap.reset();
        for e in &mut self.env {
            e.reset();
        }
        for (b, s) in self.makeup.iter_mut().enumerate() {
            s.target = db_to_gain(self.v[mb::gain(b) as usize]);
            s.snap(s.target);
        }
        self.mix.target = self.v[mb::MIX as usize] / 100.0;
        self.mix.snap(self.mix.target);
        self.out.target = db_to_gain(self.v[mb::OUTPUT as usize]);
        self.out.snap(self.out.target);
        self.tick = 0;
        self.most = [0.0; 3];
        for b in 0..3 {
            self.v[mb::reduction(b) as usize] = 0.0;
        }
    }
    fn tail(&self) -> u32 {
        0
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(MultibandCompressor, MULTIBAND_INFO, MULTIBAND_PARAMS);
