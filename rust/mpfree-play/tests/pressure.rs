//! Pad pressure in clips: the recorder thins the armed track's pad
//! pressure onto the notes it belongs to (a value 2 steps from the last
//! one kept at once, a one-step value once it has lasted 10 ms), the
//! player sends a note's values on their ticks after quantise and swing
//! moved it, until the note ends, and both are the same for any period
//! size.

use mpfree_play::{Event, Launch, Out, PlayView, Player, Recorder, Sink, Taken, Takes};
use mpfree_seq::{Clip, Note, Project, Quantise, Speed, Touch, TouchPoint, Track};
use mpfree_transport::{Block, Command, Transport};

/// Frames per tick at 120 BPM and 44.1 kHz.
const K: f64 = 22.968_75;

/// Frame of song tick `t` at 120 BPM from frame 0.
fn frame(t: i64) -> u64 {
    (t as f64 * K).round() as u64
}

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

fn press(note: u8) -> Event {
    Event::NoteOn {
        note,
        velocity: 100,
    }
}

fn lift(note: u8) -> Event {
    Event::NoteOff { note }
}

fn squeeze(note: u8, value: u8) -> Event {
    Event::Pressure { note, value }
}

struct Session {
    project: Project,
    commands: Vec<(u64, Command)>,
    /// Live input at absolute frames, in frame order.
    live: Vec<(u64, Event)>,
    frames: u64,
}

struct Run {
    /// Recorded notes, with their pressure as the clip keeps it.
    taken: Vec<(Taken, Vec<TouchPoint>)>,
    played: Vec<Out>,
    project: Project,
    touch_dropped: u64,
}

/// A clip in slot 0 of track 0, launched from tick 0 and armed.
fn session(clip: Clip) -> Session {
    let mut p = Project::new();
    let mut t = Track::new("keys", 0);
    t.slots[0] = Some(clip);
    p.tracks.push(t);
    Session {
        project: p,
        commands: vec![(0, Command::Play)],
        live: vec![],
        frames: 0,
    }
}

const LAUNCH: Launch = Launch {
    track: 0,
    slot: 0,
    origin: 0,
    start: 0,
};

/// Play the session in periods of `size` frames. Recorded notes go into
/// the clip with their pressure (through [`Takes::add_touched`]) and the
/// view is compiled again.
fn run(s: &Session, size: u32) -> Run {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    for &(f, c) in &s.commands {
        assert!(t.command(f, c));
    }
    let mut project = s.project.clone();
    let compile = |p: &Project| {
        let mut v = PlayView::compile(p, &[LAUNCH]).unwrap();
        v.set_armed(Some(0));
        v
    };
    let mut view = compile(&project);
    let mut player = Player::new();
    let mut rec = Recorder::new();
    let mut block = Block::new();
    let mut log = Log::default();
    let mut takes = Takes::default();
    let mut taken = vec![];
    let mut live = s.live.iter().peekable();
    while t.frame() < s.frames {
        let n = size.min((s.frames - t.frame()) as u32);
        let start = t.frame();
        t.process(n, &mut block);
        player.process(&view, &block, &mut log);
        while let Some(&&(f, e)) = live.peek() {
            if f >= start + u64::from(n) {
                break;
            }
            rec.input((f - start) as u32, e);
            live.next();
        }
        rec.process(&view, &block);
        if !rec.taken().is_empty() {
            for k in rec.taken() {
                let points = rec.touch_points(&k.note).to_vec();
                let (note, kept) = takes.add_touched(&mut project, k, &points).unwrap();
                assert!(kept);
                let clip = project.tracks[0].slots[0].as_ref().unwrap();
                assert_eq!(clip.touch_of(&note), &points[..]);
                taken.push((Taken { note, ..*k }, points));
            }
            view = compile(&project);
        }
    }
    assert_eq!(rec.dropped(), 0);
    assert_eq!(player.stats().dropped, 0);
    Run {
        taken,
        played: log.0,
        project,
        touch_dropped: rec.touch_dropped(),
    }
}

const SIZES: [u32; 6] = [1, 7, 64, 128, 333, 4096];

fn pt(at: u32, value: u8) -> TouchPoint {
    TouchPoint { at, value }
}

/// (frame, event) of the played pressure and notes, for one note number.
fn heard(r: &Run, note: u8) -> Vec<(u64, Event)> {
    r.played
        .iter()
        .filter(|o| match o.event {
            Event::NoteOn { note: n, .. } | Event::NoteOff { note: n } => n == note,
            Event::Pressure { note: n, .. } => n == note,
            _ => false,
        })
        .map(|o| (o.frame, o.event))
        .collect()
}

