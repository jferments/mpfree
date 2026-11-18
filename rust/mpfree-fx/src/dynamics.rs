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
use crate::limit::PeakLimit;
use crate::svf::{Svf, SvfMode};
use crate::{
    db_to_gain, defaults, fast_db_to_gain, fast_level_db, flush, gain_to_db, FixedDelay, Preset,
};

/// The soft-knee gain reduction in dB for `over` dB above the threshold at
/// `ratio`, with a knee `knee` dB wide.
#[inline]
fn knee_reduction(over: f32, ratio: f32, knee: f32) -> f32 {
    slope_reduction(over, ratio_slope(ratio), knee)
}

/// The curve's slope above the knee for `ratio`: the share of each dB
/// over the threshold that is taken off.
#[inline]
fn ratio_slope(ratio: f32) -> f32 {
    1.0 - 1.0 / ratio.max(1.0)
}

/// [`knee_reduction`] with the slope given (so it can glide).
#[inline]
fn slope_reduction(over: f32, slope: f32, knee: f32) -> f32 {
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
/// arrives, so nothing passes the ceiling and there is no overshoot. The
/// gain holds still for a moment after a peak (a quarter of Release, at
/// most 5 ms), so it doesn't ride inside bass cycles, then comes back in
/// decibels, 63 % of the way by Release. Auto release lets go at a third
/// of Release after a short peak and at three times it after a sustained
/// push, which keeps loud material from pumping. True peak also catches
/// the peaks between samples that a converter would make, as a 4x meter
/// reads them. See [`crate::limit`].
pub struct Limiter {
    v: [f32; 6],
    rate: u32,
    gain: Smooth,
    ceiling: Smooth,
    core: PeakLimit,
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
            gain: Smooth::new(db_to_gain(v[0])),
            ceiling: Smooth::new(db_to_gain(v[1])),
            core: PeakLimit::new(),
            least: 1.0,
        };
        l.setup(48_000);
        l
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            limiter::GAIN => self.gain.target = db_to_gain(v),
            limiter::CEILING => self.ceiling.target = db_to_gain(v),
            limiter::RELEASE | limiter::AUTO => self.core.set_release(
                self.v[limiter::RELEASE as usize],
                self.v[limiter::AUTO as usize] >= 0.5,
            ),
            limiter::TRUE_PEAK => self.core.set_true_peak(v >= 0.5),
            _ => {}
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        // A quiet piece with the limiter at rest needs no limiting, only
        // its delay (the same numbers as frame by frame).
        let steady = self.gain.settled() && self.ceiling.settled();
        if steady
            && self
                .core
                .quiet_block(l, r, self.gain.target, self.ceiling.target)
        {
            return self.show_reduction();
        }
        for i in 0..l.len() {
            let gain = self.gain.glide();
            let ceiling = self.ceiling.glide();
            let ([a, b], g) = self.core.run([l[i] * gain, r[i] * gain], ceiling);
            self.least = self.least.min(g);
            // The gain already keeps every frame under the ceiling; the
            // clamp only acts while the ceiling glides down.
            l[i] = (a * g).clamp(-ceiling, ceiling);
            r[i] = (b * g).clamp(-ceiling, ceiling);
        }
        self.show_reduction();
    }
    fn show_reduction(&mut self) {
        self.v[limiter::REDUCTION as usize] = -gain_to_db(self.least).min(0.0);
        self.least = 1.0;
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.gain.set_time(rate, 20.0);
        self.ceiling.set_time(rate, 20.0);
        self.core.set_release(
            self.v[limiter::RELEASE as usize],
            self.v[limiter::AUTO as usize] >= 0.5,
        );
        self.core
            .set_true_peak(self.v[limiter::TRUE_PEAK as usize] >= 0.5);
        self.core.setup(rate, LIMIT_AHEAD_MS);
        self.clear();
    }
    fn clear(&mut self) {
        self.core.clear();
        self.least = 1.0;
        self.v[limiter::REDUCTION as usize] = 0.0;
        self.gain.snap(self.gain.target);
        self.ceiling.snap(self.ceiling.target);
    }
    fn tail(&self) -> u32 {
        self.lat()
    }
    fn rest(&self) -> u32 {
        self.core.rest()
    }
    fn lat(&self) -> u32 {
        self.core.latency()
    }
}

