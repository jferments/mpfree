//! Waveform peak pyramid.
//!
//! For each channel, the minimum, maximum and RMS of every block of 64
//! frames, then of every 256, 1024 and 4096 frames, stored as 16-bit
//! values. That is about 3 % of the audio's size as 32-bit floats. To draw
//! a view, [`PeakPyramid::columns`] picks the coarsest level whose blocks
//! still fit inside one pixel column, so a 1280-pixel view costs about 1280
//! lookups at any zoom. Closer than 64 frames per column it reads the audio
//! itself, down to single samples.
//!
//! The pyramid is saved next to its source as a cache (`to_bytes`,
//! `from_bytes`) and can always be rebuilt from the audio.

/// Frames per block at the finest level.
pub const BASE_BLOCK: usize = 64;
/// Number of levels; each is 4 times coarser than the one before.
pub const LEVELS: usize = 4;
const FACTOR: usize = 4;
const MAGIC: &[u8; 8] = b"MPFPEAK1";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Column {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PeakError {
    BadInput(&'static str),
    BadCache(&'static str),
}

impl std::fmt::Display for PeakError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PeakError {}

/// One summary value as stored: min, max, RMS in 1/32767 of full scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cell {
    min: i16,
    max: i16,
    rms: i16,
}

fn q(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}
fn uq(x: i16) -> f32 {
    f32::from(x) / 32767.0
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeakPyramid {
    channels: usize,
    frames: u64,
    /// levels[k][block * channels + ch]
    levels: Vec<Vec<Cell>>,
}

impl PeakPyramid {
    /// Builds the pyramid from interleaved samples.
    pub fn build(data: &[f32], channels: usize) -> Result<PeakPyramid, PeakError> {
        if channels == 0 || channels > 2 {
            return Err(PeakError::BadInput("channels"));
        }
        if !data.len().is_multiple_of(channels) {
            return Err(PeakError::BadInput("length"));
        }
        let frames = data.len() / channels;
        let blocks = frames.div_ceil(BASE_BLOCK);
        let mut base = Vec::with_capacity(blocks * channels);
        for b in 0..blocks {
            let f0 = b * BASE_BLOCK;
            let f1 = (f0 + BASE_BLOCK).min(frames);
            for ch in 0..channels {
                let (mut mn, mut mx, mut sq) = (f32::MAX, f32::MIN, 0.0f64);
                for f in f0..f1 {
                    let x = data[f * channels + ch];
                    let x = if x.is_finite() { x } else { 0.0 };
                    mn = mn.min(x);
                    mx = mx.max(x);
                    sq += f64::from(x) * f64::from(x);
                }
                base.push(Cell {
                    min: q(mn),
                    max: q(mx),
                    rms: q((sq / (f1 - f0) as f64).sqrt() as f32),
                });
            }
        }
        let mut levels = vec![base];
        for _ in 1..LEVELS {
            let prev = levels.last().unwrap();
            let pblocks = prev.len() / channels;
            let nblocks = pblocks.div_ceil(FACTOR);
            let mut next = Vec::with_capacity(nblocks * channels);
            for b in 0..nblocks {
                for ch in 0..channels {
                    let (mut mn, mut mx, mut sq, mut n) = (i16::MAX, i16::MIN, 0.0f64, 0usize);
                    for k in b * FACTOR..((b + 1) * FACTOR).min(pblocks) {
                        let c = prev[k * channels + ch];
                        mn = mn.min(c.min);
                        mx = mx.max(c.max);
                        sq += f64::from(uq(c.rms)).powi(2);
                        n += 1;
                    }
                    next.push(Cell {
                        min: mn,
                        max: mx,
                        rms: q((sq / n as f64).sqrt() as f32),
                    });
                }
            }
            levels.push(next);
        }
        Ok(PeakPyramid {
            channels,
            frames: frames as u64,
            levels,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Bytes held in memory by the summary.
    pub fn memory_bytes(&self) -> usize {
        self.levels
            .iter()
            .map(|l| l.len() * std::mem::size_of::<Cell>())
            .sum()
    }

    /// Fills `out` (one entry per pixel column) for channel `ch`, starting at
    /// frame `start` with `frames_per_column` frames per column. Columns past
    /// the end of the audio are zero. `audio` is the interleaved source, used
    /// when zoomed closer than one finest block per column; pass `None` to
    /// always use the summary. Does not allocate.
    pub fn columns(
        &self,
        audio: Option<&[f32]>,
        ch: usize,
        start: f64,
        frames_per_column: f64,
        out: &mut [Column],
    ) {
        let ch = ch.min(self.channels - 1);
        let fpc = frames_per_column.max(1e-6);
        // Coarsest level whose block is no wider than a column.
        let mut level: Option<usize> = None;
        for k in 0..LEVELS {
            if (BASE_BLOCK * FACTOR.pow(k as u32)) as f64 <= fpc {
                level = Some(k);
            }
        }
        let raw = match (level, audio) {
            (None, Some(a)) if a.len() == self.frames as usize * self.channels => Some(a),
            _ => None,
        };
        let level = level.unwrap_or(0);
        let block = (BASE_BLOCK * FACTOR.pow(level as u32)) as f64;
        let cells = &self.levels[level];
        let nblocks = cells.len() / self.channels;
        for (i, col) in out.iter_mut().enumerate() {
            let f0 = start + i as f64 * fpc;
            let f1 = f0 + fpc;
            if f1 <= 0.0 || f0 >= self.frames as f64 {
                *col = Column::default();
                continue;
            }
            if let Some(a) = raw {
                // Close zoom: the samples themselves.
                let a0 = f0.max(0.0).floor() as usize;
                let a1 = (f1.ceil() as usize).min(self.frames as usize).max(a0 + 1);
                let (mut mn, mut mx, mut sq) = (f32::MAX, f32::MIN, 0.0f32);
                for f in a0..a1 {
                    let x = a[f * self.channels + ch];
                    mn = mn.min(x);
                    mx = mx.max(x);
                    sq += x * x;
                }
                *col = Column {
                    min: mn,
                    max: mx,
                    rms: (sq / (a1 - a0) as f32).sqrt(),
                };
                continue;
            }
            let b0 = (f0.max(0.0) / block).floor() as usize;
            let b1 = ((f1 / block).ceil() as usize).min(nblocks).max(b0 + 1);
            let (mut mn, mut mx, mut sq, mut n) = (i16::MAX, i16::MIN, 0.0f32, 0u32);
            for b in b0..b1.min(nblocks) {
                let c = cells[b * self.channels + ch];
                mn = mn.min(c.min);
                mx = mx.max(c.max);
                sq += uq(c.rms) * uq(c.rms);
                n += 1;
            }
            *col = if n == 0 {
                Column::default()
            } else {
                Column {
                    min: uq(mn),
                    max: uq(mx),
                    rms: (sq / n as f32).sqrt(),
                }
            };
        }
    }

    /// The cache file's bytes: magic, channels, frames, then every level.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.memory_bytes());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.channels as u32).to_le_bytes());
        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(&(LEVELS as u32).to_le_bytes());
        for l in &self.levels {
            for c in l {
                out.extend_from_slice(&c.min.to_le_bytes());
                out.extend_from_slice(&c.max.to_le_bytes());
                out.extend_from_slice(&c.rms.to_le_bytes());
            }
        }
        out
    }

    /// Reads a cache file. Any mismatch is an error, and the caller rebuilds.
    pub fn from_bytes(b: &[u8]) -> Result<PeakPyramid, PeakError> {
        if b.len() < 24 || &b[..8] != MAGIC {
            return Err(PeakError::BadCache("magic"));
        }
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let channels = u32_at(8) as usize;
        let frames = u64::from_le_bytes(b[12..20].try_into().unwrap());
        if !(1..=2).contains(&channels) || u32_at(20) as usize != LEVELS {
            return Err(PeakError::BadCache("header"));
        }
        let mut pos: usize = 24;
        let mut levels = Vec::with_capacity(LEVELS);
        let mut blocks = (frames as usize).div_ceil(BASE_BLOCK);
        for _ in 0..LEVELS {
            let n = blocks
                .checked_mul(channels)
                .ok_or(PeakError::BadCache("size"))?;
            let end = pos
                .checked_add(n.checked_mul(6).ok_or(PeakError::BadCache("size"))?)
                .ok_or(PeakError::BadCache("size"))?;
            if end > b.len() {
                return Err(PeakError::BadCache("truncated"));
            }
            let i16_at = |i: usize| i16::from_le_bytes([b[i], b[i + 1]]);
            levels.push(
                (0..n)
                    .map(|k| {
                        let p = pos + 6 * k;
                        Cell {
                            min: i16_at(p),
                            max: i16_at(p + 2),
                            rms: i16_at(p + 4),
                        }
                    })
                    .collect(),
            );
            pos = end;
            blocks = blocks.div_ceil(FACTOR);
        }
        if pos != b.len() {
            return Err(PeakError::BadCache("trailing bytes"));
        }
        Ok(PeakPyramid {
            channels,
            frames,
            levels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frames: usize, period: f32, amp: f32) -> Vec<f32> {
        (0..frames)
            .map(|i| amp * (i as f32 * std::f32::consts::TAU / period).sin())
            .collect()
    }

    /// Brute-force min/max over a frame range, for comparison.
    fn brute(data: &[f32], ch: usize, channels: usize, f0: usize, f1: usize) -> (f32, f32) {
        let mut mn = f32::MAX;
        let mut mx = f32::MIN;
        for f in f0..f1 {
            let x = data[f * channels + ch];
            mn = mn.min(x);
            mx = mx.max(x);
        }
        (mn, mx)
    }

    #[test]
    fn summary_is_about_three_percent() {
        let d = vec![0.1f32; 44_100 * 10 * 2];
        let p = PeakPyramid::build(&d, 2).unwrap();
        let ratio = p.memory_bytes() as f64 / (d.len() * 4) as f64;
        assert!(ratio < 0.04, "{ratio}");
    }

    #[test]
    fn every_zoom_matches_brute_force() {
        // Stereo: left a sine, right a burst in the middle.
        let frames = 100_000;
        let l = sine(frames, 1000.0, 0.8);
        let mut d = Vec::with_capacity(frames * 2);
        for (i, x) in l.iter().enumerate() {
            d.push(*x);
            d.push(if (40_000..41_000).contains(&i) {
                -0.9
            } else {
                0.0
            });
        }
        let p = PeakPyramid::build(&d, 2).unwrap();
        let tol = 1.0 / 32767.0 + 1e-6;
        for fpc in [64.0, 256.0, 1000.0, 4096.0, 20_000.0] {
            let cols = (frames as f64 / fpc).ceil() as usize;
            let mut out = vec![Column::default(); cols];
            for ch in 0..2 {
                p.columns(None, ch, 0.0, fpc, &mut out);
                for (i, c) in out.iter().enumerate() {
                    let f0 = (i as f64 * fpc) as usize;
                    let f1 = (((i + 1) as f64 * fpc) as usize).min(frames);
                    let (mn, mx) = brute(&d, ch, 2, f0, f1);
                    // Blocks can reach a little past the column's edges, so the
                    // summary may be wider than the exact range but never narrower.
                    assert!(
                        c.min <= mn + tol && c.max >= mx - tol,
                        "fpc {fpc} col {i} ch {ch}: {c:?} vs {mn} {mx}"
                    );
                }
            }
        }
        // The burst (frames 40 000 to 41 000) shows on the right channel at the
        // right place at a coarse zoom. Column 9 ends at 40 000, but its last
        // 1024-frame block reaches 39 936 to 40 960, so it shows the burst too:
        // a peak can spread into a neighbour by less than one block, never
        // further and never missing.
        let mut out = vec![Column::default(); 25];
        p.columns(None, 1, 0.0, 4000.0, &mut out);
        assert!(out[10].min < -0.89 && out[9].min < -0.89);
        assert!(out[8].min == 0.0 && out[11].min == 0.0);
    }

    #[test]
    fn close_zoom_reads_the_samples() {
        let d: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        let p = PeakPyramid::build(&d, 1).unwrap();
        let mut out = vec![Column::default(); 10];
        p.columns(Some(&d), 0, 500.0, 1.0, &mut out);
        for (i, c) in out.iter().enumerate() {
            assert_eq!(c.min, (500 + i) as f32 / 1000.0);
            assert_eq!(c.max, c.min);
        }
        // Without the audio it falls back to the finest summary.
        p.columns(None, 0, 500.0, 1.0, &mut out);
        assert!(out[0].max >= 0.5 && out[0].min <= 0.5);
    }

    #[test]
    fn columns_past_the_end_are_empty() {
        let d = vec![0.5f32; 1000];
        let p = PeakPyramid::build(&d, 1).unwrap();
        let mut out = vec![
            Column {
                min: 9.0,
                max: 9.0,
                rms: 9.0
            };
            4
        ];
        p.columns(None, 0, 900.0, 100.0, &mut out);
        assert!((out[0].max - 0.5).abs() < 1e-4);
        assert_eq!(out[1], Column::default());
        p.columns(None, 0, -500.0, 100.0, &mut out);
        assert_eq!(out[0], Column::default());
    }

    #[test]
    fn rms_of_a_sine() {
        let d = sine(65_536, 64.0, 1.0);
        let p = PeakPyramid::build(&d, 1).unwrap();
        let mut out = vec![Column::default(); 1];
        p.columns(None, 0, 0.0, 65_536.0, &mut out);
        assert!(
            (out[0].rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "{}",
            out[0].rms
        );
    }

    #[test]
    fn cache_round_trip_and_damage() {
        let d = sine(12_345, 300.0, 0.5);
        let p = PeakPyramid::build(&d, 1).unwrap();
        let b = p.to_bytes();
        assert_eq!(PeakPyramid::from_bytes(&b).unwrap(), p);
        assert!(PeakPyramid::from_bytes(&b[..b.len() - 1]).is_err());
        let mut bad = b.clone();
        bad[0] = b'X';
        assert!(PeakPyramid::from_bytes(&bad).is_err());
        let mut extra = b;
        extra.push(0);
        assert!(PeakPyramid::from_bytes(&extra).is_err());
    }

    #[test]
    fn bad_input_is_refused_and_odd_values_are_safe() {
        assert!(PeakPyramid::build(&[0.0; 3], 2).is_err());
        assert!(PeakPyramid::build(&[0.0; 3], 3).is_err());
        let p = PeakPyramid::build(&[f32::NAN, 2.0, -2.0, 0.0], 1).unwrap();
        let mut out = vec![Column::default(); 1];
        p.columns(None, 0, 0.0, 64.0, &mut out);
        assert_eq!(
            (out[0].min, out[0].max),
            (-1.0, 1.0),
            "clamped to full scale"
        );
    }
}