#[test]
fn pressure_is_thinned_onto_its_note_the_same_for_any_period_size() {
    let mut s = session(Clip::new("take", 3840));
    s.commands = vec![(0, Command::Record)];
    let settle = 441; // 10 ms at 44.1 kHz
    s.live = vec![
        (0, press(1)),
        // 10 steps from 0: kept at once.
        (100, squeeze(1, 10)),
        // One step and back: a finger's jitter, not kept.
        (200, squeeze(1, 11)),
        (300, squeeze(1, 10)),
        // One step that lasts 10 ms: kept when the time is up.
        (400, squeeze(1, 11)),
        (400 + settle + 200, squeeze(1, 11)),
        // A big move: kept at once.
        (2000, squeeze(1, 30)),
        // One step, let go before 10 ms: not kept.
        (2100, squeeze(1, 31)),
        (2300, lift(1)),
        // Pressure on a pad not held is nothing.
        (3000, squeeze(1, 90)),
        (3000, squeeze(2, 90)),
    ];
    s.frames = 20_000;
    let tick = |f: u64| (f as f64 / K).round() as u32;
    let want = vec![
        pt(tick(100), 10),
        pt(tick(400 + settle), 11),
        pt(tick(2000), 30),
    ];
    for size in SIZES {
        let r = run(&s, size);
        assert_eq!(r.taken.len(), 1, "size {size}");
        let (k, points) = &r.taken[0];
        assert_eq!(k.note.note, 1);
        assert_eq!(points, &want, "size {size}");
        assert_eq!(k.note.touch, Touch { first: 0, len: 3 });
        assert_eq!(r.touch_dropped, 0);
    }
}

#[test]
fn pressure_is_timed_in_the_clips_own_ticks_from_the_notes_start() {
    // A clip at double speed: 100 song ticks after the press are 200 of
    // the clip's.
    let mut c = Clip::new("fast", 3840);
    c.speed = Speed::new(2, 1).unwrap();
    let mut s = session(c);
    s.commands = vec![(0, Command::Record)];
    s.live = vec![
        (frame(480), press(4)),
        (frame(580), squeeze(4, 50)),
        (frame(900), squeeze(4, 90)),
        (frame(1000), lift(4)),
    ];
    s.frames = frame(4000);
    for size in SIZES {
        let r = run(&s, size);
        let (k, points) = &r.taken[0];
        assert_eq!(k.note.tick, 960, "the clip tick of song tick 480");
        assert_eq!(points, &vec![pt(200, 50), pt(840, 90)], "size {size}");
    }
}

/// One note with pressure: tick 150, 960 long, values at 0, 240, 480
/// and one past its end.
fn pressed_clip() -> Clip {
    let mut c = Clip::new("pressed", 3840);
    let touch = c
        .add_touch(&[pt(0, 20), pt(240, 60), pt(480, 100), pt(1200, 5)])
        .unwrap();
    c.add_note(Note {
        touch,
        ..Note::new(150, 960, 7, 100)
    })
    .unwrap();
    c
}

#[test]
fn pressure_plays_on_its_note_wherever_quantise_puts_it() {
    let mut c = pressed_clip();
    // Quantise moves the note from tick 150 to 240: its pressure goes too.
    c.quantise = Quantise::new(240, 100);
    let mut s = session(c);
    s.frames = frame(3000);
    let want = vec![
        (frame(240), press(7)),
        (frame(240), squeeze(7, 20)),
        (frame(480), squeeze(7, 60)),
        (frame(720), squeeze(7, 100)),
        // The value past the note's end is not sent.
        (frame(1200), lift(7)),
    ];
    for size in SIZES {
        let r = run(&s, size);
        assert_eq!(heard(&r, 7), want, "size {size}");
    }
}

#[test]
fn a_ratchet_keeps_its_pressure_across_its_hits() {
    let mut c = Clip::new("ratchet", 3840);
    let touch = c.add_touch(&[pt(0, 10), pt(600, 70)]).unwrap();
    c.add_note(Note {
        touch,
        ratchet: 2,
        ..Note::new(0, 960, 3, 100)
    })
    .unwrap();
    let mut s = session(c);
    s.frames = frame(2000);
    let want = vec![
        (frame(0), press(3)),
        (frame(0), squeeze(3, 10)),
        (frame(480), lift(3)),
        (frame(480), press(3)),
        (frame(600), squeeze(3, 70)),
        (frame(960), lift(3)),
    ];
    for size in SIZES {
        let r = run(&s, size);
        assert_eq!(heard(&r, 3), want, "size {size}");
    }
}

