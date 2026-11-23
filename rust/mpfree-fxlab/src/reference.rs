//! Y8 of the DSP quality plan (`docs/dev/design/dsp-quality.md`, step
//! 0.5): each aliasing case (A1's engines and settings, at C5, C6 and C7)
//! rendered at 44.1 kHz and again at 8 times the rate, the 8x render
//! brought down to 44.1 kHz with a long linear-phase filter, and the two
//! compared by their spectra:
//!
//! - **fizz:** the energy off the harmonics against the energy on them,
//!   20 Hz to 20 kHz, in the 44.1 kHz render and in the reference. What
//!   the reference has too (FM feedback's noise, a bore's breath) is the
//!   sound; what only the 44.1 kHz render has is folding.
//! - **harmonics:** the largest difference in any harmonic within 60 dB of
//!   the loudest, below 16 kHz: what a fix may not take away.
//! - **top:** the harmonics' power from 16 to 20 kHz against the
//!   reference's: the top end an oscillator's band limit or a filter near
//!   the top gives up (negative) or adds.
//! - **envelope:** the largest difference in any third-octave band below
//!   16 kHz within 60 dB of the loudest.
//! - **level:** RMS against RMS.
//!
//! The reference is trusted only where it has converged: with `sixteen`,
//! each case is also rendered at 16 times the rate and the 8x reference is
//! compared with it (see [`Y8::agrees`]). Desktop only.
//!
//! [`y3`] measures Y3 the same way: Analog's poly-mod at full from
//! oscillator 2 into the cutoff, into oscillator 1's pitch (with sync, so
//! the note repeats exactly and its harmonics can be read), and into
//! both; and a fast filter envelope on each synth with one, struck again
//! every eighth of a second. For a struck note, a harmonic is its line,
//! the part every strike shares; the rest of each strike counts with the
//! fizz, in the render and in its reference alike. Reported only.

use std::f64::consts::PI;

use crate::fft::power_spectrum;
use crate::synth::{self, Engine};

/// The decimation filter: passband to 20 kHz, stopband from 24.1 kHz
/// (whatever folds from there lands above 20 kHz at 44.1 kHz), Kaiser
/// window with beta 12.3 (about 120 dB down).
const PASS_HZ: f64 = 20_000.0;
const STOP_HZ: f64 = 24_100.0;
const KAISER_BETA: f64 = 12.3;

/// The zeroth-order modified Bessel function of the first kind.
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = x * x / 4.0;
    for k in 1..60 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// A linear-phase low-pass for a signal at `rate` to be decimated to
/// 44.1 kHz: odd length, unity gain at DC.
pub fn decimation_taps(rate: u32) -> Vec<f64> {
    let fs = f64::from(rate);
    let width = (STOP_HZ - PASS_HZ) / fs;
    // Kaiser's estimate for 120 dB, made odd.
    let n = ((120.0 - 8.0) / (2.285 * 2.0 * PI * width)).ceil() as usize | 1;
    let fc = 0.5 * (PASS_HZ + STOP_HZ) / fs;
    let m = (n - 1) as f64 / 2.0;
    let i0b = bessel_i0(KAISER_BETA);
    let mut h: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 - m;
            let sinc = if t == 0.0 {
                2.0 * fc
            } else {
                (2.0 * PI * fc * t).sin() / (PI * t)
            };
            let r = t / m;
            sinc * bessel_i0(KAISER_BETA * (1.0 - r * r).max(0.0).sqrt()) / i0b
        })
        .collect();
    let sum: f64 = h.iter().sum();
    for v in &mut h {
        *v /= sum;
    }
    h
}

/// `x` (at `factor` times the output rate) filtered and decimated by
/// `factor`, delay removed: output frame `j` is input frame `j * factor`.
pub fn decimate(x: &[f64], taps: &[f64], factor: usize) -> Vec<f64> {
    let half = taps.len() / 2;
    let out_len = x.len() / factor;
    (0..out_len)
        .map(|j| {
            let c = j * factor;
            let mut acc = 0.0;
            for (k, &h) in taps.iter().enumerate() {
                let i = c as isize + k as isize - half as isize;
                if i >= 0 && (i as usize) < x.len() {
                    acc += h * x[i as usize];
                }
            }
            acc
        })
        .collect()
}