node_impl!(Limiter, LIMITER_INFO, LIMITER_PARAMS, rest);

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
    /// Key listen: plays the key as the filters leave it, to set them by
    /// ear.
    pub const LISTEN: u16 = 9;
    /// Mode: 0 Gate (shuts to Range), 1 Expander (turns down by Ratio
    /// under the threshold, at most to Range).
    pub const MODE: u16 = 10;
    /// The expander's ratio: dB down per dB under the threshold, plus one
    /// (2: a sound 3 dB under comes out 6 dB under).
    pub const RATIO: u16 = 11;
}

const GATE_MODES: &[&str] = &["Gate", "Expander"];

/// The Sidechain switch's values: its own sound, or the strip the mixer
/// feeds it (`crate::sidechain`).
const KEYS: &[&str] = &["Off", "On"];
const GATE_AHEAD_MS: f32 = 1.0;
/// The gate shuts this far under the threshold it opens at, so a level
/// hovering at the threshold doesn't chatter.
const HYSTERESIS_DB: f32 = 4.0;
/// The shortest hold: the gate stays open at least this long after the key
/// last reached the level it shuts at, so the gaps between a bass note's
/// peaks (12.5 ms at 40 Hz) never shut it.
const GATE_MIN_HOLD_MS: f32 = 20.0;
/// How fast the gain rises to a Range raised while the gate is shut.
const GATE_RANGE_RISE_MS: f32 = 10.0;
/// The expander's level: each peak of the key is held for Hold (at least
/// 20 ms); a lower peak within this much of it takes over (so a fading
/// note is followed peak by peak), and with none it falls 60 dB in 20 ms.
const EXPAND_REARM_DB: f32 = -1.0;

