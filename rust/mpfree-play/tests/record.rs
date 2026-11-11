//! The clip recorder: notes played while recording land in the armed
//! track's clip at their exact ticks, with their lengths; punch out, loop
//! jumps and stop end held notes; a recorded note never plays again right
//! after it was heard; replace recording erases the old notes for one
//! pass; a take undoes and redoes as a whole; knob moves become parameter
//! locks on the steps they belong to; and the notes and locks are the
//! same for any period size.

use mpfree_play::{
    Erased, ErasedLock, Event, Launch, Out, PlayView, Player, Recorder, Sink, Taken, TakenLock,
    Takes,
};
use mpfree_seq::mpfree_param::{OnTrack, ParamAddr, RelAddr};
use mpfree_seq::{Clip, Lock, Note, Project, Quantise, Speed, Track};
use mpfree_transport::time::Swing;
use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, Command, Transport};

/// Frames per tick at 120 BPM and 44.1 kHz.
const K: f64 = 22.968_75;

/// The frame of tick `t` from frame `from` at 120 BPM.
fn at(from: u64, t: f64) -> u64 {
    from + (t * K).round() as u64
}

fn on(note: u8) -> Event {
    Event::NoteOn {
        note,
        velocity: 90 + note,
    }
}

fn off(note: u8) -> Event {
    Event::NoteOff { note }
}

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

struct Session {
    project: Project,
    launches: Vec<Launch>,
    armed: Option<usize>,
    commands: Vec<(u64, Command)>,
    /// Live notes at absolute frames, in frame order.
    live: Vec<(u64, Event)>,
    frames: u64,
}

struct Run {
    taken: Vec<Taken>,
    /// Recorded locks as handed over, period by period.
    locks: Vec<TakenLock>,
    /// Knob moves on another track.
    stray: u64,
    erased_locks: Vec<ErasedLock>,
    erased: Vec<Erased>,
    played: Vec<Out>,
    project: Project,
    takes: Takes,
}

/// Play the session in periods from `next`. After each period the control
/// thread removes the erased notes from their clips and adds the recorded
/// notes and locks (through [`Takes`], which remembers them for undo), and
/// compiles a new view.
fn run(s: &Session, mut next: impl FnMut() -> u32) -> Run {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    for &(f, c) in &s.commands {
        assert!(t.command(f, c));
    }
    let mut project = s.project.clone();
    let compile = |p: &Project| {
        let mut v = PlayView::compile(p, &s.launches).unwrap();
        v.set_armed(s.armed);
        v
    };
    let mut view = compile(&project);
    let mut player = Player::new();
    let mut rec = Recorder::new();
    let mut block = Block::new();
    let mut log = Log::default();
    let mut taken = vec![];
    let mut locks = vec![];
    let mut erased = vec![];
    let mut erased_locks = vec![];
    let mut takes = Takes::default();
    let mut live = s.live.iter().peekable();
    while t.frame() < s.frames {
        let n = next().min((s.frames - t.frame()) as u32);
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
        if !rec.taken().is_empty()
            || !player.erased().is_empty()
            || !rec.taken_locks().is_empty()
            || !player.erased_locks().is_empty()
        {
            for e in player.erased() {
                assert!(
                    takes.erase(&mut project, e),
                    "an erased note is in its clip"
                );
                erased.push(*e);
            }
            for e in player.erased_locks() {
                assert!(
                    takes.erase_lock(&mut project, e),
                    "an erased lock is in its clip"
                );
                erased_locks.push(*e);
            }
            for k in rec.taken() {
                takes.add(&mut project, k).unwrap();
                taken.push(*k);
            }
            for k in rec.taken_locks() {
                takes.add_lock(&mut project, k).unwrap();
                locks.push(*k);
            }
            view = compile(&project);
        }
    }
    assert_eq!(rec.dropped(), 0);
    assert_eq!(player.stats().dropped, 0);
    Run {
        taken,
        locks,
        stray: rec.stray_locks(),
        erased_locks,
        erased,
        played: log.0,
        project,
        takes,
    }
}

fn session(clip: Clip, origin: i64) -> Session {
    let mut p = Project::new();
    let mut tr = Track::new("keys", 0);
    tr.slots[3] = Some(clip);
    p.tracks.push(tr);
    Session {
        project: p,
        launches: vec![Launch {
            track: 0,
            slot: 3,
            origin,
            start: origin,
        }],
        armed: Some(0),
        commands: vec![(0, Command::Record)],
        live: vec![],
        frames: 0,
    }
}

