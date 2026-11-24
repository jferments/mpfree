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
//! melody|bass`, `--sounds a|b|now|round2|plain` (which sounds play: the
//! two sets on trial in S0, the set round 3 used, the set round 2 used, or
//! plain keys with no drums, pad or effects, to check the notes alone).
//! Every file is brought to the same loudness (-18 LUFS), so files compare
//! fairly; with `a`, `b` and `plain` each part is also set by how loud it
//! sounds against the melody, not by a fixed gain. With `EXPLAIN=1` in the
//! environment, the long listen's two runs are explained on stderr: the key
//! read, the chords, and each bar's plan with every note's role.
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
//! Every file of a phrase plays on the same sounds at the same level (no
//! file is made louder than another), picked for the phrase's style: the
//! melody on Keys "Funk Clav" (Groove), Keys "Soft Tines" (Minimal),
//! Wind "Warm Clarinet" (a waltz) or Wind "Tenor Sax" (Jazz-ish), a bass
//! on Pluck "Upright Bass"; a drummer on the style's kit (House, Boom Bap
//! or Tight) with fills, ghost notes and crashes; a plate reverb and a
//! dotted-1/8 echo; and in `long` only the chords on Strings "Ballad Pad"
//! and a bass line from them (the chord's own third and fifth). Each half
//! of the long listen plays over the chords it was written over. The
//! pairs have no chords under them, so neither side is helped by the
//! composer's own.
//! `--parts` makes the Add a part listening gate (S2) instead: rows
//! played twice through, three ways, blind (parts added by role, every part
//! on Support, and the root on every kick with a block chord on each
//! change). With `--demo` the rows are the riff with its beat, the tune
//! alone, the riff's beat alone and the swung line with its beat; with
//! `--midi` your clip with the style's beat.
//!
//! `--phrases` is another set altogether: the arp's phrases as they play,
//! one file each (`examples/listen/phrases.rs`). The arp's own engine
//! plays every phrase MPFree ships (or, with `--phrases-from FILE`, every
//! phrase of a library file) from a held note that moves through the key,
//! on a sound, at a tempo and over a beat picked for it, through this
//! tool's mix. `--only TEXT` keeps the phrases whose name or kind holds
//! the text; `EXPLAIN=1` prints the notes each one played. It writes the
//! WAVs, report.txt and phrases.json (for the listening page).
//!
//! `--ideas` is the blind listen for the composer's phrases (P2,
//! `examples/listen/ideas.rs`): twelve pairs, every kind of phrase in
//! every style, each pair one request made two ways, the composer's idea
//! and the simple way of its kind, heard as `--phrases` plays a phrase,
//! in one setting, shuffled into A and B. Every pair takes `--seed` as it
//! is, so none is picked. It writes the WAVs, report.txt and answers.json
//! (which is which, each phrase's text and what its checks found).
//!
//! It never touches any device: it reads only the MIDI or library file
//! given and writes only into its one new folder.
#![forbid(unsafe_code)]

use mpfree_compose::baseline::{
    motif_repeated, regroove, simpler, try_bar_one_sided, Feel, Simpler,
};
use mpfree_compose::critic::{judge_continue, Verdict};
use mpfree_compose::draft::drafts;
use mpfree_compose::style::{BassLine, DrumMap};
use mpfree_compose::{
    add_part, continue_notes, first_engine, safe_controls, sound_of, try_bar, AddPart, Added,
    Continue, Continued, Control, DrumHit, Heard, InstrumentKind, Knob, PlainNote, Relation, Role,
    Row, RowPart, Settings, SoundLevel, SoundMove, Source, Style, ToHear, GROOVE, JAZZISH, MINIMAL,
};
use mpfree_context::chord::{Chord, ChordSpan, ChordTimeline, Heard as ChordHeard, Quality, Tone};
use mpfree_context::groove::{Lane, STEP};
use mpfree_context::listen::{listen, Played};
use mpfree_context::{MusicalContext, PartRole, ReadBudget};
use mpfree_fx::ids::plate;
use mpfree_fx::{Delay, Plate};
use mpfree_midi::smf::{self, Smf, SmfTrack};
use mpfree_midi::Msg;
use mpfree_node::{Ctx, Curve, Node, NodeEvent, Timed};
use mpfree_seq::Note;
use mpfree_synth::{Bass, Drums, Keys, Pluck, Poly, Strings, Wind};
use mpfree_transport::time::{Meter, Swing, Tick, PPQ};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[path = "listen/phrases.rs"]
mod phrases;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;

/// Which sounds the files play (`--sounds`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sounds {
    /// The two sets on trial in S0, each part set by how loud it sounds.
    A,
    B,
    /// The sounds and fixed gains of round 3, and of round 2.
    Now,
    Round2,
    /// Melody and bass on plain keys, no drums, pad or effects.
    Plain,
}

static SOUNDS: std::sync::OnceLock<Sounds> = std::sync::OnceLock::new();

fn sounds() -> Sounds {
    *SOUNDS.get().unwrap_or(&Sounds::A)
}

/// `--lead NAME`: the melody on the synth's preset NAME (a filtered lead
/// for the sound moves' gate) in place of the set's.
static LEAD: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The loudness every file is brought to, in LUFS.
const FILE_LUFS: f64 = -18.0;
/// Seconds after the last bar, for the release.
const TAIL: f64 = 2.0;
/// Bars Continue adds in the short cases.
const SHORT: u8 = 4;
/// The most bars read from a MIDI file.
const MOST_BARS: usize = 8;

// ---------------------------------------------------------------- phrases

/// A clip to carry on: its notes as written, and how it plays.
#[derive(Clone)]
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
        row_length: 0,
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
    /// Parts added to a row (S2), each on its own sound.
    extra: Vec<Extra>,
    /// Whether the phrase's drummer plays.
    drummer: bool,
    /// The melody's sound moves (S3), on its preset's safe controls.
    moves: Vec<SoundMove>,
}

/// What an added part is, for its sound.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExtraKind {
    Bass,
    Chords,
    Drums,
}

/// A note of an added part: (start, length, pitch, velocity), in ticks
/// from the clip's start.
type NoteOf = (Tick, Tick, u8, u8);

