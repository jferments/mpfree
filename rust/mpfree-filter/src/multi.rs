//! The multimode state-variable filter: low-pass, high-pass, band-pass and
//! notch at 12 or 24 dB an octave, Butterworth-shaped with its resonance
//! on the last stage, the resonant stages' band-pass states bending as an
//! analog filter's resonance loop saturates (the Filter effect's Multi
//! model, FX quality round 2). Moved here from `mpfree-fx` bit for bit.
//! The pads run the same stages ([`Multi`], with a per-frame glide), but
//! linear, with a guard that damps a loud sound's ring instead of bending
//! it: a bend inside the loop pushes a loud, lopsided wave (a sawtooth)
//! off centre, and the pads hear their sounds at full level.

use crate::svf::{res_to_q, Svf, SvfCoefs, SvfMode};

/// Where the resonance starts to bend, and how much more it can give, in
/// the band-pass state's units (full scale 1), at Resonance 25 % and up.
/// Below 25 % both rise as 1 / (Resonance / 25 %), so a gentle filter never
/// bends a mix-level sound, and at 0 nothing bends at all.
pub const BEND_KNEE: f32 = 0.35;
pub const BEND_ROOM: f32 = 0.3;
pub const BEND_FULL_RES: f32 = 0.25;

/// Sets a multimode filter's one or two stages (the Filter effect's Multi
/// model and the pads' state-variable types).
#[allow(clippy::too_many_arguments)]
pub fn set_stages(
    a: &mut Svf,
    b: &mut Svf,
    mode: SvfMode,
    cascaded: bool,
    hz: f32,
    res: f32,
    gain: f32,
    rate: u32,
) {
    let q = res_to_q(res);
    if cascaded {
        // 24 dB: a Butterworth-shaped first stage, resonance on the second.
        let q1 = if matches!(mode, SvfMode::LowPass | SvfMode::HighPass) {
            0.541
        } else {
            q
        };
        let q2 = if matches!(mode, SvfMode::LowPass | SvfMode::HighPass) {
            1.307 * (q / 0.707)
        } else {
            q
        };
        a.set(mode, hz, q1, gain, rate);
        b.set(mode, hz, q2, gain, rate);
    } else {
        a.set(mode, hz, q, gain, rate);
    }
}

/// One filter's stages as set: the two `Svf`s, whether the second runs,
/// and which of them bend (the resonant ones).
#[derive(Clone, Copy, Debug)]
pub struct Stages {
    pub a: Svf,
    pub b: Svf,
    pub cascaded: bool,
    pub bend_a: bool,
    pub bend_b: bool,
}

impl Stages {
    /// One sample of channel `ch`; with an infinite `knee` (Resonance 0)
    /// every stage is the plain linear one.
    #[inline]
    pub fn run(&mut self, ch: usize, x: f32, knee: f32, room: f32) -> f32 {
        let bends = knee.is_finite();
        let mut x = if self.bend_a && bends {
            self.a.run_bounded(ch, x, knee, room)
        } else {
            self.a.run(ch, x)
        };
        if self.cascaded {
            x = if self.bend_b && bends {
                self.b.run_bounded(ch, x, knee, room)
            } else {
                self.b.run(ch, x)
            };
        }
        x
    }

    /// One sample of channel `ch` with the resonant stages clipped at `lim`
    /// (Hard); with an infinite `lim` (Resonance 0) every stage is linear.
    #[inline]
    pub fn run_clip(&mut self, ch: usize, x: f32, lim: f32) -> f32 {
        let clips = lim.is_finite();
        let mut x = if self.bend_a && clips {
            self.a.run_clipped(ch, x, lim)
        } else {
            self.a.run(ch, x)
        };
        if self.cascaded {
            x = if self.bend_b && clips {
                self.b.run_clipped(ch, x, lim)
            } else {
                self.b.run(ch, x)
            };
        }
        x
    }

    /// Carries on from `from`, the same filter at another rate: a
    /// state-variable filter's states are its band-pass and low-pass, so
    /// they mean nearly the same at any rate. A second stage that `from`
    /// doesn't run starts from silence.
    pub fn take_states(&mut self, from: &Stages) {
        self.a.copy_states(&from.a);
        if from.cascaded {
            self.b.copy_states(&from.b);
        } else {
            self.b.reset();
        }
    }

