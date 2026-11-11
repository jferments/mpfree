//! Band-limited wavetables: a stack of single-cycle frames, each kept at
//! half-octave steps of bandwidth (a mip level per step), so a note only
//! reads harmonics that fit below Nyquist. Built off the audio thread from
//! frames of any length; read on the audio thread with no allocation.
//!
//! Memory: a frame keeps every level, about 49 KB (2048 samples at the top
//! four levels, shrinking with the bandwidth below). A 64-frame table is
//! about 3.2 MB, a 256-frame import about 12.6 MB. [`Table::bytes`] says
//! exactly.

use super::fft::fft;

/// The length frames are brought to before analysis (Serum's frame size).
pub const FRAME: usize = 2048;
/// The most frames a table holds.
pub const MAX_FRAMES: usize = 256;
/// The most harmonics any level holds (the top level, for notes below about
/// 43 Hz at 44.1 kHz).
pub const MAX_HARMONICS: usize = 512;
/// Harmonics kept per level, half an octave apart.
pub const LEVEL_HARMONICS: [usize; 17] = [
    512, 362, 256, 181, 128, 90, 64, 45, 32, 22, 16, 11, 8, 5, 4, 2, 1,
];
/// Points per frame in [`Table::preview`].
pub const PREVIEW: usize = 128;

/// One bandwidth of every frame. Each frame is `len` samples plus a guard
/// sample (a copy of the first) so interpolation never wraps.
#[derive(Clone, Debug)]
pub struct Level {
    pub harmonics: usize,
    pub len: usize,
    data: Box<[f32]>,
}

impl Level {
    /// Every frame, each `len + 1` samples ([`Level::stride`]).
    #[inline]
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// Samples from one frame's start to the next's.
    #[inline]
    pub fn stride(&self) -> usize {
        self.len + 1
    }

    /// Frame `f`'s samples, `len + 1` long (the last repeats the first).
    #[inline]
    pub fn frame(&self, f: usize) -> &[f32] {
        let s = f * (self.len + 1);
        &self.data[s..s + self.len + 1]
    }
}

/// A frame's harmonics 1 to [`MAX_HARMONICS`] as complex amplitudes
/// (index 0 is DC and unused).
#[derive(Clone, Debug)]
pub struct Spectrum {
    pub re: Vec<f64>,
    pub im: Vec<f64>,
}

impl Spectrum {
    pub fn zero() -> Spectrum {
        Spectrum {
            re: vec![0.0; MAX_HARMONICS + 1],
            im: vec![0.0; MAX_HARMONICS + 1],
        }
    }

    /// Sine harmonics with these amplitudes (`amps[0]` is harmonic 1): an
    /// additive frame. Sine phase, so a frame starts at zero.
    pub fn from_sines(amps: &[f64]) -> Spectrum {
        let mut s = Spectrum::zero();
        for (k, &a) in amps.iter().enumerate().take(MAX_HARMONICS) {
            // sin = -i/2 (e^{+i} - e^{-i}): bin k of a forward FFT holds -i*a/2.
            s.im[k + 1] = -a / 2.0;
        }
        s
    }

    /// The spectrum of one cycle of any length. A power-of-two length of
    /// [`FRAME`] or more is analysed as it is (generators draw at 16384 so
    /// their steps fold back far below the kept harmonics); any other length
    /// is resampled to [`FRAME`] first.
    pub fn of_cycle(cycle: &[f32]) -> Spectrum {
        let x = if cycle.len() >= FRAME && cycle.len().is_power_of_two() {
            cycle.to_vec()
        } else {
            resample_cycle(cycle, FRAME)
        };
        let n = x.len();
        let mut re: Vec<f64> = x.iter().map(|&v| f64::from(v)).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        let mut s = Spectrum::zero();
        for k in 1..=MAX_HARMONICS {
            s.re[k] = re[k] / n as f64;
            s.im[k] = im[k] / n as f64;
        }
        s
    }

