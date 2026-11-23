//! Spleeter's U-Net, one per stem, as `spleeter/model/functions/unet.py`
//! builds it: six 5×5 stride-2 convolutions (16 to 512 filters, each
//! batch-normed and through an ELU), six 5×5 stride-2 transposed
//! convolutions (ELU, then batch norm, each but the first joined with the
//! encoder's output of its size), then a 4×4 convolution dilated by 2 and a
//! sigmoid: a soft mask, times the mix's magnitude. Tensors are rows (STFT
//! frames) × columns (bins) × channels.
//!
//! Every layer is a GEMM over patches gathered straight into the kernel's
//! packed layout ([`run_gather`]), except the two narrow ones: the last
//! transposed convolution (to one channel) scatters a GEMM to its 25 taps
//! ([`run_scatter`]) and the mask layer (one channel to two) runs row by
//! row ([`run_mask`]), so neither pads its outputs to the kernel's eight.

use crate::gemm::{gemm, macs, pack_a, BView, PackedB, Work, KC};
use mpfree_simd::{GEMM_MR as MR, GEMM_NR as NR};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tensor {
    pub h: usize,
    pub w: usize,
    pub c: usize,
    pub data: Vec<f32>,
}

impl Tensor {
    pub fn zeros(h: usize, w: usize, c: usize) -> Tensor {
        Tensor {
            h,
            w,
            c,
            data: vec![0.0; h * w * c],
        }
    }
}

#[inline]
fn elu(x: f32) -> f32 {
    if x > 0.0 {
        x
    } else {
        x.exp() - 1.0
    }
}

#[inline]
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// The network's shape: the encoder's filters (the decoder's mirror them)
/// and the rows of a segment. Spleeter's is [`Shape::SPLEETER`]; the tests
/// run the same code on small ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shape {
    pub filters: [usize; 6],
    /// STFT frames a segment (a multiple of 64: six halvings).
    pub rows: usize,
}

/// What a tensor in the model file is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    /// A kernel, and about how many inputs each output sums (its fan-in).
    Kernel(usize),
    Bias,
    /// A batch norm's scale (gamma / sqrt(variance + 0.001)) and shift
    /// (beta − mean · scale).
    Scale,
    Shift,
}

impl Shape {
    pub const SPLEETER: Shape = Shape {
        filters: [16, 32, 64, 128, 256, 512],
        rows: 512,
    };

    /// The decoder's outputs, layer by layer: the encoder's filters back
    /// down, then one channel.
    pub fn outs(&self) -> [usize; 6] {
        let f = self.filters;
        [f[4], f[3], f[2], f[1], f[0], 1]
    }

    /// The decoder's inputs: the deepest encoder output, then each layer's
    /// output joined with the encoder's of its size.
    pub fn ins(&self) -> [usize; 6] {
        let f = self.filters;
        [f[5], 2 * f[4], 2 * f[3], 2 * f[2], 2 * f[1], 2 * f[0]]
    }

    /// One stem's tensors in file order, each (values, kind): seven
    /// convolutions (kernel, bias; the seventh the mask layer's), six
    /// transposed ones, then twelve batch norms (scale, shift).
    pub fn stem_layout(&self) -> Vec<(usize, Kind)> {
        let mut out = Vec::new();
        let mut cin = 2;
        for &f in &self.filters {
            out.push((25 * cin * f, Kind::Kernel(25 * cin)));
            out.push((f, Kind::Bias));
            cin = f;
        }
        out.push((32, Kind::Kernel(16)));
        out.push((2, Kind::Bias));
        let (outs, ins) = (self.outs(), self.ins());
        for j in 0..6 {
            out.push((25 * outs[j] * ins[j], Kind::Kernel(25 * ins[j] / 4)));
            out.push((outs[j], Kind::Bias));
        }
        for j in 0..12 {
            let n = if j < 6 { self.filters[j] } else { outs[j - 6] };
            out.push((n, Kind::Scale));
            out.push((n, Kind::Shift));
        }
        out
    }

    /// The most floats a U-Net run holds at once for a segment of `bins`
    /// columns, besides its input and the weights: the encoder's outputs
    /// kept for the skips, plus the layer's input and output, at the
    /// layer where that is most (the same walk [`Unet::run`] takes), and
    /// the mask layer's input and output.
    pub fn peak_floats(&self, bins: usize) -> usize {
        let (mut h, mut w) = (self.rows, bins);
        let mut raw = Vec::new();
        let mut most = 0;
        // The encoder: kept outputs, this layer's input (from the second
        // layer on), its output and the next layer's input beside it.
        let mut input = 0;
        for &f in &self.filters {
            (h, w) = (h.div_ceil(2), w.div_ceil(2));
            let out = h * w * f;
            let kept: usize = raw.iter().sum();
            most = most.max(kept + input + 2 * out);
            raw.push(out);
            input = out;
        }
        // The decoder: kept outputs, the skip and the last output (the
        // input), and this output; the last layer's scatter also holds its
        // output's rows a second time while it sums them.
        let mut skip = raw.pop().unwrap_or(0);
        let mut prev = 0;
        for (j, &c) in self.outs().iter().enumerate() {
            (h, w) = (h * 2, w * 2);
            let out = h * w * c;
            let kept: usize = raw.iter().sum();
            let sums = if j == 5 { out } else { 0 };
            most = most.max(kept + skip + prev + out + sums);
            prev = out;
            skip = raw.pop().unwrap_or(0);
        }
        // The mask layer: its input (one channel) and the mask.
        most.max(prev + h * w * 2)
    }
}

