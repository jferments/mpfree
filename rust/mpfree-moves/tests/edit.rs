//! Step page edits keep every lane as it was on the ticks they do not
//! touch; a write saved with `put_saving` comes back exactly with
//! `restore`; `check` finds unsound moves.

use mpfree_moves::*;
use mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr, Strip};

const PAGE: u32 = 960;
const LEN: u32 = 4 * PAGE;

fn targets() -> [(Target, Map); 3] {
    [
        (
            Target::Fixed(ParamAddr::new(Owner::Insert(Strip::Master, 3), 66)),
            Map::linear(-100.0, 100.0),
        ),
        (
            Target::Track(RelAddr::new(OnTrack::Insert(2), 0)),
            Map::linear(20.0, 20_000.0),
        ),
        (
            Target::Track(RelAddr::new(OnTrack::Insert(2), 69)),
            Map::toggle(),
        ),
    ]
}

/// A small fixed random source.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }
    fn unit(&mut self) -> f32 {
        self.below(10_001) as f32 / 10_000.0
    }
}

/// A gesture of a few points from about `start`, ending or not, maybe
/// crossing the loop's end or running longer than the clip.
fn gesture(r: &mut Lcg, len: u32) -> Vec<(f64, f32, Kind)> {
    let start = f64::from(r.below(len));
    let n = 1 + r.below(12);
    let mut t = start;
    let mut out = Vec::new();
    for _ in 0..n {
        let kind = if r.below(4) == 0 {
            Kind::Step
        } else {
            Kind::Ramp
        };
        out.push((t, r.unit(), kind));
        let far = r.below(8) == 0;
        t += f64::from(1 + r.below(if far { len } else { 200 }));
    }
    if r.below(3) != 0 {
        out.push((t, 0.0, Kind::End));
    }
    out
}

fn random_moves(seed: u64, gestures: usize) -> Moves {
    let mut r = Lcg(seed);
    let mut m = Moves::new();
    let ts = targets();
    for _ in 0..gestures {
        let (target, map) = ts[r.below(3) as usize];
        let g = gesture(&mut r, LEN);
        m.put(target, map, &g, LEN).unwrap();
    }
    m
}

/// Each target's knob position on every tick of a clip `len` long.
fn states(m: &Moves, len: u32) -> Vec<Vec<Option<f32>>> {
    targets()
        .iter()
        .map(|(t, _)| {
            (0..len)
                .map(|k| m.lane(*t).and_then(|l| l.state_at(f64::from(k))))
                .collect()
        })
        .collect()
}

fn same(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => (x - y).abs() < 1e-4,
        _ => false,
    }
}

/// `new[t]` is `old[map(t)]` on every tick `t` of the new clip.
fn follows(
    old: &[Vec<Option<f32>>],
    new: &[Vec<Option<f32>>],
    what: &str,
    map: impl Fn(usize) -> usize,
) {
    for (lane, (o, n)) in old.iter().zip(new).enumerate() {
        for t in 0..n.len() {
            let s = map(t);
            assert!(
                same(n[t], o[s]),
                "{what}: lane {lane} tick {t} is {:?}, was {:?} on tick {s}",
                n[t],
                o[s]
            );
        }
    }
}

#[test]
fn a_saved_write_comes_back_exactly() {
    let ts = targets();
    for seed in 0..300u64 {
        let mut m = random_moves(seed, (seed % 6) as usize);
        let before = m.clone();
        let mut r = Lcg(seed ^ 0xA5A5);
        let (target, map) = ts[r.below(3) as usize];
        let g = gesture(&mut r, LEN);
        let mut saved: Vec<(u32, u32, Vec<Point>)> = Vec::new();
        m.put_saving(target, map, &g, LEN, |a, b, old| {
            saved.push((a, b, old.to_vec()))
        })
        .unwrap();
        assert!(!saved.is_empty() && saved.len() <= 2);
        for (a, b, old) in saved.iter().rev() {
            m.restore(target, map, *a, *b, old).unwrap();
        }
        assert_eq!(m, before, "seed {seed}: {g:?}");
    }
}

