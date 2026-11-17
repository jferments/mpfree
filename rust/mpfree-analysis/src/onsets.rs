//! Transient (onset) detection for transient chop.
//!
//! The method from `docs/dev/design/sampler-architecture.md`:
//!
//! 1. Mix to mono and take a 1024-point FFT every 256 frames (5.8 ms at
//!    44.1 kHz), Hann windowed and centred on the hop.
//! 2. Spectral flux on log magnitude: for each hop, the sum over bins of how
//!    much the compressed magnitude rose since the previous hop. Rises count,
//!    falls don't, so a new hit stands out even over a ringing tail.
//! 3. An adaptive threshold: the median flux over ±8 hops. Every local peak
//!    above it is a candidate, scored by how far above it is, relative to the
//!    strongest candidate in the file.
//! 4. [`OnsetAnalysis::pick`] keeps the candidates whose score clears the
//!    sensitivity, at least 30 ms apart (the stronger one wins), so turning
//!    the sensitivity knob never recomputes the spectrum.
//! 5. Each kept onset is moved to where the hit really starts: the energy in
//!    32-frame blocks around it is searched for the biggest jump over the
//!    1024 frames before it, then walked
//!    back to the first block that stands clearly above the sound before it,
//!    then to the zero crossing just before that, so the slice starts
//!    without a click and without cutting off the attack.

use crate::fft::{hann, Fft};
use crate::zero::previous_zero_crossing;
use crate::AnalysisError;

pub const FFT_SIZE: usize = 1024;
pub const HOP: usize = 256;
/// Median window for the adaptive threshold, in hops each side.
const MEDIAN_HOPS: usize = 8;
/// Candidates closer than this keep only the stronger one.
pub const MIN_GAP_MS: f32 = 30.0;
/// Energy blocks used to refine an onset.
const BLOCK: usize = 32;

/// One onset candidate: where it is and how strong it is (0 to 1, where 1 is
/// the strongest in the file).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Onset {
    pub frame: u64,
    pub score: f32,
}

/// Everything transient chop and tempo detection need from one pass over a
/// source. Built once off the audio thread; cheap to query afterwards.
#[derive(Clone, Debug)]
pub struct OnsetAnalysis {
    rate: u32,
    frames: u64,
    /// Spectral flux, one value per hop: the onset strength curve.
    flux: Vec<f32>,
    /// Candidates in time order, already refined to the start of the hit.
    candidates: Vec<Onset>,
}

