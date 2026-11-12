//! Lanes play on a fixed tick grid whatever the period, hold and end where
//! they say, and come back where a clip is launched into them; gestures
//! written into a clip replace what was there, cross the loop's end and
//! hand back to older moves; the log puts moves on the song's ticks and
//! cuts phrases at a bar of stillness.

use mpfree_moves::*;
use mpfree_node::{Curve, ParamInfo, Unit};
use mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr, Strip};
use mpfree_transport::{Block, Command, Transport};

const BAR: u32 = 3840;

fn dj() -> Target {
    Target::Fixed(ParamAddr::new(Owner::Insert(Strip::Master, 3), 66))
}
fn cutoff() -> Target {
    Target::Track(RelAddr::new(OnTrack::Insert(2), 0))
}
fn hold() -> Target {
    Target::Track(RelAddr::new(OnTrack::Insert(2), 69))
}

fn lin() -> Map {
    Map::linear(-100.0, 100.0)
}

/// Everything `play` says over `from..to`, cut at `cuts`.
fn played(m: &Moves, cuts: &[f64]) -> Vec<(f64, Target, Out)> {
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        m.play(w[0], w[1], |t, tg, o| out.push((t, tg, o)));
    }
    out
}

fn one_ramp() -> Moves {
    let mut m = Moves::new();
    m.put(
        dj(),
        lin(),
        &[
            (0.0, 0.5, Kind::Ramp),
            (96.0, 1.0, Kind::Ramp),
            (200.0, 0.0, Kind::End),
        ],
        BAR,
    )
    .unwrap();
    m
}

#[test]
fn a_ramp_sends_every_grid_tick_and_an_end_lets_go() {
    let m = one_ramp();
    let out = played(&m, &[0.0, f64::from(BAR)]);
    let ticks: Vec<f64> = out.iter().map(|o| o.0).collect();
    let mut want: Vec<f64> = (0..=12).map(|k| f64::from(k * GRID)).collect();
    want.push(200.0);
    assert_eq!(ticks, want);
    assert_eq!(out[0].2, Out::Value(0.0));
    // Half way up the ramp: knob 0.75 is +50.
    assert_eq!(out[6].2, Out::Value(50.0));
    assert_eq!(out[12].2, Out::Value(100.0));
    assert_eq!(out[13].2, Out::End);
    assert!(out.iter().all(|o| o.1 == dj()));
}

#[test]
fn the_same_values_however_the_ticks_are_cut() {
    let mut m = one_ramp();
    m.put(
        cutoff(),
        Map::linear(0.0, 1.0),
        &(0..300)
            .map(|i| {
                (
                    1000.0 + f64::from(i) * 3.7,
                    ((i as f32) * 0.05).sin() * 0.5 + 0.5,
                    Kind::Ramp,
                )
            })
            .chain(std::iter::once((2200.0, 0.0, Kind::End)))
            .collect::<Vec<_>>(),
        BAR,
    )
    .unwrap();
    let whole = played(&m, &[0.0, f64::from(BAR)]);
    // Periods of every odd size, starting on fractions of a tick.
    let mut cuts = vec![0.0];
    let mut t = 0.0;
    let mut k = 1u32;
    while t < f64::from(BAR) {
        k = (k * 37 + 11) % 97;
        t = (t + 0.37 + f64::from(k) * 0.61).min(f64::from(BAR));
        cuts.push(t);
    }
    assert_eq!(played(&m, &cuts), whole);
    // And in tick order across the lanes.
    assert!(whole.windows(2).all(|w| w[0].0 <= w[1].0));
}

#[test]
fn a_switch_holds_each_value() {
    let mut m = Moves::new();
    m.put(
        hold(),
        Map::toggle(),
        &[
            (480.0, 1.0, Kind::Ramp),
            (960.0, 0.0, Kind::Ramp),
            (1440.0, 0.0, Kind::End),
        ],
        BAR,
    )
    .unwrap();
    let out = played(&m, &[0.0, f64::from(BAR)]);
    assert_eq!(
        out,
        [
            (480.0, hold(), Out::Value(1.0)),
            (960.0, hold(), Out::Value(0.0)),
            (1440.0, hold(), Out::End),
        ]
    );
    assert_eq!(m.lane(hold()).unwrap().points[0].kind, Kind::Step);
}