#[test]
fn a_loop_jump_ends_the_pressure_and_the_next_pass_starts_it_again() {
    let mut c = Clip::new("long", 3840);
    let touch = c.add_touch(&[pt(0, 40), pt(1200, 80)]).unwrap();
    c.add_note(Note {
        touch,
        ..Note::new(0, 1920, 5, 100)
    })
    .unwrap();
    let mut s = session(c);
    s.commands = vec![(0, Command::SetLoop(Some((0, 960)))), (0, Command::Play)];
    s.frames = frame(960 * 3);
    for size in SIZES {
        let r = run(&s, size);
        let values: Vec<_> = heard(&r, 5)
            .into_iter()
            .filter_map(|(f, e)| matches!(e, Event::Pressure { .. }).then_some((f, e)))
            .collect();
        // Each pass starts the note and its first value; the jump cuts it
        // before the second.
        assert_eq!(values.len(), 3, "size {size}: {values:?}");
        assert!(values.iter().all(|(_, e)| *e == squeeze(5, 40)));
    }
}

#[test]
fn what_was_recorded_plays_back_on_the_next_pass() {
    let mut s = session(Clip::new("take", 1920));
    s.commands = vec![(0, Command::SetLoop(Some((0, 1920)))), (0, Command::Record)];
    s.live = vec![
        (frame(100), press(9)),
        (frame(160), squeeze(9, 30)),
        (frame(400), squeeze(9, 64)),
        (frame(700), squeeze(9, 127)),
        (frame(1000), lift(9)),
    ];
    s.frames = frame(1920 * 2);
    for size in SIZES {
        let r = run(&s, size);
        let clip = r.project.tracks[0].slots[0].as_ref().unwrap();
        assert_eq!(clip.notes.len(), 1);
        assert_eq!(clip.touch_of(&clip.notes[0]).len(), 3);
        // The second pass plays the values back, each on the tick it was
        // played on (60, 300 and 600 ticks after the note's start).
        let pass2: Vec<_> = heard(&r, 9)
            .into_iter()
            .filter(|&(f, e)| f >= frame(1920) && matches!(e, Event::Pressure { .. }))
            .collect();
        assert_eq!(
            pass2,
            vec![
                (frame(1920 + 160), squeeze(9, 30)),
                (frame(1920 + 400), squeeze(9, 64)),
                (frame(1920 + 700), squeeze(9, 127)),
            ],
            "size {size}"
        );
    }
}

#[test]
fn a_flood_of_pressure_never_crowds_out_notes_and_losses_are_counted() {
    let mut s = session(Clip::new("take", 3840));
    s.commands = vec![(0, Command::Record)];
    let mut live = vec![(0, press(1))];
    // 600 values in one period: the queue takes 512.
    for i in 0..600u64 {
        live.push((10, squeeze(1, (i % 2 * 20) as u8)));
    }
    live.push((20, press(2)));
    live.push((30, lift(2)));
    live.push((40, lift(1)));
    s.live = live;
    s.frames = 20_000;
    let r = run(&s, 4096);
    let notes: Vec<u8> = r.taken.iter().map(|(k, _)| k.note.note).collect();
    assert_eq!(notes, vec![1, 2], "both notes recorded, by where they play");
    assert_eq!(r.touch_dropped, 88);
}

#[test]
fn a_note_keeps_at_most_its_buffers_worth_and_counts_the_rest() {
    let mut s = session(Clip::new("take", 3840 * 4));
    s.commands = vec![(0, Command::Record)];
    let n = mpfree_play::TOUCH_POINTS as u64 + 52;
    let mut live = vec![(0, press(1))];
    // Every value 10 steps from the last: each is kept.
    live.extend((0..n).map(|i| (10 + 2 * i, squeeze(1, (i % 2 * 10) as u8 + 10))));
    live.push((10 + 2 * n, lift(1)));
    s.live = live;
    s.frames = 20_000;
    let r = run(&s, 128);
    assert_eq!(r.taken[0].1.len(), mpfree_play::TOUCH_POINTS);
    assert_eq!(r.touch_dropped, 52);
}
