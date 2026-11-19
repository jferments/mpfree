//! Reading the wavetables ([`crate::wavetable::table`]) as an oscillator:
//! where a voice reads during one control step ([`Run`]: two neighbouring
//! frames of one level and the crossfade between them) and how it bends
//! its phase first ([`Warp`]). Poly's Table wave reads a control step at a
//! time ([`render`], [`render_fm_pair`]); the Wavetable synth reads a frame
//! at a time ([`Run::read`]).
//!
//! A level's length is a power of two, so an unbent phase (32 bits, as
//! Poly's other waves keep it, [`crate::osc::phase`]) indexes it with a
//! shift and no float-to-integer conversion. A bent one is worked out as a
//! fraction of the cycle, and its index never passes the level's last
//! sample, so the read stays inside its frame whatever the warp gives.

use crate::wavetable::table::Table;

/// Frames per control step: both engines step every 16 frames.
const STEP: f32 = 16.0;
/// One cycle of a 32-bit phase, as a float.
const ONE: f32 = 4_294_967_296.0;
/// The highest phase a warp reads from.
const UNDER_ONE: f32 = 0.999_999;

/// How an oscillator bends its phase this control step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Warp {
    #[default]
    Off,
    /// Phase distortion: the knee at `k`, slopes `a` before it, `b` after.
    Bend {
        k: f32,
        a: f32,
        b: f32,
    },
    Sync {
        ratio: f32,
    },
    Mirror {
        amt: f32,
    },
    Fm {
        index: f32,
    },
}

/// The Warp menu's FM entry ([`crate::wavetable::params::WARP_MODES`]).
pub(crate) const WARP_FM: u8 = 4;

impl Warp {
    pub(crate) fn new(mode: u8, amt: f32) -> Warp {
        let amt = amt.clamp(0.0, 1.0);
        match mode {
            1 => {
                let k = 0.5 - 0.48 * amt;
                Warp::Bend {
                    k,
                    a: 0.5 / k,
                    b: 0.5 / (1.0 - k),
                }
            }
            2 => Warp::Sync {
                ratio: 1.0 + 7.0 * amt,
            },
            3 => Warp::Mirror { amt },
            WARP_FM => Warp::Fm { index: amt * 0.5 },
            _ => Warp::Off,
        }
    }

    /// How many times faster the table is read, for the mip level.
    pub(crate) fn speed(self) -> f32 {
        match self {
            Warp::Bend { a, .. } => a,
            Warp::Sync { ratio } => ratio,
            Warp::Mirror { amt } if amt > 0.0 => 2.0,
            Warp::Fm { index } => 1.0 + 8.0 * index,
            _ => 1.0,
        }
    }

    /// The phase read from, given the running phase and the other
    /// oscillator's sample (for FM): under 1, always. The bend's end and
    /// FM a hair under a whole cycle round up to 1 in floats, and a read
    /// there would pass its frame (and the table, on its last frame), so
    /// those two are held just under it, as the mirror always was.
    #[inline]
    pub(crate) fn apply(self, p: f32, other: f32) -> f32 {
        match self {
            Warp::Off => p,
            Warp::Bend { k, a, b } => {
                if p < k {
                    p * a
                } else {
                    (0.5 + (p - k) * b).min(UNDER_ONE)
                }
            }
            Warp::Sync { ratio } => (p * ratio).fract(),
            Warp::Mirror { amt } => {
                let m = 1.0 - (1.0 - 2.0 * p).abs();
                (p + (m - p) * amt).clamp(0.0, UNDER_ONE)
            }
            Warp::Fm { index } => {
                let x = p + index * other;
                (x - x.floor()).min(UNDER_ONE)
            }
        }
    }
}

/// Where an oscillator reads during one control step: two neighbouring
/// frames of its level and the crossfade between them, ramped per frame.
/// A fast sweep that crosses a frame inside a step extrapolates past the
/// pair for the rest of that step (16 frames), which is inaudible.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Run {
    pub(crate) off0: usize,
    pub(crate) off1: usize,
    pub(crate) t: f32,
    pub(crate) t_inc: f32,
    pub(crate) len: f32,
    /// How far a 32-bit phase shifts down to index the level.
    shift: u32,
}

impl Run {
    /// A run on `level` of `tab` from frame position `from` to `to`.
    pub(crate) fn new(tab: &Table, level: usize, from: f32, to: f32) -> Run {
        let lv = &tab.levels()[level];
        let len = lv.len as f32;
        let shift = 32 - lv.len.trailing_zeros();
        let last = tab.frames() - 1;
        if last == 0 {
            return Run {
                len,
                shift,
                ..Run::default()
            };
        }
        let (from, to) = (from.clamp(0.0, last as f32), to.clamp(0.0, last as f32));
        let f0 = (from.min(to) as usize).min(last - 1);
        let st = lv.stride();
        Run {
            off0: f0 * st,
            off1: (f0 + 1) * st,
            t: from - f0 as f32,
            t_inc: (to - from) / STEP,
            len,
            shift,
        }
    }