/// A layer's input: one tensor, or two joined along channels (a skip
/// join, read in place rather than copied into one).
#[derive(Clone, Copy)]
pub(crate) struct Input<'a> {
    pub a: &'a Tensor,
    pub b: Option<&'a Tensor>,
}

impl<'a> Input<'a> {
    pub fn one(a: &'a Tensor) -> Input<'a> {
        Input { a, b: None }
    }
    fn c(&self) -> usize {
        self.a.c + self.b.map_or(0, |b| b.c)
    }
}

/// A convolution as a gather: each output pixel (gy, gx) of a grid reads
/// input (gy·stride + dy, gx·stride + dx) for each tap, and lands at
/// (gy·ys + yo, gx·xs + xo) of the output.
pub(crate) struct Gather {
    pub taps: Vec<(i32, i32)>,
    pub stride: usize,
    pub cin: usize,
    pub cout: usize,
    pub b: PackedB,
    pub bias: Vec<f32>,
}

pub(crate) struct Place {
    pub grid_h: usize,
    pub grid_w: usize,
    pub ys: usize,
    pub yo: usize,
    pub xs: usize,
    pub xo: usize,
}

impl Place {
    fn whole(h: usize, w: usize) -> Place {
        Place {
            grid_h: h,
            grid_w: w,
            ys: 1,
            yo: 0,
            xs: 1,
            xo: 0,
        }
    }
}

/// What happens to a gather's sums.
pub(crate) enum Post<'a> {
    /// The encoder's: the sum is kept (the skip), and its batch-normed ELU
    /// goes to a second tensor (the next layer's input).
    Encoder {
        next: &'a mut Tensor,
        scale: &'a [f32],
        shift: &'a [f32],
    },
    /// The decoder's: ELU, then the batch norm.
    Decoder { scale: &'a [f32], shift: &'a [f32] },
}

/// A grid row of a gather: its index, the output row it writes, and the
/// encoder's second output row beside it.
type Row<'a> = (usize, &'a mut [f32], Option<&'a mut [f32]>);

/// Pixels a tile: the gathered patches near 256 kB.
fn tile(k: usize, grid_w: usize) -> usize {
    ((65536 / k.max(1)).clamp(4, 512) / 4 * 4)
        .min(grid_w.div_ceil(4) * 4)
        .max(4)
}

/// Floats one thread of [`run_gather`] holds: its patches, its sums and
/// a pixel of zeros.
fn gather_floats(g: &Gather, grid_w: usize) -> usize {
    let k = g.taps.len() * g.cin;
    let t = tile(k, grid_w);
    t.div_ceil(MR) * MR * k + t * g.b.panels * NR + g.cin
}

/// Splits `rows` among `threads`, at least one a thread.
fn share(rows: usize, threads: usize) -> usize {
    rows.div_ceil(threads.max(1)).max(1)
}

pub(crate) fn run_gather(
    g: &Gather,
    input: Input,
    out: &mut Tensor,
    at: &Place,
    post: Post,
    threads: usize,
    work: &mut Work,
) {
    assert_eq!(input.c(), g.cin);
    if let Some(b) = input.b {
        assert!(b.h == input.a.h && b.w == input.a.w);
    }
    assert_eq!(out.c, g.cout);
    let (ih, iw) = (input.a.h, input.a.w);
    let k = g.taps.len() * g.cin;
    let row_len = out.w * out.c;
    let keep = |y: usize| {
        y >= at.yo && (y - at.yo).is_multiple_of(at.ys) && (y - at.yo) / at.ys < at.grid_h
    };
    type Bn<'b> = (&'b [f32], &'b [f32]);
    let (also, (scale, shift), decoder): (Vec<Option<&mut [f32]>>, Bn, Option<Bn>) = match post {
        Post::Encoder { next, scale, shift } => {
            assert!(next.h == out.h && next.w == out.w && next.c == out.c);
            let rows = next.data.chunks_mut(row_len).enumerate();
            let rows = rows.filter(|(y, _)| keep(*y)).map(|(_, r)| Some(r));
            (rows.collect(), (scale, shift), None)
        }
        Post::Decoder { scale, shift } => {
            let none = (0..at.grid_h).map(|_| None).collect();
            (none, (&[], &[]), Some((scale, shift)))
        }
    };
    // The output rows this grid writes, each a disjoint slice.
    let mut rows: Vec<Row> = out
        .data
        .chunks_mut(row_len)
        .enumerate()
        .filter(|(y, _)| keep(*y))
        .zip(also)
        .map(|((y, r), a)| ((y - at.yo) / at.ys, r, a))
        .collect();
    let per = share(rows.len(), threads);
    let tile = tile(k, at.grid_w);
    let padded: usize = (0..at.grid_w)
        .step_by(tile)
        .map(|x0| tile.min(at.grid_w - x0).div_ceil(MR) * MR)
        .sum();
    work.macs += macs(padded, &g.b) * at.grid_h as u64;
    let bv: BView = g.b.view(work);
    std::thread::scope(|sc| {
        for chunk in rows.chunks_mut(per) {
            sc.spawn(move || {
                // Patches go straight into the kernel's packed layout.
                let mut packed = vec![0f32; tile.div_ceil(MR) * MR * k];
                let ldc = g.b.panels * NR;
                let mut c = vec![0f32; tile * ldc];
                let zeros = vec![0f32; g.cin];
                for (gy, row, also_row) in chunk.iter_mut() {
                    let gy = *gy;
                    let mut gx0 = 0;
                    while gx0 < at.grid_w {
                        let m = tile.min(at.grid_w - gx0);
                        let rp = m.div_ceil(MR);
                        for p in 0..rp * MR {
                            let (pn, lane) = (p / MR, p % MR);
                            let gx = gx0 + p;
                            for (t, &(dy, dx)) in g.taps.iter().enumerate() {
                                let iy = (gy * g.stride) as i32 + dy;
                                let ix = (gx * g.stride) as i32 + dx;
                                let col = t * g.cin;
                                let inside = p < m
                                    && iy >= 0
                                    && ix >= 0
                                    && (iy as usize) < ih
                                    && (ix as usize) < iw;
                                if !inside {
                                    put(&mut packed, k, rp, pn, lane, col, &zeros);
                                    continue;
                                }
                                let px = iy as usize * iw + ix as usize;
                                let ca = input.a.c;
                                let a = &input.a.data[px * ca..(px + 1) * ca];
                                put(&mut packed, k, rp, pn, lane, col, a);
                                if let Some(b) = input.b {
                                    let bp = &b.data[px * b.c..(px + 1) * b.c];
                                    put(&mut packed, k, rp, pn, lane, col + ca, bp);
                                }
                            }
                        }
                        gemm(m, &packed, bv, &mut c);
                        for p in 0..m {
                            let ox = (gx0 + p) * at.xs + at.xo;
                            let dst = &mut row[ox * g.cout..(ox + 1) * g.cout];
                            let sums = &c[p * ldc..p * ldc + g.cout];
                            for co in 0..g.cout {
                                let v = sums[co] + g.bias[co];
                                dst[co] = match decoder {
                                    Some((s, b)) => elu(v) * s[co] + b[co],
                                    None => v,
                                };
                            }
                            if let Some(ar) = also_row.as_deref_mut() {
                                let ad = &mut ar[ox * g.cout..(ox + 1) * g.cout];
                                for co in 0..g.cout {
                                    ad[co] = elu(dst[co] * scale[co] + shift[co]);
                                }
                            }
                        }
                        gx0 += m;
                    }
                }
            });
        }
    });
}