static GATE_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.gate",
    name: "Gate",
    kind: Kind::Effect,
};
static GATE_PARAMS: [ParamInfo; 12] = [
    param(0, "Threshold", "Gate", -80.0, 0.0, -40.0, Db, Linear),
    param(1, "Range", "Gate", -80.0, 0.0, -80.0, Db, Linear),
    param(2, "Attack", "Gate", 0.1, 50.0, 0.5, Ms, Log),
    param(3, "Hold", "Gate", 0.0, 500.0, 30.0, Ms, Linear),
    param(4, "Release", "Gate", 5.0, 2000.0, 120.0, Ms, Log),
    param(5, "SC low cut", "Trigger", 20.0, 2000.0, 20.0, Hz, Log),
    param(
        6,
        "SC high cut",
        "Trigger",
        500.0,
        20_000.0,
        20_000.0,
        Hz,
        Log,
    ),
    param(7, "Sidechain", "Trigger", 0.0, 1.0, 0.0, NONE, Choice(KEYS)),
    meter(8, "Gain", "Gate", -80.0, 0.0, Db),
    param(9, "Listen", "Trigger", 0.0, 1.0, 0.0, NONE, Toggle),
    param(10, "Mode", "Gate", 0.0, 1.0, 0.0, NONE, Choice(GATE_MODES)),
    param(11, "Ratio", "Gate", 1.2, 10.0, 2.0, NONE, Log),
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
            (gate::MODE, 1.0),
            (gate::RATIO, 2.0),
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
/// the first edge survives, holds, then fades over Release. It opens when
/// the key reaches the threshold and shuts once the key hasn't reached
/// 4 dB under it for Hold (at least 20 ms, so a bass note's cycles read as
/// one steady level). The key can be filtered (so the kick's thump or the
/// hi-hat's hiss doesn't open it) or come from the sidechain, for rhythmic
/// gating; Listen plays it. As an expander it turns the sound down by
/// Ratio under the threshold instead (at most to Range), for a softer
/// clean-up.
pub struct Gate {
    v: [f32; 12],
    rate: u32,
    ahead: usize,
    delay: [kit::Line; 2],
    key_hp: Svf,
    key_lp: Svf,
    open_at: f32,
    shut_at: f32,
    open: bool,
    /// Frames since the key last reached `shut_at` while open.
    since: u32,
    hold_frames: u32,
    g: f32,
    floor: f32,
    att_step: f32,
    rel_mult: f32,
    /// The one-pole that lifts the gain to a Range raised while shut.
    rise: f32,
    /// The expander's held peak of the key, the frames it has left to
    /// hold, what it falls by each frame after, the key's last level and
    /// whether it was rising.
    peak: f32,
    peak_left: u32,
    peak_fall: f32,
    last: f32,
    rising: bool,
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
            open_at: 0.0,
            shut_at: 0.0,
            open: false,
            since: 0,
            hold_frames: 0,
            g: 0.0,
            floor: 0.0,
            att_step: 1.0,
            rel_mult: 0.0,
            rise: 1.0,
            peak: 0.0,
            peak_left: 0,
            peak_fall: 1.0,
            last: 0.0,
            rising: false,
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
        let depth = self.v[gate::RANGE as usize].clamp(-80.0, -0.01);
        self.rel_mult = db_to_gain(depth / nr);
        let hold = self.v[gate::HOLD as usize].max(GATE_MIN_HOLD_MS);
        self.hold_frames = (hold * 0.001 * fs) as u32;
        self.rise = 1.0 - (-1.0 / (GATE_RANGE_RISE_MS * 0.001 * fs)).exp();
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
        let listen = self.v[gate::LISTEN as usize] >= 0.5;
        let expand = self.v[gate::MODE as usize] >= 0.5;
        let thr = self.v[gate::THRESHOLD as usize];
        let slope = self.v[gate::RATIO as usize] - 1.0;
        let rearm = db_to_gain(EXPAND_REARM_DB);
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
            if expand {
                // The key's peaks, each held (a lower one within 1 dB
                // takes over), then falling.
                if self.rising && level < self.last && self.last >= self.peak * rearm {
                    self.peak = self.last;
                    self.peak_left = self.hold_frames;
                }
                self.rising = level > self.last;
                self.last = level;
                if level >= self.peak {
                    self.peak = level;
                    self.peak_left = self.hold_frames;
                } else if self.peak_left > 0 {
                    self.peak_left -= 1;
                } else {
                    self.peak = flush(self.peak * self.peak_fall);
                }
                // Under the threshold, Ratio's dB down per dB, at most to
                // Range; up at Attack's pace, down at Release's.
                let want = if self.peak >= self.open_at {
                    1.0
                } else {
                    let db = (fast_level_db(self.peak) - thr) * slope;
                    if db <= -80.0 {
                        self.floor
                    } else {
                        fast_db_to_gain(db).max(self.floor)
                    }
                };
                if self.g < want {
                    self.g = (self.g + self.att_step).min(want);
                } else if self.g > want {
                    self.g = (self.g * self.rel_mult).max(want);
                    if self.g < 1e-4 && want == 0.0 {
                        self.g = 0.0;
                    }
                }
                self.least = self.least.min(self.g);
                self.delay[0].push(l[i]);
                self.delay[1].push(r[i]);
                if listen {
                    l[i] = k[0];
                    r[i] = k[1];
                } else {
                    l[i] = self.delay[0].tap(self.ahead + 1) * self.g;
                    r[i] = self.delay[1].tap(self.ahead + 1) * self.g;
                }
                continue;
            }
            // Open at the threshold; shut once the key hasn't reached the
            // threshold less the hysteresis for Hold (at least 20 ms), so
            // the dips between a low note's peaks don't count.
            if level >= self.open_at {
                self.open = true;
                self.since = 0;
            } else if self.open {
                if level >= self.shut_at {
                    self.since = 0;
                } else {
                    self.since += 1;
                    if self.since > self.hold_frames {
                        self.open = false;
                    }
                }
            }
            if self.open {
                self.g = (self.g + self.att_step).min(1.0);
            } else if self.g > self.floor {
                self.g = (self.g * self.rel_mult).max(self.floor);
                if self.g < 1e-4 && self.floor == 0.0 {
                    self.g = 0.0;
                }
            } else if self.g < self.floor {
                // Range raised while shut: rise to it.
                self.g = (self.g + (self.floor - self.g) * self.rise).min(self.floor);
            }
            self.least = self.least.min(self.g);
            self.delay[0].push(l[i]);
            self.delay[1].push(r[i]);
            if listen {
                l[i] = k[0];
                r[i] = k[1];
            } else {
                l[i] = self.delay[0].tap(self.ahead + 1) * self.g;
                r[i] = self.delay[1].tap(self.ahead + 1) * self.g;
            }
        }
        self.v[gate::GAIN as usize] = gain_to_db(self.least).max(-80.0);
        self.least = 1.0;
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.peak_fall = db_to_gain(-60.0 / (0.02 * rate as f32));
        self.ahead = ((GATE_AHEAD_MS * 0.001 * rate as f32).round() as usize).max(1);
        self.delay = [
            kit::Line::new(self.ahead + 4),
            kit::Line::new(self.ahead + 4),
        ];
        self.derive();
        self.clear();
    }
    fn clear(&mut self) {
        for d in &mut self.delay {
            d.clear();
        }
        self.key_hp.reset();
        self.key_lp.reset();
        self.open = false;
        self.since = 0;
        self.peak = 0.0;
        self.peak_left = 0;
        self.last = 0.0;
        self.rising = false;
        self.g = self.floor;
        self.least = 1.0;
        self.v[gate::GAIN as usize] = 0.0;
    }
    fn tail(&self) -> u32 {
        self.ahead as u32
    }
    fn rest(&self) -> u32 {
        // Keyed, it follows the key through silence.
        if self.v[gate::KEY as usize] >= 0.5 {
            return u32::MAX;
        }
        // Hold (at least 20 ms) and Release take the gain down to Range,
        // where a reset puts it; 100 ms more for the key's filters to ring
        // out and a raised Range to be reached.
        let hold = self.v[gate::HOLD as usize].max(GATE_MIN_HOLD_MS);
        let ms = hold + self.v[gate::RELEASE as usize] + 100.0;
        (ms * 0.001 * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        self.ahead as u32
    }

    /// For the Channel strip while its gate is switched off: keeps the
    /// look-ahead filled with the sound passing by, so the gate comes back
    /// on without a gap.
    pub(crate) fn feed(&mut self, l: &[f32], r: &[f32]) {
        for (a, b) in l.iter().zip(r) {
            self.delay[0].push(*a);
            self.delay[1].push(*b);
        }
    }

    /// For the Channel strip switching its gate back on: open, as the
    /// sound passed while it was off, and shutting from there over Hold
    /// and Release if the key is quiet.
    pub(crate) fn resume_open(&mut self) {
        self.key_hp.reset();
        self.key_lp.reset();
        self.open = true;
        self.since = 0;
        self.g = 1.0;
        self.least = 1.0;
    }
}

