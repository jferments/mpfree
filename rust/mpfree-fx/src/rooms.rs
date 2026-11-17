//! Three classic reverbs, lighter than the Cloud reverb: [`Room`] (diffused
//! early reflections and a sixteen-line feedback delay network), [`Plate`]
//! (Dattorro's figure-eight tank) and [`Spring`] (chirping stretched
//! all-pass chains in a feedback loop).
//!
//! All three read their delay lines with cubic interpolation where the
//! length moves (Size, modulation) and whole frames elsewhere, flush
//! denormals in every loop, and glide every control.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{
    self, node_impl, param, Blend, Choice, Glide, Hz, Line, Linear, Log, Ms, Percent, NONE,
};
use crate::svf::{Svf, SvfMode};
use crate::{defaults, flush, lerp, Preset};

/// A Schroeder all-pass around a delay line, in Dattorro's form:
/// `w = x + g z`, `y = z - g w`, where `z` is the line's output.
#[derive(Clone, Debug, Default)]
struct Allpass {
    line: Line,
}

impl Allpass {
    fn new(frames: usize) -> Allpass {
        Allpass {
            line: Line::new(frames + 2),
        }
    }
    /// One frame through `d` whole frames of delay.
    #[inline]
    fn run(&mut self, x: f32, g: f32, d: usize) -> f32 {
        let z = self.line.tap(d);
        let w = flush(x + g * z);
        self.line.push(w);
        z - g * w
    }
    /// One frame through a fractional, moving delay of `d` frames (at
    /// least 2).
    #[inline]
    fn run_frac(&mut self, x: f32, g: f32, d: f32) -> f32 {
        let z = self.line.read(d - 1.0);
        let w = flush(x + g * z);
        self.line.push(w);
        z - g * w
    }
    fn clear(&mut self) {
        self.line.clear();
    }
}

/// A one-pole low-pass for damping: `y += (x - y) * k`.
#[derive(Clone, Copy, Debug, Default)]
struct Damp {
    y: f32,
}

impl Damp {
    #[inline]
    fn run(&mut self, x: f32, k: f32) -> f32 {
        self.y = flush(self.y + (x - self.y) * k);
        self.y
    }
}

/// The one-pole coefficient for a corner at `hz`.
fn pole(hz: f32, rate: u32) -> f32 {
    1.0 - (-std::f32::consts::TAU * hz / rate as f32).exp()
}

// ---------------------------------------------------------------- the network

/// The lines of the late network.
const LINES: usize = 16;
/// The most first echoes a side.
const TAPS: usize = 12;
/// The diffusers' gain at Diffusion 100 %.
const DIFF_MAX: f32 = 0.75;
/// Decay is the time to fall 60 dB at this frequency, the middle of the
/// 500 Hz and 1 kHz octaves room acoustics quote it at.
const MID_HZ: f32 = 707.0;
/// How long a Pre-delay change crossfades, ms.
const PRE_FADE_MS: f32 = 20.0;
/// How long the reads take to swap between moving and still, ms.
const SETTLE_MS: f32 = 3.0;
/// The loss filters' glide (time constant), ms.
const LOSS_MS: f32 = 15.0;
/// The lines that wobble, the eight longest: a slow wobble on a long line
/// keeps the tail's resonances drifting apart instead of ringing, where
/// on a short one it is heard as pitch (fewer leave the tail ringing:
/// fxlab's R5).
const MOD_FROM: usize = 8;
/// The most frames the network works through a stage at a time.
const CHUNK: usize = 64;