fn notes(taken: &[Taken]) -> Vec<(i64, i64, u8)> {
    taken
        .iter()
        .map(|k| (k.note.tick, k.note.len, k.note.note))
        .collect()
}

fn ons(played: &[Out]) -> Vec<(u64, u8)> {
    played
        .iter()
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some((o.frame, note)),
            _ => None,
        })
        .collect()
}

/// A bar with a note on each beat (notes 10 to 13), playing from tick 0.
fn four_beats() -> Session {
    let mut c = Clip::new("beats", 3840);
    c.notes = (0..4)
        .map(|i| Note {
            tick: i * 960,
            len: 100,
            note: 10 + i as u8,
            velocity: 100,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
        })
        .collect();
    let mut s = session(c, 0);
    s.frames = 2 * 88_200;
    s
}

fn clip_notes(r: &Run) -> Vec<(i64, u8)> {
    let mut n: Vec<_> = r.project.tracks[0].slots[3]
        .as_ref()
        .unwrap()
        .notes
        .iter()
        .map(|n| (n.tick, n.note))
        .collect();
    n.sort();
    n
}

#[test]
fn notes_land_on_the_ticks_they_were_played_on() {
    let mut s = session(Clip::new("take", 3840), 0);
    // Frames 5800 and 22_500 fall between ticks: 252.5 and 979.6 round to 253 and 980.
    s.live = vec![
        (0, on(1)),
        (2000, off(1)),
        (5800, on(2)),
        (9000, off(2)),
        (22_500, on(3)),
        (30_000, off(3)),
    ];
    s.frames = 40_000;
    let r = run(&s, || 128);
    assert_eq!(
        notes(&r.taken),
        vec![(0, 87, 1), (253, 139, 2), (980, 326, 3)]
    );
    assert!(r
        .taken
        .iter()
        .all(|k| k.track == 0 && k.slot == 3 && k.note.velocity == 90 + k.note.note));
    assert_eq!(
        r.project.tracks[0].slots[3].as_ref().unwrap().notes.len(),
        3
    );
}

#[test]
fn clip_ticks_follow_the_clip_speed_origin_and_length() {
    // One beat at double speed (480 song ticks a pass), from song tick 1920.
    let mut c = Clip::new("fast", 960);
    c.speed = Speed::new(2, 1).unwrap();
    let mut s = session(c, 1920);
    let t = 1920.0 + 3.0 * 480.0 + 120.0;
    s.live = vec![
        (at(0, 1000.0), on(5)),
        (at(0, 1100.0), off(5)),
        (at(0, t), on(7)),
        (at(0, t + 100.0), off(7)),
    ];
    s.frames = at(0, t + 1000.0);
    let r = run(&s, || 256);
    // Before the origin nothing is recorded; the fourth pass's 120 song ticks are clip tick 240.
    assert_eq!(notes(&r.taken), vec![(240, 200, 7)]);
}

#[test]
fn punch_out_loop_jumps_and_stop_end_held_notes() {
    let mut s = session(Clip::new("take", 3840), 0);
    let bar = 88_200;
    s.commands = vec![
        (0, Command::SetLoop(Some((0, 3840)))),
        (0, Command::Record),
        (at(bar, 500.0), Command::Record),
        (at(bar, 1000.0), Command::Record),
        (at(bar, 1500.0), Command::Stop),
    ];
    s.live = vec![
        // Held over the loop end: cut there.
        (at(0, 3000.0), on(1)),
        (at(bar, 50.0), off(1)),
        // Held over the punch out at 500: cut there.
        (at(bar, 100.0), on(2)),
        (at(bar, 600.0), off(2)),
        // Played between punch out and punch in: not recorded.
        (at(bar, 700.0), on(3)),
        (at(bar, 800.0), off(3)),
        // Held over the stop at 1500: cut there.
        (at(bar, 1200.0), on(4)),
        (at(bar, 1700.0), off(4)),
    ];
    s.frames = at(bar, 2000.0);
    let r = run(&s, || 100);
    assert_eq!(
        notes(&r.taken),
        vec![(3000, 840, 1), (100, 400, 2), (1200, 300, 4)]
    );
}

#[test]
fn a_note_pressed_again_ends_the_held_one_first() {
    let mut s = session(Clip::new("take", 3840), 0);
    s.live = vec![
        (at(0, 100.0), on(9)),
        (at(0, 400.0), on(9)),
        (at(0, 500.0), off(9)),
        (at(0, 600.0), off(9)),
    ];
    s.frames = at(0, 1000.0);
    let r = run(&s, || 64);
    assert_eq!(notes(&r.taken), vec![(100, 300, 9), (400, 100, 9)]);
}