/// An added part's notes.
struct Extra {
    kind: ExtraKind,
    notes: Vec<NoteOf>,
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

fn finish(c: Case, seed: u64) -> Case {
    let ord = order(c.takes.len(), seed, &c.id);
    finish_in(c, &ord)
}

/// Three takes over several cases in a Latin square, so no way sits in the
/// same place in every case: case `k` of a set, the square turned by the
/// seed.
fn finish_square(c: Case, seed: u64, k: usize) -> Case {
    const SQUARES: [[usize; 3]; 6] = [
        [0, 1, 2],
        [2, 0, 1],
        [1, 2, 0],
        [0, 2, 1],
        [1, 0, 2],
        [2, 1, 0],
    ];
    if c.takes.len() != 3 {
        return finish(c, seed);
    }
    let turn = order(6, seed, "square")[0];
    let ord = SQUARES[3 * ((k / 3 + turn) % 2) + (k + turn) % 3];
    finish_in(c, &ord)
}

fn finish_in(mut c: Case, ord: &[usize]) -> Case {
    let n = c.takes.len();
    if n > 1 {
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
                        extra: Vec::new(),
                        drummer: true,
                        moves: Vec::new(),
                    },
                    Take {
                        what: words.to_string(),
                        mpfree: false,
                        notes: whole(&yours, &simpler(&req, way)),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                        extra: Vec::new(),
                        drummer: true,
                        moves: Vec::new(),
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
                        extra: Vec::new(),
                        drummer: true,
                        moves: Vec::new(),
                    },
                    Take {
                        what: format!("random notes in {}'s scale and rhythm", style.name),
                        mpfree: false,
                        notes: whole(&yours, &simpler(&req, Simpler::Random)),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                        extra: Vec::new(),
                        drummer: true,
                        moves: Vec::new(),
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
                    extra: Vec::new(),
                    drummer: true,
                    moves: Vec::new(),
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
                        extra: Vec::new(),
                        drummer: true,
                        moves: Vec::new(),
                    },
                    Take {
                        what: words.clone(),
                        mpfree: false,
                        notes: whole(&yours, &regroove(&made, feel, p.bpm, s.seed)),
                        from_bar: 0,
                        to_bar: short_to,
                        pad: None,
                        extra: Vec::new(),
                        drummer: true,
                        moves: Vec::new(),
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
                        extra: Vec::new(),
                        drummer: true,
                        moves: Vec::new(),
                    },
                    Take {
                        what: "a four-bar motif repeated with small random changes".into(),
                        mpfree: false,
                        notes: whole(&yours, &motif_repeated(&req16, &made)),
                        from_bar: 0,
                        to_bar: long_to,
                        pad: None,
                        extra: Vec::new(),
                        drummer: true,
                        moves: Vec::new(),
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
                                extra: Vec::new(),
                                drummer: true,
                                moves: Vec::new(),
                            },
                            Take {
                                what: "tried again without looking at the bar after".into(),
                                mpfree: false,
                                notes: whole(&yours, &one.notes),
                                from_bar: bar - 1,
                                to_bar: bar + 2,
                                pad: None,
                                extra: Vec::new(),
                                drummer: true,
                                moves: Vec::new(),
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
                extra: Vec::new(),
                drummer: true,
                moves: Vec::new(),
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
                extra: Vec::new(),
                drummer: true,
                moves: Vec::new(),
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

/// `--gate`: the S1 listening gate for one phrase. Your bars, then 8 new
/// bars three ways, blind, each with its own chords as a quiet pad and
/// bass: music first (S1), the first engine, and your bars looped over
/// your chords looped (the plain loop to beat).
fn gate_cases(p: &Phrase, seed: u64) -> Made {
    const NEW: u8 = 8;
    let clip = clip_of(&p.notes, p.bars, p.meter);
    let yours = yours_as_notes(&p.notes);
    let mut notes = String::new();
    let _ = writeln!(
        notes,
        "{}: {} bars of {} at {:.0} BPM; read as {}; style {}; seed {seed}",
        p.name,
        p.bars,
        meter_words(p.meter),
        p.bpm,
        clip.context.key.words(),
        style_words(&p.style)
    );
    let s = settings(p.style, NEW, seed);
    let req = request(p, &clip, &s);
    let to_bar = p.bars + usize::from(NEW);
    let made = (|| -> Result<Case, String> {
        let s1 = drafts(&req).map_err(|e| e.words())?.remove(0);
        let first = first_engine(&req).map_err(|e| e.words())?;
        let _ = writeln!(
            notes,
            "  S1 draft {}: {}\n  S1 critic: {}\n  first engine critic: {}",
            s1.number + 1,
            s1.sketch
                .gestures
                .iter()
                .map(|g| g.words())
                .collect::<Vec<_>>()
                .join(", "),
            s1.verdict.words(),
            judge_continue(&req, &first.notes, &first.chords).words()
        );
        if std::env::var_os("EXPLAIN").is_some() {
            show(p, "S1", &s1.made, &s1.sketch.gestures);
            show(p, "first", &first, &[]);
        }
        let looped = simpler(&req, Simpler::Repeat);
        let take = |what: &str, mpfree: bool, new: &[Note], pad: ChordTimeline| Take {
            what: what.to_string(),
            mpfree,
            notes: whole(&yours, new),
            from_bar: 0,
            to_bar,
            pad: Some(pad),
            extra: Vec::new(),
            drummer: true,
            moves: Vec::new(),
        };
        Ok(Case {
            id: format!("{}-gate", p.slug),
            phrase: p.name.clone(),
            test: "gate",
            question: "Which would you keep?",
            about: format!(
                "your {} bars, then 8 new bars in {}",
                p.bars,
                style_words(&p.style)
            ),
            takes: vec![
                take(
                    "music first (S1)",
                    true,
                    &s1.made.notes,
                    s1.made.chords.clone(),
                ),
                take("first engine", true, &first.notes, first.chords.clone()),
                take(
                    "your bars looped, over your chords looped",
                    false,
                    &looped,
                    looped_chords(
                        &first.chords,
                        p.bars as Tick * p.meter.bar_ticks(),
                        to_bar as Tick * p.meter.bar_ticks(),
                    ),
                ),
            ],
            labels: Vec::new(),
        })
    })();
    let cases = match made {
        Ok(c) => vec![finish(c, seed)],
        Err(e) => {
            let _ = writeln!(notes, "  gate: left out, {e}");
            Vec::new()
        }
    };
    Made { cases, notes }
}

// ------------------------------------------------------------ S3: playing

/// The S3 gate: the same new bars (the same notes, places and chords)
/// played with the phrase and the instrument ([`mpfree_compose::perform`])
/// and played as before, the style's gesture at 0. Two cases per phrase,
/// on two seeds, so the pairs are not all one tune.
fn gestures_cases(p: &Phrase, seed: u64) -> Made {
    const NEW: u8 = 8;
    let clip = clip_of(&p.notes, p.bars, p.meter);
    let yours = yours_as_notes(&p.notes);
    let mut notes = String::new();
    let to_bar = p.bars + usize::from(NEW);
    let mut cases = Vec::new();
    for k in 0..2u64 {
        let sd = seed + k;
        let _ = writeln!(
            notes,
            "{}: {} bars of {} at {:.0} BPM; read as {}; style {}; seed {sd}",
            p.name,
            p.bars,
            meter_words(p.meter),
            p.bpm,
            clip.context.key.words(),
            style_words(&p.style)
        );
        let played = settings(p.style, NEW, sd);
        let mut flat_style = p.style;
        flat_style.playing.gesture = 0.0;
        let typed = settings(flat_style, NEW, sd);
        let made = (|| -> Result<Case, String> {
            let a = continue_notes(&request(p, &clip, &played)).map_err(|e| e.words())?;
            let b = continue_notes(&request(p, &clip, &typed)).map_err(|e| e.words())?;
            let same = a.notes.len() == b.notes.len()
                && a.notes.iter().zip(&b.notes).all(|(x, y)| x.note == y.note);
            if !same {
                return Err("the two ways made different notes".into());
            }
            let moved = |c: &Continued| {
                c.part
                    .decisions
                    .iter()
                    .map(|d| f64::from(d.timing.play).abs())
                    .fold(0.0, f64::max)
            };
            let spread = |c: &Continued| {
                let v: Vec<f64> = c.notes.iter().map(|n| f64::from(n.velocity)).collect();
                let m = v.iter().sum::<f64>() / v.len().max(1) as f64;
                (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len().max(1) as f64).sqrt()
            };
            let _ = writeln!(
                notes,
                "  {} notes; played: loudness spread {:.1}, timing moved up to {:.1} ms; as before: spread {:.1}",
                a.notes.len(),
                spread(&a),
                moved(&a) * 60_000.0 / (p.bpm * f64::from(PPQ)),
                spread(&b),
            );
            let take = |what: &str, new: &[Note], pad: ChordTimeline| Take {
                what: what.to_string(),
                mpfree: true,
                notes: whole(&yours, new),
                from_bar: 0,
                to_bar,
                pad: Some(pad),
                extra: Vec::new(),
                drummer: true,
                moves: Vec::new(),
            };
            Ok(Case {
                id: format!("{}-gestures-{}", p.slug, k + 1),
                phrase: p.name.clone(),
                test: "gestures",
                question: "Which would you keep?",
                about: format!(
                    "your {} bars, then the same 8 new bars in {}, played two ways",
                    p.bars,
                    style_words(&p.style)
                ),
                takes: vec![
                    take("played with the phrase (S3)", &a.notes, a.chords.clone()),
                    take("played as before", &b.notes, b.chords.clone()),
                ],
                labels: Vec::new(),
            })
        })();
        // One of each phrase's two pairs has the new playing first, the
        // other second, which one by the seed.
        let first = order(2, seed, &p.slug)[0] as u64 == k;
        match made {
            Ok(c) => cases.push(finish_in(c, if first { &[0, 1] } else { &[1, 0] })),
            Err(e) => {
                let _ = writeln!(notes, "  gestures: left out, {e}");
            }
        }
    }
    Made { cases, notes }
}

/// S3's second gate: the same new bars, played the same way, with the
/// sound following the phrase ([`mpfree_compose::sound`]) and with the
/// preset as it is. Two cases per phrase: one at Subtle (the default),
/// one at Bold, on two seeds.
fn sound_cases(p: &Phrase, seed: u64) -> Made {
    const NEW: u8 = 8;
    let clip = clip_of(&p.notes, p.bars, p.meter);
    let yours = yours_as_notes(&p.notes);
    let mut notes = String::new();
    let to_bar = p.bars + usize::from(NEW);
    let mut cases = Vec::new();
    let safe = safe_of(melody_sound(p).as_ref());
    for (k, level) in [SoundLevel::Subtle, SoundLevel::Bold]
        .into_iter()
        .enumerate()
    {
        let k = k as u64;
        let sd = seed + k;
        let _ = writeln!(
            notes,
            "{}: {} bars of {} at {:.0} BPM; read as {}; style {}; seed {sd}; sound {}",
            p.name,
            p.bars,
            meter_words(p.meter),
            p.bpm,
            clip.context.key.words(),
            style_words(&p.style),
            level.name()
        );
        let set = settings(p.style, NEW, sd);
        let made = (|| -> Result<Case, String> {
            let req = request(p, &clip, &set);
            let a = continue_notes(&req).map_err(|e| e.words())?;
            let moves = sound_of(&req, &a, level);
            if moves.is_empty() {
                return Err("no sound moves".into());
            }
            let count = |c: Control| moves.iter().filter(|m| m.control == c).count();
            let turned = |c: Control| match (c, safe.get(c)) {
                (Control::Space, _) => "the echo send".to_string(),
                (_, Some(k)) => format!("knob {} from {:.0}", k.id, k.base),
                (_, None) => "nothing on this preset".to_string(),
            };
            let _ = writeln!(
                notes,
                "  {} notes; moves: brightness {} ({}), length {} ({}), space {} ({})",
                a.notes.len(),
                count(Control::Brightness),
                turned(Control::Brightness),
                count(Control::Length),
                turned(Control::Length),
                count(Control::Space),
                turned(Control::Space),
            );
            let take = |what: &str, moves: Vec<SoundMove>| Take {
                what: what.to_string(),
                mpfree: true,
                notes: whole(&yours, &a.notes),
                from_bar: 0,
                to_bar,
                pad: Some(a.chords.clone()),
                extra: Vec::new(),
                drummer: true,
                moves,
            };
            Ok(Case {
                id: format!("{}-sound-{}", p.slug, level.name().to_ascii_lowercase()),
                phrase: p.name.clone(),
                test: "sound",
                question: "Which would you keep?",
                about: format!(
                    "your {} bars, then the same 8 new bars in {}, the same playing, the sound two ways",
                    p.bars,
                    style_words(&p.style)
                ),
                takes: vec![
                    take(&format!("sound moving with the phrase (S3, {})", level.name()), moves),
                    take("the preset as it is", Vec::new()),
                ],
                labels: Vec::new(),
            })
        })();
        // One of each phrase's two pairs has the moving sound first, the
        // other second, which one by the seed.
        let first = order(2, seed, &p.slug)[0] as u64 == k;
        match made {
            Ok(c) => cases.push(finish_in(c, if first { &[0, 1] } else { &[1, 0] })),
            Err(e) => {
                let _ = writeln!(notes, "  sound: left out, {e}");
            }
        }
    }
    Made { cases, notes }
}

// ------------------------------------------------------------ S2: parts

/// A row for the parts gate: a phrase looped to four bars or more, with
/// its tune or without, with the phrase's beat or without.
struct GateRow {
    phrase: Phrase,
    tune: bool,
    beat: bool,
    /// Chords picked for the row, one per bar of the phrase, as you would
    /// set them on its Harmony line; none to play over the chords heard.
    picked: Vec<Chord>,
}

/// The rows `--parts --demo` plays: the riff with its beat, the F major
/// tune alone, the riff's beat alone, and the swung line with its beat.
/// The riff's chords are picked (Cm7, then Fm7: heard alone, the riff reads
/// as one Fm7 for both bars), so the parts have a change to lead into; the
/// others play over the chords heard.
fn gate_rows(phrases: Vec<Phrase>) -> Vec<GateRow> {
    let riff_chords = vec![
        Chord::new(0, Quality::Minor7),
        Chord::new(5, Quality::Minor7),
    ];
    let mut rows = Vec::new();
    for p in phrases {
        match p.slug.as_str() {
            "riff" => {
                rows.push(GateRow {
                    phrase: Phrase {
                        slug: "riff".into(),
                        name: format!("{}, with its beat", p.name),
                        ..p.clone()
                    },
                    tune: true,
                    beat: true,
                    picked: riff_chords.clone(),
                });
                rows.push(GateRow {
                    phrase: Phrase {
                        slug: "beat".into(),
                        name: "the riff's beat alone".into(),
                        ..p
                    },
                    tune: false,
                    beat: true,
                    picked: riff_chords.clone(),
                });
            }
            "tune" => rows.push(GateRow {
                phrase: Phrase {
                    name: format!("{}, alone", p.name),
                    ..p
                },
                tune: true,
                beat: false,
                picked: Vec::new(),
            }),
            "swung" => rows.push(GateRow {
                phrase: Phrase {
                    name: format!("{}, with its beat", p.name),
                    ..p
                },
                tune: true,
                beat: true,
                picked: Vec::new(),
            }),
            _ => {}
        }
    }
    rows
}

/// The drummer's hits over `bars` bars, by lane, as a row hears them.
fn beat_hits(p: &Phrase, bars: usize) -> Vec<DrumHit> {
    let bar = p.meter.bar_ticks();
    let mut v = Vec::new();
    for b in 0..bars {
        for (tick, note, velocity) in drummer(p, b) {
            let lane = match note {
                KICK => Lane::Kick,
                SNARE | CLAP | RIM => Lane::Snare,
                HAT | OPEN_HAT | RIDE | CYMBAL => Lane::Hats,
                _ => Lane::Other,
            };
            v.push(DrumHit {
                at: b as Tick * bar + tick,
                lane,
                velocity,
            });
        }
    }
    v
}

/// The pads a drum part added to the row plays: the phrase's kit, the
/// ride for Jazz-ish's hats and the clap for the house kit's backbeat.
fn kit_pads(p: &Phrase) -> [Option<u8>; 4] {
    let (snare, hats) = match p.style.bass.drums {
        DrumMap::Ride => (SNARE, RIDE),
        DrumMap::FourOnTheFloor => (CLAP, HAT),
        _ => (SNARE, HAT),
    };
    [Some(KICK), Some(snare), Some(hats), Some(TOM_MID)]
}

/// What the row is heard from, kept so each added part can be heard by
/// the next.
struct RowSource<'a> {
    p: &'a Phrase,
    parts: Vec<RowPart>,
    length: Tick,
    context: &'a MusicalContext,
}

impl RowSource<'_> {
    /// Add a part by `relation` to the row as it is, keep it in the row,
    /// and say what the critic found.
    fn add(
        &mut self,
        part: PartRole,
        relation: Relation,
        seed: u64,
        notes: &mut String,
    ) -> Result<Added, String> {
        let row = Row::hear(&ToHear {
            parts: &self.parts,
            length: self.length,
            meter: self.p.meter,
            key: &self.context.key,
            played_pcs: self.context.played_pcs,
            chords: &self.context.chords,
            groove: None,
            lead: None,
        });
        let s = settings(self.p.style, 8, seed);
        let a = add_part(&AddPart {
            row: &row,
            part,
            relation,
            settings: &s,
            kind: match part {
                PartRole::Bass => InstrumentKind::Bass,
                PartRole::Chords => InstrumentKind::Chords,
                PartRole::Drums => InstrumentKind::Percussive,
                PartRole::Melody => InstrumentKind::OneNote,
            },
            pads: kit_pads(self.p),
            swing: self.p.swing,
        })
        .map_err(|e| e.words().to_string())?;
        let _ = writeln!(
            notes,
            "    {part:?} on {}: draft {}, {} notes; {}",
            relation.words(),
            a.draft + 1,
            a.events.len(),
            a.verdict.words().replace('\n', "; ")
        );
        self.parts.push(RowPart {
            role: part,
            heard: if part == PartRole::Drums {
                Heard::Hits(
                    a.events
                        .iter()
                        .map(|e| DrumHit {
                            at: e.at,
                            lane: match e.role {
                                Role::Drum(l) => l,
                                _ => Lane::Other,
                            },
                            velocity: e.velocity,
                        })
                        .collect(),
                )
            } else {
                Heard::Notes(
                    a.events
                        .iter()
                        .map(|e| PlainNote {
                            at: e.at,
                            len: e.len,
                            pitch: e.pitch,
                            velocity: e.velocity,
                        })
                        .collect(),
                )
            },
        });
        Ok(a)
    }
}

/// A part's notes over `passes` times through a row `length` long.
fn looped(notes: impl Iterator<Item = NoteOf> + Clone, length: Tick, passes: usize) -> Vec<NoteOf> {
    (0..passes as Tick)
        .flat_map(|k| {
            notes
                .clone()
                .map(move |(at, len, p, v)| (at + k * length, len, p, v))
        })
        .collect()
}

fn events_of(a: &Added) -> impl Iterator<Item = NoteOf> + Clone + '_ {
    a.events.iter().map(|e| (e.at, e.len, e.pitch, e.velocity))
}