    /// One sample at phase `ph` (0 to 1): linear in phase, then between
    /// the two frames.
    #[inline]
    pub(crate) fn read(&self, d: &[f32], ph: f32) -> f32 {
        let x = ph * self.len;
        let i = x as usize;
        let fr = x - i as f32;
        let (a0, a1) = (d[self.off0 + i], d[self.off0 + i + 1]);
        let (b0, b1) = (d[self.off1 + i], d[self.off1 + i + 1]);
        let s0 = a0 + (a1 - a0) * fr;
        let s1 = b0 + (b1 - b0) * fr;
        s0 + (s1 - s0) * self.t
    }
}

/// The two frames' samples at index `i` and the next, between them by
/// `fr`, then between the frames by `t`.
#[inline(always)]
fn pair(f0: &[f32], f1: &[f32], i: usize, fr: f32, t: f32) -> f32 {
    let (a0, a1) = (f0[i], f0[i + 1]);
    let (b0, b1) = (f1[i], f1[i + 1]);
    let s0 = a0 + (a1 - a0) * fr;
    let s1 = b0 + (b1 - b0) * fr;
    s0 + (s1 - s0) * t
}

/// The read of a bent phase `w` (0 to 1): the index never passes the
/// level's last sample, so a warp that lands on 1 reads the cycle's end.
#[inline(always)]
fn at(f0: &[f32], f1: &[f32], len: f32, w: f32, t: f32) -> f32 {
    let x = w * len;
    let i = (x as usize).min(f0.len() - 2);
    pair(f0, f1, i, x - i as f32, t)
}

/// `buf[j] = 0 + y * level` (`ADD` false) or `buf[j] += y * level`.
#[inline(always)]
fn put<const ADD: bool>(x: &mut f32, y: f32, level: f32) {
    *x = if ADD { *x } else { 0.0 } + y * level;
}

/// One oscillator of one unison stack over `buf`, written (`ADD` false)
/// or added, times `level`: level data `d` through `run` from its
/// crossfade `t`, from phase `ph` by `dt` a frame (32-bit phases), each
/// phase bent by `warp`. `fm` is the other oscillator's samples (any
/// length when the warp isn't FM). Advances the phase and returns the
/// crossfade after the last frame, which is the same for every stack.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(crate) fn render<const ADD: bool>(
    run: &Run,
    t: f32,
    d: &[f32],
    warp: Warp,
    fm: &[f32],
    buf: &mut [f32],
    ph: &mut u32,
    dt: u32,
    level: f32,
) -> f32 {
    let n = run.len as usize + 1;
    let (f0, f1) = (&d[run.off0..run.off0 + n], &d[run.off1..run.off1 + n]);
    let (len, inc) = (run.len, run.t_inc);
    let (mut p, mut t) = (*ph, t);
    let unit = 1.0 / ONE;
    match warp {
        Warp::Off => {
            let mask = (1u32 << run.shift) - 1;
            let scale = 1.0 / (1u64 << run.shift) as f32;
            for x in buf.iter_mut() {
                let i = (p >> run.shift) as usize;
                let fr = (p & mask) as f32 * scale;
                put::<ADD>(x, pair(f0, f1, i, fr, t), level);
                p = p.wrapping_add(dt);
                t += inc;
            }
        }
        Warp::Fm { index } => {
            for (x, &o) in buf.iter_mut().zip(fm) {
                let w = p as f32 * unit + index * o;
                put::<ADD>(x, at(f0, f1, len, w - w.floor(), t), level);
                p = p.wrapping_add(dt);
                t += inc;
            }
        }
        w => {
            for x in buf.iter_mut() {
                let q = w.apply(p as f32 * unit, 0.0);
                put::<ADD>(x, at(f0, f1, len, q, t), level);
                p = p.wrapping_add(dt);
                t += inc;
            }
        }
    }
    *ph = p;
    t
}