/// Y8's numbers for one case and note.
#[derive(Clone, Debug)]
pub struct Y8 {
    /// "Poly saw, C6".
    pub case: String,
    /// Fizz (off-harmonic energy against on-harmonic, dB) at 44.1 kHz and
    /// in the 8x reference.
    pub fizz_db: f64,
    pub ref_fizz_db: f64,
    /// The largest harmonic difference below 16 kHz, 44.1 kHz against the
    /// reference, dB.
    pub harmonic_err_db: f64,
    /// The harmonics' power from 16 to 20 kHz against the reference's, dB;
    /// None when the reference has none there within 60 dB of its loudest.
    pub top_db: Option<f64>,
    /// The largest third-octave band difference below 16 kHz, dB.
    pub envelope_err_db: f64,
    /// RMS against RMS, dB.
    pub level_db: f64,
    /// With 16x: the 8x reference against the 16x one (the largest
    /// harmonic difference below 20 kHz, dB) and the 16x reference's fizz.
    pub converge_harmonic_db: Option<f64>,
    pub ref16_fizz_db: Option<f64>,
}

/// Fizz below this (dB) is the measure's own floor (the window, the
/// guard bins round each harmonic): two references both under it agree.
pub const FIZZ_FLOOR_DB: f64 = -80.0;

impl Y8 {
    /// Whether the 8x reference has converged (None without the 16x
    /// render): its harmonics within 1 dB of the 16x one's, and its fizz
    /// within 3 dB of the 16x fizz or both under [`FIZZ_FLOOR_DB`].
    pub fn agrees(&self) -> Option<bool> {
        let (h, f16) = (self.converge_harmonic_db?, self.ref16_fizz_db?);
        let fizz_ok = (self.ref_fizz_db - f16).abs() <= 3.0
            || (self.ref_fizz_db < FIZZ_FLOOR_DB && f16 < FIZZ_FLOOR_DB);
        Some(h <= 1.0 && fizz_ok)
    }
}

/// Where "the top" starts: above it, an oscillator's band limit and a
/// filter near Nyquist decide what is left.
const TOP_HZ: f64 = 16_000.0;

/// The spectral picture of one render, from its own fundamental.
struct Picture {
    f0: f64,
    /// Harmonic powers (by number from 1) below 20 kHz.
    harmonics: Vec<f64>,
    /// Power off the harmonics, 20 Hz to 20 kHz.
    off: f64,
    /// Third-octave band powers from 20 Hz to 20 kHz.
    bands: Vec<f64>,
    rms: f64,
}

const N: usize = 65_536;

fn picture(x: &[f64], rate: u32, f0: f64, from: usize) -> Picture {
    let seg = &x[from..from + N];
    let w = synth::blackman_harris(N);
    let v: Vec<f64> = seg.iter().zip(&w).map(|(a, b)| a * b).collect();
    let ps = power_spectrum(&v);
    let bin = f64::from(rate) / N as f64;
    let guard = 8.0;
    let top = (20_000.0 / f0).floor() as usize;
    let mut harmonics = vec![0.0f64; top.max(1)];
    let mut off = 0.0;
    let mut bands = vec![0.0f64; 31];
    for (k, &p) in ps.iter().enumerate() {
        let f = k as f64 * bin;
        if !(20.0..=20_000.0).contains(&f) {
            continue;
        }
        let h = (f / f0).round().max(1.0);
        if ((f - h * f0) / bin).abs() <= guard {
            let i = h as usize - 1;
            if i < harmonics.len() {
                harmonics[i] += p;
            }
        } else {
            off += p;
        }
        let b = ((f / 20.0).log2() * 3.0).floor() as usize;
        if b < bands.len() {
            bands[b] += p;
        }
    }
    let rms = (seg.iter().map(|a| a * a).sum::<f64>() / N as f64).sqrt();
    Picture {
        f0,
        harmonics,
        off,
        bands,
        rms,
    }
}

fn db10(p: f64) -> f64 {
    10.0 * p.max(1e-30).log10()
}

fn fizz(p: &Picture) -> f64 {
    db10(p.off / p.harmonics.iter().sum::<f64>().max(1e-300)).max(-160.0)
}

/// The harmonics below [`TOP_HZ`].
fn below_top(p: &Picture) -> &[f64] {
    let n = ((TOP_HZ / p.f0).floor() as usize).min(p.harmonics.len());
    &p.harmonics[..n]
}

