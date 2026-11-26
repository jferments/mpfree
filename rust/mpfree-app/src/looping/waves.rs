//! The loops' waveforms, built a slice at a time: the Looper tab's, the
//! cells' and the ring's, and the first pass's as it records.

use super::*;

/// The loop's waveform for the Looper tab: min, max and RMS of the playing
/// layers, summed, in [`WAVE_COLUMNS`] columns per channel; and for the
/// Play page's strip, each layer's peak (either side) per column.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoopWave {
    pub frames: u64,
    /// While the first pass records: how many of its frames the columns
    /// hold so far, from the start (`frames` is then the length it is
    /// drawn against). None once the loop is whole.
    pub recorded: Option<u64>,
    pub columns: Vec<[WaveColumn; 2]>,
    /// `layers[k][col]`: layer k's peak in that column, 0 to 1.
    pub layers: Vec<Vec<f32>>,
}

impl LoopWave {
    /// While the first pass records: how far it has got along the wave,
    /// 0 to 1, for the playhead.
    pub fn live(&self) -> Option<f32> {
        let r = self.recorded?;
        (self.frames > 0).then(|| (r as f32 / self.frames as f32).min(1.0))
    }
}

impl Peaks for LoopWave {
    fn frames(&self) -> u64 {
        self.frames
    }

    fn channels(&self) -> usize {
        2
    }

    fn column(&self, ch: usize, f0: f64, f1: f64) -> WaveColumn {
        let n = self.columns.len();
        if n == 0 || ch > 1 || f1 <= 0.0 || f0 >= self.frames as f64 {
            return WaveColumn::default();
        }
        let per = self.frames as f64 / n as f64;
        let a = ((f0.max(0.0) / per).floor() as usize).min(n - 1);
        let b = ((f1 / per).ceil() as usize).clamp(a + 1, n);
        let mut c = WaveColumn {
            min: f32::MAX,
            max: f32::MIN,
            rms: 0.0,
        };
        let mut sq = 0.0;
        for col in &self.columns[a..b] {
            c.min = c.min.min(col[ch].min);
            c.max = c.max.max(col[ch].max);
            sq += col[ch].rms * col[ch].rms;
        }
        c.rms = (sq / (b - a) as f32).sqrt();
        c
    }
}

/// One column of one channel as it fills: min, max, sum of squares and
/// frames.
type Acc = (f32, f32, f32, u32);

pub(crate) const NO_ACC: Acc = (f32::MAX, f32::MIN, 0.0, 0);

pub(crate) fn acc_merge(a: Acc, b: Acc) -> Acc {
    (a.0.min(b.0), a.1.max(b.1), a.2 + b.2, a.3 + b.3)
}

pub(crate) fn column(a: Acc) -> WaveColumn {
    if a.3 == 0 {
        WaveColumn::default()
    } else {
        WaveColumn {
            min: a.0,
            max: a.1,
            rms: (a.2 / a.3 as f32).sqrt(),
        }
    }
}

/// The columns frames `from..from + n` of a wave of `len` frames fall in
/// (column `c` starts at frame `ceil(c * len / WAVE_COLUMNS)`), each with
/// its frames, counted from `from`.
pub(crate) fn runs(
    from: usize,
    n: usize,
    len: usize,
) -> impl Iterator<Item = (usize, Range<usize>)> {
    runs_in(from, n, len, WAVE_COLUMNS)
}

/// [`runs`] for a wave of `cols` columns.
fn runs_in(
    from: usize,
    n: usize,
    len: usize,
    cols: usize,
) -> impl Iterator<Item = (usize, Range<usize>)> {
    let (start, end, len) = (from as u64, (from + n) as u64, len.max(1) as u64);
    let cols = cols as u64;
    let mut f = start;
    std::iter::from_fn(move || {
        if f >= end {
            return None;
        }
        let col = (f * cols / len).min(cols - 1);
        let stop = if col + 1 < cols {
            ((col + 1) * len).div_ceil(cols).min(end)
        } else {
            end
        };
        let r = (f - start) as usize..(stop - start) as usize;
        f = stop;
        Some((col as usize, r))
    })
}

// The two below keep four lanes (two frames' left and right) apart, so
// that each step does not wait for the one before: about four times
// quicker than one running min, max and sum.