#[test]
fn a_refused_write_saves_nothing() {
    let mut m = Moves::new();
    let mut called = false;
    let e = m.put_saving(
        targets()[0].0,
        targets()[0].1,
        &[(10.0, 0.5, Kind::Ramp), (5.0, 0.2, Kind::Ramp)],
        LEN,
        |_, _, _| called = true,
    );
    assert_eq!(e, Err(MovesError::Bad));
    assert!(!called && m.is_empty());
}

#[test]
fn copying_a_page_plays_it_there_and_leaves_the_rest() {
    for seed in 0..200u64 {
        let m = random_moves(seed, 8);
        let old = states(&m, LEN);
        let mut r = Lcg(seed * 7 + 1);
        let from = r.below(LEN);
        let to = from + 1 + r.below(LEN - from);
        let at = r.below(LEN);
        let mut c = m.clone();
        c.copy_range(from, to, at, LEN).unwrap();
        c.check(LEN).unwrap();
        let w = (to - from).min(LEN - at) as usize;
        let (from, at) = (from as usize, at as usize);
        let new = states(&c, LEN);
        follows(
            &old,
            &new,
            &format!("seed {seed} copy {from}.. onto {at} ({w})"),
            |t| {
                if t >= at && t < at + w {
                    t - at + from
                } else {
                    t
                }
            },
        );
    }
}

#[test]
fn cutting_a_stretch_closes_the_gap() {
    for seed in 0..200u64 {
        let m = random_moves(seed, 8);
        let old = states(&m, LEN);
        let mut r = Lcg(seed * 11 + 3);
        let from = r.below(LEN);
        let to = from + 1 + r.below(LEN - from);
        if to - from == LEN {
            continue;
        }
        let mut c = m.clone();
        c.cut(from, to, LEN).unwrap();
        let len = LEN - (to - from);
        c.check(len).unwrap();
        let new = states(&c, len);
        let (from, w) = (from as usize, (to - from) as usize);
        follows(&old, &new, &format!("seed {seed} cut {from}..+{w}"), |t| {
            if t < from {
                t
            } else {
                t + w
            }
        });
    }
}

#[test]
fn inserting_a_gap_holds_through_it() {
    for seed in 0..200u64 {
        let m = random_moves(seed, 8);
        let old = states(&m, LEN);
        let mut r = Lcg(seed * 13 + 5);
        let at = r.below(LEN);
        let n = 1 + r.below(2 * PAGE);
        let mut c = m.clone();
        c.insert(at, n).unwrap();
        let len = LEN + n;
        c.check(len).unwrap();
        let new = states(&c, len);
        // Lanes with a point on `at`: that point moves on, and the gap
        // holds what came before it (checked on a known lane below).
        let on_at: Vec<bool> = targets()
            .iter()
            .map(|(t, _)| {
                m.lane(*t)
                    .is_some_and(|l| l.points.iter().any(|p| p.tick == at))
            })
            .collect();
        let (at, n) = (at as usize, n as usize);
        let mut old = old;
        let mut new = new;
        for (k, &skip) in on_at.iter().enumerate() {
            if skip {
                for t in at..at + n {
                    new[k][t] = old[k][at];
                }
            }
        }
        follows(
            &old,
            &new,
            &format!("seed {seed} insert {n} at {at}"),
            |t| {
                if t < at {
                    t
                } else if t < at + n {
                    at
                } else {
                    t - n
                }
            },
        );
        old.clear();
    }
}

#[test]
fn shortening_the_clip_keeps_what_is_left() {
    for seed in 0..200u64 {
        let m = random_moves(seed, 8);
        let old = states(&m, LEN);
        let len = 1 + Lcg(seed * 17 + 9).below(LEN - 1);
        let mut c = m.clone();
        c.truncate(len).unwrap();
        c.check(len).unwrap();
        let new = states(&c, len);
        follows(&old, &new, &format!("seed {seed} truncate {len}"), |t| t);
    }
}

