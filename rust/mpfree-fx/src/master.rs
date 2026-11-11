//! Master safety: the last stage of every output, so nothing reaches the
//! converter clipped unless the user asks for it.
//!
//! In order: trim, DC block, then one of three modes.
//!
//! - **Safe limiter** (default): a 1.5 ms look-ahead peak limiter. The gain
//!   for each frame is the smallest gain any frame in the look-ahead window
//!   needs, released slowly and then averaged over the window, so it is
//!   always low enough by the time the peak arrives and never moves faster
//!   than the window. The output never goes above the ceiling.
//! - **Soft clip**: a smooth saturator from 6 dB under the ceiling up to
//!   the ceiling (deliberate colour, still never over).
//! - **Raw**: no limiting; what is over 0 dBFS clips in the converter. A
//!   deliberate choice for people who want that sound.
//!
//! Every mode counts overs (frames that came in above the ceiling) and
//! frames that leave above 0 dBFS, as meters. Nothing fails, stops or
//! mutes because of level. The look-ahead delay is the same in every mode,
//! so switching modes never moves the sound in time.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::{db_to_gain, defaults, gain_to_db, DcBlock, FixedDelay};

/// [`MasterSafety`] parameter ids.
pub mod ids {
    pub const MODE: u16 = 0;
    pub const TRIM: u16 = 1;
    pub const CEILING: u16 = 2;
    pub const RELEASE: u16 = 3;
    /// Meter: limiting now, dB (0 or below).
    pub const REDUCTION: u16 = 4;
    /// Meter: frames that came in above the ceiling.
    pub const OVERS: u16 = 5;
    /// Meter: frames that left above 0 dBFS (only Raw mode can make these).
    pub const CLIPPED: u16 = 6;
}

const MODES: &[&str] = &["Safe limiter", "Soft clip", "Raw"];
const LOOKAHEAD_MS: f32 = 1.5;

static INFO: NodeInfo = NodeInfo { id: "mpfree.fx.master_safety", name: "Master safety", kind: Kind::Effect };
static PARAMS: [ParamInfo; 7] = [
    ParamInfo { id: 0, name: "Mode", group: "Master safety", min: 0.0, max: 2.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(MODES), readonly: false },
    ParamInfo { id: 1, name: "Trim", group: "Master safety", min: -24.0, max: 12.0, default: 0.0, unit: Unit::Db, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 2, name: "Ceiling", group: "Master safety", min: -6.0, max: 0.0, default: -0.3, unit: Unit::Db, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 3, name: "Release", group: "Master safety", min: 10.0, max: 1000.0, default: 80.0, unit: Unit::Ms, curve: Curve::Log, readonly: false },
    ParamInfo { id: 4, name: "Limiting", group: "Meters", min: -60.0, max: 0.0, default: 0.0, unit: Unit::Db, curve: Curve::Linear, readonly: true },
    ParamInfo { id: 5, name: "Overs", group: "Meters", min: 0.0, max: 1.0e12, default: 0.0, unit: Unit::None, curve: Curve::Linear, readonly: true },
    ParamInfo { id: 6, name: "Clipped", group: "Meters", min: 0.0, max: 1.0e12, default: 0.0, unit: Unit::None, curve: Curve::Linear, readonly: true },
];

/// Counters since the last [`MasterSafety::clear_stats`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterStats {
    /// Frames that came in above the ceiling.
    pub overs: u64,
    /// Frames that left above 0 dBFS.
    pub clipped: u64,
    /// The most limiting applied, dB (0 or below).
    pub max_reduction_db: f32,
    /// The loudest frame out, dBFS.
    pub peak_out_db: f32,
}

impl Default for MasterStats {
    fn default() -> Self { MasterStats { overs: 0, clipped: 0, max_reduction_db: 0.0, peak_out_db: -180.0 } }
}

/// The Master safety stage. See the module docs.
pub struct MasterSafety {
    v: [f32; 7],
    rate: u32,
    trim: Smooth,
    look: usize,
    delay: [FixedDelay; 2],
    dc: [DcBlock; 2],
    /// Monotonic queue of (frame, needed gain) for the window minimum.
    q: Vec<(u64, f32)>,
    q_head: usize,
    q_len: usize,
    n: u64,
    env: f32,
    rel: f32,
    /// The last `look` envelope values and their sum, for the average.
    boxv: Vec<f32>,
    box_pos: usize,
    box_sum: f64,
    stats: MasterStats,
    gain_now: f32,
}

impl Default for MasterSafety {
    fn default() -> Self { Self::new() }
}

impl MasterSafety {
    pub fn new() -> MasterSafety {
        let v = defaults(&PARAMS);
        let mut m = MasterSafety {
            v,
            rate: 0,
            trim: Smooth::new(v[1]),
            look: 1,
            delay: [FixedDelay::new(1), FixedDelay::new(1)],
            dc: [DcBlock::new(), DcBlock::new()],
            q: Vec::new(),
            q_head: 0,
            q_len: 0,
            n: 0,
            env: 1.0,
            rel: 0.0,
            boxv: Vec::new(),
            box_pos: 0,
            box_sum: 0.0,
            stats: MasterStats::default(),
            gain_now: 1.0,
        };
        m.prepare(48_000, mpfree_node::MAX_BLOCK);
        m
    }

    pub fn stats(&self) -> MasterStats { self.stats }

