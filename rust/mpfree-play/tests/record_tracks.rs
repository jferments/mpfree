//! Several armed tracks record in one take (the MPC's SHIFT + a track's
//! arm, MPC 3.10.1 guide p. 62, Rec Arm Behavior): the armed track and the
//! others (`PlayView::set_also`) each record their own notes into their own
//! playing clip with their own recorder (`Recorder::also`), in one take
//! that one undo takes back; a replace take erases each armed track's old
//! notes over one pass of its own clip, whatever its length; a track that
//! isn't armed only plays.

use mpfree_play::{Event, Launch, Out, PlayView, Player, Recorder, Sink, Takes};
use mpfree_seq::{Clip, Note, Project, Track};
use mpfree_transport::{Block, Command, Transport};

/// Frames per tick at 120 BPM and 44.1 kHz.
const K: f64 = 22.968_75;
const BAR: u64 = 88_200;

fn at(from: u64, t: f64) -> u64 {
    from + (t * K).round() as u64
}

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

fn note(tick: i64, n: u8) -> Note {
    Note {
        tick,
        len: 100,
        note: n,
        velocity: 100,
        chance: 100,
        cond: mpfree_seq::Cond::Always,
        ratchet: 1,
        touch: mpfree_seq::Touch::NONE,
        on: true,
        ramp: 0,
        join: mpfree_seq::Join::Attack,
        group: 0,
        heard: false,
    }
}

/// Four tracks, each playing clip 0: track `i`'s clip is `lens[i]` ticks
/// long with a note every 480 ticks (note `10 * i` and on).
fn project(lens: [i64; 4]) -> (Project, Vec<Launch>) {
    let mut p = Project::new();
    let mut launches = vec![];
    for (i, &len) in lens.iter().enumerate() {
        let mut t = Track::new(&format!("t{i}"), i as u16);
        let mut c = Clip::new("c", len);
        c.notes = (0..len / 480)
            .map(|k| note(k * 480, (10 * i as i64 + k) as u8))
            .collect();
        t.slots[0] = Some(c);
        p.tracks.push(t);
        launches.push(Launch {
            track: i,
            slot: 0,
            origin: 0,
            start: 0,
        });
    }
    (p, launches)
}

struct Run {
    project: Project,
    takes: Takes,
    /// (track, clip tick, note) of the notes erased.
    erased: Vec<(usize, i64, u8)>,
    /// (track, clip tick, note, take) of the notes recorded.
    taken: Vec<(usize, i64, u8, u32)>,
}

/// Play `project` with track 0 armed and `also` armed too, the transport
/// taking `commands`, and `live` keys (absolute frame, track, event): track
/// 0's to the armed track's recorder, another armed track's to its own,
/// and a track that isn't armed's to nobody (it only plays). After each
/// period the control thread makes the changes through [`Takes`] and
/// compiles again, as the app does.
fn run(
    mut project: Project,
    launches: &[Launch],
    also: u128,
    commands: &[(u64, Command)],
    live: &[(u64, usize, Event)],
    frames: u64,
    period: u32,
) -> Run {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    for &(f, c) in commands {
        assert!(t.command(f, c));
    }
    let compile = |p: &Project| {
        let mut v = PlayView::compile(p, launches).unwrap();
        v.set_armed(Some(0));
        v.set_also(also);
        v
    };
    let mut view = compile(&project);
    let mut player = Player::new();
    let mut rec = Recorder::new();
    let mut others: Vec<Recorder> = (0..3).map(Recorder::also).collect();
    let mut block = Block::new();
    let mut log = Log::default();
    let mut takes = Takes::default();
    let mut erased = vec![];
    let mut taken = vec![];
    let mut live = live.iter().peekable();
    while t.frame() < frames {
        let n = period.min((frames - t.frame()) as u32);
        let start = t.frame();
        t.process(n, &mut block);
        player.process(&view, &block, &mut log);
        while let Some(&&(f, track, e)) = live.peek() {
            if f >= start + u64::from(n) {
                break;
            }
            let offset = (f - start) as u32;
            if view.armed() == Some(track) {
                rec.input(offset, e);
            } else if let Some(k) = view.also_rank(track) {
                others[k].input(offset, e);
            }
            live.next();
        }
        let take = rec.take();
        rec.process(&view, &block);
        for r in &mut others {
            r.set_take(take);
            r.process(&view, &block);
        }
        let mut changed = false;
        for e in player.erased() {
            assert!(
                takes.erase(&mut project, e),
                "an erased note is in its clip"
            );
            erased.push((e.track, e.note.tick, e.note.note));
            changed = true;
        }
        for k in rec
            .taken()
            .iter()
            .chain(others.iter().flat_map(|r| r.taken()))
        {
            takes.add(&mut project, k).unwrap();
            taken.push((k.track, k.note.tick, k.note.note, k.take));
            changed = true;
        }
        if changed {
            view = compile(&project);
        }
    }
    assert_eq!(rec.dropped(), 0);
    assert!(others.iter().all(|r| r.dropped() == 0));
    assert_eq!(player.stats().dropped, 0);
    taken.sort();
    erased.sort();
    Run {
        project,
        takes,
        erased,
        taken,
    }
}