/// The simple way: the chord's root on every kick, held to the next, and
/// a block chord held through each chord.
fn simple_parts(
    chords: &ChordTimeline,
    kicks: &[Tick],
    length: Tick,
) -> (Vec<NoteOf>, Vec<NoteOf>) {
    let mut bass = Vec::new();
    for (i, &k) in kicks.iter().enumerate() {
        let next = kicks.get(i + 1).copied().unwrap_or(length);
        let Some(c) = chords.span_at(k).and_then(|sp| sp.answer.chord()) else {
            continue;
        };
        let pitch = 36 + c.root;
        bass.push((k, ((next - k) * 9 / 10).max(STEP), pitch, 100));
    }
    let mut block = Vec::new();
    for span in chords.spans() {
        let Some(c) = span.answer.chord() else {
            continue;
        };
        // Root position from C3 to B3, under most tunes.
        let root = 48 + i32::from(c.root);
        for pc in 0..12 {
            if c.pcs() & (1 << pc) != 0 {
                let pitch = root + (pc - i32::from(c.root)).rem_euclid(12);
                block.push((
                    span.start,
                    (span.end().min(length) - span.start - STEP / 2).max(STEP),
                    pitch as u8,
                    76,
                ));
            }
        }
    }
    (bass, block)
}

/// `--parts`: the S2 listening gate for one row, played twice through,
/// three ways, blind: parts added by S2 (a bass on Support, chords on
/// Answer, or on Interlock with no tune, and over a tune alone the drums
/// first, on Support), the simple way (the chord's root on every kick and
/// a block chord on each change, with the phrase's own drummer), and S2
/// with every part on Support (roles off).
fn parts_cases(r: &GateRow, seed: u64, k: usize) -> Made {
    const PASSES: usize = 2;
    let p = &r.phrase;
    let reps = 4usize.div_ceil(p.bars).max(1);
    let bars = p.bars * reps;
    let bar = p.meter.bar_ticks();
    let length = bars as Tick * bar;
    let phrase_len = p.bars as Tick * bar;
    let tune: Vec<PlainNote> = (0..reps as Tick)
        .flat_map(|k| {
            p.notes.iter().map(move |n| PlainNote {
                at: n.at + k * phrase_len,
                ..*n
            })
        })
        .collect();
    // The chords: the phrase's own read, or the ones picked, looped with
    // it (contract 4).
    let mut clip = clip_of(&p.notes, p.bars, p.meter);
    let chords = if r.picked.is_empty() {
        clip.context.chords.clone()
    } else {
        let v: Vec<(Chord, Tick)> = r.picked.iter().map(|&c| (c, bar)).collect();
        ChordTimeline::of(&v, ChordHeard::Set)
    };
    clip.context.chords = looped_chords(&chords, phrase_len, length);
    let beat = beat_hits(p, bars);
    let mut notes = String::new();
    let chord_words = format!(
        "{} {}",
        if r.picked.is_empty() {
            "chords heard:"
        } else {
            "chords picked:"
        },
        chords.names()
    );
    let _ = writeln!(
        notes,
        "{}: {bars} bars of {} at {:.0} BPM; read as {}; {chord_words}; style {}; seed {seed}",
        p.name,
        meter_words(p.meter),
        p.bpm,
        clip.context.key.words(),
        style_words(&p.style)
    );
    let start = || -> RowSource {
        let mut parts = Vec::new();
        if r.tune {
            parts.push(RowPart {
                role: PartRole::Melody,
                heard: Heard::Notes(tune.clone()),
            });
        }
        if r.beat {
            parts.push(RowPart {
                role: PartRole::Drums,
                heard: Heard::Hits(beat.clone()),
            });
        }
        RowSource {
            p,
            parts,
            length,
            context: &clip.context,
        }
    };
    let melody: Vec<Note> = if r.tune {
        yours_as_notes(&looped_plain(&tune, length, PASSES))
    } else {
        Vec::new()
    };
    let to_bar = bars * PASSES;
    let take = |what: &str, mpfree: bool, extra: Vec<Extra>, drummer: bool| Take {
        what: what.to_string(),
        mpfree,
        notes: melody.clone(),
        from_bar: 0,
        to_bar,
        pad: None,
        extra,
        drummer,
        moves: Vec::new(),
    };
    let made = (|| -> Result<Case, String> {
        let chord_role = if r.tune {
            Relation::Answer
        } else {
            Relation::Interlock
        };
        let mut ways = Vec::new();
        for (what, roles_off) in [("S2 parts", false), ("S2, every part on Support", true)] {
            let _ = writeln!(notes, "  {what}:");
            let mut src = start();
            let mut extra = Vec::new();
            if !r.beat {
                let d = src.add(PartRole::Drums, Relation::Support, seed, &mut notes)?;
                extra.push(Extra {
                    kind: ExtraKind::Drums,
                    notes: looped(events_of(&d), length, PASSES),
                });
            }
            let b = src.add(PartRole::Bass, Relation::Support, seed, &mut notes)?;
            let c = src.add(
                PartRole::Chords,
                if roles_off {
                    Relation::Support
                } else {
                    chord_role
                },
                seed,
                &mut notes,
            )?;
            extra.push(Extra {
                kind: ExtraKind::Bass,
                notes: looped(events_of(&b), length, PASSES),
            });
            extra.push(Extra {
                kind: ExtraKind::Chords,
                notes: looped(events_of(&c), length, PASSES),
            });
            ways.push(take(what, true, extra, r.beat));
        }
        let mut kicks: Vec<Tick> = beat
            .iter()
            .filter(|h| h.lane == Lane::Kick)
            .map(|h| h.at)
            .collect();
        kicks.sort_unstable();
        kicks.dedup();
        let (bass, block) = simple_parts(&clip.context.chords, &kicks, length);
        let simple = take(
            "the simple way: the root on every kick, a block chord on each change",
            false,
            vec![
                Extra {
                    kind: ExtraKind::Bass,
                    notes: looped(bass.iter().copied(), length, PASSES),
                },
                Extra {
                    kind: ExtraKind::Chords,
                    notes: looped(block.iter().copied(), length, PASSES),
                },
            ],
            true,
        );
        let roles_off = ways.pop().expect("two ways");
        let s2 = ways.pop().expect("two ways");
        Ok(Case {
            id: format!("{}-parts", p.slug),
            phrase: p.name.clone(),
            test: "parts",
            question: "Which would you keep?",
            about: format!(
                "{} in {}, {bars} bars twice through, with a bass and chords{}; {chord_words}",
                p.name,
                style_words(&p.style),
                if r.beat { "" } else { " (and drums)" }
            ),
            takes: vec![s2, simple, roles_off],
            labels: Vec::new(),
        })
    })();
    let cases = match made {
        Ok(c) => vec![finish_square(c, seed, k)],
        Err(e) => {
            let _ = writeln!(notes, "  parts: left out, {e}");
            Vec::new()
        }
    };
    Made { cases, notes }
}

