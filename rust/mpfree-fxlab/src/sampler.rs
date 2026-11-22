//! The sampler in the lab (the sampler review's engine item 7): the same
//! kind of numbers as the effects' quality report, measured on a pad of
//! `mpfree-sampler` playing a test sound, and a ratchet of their own
//! (`tests/sampler.rs`, `quality/sampler.tsv`) that keeps every one from
//! getting worse.
//!
//! - **S1, pitched reads:** a sine on the pad, tuned so its output lands
//!   on one FFT bin (as [`harmonics`] does), at 3 and 9 kHz out, a fifth
//!   down, a semitone up (both the Hermite read), a fifth up (the
//!   band-limited read) and an octave and a fifth up (an octave copy): the
//!   loudest other bin from 20 Hz to 20 kHz against the tone, where it is,
//!   and the tone's level against the same tone at pitch.
//! - **S2, what must fold away:** a 15 kHz sine an octave up (30 kHz,
//!   above what 44.1 kHz carries) and a 96 kHz file's 30 kHz tone at pitch:
//!   what is heard of them against the tone.
//! - **S3, loop seams:** a held 440 Hz sine looped over a length that
//!   isn't whole periods, forward with no crossfade and with one, and
//!   ping-pong: the sharpest step in the loop against the sine's own
//!   ([`crate::zipper`]'s measure).
//! - **S4, stops:** the same step when a voice is stolen (its pad hit
//!   again at polyphony 1), choked (another pad of its mute group hit) and
//!   let go with the Play tab's shortest release (1 ms) and with none.
//! - **S5, the filter and the read:** the pad's response from a sweep
//!   ([`response`]) with each filter type against its analog prototype
//!   (the state-variable filter's `1 / (s² + ks + 1)`, `k = 2 - 1.95 ×
//!   resonance`; the Notch's `(1 + s²)` over it; the Ladder's `m / ((1 +
//!   s)⁴ + k)`; the Vowel's two band-passes in parallel), from 30 Hz to
//!   8 kHz wherever the prototype is above −30 dB; and with no filter, how
//!   flat the read is.
//! - **S6, Drive's aliases:** a 2.5 and a 5 kHz sine at 0 dBFS on a pad
//!   whose Ladder is open (20 kHz) and driven 12 and 24 dB: the loudest
//!   bin that is no harmonic of the tone, from 20 Hz to 20 kHz, against
//!   the tone (sampler R5.6; the Drive runs its curve at twice the rate
//!   with first-order ADAA, as the effects' Drive does).
//! - **S7, the Comb:** where its lowest peak lands against the cutoff, in
//!   cents, from a click on the pad at Resonance 1 (220 Hz and 3.3 kHz,
//!   the delay a fraction of a frame there); noise through it at full
//!   feedback against the same noise dry (its gain keeps a full sound's
//!   loudness); and a sine right on a peak at full feedback, against dry
//!   (information: how far a tone can stand out). It is a straight delay
//!   and sum, so it makes no aliases of its own; the S5 rows hold its
//!   response against the prototype.
//!
//! Cost per voice is the bench's: `mpfree-bench`'s sampler cases, counted
//! on CI by the instruction gate. Design: `docs/dev/design/fx-quality.md`
//! ("The sampler").

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{
    Fade, FadeCurve, Layer, Loop, LoopMode, Pad, PlayMode, Project, Region, Source,
};

use crate::fft::{bin_db, power_spectrum};
use crate::harmonics;
use crate::quality::{Better, Row};
use crate::response;
use crate::signals::{coherent, db_to_amp, pink, rms_of, sine, Rng};
use crate::wav::Stereo;

/// 32.32 fixed point: how the engine holds a voice's speed.
const ONE: f64 = 4_294_967_296.0;
/// The sines' level, dBFS peak.
const LEVEL_DB: f64 = -6.0;

/// The ratchet file's header.
const HEADER: &str =
    "# The sampler's quality ratchet: every number may stay or get better, never worse.\n\
# Written by `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test sampler`;\n\
# see docs/dev/design/fx-quality.md (\"The sampler\"). key<TAB>value\n";

/// Every number, in the order measured.
pub fn report(rate: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    pitched(&mut rows, rate);
    pitched_y5(&mut rows, rate);
    folds(&mut rows, rate);
    kept_top(&mut rows, rate);
    seams(&mut rows, rate);
    stops(&mut rows, rate);
    filters(&mut rows, rate);
    drive(&mut rows, rate);
    comb(&mut rows, rate);
    rows
}