/// Both oscillators of one stack on FM, each moving the other's phase,
/// into `buf` as `a * levels[0] + b * levels[1]`: the second reads with
/// the first's sample from the frame before (`last`, kept between calls),
/// the first with the second's from this frame, as on the Wavetable synth.
/// Returns each run's crossfade after the last frame.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(crate) fn render_fm_pair(
    runs: [&Run; 2],
    ts: [f32; 2],
    ds: [&[f32]; 2],
    index: [f32; 2],
    buf: &mut [f32],
    ph: [&mut u32; 2],
    dt: [u32; 2],
    levels: [f32; 2],
    last: &mut f32,
) -> [f32; 2] {
    fn frames<'a>(r: &Run, d: &'a [f32]) -> (&'a [f32], &'a [f32]) {
        let n = r.len as usize + 1;
        (&d[r.off0..r.off0 + n], &d[r.off1..r.off1 + n])
    }
    let (a0, a1) = frames(runs[0], ds[0]);
    let (b0, b1) = frames(runs[1], ds[1]);
    let (la, lb) = (runs[0].len, runs[1].len);
    let unit = 1.0 / ONE;
    let [pa, pb] = ph;
    let (mut p, mut q) = (*pa, *pb);
    let [mut ta, mut tb] = ts;
    let mut a = *last;
    for x in buf.iter_mut() {
        let wb = q as f32 * unit + index[1] * a;
        let b = at(b0, b1, lb, wb - wb.floor(), tb);
        let wa = p as f32 * unit + index[0] * b;
        a = at(a0, a1, la, wa - wa.floor(), ta);
        *x = 0.0 + a * levels[0] + b * levels[1];
        p = p.wrapping_add(dt[0]);
        q = q.wrapping_add(dt[1]);
        ta += runs[0].t_inc;
        tb += runs[1].t_inc;
    }
    *pa = p;
    *pb = q;
    *last = a;
    [ta, tb]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wavetable::factory;

    #[test]
    fn warps_stay_in_the_cycle() {
        for mode in 0..5u8 {
            for amt in [0.0, 0.3, 1.0] {
                let w = Warp::new(mode, amt);
                for k in 0..1000 {
                    let p = k as f32 / 1000.0;
                    let q = w.apply(p, (k as f32 * 0.37).sin());
                    assert!((0.0..1.0).contains(&q), "mode {mode} amt {amt} p {p}: {q}");
                }
            }
        }
        assert_eq!(Warp::new(1, 0.0).apply(0.3, 0.0), 0.3);
        // Where floats round up to a whole cycle: the bend's end, and FM
        // pulling a phase of 0 a hair below it.
        let end = 1.0 - f32::EPSILON / 2.0;
        for amt in [0.0, 0.5, 1.0] {
            assert!(Warp::new(1, amt).apply(end, 0.0) < 1.0, "bend {amt}");
        }
        assert!(Warp::new(4, 1.0).apply(0.0, -1e-9) < 1.0);
    }

    /// A step of the block read gives what the frame-at-a-time read does,
    /// on every level and at the table's last pair of frames, bent or not.
    #[test]
    fn the_block_read_matches_the_frame_read() {
        let tab = &factory()[3];
        let last = (tab.frames() - 1) as f32;
        for level in [0, 5, 16] {
            let d = tab.levels()[level].data();
            for mode in 0..4u8 {
                let w = Warp::new(mode, 0.4);
                let run = Run::new(tab, level, last - 0.7, last);
                let mut buf = [0.0f32; 16];
                let (mut p, dt) = (0x1234_5678u32, 0x0321_0000u32);
                let t = render::<false>(&run, run.t, d, w, &[], &mut buf, &mut p, dt, 1.0);
                let mut r = run;
                let mut q = 0x1234_5678u32;
                for (j, &y) in buf.iter().enumerate() {
                    let ph = w.apply(q as f32 / ONE, 0.0);
                    let want = r.read(d, ph);
                    assert!(
                        (y - want).abs() < 1e-5,
                        "level {level} mode {mode} frame {j}"
                    );
                    q = q.wrapping_add(dt);
                    r.t += r.t_inc;
                }
                assert_eq!(p, q);
                assert!((t - r.t).abs() < 1e-6);
            }
        }
    }

    /// FM that lands a hair under a whole cycle (which rounds to 1) still
    /// reads inside the last frame.
    #[test]
    fn fm_reads_inside_the_last_frame() {
        let tab = &factory()[0];
        let last = (tab.frames() - 1) as f32;
        let d = tab.levels()[0].data();
        let run = Run::new(tab, 0, last, last);
        let fm = [-1e-9f32; 16];
        let mut buf = [0.0f32; 16];
        let mut p = 0u32;
        render::<false>(
            &run,
            run.t,
            d,
            Warp::new(WARP_FM, 1.0),
            &fm,
            &mut buf,
            &mut p,
            0,
            1.0,
        );
        assert!(buf.iter().all(|x| x.is_finite()));
    }
}
