//! WORKSTATION ONLY: hear the composer (phase 0C) beside its simpler ways,
//! as rendered files, for the blind listening tests the design sets
//! (docs/dev/design/compose.md, "Listening tests").
//!
//! ```text
//! cargo run --release -p mpfree-compose --example listen -- --demo
//! cargo run --release -p mpfree-compose --example listen -- --midi my-clip.mid [--from 1] [--bars 2]
//! ```
//!
//! `--demo` makes the whole set from four built-in phrases (a C minor
//! riff, an F major tune, a swung A minor line and a D Dorian waltz in
//! 3/4). `--midi FILE` makes it from your own clip: a standard MIDI file,
//! as the MPC or any DAW exports it. Its first track with notes is read
//! (`--track N` for another), from bar `--from` (1) for `--bars` bars (2,
//! at most 8), at the file's own tempo and meter. Where notes start
//! together only the top one is kept, so a melody comes out of a played
//! part (`--all-notes` keeps them all).
//!
//! Other options: `--out DIR` (default ~/mpfree-work/compose-listen-<UTC>),
//! `--seed N` (1; every case adds its own number), `--style
//! minimal|groove|jazzish` (the clip's style for the tests that don't
//! name one; Minimal, the default style, unless set), `--bpm N`, `--swing
//! 1/8:58` (the swing the clip plays with; straight unless set), `--role
//! melody|bass`.
//!
//! For each phrase it writes these cases, every one a WAV and a MIDI file
//! of what is heard (swing applied), and lists them in report.txt and
//! answers.json (which file is which, for the listening page):
//!
//! | Case | Files | Played beside |
//! |---|---|---|
//! | keep-repeat | A, B | Continue by 4 bars; your phrase repeated |
//! | keep-moved | A, B | Continue; your phrase moved in the scale |
//! | random-minimal, -groove, -jazzish | A, B | Continue in that style; random notes from its scale and rhythm |
//! | styles | X, Y, Z | Continue in Minimal, Groove and Jazz-ish, shuffled |
//! | feel-plain | A, B | Continue; the same notes on the grid (a fixed swing when the clip swings) |
//! | feel-humanize | A, B | Continue; the same notes with ordinary humanize (10 ms) |
//! | sixteen | A, B | Continue by 16 bars; a four-bar motif repeated with small random changes |
//! | try-bar-1, -2 | A, B | one bar of the 16 tried again, heard with its neighbours; the same bar made without looking at the bar after it |
//! | motif | one | Continue by 16 bars: do you hear your motif? |
//! | long | one | Continue by 16 bars twice (about 32 bars), with the chords as a quiet pad |
//!
//! Every file plays on the same sounds at the same level (no file is made
//! louder than another): the melody on Keys "Mellow Tines" (a bass on
//! Pluck "Upright Bass"), a plain beat on the drum synth's "Tight Kit", and
//! in `long` only the chords on Strings "Ballad Pad". The pairs have no
//! chords under them, so neither side is helped by the composer's own.
//! It never touches any device: it reads only the MIDI file given and
//! writes only into its one new folder.
#![forbid(unsafe_code)]

use mpfree_compose::baseline::{
    motif_repeated, regroove, simpler, try_bar_one_sided, Feel, Simpler,
};
use mpfree_compose::{
    continue_notes, try_bar, Continue, Continued, InstrumentKind, PlainNote, Settings, Source,
    Style, GROOVE, JAZZISH, MINIMAL,
};
use mpfree_context::chord::ChordTimeline;
use mpfree_context::groove::STEP;
use mpfree_context::listen::{listen, Played};
use mpfree_context::{MusicalContext, PartRole, ReadBudget};
use mpfree_midi::smf::{self, Smf, SmfTrack};
use mpfree_midi::Msg;
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_seq::Note;
use mpfree_synth::{Drums, Keys, Pluck, Strings};
use mpfree_transport::time::{Meter, Swing, Tick, PPQ};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
/// Seconds after the last bar, for the release.
const TAIL: f64 = 2.0;
/// Bars Continue adds in the short cases.
const SHORT: u8 = 4;
/// The most bars read from a MIDI file.
const MOST_BARS: usize = 8;

// ---------------------------------------------------------------- phrases

/// A clip to carry on: its notes as written, and how it plays.
struct Phrase {
    slug: String,
    name: String,
    notes: Vec<PlainNote>,
    bars: usize,
    meter: Meter,
    bpm: f64,
    swing: Swing,
    style: Style,
    role: PartRole,
}

/// Notes in 1/16 steps: (start, length, pitch, velocity).
fn steps(notes: &[(Tick, Tick, u8, u8)]) -> Vec<PlainNote> {
    notes
        .iter()
        .map(|&(at, len, pitch, velocity)| PlainNote {
            at: at * STEP,
            len: len * STEP,
            pitch,
            velocity,
        })
        .collect()
}