    /// [`Stages::run`] over a block of channel `ch`, in place, a stage at a
    /// time (each stage hears only the one before it, so the numbers are
    /// the same as frame by frame).
    pub fn run_block(&mut self, ch: usize, x: &mut [f32], knee: f32, room: f32) {
        let bends = knee.is_finite();
        if self.bend_a && bends {
            self.a.run_bounded_slice(ch, x, knee, room);
        } else {
            self.a.run_slice(ch, x);
        }
        if self.cascaded {
            if self.bend_b && bends {
                self.b.run_bounded_slice(ch, x, knee, room);
            } else {
                self.b.run_slice(ch, x);
            }
        }
    }
}

/// The bend's knee and room for Resonance `res` (0 to 1): [`BEND_KNEE`] and
/// [`BEND_ROOM`] from Resonance [`BEND_FULL_RES`] up, rising as one over
/// the Resonance below it, and nothing bending at 0 (an infinite knee).
#[inline]
pub fn bend_for(res: f32) -> (f32, f32) {
    let r = (res / BEND_FULL_RES).min(1.0);
    if r > 0.0 {
        (BEND_KNEE / r, BEND_ROOM / r)
    } else {
        (f32::INFINITY, 1.0)
    }
}

/// Whether a mode's stages bend, as the Filter effect sets them: a
/// low-pass's or high-pass's resonant stage only (the second at 24 dB; the
/// first is a flat Butterworth one), and both of a band-pass's. The notch,
/// bell and shelves don't ring up at their output, so they stay linear.
#[inline]
pub fn bends(mode: SvfMode, cascaded: bool) -> (bool, bool) {
    let resonant = matches!(
        mode,
        SvfMode::LowPass | SvfMode::HighPass | SvfMode::BandPass
    );
    (
        resonant && (!cascaded || mode == SvfMode::BandPass),
        resonant && cascaded,
    )
}

/// The two stages' Q at Resonance `res`, as [`set_stages`] gives them: at
/// 24 dB a low-pass or high-pass is a Butterworth pair (0.541 and 1.307)
/// with the resonance on the second; anything else is `res_to_q(res)`.
#[inline]
pub fn stage_qs(mode: SvfMode, cascaded: bool, res: f32) -> (f32, f32) {
    stage_qs_at(mode, cascaded, res_to_q(res))
}

/// [`stage_qs`] for a filter of Q `q` (the resonant stage's at 12 dB).
#[inline]
pub fn stage_qs_at(mode: SvfMode, cascaded: bool, q: f32) -> (f32, f32) {
    if cascaded && matches!(mode, SvfMode::LowPass | SvfMode::HighPass) {
        (0.541, 1.307 * (q / 0.707))
    } else {
        (q, q)
    }
}

/// A [`Multi`]'s curve at `hz` (dB) set to `cutoff_hz` and `res` at
/// `rate`: the stages' exact response (for any sound short of one loud
/// enough for the guard to ease the resonance), for drawing, so the curve
/// shows what is heard up to Nyquist, where the bilinear transform pulls a
/// low-pass down and cramps a high cutoff's peak.
pub fn response_db(
    mode: SvfMode,
    cascaded: bool,
    cutoff_hz: f32,
    res: f32,
    hz: f32,
    rate: u32,
) -> f32 {
    let [c1, c2] = stage_coefs(mode, cascaded, cutoff_hz, res, rate, MULTI_TOP);
    let power = |c: SvfCoefs| {
        let mut s = Svf::default();
        s.set_coefs(c);
        let (re, im) = s.response(hz, rate);
        re * re + im * im
    };
    let mut p = power(c1);
    if cascaded {
        p *= power(c2);
    }
    (10.0 * p.max(1e-24).log10()) as f32
}

/// The designs [`set_stages`] gives the two stages, without setting them,
/// the cutoff kept under `top` of the rate ([`SvfCoefs::bilinear_under`];
/// `set_stages` keeps it under 0.45).
#[inline]
pub fn stage_coefs(
    mode: SvfMode,
    cascaded: bool,
    hz: f32,
    res: f32,
    rate: u32,
    top: f32,
) -> [SvfCoefs; 2] {
    let (q1, q2) = stage_qs(mode, cascaded, res);
    [
        SvfCoefs::bilinear_under(mode, hz, q1, 0.0, rate, top),
        SvfCoefs::bilinear_under(mode, hz, q2, 0.0, rate, top),
    ]
}

