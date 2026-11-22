//! Phase 0B's gate: the model checked against the real engine. Results
//! play through the real play view and player (`mpfree-play`), and Keep
//! and UNDO go through the real take undo (`mpfree_play::Takes::edit`,
//! the transaction the app's step edits use).
//!
//! - `heard` (the timing contract's step 4) lands on the player's frame
//!   for every note, with quantise, speed, the clip's swing and the
//!   transport's, and a clip that started off the bar line;
//! - one phrase is auditioned in place of a clip, kept as one take that
//!   plays the same notes on the same frames, and undone back to the clip
//!   as it was, revision and all;
//! - a step edit to the clip while its result plays invalidates it, and
//!   so does an edit to a clip the reading listened to.

use mpfree_compose::bind::{audition, revision, Binding, Stale};
use mpfree_compose::preview::{Arrival, Preview, Proposal, Refusal, State};
use mpfree_compose::realize::{realize, Fit};
use mpfree_compose::timing::{heard_tick, Playback};
use mpfree_compose::{
    clip_notes, continuation, InstrumentKind, Motion, NoteDecision, Part, PartId, Role,
};
use mpfree_context::chord::{Chord, ChordTimeline, Heard, Quality, Tone};
use mpfree_context::key::{Key, Mode};
use mpfree_context::{ClipRead, PartRole};
use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink, Takes};
use mpfree_seq::steps::Grid;
use mpfree_seq::{Clip, Holds, Note, Project, Quantise, Speed, Track};
use mpfree_transport::time::{Swing, Tick, PPQ};
use mpfree_transport::{Block, Command, Transport};

const BEAT: Tick = PPQ as Tick;
const BAR: Tick = 4 * BEAT;
/// 120 BPM at 48 kHz: 25 frames a tick, so a tick is a whole frame.
const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
const FRAMES_PER_TICK: u64 = 25;

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push(o);
    }
}

/// Note-ons as (frame, note), in the order played, for `ticks` song
/// ticks from the start, with the transport's swing.
fn play(view: &PlayView, transport_swing: Option<Swing>, ticks: Tick) -> Vec<(u64, u8)> {
    let mut t = Transport::new(RATE, BPM).unwrap();
    if let Some(s) = transport_swing {
        assert!(t.command(0, Command::SetSwing(s)));
    }
    assert!(t.command(0, Command::Play));
    let frames = ticks as u64 * FRAMES_PER_TICK;
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    while t.frame() < frames {
        t.process(256.min((frames - t.frame()) as u32), &mut block);
        player.process(view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped + player.stats().late_offs, 0);
    log.0
        .iter()
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some((o.frame, note)),
            _ => None,
        })
        .collect()
}

fn one_clip(c: Clip) -> Project {
    let mut p = Project::new();
    let mut t = Track::new("Lead", 0);
    t.slots[0] = Some(c);
    p.tracks.push(t);
    p.give_clip_ids();
    p
}

fn launch(origin: Tick) -> [Launch; 1] {
    [Launch {
        track: 0,
        slot: 0,
        origin,
        start: origin,
    }]
}

/// What `heard` says the player plays: every note on every pass that
/// starts before `ticks`, as (frame, note), in time order.
fn predicted(c: &Clip, swing: Swing, origin: Tick, ticks: Tick) -> Vec<(u64, u8)> {
    let song_len = c.speed.to_song(c.length as f64) as Tick;
    let mut out = Vec::new();
    let mut start = origin;
    while start < ticks {
        let pb = Playback {
            quantise: c.quantise,
            speed: c.speed,
            swing: c.swing.unwrap_or(swing),
            length: c.length,
            start,
        };
        for n in &c.notes {
            let at = heard_tick(n.tick, &pb);
            if (0..ticks).contains(&at) {
                out.push((at as u64 * FRAMES_PER_TICK, n.note));
            }
        }
        start += song_len;
    }
    out.sort();
    out
}

fn sorted(mut v: Vec<(u64, u8)>) -> Vec<(u64, u8)> {
    v.sort();
    v
}

/// Notes on and off the grid, near its lines and at the clip's very end.
fn uneven_clip() -> Clip {
    let mut c = Clip::new("uneven", BAR);
    c.holds = Holds::Notes;
    for (i, tick) in [0, 233, 240, 487, 720, 1199, 1440, 1921, 2650, 3000, 3839]
        .into_iter()
        .enumerate()
    {
        c.notes.push(Note::new(tick, 60, 60 + i as u8, 100));
    }
    c
}

