//! The sampler's listening set (sampler refactor phase 4, step 18): sounds
//! a musician puts on a grain pad, each played through a pad in every
//! grain mode, with numbers for how steady the level stays and how far
//! peaks rise, and the renders to listen to.
//!
//! - **The sounds** ([`sounds`]), 4 s each at 44.1 kHz, all made in
//!   MPFree, no samples: drum hits (a factory kit's kick, snare and hats,
//!   one at a time), a drum loop (two bars at 100 BPM), an electric piano
//!   (Keys, Soft Tines: a phrase and a chord), a bass line (Bass, Round
//!   Mono), a sung line (Choir, Aah Choir, one voice), a steady tone (a
//!   220 Hz saw-like tone, band-limited), pink noise, a tone that turns
//!   into noise (the tone for 1.5 s, a 1 s crossfade at one loudness,
//!   then noise) and a wide pad (detuned saws, different on each side:
//!   left and right correlate at about 0.3). The steady four (tone,
//!   noise, tone into noise, wide pad) are -20 dBFS RMS; the others peak
//!   at -6 dBFS.
//! - **The cases** ([`CASES`]): a pad of the sound in each grain mode as
//!   the Grain tool starts it (`mpfree_sampler_ui::grain_tab::
//!   mode_settings`): Cloud frozen on one spot and Cloud moving through
//!   the sound (scan 1), Stretch at the sound's speed and at half speed,
//!   Repeat (sixteenths at 120 BPM) and Tone (A2), both on one spot. The
//!   spot is 40 % into the sound, or a snare's start for the drums. 5 s
//!   each, the pad held, its envelope open.
//! - **The numbers**, against the same pad playing the sound plainly
//!   ([`plain`], no grains, so the pad's pan law and velocity cancel
//!   out): the **level change** (integrated loudness, LU), the **wobble**
//!   (how far the level swings: the standard deviation, in dB, of the RMS
//!   level over 50 ms windows every 10 ms, from 0.5 s to 0.5 s before the
//!   sound ends, less the plain one's; only for the steady sounds, where the
//!   sound itself holds still) and the **peak rise** (sample peak against
//!   the plain one's, dB: grains that overlap add up). For the tone into
//!   noise read through at its own speed (Cloud moving, Stretch), the
//!   **jump**: how far the level moves from the tone to the noise, against
//!   the plain play's: the level law's step 20 number. The ratchet (`tests/listening.rs`,
//!   `quality/listening.tsv`) keeps each from getting worse: steps 19 and
//!   20 are measured against it, and the owner's ear decides.
//!
//! `fxlab listening --out DIR` writes every sound and render as a WAV
//! (`<sound>.wav` played plainly, `<sound>-<case>.wav`) and the numbers as `report.txt`,
//! for the listening pages.

use mpfree_node::Node;
use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{
    GrainMode, GrainParams, Layer, Pad, PlayMode, Project, Region, Source,
};
use mpfree_sampler_ui::grain_tab::mode_settings;

use crate::loudness;
use crate::quality::{Better, Row};
use crate::signals::{db_to_amp, pink, Rng};
use crate::synth::{self, Engine as Synth};
use crate::wav::Stereo;

/// Each sound's length, seconds.
pub const SOUND_SECS: f64 = 4.0;
/// Each render's length, seconds.
pub const RENDER_SECS: f64 = 5.0;
/// The tempo Repeat's sixteenths follow.
const BPM: f64 = 120.0;
/// Where the frozen cases read, across the sound, unless the sound says
/// (the drums read from a snare's start).
const AT: f32 = 0.4;
/// The steady sounds' level, dBFS RMS.
const STEADY_RMS_DB: f64 = -20.0;
/// The other sounds' peak, dBFS.
const PEAK_DB: f64 = -6.0;

/// Where the tone starts turning into noise, and over how long, seconds.
const FADE_AT: f64 = 1.5;
const FADE_SECS: f64 = 1.0;

