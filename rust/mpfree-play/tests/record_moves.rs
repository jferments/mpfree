//! Perform FX gestures recorded as moves: while the transport records,
//! each gesture lands on the armed track's clip ticks, ends where it ends
//! (or at a punch out), runs on past the clip's end, names a parameter on
//! the track relatively and any other fixed, undoes and redoes with its
//! take, goes into the play view in place exactly as a compile would, and
//! comes out the same at any period size.

use mpfree_play::{Launch, PlayView, Recorder, TakenMove, Takes};
use mpfree_seq::mpfree_moves::{Input, Kind, Map, Point, Target};
use mpfree_seq::mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr, Strip};
use mpfree_seq::{Clip, Project, Track};
use mpfree_transport::{Block, Command, Transport};

/// Frames per tick at 120 BPM and 44.1 kHz.
const K: f64 = 22.968_75;

fn frame(t: f64) -> u64 {
    (t * K).round() as u64
}

/// The armed track's instrument parameter 3.
fn cut() -> ParamAddr {
    ParamAddr::new(Owner::Inst(0), 3)
}

fn mv(addr: ParamAddr, value: f32) -> Input {
    Input::Move {
        addr,
        value,
        map: Map::linear(0.0, 1.0),
    }
}

struct Session {
    project: Project,
    launches: Vec<Launch>,
    commands: Vec<(u64, Command)>,
    /// Gestures at song ticks (120 BPM from frame 0), in order.
    moves: Vec<(f64, Input)>,
    frames: u64,
}

/// A handed-over gesture and its points.
type TakenWithPoints = (TakenMove, Vec<(f64, f32, Kind)>);

struct Run {
    taken: Vec<TakenWithPoints>,
    project: Project,
    takes: Takes,
    compiles: usize,
    in_place: usize,
}

fn session(length: i64) -> Session {
    let mut p = Project::new();
    let mut tr = Track::new("keys", 0);
    tr.slots[3] = Some(Clip::new("c", length));
    p.tracks.push(tr);
    Session {
        project: p,
        launches: vec![Launch {
            track: 0,
            slot: 3,
            origin: 0,
            start: 0,
        }],
        commands: vec![(0, Command::Record)],
        moves: vec![],
        frames: 0,
    }
}

/// Play the session in periods of `period`. Each gesture handed over goes
/// into the clip through [`Takes::put_move`] and into the view in place
/// when it has room; the view must then be what a compile gives.
fn run(s: &Session, period: u32) -> Run {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    for &(f, c) in &s.commands {
        assert!(t.command(f, c));
    }
    let mut project = s.project.clone();
    let compile = |p: &Project| {
        let mut v = PlayView::compile(p, &s.launches).unwrap();
        v.set_armed(Some(0));
        v.make_room(64, 1024);
        v
    };
    let mut view = compile(&project);
    let mut rec = Recorder::new();
    let mut block = Block::new();
    let mut takes = Takes::default();
    let mut taken = vec![];
    let (mut compiles, mut in_place) = (0, 0);
    let mut moves = s.moves.iter().peekable();
    while t.frame() < s.frames {
        let n = period.min((s.frames - t.frame()) as u32);
        let start = t.frame();
        t.process(n, &mut block);
        while let Some(&&(tick, input)) = moves.peek() {
            let f = frame(tick);
            if f >= start + u64::from(n) {
                break;
            }
            rec.input_move((f - start) as u32, input);
            moves.next();
        }
        rec.process(&view, &block);
        let mut compile_now = false;
        for k in rec.taken_moves() {
            let points = rec.move_points(k).to_vec();
            takes.put_move(&mut project, k, &points).unwrap();
            if view.put_move(k.track, k.slot, k.target, k.map, &points, k.len) {
                in_place += 1;
            } else {
                compile_now = true;
            }
            taken.push((*k, points));
        }
        if compile_now {
            view = compile(&project);
            compiles += 1;
        }
        assert_eq!(view, compile(&project), "the view as a compile makes it");
    }
    assert_eq!(rec.dropped(), 0);
    Run {
        taken,
        project,
        takes,
        compiles,
        in_place,
    }
}

/// Points rounded to whole ticks, for comparing.
fn rounded(points: &[(f64, f32, Kind)]) -> Vec<(i64, f32, Kind)> {
    points
        .iter()
        .map(|&(t, u, k)| (t.round() as i64, (u * 1000.0).round() / 1000.0, k))
        .collect()
}

fn lane(p: &Project, target: Target) -> Vec<Point> {
    p.clip(0, 3)
        .unwrap()
        .moves
        .lane(target)
        .map(|l| l.points.clone())
        .unwrap_or_default()
}

#[test]
fn a_gesture_while_recording_becomes_a_lane_and_undoes_with_its_take() {
    let mut s = session(3840);
    s.moves = vec![
        (100.0, mv(cut(), 0.2)),
        (200.0, mv(cut(), 0.5)),
        (300.0, mv(cut(), 0.9)),
        (400.0, Input::End { addr: cut() }),
    ];
    s.frames = frame(1000.0);
    let mut r = run(&s, 128);
    assert_eq!(r.taken.len(), 1);
    let (k, points) = &r.taken[0];
    let target = Target::Track(RelAddr::new(OnTrack::Inst, 3));
    assert_eq!(
        (k.track, k.slot, k.target, k.len, k.take),
        (0, 3, target, 3840, 1)
    );
    assert_eq!(
        rounded(points),
        vec![
            (100, 0.2, Kind::Ramp),
            (200, 0.5, Kind::Ramp),
            (300, 0.9, Kind::Ramp),
            (400, 0.0, Kind::End)
        ]
    );
    let written = lane(&r.project, target);
    assert_eq!(
        written.iter().map(|p| (p.tick, p.kind)).collect::<Vec<_>>(),
        vec![
            (100, Kind::Ramp),
            (200, Kind::Ramp),
            (300, Kind::Ramp),
            (400, Kind::End)
        ]
    );
    assert_eq!(r.takes.undo(&mut r.project), Some(1));
    assert!(
        r.project.clip(0, 3).unwrap().moves.is_empty(),
        "undo takes the lane away"
    );
    assert_eq!(r.takes.redo(&mut r.project), Some(1));
    assert_eq!(lane(&r.project, target), written, "redo puts it back");
}