    /// Crossfades two spectra (`t` 0 gives `self`).
    pub fn lerp(&self, other: &Spectrum, t: f64) -> Spectrum {
        let mut s = Spectrum::zero();
        for k in 0..=MAX_HARMONICS {
            s.re[k] = self.re[k] + (other.re[k] - self.re[k]) * t;
            s.im[k] = self.im[k] + (other.im[k] - self.im[k]) * t;
        }
        s
    }

    /// The frame at length `len` with harmonics up to `harmonics`.
    fn render(&self, harmonics: usize, len: usize) -> Vec<f64> {
        let mut re = vec![0.0; len];
        let mut im = vec![0.0; len];
        let top = harmonics.min(len / 2 - 1).min(MAX_HARMONICS);
        for k in 1..=top {
            re[k] = self.re[k];
            im[k] = self.im[k];
            re[len - k] = self.re[k];
            im[len - k] = -self.im[k];
        }
        fft(&mut re, &mut im, true);
        re
    }
}

/// Brings one cycle to `len` samples with periodic cubic interpolation.
pub fn resample_cycle(cycle: &[f32], len: usize) -> Vec<f32> {
    let n = cycle.len();
    if n == len {
        return cycle.to_vec();
    }
    if n == 0 {
        return vec![0.0; len];
    }
    let at = |i: isize| cycle[i.rem_euclid(n as isize) as usize];
    (0..len)
        .map(|j| {
            let x = j as f64 * n as f64 / len as f64;
            let i = x.floor() as isize;
            let t = (x - i as f64) as f32;
            let (y0, y1, y2, y3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            // Catmull-Rom.
            let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
            let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let c = -0.5 * y0 + 0.5 * y2;
            ((a * t + b) * t + c) * t + y1
        })
        .collect()
}

/// A level's table length: 8 samples per cycle of its top harmonic at
/// least, 64 to [`FRAME`].
fn level_len(harmonics: usize) -> usize {
    (harmonics * 8).next_power_of_two().clamp(64, FRAME)
}

/// A built wavetable. Share it with `Arc`; never drop one on the audio
/// thread (hand it back to the control thread).
#[derive(Clone, Debug)]
pub struct Table {
    pub name: String,
    frames: usize,
    levels: Vec<Level>,
    preview: Vec<f32>,
}

/// Why a table could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableError {
    Empty,
    TooManyFrames(usize),
    Silent,
    /// Import: the audio isn't a whole number of frames and is too long to be
    /// one cycle.
    NotFrames(usize),
    /// From a sample: no steady pitch to cut cycles from.
    NoPitch,
    Wav(&'static str),
}

impl std::fmt::Display for TableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableError::Empty => write!(f, "no frames"),
            TableError::TooManyFrames(n) => write!(f, "{n} frames (at most {MAX_FRAMES})"),
            TableError::Silent => write!(f, "the table is silent"),
            TableError::NotFrames(n) => write!(f, "{n} samples is not a whole number of frames"),
            TableError::NoPitch => write!(f, "no steady pitch found to cut cycles from"),
            TableError::Wav(why) => write!(f, "WAV: {why}"),
        }
    }
}

impl std::error::Error for TableError {}