fn looped_plain(notes: &[PlainNote], length: Tick, passes: usize) -> Vec<PlainNote> {
    (0..passes as Tick)
        .flat_map(|k| {
            notes.iter().map(move |n| PlainNote {
                at: n.at + k * length,
                ..*n
            })
        })
        .collect()
}

/// The chords under your bars (up to `yours`), round and round to `to`.
fn looped_chords(chords: &ChordTimeline, yours: Tick, to: Tick) -> ChordTimeline {
    let mut spans: Vec<ChordSpan> = Vec::new();
    let mut off = 0;
    while off < to {
        for s in chords.spans().iter().filter(|s| s.start < yours) {
            let start = s.start + off;
            if start >= to {
                break;
            }
            let len = s.len.min(yours - s.start).min(to - start);
            spans.push(ChordSpan { start, len, ..*s });
        }
        off += yours.max(1);
    }
    ChordTimeline::new(spans).unwrap_or_else(|_| chords.clone())
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
    let chords = joined_chords(
        &first.chords,
        &second.chords,
        bars as Tick * p.meter.bar_ticks(),
    );
    if std::env::var_os("EXPLAIN").is_some() {
        explain(p, "first", &clip.context, &first);
        explain(p, "second", &again.context, &second);
    }
    Ok((whole(&all, &second.notes), bars + 16, chords))
}