/// The harmonics' power from [`TOP_HZ`] to 20 kHz.
fn top_power(p: &Picture) -> f64 {
    p.harmonics
        .iter()
        .enumerate()
        .filter(|(i, _)| (*i + 1) as f64 * p.f0 >= TOP_HZ)
        .map(|(_, &v)| v)
        .sum()
}

/// The third-octave bands that end below [`TOP_HZ`].
fn bands_below_top(p: &Picture) -> &[f64] {
    let n = (0..p.bands.len())
        .take_while(|&b| 20.0 * ((b + 1) as f64 / 3.0).exp2() <= TOP_HZ)
        .count();
    &p.bands[..n]
}

/// The largest difference (dB) between `a` and `b` in entries within 60 dB
/// of `b`'s loudest.
fn worst_diff(a: &[f64], b: &[f64]) -> f64 {
    let top = b.iter().cloned().fold(0.0, f64::max);
    a.iter()
        .zip(b)
        .filter(|(_, &q)| q > top * 1e-6)
        .map(|(&p, &q)| (db10(p) - db10(q)).abs())
        .fold(0.0, f64::max)
}

/// How often a struck case strikes its note (s); each strike is held
/// half as long.
const STRIKE_S: f64 = 0.125;

/// One case's note rendered at `factor` times 44.1 kHz, as the A1 rows
/// play it: held 2 s in a 2 s render, after as long a settle in seconds;
/// or `struck`, played every [`STRIKE_S`] for the 2 s.
fn render(e: Engine, set: &[(u16, f32)], key: u8, factor: usize, struck: bool) -> Vec<f64> {
    let rate = 44_100 * factor as u32;
    let mut n = e.plain(rate);
    for &(id, v) in set {
        n.set_param(id, v);
    }
    let notes: Vec<(u8, u8, f64, f64)> = if struck {
        let strikes = (2.0 / STRIKE_S).round() as usize;
        (0..strikes)
            .map(|i| (key, 100, i as f64 * STRIKE_S, STRIKE_S / 2.0))
            .collect()
    } else {
        vec![(key, 100, 0.0, 2.0)]
    };
    let score = synth::score(rate, &notes);
    let len = synth::frames(2.0, rate);
    synth::render_after(n.as_mut(), rate, len, &score, synth::SETTLE_BLOCKS * factor).mid()
}

/// A render at `factor` times 44.1 kHz brought down to 44.1 kHz.
fn reference(e: Engine, set: &[(u16, f32)], key: u8, factor: usize, struck: bool) -> Vec<f64> {
    let x = render(e, set, key, factor, struck);
    decimate(&x, &decimation_taps(44_100 * factor as u32), factor)
}

/// One case at one note: its name, the engine and settings over its plain
/// patch, the note, and whether it is struck again and again.
type Job = (String, Engine, Vec<(u16, f32)>, u8, bool);

/// Y8 for every A1 case at C5, C6 and C7; with `sixteen`, the 16x check
/// as well. Cases run on every core.
pub fn y8(sixteen: bool) -> Vec<Y8> {
    let mut jobs = Vec::new();
    for (what, e, set) in synth::alias_cases() {
        for (k, label) in [(72u8, "C5"), (84, "C6"), (96, "C7")] {
            jobs.push((format!("{what}, {label}"), e, set.clone(), k, false));
        }
    }
    measure(&jobs, sixteen)
}