node_impl!(Gate, GATE_INFO, GATE_PARAMS, rest);

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

/// The de-esser's look-ahead (decision 11.2 of FX quality round 4): the
/// sound is held back this long, so the gain is already down when an s
/// arrives.
const DEESS_AHEAD_MS: f32 = 2.0;

/// The split sits this far under Freq (a sixth of an octave), so at full
/// reduction the cut is within a dB of Range from 1.5 times Freq up.
const DEESS_SPLIT: f32 = 0.9;

/// A de-esser: turns the "s" and "t" sounds down when they jump out. It
/// listens to the band above Freq on the louder side, so an s panned to
/// one side is caught as in the centre; in Relative detection it compares
/// that band with the whole signal, so the threshold works at any input
/// level. The sound is held back 2 ms, so the gain is down by the time
/// each s arrives. Split mode turns down the band above an eighth-order
/// Linkwitz-Riley split just under Freq, whose bands are in phase, so
/// nothing under it is ever lifted; Wide turns the whole signal down.
/// Both pass the split's all-pass (flat, the phase turned around the
/// split), so switching modes at rest doesn't click. Listen plays the
/// band alone, to find the frequency.
pub struct DeEsser {
    v: [f32; 7],
    rate: u32,
    /// The detector's high-pass at Freq, per side.
    detect: kit::HighPass2,
    split: kit::Split8,
    ahead: [FixedDelay; 2],
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
            detect: kit::HighPass2::default(),
            split: kit::Split8::default(),
            ahead: [FixedDelay::default(), FixedDelay::default()],
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
        self.detect.set(hz, self.rate);
        self.split.set(hz * DEESS_SPLIT, self.rate);
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let thr = self.v[deesser::THRESHOLD as usize];
        let range = self.v[deesser::RANGE as usize];
        let wide = self.v[deesser::MODE as usize] >= 0.5;
        let absolute = self.v[deesser::DETECT as usize] >= 0.5;
        let listen = self.v[deesser::LISTEN as usize] >= 0.5;
        for i in 0..l.len() {
            // The filters follow a gliding Freq every 16 frames, and land
            // on it the frame the glide ends (as a reset would put them).
            let gliding = self.freq.value != self.freq.target;
            if gliding && self.tick == 0 {
                let hz = crate::fast_exp2(self.freq.value);
                self.set_filters(hz);
            }
            self.tick = (self.tick + 1) & 15;
            self.freq.glide();
            if gliding && self.freq.value == self.freq.target {
                self.set_filters(self.v[deesser::FREQ as usize]);
            }
            // Detection on the sound as it comes in, 2 ms ahead of what
            // is heard, from the louder side.
            let x = [l[i], r[i]];
            let band = self.detect.run(x);
            let band = band[0].abs().max(band[1].abs());
            let b = fast_level_db(self.band_env.run(band));
            let over = if absolute {
                b - thr
            } else {
                // How far the band stands above the whole signal, against
                // the threshold taken as a level relative to it.
                let f = fast_level_db(self.full_env.run(x[0].abs().max(x[1].abs())));
                (b - f) - (thr * 0.5)
            };
            let red = knee_reduction(over, DEESS_RATIO, 4.0).min(range);
            self.most = self.most.max(red);
            let g = fast_db_to_gain(-red);
            let d = [self.ahead[0].run(x[0]), self.ahead[1].run(x[1])];
            let (ap, lo) = self.split.split(d);
            for (ch, y) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                *y = if listen {
                    ap[ch] - lo[ch]
                } else if wide {
                    ap[ch] * g
                } else {
                    // The low band plus the high band at the gain.
                    g * ap[ch] + (1.0 - g) * lo[ch]
                };
            }
        }
        self.v[deesser::REDUCTION as usize] = self.most;
        self.most = 0.0;
    }
    fn frames_ahead(&self) -> usize {
        ((DEESS_AHEAD_MS * 0.001 * self.rate as f32).round() as usize).max(1)
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let n = self.frames_ahead();
        self.ahead = [FixedDelay::new(n), FixedDelay::new(n)];
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
        for d in &mut self.ahead {
            d.reset();
        }
        self.band_env.reset();
        self.full_env.reset();
        self.tick = 0;
        self.most = 0.0;
        self.v[deesser::REDUCTION as usize] = 0.0;
    }
    fn tail(&self) -> u32 {
        0
    }
    fn rest(&self) -> u32 {
        // Ten of the followers' 60 ms release, the filters settling and
        // the look-ahead.
        (0.65 * self.rate as f32) as u32 + self.frames_ahead() as u32
    }
    fn lat(&self) -> u32 {
        self.frames_ahead() as u32
    }
}

