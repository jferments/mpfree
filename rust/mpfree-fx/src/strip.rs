//! The [`ChannelStrip`]: a gate, a compressor and an EQ in one insert, as
//! on a mixing desk (the MPC's AIR Channel Strip).
//!
//! It is built from MPFree's own [`Gate`], [`Compressor`] and [`Eq`], so
//! each stage sounds and meters as it does on its own, with the controls
//! a channel needs on one page: the gate's threshold, range, attack and
//! release; the compressor's threshold, ratio, attack, release and
//! makeup; and an EQ of a low cut, a low shelf, a mid bell and a high
//! shelf. Order puts the EQ before or after the dynamics.
//!
//! Each stage's switch crossfades it in and out over 20 ms (no click),
//! and a stage that is off and faded out isn't run at all, so a strip
//! with only its EQ on costs about what the EQ does. The gate looks 1 ms
//! ahead, and the strip keeps that latency whether the gate is on or
//! not (its dry path is delayed to match), so turning the gate on never
//! moves the latency.

use mpfree_node::{Ctx, Kind, Node, NodeInfo, ParamInfo, Smooth};

use crate::comp::ids as comp;
use crate::dynamics::gate;
use crate::eq::ids as eq;
use crate::kit::{
    meter, node_impl, param, Choice, Db, Glide, Hz, Line, Linear, Log, Ms, Toggle, NONE,
};
use crate::{defaults, fast_db_to_gain, Compressor, Eq, Gate, Preset};

/// [`ChannelStrip`] parameter ids.
pub mod ids {
    pub const GATE: u16 = 0;
    pub const GATE_THRESHOLD: u16 = 1;
    pub const GATE_RANGE: u16 = 2;
    pub const GATE_ATTACK: u16 = 3;
    pub const GATE_RELEASE: u16 = 4;
    pub const COMP: u16 = 5;
    pub const COMP_THRESHOLD: u16 = 6;
    pub const RATIO: u16 = 7;
    pub const COMP_ATTACK: u16 = 8;
    pub const COMP_RELEASE: u16 = 9;
    pub const MAKEUP: u16 = 10;
    pub const EQ: u16 = 11;
    /// The low cut; off at its lowest, 20 Hz.
    pub const LOW_CUT: u16 = 12;
    pub const LOW_FREQ: u16 = 13;
    pub const LOW_GAIN: u16 = 14;
    pub const MID_FREQ: u16 = 15;
    pub const MID_GAIN: u16 = 16;
    pub const MID_Q: u16 = 17;
    pub const HIGH_FREQ: u16 = 18;
    pub const HIGH_GAIN: u16 = 19;
    pub const ORDER: u16 = 20;
    pub const OUTPUT: u16 = 21;
    /// Meter: the gate's gain now, in dB (0 open).
    pub const GATE_GAIN: u16 = 22;
    /// Meter: the compressor's gain reduction now, in dB (0 or below).
    pub const REDUCTION: u16 = 23;
}