/// The rows as text, one a line.
pub fn to_text(rows: &[Row]) -> String {
    let mut s = String::new();
    for r in rows {
        let target = match (r.target, r.met()) {
            (Some(t), Some(true)) => format!("target {t}: met"),
            (Some(t), _) => format!("target {t}: not yet"),
            (None, _) => String::new(),
        };
        s.push_str(&format!(
            "{:<4} {:<52} {:>10.3}  {target}\n",
            r.target_id,
            r.key(),
            r.value
        ));
    }
    s
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

/// What fails the ratchet: a row worse than recorded, a row not recorded
/// yet, a recorded row no longer measured.
pub fn ratchet_check(rows: &[Row], recorded: &[(String, f64)]) -> Vec<String> {
    let mut problems = Vec::new();
    for r in rows.iter().filter(|r| r.better != Better::Info) {
        let key = r.key();
        match recorded.iter().find(|(k, _)| *k == key) {
            Some((_, old)) if r.worse_than(*old) => problems.push(format!(
                "worse: {key} is {} (was {}, {:?} is better, tolerance {})",
                num(r.value),
                num(*old),
                r.better,
                r.tol
            )),
            Some(_) => {}
            None => problems.push(format!("not recorded: {key} = {}", num(r.value))),
        }
    }
    for (k, _) in recorded {
        if !rows.iter().any(|r| r.key() == *k) {
            problems.push(format!("no longer measured: {k}"));
        }
    }
    problems
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

/// Levels far below hearing read as −140 dB, so numerical noise is never
/// held to account.
fn floor(db: f64) -> f64 {
    db.max(-140.0)
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
        subject: format!("sampler/{subject}"),
        metric: metric.to_string(),
        value,
        better,
        target,
        tol,
    });
}

// ---------------------------------------------------------------------------
// Playing the pad.

/// A sound holding `x` (mono) at `rate`.
fn sound(x: &[f64], rate: u32) -> Source {
    Source::new("lab", rate, 1, x.iter().map(|&v| v as f32).collect()).expect("a lab sound")
}

/// A pad playing all of `s` once, its envelope open, as `shape` leaves it.
fn pad(s: &Source, shape: impl FnOnce(&mut Pad)) -> Pad {
    let mut r = Region::whole(s);
    r.mode = PlayMode::OneShot;
    let mut p = Pad::new("lab", Layer::new(r));
    p.amp = Ahdsr::OPEN;
    shape(&mut p);
    p
}

/// A kit of `pads` (pad 0 first) on their sounds.
fn kit(pads: Vec<(Source, Pad)>) -> Project {
    let mut p = Project::new();
    for (i, (s, pad)) in pads.into_iter().enumerate() {
        p.add_source(s);
        p.set_pad(i, Some(pad), None).expect("a lab pad");
    }
    p
}

/// `frames` frames of `project` at `rate`, each event applied at its frame.
fn play(project: &Project, rate: u32, events: &[(usize, Event)], frames: usize) -> Stereo {
    // Made here, so a voice reading an octave up finds them at once.
    for s in project.sources() {
        s.make_octaves();
    }
    let mut e = Engine::new(rate, 8);
    e.set_view(EngineView::build(project, rate));
    let (mut l, mut r) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    let (mut bl, mut br) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let (mut at, mut next) = (0, 0);
    while at < frames {
        while next < events.len() && events[next].0 <= at {
            e.apply(events[next].1);
            next += 1;
        }
        let until = events.get(next).map_or(frames, |ev| ev.0.min(frames));
        let n = 128.min(until - at);
        e.render(&mut bl[..n], &mut br[..n]);
        l.extend(bl[..n].iter().map(|&x| f64::from(x)));
        r.extend(br[..n].iter().map(|&x| f64::from(x)));
        at += n;
    }
    Stereo::new(l, r, rate)
}

/// `x` (mono, at `rate`) played once on a pad with `filter` (`None`: the
/// pad's filter off), the left channel's first `frames` frames: for
/// [`crate::filters`].
pub(crate) fn through_pad(x: &[f64], filter: Option<Filter>, rate: u32, frames: usize) -> Vec<f64> {
    let s = sound(x, rate);
    let p = pad(&s, |p| {
        if let Some(f) = filter {
            p.filter = f;
        }
    });
    play(&kit(vec![(s, p)]), rate, &[(0, hit(0))], frames).left
}

fn hit(pad: u8) -> Event {
    Event::NoteOn { pad, velocity: 127 }
}

// ---------------------------------------------------------------------------
// S1 and S2: pitched reads.

