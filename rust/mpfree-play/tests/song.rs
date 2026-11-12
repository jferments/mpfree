//! Song mode: the arrangement's placements play on their ticks. A
//! placement longer than its clip loops it, an offset starts it part way
//! in, a note held over a placement's end ends on it, a linked placement
//! plays its slot's clip as it is now, a clip launched on a track takes it
//! over until it stops (then the song plays on there), moves chase into a
//! placement, and a locate or a loop jump lands in the right place. All of
//! it the same at any period size, with the control thread applying
//! started cues at once, late or never.

use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Sink, Started};
use mpfree_seq::mpfree_moves::{Kind, Map, Target};
use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
use mpfree_seq::{Clip, Note, Placement, Project, Track};
use mpfree_transport::{Block, Command, Transport};

const BAR: i64 = 3840;
/// 120 BPM at 48 kHz: 25 frames a tick.
const RATE: u32 = 48_000;
const TICK: u64 = 25;

fn at(tick: i64) -> u64 {
    tick as u64 * TICK
}

#[derive(Default)]
struct Log(Vec<(u64, u16, Event)>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push((o.frame, o.track, o.event));
    }
}

#[derive(Clone)]
struct Session {
    project: Project,
    launches: Vec<Launch>,
    commands: Vec<(u64, Command)>,
    cues: Vec<(u64, Cue)>,
    frames: u64,
}

#[derive(Debug, PartialEq)]
struct Run {
    played: Vec<(u64, u16, Event)>,
    started: Vec<Started>,
}

fn run(s: &Session, period: u32, apply: Option<usize>) -> Run {
    let mut t = Transport::new(RATE, 120.0).unwrap();
    for &(f, c) in &s.commands {
        assert!(t.command(f, c));
    }
    let compile = |l: &[Launch], c: &[Cue]| {
        let mut v = PlayView::compile_cued(&s.project, l, c).unwrap();
        v.set_song(&s.project, true).unwrap();
        v
    };
    let (mut launches, mut cues) = (s.launches.clone(), Vec::new());
    let mut view = compile(&launches, &cues);
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let mut started = vec![];
    let mut waiting: Vec<(usize, Started)> = vec![];
    let mut next_cue = 0;
    while t.frame() < s.frames {
        let f = t.frame();
        let n = period.min((s.frames - f) as u32);
        let mut changed = false;
        for w in &mut waiting {
            if w.0 == 0 {
                w.1.apply(&mut launches, &mut cues);
                changed = true;
            }
            w.0 = w.0.wrapping_sub(1);
        }
        waiting.retain(|w| w.0 != usize::MAX);
        while next_cue < s.cues.len() && s.cues[next_cue].0 <= f {
            let c = s.cues[next_cue].1;
            cues.retain(|x| x.track != c.track);
            cues.push(c);
            next_cue += 1;
            changed = true;
        }
        if changed {
            view = compile(&launches, &cues);
        }
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
        for st in player.started() {
            started.push(*st);
            if let Some(d) = apply {
                waiting.push((d, *st));
            }
        }
    }
    assert_eq!(player.stats().dropped + player.stats().late_offs, 0);
    Run {
        played: log.0,
        started,
    }
}

fn same_everywhere(s: &Session) -> Run {
    same_applied(s, &[Some(0), Some(5), None])
}

/// As [`same_everywhere`], with the control thread applying started cues
/// in each of these ways. (Never applying a cue and then cueing the same
/// track again takes the first switch back, so that session leaves None
/// out.)
fn same_applied(s: &Session, applies: &[Option<usize>]) -> Run {
    let first = run(s, 128, Some(0));
    for period in [1, 7, 64, 1000, 4096] {
        for &apply in applies {
            let r = run(s, period, apply);
            assert_eq!(r, first, "period {period}, applied {apply:?}");
        }
    }
    first
}