/// The ratchet file's header.
const HEADER: &str =
    "# The sampler's listening set ratchet (phase 4, step 18): every number may stay or get better, never worse.\n\
# Written by `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test listening`;\n\
# see docs/dev/design/sampler-architecture.md (\"Phase 4\"). key<TAB>value\n";

/// A sound of the set.
pub struct Sound {
    /// Its file name and the ratchet's word for it.
    pub name: &'static str,
    /// What it is, in a few words, for a listening page.
    pub about: &'static str,
    /// It holds one level throughout, so a wobble is the grains'.
    pub steady: bool,
    /// Where the frozen cases read it, across the sound.
    pub at: f32,
    pub audio: Stereo,
}

/// A way a pad plays the sound.
#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub name: &'static str,
    pub about: &'static str,
    pub mode: GrainMode,
    /// The scan, when not the mode's own.
    pub scan: Option<f32>,
    /// Where it reads from, across the sound; `None` reads the sound's
    /// own spot ([`Sound::at`]).
    pub position: Option<f32>,
    /// Steady stretch (step 19) on.
    pub align: bool,
}

/// Every case, in order.
pub const CASES: [Case; 7] = [
    Case {
        name: "cloud",
        about: "Cloud, frozen on one spot",
        mode: GrainMode::Cloud,
        scan: None,
        position: None,
        align: false,
    },
    Case {
        name: "cloud-moving",
        about: "Cloud moving through the sound (scan 1)",
        mode: GrainMode::Cloud,
        scan: Some(1.0),
        position: Some(0.0),
        align: false,
    },
    Case {
        name: "stretch",
        about: "Stretch at the sound's speed",
        mode: GrainMode::Stretch,
        scan: None,
        position: Some(0.0),
        align: false,
    },
    Case {
        name: "stretch-half",
        about: "Stretch at half speed",
        mode: GrainMode::Stretch,
        scan: Some(0.5),
        position: Some(0.0),
        align: false,
    },
    Case {
        name: "stretch-half-steady",
        about: "Stretch at half speed, steady (step 19)",
        mode: GrainMode::Stretch,
        scan: Some(0.5),
        position: Some(0.0),
        align: true,
    },
    Case {
        name: "repeat",
        about: "Repeat: sixteenths at 120 BPM, on one spot",
        mode: GrainMode::Repeat,
        scan: None,
        position: None,
        align: false,
    },
    Case {
        name: "tone",
        about: "Tone: A2, from one spot",
        mode: GrainMode::Tone,
        scan: None,
        position: None,
        align: false,
    },
];

pub(crate) fn frames(secs: f64, rate: u32) -> usize {
    (secs * f64::from(rate)).round() as usize
}

/// `s` cut or padded with silence to `n` frames.
fn scale(s: Stereo, g: f64) -> Stereo {
    let l = s.left.iter().map(|x| x * g).collect();
    let r = s.right.iter().map(|x| x * g).collect();
    Stereo::new(l, r, s.rate)
}

fn peak(s: &Stereo) -> f64 {
    s.left
        .iter()
        .chain(&s.right)
        .fold(0.0f64, |m, x| m.max(x.abs()))
}

fn rms(s: &Stereo) -> f64 {
    let n = (s.left.len() + s.right.len()).max(1) as f64;
    (s.left.iter().chain(&s.right).map(|x| x * x).sum::<f64>() / n).sqrt()
}

/// `s` with its peak at [`PEAK_DB`].
pub(crate) fn to_peak(s: Stereo) -> Stereo {
    let p = peak(&s).max(1e-9);
    scale(s, db_to_amp(PEAK_DB) / p)
}

/// `s` at [`STEADY_RMS_DB`] RMS.
fn to_rms(s: Stereo) -> Stereo {
    let r = rms(&s).max(1e-9);
    scale(s, db_to_amp(STEADY_RMS_DB) / r)
}