/// The peak (either side) of `x`, from `p`.
fn peak_of(p: f32, x: &[[f32; 2]]) -> f32 {
    let mut m = [0.0f32; 4];
    let pairs = x.chunks_exact(2);
    for v in pairs.remainder() {
        for (m, v) in m.iter_mut().zip(v) {
            *m = if v.abs() > *m { v.abs() } else { *m };
        }
    }
    for p in pairs {
        let v = [p[0][0], p[0][1], p[1][0], p[1][1]];
        for (m, v) in m.iter_mut().zip(v) {
            *m = if v.abs() > *m { v.abs() } else { *m };
        }
    }
    m.iter().fold(p, |a, &b| a.max(b))
}

/// `a` (left, right) with the frames `x` added.
fn acc_run(a: [Acc; 2], x: &[[f32; 2]]) -> [Acc; 2] {
    let mut lo = [f32::MAX; 4];
    let mut hi = [f32::MIN; 4];
    let mut sq = [0.0f32; 4];
    let pairs = x.chunks_exact(2);
    for v in pairs.remainder() {
        for i in 0..2 {
            lo[i] = if v[i] < lo[i] { v[i] } else { lo[i] };
            hi[i] = if v[i] > hi[i] { v[i] } else { hi[i] };
            sq[i] += v[i] * v[i];
        }
    }
    for p in pairs {
        let v = [p[0][0], p[0][1], p[1][0], p[1][1]];
        for i in 0..4 {
            lo[i] = if v[i] < lo[i] { v[i] } else { lo[i] };
            hi[i] = if v[i] > hi[i] { v[i] } else { hi[i] };
            sq[i] += v[i] * v[i];
        }
    }
    let ch = |c: usize| {
        let a = a[c];
        (
            a.0.min(lo[c]).min(lo[c + 2]),
            a.1.max(hi[c]).max(hi[c + 2]),
            a.2 + sq[c] + sq[c + 2],
            a.3 + x.len() as u32,
        )
    };
    [ch(0), ch(1)]
}

/// The waves sent to the screen, each made again in place once the screen
/// has let go of it, so that the audio thread never allocates one.
pub(crate) struct WavePool {
    waves: Vec<Arc<LoopWave>>,
    /// Layer rows no wave is using.
    spare: Vec<Vec<f32>>,
    /// Times the screen still held every wave, so the next one waited a
    /// period (should stay 0: the screen keeps one, and one or two may be
    /// on their way).
    pub(crate) waits: u64,
}

impl WavePool {
    pub(crate) fn new() -> WavePool {
        let wave = || {
            Arc::new(LoopWave {
                frames: 0,
                recorded: None,
                columns: vec![[WaveColumn::default(); 2]; WAVE_COLUMNS],
                layers: Vec::with_capacity(MAX_LAYERS),
            })
        };
        WavePool {
            waves: (0..WAVES).map(|_| wave()).collect(),
            spare: (0..WAVES * MAX_LAYERS)
                .map(|_| vec![0.0; WAVE_COLUMNS])
                .collect(),
            waits: 0,
        }
    }

    /// A wave with `layers` layer rows, as `fill` makes it; None (counted)
    /// while the screen holds every wave.
    fn make(&mut self, layers: usize, fill: impl FnOnce(&mut LoopWave)) -> Option<Arc<LoopWave>> {
        let Some(i) = self
            .waves
            .iter_mut()
            .position(|w| Arc::get_mut(w).is_some())
        else {
            self.waits += 1;
            return None;
        };
        let w = Arc::get_mut(&mut self.waves[i]).expect("only the pool holds it");
        let layers = layers.min(MAX_LAYERS);
        while w.layers.len() > layers {
            self.spare.extend(w.layers.pop());
        }
        while w.layers.len() < layers {
            match self.spare.pop() {
                Some(row) => w.layers.push(row),
                None => break,
            }
        }
        fill(w);
        Some(Arc::clone(&self.waves[i]))
    }
}

/// Builds the loop's [`LoopWave`] a slice at a time: [`WAVE_BUDGET`]
/// layer-frames a period, each layer read a block at a time.
pub(crate) struct WaveBuilder {
    /// What the wave is of: the loop's length, layers and phase, and which
    /// loop.
    of: (usize, usize, Phase, usize),
    at: usize,
    frames: usize,
    layers: usize,
    acc: Vec<[Acc; 2]>,
    /// Each layer's peak per column.
    peaks: Vec<Vec<f32>>,
    /// One slice of one layer, and of the layers summed.
    buf: Vec<[f32; 2]>,
    sum: Vec<[f32; 2]>,
    building: bool,
}