/// Values for columns `col..` of packed row (pn, lane), in
/// [`pack_a`]'s layout: KC blocks of row panels, each panel kc columns of
/// MR lanes.
#[inline]
fn put(packed: &mut [f32], k: usize, rp: usize, pn: usize, lane: usize, col: usize, src: &[f32]) {
    let (mut col, mut i) = (col, 0);
    while i < src.len() {
        let (b, kk) = (col / KC, col % KC);
        let kc = KC.min(k - b * KC);
        let run = (KC - kk).min(src.len() - i);
        let mut off = b * KC * rp * MR + pn * kc * MR + kk * MR + lane;
        for &v in &src[i..i + run] {
            packed[off] = v;
            off += MR;
        }
        i += run;
        col += run;
    }
}

/// A 5×5 stride-2 'same' transposed convolution to one channel, as a GEMM
/// to all 25 kernel taps of each input pixel (K = cin, N = 25), each tap's
/// value then added into the output pixel it lands on.
pub(crate) struct Scatter {
    pub cin: usize,
    pub b: PackedB,
    pub bias: f32,
}

impl Scatter {
    /// From TF's kernel [5, 5, cout = 1, cin]: column kh·5 + kw, row ci.
    fn new(kernel: &[f32], bias: &[f32], cin: usize) -> Scatter {
        let b = PackedB::new(cin, 25, |ci, t| kernel[t * cin + ci]);
        Scatter {
            cin,
            b,
            bias: bias[0],
        }
    }
}

/// Output rows a thread of [`run_scatter`] owns, and the input rows that
/// reach them (input row iy lands on output rows 2·iy − 1 to 2·iy + 3).
fn scatter_rows(band: usize, i: usize, oh: usize, h: usize) -> (usize, usize, usize, usize) {
    let (r0, r1) = (i * band, ((i + 1) * band).min(oh));
    let lo = r0.saturating_sub(3) / 2;
    let hi = (r1 + 1).div_ceil(2).min(h);
    (r0, r1, lo, hi)
}

/// Floats one thread of [`run_scatter`] holds besides its rows' sums: an
/// input row packed and joined, and its 25 taps' values.
fn scatter_floats(s: &Scatter, w: usize) -> usize {
    let wp = w.div_ceil(MR) * MR;
    wp * s.cin + w * s.b.panels * NR + w * s.cin
}