#[test]
fn a_lane_says_where_it_is_anywhere_in_the_clip() {
    let m = one_ramp();
    let lane = m.lane(dj()).unwrap();
    assert_eq!(lane.value_at(48.0), Some(50.0));
    assert_eq!(lane.value_at(150.0), Some(100.0));
    assert_eq!(lane.value_at(300.0), None);
    let mut seen = Vec::new();
    m.chase(48.0, |t, o| seen.push((t, o)));
    assert_eq!(seen, [(dj(), Out::Value(50.0))]);
    seen.clear();
    m.chase(3000.0, |t, o| seen.push((t, o)));
    assert_eq!(seen, [(dj(), Out::End)]);

    // A gesture still under way at the end of the pass holds into the
    // next one until its first point.
    let mut m = Moves::new();
    m.put(
        dj(),
        lin(),
        &[(1000.0, 0.25, Kind::Ramp), (2000.0, 0.75, Kind::Step)],
        BAR,
    )
    .unwrap();
    assert_eq!(m.lane(dj()).unwrap().value_at(10.0), Some(50.0));
}

#[test]
fn a_gesture_replaces_what_was_there_and_hands_back() {
    // An old sweep over beats 1 to 3, a new gesture over beat 2: the old
    // points under it go, and where it ends the old sweep goes on.
    let mut m = Moves::new();
    m.put(
        dj(),
        lin(),
        &[
            (0.0, 0.0, Kind::Ramp),
            (2880.0, 1.0, Kind::Ramp),
            (2880.0, 0.0, Kind::End),
        ],
        BAR,
    )
    .unwrap();
    // A value and an end on one tick: the end moves on a tick, so the
    // value is heard.
    let old = m.lane(dj()).unwrap().clone();
    assert_eq!(old.points.len(), 3);
    assert_eq!((old.points[2].tick, old.points[2].kind), (2881, Kind::End));
    let old = {
        let mut m = Moves::new();
        let pts: Vec<_> = (0..=36)
            .map(|i| (f64::from(i * 80), i as f32 / 36.0, Kind::Ramp))
            .chain(std::iter::once((2900.0, 0.0, Kind::End)))
            .collect();
        m.put(dj(), lin(), &pts, BAR).unwrap();
        m
    };
    assert_eq!(
        old.lane(dj()).unwrap().points.len(),
        3,
        "a straight ramp thins to its ends"
    );
    let mut m = old.clone();
    m.put(
        dj(),
        lin(),
        &[(960.0, 0.9, Kind::Step), (1920.0, 0.0, Kind::End)],
        BAR,
    )
    .unwrap();
    let lane = m.lane(dj()).unwrap();
    let was = old.lane(dj()).unwrap().value_at(500.0).unwrap();
    assert!((lane.value_at(500.0).unwrap() - was).abs() < 1e-3);
    assert_eq!(lane.value_at(1500.0), Some(lin().value(0.9)));
    // From 1920 the old sweep, where it was.
    let back = old.lane(dj()).unwrap().state_at(1920.0).unwrap();
    assert_eq!(lane.state_at(1920.0), Some(back));
    assert!(
        (lane.state_at(2400.0).unwrap() - old.lane(dj()).unwrap().state_at(2400.0).unwrap()).abs()
            < 1e-6
    );
    assert_eq!(lane.value_at(3000.0), None);
}

#[test]
fn a_gesture_crossing_the_loops_end_is_written_as_two() {
    let mut m = Moves::new();
    let len = BAR;
    // From 3/4 of the way through to a quarter into the next pass, rising.
    let pts = [
        (2880.0, 0.0, Kind::Ramp),
        (4800.0, 1.0, Kind::Ramp),
        (4800.0, 0.0, Kind::End),
    ];
    m.put(dj(), lin(), &pts, len).unwrap();
    let lane = m.lane(dj()).unwrap();
    // Tick 0 starts where the line crossed (half way), the last tick is
    // just before it.
    let at0 = lane.state_at(0.0).unwrap();
    assert!((at0 - 0.5).abs() < 1e-3, "{at0}");
    let last = lane.state_at(f64::from(len - 1)).unwrap();
    assert!((last - 0.5).abs() < 1e-3, "{last}");
    assert!((lane.state_at(480.0).unwrap() - 0.75).abs() < 1e-3);
    assert_eq!(lane.state_at(960.0), Some(1.0));
    assert_eq!(lane.state_at(961.0), None);
    assert_eq!(lane.state_at(2000.0), None);
    assert!((lane.state_at(3360.0).unwrap() - 0.25).abs() < 1e-3);
    // Points in order, one per tick.
    assert!(lane.points.windows(2).all(|w| w[0].tick < w[1].tick));
}