impl WaveBuilder {
    pub(crate) fn new() -> WaveBuilder {
        WaveBuilder {
            of: (0, 0, Phase::Empty, 0),
            at: 0,
            frames: 0,
            layers: 0,
            acc: vec![[NO_ACC; 2]; WAVE_COLUMNS],
            peaks: vec![vec![0.0; WAVE_COLUMNS]; MAX_LAYERS],
            buf: vec![[0.0; 2]; WAVE_SLICE],
            sum: vec![[0.0; 2]; WAVE_SLICE],
            building: false,
        }
    }

    fn start(&mut self, frames: usize, layers: usize, of: (usize, usize, Phase, usize)) {
        self.of = of;
        self.frames = frames;
        self.layers = layers.min(MAX_LAYERS);
        self.at = 0;
        self.building = frames > 0;
        self.acc.fill([NO_ACC; 2]);
        for l in self.peaks.iter_mut() {
            l.fill(0.0);
        }
    }

    /// One slice: true once every slice is done and the wave is ready
    /// ([`WaveBuilder::finish`]).
    fn step(&mut self, looper: &Looper) -> bool {
        if !self.building {
            return false;
        }
        let len = self.frames;
        if self.at < len {
            let from = self.at;
            let n = (WAVE_BUDGET / self.layers.max(1))
                .clamp(WAVE_MIN, WAVE_SLICE)
                .min(len - from);
            let sum = &mut self.sum[..n];
            sum.fill([0.0; 2]);
            for k in 0..self.layers {
                let Some(l) = looper.layer(k) else { continue };
                if l.len() < 2 * (from + n) {
                    continue;
                }
                let buf = &mut self.buf[..n];
                l.read(from, buf);
                let peaks = &mut self.peaks[k];
                for (c, r) in runs(from, n, len) {
                    peaks[c] = peak_of(peaks[c], &buf[r]);
                }
                for (s, x) in sum.iter_mut().zip(buf.iter()) {
                    s[0] += x[0];
                    s[1] += x[1];
                }
            }
            for (c, r) in runs(from, n, len) {
                self.acc[c] = acc_run(self.acc[c], &sum[r]);
            }
            self.at += n;
        }
        self.at >= len
    }

    /// The finished wave, in one of `pool`'s; None while the screen holds
    /// them all (it is ready again next period).
    fn finish(&mut self, pool: &mut WavePool) -> Option<Arc<LoopWave>> {
        let w = pool.make(self.layers, |w| {
            w.frames = self.frames as u64;
            w.recorded = None;
            for (to, a) in w.columns.iter_mut().zip(&self.acc) {
                *to = [column(a[0]), column(a[1])];
            }
            for (to, p) in w.layers.iter_mut().zip(&self.peaks) {
                for (t, v) in to.iter_mut().zip(p) {
                    *t = v.min(1.0);
                }
            }
        })?;
        self.building = false;
        Some(w)
    }
}

/// A loop's small wave for its cells on the Clips grid: each column's
/// peak over every layer, 0 to 255.
pub type CellWave = [u8; CELL_WAVE];

/// Columns in a loop's wave round its ring on the Looper tab's deck: three
/// to each of a cell's, so a cell's column is the loudest of its three.
pub const RING_WAVE: usize = 3 * CELL_WAVE;

/// A loop's wave round its ring on the deck: each column's peak over
/// every layer, 0 to 255.
pub type RingWave = [u8; RING_WAVE];

/// A cell's wave from a ring's: the loudest of each three columns.
pub fn cell_of_ring(r: &RingWave) -> CellWave {
    let mut c = [0u8; CELL_WAVE];
    for (i, v) in c.iter_mut().enumerate() {
        *v = r[3 * i].max(r[3 * i + 1]).max(r[3 * i + 2]);
    }
    c
}

/// Makes each loop's [`RingWave`] in turn (and from it its [`CellWave`]),
/// a slice a period ([`WAVE_BUDGET`] layer-frames, as the loop's own
/// wave), whenever a loop's frames, layers or sound (`Status::version`)
/// changed. A loop recording or overdubbing keeps the one it had until it
/// plays.
pub(crate) struct CellWaves {
    /// Each loop's waves, and what they are of: frames, layers and version.
    pub(crate) done: [CellWave; MAX_LOOPS],
    pub(crate) rings: [RingWave; MAX_LOOPS],
    of: [(usize, usize, u64); MAX_LOOPS],
    /// The loop being made and what of, how far it is, and its peaks.
    making: Option<(usize, (usize, usize, u64))>,
    at: usize,
    peaks: [f32; RING_WAVE],
    buf: Vec<[f32; 2]>,
}