impl Table {
    /// Builds a table from frame spectra, normalised so the loudest frame
    /// peaks at 1. Allocates; call it off the audio thread.
    pub fn from_spectra(name: &str, spectra: &[Spectrum]) -> Result<Table, TableError> {
        if spectra.is_empty() {
            return Err(TableError::Empty);
        }
        if spectra.len() > MAX_FRAMES {
            return Err(TableError::TooManyFrames(spectra.len()));
        }
        let frames = spectra.len();
        // Peak of the full-band frames sets the gain for the whole table.
        let full: Vec<Vec<f64>> = spectra
            .iter()
            .map(|s| s.render(MAX_HARMONICS, FRAME))
            .collect();
        let peak = full
            .iter()
            .flat_map(|f| f.iter())
            .fold(0.0f64, |m, v| m.max(v.abs()));
        // NaN counts as silent too.
        if peak.is_nan() || peak <= 1e-9 {
            return Err(TableError::Silent);
        }
        let gain = 1.0 / peak;
        let mut levels = Vec::with_capacity(LEVEL_HARMONICS.len());
        for &h in &LEVEL_HARMONICS {
            let len = level_len(h);
            let mut data = Vec::with_capacity(frames * (len + 1));
            for (f, s) in spectra.iter().enumerate() {
                let x = if h == MAX_HARMONICS && len == FRAME {
                    full[f].clone()
                } else {
                    s.render(h, len)
                };
                data.extend(x.iter().map(|&v| (v * gain) as f32));
                data.push((x[0] * gain) as f32);
            }
            levels.push(Level {
                harmonics: h,
                len,
                data: data.into_boxed_slice(),
            });
        }
        let mut preview = Vec::with_capacity(frames * PREVIEW);
        for f in &full {
            preview.extend((0..PREVIEW).map(|i| (f[i * FRAME / PREVIEW] * gain) as f32));
        }
        Ok(Table {
            name: name.to_string(),
            frames,
            levels,
            preview,
        })
    }

    /// Builds a table from frames of `frame_len` samples laid end to end.
    pub fn from_frames(name: &str, samples: &[f32], frame_len: usize) -> Result<Table, TableError> {
        if frame_len == 0 || samples.len() < frame_len {
            return Err(TableError::Empty);
        }
        if !samples.len().is_multiple_of(frame_len) {
            return Err(TableError::NotFrames(samples.len()));
        }
        let n = samples.len() / frame_len;
        if n > MAX_FRAMES {
            return Err(TableError::TooManyFrames(n));
        }
        let spectra: Vec<Spectrum> = samples
            .chunks_exact(frame_len)
            .map(Spectrum::of_cycle)
            .collect();
        Table::from_spectra(name, &spectra)
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// The level for a phase increment `dt` (cycles per sample): the most
    /// harmonics that stay below Nyquist.
    #[inline]
    pub fn level_for(&self, dt: f32) -> usize {
        let dt = dt.abs();
        let mut i = 0;
        while i + 1 < self.levels.len() && self.levels[i].harmonics as f32 * dt > 0.5 {
            i += 1;
        }
        i
    }

    /// Reads the table at frame position `pos` (0 to frames - 1, between
    /// frames crossfades) and phase `ph` (0 to 1) on `level`.
    #[inline]
    pub fn read(&self, level: usize, pos: f32, ph: f32) -> f32 {
        let lv = &self.levels[level];
        read_level(lv, self.frames, pos, ph)
    }

    /// [`PREVIEW`] points of frame `f` at full bandwidth, for drawing.
    pub fn preview(&self, f: usize) -> &[f32] {
        let f = f.min(self.frames - 1);
        &self.preview[f * PREVIEW..(f + 1) * PREVIEW]
    }

    /// Bytes this table holds.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.data.len() * 4).sum::<usize>()
            + self.preview.len() * 4
            + self.name.len()
    }
}

