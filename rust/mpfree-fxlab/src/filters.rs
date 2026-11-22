//! DSP quality's filter measures (Y1a, Y1b, Y2a and Y2b of
//! `docs/dev/design/dsp-quality.md`, step 0.2, for step 2.2): every synth
//! and sampler filter on its own against the curve its page draws, and a
//! ratchet of their own (`tests/filters.rs`, `quality/filters.tsv`).
//!
//! - **The filters:** Poly's four (Wavetable runs the same,
//!   `mpfree_synth::dsp::Filter`), Analog's four, Squelch's and Bass's
//!   ladders, each through its engine's `FilterProbe` (the voice's own
//!   solve, coefficients and make-up, and for Squelch and Bass the inner
//!   rate and half-band), and every kind of the sampler's filter, played
//!   on a pad and divided by the same pad with its filter off. Each is
//!   held to the curve its page draws: `mpfree_synth_ui::visual::
//!   filter_gain`, `analog::filter_gain`, `squelch::response`,
//!   `bass::ladder_response` and `mpfree_sampler_ui::play_widgets::
//!   filter_db`. Resonance is a share of each panel's range.
//! - **Y1a, small signal:** the response to a -40 dBFS impulse, cutoff at
//!   5, 10 and 15 kHz, resonance 0, 0.5 and 0.9: the largest error below
//!   the cutoff where the drawn curve is within 24 dB of 0 dB (target 1
//!   dB), and from 18 to 20 kHz where it is within 30 dB (characterised,
//!   no target, kept by the ratchet).
//! - **Y1b, driven,** for the filters meant to saturate (Analog's,
//!   Squelch's, Bass's and the sampler's Ladder; the others are linear):
//!   sines at a quarter of the cutoff, half and the cutoff at -20, -10 and
//!   -3 dBFS against the same sines at -40 dBFS, how far the gain moves
//!   (reported, not kept: saturating is the filters' sound).
//! - **Y2a, resonance tracking:** at resonance 0.9, the measured
//!   response's peak against the drawn curve's, cutoff 100 Hz to 10 kHz
//!   (target 25 cents).
//! - **Y2b, self-oscillation,** for the filters meant to sing (Analog's
//!   LP 24, Bass's ladder, the sampler's Ladder): one -40 dBFS impulse at
//!   full resonance, then 10 seconds: the pitch over the last 0.74 s
//!   against the cutoff (target 10 cents) and how far the level moves
//!   from the fifth second to the last (target 1 dB).

use std::f64::consts::TAU;

use mpfree_sampler::dsp::{Filter as PadFilter, FilterKind};
use mpfree_synth::dsp::{Filter as SynthFilter, FilterCoefs, FilterMode};
use mpfree_synth::{analog, bass, squelch};

use crate::fft::Fft;
use crate::quality::{Better, Row};
use crate::sampler;
use crate::synth::blackman_harris;

/// The small signal: -40 dBFS.
const SMALL: f64 = 0.01;
/// Y1a's cutoffs (Hz) and resonances (a share of the panel's range).
const CUTOFFS: [f32; 3] = [5_000.0, 10_000.0, 15_000.0];
const RESONANCES: [f32; 3] = [0.0, 0.5, 0.9];
/// Y1b's levels (dBFS peak).
const DRIVEN_DB: [f64; 3] = [-20.0, -10.0, -3.0];
/// Y2a's cutoffs and resonance.
const PEAK_CUTOFFS: [f32; 7] = [100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0];
const PEAK_RESONANCE: f32 = 0.9;
/// Y2b's cutoffs and how long each sings.
const SING_CUTOFFS: [f32; 4] = [110.0, 440.0, 1_760.0, 7_040.0];
const SING_SECS: usize = 10;
/// Y1a's window and Y2a's (frames at the host rate).
const IR_FRAMES: usize = 1 << 16;
const PEAK_FRAMES: usize = 1 << 17;
/// Where the pad's impulse sits in its sound: clear of the envelope's
/// start.
const PAD_LEAD: usize = 4_096;

/// A filter at one setting: `x`, at the filter's own rate (`os` samples a
/// frame), to its output at the host rate, at the input's level.
struct Set {
    os: usize,
    run: Run,
}