fn demo_phrases() -> Vec<Phrase> {
    let four = Meter::default();
    vec![
        Phrase {
            slug: "riff".into(),
            name: "C minor riff (the second prototype's)".into(),
            notes: steps(&[
                (0, 2, 60, 100),
                (2, 1, 63, 88),
                (3, 1, 67, 92),
                (4, 2, 65, 96),
                (6, 2, 63, 86),
                (8, 3, 62, 94),
                (11, 1, 63, 80),
                (12, 4, 60, 90),
                (16, 2, 67, 100),
                (18, 1, 68, 86),
                (19, 1, 67, 84),
                (20, 2, 65, 94),
                (22, 2, 63, 84),
                (24, 2, 62, 92),
                (26, 2, 63, 84),
                (28, 4, 65, 88),
            ]),
            bars: 2,
            meter: four,
            bpm: 120.0,
            swing: Swing::default(),
            style: GROOVE,
            role: PartRole::Melody,
        },
        Phrase {
            slug: "tune".into(),
            name: "F major tune".into(),
            notes: steps(&[
                (0, 4, 65, 92),
                (4, 2, 69, 84),
                (6, 2, 72, 90),
                (8, 3, 70, 96),
                (11, 1, 69, 78),
                (12, 4, 67, 86),
                (16, 2, 69, 90),
                (18, 2, 67, 80),
                (20, 2, 65, 88),
                (22, 2, 64, 78),
                (24, 2, 62, 84),
                (26, 2, 64, 80),
                (28, 4, 67, 90),
            ]),
            bars: 2,
            meter: four,
            bpm: 92.0,
            swing: Swing::default(),
            style: MINIMAL,
            role: PartRole::Melody,
        },
        Phrase {
            slug: "swung".into(),
            name: "A minor line, swung eighths".into(),
            notes: steps(&[
                (0, 2, 69, 96),
                (2, 2, 72, 80),
                (4, 2, 76, 92),
                (6, 2, 74, 78),
                (8, 2, 72, 90),
                (10, 2, 71, 74),
                (12, 4, 69, 88),
                (16, 2, 67, 90),
                (18, 2, 69, 78),
                (20, 2, 72, 92),
                (22, 2, 71, 80),
                (24, 2, 69, 86),
                (26, 2, 67, 76),
                (28, 4, 64, 90),
            ]),
            bars: 2,
            meter: four,
            bpm: 100.0,
            swing: Swing::new(PPQ / 2, 62).expect("a swing the transport allows"),
            style: JAZZISH,
            role: PartRole::Melody,
        },
        Phrase {
            slug: "waltz".into(),
            name: "D Dorian waltz in 3/4".into(),
            notes: steps(&[
                (0, 4, 62, 96),
                (4, 4, 65, 80),
                (8, 4, 69, 84),
                (12, 6, 71, 92),
                (18, 2, 69, 78),
                (20, 4, 67, 80),
                (24, 4, 65, 90),
                (28, 4, 67, 80),
                (32, 4, 69, 84),
                (36, 8, 64, 88),
                (44, 4, 60, 74),
            ]),
            bars: 4,
            meter: Meter::new(3, 4).expect("3/4"),
            bpm: 96.0,
            swing: Swing::default(),
            style: MINIMAL,
            role: PartRole::Melody,
        },
    ]
}

/// The notes of one track of a MIDI file, from `from` (a bar, from 1) for
/// `bars` bars.
#[allow(clippy::too_many_arguments)]
fn midi_phrase(
    path: &Path,
    track: Option<usize>,
    from: usize,
    bars: usize,
    all_notes: bool,
    bpm: Option<f64>,
    swing: Swing,
    style: Style,
    role: PartRole,
) -> Result<(Phrase, String), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let reading = smf::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = reading.smf;
    let with_notes: Vec<&SmfTrack> = file
        .tracks
        .iter()
        .filter(|t| {
            t.events
                .iter()
                .any(|(_, m)| matches!(m, Msg::NoteOn { vel, .. } if *vel > 0))
        })
        .collect();
    if with_notes.is_empty() {
        return Err(format!("{} has no notes", path.display()));
    }
    let n = track.unwrap_or(1);
    let t = with_notes.get(n.wrapping_sub(1)).ok_or_else(|| {
        format!(
            "{} has {} tracks with notes; --track {n} isn't one",
            path.display(),
            with_notes.len()
        )
    })?;
    let meter = file
        .meters
        .iter()
        .find(|m| m.0 == 0)
        .and_then(|&(_, beats, unit)| Meter::new(beats, unit))
        .unwrap_or_default();
    let bpm = bpm.unwrap_or_else(|| file.tempos.iter().find(|t| t.0 == 0).map_or(120.0, |t| t.1));
    // Note-ons paired with their note-offs, first in first out.
    let mut open: Vec<(u8, u8, u64, u8)> = Vec::new();
    let mut played: Vec<(u64, u64, u8, u8)> = Vec::new();
    let mut events = t.events.clone();
    events.sort_by_key(|e| e.0);
    for (at, m) in events {
        match m {
            Msg::NoteOn { ch, note, vel } if vel > 0 => open.push((ch, note, at, vel)),
            Msg::NoteOn { ch, note, .. } | Msg::NoteOff { ch, note, .. } => {
                if let Some(i) = open.iter().position(|o| o.0 == ch && o.1 == note) {
                    let (_, _, on, vel) = open.remove(i);
                    played.push((on, at.saturating_sub(on).max(1), note, vel));
                }
            }
            _ => {}
        }
    }
    for (_, note, on, vel) in open {
        played.push((on, file.end.saturating_sub(on).max(1), note, vel));
    }
    let bar = meter.bar_ticks() as u64;
    let start = (from.max(1) as u64 - 1) * bar;
    let end = start + bars as u64 * bar;
    let mut inside: Vec<(u64, u64, u8, u8)> = played
        .into_iter()
        .filter(|n| (start..end).contains(&n.0))
        .map(|(on, len, note, vel)| (on - start, len.min(end - on), note, vel))
        .collect();
    inside.sort_by_key(|n| (n.0, std::cmp::Reverse(n.2)));
    let read = inside.len();
    if !all_notes {
        // Notes that start within a 1/64 of each other start together: the top one is kept.
        let mut kept: Vec<(u64, u64, u8, u8)> = Vec::new();
        for n in inside {
            match kept.last() {
                Some(k) if n.0 - k.0 < PPQ as u64 / 16 => {}
                _ => kept.push(n),
            }
        }
        inside = kept;
    }
    if inside.is_empty() {
        return Err(format!(
            "no notes start in bars {from} to {} of track {n}",
            from + bars - 1
        ));
    }
    let notes = inside
        .iter()
        .map(|&(at, len, pitch, velocity)| PlainNote {
            at: at as Tick,
            len: len as Tick,
            pitch,
            velocity,
        })
        .collect();
    let stem = path
        .file_stem()
        .map_or("clip".into(), |s| s.to_string_lossy().to_string());
    let name = if t.name.is_empty() {
        format!("{stem}, track {n}")
    } else {
        format!("{stem}, track {n} ({})", t.name)
    };
    let kept = inside.len();
    let said = format!(
        "{}: track {n} of {} with notes, bars {from} to {}, {} at {bpm:.1} BPM; {} notes read{}",
        path.display(),
        with_notes.len(),
        from + bars - 1,
        meter_words(meter),
        read,
        if read != kept {
            format!(", {kept} kept (the top note where notes start together)")
        } else {
            String::new()
        }
    );
    Ok((
        Phrase {
            slug: slug(&stem),
            name,
            notes,
            bars,
            meter,
            bpm,
            swing,
            style,
            role,
        },
        said,
    ))
}