/// A band-limited saw-like tone at `hz` (harmonics 1/k up to 8 kHz), with
/// a slightly different phase set for `seed`.
fn tone(hz: f64, n: usize, rate: u32, seed: u64) -> Vec<f64> {
    let mut rng = Rng::new(seed);
    let mut out = vec![0.0; n];
    let mut k = 1.0;
    while hz * k <= 8000.0 {
        let ph = rng.unit() * std::f64::consts::TAU;
        let w = std::f64::consts::TAU * hz * k / f64::from(rate);
        for (i, o) in out.iter_mut().enumerate() {
            *o += (w * i as f64 + ph).sin() / k;
        }
        k += 1.0;
    }
    out
}

/// `node` playing `notes` (note, velocity, start s, length s) for the
/// sound's length.
pub(crate) fn played(mut node: Box<dyn Node>, notes: &[(u8, u8, f64, f64)], rate: u32) -> Stereo {
    let score = synth::score(rate, notes);
    synth::render(node.as_mut(), rate, frames(SOUND_SECS, rate), &score)
}

/// A factory preset of `engine` by name (its first if the name is gone).
pub(crate) fn preset(engine: Synth, name: &str) -> usize {
    engine.presets().iter().position(|p| p == name).unwrap_or(0)
}

/// Every sound of the set, in order.
pub fn sounds(rate: u32) -> Vec<Sound> {
    let n = frames(SOUND_SECS, rate);
    let kit = 0;
    let base = synth::kit_base(kit, rate);
    // The kit's kick, snare and closed hat (its first, second and third
    // pads... as the kit tour's beat plays them: base, base + 1, base + 2).
    let (kick, snare, hat) = (base, base + 1, base + 2);
    let hits: Vec<(u8, u8, f64, f64)> = [
        (kick, 0.0),
        (snare, 0.7),
        (hat, 1.4),
        (kick, 2.0),
        (hat, 2.5),
        (snare, 3.0),
    ]
    .iter()
    .map(|&(k, t)| (k, 115, t, 0.05))
    .collect();
    let step = 60.0 / 100.0 / 4.0;
    let mut beat = Vec::new();
    for i in 0..32 {
        let t = f64::from(i) * step;
        if t >= SOUND_SECS {
            break;
        }
        if i % 8 == 0 || i % 16 == 10 {
            beat.push((kick, 120, t, 0.05));
        }
        if i % 8 == 4 {
            beat.push((snare, 115, t, 0.05));
        }
        if i % 2 == 0 {
            beat.push((hat, if i % 4 == 0 { 100 } else { 80 }, t, 0.05));
        }
    }
    let keys = Synth::Keys.make(preset(Synth::Keys, "Soft Tines"), rate);
    let piano = [
        (60, 96, 0.0, 0.9),
        (64, 90, 0.4, 0.8),
        (67, 92, 0.8, 0.9),
        (72, 100, 1.2, 0.7),
        (57, 90, 2.0, 1.8),
        (60, 88, 2.0, 1.8),
        (64, 88, 2.0, 1.8),
        (67, 92, 2.0, 1.8),
    ];
    let bass = Synth::Bass.make(preset(Synth::Bass, "Round Mono"), rate);
    let line = [
        (36, 110, 0.0, 0.45),
        (36, 100, 0.5, 0.2),
        (43, 105, 0.75, 0.45),
        (41, 105, 1.25, 0.7),
        (38, 110, 2.0, 0.45),
        (38, 100, 2.5, 0.2),
        (45, 105, 2.75, 0.45),
        (43, 105, 3.25, 0.6),
    ];
    let choir = Synth::Choir.make(preset(Synth::Choir, "Aah Choir"), rate);
    let sung = [
        (64, 100, 0.0, 0.9),
        (67, 100, 1.0, 0.9),
        (69, 100, 2.0, 0.6),
        (67, 100, 2.7, 1.2),
    ];
    let steady_tone = tone(220.0, n, rate, 1);
    let noise = pink(n, 1.0, 7);
    // Tone, then a crossfade at one loudness (equal power), then noise.
    let t_rms = crate::signals::rms_of(&steady_tone);
    let n_rms = crate::signals::rms_of(&noise);
    let (fade_at, fade_len) = (frames(FADE_AT, rate), frames(FADE_SECS, rate));
    let into: Vec<f64> = (0..n)
        .map(|i| {
            let x = ((i as f64 - fade_at as f64) / fade_len as f64).clamp(0.0, 1.0);
            let (gt, gn) = (
                (x * std::f64::consts::FRAC_PI_2).cos(),
                (x * std::f64::consts::FRAC_PI_2).sin(),
            );
            steady_tone[i] / t_rms * gt + noise[i] / n_rms * gn
        })
        .collect();
    // A wide pad: an A minor chord of tones, detuned and phased apart on
    // each side.
    let side = |seed: u64, cents: f64| -> Vec<f64> {
        let mut out = vec![0.0; n];
        for (j, hz) in [110.0, 220.0, 261.63, 329.63].into_iter().enumerate() {
            let det = hz * (cents / 1200.0).exp2();
            for (o, x) in out.iter_mut().zip(tone(det, n, rate, seed + j as u64)) {
                *o += x;
            }
        }
        out
    };
    let wide = Stereo::new(side(11, -6.0), side(23, 6.0), rate);
    vec![
        Sound {
            name: "drum-hits",
            about: "drum hits: a kick, snare and hat, one at a time",
            steady: false,
            at: 0.7 / SOUND_SECS as f32,
            audio: to_peak(played(synth::kit_node(kit, rate), &hits, rate)),
        },
        Sound {
            name: "drum-loop",
            about: "a drum loop: two bars at 100 BPM",
            steady: false,
            at: 0.6 / SOUND_SECS as f32,
            audio: to_peak(played(synth::kit_node(kit, rate), &beat, rate)),
        },
        Sound {
            name: "piano",
            about: "an electric piano (Keys, Soft Tines): a phrase, then a chord",
            steady: false,
            at: AT,
            audio: to_peak(played(keys, &piano, rate)),
        },
        Sound {
            name: "bass",
            about: "a bass line (Bass, Round Mono)",
            steady: false,
            at: AT,
            audio: to_peak(played(bass, &line, rate)),
        },
        Sound {
            name: "vocal",
            about: "a sung line (Choir, Aah Choir)",
            steady: false,
            at: AT,
            audio: to_peak(played(choir, &sung, rate)),
        },
        Sound {
            name: "tone",
            about: "a steady tone (220 Hz, saw-like)",
            steady: true,
            at: AT,
            audio: to_rms(Stereo::mono(steady_tone, rate)),
        },
        Sound {
            name: "noise",
            about: "pink noise",
            steady: true,
            at: AT,
            audio: to_rms(Stereo::mono(noise, rate)),
        },
        Sound {
            name: "tone-to-noise",
            about: "a tone that turns into noise at one loudness",
            steady: true,
            at: AT,
            audio: to_rms(Stereo::mono(into, rate)),
        },
        Sound {
            name: "wide-pad",
            about: "a wide stereo pad (different on each side)",
            steady: true,
            at: AT,
            audio: to_rms(wide),
        },
    ]
}

