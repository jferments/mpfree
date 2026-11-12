//! Character: the colour of analog gear as four small, cheap processors in
//! the spirit of Airwindows' single-purpose plug-ins (original code, not a
//! port): **Console**, a channel's rounded warmth (a sine-shaped curve,
//! odd harmonics); **Tape**, soft saturation, a head bump around 75 Hz and
//! a little top-end loss; **Iron**, a transformer's low end that thickens
//! and saturates first (with a touch of even harmonics); and **Soften**, a
//! smooth slew limit that takes the edge off loud, harsh highs and leaves
//! quiet ones alone. One Amount knob, Output and Mix.
//!
//! Quiet signals come out at the level they went in, in every mode, so
//! Amount changes the colour and how hard the peaks are held, not the
//! level of quiet parts. No oversampling: the curves are gentle enough
//! that their aliasing stays low, and each mode is a few multiplies a
//! frame. A change of mode crossfades over 10 ms.

use std::f32::consts::{FRAC_PI_2, PI};

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfMode};
use crate::{db_to_gain, defaults, flush, lerp, soft, DcBlock};

/// [`Character`] parameter ids.
pub mod ids {
    pub const MODE: u16 = 0;
    pub const AMOUNT: u16 = 1;
    pub const OUTPUT: u16 = 2;
    pub const MIX: u16 = 3;

    /// The modes, as Mode's values.
    pub const CONSOLE: f32 = 0.0;
    pub const TAPE: f32 = 1.0;
    pub const IRON: f32 = 2.0;
    pub const SOFTEN: f32 = 3.0;
}

const MODES: &[&str] = &["Console", "Tape", "Iron", "Soften"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.character",
    name: "Character",
    kind: Kind::Effect,
};