node_impl!(DeEsser, DEESSER_INFO, DEESSER_PARAMS, rest);

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
    /// Adds back what each band's curve takes from a mix-level sound.
    pub const AUTO_MAKEUP: u16 = 19;
    pub const BANDS: usize = 3;
}

const SOLOS: &[&str] = &["Off", "Low", "Mid", "High"];

static MULTIBAND_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.multiband",
    name: "Multiband comp",
    kind: Kind::Effect,
};
static MULTIBAND_PARAMS: [ParamInfo; 20] = [
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
    param(19, "Auto makeup", "Bands", 0.0, 1.0, 0.0, NONE, Toggle),
];

/// The bands' knee, dB.
const MB_KNEE: f32 = 6.0;

/// Where Auto makeup reads each band's curve: about the peak level the
/// detector reads from pink noise at -18 dBFS RMS (a mix level), less the
/// band's share of it by octaves. A dB under the Compressor's -10, which
/// left the heavy ratios (Drum crush's 8:1) 1.7 LU loud on pink.
const AUTO_REF_DB: f32 = -11.0;

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
            (mb::AUTO_MAKEUP, 1.0),
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
            (mb::AUTO_MAKEUP, 1.0),
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
            (mb::AUTO_MAKEUP, 1.0),
        ],
    },
    Preset {
        name: "Drum crush",
        values: &[
            (mb::threshold(0), -30.0),
            (mb::ratio(0), 8.0),
            (mb::threshold(1), -30.0),
            (mb::ratio(1), 8.0),
            (mb::threshold(2), -30.0),
            (mb::ratio(2), 8.0),
            (mb::ATTACK, 1.0),
            (mb::RELEASE, 60.0),
            (mb::AUTO_MAKEUP, 1.0),
        ],
    },
];