/// Track 0, slot 0: A, a bar of quarter notes (note 1, 100 ticks long).
/// Slot 1: B, a bar of notes 10 to 13 on the beats, each a beat long.
fn session(frames: u64) -> Session {
    let mut p = Project::new();
    let mut t = Track::new("drums", 0);
    let mut a = Clip::new("A", BAR);
    a.notes = (0..4).map(|i| Note::new(i * 960, 100, 1, 100)).collect();
    let mut b = Clip::new("B", BAR);
    b.notes = (0..4)
        .map(|i| Note::new(i * 960, 960, 10 + i as u8, 100))
        .collect();
    t.slots[0] = Some(a);
    t.slots[1] = Some(b);
    p.tracks.push(t);
    Session {
        project: p,
        launches: vec![],
        commands: vec![(0, Command::Play)],
        cues: vec![],
        frames,
    }
}

/// (song tick, note) of each note-on.
fn ons(r: &Run) -> Vec<(i64, u8)> {
    r.played
        .iter()
        .filter_map(|&(f, _, e)| match e {
            Event::NoteOn { note, .. } => Some(((f / TICK) as i64, note)),
            _ => None,
        })
        .collect()
}

fn offs(r: &Run) -> Vec<(i64, u8)> {
    r.played
        .iter()
        .filter_map(|&(f, _, e)| match e {
            Event::NoteOff { note } => Some(((f / TICK) as i64, note)),
            _ => None,
        })
        .collect()
}

#[test]
fn placements_play_on_their_ticks_looping_and_from_their_offset() {
    let mut s = session(at(5 * BAR));
    let song = &mut s.project.arrangement;
    // A for two bars from bar 1 (it loops); B from bar 3 for one bar,
    // from its second beat.
    song.place(Placement::linked(0, 0, BAR, 2 * BAR)).unwrap();
    let mut b = Placement::linked(0, 1, 3 * BAR, BAR);
    b.offset = 960;
    song.place(b).unwrap();
    let r = same_everywhere(&s);
    let mut want: Vec<(i64, u8)> = (0..8).map(|i| (BAR + i * 960, 1)).collect();
    want.extend([
        (3 * BAR, 11),
        (3 * BAR + 960, 12),
        (3 * BAR + 1920, 13),
        (3 * BAR + 2880, 10),
    ]);
    assert_eq!(ons(&r), want);
    // B's last note (10, from its top again) ends on the placement's end.
    assert_eq!(offs(&r).last(), Some(&(4 * BAR, 10)));
    assert!(r.started.is_empty());
}

#[test]
fn a_note_held_over_a_placements_end_ends_on_it() {
    let mut s = session(at(3 * BAR));
    s.project
        .arrangement
        .place(Placement::linked(0, 1, 0, BAR + 480))
        .unwrap();
    let r = same_everywhere(&s);
    // The second pass's first note (10, a beat long) is cut at 1.5 beats.
    assert_eq!(ons(&r).last(), Some(&(BAR, 10)));
    assert_eq!(offs(&r).last(), Some(&(BAR + 480, 10)));
}

#[test]
fn a_launched_clip_takes_the_track_over_and_the_song_plays_on_after_its_stop() {
    let mut s = session(at(6 * BAR));
    s.project
        .arrangement
        .place(Placement::linked(0, 0, 0, 6 * BAR))
        .unwrap();
    s.cues = vec![
        (
            at(BAR + 100),
            Cue {
                track: 0,
                slot: Some(1),
            },
        ),
        (
            at(3 * BAR + 100),
            Cue {
                track: 0,
                slot: None,
            },
        ),
    ];
    let r = same_applied(&s, &[Some(0), Some(5)]);
    let bars: Vec<Vec<u8>> = (0..6)
        .map(|b| {
            ons(&r)
                .iter()
                .filter(|o| o.0 / BAR == b)
                .map(|o| o.1)
                .collect()
        })
        .collect();
    assert_eq!(
        bars,
        vec![
            vec![1, 1, 1, 1],
            vec![1, 1, 1, 1],
            vec![10, 11, 12, 13],
            vec![10, 11, 12, 13],
            vec![1, 1, 1, 1],
            vec![1, 1, 1, 1],
        ],
        "B from bar 2 to its stop on bar 4, then the song again"
    );
}