static PARAMS: [ParamInfo; 4] = [
    ParamInfo {
        id: ids::MODE,
        name: "Mode",
        group: "Character",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(MODES),
        readonly: false,
    },
    ParamInfo {
        id: ids::AMOUNT,
        name: "Amount",
        group: "Character",
        min: 0.0,
        max: 100.0,
        default: 40.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::OUTPUT,
        name: "Output",
        group: "Output",
        min: -12.0,
        max: 12.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::MIX,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
];

/// The drive into the curves at Amount 100.
const DRIVE_DB: f32 = 12.0;
/// Iron drives its low band this much harder than the rest.
const IRON_LOW: f32 = 2.0;
/// Where Iron splits its low band off.
const IRON_HZ: f32 = 120.0;
/// Iron's even harmonic at Amount 100 (of the low band squared).
const IRON_EVEN: f32 = 0.15;
/// Tape's head bump: where, how wide and how much at Amount 100.
const BUMP_HZ: f32 = 75.0;
const BUMP_Q: f32 = 1.2;
const BUMP_DB: f32 = 3.0;
/// Tape's top-end loss: the corner at Amount 0 (out of the way), halving
/// with Amount 100.
const TAPE_TOP_HZ: f32 = 22_000.0;
/// Soften: a full-scale sine at this frequency has its steepest steps
/// held to about four fifths, at Amount 0 and 100 (log between). Quieter
/// or lower sounds have smaller steps and pass untouched.
const SOFTEN_HI_HZ: f32 = 40_000.0;
const SOFTEN_LO_HZ: f32 = 2500.0;
/// A mode change's crossfade, ms.
const FADE_MS: f32 = 10.0;
/// The coefficients that follow Amount are worked out every this many
/// frames while it glides (on a grid counted from the stream's start, so
/// the output is the same in any block size).
const GRID: u32 = 16;

/// Console's curve: like `sin` up to pi/2 and held at 1 beyond, as an odd
/// quintic with unity slope at 0 and a flat top at pi/2 (so it never folds
/// back). Odd harmonics only, and a rounded, gradual top.
#[inline]
pub(crate) fn console(x: f32) -> f32 {
    let t = (x * (1.0 / FRAC_PI_2)).clamp(-1.0, 1.0);
    let t2 = t * t;
    // a t + b t^3 + c t^5 with a = pi/2, a + b + c = 1, a + 3b + 5c = 0.
    const A: f32 = FRAC_PI_2;
    const C: f32 = A - 1.5;
    const B: f32 = 2.5 - 2.0 * A;
    t * (A + t2 * (B + t2 * C))
}

/// The per-channel state of every mode.
#[derive(Clone, Copy, Debug)]
struct Chan {
    /// Tape's top-end loss (one pole).
    top: f32,
    /// Iron's low band (one pole).
    low: f32,
    /// Soften's last output.
    last: f32,
}

impl Chan {
    const ZERO: Chan = Chan {
        top: 0.0,
        low: 0.0,
        last: 0.0,
    };
}

/// The colour of analog gear: Console, Tape, Iron or Soften.
#[derive(Clone)]
pub struct Character {
    v: [f32; 4],
    rate: u32,
    amount: Smooth,
    drive: Smooth,
    out: Smooth,
    mix: Smooth,
    mode: usize,
    /// The mode faded from, and the frames of the fade left.
    from: usize,
    fade: u32,
    fade_len: u32,
    ch: [Chan; 2],
    bump: Svf,
    iron_dc: [DcBlock; 2],
    /// Coefficients that follow Amount, and the Amount they were worked
    /// out for.
    top_c: f32,
    soften_d: f32,
    soften_inv: f32,
    coef_at: f32,
    low_c: f32,
    grid: u32,
}

impl Default for Character {
    fn default() -> Self {
        Self::new()
    }
}

impl Character {
    pub fn new() -> Character {
        let v = defaults(&PARAMS);
        let mut c = Character {
            v,
            rate: 48_000,
            amount: Smooth::new(v[1] / 100.0),
            drive: Smooth::new(db_to_gain(DRIVE_DB * v[1] / 100.0)),
            out: Smooth::new(db_to_gain(v[2])),
            mix: Smooth::new(v[3] / 100.0),
            mode: v[0] as usize,
            from: v[0] as usize,
            fade: 0,
            fade_len: 480,
            ch: [Chan::ZERO; 2],
            bump: Svf::default(),
            iron_dc: [DcBlock::new(), DcBlock::new()],
            top_c: 1.0,
            soften_d: 1.0,
            soften_inv: 1.0,
            coef_at: f32::NAN,
            low_c: 0.0,
            grid: 0,
        };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::MODE => {
                let m = v as usize;
                if m != self.mode {
                    // Fade from what is heard now; a change mid-fade fades
                    // from the mode that was coming in.
                    self.from = self.mode;
                    self.mode = m;
                    self.fade = self.fade_len;
                    self.clear_mode(m);
                }
            }
            ids::AMOUNT => {
                self.amount.target = v / 100.0;
                self.drive.target = db_to_gain(DRIVE_DB * v / 100.0);
            }
            ids::OUTPUT => self.out.target = db_to_gain(v),
            ids::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    /// Clears a mode's state, so one coming in starts from silence.
    fn clear_mode(&mut self, m: usize) {
        for ch in &mut self.ch {
            match m {
                1 => ch.top = 0.0,
                2 => ch.low = 0.0,
                3 => ch.last = 0.0,
                _ => {}
            }
        }
        match m {
            1 => self.bump.reset(),
            2 => {
                for d in &mut self.iron_dc {
                    d.reset();
                }
            }
            _ => {}
        }
    }

    /// Works out what follows Amount (the head bump, the top-end loss and
    /// Soften's step) for Amount `a` (0 to 1).
    fn coefs(&mut self, a: f32) {
        let fs = self.rate as f32;
        self.bump
            .set(SvfMode::Bell, BUMP_HZ, BUMP_Q, BUMP_DB * a, self.rate);
        let top = (TAPE_TOP_HZ * (-a).exp2()).min(0.45 * fs);
        self.top_c = 1.0 - (-2.0 * PI * top / fs).exp();
        let hz = SOFTEN_HI_HZ * (SOFTEN_LO_HZ / SOFTEN_HI_HZ).powf(a);
        // A full-scale sine's steepest step at `hz`: `soft` holds a step
        // of that to 0.78 of itself, and never lets one past it.
        self.soften_d = 2.0 * PI * hz / fs;
        self.soften_inv = 1.0 / self.soften_d;
        self.coef_at = a;
    }

    /// One sample of channel `c` through mode `m`, at drive `g` (and its
    /// reciprocal `ig`, one divide a frame instead of one a sample).
    #[inline]
    fn shape(&mut self, m: usize, c: usize, x: f32, g: f32, ig: f32, a: f32) -> f32 {
        match m {
            0 => console(g * x) * ig,
            1 => {
                let s = soft(g * x) * ig;
                let s = self.bump.run(c, s);
                let ch = &mut self.ch[c];
                ch.top = flush(ch.top + self.top_c * (s - ch.top));
                ch.top
            }
            2 => {
                let ch = &mut self.ch[c];
                ch.low = flush(ch.low + self.low_c * (x - ch.low));
                let (low, high) = (ch.low, x - ch.low);
                // The even harmonic grows with how hard the low band is
                // driven, as a transformer's core fills.
                let u = console(g * IRON_LOW * low);
                let y = (u + IRON_EVEN * a * u * u) * (ig * (1.0 / IRON_LOW)) + high;
                self.iron_dc[c].run(y)
            }
            _ => {
                let ch = &mut self.ch[c];
                let d = self.soften_d;
                ch.last = flush(ch.last + d * soft((x - ch.last) * self.soften_inv));
                ch.last
            }
        }
    }

    fn run(&mut self, left: &mut [f32], right: &mut [f32]) {
        for i in 0..left.len() {
            if self.grid == 0 && self.amount.value != self.coef_at {
                self.coefs(self.amount.value);
            }
            self.grid = if self.grid + 1 == GRID {
                0
            } else {
                self.grid + 1
            };
            let a = self.amount.next();
            let g = self.drive.next();
            let ig = 1.0 / g;
            let out = self.out.next();
            let mix = self.mix.next();
            let x = [left[i], right[i]];
            let mut y = [0.0f32; 2];
            for c in 0..2 {
                let mut w = self.shape(self.mode, c, x[c], g, ig, a);
                if self.fade > 0 {
                    let old = self.shape(self.from, c, x[c], g, ig, a);
                    let t = self.fade as f32 / self.fade_len as f32;
                    w = lerp(w, old, t);
                }
                y[c] = lerp(x[c], w, mix) * out;
            }
            self.fade = self.fade.saturating_sub(1);
            left[i] = y[0];
            right[i] = y[1];
        }
    }

    /// The curve a sample at `x` meets at the mode and Amount set (Soften,
    /// a limit on steps rather than levels, has none: it is the identity).
    /// Tape's and Iron's filters are left out. For drawing.
    pub fn curve(&self, x: f32) -> f32 {
        let a = self.v[ids::AMOUNT as usize] / 100.0;
        let g = db_to_gain(DRIVE_DB * a);
        match self.v[ids::MODE as usize] as usize {
            0 => console(g * x) / g,
            1 => soft(g * x) / g,
            2 => {
                let gl = g * IRON_LOW;
                let u = console(gl * x);
                (u + IRON_EVEN * a * u * u) / gl
            }
            _ => x,
        }
    }

    /// A sine at `hz` and `amp` through a copy of this effect as set (Mix
    /// and Output left out), once it has settled: `out.len()` points over
    /// one cycle. For drawing (allocates).
    pub fn sine_through(&self, hz: f32, amp: f32, out: &mut [f32]) {
        let rate = 48_000u32;
        let mut c = Character::new();
        c.prepare(rate, mpfree_node::MAX_BLOCK);
        for id in [ids::MODE, ids::AMOUNT] {
            c.apply(id, self.v[id as usize]);
        }
        c.reset();
        let cycle = (rate as f32 / hz).round().max(2.0) as usize;
        let settle = (rate as usize / 10).div_ceil(cycle) * cycle;
        let mut l: Vec<f32> = (0..settle + cycle)
            .map(|k| amp * (2.0 * PI * k as f32 / cycle as f32).sin())
            .collect();
        let mut r = l.clone();
        for (lc, rc) in l
            .chunks_mut(mpfree_node::MAX_BLOCK)
            .zip(r.chunks_mut(mpfree_node::MAX_BLOCK))
        {
            c.run(lc, rc);
        }
        let n = out.len().max(1);
        for (k, o) in out.iter_mut().enumerate() {
            let at = settle + (k * (cycle - 1)) / (n - 1).max(1);
            *o = l[at];
        }
    }
}

impl Node for Character {
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
        for s in [
            &mut self.amount,
            &mut self.drive,
            &mut self.out,
            &mut self.mix,
        ] {
            s.set_time(self.rate, 20.0);
        }
        self.fade_len = ((FADE_MS * 0.001 * self.rate as f32) as u32).max(1);
        self.low_c = 1.0 - (-2.0 * PI * IRON_HZ / self.rate as f32).exp();
        for d in &mut self.iron_dc {
            d.set(10.0, self.rate);
        }
        self.coefs(self.amount.value);
    }
    fn reset(&mut self) {
        for s in [
            &mut self.amount,
            &mut self.drive,
            &mut self.out,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
        self.from = self.mode;
        self.fade = 0;
        self.ch = [Chan::ZERO; 2];
        self.bump.reset();
        for d in &mut self.iron_dc {
            d.reset();
        }
        self.grid = 0;
        self.coefs(self.amount.value);
    }
    fn tail_frames(&self) -> u32 {
        // Tape's head bump and Iron's DC blocker ring for a few tens of ms.
        self.rate / 10
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_curve_is_unity_at_zero_flat_at_its_top_and_never_folds() {
        let d = 1e-3;
        assert!(((console(d) - console(-d)) / (2.0 * d) - 1.0).abs() < 1e-3);
        assert!((console(FRAC_PI_2) - 1.0).abs() < 1e-6);
        assert_eq!(console(10.0), console(FRAC_PI_2));
        assert_eq!(console(-10.0), -console(10.0));
        let mut was = console(-2.0);
        for k in -199..=200 {
            let y = console(k as f32 / 100.0);
            assert!(y >= was, "{k}: {y} after {was}");
            was = y;
        }
        // Within 0.5 % of the sine it stands for.
        for k in 0..=100 {
            let x = FRAC_PI_2 * k as f32 / 100.0;
            assert!((console(x) - x.sin()).abs() < 5e-3, "{x}");
        }
    }
}