/// The orders the stages can run in.
pub const ORDERS: &[&str] = &["Dynamics, EQ", "EQ, dynamics"];
/// The low cut's lowest setting, where it is off.
const LOW_CUT_OFF: f32 = 20.0;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.channel_strip",
    name: "Channel strip",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 24] = [
    param(0, "Gate", "Strip", 0.0, 1.0, 0.0, NONE, Toggle),
    param(1, "Gate thresh", "Gate", -80.0, 0.0, -50.0, Db, Linear),
    param(2, "Gate range", "Gate", -80.0, 0.0, -30.0, Db, Linear),
    param(3, "Gate attack", "Gate", 0.1, 50.0, 0.5, Ms, Log),
    param(4, "Gate release", "Gate", 5.0, 2000.0, 120.0, Ms, Log),
    param(5, "Comp", "Strip", 0.0, 1.0, 1.0, NONE, Toggle),
    param(
        6,
        "Comp thresh",
        "Compressor",
        -60.0,
        0.0,
        -18.0,
        Db,
        Linear,
    ),
    param(7, "Ratio", "Compressor", 1.0, 20.0, 3.0, NONE, Log),
    param(8, "Comp attack", "Compressor", 0.1, 100.0, 10.0, Ms, Log),
    param(
        9,
        "Comp release",
        "Compressor",
        10.0,
        2000.0,
        150.0,
        Ms,
        Log,
    ),
    param(10, "Makeup", "Compressor", 0.0, 24.0, 0.0, Db, Linear),
    param(11, "EQ", "Strip", 0.0, 1.0, 1.0, NONE, Toggle),
    param(
        12,
        "Low cut",
        "EQ",
        LOW_CUT_OFF,
        1000.0,
        LOW_CUT_OFF,
        Hz,
        Log,
    ),
    param(13, "Low freq", "EQ", 20.0, 1000.0, 100.0, Hz, Log),
    param(14, "Low gain", "EQ", -12.0, 12.0, 0.0, Db, Linear),
    param(15, "Mid freq", "EQ", 40.0, 16_000.0, 1000.0, Hz, Log),
    param(16, "Mid gain", "EQ", -18.0, 18.0, 0.0, Db, Linear),
    param(17, "Mid Q", "EQ", 0.4, 10.0, 1.0, NONE, Log),
    param(18, "High freq", "EQ", 1200.0, 20_000.0, 8000.0, Hz, Log),
    param(19, "High gain", "EQ", -12.0, 12.0, 0.0, Db, Linear),
    param(20, "Order", "Strip", 0.0, 1.0, 0.0, NONE, Choice(ORDERS)),
    param(21, "Output", "Strip", -24.0, 12.0, 0.0, Db, Linear),
    meter(22, "Gate gain", "Gate", -80.0, 0.0, Db),
    meter(23, "Reduction", "Compressor", -60.0, 0.0, Db),
];

/// Channel strip factory presets.
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Vocal",
        values: &[
            (ids::LOW_CUT, 90.0),
            (ids::COMP_THRESHOLD, -22.0),
            (ids::RATIO, 3.0),
            (ids::MAKEUP, 4.0),
            (ids::MID_FREQ, 350.0),
            (ids::MID_GAIN, -2.5),
            (ids::HIGH_FREQ, 10_000.0),
            (ids::HIGH_GAIN, 3.0),
        ],
    },
    Preset {
        name: "Kick",
        values: &[
            (ids::GATE, 1.0),
            (ids::GATE_THRESHOLD, -35.0),
            (ids::GATE_RANGE, -40.0),
            (ids::GATE_RELEASE, 80.0),
            (ids::COMP_THRESHOLD, -16.0),
            (ids::RATIO, 4.0),
            (ids::COMP_ATTACK, 20.0),
            (ids::MAKEUP, 3.0),
            (ids::LOW_FREQ, 60.0),
            (ids::LOW_GAIN, 3.0),
            (ids::MID_FREQ, 400.0),
            (ids::MID_GAIN, -4.0),
            (ids::MID_Q, 1.5),
        ],
    },
    Preset {
        name: "Snare",
        values: &[
            (ids::GATE, 1.0),
            (ids::GATE_THRESHOLD, -30.0),
            (ids::GATE_RANGE, -25.0),
            (ids::LOW_CUT, 80.0),
            (ids::COMP_THRESHOLD, -18.0),
            (ids::RATIO, 4.0),
            (ids::COMP_ATTACK, 5.0),
            (ids::MAKEUP, 3.0),
            (ids::MID_FREQ, 200.0),
            (ids::MID_GAIN, 2.0),
            (ids::HIGH_FREQ, 6000.0),
            (ids::HIGH_GAIN, 3.0),
        ],
    },
    Preset {
        name: "Bass",
        values: &[
            (ids::LOW_CUT, 30.0),
            (ids::COMP_THRESHOLD, -20.0),
            (ids::RATIO, 5.0),
            (ids::COMP_ATTACK, 15.0),
            (ids::MAKEUP, 4.0),
            (ids::MID_FREQ, 800.0),
            (ids::MID_GAIN, 2.0),
        ],
    },
    Preset {
        name: "Gentle glue",
        values: &[
            (ids::COMP_THRESHOLD, -12.0),
            (ids::RATIO, 2.0),
            (ids::COMP_ATTACK, 30.0),
            (ids::COMP_RELEASE, 250.0),
            (ids::MAKEUP, 1.5),
        ],
    },
];

