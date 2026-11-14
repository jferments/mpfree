//! Two more echoes: [`TapeDelay`] (a tape loop with up to three playback
//! heads, wow, flutter, saturation and the loss each pass adds) and
//! [`MultiTap`] (four tempo taps with their own level and pan).
//!
//! Both read their line with cubic interpolation and glide their times, so
//! a time change bends the pitch the way tape does instead of clicking.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{self, node_impl, param, Choice, Hz, Line, Linear, Log, Ms, Percent, NONE};
use crate::svf::{Svf, SvfMode};
use crate::{defaults, fast_tanh, flush, lerp, Preset, Rng};

/// The longest echo either keeps, in seconds.
const MAX_SECONDS: f32 = 4.0;

// ---------------------------------------------------------------- tape

/// [`TapeDelay`] parameter ids.
pub mod tape {
    pub const TIME: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const FEEDBACK: u16 = 2;
    pub const HEADS: u16 = 3;
    pub const WOW: u16 = 4;
    pub const FLUTTER: u16 = 5;
    pub const SATURATION: u16 = 6;
    pub const TONE: u16 = 7;
    pub const LOW_CUT: u16 = 8;
    pub const SPREAD: u16 = 9;
    pub const MIX: u16 = 10;
}

const TAPE_SYNCS: &[&str] = &[
    "Free", "1/16", "1/8T", "1/8", "1/8D", "1/4T", "1/4", "1/4D", "1/2", "1 bar",
];
const TAPE_SYNC_BEATS: [f32; 10] = [
    0.0,
    0.25,
    1.0 / 3.0,
    0.5,
    0.75,
    2.0 / 3.0,
    1.0,
    1.5,
    2.0,
    4.0,
];
const HEADS: &[&str] = &["One", "Two", "Three"];
/// The heads sit at these multiples of Time (the spacing of a 1970s
/// three-head echo).
const HEAD_AT: [f32; 3] = [1.0, 1.95, 2.9];

static TAPE_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.tape_delay",
    name: "Tape delay",
    kind: Kind::Effect,
};
static TAPE_PARAMS: [ParamInfo; 11] = [
    param(0, "Time", "Echo", 20.0, 1300.0, 330.0, Ms, Log),
    param(1, "Sync", "Echo", 0.0, 9.0, 0.0, NONE, Choice(TAPE_SYNCS)),
    param(2, "Feedback", "Echo", 0.0, 110.0, 45.0, Percent, Linear),
    param(3, "Heads", "Echo", 0.0, 2.0, 0.0, NONE, Choice(HEADS)),
    param(4, "Wow", "Tape", 0.0, 100.0, 25.0, Percent, Linear),
    param(5, "Flutter", "Tape", 0.0, 100.0, 25.0, Percent, Linear),
    param(6, "Saturation", "Tape", 0.0, 100.0, 35.0, Percent, Linear),
    param(7, "Tone", "Tape", 800.0, 16_000.0, 5000.0, Hz, Log),
    param(8, "Low cut", "Tape", 20.0, 800.0, 120.0, Hz, Log),
    param(9, "Spread", "Echo", 0.0, 100.0, 50.0, Percent, Linear),
    param(10, "Mix", "Echo", 0.0, 100.0, 30.0, Percent, Linear),
];

/// Tape delay factory presets.
pub static TAPE_PRESETS: &[Preset] = &[
    Preset {
        name: "Slapback",
        values: &[
            (tape::TIME, 110.0),
            (tape::FEEDBACK, 10.0),
            (tape::WOW, 10.0),
        ],
    },
    Preset {
        name: "Dub echo",
        values: &[
            (tape::SYNC, 4.0),
            (tape::FEEDBACK, 80.0),
            (tape::TONE, 2500.0),
            (tape::SATURATION, 55.0),
        ],
    },
    Preset {
        name: "Space heads",
        values: &[
            (tape::TIME, 180.0),
            (tape::HEADS, 2.0),
            (tape::FEEDBACK, 60.0),
        ],
    },
    Preset {
        name: "Worn tape",
        values: &[
            (tape::WOW, 80.0),
            (tape::FLUTTER, 70.0),
            (tape::TONE, 2000.0),
        ],
    },
    Preset {
        name: "Runaway",
        values: &[
            (tape::FEEDBACK, 105.0),
            (tape::SATURATION, 70.0),
            (tape::SYNC, 6.0),
        ],
    },
];

