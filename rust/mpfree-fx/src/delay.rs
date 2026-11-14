//! Delay: free or tempo-synced echo. Repeats pass a high-pass, a tone
//! low-pass and a soft saturator, so they darken and roughen like tape
//! instead of repeating sterile copies, and feedback above 100 % is safe
//! (the saturator bounds it). Time changes glide, bending pitch like a tape
//! delay instead of clicking. Freeze holds the buffer as a loop.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfMode};
use crate::{db_to_gain, defaults, flush, soft};

/// [`Delay`] parameter ids.
pub mod ids {
    pub const SYNC: u16 = 0;
    pub const TIME: u16 = 1;
    pub const FEEDBACK: u16 = 2;
    pub const PING_PONG: u16 = 3;
    pub const TONE: u16 = 4;
    pub const SATURATION: u16 = 5;
    pub const MIX: u16 = 6;
    pub const FREEZE: u16 = 7;
}

const SYNCS: &[&str] = &[
    "Free", "1/32", "1/16T", "1/16", "1/16D", "1/8T", "1/8", "1/8D", "1/4T", "1/4", "1/4D", "1/2",
    "1 bar",
];
/// Beats per sync choice (index 0, Free, is unused).
const SYNC_BEATS: [f64; 13] = [
    0.0,
    0.125,
    1.0 / 6.0,
    0.25,
    0.375,
    1.0 / 3.0,
    0.5,
    0.75,
    2.0 / 3.0,
    1.0,
    1.5,
    2.0,
    4.0,
];
/// The longest delay, seconds (1 bar at 60 BPM).
const MAX_SECONDS: f32 = 4.0;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.delay",
    name: "Delay",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 8] = [
    ParamInfo {
        id: 0,
        name: "Sync",
        group: "Time",
        min: 0.0,
        max: 12.0,
        default: 7.0,
        unit: Unit::None,
        curve: Curve::Choice(SYNCS),
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Time",
        group: "Time",
        min: 1.0,
        max: 4000.0,
        default: 375.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Feedback",
        group: "Repeats",
        min: 0.0,
        max: 110.0,
        default: 35.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Ping-pong",
        group: "Repeats",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Tone",
        group: "Repeats",
        min: 500.0,
        max: 20_000.0,
        default: 7_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Saturation",
        group: "Repeats",
        min: 0.0,
        max: 100.0,
        default: 20.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 30.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Freeze",
        group: "Output",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
];

/// One channel's delay line with a cubic (Hermite) fractional read.
struct Line {
    buf: Vec<f32>,
    w: usize,
}

impl Line {
    fn new(len: usize) -> Line {
        Line {
            buf: vec![0.0; len.max(8)],
            w: 0,
        }
    }
    #[inline]
    fn write(&mut self, x: f32) {
        self.buf[self.w] = x;
        self.w += 1;
        if self.w == self.buf.len() {
            self.w = 0;
        }
    }
    /// The signal `d` frames ago (`d` = 1 is the last sample written),
    /// interpolated; `d` is kept at 2 or more so every point is in the past.
    #[inline]
    fn read(&self, d: f32) -> f32 {
        let n = self.buf.len();
        let d = d.clamp(2.0, (n - 3) as f32);
        // Truncation is floor here (d > 0), with no library call.
        let di = d as usize as f32;
        let t = d - di;
        // `w + n - k` lies in [w + 1, w + n - 1] for 1 <= k < n, so one
        // conditional subtract wraps it (no integer division per read).
        let at = |k: usize| -> f32 {
            let i = self.w + n - k;
            self.buf[if i >= n { i - n } else { i }]
        };
        let k = di as usize;
        // Points at delays k-1, k, k+1, k+2; t moves from k toward k+1.
        let (x0, x1, x2, x3) = (at(k - 1), at(k), at(k + 1), at(k + 2));
        let c1 = 0.5 * (x2 - x0);
        let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
        let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
        ((c3 * t + c2) * t + c1) * t + x1
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
    }
}

/// Stereo delay with sync, ping-pong, filtered and saturated feedback, and
/// freeze.
pub struct Delay {
    v: [f32; 8],
    rate: u32,
    frames: Smooth,
    fb: Smooth,
    log_tone: Smooth,
    sat: Smooth,
    mix: Smooth,
    frozen: Smooth,
    lines: [Line; 2],
    tone: Svf,
    hp: Svf,
    /// The tempo of the last block, for `reset`.
    bpm: f64,
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Delay {
    /// The delay time in ms for the settings and a tempo (`bpm`), as the
    /// echo will play it. For drawing.
    pub fn time_ms(&self, bpm: f64) -> f32 {
        self.target_frames(bpm) * 1000.0 / self.rate as f32
    }

    /// Allocates for 48 kHz; [`Node::prepare`] reallocates for other rates.
    pub fn new() -> Delay {
        let v = defaults(&PARAMS);
        let mut d = Delay {
            v,
            rate: 0,
            frames: Smooth::new(0.375 * 48_000.0),
            fb: Smooth::new(v[2] / 100.0),
            log_tone: Smooth::new(v[4].log2()),
            sat: Smooth::new(v[5] / 100.0),
            mix: Smooth::new(v[6] / 100.0),
            frozen: Smooth::new(0.0),
            lines: [Line::new(8), Line::new(8)],
            tone: Svf::default(),
            hp: Svf::default(),
            bpm: 120.0,
        };
        d.prepare(48_000, mpfree_node::MAX_BLOCK);
        d
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::FEEDBACK => self.fb.target = v / 100.0,
            ids::TONE => self.log_tone.target = v.log2(),
            ids::SATURATION => self.sat.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            ids::FREEZE => self.frozen.target = v,
            _ => {}
        }
    }

    /// The delay in frames for the current settings and tempo.
    fn target_frames(&self, bpm: f64) -> f32 {
        let sync = self.v[0] as usize;
        let secs = if sync == 0 || bpm <= 0.0 {
            self.v[1] as f64 / 1000.0
        } else {
            SYNC_BEATS[sync.min(12)] * 60.0 / bpm
        };
        (secs as f32 * self.rate as f32).clamp(1.0, MAX_SECONDS * self.rate as f32)
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let pp = self.v[3] >= 0.5;
        // The saturation's gain and its inverse, worked out again only
        // while the amount glides (powf per sample is a library call on the
        // MPC's ARM).
        let (mut sat_was, mut drive, mut undrive) = (f32::NAN, 1.0, 1.0);
        for i in 0..l.len() {
            let d = self.frames.next();
            let fz = self.frozen.next();
            let fb = self.fb.next() + fz * (1.0 - self.fb.value);
            if !self.log_tone.settled() {
                self.log_tone.next();
                self.tone.set(
                    SvfMode::LowPass,
                    self.log_tone.value.exp2(),
                    0.707,
                    0.0,
                    self.rate,
                );
            }
            let sat = self.sat.next();
            if sat != sat_was {
                sat_was = sat;
                drive = db_to_gain(18.0 * sat);
                undrive = 1.0 / drive;
            }
            let mix = self.mix.next();
            let ya = self.lines[0].read(d);
            let yb = self.lines[1].read(d);
            let mut loopv = [ya, yb];
            for (c, y) in loopv.iter_mut().enumerate() {
                let f = self.tone.run(c, self.hp.run(c, *y));
                let s = soft(f * drive) * undrive;
                // Frozen: the loop keeps the raw repeats, so it never decays.
                *y = flush(s + fz * (*y - s));
            }
            let inp = 1.0 - fz;
            let (x0, x1) = (l[i], r[i]);
            let (wa, wb) = if pp {
                (0.5 * (x0 + x1) * inp + fb * loopv[1], fb * loopv[0])
            } else {
                (x0 * inp + fb * loopv[0], x1 * inp + fb * loopv[1])
            };
            self.lines[0].write(wa);
            self.lines[1].write(wb);
            l[i] = crate::lerp(x0, ya, mix);
            r[i] = crate::lerp(x1, yb, mix);
        }
    }
}

impl Node for Delay {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        if rate != self.rate {
            let len = (MAX_SECONDS * rate as f32) as usize + 8;
            self.lines = [Line::new(len), Line::new(len)];
        }
        self.rate = rate;
        self.frames.set_time(rate, 60.0);
        for s in [
            &mut self.fb,
            &mut self.log_tone,
            &mut self.sat,
            &mut self.mix,
        ] {
            s.set_time(rate, 20.0);
        }
        self.frozen.set_time(rate, 30.0);
        self.frames.snap(self.target_frames(self.bpm));
        self.hp.set(SvfMode::HighPass, 60.0, 0.707, 0.0, rate);
        self.tone.set(
            SvfMode::LowPass,
            self.log_tone.value.exp2(),
            0.707,
            0.0,
            rate,
        );
    }
    fn reset(&mut self) {
        for s in [
            &mut self.fb,
            &mut self.log_tone,
            &mut self.sat,
            &mut self.mix,
            &mut self.frozen,
        ] {
            s.snap(s.target);
        }
        self.frames.snap(self.target_frames(self.bpm));
        for line in &mut self.lines {
            line.clear();
        }
        self.tone.reset();
        self.hp.reset();
        self.tone.set(
            SvfMode::LowPass,
            self.log_tone.value.exp2(),
            0.707,
            0.0,
            self.rate,
        );
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            self.bpm = ctx.bpm;
            self.frames.target = self.target_frames(ctx.bpm);
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn rest_frames(&self) -> u32 {
        // What it holds is its tail: once rung out it is as good as reset.
        self.tail_frames()
    }
    fn tail_frames(&self) -> u32 {
        if self.v[7] >= 0.5 {
            return u32::MAX;
        }
        let fb = (self.v[2] / 100.0).clamp(0.0, 0.999);
        let d = self.frames.target.max(1.0);
        // Repeats until 60 dB down, capped at 30 s.
        let repeats = if fb <= 0.001 {
            1.0
        } else {
            (-60.0 / (20.0 * fb.log10())).ceil() + 1.0
        };
        (d * repeats).min(30.0 * self.rate as f32) as u32
    }
}