fn on(n: u8) -> Event {
    Event::NoteOn {
        note: n,
        velocity: 100,
    }
}

fn off(n: u8) -> Event {
    Event::NoteOff { note: n }
}

fn clip(r: &Run, track: usize) -> Vec<(i64, u8)> {
    let mut n: Vec<_> = r.project.tracks[track].slots[0]
        .as_ref()
        .unwrap()
        .notes
        .iter()
        .map(|n| (n.tick, n.note))
        .collect();
    n.sort();
    n
}

/// Keys on tracks 0, 1, 2 and 3: note 100 + track at `tick` + 100 per
/// track, let go 50 ticks later, in bar 1.
fn keys() -> Vec<(u64, usize, Event)> {
    let mut live = vec![];
    for t in 0..4usize {
        let tick = 1200.0 + 100.0 * t as f64;
        live.push((at(BAR, tick), t, on(100 + t as u8)));
        live.push((at(BAR, tick + 50.0), t, off(100 + t as u8)));
    }
    live.sort_by_key(|k| k.0);
    live
}

#[test]
fn three_armed_tracks_record_in_one_pass_and_one_undo_takes_it_back() {
    let (p, launches) = project([3840; 4]);
    let before = p.clone();
    // Overdub: nothing is erased. Tracks 0, 1 and 2 armed; track 3 not.
    let commands = [(0, Command::ToggleOverdub), (0, Command::Record)];
    for period in [64, 333, 4096] {
        let mut r = run(
            p.clone(),
            &launches,
            1 << 1 | 1 << 2,
            &commands,
            &keys(),
            2 * BAR,
            period,
        );
        assert!(r.erased.is_empty());
        assert_eq!(
            r.taken,
            vec![(0, 1200, 100, 1), (1, 1300, 101, 1), (2, 1400, 102, 1)],
            "each note in its own track's clip, at its tick, in take 1 (period {period})"
        );
        assert!(clip(&r, 1).contains(&(1300, 101)));
        assert!(clip(&r, 2).contains(&(1400, 102)));
        assert!(
            !clip(&r, 3).iter().any(|n| n.1 == 103),
            "a track that isn't armed only plays"
        );
        let shape: Vec<(u32, usize)> = r
            .takes
            .done()
            .iter()
            .map(|t| (t.number, t.added.len()))
            .collect();
        assert_eq!(shape, vec![(1, 3)], "one take across the three tracks");
        assert_eq!(r.takes.undo(&mut r.project), Some(1));
        for t in 0..4 {
            assert_eq!(
                r.project.tracks[t].slots[0].as_ref().unwrap().notes,
                before.tracks[t].slots[0].as_ref().unwrap().notes,
                "one undo takes the take back on every track"
            );
        }
        assert_eq!(r.takes.undo(&mut r.project), None);
    }
}

#[test]
fn a_replace_take_erases_each_armed_tracks_old_notes_over_its_own_pass() {
    // Clips of one bar, two bars and half a bar on tracks 0 to 2, and one
    // bar on track 3 (not armed). Punch in at tick 1000 with overdub off.
    let (p, launches) = project([3840, 7680, 1920, 3840]);
    let commands = [(0, Command::Play), (at(0, 1000.0), Command::Record)];
    let r = run(p, &launches, 1 << 1 | 1 << 2, &commands, &[], 3 * BAR, 256);
    let mut want = vec![];
    // Track 0: the rest of its one bar.
    want.extend((3..8).map(|k| (0, k * 480, k as u8)));
    // Track 1: the rest of its two bars.
    want.extend((3..16).map(|k| (1, k * 480, 10 + k as u8)));
    // Track 2: the rest of its half bar.
    want.extend((3..4).map(|k| (2, k * 480, 20 + k as u8)));
    want.sort();
    assert_eq!(r.erased, want, "each track's pass, from the punch in");
    // From each clip's next pass the take goes on as overdub: what was
    // before the punch in is kept.
    assert_eq!(clip(&r, 0), vec![(0, 0), (480, 1), (960, 2)]);
    assert_eq!(clip(&r, 1), vec![(0, 10), (480, 11), (960, 12)]);
    assert_eq!(clip(&r, 2), vec![(0, 20), (480, 21), (960, 22)]);
    assert_eq!(
        clip(&r, 3).len(),
        8,
        "a track that isn't armed keeps its notes"
    );
}

