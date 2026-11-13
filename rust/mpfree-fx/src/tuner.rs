//! The **Tuner**: the note a single sound is on and how many cents sharp
//! or flat, for tuning a guitar, a bass, a voice or a synth. It passes the
//! sound through untouched (or mutes it, to tune in silence) and reports
//! on its meters: the note, the cents and the frequency.
//!
//! It hears the pitch in two steps, every 20 ms:
//!
//! 1. **Where:** the vocal effects' YIN tracker ([`Tracker`]) over one wide
//!    band, 27 to 1600 Hz (a five-string bass's low B to the top of a
//!    voice), at about 6 kHz. That finds the period to within a frame or
//!    so, and says when nothing is clearly pitched.
//! 2. **Exactly:** the difference function again at the full rate, on a
//!    20 ms window, first at the lags around that period (the dip, refined
//!    by a parabola), then around 8, 64 and so on whole periods, up to as
//!    many as fit in 45 ms: each dip, divided by the periods it spans,
//!    gives the period to a few thousandths of a frame, well under a cent
//!    from the lowest note to the highest.
//!
//! The reading glides (about 50 ms) while the note holds, jumps when it
//! moves more than 30 cents, and holds for 300 ms after the sound stops,
//! so a plucked string's reading doesn't vanish as it fades.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::vocal::{diff, Band, Tracker, NOTE_NAMES};

/// The one band the tuner listens over.
const TUNER_BAND: [Band; 1] = [Band {
    lo: 27.0,
    hi: 1600.0,
    rate: 6_000.0,
    cut: 0.3,
    every: 0.02,
}];

/// The refining window and the longest span of whole periods, in seconds.
const WINDOW_S: f32 = 0.02;
const SPAN_S: f32 = 0.045;
/// How many looks (20 ms each) a reading holds after the sound stops.
const HOLD_LOOKS: u32 = 15;

/// [`Tuner`] parameter ids.
pub mod ids {
    /// The pitch of A4, 415 to 466 Hz.
    pub const REFERENCE: u16 = 0;
    /// Off, On: silence the output while tuning.
    pub const MUTE: u16 = 1;
    /// Meter: the note ([`crate::vocal::NOTE_NAMES`]; 0 for none).
    pub const NOTE: u16 = 2;
    /// Meter: cents from the note, -50 to 50.
    pub const CENTS: u16 = 3;
    /// Meter: the frequency heard (0 for none).
    pub const HZ: u16 = 4;
}

const OFF_ON: &[&str] = &["Off", "On"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.tuner",
    name: "Tuner",
    kind: Kind::Effect,
};

static PARAMS: [ParamInfo; 5] = [
    ParamInfo {
        id: ids::REFERENCE,
        name: "Reference",
        group: "Tuner",
        min: 415.0,
        max: 466.0,
        default: 440.0,
        unit: Unit::Hz,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::MUTE,
        name: "Mute",
        group: "Tuner",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(OFF_ON),
        readonly: false,
    },
    ParamInfo {
        id: ids::NOTE,
        name: "Note",
        group: "Meters",
        min: 0.0,
        max: 127.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(NOTE_NAMES),
        readonly: true,
    },
    ParamInfo {
        id: ids::CENTS,
        name: "Cents",
        group: "Meters",
        min: -50.0,
        max: 50.0,
        default: 0.0,
        unit: Unit::Cents,
        curve: Curve::Linear,
        readonly: true,
    },
    ParamInfo {
        id: ids::HZ,
        name: "Frequency",
        group: "Meters",
        min: 0.0,
        max: 2000.0,
        default: 0.0,
        unit: Unit::Hz,
        curve: Curve::Linear,
        readonly: true,
    },
];

/// The Tuner (see the module docs).
pub struct Tuner {
    v: [f32; 5],
    rate: u32,
    tracker: Tracker,
    /// The input's middle at the full rate, the newest at `w`.
    hist: Vec<f32>,
    mask: usize,
    w: usize,
    /// The newest `window + span + slack` frames, oldest first, at a look.
    lin: Vec<f32>,
    window: usize,
    span: usize,
    /// The reading (Hz, 0 for none) and how many looks it has been held.
    hz: f32,
    held: u32,
    gain: Smooth,
}

impl Default for Tuner {
    fn default() -> Self {
        Self::new()
    }
}

impl Tuner {
    pub fn new() -> Tuner {
        let v = crate::defaults(&PARAMS);
        let mut n = Tuner {
            v,
            rate: 48_000,
            tracker: Tracker::new(&TUNER_BAND),
            hist: Vec::new(),
            mask: 0,
            w: 0,
            lin: Vec::new(),
            window: 960,
            span: 2160,
            hz: 0.0,
            held: 0,
            gain: Smooth::new(1.0),
        };
        n.prepare(48_000, mpfree_node::MAX_BLOCK);
        n
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        if p.readonly {
            return;
        }
        let v = p.clamp(value);
        let v = if matches!(p.curve, Curve::Choice(_)) {
            v.round()
        } else {
            v
        };
        self.v[id as usize] = v;
        match id {
            ids::MUTE => self.gain.target = 1.0 - v,
            ids::REFERENCE => self.show(),
            _ => {}
        }
    }