fn meter_words(m: Meter) -> String {
    format!("{}/{}", m.beats, m.unit)
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "clip".into()
    } else {
        out
    }
}

// ---------------------------------------------------------------- requests

/// What Continue reads from a phrase: its notes, key and chords.
struct Clip {
    notes: Vec<(PlainNote, Source)>,
    context: MusicalContext,
    length: Tick,
}

fn clip_of(notes: &[PlainNote], bars: usize, meter: Meter) -> Clip {
    let length = bars as Tick * meter.bar_ticks();
    let played: Vec<Played> = notes
        .iter()
        .map(|n| Played::new(n.at, n.len, n.pitch, n.velocity))
        .collect();
    let r = listen(&played, length, ReadBudget::default(), meter);
    Clip {
        notes: notes.iter().map(|&n| (n, Source::You)).collect(),
        context: MusicalContext {
            key: r.key,
            chords: r.chords,
            groove: None,
            played_pcs: r.played_pcs,
            motifs: Vec::new(),
            read: Vec::new(),
            cut_short: r.cut_short,
        },
        length,
    }
}

fn request<'a>(p: &Phrase, clip: &'a Clip, settings: &'a Settings) -> Continue<'a> {
    Continue {
        notes: &clip.notes,
        length: clip.length,
        context: &clip.context,
        settings,
        role: p.role,
        kind: kind_of(p.role),
        meter: p.meter,
        bpm: p.bpm,
        swing: p.swing,
        loops: true,
    }
}

fn kind_of(role: PartRole) -> InstrumentKind {
    match role {
        PartRole::Bass => InstrumentKind::Bass,
        _ => InstrumentKind::Chordal,
    }
}

fn settings(style: Style, bars: u8, seed: u64) -> Settings {
    Settings {
        style,
        bars,
        seed,
        ..Settings::default()
    }
}

fn yours_as_notes(notes: &[PlainNote]) -> Vec<Note> {
    notes
        .iter()
        .map(|n| Note::new(n.at, n.len.max(1), n.pitch, n.velocity.clamp(1, 127)))
        .collect()
}

// ---------------------------------------------------------------- cases

/// One file to hear: the clip's notes as written, the bars heard, and
/// what it is (the answer, kept from the listener until they choose).
struct Take {
    what: String,
    mpfree: bool,
    notes: Vec<Note>,
    from_bar: usize,
    to_bar: usize,
    pad: Option<ChordTimeline>,
}

struct Case {
    id: String,
    phrase: String,
    test: &'static str,
    question: &'static str,
    about: String,
    /// Shuffled into A and B (or X, Y and Z); a single take is unlabelled.
    takes: Vec<Take>,
    labels: Vec<String>,
}

struct Made {
    cases: Vec<Case>,
    notes: String,
}

fn whole(yours: &[Note], new: &[Note]) -> Vec<Note> {
    let mut all: Vec<Note> = yours.iter().chain(new).copied().collect();
    all.sort_by_key(|n| (n.tick, n.note));
    all
}

fn style_words(s: &Style) -> &'static str {
    s.name
}

/// A small stable shuffle: the order of `n` takes for a case.
fn order(n: usize, seed: u64, case: &str) -> Vec<usize> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ seed;
    for b in case.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let mut idx: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        let j = (h % (i as u64 + 1)) as usize;
        idx.swap(i, j);
    }
    idx
}

fn finish(mut c: Case, seed: u64) -> Case {
    let n = c.takes.len();
    if n > 1 {
        let ord = order(n, seed, &c.id);
        let mut takes: Vec<Option<Take>> = c.takes.into_iter().map(Some).collect();
        c.takes = ord
            .iter()
            .map(|&i| takes[i].take().expect("each once"))
            .collect();
        let names: &[&str] = if n == 2 {
            &["A", "B"]
        } else {
            &["X", "Y", "Z"]
        };
        c.labels = names.iter().take(n).map(|s| s.to_string()).collect();
    } else {
        c.labels = vec![String::new()];
    }
    c
}