/// Frames a stage is run in at a time (its dry copy lives on the stack).
const CHUNK: usize = 64;

/// One stage's switch: how far in it is now, and whether it ran last.
#[derive(Clone, Copy, Debug)]
struct Switch {
    on: Smooth,
    ran: bool,
}

impl Switch {
    fn new(on: bool) -> Switch {
        Switch {
            on: Smooth::new(if on { 1.0 } else { 0.0 }),
            ran: on,
        }
    }
    /// Off and faded out: the stage needn't run.
    fn idle(&self) -> bool {
        self.on.value == 0.0 && self.on.target == 0.0
    }
}

/// A gate, a compressor and an EQ in one insert.
pub struct ChannelStrip {
    v: [f32; 24],
    rate: u32,
    gate: Gate,
    comp: Compressor,
    eq: Eq,
    /// Gate, compressor, EQ.
    switch: [Switch; 3],
    /// The gate's dry path, delayed as the gate delays.
    dry: [Line; 2],
    output: Smooth,
}

impl Default for ChannelStrip {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelStrip {
    pub fn new() -> ChannelStrip {
        let v = defaults(&PARAMS);
        let mut s = ChannelStrip {
            v,
            rate: 0,
            gate: Gate::new(),
            comp: Compressor::new(),
            eq: Eq::new(),
            switch: [
                Switch::new(v[0] >= 0.5),
                Switch::new(v[5] >= 0.5),
                Switch::new(v[11] >= 0.5),
            ],
            dry: [Line::default(), Line::default()],
            output: Smooth::new(1.0),
        };
        // The EQ's bands as the strip uses them: a low shelf, a bell, an
        // unused bell and a high shelf; no high cut.
        s.eq.set_param(eq::band_type(0), 1.0);
        s.eq.set_param(eq::band_type(1), 0.0);
        s.eq.set_param(eq::gain(2), 0.0);
        s.eq.set_param(eq::band_type(3), 2.0);
        s.eq.set_param(eq::HIGH_SLOPE, 0.0);
        s.gate.set_param(gate::RANGE, v[2]);
        for id in 0..ids::GATE_GAIN {
            s.apply(id, v[id as usize]);
        }
        s.setup(48_000);
        s.clear();
        s
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            ids::GATE | ids::COMP | ids::EQ => {
                let k = match id {
                    ids::GATE => 0,
                    ids::COMP => 1,
                    _ => 2,
                };
                self.switch[k].on.target = if v >= 0.5 { 1.0 } else { 0.0 };
            }
            ids::GATE_THRESHOLD => self.gate.set_param(gate::THRESHOLD, v),
            ids::GATE_RANGE => self.gate.set_param(gate::RANGE, v),
            ids::GATE_ATTACK => self.gate.set_param(gate::ATTACK, v),
            ids::GATE_RELEASE => self.gate.set_param(gate::RELEASE, v),
            ids::COMP_THRESHOLD => self.comp.set_param(comp::THRESHOLD, v),
            ids::RATIO => self.comp.set_param(comp::RATIO, v),
            ids::COMP_ATTACK => self.comp.set_param(comp::ATTACK, v),
            ids::COMP_RELEASE => self.comp.set_param(comp::RELEASE, v),
            ids::MAKEUP => self.comp.set_param(comp::MAKEUP, v),
            ids::LOW_CUT => {
                let on = v > LOW_CUT_OFF;
                self.eq.set_param(eq::LOW_CUT, v);
                self.eq.set_param(eq::LOW_SLOPE, if on { 1.0 } else { 0.0 });
            }
            ids::LOW_FREQ => self.eq.set_param(eq::freq(0), v),
            ids::LOW_GAIN => self.eq.set_param(eq::gain(0), v),
            ids::MID_FREQ => self.eq.set_param(eq::freq(1), v),
            ids::MID_GAIN => self.eq.set_param(eq::gain(1), v),
            ids::MID_Q => self.eq.set_param(eq::q(1), v),
            ids::HIGH_FREQ => self.eq.set_param(eq::freq(3), v),
            ids::HIGH_GAIN => self.eq.set_param(eq::gain(3), v),
            ids::OUTPUT => self.output.target = fast_db_to_gain(v),
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    /// Runs stage `k` over one chunk and crossfades it with its dry
    /// signal by its switch.
    fn stage(&mut self, k: usize, ctx: &Ctx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let mut dl = [0.0f32; CHUNK];
        let mut dr = [0.0f32; CHUNK];
        if k == 0 {
            // The gate's dry path, delayed as the gate's output is.
            let d = self.gate.latency() as usize;
            for i in 0..n {
                self.dry[0].push(l[i]);
                self.dry[1].push(r[i]);
                dl[i] = self.dry[0].tap(d + 1);
                dr[i] = self.dry[1].tap(d + 1);
            }
        } else {
            dl[..n].copy_from_slice(l);
            dr[..n].copy_from_slice(r);
        }
        let sw = &mut self.switch[k];
        if sw.idle() {
            if k == 0 {
                l.copy_from_slice(&dl[..n]);
                r.copy_from_slice(&dr[..n]);
            }
            sw.ran = false;
            return;
        }
        if !sw.ran {
            // Back on after a rest: start the stage fresh rather than
            // from where it stopped.
            match k {
                0 => self.gate.reset(),
                1 => self.comp.reset(),
                _ => self.eq.reset(),
            }
            self.switch[k].ran = true;
        }
        match k {
            0 => self.gate.process(ctx, &[], l, r),
            1 => self.comp.process(ctx, &[], l, r),
            _ => self.eq.process(ctx, &[], l, r),
        }
        let sw = &mut self.switch[k];
        if sw.on.settled() && sw.on.value == 1.0 {
            return;
        }
        for i in 0..n {
            let g = sw.on.glide();
            l[i] = dl[i] + (l[i] - dl[i]) * g;
            r[i] = dr[i] + (r[i] - dr[i]) * g;
        }
    }
    fn run(&mut self, ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let order: [usize; 3] = if self.v[20] >= 0.5 {
            [2, 0, 1]
        } else {
            [0, 1, 2]
        };
        let mut gate_gain = 0.0f32;
        let mut reduction = 0.0f32;
        let mut at = 0;
        while at < l.len() {
            let to = (at + CHUNK).min(l.len());
            for &k in &order {
                self.stage(k, ctx, &mut l[at..to], &mut r[at..to]);
            }
            if self.switch[0].ran {
                gate_gain = gate_gain.min(self.gate.param(gate::GAIN));
            }
            if self.switch[1].ran {
                reduction = reduction.min(self.comp.param(comp::REDUCTION));
            }
            for i in at..to {
                let g = self.output.glide();
                l[i] *= g;
                r[i] *= g;
            }
            at = to;
        }
        self.v[ids::GATE_GAIN as usize] = gate_gain;
        self.v[ids::REDUCTION as usize] = reduction;
    }
    fn setup(&mut self, rate: u32) {
        self.gate.prepare(rate, CHUNK);
        self.comp.prepare(rate, CHUNK);
        self.eq.prepare(rate, CHUNK);
        if rate != self.rate {
            let n = self.gate.latency() as usize + 4;
            self.dry = [Line::new(n), Line::new(n)];
        }
        self.rate = rate;
        for s in &mut self.switch {
            s.on.set_time(rate, 20.0 / 3.0);
        }
        self.output.set_time(rate, 20.0);
    }
    fn clear(&mut self) {
        self.gate.reset();
        self.comp.reset();
        self.eq.reset();
        for d in &mut self.dry {
            d.clear();
        }
        for s in &mut self.switch {
            s.on.snap(s.on.target);
            s.ran = s.on.value > 0.0;
        }
        self.output.snap(self.output.target);
        self.v[ids::GATE_GAIN as usize] = 0.0;
        self.v[ids::REDUCTION as usize] = 0.0;
    }
    fn tail(&self) -> u32 {
        self.gate.tail_frames() + self.comp.tail_frames() + self.eq.tail_frames()
    }
    fn lat(&self) -> u32 {
        self.gate.latency()
    }
}

node_impl!(ChannelStrip, INFO, PARAMS);

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut ChannelStrip, l: &mut [f32], r: &mut [f32], block: usize) {
        let ctx = Ctx::new(48_000, 0);
        let n = l.len();
        let mut k = 0;
        while k < n {
            let e = (k + block).min(n);
            s.process(&ctx, &[], &mut l[k..e], &mut r[k..e]);
            k = e;
        }
    }