    /// Zeroes the counters (control thread or audio thread; no allocation).
    pub fn clear_stats(&mut self) {
        self.stats = MasterStats::default();
        self.v[ids::OVERS as usize] = 0.0;
        self.v[ids::CLIPPED as usize] = 0.0;
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else { return };
        if p.readonly { return; }
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::TRIM => self.trim.target = v,
            ids::RELEASE => self.rel = 1.0 - (-1.0 / (v * 0.001 * self.rate as f32)).exp(),
            _ => {}
        }
    }

    fn clear_state(&mut self) {
        for d in &mut self.delay { d.reset(); }
        for d in &mut self.dc { d.reset(); }
        self.q_head = 0;
        self.q_len = 0;
        self.env = 1.0;
        self.boxv.fill(1.0);
        self.box_pos = 0;
        self.box_sum = self.look as f64;
        self.gain_now = 1.0;
    }

    /// The smallest needed gain over the last `look + 1` frames, after
    /// pushing `need` for frame `self.n`.
    #[inline]
    fn window_min(&mut self, need: f32) -> f32 {
        let cap = self.q.len();
        // Ring positions: head < cap and len <= cap, so one wrap is enough
        // (no division per sample).
        let wrap = |i: usize| if i >= cap { i - cap } else { i };
        // Drop queued values that are no smaller than the new one.
        while self.q_len > 0 {
            let back = wrap(self.q_head + self.q_len - 1);
            if self.q[back].1 >= need { self.q_len -= 1; } else { break; }
        }
        let at = wrap(self.q_head + self.q_len);
        self.q[at] = (self.n, need);
        self.q_len += 1;
        // Drop values older than the window.
        let oldest = self.n.saturating_sub(self.look as u64);
        while self.q[self.q_head].0 < oldest {
            self.q_head = wrap(self.q_head + 1);
            self.q_len -= 1;
        }
        self.q[self.q_head].1
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mode = self.v[0] as usize;
        let ceil = db_to_gain(self.v[2]);
        let knee = ceil * 0.5;
        let look = self.look as f64;
        // The trim as a gain, worked out again only while it glides; the
        // loudest output, turned into dB once per piece.
        let (mut t_db, mut t) = (f32::NAN, 1.0);
        let mut loudest = 0.0f32;
        for i in 0..l.len() {
            let d = self.trim.next();
            if d != t_db {
                t_db = d;
                t = db_to_gain(d);
            }
            let a = self.dc[0].run(l[i] * t);
            let b = self.dc[1].run(r[i] * t);
            let peak = a.abs().max(b.abs());
            if peak > ceil { self.stats.overs += 1; }
            let need = if peak > ceil { ceil / peak } else { 1.0 };
            let m = self.window_min(need);
            self.n += 1;
            self.env = if m < self.env { m } else { self.env + (m - self.env) * self.rel };
            self.box_sum += (self.env - self.boxv[self.box_pos]) as f64;
            self.boxv[self.box_pos] = self.env;
            self.box_pos += 1;
            if self.box_pos == self.boxv.len() { self.box_pos = 0; }
            let g = ((self.box_sum / look) as f32).min(1.0);
            let (da, db) = (self.delay[0].run(a), self.delay[1].run(b));
            let (ya, yb) = match mode {
                0 => {
                    // The average can sit a rounding error above what the
                    // peak needs; never let that through.
                    let (x, y) = (da * g, db * g);
                    (x.clamp(-ceil, ceil), y.clamp(-ceil, ceil))
                }
                1 => {
                    let sc = |x: f32| if x.abs() <= knee { x } else { x.signum() * (knee + (ceil - knee) * crate::fast_tanh((x.abs() - knee) / (ceil - knee))) };
                    (sc(da), sc(db))
                }
                _ => (da, db),
            };
            self.gain_now = if mode == 0 { g } else { 1.0 };
            let out_peak = ya.abs().max(yb.abs());
            if out_peak > 1.0 { self.stats.clipped += 1; }
            loudest = loudest.max(out_peak);
            l[i] = ya;
            r[i] = yb;
        }
        let pd = gain_to_db(loudest);
        if pd > self.stats.peak_out_db { self.stats.peak_out_db = pd; }
        let red = gain_to_db(self.gain_now).min(0.0);
        if red < self.stats.max_reduction_db { self.stats.max_reduction_db = red; }
        self.v[ids::REDUCTION as usize] = red.max(-60.0);
        self.v[ids::OVERS as usize] = self.stats.overs as f32;
        self.v[ids::CLIPPED as usize] = self.stats.clipped as f32;
    }
}

impl Node for MasterSafety {
    fn info(&self) -> &'static NodeInfo { &INFO }
    fn params(&self) -> &'static [ParamInfo] { &PARAMS }
    fn set_param(&mut self, id: u16, value: f32) { self.apply(id, value); }
    fn param(&self, id: u16) -> f32 { self.v.get(id as usize).copied().unwrap_or(f32::NAN) }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        if rate != self.rate {
            self.look = ((LOOKAHEAD_MS * 0.001 * rate as f32).round() as usize).max(1);
            self.delay = [FixedDelay::new(self.look), FixedDelay::new(self.look)];
            self.q = vec![(0, 1.0); self.look + 2];
            self.boxv = vec![1.0; self.look];
        }
        self.rate = rate;
        self.trim.set_time(rate, 20.0);
        for d in &mut self.dc { d.set(5.0, rate); }
        self.apply(ids::RELEASE, self.v[ids::RELEASE as usize]);
        self.clear_state();
    }
    fn reset(&mut self) {
        self.trim.snap(self.trim.target);
        self.clear_state();
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| if let NodeEvent::Param { id, value } = *ev { self.apply(id, value) }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn latency(&self) -> u32 { self.look as u32 }
    fn tail_frames(&self) -> u32 { self.look as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_buffers_match_latency() {
        let m = MasterSafety::new();
        assert_eq!(m.latency(), 72);
        assert_eq!(m.delay[0].frames(), 72);
    }
}