/// With `EXPLAIN` set, one run of the long listen on stderr: the key it
/// read, the chords it heard in the bars it was given, and each bar's plan
/// (job, source, shift, tension, activity), chords, and every note's 1/16
/// step, role and length with the pitch played. How round 2's faults were
/// found.
fn explain(p: &Phrase, run: &str, heard: &MusicalContext, c: &Continued) {
    let bar = p.meter.bar_ticks();
    let step = PPQ as Tick / 4;
    let spans = |tl: &ChordTimeline| -> String {
        tl.spans()
            .iter()
            .map(|s| {
                let b = |t: Tick| t as f64 / bar as f64 + 1.0;
                let name = s.answer.chord().map_or("?".into(), |c| c.name());
                format!("{:.1}-{:.1}:{name}", b(s.start), b(s.end()))
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    eprintln!(
        "KEY {} {run} {} | heard: {}",
        p.slug,
        heard.key.words(),
        spans(&heard.chords)
    );
    eprintln!("CHORDS {} {run} {}", p.slug, spans(&c.chords));
    for (b, bp) in c.plan.iter().enumerate() {
        let (from, to) = (b as Tick * bar, (b as Tick + 1) * bar);
        let chords: Vec<String> = c
            .chords
            .spans()
            .iter()
            .filter(|s| s.start < to && s.end() > from)
            .map(|s| s.answer.chord().map_or("?".into(), |c| c.name()))
            .collect();
        let notes: Vec<String> = c
            .part
            .decisions
            .iter()
            .filter(|d| (from..to).contains(&d.at))
            .map(|d| format!("{}:{:?}/{}", (d.at - from) / step, d.role, d.len / step))
            .collect();
        let pitches: Vec<String> = c
            .events
            .iter()
            .filter(|e| (from..to).contains(&e.at))
            .map(|e| e.pitch.to_string())
            .collect();
        eprintln!(
            "EXPLAIN {} {run} bar{} {:?} src{} sh{} t{:.2} a{:.2} [{}] {} | {}",
            p.slug,
            b + 1,
            bp.job,
            bp.source,
            bp.shift,
            bp.tension,
            bp.activity,
            chords.join(","),
            notes.join(" "),
            pitches.join(" ")
        );
    }
}

/// The chords each half of the long listen was written over: the first
/// run's up to `cut` (its own bars as heard and the chords it suggested),
/// the second run's after. The second run's reading of the first run's
/// notes is not what those notes were written over, and played under them
/// it clashes.
fn joined_chords(first: &ChordTimeline, second: &ChordTimeline, cut: Tick) -> ChordTimeline {
    let mut spans: Vec<ChordSpan> = first
        .spans()
        .iter()
        .filter(|s| s.start < cut)
        .map(|s| ChordSpan {
            len: s.len.min(cut - s.start),
            ..*s
        })
        .collect();
    spans.extend(second.spans().iter().filter(|s| s.end() > cut).map(|s| {
        let start = s.start.max(cut);
        ChordSpan {
            start,
            len: s.end() - start,
            ..*s
        }
    }));
    ChordTimeline::new(spans).unwrap_or_else(|_| second.clone())
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
    /// How much goes to the reverb and the echo.
    reverb: f32,
    delay: f32,
    /// (frame, note-off before note-on at the same frame, event)
    events: Vec<(u64, u8, NodeEvent)>,
    /// How far under the lead it sits, in LU, in the sets mixed by how
    /// loud each part sounds ([`mix`]); the lead itself is at 0.
    below: f64,
    /// Drums or a pad: left out on plain keys.
    backing: bool,
    /// The echo send times this from each frame on (sound moves' Space),
    /// in time order; none: as `delay` says throughout.
    sends: Vec<(u64, f32)>,
}

impl Part {
    /// The lead: every other part is set against it.
    fn new(node: Box<dyn Node>, gain: f32, reverb: f32, delay: f32) -> Part {
        Part {
            node,
            gain,
            reverb,
            delay,
            events: Vec::new(),
            below: 0.0,
            backing: false,
            sends: Vec::new(),
        }
    }

    /// The part `lu` under the lead.
    fn under(mut self, lu: f64) -> Part {
        self.below = lu;
        self
    }

    /// Drums or a pad.
    fn backing(mut self) -> Part {
        self.backing = true;
        self
    }

    fn note(&mut self, on: u64, off: u64, note: u8, velocity: u8) {
        self.events
            .push((on, 1, NodeEvent::NoteOn { note, velocity }));
        self.events
            .push((off.max(on + 1), 0, NodeEvent::NoteOff { note }));
    }
}

/// The safe controls of the preset `node` holds ([`safe_controls`]).
fn safe_of(node: &dyn Node) -> mpfree_compose::SafeControls {
    let knobs: Vec<Knob> = node
        .params()
        .iter()
        .filter(|i| !i.readonly)
        .map(|i| Knob {
            id: i.id,
            name: i.name,
            group: i.group,
            min: i.min,
            max: i.max,
            log: i.curve == Curve::Log,
            value: node.param(i.id),
        })
        .collect();
    safe_controls(&knobs)
}

/// The echo send at a full Space move: three times the part's own.
const SPACE_SEND: f32 = 2.0;

/// Sound moves on `part` as the app plays Continue's lane: each move sets
/// its knob (the preset's safe control) just before its note starts, and
/// the knob goes back to the preset's value at the next note that has no
/// move of its own; Space raises the echo send the same way.
fn sound_moves_on(
    part: &mut Part,
    moves: &[SoundMove],
    notes: &[Note],
    from: Tick,
    to: Tick,
    at: &dyn Fn(Tick) -> u64,
) {
    if moves.is_empty() {
        return;
    }
    let safe = safe_of(part.node.as_ref());
    let mut onsets: Vec<Tick> = notes
        .iter()
        .map(|n| n.tick)
        .filter(|&t| t >= from && t < to)
        .collect();
    onsets.sort_unstable();
    onsets.dedup();
    // A move's note may be played a few ms ahead of its step: the move
    // goes in a little before the earliest note on it.
    let on = |tick: Tick| {
        let t = onsets
            .iter()
            .copied()
            .filter(|&o| (o - tick).abs() <= STEP / 2)
            .min()
            .unwrap_or(tick)
            .min(tick);
        at(t).saturating_sub(64)
    };
    for c in Control::ALL {
        let mine: Vec<&SoundMove> = moves
            .iter()
            .filter(|m| m.control == c && m.at >= from && m.at < to)
            .collect();
        for (j, m) in mine.iter().enumerate() {
            // The next note after this one, and whether it has a move.
            let next = onsets.iter().copied().find(|&o| o > m.at + STEP / 2);
            let moved_next = next.is_some_and(|nx| {
                mine.get(j + 1)
                    .is_some_and(|q| (q.at - nx).abs() <= STEP / 2)
            });
            match c {
                Control::Space => {
                    part.sends
                        .push((on(m.at), 1.0 + SPACE_SEND * m.amount.max(0.0)));
                    if let (Some(nx), false) = (next, moved_next) {
                        part.sends.push((on(nx), 1.0));
                    }
                }
                _ => {
                    let Some(k) = safe.get(c) else {
                        continue;
                    };
                    part.events.push((
                        on(m.at),
                        0,
                        NodeEvent::Param {
                            id: k.id,
                            value: k.value(m.amount),
                        },
                    ));
                    if let (Some(nx), false) = (next, moved_next) {
                        part.events.push((
                            on(nx),
                            0,
                            NodeEvent::Param {
                                id: k.id,
                                value: k.base,
                            },
                        ));
                    }
                }
            }
        }
    }
    part.sends.sort_by_key(|s| s.0);
}

/// The melody's sound, picked for the phrase's style and the sound set:
/// the same in every file of a phrase. A bass plays on Pluck "Upright Bass".
fn melody_sound(p: &Phrase) -> Box<dyn Node> {
    use Sounds::*;
    let set = sounds();
    if set == Plain {
        return keys("Ballad");
    }
    if p.role == PartRole::Bass {
        return pluck("Upright Bass");
    }
    if let Some(name) = LEAD.get() {
        let mut s = Poly::new(RATE);
        s.load_preset(preset(Poly::presets(), name, |p| p.name));
        return Box::new(s);
    }
    // Riff (Groove), swung line (Jazz-ish), waltz, tune (Minimal).
    match (p.style.bass.drums, p.meter.beats, set) {
        (DrumMap::BoomBap, _, B) => keys("Soft Clav"),
        (DrumMap::BoomBap, _, _) => keys("Funk Clav"),
        (DrumMap::Ride, _, Round2) => keys("Mellow Tines"),
        (DrumMap::Ride, _, B) => pluck("Vibes"),
        (DrumMap::Ride, _, _) => wind("Tenor Sax"),
        (_, 3, Round2) => pluck("Nylon"),
        (_, 3, B) => wind("Concert Flute"),
        (_, 3, _) => wind("Warm Clarinet"),
        (_, _, Round2) => {
            let mut s = Poly::new(RATE);
            s.load_preset(preset(Poly::presets(), "Harmonic Pluck", |p| p.name));
            Box::new(s)
        }
        (_, _, Now) => keys("Soft Tines"),
        (_, _, B) => pluck("Marimba"),
        (_, _, _) => keys("Stage Tines"),
    }
}

fn keys(name: &str) -> Box<dyn Node> {
    let mut s = Keys::new(RATE);
    s.load_preset(preset(Keys::presets(), name, |p| p.name));
    Box::new(s)
}

fn pluck(name: &str) -> Box<dyn Node> {
    let mut s = Pluck::new(RATE);
    s.load_preset(preset(Pluck::presets(), name, |p| p.name));
    Box::new(s)
}

fn wind(name: &str) -> Box<dyn Node> {
    let mut s = Wind::new(RATE);
    s.load_preset(preset(Wind::presets(), name, |p| p.name));
    Box::new(s)
}

/// The bass under the long listen, for the phrase's style. In the S0 sets
/// the synth basses answer velocity over 24 dB instead of their presets'
/// 12, so a bass line's accents can be heard.
fn bass_sound(p: &Phrase) -> Box<dyn Node> {
    if sounds() == Sounds::Plain {
        return keys("Ballad");
    }
    let name = match p.style.bass.line {
        BassLine::Walking => return pluck("Upright Bass"),
        BassLine::Sixteenths => "Funk Bass",
        BassLine::Offbeat => "Round Mono",
    };
    let mut s = Bass::new(RATE);
    s.load_preset(preset(Bass::presets(), name, |p| p.name));
    if matches!(sounds(), Sounds::A | Sounds::B) {
        s.set_param(mpfree_synth::bass_params::VELOCITY, 24.0);
    }
    Box::new(s)
}

/// Chords added to a row: held on a pad for Minimal, on tines otherwise.
fn chord_sound(p: &Phrase) -> Box<dyn Node> {
    if sounds() == Sounds::Plain {
        return keys("Ballad");
    }
    match p.style.bass.comping {
        mpfree_compose::style::Comping::Pad => {
            let mut s = Strings::new(RATE);
            s.load_preset(preset(Strings::presets(), "Ballad Pad", |p| p.name));
            Box::new(s)
        }
        _ => keys("Mellow Tines"),
    }
}

/// The phrase's kit, for drums added to a row.
fn kit(p: &Phrase) -> Box<dyn Node> {
    let mut d = Drums::new(RATE);
    d.load_preset(preset(Drums::presets(), kit_name(p), |k| k.name));
    Box::new(d)
}

/// The drum kit for the phrase's style.
fn kit_name(p: &Phrase) -> &'static str {
    match (p.style.bass.drums, p.meter.beats) {
        (DrumMap::BoomBap, _) => "Boom Bap Kit",
        (DrumMap::FourOnTheFloor, 4) => "House Kit",
        _ => "Tight Kit",
    }
}

fn preset<'a, P>(all: &'a [P], name: &str, name_of: impl Fn(&P) -> &str) -> &'a P {
    all.iter()
        .find(|p| name_of(p) == name)
        .unwrap_or_else(|| panic!("no preset {name}"))
}

// Pads of the factory kits (pad 1 is note 36).
const KICK: u8 = 36;
const SNARE: u8 = 37;
const CLAP: u8 = 38;
const HAT: u8 = 39;
const OPEN_HAT: u8 = 40;
const TOM_LOW: u8 = 41;
const TOM_MID: u8 = 42;
const TOM_HIGH: u8 = 43;
const RIM: u8 = 44;
/// The House Kit's ride.
const RIDE: u8 = 45;
/// Pad 11: the crash (House, Boom Bap), the long cymbal (Tight, played as
/// a ride).
const CYMBAL: u8 = 46;

/// A small stable hash, for the drummer's humanize.
fn hash(a: u64, b: u64, c: u64) -> u64 {
    let mut h = 0x9e37_79b9_7f4a_7c15u64 ^ a.wrapping_mul(0xff51_afd7_ed55_8ccd);
    for x in [b, c] {
        h ^= x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        h ^= h >> 31;
    }
    h
}

/// One bar of a drummer for the phrase's style: (tick in the bar, pad,
/// velocity), on the 1/16 grid. A crash every eight bars, a fill before
/// every fourth bar line, accents, ghost notes and an open hat now and
/// then. The same in every file of a phrase, so neither side of a pair
/// is helped by it.
fn drummer(p: &Phrase, bar: usize) -> Vec<(Tick, u8, u8)> {
    let s = PPQ as Tick / 4;
    let steps = (p.meter.bar_ticks() / s).max(1) as usize;
    let fill = bar % 4 == 3;
    let crash = bar.is_multiple_of(8);
    let mut h: Vec<(usize, u8, u8)> = Vec::new();
    match (p.style.bass.drums, steps) {
        (DrumMap::FourOnTheFloor, 16) => {
            for (k, v) in [(0, 112), (4, 100), (8, 108), (12, 100)] {
                h.push((k, KICK, v));
            }
            h.push((4, CLAP, 100));
            h.push((12, CLAP, 104));
            for st in 0..16 {
                if fill && st >= 12 {
                    continue;
                }
                if st % 4 == 2 {
                    h.push((st, OPEN_HAT, if st == 14 { 86 } else { 76 }));
                } else if st % 2 == 1 {
                    h.push((st, HAT, if st % 4 == 3 { 52 } else { 40 }));
                }
            }
            if bar % 2 == 1 && !fill {
                h.push((15, KICK, 62));
            }
            if fill {
                for (i, st) in (12..16).enumerate() {
                    h.push((st, SNARE, 58 + 14 * i as u8));
                }
            } else {
                h.push((2, RIDE, 50));
                h.push((10, RIDE, 50));
            }
        }
        (DrumMap::BoomBap, 16) => {
            let a = bar.is_multiple_of(2);
            let kicks: &[(usize, u8)] = if a {
                &[(0, 112), (6, 92), (10, 102)]
            } else {
                &[(0, 112), (3, 78), (8, 104), (14, 86)]
            };
            for &(k, v) in kicks {
                if !(fill && k >= 12) {
                    h.push((k, KICK, v));
                }
            }
            h.push((4, SNARE, 110));
            let ghosts: &[usize] = if a { &[7, 15] } else { &[6, 11] };
            for &g in ghosts {
                if !(fill && g >= 12) {
                    h.push((g, SNARE, 34 + (hash(bar as u64, g as u64, 7) % 10) as u8));
                }
            }
            let open = !a && !fill;
            for st in 0..16 {
                if fill && st >= 12 {
                    continue;
                }
                if open && st == 14 {
                    h.push((st, OPEN_HAT, 74));
                    continue;
                }
                let v = match st % 4 {
                    0 => 88,
                    2 => 70,
                    _ => 44,
                };
                h.push((st, HAT, v));
            }
            if fill {
                h.push((12, SNARE, 112));
                h.push((13, TOM_HIGH, 84));
                h.push((14, TOM_MID, 92));
                h.push((15, TOM_LOW, 100));
            } else {
                h.push((12, SNARE, 108));
            }
        }
        (DrumMap::Ride, 16) => {
            // Ding, ding-a ding, ding-a: the swing comes from the clip.
            for (st, v) in [(0, 86), (4, 100), (6, 66), (8, 84), (12, 100), (14, 66)] {
                h.push((st, CYMBAL, v));
            }
            h.push((4, HAT, 58));
            h.push((12, HAT, 58));
            for st in [0, 4, 8, 12] {
                h.push((st, KICK, 34));
            }
            // The snare comps on a couple of off-beats.
            let offs = [2usize, 6, 10, 14];
            for (i, &st) in offs.iter().enumerate() {
                let r = hash(bar as u64, i as u64, 11) % 100;
                if r < 30 && !(fill && st >= 10) {
                    h.push((st, SNARE, 44 + (r % 20) as u8));
                }
            }
            if fill {
                h.push((10, SNARE, 78));
                h.push((14, SNARE, 88));
                h.push((14, KICK, 80));
            }
        }
        (_, 12) => {
            // A waltz: the bass drum on one, the rim on two and three.
            h.push((0, KICK, 100));
            h.push((4, RIM, 58));
            h.push((8, RIM, 62));
            for st in (0..12).step_by(2) {
                if fill && st >= 8 {
                    continue;
                }
                h.push((st, HAT, if st % 4 == 0 { 54 } else { 38 }));
            }
            if bar % 2 == 1 && !fill {
                h.push((10, KICK, 58));
            }
            if fill {
                h.push((8, SNARE, 66));
                h.push((10, SNARE, 74));
                h.push((11, SNARE, 84));
            }
        }
        _ => {
            // Any other meter: a plain beat.
            let beat = steps / usize::from(p.meter.beats.max(1));
            h.push((0, KICK, 110));
            for b in 1..usize::from(p.meter.beats) {
                h.push((b * beat, SNARE, 90));
            }
            for st in (0..steps).step_by(2) {
                h.push((st, HAT, if st % beat.max(1) == 0 { 80 } else { 56 }));
            }
        }
    }
    if crash {
        h.retain(|&(st, n, _)| !(st == 0 && matches!(n, HAT | OPEN_HAT | RIDE)));
        h.push((0, CYMBAL, 96));
    }
    // A little humanize: velocity within about 5 either way, timing
    // within a 1/128 (the same every run).
    h.into_iter()
        .map(|(st, n, v)| {
            let r = hash(bar as u64, st as u64, u64::from(n));
            let dv = (r % 11) as i32 - 5;
            let dt = ((r >> 8) % 9) as Tick - 4;
            let tick = (st as Tick * s + dt * PPQ as Tick / 256).max(0);
            (tick, n, (i32::from(v) + dv).clamp(1, 127) as u8)
        })
        .collect()
}