impl OnsetAnalysis {
    /// Analyses interleaved audio with 1 or 2 channels.
    pub fn analyse(
        data: &[f32],
        channels: usize,
        rate: u32,
    ) -> Result<OnsetAnalysis, AnalysisError> {
        let mono = crate::mono(data, channels)?;
        if rate == 0 {
            return Err(AnalysisError::BadInput("rate"));
        }
        let frames = mono.len();
        let hops = frames / HOP + 1;
        let fft = Fft::new(FFT_SIZE);
        let win = hann(FFT_SIZE);
        let bins = FFT_SIZE / 2;
        // A full-scale sine has a windowed magnitude of about FFT_SIZE / 4.
        let norm = 4.0 / FFT_SIZE as f32;
        let bands = log_bands(bins);
        let mut prev = vec![0.0f32; bands.len()];
        let mut cur = vec![0.0f32; bands.len()];
        let mut re = vec![0.0f32; FFT_SIZE];
        let mut im = vec![0.0f32; FFT_SIZE];
        let mut flux = Vec::with_capacity(hops);
        for h in 0..hops {
            let centre = (h * HOP) as isize;
            for (i, (r, w)) in re.iter_mut().zip(&win).enumerate() {
                let f = centre - (FFT_SIZE / 2) as isize + i as isize;
                *r = if f >= 0 && (f as usize) < frames {
                    mono[f as usize] * w
                } else {
                    0.0
                };
            }
            im.iter_mut().for_each(|x| *x = 0.0);
            fft.forward(&mut re, &mut im);
            let mut sum = 0.0f32;
            for (j, &(a, b)) in bands.iter().enumerate() {
                let m = (a..b).map(|k| re[k].hypot(im[k])).sum::<f32>() / (b - a) as f32;
                cur[j] = (1.0 + 100.0 * norm * m).ln();
                sum += (cur[j] - prev[j]).max(0.0);
            }
            // The start of the audio is always a slice boundary, so hop 0 is no onset.
            flux.push(if h == 0 { 0.0 } else { sum });
            std::mem::swap(&mut prev, &mut cur);
        }

        // Local peaks above the running median, with their excess.
        let mut raw: Vec<(usize, f32)> = Vec::new();
        let mut window = Vec::with_capacity(2 * MEDIAN_HOPS + 1);
        for h in 1..hops {
            let lo = h.saturating_sub(3);
            let hi = (h + 3).min(hops - 1);
            if (lo..=hi).any(|j| flux[j] > flux[h] || (flux[j] == flux[h] && j < h)) {
                continue;
            }
            window.clear();
            window.extend_from_slice(
                &flux[h.saturating_sub(MEDIAN_HOPS)..(h + MEDIAN_HOPS + 1).min(hops)],
            );
            window.sort_by(f32::total_cmp);
            let median = window[window.len() / 2];
            let excess = flux[h] - median;
            if excess > 0.0 {
                raw.push((h, excess));
            }
        }
        let strongest = raw.iter().map(|r| r.1).fold(0.0f32, f32::max);
        // Digital silence or a steady tone: nothing to chop.
        if strongest < 0.5 {
            return Ok(OnsetAnalysis {
                rate,
                frames: frames as u64,
                flux,
                candidates: Vec::new(),
            });
        }

        let mut candidates: Vec<Onset> = Vec::with_capacity(raw.len());
        for (i, &(h, excess)) in raw.iter().enumerate() {
            // Search from a bit before the hop's window to the window's end,
            // but not into the previous or next candidate's attack.
            let centre = h * HOP;
            let lo = centre.saturating_sub(FFT_SIZE / 2);
            let lo = candidates
                .last()
                .map_or(lo, |c| lo.max(c.frame as usize + BLOCK));
            let hi = (centre + FFT_SIZE).min(frames);
            let hi = raw.get(i + 1).map_or(hi, |n| {
                hi.min((n.0 * HOP).saturating_sub(FFT_SIZE / 2).max(lo + BLOCK))
            });
            let at = refine(&mono, lo, hi.max(lo + BLOCK).min(frames))
                .unwrap_or(centre.min(frames.saturating_sub(1)));
            candidates.push(Onset {
                frame: at as u64,
                score: excess / strongest,
            });
        }
        Ok(OnsetAnalysis {
            rate,
            frames: frames as u64,
            flux,
            candidates,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }
    pub fn frames(&self) -> u64 {
        self.frames
    }
    /// The onset strength curve, one value per [`HOP`] frames.
    pub fn flux(&self) -> &[f32] {
        &self.flux
    }
    /// Every candidate, in time order.
    pub fn candidates(&self) -> &[Onset] {
        &self.candidates
    }

    /// The analysis as bytes, for a cache file beside its source: magic,
    /// rate, frames, the strength curve and the candidates, little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + 4 * self.flux.len() + 12 * self.candidates.len());
        out.extend_from_slice(CACHE_MAGIC);
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(&(self.flux.len() as u32).to_le_bytes());
        for x in &self.flux {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.extend_from_slice(&(self.candidates.len() as u32).to_le_bytes());
        for c in &self.candidates {
            out.extend_from_slice(&c.frame.to_le_bytes());
            out.extend_from_slice(&c.score.to_le_bytes());
        }
        out
    }

    /// Reads [`OnsetAnalysis::to_bytes`]. Anything that doesn't add up (a
    /// strength curve of the wrong length for its frames, candidates out of
    /// order or past the end, a score that isn't from 0 to 1, bytes left
    /// over) is an error, and the caller analyses again.
    pub fn from_bytes(b: &[u8]) -> Result<OnsetAnalysis, AnalysisError> {
        let bad = || AnalysisError::BadInput("cache");
        let mut r = Reader { b, at: 0 };
        if r.take(8).ok_or_else(bad)? != CACHE_MAGIC {
            return Err(bad());
        }
        let rate = r.u32().ok_or_else(bad)?;
        let frames = r.u64().ok_or_else(bad)?;
        let n = r.u32().ok_or_else(bad)? as usize;
        if rate == 0 || n != flux_len(frames) || n > r.left() / 4 {
            return Err(bad());
        }
        let flux: Vec<f32> = (0..n)
            .map(|_| r.f32().filter(|x| x.is_finite()))
            .collect::<Option<_>>()
            .ok_or_else(bad)?;
        let m = r.u32().ok_or_else(bad)? as usize;
        if m > r.left() / 12 {
            return Err(bad());
        }
        let candidates: Vec<Onset> = (0..m)
            .map(|_| {
                let frame = r.u64()?;
                let score = r.f32()?;
                Some(Onset { frame, score })
            })
            .collect::<Option<_>>()
            .ok_or_else(bad)?;
        let in_order = candidates.windows(2).all(|w| w[0].frame <= w[1].frame);
        let fit = candidates
            .iter()
            .all(|c| c.frame < frames.max(1) && (0.0..=1.0).contains(&c.score));
        if r.left() != 0 || !in_order || !fit {
            return Err(bad());
        }
        Ok(OnsetAnalysis {
            rate,
            frames,
            flux,
            candidates,
        })
    }

    /// Onset frames for a sensitivity from 0 (only the strongest hits) to 1
    /// (every candidate), at least [`MIN_GAP_MS`] apart, in time order. More
    /// sensitivity never gives fewer onsets.
    pub fn pick(&self, sensitivity: f32) -> Vec<u64> {
        let s = sensitivity.clamp(0.0, 1.0);
        let threshold = 0.02 + 0.48 * (1.0 - s) * (1.0 - s);
        let gap = (MIN_GAP_MS / 1000.0 * self.rate as f32) as u64;
        let mut keep: Vec<&Onset> = self
            .candidates
            .iter()
            .filter(|c| c.score >= threshold)
            .collect();
        // Stronger first; an onset too close to a stronger kept one goes.
        keep.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.frame.cmp(&b.frame)));
        let mut out: Vec<u64> = Vec::with_capacity(keep.len());
        for c in keep {
            if out.iter().all(|&f| f.abs_diff(c.frame) >= gap) {
                out.push(c.frame);
            }
        }
        out.sort_unstable();
        out
    }
}

