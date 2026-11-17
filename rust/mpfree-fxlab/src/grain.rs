//! The grain engine in the lab (sampler roadmap R7, step A1): numbers for
//! how `mpfree-grain` sounds, measured on a [`GrainVoice`] reading test
//! sounds, with a ratchet of their own (`tests/grain.rs`,
//! `quality/grain.tsv`) that keeps every one from getting worse.
//!
//! Every number is the voice alone (no amp envelope, filter or effects
//! after it), at 44.1 kHz in 128-frame blocks, with fixed seeds. The
//! sources are pink noise at -20 dBFS RMS, a "pad" (an A minor chord of
//! slightly detuned band-limited saws, -20 dBFS RMS), a 1 kHz sine and a
//! steady level (DC), which shows the sum of the grains' windows. Unless
//! a row says otherwise the grains read from 30 % into the source, with
//! the voice's defaults (80 ms grains, 25 a second, Hann, timing 30 %
//! random, no spray, no scan).
//!
//! - **G1, the level holds:** loudness (LU, [`loudness::integrated`])
//!   against the source's at 40, 80 and 400 ms grains overlapping 2, 4
//!   and 16 times, sprayed, scanning, and scanning with spray: the range
//!   over all of them, and the defaults' offset.
//! - **G2, coherent clouds:** a granular time stretch (scan 1, no spray)
//!   against the same cloud sprayed, at overlaps 16 and 64.
//! - **G3, smooth:** the DC level through the grains, whose output is
//!   the window sum through the level law: its ripple (largest over
//!   smallest) and its modulation (RMS of the change against the mean),
//!   at the defaults and with each window at overlap 2; and, for a 1 kHz
//!   sine at the defaults, the first grain-rate sideband (from a long
//!   transform) and how much its level flutters (shown, not kept).
//! - **G4, no comb:** a frozen cloud (no scan, no spray, 80 ms grains) of
//!   the pad and of the sine: how far its level swings as density moves
//!   from 15 to 60 a second, one at a time.
//! - **G5, clean pitching:** (read as a pad reads, through
//!   `mpfree_sampler::grain_clip::GrainClip`) a sine pitched by the grains (200 ms, 20 a
//!   second, spray 0.3): the worst spur from 20 Hz to 20 kHz (power within
//!   60 Hz of the loudest bin at least 200 Hz from the wanted tone, and
//!   never within 140 Hz of it) against the same
//!   tone through the same cloud at pitch.
//! - **G6, no silent thinning:** clouds that want more grains than a
//!   voice holds (2 s grains at 50 and 80 a second) against the same
//!   density with grains that fit (400 ms), and the share of grains
//!   skipped.
//! - **G7, width:** full spread against none on pink: the level change,
//!   the left/right correlation and what folding to mono loses. (The
//!   correlation's target is 0.45, not the plan's 0.3: folding to mono
//!   keeps `(1 + ρ) / 2` of the power, so 1.5 dB needs ρ ≥ 0.41.)
//! - **G8, onset:** from the start to the first 10 ms whose RMS is within
//!   3 dB of the steady level, on the pad, with 200 ms to 1 s grains.
//!
//! Cost is the bench's (`mpfree-bench`'s "Sampler grains (per grain)"
//! scaling case, on CI by the instruction gate and on the MPC in the
//! device round). Design: `docs/dev/design/fx-quality.md` ("The grain
//! engine"); plan: mpfree-claude `sampler/granular/plan.md`.

use mpfree_grain::{Budget, GrainVoice, Params, Stats, Window};
use mpfree_sampler::bandlimit::Octaves;
use mpfree_sampler::grain_clip::GrainClip;

use crate::fft::Fft;
use crate::loudness;
use crate::quality::{Better, Row};
use crate::signals::{pink, sine};
use crate::wav::Stereo;

/// The ratchet file's header.
const HEADER: &str =
    "# The grain engine's quality ratchet: every number may stay or get better, never worse.\n\
# Written by `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test grain`;\n\
# see docs/dev/design/fx-quality.md (\"The grain engine\"). key<TAB>value\n";

/// The sources' length: long enough for the longest render's reads.
const SOURCE_SECS: f64 = 6.0;
/// Where grains read unless a row says otherwise.
const POSITION: f32 = 0.3;