/// How high a [`Multi`]'s cutoff goes: 49 % of the rate (21.6 kHz at 44.1
/// kHz), where the Filter effect's stops at 45 %. A pad's filter opened
/// all the way (as a live cutoff brings one in from a pad with none) is
/// then 3 dB down at 21.6 kHz and passes nearly all a sound has, as the
/// pad's filter did before it moved onto these stages.
pub const MULTI_TOP: f32 = 0.49;

/// The resonance guard (the pads' [`Multi`]): the ring a resonant stage's
/// band-pass state may reach before the guard damps it (full scale 1, so
/// 8 dB under it; the pad's level and velocity come after the filter). A
/// sound whose ring stays under it hears the filter as set: at full
/// Resonance (Q 40) that is any sound under about -40 dBFS at the cutoff.
pub const GUARD_KNEE: f32 = 0.4;

/// How hard the guard holds a ring past [`GUARD_KNEE`]: the stage's
/// damping goes as the ring over the knee to this power, so a ring settles
/// where `ring^(p + 1) = drive * Q * knee^p`: every 10 dB more drive rings
/// 2 dB louder, and a full-scale sine on the cutoff at full Resonance
/// rings at about full scale (32 dB over it, linear).
pub const GUARD_POWER: i32 = 4;

/// The window the guard holds a ring's peak over: [`GUARD_BLOCKS`] blocks
/// of [`GUARD_BLOCK`] frames (46 to 58 ms at 44.1 kHz, longer than a cycle
/// of any note over 22 Hz), so that the damping holds still across a held
/// note's cycles. Damping that moves inside a cycle pushes a lopsided wave
/// off centre, as the Filter effect's bend does.
pub const GUARD_BLOCK: u32 = 512;
pub const GUARD_BLOCKS: usize = 4;

/// How fast the guard lets go once a loud peak has left its window: 6 dB
/// in this time.
pub const GUARD_FALL_S: f32 = 0.05;

/// How far over a ring's window peak the held peak may sit and stay: a
/// ring within it of the held peak holds the damping still, so that the
/// guard settles instead of hunting (damping a little, then letting go a
/// little, as the ring follows each change a moment later).
pub const GUARD_SLACK: f32 = 1.03;

// A rise takes the fourth root of the ring's overshoot (two square roots).
const _: () = assert!(GUARD_POWER == 4);

/// The guard's damping for a held peak of `held`: 1 up to the knee, then
/// the peak over it to [`GUARD_POWER`].
#[inline]
fn damping_for(held: f32) -> f32 {
    let r = held * (1.0 / GUARD_KNEE);
    if r > 1.0 {
        r.powi(GUARD_POWER)
    } else {
        1.0
    }
}

/// One voice's multimode filter (the pads'): [`Stages`] whose cutoff and
/// resonance glide to each new setting over the frames until the next
/// one, so a filter envelope or LFO set at a control rate moves the
/// filter smoothly every frame. Stereo, both channels a frame at a time.
///
/// Its stages are linear, so it adds no harmonics and no DC offset; a
/// guard holds a loud sound's ring back instead of bending it. Each
/// resonant stage (a low-pass's, high-pass's or band-pass's last, and both
/// of a 24 dB band-pass's) keeps the peak of its band-pass state over a
/// window of about 50 ms, and once that peak is past [`GUARD_KNEE`] the
/// stage takes more damping (its `k` times the peak over the knee to
/// [`GUARD_POWER`], at most Resonance 0's Butterworth damping). A peak
/// higher than the one held is damped in the frame it comes, so a loud
/// note's first ring is held as it builds; the damping holds still while
/// the window holds the peak, and lets go 6 dB every [`GUARD_FALL_S`] once
/// it has gone. The Filter effect's bend inside the loop holds the peak
/// too, but on a loud lopsided wave it pushes the output off centre (a
/// full-scale sawtooth through a 2 kHz low-pass at Resonance 30 %: -0.12,
/// about -18 dB), which a pad, hearing its sound at full level, can't have.
#[derive(Clone, Copy, Debug)]
pub struct Multi {
    pub st: Stages,
    /// The setting's designs the glide runs from and to (undamped), and
    /// its frames: `left` of `span` still to go (0: landed, nothing to do
    /// per frame).
    from: [SvfCoefs; 2],
    to: [SvfCoefs; 2],
    left: u32,
    span: u32,
    mode: SvfMode,
    /// Each stage's damping, its `k` times this (1: as set), and the most
    /// it may take (the stage's Butterworth damping over its own, so a held
    /// ring never dulls the filter past Resonance 0's).
    damp: [f32; 2],
    most: [f32; 2],
    /// Each guarded stage's held peak (at least the knee; infinite for a
    /// stage the guard leaves alone), its peak since the window's block
    /// began, and the window's blocks.
    held: [f32; 2],
    cur: [f32; 2],
    win: [[f32; GUARD_BLOCKS]; 2],
    /// Frames into the window's block, and the block written next.
    frames: u32,
    wi: usize,
    /// Both stages are guarded (a 24 dB band-pass).
    both: bool,
    /// The held peak's fall over `fall_frames` frames at `fall_rate`.
    fall: f32,
    fall_frames: u32,
    fall_rate: u32,
    rate: u32,
    /// Set at least once (the first setting lands at once).
    set: bool,
}