#[test]
fn a_gesture_longer_than_the_clip_keeps_its_last_pass() {
    let mut m = Moves::new();
    let pts: Vec<_> = (0..=100)
        .map(|i| (f64::from(i) * 100.0, (i % 2) as f32, Kind::Ramp))
        .collect();
    m.put(dj(), lin(), &pts, BAR).unwrap();
    let lane = m.lane(dj()).unwrap();
    // The last 3840 ticks: 6161 to 10000, so clip ticks from 6161 % 3840.
    assert!(lane.points.windows(2).all(|w| w[0].tick < w[1].tick));
    assert!(lane.points.iter().all(|p| p.tick < BAR));
    // Tick of song 10000 is 10000 - 7680 = 2320: the last point, 0 (even).
    assert_eq!(lane.state_at(2320.0), Some(0.0));
    assert_eq!(
        lane.state_at(2300.0).map(|u| (u * 100.0).round()),
        Some(20.0)
    );
}

#[test]
fn moves_refuse_nonsense_and_clear() {
    let mut m = one_ramp();
    assert_eq!(
        m.put(
            dj(),
            lin(),
            &[(5.0, 0.0, Kind::Ramp), (4.0, 0.0, Kind::End)],
            BAR
        ),
        Err(MovesError::Bad)
    );
    assert_eq!(
        m.put(dj(), lin(), &[(5.0, 0.0, Kind::Ramp)], 0),
        Err(MovesError::Bad)
    );
    for k in 0..MAX_LANES as u16 {
        let t = Target::Fixed(ParamAddr::macro_value(k as u8));
        let _ = m.put(t, lin(), &[(1.0, 0.5, Kind::Step)], BAR);
    }
    let full = Target::Fixed(ParamAddr::morph_pos(0));
    assert_eq!(
        m.put(full, lin(), &[(1.0, 0.5, Kind::Step)], BAR),
        Err(MovesError::Full)
    );
    assert_eq!(m.lanes().len(), MAX_LANES);
    m.clear(0, 100);
    assert_eq!(m.lanes().len(), 1);
    assert!(m.lane(dj()).unwrap().points.iter().all(|p| p.tick >= 100));
    assert!(m.remove(dj()).is_some());
    assert!(m.is_empty());
}

#[test]
fn a_map_follows_its_parameter() {
    let cut = ParamInfo {
        id: 0,
        name: "Cutoff",
        group: "",
        min: 20.0,
        max: 20_000.0,
        default: 1000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    };
    let m = Map::of(&cut);
    assert!((m.value(0.5) - 632.4555).abs() < 0.01);
    assert!((m.unit(632.4555) - 0.5).abs() < 1e-5);
    assert!((m.value(m.unit(1234.0)) - 1234.0).abs() < 0.01);
    let choice = Map::of(&ParamInfo {
        curve: Curve::Choice(&["a", "b", "c"]),
        min: 0.0,
        max: 2.0,
        ..cut
    });
    assert!(choice.stepped());
    assert_eq!(choice.value(0.6), 1.0);
    let addr = ParamAddr::new(Owner::Insert(Strip::Track(4), 2), 7);
    assert_eq!(
        Target::for_track(addr, 4),
        Target::Track(RelAddr::new(OnTrack::Insert(2), 7))
    );
    assert_eq!(Target::for_track(addr, 5), Target::Fixed(addr));
    assert_eq!(Target::for_track(addr, 4).on(9), addr.with_track(9));
}

// ---- The log ---------------------------------------------------------------

const RATE: u32 = 48_000;
/// At 120 BPM and 48 kHz a tick is 25 frames.
const TICK: u64 = 25;

struct Rig {
    transport: Box<Transport>,
    block: Box<Block>,
    log: Box<MoveLog>,
    period: u32,
    moves: Vec<(u64, Input)>,
    next: usize,
}