impl CellWaves {
    pub(crate) fn new() -> CellWaves {
        CellWaves {
            done: [[0; CELL_WAVE]; MAX_LOOPS],
            rings: [[0; RING_WAVE]; MAX_LOOPS],
            of: [(0, 0, 0); MAX_LOOPS],
            making: None,
            at: 0,
            peaks: [0.0; RING_WAVE],
            buf: vec![[0.0; 2]; WAVE_SLICE],
        }
    }

    /// One slice of the wave being made, or every [`CELL_WAVE_EVERY`]
    /// periods a look for a loop whose wave is out of date.
    fn step(&mut self, looper: &Looper, period: u64) {
        if self.making.is_none() {
            if !period.is_multiple_of(CELL_WAVE_EVERY) {
                return;
            }
            for n in 0..MAX_LOOPS {
                let Some(s) = looper.loop_status(n) else {
                    continue;
                };
                let of = (s.frames, s.layers, s.version);
                let busy = matches!(
                    s.phase,
                    Phase::Armed | Phase::Recording | Phase::Overdubbing
                );
                if busy || of == self.of[n] {
                    continue;
                }
                if s.frames == 0 || s.phase == Phase::Empty {
                    self.done[n] = [0; CELL_WAVE];
                    self.rings[n] = [0; RING_WAVE];
                    self.of[n] = of;
                    continue;
                }
                self.making = Some((n, of));
                self.at = 0;
                self.peaks = [0.0; RING_WAVE];
                break;
            }
        }
        let Some((n, of)) = self.making else {
            return;
        };
        // The loop changed under it: it starts again on the next look.
        let now = looper
            .loop_status(n)
            .map(|s| (s.frames, s.layers, s.version));
        if now != Some(of) {
            self.making = None;
            return;
        }
        let (len, layers, _) = of;
        let from = self.at;
        let k = (WAVE_BUDGET / layers.max(1))
            .clamp(WAVE_MIN, WAVE_SLICE)
            .min(len - from);
        for layer in 0..layers.min(MAX_LAYERS) {
            let Some(l) = looper.loop_layer(n, layer) else {
                continue;
            };
            if l.len() < 2 * (from + k) {
                continue;
            }
            let buf = &mut self.buf[..k];
            l.read(from, buf);
            for (c, r) in runs_in(from, k, len, RING_WAVE) {
                self.peaks[c] = peak_of(self.peaks[c], &buf[r]);
            }
        }
        self.at += k;
        if self.at >= len {
            for (d, p) in self.rings[n].iter_mut().zip(&self.peaks) {
                *d = (p.min(1.0) * 255.0).round() as u8;
            }
            self.done[n] = cell_of_ring(&self.rings[n]);
            self.of[n] = of;
            self.making = None;
        }
    }
}

/// The first pass's waveform as it records, the new frames each period.
pub(crate) struct LiveWave {
    /// The loop whose first pass it is (None: not making one).
    of: Option<usize>,
    /// The frames the columns span: the pass's length when its bars are
    /// set, or two bars; twice that each time the pass gets to the end.
    scale: usize,
    /// Frames of the pass in the columns, and as many as when last sent.
    done: usize,
    sent: usize,
    acc: Vec<[Acc; 2]>,
    peaks: Vec<f32>,
    buf: Vec<[f32; 2]>,
}

impl LiveWave {
    pub(crate) fn new() -> LiveWave {
        LiveWave {
            of: None,
            scale: 0,
            done: 0,
            sent: 0,
            acc: vec![[NO_ACC; 2]; WAVE_COLUMNS],
            peaks: vec![0.0; WAVE_COLUMNS],
            buf: vec![[0.0; 2]; WAVE_SLICE],
        }
    }

    fn start(&mut self, n: usize, scale: usize) {
        self.of = Some(n);
        self.scale = scale.max(WAVE_COLUMNS);
        self.done = 0;
        self.sent = 0;
        self.acc.fill([NO_ACC; 2]);
        self.peaks.fill(0.0);
    }

    /// Twice the frames in the same columns: each pair becomes one.
    fn widen(&mut self) {
        let half = WAVE_COLUMNS / 2;
        for c in 0..half {
            let (a, b) = (self.acc[2 * c], self.acc[2 * c + 1]);
            self.acc[c] = [acc_merge(a[0], b[0]), acc_merge(a[1], b[1])];
            self.peaks[c] = self.peaks[2 * c].max(self.peaks[2 * c + 1]);
        }
        self.acc[half..].fill([NO_ACC; 2]);
        self.peaks[half..].fill(0.0);
        self.scale *= 2;
    }