/// A tape echo: the sound goes onto a loop of tape and comes back from up
/// to three playback heads. Each pass loses some top (Tone) and bottom
/// (Low cut) and is saturated, so the repeats darken and thicken; the
/// saturation also keeps Feedback over 100 % from running away, so it
/// can howl and stay musical. Wow (a slow, wandering speed drift) and
/// flutter (a fast, small one) move the pitch; a time change slides the
/// pitch as the motor changes speed.
pub struct TapeDelay {
    v: [f32; 11],
    rate: u32,
    line: Line,
    time: Smooth,
    bpm: f64,
    feedback: Smooth,
    wow: Smooth,
    flutter: Smooth,
    sat: Smooth,
    spread: Smooth,
    mix: Smooth,
    tone: Svf,
    low_cut: Svf,
    bump: Svf,
    fb_sample: f32,
    wow_phase: f32,
    wow_inc: f32,
    flut_phase: [f32; 2],
    flut_inc: [f32; 2],
    drift: f32,
    drift_to: f32,
    rng: Rng,
}

impl Default for TapeDelay {
    fn default() -> Self {
        Self::new()
    }
}

impl TapeDelay {
    pub fn new() -> TapeDelay {
        let v = defaults(&TAPE_PARAMS);
        let mut t = TapeDelay {
            v,
            rate: 48_000,
            line: Line::default(),
            time: Smooth::new(v[0]),
            bpm: 120.0,
            feedback: Smooth::new(v[2] / 100.0),
            wow: Smooth::new(0.0),
            flutter: Smooth::new(0.0),
            sat: Smooth::new(0.0),
            spread: Smooth::new(0.0),
            mix: Smooth::new(0.3),
            tone: Svf::default(),
            low_cut: Svf::default(),
            bump: Svf::default(),
            fb_sample: 0.0,
            wow_phase: 0.0,
            wow_inc: 0.0,
            flut_phase: [0.0; 2],
            flut_inc: [0.0; 2],
            drift: 0.0,
            drift_to: 0.0,
            rng: Rng::new(0x7A9E),
        };
        t.setup(48_000);
        t
    }

