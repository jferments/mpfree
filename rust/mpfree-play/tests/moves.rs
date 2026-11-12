//! FX moves in clips: a clip's lanes play as `Event::Lane` values on the
//! clip's own ticks, end with an unlock, chase on a jump, give way to a
//! step's parameter lock for its step and take the parameter back at the
//! next note, end on a switch, and play the same at any period size.

use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::mpfree_moves::{Map, Point, Target};
use mpfree_seq::mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr, Strip};
use mpfree_seq::{Clip, Lock, Note, Project, Track};
use mpfree_transport::{Block, Command, Transport};

const STEP: i64 = 240;
const BAR: i64 = 3840;
const CUT: RelAddr = RelAddr::new(OnTrack::Inst, 7);
/// Frames in one bar at 120 BPM: 2 s.
const BAR_FRAMES: u64 = 88_200;
/// Frames a tick at 120 BPM.
const FPT: f64 = 22_050.0 / 960.0;

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum What {
    On(u8),
    Off(u8),
    Lock(ParamAddr, f32),
    Unlock(ParamAddr),
    Lane(ParamAddr, f32),
    AllOff,
}

fn what(log: &[Out]) -> Vec<(u64, u16, What)> {
    log.iter()
        .map(|o| {
            let w = match o.event {
                Event::NoteOn { note, .. } => What::On(note),
                Event::NoteOff { note } => What::Off(note),
                Event::Lock { addr, value } => What::Lock(addr, value),
                Event::Unlock { addr } => What::Unlock(addr),
                Event::Lane { addr, value } => What::Lane(addr, value),
                Event::AllOff => What::AllOff,
                Event::Pressure { .. } => unreachable!("no pressure here"),
            };
            (o.frame, o.track, w)
        })
        .collect()
}

fn inst(t: u8, p: u16) -> ParamAddr {
    ParamAddr::new(Owner::Inst(t), p)
}

/// A lane on the track's cutoff from `points`, with the value equal to
/// the knob position times `scale`.
fn with_lane(mut c: Clip, target: Target, scale: f32, points: &[Point]) -> Clip {
    let len = c.length as u32;
    let map = Map::linear(0.0, scale);
    let pts: Vec<_> = points
        .iter()
        .map(|p| (f64::from(p.tick), p.u, p.kind))
        .collect();
    c.moves.put(target, map, &pts, len).unwrap();
    assert_eq!(
        c.moves.lanes()[0].points.len(),
        points.len(),
        "put kept every point"
    );
    c
}

fn clip(length: i64, notes: &[(i64, u8)], locks: &[Lock]) -> Clip {
    let mut c = Clip::new("c", length);
    c.notes = notes
        .iter()
        .map(|&(tick, n)| Note::new(tick, STEP / 2, n, 100))
        .collect();
    c.locks = locks.to_vec();
    c
}

fn project(tracks: usize) -> Project {
    let mut p = Project::new();
    for i in 0..tracks {
        p.tracks.push(Track::new("t", i as u16));
    }
    p
}

fn one(c: Clip) -> PlayView {
    let mut p = project(1);
    p.tracks[0].slots[0] = Some(c);
    PlayView::compile(
        &p,
        &[Launch {
            track: 0,
            slot: 0,
            origin: 0,
            start: 0,
        }],
    )
    .unwrap()
}