type Run = Box<dyn FnMut(&[f64]) -> Vec<f64>>;

type Make = Box<dyn Fn(f32, f32) -> Set>;
type Drawn = Box<dyn Fn(f32, f32, f64) -> f64>;

/// A filter type and how to set it up.
struct Subject {
    name: String,
    /// The filter at a cutoff (Hz) and resonance (0 to 1), at rest.
    make: Make,
    /// The drawn curve's gain (linear) at a cutoff, resonance and Hz.
    drawn: Drawn,
    /// It is meant to saturate (Y1b).
    saturates: bool,
    /// Its cutoff names a resonant peak (Y2a).
    peak: bool,
    /// It is meant to sing at full resonance (Y2b).
    sings: bool,
}

/// Every filter, at `rate`.
fn subjects(rate: u32) -> Vec<Subject> {
    let mut v = Vec::new();
    for (mode, index, label) in [
        (FilterMode::Lp24, 0.0f32, "LP 24"),
        (FilterMode::Lp12, 1.0, "LP 12"),
        (FilterMode::Bp12, 2.0, "BP 12"),
        (FilterMode::Hp12, 3.0, "HP 12"),
    ] {
        v.push(Subject {
            name: format!("Poly {label}"),
            make: Box::new(move |fc, r| {
                let c = FilterCoefs::new(fc, r, rate as f32);
                let mut f = SynthFilter::default();
                Set {
                    os: 1,
                    run: Box::new(move |x| {
                        x.iter()
                            .map(|&s| f64::from(f.process(mode, &c, s as f32)))
                            .collect()
                    }),
                }
            }),
            drawn: Box::new(move |fc, r, hz| {
                f64::from(mpfree_synth_ui::visual::filter_gain(
                    index, fc, r, hz as f32,
                ))
            }),
            saturates: false,
            peak: true,
            sings: false,
        });
    }
    for (index, label) in [
        (0.0f32, "LP 24"),
        (1.0, "LP 12"),
        (2.0, "BP 12"),
        (3.0, "HP 12"),
    ] {
        v.push(Subject {
            name: format!("Analog {label}"),
            make: Box::new(move |fc, r| {
                let mut p = analog::FilterProbe::new(rate);
                p.set(index, fc, 100.0 * r, 0.0);
                Set {
                    os: 1,
                    run: Box::new(move |x| {
                        x.iter().map(|&s| f64::from(p.tick(s as f32))).collect()
                    }),
                }
            }),
            drawn: Box::new(move |fc, r, hz| {
                f64::from(analog::filter_gain(index, fc, 100.0 * r, hz as f32))
            }),
            saturates: true,
            peak: true,
            sings: index == 0.0,
        });
    }
    v.push(Subject {
        name: "Squelch".into(),
        make: Box::new(move |fc, r| {
            let mut p = squelch::FilterProbe::new(rate);
            p.set(fc, 100.0 * r);
            let os = p.oversampling();
            Set {
                os,
                run: Box::new(move |x| frames(x, os, |f| p.frame(f))),
            }
        }),
        drawn: Box::new(|fc, r, hz| f64::from(squelch::response(fc, 100.0 * r, hz as f32))),
        saturates: true,
        peak: true,
        sings: false,
    });
    v.push(Subject {
        name: "Bass".into(),
        make: Box::new(move |fc, r| {
            let mut p = bass::FilterProbe::new(rate);
            p.set(fc, 100.0 * r, 0.0);
            let os = p.oversampling();
            Set {
                os,
                run: Box::new(move |x| frames(x, os, |f| p.frame(f))),
            }
        }),
        drawn: Box::new(|fc, r, hz| f64::from(bass::ladder_response(fc, 100.0 * r, hz as f32))),
        saturates: true,
        peak: true,
        sings: true,
    });
    let (delay, gain) = pad_path(rate);
    for (label, kind, poles) in [
        ("low-pass 2-pole", FilterKind::LowPass, 2u8),
        ("low-pass 4-pole", FilterKind::LowPass, 4),
        ("high-pass 2-pole", FilterKind::HighPass, 2),
        ("high-pass 4-pole", FilterKind::HighPass, 4),
        ("band-pass 2-pole", FilterKind::BandPass, 2),
        ("band-pass 4-pole", FilterKind::BandPass, 4),
        ("notch 2-pole", FilterKind::Notch, 2),
        ("notch 4-pole", FilterKind::Notch, 4),
        ("Ladder", FilterKind::Ladder, 4),
        ("Vowel", FilterKind::Vowel, 2),
        ("Comb", FilterKind::Comb, 2),
    ] {
        let filter = move |fc: f32, r: f32| PadFilter {
            kind,
            poles,
            cutoff_hz: fc,
            resonance: r,
            env_octaves: 0.0,
            drive_db: 0.0,
        };
        v.push(Subject {
            name: format!("sampler {label}"),
            make: Box::new(move |fc, r| {
                let f = filter(fc, r);
                Set {
                    os: 1,
                    run: Box::new(move |x| {
                        // The impulse or tone after a lead, so the pad's
                        // envelope is open; the pad's own gain and delay
                        // taken out.
                        let mut s = vec![0.0; PAD_LEAD];
                        s.extend_from_slice(x);
                        let y = sampler::through_pad(&s, Some(f), rate, s.len() + delay);
                        y[PAD_LEAD + delay..].iter().map(|v| v / gain).collect()
                    }),
                }
            }),
            drawn: Box::new(move |fc, r, hz| {
                let db = mpfree_sampler_ui::play_widgets::filter_db(&filter(fc, r), hz as f32);
                10f64.powf(f64::from(db) / 20.0)
            }),
            saturates: kind == FilterKind::Ladder,
            peak: matches!(
                kind,
                FilterKind::LowPass
                    | FilterKind::HighPass
                    | FilterKind::BandPass
                    | FilterKind::Ladder
            ),
            sings: kind == FilterKind::Ladder,
        });
    }
    v
}