    #[test]
    fn everything_off_is_the_input_one_latency_late() {
        let mut s = ChannelStrip::new();
        s.prepare(48_000, 128);
        s.set_param(ids::COMP, 0.0);
        s.set_param(ids::EQ, 0.0);
        s.reset();
        let lat = s.latency() as usize;
        assert!(lat > 0);
        let mut l = vec![0.0; 512];
        let mut r = vec![0.0; 512];
        l[3] = 1.0;
        r[3] = -0.5;
        run(&mut s, &mut l, &mut r, 128);
        let at = l.iter().position(|v| *v != 0.0).unwrap();
        assert_eq!(at, 3 + lat, "the impulse comes out one latency late");
        assert!((l[at] - 1.0).abs() < 1e-6 && (r[at] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_gate_never_moves_the_latency_and_shuts_quiet_sound() {
        let mut s = ChannelStrip::new();
        s.prepare(48_000, 128);
        let off = s.latency();
        s.set_param(ids::GATE, 1.0);
        assert_eq!(s.latency(), off);
        s.set_param(ids::GATE_THRESHOLD, -30.0);
        s.set_param(ids::GATE_RANGE, -60.0);
        s.reset();
        // A quiet hum (-46 dB) is gated down by the range.
        let n = 24_000;
        let mut l: Vec<f32> = (0..n)
            .map(|i| 0.005 * (i as f32 * 100.0 / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut r = l.clone();
        run(&mut s, &mut l, &mut r, 128);
        let peak = l[12_000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.005 * 0.01, "{peak}");
        assert!(s.param(ids::GATE_GAIN) < -50.0);
    }

    #[test]
    fn the_compressor_turns_loud_sound_down_and_meters_it() {
        let mut s = ChannelStrip::new();
        s.prepare(48_000, 128);
        s.set_param(ids::COMP_THRESHOLD, -30.0);
        s.set_param(ids::RATIO, 10.0);
        s.reset();
        let n = 24_000;
        let mut l: Vec<f32> = (0..n)
            .map(|i| 0.5 * (i as f32 * 200.0 / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut r = l.clone();
        run(&mut s, &mut l, &mut r, 128);
        let peak = l[12_000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.25, "{peak}");
        assert!(s.param(ids::REDUCTION) < -6.0);
    }

    #[test]
    fn output_does_not_depend_on_the_block_size() {
        let render = |block: usize| {
            let mut s = ChannelStrip::new();
            s.prepare(48_000, 512);
            s.set_param(ids::GATE, 1.0);
            s.set_param(ids::LOW_CUT, 120.0);
            s.set_param(ids::MID_GAIN, 6.0);
            s.reset();
            let n = 9_000;
            let mut l: Vec<f32> = (0..n)
                .map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5)
                .collect();
            let mut r = l.clone();
            run(&mut s, &mut l, &mut r, block);
            l
        };
        let a = render(37);
        let b = render(512);
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert!((x - y).abs() < 1e-5, "{i}: {x} {y}");
        }
    }

    #[test]
    fn switching_a_stage_off_does_not_click() {
        let mut s = ChannelStrip::new();
        s.prepare(48_000, 128);
        s.set_param(ids::MID_GAIN, 12.0);
        s.set_param(ids::MID_FREQ, 220.0);
        s.reset();
        let n = 9_600;
        let mut l: Vec<f32> = (0..n)
            .map(|i| 0.1 * (i as f32 * 220.0 / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut r = l.clone();
        let ctx = Ctx::new(48_000, 0);
        s.process(&ctx, &[], &mut l[..4800], &mut r[..4800]);
        s.set_param(ids::EQ, 0.0);
        s.process(&ctx, &[], &mut l[4800..], &mut r[4800..]);
        let slope = 0.4 * 220.0 / 48_000.0 * std::f32::consts::TAU;
        for i in 4000..n {
            assert!((l[i] - l[i - 1]).abs() < 1.5 * slope, "{i}");
        }
    }
}