/// The output (2h × 2w × 1) of a [`Scatter`] on input (h × w × cin),
/// through the ELU and the batch norm (scale, shift). Each thread owns a
/// band of output rows and works out the input rows that reach it.
pub(crate) fn run_scatter(
    s: &Scatter,
    input: Input,
    out: &mut Tensor,
    scale: f32,
    shift: f32,
    threads: usize,
    work: &mut Work,
) {
    assert_eq!(input.c(), s.cin);
    let (h, w) = (input.a.h, input.a.w);
    let (oh, ow) = (2 * h, 2 * w);
    assert!(out.h == oh && out.w == ow && out.c == 1);
    let ldc = s.b.panels * NR;
    let band = share(oh, threads);
    for i in 0..oh.div_ceil(band) {
        let (_, _, lo, hi) = scatter_rows(band, i, oh, h);
        work.macs += macs(w, &s.b) * (hi - lo) as u64;
    }
    let bv = s.b.view(work);
    std::thread::scope(|sc| {
        for (i, rows) in out.data.chunks_mut(band * ow).enumerate() {
            sc.spawn(move || {
                let (r0, r1, lo, hi) = scatter_rows(band, i, oh, h);
                let mut acc = vec![0f32; rows.len()];
                let mut packed = Vec::with_capacity(w.div_ceil(MR) * MR * s.cin);
                let mut z = vec![0f32; w * ldc];
                let mut joined = vec![0f32; if input.b.is_some() { w * s.cin } else { 0 }];
                for iy in lo..hi {
                    let a: &[f32] = match input.b {
                        Some(b) => {
                            let ca = input.a.c;
                            for ix in 0..w {
                                let px = iy * w + ix;
                                let dst = &mut joined[ix * s.cin..(ix + 1) * s.cin];
                                dst[..ca].copy_from_slice(&input.a.data[px * ca..(px + 1) * ca]);
                                dst[ca..].copy_from_slice(&b.data[px * b.c..(px + 1) * b.c]);
                            }
                            &joined
                        }
                        None => &input.a.data[iy * w * s.cin..(iy + 1) * w * s.cin],
                    };
                    pack_a(a, w, s.cin, &mut packed);
                    gemm(w, &packed, bv, &mut z);
                    for kh in 0..5 {
                        let oy = 2 * iy + kh;
                        if oy <= r0 || oy > r1 {
                            continue;
                        }
                        let o = oy - 1 - r0;
                        let orow = &mut acc[o * ow..(o + 1) * ow];
                        for ix in 0..w {
                            let zt = &z[ix * ldc + kh * 5..ix * ldc + kh * 5 + 5];
                            for (kw, &v) in zt.iter().enumerate() {
                                let ox = 2 * ix + kw;
                                if ox >= 1 && ox - 1 < ow {
                                    orow[ox - 1] += v;
                                }
                            }
                        }
                    }
                }
                for (d, &v) in rows.iter_mut().zip(&acc) {
                    *d = elu(v + s.bias) * scale + shift;
                }
                // Every output row of the band was reached.
                debug_assert_eq!(rows.len(), (r1 - r0) * ow);
            });
        }
    });
}

/// The mask layer, a 4×4 'same' convolution dilated by 2 from one channel
/// to two, then the sigmoid, times the mix's magnitude `x`: row by row
/// over zero-padded input rows.
pub(crate) fn run_mask(
    w: &[f32],
    bias: &[f32],
    input: &Tensor,
    x: &Tensor,
    out: &mut Tensor,
    threads: usize,
) {
    assert!(input.c == 1 && out.c == 2 && x.c == 2 && w.len() == 32 && bias.len() == 2);
    let (h, wd) = (input.h, input.w);
    let band = share(h, threads);
    std::thread::scope(|sc| {
        for (bi, rows) in out.data.chunks_mut(band * wd * 2).enumerate() {
            sc.spawn(move || {
                let y0 = bi * band;
                let mut padded = vec![0f32; wd + 6];
                let mut acc = vec![[0f32; 2]; wd];
                for (r, orow) in rows.chunks_mut(wd * 2).enumerate() {
                    let y = y0 + r;
                    acc.iter_mut().for_each(|a| *a = [0.0; 2]);
                    for kh in 0..4 {
                        let iy = (y + kh * 2).checked_sub(3).filter(|&iy| iy < h);
                        match iy {
                            Some(iy) => padded[3..3 + wd]
                                .copy_from_slice(&input.data[iy * wd..(iy + 1) * wd]),
                            None => padded.fill(0.0),
                        }
                        for kw in 0..4 {
                            let (w0, w1) = (w[(kh * 4 + kw) * 2], w[(kh * 4 + kw) * 2 + 1]);
                            let src = &padded[kw * 2..kw * 2 + wd];
                            for (a, &v) in acc.iter_mut().zip(src) {
                                a[0] += v * w0;
                                a[1] += v * w1;
                            }
                        }
                    }
                    let xr = &x.data[y * wd * 2..(y + 1) * wd * 2];
                    for (i, a) in acc.iter().enumerate() {
                        for c in 0..2 {
                            orow[i * 2 + c] = sigmoid(a[c] + bias[c]) * xr[i * 2 + c];
                        }
                    }
                }
            });
        }
    });
}

/// A 5×5 stride-2 'same' convolution from TF's kernel [5, 5, cin, cout].
fn conv_s2(kernel: &[f32], bias: &[f32], cin: usize, cout: usize) -> Gather {
    let taps = (0..25).map(|t| (t / 5 - 1, t % 5 - 1)).collect();
    let b = PackedB::new(25 * cin, cout, |r, c| kernel[r * cout + c]);
    Gather {
        taps,
        stride: 2,
        cin,
        cout,
        b,
        bias: bias.to_vec(),
    }
}

/// One output parity (py, px) of a 5×5 stride-2 'same' transposed
/// convolution, TF kernel [5, 5, cout, cin]: output (2i + py, 2j + px)
/// takes input (i + (py + 1 − kh) / 2, j + (px + 1 − kw) / 2) for the kh,
/// kw of its parity.
fn deconv_part(
    kernel: &[f32],
    bias: &[f32],
    cin: usize,
    cout: usize,
    py: usize,
    px: usize,
) -> Gather {
    let ks =
        |p: usize| -> Vec<usize> { (0..5).filter(|k| (p + 5 - k).is_multiple_of(2)).collect() };
    let mut taps = Vec::new();
    let mut which = Vec::new();
    for &kh in &ks(py) {
        for &kw in &ks(px) {
            taps.push((
                ((py + 1) as i32 - kh as i32) / 2,
                ((px + 1) as i32 - kw as i32) / 2,
            ));
            which.push((kh, kw));
        }
    }
    let b = PackedB::new(taps.len() * cin, cout, |r, co| {
        let (t, ci) = (r / cin, r % cin);
        let (kh, kw) = which[t];
        kernel[((kh * 5 + kw) * cout + co) * cin + ci]
    });
    Gather {
        taps,
        stride: 1,
        cin,
        cout,
        b,
        bias: bias.to_vec(),
    }
}