/// `x` a frame of `os` samples at a time through `frame`.
fn frames(x: &[f64], os: usize, mut frame: impl FnMut(&[f32]) -> f32) -> Vec<f64> {
    let mut buf = vec![0.0f32; os];
    x.chunks(os)
        .map(|c| {
            for (b, &s) in buf.iter_mut().zip(c) {
                *b = s as f32;
            }
            f64::from(frame(&buf))
        })
        .collect()
}

/// The pad with its filter off: the frames from a sound's start to where
/// it comes out, and its gain (the read at pitch is a plain copy).
fn pad_path(rate: u32) -> (usize, f64) {
    let mut x = vec![0.0; 2 * PAD_LEAD];
    x[PAD_LEAD] = SMALL;
    let y = sampler::through_pad(&x, None, rate, x.len());
    let at = (0..y.len())
        .max_by(|&a, &b| y[a].abs().total_cmp(&y[b].abs()))
        .unwrap_or(PAD_LEAD);
    (at - PAD_LEAD, y[at] / SMALL)
}

/// The response to a -40 dBFS impulse, `n` frames at the host rate (a
/// power of two), as gain per FFT bin: (re, im).
fn spectrum(s: &mut Set, n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut x = vec![0.0; n * s.os];
    x[0] = SMALL;
    let y = (s.run)(&x);
    // Down from the inner rate, the band below half the host rate comes
    // out at 1/os of its gain.
    let scale = s.os as f64 / SMALL;
    let mut re: Vec<f64> = y.iter().take(n).map(|v| v * scale).collect();
    re.resize(n, 0.0);
    let mut im = vec![0.0; n];
    Fft::new(n).forward(&mut re, &mut im);
    (re, im)
}

fn db20(a: f64) -> f64 {
    20.0 * a.max(1e-30).log10()
}

/// The log grid Y1a reads: 48 points an octave from 20 Hz to 20 kHz, as
/// FFT bins of `n` at `rate` (each once).
fn grid(n: usize, rate: u32) -> Vec<usize> {
    let mut bins: Vec<usize> = (0..)
        .map(|i| 20.0 * 2f64.powf(f64::from(i) / 48.0))
        .take_while(|&f| f <= 20_000.0)
        .map(|f| (f * n as f64 / f64::from(rate)).round() as usize)
        .collect();
    bins.dedup();
    bins
}