/// A bass line under the long listen, from the chords, as the style's bass
/// plays: a root on every off-beat (Minimal), roots and fifths with the
/// kick (Groove), or walking quarter notes into each chord (Jazz-ish).
fn bass_line(p: &Phrase, chords: &ChordTimeline, from: Tick, to: Tick) -> Vec<NoteOf> {
    let beat = p.meter.beat_ticks();
    let bar = p.meter.bar_ticks();
    let s = PPQ as Tick / 4;
    let chord_at = |t: Tick| -> Option<Chord> {
        chords
            .spans()
            .iter()
            .find(|c| c.contains(t))
            .and_then(|c| c.answer.chord())
    };
    // From E2 up to D#3.
    let root_at = |t: Tick| chord_at(t).map(|c| 40 + (i32::from(c.root) - 4).rem_euclid(12));
    // The chord's own third and fifth above the root (a diminished
    // chord's fifth is flat), so the bass never plays outside its chord.
    let above = |t: Tick, r: i32, tone: Tone| {
        chord_at(t).map_or(r + 7, |c| {
            r + (i32::from(c.tone(tone)) - i32::from(c.root)).rem_euclid(12)
        })
    };
    let mut out = Vec::new();
    let mut t = from - from.rem_euclid(bar);
    while t < to {
        match p.style.bass.line {
            BassLine::Offbeat => {
                for b in 0..Tick::from(p.meter.beats) {
                    let at = t + b * beat + beat / 2;
                    if let Some(r) = root_at(at) {
                        out.push((at, beat / 2 - s / 2, r as u8, 96));
                    }
                }
            }
            BassLine::Sixteenths => {
                let steps: &[(Tick, Tick, bool)] = if (t / bar) % 2 == 0 {
                    &[(0, 5, false), (6, 2, false), (10, 3, true), (14, 2, false)]
                } else {
                    &[(0, 3, false), (3, 2, true), (8, 4, false), (14, 2, true)]
                };
                for &(st, len, fif) in steps {
                    let at = t + st * s;
                    if at >= t + bar {
                        continue;
                    }
                    if let Some(r) = root_at(at) {
                        let n = if fif { above(at, r, Tone::Fifth) } else { r };
                        out.push((at, len * s - s / 3, n as u8, if st == 0 { 104 } else { 88 }));
                    }
                }
            }
            BassLine::Walking => {
                let beats = Tick::from(p.meter.beats);
                for b in 0..beats {
                    let at = t + b * beat;
                    let Some(r) = root_at(at) else { continue };
                    let next = root_at(at + beat).unwrap_or(r);
                    let n = if b == 0 {
                        r
                    } else if b + 1 == beats || root_at(at + beat) != root_at(at) {
                        // A half step into the next chord's root.
                        next + if next > r { -1 } else { 1 }
                    } else if b % 2 == 1 {
                        above(at, r, Tone::Third)
                    } else {
                        above(at, r, Tone::Fifth)
                    };
                    out.push((
                        at,
                        beat - s / 2,
                        n.clamp(28, 60) as u8,
                        if b == 0 { 100 } else { 86 },
                    ));
                }
            }
        }
        t += bar;
    }
    out.retain(|n| n.0 >= from && n.0 < to);
    out
}

/// How far under the lead the drummer and a bass sit, in LU, for the
/// phrase's style.
fn sits(p: &Phrase) -> (f64, f64) {
    match p.style.bass.drums {
        DrumMap::BoomBap => (-3.0, -3.0),
        DrumMap::Ride => (-6.0, -5.0),
        _ => (-4.0, -4.0),
    }
}

