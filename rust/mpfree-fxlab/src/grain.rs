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
//! - **G11, a tone's pitch (step B5):** short regular grains of one spot
//!   of the pad, four cycles long, at A2, A4 and B5: the note heard (the
//!   spectrum's peak near it, from a long transform) against the note
//!   asked for, in cents.
//! - **G1 again, variation (step B1):** full size, level and peaked
//!   spray, each against the same cloud without it, on pink and the pad:
//!   the cloud keeps its loudness.
//! - **G12, a stretch keeps its hits (step B5):** "drums" (noise bursts
//!   from silence every quarter second, each decaying 40 ms) stretched as
//!   the Stretch mode sets it (90 ms grains, 45 a second, regular, from
//!   the start) to half speed and to double, with the bursts' starts as
//!   its hits: around each hit's time, the loudest millisecond in the
//!   30 ms before it against the hit's (a slowed stretch plays each hit
//!   early, from every grain that reads ahead), how long after it the
//!   level first reaches half the hit's (a sped-up stretch's grains
//!   reach it late, one by one), and how far the level rises again in
//!   the 60 ms after the hit's loudest millisecond (a second attack: a
//!   flam, over 4 ms a millisecond at a time); the medians over the hits.
//!   Shown: the stretch's loudness against the drums' (louder sped up,
//!   with more whole hits a second) and the same without hits.
//! - **G13, movement keeps the level (step B3):** a sprayed cloud (80 ms
//!   grains, 80 a second, spray 0.3) on pink and the pad, with its LFO
//!   (a sine, 0.5 Hz) on position (full depth), size, density and pitch
//!   (half depth: 2.8 times, 4 times, 6 semitones either way) and spray
//!   (from 0.5, full depth), or its envelope on size (half a second up,
//!   a second down to half): the loudness against the same cloud
//!   unmoved, and how much further its level swings (over 400 ms, every
//!   100 ms, as a momentary meter moves).
//! - **G14, the presets keep the level (step B6):** each grain preset
//!   (`mpfree_sampler::grain_presets`) on pink and the pad at the same
//!   position: the loudness against the default cloud's there. A
//!   stutter's grains fill half of each sixteenth, with their fades, so
//!   it sits about 4 LU down.
//! - **G15, feedback can't run away (step C1):** the Grains effect
//!   (`GrainFx`, 2 s of memory, fully wet, reading half way back) on 4 s
//!   of pink at -14 dBFS RMS (peaks near full scale), then 16 s of
//!   silence, at full feedback: its
//!   sample peak, and how far its level rises from seconds 8-12 to 16-20
//!   (none, if the loop holds). Half feedback: how fast the trail falls,
//!   dB a second over seconds 6 to 16, shown.
//! - **G16, repeat keeps the level and joins smoothly (step C2):** the
//!   Grains effect fully dry on pink at 120 BPM, repeating a quarter note
//!   and a sixteenth from 2 s: the level from half a second on against the
//!   stretch of pink it repeats; and on a 110 Hz tone at -20 dBFS, the
//!   biggest step from one sample to the next within 10 ms of each join
//!   (where one repeat fades into the next) against the tone's own (a
//!   cut with no fade jumps up to 140 times as far).
//! - **G17, a filter on each grain (step C4):** the default cloud on pink
//!   through each grain's low pass and high pass at 1 kHz (no
//!   resonance): the level two octaves past the cutoff (4 kHz and 250 Hz,
//!   ±1/6 octave) and two octaves inside it, against the same cloud
//!   unfiltered. A band pass at 1 kHz at full resonance (Q 12) with full
//!   spray (each grain's centre up to three octaves either way, even in
//!   octaves), on a cloud reading all over the pink (spray 0.5): how far
//!   apart its levels at 250 Hz, 1 kHz and 4 kHz (an octave each) are
//!   against the cloud unfiltered (an even spread colours each octave
//!   alike). A low pass at 1 kHz at full resonance: its sample peak
//!   against the unfiltered cloud's (resonance that can't run away).
//!   Shown: the sprayed band's loudness against its cloud's.
//!
//! Cost is the bench's (`mpfree-bench`'s "Sampler grains (per grain)"
//! scaling case, on CI by the instruction gate and on the MPC in the
//! device round). Design: `docs/dev/design/fx-quality.md` ("The grain
//! engine"); plan: mpfree-claude `sampler/granular/plan.md`.