/// Every case for one phrase.
fn cases(p: &Phrase, seed: u64) -> Made {
    let clip = clip_of(&p.notes, p.bars, p.meter);
    let yours = yours_as_notes(&p.notes);
    let mut out: Vec<Case> = Vec::new();
    let mut notes = String::new();
    let _ = writeln!(
        notes,
        "{}: {} bars of {} at {:.0} BPM{}; read as {}; style {}",
        p.name,
        p.bars,
        meter_words(p.meter),
        p.bpm,
        if p.swing.amount != 50 {
            format!(
                ", swing {}% on {}",
                p.swing.amount,
                grid_words(p.swing.grid)
            )
        } else {
            String::new()
        },
        clip.context.key.words(),
        style_words(&p.style)
    );
    let short_to = p.bars + usize::from(SHORT);
    let mut k: u64 = 0;
    let mut next_seed = || {
        k += 1;
        seed.wrapping_mul(1000).wrapping_add(k)
    };
    let fail = |id: &str, why: String, notes: &mut String| {
        let _ = writeln!(notes, "  {id}: left out, {why}");
    };

    // Continue, beside your phrase repeated and moved in the scale.
    for (id, way, words) in [
        ("keep-repeat", Simpler::Repeat, "your phrase repeated"),
        (
            "keep-moved",
            Simpler::Moved,
            "your phrase moved in the scale",
        ),
    ] {
        let s = settings(p.style, SHORT, next_seed());
        let req = request(p, &clip, &s);
        match continue_notes(&req) {
            Ok(made) => out.push(Case {
                id: format!("{}-{id}", p.slug),
                phrase: p.name.clone(),
                test: "keep",
                question: "Which would you keep?",
                about: format!(
                    "Continue by {SHORT} bars in {}, beside {words}",
                    style_words(&p.style)
                ),
                takes: vec![
                    Take {
                        what: format!("mpfree, {}", style_words(&p.style)),
                        mpfree: true,
                        notes: whole(&yours, &made.notes),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                    },
                    Take {
                        what: words.to_string(),
                        mpfree: false,
                        notes: whole(&yours, &simpler(&req, way)),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                    },
                ],
                labels: Vec::new(),
            }),
            Err(e) => fail(id, e.words(), &mut notes),
        }
    }

    // Each style, beside random notes from its scale and rhythm.
    for style in [MINIMAL, GROOVE, JAZZISH] {
        let id = format!("random-{}", slug(style.name));
        let s = settings(style, SHORT, next_seed());
        let req = request(p, &clip, &s);
        match continue_notes(&req) {
            Ok(made) => out.push(Case {
                id: format!("{}-{id}", p.slug),
                phrase: p.name.clone(),
                test: "keep",
                question: "Which would you keep?",
                about: format!(
                    "Continue by {SHORT} bars in {}, beside random notes from its scale and rhythm",
                    style.name
                ),
                takes: vec![
                    Take {
                        what: format!("mpfree, {}", style.name),
                        mpfree: true,
                        notes: whole(&yours, &made.notes),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                    },
                    Take {
                        what: format!("random notes in {}'s scale and rhythm", style.name),
                        mpfree: false,
                        notes: whole(&yours, &simpler(&req, Simpler::Random)),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                    },
                ],
                labels: Vec::new(),
            }),
            Err(e) => fail(&id, e.words(), &mut notes),
        }
    }

    // The three styles, shuffled.
    {
        let seed = next_seed();
        let mut takes = Vec::new();
        for style in [MINIMAL, GROOVE, JAZZISH] {
            let s = settings(style, SHORT, seed);
            match continue_notes(&request(p, &clip, &s)) {
                Ok(made) => takes.push(Take {
                    what: style.name.to_string(),
                    mpfree: true,
                    notes: whole(&yours, &made.notes),
                    from_bar: 0,
                    to_bar: short_to,
                    pad: None,
                }),
                Err(e) => fail(
                    "styles",
                    format!("{}: {}", style.name, e.words()),
                    &mut notes,
                ),
            }
        }
        if takes.len() == 3 {
            out.push(Case {
                id: format!("{}-styles", p.slug),
                phrase: p.name.clone(),
                test: "styles",
                question: "Which is Minimal, which Groove, which Jazz-ish?",
                about: format!("Continue by {SHORT} bars in each style, the same seed, shuffled"),
                takes,
                labels: Vec::new(),
            });
        }
    }

    // The groove: mpfree's feel beside the plain ones, the same notes.
    let plain = if p.swing.amount != 50 {
        "a fixed swing (the clip's own)"
    } else {
        "the grid"
    };
    // Minimal has no drift: its feel is the grid, so the groove cases use
    // Groove there.
    let felt = if p.style.drift_ms > 0.0 {
        p.style
    } else {
        GROOVE
    };
    for (id, feel, words) in [
        ("feel-plain", Feel::Grid, plain.to_string()),
        (
            "feel-humanize",
            Feel::Humanize { ms: 10.0 },
            "ordinary humanize, 10 ms".to_string(),
        ),
    ] {
        let s = settings(felt, SHORT, next_seed());
        let req = request(p, &clip, &s);
        match continue_notes(&req) {
            Ok(made) => out.push(Case {
                id: format!("{}-{id}", p.slug),
                phrase: p.name.clone(),
                test: "groove",
                question: "Which feels better?",
                about: format!(
                    "Continue by {SHORT} bars in {}: the same notes, mpfree's feel beside {words}",
                    style_words(&felt)
                ),
                takes: vec![
                    Take {
                        what: format!("mpfree's feel ({})", style_words(&felt)),
                        mpfree: true,
                        notes: whole(&yours, &made.notes),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                    },
                    Take {
                        what: words.clone(),
                        mpfree: false,
                        notes: whole(&yours, &regroove(&made, feel, p.bpm, s.seed)),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                    },
                ],
                labels: Vec::new(),
            }),
            Err(e) => fail(id, e.words(), &mut notes),
        }
    }

    // Sixteen bars, beside a four-bar motif repeated; then two bars of it
    // tried again, each beside the bar made without looking ahead.
    let long_to = p.bars + 16;
    let s16 = settings(p.style, 16, next_seed());
    let req16 = request(p, &clip, &s16);
    match continue_notes(&req16) {
        Ok(made) => {
            out.push(Case {
                id: format!("{}-sixteen", p.slug),
                phrase: p.name.clone(),
                test: "keep",
                question: "Which would you keep?",
                about: format!(
                    "Continue by 16 bars in {}, beside a four-bar motif repeated with small random changes",
                    style_words(&p.style)
                ),
                takes: vec![
                    Take {
                        what: format!("mpfree, {}", style_words(&p.style)),
                        mpfree: true,
                        notes: whole(&yours, &made.notes),
                        from_bar: 0,
                        to_bar: long_to,
                        pad: None,
                    },
                    Take {
                        what: "a four-bar motif repeated with small random changes".into(),
                        mpfree: false,
                        notes: whole(&yours, &motif_repeated(&req16, &made)),
                        from_bar: 0,
                        to_bar: long_to,
                        pad: None,
                    },
                ],
                labels: Vec::new(),
            });
            // How often looking ahead changes the bar at all, over every
            // new bar but the last and 20 attempts each.
            let (mut same, mut tried) = (0, 0);
            let mut differ: Vec<(usize, u16)> = Vec::new();
            for bar in p.bars..long_to - 1 {
                for attempt in 1..=20u16 {
                    if let (Ok(a), Ok(b)) = (
                        try_bar(&req16, &made, bar, attempt),
                        try_bar_one_sided(&req16, &made, bar, attempt),
                    ) {
                        tried += 1;
                        if a.notes == b.notes {
                            same += 1;
                        } else {
                            differ.push((bar, attempt));
                        }
                    }
                }
            }
            let _ = writeln!(
                notes,
                "  Try again, both ways: the same bar in {same} of {tried} tries ({:.0}%); the pairs below are two of the tries where they differ",
                100.0 * same as f64 / tried.max(1) as f64
            );
            // Two of the tries that differ, a few bars apart, away from your bars.
            let first = differ
                .iter()
                .find(|d| d.0 >= p.bars + 2)
                .or(differ.first())
                .copied();
            let second = first.and_then(|f| {
                differ
                    .iter()
                    .find(|d| d.0 >= f.0 + 3)
                    .or(differ.iter().find(|d| d.0 != f.0))
                    .copied()
            });
            if second.is_none() {
                fail(
                    "try-bar",
                    format!("only {} of the tries differ", differ.len()),
                    &mut notes,
                );
            }
            for (n, pick) in [first, second].into_iter().enumerate() {
                let Some((bar, attempt)) = pick else { continue };
                let id = format!("try-bar-{}", n + 1);
                let found = match (
                    try_bar(&req16, &made, bar, attempt),
                    try_bar_one_sided(&req16, &made, bar, attempt),
                ) {
                    (Ok(a), Ok(b)) => Ok((attempt, a, b)),
                    (Err(e), _) | (_, Err(e)) => Err(e.words()),
                };
                match found {
                    Ok((attempt, two, one)) => out.push(Case {
                        id: format!("{}-{id}", p.slug),
                        phrase: p.name.clone(),
                        test: "try",
                        question: "In which does the middle bar belong between its neighbours?",
                        about: format!(
                            "Bar {} of {long_to} tried again (attempt {attempt}), heard with the bar before and after",
                            bar + 1
                        ),
                        takes: vec![
                            Take {
                                what: "tried again looking both ways (mpfree)".into(),
                                mpfree: true,
                                notes: whole(&yours, &two.notes),
                                from_bar: bar - 1,
                                to_bar: bar + 2,
                                pad: None,
                            },
                            Take {
                                what: "tried again without looking at the bar after".into(),
                                mpfree: false,
                                notes: whole(&yours, &one.notes),
                                from_bar: bar - 1,
                                to_bar: bar + 2,
                                pad: None,
                            },
                        ],
                        labels: Vec::new(),
                    }),
                    Err(e) => fail(&id, e, &mut notes),
                }
            }
        }
        Err(e) => fail("sixteen", e.words(), &mut notes),
    }

    // Do you hear your motif?
    let s = settings(p.style, 16, next_seed());
    match continue_notes(&request(p, &clip, &s)) {
        Ok(made) => out.push(Case {
            id: format!("{}-motif", p.slug),
            phrase: p.name.clone(),
            test: "motif",
            question: "Do you hear your motif?",
            about: format!("Continue by 16 bars in {}", style_words(&p.style)),
            takes: vec![Take {
                what: format!("mpfree, {}", style_words(&p.style)),
                mpfree: true,
                notes: whole(&yours, &made.notes),
                from_bar: 0,
                to_bar: long_to,
                pad: None,
            }],
            labels: Vec::new(),
        }),
        Err(e) => fail("motif", e.words(), &mut notes),
    }

    // About 32 bars: Continue by 16, then the result by 16 again.
    let seed_a = next_seed();
    let seed_b = next_seed();
    match long(p, &clip, &yours, seed_a, seed_b) {
        Ok((all, bars, chords)) => out.push(Case {
            id: format!("{}-long", p.slug),
            phrase: p.name.clone(),
            test: "long",
            question: "Listen through: how does it hold up?",
            about: format!(
                "Continue by 16 bars in {}, then the result by 16 again: {bars} bars, the chords as a quiet pad",
                style_words(&p.style)
            ),
            takes: vec![Take {
                what: format!("mpfree, {}", style_words(&p.style)),
                mpfree: true,
                notes: all,
                from_bar: 0,
                to_bar: bars,
                pad: Some(chords),
            }],
            labels: Vec::new(),
        }),
        Err(e) => fail("long", e, &mut notes),
    }

    Made {
        cases: out.into_iter().map(|c| finish(c, seed)).collect(),
        notes,
    }
}