/// The gain of `x` at `f` Hz (a windowed single-bin transform).
fn amp_at(x: &[f64], f: f64, rate: u32) -> f64 {
    let w = blackman_harris(x.len());
    let (mut re, mut im) = (0.0, 0.0);
    for (i, (&s, &wi)) in x.iter().zip(&w).enumerate() {
        let ph = TAU * f * i as f64 / f64::from(rate);
        re += s * wi * ph.cos();
        im -= s * wi * ph.sin();
    }
    2.0 * re.hypot(im) / w.iter().sum::<f64>()
}

/// The gain at `f` Hz for a sine of peak `amp`: 0.1 s to settle, 0.2 s
/// read.
fn tone_gain(s: &mut Set, f: f64, amp: f64, rate: u32) -> f64 {
    let host = (0.3 * f64::from(rate)) as usize;
    let fs = f64::from(rate) * s.os as f64;
    let x: Vec<f64> = (0..host * s.os)
        .map(|i| amp * (TAU * f * i as f64 / fs).sin())
        .collect();
    let y = (s.run)(&x);
    let from = host / 3;
    amp_at(&y[from..host.min(y.len())], f, rate) / amp
}

/// The drawn curve's peak between a quarter and four times `fc` (below
/// half of `rate`), if it has one inside.
fn drawn_peak(drawn: &dyn Fn(f64) -> f64, fc: f64, rate: u32) -> Option<f64> {
    let (lo, hi) = (fc / 4.0, (fc * 4.0).min(0.49 * f64::from(rate)));
    let steps = (1200.0 * (hi / lo).log2()).ceil() as usize;
    let at = |i: usize| lo * 2f64.powf(i as f64 / 1200.0);
    let best = (0..=steps).max_by(|&a, &b| drawn(at(a)).total_cmp(&drawn(at(b))))?;
    if best == 0 || best == steps {
        return None;
    }
    let (a, b, c) = (
        db20(drawn(at(best - 1))),
        db20(drawn(at(best))),
        db20(drawn(at(best + 1))),
    );
    Some(at(best) * 2f64.powf(parabola(a, b, c) / 1200.0))
}

/// Where a parabola through three equally spaced points peaks, in steps
/// from the middle one.
fn parabola(a: f64, b: f64, c: f64) -> f64 {
    let d = a - 2.0 * b + c;
    if d.abs() > 1e-12 {
        (0.5 * (a - c) / d).clamp(-0.5, 0.5)
    } else {
        0.0
    }
}

/// The loudest bin of `db` (per bin of `n` at `rate`) between `lo` and
/// `hi` Hz, refined between bins.
fn peak_hz(db: &[f64], n: usize, rate: u32, lo: f64, hi: f64) -> f64 {
    let hz = f64::from(rate) / n as f64;
    let (a, b) = (
        (lo / hz).floor().max(1.0) as usize,
        ((hi / hz).ceil() as usize).min(db.len() - 2),
    );
    let k = (a..=b)
        .max_by(|&x, &y| db[x].total_cmp(&db[y]))
        .unwrap_or(a);
    (k as f64 + parabola(db[k - 1], db[k], db[k + 1])) * hz
}

fn cents(f: f64, to: f64) -> f64 {
    1200.0 * (f / to).log2()
}

#[allow(clippy::too_many_arguments)]
fn put(
    rows: &mut Vec<Row>,
    id: &'static str,
    subject: String,
    metric: &str,
    value: f64,
    better: Better,
    target: Option<f64>,
    tol: f64,
) {
    rows.push(Row {
        target_id: id,
        subject: format!("filters/{subject}"),
        metric: metric.to_string(),
        value,
        better,
        target,
        tol,
    });
}

fn khz(fc: f32) -> String {
    let k = f64::from(fc) / 1000.0;
    if k >= 1.0 {
        format!("{k} kHz")
    } else {
        format!("{fc} Hz")
    }
}

/// Every filter measure at `rate`.
pub fn report(rate: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    for s in subjects(rate) {
        small_signal(&mut rows, &s, rate);
        if s.saturates {
            driven(&mut rows, &s, rate);
        }
        if s.peak {
            tracking(&mut rows, &s, rate);
        }
        if s.sings {
            singing(&mut rows, &s, rate);
        }
    }
    rows
}