#[test]
fn nothing_is_recorded_unless_armed_and_recording() {
    let live = vec![(at(0, 100.0), on(1)), (at(0, 200.0), off(1))];
    let mut s = session(Clip::new("take", 3840), 0);
    s.live = live.clone();
    s.frames = at(0, 400.0);

    s.commands = vec![(0, Command::Play)];
    assert!(run(&s, || 128).taken.is_empty(), "playing, not recording");

    s.commands = vec![(0, Command::Record)];
    s.armed = None;
    assert!(run(&s, || 128).taken.is_empty(), "not armed");

    s.armed = Some(0);
    s.launches.clear();
    assert!(
        run(&s, || 128).taken.is_empty(),
        "the armed track has no clip playing"
    );

    // During a one-bar count-in nothing is recorded; after it, notes are.
    let mut s = session(Clip::new("take", 3840), 0);
    s.commands = vec![(0, Command::SetCountIn(1)), (0, Command::Record)];
    s.live = vec![
        (at(0, 100.0), on(1)),
        (at(0, 200.0), off(1)),
        (at(0, 3940.0), on(2)),
        (at(0, 4040.0), off(2)),
    ];
    s.frames = at(0, 5000.0);
    assert_eq!(notes(&run(&s, || 128).taken), vec![(100, 100, 2)]);
}

#[test]
fn a_recorded_note_does_not_play_again_right_after_it_was_heard() {
    // Quantise to 1/16 with 66 % swing: a note played at 230 plays at 317,
    // after the moment it was played; one played at 970 plays at 960,
    // before it.
    let mut c = Clip::new("take", 3840);
    c.quantise = Quantise::new(240, 100);
    let mut s = session(c, 0);
    s.commands = vec![
        (0, Command::SetSwing(Swing::new(240, 66).unwrap())),
        (0, Command::SetLoop(Some((0, 3840)))),
        (0, Command::Record),
    ];
    s.live = vec![
        (at(0, 230.0), on(1)),
        (at(0, 260.0), off(1)),
        (at(0, 970.0), on(2)),
        (at(0, 1000.0), off(2)),
    ];
    s.frames = 2 * 88_200;
    let r = run(&s, || 128);
    // The clip keeps the played ticks; quantise is applied on playback.
    assert_eq!(notes(&r.taken), vec![(230, 30, 1), (970, 30, 2)]);
    // Nothing plays in the first pass; in the second, both play on the grid.
    assert_eq!(
        ons(&r.played),
        vec![(at(88_200, 317.0), 1), (at(88_200, 960.0), 2)]
    );
}

#[test]
fn replace_recording_erases_the_old_notes_for_one_pass() {
    // Play, punch in at 1000 with overdub off, play note 5 at 2000.
    let mut s = four_beats();
    s.commands = vec![(0, Command::Play), (at(0, 1000.0), Command::Record)];
    s.live = vec![(at(0, 2000.0), on(5)), (at(0, 2100.0), off(5))];
    let r = run(&s, || 128);
    // The beats at 1920 and 2880 are erased as the playhead reaches them;
    // from the clip's next pass the take goes on as overdub.
    let erased: Vec<(i64, u8)> = r
        .erased
        .iter()
        .map(|e| (e.note.tick, e.note.note))
        .collect();
    assert_eq!(erased, vec![(1920, 12), (2880, 13)]);
    assert_eq!(clip_notes(&r), vec![(0, 10), (960, 11), (2000, 5)]);
    let bar = 88_200;
    assert_eq!(
        ons(&r.played),
        vec![
            (0, 10),
            (at(0, 960.0), 11),
            (bar, 10),
            (at(bar, 960.0), 11),
            (at(bar, 2000.0), 5)
        ],
        "erased notes are not played"
    );
}

