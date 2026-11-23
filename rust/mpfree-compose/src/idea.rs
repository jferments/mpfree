//! New ideas (phrases, P2): a phrase for the phrase player, made to
//! order: a bass line, a riff, chord stabs or a running line of one, two
//! or four bars, in a style, as busy as asked.
//!
//! A phrase plays from one held note, so an idea is written from a home
//! note in C, major or minor, over a short loop of chords from home that
//! the style's own chord moves suggest (or over the home chord alone),
//! and it carries that loop with it. An idea is Add a part
//! ([`crate::parts`]) for a band that isn't there:
//!
//! - **The band** is a row heard with no tune: the loop's chords
//!   ([`mod@crate::progression`], led home at the start and back at the end)
//!   and the style's own beat, added first as Add a part adds drums. A
//!   bass line is written against a kick, so it is made against one.
//! - **Bass and Chords** are Add a part's bass and chords for that row:
//!   their rhythm from the style's cells with named changes
//!   ([`crate::cells`]), their notes chosen for the whole part, the best
//!   of four drafts by the part's checks. A bass plays with the kick or
//!   between the kicks as its style's bass does; Try again takes the
//!   other way.
//! - **A riff** has the same rhythm of cells, and a tune of its own: one
//!   period (a bar, or two) chosen whole along a shape, then brought back
//!   over the loop on each chord's root, in key, as the phrase player
//!   itself moves a phrase; the plan's high point is lifted and its close
//!   leads back into the first note. Add a part's second line has no such
//!   tune (it sits under one), so a riff does not come from it.
//! - **A line** is a pulse that moves: one beat's rhythm on every beat,
//!   and a figure of chord notes, rising or falling from the chord's
//!   root, third or fifth, that starts again in every bar and follows
//!   each chord, a chord note higher from the plan's landing and from
//!   its high point.
//!
//! Every kind is made as four whole drafts, and the best is offered. A
//! draft is first written as a phrase writes notes (on its grid, as long
//! as its steps with the last one short, plain or held, and plain,
//! accented or a ghost), and then judged on what was written: a phrase
//! cannot hold a note for nine tenths of a step or play a ghost off a
//! swung grid, so checks on the notes as made would pass rests and
//! closes that nobody will hear. A bass and chords are judged by Add a
//! part's checks, a riff by its own and the critic's
//! ([`crate::critic`]), a line by its own.
//!
//! [`Idea::phrase`] is the one place an idea becomes the arp's
//! [`Phrase`]: the app and the listening tool both play what it returns.
//! [`next_idea`] is Try again: the next attempt whose phrase was not
//! offered before. The same request always makes the same idea.
//!
//! Design, with what was weighed for each choice and what was measured:
//! `docs/dev/design/compose.md`, "New ideas: phrases made to order (P2)".

use mpfree_arp::{Phrase, PhraseKind, Rate};
use mpfree_context::chord::{ChordAnswer, ChordTimeline, Heard as ChordHeard};
use mpfree_context::groove::STEP;
use mpfree_context::key::{Candidate, Key, KeyReading, Mode};
use mpfree_context::PartRole;
use mpfree_transport::time::{Meter, Swing, Tick};

use crate::cells::Stroke;
use crate::critic::Check;
use crate::dice::{Dice, DiceName, Job};
use crate::event::{Event, Role};
use crate::parts::{self, add_part_again, AddPart, Relation};
use crate::plan::{BarJob, BarPlan};
use crate::profile::InstrumentKind;
use crate::progression::{diatonic, progression_led, Need};
use crate::row::{DrumHit, Heard, Row, RowPart, ToHear};
use crate::settings::{Ending, Settings};
use crate::style::{BassLine, Feel, Style};

mod line;
mod riff;
mod written;

use line::line;
use riff::riff;
use written::{home_of, written};