enum Deconv {
    /// The four output parities, each a gather.
    Parts(Vec<(usize, usize, Gather)>, usize),
    /// The last, to one channel.
    Scatter(Scatter),
}

/// One stem's network.
pub(crate) struct Unet {
    enc: Vec<Gather>,
    dec: Vec<Deconv>,
    /// The mask layer's kernel [4, 4, 1, 2] (each weight as its half) and
    /// bias.
    mask_w: Vec<f32>,
    mask_b: Vec<f32>,
    /// Batch norms (scale, shift): encoder 0-5, decoder 6-11.
    bn: Vec<(Vec<f32>, Vec<f32>)>,
}

impl Unet {
    /// From the stem's tensors in file order ([`Shape::stem_layout`]).
    pub fn build(shape: &Shape, mut next: impl FnMut() -> Vec<f32>) -> Unet {
        let mut enc = Vec::new();
        let mut cin = 2;
        for &f in &shape.filters {
            let (k, b) = (next(), next());
            enc.push(conv_s2(&k, &b, cin, f));
            cin = f;
        }
        let (mk, mask_b) = (next(), next());
        let mask_w = mk
            .iter()
            .map(|&v| mpfree_simd::plain::half(crate::half::to_f16(v)))
            .collect();
        let (outs, ins) = (shape.outs(), shape.ins());
        let mut dec = Vec::new();
        for j in 0..6 {
            let (k, b) = (next(), next());
            if j == 5 {
                dec.push(Deconv::Scatter(Scatter::new(&k, &b, ins[j])));
                continue;
            }
            let mut parts = Vec::new();
            for py in 0..2 {
                for px in 0..2 {
                    parts.push((py, px, deconv_part(&k, &b, ins[j], outs[j], py, px)));
                }
            }
            dec.push(Deconv::Parts(parts, outs[j]));
        }
        let bn = (0..12).map(|_| (next(), next())).collect();
        Unet {
            enc,
            dec,
            mask_w,
            mask_b,
            bn,
        }
    }

    /// Every gather: the encoder's and the decoder's parities.
    fn gathers(&self) -> impl Iterator<Item = &Gather> {
        let dec = self.dec.iter().flat_map(|d| match d {
            Deconv::Parts(parts, _) => parts.iter().map(|(_, _, g)| g).collect::<Vec<_>>(),
            Deconv::Scatter(_) => Vec::new(),
        });
        self.enc.iter().chain(dec)
    }

    fn scatter(&self) -> Option<&Scatter> {
        self.dec.iter().find_map(|d| match d {
            Deconv::Scatter(s) => Some(s),
            Deconv::Parts(..) => None,
        })
    }

    /// Every packed weight panel.
    fn panels(&self) -> impl Iterator<Item = &PackedB> {
        let scatter = self.scatter().map(|s| &s.b);
        self.gathers().map(|g| &g.b).chain(scatter)
    }

    /// Bytes the weights take.
    pub fn bytes(&self) -> usize {
        let bn: usize = self.bn.iter().map(|(s, b)| s.len() + b.len()).sum();
        let bias: usize = self.gathers().map(|g| g.bias.len()).sum();
        let floats = bn + bias + self.mask_w.len() + self.mask_b.len() + 1;
        self.panels().map(PackedB::bytes).sum::<usize>() + floats * 4
    }

    /// Whether `o` holds the same weights, bit for bit.
    #[cfg(test)]
    pub fn same_weights(&self, o: &Unet) -> bool {
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        let floats = |u: &Unet| {
            let mut all = Vec::new();
            u.gathers().for_each(|g| all.extend(bits(&g.bias)));
            u.bn.iter()
                .for_each(|(s, b)| all.extend(bits(s).into_iter().chain(bits(b))));
            all.extend(bits(&u.mask_w).into_iter().chain(bits(&u.mask_b)));
            all.extend(u.scatter().map(|s| s.bias.to_bits()));
            all
        };
        self.panels()
            .map(PackedB::halves)
            .eq(o.panels().map(PackedB::halves))
            && floats(self) == floats(o)
    }

    /// Floats the widest layer's weights take unpacked.
    pub fn widest(&self) -> usize {
        self.panels().map(PackedB::len).max().unwrap_or(0)
    }

    /// The most floats a thread of any layer holds for a segment of
    /// `bins` columns.
    pub fn thread_floats(&self, bins: usize) -> usize {
        let (mut most, mut w) = (0, bins);
        for g in &self.enc {
            w = w.div_ceil(2);
            most = most.max(gather_floats(g, w));
        }
        for d in &self.dec {
            match d {
                Deconv::Parts(parts, _) => {
                    for (_, _, g) in parts {
                        most = most.max(gather_floats(g, w));
                    }
                }
                Deconv::Scatter(s) => most = most.max(scatter_floats(s, w)),
            }
            w *= 2;
        }
        // The mask layer's padded row and two sums a column.
        most.max(3 * bins + 6)
    }