#[test]
fn overdub_keeps_the_old_notes_and_ends_a_replace_pass() {
    let mut s = four_beats();
    s.commands = vec![
        (0, Command::ToggleOverdub),
        (0, Command::Play),
        (at(0, 1000.0), Command::Record),
    ];
    s.live = vec![(at(0, 2000.0), on(5)), (at(0, 2100.0), off(5))];
    let r = run(&s, || 128);
    assert!(r.erased.is_empty());
    assert_eq!(
        clip_notes(&r),
        vec![(0, 10), (960, 11), (1920, 12), (2000, 5), (2880, 13)]
    );

    // Overdub turned on at 2400 ends the replace pass: only 1920 goes.
    let mut s = four_beats();
    s.commands = vec![
        (0, Command::Play),
        (at(0, 1000.0), Command::Record),
        (at(0, 2400.0), Command::ToggleOverdub),
    ];
    let r = run(&s, || 128);
    assert_eq!(clip_notes(&r), vec![(0, 10), (960, 11), (2880, 13)]);
}

#[test]
fn a_loop_jump_ends_the_replace_pass_and_a_new_take_replaces_again() {
    // A two-bar clip in a one-bar loop: the replace pass ends at the jump,
    // so the second bar is never touched. After the jump take 1 goes on as
    // overdub, and keeps note 9 played at 1200. Punching out and in starts
    // take 2, which replaces what the playhead reaches after the punch in.
    let mut c = Clip::new("two bars", 7680);
    c.notes = (0..8)
        .map(|i| Note {
            tick: i * 960,
            len: 100,
            note: 20 + i as u8,
            velocity: 100,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
        })
        .collect();
    let mut s = session(c, 0);
    let bar = 88_200;
    s.commands = vec![
        (0, Command::SetLoop(Some((0, 3840)))),
        (0, Command::Record),
        (at(bar, 1500.0), Command::Record),
        (at(2 * bar, 500.0), Command::Record),
    ];
    s.live = vec![(at(bar, 1200.0), on(9)), (at(bar, 1300.0), off(9))];
    s.frames = 3 * bar;
    let r = run(&s, || 333);
    // Take 1 erases bar 1 (all four beats); take 2 erases note 9.
    let erased: Vec<(i64, u8)> = r
        .erased
        .iter()
        .map(|e| (e.note.tick, e.note.note))
        .collect();
    assert_eq!(
        erased,
        vec![(0, 20), (960, 21), (1920, 22), (2880, 23), (1200, 9)]
    );
    assert_eq!(
        clip_notes(&r),
        vec![(3840, 24), (4800, 25), (5760, 26), (6720, 27)]
    );
}

#[test]
fn a_muted_armed_track_is_still_replaced() {
    let mut s = four_beats();
    s.project.tracks[0].mute = true;
    s.commands = vec![(0, Command::Record)];
    let r = run(&s, || 128);
    assert_eq!(r.erased.len(), 4);
    assert!(clip_notes(&r).is_empty() && r.played.is_empty());
}

#[test]
fn a_take_undoes_and_redoes_as_a_whole() {
    // The loop-jump session: take 1 erases bar 1 and records note 9, take 2
    // erases note 9.
    let mut c = Clip::new("two bars", 7680);
    c.notes = (0..8)
        .map(|i| Note {
            tick: i * 960,
            len: 100,
            note: 20 + i as u8,
            velocity: 100,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
        })
        .collect();
    let original = c.notes.clone();
    let mut s = session(c, 0);
    let bar = 88_200;
    s.commands = vec![
        (0, Command::SetLoop(Some((0, 3840)))),
        (0, Command::Record),
        (at(bar, 1500.0), Command::Record),
        (at(2 * bar, 500.0), Command::Record),
    ];
    s.live = vec![(at(bar, 1200.0), on(9)), (at(bar, 1300.0), off(9))];
    s.frames = 3 * bar;
    let mut r = run(&s, || 333);
    let shape: Vec<(u32, usize, usize)> = r
        .takes
        .done()
        .iter()
        .map(|t| (t.number, t.added.len(), t.erased.len()))
        .collect();
    assert_eq!(shape, vec![(1, 1, 4), (2, 0, 1)]);
    let last = vec![(3840, 24), (4800, 25), (5760, 26), (6720, 27)];
    assert_eq!(clip_notes(&r), last);

    let with_9 = [vec![(1200, 9)], last.clone()].concat();
    assert_eq!(r.takes.undo(&mut r.project), Some(2));
    assert_eq!(clip_notes(&r), with_9, "note 9 is back");
    assert_eq!(r.takes.undo(&mut r.project), Some(1));
    let mut notes = r.project.tracks[0].slots[3].as_ref().unwrap().notes.clone();
    notes.sort_by_key(|n| n.tick);
    assert_eq!(
        notes, original,
        "the clip is as it was before recording, lengths and velocities too"
    );
    assert_eq!(r.takes.undo(&mut r.project), None);
    assert_eq!(r.takes.redo(&mut r.project), Some(1));
    assert_eq!(clip_notes(&r), with_9);
    assert_eq!(r.takes.redo(&mut r.project), Some(2));
    assert_eq!(clip_notes(&r), last);
    assert_eq!(r.takes.redo(&mut r.project), None);
}