impl Default for Multi {
    fn default() -> Multi {
        let c = SvfCoefs::bilinear(SvfMode::LowPass, 1000.0, 0.707, 0.0, 48_000);
        Multi {
            st: Stages {
                a: Svf::default(),
                b: Svf::default(),
                cascaded: false,
                bend_a: false,
                bend_b: false,
            },
            from: [c; 2],
            to: [c; 2],
            left: 0,
            span: 0,
            mode: SvfMode::LowPass,
            damp: [1.0; 2],
            most: [1.0; 2],
            held: [f32::INFINITY; 2],
            cur: [0.0; 2],
            win: [[0.0; GUARD_BLOCKS]; 2],
            frames: 0,
            wi: 0,
            both: false,
            fall: 1.0,
            fall_frames: 0,
            fall_rate: 0,
            rate: 48_000,
            set: false,
        }
    }
}

impl Multi {
    /// Heads for a new setting, there in `frames` frames (0 or the first
    /// setting: at once). `res` is 0 to 1 ([`res_to_q`]: Q 0.707 to 40). A
    /// guard holding a ring back goes on holding it, within what the new
    /// setting may take.
    #[inline]
    pub fn set(
        &mut self,
        mode: SvfMode,
        cascaded: bool,
        hz: f32,
        res: f32,
        rate: u32,
        frames: u32,
    ) {
        let jump = !self.set || frames == 0 || cascaded != self.st.cascaded;
        let to = stage_coefs(mode, cascaded, hz, res, rate, MULTI_TOP);
        let resonant = matches!(
            mode,
            SvfMode::LowPass | SvfMode::HighPass | SvfMode::BandPass
        );
        let guarded = if cascaded {
            [resonant && mode == SvfMode::BandPass, resonant]
        } else {
            [resonant, false]
        };
        // Butterworth's damping, Resonance 0's.
        let (q1, q2) = stage_qs_at(mode, cascaded, 0.707);
        for (i, q0) in [q1, q2].into_iter().enumerate() {
            if guarded[i] {
                if !self.held[i].is_finite() {
                    self.held[i] = GUARD_KNEE;
                    self.cur[i] = 0.0;
                    self.win[i] = [0.0; GUARD_BLOCKS];
                }
                self.most[i] = (1.0 / (q0 * to[i].k)).max(1.0);
                self.damp[i] = damping_for(self.held[i]).min(self.most[i]);
            } else {
                self.held[i] = f32::INFINITY;
                self.damp[i] = 1.0;
                self.most[i] = 1.0;
            }
        }
        self.both = guarded[0] && guarded[1];
        self.mode = mode;
        self.rate = rate;
        self.st.cascaded = cascaded;
        self.aim(to, frames, jump);
        self.set = true;
    }

    /// Stage `i`'s design `c` with the guard's damping: its `k` (and the
    /// high-pass's mix of it) times the stage's damping.
    #[inline(always)]
    fn damped(&self, c: SvfCoefs, i: usize) -> SvfCoefs {
        let f = self.damp[i];
        if f == 1.0 {
            return c;
        }
        SvfCoefs {
            k: c.k * f,
            m1: if self.mode == SvfMode::HighPass {
                c.m1 * f
            } else {
                c.m1
            },
            ..c
        }
    }

    /// Glides to `to` over `frames` frames, or lands there at once.
    #[inline]
    fn aim(&mut self, to: [SvfCoefs; 2], frames: u32, jump: bool) {
        if jump || frames == 0 {
            self.from = to;
            self.left = 0;
            self.span = 0;
            self.to = to;
            self.land();
        } else {
            // From where the glide is now.
            self.from = self.now();
            self.left = frames;
            self.span = frames;
            self.to = to;
        }
    }