    /// The stem's estimate of the magnitude for one segment (rows × bins
    /// × 2): the mask times `x`.
    pub fn run(&self, x: &Tensor, threads: usize, work: &mut Work) -> Tensor {
        // The encoder's outputs before their batch norms: the skips.
        let mut raw: Vec<Tensor> = Vec::new();
        // The last layer's batch-normed ELU: the next one's input.
        let mut rel: Option<Tensor> = None;
        for (i, g) in self.enc.iter().enumerate() {
            let cur = rel.as_ref().unwrap_or(x);
            let (h, w) = (cur.h.div_ceil(2), cur.w.div_ceil(2));
            let mut out = Tensor::zeros(h, w, g.cout);
            let mut next = Tensor::zeros(h, w, g.cout);
            let (scale, shift) = (&self.bn[i].0, &self.bn[i].1);
            let post = Post::Encoder {
                next: &mut next,
                scale,
                shift,
            };
            run_gather(
                g,
                Input::one(cur),
                &mut out,
                &Place::whole(h, w),
                post,
                threads,
                work,
            );
            raw.push(out);
            rel = Some(next);
        }
        drop(rel);
        // Each layer's input: a skip and the last output, joined.
        let mut skip = raw.pop().expect("six encoder layers");
        let mut prev: Option<Tensor> = None;
        for (j, d) in self.dec.iter().enumerate() {
            let input = Input {
                a: &skip,
                b: prev.as_ref(),
            };
            let (ih, iw) = (skip.h, skip.w);
            let (scale, shift) = (&self.bn[6 + j].0, &self.bn[6 + j].1);
            let out = match d {
                Deconv::Parts(parts, cout) => {
                    let mut out = Tensor::zeros(ih * 2, iw * 2, *cout);
                    for (py, px, g) in parts {
                        let at = Place {
                            grid_h: ih,
                            grid_w: iw,
                            ys: 2,
                            yo: *py,
                            xs: 2,
                            xo: *px,
                        };
                        let post = Post::Decoder { scale, shift };
                        run_gather(g, input, &mut out, &at, post, threads, work);
                    }
                    out
                }
                Deconv::Scatter(s) => {
                    let mut out = Tensor::zeros(ih * 2, iw * 2, 1);
                    run_scatter(s, input, &mut out, scale[0], shift[0], threads, work);
                    out
                }
            };
            (skip, prev) = match raw.pop() {
                Some(r) => (r, Some(out)),
                None => (out, None),
            };
        }
        drop(prev);
        let mut mask = Tensor::zeros(skip.h, skip.w, 2);
        run_mask(&self.mask_w, &self.mask_b, &skip, x, &mut mask, threads);
        mask
    }
}

#[cfg(test)]
pub(crate) mod reference {
    //! The layers written as their definitions, slowly, in f64: what the
    //! fast ones must match.

    use super::Tensor;