/// Y1a.
fn small_signal(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    let hz = f64::from(rate) / IR_FRAMES as f64;
    let bins = grid(IR_FRAMES, rate);
    for fc in CUTOFFS {
        for r in RESONANCES {
            let (re, im) = spectrum(&mut (s.make)(fc, r), IR_FRAMES);
            let (mut below, mut top) = (0.0f64, None::<f64>);
            for &k in &bins {
                let f = k as f64 * hz;
                let want = db20((s.drawn)(fc, r, f));
                let err = (db20(re[k].hypot(im[k])) - want).abs();
                if f <= f64::from(fc) && want >= -24.0 {
                    below = below.max(err);
                }
                if (18_000.0..=20_000.0).contains(&f) && want >= -30.0 {
                    top = Some(top.unwrap_or(0.0).max(err));
                }
            }
            let subject = format!("{}[{}, resonance {r}]", s.name, khz(fc));
            put(
                rows,
                "Y1a",
                subject.clone(),
                "below_cutoff_err_db",
                below,
                Better::Lower,
                Some(1.0),
                0.05,
            );
            if let Some(t) = top {
                put(
                    rows,
                    "Y1a",
                    subject,
                    "err_18_20k_db",
                    t,
                    Better::Lower,
                    None,
                    0.05,
                );
            }
        }
    }
}

/// Y1b.
fn driven(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    for fc in CUTOFFS {
        for r in RESONANCES {
            let freqs = [0.25, 0.5, 1.0].map(|m| m * f64::from(fc));
            let quiet: Vec<f64> = freqs
                .iter()
                .map(|&f| tone_gain(&mut (s.make)(fc, r), f, SMALL, rate))
                .collect();
            for level in DRIVEN_DB {
                let amp = 10f64.powf(level / 20.0);
                let change = freqs
                    .iter()
                    .zip(&quiet)
                    .map(|(&f, &q)| db20(tone_gain(&mut (s.make)(fc, r), f, amp, rate)) - db20(q))
                    .fold(0.0f64, |a, c| if c.abs() > a.abs() { c } else { a });
                put(
                    rows,
                    "Y1b",
                    format!("{}[{}, resonance {r}]", s.name, khz(fc)),
                    &format!("gain_change_at_{}dbfs_db", level as i32),
                    change,
                    Better::Info,
                    None,
                    0.0,
                );
            }
        }
    }
}

/// Y2a.
fn tracking(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    let mut worst = None::<f64>;
    for fc in PEAK_CUTOFFS {
        let drawn = |f: f64| (s.drawn)(fc, PEAK_RESONANCE, f);
        let Some(want) = drawn_peak(&drawn, f64::from(fc), rate) else {
            continue;
        };
        let (re, im) = spectrum(&mut (s.make)(fc, PEAK_RESONANCE), PEAK_FRAMES);
        let db: Vec<f64> = re
            .iter()
            .zip(&im)
            .take(PEAK_FRAMES / 2)
            .map(|(a, b)| db20(a.hypot(*b)))
            .collect();
        let got = peak_hz(&db, PEAK_FRAMES, rate, want / 2.0, want * 2.0);
        let c = cents(got, want);
        put(
            rows,
            "Y2a",
            format!("{}[{}, resonance {PEAK_RESONANCE}]", s.name, khz(fc)),
            "peak_cents",
            c,
            Better::Info,
            None,
            0.0,
        );
        if worst.is_none_or(|w| c.abs() > w.abs()) {
            worst = Some(c);
        }
    }
    if let Some(w) = worst {
        put(
            rows,
            "Y2a",
            format!("{}[100 Hz to 10 kHz, resonance {PEAK_RESONANCE}]", s.name),
            "peak_worst_cents",
            w,
            Better::Near(0.0),
            Some(25.0),
            0.5,
        );
    }
}