/// Every number, in the order measured.
pub fn report(rate: u32) -> Vec<Row> {
    let lab = Lab::new(rate);
    let mut rows = Vec::new();
    levels(&mut rows, &lab);
    coherent(&mut rows, &lab);
    smooth(&mut rows, &lab);
    comb(&mut rows, &lab);
    pitched(&mut rows, &lab);
    thinning(&mut rows, &lab);
    width(&mut rows, &lab);
    onsets(&mut rows, &lab);
    rows
}

/// The ratchet file for `rows`.
pub fn ratchet_text(rows: &[Row]) -> String {
    let mut lines: Vec<String> = rows
        .iter()
        .filter(|r| r.better != Better::Info)
        .map(|r| format!("{}\t{}", r.key(), num(r.value)))
        .collect();
    lines.sort();
    let mut s = String::from(HEADER);
    for l in lines {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
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
        subject: format!("grain/{subject}"),
        metric: metric.to_string(),
        value,
        better,
        target,
        tol,
    });
}

// ---------------------------------------------------------------------------
// Sources and rendering.

struct Lab {
    rate: u32,
    pink: Vec<f32>,
    pad: Vec<f32>,
    tone: Vec<f32>,
    dc: Vec<f32>,
}

impl Lab {
    fn new(rate: u32) -> Lab {
        let n = (SOURCE_SECS * f64::from(rate)) as usize;
        let f = |v: Vec<f64>| v.into_iter().map(|x| x as f32).collect::<Vec<f32>>();
        Lab {
            rate,
            pink: f(pink(n, 0.1, 3)),
            pad: pad_sound(n, rate),
            tone: f(sine(1000.0, 0.25, n, rate)),
            dc: vec![0.5; n],
        }
    }
}

/// The pad source: an A minor chord of detuned band-limited saws.
fn pad_sound(n: usize, rate: u32) -> Vec<f32> {
    {
        let notes = [220.0, 261.63, 329.63, 440.0];
        let mut v = vec![0.0f64; n];
        for (k, f0) in notes.iter().enumerate() {
            for det in [-0.003, 0.003] {
                let f = f0 * (1.0 + det);
                let harmonics = ((19_000.0 / f) as usize).min(40);
                for h in 1..=harmonics {
                    // A rotating phasor instead of a sine a frame.
                    let w = std::f64::consts::TAU * f * h as f64 / f64::from(rate);
                    let (c, s) = (w.cos(), w.sin());
                    let ph = (k * 7 + h * 3) as f64;
                    let (mut re, mut im) = (ph.cos(), ph.sin());
                    let a = 1.0 / h as f64;
                    for x in v.iter_mut() {
                        *x += a * im;
                        let r = re * c - im * s;
                        im = re * s + im * c;
                        re = r;
                    }
                }
            }
        }
        let rms = (v.iter().map(|x| x * x).sum::<f64>() / n as f64).sqrt();
        v.into_iter().map(|x| (x * 0.1 / rms) as f32).collect()
    }
}

/// `secs` of `src` (mono) through a voice with `p` from its start, and
/// the voice's counters. The voice reads `src` as a pad does: grains
/// pitched more than two semitones up read band-limited, from the
/// sound's octave copies past an octave (made here only for those).
fn render(src: &[f32], rate: u32, p: Params, secs: f64, seed: u64) -> (Stereo, Stats) {
    let up = p.pitch.max(0.0) + p.pitch_spray.max(0.0) > 2.0;
    let octaves = up.then(|| Octaves::make(src, 1));
    let clip = GrainClip::new(src, 1, rate)
        .expect("a lab clip")
        .clean(octaves.as_ref());
    let mut v = GrainVoice::new(rate, seed);
    v.set_params(p);
    v.start();
    let mut budget = Budget::new(256);
    let n = (secs * f64::from(rate)) as usize;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    for s in (0..n).step_by(128) {
        let e = (s + 128).min(n);
        v.render(&clip, &mut budget, s as u64, &mut l[s..e], &mut r[s..e]);
    }
    let f = |x: Vec<f32>| x.into_iter().map(f64::from).collect::<Vec<f64>>();
    (Stereo::new(f(l), f(r), rate), v.stats())
}

/// The steady part: from `from` seconds to the end.
fn after(s: &Stereo, from: f64) -> Stereo {
    let i = ((from * f64::from(s.rate)) as usize).min(s.left.len());
    Stereo::new(s.left[i..].to_vec(), s.right[i..].to_vec(), s.rate)
}