use mpfree_grain::{
    Budget, GrainEnv, GrainFx, GrainLfo, GrainVoice, LfoShape, MotionTarget, Params, SprayShape,
    Stats, Window,
};
use mpfree_sampler::bandlimit::Octaves;
use mpfree_sampler::grain_clip::GrainClip;
use mpfree_sampler::grain_presets;
use mpfree_sampler::model::GrainMode;

use crate::fft::Fft;
use crate::loudness;
use crate::quality::{Better, Row};
use crate::signals::{pink, sine, white};
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
    variation(&mut rows, &lab);
    tone(&mut rows, &lab);
    hits(&mut rows, &lab);
    movement(&mut rows, &lab);
    presets(&mut rows, &lab);
    feedback(&mut rows, &lab);
    repeat(&mut rows, &lab);
    filtered(&mut rows, &lab);
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
    // As high as the movement takes the pitch, too.
    let moved = |t: MotionTarget, depth: f32| {
        if t == MotionTarget::Pitch {
            12.0 * depth.abs()
        } else {
            0.0
        }
    };
    let up = p.pitch.max(0.0)
        + p.pitch_spray.max(0.0)
        + moved(p.lfo.target, p.lfo.depth)
        + moved(p.env.target, p.env.depth)
        > 2.0;
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
        // The Shape knob (step B2) at its square, triangle, swell and pluck.
        (Window::Shape { shape: 0.0 }, "shape 0 %, square", true),
        (Window::Shape { shape: 0.4 }, "shape 40 %, triangle", true),
        (Window::Shape { shape: 0.8 }, "shape 80 %, swell", true),
        (Window::Shape { shape: 1.0 }, "shape 100 %, pluck", true),
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

// ---------------------------------------------------------------------------
// G1 again: variation (step B1).