/// Play `frames` frames in periods of `period`, with the commands.
fn run_by(
    view: &PlayView,
    cmds: &[(u64, Command)],
    frames: u64,
    period: u32,
) -> (Vec<Out>, Player) {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    for &(at, c) in cmds {
        assert!(t.command(at, c));
    }
    t.command(0, Command::Play);
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    while t.frame() < frames {
        t.process(period.min((frames - t.frame()) as u32), &mut block);
        player.process(view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped, 0);
    (log.0, player)
}

fn run(view: &PlayView, cmds: &[(u64, Command)], frames: u64) -> (Vec<Out>, Player) {
    run_by(view, cmds, frames, 128)
}

/// The frame of song tick `t` at 120 BPM from frame 0.
fn frame_of(t: i64) -> u64 {
    (t as f64 * FPT).round() as u64
}

/// A ramp from 0 at tick 0 to 1 at tick 960, ended on tick 961.
fn ramp() -> [Point; 3] {
    [Point::ramp(0, 0.0), Point::ramp(960, 1.0), Point::end(961)]
}

#[test]
fn a_ramp_plays_on_every_eighth_tick_and_ends_with_an_unlock_each_pass() {
    let view = one(with_lane(
        clip(BAR, &[], &[]),
        Target::Track(CUT),
        1.0,
        &ramp(),
    ));
    let (log, player) = run(&view, &[], 2 * BAR_FRAMES - 100);
    let cut = inst(0, 7);
    let e = what(&log);
    for pass in 0..2 {
        let base = pass * BAR;
        let got: Vec<_> = e
            .iter()
            .filter(|x| x.0 >= frame_of(base) && x.0 < frame_of(base + BAR))
            .collect();
        // 0, 8, ..., 960: 121 values, then the end.
        assert_eq!(got.len(), 122, "pass {pass}");
        for (k, x) in got[..121].iter().enumerate() {
            let t = 8 * k as i64;
            assert_eq!(x.0, frame_of(base + t), "pass {pass} value {k}");
            match x.2 {
                What::Lane(a, v) => {
                    assert_eq!(a, cut);
                    assert!((v - t as f32 / 960.0).abs() < 1e-6, "{v} at {t}");
                }
                w => panic!("{w:?} at {t}"),
            }
        }
        assert_eq!(got[121].2, What::Unlock(cut));
        assert_eq!(got[121].0, frame_of(base + 961));
    }
    assert_eq!(player.locked(), 0);
    assert_eq!(player.stats().lanes, 242);
}

#[test]
fn a_step_lock_beats_the_lane_on_its_step_and_hands_back_at_the_next_note() {
    // The lane's value is its tick, all bar long.
    let lane = [Point::ramp(0, 0.0), Point::ramp(3832, 1.0)];
    let c = clip(
        BAR,
        &[(0, 60), (960, 60), (1920, 60)],
        &[Lock::new(960, None, CUT, -5.0)],
    );
    let view = one(with_lane(c, Target::Track(CUT), 3832.0, &lane));
    let (log, _) = run(&view, &[], frame_of(2000));
    let cut = inst(0, 7);
    let e: Vec<_> = what(&log)
        .into_iter()
        .filter(|x| !matches!(x.2, What::Off(_)))
        .collect();
    let at = |t: i64| -> Vec<What> {
        e.iter()
            .filter(|x| x.0 == frame_of(t))
            .map(|x| x.2)
            .collect()
    };
    let near =
        |w: &What, t: f32| matches!(*w, What::Lane(a, v) if a == cut && (v - t).abs() < 0.01);
    // Before the lock: the lane, beside the first note.
    let first = at(0);
    assert_eq!(first.len(), 2, "{first:?}");
    assert!(
        near(&first[0], 0.0) && first[1] == What::On(60),
        "{first:?}"
    );
    assert!(near(&at(952)[0], 952.0));
    // The lock wins its step; the lane is silent under it ...
    assert_eq!(at(960), vec![What::Lock(cut, -5.0), What::On(60)]);
    for t in (968..1920).step_by(8) {
        assert!(at(t).is_empty(), "{t}: {:?}", at(t));
    }
    // ... and at the next note without a lock, the lane takes over again
    // with its value there, before the note.
    let back = at(1920);
    assert_eq!(back.len(), 2, "{back:?}");
    assert!(
        near(&back[0], 1920.0) && back[1] == What::On(60),
        "{back:?}"
    );
    assert!(near(&at(1928)[0], 1928.0));
    assert!(!e.iter().any(|x| matches!(x.2, What::Unlock(_))));
}

#[test]
fn a_lane_ending_under_a_lock_leaves_the_parameter_free_at_the_next_note() {
    let c = clip(
        BAR,
        &[(0, 60), (STEP, 60), (2 * STEP, 60)],
        &[Lock::new(0, None, CUT, 7.0)],
    );
    let lane = [Point::step(0, 0.5), Point::end(STEP as u32 / 2)];
    let view = one(with_lane(c, Target::Track(CUT), 1.0, &lane));
    let (log, player) = run(&view, &[], frame_of(3 * STEP + 10));
    let cut = inst(0, 7);
    let e: Vec<_> = what(&log)
        .into_iter()
        .filter(|x| !matches!(x.2, What::Off(_)))
        .map(|x| x.2)
        .collect();
    assert_eq!(
        e,
        vec![
            What::Lock(cut, 7.0),
            What::On(60),
            What::Unlock(cut),
            What::On(60),
            What::On(60)
        ]
    );
    assert_eq!(player.locked(), 0);
}

#[test]
fn a_locate_into_a_gesture_chases_its_value() {
    let view = one(with_lane(
        clip(BAR, &[], &[]),
        Target::Track(CUT),
        1.0,
        &ramp(),
    ));
    // Rolling from 0, then a jump to tick 500 at frame 1000.
    let (log, _) = run(&view, &[(1000, Command::Locate(500))], 2000);
    let cut = inst(0, 7);
    let e = what(&log);
    let at: Vec<_> = e.iter().filter(|x| x.0 == 1000).map(|x| x.2).collect();
    // The jump unlocks, then the lane's value at 500 comes back on the
    // same frame (and 504 is on the grid one frame-tick later).
    assert_eq!(at[0], What::Unlock(cut), "{at:?}");
    assert!(
        matches!(at[1], What::Lane(a, v) if a == cut && (v - 500.0 / 960.0).abs() < 1e-6),
        "{at:?}"
    );
    let next = e.iter().find(|x| x.0 > 1000).unwrap();
    assert!(
        matches!(next.2, What::Lane(_, v) if (v - 504.0 / 960.0).abs() < 1e-6),
        "{next:?}"
    );
}

#[test]
fn a_switch_ends_the_old_clips_moves() {
    let mut p = project(1);
    p.tracks[0].slots[0] = Some(with_lane(
        clip(BAR, &[], &[]),
        Target::Track(CUT),
        1.0,
        &[Point::step(0, 0.25)],
    ));
    p.tracks[0].slots[1] = Some(clip(BAR, &[(0, 62)], &[]));
    let launch = [Launch {
        track: 0,
        slot: 0,
        origin: 0,
        start: 0,
    }];
    let playing = PlayView::compile(&p, &launch).unwrap();
    let cued = PlayView::compile_cued(
        &p,
        &launch,
        &[Cue {
            track: 0,
            slot: Some(1),
        }],
    )
    .unwrap();
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let (mut player, mut block, mut l) = (Player::new(), Block::new(), Log::default());
    while t.frame() < 2 * BAR_FRAMES + 100 {
        t.process(128, &mut block);
        let v = if t.frame() <= 1024 { &playing } else { &cued };
        player.process(v, &block, &mut l);
    }
    let cut = inst(0, 7);
    let e: Vec<_> = what(&l.0)
        .into_iter()
        .filter(|x| !matches!(x.2, What::Off(_)))
        .collect();
    assert_eq!(
        e.iter().map(|x| (x.0, x.2)).collect::<Vec<_>>(),
        vec![
            (0, What::Lane(cut, 0.25)),
            (BAR_FRAMES, What::Unlock(cut)),
            (BAR_FRAMES, What::On(62)),
            (2 * BAR_FRAMES, What::On(62)),
        ]
    );
    assert_eq!(player.locked(), 0);
}

#[test]
fn a_fixed_target_moves_the_same_parameter_from_any_track() {
    let master = ParamAddr::new(Owner::Strip(Strip::Master), 3);
    let mut p = project(2);
    p.tracks[1].slots[0] = Some(with_lane(
        clip(BAR, &[], &[]),
        Target::Fixed(master),
        2.0,
        &[Point::step(0, 0.5), Point::end(100)],
    ));
    let view = PlayView::compile(
        &p,
        &[Launch {
            track: 1,
            slot: 0,
            origin: 0,
            start: 0,
        }],
    )
    .unwrap();
    let (log, _) = run(&view, &[], frame_of(200));
    assert_eq!(
        what(&log),
        vec![
            (0, 1, What::Lane(master, 1.0)),
            (frame_of(100), 1, What::Unlock(master))
        ]
    );
}

#[test]
fn a_muted_track_sends_no_moves_and_lets_go_of_them() {
    let mut p = project(1);
    p.tracks[0].slots[0] = Some(with_lane(
        clip(BAR, &[], &[]),
        Target::Track(CUT),
        1.0,
        &[Point::step(0, 0.25), Point::step(2000, 0.75)],
    ));
    let launch = [Launch {
        track: 0,
        slot: 0,
        origin: 0,
        start: 0,
    }];
    let on = PlayView::compile(&p, &launch).unwrap();
    p.tracks[0].mute = true;
    let muted = PlayView::compile(&p, &launch).unwrap();
    p.tracks[0].mute = false;
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let (mut player, mut block, mut l) = (Player::new(), Block::new(), Log::default());
    // Muted from frame 1024 to the middle of the bar, then back.
    while t.frame() < frame_of(3800) {
        t.process(128, &mut block);
        let v = if t.frame() <= 1024 || t.frame() > frame_of(2500) {
            &on
        } else {
            &muted
        };
        player.process(v, &block, &mut l);
    }
    let cut = inst(0, 7);
    let e: Vec<_> = what(&l.0).into_iter().map(|x| x.2).collect();
    // On, let go when muted, the value at the unmute (0.75 since 2000).
    assert_eq!(
        e,
        vec![
            What::Lane(cut, 0.25),
            What::Unlock(cut),
            What::Lane(cut, 0.75)
        ]
    );
}

/// Every lane event, at every period size, on the same frame.
#[test]
fn moves_play_the_same_at_any_period_size() {
    let lane = [
        Point::ramp(0, 0.0),
        Point::ramp(1000, 1.0),
        Point::step(1500, 0.3),
        Point::end(1700),
        Point::ramp(2900, 0.2),
        Point::ramp(3839, 0.9),
    ];
    let c = clip(
        BAR,
        &[(0, 60), (960, 60), (1920, 60), (2880, 60)],
        &[
            Lock::new(960, None, CUT, 5.0),
            Lock::new(2880, None, CUT, 6.0),
        ],
    );
    // And a 7/16 clip at 3/4 speed on a second track, moving a fixed
    // parameter.
    let mut slow = clip(1680, &[(0, 40), (720, 41)], &[]);
    slow.speed = mpfree_seq::Speed::new(3, 4).unwrap();
    let slow = with_lane(
        slow,
        Target::Fixed(ParamAddr::new(Owner::Macro(2), 0)),
        1.0,
        &[
            Point::ramp(100, 0.1),
            Point::ramp(1600, 0.8),
            Point::end(1650),
        ],
    );
    let mut p = project(2);
    p.tracks[0].slots[0] = Some(with_lane(c, Target::Track(CUT), 1.0, &lane));
    p.tracks[1].slots[3] = Some(slow);
    let launch = |track, slot, origin| Launch {
        track,
        slot,
        origin,
        start: origin,
    };
    let view = PlayView::compile(&p, &[launch(0, 0, 0), launch(1, 3, 960)]).unwrap();
    let cmds = [
        (100_000, Command::Locate(1234)),
        (150_000, Command::SetLoop(Some((BAR / 2, BAR + BAR / 2)))),
        (300_000, Command::SetTempo(97.0)),
    ];
    let frames = 4 * BAR_FRAMES;
    let (want, _) = run_by(&view, &cmds, frames, 128);
    let lanes = want
        .iter()
        .filter(|o| matches!(o.event, Event::Lane { .. }))
        .count();
    eprintln!("{lanes} lane values");
    assert!(lanes > 600, "{lanes}");
    for period in [1, 7, 64, 333, 1024, 4096] {
        let (got, _) = run_by(&view, &cmds, frames, period);
        let a: Vec<_> = want.iter().map(|o| (o.frame, o.track, o.event)).collect();
        let b: Vec<_> = got.iter().map(|o| (o.frame, o.track, o.event)).collect();
        assert_eq!(a.len(), b.len(), "period {period}");
        for (k, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(x, y, "period {period}, event {k}");
        }
    }
}