/// What makes a reverb built on [`Net`] the one it is: its lengths, its
/// first echoes and its wobble.
trait Voice {
    /// The lines' lengths in ms at size 1.
    const LINES_MS: [f32; LINES];
    /// Each side's input diffusers, ms (fixed, whatever the size).
    const DIFF_MS: [[f32; 4]; 2];
    /// A source's first echoes, for a source on the left and on the
    /// right: (ms at size 1, gain on the source's own side, gain on the
    /// other side).
    const EARLY: [&'static [(f32, f32, f32)]; 2];
    /// The smallest and largest size, as a share of the lengths above.
    const SIZE_MIN: f32;
    const SIZE_MAX: f32;
    /// The longest Pre-delay, ms.
    const PRE_MAX_MS: f32;
    /// The wobble's depth at Mod 100 %, frames at 48 kHz.
    const MOD_FRAMES: f32;
    /// The first wobbling line's rate, Hz; each after it 5 % faster.
    const MOD_HZ: f32;
    /// The wet level, so the reverb plays as loud as the dry on pink
    /// noise at its defaults and presets (FX quality, R6).
    const TRIM: f32;
}

/// Each line's loss for a decay of `rt` seconds at [`MID_HZ`] and of `rt`
/// times `1 - 0.9 damping` at the top of the band (Jot's absorption
/// filter: a one-pole low-pass `c0 / (1 - c1 z^-1)` per line, set from
/// its length so every line falls at the same rate). Returns `(c0, c1)`.
pub fn room_loss(secs: f32, rt: f32, damping: f32, rate: u32) -> (f32, f32) {
    let rt = rt.max(0.02);
    let rt_hi = (rt * (1.0 - 0.9 * damping)).max(0.02);
    let g_mid = 10f32.powf(-3.0 * secs / rt);
    let g_top = 10f32.powf(-3.0 * secs / rt_hi);
    let cw = (std::f32::consts::TAU * MID_HZ / rate as f32).cos();
    // The one-pole's gain at DC is `g`, at Nyquist `g (1 - b) / (1 + b)`
    // and at the mid frequency `g m(b)`, a little under `g`: find the `g`
    // and `b` that put the mid and the top where they belong (two rounds
    // settle it, `m` is within 1 % of 1).
    let mut g = g_mid;
    let mut b = 0.0f32;
    for _ in 0..3 {
        let ratio = (g_top / g).clamp(1e-6, 1.0);
        b = (1.0 - ratio) / (1.0 + ratio);
        let m = (1.0 - b) / (1.0 - 2.0 * b * cw + b * b).max(1e-12).sqrt();
        g = (g_mid / m).min(0.9999);
    }
    (g * (1.0 - b), b)
}

/// A 16-point Hadamard transform in place (adds and subtracts only).
#[inline(always)]
fn hadamard16(x: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        let mut i = 0;
        while i < LINES {
            for j in i..i + h {
                let (a, b) = (x[j], x[j + h]);
                x[j] = a + b;
                x[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
}

/// The sign of row `r`, column `c` of the 16-point Hadamard matrix.
const fn had(r: usize, c: usize) -> f32 {
    if (r & c).count_ones().is_multiple_of(2) {
        1.0
    } else {
        -1.0
    }
}

/// How each side's input goes into the lines and how the lines make each
/// side's output: the left feeds the even lines and hears them at full
/// strength (the odd at half), the right the other way round, with signs
/// from two rows of the Hadamard matrix, so the two outputs are
/// uncorrelated and a source's first echoes come back on its own side.
const IN_SIGN: [f32; LINES] = {
    let mut v = [0.0; LINES];
    let mut k = 0;
    while k < LINES {
        v[k] = had(5, k);
        k += 1;
    }
    v
};
#[cfg(test)]
const OUT_VEC: [[f32; LINES]; 2] = {
    let mut v = [[0.0; LINES]; 2];
    let mut k = 0;
    while k < LINES {
        let near = if k % 2 == 0 { 1.0 } else { 0.5 };
        let far = if k % 2 == 0 { 0.5 } else { 1.0 };
        v[0][k] = had(5, k) * near;
        v[1][k] = had(10, k) * far;
        k += 1;
    }
    v
};

/// A ring of samples a power of two long: every ring is written once a
/// frame at the same count `w`, so a read `d` frames back is one mask,
/// with no clamps and no write pointer of its own.
#[derive(Clone, Debug, Default)]
struct Ring {
    buf: Vec<f32>,
    mask: usize,
}

impl Ring {
    /// Room for `frames` of delay (control thread).
    fn new(frames: usize) -> Ring {
        let n = (frames + 4).next_power_of_two();
        Ring {
            buf: vec![0.0; n],
            mask: n - 1,
        }
    }
    /// What was written at count `w - d`.
    #[inline(always)]
    fn at(&self, w: usize, d: usize) -> f32 {
        self.buf[w.wrapping_sub(d) & self.mask]
    }
    /// `d` frames back from count `w` (at least 1), cubic as
    /// [`Line::read`].
    #[inline(always)]
    fn cubic(&self, w: usize, d: f32) -> f32 {
        let i = d as i32 as usize;
        let t = d - i as i32 as f32;
        let (x0, x1, x2, x3) = (
            self.at(w, i - 1),
            self.at(w, i),
            self.at(w, i + 1),
            self.at(w, i + 2),
        );
        let c1 = 0.5 * (x2 - x0);
        let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
        let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
        ((c3 * t + c2) * t + c1) * t + x1
    }
    /// The two runs of the buffer holding counts `start` to
    /// `start + n - 1`, in order.
    #[inline]
    fn runs(&self, start: usize, n: usize) -> (&[f32], &[f32]) {
        let p = start & self.mask;
        let a = n.min(self.buf.len() - p);
        (&self.buf[p..p + a], &self.buf[..n - a])
    }
    /// Reads counts `start` onwards into `out`.
    #[inline]
    fn read(&self, start: usize, out: &mut [f32]) {
        let (a, b) = self.runs(start, out.len());
        out[..a.len()].copy_from_slice(a);
        out[a.len()..].copy_from_slice(b);
    }
    /// Writes `xs` at counts `start` onwards.
    #[inline]
    fn write(&mut self, start: usize, xs: &[f32]) {
        let p = start & self.mask;
        let a = xs.len().min(self.buf.len() - p);
        self.buf[p..p + a].copy_from_slice(&xs[..a]);
        self.buf[..xs.len() - a].copy_from_slice(&xs[a..]);
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// The cubic of [`Line::read`], `t` of a frame further back than count
/// `p` in a ring's buffer (`m` its length less one).
#[inline(always)]
fn cubic_back(buf: &[f32], m: usize, p: usize, t: f32) -> f32 {
    let x0 = buf[p.wrapping_add(1) & m];
    let x1 = buf[p & m];
    let x2 = buf[p.wrapping_sub(1) & m];
    let x3 = buf[p.wrapping_sub(2) & m];
    // The same cubic in fewer operations (de Soras's arrangement).
    let c = 0.5 * (x2 - x0);
    let v = x1 - x2;
    let w = c + v;
    let a = w + v + 0.5 * (x3 - x1);
    ((a * t - (w + a)) * t + c) * t + x1
}

/// The reverb under [`Room`] and [`Plate`]: each side's input through a
/// pre-delay and four all-passes, twelve first echoes a
/// side from a stereo pattern scaled by the size (a source keeps its
/// side), then a sixteen-line feedback delay network mixed by a Hadamard
/// matrix, each line damped by a one-pole set from its length, the decay
/// (the time to fall 60 dB in the mids) and the damping (how much sooner
/// the top dies), so every line falls at the same rate. The eight longest
/// lines wobble slowly to keep the tail from ringing. The gate cuts the
/// tail that long after the sound stops. Pre-delay crossfades to a new
/// time instead of sliding, so turning it never bends the pitch.
struct Net<V: Voice> {
    rate: u32,
    /// Frames since reset: where every ring was last written.
    w: usize,
    /// Frames a stage at a time: [`CHUNK`], or fewer at a rate so low
    /// that the shortest line is shorter.
    chunk: usize,
    /// Frames into the current chunk. Chunks fall on a fixed grid from
    /// reset, whatever the host's blocks, and every choice a chunk makes
    /// is made at its start, so the output is the same, bit for bit, at
    /// any block size.
    cell: usize,
    /// This chunk's choices: whole-frame reads for the size, for the
    /// wobbling lines, and whether they wobble.
    cell_still: bool,
    cell_wob_still: bool,
    cell_wobbling: bool,
    pre: [Ring; 2],
    /// The pre-delay read (frames, as `tap`'s argument) and the one it is
    /// fading to, how far (0 to 1) and by how much a frame.
    pre_at: usize,
    pre_to: usize,
    pre_k: f32,
    pre_step: f32,
    /// The pre-delay wanted, frames.
    pre_want: usize,
    early: [Ring; 2],
    /// The early taps for the size `placed`, whole frames (a moving
    /// size reads fractions, worked out a frame at a time).
    taps_i: [[usize; TAPS]; 2],
    diff: [[Ring; 4]; 2],
    diff_d: [[usize; 4]; 2],
    diff_g: Smooth,
    lines: [Ring; LINES],
    /// Each line's length at the size `placed`, whole frames.
    base_i: [usize; LINES],
    /// Decay (seconds) and Damping (0 to 1).
    rt: f32,
    damping: f32,
    /// Each line's loss filter, `s = c0 z + c1 s`, and its state; and
    /// what the filters are gliding to after Decay, Damping or Size
    /// moved (so they never zip).
    c0: [f32; LINES],
    c1: [f32; LINES],
    c0_to: [f32; LINES],
    c1_to: [f32; LINES],
    loss_moving: bool,
    /// Whether the loss filters move in this chunk, and by how much a
    /// frame.
    ramp: bool,
    dc0: [f32; LINES],
    dc1: [f32; LINES],
    /// One over the glide's time constant, in frames.
    loss_rate: f32,
    state: [f32; LINES],
    /// How far over to the still reads: 0 moving, 1 still.
    still_k: f32,
    /// The same for the early taps, which follow the size alone.
    early_k: f32,
    still_step: f32,
    placed: f32,
    /// The size the loss filters were worked out for.
    lossed: f32,
    phase: [f32; LINES - MOD_FROM],
    lfo_step: [f32; LINES - MOD_FROM],
    wob: [f32; LINES - MOD_FROM],
    dwob: [f32; LINES - MOD_FROM],
    /// The creeping read of each wobbling line while the size holds: how
    /// far back (whole frames) and the fraction beyond.
    creep_d: [usize; LINES - MOD_FROM],
    creep_t: [f32; LINES - MOD_FROM],
    mod_frames: f32,
    /// The late network's input gain, so a longer Decay rings longer
    /// rather than louder.
    late: Smooth,
    /// Frames since reset, for the 16-frame grid.
    n: u32,
    low_cut: Svf,
    /// The size, a share of the lengths at size 1.
    size: Smooth,
    early_gain: Smooth,
    modd: Smooth,
    width: Smooth,
    blend: Blend,
    /// The gate's hold, ms (0: off), and its state.
    gate_ms: f32,
    env: f32,
    env_fall: f32,
    held: u32,
    gate_g: f32,
    gate_step: f32,
    voice: std::marker::PhantomData<V>,
}

impl<V: Voice> Net<V> {
    fn new() -> Net<V> {
        Net {
            rate: 48_000,
            w: 0,
            chunk: CHUNK,
            cell: 0,
            cell_still: false,
            cell_wob_still: false,
            cell_wobbling: false,
            pre: Default::default(),
            pre_at: 0,
            pre_to: 0,
            pre_k: 0.0,
            pre_step: 1.0,
            pre_want: 0,
            early: Default::default(),
            taps_i: [[2; TAPS]; 2],
            diff: Default::default(),
            diff_d: [[1; 4]; 2],
            diff_g: Smooth::new(0.0),
            lines: Default::default(),
            base_i: [1; LINES],
            rt: 1.0,
            damping: 0.5,
            c0: [0.0; LINES],
            c1: [0.0; LINES],
            c0_to: [0.0; LINES],
            c1_to: [0.0; LINES],
            loss_moving: false,
            ramp: false,
            dc0: [0.0; LINES],
            dc1: [0.0; LINES],
            loss_rate: 1.0,
            state: [0.0; LINES],
            still_k: 1.0,
            early_k: 1.0,
            still_step: 1.0,
            placed: f32::NAN,
            lossed: f32::NAN,
            phase: [0.0; LINES - MOD_FROM],
            lfo_step: [0.0; LINES - MOD_FROM],
            wob: [0.0; LINES - MOD_FROM],
            dwob: [0.0; LINES - MOD_FROM],
            creep_d: [0; LINES - MOD_FROM],
            creep_t: [0.0; LINES - MOD_FROM],
            mod_frames: V::MOD_FRAMES,
            late: Smooth::new(1.0),
            n: 0,
            low_cut: Svf::default(),
            size: Smooth::new(1.0),
            early_gain: Smooth::new(0.6),
            modd: Smooth::new(0.0),
            width: Smooth::new(1.0),
            blend: Blend::new(0.25, 0),
            gate_ms: 0.0,
            env: 0.0,
            env_fall: 0.0,
            held: 0,
            gate_g: 1.0,
            gate_step: 0.0,
            voice: std::marker::PhantomData,
        }
    }

    /// Works out every line's loss filter for a size.
    fn set_loss(&mut self, size: f32) {
        let rt = self.rt;
        for k in 0..LINES {
            let secs = V::LINES_MS[k] * 0.001 * size;
            // The Hadamard matrix's 1/4 goes in here.
            let (c0, c1) = room_loss(secs, rt, self.damping, self.rate);
            self.c0_to[k] = 0.25 * c0;
            self.c1_to[k] = c1;
        }
        self.loss_moving = true;
        // A network whose lines keep `g` a pass holds `1 / (1 - g^2)` of
        // the power going in: feed it the inverse, at the mean line.
        let mean = V::LINES_MS.iter().sum::<f32>() / LINES as f32 * 0.001 * size;
        let g = 10f32.powf(-3.0 * mean / rt.max(0.02));
        self.late.target = (1.0 - g * g).max(0.0).sqrt();
        self.lossed = size;
    }

    /// Decay in seconds and Damping (0 to 1).
    fn set_decay(&mut self, rt: f32, damping: f32) {
        self.rt = rt;
        self.damping = damping;
        self.set_loss(self.size.value);
    }

    fn set_predelay_ms(&mut self, ms: f32) {
        self.pre_want = (ms * 0.001 * self.rate as f32).round() as usize;
    }

    /// Places the early taps and the lines' lengths for a size (every
    /// frame while the size glides, once when it settles).
    fn place(&mut self, size: f32) {
        let ms = self.rate as f32 * 0.001;
        for side in 0..2 {
            for (k, &(t, _, _)) in V::EARLY[side].iter().enumerate() {
                self.taps_i[side][k] = (t * ms * size).round() as usize;
            }
        }
        for k in 0..LINES {
            self.base_i[k] = (V::LINES_MS[k] * ms * size).round() as usize;
        }
        self.placed = size;
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mut at = 0;
        while at < l.len() {
            if self.cell == 0 {
                self.begin_chunk();
            }
            let n = (l.len() - at).min(self.chunk - self.cell);
            self.run_chunk(&mut l[at..at + n], &mut r[at..at + n]);
            at += n;
            self.cell += n;
            if self.cell >= self.chunk {
                self.cell = 0;
            }
        }
    }

    /// A chunk's choices, at its first frame. The size moves every read;
    /// once it holds still and the reads have settled, they take whole
    /// frames.
    fn begin_chunk(&mut self) {
        self.cell_still = self.size.value == self.size.target
            && self.placed == self.size.value
            && self.early_k >= 1.0;
        self.cell_wobbling = self.modd.value > 0.0
            || self.modd.target > 0.0
            || self.wob.iter().chain(&self.dwob).any(|&x| x != 0.0);
        // The wobbling lines settle onto whole frames once Mod is off.
        self.cell_wob_still = self.cell_still && !self.cell_wobbling && self.still_k >= 1.0;
        if self.size.value != self.lossed {
            self.set_loss(self.size.value);
        }
        // The loss filters glide, a straight line across the chunk.
        self.ramp = self.loss_moving;
        if self.ramp {
            let n = self.chunk as f32;
            let per = (1.0 - (-n * self.loss_rate).exp()) / n;
            let mut most = 0.0f32;
            for k in 0..LINES {
                let (e0, e1) = (self.c0_to[k] - self.c0[k], self.c1_to[k] - self.c1[k]);
                self.dc0[k] = e0 * per;
                self.dc1[k] = e1 * per;
                most = most.max(e0.abs()).max(e1.abs());
            }
            if most < 1e-6 {
                (self.c0, self.c1) = (self.c0_to, self.c1_to);
                self.ramp = false;
                self.loss_moving = false;
            }
        }
    }

    /// Up to the rest of a chunk, a stage at a time: the pre-delay, the
    /// diffusers, the early echoes, then the network, whose lines are
    /// never shorter than a chunk, so every read in it was written before
    /// it.
    fn run_chunk(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len()).min(CHUNK);
        if n == 0 {
            return;
        }
        let ms = self.rate as f32 * 0.001;
        // Where this chunk's first frame is written.
        let w1 = self.w.wrapping_add(1);
        self.w = self.w.wrapping_add(n);
        let n0 = self.n;
        self.n = n0.wrapping_add(n as u32);

        // Controls, a frame at a time.
        let (still, wobbling) = (self.cell_still, self.cell_wobbling);
        let wob_still = self.cell_wob_still;
        let mut sz = [self.size.value; CHUNK];
        let mut ek = [1.0f32; CHUNK];
        if !still {
            for i in 0..n {
                let size = self.size.glide();
                if size != self.size.target {
                    self.early_k = 0.0;
                } else if self.early_k < 1.0 {
                    self.early_k = (self.early_k + self.still_step).min(1.0);
                }
                sz[i] = size;
                ek[i] = self.early_k;
            }
        }
        let mut sk = [1.0f32; CHUNK];
        if !wob_still {
            for i in 0..n {
                if wobbling || sz[i] != self.size.target {
                    self.still_k = 0.0;
                } else if self.still_k < 1.0 {
                    self.still_k = (self.still_k + self.still_step).min(1.0);
                }
                sk[i] = self.still_k;
            }
        }
        let mut md = [0.0f32; CHUNK];
        if wobbling {
            for m in &mut md[..n] {
                *m = self.modd.value;
                self.modd.glide();
            }
        }

        // Pre-delay, each side, crossfading to a new time.
        self.pre[0].write(w1, &l[..n]);
        self.pre[1].write(w1, &r[..n]);
        let mut d = [[0.0f32; CHUNK]; 2];
        if self.pre_k == 0.0 && self.pre_want == self.pre_at {
            for (side, ds) in d.iter_mut().enumerate() {
                self.pre[side].read(w1.wrapping_sub(self.pre_at), &mut ds[..n]);
            }
        } else {
            for i in 0..n {
                if self.pre_k == 0.0 && self.pre_want != self.pre_at {
                    self.pre_to = self.pre_want;
                    self.pre_k = self.pre_step;
                } else if self.pre_k > 0.0 {
                    self.pre_k += self.pre_step;
                    if self.pre_k >= 1.0 {
                        self.pre_at = self.pre_to;
                        self.pre_k = 0.0;
                    }
                }
                let w = w1.wrapping_add(i);
                for (side, ds) in d.iter_mut().enumerate() {
                    let a = self.pre[side].at(w, self.pre_at);
                    ds[i] = if self.pre_k > 0.0 {
                        a + self.pre_k * (self.pre[side].at(w, self.pre_to) - a)
                    } else {
                        a
                    };
                }
            }
        }
        // Each side's input, diffused: the network's input, and what the
        // early echoes repeat, so each is a cluster rather than a click
        // (the echoes themselves stay out of the network: their comb
        // would colour the whole tail).
        let mut gd = [self.diff_g.value; CHUNK];
        if self.diff_g.value != self.diff_g.target {
            for g in &mut gd[..n] {
                *g = self.diff_g.glide();
            }
        }
        let mut lt = [self.late.value; CHUNK];
        if self.late.value != self.late.target {
            for g in &mut lt[..n] {
                *g = self.late.glide();
            }
        }
        let mut u = d;
        for (side, us) in u.iter_mut().enumerate() {
            // Schroeder all-passes, `v = x + g z`, `y = z - g v`.
            for (ap, &dd) in self.diff[side].iter_mut().zip(&self.diff_d[side]) {
                let buf = &mut ap.buf[..];
                if buf.is_empty() {
                    continue;
                }
                let m = buf.len() - 1;
                for (i, (x, &g)) in us[..n].iter_mut().zip(&gd[..n]).enumerate() {
                    let w = w1.wrapping_add(i);
                    let z = buf[w.wrapping_sub(dd) & m];
                    let v = flush(*x + g * z);
                    buf[w & m] = v;
                    *x = z - g * v;
                }
            }
        }

        // Early echoes: each side's source through its own pattern, onto
        // both sides.
        self.early[0].write(w1, &u[0][..n]);
        self.early[1].write(w1, &u[1][..n]);
        let mut er = [[0.0f32; CHUNK]; 2];
        for src in 0..2 {
            let line = &self.early[src];
            let (a_side, b_side) = er.split_at_mut(1);
            let (own, far) = if src == 0 {
                (&mut a_side[0], &mut b_side[0])
            } else {
                (&mut b_side[0], &mut a_side[0])
            };
            if still {
                for (&at, &(_, a, b)) in self.taps_i[src].iter().zip(V::EARLY[src]) {
                    let (x0, x1) = line.runs(w1.wrapping_sub(at), n);
                    let (o0, o1) = own[..n].split_at_mut(x0.len());
                    let (f0, f1) = far[..n].split_at_mut(x0.len());
                    for ((o, f), &x) in o0.iter_mut().zip(f0.iter_mut()).zip(x0) {
                        *o += a * x;
                        *f += b * x;
                    }
                    for ((o, f), &x) in o1.iter_mut().zip(f1.iter_mut()).zip(x1) {
                        *o += a * x;
                        *f += b * x;
                    }
                }
            } else {
                for i in 0..n {
                    let w = w1.wrapping_add(i);
                    let (mut o, mut f) = (0.0f32, 0.0f32);
                    for &(t, a, b) in V::EARLY[src] {
                        let dd = t * ms * sz[i];
                        let c = line.cubic(w, dd);
                        let x = c + ek[i] * (line.at(w, dd.round() as usize) - c);
                        o += a * x;
                        f += b * x;
                    }
                    own[i] += o;
                    far[i] += f;
                }
            }
        }

        for us in &mut u {
            for (x, &g) in us[..n].iter_mut().zip(&lt[..n]) {
                *x *= g;
            }
        }

        // The network. Read every line for the whole chunk (whole frames
        // while the size holds still, the eight longest wobbling)...
        let mut z = [[0.0f32; CHUNK]; LINES];
        for (k, zk) in z.iter_mut().enumerate() {
            let line = &self.lines[k];
            let zk = &mut zk[..n];
            if k >= MOD_FROM && wobbling {
                let j = k - MOD_FROM;
                let (mut wob, mut dwob, mut ph) = (self.wob[j], self.dwob[j], self.phase[j]);
                let step = self.lfo_step[j];
                let buf = &line.buf[..];
                if buf.is_empty() {
                    continue;
                }
                let m = buf.len() - 1;
                let base = V::LINES_MS[k] * ms;
                let lfo = |i: usize, ph: &mut f32, wob: f32, dwob: &mut f32| -> f32 {
                    *ph = kit::wrap(*ph + step);
                    let to = md[i] * self.mod_frames * kit::sine(*ph);
                    if to == 0.0 && wob.abs() < 1e-3 {
                        // Mod is off and the wobble has come home.
                        *dwob = 0.0;
                        0.0
                    } else {
                        *dwob = (to - wob) * (1.0 / 16.0);
                        wob
                    }
                };
                if still {
                    // The size holds: the read creeps, a whole frame now
                    // and then, with no conversions (from where it was,
                    // inside a chunk).
                    if self.cell == 0 {
                        let d0 = base * sz[0] + wob;
                        self.creep_t[j] = d0 - (d0 as i32) as f32;
                        self.creep_d[j] = d0 as i32 as usize;
                    }
                    let mut t = self.creep_t[j];
                    let mut p = w1.wrapping_sub(self.creep_d[j]);
                    for (i, x) in zk.iter_mut().enumerate() {
                        if n0.wrapping_add(i as u32) & 15 == 0 {
                            let was = wob;
                            wob = lfo(i, &mut ph, wob, &mut dwob);
                            t += wob - was;
                        }
                        wob += dwob;
                        t += dwob;
                        if t >= 1.0 {
                            t -= 1.0;
                            p = p.wrapping_sub(1);
                        } else if t < 0.0 {
                            t += 1.0;
                            p = p.wrapping_add(1);
                        }
                        *x = cubic_back(buf, m, p, t);
                        p = p.wrapping_add(1);
                    }
                    self.creep_t[j] = t;
                    self.creep_d[j] = w1.wrapping_add(n).wrapping_sub(p);
                } else {
                    for (i, x) in zk.iter_mut().enumerate() {
                        if n0.wrapping_add(i as u32) & 15 == 0 {
                            wob = lfo(i, &mut ph, wob, &mut dwob);
                        }
                        wob += dwob;
                        let dd = base * sz[i] + wob;
                        // Through i32: one instruction each way on ARMv7.
                        let di = dd as i32;
                        let p = w1.wrapping_add(i).wrapping_sub(di as usize);
                        *x = cubic_back(buf, m, p, dd - di as f32);
                    }
                }
                self.wob[j] = wob;
                self.dwob[j] = dwob;
                self.phase[j] = ph;
            } else if (k < MOD_FROM && still) || wob_still {
                line.read(w1.wrapping_sub(self.base_i[k]), zk);
            } else {
                let kk = if k >= MOD_FROM { &sk } else { &ek };
                for (i, x) in zk.iter_mut().enumerate() {
                    let w = w1.wrapping_add(i);
                    let dd = V::LINES_MS[k] * ms * sz[i];
                    let c = line.cubic(w, dd);
                    *x = c + kk[i] * (line.at(w, dd.round() as usize) - c);
                }
            }
        }
        // ...then a frame at a time damp, mix and feed back...
        let mut y = [[0.0f32; CHUNK]; 2];
        let mut st = self.state;
        let (mut c0, mut c1) = (self.c0, self.c1);
        let (dc0, dc1, ramp) = (self.dc0, self.dc1, self.ramp);
        for i in 0..n {
            if ramp {
                for k in 0..LINES {
                    c0[k] += dc0[k];
                    c1[k] += dc1[k];
                }
            }
            for k in 0..LINES {
                st[k] = flush(c0[k] * z[k][i] + c1[k] * st[k]);
            }
            let mut s = st;
            hadamard16(&mut s);
            for k in 0..LINES {
                z[k][i] = s[k] + IN_SIGN[k] * u[k & 1][i];
            }
            // The outputs are [`OUT_VEC`] applied to the lines, read off
            // the transform: `had(r, k) * had(1, k)` is `had(r ^ 1, k)`.
            y[0][i] = 0.75 * s[5] + 0.25 * s[4];
            y[1][i] = 0.75 * s[10] - 0.25 * s[11];
        }
        self.state = st;
        (self.c0, self.c1) = (c0, c1);
        // ...and write the chunk back.
        for (line, zk) in self.lines.iter_mut().zip(&z) {
            line.write(w1, &zk[..n]);
        }
        if !still {
            self.place(self.size.value);
        }

        // Out: level, low cut, gate, width and the blend with the dry.
        let gate_frames = (self.gate_ms * ms) as u32;
        let gate_on = gate_frames > 0;
        for i in 0..n {
            let early_gain = self.early_gain.glide();
            let mut v = [0.0f32; 2];
            for side in 0..2 {
                // The 4 undoes the 1/4 folded into the loss filters.
                v[side] = self.low_cut.run(
                    side,
                    V::TRIM * (4.0 * y[side][i] + early_gain * er[side][i]),
                );
            }
            // The gate, keyed by the dry input.
            if gate_on {
                let level = 0.5 * (l[i] + r[i]).abs();
                self.env = if level > self.env {
                    level
                } else {
                    self.env * self.env_fall
                };
                if self.env > 0.01 {
                    self.held = gate_frames;
                    self.gate_g = 1.0;
                } else if self.held > 0 {
                    self.held -= 1;
                } else {
                    self.gate_g = (self.gate_g - self.gate_step).max(0.0);
                }
                v[0] *= self.gate_g;
                v[1] *= self.gate_g;
            }
            let w = self.width.glide();
            let mid = 0.5 * (v[0] + v[1]);
            let g = self.blend.next();
            l[i] = kit::blend(l[i], lerp(mid, v[0], w), g);
            r[i] = kit::blend(r[i], lerp(mid, v[1], w), g);
        }
    }

    /// Everything that follows the rate (control thread); the owner then
    /// applies its parameters and clears.
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let ms = rate as f32 * 0.001;
        // A chunk is written into the pre-delay and the early taps before
        // it is read: room for a chunk more.
        let pre = ((V::PRE_MAX_MS + 1.0) * ms) as usize + 2 + CHUNK;
        self.pre = [Ring::new(pre), Ring::new(pre)];
        let last = |side: usize| V::EARLY[side].last().map_or(0.0, |e| e.0);
        let longest = last(0).max(last(1));
        let early = (longest * V::SIZE_MAX * ms) as usize + 8 + CHUNK;
        self.early = [Ring::new(early), Ring::new(early)];
        self.mod_frames = V::MOD_FRAMES * rate as f32 / 48_000.0;
        // A chunk's reads from the network come from before it (with the
        // cubic's frame either side).
        let shortest = (0..LINES)
            .map(|k| {
                let wob = if k >= MOD_FROM { self.mod_frames } else { 0.0 };
                (V::LINES_MS[k] * V::SIZE_MIN * ms - wob) as usize
            })
            .min()
            .unwrap_or(0);
        self.chunk = shortest.saturating_sub(3).clamp(1, CHUNK);
        for k in 0..LINES {
            let extra = if k >= MOD_FROM {
                self.mod_frames as usize + 8
            } else {
                8
            };
            self.lines[k] = Ring::new((V::LINES_MS[k] * V::SIZE_MAX * ms) as usize + extra);
        }
        for side in 0..2 {
            for (j, &t) in V::DIFF_MS[side].iter().enumerate() {
                let d = ((t * ms).round() as usize).max(1);
                self.diff[side][j] = Ring::new(d);
                self.diff_d[side][j] = d;
            }
        }
        for k in 0..LINES - MOD_FROM {
            self.lfo_step[k] = 16.0 * V::MOD_HZ / rate as f32 * (1.0 + 0.05 * k as f32);
        }
        self.env_fall = crate::db_to_gain(-60.0 / (0.03 * rate as f32));
        // The gate closes over 4 ms.
        self.gate_step = 1.0 / (4.0 * ms);
        self.pre_step = 1.0 / (PRE_FADE_MS * ms);
        self.still_step = 1.0 / (SETTLE_MS * ms);
        self.loss_rate = 1.0 / (LOSS_MS * ms);
        for s in [
            &mut self.size,
            &mut self.early_gain,
            &mut self.modd,
            &mut self.width,
            &mut self.diff_g,
            &mut self.late,
        ] {
            s.set_time(rate, 50.0);
        }
        // The size moves the lines: slowly, so the tail never bends in
        // pitch.
        self.size.set_time(rate, 300.0);
        self.blend.set_time(rate, 50.0);
    }

    fn clear(&mut self) {
        for l in self.pre.iter_mut().chain(&mut self.early) {
            l.clear();
        }
        for l in &mut self.lines {
            l.clear();
        }
        for side in &mut self.diff {
            for ap in side {
                ap.clear();
            }
        }
        self.w = 0;
        self.cell = 0;
        self.ramp = false;
        self.state = [0.0; LINES];
        for (k, p) in self.phase.iter_mut().enumerate() {
            *p = (k as f32 * 0.618_034).fract();
        }
        self.wob = [0.0; LINES - MOD_FROM];
        self.dwob = [0.0; LINES - MOD_FROM];
        self.n = 0;
        self.low_cut.reset();
        self.env = 0.0;
        self.held = 0;
        self.gate_g = 1.0;
        for s in [
            &mut self.size,
            &mut self.early_gain,
            &mut self.modd,
            &mut self.width,
            &mut self.diff_g,
        ] {
            s.snap(s.target);
        }
        self.blend.snap();
        self.pre_at = self.pre_want;
        self.pre_to = self.pre_want;
        self.pre_k = 0.0;
        self.still_k = 1.0;
        self.early_k = 1.0;
        self.placed = f32::NAN;
        let size = self.size.value;
        self.place(size);
        self.set_loss(size);
        (self.c0, self.c1) = (self.c0_to, self.c1_to);
        self.loss_moving = false;
        self.late.snap(self.late.target);
    }
}

// ---------------------------------------------------------------- plate

/// [`Plate`] parameter ids.
pub mod plate {
    pub const PREDELAY: u16 = 0;
    pub const SIZE: u16 = 1;
    pub const DECAY: u16 = 2;
    pub const DAMPING: u16 = 3;
    pub const TONE: u16 = 4;
    pub const MOD: u16 = 5;
    pub const LOW_CUT: u16 = 6;
    pub const WIDTH: u16 = 7;
    pub const MIX: u16 = 8;
    /// How Mix blends ([`crate::kit::MIX_CURVES`]).
    pub const MIX_CURVE: u16 = 9;
}

static PLATE_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.plate",
    name: "Plate reverb",
    kind: Kind::Effect,
};
static PLATE_PARAMS: [ParamInfo; 10] = [
    param(0, "Pre-delay", "Plate", 0.0, 250.0, 10.0, Ms, Linear),
    param(1, "Size", "Plate", 50.0, 150.0, 100.0, Percent, Linear),
    param(2, "Decay", "Plate", 200.0, 20_000.0, 2000.0, Ms, Log),
    param(3, "Damping", "Plate", 0.0, 100.0, 35.0, Percent, Linear),
    param(4, "Tone", "Plate", 1000.0, 20_000.0, 12_000.0, Hz, Log),
    param(5, "Mod", "Plate", 0.0, 100.0, 40.0, Percent, Linear),
    param(6, "Low cut", "Output", 20.0, 1000.0, 80.0, Hz, Log),
    param(7, "Width", "Output", 0.0, 100.0, 100.0, Percent, Linear),
    param(8, "Mix", "Output", 0.0, 100.0, 25.0, Percent, Linear),
    param(
        9,
        "Mix curve",
        "Output",
        0.0,
        2.0,
        0.0,
        NONE,
        Choice(kit::MIX_CURVES),
    ),
];

/// Plate factory presets.
pub static PLATE_PRESETS: &[Preset] = &[
    Preset {
        name: "Vocal plate",
        values: &[(plate::DECAY, 1800.0), (plate::PREDELAY, 30.0)],
    },
    Preset {
        name: "Snare plate",
        values: &[
            (plate::DECAY, 1200.0),
            (plate::SIZE, 80.0),
            (plate::DAMPING, 20.0),
        ],
    },
    Preset {
        name: "Dark plate",
        values: &[
            (plate::DECAY, 3500.0),
            (plate::DAMPING, 70.0),
            (plate::TONE, 4000.0),
        ],
    },
    Preset {
        name: "Endless plate",
        values: &[
            (plate::DECAY, 18000.0),
            (plate::SIZE, 140.0),
            (plate::MOD, 80.0),
        ],
    },
];

/// The reference rate of Dattorro's published lengths.
const DATTORRO_RATE: f32 = 29_761.0;
const PLATE_IN: [usize; 4] = [142, 107, 379, 277];
/// Tank: [modulated all-pass, delay, all-pass, delay] for each half.
const PLATE_TANK: [[usize; 4]; 2] = [[672, 4453, 1800, 3720], [908, 4217, 2656, 3163]];
/// Output taps: (half, element 1 delay / 2 all-pass / 3 delay, frames,
/// sign), left then right, from the paper's table.
const PLATE_TAPS: [[(usize, usize, usize, f32); 7]; 2] = [
    [
        (1, 1, 266, 1.0),
        (1, 1, 2974, 1.0),
        (1, 2, 1913, -1.0),
        (1, 3, 1996, 1.0),
        (0, 1, 1990, -1.0),
        (0, 2, 187, -1.0),
        (0, 3, 1066, -1.0),
    ],
    [
        (0, 1, 353, 1.0),
        (0, 1, 3627, 1.0),
        (0, 2, 1228, -1.0),
        (0, 3, 2673, 1.0),
        (1, 1, 2111, -1.0),
        (1, 2, 335, -1.0),
        (1, 3, 121, -1.0),
    ],
];
/// The tank's modulation: Dattorro's 16 frames at his rate, at about 1 Hz.
const PLATE_EXCURSION: f32 = 16.0;

/// A plate reverb after Jon Dattorro's 1997 design: four input diffusers,
/// then a figure-eight tank of two halves that feed each other, each a
/// modulated all-pass, a delay, damping and a second all-pass. The output
/// is taken from seven points per side, so the two sides are dense and
/// different. The modulation keeps the tank from ringing; Decay is the
/// time to fall 60 dB.
pub struct Plate {
    v: [f32; 10],
    rate: u32,
    /// Frames per Dattorro frame at this rate.
    scale: f32,
    pre: Line,
    bw: Damp,
    input: [Allpass; 4],
    mod_ap: [Allpass; 2],
    delay1: [Line; 2],
    ap2: [Allpass; 2],
    delay2: [Line; 2],
    damp: [Damp; 2],
    /// The value each half last sent to the other.
    feed: [f32; 2],
    lfo: f32,
    lfo_inc: f32,
    low_cut: Svf,
    size: Smooth,
    decay_g: Smooth,
    damping: Smooth,
    pre_frames: Smooth,
    modd: Smooth,
    width: Smooth,
    blend: Blend,
    tone_k: f32,
}

impl Default for Plate {
    fn default() -> Self {
        Self::new()
    }
}

impl Plate {
    pub fn new() -> Plate {
        let v = defaults(&PLATE_PARAMS);
        let mut p = Plate {
            v,
            rate: 48_000,
            scale: 1.0,
            pre: Line::default(),
            bw: Damp::default(),
            input: Default::default(),
            mod_ap: Default::default(),
            delay1: Default::default(),
            ap2: Default::default(),
            delay2: Default::default(),
            damp: [Damp::default(); 2],
            feed: [0.0; 2],
            lfo: 0.0,
            lfo_inc: 0.0,
            low_cut: Svf::default(),
            size: Smooth::new(1.0),
            decay_g: Smooth::new(0.5),
            damping: Smooth::new(0.5),
            pre_frames: Smooth::new(0.0),
            modd: Smooth::new(0.0),
            width: Smooth::new(1.0),
            blend: Blend::new(0.25, 0),
            tone_k: 1.0,
        };
        p.setup(48_000);
        p
    }

    /// The per-pass decay gain for Decay seconds at the size now: each
    /// half of the loop passes two decay gains, so a full trip of
    /// `half` seconds multiplies by g squared.
    fn decay_gain(&self) -> f32 {
        let half: usize = PLATE_TANK[0].iter().sum();
        let secs = half as f32 / DATTORRO_RATE * (self.v[plate::SIZE as usize] / 100.0);
        let rt = self.v[plate::DECAY as usize] * 0.001;
        10f32.powf(-3.0 * secs / (2.0 * rt)).min(0.9995)
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            plate::PREDELAY => self.pre_frames.target = v * 0.001 * self.rate as f32,
            plate::SIZE => {
                self.size.target = v / 100.0;
                self.decay_g.target = self.decay_gain();
            }
            plate::DECAY => self.decay_g.target = self.decay_gain(),
            plate::DAMPING => self.damping.target = 1.0 - 0.85 * v / 100.0,
            plate::TONE => self.tone_k = pole(v, self.rate).min(1.0),
            plate::MOD => self.modd.target = v / 100.0,
            plate::LOW_CUT => self
                .low_cut
                .set(SvfMode::HighPass, v, 0.707, 0.0, self.rate),
            plate::WIDTH => self.width.target = v / 100.0,
            plate::MIX => self.blend.set_mix(v / 100.0),
            plate::MIX_CURVE => self.blend.set_curve(v),
            _ => {}
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let sc = self.scale;
        for i in 0..l.len() {
            let size = self.size.glide();
            let g = self.decay_g.glide();
            let damping = self.damping.glide();
            let modd = self.modd.glide();
            let pre = self.pre_frames.glide();
            let s = sc * size;
            let x = 0.5 * (l[i] + r[i]);
            self.pre.push(x);
            let mut d = if pre >= 2.0 {
                self.pre.read(pre - 1.0)
            } else {
                x
            };
            d = self.bw.run(d, self.tone_k);
            for (k, ap) in self.input.iter_mut().enumerate() {
                let gain = if k < 2 { 0.75 } else { 0.625 };
                d = ap.run(d, gain, ((PLATE_IN[k] as f32 * s) as usize).max(1));
            }
            self.lfo = kit::wrap(self.lfo + self.lfo_inc);
            let exc = PLATE_EXCURSION * sc * modd;
            let diff2 = (g + 0.15).clamp(0.25, 0.5);
            let mut out_half = [0.0f32; 2];
            for h in 0..2 {
                let len = &PLATE_TANK[h];
                let m = if h == 0 {
                    kit::sine(self.lfo)
                } else {
                    kit::cosine(self.lfo)
                };
                let into = d + self.feed[1 - h];
                let a = self.mod_ap[h].run_frac(
                    into,
                    -0.7,
                    (len[0] as f32 * s + exc * m).max(2.0 + exc),
                );
                let z1 = self.delay1[h].tap(((len[1] as f32 * s) as usize).max(1));
                self.delay1[h].push(a);
                let b = self.damp[h].run(z1, damping) * g;
                let c = self.ap2[h].run(b, diff2, ((len[2] as f32 * s) as usize).max(1));
                let z2 = self.delay2[h].tap(((len[3] as f32 * s) as usize).max(1));
                self.delay2[h].push(c);
                out_half[h] = z2 * g;
            }
            self.feed = out_half;
            let mut y = [0.0f32; 2];
            for (side, taps) in PLATE_TAPS.iter().enumerate() {
                for &(h, el, at, sign) in taps {
                    let at = ((at as f32 * s) as usize).max(1);
                    let v = match el {
                        1 => self.delay1[h].tap(at),
                        2 => self.ap2[h].line.tap(at),
                        _ => self.delay2[h].tap(at),
                    };
                    y[side] += sign * v;
                }
                y[side] = self.low_cut.run(side, 0.6 * y[side]);
            }
            let w = self.width.glide();
            let mid = 0.5 * (y[0] + y[1]);
            let (wl, wr) = (lerp(mid, y[0], w), lerp(mid, y[1], w));
            let g = self.blend.next();
            l[i] = kit::blend(l[i], wl, g);
            r[i] = kit::blend(r[i], wr, g);
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.scale = rate as f32 / DATTORRO_RATE;
        // Room for Size 150 % and the excursion.
        let room = |n: usize| (n as f32 * self.scale * 1.5) as usize + 64;
        self.pre = Line::new((0.26 * rate as f32) as usize);
        self.input = PLATE_IN.map(|n| Allpass::new(room(n)));
        for h in 0..2 {
            self.mod_ap[h] = Allpass::new(room(PLATE_TANK[h][0]));
            self.delay1[h] = Line::new(room(PLATE_TANK[h][1]));
            self.ap2[h] = Allpass::new(room(PLATE_TANK[h][2]));
            self.delay2[h] = Line::new(room(PLATE_TANK[h][3]));
        }
        self.lfo_inc = 1.0 / rate as f32;
        for s in [
            &mut self.size,
            &mut self.decay_g,
            &mut self.damping,
            &mut self.modd,
            &mut self.width,
        ] {
            s.set_time(rate, 50.0);
        }
        self.blend.set_time(rate, 50.0);
        self.pre_frames.set_time(rate, 100.0);
        for id in 0..PLATE_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.pre.clear();
        self.bw = Damp::default();
        for a in self
            .input
            .iter_mut()
            .chain(&mut self.mod_ap)
            .chain(&mut self.ap2)
        {
            a.clear();
        }
        for d in self.delay1.iter_mut().chain(&mut self.delay2) {
            d.clear();
        }
        self.damp = [Damp::default(); 2];
        self.feed = [0.0; 2];
        self.lfo = 0.0;
        self.low_cut.reset();
        for s in [
            &mut self.size,
            &mut self.decay_g,
            &mut self.damping,
            &mut self.pre_frames,
            &mut self.modd,
            &mut self.width,
        ] {
            s.snap(s.target);
        }
        self.blend.snap();
    }

    fn tail(&self) -> u32 {
        ((self.v[plate::DECAY as usize] * 0.0012 + 0.3) * self.rate as f32) as u32
    }
    fn rest(&self) -> u32 {
        // Its modulation keeps time across silence (reset, the wobble
        // would start again elsewhere); without it, what it holds is its
        // tail, here to 100 dB down (Decay is the time to fall 60).
        if self.v[plate::MOD as usize] > 0.0 || self.modd.value > 0.0 {
            return u32::MAX;
        }
        let secs = self.v[plate::DECAY as usize] * 0.001 * 100.0 / 60.0
            + self.v[plate::PREDELAY as usize] * 0.001
            + 0.3;
        (secs * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Plate, PLATE_INFO, PLATE_PARAMS, rest);

// ---------------------------------------------------------------- room

/// [`Room`] parameter ids.
pub mod room {
    pub const PREDELAY: u16 = 0;
    pub const SIZE: u16 = 1;
    pub const DECAY: u16 = 2;
    pub const DAMPING: u16 = 3;
    pub const EARLY: u16 = 4;
    pub const MOD: u16 = 5;
    pub const GATE: u16 = 6;
    pub const LOW_CUT: u16 = 7;
    pub const WIDTH: u16 = 8;
    pub const MIX: u16 = 9;
    /// How Mix blends ([`crate::kit::MIX_CURVES`]).
    pub const MIX_CURVE: u16 = 10;
    /// How quickly the tail turns dense: the input diffusers' strength.
    pub const DIFFUSION: u16 = 11;
}

/// The room's voice.
struct RoomVoice;

impl Voice for RoomVoice {
    /// About geometric from 15 to 75 ms at Size 100 %, nudged off simple
    /// ratios so no two lines' echoes keep landing together.
    const LINES_MS: [f32; LINES] = [
        15.3, 16.9, 18.7, 20.9, 23.1, 25.9, 28.4, 31.7, 35.3, 39.1, 43.9, 48.7, 54.1, 60.7, 67.1,
        74.3,
    ];
    const DIFF_MS: [[f32; 4]; 2] = [[1.53, 2.47, 3.91, 5.87], [1.71, 2.69, 4.13, 6.07]];
    /// The nearest walls answer first and mostly on the source's side, so
    /// a sound keeps its place.
    const EARLY: [&'static [(f32, f32, f32)]; 2] = [
        &[
            (2.9, 0.95, 0.30),
            (5.1, 0.42, 0.66),
            (7.6, 0.78, 0.26),
            (10.3, 0.36, 0.52),
            (13.1, 0.62, 0.24),
            (16.4, 0.33, 0.42),
            (19.9, 0.50, 0.22),
            (23.6, 0.30, 0.34),
            (27.8, 0.40, 0.20),
            (32.3, 0.27, 0.28),
            (37.1, 0.32, 0.18),
            (42.6, 0.24, 0.22),
        ],
        &[
            (3.4, 0.95, 0.30),
            (5.8, 0.42, 0.66),
            (8.3, 0.78, 0.26),
            (11.1, 0.36, 0.52),
            (14.2, 0.62, 0.24),
            (17.5, 0.33, 0.42),
            (21.2, 0.50, 0.22),
            (25.3, 0.30, 0.34),
            (29.6, 0.40, 0.20),
            (34.4, 0.27, 0.28),
            (39.4, 0.32, 0.18),
            (45.1, 0.24, 0.22),
        ],
    ];
    const SIZE_MIN: f32 = 0.25;
    const SIZE_MAX: f32 = 1.5;
    const PRE_MAX_MS: f32 = 100.0;
    /// A held note in the tail smears by under 2 cents at the default Mod
    /// (fxlab, R5 and R9).
    const MOD_FRAMES: f32 = 96.0;
    const MOD_HZ: f32 = 0.43;
    const TRIM: f32 = 0.369;
}

static ROOM_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.room",
    name: "Room reverb",
    kind: Kind::Effect,
};
static ROOM_PARAMS: [ParamInfo; 12] = [
    param(0, "Pre-delay", "Room", 0.0, 100.0, 5.0, Ms, Linear),
    param(1, "Size", "Room", 0.0, 100.0, 45.0, Percent, Linear),
    param(2, "Decay", "Room", 100.0, 5000.0, 800.0, Ms, Log),
    param(3, "Damping", "Room", 0.0, 100.0, 50.0, Percent, Linear),
    param(4, "Early", "Room", 0.0, 100.0, 60.0, Percent, Linear),
    param(5, "Mod", "Room", 0.0, 100.0, 25.0, Percent, Linear),
    param(6, "Gate", "Room", 0.0, 600.0, 0.0, Ms, Linear),
    param(7, "Low cut", "Output", 20.0, 1000.0, 60.0, Hz, Log),
    param(8, "Width", "Output", 0.0, 100.0, 100.0, Percent, Linear),
    param(9, "Mix", "Output", 0.0, 100.0, 25.0, Percent, Linear),
    param(
        10,
        "Mix curve",
        "Output",
        0.0,
        2.0,
        0.0,
        NONE,
        Choice(kit::MIX_CURVES),
    ),
    param(11, "Diffusion", "Room", 0.0, 100.0, 70.0, Percent, Linear),
];

/// Room factory presets.
pub static ROOM_PRESETS: &[Preset] = &[
    Preset {
        name: "Small booth",
        values: &[
            (room::SIZE, 15.0),
            (room::DECAY, 350.0),
            (room::EARLY, 80.0),
        ],
    },
    Preset {
        name: "Drum room",
        values: &[
            (room::SIZE, 50.0),
            (room::DECAY, 900.0),
            (room::DAMPING, 35.0),
        ],
    },
    Preset {
        name: "Big studio",
        values: &[
            (room::SIZE, 85.0),
            (room::DECAY, 1800.0),
            (room::PREDELAY, 15.0),
        ],
    },
    Preset {
        name: "Gated snare",
        values: &[
            (room::SIZE, 70.0),
            (room::DECAY, 2500.0),
            (room::GATE, 250.0),
            (room::DAMPING, 20.0),
        ],
    },
];

/// The early reflections' times (ms) and their gain on the source's own
/// side, for drawing: the left source's pattern at `size` (a share of
/// the times at Size 100 %).
pub fn room_early_taps(size: f32) -> impl Iterator<Item = (f32, f32)> {
    RoomVoice::EARLY[0]
        .iter()
        .map(move |&(t, g, _)| (t * size, g))
}

/// The Room's Size, 0 to 100 %, as a share of its lengths at 100 %.
pub fn room_size(v: f32) -> f32 {
    RoomVoice::SIZE_MIN + (RoomVoice::SIZE_MAX - RoomVoice::SIZE_MIN) * v / 100.0
}

/// A room: twelve early reflections a side from a stereo pattern scaled
/// by Size (a source keeps its side), each side's input through four
/// short all-passes (Diffusion), then a sixteen-line feedback delay
/// network mixed by a Hadamard matrix, each line damped by a one-pole set
/// from its length, Decay (the time to fall 60 dB in the mids) and
/// Damping (how much sooner the top dies), so every line falls at the
/// same rate. The eight longest lines wobble slowly (Mod) to keep the tail
/// from ringing. Gate cuts the tail that many ms after the sound stops,
/// the 1980s gated snare. Pre-delay crossfades to a new time instead of
/// sliding, so turning it never bends the pitch.
pub struct Room {
    v: [f32; 12],
    net: Net<RoomVoice>,
}

impl Default for Room {
    fn default() -> Self {
        Self::new()
    }
}

impl Room {
    pub fn new() -> Room {
        let mut r = Room {
            v: defaults(&ROOM_PARAMS),
            net: Net::new(),
        };
        r.setup(48_000);
        r
    }

    fn apply(&mut self, id: u16, v: f32) {
        let net = &mut self.net;
        match id {
            room::PREDELAY => net.set_predelay_ms(v),
            room::SIZE => net.size.target = room_size(v),
            room::DECAY | room::DAMPING => net.set_decay(
                self.v[room::DECAY as usize] * 0.001,
                self.v[room::DAMPING as usize] / 100.0,
            ),
            room::EARLY => net.early_gain.target = v / 100.0,
            room::MOD => net.modd.target = v / 100.0,
            room::GATE => net.gate_ms = v,
            room::LOW_CUT => net.low_cut.set(SvfMode::HighPass, v, 0.707, 0.0, net.rate),
            room::WIDTH => net.width.target = v / 100.0,
            room::MIX => net.blend.set_mix(v / 100.0),
            room::MIX_CURVE => net.blend.set_curve(v),
            room::DIFFUSION => net.diff_g.target = DIFF_MAX * v / 100.0,
            _ => {}
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        self.net.run(l, r);
    }

    fn setup(&mut self, rate: u32) {
        self.net.setup(rate);
        for id in 0..ROOM_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.net.clear();
    }

    fn tail(&self) -> u32 {
        ((self.v[room::DECAY as usize] * 0.0012 + 0.2) * self.net.rate as f32) as u32
    }
    fn rest(&self) -> u32 {
        // Its modulation keeps time across silence, and its gate shuts
        // where a reset would open it; without either, what it holds is
        // its tail, here to 100 dB down (Decay is the time to fall 60).
        if self.v[room::MOD as usize] > 0.0
            || self.net.modd.value > 0.0
            || self.v[room::GATE as usize] > 0.0
        {
            return u32::MAX;
        }
        let secs = self.v[room::DECAY as usize] * 0.001 * 100.0 / 60.0
            + self.v[room::PREDELAY as usize] * 0.001
            + 0.2;
        (secs * self.net.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Room, ROOM_INFO, ROOM_PARAMS, rest);

// ---------------------------------------------------------------- spring

/// [`Spring`] parameter ids.
pub mod spring {
    pub const LENGTH: u16 = 0;
    pub const DECAY: u16 = 1;
    pub const TENSION: u16 = 2;
    pub const DRIP: u16 = 3;
    pub const TONE: u16 = 4;
    pub const LOW_CUT: u16 = 5;
    pub const MIX: u16 = 6;
    /// How Mix blends ([`crate::kit::MIX_CURVES`]).
    pub const MIX_CURVE: u16 = 7;
}

/// Stretched all-pass stages per spring (Parker's low chirp path, cut to
/// fit the MPC: 100 in the full model).
const STAGES: usize = 40;
/// The stretch: the chirp sits below rate / (2 K), about 4.8 kHz at 48 kHz.
const STRETCH: usize = 5;

static SPRING_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.spring",
    name: "Spring reverb",
    kind: Kind::Effect,
};
static SPRING_PARAMS: [ParamInfo; 8] = [
    param(0, "Length", "Spring", 20.0, 80.0, 40.0, Ms, Linear),
    param(1, "Decay", "Spring", 500.0, 8000.0, 2500.0, Ms, Log),
    param(2, "Tension", "Spring", 0.0, 100.0, 50.0, Percent, Linear),
    param(3, "Drip", "Spring", 0.0, 100.0, 50.0, Percent, Linear),
    param(4, "Tone", "Spring", 1000.0, 8000.0, 4000.0, Hz, Log),
    param(5, "Low cut", "Spring", 20.0, 1000.0, 150.0, Hz, Log),
    param(6, "Mix", "Spring", 0.0, 100.0, 30.0, Percent, Linear),
    param(
        7,
        "Mix curve",
        "Output",
        0.0,
        2.0,
        0.0,
        NONE,
        Choice(kit::MIX_CURVES),
    ),
];

/// Spring factory presets.
pub static SPRING_PRESETS: &[Preset] = &[
    Preset {
        name: "Amp spring",
        values: &[(spring::DECAY, 2000.0), (spring::DRIP, 40.0)],
    },
    Preset {
        name: "Dub splash",
        values: &[
            (spring::DECAY, 4500.0),
            (spring::DRIP, 90.0),
            (spring::TENSION, 70.0),
        ],
    },
    Preset {
        name: "Short tank",
        values: &[
            (spring::LENGTH, 25.0),
            (spring::DECAY, 1000.0),
            (spring::TONE, 6000.0),
        ],
    },
];

/// One spring: the chirp chain, the round-trip delay and its loss.
#[derive(Clone, Debug)]
struct OneSpring {
    /// Each stage's last STRETCH inputs and outputs (fixed arrays, so
    /// the chain's loop has no bounds checks).
    xs: [[f32; STRETCH]; STAGES],
    ys: [[f32; STRETCH]; STAGES],
    at: usize,
    line: Line,
    lp: Svf,
    fb: f32,
}

impl Default for OneSpring {
    fn default() -> Self {
        OneSpring {
            xs: [[0.0; STRETCH]; STAGES],
            ys: [[0.0; STRETCH]; STAGES],
            at: 0,
            line: Line::default(),
            lp: Svf::default(),
            fb: 0.0,
        }
    }
}

impl OneSpring {
    fn new(frames: usize) -> OneSpring {
        OneSpring {
            line: Line::new(frames),
            ..OneSpring::default()
        }
    }

    /// The chirp: STAGES first-order all-passes in z^-K, each
    /// `y = a x + x[n-K] - a y[n-K]`, which delays lows more than highs
    /// below rate / 2K, the spring's falling "pew".
    #[inline]
    fn chirp(&mut self, mut x: f32, a: f32) -> f32 {
        let at = self.at.min(STRETCH - 1);
        for (xs, ys) in self.xs.iter_mut().zip(self.ys.iter_mut()) {
            let y = flush(a * (x - ys[at]) + xs[at]);
            xs[at] = x;
            ys[at] = y;
            x = y;
        }
        self.at = if at + 1 == STRETCH { 0 } else { at + 1 };
        x
    }

    fn clear(&mut self) {
        self.xs = [[0.0; STRETCH]; STAGES];
        self.ys = [[0.0; STRETCH]; STAGES];
        self.at = 0;
        self.line.clear();
        self.lp.reset();
        self.fb = 0.0;
    }
}

/// A spring tank: two springs of slightly different lengths (one per
/// side). Each passes the sound through a long chain of stretched
/// all-passes, which smears it into the spring's chirp, then round a
/// delay of Length with a loss for Decay, so each return chirps again.
/// Drip feeds more of each chirp back into the spring, for the dub splash
/// a tap on the tank gives. After Välimäki, Parker and Abel's parametric
/// spring model, with the chain cut to 40 stages for the MPC.
pub struct Spring {
    v: [f32; 8],
    rate: u32,
    springs: [OneSpring; 2],
    in_hp: Svf,
    length: Smooth,
    gain: f32,
    a: f32,
    drip: Smooth,
    blend: Blend,
}

impl Default for Spring {
    fn default() -> Self {
        Self::new()
    }
}

impl Spring {
    pub fn new() -> Spring {
        let v = defaults(&SPRING_PARAMS);
        let mut s = Spring {
            v,
            rate: 48_000,
            springs: Default::default(),
            in_hp: Svf::default(),
            length: Smooth::new(v[0]),
            gain: 0.5,
            a: -0.6,
            drip: Smooth::new(0.5),
            blend: Blend::new(0.3, 0),
        };
        s.setup(48_000);
        s
    }

    fn set_gain(&mut self) {
        let secs = self.v[spring::LENGTH as usize] * 0.001;
        self.gain = 10f32
            .powf(-3.0 * secs / (self.v[spring::DECAY as usize] * 0.001))
            .min(0.98);
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            spring::LENGTH => {
                self.length.target = v;
                self.set_gain();
            }
            spring::DECAY => self.set_gain(),
            // Negative, so lows are delayed more than highs below the
            // corner (a positive coefficient chirps the other way).
            spring::TENSION => self.a = -(0.45 + 0.3 * v / 100.0),
            spring::DRIP => self.drip.target = v / 100.0,
            spring::TONE => {
                for s in &mut self.springs {
                    s.lp.set(SvfMode::LowPass, v, 0.6, 0.0, self.rate);
                }
            }
            spring::LOW_CUT => self.in_hp.set(SvfMode::HighPass, v, 0.707, 0.0, self.rate),
            spring::MIX => self.blend.set_mix(v / 100.0),
            spring::MIX_CURVE => self.blend.set_curve(v),
            _ => {}
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let ms = self.rate as f32 * 0.001;
        for i in 0..l.len() {
            let len = self.length.glide();
            let drip = self.drip.glide();
            let x = self.in_hp.run(0, 0.5 * (l[i] + r[i]));
            let mut y = [0.0f32; 2];
            for (k, s) in self.springs.iter_mut().enumerate() {
                // The second spring is 13 % longer, so the sides differ.
                let frames = len * ms * (1.0 + 0.13 * k as f32);
                let back = s.line.read(frames - 1.0);
                let into = x + s.fb;
                let c = s.chirp(into, self.a);
                let c = s.lp.run(0, c);
                s.line.push(c);
                // What comes back round, chirped again, plus Drip's extra
                // ring of the fresh chirp.
                s.fb = flush(self.gain * (back + drip * 0.25 * (c - back)));
                y[k] = back + 0.3 * c;
            }
            let g = self.blend.next();
            l[i] = kit::blend(l[i], y[0], g);
            r[i] = kit::blend(r[i], y[1], g);
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let frames = (80.0 * 1.13 * rate as f32 * 0.001) as usize + 8;
        self.springs = [OneSpring::new(frames), OneSpring::new(frames)];
        self.length.set_time(rate, 200.0);
        self.drip.set_time(rate, 30.0);
        self.blend.set_time(rate, 20.0);
        for id in 0..SPRING_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        for s in &mut self.springs {
            s.clear();
        }
        self.in_hp.reset();
        self.length.snap(self.length.target);
        self.drip.snap(self.drip.target);
        self.blend.snap();
    }

    fn tail(&self) -> u32 {
        ((self.v[spring::DECAY as usize] * 0.0012 + 0.2) * self.rate as f32) as u32
    }
    fn rest(&self) -> u32 {
        // What it holds is its tail, here to 100 dB down (Decay is the
        // time to fall 60).
        let secs = self.v[spring::DECAY as usize] * 0.001 * 100.0 / 60.0 + 0.2;
        (secs * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Spring, SPRING_INFO, SPRING_PARAMS, rest);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outputs_read_off_the_transform_are_the_output_vectors() {
        let mut seed = 7u32;
        for _ in 0..20 {
            let mut v = [0.0f32; LINES];
            for x in &mut v {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *x = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            }
            let want: Vec<f32> = (0..2)
                .map(|o| (0..LINES).map(|k| OUT_VEC[o][k] * v[k]).sum())
                .collect();
            let mut s = v;
            hadamard16(&mut s);
            assert!((0.75 * s[5] + 0.25 * s[4] - want[0]).abs() < 1e-5);
            assert!((0.75 * s[10] - 0.25 * s[11] - want[1]).abs() < 1e-5);
        }
    }

    #[test]
    fn every_line_is_longer_than_a_chunk_at_every_rate() {
        for rate in [22_050, 44_100, 48_000, 96_000] {
            let mut r = Room::new();
            r.setup(rate);
            let ms = rate as f32 * 0.001;
            for k in 0..LINES {
                let wob = if k >= MOD_FROM { r.net.mod_frames } else { 0.0 };
                let shortest = RoomVoice::LINES_MS[k] * RoomVoice::SIZE_MIN * ms - wob;
                assert!(shortest >= r.net.chunk as f32 + 2.0, "{rate} {k}");
            }
        }
    }
}