fn long(
    p: &Phrase,
    clip: &Clip,
    yours: &[Note],
    seed_a: u64,
    seed_b: u64,
) -> Result<(Vec<Note>, usize, ChordTimeline), String> {
    let s = settings(p.style, 16, seed_a);
    let first = continue_notes(&request(p, clip, &s)).map_err(|e| e.words())?;
    let all = whole(yours, &first.notes);
    let bars = p.bars + 16;
    let as_played: Vec<PlainNote> = all
        .iter()
        .map(|n| PlainNote {
            at: n.tick,
            len: n.len,
            pitch: n.note,
            velocity: n.velocity,
        })
        .collect();
    let again = clip_of(&as_played, bars, p.meter);
    let s = settings(p.style, 16, seed_b);
    let second: Continued = continue_notes(&request(p, &again, &s)).map_err(|e| e.words())?;
    Ok((whole(&all, &second.notes), bars + 16, second.chords))
}

fn grid_words(grid: u32) -> &'static str {
    match grid {
        g if g == PPQ / 2 => "1/8",
        g if g == PPQ / 4 => "1/16",
        _ => "its grid",
    }
}

// ---------------------------------------------------------------- sound

/// Frames from the clip's start to a tick as heard (the clip's swing applied).
fn frame(p: &Phrase, tick: f64) -> u64 {
    let heard = p.swing.apply(tick);
    (heard / f64::from(PPQ) * 60.0 / p.bpm * f64::from(RATE))
        .round()
        .max(0.0) as u64
}