    /// The sum of squared differences between the newest `window` frames
    /// and the ones `lag` frames before them (`lin` holds both).
    fn d(&self, lag: usize) -> f32 {
        let n = self.lin.len();
        let a = &self.lin[n - self.window..];
        let b = &self.lin[n - self.window - lag..n - lag];
        diff(a, b)
    }

    /// The lag from `lo` to `hi` where the difference dips lowest, refined
    /// by a parabola; None when the lowest is at either end (no dip there).
    fn dip(&self, lo: usize, hi: usize) -> Option<f32> {
        let most = self.lin.len() - self.window;
        let (lo, hi) = (lo.max(2), hi.min(most - 1));
        if hi < lo + 2 {
            return None;
        }
        let (mut best, mut at) = (f32::MAX, lo);
        let mut prev = [0.0f32; 3];
        let mut found = [0.0f32; 3];
        for lag in lo..=hi {
            let v = self.d(lag);
            prev = [prev[1], prev[2], v];
            if v < best {
                best = v;
                at = lag;
            }
            if lag == at + 1 {
                found = prev;
            }
        }
        if at == lo || at == hi {
            return None;
        }
        let (a, b, c) = (found[0], found[1], found[2]);
        let den = a - 2.0 * b + c;
        let shift = if den > 1e-12 {
            (0.5 * (a - c) / den).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        Some(at as f32 + shift)
    }

    /// The period, exactly, near the tracker's `coarse` one (frames).
    fn refine(&mut self, coarse: f32) -> Option<f32> {
        let n = self.lin.len();
        for (k, v) in self.lin.iter_mut().enumerate() {
            *v = self.hist[(self.w + 1 + self.mask + 1 - n + k) & self.mask];
        }
        // The tracker's frame or so, either side.
        let slack = (self.rate as f32 / TUNER_BAND[0].rate).ceil() as usize + 2;
        let c = coarse as usize;
        let one = self.dip(c.saturating_sub(slack), c + slack + 1)?;
        // Then whole periods, up to as many as fit in the span, eight times
        // more each step (so the last step's error, times eight, stays
        // inside the three lags searched either side): one dip's error is
        // shared out among them all.
        let most = (self.span as f32 / one) as usize;
        let (mut p, mut k) = (one, 1usize);
        while k < most {
            let next = (k * 8).min(most);
            let at = (p * next as f32).round() as usize;
            match self.dip(at.saturating_sub(3), at + 3) {
                Some(lag) => {
                    p = lag / next as f32;
                    k = next;
                }
                None => break,
            }
        }
        Some(p)
    }

    /// Puts the reading on the meters, against the reference.
    fn show(&mut self) {
        if self.hz <= 0.0 {
            self.v[ids::NOTE as usize] = 0.0;
            self.v[ids::CENTS as usize] = 0.0;
            self.v[ids::HZ as usize] = 0.0;
            return;
        }
        let midi = 69.0 + 12.0 * (self.hz / self.v[ids::REFERENCE as usize]).log2();
        let note = midi.round().clamp(1.0, 127.0);
        self.v[ids::NOTE as usize] = note;
        self.v[ids::CENTS as usize] = (100.0 * (midi - note)).clamp(-50.0, 50.0);
        self.v[ids::HZ as usize] = self.hz.min(2000.0);
    }

    /// After the tracker looks: refine, glide or jump, hold or let go.
    fn look(&mut self) {
        let coarse = self.tracker.period;
        let now = if coarse > 0.0 {
            self.refine(coarse).map(|p| self.rate as f32 / p)
        } else {
            None
        };
        match now {
            Some(hz) => {
                let jump = self.hz <= 0.0 || (1200.0 * (hz / self.hz).log2()).abs() > 30.0;
                self.hz = if jump {
                    hz
                } else {
                    self.hz + 0.35 * (hz - self.hz)
                };
                self.held = 0;
            }
            None if self.hz > 0.0 && self.held < HOLD_LOOKS => self.held += 1,
            None => self.hz = 0.0,
        }
        self.show();
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let m = 0.5 * (l[i] + r[i]);
            self.w = (self.w + 1) & self.mask;
            self.hist[self.w] = m;
            if self.tracker.push(m) {
                self.look();
            }
            let g = self.gain.next();
            l[i] *= g;
            r[i] *= g;
        }
    }
}

impl Node for Tuner {
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
        self.rate = rate.max(1);
        self.tracker.prepare(self.rate, 0);
        let rate = self.rate as f32;
        self.window = (WINDOW_S * rate) as usize;
        // Room for the lowest note's period plus its slack, at least.
        self.span = ((SPAN_S * rate) as usize).max((1.2 * rate / TUNER_BAND[0].lo) as usize);
        let slack = (rate / TUNER_BAND[0].rate).ceil() as usize + 8;
        let n = self.window + self.span + slack;
        self.lin = vec![0.0; n];
        let size = (n + 1).next_power_of_two();
        self.hist = vec![0.0; size];
        self.mask = size - 1;
        self.gain.set_time(self.rate, 10.0);
        self.reset();
    }
    fn reset(&mut self) {
        self.tracker.reset();
        self.hist.fill(0.0);
        self.w = 0;
        self.hz = 0.0;
        self.held = 0;
        self.gain.snap(1.0 - self.v[ids::MUTE as usize]);
        self.show();
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
}