impl Rig {
    fn new(period: u32) -> Rig {
        Rig {
            transport: Box::new(Transport::new(RATE, 120.0).unwrap()),
            block: Box::new(Block::new()),
            log: Box::new(MoveLog::new()),
            period,
            moves: Vec::new(),
            next: 0,
        }
    }
    fn at(&mut self, tick: u64, input: Input) {
        self.moves.push((tick * TICK, input));
        self.moves.sort_by_key(|m| m.0);
    }
    fn run_to(&mut self, frame: u64) {
        while self.transport.frame() < frame {
            let start = self.transport.frame();
            let n = (frame - start).min(u64::from(self.period)) as u32;
            while self.next < self.moves.len() && self.moves[self.next].0 < start + u64::from(n) {
                let (f, i) = self.moves[self.next];
                self.log.input((f - start) as u32, i);
                self.next += 1;
            }
            assert!(self.transport.process(n, &mut self.block));
            self.log.process(&self.block);
        }
    }
}

fn mv(param: u16, value: f32) -> Input {
    Input::Move {
        addr: ParamAddr::new(Owner::Insert(Strip::Master, 3), param),
        value,
        map: lin(),
    }
}
fn end(param: u16) -> Input {
    Input::End {
        addr: ParamAddr::new(Owner::Insert(Strip::Master, 3), param),
    }
}

#[test]
fn the_log_puts_moves_on_the_songs_ticks_and_cuts_phrases() {
    for period in [64, 128, 333] {
        let mut r = Rig::new(period);
        assert!(r.transport.command(0, Command::Play));
        // A sweep in bar 1, then (after a bar of stillness) two knobs
        // moving together in bar 4.
        r.at(100, mv(66, -100.0));
        r.at(400, mv(66, 0.0));
        r.at(700, mv(66, 100.0));
        r.at(900, end(66));
        r.at(3 * 3840 + 10, mv(66, 50.0));
        r.at(3 * 3840 + 20, mv(67, 10.0));
        r.at(3 * 3840 + 500, end(66));
        r.at(3 * 3840 + 900, mv(67, 30.0));
        r.at(3 * 3840 + 950, end(67));
        r.run_to(5 * 3840 * TICK);
        let all: Vec<(f64, f32, Kind)> = r.log.since(0).map(|e| (e.song, e.u, e.kind)).collect();
        assert_eq!(all.len(), 9);
        assert_eq!(all[0], (100.0, 0.0, Kind::Ramp));
        assert_eq!(all[2], (700.0, 1.0, Kind::Ramp));
        assert_eq!(all[3], (900.0, 1.0, Kind::End));
        // The phrase is the last one, two gestures in the order they
        // began.
        let p = r.log.phrase();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].addr.param, 66);
        assert_eq!(p[0].song, f64::from(3 * 3840 + 10));
        assert_eq!(p[0].points.len(), 2);
        assert_eq!(p[1].points.len(), 3);
        assert_eq!(p[1].points[2].2, Kind::End);
        assert_eq!(
            r.log.phrase_span(),
            Some((f64::from(3 * 3840 + 10), f64::from(3 * 3840 + 950)))
        );
        // Into a 2-bar clip that started on bar 1: bar 4 is its second bar.
        let pts = p[1].in_clip(0.0, 2 * 3840);
        assert_eq!(pts[0].0, f64::from(3840 + 20));
        let mut m = Moves::new();
        m.put(Target::Fixed(p[1].addr), p[1].map, &pts, 2 * 3840)
            .unwrap();
        assert_eq!(
            m.lane(Target::Fixed(p[1].addr))
                .unwrap()
                .value_at(f64::from(3840 + 900)),
            Some(30.0)
        );
        r.log.forget();
        assert!(r.log.phrase().is_empty());
        assert_eq!(r.log.dropped(), 0);
    }
}

#[test]
fn a_stop_ends_every_gesture_and_stopped_moves_are_not_kept() {
    let mut r = Rig::new(128);
    assert!(r.transport.command(0, Command::Play));
    r.at(100, mv(66, 20.0));
    r.at(200, mv(67, 20.0));
    r.run_to(1000 * TICK);
    assert!(r.log.moving());
    assert!(r.transport.command(1000 * TICK, Command::Stop));
    r.at(1100, mv(66, 90.0));
    r.run_to(1200 * TICK);
    assert!(!r.log.moving());
    let kinds: Vec<Kind> = r.log.since(0).map(|e| e.kind).collect();
    assert_eq!(kinds, [Kind::Ramp, Kind::Ramp, Kind::End, Kind::End]);
    let p = r.log.phrase();
    assert!(p.iter().all(|g| g.points.last().unwrap().2 == Kind::End));
}