/// A Y3 case: its name, the engine and settings over its plain patch, and
/// whether it is struck again and again.
type Y3Case = (&'static str, Engine, Vec<(u16, f32)>, bool);

/// Y3's cases: Analog's poly-mod at full, and a fast filter envelope on
/// each synth with one (struck).
fn y3_cases() -> Vec<Y3Case> {
    use mpfree_synth::{
        analog_params as an, bass_params as bp, params as pp, squelch_params as sp,
    };
    // Oscillator 2 exactly at the note (its fine tune is 6 cents up
    // otherwise), heard only through the poly-mod (into the pitch unless
    // turned off).
    let pm = |more: &[(u16, f32)]| {
        let mut v = vec![(an::OSC2_FINE, 0.0), (an::PM_OSC2, 100.0)];
        v.extend_from_slice(more);
        v
    };
    vec![
        (
            "Analog poly-mod > cutoff 100",
            Engine::Analog,
            pm(&[
                (an::PM_PITCH, 0.0),
                (an::PM_CUTOFF, 1.0),
                (an::CUTOFF, 1_000.0),
                (an::RESONANCE, 50.0),
            ]),
            false,
        ),
        (
            "Analog poly-mod > pitch 100, sync",
            Engine::Analog,
            pm(&[(an::PM_PITCH, 1.0), (an::SYNC, 1.0)]),
            false,
        ),
        (
            "Analog poly-mod > pitch and cutoff 100, sync",
            Engine::Analog,
            pm(&[
                (an::PM_PITCH, 1.0),
                (an::PM_CUTOFF, 1.0),
                (an::SYNC, 1.0),
                (an::CUTOFF, 1_000.0),
                (an::RESONANCE, 50.0),
            ]),
            false,
        ),
        // Each envelope at its fastest decay or 20 ms, its whole sweep up
        // from a low cutoff, resonance at half.
        (
            "Poly fast filter envelope",
            Engine::Poly,
            vec![
                (pp::CUTOFF, 200.0),
                (pp::RESONANCE, 50.0),
                (pp::FENV_AMOUNT, 5.0),
                (pp::FENV_A, 0.1),
                (pp::FENV_D, 20.0),
                (pp::FENV_S, 0.0),
            ],
            true,
        ),
        (
            "Analog fast filter envelope",
            Engine::Analog,
            vec![
                (an::CUTOFF, 200.0),
                (an::RESONANCE, 50.0),
                (an::ENV_AMT, 100.0),
                (an::F_ATTACK, 1.0),
                (an::F_DECAY, 20.0),
                (an::F_SUSTAIN, 0.0),
            ],
            true,
        ),
        (
            "Squelch fast filter envelope",
            Engine::Squelch,
            vec![
                (sp::CUTOFF, 200.0),
                (sp::RESONANCE, 50.0),
                (sp::ENV_MOD, 100.0),
                (sp::DECAY, 200.0),
            ],
            true,
        ),
        (
            "Bass fast filter envelope",
            Engine::Bass,
            vec![
                (bp::CUTOFF, 200.0),
                (bp::RESONANCE, 50.0),
                (bp::ENV_AMOUNT, 100.0),
                (bp::FILTER_DECAY, 20.0),
            ],
            true,
        ),
    ]
}

/// Y3 (reported only) at C4 and C6; with `sixteen`, the 16x check as well.
pub fn y3(sixteen: bool) -> Vec<Y8> {
    let mut jobs = Vec::new();
    for (what, e, set, struck) in y3_cases() {
        for (k, label) in [(60u8, "C4"), (84, "C6")] {
            jobs.push((format!("{what}, {label}"), e, set.clone(), k, struck));
        }
    }
    measure(&jobs, sixteen)
}

/// Each job against its 8x reference (and the 16x check with `sixteen`),
/// on every core.
fn measure(jobs: &[Job], sixteen: bool) -> Vec<Y8> {
    let rate = 44_100;
    let from = synth::frames(0.3, rate);
    let run = |(case, e, set, k, struck): &Job| {
        let (e, k, struck) = (*e, *k, *struck);
        let x = render(e, set, k, 1, struck);
        let r8 = reference(e, set, k, 8, struck);
        let near = 440.0 * ((f64::from(k) - 69.0) / 12.0).exp2();
        let f0 = synth::freq_near(&r8, rate, near, from).unwrap_or(near);
        let fx = synth::freq_near(&x, rate, near, from).unwrap_or(near);
        let (px, p8) = (picture(&x, rate, fx, from), picture(&r8, rate, f0, from));
        let (converge, f16) = if sixteen {
            let r16 = reference(e, set, k, 16, struck);
            let f = synth::freq_near(&r16, rate, near, from).unwrap_or(near);
            let p16 = picture(&r16, rate, f, from);
            (
                Some(worst_diff(&p8.harmonics, &p16.harmonics)),
                Some(fizz(&p16)),
            )
        } else {
            (None, None)
        };
        let (hx, h8) = (below_top(&px), below_top(&p8));
        let n = hx.len().min(h8.len());
        let loudest = p8.harmonics.iter().cloned().fold(0.0, f64::max);
        let top8 = top_power(&p8);
        Y8 {
            case: case.clone(),
            fizz_db: fizz(&px),
            ref_fizz_db: fizz(&p8),
            harmonic_err_db: worst_diff(&hx[..n], &h8[..n]),
            top_db: (top8 > loudest * 1e-6).then(|| db10(top_power(&px)) - db10(top8)),
            envelope_err_db: worst_diff(bands_below_top(&px), bands_below_top(&p8)),
            level_db: 20.0 * (px.rms / p8.rms.max(1e-30)).max(1e-30).log10(),
            converge_harmonic_db: converge,
            ref16_fizz_db: f16,
        }
    };
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out: Vec<std::sync::Mutex<Option<Y8>>> =
        jobs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..threads.min(jobs.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= jobs.len() {
                    break;
                }
                *out[i].lock().unwrap() = Some(run(&jobs[i]));
            });
        }
    });
    out.into_iter()
        .map(|m| m.into_inner().unwrap().expect("every case measured"))
        .collect()
}