#[test]
fn a_late_note_joins_its_own_take() {
    // Note 1 is held at the punch out at 1905 and quantise moves it to
    // 1920, so it waits for the playhead and arrives after take 2 has
    // begun at 1910 and erased the old note at 1920. It still belongs to
    // take 1, and undoing take 2 leaves it.
    let mut c = Clip::new("bar", 3840);
    c.quantise = Quantise::new(960, 100);
    c.notes = vec![
        Note {
            tick: 0,
            len: 100,
            note: 30,
            velocity: 100,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
        },
        Note {
            tick: 1920,
            len: 100,
            note: 31,
            velocity: 100,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
        },
    ];
    let mut s = session(c, 0);
    s.commands = vec![
        (0, Command::Record),
        (at(0, 1905.0), Command::Record),
        (at(0, 1910.0), Command::Record),
    ];
    s.live = vec![
        (at(0, 1900.0), on(1)),
        (at(0, 2000.0), off(1)),
        (at(0, 2500.0), on(2)),
        (at(0, 2600.0), off(2)),
    ];
    s.frames = at(0, 3500.0);
    let reference = run(&s, || 128);
    // Each take: its number, the notes it added, the notes it erased.
    type Shape = Vec<(u32, Vec<(i64, u8)>, Vec<(i64, u8)>)>;
    let takes = |r: &Run| -> Shape {
        let keys =
            |v: &[(usize, usize, Note)]| v.iter().map(|(_, _, n)| (n.tick, n.note)).collect();
        r.takes
            .done()
            .iter()
            .map(|t| (t.number, keys(&t.added), keys(&t.erased)))
            .collect()
    };
    assert_eq!(
        takes(&reference),
        vec![
            (1, vec![(1900, 1)], vec![(0, 30)]),
            (2, vec![(2500, 2)], vec![(1920, 31)])
        ]
    );
    // The erase at 1920 comes before note 1 is handed over at 1922.
    let order: Vec<u32> = reference
        .erased
        .iter()
        .map(|e| e.take)
        .chain(reference.taken.iter().map(|k| k.take))
        .collect();
    assert_eq!(order, vec![1, 2, 1, 2]);
    for n in [1u32, 7, 4096] {
        assert_eq!(takes(&run(&s, || n)), takes(&reference), "period size {n}");
    }
    let mut r = reference;
    assert_eq!(r.takes.undo(&mut r.project), Some(2));
    assert_eq!(clip_notes(&r), vec![(1900, 1), (1920, 31)]);
    assert_eq!(r.takes.undo(&mut r.project), Some(1));
    assert_eq!(clip_notes(&r), vec![(0, 30), (1920, 31)]);
}

fn xorshift(seed: u64) -> impl FnMut() -> u64 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

/// A busy take: swing, a loop, punch in and out (each punch in a new take
/// that replaces), a tempo change, a locate, a stop and a restart, overdub
/// at the end, with 300 notes of random timing and length, some
/// overlapping, some pressed again while held, on a quantised clip at 3/4
/// speed.
fn busy() -> Session {
    let mut c = Clip::new("take", 2 * 3840);
    c.quantise = Quantise::new(240, 60);
    c.speed = Speed::new(3, 4).unwrap();
    let mut s = session(c, 0);
    s.commands = vec![
        (0, Command::SetSwing(Swing::new(240, 58).unwrap())),
        (0, Command::SetLoop(Some((0, 7680)))),
        (500, Command::Record),
        (90_000, Command::Record),
        (120_000, Command::Record),
        (150_000, Command::SetTempo(97.0)),
        (200_000, Command::Locate(2_000)),
        (260_000, Command::Stop),
        (270_000, Command::Record),
        (330_000, Command::ToggleOverdub),
    ];
    let mut r = xorshift(7);
    let mut live = vec![];
    for _ in 0..300 {
        let f = r() % 390_000;
        let note = (r() % 12) as u8;
        live.push((
            f,
            Event::NoteOn {
                note,
                velocity: 1 + (r() % 127) as u8,
            },
        ));
        live.push((f + 1 + r() % 20_000, Event::NoteOff { note }));
    }
    // Knob moves, from their own stream so the notes stay as they were:
    // the instrument's parameters and some pads' own.
    let mut r = xorshift(11);
    for _ in 0..400 {
        let on = match r() % 3 {
            0 => OnTrack::Inst,
            1 => OnTrack::Strip,
            _ => OnTrack::Pad((r() % 12) as u8),
        };
        let value = (r() % 1000) as f32 / 1000.0;
        live.push((r() % 390_000, knob(on, (r() % 3) as u16, value)));
    }
    live.sort_by_key(|e| e.0);
    s.live = live;
    s.frames = 400_000;
    s
}