#[test]
fn a_locate_and_a_loop_jump_land_in_the_right_placement() {
    let mut s = session(at(6 * BAR));
    let song = &mut s.project.arrangement;
    song.place(Placement::linked(0, 1, 0, 2 * BAR)).unwrap();
    song.place(Placement::linked(0, 0, 2 * BAR, 4 * BAR))
        .unwrap();
    // A loop over bars 2 and 3 (B then A), then a locate into A.
    s.commands.push((0, Command::SetLoop(Some((BAR, 3 * BAR)))));
    s.commands.push((at(BAR / 2), Command::Locate(BAR + 480)));
    s.frames = at(4 * BAR);
    let r = same_everywhere(&s);
    let frames: Vec<(u64, u8)> = r
        .played
        .iter()
        .filter_map(|&(f, _, e)| match e {
            Event::NoteOn { note, .. } => Some((f, note)),
            _ => None,
        })
        .collect();
    // Frame 48000 plays song tick 4320 (the locate); frame 228000 plays
    // 3840 again (the loop jump back from 11520).
    let after_locate = |tick: i64| 48_000 + (tick - (BAR + 480)) as u64 * TICK;
    let after_loop = |tick: i64| 228_000 + (tick - BAR) as u64 * TICK;
    let mut want = vec![(0, 10), (at(960), 11)];
    want.extend([(4800, 11), (5760, 12), (6720, 13)].map(|(t, n)| (after_locate(t), n)));
    want.extend((0..4).map(|i| (after_locate(2 * BAR + i * 960), 1)));
    want.extend((0..4).map(|i| (after_loop(BAR + i * 960), 10 + i as u8)));
    want.extend((0..3).map(|i| (after_loop(2 * BAR + i * 960), 1)));
    assert_eq!(frames, want);
}

#[test]
fn moves_chase_into_a_placement_and_follow_its_slot() {
    let mut s = session(at(4 * BAR));
    let target = Target::Track(RelAddr::new(OnTrack::Inst, 3));
    let pts = [
        (0.0, 0.0, Kind::Ramp),
        (f64::from(BAR as u32 - 1), 1.0, Kind::Ramp),
    ];
    let a = s.project.tracks[0].slots[0].as_mut().unwrap();
    a.moves
        .put(target, Map::linear(0.0, 1.0), &pts, BAR as u32)
        .unwrap();
    // From the clip's middle: the first value is the lane's there.
    let mut pl = Placement::linked(0, 0, BAR, BAR);
    pl.offset = 1920;
    s.project.arrangement.place(pl).unwrap();
    let r = same_everywhere(&s);
    let lanes: Vec<(i64, f32)> = r
        .played
        .iter()
        .filter_map(|&(f, _, e)| match e {
            Event::Lane { value, .. } => Some(((f / TICK) as i64, value)),
            _ => None,
        })
        .collect();
    let first = lanes.first().unwrap();
    assert_eq!(first.0, BAR);
    assert!((first.1 - 0.5).abs() < 0.01, "{lanes:?}");
    assert!(
        lanes.iter().all(|l| l.0 >= BAR && l.0 < 2 * BAR + 8),
        "{lanes:?}"
    );
}

#[test]
fn song_mode_off_plays_only_the_launches() {
    let mut s = session(at(2 * BAR));
    s.project
        .arrangement
        .place(Placement::linked(0, 1, 0, 2 * BAR))
        .unwrap();
    let mut view = PlayView::compile(&s.project, &[]).unwrap();
    view.set_song(&s.project, false).unwrap();
    let mut t = Transport::new(RATE, 120.0).unwrap();
    t.command(0, Command::Play);
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    while t.frame() < s.frames {
        t.process(512, &mut block);
        player.process(&view, &block, &mut log);
    }
    assert!(log.0.is_empty());
    assert!(!view.song());
}