/// One New idea request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NewIdea {
    pub kind: PhraseKind,
    pub style: Style,
    /// Written in a minor key (else major).
    pub minor: bool,
    /// 1, 2 or 4.
    pub bars: u8,
    /// Sparse (0) to Busy (1).
    pub activity: f32,
    pub seed: u64,
    /// Try again's count (0 the first time).
    pub attempt: u16,
}

/// What an idea's checks found.
#[derive(Clone, Debug, PartialEq)]
pub struct IdeaVerdict {
    pub checks: Vec<(&'static str, Check)>,
}

impl IdeaVerdict {
    pub fn passed(&self) -> usize {
        self.checks.iter().filter(|(_, c)| c.pass).count()
    }

    pub fn all(&self) -> bool {
        self.passed() == self.checks.len()
    }

    pub fn score(&self) -> f32 {
        self.checks.iter().map(|(_, c)| c.score).sum()
    }

    pub fn words(&self) -> String {
        self.checks
            .iter()
            .map(|(name, c)| format!("{} {name}: {}", if c.pass { "+" } else { "-" }, c.words))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// A phrase made to order, before it is the arp's.
#[derive(Clone, Debug, PartialEq)]
pub struct Idea {
    pub kind: PhraseKind,
    /// The key it is written in: C major or C minor.
    pub key: Key,
    pub bars: u8,
    /// The loop of chords it was made over, which it carries.
    pub chords: ChordTimeline,
    /// Its rhythm.
    pub strokes: Vec<Stroke>,
    /// Its notes as a phrase writes them, in ticks from its start.
    pub events: Vec<Event>,
    /// The note it is written from: the held note plays this one.
    pub home: u8,
    /// The grid it is written on: 1/8 or 1/16.
    pub grid: Rate,
    /// Written to swing.
    pub swung: bool,
    pub verdict: IdeaVerdict,
}

/// Whole drafts made of an idea; the best is offered.
pub const DRAFTS: u16 = 4;

/// How loud a phrase writes a ghost note.
pub const GHOST_VELOCITY: u8 = 48;

const METER: Meter = Meter { beats: 4, unit: 4 };

fn key_of(minor: bool) -> Key {
    Key::new(0, if minor { Mode::Minor } else { Mode::Major })
}

fn bars_of(req: &NewIdea) -> u8 {
    match req.bars {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 4,
    }
}

fn settings_of(req: &NewIdea) -> Settings {
    Settings {
        style: req.style,
        activity: req.activity.clamp(0.0, 1.0),
        bars: bars_of(req),
        ending: Ending::LoopsBack,
        seed: req.seed,
        ..Settings::default()
    }
}

const fn part_number(kind: PhraseKind) -> u16 {
    match kind {
        PhraseKind::Bass => 0x31,
        PhraseKind::Riff => 0x32,
        PhraseKind::Chords => 0x33,
        PhraseKind::Line => 0x34,
    }
}

fn dice(req: &NewIdea, job: Job, draft: u16) -> Dice {
    DiceName {
        seed: req.seed,
        part: part_number(req.kind),
        phrase: draft,
        job,
        bar: 0,
        attempt: req.attempt,
    }
    .dice()
}

// ------------------------------------------------------------- the band

/// The loop an idea is made over: the home chord alone, or the style's
/// own moves from home, leading back into it.
fn chord_loop(req: &NewIdea, key: Key, settings: &Settings) -> ChordTimeline {
    let bars = usize::from(settings.bars);
    let bar = METER.bar_ticks();
    let st = &req.style;
    let mut dice = dice(req, Job::Harmony, 0);
    // One bar is the held note's chord; longer ones move more often than
    // not.
    let moves = match bars {
        1 => false,
        2 => dice.chance(0.5),
        _ => dice.chance(0.75),
    };
    if !moves {
        let home = diatonic(key, 0, st.harmony.sevenths);
        return ChordTimeline::whole(
            bars as Tick * bar,
            ChordAnswer::Chord(home),
            ChordHeard::Suggested,
        );
    }
    let plan: Vec<BarPlan> = (0..bars)
        .map(|b| BarPlan {
            job: if b + 1 == bars {
                BarJob::Land(Ending::LoopsBack)
            } else {
                BarJob::Develop
            },
            source: 0,
            shift: 0,
            tension: 0.5,
            activity: 1.0,
            similarity: 0.5,
        })
        .collect();
    progression_led(
        &ChordTimeline::default(),
        Some(key),
        &st.harmony,
        &plan,
        settings.variation,
        req.seed,
        req.attempt,
        METER,
        &[],
        &[Need::Degrees {
            at: 0,
            degrees: vec![0],
        }],
    )
}

fn hear(key: Key, chords: &ChordTimeline, parts: &[RowPart], bars: u8) -> Row {
    let reading = KeyReading::new(&[Candidate { key, odds: 0.9 }], 7);
    Row::hear(&ToHear {
        parts,
        length: Tick::from(bars) * METER.bar_ticks(),
        meter: METER,
        key: &reading,
        played_pcs: key.pcs(),
        chords,
        groove: None,
        lead: None,
    })
}

/// The band an idea is made for: the loop's chords and the style's beat.
fn band(key: Key, chords: &ChordTimeline, settings: &Settings) -> Row {
    let alone = hear(key, chords, &[], settings.bars);
    let drums = add_part_again(
        &AddPart {
            row: &alone,
            part: PartRole::Drums,
            relation: Relation::Support,
            settings,
            kind: InstrumentKind::Percussive,
            pads: [Some(36), Some(38), Some(42), Some(47)],
            swing: Swing::default(),
        },
        0,
    );
    let hits: Vec<DrumHit> = drums
        .map(|d| d.events)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| match e.role {
            Role::Drum(lane) => Some(DrumHit {
                at: e.at,
                lane,
                velocity: e.velocity,
            }),
            _ => None,
        })
        .collect();
    let parts = [RowPart {
        role: PartRole::Drums,
        heard: Heard::Hits(hits),
    }];
    hear(key, chords, &parts, settings.bars)
}

// ------------------------------------------------------------- an idea

/// What a maker makes: the rhythm, the notes as made, and where the
/// accents fall.
struct Draft {
    strokes: Vec<Stroke>,
    events: Vec<Event>,
    accents: Vec<Tick>,
}

impl Draft {
    /// The draft as it will be heard: written as a phrase writes notes,
    /// then judged by `judge` on the rhythm and the written notes.
    fn heard(
        self,
        row: &Row,
        swung: bool,
        judge: impl FnOnce(&[Stroke], &[Event]) -> IdeaVerdict,
    ) -> Made {
        let (events, grid) = written(self.events, &self.accents, swung, row);
        let verdict = judge(&self.strokes, &events);
        Made {
            strokes: self.strokes,
            events,
            grid,
            verdict,
        }
    }
}

/// A draft written and judged.
struct Made {
    strokes: Vec<Stroke>,
    /// Its notes as a phrase writes them.
    events: Vec<Event>,
    grid: Rate,
    verdict: IdeaVerdict,
}

/// A new idea.
pub fn new_idea(req: &NewIdea) -> Idea {
    let key = key_of(req.minor);
    let settings = settings_of(req);
    let chords = chord_loop(req, key, &settings);
    let row = band(key, &chords, &settings);
    let swung = req.style.rhythm.feel == Feel::Swing8;
    let made = best(|d| match req.kind {
        PhraseKind::Bass | PhraseKind::Chords => part(req, &row, &settings, swung, d),
        PhraseKind::Riff => riff(req, &row, &settings, swung, d),
        PhraseKind::Line => line(req, &row, &settings, swung, d),
    });
    let home = home_of(req.kind, &made.events, key);
    Idea {
        kind: req.kind,
        key,
        bars: settings.bars,
        chords,
        strokes: made.strokes,
        events: made.events,
        home,
        grid: made.grid,
        swung,
        verdict: made.verdict,
    }
}

/// Attempts Try again makes for a phrase that was not offered before.
pub const TRIES: u16 = 8;

/// Try again: the idea of `req`'s attempt, or of the first attempt after
/// it whose phrase is none of `offered` (what the musician can still page
/// back to), and the attempt it was, so the next Try again asks from one
/// past it. Of two bars of chords in a sparse style there are only so
/// many, and one attempt in four or five makes a phrase an earlier one
/// made; without this, Try again would now and then answer with what it
/// was asked to replace.
///
/// `None` when [`TRIES`] attempts in a row made only phrases already
/// offered: there is nothing new to offer for this request, and the
/// caller says so instead of offering one again.
pub fn next_idea(req: &NewIdea, offered: &[Phrase]) -> Option<(Idea, u16)> {
    (0..TRIES).find_map(|k| {
        let attempt = req.attempt.wrapping_add(k);
        let idea = new_idea(&NewIdea { attempt, ..*req });
        (!offered.contains(&idea.phrase())).then_some((idea, attempt))
    })
}

/// The best of [`DRAFTS`] whole drafts: the most checks passed, then the
/// best score; of equals, the first.
fn best(draft: impl Fn(u16) -> Made) -> Made {
    let mut best: Option<Made> = None;
    for d in 0..DRAFTS {
        let m = draft(d);
        let better = best.as_ref().is_none_or(|b| {
            (m.verdict.passed(), m.verdict.score()) > (b.verdict.passed(), b.verdict.score())
        });
        if better {
            best = Some(m);
        }
    }
    best.expect("at least one draft")
}

/// One bar goes somewhere when a note of it is three semitones or more
/// from its first, either way. It has no room to wait for a high point,
/// which is what the critic asks of a tune, and a bar that falls goes
/// somewhere as one that rises does.
fn moves_off(pitches: &[i32]) -> Check {
    let Some(&first) = pitches.first() else {
        return Check::new(false, 0.0, "no notes".into());
    };
    let far = pitches
        .iter()
        .map(|&p| (p - first).abs())
        .max()
        .unwrap_or(0);
    Check::new(
        far >= 3,
        (far as f32 / 7.0).min(1.0),
        format!("{far} semitones from its first note at its farthest"),
    )
}

fn pushed_ticks(strokes: &[Stroke]) -> Vec<Tick> {
    strokes
        .iter()
        .filter(|s| s.pushed && !s.ghost)
        .map(|s| Tick::from(s.at) * STEP)
        .collect()
}

/// How a bass or chords play with the band's beat: as the style's own
/// do, and the other way on every second try.
fn relation_of(req: &NewIdea) -> Relation {
    let (usual, other) = match (req.kind, req.style.bass.line) {
        // A house bass sits between the kicks.
        (PhraseKind::Bass, BassLine::Offbeat) => (Relation::Interlock, Relation::Support),
        // A walking bass walks when busy and plays in two when not.
        (PhraseKind::Bass, BassLine::Walking) => {
            let r = if req.activity >= 0.5 {
                Relation::Interlock
            } else {
                Relation::Support
            };
            (r, r)
        }
        _ => (Relation::Support, Relation::Interlock),
    };
    if req.attempt.is_multiple_of(2) {
        usual
    } else {
        other
    }
}

/// Draft `d` of a bass or chords: Add a part's draft for the band,
/// written as a phrase and judged again by the part's checks on what was
/// written. The band always has chords and neither kind is an answer, so
/// nothing Add a part refuses can be asked here.
fn part(req: &NewIdea, row: &Row, settings: &Settings, swung: bool, d: u16) -> Made {
    let (part, kind) = match req.kind {
        PhraseKind::Bass => (PartRole::Bass, InstrumentKind::Bass),
        _ => (PartRole::Chords, InstrumentKind::Chords),
    };
    let band = AddPart {
        row,
        part,
        relation: relation_of(req),
        settings,
        kind,
        pads: [None; 4],
        swing: Swing::default(),
    };
    let added = parts::draft(&band, d, req.attempt);
    Draft {
        accents: pushed_ticks(&added.strokes),
        strokes: added.strokes,
        events: added.events,
    }
    .heard(row, swung, |strokes, heard| {
        let mut v = parts::judge(&band, strokes, heard);
        if settings.bars <= 2 {
            // Add a part asks a riff to come back within the row. A
            // phrase is a loop: two bars that differ come back by going
            // round, which a row played once in a song does not.
            v.repeats = Check::new(
                true,
                1.0,
                "a loop of two bars comes round by itself".to_string(),
            );
        }
        IdeaVerdict {
            checks: v
                .checks()
                .iter()
                .map(|(name, c)| (*name, (*c).clone()))
                .collect(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::STYLES;
    use mpfree_arp::phrase::{ACCENT_VELOCITY, GATE_SHORT, GATE_WRITTEN, PLAIN_VELOCITY};

    /// Every kind, style, scale and length, sparse, middling and busy,
    /// `seeds` of each.
    fn requests(seeds: u64) -> Vec<NewIdea> {
        let mut v = Vec::new();
        for kind in PhraseKind::ALL {
            for style in STYLES {
                for minor in [true, false] {
                    for bars in [1u8, 2, 4] {
                        for activity in [0.2f32, 0.5, 0.8] {
                            for seed in 1..=seeds {
                                v.push(NewIdea {
                                    kind,
                                    style,
                                    minor,
                                    bars,
                                    activity,
                                    seed,
                                    attempt: 0,
                                });
                            }
                        }
                    }
                }
            }
        }
        v
    }

    fn row_of(req: &NewIdea) -> Row {
        let key = key_of(req.minor);
        let settings = settings_of(req);
        band(key, &chord_loop(req, key, &settings), &settings)
    }

    fn check<'a>(idea: &'a Idea, name: &str) -> &'a Check {
        &idea
            .verdict
            .checks
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("no check named {name}"))
            .1
    }

    #[test]
    fn an_idea_is_a_phrase_that_reads_back_from_its_text() {
        // Keep writes an idea into the owner's phrases as text, so what
        // comes back from the text has to be the phrase that was heard.
        for req in requests(1) {
            let idea = new_idea(&req);
            let p = idea.phrase();
            assert!(!p.is_empty(), "{req:?}");
            assert_eq!((p.kind, p.bars), (req.kind, req.bars), "{req:?}");
            assert_eq!(p.swung, req.style.rhythm.feel == Feel::Swing8);
            assert_eq!(Phrase::parse(&p.to_text()), Ok(p), "{}", p.to_text());
        }
    }

    #[test]
    fn the_same_request_makes_the_same_idea() {
        for req in requests(1).into_iter().step_by(7) {
            assert_eq!(new_idea(&req), new_idea(&req));
        }
    }

    #[test]
    fn an_idea_is_written_as_a_phrase_writes_notes() {
        // On its grid; as long as whole steps with the last one short
        // (a quarter of a step) or plain (four fifths), or held for whole
        // steps; plain, accented or a ghost.
        for req in requests(1) {
            let idea = new_idea(&req);
            let unit = match idea.grid {
                Rate::Eighth => 2 * STEP,
                _ => STEP,
            };
            let short = unit * Tick::from(GATE_SHORT) / 100;
            let plain = unit * Tick::from(GATE_WRITTEN) / 100;
            for e in &idea.events {
                assert_eq!(e.at % unit, 0, "{req:?}");
                assert!(
                    e.len == short || [0, plain].contains(&(e.len % unit)),
                    "{req:?}: a note {} ticks long",
                    e.len
                );
                assert!(e.len > 0);
                assert!(
                    [PLAIN_VELOCITY, ACCENT_VELOCITY, GHOST_VELOCITY].contains(&e.velocity),
                    "{req:?}"
                );
            }
        }
    }

    #[test]
    fn try_again_offers_a_phrase_that_was_not_offered() {
        // One bar is where there are fewest: twelve taps of each kind,
        // with the last eight kept to page back to.
        for kind in PhraseKind::ALL {
            for style in STYLES {
                let mut history: Vec<Phrase> = Vec::new();
                let mut attempt = 0;
                for tap in 0..12 {
                    let req = NewIdea {
                        kind,
                        style,
                        minor: true,
                        bars: 1,
                        activity: 0.5,
                        seed: 3,
                        attempt,
                    };
                    let (idea, at) = next_idea(&req, &history)
                        .unwrap_or_else(|| panic!("{} {kind:?}: tap {tap} ran dry", style.name));
                    assert!(at >= attempt);
                    assert_eq!(idea, new_idea(&NewIdea { attempt: at, ..req }));
                    assert!(!history.contains(&idea.phrase()));
                    history.push(idea.phrase());
                    if history.len() > 8 {
                        history.remove(0);
                    }
                    attempt = at + 1;
                }
            }
        }
    }

    #[test]
    fn try_again_says_so_when_nothing_new_is_left() {
        let req = NewIdea {
            kind: PhraseKind::Chords,
            style: STYLES[0],
            minor: true,
            bars: 1,
            activity: 0.5,
            seed: 1,
            attempt: 0,
        };
        // Nothing offered yet: the request's own idea.
        assert_eq!(next_idea(&req, &[]), Some((new_idea(&req), 0)));
        // Every attempt it would make already offered: nothing.
        let all: Vec<Phrase> = (0..TRIES)
            .map(|attempt| new_idea(&NewIdea { attempt, ..req }).phrase())
            .collect();
        assert_eq!(next_idea(&req, &all), None);
        // The first of them offered: the next that differs from it.
        let (idea, at) = next_idea(&req, &all[..1]).expect("another");
        assert!(at >= 1);
        assert_ne!(idea.phrase(), all[0]);
    }

    /// How many of `reqs` of `kind` and `bars` pass every check.
    fn passing(reqs: &[NewIdea], kind: PhraseKind, bars: u8) -> (usize, usize) {
        let of: Vec<&NewIdea> = reqs
            .iter()
            .filter(|r| r.kind == kind && r.bars == bars)
            .collect();
        let pass = of.iter().filter(|r| new_idea(r).verdict.all()).count();
        (pass, of.len())
    }

    #[test]
    fn ideas_pass_their_checks() {
        // A ratchet: of 36 ideas of each kind and length (two of every
        // style, scale and way of busy), this many pass every check
        // today, and no fewer may tomorrow. What a bass misses is Add a
        // part's own: with the kick six notes in ten, and a bar of four
        // that comes back. The whole count, of 720 each, is
        // `print_how_often_ideas_pass`.
        let reqs = requests(2);
        for (kind, floors) in [
            (PhraseKind::Bass, [36, 32, 26]),
            (PhraseKind::Riff, [36, 36, 36]),
            (PhraseKind::Chords, [36, 36, 36]),
            (PhraseKind::Line, [36, 36, 36]),
        ] {
            for (bars, floor) in [1u8, 2, 4].into_iter().zip(floors) {
                let (pass, of) = passing(&reqs, kind, bars);
                assert_eq!(of, 36);
                assert!(
                    pass >= floor,
                    "{kind:?} of {bars} bars: {pass} of {of} pass every check, {floor} asked"
                );
            }
        }
    }

    #[test]
    fn a_riff_is_a_tune_that_breathes_and_leads_back() {
        // Whatever else a riff misses, these three it never does.
        for req in requests(2) {
            if req.kind != PhraseKind::Riff {
                continue;
            }
            let idea = new_idea(&req);
            for name in ["has a tune", "breathes", "leads back"] {
                let c = check(&idea, name);
                assert!(c.pass, "{req:?}: {name}: {}", c.words);
            }
        }
    }

    #[test]
    fn a_riff_rests_where_its_plan_rests() {
        let mut rests = 0;
        for req in requests(2) {
            if req.kind != PhraseKind::Riff || req.bars < 2 {
                continue;
            }
            let row = row_of(&req);
            let idea = new_idea(&req);
            for &(from, to) in &row.plan.rests {
                rests += 1;
                let (a, b) = (row.tick(from), row.tick(to));
                assert!(
                    !idea.events.iter().any(|e| e.at < b && e.at + e.len > a),
                    "{req:?}: a note sounds in the rest from step {from} to {to}: {}",
                    idea.phrase().to_text()
                );
            }
        }
        assert!(rests > 0);
    }

    #[test]
    fn two_bars_come_round_by_themselves() {
        // Add a part asks a part to come back within its row; a phrase
        // of two bars comes back by going round, so it is not asked.
        for req in requests(1) {
            if !matches!(req.kind, PhraseKind::Bass | PhraseKind::Chords) || req.bars > 2 {
                continue;
            }
            let idea = new_idea(&req);
            assert!(check(&idea, "returns, not stuck").pass, "{req:?}");
        }
    }

    #[test]
    fn one_bar_goes_somewhere_when_it_moves_off_its_first_note() {
        assert!(moves_off(&[60, 62, 63, 60]).pass);
        // Falling counts as rising does.
        assert!(moves_off(&[67, 65, 63, 60]).pass);
        assert!(!moves_off(&[60, 62, 60, 58]).pass);
        assert!(!moves_off(&[60, 60, 60]).pass);
        assert!(!moves_off(&[]).pass);
    }

    #[test]
    fn a_bass_takes_the_other_way_with_the_kick_on_every_second_try() {
        let mut req = NewIdea {
            kind: PhraseKind::Bass,
            style: STYLES[0],
            minor: true,
            bars: 2,
            activity: 0.5,
            seed: 1,
            attempt: 0,
        };
        let usual = relation_of(&req);
        req.attempt = 1;
        assert_ne!(relation_of(&req), usual);
        req.attempt = 2;
        assert_eq!(relation_of(&req), usual);
    }

    /// How often each check passes over many ideas (`IDEA_SEEDS` of every
    /// kind, length, style, scale and three ways of busy; 40 unless set),
    /// and how many of them differ. With `IDEA_SHOW=1`, three that miss
    /// each check. `cargo test --release -p mpfree-compose --lib
    /// print_how_often_ideas_pass -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_how_often_ideas_pass() {
        use std::collections::{BTreeMap, BTreeSet};
        let seeds: u64 = std::env::var("IDEA_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(40);
        for kind in PhraseKind::ALL {
            for bars in [1u8, 2, 4] {
                let mut all = 0usize;
                let mut total = 0usize;
                let mut misses: BTreeMap<&'static str, usize> = BTreeMap::new();
                let mut shown: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
                let mut texts: BTreeSet<String> = BTreeSet::new();
                for style in STYLES {
                    for minor in [true, false] {
                        for activity in [0.2f32, 0.5, 0.8] {
                            for seed in 1..=seeds {
                                let idea = new_idea(&NewIdea {
                                    kind,
                                    style,
                                    minor,
                                    bars,
                                    activity,
                                    seed,
                                    attempt: 0,
                                });
                                total += 1;
                                all += usize::from(idea.verdict.all());
                                for (name, c) in &idea.verdict.checks {
                                    if !c.pass {
                                        *misses.entry(name).or_default() += 1;
                                        let v = shown.entry(name).or_default();
                                        if v.len() < 3 {
                                            v.push(format!(
                                                "{} [{}] {}\n        {}",
                                                style.name,
                                                idea.chords.names(),
                                                idea.phrase().to_text(),
                                                c.words
                                            ));
                                        }
                                    }
                                }
                                texts.insert(format!(
                                    "{}{minor}{}",
                                    style.name,
                                    idea.phrase().to_text()
                                ));
                            }
                        }
                    }
                }
                println!(
                    "{} {bars} bars: {all} of {total} pass every check ({:.0} %), {} different ({:.0} %); misses {misses:?}",
                    kind.label(),
                    100.0 * all as f64 / total as f64,
                    texts.len(),
                    100.0 * texts.len() as f64 / total as f64,
                );
                if std::env::var("IDEA_SHOW").is_ok() {
                    for (name, v) in &shown {
                        for x in v {
                            println!("    - {name}: {x}");
                        }
                    }
                }
            }
        }
    }

    /// Sittings of Try again, 24 taps each with the last eight kept: how
    /// often nothing new came, and how long an idea takes to make.
    /// `cargo test --release -p mpfree-compose --lib
    /// print_try_again_sittings -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_try_again_sittings() {
        use std::time::Instant;
        for kind in PhraseKind::ALL {
            for bars in [1u8, 2, 4] {
                let (mut taps, mut none, mut made) = (0usize, 0usize, 0usize);
                let mut worst = 0usize;
                let t0 = Instant::now();
                for style in STYLES {
                    for minor in [true, false] {
                        for activity in [0.2f32, 0.5, 0.8] {
                            for seed in 1..=3u64 {
                                let mut history: Vec<Phrase> = Vec::new();
                                let mut attempt = 0u16;
                                let mut dry = 0usize;
                                for _ in 0..24 {
                                    taps += 1;
                                    let req = NewIdea {
                                        kind,
                                        style,
                                        minor,
                                        bars,
                                        activity,
                                        seed,
                                        attempt,
                                    };
                                    match next_idea(&req, &history) {
                                        Some((idea, a)) => {
                                            made += usize::from(a - attempt) + 1;
                                            attempt = a + 1;
                                            history.push(idea.phrase());
                                            if history.len() > 8 {
                                                history.remove(0);
                                            }
                                        }
                                        None => {
                                            made += usize::from(TRIES);
                                            attempt += TRIES;
                                            none += 1;
                                            dry += 1;
                                        }
                                    }
                                }
                                worst = worst.max(dry);
                            }
                        }
                    }
                }
                println!(
                    "{} {bars} bars: {none} of {taps} taps found nothing new ({:.1} %), {worst} of 24 in the worst sitting; {:.2} ideas made a tap, {:.1} ms an idea",
                    kind.label(),
                    100.0 * none as f64 / taps as f64,
                    made as f64 / taps as f64,
                    t0.elapsed().as_secs_f64() * 1000.0 / made.max(1) as f64,
                );
            }
        }
    }

    /// Ideas to read: three of every style and length in a minor key, of
    /// every kind or of `IDEA_KIND` (bass, riff, chords, line), each with
    /// its chords, its phrase and the checks it misses. `cargo test -p
    /// mpfree-compose --lib print_ideas -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_ideas() {
        let only = std::env::var("IDEA_KIND").unwrap_or_default();
        for kind in PhraseKind::ALL {
            if !only.is_empty() && !kind.word().eq_ignore_ascii_case(&only) {
                continue;
            }
            for style in STYLES {
                for bars in [1u8, 2, 4] {
                    for seed in 1..=3u64 {
                        let idea = new_idea(&NewIdea {
                            kind,
                            style,
                            minor: true,
                            bars,
                            activity: 0.5,
                            seed,
                            attempt: 0,
                        });
                        println!(
                            "{} {} {} bars seed {seed}: [{}] home {} passed {}/{}",
                            style.name,
                            kind.label(),
                            bars,
                            idea.chords.names(),
                            idea.home,
                            idea.verdict.passed(),
                            idea.verdict.checks.len(),
                        );
                        println!("  {}", idea.phrase().to_text());
                        for (name, c) in &idea.verdict.checks {
                            if !c.pass {
                                println!("    - {name}: {}", c.words);
                            }
                        }
                    }
                }
            }
        }
    }
}