/// `sound` played through a pad as `case` sets it, [`RENDER_SECS`] long.
pub fn render(sound: &Sound, case: &Case, rate: u32) -> Stereo {
    play(sound, Some(case), rate)
}

/// The sound played plainly by the same pad (no grains): what each case
/// is measured against, so the pad's own pan law and velocity curve
/// cancel out.
pub fn plain(sound: &Sound, rate: u32) -> Stereo {
    play(sound, None, rate)
}

fn play(sound: &Sound, case: Option<&Case>, rate: u32) -> Stereo {
    let a = &sound.audio;
    let stereo = a.left != a.right;
    let data: Vec<f32> = if stereo {
        a.left
            .iter()
            .zip(&a.right)
            .flat_map(|(&l, &r)| [l as f32, r as f32])
            .collect()
    } else {
        a.left.iter().map(|&x| x as f32).collect()
    };
    let src = Source::new(sound.name, rate, if stereo { 2 } else { 1 }, data)
        .expect("a listening set sound");
    src.make_octaves();
    src.find_hits();
    let mut r = Region::whole(&src);
    if let Some(case) = case {
        r.mode = PlayMode::Granular;
        r.grain_mode = case.mode;
        let mut g = mode_settings(
            &GrainParams {
                position: case.position.unwrap_or(sound.at),
                ..GrainParams::default()
            },
            case.mode,
            BPM,
        );
        if let Some(s) = case.scan {
            g.scan = s;
        }
        g.align = case.align;
        r.grain = g;
    }
    let mut pad = Pad::new(sound.name, Layer::new(r));
    pad.amp = Ahdsr::OPEN;
    let mut p = Project::new();
    p.add_source(src);
    p.set_pad(0, Some(pad), None).expect("a listening set pad");
    let mut e = Engine::new(rate, 8);
    e.set_tempo(BPM);
    e.set_view(EngineView::build(&p, rate));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let n = frames(RENDER_SECS, rate);
    let (mut l, mut rr) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let (mut bl, mut br) = ([0.0f32; 128], [0.0f32; 128]);
    while l.len() < n {
        let k = 128.min(n - l.len());
        e.render(&mut bl[..k], &mut br[..k]);
        l.extend(bl[..k].iter().map(|&x| f64::from(x)));
        rr.extend(br[..k].iter().map(|&x| f64::from(x)));
    }
    Stereo::new(l, rr, rate)
}