/// A three-band compressor: two Linkwitz-Riley crossovers split the sound,
/// each band has its own threshold, ratio and gain, and the bands sum back
/// flat (the low band passes an all-pass at the upper crossover, so all
/// three stay in phase). The dry is the bands' sum at unity (the input
/// through the crossovers' all-passes), so any Mix adds the two in phase.
/// Each band's detector is the
/// Compressor's (the smooth decoupled peak detector on the reduction in
/// decibels, its release solved through the attack), so the curve, Attack
/// and Release mean what they say. Thresholds and ratios glide. Attack and
/// release are shared; Solo plays one band alone while setting up; Auto
/// makeup adds back what each band's curve takes from a mix-level sound.
pub struct MultibandCompressor {
    v: [f32; 20],
    rate: u32,
    lo_x: kit::Crossover,
    hi_x: kit::Crossover,
    /// The low band's all-pass (the upper crossover's sum).
    lo_ap: kit::CrossoverPhase,
    x_lo: Smooth,
    x_hi: Smooth,
    tick: u32,
    /// Per band: the threshold (dB) and the curve's slope (1 - 1/ratio).
    thr: [Smooth; 3],
    slope: [Smooth; 3],
    /// Per band, in positive dB of reduction: the detector's hold (up at
    /// once, down at the release) and the attack's smoothing after it.
    hold: [f32; 3],
    env: [f32; 3],
    att: f32,
    rel: f32,
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
        let thr = |b: usize| Smooth::new(v[mb::threshold(b) as usize]);
        let slope = |b: usize| Smooth::new(ratio_slope(v[mb::ratio(b) as usize]));
        let mut m = MultibandCompressor {
            v,
            rate: 48_000,
            lo_x: kit::Crossover::default(),
            hi_x: kit::Crossover::default(),
            lo_ap: kit::CrossoverPhase::default(),
            x_lo: Smooth::new(v[0].log2()),
            x_hi: Smooth::new(v[1].log2()),
            tick: 0,
            thr: [thr(0), thr(1), thr(2)],
            slope: [slope(0), slope(1), slope(2)],
            hold: [0.0; 3],
            env: [0.0; 3],
            att: 0.0,
            rel: 0.0,
            makeup: [Smooth::new(1.0), Smooth::new(1.0), Smooth::new(1.0)],
            mix: Smooth::new(1.0),
            out: Smooth::new(1.0),
            most: [0.0; 3],
        };
        m.setup(48_000);
        m
    }

    /// Band `b`'s edges, Hz: 20 Hz, the two crossovers (the upper at least
    /// an octave over the lower), 20 kHz.
    fn edges(&self, b: usize) -> (f32, f32) {
        let lo = self.v[mb::LOW_X as usize];
        let hi = self.v[mb::HIGH_X as usize].max(2.0 * lo);
        match b {
            0 => (20.0, lo),
            1 => (lo, hi),
            _ => (hi, 20_000.0),
        }
    }

    /// Auto makeup for band `b`, dB (0 when it is off): what the band's
    /// curve takes at its share of a mix-level sound, the share by octaves
    /// (pink noise has the same power in each).
    pub fn auto_db(&self, b: usize) -> f32 {
        if self.v[mb::AUTO_MAKEUP as usize] < 0.5 {
            return 0.0;
        }
        let (lo, hi) = self.edges(b);
        let share = (hi / lo).log2() / (20_000.0f32 / 20.0).log2();
        let at = AUTO_REF_DB + 10.0 * share.log10();
        knee_reduction(
            at - self.v[mb::threshold(b) as usize],
            self.v[mb::ratio(b) as usize],
            MB_KNEE,
        )
    }

    fn set_makeup(&mut self) {
        for b in 0..mb::BANDS {
            self.makeup[b].target = db_to_gain(self.v[mb::gain(b) as usize] + self.auto_db(b));
        }
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            mb::LOW_X => self.x_lo.target = v.log2(),
            mb::HIGH_X => self.x_hi.target = v.log2(),
            mb::ATTACK | mb::RELEASE => self.set_times(),
            mb::MIX => self.mix.target = v / 100.0,
            mb::OUTPUT => self.out.target = db_to_gain(v),
            _ => {
                for b in 0..mb::BANDS {
                    if id == mb::threshold(b) {
                        self.thr[b].target = v;
                    } else if id == mb::ratio(b) {
                        self.slope[b].target = ratio_slope(v);
                    }
                }
            }
        }
        // A gain, Auto makeup, a threshold, a ratio or a crossover moves
        // what each band gets back.
        self.set_makeup();
    }

    fn set_times(&mut self) {
        let fs = self.rate as f32;
        let attack = self.v[mb::ATTACK as usize] * 0.001;
        let release = self.v[mb::RELEASE as usize] * 0.001;
        self.att = (-1.0 / (attack * fs)).exp();
        self.rel = (-1.0 / (crate::comp::release_tau(attack, false, release) * fs)).exp();
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
        let (att, rel) = (self.att, self.rel);
        for i in 0..l.len() {
            // The crossovers follow gliding frequencies every 16 frames,
            // and land on them the frame the glides end (as a reset would
            // put them).
            let gliding =
                self.x_lo.value != self.x_lo.target || self.x_hi.value != self.x_hi.target;
            if gliding && self.tick == 0 {
                self.set_crossovers();
            }
            self.tick = (self.tick + 1) & 15;
            self.x_lo.glide();
            self.x_hi.glide();
            if gliding && self.x_lo.value == self.x_lo.target && self.x_hi.value == self.x_hi.target
            {
                self.set_crossovers();
            }
            let x = [l[i], r[i]];
            let mut bands = [[0.0f32; 2]; 3];
            for ch in 0..2 {
                let (lo, rest) = self.lo_x.split(ch, x[ch]);
                let (mid, hi) = self.hi_x.split(ch, rest);
                bands[0][ch] = self.lo_ap.run(ch, lo);
                bands[1][ch] = mid;
                bands[2][ch] = hi;
            }
            // The dry is the bands at unity, so each band's weight is its
            // gain mixed with 1 (a band soloed out keeps only the dry's).
            let mix = self.mix.glide();
            let dry = 1.0 - mix;
            let mut y = [0.0f32; 2];
            for b in 0..3 {
                let level = bands[b][0].abs().max(bands[b][1].abs());
                let t = self.thr[b].glide();
                let k = self.slope[b].glide();
                let want = slope_reduction(fast_level_db(level) - t, k, MB_KNEE);
                // The Compressor's detector: the hold jumps to what the
                // curve asks for and lets go at the release, toward it; the
                // attack smooths the hold.
                self.hold[b] = flush(want.max(rel * self.hold[b] + (1.0 - rel) * want));
                self.env[b] = flush(att * self.env[b] + (1.0 - att) * self.hold[b]);
                let red = self.env[b];
                self.most[b] = self.most[b].max(red);
                let g = fast_db_to_gain(-red) * self.makeup[b].glide();
                let w = if solo == 0 || solo == b + 1 {
                    dry + mix * g
                } else {
                    dry
                };
                y[0] += bands[b][0] * w;
                y[1] += bands[b][1] * w;
            }
            let out = self.out.glide();
            l[i] = y[0] * out;
            r[i] = y[1] * out;
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
        for s in self
            .makeup
            .iter_mut()
            .chain(self.thr.iter_mut())
            .chain(self.slope.iter_mut())
        {
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
        self.hold = [0.0; 3];
        self.env = [0.0; 3];
        self.set_makeup();
        for b in 0..mb::BANDS {
            self.thr[b].snap(self.v[mb::threshold(b) as usize]);
            self.slope[b].snap(ratio_slope(self.v[mb::ratio(b) as usize]));
            let m = self.makeup[b].target;
            self.makeup[b].snap(m);
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

    fn rest(&self) -> u32 {
        // Ten of the bands' release, and the crossovers settling.
        let ms = 10.0 * self.v[mb::RELEASE as usize] + 100.0;
        (ms * 0.001 * self.rate as f32) as u32
    }

    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(MultibandCompressor, MULTIBAND_INFO, MULTIBAND_PARAMS, rest);