#[test]
fn the_take_is_the_same_for_every_period_size() {
    let s = busy();
    let reference = run(&s, || 128);
    assert!(reference.taken.len() > 100, "{}", reference.taken.len());
    assert!(
        reference.erased.len() > 20,
        "the later takes replace notes: {}",
        reference.erased.len()
    );
    for n in [1u32, 7, 64, 333, 1024, 4096, MAX_PERIOD] {
        let r = run(&s, || n);
        assert!(r.taken == reference.taken, "period size {n} differs");
        assert!(
            r.erased == reference.erased,
            "period size {n}: erased notes differ"
        );
        assert!(
            r.project == reference.project,
            "period size {n}: the clip differs (notes or locks)"
        );
    }
    for seed in [3u64, 0xbeef] {
        let mut x = xorshift(seed);
        let r = run(&s, || (x() % 3000) as u32 + 1);
        assert!(
            r.taken == reference.taken && r.erased == reference.erased,
            "random sizes (seed {seed}) differ"
        );
    }
    for seed in [5u64, 0xf00d] {
        let mut x = xorshift(seed);
        let r = run(&s, || (x() % 3000) as u32 + 1);
        assert!(
            r.project == reference.project,
            "random sizes (seed {seed}): the clip differs"
        );
    }
    let clip = reference.project.tracks[0].slots[3].as_ref().unwrap();
    assert_eq!(clip.check(), Ok(()));
    assert!(clip.locks.len() > 50, "{}", clip.locks.len());
    assert!(
        clip.locks.iter().any(|l| l.sound.is_some()),
        "pads' own locks"
    );
    assert!(clip.notes.iter().all(|n: &Note| n.len <= clip.length));
}

// Knob moves while recording: parameter locks.

/// A knob move on track 0's parameter `param` of `on`.
fn knob(on: OnTrack, param: u16, value: f32) -> Event {
    Event::Lock {
        addr: RelAddr::new(on, param).on(0),
        value,
    }
}

/// The clip's locks as (tick, row, parameter, value), in order.
fn clip_locks(r: &Run) -> Vec<(i64, Option<u8>, u16, f32)> {
    let mut l: Vec<_> = r.project.tracks[0].slots[3]
        .as_ref()
        .unwrap()
        .locks
        .iter()
        .map(|l| (l.tick, l.sound, l.addr.param, l.value))
        .collect();
    l.sort_by_key(|x| (x.0, x.1, x.2));
    l
}

/// [`four_beats`] recording with overdub on, so the beats stay.
fn four_beats_overdub() -> Session {
    let mut s = four_beats();
    s.commands = vec![(0, Command::ToggleOverdub), (0, Command::Record)];
    s
}

#[test]
fn a_knob_turned_while_a_pad_is_held_locks_its_note() {
    let mut s = session(Clip::new("take", 3840), 0);
    s.live = vec![
        // Note 2 from tick 253; the knob turns while it is held, over two
        // periods.
        (5800, on(2)),
        (7000, knob(OnTrack::Inst, 3, 0.25)),
        (8000, knob(OnTrack::Inst, 3, 0.5)),
        (9000, off(2)),
        // Tick 1306, nothing played near: trigless, on the line at 1200.
        (30_000, knob(OnTrack::Inst, 3, 0.75)),
    ];
    s.frames = 40_000;
    let mut r = run(&s, || 128);
    assert_eq!(r.locks.len(), 3, "one per period the knob moved in");
    let after = vec![(253, None, 3, 0.5), (1200, None, 3, 0.75)];
    assert_eq!(clip_locks(&r), after);
    let take = &r.takes.done()[0];
    assert_eq!(r.takes.done().len(), 1, "the notes and locks are one take");
    assert_eq!(
        (
            take.added.len(),
            take.locks_added.len(),
            take.locks_erased.len()
        ),
        (1, 2, 0),
        "the turn is one lock"
    );
    assert_eq!(r.takes.undo(&mut r.project), Some(1));
    assert_eq!(clip_locks(&r), vec![]);
    assert_eq!(clip_notes(&r), vec![]);
    assert_eq!(r.takes.redo(&mut r.project), Some(1));
    assert_eq!(clip_locks(&r), after);
}