fn variation(rows: &mut Vec<Row>, lab: &Lab) {
    let p = Params {
        density: 40.0,
        spray: 0.3,
        ..base()
    };
    for (name, src) in [("pink", &lab.pink), ("pad", &lab.pad)] {
        let (plain, _) = steady(src, lab.rate, p, 2.0, 8);
        let plain = lufs(&plain);
        for (what, q) in [
            (
                "size spray full",
                Params {
                    size_spray: 1.0,
                    ..p
                },
            ),
            (
                "level spray full",
                Params {
                    level_spray: 1.0,
                    ..p
                },
            ),
            (
                "peaked spray",
                Params {
                    spray_shape: SprayShape::Peaked,
                    ..p
                },
            ),
        ] {
            let (varied, _) = steady(src, lab.rate, q, 2.0, 8);
            put(
                rows,
                "G1",
                format!("vary[{name}, {what}]"),
                "level_change_lu",
                lufs(&varied) - plain,
                Better::Near(0.0),
                Some(1.0),
                0.1,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// G11: a tone's pitch (step B5).

fn tone(rows: &mut Vec<Row>, lab: &Lab) {
    for (name, hz) in [("A2", 110.0f64), ("A4", 440.0), ("B5", 987.767)] {
        let p = Params {
            density: hz as f32,
            size_ms: (4000.0 / hz) as f32,
            timing: 0.0,
            ..base()
        };
        let (s, _) = steady(&lab.pad, lab.rate, p, 2.0, 11);
        let sp = welch(&s.left, N_LONG);
        let bin = f64::from(lab.rate) / N_LONG as f64;
        // The loudest bin within 60 cents of the note, its place refined
        // by a parabola through its neighbours' log powers.
        let lo = (hz * (-0.05f64).exp2() / bin).floor() as usize;
        let hi = (hz * 0.05f64.exp2() / bin).ceil() as usize;
        let k = (lo..=hi)
            .max_by(|&a, &b| sp[a].total_cmp(&sp[b]))
            .unwrap_or(lo);
        let (a, b, c) = (
            sp[k - 1].max(1e-30).ln(),
            sp[k].max(1e-30).ln(),
            sp[k + 1].max(1e-30).ln(),
        );
        let d = 0.5 * (a - c) / (a - 2.0 * b + c);
        let heard = (k as f64 + if d.is_finite() { d } else { 0.0 }) * bin;
        put(
            rows,
            "G11",
            format!("tone[{name}]"),
            "pitch_cents",
            1200.0 * (heard / hz).log2(),
            Better::Near(0.0),
            Some(5.0),
            0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// G12: a stretch keeps its hits (step B5).

/// Bursts of white noise from silence every `every` seconds, each
/// decaying with a 40 ms time constant from 0.5: (the sound, each burst's
/// start).
fn drums(secs: f64, every: f64, rate: u32) -> (Vec<f32>, Vec<u64>) {
    let n = (secs * f64::from(rate)) as usize;
    let gap = (every * f64::from(rate)) as usize;
    let noise = white(n, 0.5, 12);
    let mut out = vec![0.0f32; n];
    let mut starts = Vec::new();
    for (i, x) in out.iter_mut().enumerate() {
        let k = i % gap;
        if k == 0 {
            starts.push(i as u64);
        }
        *x = (noise[i] * (-(k as f64) / (0.04 * f64::from(rate))).exp()) as f32;
    }
    (out, starts)
}

fn hits(rows: &mut Vec<Row>, lab: &Lab) {
    let rate = lab.rate;
    let (src, starts) = drums(4.0, 0.25, rate);
    let src_lufs = source_lufs(&src, rate);
    for (name, scan, secs) in [("half speed", 0.5f32, 3.5f64), ("double speed", 2.0, 1.9)] {
        let p = Params {
            position: 0.0,
            scan,
            size_ms: 90.0,
            density: 45.0,
            timing: 0.0,
            ..Params::default()
        };
        for keep in [true, false] {
            let marks: &[u64] = if keep { &starts } else { &[] };
            let clip = GrainClip::new(&src, 1, rate)
                .expect("a lab clip")
                .clean(None)
                .with_hits(marks);
            let mut v = GrainVoice::new(rate, 12);
            v.set_params(p);
            v.start();
            let mut budget = Budget::new(256);
            let n = (secs * f64::from(rate)) as usize;
            let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
            for s in (0..n).step_by(128) {
                let e = (s + 128).min(n);
                v.render(&clip, &mut budget, s as u64, &mut l[s..e], &mut r[s..e]);
            }
            let mono: Vec<f64> = l
                .iter()
                .zip(&r)
                .map(|(&a, &b)| 0.5 * (f64::from(a) + f64::from(b)))
                .collect();
            let ms = |t: f64| (t * f64::from(rate) / 1000.0) as isize;
            let level = |a: isize, b: isize| {
                let (a, b) = (a.max(0) as usize, (b.max(0) as usize).min(mono.len()));
                rms(&mono[a..b.max(a)])
            };
            // The loudest millisecond from `a` to `b`, stepping 0.25 ms.
            let loudest = |a: isize, b: isize| {
                let mut best = (0.0f64, a);
                let mut t = a;
                while t + ms(1.0) <= b {
                    let x = level(t, t + ms(1.0));
                    if x > best.0 {
                        best = (x, t);
                    }
                    t += ms(0.25);
                }
                best
            };
            let (mut pre, mut attack, mut flam) = (Vec::new(), Vec::new(), Vec::new());
            for &h in &starts[1..] {
                let at = (h as f64 / f64::from(scan)).round() as isize;
                if at + ms(80.0) > mono.len() as isize {
                    break;
                }
                let (peak, peak_at) = loudest(at, at + ms(30.0));
                pre.push(db(loudest(at - ms(30.0), at - ms(2.0)).0 / peak));
                // Half the hit's level, in half-millisecond steps.
                let mut t = at;
                while t < at + ms(60.0) && level(t, t + ms(0.5)) < 0.5 * peak {
                    t += ms(0.5);
                }
                attack.push((t + ms(0.5) - at) as f64 * 1000.0 / f64::from(rate));
                // The largest rise of the level (over 4 ms, a millisecond
                // at a time) over the lowest since the peak.
                let (mut low, mut rise) = (f64::INFINITY, 0.0f64);
                let mut t = peak_at;
                while t + ms(4.0) <= at + ms(60.0) {
                    let x = level(t, t + ms(4.0));
                    low = low.min(x);
                    rise = rise.max(db(x / low));
                    t += ms(1.0);
                }
                flam.push(rise);
            }
            let median = |mut v: Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            };
            let out = Stereo::new(
                l.iter().map(|&x| f64::from(x)).collect(),
                r.iter().map(|&x| f64::from(x)).collect(),
                rate,
            );
            let lu = lufs(&out) - src_lufs;
            let subject = if keep {
                format!("hits[drums, {name}]")
            } else {
                format!("hits[drums, {name}, without hits]")
            };
            let info = |b: Better| if keep { b } else { Better::Info };
            let target = |t: f64| keep.then_some(t);
            put(
                rows,
                "G12",
                subject.clone(),
                "pre_db",
                median(pre),
                info(Better::Lower),
                target(-24.0),
                1.0,
            );
            put(
                rows,
                "G12",
                subject.clone(),
                "attack_ms",
                median(attack),
                info(Better::Lower),
                target(1.0),
                0.25,
            );
            put(
                rows,
                "G12",
                subject.clone(),
                "flam_db",
                median(flam),
                info(Better::Lower),
                target(3.0),
                0.5,
            );
            // Shown: a stretch keeping its hits has as many a second as
            // its tempo, each whole, so it is louder sped up.
            put(
                rows,
                "G12",
                subject,
                "level_change_lu",
                lu,
                Better::Info,
                None,
                0.2,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// G13: movement keeps the level (step B3).

/// How far the level swings (dB): over 400 ms, every 100 ms, as a
/// momentary loudness meter moves.
fn swing(s: &Stereo) -> f64 {
    let hop = (s.rate / 10) as usize;
    let levels: Vec<f64> = (0..s.left.len().saturating_sub(4 * hop))
        .step_by(hop)
        .map(|i| db(rms(&s.left[i..i + 4 * hop])))
        .collect();
    let lo = levels.iter().cloned().fold(f64::MAX, f64::min);
    let hi = levels.iter().cloned().fold(f64::MIN, f64::max);
    hi - lo
}

fn movement(rows: &mut Vec<Row>, lab: &Lab) {
    let p = Params {
        density: 80.0,
        spray: 0.3,
        ..base()
    };
    let lfo = |target, depth| GrainLfo {
        shape: LfoShape::Sine,
        rate_hz: 0.5,
        beats: None,
        target,
        depth,
    };
    let cases = [
        (
            "lfo position",
            Params {
                lfo: lfo(MotionTarget::Position, 1.0),
                ..p
            },
        ),
        (
            "lfo size",
            Params {
                lfo: lfo(MotionTarget::Size, 0.5),
                ..p
            },
        ),
        (
            "lfo density",
            Params {
                lfo: lfo(MotionTarget::Density, 0.5),
                ..p
            },
        ),
        (
            "lfo pitch",
            Params {
                lfo: lfo(MotionTarget::Pitch, 0.5),
                ..p
            },
        ),
        (
            "lfo spray",
            Params {
                spray: 0.5,
                lfo: lfo(MotionTarget::Spray, 1.0),
                ..p
            },
        ),
        (
            "env size",
            Params {
                env: GrainEnv {
                    attack_ms: 500.0,
                    decay_ms: 1000.0,
                    sustain: 0.5,
                    target: MotionTarget::Size,
                    depth: 0.5,
                },
                ..p
            },
        ),
    ];
    for (name, src) in [("pink", &lab.pink), ("pad", &lab.pad)] {
        // Two of the LFO's cycles, after the cloud fills in.
        let (still, _) = steady(src, lab.rate, p, 4.0, 13);
        let (level, spread) = (lufs(&still), swing(&still));
        for (what, q) in cases {
            let (s, _) = steady(src, lab.rate, q, 4.0, 13);
            put(
                rows,
                "G13",
                format!("move[{name}, {what}]"),
                "level_change_lu",
                lufs(&s) - level,
                Better::Near(0.0),
                Some(1.0),
                0.1,
            );
            put(
                rows,
                "G13",
                format!("move[{name}, {what}]"),
                "extra_swing_db",
                swing(&s) - spread,
                Better::Lower,
                Some(3.0),
                // A random cloud's level wanders a few tenths either way.
                0.5,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// G14: the presets (step B6).

fn presets(rows: &mut Vec<Row>, lab: &Lab) {
    for (name, src) in [("pink", &lab.pink), ("pad", &lab.pad)] {
        let (plain, _) = steady(src, lab.rate, base(), 4.0, 14);
        let plain = lufs(&plain);
        for preset in grain_presets::presets() {
            let q = Params {
                position: POSITION,
                ..preset.grain
            };
            let (s, _) = steady(src, lab.rate, q, 4.0, 14);
            // A stutter's grains fill half of each sixteenth, fading in
            // and out: its gaps are its sound.
            let at = if preset.mode == GrainMode::Repeat {
                -4.0
            } else {
                0.0
            };
            put(
                rows,
                "G14",
                format!("preset[{name}, {}]", preset.name),
                "level_change_lu",
                lufs(&s) - plain,
                Better::Near(at),
                Some(1.5),
                0.1,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// G15: the Grains effect's feedback (step C1).

/// 4 s of `src` scaled by `gain`, then silence, `secs` in all, through a
/// fully wet Grains effect with `feedback`.
fn fed_back(src: &[f32], rate: u32, gain: f32, feedback: f32, secs: usize) -> Stereo {
    let mut fx = GrainFx::new(rate, 2.0, 15).expect("a lab effect");
    fx.set_params(Params {
        position: 0.5,
        spray: 0.1,
        ..Params::default()
    });
    fx.set_mix(1.0);
    fx.set_feedback(feedback);
    let mut budget = Budget::new(64);
    let n = secs * rate as usize;
    let k = (4 * rate as usize).min(src.len());
    let mut l = vec![0.0f32; n];
    l[..k].iter_mut().zip(src).for_each(|(o, x)| *o = x * gain);
    let mut r = l.clone();
    for s in (0..n).step_by(128) {
        let e = (s + 128).min(n);
        fx.process(&mut budget, s as u64, &mut l[s..e], &mut r[s..e]);
    }
    let f = |x: Vec<f32>| x.into_iter().map(f64::from).collect::<Vec<f64>>();
    Stereo::new(f(l), f(r), rate)
}

fn feedback(rows: &mut Vec<Row>, lab: &Lab) {
    let rate = lab.rate as usize;
    // The lab's pink is -20 dBFS RMS: twice is -14, peaking near 0 dBFS.
    let full = fed_back(&lab.pink, lab.rate, 2.0, 1.0, 20);
    let level = |s: &Stereo, a: usize, b: usize| db(rms(&s.left[a * rate..b * rate]));
    put(
        rows,
        "G15",
        "feedback[pink -14 dBFS, full]".into(),
        "peak_dbfs",
        loudness::sample_peak(&full),
        Better::Lower,
        Some(3.0),
        0.1,
    );
    put(
        rows,
        "G15",
        "feedback[pink -14 dBFS, full]".into(),
        "growth_db",
        level(&full, 16, 20) - level(&full, 8, 12),
        Better::Lower,
        Some(1.0),
        0.3,
    );
    put(
        rows,
        "G15",
        "feedback[pink -14 dBFS, full]".into(),
        "held_dbfs",
        level(&full, 16, 20),
        Better::Info,
        None,
        0.0,
    );
    let half = fed_back(&lab.pink, lab.rate, 2.0, 0.5, 16);
    put(
        rows,
        "G15",
        "feedback[pink -14 dBFS, half]".into(),
        "fall_db_per_s",
        (level(&half, 5, 7) - level(&half, 14, 16)) / 9.0,
        Better::Info,
        None,
        0.0,
    );
}

// ---------------------------------------------------------------------------
// G16: the Grains effect's repeat (step C2).

/// The Grains effect fully dry on up to 6 s of `src`, at 120 BPM,
/// repeating `beats` beats from 2 s: its left side, and that frame.
fn repeated(src: &[f32], rate: u32, beats: f64) -> (Vec<f64>, usize) {
    let mut fx = GrainFx::new(rate, 2.0, 16).expect("a lab effect");
    fx.set_tempo(0.5 * f64::from(rate));
    let mut budget = Budget::new(64);
    let n = src.len().min(6 * rate as usize);
    let (mut l, mut r) = (src[..n].to_vec(), src[..n].to_vec());
    let t0 = 2 * rate as usize;
    fx.process(&mut budget, 0, &mut l[..t0], &mut r[..t0]);
    fx.start_repeat(beats);
    for s in (t0..n).step_by(128) {
        let e = (s + 128).min(n);
        fx.process(&mut budget, s as u64, &mut l[s..e], &mut r[s..e]);
    }
    (l.into_iter().map(f64::from).collect(), t0)
}

fn repeat(rows: &mut Vec<Row>, lab: &Lab) {
    let rate = lab.rate as usize;
    // A low tone at -20 dBFS: a join that clicks jumps far more in a
    // sample than the tone ever does.
    let tone: Vec<f32> = sine(110.0, 0.1, 6 * rate, lab.rate)
        .into_iter()
        .map(|v| v as f32)
        .collect();
    let step = |x: &[f64]| {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f64::max)
    };
    for (name, beats) in [("1/4", 1.0), ("1/16", 0.25)] {
        let len = beats * 0.5 * f64::from(lab.rate);
        // The level: the repeats, from half a second on, against the
        // stretch of pink they repeat.
        let (out, t0) = repeated(&lab.pink, lab.rate, beats);
        put(
            rows,
            "G16",
            format!("repeat[pink, {name}]"),
            "level_change_db",
            db(rms(&out[t0 + rate / 2..])) - db(rms(&out[t0 - len as usize..t0])),
            Better::Near(0.0),
            Some(0.5),
            0.1,
        );
        // The joins: the biggest step from one sample to the next within
        // 10 ms of where each repeat starts, against the tone's own.
        let (out, t0) = repeated(&tone, lab.rate, beats);
        let mut worst = 0.0f64;
        for k in 1.. {
            let at = t0 + (k as f64 * len).round() as usize;
            if at + rate / 100 >= out.len() {
                break;
            }
            worst = worst.max(step(&out[at - rate / 100..at + rate / 100]));
        }
        put(
            rows,
            "G16",
            format!("repeat[tone 110 Hz, {name}]"),
            "join_step_ratio",
            worst / step(&out[..t0]),
            Better::Lower,
            Some(1.5),
            0.05,
        );
    }
}

// ---------------------------------------------------------------------------
// G17: a filter on each grain (step C4).

fn filtered(rows: &mut Vec<Row>, lab: &Lab) {
    use mpfree_grain::{FilterKind, GrainFilter};
    let cloud = |filter: GrainFilter, secs: f64| {
        let p = Params { filter, ..base() };
        steady(&lab.pink, lab.rate, p, secs, 17).0
    };
    let filter = |kind, resonance, spray| GrainFilter {
        kind,
        cutoff_hz: 1_000.0,
        resonance,
        spray,
    };
    let dry = cloud(GrainFilter::default(), 5.0);
    let n = 4096;
    let dry_sp = welch(&dry.left, n);
    // Power within `oct` octaves of `hz`, against the dry cloud's, dB.
    let rel = |sp: &[f64], hz: f64, oct: f64| {
        let bw = hz * (2f64.powf(oct) - 1.0);
        10.0 * (band(sp, lab.rate, hz, bw) / band(&dry_sp, lab.rate, hz, bw)).log10()
    };
    for (name, kind, stop, pass) in [
        ("low pass", FilterKind::Low, 4_000.0, 250.0),
        ("high pass", FilterKind::High, 250.0, 4_000.0),
    ] {
        let sp = welch(&cloud(filter(kind, 0.0, 0.0), 5.0).left, n);
        put(
            rows,
            "G17",
            format!("filter[pink, {name} 1 kHz]"),
            "stop_db",
            rel(&sp, stop, 1.0 / 6.0),
            Better::Lower,
            Some(-20.0),
            0.3,
        );
        put(
            rows,
            "G17",
            format!("filter[pink, {name} 1 kHz]"),
            "pass_db",
            rel(&sp, pass, 1.0 / 6.0),
            Better::Near(0.0),
            Some(0.5),
            0.1,
        );
    }
    // Grains reading all over the pink (position spray 0.5), so each
    // octave has the same to colour, for 8 s (200 grains), each band
    // an octave wide; against the same cloud unfiltered.
    let spread = |filter: GrainFilter| {
        let p = Params {
            spray: 0.5,
            filter,
            ..base()
        };
        steady(&lab.pink, lab.rate, p, 8.0, 17).0
    };
    let sprayed = spread(filter(FilterKind::Band, 1.0, 1.0));
    let (sp, open) = (
        welch(&sprayed.left, n),
        welch(&spread(GrainFilter::default()).left, n),
    );
    let at = [250.0, 1_000.0, 4_000.0].map(|hz| {
        let bw = hz * (2f64.sqrt() - 1.0);
        10.0 * (band(&sp, lab.rate, hz, bw) / band(&open, lab.rate, hz, bw)).log10()
    });
    let hi = at.iter().cloned().fold(f64::MIN, f64::max);
    let lo = at.iter().cloned().fold(f64::MAX, f64::min);
    put(
        rows,
        "G17",
        "filter[pink, band 1 kHz, Q 12, full spray]".into(),
        "octave_spread_db",
        hi - lo,
        Better::Lower,
        Some(3.0),
        0.3,
    );
    put(
        rows,
        "G17",
        "filter[pink, band 1 kHz, Q 12, full spray]".into(),
        "loudness_lu",
        lufs(&sprayed) - lufs(&spread(GrainFilter::default())),
        Better::Info,
        None,
        0.0,
    );
    let ringing = cloud(filter(FilterKind::Low, 1.0, 0.0), 5.0);
    put(
        rows,
        "G17",
        "filter[pink, low pass 1 kHz, Q 12]".into(),
        "peak_gain_db",
        loudness::sample_peak(&ringing) - loudness::sample_peak(&dry),
        Better::Lower,
        Some(12.0),
        0.3,
    );
}