/// Groups FFT bins into bands about a quarter of an octave wide (one bin
/// each at the bottom), so a kick's few low bins count as much as a hat's
/// many high ones.
fn log_bands(bins: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut k = 1;
    while k < bins {
        let w = ((k as f32 * 0.19) as usize).max(1).min(bins - k);
        out.push((k, k + w));
        k += w;
    }
    out
}

/// Where the hit inside `mono[lo..hi]` starts, or None if the span is flat.
fn refine(mono: &[f32], lo: usize, hi: usize) -> Option<usize> {
    let blocks = (hi - lo) / BLOCK;
    if blocks < 2 {
        return None;
    }
    let e: Vec<f32> = (0..blocks)
        .map(|b| {
            let s = &mono[lo + b * BLOCK..lo + (b + 1) * BLOCK];
            s.iter().map(|x| x * x).sum::<f32>() / BLOCK as f32
        })
        .collect();
    // Energy over the 32 blocks (1024 frames, longer than one cycle of a
    // 50 Hz kick tail) before each block: the "before".
    let base = |b: usize| -> f32 {
        let from = b.saturating_sub(32);
        if from == b {
            return 0.0;
        }
        e[from..b].iter().sum::<f32>() / (b - from) as f32
    };
    // The block that jumps most above what came before it. The floor keeps
    // tiny wiggles in near-silence from counting as huge jumps.
    let floor = 1e-3 * e.iter().fold(0.0f32, |a, &b| a.max(b)) + 1e-12;
    let (mut best, mut rise) = (None, 0.0f32);
    for b in 1..blocks {
        let r = e[b] / (base(b) + floor);
        if r > rise {
            rise = r;
            best = Some(b);
        }
    }
    let mut b = best?;
    let peak = e[b];
    // Walk back while the block before still stands clearly above what came
    // before it: that is still the attack.
    while b > 1 && e[b - 1] > 3.0 * base(b - 1) + 1e-12 && e[b - 1] > 0.01 * peak {
        b -= 1;
    }
    let start = lo + b * BLOCK;
    // The zero crossing just before, if there is one close by.
    Some(previous_zero_crossing(mono, start, BLOCK).unwrap_or(start))
}

/// The first bytes of [`OnsetAnalysis::to_bytes`].
const CACHE_MAGIC: &[u8; 8] = b"MPFONS01";