    /// The pass's frames since the last step (a slice at most; the rest
    /// next period), `recorded` frames of it being written.
    fn step(&mut self, pass: &mpfree_looper::LayerView, recorded: usize) {
        let n = recorded.saturating_sub(self.done).min(WAVE_SLICE);
        if n == 0 || pass.len() < 2 * (self.done + n) {
            return;
        }
        while self.done + n > self.scale {
            self.widen();
        }
        let buf = &mut self.buf[..n];
        pass.read(self.done, buf);
        for (c, r) in runs(self.done, n, self.scale) {
            let x = &buf[r];
            self.peaks[c] = peak_of(self.peaks[c], x);
            self.acc[c] = acc_run(self.acc[c], x);
        }
        self.done += n;
    }

    /// The wave so far, in one of `pool`'s.
    fn make(&mut self, pool: &mut WavePool) -> Option<Arc<LoopWave>> {
        let w = pool.make(1, |w| {
            w.frames = self.scale as u64;
            w.recorded = Some(self.done as u64);
            for (to, a) in w.columns.iter_mut().zip(&self.acc) {
                *to = [column(a[0]), column(a[1])];
            }
            if let Some(row) = w.layers.first_mut() {
                for (t, v) in row.iter_mut().zip(&self.peaks) {
                    *t = v.min(1.0);
                }
            }
        })?;
        self.sent = self.done;
        Some(w)
    }
}

impl LoopHost {
    /// One slice of the loop's waveform; the finished wave when it is
    /// done. The wave starts again when the loop, its layers or the phase
    /// change, and every `refresh` periods while overdubbing. While the
    /// selected loop's first pass records, its wave so far instead, every
    /// [`LIVE_EVERY`] periods. Costs at most [`WAVE_BUDGET`] layer-frames
    /// a period, and never allocates: the waves are made again in place
    /// once the screen lets go of them. While that wave is done, each
    /// loop's small wave for its cells on the Clips grid instead
    /// ([`LooperView::cell_waves`]), on the same budget.
    pub fn wave_step(&mut self, period: u64, refresh: u64) -> Option<Arc<LoopWave>> {
        let w = self.loop_wave_step(period, refresh);
        if w.is_none() && !self.wave.building && self.live.of.is_none() {
            self.cells.step(&self.looper, period);
        }
        w
    }

    fn loop_wave_step(&mut self, period: u64, refresh: u64) -> Option<Arc<LoopWave>> {
        let sel = self.looper.selected();
        let s = self.looper.status();
        if s.phase == Phase::Recording {
            return self.live_step(sel, &s, period);
        }
        self.live.of = None;
        let of = (s.frames, s.layers, s.phase, sel);
        let changed = of != self.wave.of;
        if s.frames == 0 {
            if self.wave_shown {
                // Cleared, thrown away or another loop: the empty wave
                // (next period again if the screen holds every wave).
                let w = self.waves.make(0, |w| {
                    w.frames = 0;
                    w.recorded = None;
                })?;
                self.wave_shown = false;
                self.wave.start(0, 0, of);
                return Some(w);
            }
            if changed {
                self.wave.start(0, 0, of);
            }
            return None;
        }
        let overdubbing =
            s.phase == Phase::Overdubbing && refresh > 0 && period.is_multiple_of(refresh);
        if changed || overdubbing && !self.wave.building {
            self.wave.start(s.frames, s.layers, of);
        }
        if !self.wave.step(&self.looper) {
            return None;
        }
        let w = self.wave.finish(&mut self.waves)?;
        self.wave_shown = true;
        Some(w)
    }

    /// The selected loop's first pass, recording: its new frames into the
    /// live wave, and the wave so far when due.
    fn live_step(&mut self, sel: usize, s: &Status, period: u64) -> Option<Arc<LoopWave>> {
        if self.live.of != Some(sel) {
            let bar = frames_at(self.bar_ticks.max(1), self.bpm.max(1.0), self.rate);
            self.live
                .start(sel, bar * s.bars.map_or(2, |b| b.max(1) as usize));
        }
        if let Some(pass) = self.looper.first_pass(sel) {
            self.live.step(&pass, s.recorded);
        }
        if !period.is_multiple_of(LIVE_EVERY) || self.live.done == self.live.sent {
            return None;
        }
        let w = self.live.make(&mut self.waves)?;
        self.wave_shown = true;
        Some(w)
    }
}