/// The rows as text, one case a line.
pub fn to_text(rows: &[Y8]) -> String {
    table("Y8", rows)
}

/// Y3's rows as text, one case a line.
pub fn y3_text(rows: &[Y8]) -> String {
    table("Y3", rows)
}

fn table(id: &str, rows: &[Y8]) -> String {
    let w = rows.iter().map(|r| r.case.len()).max().unwrap_or(0).max(38);
    let mut s = format!(
        "{id}   {:<w$}    fizz  ref fizz  16x fizz  harmonics     top  envelope   level  8x vs 16x  agrees\n",
        "case"
    );
    for r in rows {
        let opt = |v: Option<f64>, p: usize| v.map_or("-".to_string(), |v| format!("{v:.p$}"));
        s.push_str(&format!(
            "{id}   {:<w$} {:>7.1} {:>9.1} {:>9} {:>10.2} {:>7} {:>9.2} {:>7.2} {:>10} {:>7}\n",
            r.case,
            r.fizz_db,
            r.ref_fizz_db,
            opt(r.ref16_fizz_db, 1),
            r.harmonic_err_db,
            opt(r.top_db, 1),
            r.envelope_err_db,
            r.level_db,
            opt(r.converge_harmonic_db, 2),
            r.agrees().map_or("-", |a| if a { "yes" } else { "no" }),
        ));
    }
    let checked: Vec<_> = rows.iter().filter_map(|r| r.agrees()).collect();
    if !checked.is_empty() {
        s.push_str(&format!(
            "{id}: the 8x reference agrees with 16x in {} of {} cases\n",
            checked.iter().filter(|a| **a).count(),
            checked.len()
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The filter's gain at `f` Hz for a signal at `rate`.
    fn gain_db(taps: &[f64], f: f64, rate: u32) -> f64 {
        let w = 2.0 * PI * f / f64::from(rate);
        let (mut re, mut im) = (0.0, 0.0);
        for (k, &h) in taps.iter().enumerate() {
            re += h * (w * k as f64).cos();
            im -= h * (w * k as f64).sin();
        }
        20.0 * re.hypot(im).max(1e-30).log10()
    }

    #[test]
    fn the_decimation_filter_is_flat_to_20_khz_and_120_db_down_from_24_1() {
        for factor in [8usize, 16] {
            let rate = 44_100 * factor as u32;
            let h = decimation_taps(rate);
            assert_eq!(h.len() % 2, 1);
            for f in [20.0, 1_000.0, 10_000.0, 18_000.0, 20_000.0] {
                let g = gain_db(&h, f, rate);
                assert!(g.abs() < 0.001, "{factor}x: {g} dB at {f} Hz");
            }
            let mut f = STOP_HZ;
            while f < f64::from(rate) / 2.0 {
                let g = gain_db(&h, f, rate);
                assert!(g < -115.0, "{factor}x: {g} dB at {f} Hz");
                f += 997.0;
            }
        }
    }

    #[test]
    fn decimation_keeps_a_tone_and_its_timing() {
        let factor = 8;
        let rate = 44_100 * factor as u32;
        let f = 1_234.5;
        let x: Vec<f64> = (0..rate as usize / 4)
            .map(|i| (2.0 * PI * f * i as f64 / f64::from(rate)).sin())
            .collect();
        let y = decimate(&x, &decimation_taps(rate), factor);
        // Away from the ends, output frame j is input frame 8 j.
        for j in 2_000..2_100 {
            assert!((y[j] - x[j * factor]).abs() < 1e-5, "frame {j}");
        }
    }
}