#[test]
fn heard_lands_on_the_players_frame_for_every_note() {
    let swing16 = Swing::new(240, 62).unwrap();
    let swing8 = Swing::new(480, 58).unwrap();
    let cases: Vec<(&str, Clip, Option<Swing>, Tick)> = {
        let plain = uneven_clip();
        let mut quantised = uneven_clip();
        quantised.quantise = Quantise::new(240, 50);
        let mut half = uneven_clip();
        half.speed = Speed::new(1, 2).unwrap();
        let mut double = uneven_clip();
        double.speed = Speed::new(2, 1).unwrap();
        let mut own_swing = uneven_clip();
        own_swing.swing = Some(swing8);
        let mut everything = uneven_clip();
        everything.quantise = Quantise::new(480, 75);
        everything.speed = Speed::new(3, 4).unwrap();
        everything.swing = Some(swing16);
        vec![
            ("plain", plain.clone(), None, 0),
            ("transport swing", plain.clone(), Some(swing16), 0),
            ("off the bar line", plain, Some(swing16), BAR / 2 + 30),
            ("quantise", quantised, Some(swing16), 0),
            ("half speed", half, Some(swing16), 0),
            ("double speed", double, Some(swing8), 0),
            ("the clip's own swing wins", own_swing, Some(swing16), 0),
            ("everything at once", everything, Some(swing8), 120),
        ]
    };
    for (name, c, transport, origin) in cases {
        let p = one_clip(c.clone());
        let view = PlayView::compile(&p, &launch(origin)).unwrap();
        let ticks = 4 * BAR;
        let heard = sorted(play(&view, transport, ticks));
        let want = predicted(&c, transport.unwrap_or_default(), origin, ticks);
        assert!(!want.is_empty(), "{name}");
        assert_eq!(heard, want, "{name}: heard and the player differ");
    }
}

/// A one-bar answer to a C minor riff, from decisions realised under the
/// chord and kept as clip notes in the clip's second bar.
fn phrase() -> Vec<Note> {
    let line = ChordTimeline::of(&[(Chord::new(0, Quality::Minor), 2 * BAR)], Heard::Heard);
    let fit = Fit {
        harmony: &line,
        key: Some(Key::new(0, Mode::Minor)),
        played_pcs: 0,
        origin: 0,
    };
    let d = |at, role, m| NoteDecision::new(BAR + at, BEAT / 2, 96, role, m);
    let part = Part {
        id: PartId(1),
        role: PartRole::Melody,
        kind: InstrumentKind::OneNote,
        register: 67,
        decisions: vec![
            d(0, Role::Chord(Tone::Fifth), Motion::NEAREST),
            d(BEAT, Role::Passing, Motion::down(1)),
            d(2 * BEAT, Role::Chord(Tone::Third), Motion::down(4)),
            d(5 * BEAT / 2, Role::Neighbour, Motion::up(1)),
            d(3 * BEAT, Role::Chord(Tone::Root), Motion::down(3)),
        ],
    };
    let events = realize(&part, &fit);
    clip_notes(&events, BPM, 2 * BAR).unwrap()
}

fn riff() -> Clip {
    let mut c = Clip::new("riff", BAR);
    c.holds = Holds::Notes;
    c.notes = vec![
        Note::new(0, 240, 60, 110),
        Note::new(480, 240, 63, 90),
        Note::new(960, 480, 67, 100),
        Note::new(1920, 240, 70, 95),
        Note::new(2880, 480, 67, 100),
    ];
    c
}