    /// The stages set to the setting itself, damped.
    #[inline]
    fn land(&mut self) {
        let (a, b) = (self.damped(self.to[0], 0), self.damped(self.to[1], 1));
        self.st.a.set_coefs(a);
        self.st.b.set_coefs(b);
    }

    /// The guard's window, for the host to run every `frames` frames (the
    /// pads' control step), after the frames and before any new setting:
    /// each guarded stage's peak since the block began goes into the
    /// window when a block ends, the held peak falls to the window's (6 dB
    /// in [`GUARD_FALL_S`], never under the knee), and the damping eases
    /// with it. A peak over the held one is damped in its own frame
    /// ([`Multi::process`]). Quiet, it costs a few compares.
    #[inline]
    pub fn guard(&mut self, frames: u32) {
        if !self.set || !(self.held[0].is_finite() || self.held[1].is_finite()) {
            return;
        }
        if self.fall_frames != frames || self.fall_rate != self.rate {
            let s = frames as f32 / (GUARD_FALL_S * self.rate as f32);
            self.fall = 0.5f32.powf(s);
            (self.fall_frames, self.fall_rate) = (frames, self.rate);
        }
        self.frames += frames;
        let shift = self.frames >= GUARD_BLOCK;
        let mut eased = false;
        for i in 0..2 {
            if !self.held[i].is_finite() {
                continue;
            }
            if shift {
                self.win[i][self.wi] = self.cur[i];
                self.cur[i] = 0.0;
            }
            let window = self.win[i].iter().fold(self.cur[i], |m, &v| m.max(v));
            let floor = (window * GUARD_SLACK).max(GUARD_KNEE);
            if floor < self.held[i] {
                let held = floor.max(self.held[i] * self.fall);
                self.held[i] = held;
                let f = damping_for(held).min(self.most[i]);
                if f != self.damp[i] {
                    self.damp[i] = f;
                    eased = true;
                }
            }
        }
        if shift {
            self.frames -= GUARD_BLOCK;
            self.wi = (self.wi + 1) % GUARD_BLOCKS;
        }
        if eased && self.left == 0 {
            self.land();
        }
    }

    /// The designs the glide is at (undamped).
    fn now(&self) -> [SvfCoefs; 2] {
        if self.left == 0 {
            return self.to;
        }
        let t = (self.span - self.left) as f32 / self.span as f32;
        [
            self.from[0].toward(self.to[0], t),
            self.from[1].toward(self.to[1], t),
        ]
    }