/// A steady cloud: rendered long enough for its grains to fill in, then
/// `secs` of it.
fn steady(src: &[f32], rate: u32, p: Params, secs: f64, seed: u64) -> (Stereo, Stats) {
    let warm = f64::from(p.size_ms) / 1000.0 + 0.25;
    let (s, st) = render(src, rate, p, warm + secs, seed);
    (after(&s, warm), st)
}

fn lufs(s: &Stereo) -> f64 {
    loudness::integrated(s)
}

fn source_lufs(src: &[f32], rate: u32) -> f64 {
    lufs(&Stereo::mono(
        src.iter().map(|&x| f64::from(x)).collect(),
        rate,
    ))
}

fn rms(x: &[f64]) -> f64 {
    (x.iter().map(|v| v * v).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn db(a: f64) -> f64 {
    20.0 * a.max(1e-12).log10()
}

fn base() -> Params {
    Params {
        position: POSITION,
        ..Params::default()
    }
}

// ---------------------------------------------------------------------------
// G1 and G2: levels.

fn levels(rows: &mut Vec<Row>, lab: &Lab) {
    for (name, src) in [("pink", &lab.pink), ("pad", &lab.pad)] {
        let reference = source_lufs(src, lab.rate);
        let mut seen: Vec<(String, f64)> = Vec::new();
        for (kind, spray, scan) in [
            ("sprayed", 0.3f32, 0.0f32),
            ("stretch", 0.0, 1.0),
            ("stretch sprayed", 0.3, 1.0),
        ] {
            for size in [40.0f32, 80.0, 400.0] {
                for overlap in [2.0f32, 4.0, 16.0] {
                    let p = Params {
                        size_ms: size,
                        density: overlap * 1000.0 / size,
                        spray,
                        scan,
                        ..base()
                    };
                    let (s, _) = steady(src, lab.rate, p, 1.5, 7);
                    seen.push((
                        format!("{kind}, {size} ms, overlap {overlap}"),
                        lufs(&s) - reference,
                    ));
                }
            }
        }
        let lo = seen
            .iter()
            .fold(&seen[0], |a, b| if b.1 < a.1 { b } else { a });
        let hi = seen
            .iter()
            .fold(&seen[0], |a, b| if b.1 > a.1 { b } else { a });
        let subject = format!("level[{name}]");
        put(
            rows,
            "G1",
            subject.clone(),
            "range_lu",
            hi.1 - lo.1,
            Better::Lower,
            Some(3.0),
            0.1,
        );
        put(
            rows,
            "G1",
            format!("level[{name}, quietest: {}]", lo.0),
            "lu",
            lo.1,
            Better::Info,
            None,
            0.0,
        );
        put(
            rows,
            "G1",
            format!("level[{name}, loudest: {}]", hi.0),
            "lu",
            hi.1,
            Better::Info,
            None,
            0.0,
        );
        let (s, _) = steady(src, lab.rate, base(), 1.5, 7);
        put(
            rows,
            "G1",
            format!("level[{name}, defaults]"),
            "offset_lu",
            lufs(&s) - reference,
            Better::Near(0.0),
            Some(1.0),
            0.1,
        );
    }
}

fn coherent(rows: &mut Vec<Row>, lab: &Lab) {
    for (size, overlap) in [(200.0f32, 16.0f32), (400.0, 64.0)] {
        let p = Params {
            size_ms: size,
            density: overlap * 1000.0 / size,
            ..base()
        };
        let (stretch, _) = steady(&lab.pink, lab.rate, Params { scan: 1.0, ..p }, 1.5, 2);
        let (sprayed, _) = steady(&lab.pink, lab.rate, Params { spray: 0.3, ..p }, 1.5, 2);
        put(
            rows,
            "G2",
            format!("coherent[pink, overlap {overlap}]"),
            "stretch_over_sprayed_lu",
            lufs(&stretch) - lufs(&sprayed),
            Better::Near(0.0),
            Some(1.5),
            0.1,
        );
    }
}

// ---------------------------------------------------------------------------
// G3: the window sum.

fn smooth(rows: &mut Vec<Row>, lab: &Lab) {
    let mut cases = vec![("defaults".to_string(), base(), true)];
    for (w, name, kept) in [
        (Window::Hann, "Hann", true),
        (Window::Tukey { taper: 0.5 }, "Tukey 0.5", true),
        (Window::Trapezoid { ramp: 0.1 }, "trapezoid 0.1", true),
        // Percussive on purpose: shown, not held to the target.
        (Window::Exponential { decay: 5.0 }, "exponential 5", false),
    ] {
        cases.push((
            format!("{name}, overlap 2"),
            Params {
                window: w,
                density: 25.0,
                ..base()
            },
            kept,
        ));
    }
    for (name, p, kept) in cases {
        let (s, _) = steady(&lab.dc, lab.rate, p, 1.0, 1);
        let x = &s.left;
        let mean = x.iter().sum::<f64>() / x.len() as f64;
        let (lo, hi) = x
            .iter()
            .fold((f64::MAX, f64::MIN), |a, &v| (a.0.min(v), a.1.max(v)));
        let ripple = if lo <= 1e-6 { 120.0 } else { db(hi / lo) };
        let dev: Vec<f64> = x.iter().map(|v| v - mean).collect();
        let am = db(rms(&dev) / mean.abs().max(1e-12)).max(-140.0);
        let (better, target) = if kept {
            (Better::Lower, Some(0.5))
        } else {
            (Better::Info, None)
        };
        put(
            rows,
            "G3",
            format!("smooth[{name}]"),
            "ripple_db",
            ripple,
            better,
            target,
            0.05,
        );
        let (better, target) = if kept {
            (Better::Lower, Some(-40.0))
        } else {
            (Better::Info, None)
        };
        put(
            rows,
            "G3",
            format!("smooth[{name}]"),
            "modulation_db",
            am,
            better,
            target,
            0.5,
        );
    }
    // The first sideband of a held tone at the defaults, from a long
    // transform: its bins (0.7 Hz) keep the tone's own skirt (the window's
    // main lobe, ±2.7 Hz) out of the sideband's band. A1 and A2 measured
    // it with 5.4 Hz bins, whose main lobe (±21.5 Hz) reached the band.
    let (s, _) = steady(&lab.tone, lab.rate, base(), 6.0, 5);
    let sp = welch(&s.left, N_LONG);
    let tone = band(&sp, lab.rate, 1000.0, 3.0);
    let d = f64::from(base().density);
    let side = band(&sp, lab.rate, 1000.0 + d, 3.0).max(band(&sp, lab.rate, 1000.0 - d, 3.0));
    put(
        rows,
        "G3",
        "smooth[defaults, 1 kHz]".into(),
        "sideband_db",
        (10.0 * (side / tone).max(1e-14).log10()).max(-140.0),
        Better::Lower,
        Some(-40.0),
        0.5,
    );
    // How much its level flutters (10 ms RMS: deviation over mean). Grains
    // of one spot at random times add with random phases, which a held
    // sine shows most.
    let e: Vec<f64> = s
        .left
        .chunks((f64::from(lab.rate) / 100.0) as usize)
        .map(|c| (c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64).sqrt())
        .collect();
    let mean = e.iter().sum::<f64>() / e.len() as f64;
    let dev = (e.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / e.len() as f64).sqrt();
    put(
        rows,
        "G3",
        "smooth[defaults, 1 kHz]".into(),
        "flutter_db",
        (20.0 * (dev / mean.max(1e-12)).max(1e-7).log10()).max(-140.0),
        Better::Info,
        None,
        0.5,
    );
}

// ---------------------------------------------------------------------------
// G4: a frozen cloud's comb.

fn comb(rows: &mut Vec<Row>, lab: &Lab) {
    for (name, src, target) in [("pad", &lab.pad, 3.0), ("1 kHz", &lab.tone, 6.0)] {
        let mut levels = Vec::new();
        for d in 15..=60 {
            let p = Params {
                density: d as f32,
                ..base()
            };
            let (s, _) = steady(src, lab.rate, p, 1.0, 1);
            levels.push(db(rms(&s.left)));
        }
        let lo = levels.iter().cloned().fold(f64::MAX, f64::min);
        let hi = levels.iter().cloned().fold(f64::MIN, f64::max);
        put(
            rows,
            "G4",
            format!("frozen[{name}, density 15 to 60]"),
            "swing_db",
            hi - lo,
            Better::Lower,
            Some(target),
            0.1,
        );
    }
}

// ---------------------------------------------------------------------------
// G5: pitched reads.

/// The analysis length (5.4 Hz bins at 44.1 kHz).
const N: usize = 8192;
/// The long transform for a sideband close to its tone.
const N_LONG: usize = 65536;
/// How far from the wanted tone a spur is looked for (Hz): clear of its
/// skirt, which the grains' windows widen.
const CLEAR: f64 = 200.0;

/// Averaged power spectrum (Welch, Blackman-Harris, half overlap).
fn welch(x: &[f64], n: usize) -> Vec<f64> {
    let fft = Fft::new(n);
    let w: Vec<f64> = (0..n)
        .map(|i| {
            let p = std::f64::consts::TAU * i as f64 / n as f64;
            0.35875 - 0.48829 * p.cos() + 0.14128 * (2.0 * p).cos() - 0.01168 * (3.0 * p).cos()
        })
        .collect();
    let mut acc = vec![0.0; n / 2];
    let mut count = 0;
    let mut at = 0;
    while at + n <= x.len() {
        let mut re: Vec<f64> = (0..n).map(|i| x[at + i] * w[i]).collect();
        let mut im = vec![0.0; n];
        fft.forward(&mut re, &mut im);
        for (k, a) in acc.iter_mut().enumerate() {
            *a += re[k] * re[k] + im[k] * im[k];
        }
        count += 1;
        at += n / 2;
    }
    acc.iter_mut().for_each(|a| *a /= count.max(1) as f64);
    acc
}

/// Power within `bw` Hz of `hz`.
fn band(sp: &[f64], rate: u32, hz: f64, bw: f64) -> f64 {
    let bin = f64::from(rate) / (2 * sp.len()) as f64;
    let lo = ((hz - bw) / bin).floor().max(0.0) as usize;
    let hi = (((hz + bw) / bin).ceil() as usize).min(sp.len() - 1);
    sp[lo.min(hi)..=hi].iter().sum()
}

fn pitched(rows: &mut Vec<Row>, lab: &Lab) {
    let n = (SOURCE_SECS * f64::from(lab.rate)) as usize;
    let cloud = Params {
        size_ms: 200.0,
        density: 20.0,
        spray: 0.3,
        ..base()
    };
    for (hz, semis, name) in [
        (15_000.0f64, 12.0f32, "15 kHz an octave up"),
        (9_000.0, 1.0, "9 kHz a semitone up"),
        (9_000.0, 7.0, "9 kHz a fifth up"),
        (13_500.0, -7.0, "13.5 kHz a fifth down"),
        (2_000.0, 36.0, "2 kHz three octaves up"),
        (6_000.0, 24.0, "6 kHz two octaves up"),
    ] {
        let src: Vec<f32> = sine(hz, 0.25, n, lab.rate)
            .into_iter()
            .map(|x| x as f32)
            .collect();
        let (at_pitch, _) = steady(&src, lab.rate, cloud, 2.5, 3);
        let reference = band(&welch(&at_pitch.left, N), lab.rate, hz, 60.0);
        let (s, _) = steady(
            &src,
            lab.rate,
            Params {
                pitch: semis,
                ..cloud
            },
            2.5,
            3,
        );
        let sp = welch(&s.left, N);
        let want = hz * f64::from(semis / 12.0).exp2();
        let bin = f64::from(lab.rate) / N as f64;
        let mut worst = (0.0, 0.0);
        for (k, &p) in sp.iter().enumerate().skip((20.0 / bin) as usize) {
            let f = k as f64 * bin;
            if f > 20_000.0 {
                break;
            }
            if want < 20_000.0 && (f - want).abs() < CLEAR {
                continue;
            }
            if p > worst.1 {
                worst = (f, p);
            }
        }
        // The spur's power, leaving out the wanted tone's own skirt.
        let spur: f64 = sp
            .iter()
            .enumerate()
            .filter(|(k, _)| {
                let f = *k as f64 * bin;
                (f - worst.0).abs() <= 60.0
                    && (want >= 20_000.0 || (f - want).abs() >= CLEAR - 60.0)
            })
            .map(|(_, p)| p)
            .sum();
        let metric = if want > 20_000.0 {
            "heard_db"
        } else {
            "spur_db"
        };
        put(
            rows,
            "G5",
            format!("pitch[{name}]"),
            metric,
            (10.0 * (spur / reference).max(1e-14).log10()).max(-140.0),
            Better::Lower,
            Some(-60.0),
            0.5,
        );
        put(
            rows,
            "G5",
            format!("pitch[{name}]"),
            "spur_hz",
            worst.0,
            Better::Info,
            None,
            0.0,
        );
    }
}

// ---------------------------------------------------------------------------
// G6: the cap.

fn thinning(rows: &mut Vec<Row>, lab: &Lab) {
    for density in [50.0f32, 80.0] {
        let p = Params {
            density,
            spray: 0.3,
            ..base()
        };
        let (fits, _) = steady(
            &lab.pink,
            lab.rate,
            Params {
                size_ms: 400.0,
                ..p
            },
            1.0,
            4,
        );
        let (capped, st) = steady(
            &lab.pink,
            lab.rate,
            Params {
                size_ms: 2000.0,
                ..p
            },
            1.0,
            4,
        );
        let subject = format!("cap[2 s grains, {density} a second]");
        put(
            rows,
            "G6",
            subject.clone(),
            "level_change_lu",
            lufs(&capped) - lufs(&fits),
            Better::Near(0.0),
            Some(1.0),
            0.1,
        );
        put(
            rows,
            "G6",
            subject,
            "skipped_pct",
            100.0 * st.skipped as f64 / (st.spawned + st.skipped).max(1) as f64,
            Better::Info,
            None,
            0.0,
        );
    }
}

// ---------------------------------------------------------------------------
// G7: width.

fn width(rows: &mut Vec<Row>, lab: &Lab) {
    let p = Params {
        density: 40.0,
        spray: 0.3,
        ..base()
    };
    let (none, _) = steady(&lab.pink, lab.rate, p, 2.0, 6);
    let (full, _) = steady(&lab.pink, lab.rate, Params { spread: 1.0, ..p }, 2.0, 6);
    put(
        rows,
        "G7",
        "spread[full]".into(),
        "level_change_lu",
        lufs(&full) - lufs(&none),
        Better::Near(0.0),
        Some(0.5),
        0.1,
    );
    let (l, r) = (&full.left, &full.right);
    let lr: f64 = l.iter().zip(r).map(|(a, b)| a * b).sum();
    let corr =
        lr / (l.iter().map(|a| a * a).sum::<f64>() * r.iter().map(|b| b * b).sum::<f64>()).sqrt();
    put(
        rows,
        "G7",
        "spread[full]".into(),
        "correlation",
        corr,
        Better::Lower,
        Some(0.45),
        0.02,
    );
    let mono: Vec<f64> = l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect();
    put(
        rows,
        "G7",
        "spread[full]".into(),
        "mono_fold_lu",
        (lufs(&Stereo::mono(mono, full.rate)) - lufs(&full)).abs(),
        Better::Lower,
        Some(1.5),
        0.1,
    );
}

// ---------------------------------------------------------------------------
// G8: onsets.

fn onsets(rows: &mut Vec<Row>, lab: &Lab) {
    for (size, density) in [(200.0f32, 20.0f32), (500.0, 10.0), (1000.0, 10.0)] {
        let p = Params {
            size_ms: size,
            density,
            spray: 0.3,
            ..base()
        };
        let secs = f64::from(size) / 1000.0 * 2.0 + 1.0;
        let (s, _) = render(&lab.pad, lab.rate, p, secs, 8);
        let both = |a: usize, b: usize| {
            let l = rms(&s.left[a..b]);
            let r = rms(&s.right[a..b]);
            ((l * l + r * r) / 2.0).sqrt()
        };
        let from = (f64::from(size) / 1000.0 * 1.5 * f64::from(lab.rate)) as usize;
        let level = both(from, s.left.len());
        let w = (lab.rate / 100) as usize;
        let mut ms = secs * 1000.0;
        let mut at = 0;
        while at + w <= s.left.len() {
            if both(at, at + w) >= level * 0.708 {
                ms = (at + w) as f64 * 1000.0 / f64::from(lab.rate);
                break;
            }
            at += w / 4;
        }
        put(
            rows,
            "G8",
            format!("onset[pad, {size} ms grains]"),
            "ms",
            ms,
            Better::Lower,
            Some(15.0),
            1.0,
        );
    }
}