/// The speed a pad tuned `cents` plays a sound at `src_rate` at, as the
/// engine rounds it.
fn speed(cents: i32, src_rate: u32, rate: u32) -> f64 {
    let ratio = f64::from(src_rate) / f64::from(rate) * (f64::from(cents) / 1200.0).exp2();
    (ratio * ONE).round().max(1.0) / ONE
}

/// What a pad tuned `cents` makes of a sine that comes out on the bin
/// nearest `out_hz`: the analysis, from frame `settle`.
/// What a pitched sine measured: its level (dBFS peak), and the loudest
/// other bin from 20 Hz to 20 kHz against it (dB) and where it is (Hz).
struct Pitched {
    level_db: f64,
    spur_db: f64,
    spur_hz: f64,
}

fn pitched_sine(cents: i32, out_hz: f64, rate: u32) -> Pitched {
    let settle = 4096;
    let n = harmonics::N;
    let frames = settle + n;
    let v = speed(cents, rate, rate);
    let (bin, f_out) = coherent(out_hz, n, rate);
    let x = sine(
        f_out / v,
        db_to_amp(LEVEL_DB),
        (frames as f64 * v) as usize + 64,
        rate,
    );
    let s = sound(&x, rate);
    let p = pad(&s, |p| p.tune_cents = cents);
    let out = play(&kit(vec![(s, p)]), rate, &[(0, hit(0))], frames);
    let ps = power_spectrum(&out.left[settle..settle + n]);
    let hz = |k: usize| k as f64 * f64::from(rate) / n as f64;
    let fund = ps[bin].max(1e-300);
    let (spur_k, spur) = (1..=n / 2)
        .filter(|&k| k != bin && hz(k) >= 20.0 && hz(k) <= 20_000.0)
        .map(|k| (k, ps[k]))
        .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
    Pitched {
        level_db: bin_db(fund),
        spur_db: floor(10.0 * (spur / fund).max(1e-30).log10()),
        spur_hz: hz(spur_k),
    }
}

fn pitched(rows: &mut Vec<Row>, rate: u32) {
    for out_hz in [3_000.0, 9_000.0] {
        let at_pitch = pitched_sine(0, out_hz, rate);
        for (cents, how) in [
            (-700, "a fifth down"),
            (100, "a semitone up"),
            (700, "a fifth up"),
            (1900, "an octave and a fifth up"),
        ] {
            let h = pitched_sine(cents, out_hz, rate);
            let subject = format!("pitch[{how}, {} kHz out]", out_hz / 1000.0);
            put(
                rows,
                "S1",
                subject.clone(),
                "spur_worst_db",
                h.spur_db,
                Better::Lower,
                Some(-50.0),
                0.5,
            );
            // Where it is: shown, not kept.
            put(
                rows,
                "S1",
                subject.clone(),
                "spur_hz",
                h.spur_hz.round(),
                Better::Info,
                None,
                0.0,
            );
            put(
                rows,
                "S1",
                subject,
                "level_db",
                h.level_db - at_pitch.level_db,
                Better::Near(0.0),
                Some(1.0),
                0.05,
            );
        }
    }
}

/// Y5 of the DSP quality plan (docs/dev/design/dsp-quality.md): the
/// small pitch changes and pitch-downs S1 leaves out, and a tone near the
/// top. A row whose sine would have to be above 21 kHz in the sound is
/// left out (it can't be in a 44.1 kHz file).
fn pitched_y5(rows: &mut Vec<Row>, rate: u32) {
    for out_hz in [3_000.0, 9_000.0, 13_500.0] {
        let at_pitch = pitched_sine(0, out_hz, rate);
        for (cents, how) in [
            (-1200, "an octave down"),
            (-700, "a fifth down"),
            (-200, "two semitones down"),
            (-100, "a semitone down"),
            (100, "a semitone up"),
            (200, "two semitones up"),
        ] {
            // S1 already has these two at 3 and 9 kHz.
            if out_hz < 10_000.0 && (cents == -700 || cents == 100) {
                continue;
            }
            if out_hz / (f64::from(cents) / 1200.0).exp2() > 21_000.0 {
                continue;
            }
            let h = pitched_sine(cents, out_hz, rate);
            let subject = format!("pitch[{how}, {} kHz out]", out_hz / 1000.0);
            put(
                rows,
                "Y5",
                subject.clone(),
                "spur_worst_db",
                h.spur_db,
                Better::Lower,
                Some(-50.0),
                0.5,
            );
            put(
                rows,
                "Y5",
                subject.clone(),
                "spur_hz",
                h.spur_hz.round(),
                Better::Info,
                None,
                0.0,
            );
            put(
                rows,
                "Y5",
                subject,
                "level_db",
                h.level_db - at_pitch.level_db,
                Better::Near(0.0),
                Some(1.0),
                0.05,
            );
        }
    }
}