/// The parts as heard together, `frames` long (to the next whole block):
/// each part on its own first, so that in the sets mixed by loudness
/// every part can be set where it sits under the first one (the lead)
/// whatever its preset's own level; then the echo and the plate; then the
/// whole brought to the loudness every file has. `drums_lu` is where a
/// drummer sits ([`sits`]): with a silent lead (a beat alone) the parts
/// are set against where the lead would be over the drums.
fn mix(parts: &mut [Part], drums_lu: f64, bpm: f64, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let frames = frames.div_ceil(BLOCK) * BLOCK;
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let ctx_at = |pos: usize| Ctx {
        rate: RATE,
        frame: pos as u64,
        bpm,
        beats: pos as f64 / f64::from(RATE) * bpm / 60.0,
        playing: true,
        sidechain: None,
    };
    // Each part on its own first, so the mix can be set by how loud each
    // part sounds.
    let mut dry: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
    let mut buf = Vec::new();
    for part in parts.iter_mut() {
        let (mut pl, mut pr) = (vec![0.0f32; frames], vec![0.0f32; frames]);
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
            part.node.process(&ctx_at(pos), &buf, &mut l, &mut r);
            pl[pos..pos + BLOCK].copy_from_slice(&l);
            pr[pos..pos + BLOCK].copy_from_slice(&r);
        }
        dry.push((pl, pr));
    }
    // The S0 sets: every part sits a set number of LU under the melody,
    // whatever its preset's own level (parts: melody, drums, then pad and
    // bass on the long listen). Plain keys keep only the melody and bass.
    if matches!(sounds(), Sounds::A | Sounds::B | Sounds::Plain) {
        // Set against the melody; with no melody (a beat alone), against
        // where the melody would be over the drums.
        let mut melody = loudness(&dry[0].0, &dry[0].1);
        if !melody.is_finite() && !parts[0].events.is_empty() {
            eprintln!("listen: the lead has notes but made no sound");
        }
        if !melody.is_finite() {
            melody = (1..parts.len())
                .find_map(|k| {
                    let lu = loudness(&dry[k].0, &dry[k].1);
                    (lu.is_finite() && parts[k].backing && parts[k].below == drums_lu)
                        .then_some(lu - drums_lu)
                })
                .unwrap_or(melody);
        }
        for (k, part) in parts.iter_mut().enumerate() {
            let lu = loudness(&dry[k].0, &dry[k].1);
            part.gain = if sounds() == Sounds::Plain && part.backing {
                0.0
            } else if melody.is_finite() && lu.is_finite() {
                10f64.powf((melody + part.below - lu) / 20.0) as f32
            } else {
                part.gain
            };
            if sounds() == Sounds::Plain {
                part.reverb = 0.0;
                part.delay = 0.0;
            }
            if std::env::var_os("EXPLAIN").is_some() {
                eprintln!(
                    "  mix: part {} is {lu:.1} LUFS on its own, set {:+.1} dB",
                    k + 1,
                    20.0 * f64::from(part.gain.max(1e-9)).log10()
                );
            }
        }
    }
    let (mut left, mut right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let (mut rev_l, mut rev_r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let (mut del_l, mut del_r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    for (part, (pl, pr)) in parts.iter().zip(&dry) {
        // The echo send, following the sound moves' Space over ~10 ms.
        let glide = 1.0 - (-1.0 / (0.010 * RATE as f32)).exp();
        let (mut send, mut want, mut next) = (1.0f32, 1.0f32, 0usize);
        for k in 0..frames {
            while next < part.sends.len() && part.sends[next].0 <= k as u64 {
                want = part.sends[next].1;
                next += 1;
            }
            send += (want - send) * glide;
            let (a, b) = (pl[k] * part.gain, pr[k] * part.gain);
            left[k] += a;
            right[k] += b;
            rev_l[k] += a * part.reverb;
            rev_r[k] += b * part.reverb;
            del_l[k] += a * part.delay * send;
            del_r[k] += b * part.delay * send;
        }
    }
    // The echo: a dotted 1/8, a few soft repeats, dark; a little of it
    // into the reverb too.
    let mut echo = Delay::new();
    for (id, v) in [(0u16, 7.0), (2, 32.0), (3, 1.0), (4, 3_500.0), (6, 100.0)] {
        echo.set_param(id, v);
    }
    let mut plate = Plate::new();
    for (id, v) in [
        (plate::PREDELAY, 18.0),
        (plate::DECAY, 2_200.0),
        (plate::DAMPING, 45.0),
        (plate::LOW_CUT, 180.0),
        (plate::MIX, 100.0),
    ] {
        plate.set_param(id, v);
    }
    let fx: [(&mut dyn Node, bool); 2] = [(&mut echo, true), (&mut plate, false)];
    for (node, is_echo) in fx {
        node.prepare(RATE, BLOCK);
        for pos in (0..frames).step_by(BLOCK) {
            let (src_l, src_r) = if is_echo {
                (&del_l, &del_r)
            } else {
                (&rev_l, &rev_r)
            };
            l.copy_from_slice(&src_l[pos..pos + BLOCK]);
            r.copy_from_slice(&src_r[pos..pos + BLOCK]);
            node.process(&ctx_at(pos), &[], &mut l, &mut r);
            for k in 0..BLOCK {
                left[pos + k] += l[k];
                right[pos + k] += r[k];
                if is_echo {
                    rev_l[pos + k] += l[k] * 0.3;
                    rev_r[pos + k] += r[k] * 0.3;
                }
            }
        }
    }
    // Every file at the same loudness, so files compare fairly; the
    // writer's fixed LEVEL is taken into account.
    let lu = loudness(&left, &right);
    if lu.is_finite() {
        let g = (10f64.powf((FILE_LUFS - lu) / 20.0) / f64::from(LEVEL)) as f32;
        for x in left.iter_mut().chain(right.iter_mut()) {
            *x *= g;
        }
    }
    (left, right)
}

/// The case's take as heard: the melody, a drummer, and in the long
/// listen the chords as a pad and a bass, through a plate reverb and a
/// tempo-synced echo.
fn render(p: &Phrase, t: &Take) -> (Vec<f32>, Vec<f32>, Smf) {
    let bar = p.meter.bar_ticks();
    let from = t.from_bar as Tick * bar;
    let to = t.to_bar as Tick * bar;
    let start = frame(p, from as f64);
    let at = |tick: Tick| frame(p, tick as f64).saturating_sub(start);
    // The fixed gains of rounds 2 and 3 (the clav and the sax are louder
    // than the other sounds); the S0 sets work their gains out below.
    let level = match (sounds(), p.style.bass.drums, p.meter.beats) {
        (Sounds::Round2, DrumMap::BoomBap, _) => 0.7,
        (Sounds::Round2, _, _) => 1.0,
        (_, DrumMap::BoomBap, _) => 0.6,
        (_, DrumMap::Ride, _) => 0.55,
        (_, _, 3) => 0.8,
        _ => 1.0,
    };
    let mut melody = Part::new(melody_sound(p), level, 0.22, 0.16);
    let mut midi_melody: Vec<(u64, Msg)> = Vec::new();
    let heard_tick = |tick: Tick| {
        (p.swing.apply(tick as f64) - p.swing.apply(from as f64))
            .round()
            .max(0.0) as u64
    };
    let midi_note =
        |track: &mut Vec<(u64, Msg)>, ch: u8, on: Tick, off: Tick, note: u8, vel: u8| {
            track.push((heard_tick(on), Msg::NoteOn { ch, note, vel }));
            track.push((heard_tick(off), Msg::NoteOff { ch, note, vel: 0 }));
        };
    for n in &t.notes {
        if n.tick < from || n.tick >= to {
            continue;
        }
        let end = (n.tick + n.len.max(1)).min(to);
        melody.note(at(n.tick), at(end), n.note, n.velocity);
        midi_note(&mut midi_melody, 0, n.tick, end, n.note, n.velocity);
    }
    sound_moves_on(&mut melody, &t.moves, &t.notes, from, to, &at);
    // A drummer, the same in every file of a phrase.
    let mut drums = Part::new(
        {
            let mut d = Drums::new(RATE);
            d.load_preset(preset(Drums::presets(), kit_name(p), |k| k.name));
            Box::new(d)
        },
        0.5,
        0.05,
        0.0,
    );
    let mut midi_drums: Vec<(u64, Msg)> = Vec::new();
    for b in t.from_bar..t.to_bar {
        if !t.drummer {
            break;
        }
        let b0 = b as Tick * bar;
        for (tick, note, vel) in drummer(p, b) {
            let on = at(b0 + tick);
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
            midi_note(&mut midi_drums, 9, b0 + tick, b0 + tick + 120, note, vel);
        }
    }
    // How far under the melody each part sits in the S0 sets, in LU.
    // Plain keys play only the melody and the basses.
    let (drums_lu, bass_lu) = sits(p);
    let mut parts = vec![melody, drums.under(drums_lu).backing()];
    let mut midi_bass: Vec<(u64, Msg)> = Vec::new();
    // The chords, as a quiet pad, and a bass, for the long listen.
    if let Some(chords) = &t.pad {
        let mut pad = Part::new(
            {
                let mut s = Strings::new(RATE);
                s.load_preset(preset(Strings::presets(), "Ballad Pad", |p| p.name));
                Box::new(s)
            },
            0.26,
            0.3,
            0.0,
        );
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
                    pad.note(at(s0), at(s1), pitch as u8, 64);
                }
            }
        }
        let mut bass = Part::new(bass_sound(p), 0.55, 0.0, 0.0);
        for (on, len, note, vel) in bass_line(p, chords, from, to) {
            let off = (on + len.max(1)).min(to);
            bass.note(at(on), at(off), note, vel);
            midi_note(&mut midi_bass, 1, on, off, note, vel);
        }
        parts.push(pad.under(-11.0).backing());
        parts.push(bass.under(bass_lu));
    }
    // Parts added to a row (S2), each on its own sound.
    let mut midi_extra: Vec<(&'static str, Vec<(u64, Msg)>)> = Vec::new();
    for x in &t.extra {
        let (node, gain, reverb, delay, ch, name, lu): (
            Box<dyn Node>,
            f32,
            f32,
            f32,
            u8,
            &str,
            f64,
        ) = match x.kind {
            ExtraKind::Bass => (bass_sound(p), 0.55, 0.0, 0.0, 1, "Bass", bass_lu),
            ExtraKind::Chords => (chord_sound(p), 0.3, 0.25, 0.04, 2, "Chords", -7.0),
            ExtraKind::Drums => (kit(p), 0.5, 0.05, 0.0, 9, "Drums", drums_lu),
        };
        let mut part = Part::new(node, gain, reverb, delay);
        let mut track = Vec::new();
        for &(on, len, note, vel) in &x.notes {
            if on < from || on >= to {
                continue;
            }
            let off = if x.kind == ExtraKind::Drums {
                on + 120
            } else {
                (on + len.max(1)).min(to)
            };
            part.note(at(on), at(off), note, vel);
            midi_note(&mut track, ch, on, off, note, vel);
        }
        let part = part.under(lu);
        parts.push(if x.kind == ExtraKind::Drums {
            part.backing()
        } else {
            part
        });
        midi_extra.push((name, track));
    }
    let frames = (at(to) as f64 + TAIL * f64::from(RATE)) as usize;
    let (left, right) = mix(&mut parts, drums_lu, p.bpm, frames);
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
    if !midi_bass.is_empty() {
        tracks.push(SmfTrack {
            name: "Bass line".into(),
            events: midi_bass,
        });
    }
    for (name, events) in midi_extra {
        tracks.push(SmfTrack {
            name: name.into(),
            events,
        });
    }
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

/// Integrated loudness in LUFS (ITU-R BS.1770-4: K-weighting at 48 kHz,
/// 400 ms blocks overlapping by three quarters, the absolute gate at -70
/// and the relative gate 10 LU under); minus infinity for silence.
fn loudness(left: &[f32], right: &[f32]) -> f64 {
    fn weigh(x: &[f32]) -> Vec<f64> {
        let (b0, b1, b2) = (
            1.535_124_859_586_97,
            -2.691_696_189_406_38,
            1.198_392_810_852_85,
        );
        let (a1, a2) = (-1.690_659_293_182_41, 0.732_480_774_215_85);
        let (d1, d2) = (-1.990_047_454_833_98, 0.990_072_250_366_21);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        let (mut u1, mut u2, mut z1, mut z2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&v| {
                let v = f64::from(v);
                let y = b0 * v + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                (x2, x1, y2, y1) = (x1, v, y1, y);
                let z = y - 2.0 * u1 + u2 - d1 * z1 - d2 * z2;
                (u2, u1, z2, z1) = (u1, y, z1, z);
                z
            })
            .collect()
    }
    let (wl, wr) = (weigh(left), weigh(right));
    let block = (0.4 * f64::from(RATE)) as usize;
    let mut power = Vec::new();
    let mut at = 0;
    while at + block <= wl.len() {
        let sum: f64 = wl[at..at + block]
            .iter()
            .chain(&wr[at..at + block])
            .map(|v| v * v)
            .sum();
        power.push(sum / block as f64);
        at += block / 4;
    }
    let lufs = |z: f64| -0.691 + 10.0 * z.max(1e-30).log10();
    let mean = |zs: &[f64]| zs.iter().sum::<f64>() / zs.len() as f64;
    let loud: Vec<f64> = power.into_iter().filter(|&z| lufs(z) > -70.0).collect();
    if loud.is_empty() {
        return f64::NEG_INFINITY;
    }
    let gate = lufs(mean(&loud)) - 10.0;
    let kept: Vec<f64> = loud.into_iter().filter(|&z| lufs(z) > gate).collect();
    lufs(mean(&kept))
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
        "Every file is at the same loudness ({FILE_LUFS} LUFS, with a soft knee from -1.9 dBFS instead of clipping); the loudest peak was {:.1} dBFS.",
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

/// `--critic N`: each phrase carried on by 8 bars on seeds 1 to N, by
/// mpfree and by the simpler ways, judged by the critic; how many pass
/// each check, and what it found on seed 1. Writes nothing.
fn critic_table(phrases: &[Phrase], seeds: u64) -> Result<(), String> {
    const NAMES: [&str; 6] = [
        "somewhere",
        "lands",
        "breathes",
        "yours",
        "repeats",
        "flows",
    ];
    println!(
        "{:28} {:>10} {:>6} {:>9} {:>6} {:>8} {:>6} {:>7}",
        "phrase / way", "somewhere", "lands", "breathes", "yours", "repeats", "flows", "all 6"
    );
    for p in phrases {
        let clip = clip_of(&p.notes, p.bars, p.meter);
        let mut tally: Vec<(&str, [u64; 7], Option<Verdict>)> = vec![
            ("music first (S1)", [0; 7], None),
            ("first engine", [0; 7], None),
            ("moved in the scale", [0; 7], None),
            ("repeated", [0; 7], None),
        ];
        for seed in 1..=seeds {
            let s = settings(p.style, 8, seed);
            let req = request(p, &clip, &s);
            let made = first_engine(&req).map_err(|e| e.words())?;
            let s1 = drafts(&req).map_err(|e| e.words())?;
            let best = &s1[0];
            if seed == 1 {
                println!(
                    "  {} seed 1, S1 draft {} of {}: {} | {}",
                    p.slug,
                    best.number + 1,
                    s1.len(),
                    best.sketch
                        .gestures
                        .iter()
                        .map(|g| g.words())
                        .collect::<Vec<_>>()
                        .join(", "),
                    s1.iter()
                        .map(|d| format!("draft {} passes {}", d.number + 1, d.verdict.passed()))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if seed == 1 && std::env::var_os("EXPLAIN").is_some() {
                show(p, "S1", &best.made, &best.sketch.gestures);
                show(p, "first", &made, &[]);
            }
            let ways = [
                best.made.notes.clone(),
                made.notes.clone(),
                simpler(&req, Simpler::Moved),
                simpler(&req, Simpler::Repeat),
            ];
            let chords = [&best.made.chords, &made.chords, &made.chords, &made.chords];
            for ((t, notes), chords) in tally.iter_mut().zip(ways.iter()).zip(chords) {
                let v = judge_continue(&req, notes, chords);
                let passes = v.checks().map(|(_, c)| c.pass);
                for (k, &ok) in passes.iter().enumerate() {
                    t.1[k] += u64::from(ok);
                }
                t.1[6] += u64::from(v.passed() == 6);
                if t.2.is_none() {
                    t.2 = Some(v);
                }
            }
        }
        for (way, n, _) in &tally {
            println!(
                "{:28} {:>10} {:>6} {:>9} {:>6} {:>8} {:>6} {:>7}",
                format!("{} / {way}", p.slug),
                n[0],
                n[1],
                n[2],
                n[3],
                n[4],
                n[5],
                n[6]
            );
        }
        for (way, _, v) in &tally {
            if let Some(v) = v {
                println!("  seed 1, {way}: {}", v.words());
            }
        }
    }
    println!("(out of {seeds} seeds each; checks: {})", NAMES.join(", "));
    Ok(())
}

/// With `EXPLAIN` and `--critic`: one result bar by bar, its chords, its
/// gesture, each note as step:scale place/length in 1/16s, and the notes
/// as played.
fn show(p: &Phrase, what: &str, c: &Continued, gestures: &[mpfree_compose::sketch::Gesture]) {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
    ];
    let bar = p.meter.bar_ticks();
    let step = PPQ as Tick / 4;
    let bars = (c.to / bar) as usize;
    for b in 0..bars {
        let (from, to) = (b as Tick * bar, (b as Tick + 1) * bar);
        let chords: Vec<String> = c
            .chords
            .spans()
            .iter()
            .filter(|s| s.start < to && s.end() > from)
            .map(|s| s.answer.chord().map_or("?".into(), |c| c.name()))
            .collect();
        let g = b
            .checked_sub(p.bars)
            .and_then(|i| gestures.get(i))
            .map_or(if b < p.bars { "yours" } else { "" }, |g| g.words());
        let steps = (bar / step) as i32;
        let notes: Vec<String> = c
            .line
            .notes
            .iter()
            .filter(|m| m.on.div_euclid(steps) == b as i32)
            .map(|m| format!("{}:{}/{}", m.on - b as i32 * steps, m.pos, m.len))
            .collect();
        let pitches: Vec<String> = c
            .events
            .iter()
            .filter(|e| (from..to).contains(&e.at))
            .map(|e| format!("{}{}", NAMES[usize::from(e.pitch % 12)], e.pitch / 12 - 1))
            .collect();
        println!(
            "    {} {what} bar {:2} {:11} [{}] {} | {}",
            p.slug,
            b + 1,
            g,
            chords.join(","),
            notes.join(" "),
            pitches.join(" ")
        );
    }
}

const USAGE: &str = "usage: listen --demo | --midi FILE [--track N] [--from BAR] [--bars N] [--all-notes] \
[--style minimal|groove|jazzish] [--bpm N] [--swing 1/8:58] [--role melody|bass] [--sounds a|b|now|round2|plain] [--seed N] [--gate | --parts | --gestures | --sound [--lead NAME]] [--out DIR] | --critic N (--demo | --midi FILE) \
| --phrases | --phrases-from FILE [--only TEXT] [--out DIR] | --ideas [--seed N] [--out DIR]";

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
    let mut critic: Option<u64> = None;
    let mut gate = false;
    let mut gestures = false;
    let mut sound = false;
    let mut parts = false;
    let mut phrases = false;
    let mut phrases_from: Option<PathBuf> = None;
    let mut ideas = false;
    let mut only: Option<String> = None;
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
            "--gate" => gate = true,
            "--gestures" => gestures = true,
            "--sound" => sound = true,
            "--parts" => parts = true,
            "--phrases" => phrases = true,
            "--phrases-from" => phrases_from = Some(PathBuf::from(value(&mut i)?)),
            "--ideas" => ideas = true,
            "--only" => only = Some(value(&mut i)?),
            "--critic" => {
                critic = Some(
                    value(&mut i)?
                        .parse()
                        .map_err(|_| "--critic takes a number of seeds")?,
                )
            }
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
            "--sounds" => {
                let set = match value(&mut i)?.as_str() {
                    "a" => Sounds::A,
                    "b" => Sounds::B,
                    "now" => Sounds::Now,
                    "round2" => Sounds::Round2,
                    "plain" => Sounds::Plain,
                    v => return Err(format!("--sounds {v}: a, b, now, round2 or plain")),
                };
                let _ = SOUNDS.set(set);
            }
            "--lead" => {
                let v = value(&mut i)?;
                if !Poly::presets().iter().any(|p| p.name == v) {
                    return Err(format!("--lead {v}: no such synth preset"));
                }
                let _ = LEAD.set(v);
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
    let new_dir = |kind: &str| -> Result<PathBuf, String> {
        let dir = out.clone().unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            Path::new(&home)
                .join("mpfree-work")
                .join(format!("{kind}-{}", utc_stamp()))
        });
        if dir.exists() && std::fs::read_dir(&dir).map_or(true, |mut d| d.next().is_some()) {
            return Err(format!(
                "{} is already there and not empty: give a new --out",
                dir.display()
            ));
        }
        Ok(dir)
    };
    if ideas {
        if demo || midi.is_some() || phrases || phrases_from.is_some() {
            return Err(format!(
                "give one of --demo, --midi FILE, --phrases, --phrases-from FILE or --ideas\n{USAGE}"
            ));
        }
        return phrases::ideas::run(seed, &new_dir("ideas-listen")?);
    }
    if phrases || phrases_from.is_some() {
        if demo || midi.is_some() || (phrases && phrases_from.is_some()) {
            return Err(format!(
                "give one of --demo, --midi FILE, --phrases or --phrases-from FILE\n{USAGE}"
            ));
        }
        return phrases::run(
            phrases_from.as_deref(),
            only.as_deref(),
            &new_dir("phrases-listen")?,
        );
    }
    if demo == midi.is_some() {
        return Err(format!("give --demo or --midi FILE\n{USAGE}"));
    }
    if !(1..=MOST_BARS).contains(&bars) {
        return Err(format!("--bars takes 1 to {MOST_BARS}"));
    }
    if let Some(seeds) = critic {
        let phrases = if let Some(path) = &midi {
            let (mut p, _) = midi_phrase(
                path,
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
            vec![p]
        } else {
            demo_phrases()
        };
        return critic_table(&phrases, seeds);
    }
    let dir = new_dir("compose-listen")?;
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
    if parts {
        let rows = if demo {
            gate_rows(phrases)
        } else {
            phrases
                .into_iter()
                .map(|p| GateRow {
                    phrase: Phrase {
                        name: format!("{}, with a beat", p.name),
                        ..p
                    },
                    tune: true,
                    beat: true,
                    picked: Vec::new(),
                })
                .collect()
        };
        let made: Vec<(Phrase, Made)> = rows
            .into_iter()
            .enumerate()
            .map(|(k, r)| {
                let m = parts_cases(&r, seed, k);
                (r.phrase, m)
            })
            .collect();
        return write_all(&made, &dir, seed, &header);
    }
    let made: Vec<(Phrase, Made)> = phrases
        .into_iter()
        .map(|p| {
            let m = if sound {
                sound_cases(&p, seed)
            } else if gestures {
                gestures_cases(&p, seed)
            } else if gate {
                gate_cases(&p, seed)
            } else {
                cases(&p, seed)
            };
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