    fn time_target(&self) -> f32 {
        let sync = self.v[tape::SYNC as usize] as usize;
        if sync > 0 {
            (TAPE_SYNC_BEATS[sync.min(9)] * 60_000.0 / self.bpm as f32).min(1300.0)
        } else {
            self.v[tape::TIME as usize]
        }
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            tape::TIME | tape::SYNC => self.time.target = self.time_target(),
            tape::FEEDBACK => self.feedback.target = v / 100.0,
            tape::WOW => self.wow.target = v / 100.0,
            tape::FLUTTER => self.flutter.target = v / 100.0,
            tape::SATURATION => self.sat.target = v / 100.0,
            tape::TONE => self.tone.set(SvfMode::LowPass, v, 0.6, 0.0, self.rate),
            tape::LOW_CUT => self.low_cut.set(SvfMode::HighPass, v, 0.6, 0.0, self.rate),
            tape::SPREAD => self.spread.target = v / 100.0,
            tape::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    fn begin(&mut self, ctx: &Ctx) {
        if ctx.bpm > 0.0 && ctx.bpm != self.bpm {
            self.bpm = ctx.bpm;
            self.time.target = self.time_target();
        }
    }

    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let ms = self.rate as f32 * 0.001;
        let heads = (self.v[tape::HEADS as usize] as usize).min(2) + 1;
        let per_head = 1.0 / heads as f32;
        let top = self.line.max() - 2.0;
        for i in 0..l.len() {
            let time = self.time.next();
            let fb = self.feedback.next();
            let wow = self.wow.next();
            let flutter = self.flutter.next();
            let sat = self.sat.next();
            let spread = self.spread.next();
            // Wow: a slow sine plus a drift that wanders to a new random
            // target each cycle. Flutter: two fast, small sines.
            self.wow_phase += self.wow_inc;
            if self.wow_phase >= 1.0 {
                self.wow_phase -= 1.0;
                self.drift_to = self.rng.bipolar();
            }
            self.drift += (self.drift_to - self.drift) * 0.000_02;
            self.flut_phase[0] = kit::wrap(self.flut_phase[0] + self.flut_inc[0]);
            self.flut_phase[1] = kit::wrap(self.flut_phase[1] + self.flut_inc[1]);
            let wobble = wow * 1.8 * ms * (0.6 * kit::sine(self.wow_phase) + 0.4 * self.drift)
                + flutter
                    * 0.12
                    * ms
                    * (kit::sine(self.flut_phase[0]) + 0.5 * kit::sine(self.flut_phase[1]));

            let mut sum = 0.0;
            let mut side = [0.0f32; 2];
            for (h, &at) in HEAD_AT.iter().enumerate().take(heads) {
                let d = (time * at * ms + wobble).clamp(2.0, top);
                let y = self.line.read(d - 1.0);
                sum += y;
                // Spread pans the heads apart: one left, one right, the
                // third in the middle.
                let pan = match (heads, h) {
                    (1, _) => 0.0,
                    (_, 0) => -spread,
                    (_, 1) => spread,
                    _ => 0.0,
                };
                side[0] += y * (1.0 - pan.max(0.0));
                side[1] += y * (1.0 + pan.min(0.0));
            }
            let echo = sum * per_head;
            // The loop: tone, low cut, a little head bump, saturation.
            let mut f = self.tone.run(0, echo);
            f = self.low_cut.run(0, f);
            f = self.bump.run(0, f);
            let drive = 1.0 + 4.0 * sat;
            f = lerp(f, fast_tanh(f * drive) / drive.sqrt(), sat);
            self.fb_sample = flush(f * fb);
            // Write the input plus feedback onto the tape (soft, so
            // over-unity feedback saturates instead of climbing).
            let input = 0.5 * (l[i] + r[i]);
            let w = input + self.fb_sample;
            self.line.push(if w.abs() > 1.0 { fast_tanh(w) } else { w });

            let mix = self.mix.next();
            let s = per_head;
            l[i] = lerp(l[i], side[0] * s, mix);
            r[i] = lerp(r[i], side[1] * s, mix);
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.line = Line::new((MAX_SECONDS * rate as f32) as usize);
        // A slow motor glide: half a second to settle on a new time.
        self.time.set_time(rate, 150.0);
        for s in [
            &mut self.feedback,
            &mut self.wow,
            &mut self.flutter,
            &mut self.sat,
            &mut self.spread,
            &mut self.mix,
        ] {
            s.set_time(rate, 30.0);
        }
        self.wow_inc = 0.7 / rate as f32;
        self.flut_inc = [6.3 / rate as f32, 11.7 / rate as f32];
        self.bump.set(SvfMode::Bell, 110.0, 0.8, 2.0, rate);
        for id in 0..TAPE_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.line.clear();
        self.tone.reset();
        self.low_cut.reset();
        self.bump.reset();
        self.fb_sample = 0.0;
        self.wow_phase = 0.0;
        self.flut_phase = [0.0, 0.3];
        self.drift = 0.0;
        self.drift_to = 0.0;
        self.rng = Rng::new(0x7A9E);
        for s in [
            &mut self.time,
            &mut self.feedback,
            &mut self.wow,
            &mut self.flutter,
            &mut self.sat,
            &mut self.spread,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
    }

    fn tail(&self) -> u32 {
        // Feedback can ring for a long time; the host's silence check ends
        // it sooner when it has died away.
        (MAX_SECONDS * 2.0 * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(TapeDelay, TAPE_INFO, TAPE_PARAMS);

// ---------------------------------------------------------------- multi-tap

/// [`MultiTap`] parameter ids.
pub mod multitap {
    pub const TAPS: usize = 4;
    /// Tap `t`'s time in sixteenth notes.
    pub const fn time(t: usize) -> u16 {
        3 * t as u16
    }
    pub const fn level(t: usize) -> u16 {
        1 + 3 * t as u16
    }
    pub const fn pan(t: usize) -> u16 {
        2 + 3 * t as u16
    }
    pub const FEEDBACK: u16 = 12;
    pub const TONE: u16 = 13;
    pub const LOW_CUT: u16 = 14;
    pub const MIX: u16 = 15;
}

static MULTITAP_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.multitap",
    name: "Multi-tap delay",
    kind: Kind::Effect,
};
use mpfree_node::Curve;
static MULTITAP_PARAMS: [ParamInfo; 16] = [
    param(
        0,
        "Tap 1 time",
        "Tap 1",
        1.0,
        32.0,
        3.0,
        mpfree_node::Unit::None,
        Curve::Stepped,
    ),
    param(1, "Tap 1 level", "Tap 1", 0.0, 100.0, 80.0, Percent, Linear),
    param(
        2,
        "Tap 1 pan",
        "Tap 1",
        -100.0,
        100.0,
        -60.0,
        Percent,
        Linear,
    ),
    param(
        3,
        "Tap 2 time",
        "Tap 2",
        1.0,
        32.0,
        6.0,
        mpfree_node::Unit::None,
        Curve::Stepped,
    ),
    param(4, "Tap 2 level", "Tap 2", 0.0, 100.0, 65.0, Percent, Linear),
    param(
        5,
        "Tap 2 pan",
        "Tap 2",
        -100.0,
        100.0,
        60.0,
        Percent,
        Linear,
    ),
    param(
        6,
        "Tap 3 time",
        "Tap 3",
        1.0,
        32.0,
        10.0,
        mpfree_node::Unit::None,
        Curve::Stepped,
    ),
    param(7, "Tap 3 level", "Tap 3", 0.0, 100.0, 50.0, Percent, Linear),
    param(
        8,
        "Tap 3 pan",
        "Tap 3",
        -100.0,
        100.0,
        -30.0,
        Percent,
        Linear,
    ),
    param(
        9,
        "Tap 4 time",
        "Tap 4",
        1.0,
        32.0,
        16.0,
        mpfree_node::Unit::None,
        Curve::Stepped,
    ),
    param(
        10,
        "Tap 4 level",
        "Tap 4",
        0.0,
        100.0,
        40.0,
        Percent,
        Linear,
    ),
    param(
        11,
        "Tap 4 pan",
        "Tap 4",
        -100.0,
        100.0,
        30.0,
        Percent,
        Linear,
    ),
    param(12, "Feedback", "Echo", 0.0, 95.0, 30.0, Percent, Linear),
    param(13, "Tone", "Echo", 500.0, 20_000.0, 8000.0, Hz, Log),
    param(14, "Low cut", "Echo", 20.0, 1000.0, 80.0, Hz, Log),
    param(15, "Mix", "Echo", 0.0, 100.0, 30.0, Percent, Linear),
];

use multitap as mt;

/// Multi-tap factory presets.
pub static MULTITAP_PRESETS: &[Preset] = &[
    Preset {
        name: "Dotted bounce",
        values: &[
            (mt::time(0), 3.0),
            (mt::time(1), 6.0),
            (mt::time(2), 9.0),
            (mt::time(3), 12.0),
        ],
    },
    Preset {
        name: "Triplet scatter",
        values: &[
            (mt::time(0), 2.0),
            (mt::time(1), 5.0),
            (mt::time(2), 7.0),
            (mt::time(3), 11.0),
            (mt::FEEDBACK, 45.0),
        ],
    },
    Preset {
        name: "Wide ping",
        values: &[
            (mt::pan(0), -100.0),
            (mt::pan(1), 100.0),
            (mt::pan(2), -100.0),
            (mt::pan(3), 100.0),
            (mt::time(0), 4.0),
            (mt::time(1), 8.0),
            (mt::time(2), 12.0),
        ],
    },
    Preset {
        name: "Build up",
        values: &[
            (mt::level(0), 30.0),
            (mt::level(1), 50.0),
            (mt::level(2), 70.0),
            (mt::level(3), 90.0),
            (mt::FEEDBACK, 60.0),
        ],
    },
];

/// Four echoes on the beat grid, each at its own time (1 to 32 sixteenth
/// notes at the song's tempo), level and pan; the last tap feeds back
/// through a tone and low cut for trails. Rhythms a single delay can't
/// make, and every ping-pong pattern.
pub struct MultiTap {
    v: [f32; 16],
    rate: u32,
    line: Line,
    bpm: f64,
    times: [Smooth; 4],
    levels: [Smooth; 4],
    pans: [Smooth; 4],
    feedback: Smooth,
    mix: Smooth,
    tone: Svf,
    low_cut: Svf,
    fb_sample: f32,
}

impl Default for MultiTap {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiTap {
    pub fn new() -> MultiTap {
        let v = defaults(&MULTITAP_PARAMS);
        let s = |x: f32| Smooth::new(x);
        let mut m = MultiTap {
            v,
            rate: 48_000,
            line: Line::default(),
            bpm: 120.0,
            times: [s(0.0), s(0.0), s(0.0), s(0.0)],
            levels: [s(0.0), s(0.0), s(0.0), s(0.0)],
            pans: [s(0.0), s(0.0), s(0.0), s(0.0)],
            feedback: s(0.0),
            mix: s(0.3),
            tone: Svf::default(),
            low_cut: Svf::default(),
            fb_sample: 0.0,
        };
        m.setup(48_000);
        m
    }

    /// Tap `t`'s time in frames at the tempo now.
    fn frames(&self, t: usize) -> f32 {
        let sixteenths = self.v[mt::time(t) as usize];
        let secs = sixteenths * 0.25 * 60.0 / self.bpm as f32;
        (secs * self.rate as f32).clamp(2.0, self.line.max() - 2.0)
    }

    fn apply(&mut self, id: u16, v: f32) {
        for t in 0..mt::TAPS {
            if id == mt::time(t) {
                self.times[t].target = self.frames(t);
            } else if id == mt::level(t) {
                self.levels[t].target = v / 100.0;
            } else if id == mt::pan(t) {
                self.pans[t].target = v / 100.0;
            }
        }
        match id {
            mt::FEEDBACK => self.feedback.target = v / 100.0,
            mt::TONE => self.tone.set(SvfMode::LowPass, v, 0.6, 0.0, self.rate),
            mt::LOW_CUT => self.low_cut.set(SvfMode::HighPass, v, 0.6, 0.0, self.rate),
            mt::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    fn begin(&mut self, ctx: &Ctx) {
        if ctx.bpm > 0.0 && ctx.bpm != self.bpm {
            self.bpm = ctx.bpm;
            for t in 0..mt::TAPS {
                self.times[t].target = self.frames(t);
            }
        }
    }

    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let mut y = [0.0f32; 2];
            let mut last = 0.0;
            for t in 0..mt::TAPS {
                let d = self.times[t].next();
                let level = self.levels[t].next();
                let pan = self.pans[t].next();
                let v = self.line.read(d - 1.0);
                // Constant power across the pan.
                let a = (pan + 1.0) * 0.125;
                y[0] += v * level * kit::cosine(a) * std::f32::consts::SQRT_2;
                y[1] += v * level * kit::sine(a) * std::f32::consts::SQRT_2;
                if t == mt::TAPS - 1 {
                    last = v;
                }
            }
            let mut f = self.tone.run(0, last);
            f = self.low_cut.run(0, f);
            self.fb_sample = flush(f * self.feedback.next());
            self.line.push(0.5 * (l[i] + r[i]) + self.fb_sample);
            let mix = self.mix.next();
            l[i] = lerp(l[i], y[0], mix);
            r[i] = lerp(r[i], y[1], mix);
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.line = Line::new((MAX_SECONDS * rate as f32) as usize);
        for s in self.times.iter_mut() {
            s.set_time(rate, 80.0);
        }
        for s in self
            .levels
            .iter_mut()
            .chain(self.pans.iter_mut())
            .chain([&mut self.feedback, &mut self.mix])
        {
            s.set_time(rate, 20.0);
        }
        for id in 0..MULTITAP_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.line.clear();
        self.tone.reset();
        self.low_cut.reset();
        self.fb_sample = 0.0;
        for s in self
            .times
            .iter_mut()
            .chain(self.levels.iter_mut())
            .chain(self.pans.iter_mut())
            .chain([&mut self.feedback, &mut self.mix])
        {
            s.snap(s.target);
        }
    }

    fn tail(&self) -> u32 {
        (MAX_SECONDS * 2.0 * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(MultiTap, MULTITAP_INFO, MULTITAP_PARAMS);