struct Part {
    node: Box<dyn Node>,
    gain: f32,
    /// (frame, note-off before note-on at the same frame, event)
    events: Vec<(u64, u8, NodeEvent)>,
}

impl Part {
    fn note(&mut self, on: u64, off: u64, note: u8, velocity: u8) {
        self.events
            .push((on, 1, NodeEvent::NoteOn { note, velocity }));
        self.events
            .push((off.max(on + 1), 0, NodeEvent::NoteOff { note }));
    }
}

fn melody_sound(role: PartRole) -> Box<dyn Node> {
    match role {
        PartRole::Bass => {
            let mut s = Pluck::new(RATE);
            s.load_preset(preset(Pluck::presets(), "Upright Bass", |p| p.name));
            Box::new(s)
        }
        _ => {
            let mut s = Keys::new(RATE);
            s.load_preset(preset(Keys::presets(), "Mellow Tines", |p| p.name));
            Box::new(s)
        }
    }
}

fn preset<'a, P>(all: &'a [P], name: &str, name_of: impl Fn(&P) -> &str) -> &'a P {
    all.iter()
        .find(|p| name_of(p) == name)
        .unwrap_or_else(|| panic!("no preset {name}"))
}

/// The case's take as heard: the melody, a plain beat, and the pad if any.
fn render(p: &Phrase, t: &Take) -> (Vec<f32>, Vec<f32>, Smf) {
    let bar = p.meter.bar_ticks();
    let from = t.from_bar as Tick * bar;
    let to = t.to_bar as Tick * bar;
    let start = frame(p, from as f64);
    let at = |tick: Tick| frame(p, tick as f64).saturating_sub(start);
    let mut melody = Part {
        node: melody_sound(p.role),
        gain: 1.0,
        events: Vec::new(),
    };
    let mut midi_melody: Vec<(u64, Msg)> = Vec::new();
    let heard_tick = |tick: Tick| {
        (p.swing.apply(tick as f64) - p.swing.apply(from as f64))
            .round()
            .max(0.0) as u64
    };
    for n in &t.notes {
        if n.tick < from || n.tick >= to {
            continue;
        }
        let end = (n.tick + n.len.max(1)).min(to);
        melody.note(at(n.tick), at(end), n.note, n.velocity);
        midi_melody.push((
            heard_tick(n.tick),
            Msg::NoteOn {
                ch: 0,
                note: n.note,
                vel: n.velocity,
            },
        ));
        midi_melody.push((
            heard_tick(end),
            Msg::NoteOff {
                ch: 0,
                note: n.note,
                vel: 0,
            },
        ));
    }
    // A plain beat, the same in every file of a phrase.
    let mut drums = Part {
        node: {
            let mut d = Drums::new(RATE);
            d.load_preset(preset(Drums::presets(), "Tight Kit", |k| k.name));
            Box::new(d)
        },
        gain: 0.45,
        events: Vec::new(),
    };
    let mut midi_drums: Vec<(u64, Msg)> = Vec::new();
    let beat = bar / Tick::from(p.meter.beats.max(1));
    let eighths = (bar / (PPQ as Tick / 2)).max(1);
    for b in t.from_bar..t.to_bar {
        let b0 = b as Tick * bar;
        let mut hits: Vec<(Tick, u8, u8)> = Vec::new();
        hits.push((b0, 36, 112));
        match p.meter.beats {
            4 => {
                hits.push((b0 + 2 * beat, 36, 100));
                hits.push((b0 + beat, 37, 104));
                hits.push((b0 + 3 * beat, 37, 108));
            }
            3 => {
                hits.push((b0 + beat, 37, 70));
                hits.push((b0 + 2 * beat, 37, 66));
            }
            n => hits.push((b0 + Tick::from(n.max(2) - 1) * beat, 37, 104)),
        }
        for e in 0..eighths {
            let tick = b0 + e * (PPQ as Tick / 2);
            hits.push((tick, 39, if e % 2 == 0 { 84 } else { 58 }));
        }
        for (tick, note, vel) in hits {
            let on = at(tick);
            drums.events.push((
                on,
                1,
                NodeEvent::NoteOn {
                    note,
                    velocity: vel,
                },
            ));
            drums
                .events
                .push((on + 2_000, 0, NodeEvent::NoteOff { note }));
            midi_drums.push((heard_tick(tick), Msg::NoteOn { ch: 9, note, vel }));
            midi_drums.push((
                heard_tick(tick) + 120,
                Msg::NoteOff {
                    ch: 9,
                    note,
                    vel: 0,
                },
            ));
        }
    }
    let mut parts = vec![melody, drums];
    // The chords, as a quiet pad, for the long listen.
    if let Some(chords) = &t.pad {
        let mut pad = Part {
            node: {
                let mut s = Strings::new(RATE);
                s.load_preset(preset(Strings::presets(), "Ballad Pad", |p| p.name));
                Box::new(s)
            },
            gain: 0.3,
            events: Vec::new(),
        };
        for span in chords.spans() {
            let Some(c) = span.answer.chord() else {
                continue;
            };
            let (s0, s1) = (span.start.max(from), span.end().min(to));
            if s0 >= s1 {
                continue;
            }
            // Close voicing from the root, between F3 and E4.
            let root = 53 + (i32::from(c.root) - 5).rem_euclid(12);
            for pc in 0..12 {
                if c.pcs() & (1 << pc) != 0 {
                    let pitch = root + (pc - i32::from(c.root)).rem_euclid(12);
                    pad.note(at(s0), at(s1), pitch as u8, 70);
                }
            }
        }
        parts.push(pad);
    }
    let frames = (at(to) as f64 + TAIL * f64::from(RATE)) as usize;
    let frames = frames.div_ceil(BLOCK) * BLOCK;
    let (mut left, mut right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut buf = Vec::new();
    for part in &mut parts {
        part.events.sort_by_key(|e| (e.0, e.1));
        part.node.prepare(RATE, BLOCK);
        let mut i = 0;
        for pos in (0..frames).step_by(BLOCK) {
            buf.clear();
            while i < part.events.len() && part.events[i].0 < (pos + BLOCK) as u64 {
                let (f, _, e) = part.events[i];
                buf.push(Timed::new((f.max(pos as u64) - pos as u64) as u32, e));
                i += 1;
            }
            l.fill(0.0);
            r.fill(0.0);
            part.node
                .process(&Ctx::new(RATE, pos as u64), &buf, &mut l, &mut r);
            for k in 0..BLOCK {
                left[pos + k] += l[k] * part.gain;
                right[pos + k] += r[k] * part.gain;
            }
        }
    }
    let mut tracks = vec![SmfTrack {
        name: if p.role == PartRole::Bass {
            "Bass"
        } else {
            "Melody"
        }
        .into(),
        events: midi_melody,
    }];
    tracks.push(SmfTrack {
        name: "Beat".into(),
        events: midi_drums,
    });
    let length = heard_tick(to);
    let smf = Smf {
        name: t.what.clone(),
        tempos: vec![(0, p.bpm)],
        meters: vec![(0, p.meter.beats, p.meter.unit)],
        tracks,
        end: length,
    };
    (left, right, smf)
}

/// The same fixed level for every file, with a soft knee instead of
/// clipping; the peak before it is reported.
const LEVEL: f32 = 2.0;

fn soft(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
    }
}