#[test]
fn a_turn_just_after_a_note_locks_it_else_the_nearest_line() {
    let mut s = four_beats_overdub();
    s.live = vec![
        // 100 after the beat at 0: on it.
        (at(0, 100.0), knob(OnTrack::Inst, 3, 0.1)),
        // 600: too long after; trigless on the line at 720.
        (at(0, 600.0), knob(OnTrack::Inst, 3, 0.2)),
        // 1000: the beat at 960.
        (at(0, 1000.0), knob(OnTrack::Inst, 3, 0.3)),
        // A pad's parameter looks only at that pad's notes: pad 12 plays
        // at 1920, so its lock at 1100 is trigless on 1200, on pad 12's
        // row; pad 11's lock at 1150 is on its note at 960.
        (at(0, 1100.0), knob(OnTrack::Pad(12), 3, 0.4)),
        (at(0, 1150.0), knob(OnTrack::Pad(11), 3, 0.5)),
    ];
    let r = run(&s, || 128);
    assert_eq!(
        clip_locks(&r),
        vec![
            (0, None, 3, 0.1),
            (720, None, 3, 0.2),
            (960, None, 3, 0.3),
            (960, Some(11), 3, 0.5),
            (1200, Some(12), 3, 0.4),
        ]
    );
    assert_eq!(r.stray, 0);
}

#[test]
fn a_knob_on_another_track_or_not_recording_is_not_recorded() {
    let mut s = four_beats();
    s.commands = vec![
        (0, Command::ToggleOverdub),
        (0, Command::Play),
        (at(0, 2000.0), Command::Record),
    ];
    s.live = vec![
        // Playing, not recording: nothing.
        (at(0, 100.0), knob(OnTrack::Inst, 3, 0.1)),
        // Recording, but another track's parameter or a project one.
        (
            at(0, 2100.0),
            Event::Lock {
                addr: RelAddr::new(OnTrack::Inst, 3).on(1),
                value: 0.2,
            },
        ),
        (
            at(0, 2200.0),
            Event::Lock {
                addr: ParamAddr::macro_value(0),
                value: 0.3,
            },
        ),
        // A value that is not a number.
        (at(0, 2300.0), knob(OnTrack::Inst, 3, f32::NAN)),
    ];
    let r = run(&s, || 128);
    assert_eq!(clip_locks(&r), vec![]);
    assert_eq!(r.stray, 2);
}

#[test]
fn a_recorded_lock_replaces_the_one_there_and_undo_puts_it_back() {
    let mut s = four_beats_overdub();
    let clip = s.project.tracks[0].slots[3].as_mut().unwrap();
    let old = Lock::new(960, None, RelAddr::new(OnTrack::Inst, 3), 0.9);
    let other = Lock::new(960, None, RelAddr::new(OnTrack::Inst, 4), 0.1);
    clip.add_lock(old).unwrap();
    clip.add_lock(other).unwrap();
    s.live = vec![
        (at(0, 1000.0), knob(OnTrack::Inst, 3, 0.3)),
        (at(0, 1100.0), knob(OnTrack::Inst, 3, 0.35)),
    ];
    let mut r = run(&s, || 128);
    assert_eq!(
        clip_locks(&r),
        vec![(960, None, 3, 0.35), (960, None, 4, 0.1)]
    );
    let take = r.takes.done().last().unwrap();
    assert_eq!(take.locks_erased, vec![(0, 3, old)]);
    assert_eq!(take.locks_added.len(), 1);
    r.takes.undo(&mut r.project).unwrap();
    assert_eq!(
        clip_locks(&r),
        vec![(960, None, 3, 0.9), (960, None, 4, 0.1)]
    );
    r.takes.redo(&mut r.project).unwrap();
    assert_eq!(
        clip_locks(&r),
        vec![(960, None, 3, 0.35), (960, None, 4, 0.1)]
    );
}