/// How far the RMS level swings over 50 ms windows every 10 ms, from
/// 0.5 s in to 0.5 s before the sound ends (the render runs on past it,
/// the plain play falls silent): the standard deviation, dB.
pub fn wobble_db(s: &Stereo) -> f64 {
    let rate = f64::from(s.rate);
    let (win, hop) = ((0.05 * rate) as usize, (0.01 * rate) as usize);
    let (from, to) = (
        (0.5 * rate) as usize,
        frames(SOUND_SECS - 0.5, s.rate).min(s.len()),
    );
    let mut levels = Vec::new();
    let mut at = from;
    while at + win <= to {
        let e: f64 = (at..at + win)
            .map(|i| s.left[i] * s.left[i] + s.right[i] * s.right[i])
            .sum::<f64>()
            / (2 * win) as f64;
        levels.push(10.0 * e.max(1e-20).log10());
        at += hop;
    }
    if levels.len() < 2 {
        return 0.0;
    }
    let mean = levels.iter().sum::<f64>() / levels.len() as f64;
    (levels.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / levels.len() as f64).sqrt()
}

/// The numbers for `sound` played as `case` (`out`).
pub struct Measured {
    pub level_change_lu: f64,
    pub wobble_db: Option<f64>,
    pub peak_rise_db: f64,
    /// Tone into noise, read through at the sound's speed or faster: how
    /// far the level moves from the tone to the noise, against the plain
    /// play's (dB). The level law's step (step 20).
    pub jump_db: Option<f64>,
}

pub fn measure(sound: &Sound, case: &Case, plain: &Stereo, out: &Stereo) -> Measured {
    let to_db = |x: f64| 20.0 * x.max(1e-12).log10();
    // Stretch's starting settings read at the sound's speed.
    let stretch = (case.mode == GrainMode::Stretch).then_some(1.0);
    let through = sound.name == "tone-to-noise" && case.scan.or(stretch).is_some_and(|s| s >= 1.0);
    Measured {
        level_change_lu: loudness::integrated(out) - loudness::integrated(plain),
        wobble_db: sound.steady.then(|| wobble_db(out) - wobble_db(plain)),
        peak_rise_db: to_db(peak(out)) - to_db(peak(plain)),
        jump_db: through.then(|| {
            // The tone before the fade and the noise after it, a little
            // way off either end; a scan of 1 reads each where it is.
            let gap = |a: f64, b: f64| span_db(out, a, b) - span_db(plain, a, b);
            gap(FADE_AT + FADE_SECS + 0.1, SOUND_SECS - 0.4) - gap(0.5, FADE_AT - 0.1)
        }),
    }
}