    /// TF's `conv2d`, 'same' padding, NHWC, kernel [kh, kw, cin, cout],
    /// dilation `d`, stride `s`.
    pub fn conv(x: &Tensor, k: &[f32], kh: usize, cout: usize, s: usize, d: usize) -> Vec<f64> {
        let (oh, ow) = (x.h.div_ceil(s), x.w.div_ceil(s));
        let span = (kh - 1) * d + 1;
        let pad = |n: usize, o: usize| (((o - 1) * s + span).saturating_sub(n)) / 2;
        let (pt, pl) = (pad(x.h, oh), pad(x.w, ow));
        let mut out = vec![0f64; oh * ow * cout];
        for oy in 0..oh {
            for ox in 0..ow {
                for a in 0..kh {
                    for b in 0..kh {
                        let iy = (oy * s + a * d) as isize - pt as isize;
                        let ix = (ox * s + b * d) as isize - pl as isize;
                        if iy < 0 || ix < 0 || iy as usize >= x.h || ix as usize >= x.w {
                            continue;
                        }
                        let px = (iy as usize * x.w + ix as usize) * x.c;
                        for ci in 0..x.c {
                            for co in 0..cout {
                                let w = k[((a * kh + b) * x.c + ci) * cout + co];
                                out[(oy * ow + ox) * cout + co] +=
                                    x.data[px + ci] as f64 * w as f64;
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// TF's `conv2d_transpose`, 5×5, stride 2, 'same' (output twice the
    /// input), kernel [5, 5, cout, cin].
    pub fn deconv(x: &Tensor, k: &[f32], cout: usize) -> Vec<f64> {
        let (oh, ow) = (2 * x.h, 2 * x.w);
        let mut out = vec![0f64; oh * ow * cout];
        for iy in 0..x.h {
            for ix in 0..x.w {
                for a in 0..5 {
                    for b in 0..5 {
                        // Padding 1 before: output 2·i + a − 1.
                        let (oy, ox) = (2 * iy + a, 2 * ix + b);
                        if oy < 1 || ox < 1 || oy > oh || ox > ow {
                            continue;
                        }
                        let (oy, ox) = (oy - 1, ox - 1);
                        for co in 0..cout {
                            for ci in 0..x.c {
                                let w = k[((a * 5 + b) * cout + co) * x.c + ci];
                                out[(oy * ow + ox) * cout + co] +=
                                    x.data[(iy * x.w + ix) * x.c + ci] as f64 * w as f64;
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn elu64(x: f64) -> f64 {
        if x > 0.0 {
            x
        } else {
            x.exp() - 1.0
        }
    }

    pub fn tensor(h: usize, w: usize, c: usize, v: Vec<f64>) -> Tensor {
        Tensor {
            h,
            w,
            c,
            data: v.into_iter().map(|x| x as f32).collect(),
        }
    }

    pub fn concat(a: &Tensor, b: &Tensor) -> Tensor {
        let mut data = Vec::with_capacity(a.data.len() + b.data.len());
        for p in 0..a.h * a.w {
            data.extend_from_slice(&a.data[p * a.c..(p + 1) * a.c]);
            data.extend_from_slice(&b.data[p * b.c..(p + 1) * b.c]);
        }
        Tensor {
            h: a.h,
            w: a.w,
            c: a.c + b.c,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::reference::*;
    use super::*;

    struct Rng(u32);
    impl Rng {
        fn unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        }
        fn vec(&mut self, n: usize, scale: f32) -> Vec<f32> {
            (0..n).map(|_| self.unit() * scale).collect()
        }
        fn tensor(&mut self, h: usize, w: usize, c: usize) -> Tensor {
            Tensor {
                h,
                w,
                c,
                data: self.vec(h * w * c, 1.0),
            }
        }
    }

    /// Weights as the network holds them: each its half.
    fn halved(k: &[f32]) -> Vec<f32> {
        k.iter()
            .map(|&v| mpfree_simd::plain::half(crate::half::to_f16(v)))
            .collect()
    }

    fn close(got: &[f32], want: &[f64], what: &str) {
        assert_eq!(got.len(), want.len(), "{what}");
        let peak = want.iter().fold(0f64, |a, &b| a.max(b.abs())).max(1e-3);
        for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
            assert!(
                (g as f64 - w).abs() < 2e-5 * peak,
                "{what} [{i}]: {g} vs {w}"
            );
        }
    }

    #[test]
    fn the_strided_convolution_is_tfs_same_convolution() {
        let mut r = Rng(1);
        for (h, w, cin, cout, threads) in [(8, 12, 2, 16, 1), (6, 10, 3, 5, 2), (4, 4, 40, 9, 3)] {
            let x = r.tensor(h, w, cin);
            let k = r.vec(25 * cin * cout, 0.3);
            let bias = r.vec(cout, 0.1);
            let (scale, shift) = (r.vec(cout, 1.0), r.vec(cout, 0.2));
            let g = conv_s2(&k, &bias, cin, cout);
            let (oh, ow) = (h.div_ceil(2), w.div_ceil(2));
            let (mut out, mut next) = (Tensor::zeros(oh, ow, cout), Tensor::zeros(oh, ow, cout));
            let mut work = Work::new(g.b.len());
            let post = Post::Encoder {
                next: &mut next,
                scale: &scale,
                shift: &shift,
            };
            run_gather(
                &g,
                Input::one(&x),
                &mut out,
                &Place::whole(oh, ow),
                post,
                threads,
                &mut work,
            );
            let mut want = conv(&x, &halved(&k), 5, cout, 2, 1);
            for (i, v) in want.iter_mut().enumerate() {
                *v += bias[i % cout] as f64;
            }
            close(&out.data, &want, "sums");
            let bn: Vec<f64> = want
                .iter()
                .enumerate()
                .map(|(i, &v)| elu64(v * scale[i % cout] as f64 + shift[i % cout] as f64))
                .collect();
            close(&next.data, &bn, "batch-normed ELU");
        }
    }

    #[test]
    fn the_four_parities_make_tfs_transposed_convolution() {
        let mut r = Rng(2);
        for (h, w, ca, cb, cout, threads) in
            [(2, 3, 4, 0, 6, 1), (4, 2, 3, 5, 9, 2), (3, 3, 8, 8, 17, 4)]
        {
            let a = r.tensor(h, w, ca);
            let b = r.tensor(h, w, cb);
            let cin = ca + cb;
            let k = r.vec(25 * cout * cin, 0.3);
            let bias = r.vec(cout, 0.1);
            let (scale, shift) = (r.vec(cout, 1.0), r.vec(cout, 0.2));
            let input = Input {
                a: &a,
                b: (cb > 0).then_some(&b),
            };
            let mut out = Tensor::zeros(2 * h, 2 * w, cout);
            let mut work = Work::new(25 * cin * 2 * cout);
            for py in 0..2 {
                for px in 0..2 {
                    let g = deconv_part(&k, &bias, cin, cout, py, px);
                    let at = Place {
                        grid_h: h,
                        grid_w: w,
                        ys: 2,
                        yo: py,
                        xs: 2,
                        xo: px,
                    };
                    let post = Post::Decoder {
                        scale: &scale,
                        shift: &shift,
                    };
                    run_gather(&g, input, &mut out, &at, post, threads, &mut work);
                }
            }
            let joined = if cb > 0 { concat(&a, &b) } else { a.clone() };
            let want: Vec<f64> = deconv(&joined, &halved(&k), cout)
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let co = i % cout;
                    elu64(v + bias[co] as f64) * scale[co] as f64 + shift[co] as f64
                })
                .collect();
            close(&out.data, &want, "transposed");
        }
    }

    #[test]
    fn the_scatter_makes_tfs_transposed_convolution_to_one_channel() {
        let mut r = Rng(3);
        for (h, w, ca, cb, threads) in [
            (4, 6, 3, 3, 1),
            (5, 3, 2, 0, 2),
            (8, 8, 16, 16, 3),
            (3, 2, 4, 4, 7),
        ] {
            let a = r.tensor(h, w, ca);
            let b = r.tensor(h, w, cb);
            let cin = ca + cb;
            let k = r.vec(25 * cin, 0.3);
            let bias = r.vec(1, 0.1);
            let s = Scatter::new(&k, &bias, cin);
            let input = Input {
                a: &a,
                b: (cb > 0).then_some(&b),
            };
            let mut out = Tensor::zeros(2 * h, 2 * w, 1);
            let mut work = Work::new(s.b.len());
            run_scatter(&s, input, &mut out, 0.9, -0.1, threads, &mut work);
            let joined = if cb > 0 { concat(&a, &b) } else { a.clone() };
            let want: Vec<f64> = deconv(&joined, &halved(&k), 1)
                .iter()
                .map(|&v| elu64(v + bias[0] as f64) * 0.9 - 0.1)
                .collect();
            close(
                &out.data,
                &want,
                &format!("scatter {h}×{w} threads {threads}"),
            );
        }
    }

    #[test]
    fn the_mask_layer_is_tfs_dilated_convolution_and_a_sigmoid() {
        let mut r = Rng(4);
        for (h, w, threads) in [(8, 8, 1), (5, 7, 2), (16, 4, 3)] {
            let input = r.tensor(h, w, 1);
            let x = r.tensor(h, w, 2);
            let k = halved(&r.vec(32, 0.5));
            let bias = r.vec(2, 0.1);
            let mut out = Tensor::zeros(h, w, 2);
            run_mask(&k, &bias, &input, &x, &mut out, threads);
            let want: Vec<f64> = conv(&input, &k, 4, 2, 1, 2)
                .iter()
                .enumerate()
                .map(|(i, &v)| 1.0 / (1.0 + (-(v + bias[i % 2] as f64)).exp()) * x.data[i] as f64)
                .collect();
            close(&out.data, &want, "mask");
        }
    }

    /// A whole small network against its layers written out.
    #[test]
    fn a_small_unet_is_its_layers_in_spleeters_order() {
        let shape = Shape {
            filters: [3, 4, 5, 6, 7, 8],
            rows: 64,
        };
        let mut r = Rng(5);
        let tensors: Vec<Vec<f32>> = shape
            .stem_layout()
            .iter()
            .map(|&(n, kind)| match kind {
                Kind::Kernel(fan) => r.vec(n, (3.0 / fan as f32).sqrt()),
                Kind::Bias | Kind::Shift => r.vec(n, 0.05),
                Kind::Scale => r.vec(n, 0.1).iter().map(|v| 1.0 + v).collect(),
            })
            .collect();
        let mut it = tensors.iter().cloned();
        let net = Unet::build(&shape, || it.next().unwrap());
        let x = Tensor {
            data: r.vec(64 * 128 * 2, 1.0).iter().map(|v| v.abs()).collect(),
            ..Tensor::zeros(64, 128, 2)
        };
        let mut work = Work::new(net.widest());
        let got = net.run(&x, 2, &mut work);

        // The same network, layer by layer, in f64 between layers' f32s.
        let t = |i: usize| &tensors[i];
        let bn = |j: usize| (t(26 + 2 * j), t(27 + 2 * j));
        let mut skips = Vec::new();
        let mut cur = x.clone();
        for (i, &f) in shape.filters.iter().enumerate() {
            let mut z = conv(&cur, &halved(t(2 * i)), 5, f, 2, 1);
            z.iter_mut()
                .enumerate()
                .for_each(|(p, v)| *v += t(2 * i + 1)[p % f] as f64);
            let (h, w) = (cur.h.div_ceil(2), cur.w.div_ceil(2));
            skips.push(tensor(h, w, f, z.clone()));
            let (s, b) = bn(i);
            let y = z
                .iter()
                .enumerate()
                .map(|(p, &v)| elu64(v * s[p % f] as f64 + b[p % f] as f64));
            cur = tensor(h, w, f, y.collect());
        }
        let mut up: Option<Tensor> = None;
        for (j, &c) in shape.outs().iter().enumerate() {
            let skip = skips.pop().unwrap();
            let input = match &up {
                Some(u) => concat(&skip, u),
                None => skip,
            };
            let z = deconv(&input, &halved(t(14 + 2 * j)), c);
            let (s, b) = bn(6 + j);
            let bias = t(15 + 2 * j);
            let y = z
                .iter()
                .enumerate()
                .map(|(p, &v)| elu64(v + bias[p % c] as f64) * s[p % c] as f64 + b[p % c] as f64);
            up = Some(tensor(input.h * 2, input.w * 2, c, y.collect()));
        }
        let last = up.unwrap();
        let want: Vec<f64> = conv(&last, &halved(t(12)), 4, 2, 1, 2)
            .iter()
            .enumerate()
            .map(|(p, &v)| 1.0 / (1.0 + (-(v + t(13)[p % 2] as f64)).exp()) * x.data[p] as f64)
            .collect();
        close(&got.data, &want, "U-Net");
    }

    #[test]
    fn spleeters_layout_has_50_tensors_and_9_8_million_weights_a_stem() {
        let l = Shape::SPLEETER.stem_layout();
        assert_eq!(l.len(), 50);
        let kernels: usize = l
            .iter()
            .filter(|(_, k)| matches!(k, Kind::Kernel(_)))
            .map(|(n, _)| n)
            .sum();
        assert_eq!(kernels, 9_819_232);
        assert_eq!(Shape::SPLEETER.ins(), [512, 512, 256, 128, 64, 32]);
        assert_eq!(Shape::SPLEETER.outs(), [256, 128, 64, 32, 16, 1]);
    }
}