/// Y6 of the DSP quality plan: a 96 kHz file's 30 kHz tone played an
/// octave down. Kept as the original (step 3.2) it is heard at 15 kHz;
/// read from the 44.1 kHz copy it is gone. Shown, not kept, until step
/// 3.2 says which one the sampler plays.
fn kept_top(rows: &mut Vec<Row>, rate: u32) {
    put(
        rows,
        "Y6",
        "kept top[30 kHz in a 96 kHz file, an octave down]".into(),
        "heard_db",
        heard_of(30_000.0, 96_000, -1200, rate),
        Better::Info,
        None,
        0.0,
    );
}

/// What is heard of a sine at `hz` in a sound at `src_rate`, played
/// `cents` up, against the sine itself: dB RMS.
fn heard_of(hz: f64, src_rate: u32, cents: i32, rate: u32) -> f64 {
    let settle = 4096;
    let frames = settle + harmonics::N;
    let v = speed(cents, src_rate, rate);
    let amp = db_to_amp(LEVEL_DB);
    let x = sine(hz, amp, (frames as f64 * v) as usize + 64, src_rate);
    let s = sound(&x, src_rate);
    let p = pad(&s, |p| p.tune_cents = cents);
    let out = play(&kit(vec![(s, p)]), rate, &[(0, hit(0))], frames);
    let seg = &out.left[settle..];
    let rms = (seg.iter().map(|x| x * x).sum::<f64>() / seg.len() as f64).sqrt();
    floor(20.0 * (rms / (amp / 2f64.sqrt())).max(1e-30).log10())
}

fn folds(rows: &mut Vec<Row>, rate: u32) {
    put(
        rows,
        "S2",
        "fold[15 kHz an octave up]".into(),
        "heard_db",
        heard_of(15_000.0, rate, 1200, rate),
        Better::Lower,
        Some(-50.0),
        0.5,
    );
    put(
        rows,
        "S2",
        "fold[30 kHz in a 96 kHz file]".into(),
        "heard_db",
        heard_of(30_000.0, 96_000, 0, rate),
        Better::Lower,
        Some(-50.0),
        0.5,
    );
}

// ---------------------------------------------------------------------------
// S3 and S4: steps.

/// The sharpest step (largest second difference) in `x`.
fn max_d2(x: &[f64]) -> f64 {
    x.windows(3)
        .fold(0.0f64, |m, w| m.max((w[2] - 2.0 * w[1] + w[0]).abs()))
}

/// The sharpest step in `out` over `test` against the sharpest over
/// `steady`, dB (0 when no sharper).
fn step_db(out: &Stereo, steady: std::ops::Range<usize>, test: std::ops::Range<usize>) -> f64 {
    let mid = out.mid();
    let a = max_d2(&mid[steady]).max(1e-30);
    let b = max_d2(&mid[test]);
    (20.0 * (b / a).max(1e-30).log10()).max(0.0)
}

/// A 440 Hz sine `secs` long, faded in over 10 ms so a voice starting
/// on it doesn't step.
fn tone(secs: f64, rate: u32) -> Source {
    let n = (secs * f64::from(rate)) as usize;
    let fade = rate as usize / 100;
    let x: Vec<f64> = sine(440.0, db_to_amp(LEVEL_DB), n, rate)
        .into_iter()
        .enumerate()
        .map(|(i, v)| v * (i as f64 / fade as f64).min(1.0))
        .collect();
    sound(&x, rate)
}

fn seams(rows: &mut Vec<Row>, rate: u32) {
    // 30,117 frames: 300.48 periods, so the loop's ends don't meet.
    let (start, end) = (20_000u64, 50_117u64);
    let frames = 3 * rate as usize;
    for (how, mode, crossfade) in [
        ("forward, no crossfade", LoopMode::Forward, 0u32),
        ("forward, 1000-frame crossfade", LoopMode::Forward, 1_000),
        ("ping-pong", LoopMode::PingPong, 0),
    ] {
        let s = tone(2.0, rate);
        let p = pad(&s, |p| {
            let r = &mut p.layers[0].region;
            r.mode = PlayMode::Loop;
            r.sustain_loop = Some(Loop {
                start,
                end,
                crossfade,
                mode,
            });
        });
        let out = play(&kit(vec![(s, p)]), rate, &[(0, hit(0))], frames);
        put(
            rows,
            "S3",
            format!("loop[{how}]"),
            "step_db",
            step_db(&out, 2_000..18_000, start as usize..frames),
            Better::Lower,
            None,
            0.5,
        );
    }
}