#[test]
fn a_phrase_is_heard_in_place_of_the_clip_kept_as_one_take_and_undone() {
    let mut project = one_clip(riff());
    let launches = launch(0);
    let ticks = 4 * BAR;
    let before = sorted(play(
        &PlayView::compile(&project, &launches).unwrap(),
        None,
        ticks,
    ));
    let clip_before = project.clip(0, 0).unwrap().clone();
    let mut takes = Takes::new(16);
    let mut preview = Preview::new();

    // Continue: ask, and the composer's result arrives.
    let b = Binding::of(&project, 0, 0).unwrap();
    let n = preview.ask(b).unwrap();
    let notes = phrase();
    assert_eq!(notes.len(), 5);
    assert!(notes.iter().all(|n| (BAR..2 * BAR).contains(&n.tick)));
    let edit = continuation(&clip_before, notes.clone(), 2 * BAR).unwrap();
    let proposal = Proposal {
        edit,
        read: vec![ClipRead {
            clip: b.clip,
            revision: b.revision,
            role: PartRole::Melody,
        }],
    };
    assert_eq!(preview.arrived(n, proposal, &project), Arrival::Taken);

    // Heard in place of the clip, through the real play view and player.
    preview.audition().unwrap();
    let (pb, pe) = preview.plays().expect("it plays");
    let heard_project = audition(&project, pb, pe).unwrap();
    let heard = sorted(play(
        &PlayView::compile(&heard_project, &launches).unwrap(),
        None,
        ticks,
    ));
    // The riff's bar, then the answer, twice: on the frames `heard` says.
    let mut longer = clip_before.clone();
    longer.length = 2 * BAR;
    longer.notes.extend(&notes);
    assert_eq!(heard, predicted(&longer, Swing::default(), 0, ticks));
    assert_ne!(heard, before);
    assert_eq!(
        revision(project.clip(0, 0).unwrap()),
        b.revision,
        "nothing written yet"
    );

    // Keep: one take, through the real take undo.
    let (kb, ke) = preview.keep(&project).unwrap();
    let take = takes.edit(&mut project, kb.track, kb.slot, &ke, None);
    let take = take.expect("the clip takes the edit");
    preview.kept(Ok(take));
    assert_eq!((preview.state(), preview.take()), (State::Idle, Some(take)));
    let kept = sorted(play(
        &PlayView::compile(&project, &launches).unwrap(),
        None,
        ticks,
    ));
    assert_eq!(kept, heard, "the kept clip plays what the preview played");
    let kept_rev = revision(project.clip(0, 0).unwrap());
    assert_ne!(kept_rev, b.revision);
    assert_eq!(project.clip(0, 0).unwrap().length, 2 * BAR);

    // UNDO takes it all back: the length, the notes, the revision.
    assert_eq!(takes.undo(&mut project), Some(take));
    let undone = project.clip(0, 0).unwrap();
    assert_eq!(undone.length, BAR);
    assert_eq!(revision(undone), b.revision);
    assert_eq!(
        sorted(play(
            &PlayView::compile(&project, &launches).unwrap(),
            None,
            ticks
        )),
        before
    );
    // REDO brings it back as kept.
    takes.redo(&mut project).unwrap();
    assert_eq!(revision(project.clip(0, 0).unwrap()), kept_rev);
}

#[test]
fn a_step_edit_while_listening_invalidates_the_result() {
    let mut project = one_clip(riff());
    let mut takes = Takes::new(16);
    let mut preview = Preview::new();
    let b = Binding::of(&project, 0, 0).unwrap();
    let n = preview.ask(b).unwrap();
    let edit = continuation(project.clip(0, 0).unwrap(), phrase(), 2 * BAR).unwrap();
    preview.arrived(
        n,
        Proposal {
            edit,
            read: Vec::new(),
        },
        &project,
    );
    preview.audition().unwrap();

    // A step tapped on the clip, as the Sequencer makes it.
    let grid = Grid::new(BEAT / 4).unwrap();
    let tap = grid.toggle(project.clip(0, 0).unwrap(), 72, 3, 100);
    assert!(!tap.is_empty());
    takes.edit(&mut project, 0, 0, &tap, None).unwrap();
    assert_eq!(preview.clip_changed(&project), Some(Stale::Changed));
    assert_eq!(
        (preview.state(), preview.plays()),
        (State::Invalidated, None)
    );
    assert_eq!(preview.keep(&project), Err(Refusal::Nothing));
    // The tap's UNDO brings the clip back to the revision it was bound to;
    // the result stays dropped, and Try again asks afresh.
    takes.undo(&mut project).unwrap();
    assert_eq!(revision(project.clip(0, 0).unwrap()), b.revision);
    assert_eq!(preview.state(), State::Invalidated);
    assert!(preview.ask(Binding::of(&project, 0, 0).unwrap()).is_ok());
}

#[test]
fn an_edit_to_a_clip_the_reading_listened_to_invalidates_too() {
    let mut project = one_clip(riff());
    let mut drums = Track::new("Drums", 1);
    let mut beat = Clip::new("beat", BAR);
    beat.notes = (0..4).map(|i| Note::new(i * BEAT, 60, 0, 120)).collect();
    drums.slots[0] = Some(beat);
    project.tracks.push(drums);
    project.give_clip_ids();
    let mut takes = Takes::new(16);

    let b = Binding::of(&project, 0, 0).unwrap();
    let heard = Binding::of(&project, 1, 0).unwrap();
    let mut preview = Preview::new();
    let n = preview.ask(b).unwrap();
    let edit = continuation(project.clip(0, 0).unwrap(), phrase(), 2 * BAR).unwrap();
    let read = vec![ClipRead {
        clip: heard.clip,
        revision: heard.revision,
        role: PartRole::Drums,
    }];
    assert_eq!(
        preview.arrived(n, Proposal { edit, read }, &project),
        Arrival::Taken
    );
    let grid = Grid::new(BEAT / 4).unwrap();
    let tap = grid.toggle(project.clip(1, 0).unwrap(), 0, 2, 100);
    takes.edit(&mut project, 1, 0, &tap, None).unwrap();
    assert_eq!(preview.clip_changed(&project), Some(Stale::Heard));
    assert_eq!(preview.state(), State::Invalidated);
}