fn wav(left: &[f32], right: &[f32]) -> (Vec<u8>, f32) {
    let mut peak = 0.0f32;
    let mut pcm = Vec::with_capacity(left.len() * 4);
    for (a, b) in left.iter().zip(right) {
        for x in [a, b] {
            let v = x * LEVEL;
            peak = peak.max(v.abs());
            pcm.extend_from_slice(&((soft(v) * 32767.0).round() as i16).to_le_bytes());
        }
    }
    let mut out = Vec::with_capacity(44 + pcm.len());
    let data = pcm.len() as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.extend_from_slice(&pcm);
    (out, peak)
}

// ---------------------------------------------------------------- output

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn file_name(c: &Case, label: &str) -> String {
    if label.is_empty() {
        c.id.clone()
    } else {
        format!("{}-{label}", c.id)
    }
}

fn write_all(
    phrases: &[(Phrase, Made)],
    dir: &Path,
    seed: u64,
    header: &str,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    let mut report = String::new();
    let mut answers = String::new();
    let _ = writeln!(
        report,
        "mpfree composer listening set (phase 0C), seed {seed}\n"
    );
    let _ = writeln!(report, "{header}\n");
    let _ = writeln!(answers, "{{\"seed\": {seed}, \"cases\": [");
    let mut first = true;
    let mut peaks = 0.0f32;
    for (p, made) in phrases {
        report.push_str(&made.notes);
        for c in &made.cases {
            let _ = writeln!(report, "  {} ({}): {}", c.id, c.question, c.about);
            let mut files = Vec::new();
            for (t, label) in c.takes.iter().zip(&c.labels) {
                let name = file_name(c, label);
                let (l, r, smf) = render(p, t);
                let (bytes, peak) = wav(&l, &r);
                peaks = peaks.max(peak);
                let wav_path = dir.join(format!("{name}.wav"));
                std::fs::write(&wav_path, bytes)
                    .map_err(|e| format!("can't write {}: {e}", wav_path.display()))?;
                let mid_path = dir.join(format!("{name}.mid"));
                std::fs::write(&mid_path, smf.bytes())
                    .map_err(|e| format!("can't write {}: {e}", mid_path.display()))?;
                let secs = l.len() as f64 / f64::from(RATE);
                let _ = writeln!(
                    report,
                    "    {:<28} {:<54} bars {}-{}, {secs:.1} s, peak {:.1} dBFS",
                    format!("{name}.wav"),
                    t.what,
                    t.from_bar + 1,
                    t.to_bar,
                    20.0 * peak.max(1e-9).log10()
                );
                files.push(format!(
                    "{{\"label\": {}, \"file\": {}, \"what\": {}, \"mpfree\": {}, \"bars\": [{}, {}], \"seconds\": {secs:.1}}}",
                    json(label),
                    json(&name),
                    json(&t.what),
                    t.mpfree,
                    t.from_bar + 1,
                    t.to_bar
                ));
            }
            if !first {
                answers.push_str(",\n");
            }
            first = false;
            let _ = write!(
                answers,
                "  {{\"id\": {}, \"phrase\": {}, \"test\": {}, \"question\": {}, \"about\": {}, \"bpm\": {:.1}, \"files\": [{}]}}",
                json(&c.id),
                json(&c.phrase),
                json(c.test),
                json(c.question),
                json(&c.about),
                p.bpm,
                files.join(", ")
            );
        }
        report.push('\n');
    }
    let _ = writeln!(answers, "\n]}}");
    let _ = writeln!(
        report,
        "Every file is at the same level (x{LEVEL}, with a soft knee from -1.9 dBFS instead of clipping); the loudest peak was {:.1} dBFS.",
        20.0 * peaks.max(1e-9).log10()
    );
    std::fs::write(dir.join("report.txt"), &report).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("answers.json"), &answers).map_err(|e| e.to_string())?;
    print!("{report}");
    println!("Wrote {}", dir.display());
    Ok(())
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let s = secs % 86_400;
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