/// One sample from a level: linear in phase, linear between frames.
#[inline]
pub fn read_level(lv: &Level, frames: usize, pos: f32, ph: f32) -> f32 {
    let last = frames - 1;
    let pos = pos.clamp(0.0, last as f32);
    let f0 = pos as usize;
    let t = pos - f0 as f32;
    let x = ph * lv.len as f32;
    let i = (x as usize).min(lv.len - 1);
    let fr = x - i as f32;
    let stride = lv.len + 1;
    let a = &lv.data[f0 * stride + i..f0 * stride + i + 2];
    let s0 = a[0] + (a[1] - a[0]) * fr;
    if t == 0.0 || f0 == last {
        return s0;
    }
    let b = &lv.data[(f0 + 1) * stride + i..(f0 + 1) * stride + i + 2];
    let s1 = b[0] + (b[1] - b[0]) * fr;
    s0 + (s1 - s0) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saw(n: usize) -> Vec<f32> {
        (0..n).map(|i| 1.0 - 2.0 * i as f32 / n as f32).collect()
    }

    fn harmonic_energy(x: &[f32], k: usize) -> f64 {
        let n = x.len();
        let (mut c, mut s) = (0.0, 0.0);
        for (i, &v) in x.iter().enumerate() {
            let a = std::f64::consts::TAU * (k * i) as f64 / n as f64;
            c += f64::from(v) * a.cos();
            s += f64::from(v) * a.sin();
        }
        (c * c + s * s).sqrt() * 2.0 / n as f64
    }

    #[test]
    fn levels_hold_only_their_harmonics() {
        let t = Table::from_frames("saw", &saw(FRAME), FRAME).unwrap();
        assert_eq!(t.frames(), 1);
        for lv in t.levels() {
            let f = &lv.frame(0)[..lv.len];
            let h = lv.harmonics;
            // The top harmonic is there, the next one is not.
            assert!(harmonic_energy(f, h) > 0.3 / h as f64, "level {h}");
            if h + 1 < lv.len / 2 {
                assert!(harmonic_energy(f, h + 1) < 1e-6, "level {h} leaks");
            }
            assert_eq!(lv.frame(0)[lv.len], lv.frame(0)[0]);
        }
    }

    #[test]
    fn a_sine_of_any_length_comes_back_a_sine() {
        let n = 600; // an AKWF-style single cycle
        let x: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * i as f32 / n as f32).sin() * 0.5)
            .collect();
        let t = Table::from_frames("sine", &x, n).unwrap();
        // Normalised to a peak of 1.
        let lv = &t.levels()[0];
        let peak = lv.frame(0).iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 1.0).abs() < 1e-3, "{peak}");
        assert!((t.read(0, 0.0, 0.25) - 1.0).abs() < 1e-3);
        assert!(harmonic_energy(&lv.frame(0)[..lv.len], 2) < 1e-4);
    }

    #[test]
    fn picks_levels_below_nyquist() {
        let t = Table::from_frames("saw", &saw(FRAME), FRAME).unwrap();
        for dt in [0.0005f32, 0.002, 0.01, 0.05, 0.2, 0.45] {
            let l = t.level_for(dt);
            assert!(
                t.levels()[l].harmonics as f32 * dt <= 0.5 || l == t.levels().len() - 1,
                "{dt}"
            );
            if l > 0 {
                assert!(t.levels()[l - 1].harmonics as f32 * dt > 0.5);
            }
        }
        assert_eq!(t.level_for(0.0), 0);
    }

    #[test]
    fn frames_crossfade() {
        let mut x = vec![0.0f32; FRAME * 2];
        x[..FRAME].copy_from_slice(&saw(FRAME));
        for (i, v) in x[FRAME..].iter_mut().enumerate() {
            *v = -(1.0 - 2.0 * i as f32 / FRAME as f32);
        }
        let t = Table::from_frames("two", &x, FRAME).unwrap();
        let a = t.read(5, 0.0, 0.1);
        let b = t.read(5, 1.0, 0.1);
        assert!((a + b).abs() < 1e-4);
        assert!(t.read(5, 0.5, 0.1).abs() < 1e-4);
        assert!(t.bytes() > 2 * 4 * FRAME);
    }

    #[test]
    fn refuses_bad_input() {
        assert_eq!(
            Table::from_frames("x", &[], FRAME).unwrap_err(),
            TableError::Empty
        );
        assert_eq!(
            Table::from_frames("x", &[0.0; 100], 64).unwrap_err(),
            TableError::NotFrames(100)
        );
        assert_eq!(
            Table::from_frames("x", &[0.0; 64], 64).unwrap_err(),
            TableError::Silent
        );
        assert!(matches!(
            Table::from_frames("x", &vec![0.1; 64 * 300], 64),
            Err(TableError::TooManyFrames(300))
        ));
    }
}