#[test]
fn a_loop_jump_ends_every_armed_tracks_replace_pass() {
    // A loop of one bar: track 1's pass would run two bars, but the jump
    // ends every pass at once, as it does the armed track's alone.
    let (p, launches) = project([3840, 7680, 1920, 3840]);
    let commands = [
        (0, Command::SetLoop(Some((0, 3840)))),
        (0, Command::Play),
        (at(0, 2000.0), Command::Record),
    ];
    let r = run(p, &launches, 1 << 1, &commands, &[], 2 * BAR, 500);
    let erased: Vec<(usize, i64)> = r.erased.iter().map(|e| (e.0, e.1)).collect();
    // Tracks 0 and 1 from 2000 to the loop's end (the jump ends the pass;
    // track 1's clip never reaches its second bar); track 2 isn't armed.
    let mut want: Vec<(usize, i64)> = (5..8).map(|k| (0, k * 480)).collect();
    want.extend((5..8).map(|k| (1, k * 480)));
    want.sort();
    assert_eq!(erased, want);
}

#[test]
fn an_also_recorder_with_no_track_records_nothing() {
    let (p, launches) = project([3840; 4]);
    let mut v = PlayView::compile(&p, &launches).unwrap();
    v.set_armed(Some(1));
    // Track 1 is the armed one: it is left out of the set, and so is a
    // track past the view's.
    v.set_also(1 << 1 | 1 << 3 | 1 << 100);
    assert_eq!(v.also(), 1 << 3);
    assert_eq!(v.armed_set(), 1 << 1 | 1 << 3);
    assert_eq!(v.also_track(0), Some(3));
    assert_eq!(v.also_track(1), None);
    assert_eq!(v.also_rank(3), Some(0));
    assert_eq!(v.also_rank(2), None);
    let mut r = Recorder::also(1);
    assert_eq!(r.track(&v), None);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    assert!(t.command(0, Command::Record));
    let mut block = Block::new();
    r.input(0, on(5));
    r.input(10, Event::Pressure { note: 5, value: 9 });
    t.process(4096, &mut block);
    r.process(&v, &block);
    r.input(0, off(5));
    t.process(4096, &mut block);
    r.process(&v, &block);
    assert!(r.taken().is_empty());
    assert_eq!(
        (r.dropped(), r.touch_dropped()),
        (0, 0),
        "no note to keep the pressure on, and nothing lost"
    );
}

/// Step sequencer refactor E3: another armed track's recorder keeps the
/// pressure on its notes as the armed track's does (a second keyboard's
/// aftertouch), handed over with the note; a lock fed to it is still
/// counted as dropped.
#[test]
fn an_also_recorder_keeps_the_pressure_on_its_notes() {
    let (p, launches) = project([3840; 4]);
    let mut v = PlayView::compile(&p, &launches).unwrap();
    v.set_armed(Some(1));
    v.set_also(1 << 3);
    let mut r = Recorder::also(0);
    assert_eq!(r.track(&v), Some(3));
    let mut t = Transport::new(44_100, 120.0).unwrap();
    assert!(t.command(0, Command::Record));
    let mut block = Block::new();
    let mut kept = vec![];
    for k in 0..40u32 {
        match k {
            1 => r.input(0, on(5)),
            2..=9 => r.input(
                100,
                Event::Pressure {
                    note: 5,
                    value: (k * 12) as u8,
                },
            ),
            10 => r.input(0, off(5)),
            _ => {}
        }
        t.process(1024, &mut block);
        r.process(&v, &block);
        for n in r.taken() {
            let pts: Vec<u8> = r.touch_points(&n.note).iter().map(|p| p.value).collect();
            kept.push((n.note.note, pts));
        }
    }
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].0, 5);
    assert_eq!(kept[0].1, vec![24, 36, 48, 60, 72, 84, 96, 108]);
    assert_eq!((r.dropped(), r.touch_dropped()), (0, 0));
    r.input(
        0,
        Event::Lock {
            addr: mpfree_param::ParamAddr::macro_value(0),
            value: 0.5,
        },
    );
    assert_eq!(r.dropped(), 1, "no room for locks");
}