/// The strength curve's length for `frames`, as [`OnsetAnalysis::analyse`]
/// makes it.
fn flux_len(frames: u64) -> usize {
    usize::try_from(frames / HOP as u64).map_or(usize::MAX, |h| h.saturating_add(1))
}

/// Little-endian reads from a cache file's bytes.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn left(&self) -> usize {
        self.b.len() - self.at
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny drum machine for tests: deterministic noise, a pitched kick,
    /// a snare and a closed hat.
    pub struct Kit {
        seed: u32,
    }
    impl Kit {
        pub fn new() -> Kit {
            Kit { seed: 0x1234_5678 }
        }
        fn noise(&mut self) -> f32 {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
        }
        /// Adds a hit of `kind` (0 kick, 1 snare, 2 hat) at `at`.
        pub fn hit(&mut self, out: &mut [f32], at: usize, kind: u8, gain: f32, rate: f32) {
            let len = match kind {
                0 => 0.8,
                1 => 0.5,
                _ => 0.12,
            };
            let n = ((len * rate) as usize).min(out.len().saturating_sub(at));
            let mut phase = 0.0f32;
            for i in 0..n {
                let t = i as f32 / rate;
                let x = match kind {
                    0 => {
                        let f = 50.0 + 100.0 * (-t * 30.0).exp();
                        phase += std::f32::consts::TAU * f / rate;
                        phase.sin() * (-t * 9.0).exp()
                    }
                    1 => {
                        0.6 * self.noise() * (-t * 18.0).exp()
                            + 0.4 * (std::f32::consts::TAU * 190.0 * t).sin() * (-t * 25.0).exp()
                    }
                    _ => {
                        let a = self.noise();
                        let b = self.noise();
                        0.5 * (a - b) * (-t * 80.0).exp()
                    }
                };
                out[at + i] += gain * x;
            }
        }
    }

    /// A one-bar-per-`bars` 4/4 beat at `bpm`: kick on 1 and 3, snare on 2
    /// and 4, hats on every eighth. Returns the audio and the hit positions
    /// as (frame, kind).
    pub fn beat(bpm: f32, bars: usize, rate: f32) -> (Vec<f32>, Vec<(usize, u8)>) {
        let eighth = 60.0 / bpm / 2.0 * rate;
        let total = (eighth * 8.0 * bars as f32) as usize + rate as usize / 2;
        let mut out = vec![0.0f32; total];
        let mut kit = Kit::new();
        let mut hits = Vec::new();
        for e in 0..8 * bars {
            let at = (e as f32 * eighth).round() as usize;
            let step = e % 8;
            if step == 0 || step == 4 {
                kit.hit(&mut out, at, 0, 0.8, rate);
                hits.push((at, 0));
            } else if step == 2 || step == 6 {
                kit.hit(&mut out, at, 1, 0.7, rate);
                hits.push((at, 1));
            } else {
                kit.hit(&mut out, at, 2, 0.25, rate);
                hits.push((at, 2));
            }
        }
        (out, hits)
    }

    #[test]
    fn reads_back_from_its_cache_bytes_and_refuses_broken_ones() {
        let (audio, _) = beat(100.0, 2, 44_100.0);
        let a = OnsetAnalysis::analyse(&audio, 1, 44_100).unwrap();
        assert!(!a.candidates().is_empty());
        let b = a.to_bytes();
        let back = OnsetAnalysis::from_bytes(&b).unwrap();
        assert_eq!(back.rate(), a.rate());
        assert_eq!(back.frames(), a.frames());
        assert_eq!(back.flux(), a.flux());
        assert_eq!(back.candidates(), a.candidates());
        assert_eq!(back.pick(0.6), a.pick(0.6));
        // Cut short, one byte over, another magic, a curve that doesn't fit
        // its frames, a score past 1: all refused.
        assert!(OnsetAnalysis::from_bytes(&b[..b.len() - 1]).is_err());
        let mut over = b.clone();
        over.push(0);
        assert!(OnsetAnalysis::from_bytes(&over).is_err());
        let mut magic = b.clone();
        magic[0] ^= 1;
        assert!(OnsetAnalysis::from_bytes(&magic).is_err());
        let mut frames = b.clone();
        frames[13] ^= 0x40; // the frames' second byte: 16 k frames off
        assert!(OnsetAnalysis::from_bytes(&frames).is_err());
        let mut score = b.clone();
        let last = score.len() - 4;
        score[last..].copy_from_slice(&1.5f32.to_le_bytes());
        assert!(OnsetAnalysis::from_bytes(&score).is_err());
        // Silence: no candidates, still read back.
        let quiet = OnsetAnalysis::analyse(&[0.0; 5_000], 1, 44_100).unwrap();
        let q = OnsetAnalysis::from_bytes(&quiet.to_bytes()).unwrap();
        assert!(q.candidates().is_empty());
        assert_eq!(q.flux().len(), quiet.flux().len());
    }

    #[test]
    fn finds_every_hit_of_a_beat_where_it_starts() {
        let rate = 44_100.0;
        let (audio, hits) = beat(100.0, 2, rate);
        // The hit at frame 0 needs no marker: the start is always a boundary.
        let hits: Vec<_> = hits.into_iter().filter(|h| h.0 > 0).collect();
        let a = OnsetAnalysis::analyse(&audio, 1, 44_100).unwrap();
        let found = a.pick(0.6);
        assert_eq!(found.len(), hits.len(), "found {found:?}");
        for (f, (h, kind)) in found.iter().zip(&hits) {
            let d = *f as i64 - *h as i64;
            // Never late enough to lose the attack, never more than 1.5 ms early.
            assert!(
                (-64..=8).contains(&d),
                "hit at {h} (kind {kind}) found at {f} ({d:+})"
            );
        }
    }

    #[test]
    fn sensitivity_adds_quieter_hits_and_never_removes_any() {
        let (audio, hits) = beat(120.0, 2, 44_100.0);
        let a = OnsetAnalysis::analyse(&audio, 1, 44_100).unwrap();
        let mut last = 0;
        for s in 0..=10 {
            let n = a.pick(s as f32 / 10.0).len();
            assert!(n >= last, "sensitivity {s}: {n} < {last}");
            last = n;
        }
        // At the bottom, fewer slices, and every one of them is a real hit.
        let low = a.pick(0.0);
        assert!(!low.is_empty() && low.len() < hits.len(), "{low:?}");
        for f in &low {
            assert!(
                hits.iter().any(|h| f.abs_diff(h.0 as u64) < 64),
                "{f} is not a hit"
            );
        }
    }

    #[test]
    fn stereo_and_noise_floor() {
        let rate = 44_100.0;
        let (mono, hits) = beat(90.0, 1, rate);
        let mut kit = Kit::new();
        let mut d = Vec::with_capacity(mono.len() * 2);
        for x in &mono {
            // Hits on the left only; a -60 dB hiss on both sides.
            d.push(x + 0.001 * kit.noise());
            d.push(0.001 * kit.noise());
        }
        let a = OnsetAnalysis::analyse(&d, 2, 44_100).unwrap();
        assert_eq!(
            a.pick(0.6).len(),
            hits.len() - 1,
            "all but the one at frame 0"
        );
    }

    #[test]
    fn silence_and_steady_tones_have_no_onsets() {
        let a = OnsetAnalysis::analyse(&vec![0.0; 44_100], 1, 44_100).unwrap();
        assert!(a.candidates().is_empty() && a.pick(1.0).is_empty());
        let tone: Vec<f32> = (0..44_100).map(|i| 0.5 * (i as f32 * 0.05).sin()).collect();
        let a = OnsetAnalysis::analyse(&tone, 1, 44_100).unwrap();
        assert!(
            a.pick(0.6).len() <= 1,
            "a tone that starts at 0 may count once: {:?}",
            a.pick(0.6)
        );
        assert!(OnsetAnalysis::analyse(&[0.0; 3], 2, 44_100).is_err());
        assert!(OnsetAnalysis::analyse(&[], 1, 44_100).is_ok());
    }

    #[test]
    fn close_hits_keep_the_stronger() {
        let rate = 44_100.0;
        let mut out = vec![0.0f32; 44_100];
        let mut kit = Kit::new();
        kit.hit(&mut out, 10_000, 2, 0.3, rate);
        kit.hit(&mut out, 10_000 + 441, 0, 0.9, rate); // 10 ms later, much louder
        kit.hit(&mut out, 30_000, 1, 0.7, rate);
        let a = OnsetAnalysis::analyse(&out, 1, 44_100).unwrap();
        let p = a.pick(0.6);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].abs_diff(10_441) < 64, "the louder kick wins: {p:?}");
        assert!(p[1].abs_diff(30_000) < 64);
    }
}