/// Y2b.
fn singing(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    const N: usize = 1 << 15;
    let second = rate as usize;
    let (mut worst_c, mut worst_g) = (None::<f64>, None::<f64>);
    for fc in SING_CUTOFFS {
        // Four equal stages turn half a cycle at the cutoff, whatever the
        // feedback, so that is where a ladder sings (the drawn curve's
        // peak sits a little above it once the feedback passes 4).
        let want = f64::from(fc);
        let mut set = (s.make)(fc, 1.0);
        let mut x = vec![0.0; SING_SECS * second * set.os];
        x[0] = SMALL;
        let y = (set.run)(&x);
        let rms = |i: usize| {
            let w = &y[i * second..((i + 1) * second).min(y.len())];
            (w.iter().map(|v| v * v).sum::<f64>() / w.len().max(1) as f64).sqrt()
        };
        let last = rms(SING_SECS - 1);
        let growth = (db20(last) - db20(rms(SING_SECS / 2 - 1))).max(-140.0);
        let c = if last > 1e-6 {
            let tail = &y[y.len() - N..];
            let w = blackman_harris(N);
            let mut re: Vec<f64> = tail.iter().zip(&w).map(|(a, b)| a * b).collect();
            let mut im = vec![0.0; N];
            Fft::new(N).forward(&mut re, &mut im);
            let db: Vec<f64> = re
                .iter()
                .zip(&im)
                .take(N / 2)
                .map(|(a, b)| db20(a.hypot(*b)))
                .collect();
            cents(peak_hz(&db, N, rate, want / 2.0, want * 2.0), want)
        } else {
            // Silent: no pitch to read.
            1200.0
        };
        let subject = format!("{}[{}, resonance 1]", s.name, khz(fc));
        put(
            rows,
            "Y2b",
            subject.clone(),
            "sing_cents",
            c,
            Better::Info,
            None,
            0.0,
        );
        put(
            rows,
            "Y2b",
            subject.clone(),
            "sing_growth_db",
            growth,
            Better::Info,
            None,
            0.0,
        );
        put(
            rows,
            "Y2b",
            subject,
            "sing_level_db",
            db20(last * 2f64.sqrt()),
            Better::Info,
            None,
            0.0,
        );
        if worst_c.is_none_or(|w| c.abs() > w.abs()) {
            worst_c = Some(c);
        }
        if worst_g.is_none_or(|w| growth.abs() > w.abs()) {
            worst_g = Some(growth);
        }
    }
    let subject = format!("{}[110 Hz to 7 kHz, resonance 1]", s.name);
    if let Some(c) = worst_c {
        put(
            rows,
            "Y2b",
            subject.clone(),
            "sing_worst_cents",
            c,
            Better::Near(0.0),
            Some(10.0),
            0.5,
        );
    }
    if let Some(g) = worst_g {
        put(
            rows,
            "Y2b",
            subject,
            "sing_growth_worst_db",
            g,
            Better::Near(0.0),
            Some(1.0),
            0.1,
        );
    }
}

/// The ratchet file's header.
const HEADER: &str =
    "# The filter measures' ratchet (DSP quality Y1a, Y2a, Y2b): every number may stay or get better, never worse.\n\
# Written by `MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test filters`;\n\
# see docs/dev/design/dsp-quality.md. key<TAB>value\n";

/// The ratchet file for `rows`.
pub fn ratchet_text(rows: &[Row]) -> String {
    let body = sampler::ratchet_text(rows);
    // The same lines under this file's own header.
    let lines: String = body
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();
    format!("{HEADER}{lines}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pad_path_is_a_plain_copy() {
        let (delay, gain) = pad_path(44_100);
        assert!(gain > 0.1 && gain <= 1.0, "gain {gain}");
        assert!(delay < 256, "delay {delay}");
    }

    #[test]
    fn a_wire_reads_flat_and_its_peak_where_drawn() {
        // A plain gain: 0 dB at every bin.
        let mut wire = Set {
            os: 1,
            run: Box::new(|x| x.to_vec()),
        };
        let (re, im) = spectrum(&mut wire, 1 << 12);
        assert!(re
            .iter()
            .zip(&im)
            .all(|(a, b)| (a.hypot(*b) - 1.0).abs() < 1e-12));
        // The drawn peak of a resonant two-pole sits where the formula says.
        let d = 0.2f64;
        let lp = |f: f64| {
            let w = f / 1000.0;
            1.0 / ((1.0 - w * w).powi(2) + (d * w).powi(2)).sqrt()
        };
        let want = 1000.0 * (1.0 - d * d / 2.0).sqrt();
        let got = drawn_peak(&lp, 1000.0, 44_100).expect("a peak");
        assert!(cents(got, want).abs() < 0.5, "{got} against {want}");
    }
}