/// The RMS level of `s` from `a` to `b` seconds, dB.
fn span_db(s: &Stereo, a: f64, b: f64) -> f64 {
    let (a, b) = (frames(a, s.rate), frames(b, s.rate).min(s.len()));
    let e: f64 = (a..b)
        .map(|i| s.left[i] * s.left[i] + s.right[i] * s.right[i])
        .sum::<f64>()
        / (2 * (b - a).max(1)) as f64;
    10.0 * e.max(1e-20).log10()
}

fn put(rows: &mut Vec<Row>, subject: &str, metric: &str, value: f64, better: Better, tol: f64) {
    rows.push(Row {
        target_id: "L18",
        subject: format!("listening/{subject}"),
        metric: metric.to_string(),
        value,
        better,
        target: None,
        tol,
    });
}

/// The rows of one sound and case.
fn rows_of(rows: &mut Vec<Row>, sound: &Sound, case: &Case, m: &Measured) {
    let subject = format!("{}[{}]", sound.name, case.name);
    // Repeat's gaps and Tone's one spot are their sound: their level is
    // shown, not held.
    let held = sound.steady && !matches!(case.mode, GrainMode::Repeat | GrainMode::Tone);
    let level = if held {
        Better::Near(0.0)
    } else {
        Better::Info
    };
    put(
        rows,
        &subject,
        "level_change_lu",
        m.level_change_lu,
        level,
        0.3,
    );
    if let Some(w) = m.wobble_db {
        put(rows, &subject, "wobble_db", w, Better::Lower, 0.3);
    }
    put(
        rows,
        &subject,
        "peak_rise_db",
        m.peak_rise_db,
        Better::Lower,
        0.5,
    );
    if let Some(j) = m.jump_db {
        put(rows, &subject, "jump_db", j, Better::Near(0.0), 0.3);
    }
}

/// Every number, sound by sound, case by case.
pub fn report(rate: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    for s in sounds(rate) {
        let base = plain(&s, rate);
        for c in &CASES {
            let out = render(&s, c, rate);
            rows_of(&mut rows, &s, c, &measure(&s, c, &base, &out));
        }
    }
    rows
}

/// The ratchet file for `rows`.
pub fn ratchet_text(rows: &[Row]) -> String {
    let body = crate::sampler::ratchet_text(rows);
    // The sampler's header, swapped for this set's.
    let start = body.lines().take_while(|l| l.starts_with('#')).count();
    let mut s = String::from(HEADER);
    for l in body.lines().skip(start) {
        s.push_str(l);
        s.push('\n');
    }
    s
}

/// Writes every sound as the pad plays it plainly (`<sound>.wav`) and render (`<sound>-<case>.wav`)
/// into `dir`, and the numbers as `report.txt`; gives the report.
pub fn write_set(dir: &std::path::Path, rate: u32) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    let mut rows = Vec::new();
    for s in sounds(rate) {
        let base = plain(&s, rate);
        crate::wav::write(&dir.join(format!("{}.wav", s.name)), &base)?;
        for c in &CASES {
            let out = render(&s, c, rate);
            rows_of(&mut rows, &s, c, &measure(&s, c, &base, &out));
            crate::wav::write(&dir.join(format!("{}-{}.wav", s.name, c.name)), &out)?;
        }
    }
    let text = crate::sampler::to_text(&rows);
    std::fs::write(dir.join("report.txt"), &text)?;
    Ok(text)
}