#[test]
fn a_punch_out_ends_a_held_gesture_there() {
    let mut s = session(3840);
    s.commands.push((frame(1000.0), Command::Record));
    s.moves = vec![(100.0, mv(cut(), 0.3)), (500.0, mv(cut(), 0.6))];
    s.frames = frame(2000.0);
    let r = run(&s, 128);
    assert_eq!(r.taken.len(), 1);
    assert_eq!(
        rounded(&r.taken[0].1),
        vec![
            (100, 0.3, Kind::Ramp),
            (500, 0.6, Kind::Ramp),
            (1000, 0.0, Kind::End)
        ]
    );
}

#[test]
fn a_gesture_over_the_clips_end_runs_on_into_the_next_pass() {
    let mut s = session(960);
    s.moves = vec![
        (900.0, mv(cut(), 0.1)),
        (1000.0, mv(cut(), 0.4)),
        (1100.0, Input::End { addr: cut() }),
    ];
    s.frames = frame(1500.0);
    let r = run(&s, 128);
    assert_eq!(
        rounded(&r.taken[0].1),
        vec![
            (900, 0.1, Kind::Ramp),
            (1000, 0.4, Kind::Ramp),
            (1100, 0.0, Kind::End)
        ]
    );
    // Written as two: to the clip's last tick, and on from tick 0.
    let target = Target::Track(RelAddr::new(OnTrack::Inst, 3));
    let ticks: Vec<_> = lane(&r.project, target)
        .iter()
        .map(|p| (p.tick, p.kind))
        .collect();
    assert_eq!(ticks.first(), Some(&(0, Kind::Ramp)), "{ticks:?}");
    assert!(
        ticks.contains(&(900, Kind::Ramp)) && ticks.contains(&(140, Kind::End)),
        "{ticks:?}"
    );
}

#[test]
fn nothing_is_recorded_while_only_playing() {
    let mut s = session(3840);
    s.commands = vec![(0, Command::Play)];
    s.moves = vec![(100.0, mv(cut(), 0.2)), (300.0, Input::End { addr: cut() })];
    s.frames = frame(1000.0);
    let r = run(&s, 128);
    assert!(r.taken.is_empty());
}

#[test]
fn a_parameter_off_the_track_gets_a_fixed_lane() {
    let master = ParamAddr::new(Owner::Insert(Strip::Master, 4), 2);
    let mut s = session(3840);
    s.moves = vec![
        (100.0, mv(master, 0.7)),
        (200.0, Input::End { addr: master }),
    ];
    s.frames = frame(1000.0);
    let r = run(&s, 128);
    assert_eq!(r.taken[0].0.target, Target::Fixed(master));
    assert_eq!(lane(&r.project, Target::Fixed(master)).len(), 2);
}

#[test]
fn a_long_gesture_is_handed_over_in_parts_and_written_whole() {
    let mut s = session(4 * 3840);
    // A value every 4 ticks for 3 bars: 2880 values, three parts.
    s.moves = (0..2880)
        .map(|i| {
            let t = 10.0 + 4.0 * f64::from(i);
            (t, mv(cut(), ((i % 100) as f32) / 100.0))
        })
        .collect();
    s.moves
        .push((10.0 + 4.0 * 2880.0, Input::End { addr: cut() }));
    s.frames = frame(4.0 * 3840.0);
    let r = run(&s, 256);
    assert_eq!(r.taken.len(), 3, "parts of 1024 values");
    let total: usize = r.taken.iter().map(|t| t.1.len()).sum();
    assert_eq!(total, 2881);
    // The parts join up: each starts after the last ended.
    for w in r.taken.windows(2) {
        assert!(w[1].1[0].0 > w[0].1.last().unwrap().0);
    }
    // The second gesture's lane needed no compile: the first made the lane
    // and the room set aside took the rest.
    assert_eq!(r.compiles, 1, "a new lane compiles once");
    assert_eq!(r.in_place, 2);
}

#[test]
fn the_same_moves_at_any_period_size() {
    let mut s = session(1920);
    s.commands.push((frame(5000.0), Command::Record));
    s.commands.push((frame(6000.0), Command::Record));
    s.moves = (0..300)
        .map(|i| {
            let t = 37.0 + 23.0 * f64::from(i);
            let input = if i % 50 == 49 {
                Input::End { addr: cut() }
            } else {
                mv(cut(), ((i * 7) % 13) as f32 / 13.0)
            };
            (t, input)
        })
        .collect();
    s.frames = frame(8000.0);
    let want = run(&s, 128);
    assert!(want.taken.len() >= 6, "{}", want.taken.len());
    for period in [1, 7, 64, 333, 1024, 4096] {
        let got = run(&s, period);
        assert_eq!(got.taken.len(), want.taken.len(), "period {period}");
        for (a, b) in got.taken.iter().zip(&want.taken) {
            assert_eq!(a.0, b.0, "period {period}");
            assert_eq!(rounded(&a.1), rounded(&b.1), "period {period}");
        }
        assert_eq!(got.project, want.project, "period {period}");
    }
}