#[test]
fn a_replace_pass_does_not_lock_the_notes_it_erased() {
    // Record with overdub off: the first pass erases the beats.
    let mut s = four_beats();
    let bar = 88_200;
    s.live = vec![
        // The beat at 960 is erased: trigless on 1200.
        (at(0, 1100.0), knob(OnTrack::Inst, 3, 0.2)),
        // Note 5, recorded at 2000, and a turn just after it.
        (at(0, 2000.0), on(5)),
        (at(0, 2050.0), off(5)),
        (at(0, 2100.0), knob(OnTrack::Inst, 3, 0.3)),
        // The next pass goes on as overdub: note 5 is in the clip now.
        (at(bar, 2150.0), knob(OnTrack::Inst, 3, 0.4)),
    ];
    for n in [1u32, 128, 4096] {
        let r = run(&s, || n);
        assert_eq!(clip_notes(&r), vec![(2000, 5)], "period size {n}");
        assert_eq!(
            clip_locks(&r),
            vec![(1200, None, 3, 0.2), (2000, None, 3, 0.4)],
            "period size {n}"
        );
        let take = r.takes.done().last().unwrap();
        assert_eq!(
            (take.locks_added.len(), take.locks_erased.len()),
            (2, 0),
            "period size {n}"
        );
    }
}

#[test]
fn a_replace_pass_erases_the_old_locks_and_undo_brings_them_back() {
    // Old locks: on the beat at 960, trigless at 1440, and pad 13's own on
    // its beat at 2880. Recording with overdub off replaces all three.
    let mut s = four_beats();
    let clip = s.project.tracks[0].slots[3].as_mut().unwrap();
    let old = [
        Lock::new(960, None, RelAddr::new(OnTrack::Inst, 3), 0.9),
        Lock::new(1440, None, RelAddr::new(OnTrack::Inst, 3), 0.5),
        Lock::new(2880, Some(13), RelAddr::new(OnTrack::Pad(13), 0), 0.7),
    ];
    for l in old {
        clip.add_lock(l).unwrap();
    }
    let bar = 88_200;
    s.live = vec![
        (at(0, 2000.0), on(5)),
        (at(0, 2050.0), knob(OnTrack::Inst, 3, 0.25)),
        (at(0, 2100.0), off(5)),
    ];
    for n in [1u32, 128, 4096] {
        let mut r = run(&s, || n);
        let gone: Vec<Lock> = r.erased_locks.iter().map(|e| e.lock).collect();
        assert_eq!(gone, old, "period size {n}: in the order reached");
        assert_eq!(
            clip_locks(&r),
            vec![(2000, None, 3, 0.25)],
            "period size {n}"
        );
        // Nothing locks in the pass being replaced; the next pass plays
        // the new lock just before note 5.
        let locks: Vec<(u64, f32)> = r
            .played
            .iter()
            .filter_map(|o| match o.event {
                Event::Lock { value, .. } => Some((o.frame, value)),
                _ => None,
            })
            .collect();
        assert_eq!(locks, vec![(at(bar, 2000.0), 0.25)], "period size {n}");
        let take = r.takes.done().last().unwrap();
        assert_eq!(
            (take.locks_erased.len(), take.locks_added.len()),
            (3, 1),
            "period size {n}"
        );
        r.takes.undo(&mut r.project).unwrap();
        let mut back: Vec<Lock> = r.project.tracks[0].slots[3].as_ref().unwrap().locks.clone();
        back.sort_by_key(|l| l.tick);
        assert_eq!(back, old, "period size {n}: undo puts them back");
    }
}

#[test]
fn a_lock_recorded_before_its_quantised_note_is_not_erased() {
    // Quantise moves note 7, played at 1430, onto 1440, ahead of the
    // playhead; its lock plays there too. Both wait until the playhead has
    // passed 1440, so the replace pass, which erases the old lock on 1440,
    // does not erase them.
    let mut c = Clip::new("bar", 3840);
    c.quantise = Quantise::new(240, 100);
    c.add_lock(Lock::new(1440, None, RelAddr::new(OnTrack::Inst, 3), 0.5))
        .unwrap();
    let mut s = session(c, 0);
    s.live = vec![
        (at(0, 1430.0), on(7)),
        (at(0, 1435.0), knob(OnTrack::Inst, 3, 0.8)),
        (at(0, 1600.0), off(7)),
    ];
    s.frames = 2 * 88_200;
    for n in [1u32, 64, 128] {
        let r = run(&s, || n);
        assert_eq!(clip_notes(&r), vec![(1430, 7)], "period size {n}");
        assert_eq!(
            clip_locks(&r),
            vec![(1430, None, 3, 0.8)],
            "period size {n}"
        );
        assert_eq!(r.erased_locks.len(), 1, "period size {n}: the old one");
    }
}