fn style_by_name(s: &str) -> Option<Style> {
    match s.to_ascii_lowercase().replace(['-', ' '], "").as_str() {
        "minimal" => Some(MINIMAL),
        "groove" => Some(GROOVE),
        "jazzish" | "jazz" => Some(JAZZISH),
        _ => None,
    }
}

fn swing_by_text(s: &str) -> Option<Swing> {
    let (grid, amount) = s.split_once(':')?;
    let grid = match grid {
        "1/8" => PPQ / 2,
        "1/16" => PPQ / 4,
        _ => return None,
    };
    Swing::new(grid, amount.parse().ok()?)
}

const USAGE: &str = "usage: listen --demo | --midi FILE [--track N] [--from BAR] [--bars N] [--all-notes] \
[--style minimal|groove|jazzish] [--bpm N] [--swing 1/8:58] [--role melody|bass] [--seed N] [--out DIR]";

fn run(args: &[String]) -> Result<(), String> {
    let mut demo = false;
    let mut midi: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut seed = 1u64;
    let mut track = None;
    let mut from = 1usize;
    let mut bars = 2usize;
    let mut all_notes = false;
    let mut style = None;
    let mut bpm = None;
    let mut swing = Swing::default();
    let mut role = PartRole::Melody;
    let mut i = 0;
    let value = |i: &mut usize| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("{} needs a value\n{USAGE}", args[*i - 1]))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--demo" => demo = true,
            "--midi" => midi = Some(PathBuf::from(value(&mut i)?)),
            "--out" => out = Some(PathBuf::from(value(&mut i)?)),
            "--seed" => {
                seed = value(&mut i)?
                    .parse()
                    .map_err(|_| "--seed takes a number")?
            }
            "--track" => {
                track = Some(
                    value(&mut i)?
                        .parse()
                        .map_err(|_| "--track takes a number")?,
                )
            }
            "--from" => {
                from = value(&mut i)?
                    .parse()
                    .map_err(|_| "--from takes a bar number")?
            }
            "--bars" => {
                bars = value(&mut i)?
                    .parse()
                    .map_err(|_| "--bars takes a number")?
            }
            "--all-notes" => all_notes = true,
            "--style" => {
                let v = value(&mut i)?;
                style = Some(
                    style_by_name(&v).ok_or(format!("no style {v}: minimal, groove or jazzish"))?,
                );
            }
            "--bpm" => {
                let v: f64 = value(&mut i)?.parse().map_err(|_| "--bpm takes a number")?;
                if !(20.0..=300.0).contains(&v) {
                    return Err("--bpm takes 20 to 300".into());
                }
                bpm = Some(v);
            }
            "--swing" => {
                let v = value(&mut i)?;
                swing = swing_by_text(&v).ok_or(format!(
                    "--swing {v}: 1/8 or 1/16, then 50 to 75, as 1/8:58"
                ))?;
            }
            "--role" => {
                role = match value(&mut i)?.as_str() {
                    "melody" => PartRole::Melody,
                    "bass" => PartRole::Bass,
                    v => return Err(format!("--role {v}: melody or bass")),
                }
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
        i += 1;
    }
    if demo == midi.is_some() {
        return Err(format!("give --demo or --midi FILE\n{USAGE}"));
    }
    if !(1..=MOST_BARS).contains(&bars) {
        return Err(format!("--bars takes 1 to {MOST_BARS}"));
    }
    let dir = out.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        Path::new(&home)
            .join("mpfree-work")
            .join(format!("compose-listen-{}", utc_stamp()))
    });
    if dir.exists() && std::fs::read_dir(&dir).map_or(true, |mut d| d.next().is_some()) {
        return Err(format!(
            "{} is already there and not empty: give a new --out",
            dir.display()
        ));
    }
    let (phrases, header) = if let Some(path) = midi {
        let (mut p, said) = midi_phrase(
            &path,
            track,
            from,
            bars,
            all_notes,
            bpm,
            swing,
            style.unwrap_or(MINIMAL),
            role,
        )?;
        p.role = role;
        (vec![p], format!("From {said}."))
    } else {
        let mut ps = demo_phrases();
        for p in &mut ps {
            if let Some(s) = style {
                p.style = s;
            }
            if let Some(b) = bpm {
                p.bpm = b;
            }
        }
        (ps, "The four built-in phrases.".to_string())
    };
    let made: Vec<(Phrase, Made)> = phrases
        .into_iter()
        .map(|p| {
            let m = cases(&p, seed);
            (p, m)
        })
        .collect();
    write_all(&made, &dir, seed, &header)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("listen: {e}");
        std::process::exit(2);
    }
}