fn stops(rows: &mut Vec<Row>, rate: u32) {
    let at = 30_000usize;
    let frames = at + 8_000;
    let steady = 2_000..at - 1_000;
    let around = at - 1_000..frames;
    let gate = |p: &mut Pad| p.layers[0].region.mode = PlayMode::Gate;

    // Stolen: the pad hit again with room for one voice. The new hit
    // starts on the fade in, so only the old voice's fade shows.
    let s = tone(3.0, rate);
    let p = pad(&s, |p| {
        gate(p);
        p.polyphony = 1;
        p.layers[0].region.fade_in = Fade {
            frames: rate / 100,
            curve: FadeCurve::Linear,
        };
    });
    let out = play(
        &kit(vec![(s, p)]),
        rate,
        &[(0, hit(0)), (at, hit(0))],
        frames,
    );
    put(
        rows,
        "S4",
        "stop[stolen]".into(),
        "step_db",
        step_db(&out, steady.clone(), around.clone()),
        Better::Lower,
        None,
        0.5,
    );

    // Choked: another pad of its mute group hit (a silent one, so only
    // the choke is heard).
    let s = tone(3.0, rate);
    let p0 = pad(&s, |p| {
        gate(p);
        p.mute_group = Some(1);
    });
    let quiet = sound(&[0.0; 1_000], rate);
    let p1 = pad(&quiet, |p| p.mute_group = Some(1));
    let out = play(
        &kit(vec![(s, p0), (quiet, p1)]),
        rate,
        &[(0, hit(0)), (at, hit(1))],
        frames,
    );
    put(
        rows,
        "S4",
        "stop[choked]".into(),
        "step_db",
        step_db(&out, steady.clone(), around.clone()),
        Better::Lower,
        None,
        0.5,
    );

    // Let go with the shortest release the Play tab sets (1 ms), and with
    // none (0 ms, which a kit or project file can hold).
    for (how, ms) in [("let go, 1 ms release", 1.0), ("let go, no release", 0.0)] {
        let s = tone(3.0, rate);
        let p = pad(&s, |p| {
            gate(p);
            p.amp.release_ms = ms;
        });
        let out = play(
            &kit(vec![(s, p)]),
            rate,
            &[(0, hit(0)), (at, Event::NoteOff { pad: 0 })],
            frames,
        );
        put(
            rows,
            "S4",
            format!("stop[{how}]"),
            "step_db",
            step_db(&out, steady.clone(), around.clone()),
            Better::Lower,
            None,
            0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// S5: the filter and the read.

/// The analog prototype of one state-variable stage at `x` = f / fc, dB.
fn stage_db(kind: FilterKind, resonance: f64, x: f64) -> f64 {
    let k = 2.0 - 1.95 * resonance.clamp(0.0, 1.0);
    let den = ((1.0 - x * x).powi(2) + (k * x).powi(2)).sqrt();
    let num = match kind {
        FilterKind::Off | FilterKind::Ladder | FilterKind::Vowel | FilterKind::Comb => return 0.0,
        FilterKind::LowPass => 1.0,
        FilterKind::BandPass => x,
        FilterKind::HighPass => x * x,
        FilterKind::Notch => (1.0 - x * x).abs(),
    };
    20.0 * (num / den).log10()
}

/// The Ladder's prototype (dB): four one-poles in a loop with feedback
/// `k = 4.2 × resonance`, then the make-up `1 + k / 2`, `m / ((1 + s)⁴ +
/// k)`, written out here rather than taken from the engine.
fn ladder_prototype_db(resonance: f64, x: f64) -> f64 {
    let k = 4.2 * resonance.clamp(0.0, 1.0);
    // (1 + jx)^4 by repeated products, as (re, im).
    let (mut re, mut im) = (1.0f64, 0.0f64);
    for _ in 0..4 {
        (re, im) = (re - im * x, re * x + im);
    }
    re += k;
    20.0 * ((1.0 + 0.5 * k) / (re * re + im * im).sqrt()).log10()
}

/// The Vowel's prototype (dB): two band-passes `k s / (s² + k s + 1)` at
/// its formants, the second at 0.6, then its gain `1.1 / sqrt k`, with `k =
/// 0.2 × 0.4^resonance` (the engine's formant table, written out here).
fn vowel_prototype_db(f: &Filter, hz: f64) -> f64 {
    let (f1, f2) = mpfree_sampler::dsp::vowel_formants(f.cutoff_hz);
    let k = 0.2 * 0.4f64.powf(f64::from(f.resonance).clamp(0.0, 1.0));
    let band = |fc: f32| {
        let x = hz / f64::from(fc);
        let (re, im) = (1.0 - x * x, k * x);
        let d = re * re + im * im;
        (k * x * im / d, k * x * re / d)
    };
    let (a, b) = (band(f1), band(f2));
    let g = 1.1 / k.sqrt();
    let (re, im) = (g * (a.0 + 0.6 * b.0), g * (a.1 + 0.6 * b.1));
    10.0 * (re * re + im * im).max(1e-30).log10()
}

/// The Comb's prototype (dB): `y = x + g y(t - 1/fc)`, `g = 0.97 ×
/// resonance`, then `sqrt(1 - g²)`: `sqrt(1 - g²) / |1 - g e^(-j 2π f /
/// fc)|`, and its DC blocker, a one-pole high-pass at 10 Hz, written out
/// here rather than taken from the engine.
fn comb_prototype_db(resonance: f64, x: f64, hz: f64) -> f64 {
    let g = 0.97 * resonance.clamp(0.0, 1.0);
    let w = 2.0 * std::f64::consts::PI * x;
    let (re, im) = (1.0 - g * w.cos(), g * w.sin());
    let h = hz / 10.0;
    let hp = h * h / (1.0 + h * h);
    10.0 * (hp * (1.0 - g * g) / (re * re + im * im)).log10()
}

/// The prototype of a filter: one stage, or two for 4 poles (the second
/// at half the resonance, as the engine sets it); the Ladder's, the
/// Vowel's and the Comb's own.
fn prototype_db(f: &Filter, hz: f64) -> f64 {
    let x = hz / f64::from(f.cutoff_hz);
    let r = f64::from(f.resonance);
    match f.kind {
        FilterKind::Ladder => return ladder_prototype_db(r, x),
        FilterKind::Vowel => return vowel_prototype_db(f, hz),
        FilterKind::Comb => return comb_prototype_db(r, x, hz),
        _ => {}
    }
    let mut db = stage_db(f.kind, r, x);
    if f.poles >= 4 {
        db += stage_db(f.kind, r * 0.5, x);
    }
    db
}

fn filters(rows: &mut Vec<Row>, rate: u32) {
    const WINDOW: usize = 16_384;
    let sweep = response::sweep(-12.0, rate);
    let frames = sweep.sweep.len() + WINDOW;
    let measure = |filter: Option<Filter>| {
        let s = sound(&sweep.sweep, rate);
        let p = pad(&s, |p| {
            if let Some(f) = filter {
                p.filter = f;
            }
        });
        let out = play(&kit(vec![(s, p)]), rate, &[(0, hit(0))], frames);
        response::analyse(&sweep, &out, WINDOW)
    };
    let plain = measure(None);
    let at_1k = plain.at(1_000.0).unwrap_or(0.0);
    let flat = plain
        .points
        .iter()
        .filter(|p| p.0 >= 30.0 && p.0 <= 16_000.0)
        .map(|p| (p.1 - at_1k).abs())
        .fold(0.0, f64::max);
    put(
        rows,
        "S5",
        "read[at pitch, no filter]".into(),
        "flatness_db",
        flat,
        Better::Lower,
        Some(0.1),
        0.05,
    );
    for (how, kind, poles, resonance) in [
        ("low-pass 2-pole", FilterKind::LowPass, 2u8, 0.0f32),
        ("low-pass 4-pole", FilterKind::LowPass, 4, 0.0),
        (
            "low-pass 2-pole, resonance 0.9",
            FilterKind::LowPass,
            2,
            0.9,
        ),
        ("high-pass 2-pole", FilterKind::HighPass, 2, 0.0),
        (
            "band-pass 2-pole, resonance 0.5",
            FilterKind::BandPass,
            2,
            0.5,
        ),
        ("notch 2-pole", FilterKind::Notch, 2, 0.0),
        ("ladder", FilterKind::Ladder, 4, 0.0),
        ("ladder, resonance 0.5", FilterKind::Ladder, 4, 0.5),
        ("vowel ah, resonance 0.5", FilterKind::Vowel, 2, 0.5),
        ("comb, resonance 0.5", FilterKind::Comb, 2, 0.5),
        ("comb, resonance 0.9", FilterKind::Comb, 2, 0.9),
    ] {
        let f = Filter {
            kind,
            poles,
            cutoff_hz: 1_000.0,
            resonance,
            env_octaves: 0.0,
            drive_db: 0.0,
        };
        let r = measure(Some(f));
        let error = r
            .points
            .iter()
            .zip(&plain.points)
            .filter(|(p, _)| p.0 >= 30.0 && p.0 <= 8_000.0 && prototype_db(&f, p.0) > -30.0)
            .map(|(p, q)| ((p.1 - q.1) - prototype_db(&f, p.0)).abs())
            .fold(0.0, f64::max);
        put(
            rows,
            "S5",
            format!("filter[{how}, 1 kHz]"),
            "error_db",
            error,
            Better::Lower,
            Some(1.0),
            0.05,
        );
    }
}

// ---------------------------------------------------------------------------
// S6: Drive's aliases.

/// The loudest bin that is no harmonic of a coherent sine at `bin`, from
/// 20 Hz to 20 kHz, against the sine (dB).
fn worst_alias_db(x: &[f64], bin: usize, rate: u32) -> f64 {
    let n = x.len();
    let ps = power_spectrum(x);
    let hz = |k: usize| k as f64 * f64::from(rate) / n as f64;
    let fund = ps[bin].max(1e-300);
    let worst = (1..=n / 2)
        .filter(|&k| k % bin != 0 && hz(k) >= 20.0 && hz(k) <= 20_000.0)
        .map(|k| ps[k])
        .fold(0.0, f64::max);
    floor(10.0 * (worst / fund).max(1e-30).log10())
}

fn drive(rows: &mut Vec<Row>, rate: u32) {
    let settle = 4096;
    let n = harmonics::N;
    for out_hz in [2_500.0, 5_000.0] {
        let (bin, f) = coherent(out_hz, n, rate);
        let x = sine(f, 1.0, settle + n + 64, rate);
        for drive_db in [12.0f32, 24.0] {
            let s = sound(&x, rate);
            let p = pad(&s, |p| {
                p.filter = Filter {
                    kind: FilterKind::Ladder,
                    poles: 4,
                    cutoff_hz: 20_000.0,
                    resonance: 0.0,
                    env_octaves: 0.0,
                    drive_db,
                };
            });
            let out = play(&kit(vec![(s, p)]), rate, &[(0, hit(0))], settle + n);
            put(
                rows,
                "S6",
                format!(
                    "drive[ladder open, {drive_db} dB, {} kHz at 0 dBFS]",
                    out_hz / 1000.0
                ),
                "alias_worst_db",
                worst_alias_db(&out.left[settle..settle + n], bin, rate),
                Better::Lower,
                // The effects' smooth curves' target (fx-quality.md G1)
                // at 12 dB; 24 dB drives hard on purpose (as G3).
                (drive_db <= 12.0).then_some(-65.0),
                0.5,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// S7: the Comb.

/// A Comb at `cutoff_hz` and `resonance`.
fn comb_filter(cutoff_hz: f32, resonance: f32) -> Filter {
    Filter {
        kind: FilterKind::Comb,
        poles: 2,
        cutoff_hz,
        resonance,
        env_octaves: 0.0,
        drive_db: 0.0,
    }
}

/// `x` played on a pad through `filter` (None: dry), `frames` long.
fn through(x: &[f64], filter: Option<Filter>, rate: u32, frames: usize) -> Vec<f64> {
    let s = sound(x, rate);
    let p = pad(&s, |p| {
        if let Some(f) = filter {
            p.filter = f;
        }
    });
    play(&kit(vec![(s, p)]), rate, &[(0, hit(0))], frames).left
}

/// The loudest bin from `lo` to `hi` Hz of `x`'s spectrum, placed between
/// bins by a parabola through its dB and its neighbours' (Hz).
fn peak_hz(x: &[f64], rate: u32, lo: f64, hi: f64) -> f64 {
    let n = x.len();
    let ps = power_spectrum(x);
    let bin_hz = f64::from(rate) / n as f64;
    let (a, b) = ((lo / bin_hz) as usize, (hi / bin_hz) as usize);
    let k = (a.max(1)..=b.min(ps.len() - 2))
        .max_by(|&i, &j| ps[i].total_cmp(&ps[j]))
        .unwrap_or(1);
    let db = |i: usize| 10.0 * ps[i].max(1e-300).log10();
    let (l, c, r) = (db(k - 1), db(k), db(k + 1));
    let den = l - 2.0 * c + r;
    let off = if den.abs() > 1e-12 {
        0.5 * (l - r) / den
    } else {
        0.0
    };
    (k as f64 + off) * bin_hz
}

fn comb(rows: &mut Vec<Row>, rate: u32) {
    let n = harmonics::N;
    // A click: the comb's answer is its ringing, peaks at the cutoff and
    // its multiples.
    // (The click's sound is as long as the window: a voice ends with its
    // sound, and the ringing with it.)
    let mut click = vec![0.0; n];
    click[0] = 0.5;
    for hz in [220.0f32, 3_300.0] {
        let out = through(&click, Some(comb_filter(hz, 1.0)), rate, n);
        let found = peak_hz(&out, rate, f64::from(hz) * 0.8, f64::from(hz) * 1.2);
        put(
            rows,
            "S7",
            format!("comb[{hz} Hz, resonance 1]"),
            "pitch_error_cents",
            (1200.0 * (found / f64::from(hz)).log2()).abs(),
            Better::Lower,
            Some(5.0),
            0.5,
        );
    }
    // Noise, and a sine on the second peak, full feedback.
    let frames = 4 * f64::from(rate) as usize;
    let settle = f64::from(rate) as usize;
    let mut rng = Rng::new(17);
    let white: Vec<f64> = (0..frames).map(|_| 0.25 * rng.bipolar()).collect();
    let pinkish = pink(frames, 0.125, 17);
    let tone = sine(440.0, 0.0625, frames, rate);
    for (how, x, better, target) in [
        // The Hermite read takes a little of the top each time round the
        // loop, so a bright sound rings softer than its own loudness.
        ("pink noise", &pinkish, Better::Near(0.0), Some(3.0)),
        ("white noise", &white, Better::Near(0.0), None),
        ("a sine on a peak", &tone, Better::Info, None),
    ] {
        let dry = through(x, None, rate, frames);
        let wet = through(x, Some(comb_filter(220.0, 1.0)), rate, frames);
        let gain = 20.0 * (rms_of(&wet[settle..]) / rms_of(&dry[settle..])).log10();
        put(
            rows,
            "S7",
            format!("comb[220 Hz, resonance 1, {how}]"),
            "level_db",
            gain,
            better,
            target,
            0.1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prototypes_are_the_textbook_curves() {
        let lp = Filter {
            kind: FilterKind::LowPass,
            poles: 2,
            cutoff_hz: 1_000.0,
            resonance: 0.0,
            env_octaves: 0.0,
            drive_db: 0.0,
        };
        // k = 2 (Q 0.5): -6 dB at the cutoff, -12 dB an octave per octave
        // far above.
        assert!((prototype_db(&lp, 1_000.0) + 6.0206).abs() < 1e-3);
        assert!((prototype_db(&lp, 16_000.0) - prototype_db(&lp, 8_000.0) + 12.0).abs() < 0.1);
        let lp4 = Filter { poles: 4, ..lp };
        assert!((prototype_db(&lp4, 1_000.0) - 2.0 * prototype_db(&lp, 1_000.0)).abs() < 1e-9);
        let hp = Filter {
            kind: FilterKind::HighPass,
            ..lp
        };
        assert!((prototype_db(&hp, 1_000.0) - prototype_db(&lp, 1_000.0)).abs() < 1e-9);
    }

    #[test]
    fn the_engine_rounds_speeds_as_the_lab_does() {
        assert_eq!(speed(1200, 44_100, 44_100), 2.0);
        assert_eq!(speed(0, 88_200, 44_100), 2.0);
        assert!((speed(700, 44_100, 44_100) - 1.498_307_077).abs() < 1e-8);
    }

    #[test]
    fn a_step_shows_and_a_steady_sine_does_not() {
        let x = sine(440.0, 0.5, 4_000, 44_100);
        let mut y = x.clone();
        for v in &mut y[3_000..] {
            *v = 0.0;
        }
        let s = Stereo::mono(x, 44_100);
        // (Its peaks fall a hair differently on the samples in each part.)
        assert!(step_db(&s, 0..1_500, 1_500..4_000) < 0.01);
        let t = Stereo::mono(y, 44_100);
        assert!(step_db(&t, 0..1_500, 1_500..4_000) > 20.0);
    }

    #[test]
    fn the_ratchet_finds_worse_new_and_gone_numbers() {
        let row = |metric: &str, value: f64| Row {
            target_id: "S9",
            subject: "sampler/x".into(),
            metric: metric.into(),
            value,
            better: Better::Lower,
            target: None,
            tol: 0.5,
        };
        let rows = vec![row("a", 1.0), row("b", 2.0)];
        let recorded = vec![
            ("sampler/x/a".to_string(), 0.2),
            ("sampler/x/c".to_string(), 0.0),
        ];
        let p = ratchet_check(&rows, &recorded);
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(p[0].starts_with("worse: sampler/x/a"));
        assert!(p[1].starts_with("not recorded: sampler/x/b"));
        assert!(p[2].starts_with("no longer measured: sampler/x/c"));
        let text = ratchet_text(&rows);
        let back = crate::quality::ratchet_read(&text).unwrap();
        assert!(ratchet_check(&rows, &back).is_empty());
    }
}