#[test]
fn page_edits_on_a_known_lane() {
    // One ramp over page 2, from 0.2 to 0.6, then an end.
    let (dj, map) = targets()[0];
    let mut m = Moves::new();
    m.put(
        dj,
        map,
        &[
            (960.0, 0.2, Kind::Ramp),
            (1919.0, 0.6, Kind::Ramp),
            (1920.0, 0.0, Kind::End),
        ],
        LEN,
    )
    .unwrap();
    // Copy page 2 onto page 4: the same ramp there, then free.
    let mut c = m.clone();
    c.copy_range(960, 1920, 2880, LEN).unwrap();
    let lane = c.lane(dj).unwrap();
    assert_eq!(lane.state_at(2880.0), Some(0.2));
    assert!((lane.state_at(3359.5).unwrap() - lane.state_at(1439.5).unwrap()).abs() < 1e-6);
    assert_eq!(lane.state_at(3839.0), Some(0.6));
    // The clip now ends on the copy's 0.6, but its start stays free.
    assert_eq!(lane.state_at(0.0), None);
    assert_eq!(lane.state_at(959.0), None);
    // Take page 1 off: the ramp starts on tick 0.
    let mut c = m.clone();
    c.cut(0, 960, LEN).unwrap();
    let lane = c.lane(dj).unwrap();
    assert_eq!(lane.points[0], Point::ramp(0, 0.2));
    assert_eq!(lane.state_at(960.0), None);
    // Open a page before it: it moves on a page; the gap is free.
    let mut c = m.clone();
    c.insert(960, 960).unwrap();
    let lane = c.lane(dj).unwrap();
    assert_eq!(lane.state_at(1500.0), None);
    assert_eq!(lane.state_at(1920.0), Some(0.2));
    assert_eq!(lane.state_at(2880.0), None);
    // Open one in the middle of the ramp: it holds 0.4 through the gap
    // and carries on from 0.4.
    let mut c = m.clone();
    c.insert(1440, 960).unwrap();
    let lane = c.lane(dj).unwrap();
    for t in [1440.0, 1900.0, 2399.0, 2400.0] {
        assert!((lane.state_at(t).unwrap() - 0.4).abs() < 1e-3, "{t}");
    }
    assert!((lane.state_at(2879.0).unwrap() - 0.6).abs() < 1e-6);
    // Cut the clip off halfway through the ramp: it holds 0.4 to the end
    // and the start stays free.
    let mut c = m.clone();
    c.truncate(1440).unwrap();
    let lane = c.lane(dj).unwrap();
    assert!((lane.state_at(1439.0).unwrap() - 0.4).abs() < 1e-3);
    assert_eq!(lane.state_at(0.0), None);
    assert_eq!(lane.state_at(959.0), None);
}

#[test]
fn check_finds_unsound_moves() {
    let (dj, map) = targets()[0];
    let mut m = Moves::new();
    m.put(
        dj,
        map,
        &[(10.0, 0.5, Kind::Ramp), (20.0, 0.7, Kind::Ramp)],
        100,
    )
    .unwrap();
    assert_eq!(m.check(100), Ok(()));
    assert_eq!(m.check(20), Err(MovesError::Bad), "a tick past the end");
    let mut bad = m.clone();
    bad.lane_mut(dj).unwrap().points[0].u = f32::NAN;
    assert_eq!(bad.check(100), Err(MovesError::Bad), "not a number");
    let mut bad = m.clone();
    bad.lane_mut(dj).unwrap().points[1].tick = 10;
    assert_eq!(bad.check(100), Err(MovesError::Bad), "two points on a tick");
    let mut bad = m.clone();
    bad.lane_mut(dj).unwrap().map.min = f32::INFINITY;
    assert_eq!(bad.check(100), Err(MovesError::Bad), "a map out of range");
}