    /// One frame, both channels: a step of the glide, then the stages
    /// ([`Svf::step_stereo`]; the host flushes every few frames with
    /// [`Multi::flush_below`]), each guarded stage's band-pass state
    /// watched. Inlined into the voice's loop.
    #[inline(always)]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.left > 0 {
            self.glide();
        }
        if self.st.cascaded {
            let (l, r) = if self.both {
                let (l, r, p) = self.st.a.step_stereo_peak(l, r);
                self.watch(0, p);
                (l, r)
            } else {
                self.st.a.step_stereo(l, r)
            };
            let (l, r, p) = self.st.b.step_stereo_peak(l, r);
            self.watch(1, p);
            (l, r)
        } else {
            let (l, r, p) = self.st.a.step_stereo_peak(l, r);
            self.watch(0, p);
            (l, r)
        }
    }

    /// Stage `i`'s band-pass peak this frame: kept for the window, and
    /// damped at once if it is over the held one.
    #[inline(always)]
    fn watch(&mut self, i: usize, p: f32) {
        self.cur[i] = self.cur[i].max(p);
        if p > self.held[i] {
            self.rise(i, p);
        }
    }

    /// A ring over the held peak: the stage damped for it from the next
    /// frame, by the ring's overshoot (`p` over the held peak; the held
    /// peak rises by its fourth root). Damping by the power of the whole
    /// overshoot would damp a ring again for the peak it reached before
    /// the last damping took hold, and the guard would hunt.
    #[inline(never)]
    fn rise(&mut self, i: usize, p: f32) {
        self.held[i] *= (p / self.held[i]).sqrt().sqrt();
        let f = damping_for(self.held[i]).min(self.most[i]);
        if f != self.damp[i] {
            self.damp[i] = f;
            if self.left == 0 {
                let c = self.damped(self.to[i], i);
                if i == 0 {
                    self.st.a.set_coefs(c);
                } else {
                    self.st.b.set_coefs(c);
                }
            }
        }
    }

    /// One step of the glide.
    #[inline(always)]
    fn glide(&mut self) {
        self.left -= 1;
        // The last step lands on the setting itself, not a rounding short
        // of it.
        let last = self.left == 0;
        let t = (self.span - self.left) as f32 / self.span as f32;
        let at = |from: SvfCoefs, to: SvfCoefs| if last { to } else { from.toward(to, t) };
        let a = self.damped(at(self.from[0], self.to[0]), 0);
        self.st.a.set_coefs(a);
        if self.st.cascaded {
            let b = self.damped(at(self.from[1], self.to[1]), 1);
            self.st.b.set_coefs(b);
        }
    }

    /// The guard's damping on the more damped of the stages: 1 when it
    /// holds nothing back.
    pub fn damping(&self) -> f32 {
        self.damp[0].max(self.damp[1])
    }

    /// Both stages to silence and the guard let go; the setting stays.
    pub fn reset(&mut self) {
        self.st.a.reset();
        self.st.b.reset();
        for i in 0..2 {
            if self.held[i].is_finite() {
                self.held[i] = GUARD_KNEE;
            }
            self.cur[i] = 0.0;
            self.win[i] = [0.0; GUARD_BLOCKS];
        }
        (self.frames, self.wi) = (0, 0);
        if self.damping() > 1.0 {
            self.damp = [1.0; 2];
            if self.left == 0 {
                self.land();
            }
        }
    }

    /// Zeros a channel of a stage once both its states are under `quiet`
    /// ([`Svf::flush_below`]); for a host to run every few frames.
    #[inline]
    pub fn flush_below(&mut self, quiet: f32) {
        self.st.a.flush_below(quiet);
        if self.st.cascaded {
            self.st.b.flush_below(quiet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_coefs_are_what_set_stages_sets() {
        for (mode, cascaded) in [
            (SvfMode::LowPass, false),
            (SvfMode::LowPass, true),
            (SvfMode::HighPass, true),
            (SvfMode::BandPass, true),
            (SvfMode::Notch, true),
        ] {
            for res in [0.0, 0.3, 1.0] {
                let (mut a, mut b) = (Svf::default(), Svf::default());
                set_stages(&mut a, &mut b, mode, cascaded, 1234.0, res, 0.0, 44_100);
                let c = stage_coefs(mode, cascaded, 1234.0, res, 44_100, 0.45);
                let (mut x, mut y) = (Svf::default(), Svf::default());
                x.set_coefs(c[0]);
                y.set_coefs(c[1]);
                for hz in [100.0, 1234.0, 9000.0] {
                    assert_eq!(a.response(hz, 44_100), x.response(hz, 44_100));
                    if cascaded {
                        assert_eq!(b.response(hz, 44_100), y.response(hz, 44_100));
                    }
                }
            }
        }
    }

    #[test]
    fn the_drawn_curve_is_the_set_filter() {
        for (mode, cascaded) in [
            (SvfMode::LowPass, false),
            (SvfMode::LowPass, true),
            (SvfMode::HighPass, true),
            (SvfMode::BandPass, false),
            (SvfMode::Notch, true),
        ] {
            let mut m = Multi::default();
            m.set(mode, cascaded, 3_000.0, 0.6, 44_100, 0);
            for hz in [50.0, 2_900.0, 18_000.0] {
                let p = |h: (f64, f64)| h.0 * h.0 + h.1 * h.1;
                let mut heard = p(m.st.a.response(hz, 44_100));
                if cascaded {
                    heard *= p(m.st.b.response(hz, 44_100));
                }
                let drawn = response_db(mode, cascaded, 3_000.0, 0.6, hz, 44_100);
                let heard = (10.0 * heard.max(1e-24).log10()) as f32;
                assert!(
                    (drawn - heard).abs() < 1e-3,
                    "{mode:?} {hz}: {drawn} {heard}"
                );
            }
        }
    }

    #[test]
    fn a_multi_opens_wider_than_the_effect() {
        // Wide open, the voice's filter goes to 49 % of the rate; the
        // effect's stages stop at 45 %.
        let mut m = Multi::default();
        m.set(SvfMode::LowPass, false, 22_000.0, 0.0, 44_100, 0);
        let p = |h: (f64, f64)| 10.0 * (h.0 * h.0 + h.1 * h.1).log10();
        let at_top = p(m.st.a.response(0.49 * 44_100.0, 44_100));
        assert!((at_top + 3.01).abs() < 0.05, "{at_top}");
        let (mut a, mut b) = (Svf::default(), Svf::default());
        set_stages(
            &mut a,
            &mut b,
            SvfMode::LowPass,
            false,
            22_000.0,
            0.0,
            0.0,
            44_100,
        );
        let effect = p(a.response(0.45 * 44_100.0, 44_100));
        assert!((effect + 3.01).abs() < 0.05, "{effect}");
    }

    #[test]
    fn a_glide_lands_on_the_setting_and_moves_smoothly() {
        let mut m = Multi::default();
        m.set(SvfMode::LowPass, true, 500.0, 0.2, 44_100, 0);
        m.set(SvfMode::LowPass, true, 4000.0, 0.2, 44_100, 16);
        let mut last = m.st.a.response(2000.0, 44_100);
        for _ in 0..16 {
            m.process(0.0, 0.0);
            let now = m.st.a.response(2000.0, 44_100);
            // Opening the filter: the gain at 2 kHz only rises.
            let p = |h: (f64, f64)| h.0 * h.0 + h.1 * h.1;
            assert!(p(now) >= p(last) - 1e-12);
            last = now;
        }
        let mut z = Svf::default();
        z.set_coefs(stage_coefs(SvfMode::LowPass, true, 4000.0, 0.2, 44_100, MULTI_TOP)[0]);
        assert_eq!(m.st.a.response(2000.0, 44_100), z.response(2000.0, 44_100));
    }

    const RATE: u32 = 44_100;
    const STEP: u32 = 16;

    /// `x` (mono, on both channels) through a [`Multi`] set once, the guard
    /// run every [`STEP`] frames as a voice runs it; the left channel out,
    /// and the guard's damping each frame.
    fn guarded(
        mode: SvfMode,
        cascaded: bool,
        hz: f32,
        res: f32,
        x: &[f32],
    ) -> (Vec<f32>, Vec<f32>) {
        let mut m = Multi::default();
        m.set(mode, cascaded, hz, res, RATE, 0);
        let mut y = Vec::with_capacity(x.len());
        let mut d = Vec::with_capacity(x.len());
        for (n, &s) in x.iter().enumerate() {
            if n > 0 && n % STEP as usize == 0 {
                m.guard(STEP);
                m.flush_below(1e-15);
            }
            y.push(m.process(s, s).0);
            d.push(m.damping());
        }
        (y, d)
    }

    /// A sawtooth at full scale, `cycle` frames a cycle, centred so that
    /// its cycles average to 0; sharp edges down.
    fn sawtooth(n: usize, cycle: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2 * (i % cycle) + 1) as f32 / cycle as f32 - 1.0)
            .collect()
    }

    fn mean(x: &[f32]) -> f32 {
        (x.iter().map(|&v| f64::from(v)).sum::<f64>() / x.len() as f64) as f32
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
    }

    #[test]
    fn a_loud_sawtooth_stays_centred_and_its_ring_is_held() {
        // A lopsided wave at full scale through a resonant low-pass, a high
        // note (1.8 kHz, 2 kHz cutoff) and a low one (110 Hz, 120 Hz): the
        // guard damps it but holds still across the cycles, so nothing
        // pushes the output off centre (the Filter effect's bend gives
        // -0.12 on the high one), and the ring is held near full scale,
        // not at 4.7 (Q 40's, linear, on the high note).
        for (cycle, hz) in [(25, 2_000.0), (401, 120.0)] {
            let x = sawtooth(2 * RATE as usize, cycle);
            // Whole cycles of the second second.
            let tail = RATE as usize / cycle * cycle;
            let from = x.len() - tail;
            for cascaded in [false, true] {
                for res in [0.3, 0.6, 1.0] {
                    let (y, d) = guarded(SvfMode::LowPass, cascaded, hz, res, &x);
                    let dc = mean(&y[from..]);
                    assert!(dc.abs() < 2e-4, "{cycle} {cascaded} res {res}: {dc}");
                    assert!(
                        peak(&y[from..]) < 1.6,
                        "{cycle} {cascaded} res {res}: {}",
                        peak(&y)
                    );
                    // Held still once it has the note.
                    let (lo, hi) = d[from..]
                        .iter()
                        .fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
                    assert_eq!(lo, hi, "{cycle} {cascaded} res {res}");
                }
            }
            let (_, d) = guarded(SvfMode::LowPass, false, hz, 1.0, &x);
            assert!(d[x.len() - 1] > 4.0, "{cycle}: {}", d[x.len() - 1]);
        }
    }

    #[test]
    fn a_quiet_sound_rings_up_in_full() {
        // -42 dBFS on the cutoff at full Resonance: under the knee, so the
        // guard never acts, and the ring is Q 40's (32 dB).
        let a = 0.008;
        let x: Vec<f32> = (0..RATE as usize)
            .map(|n| a * (std::f32::consts::TAU * 2_000.0 * n as f32 / RATE as f32).sin())
            .collect();
        for cascaded in [false, true] {
            let (y, d) = guarded(SvfMode::LowPass, cascaded, 2_000.0, 1.0, &x);
            let up = 20.0 * (peak(&y[y.len() / 2..]) / a).log10();
            assert!((up - 32.0).abs() < 0.3, "{cascaded}: {up}");
            assert!(d.iter().all(|&v| v == 1.0), "{cascaded}");
        }
    }

    #[test]
    fn the_guard_lets_go_as_the_sound_quietens() {
        // A loud half second, then a quiet one: the damping is back to
        // none within a few tenths of a second.
        let x: Vec<f32> = sawtooth(RATE as usize, 25)
            .iter()
            .enumerate()
            .map(|(n, &s)| if n < RATE as usize / 2 { s } else { 0.005 * s })
            .collect();
        let (_, d) = guarded(SvfMode::LowPass, false, 2_000.0, 1.0, &x);
        assert!(d[RATE as usize / 2 - 1] > 4.0);
        assert_eq!(d[x.len() - 1], 1.0);
    }

    #[test]
    fn at_resonance_0_and_on_a_notch_the_guard_never_acts() {
        // Butterworth is as damped as the guard goes, and a notch rings up
        // inside, not at its output: both are the linear filter as set, at
        // any level.
        let x: Vec<f32> = sawtooth(RATE as usize / 4, 25)
            .iter()
            .map(|s| 4.0 * s)
            .collect();
        for cascaded in [false, true] {
            for (mode, res) in [(SvfMode::LowPass, 0.0), (SvfMode::Notch, 1.0)] {
                let (_, d) = guarded(mode, cascaded, 2_000.0, res, &x);
                assert!(d.iter().all(|&v| v == 1.0), "{mode:?} {cascaded}");
            }
        }
    }

    #[test]
    fn a_loud_note_s_first_ring_is_held_as_it_builds() {
        // Full-scale noise from silence through resonant band-passes high
        // up, where a ring builds in a millisecond: damped in the frame it
        // rises, so even the first frames stay within 6 dB of full scale
        // (linear, the 24 dB band-pass at Resonance 0.9 rings past 20).
        let mut seed = 0x5eed_u32;
        let x: Vec<f32> = (0..RATE as usize / 4)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        for (cascaded, hz) in [(false, 10_000.0), (true, 10_000.0), (true, 3_000.0)] {
            let (y, _) = guarded(SvfMode::BandPass, cascaded, hz, 0.9, &x);
            assert!(peak(&y) < 2.0, "{cascaded} {hz}: {}", peak(&y));
        }
    }

    #[test]
    fn the_guard_eases_without_a_step() {
        // A tone swelling from silence to full scale on a resonant
        // band-pass's centre: the guard takes over on the way, and the
        // output never moves from one frame to the next by more than a
        // sine at its own peak level would (2 pi f / rate times it).
        let n = RATE as usize / 2;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let a = i as f32 / n as f32;
                a * (std::f32::consts::TAU * 700.0 * i as f32 / RATE as f32).sin()
            })
            .collect();
        for cascaded in [false, true] {
            let (y, d) = guarded(SvfMode::BandPass, cascaded, 700.0, 0.8, &x);
            assert!(d[n - 1] > 1.0, "{cascaded}");
            let top = peak(&y);
            let most = y
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max);
            let sine = std::f32::consts::TAU * 700.0 / RATE as f32 * top;
            assert!(most < 1.05 * sine, "{cascaded}: {most} {sine}");
            assert!(top < 1.2, "{cascaded}: {top}");
        }
    }
}
