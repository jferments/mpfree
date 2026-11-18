//! The track host with lanes: the slots split over other cores sound the
//! same to the bit, a late lane leaves its slots out and gives them back,
//! and the slots are placed by what they cost.

use std::time::{Duration, Instant};

use mpfree_lanes::Config;
use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed};
use mpfree_synth::{Fm, Keys, Poly};
use mpfree_track::{Instrument, Outputs, TrackHost};

const RATE: u32 = 44_100;
const BLOCK: usize = 128;

/// Each track into its own pair; a bit per frozen track.
struct Split {
    tracks: Vec<(Vec<f32>, Vec<f32>)>,
    resting: u32,
}

impl Split {
    fn new(tracks: usize) -> Split {
        Split {
            tracks: vec![(vec![0.0; BLOCK], vec![0.0; BLOCK]); tracks],
            resting: 0,
        }
    }
    fn clear(&mut self) {
        for (l, r) in &mut self.tracks {
            l.fill(0.0);
            r.fill(0.0);
        }
    }
    fn peak(&self, t: usize) -> f32 {
        let (l, r) = &self.tracks[t];
        l.iter().chain(r).fold(0f32, |m, x| m.max(x.abs()))
    }
}

impl Outputs for Split {
    fn track(&mut self, track: usize) -> Option<(&mut [f32], &mut [f32])> {
        self.tracks
            .get_mut(track)
            .map(|(l, r)| (l.as_mut_slice(), r.as_mut_slice()))
    }
    fn wants(&self, track: usize) -> bool {
        self.resting & (1 << track) == 0
    }
}

fn ctx(block: u64) -> Ctx<'static> {
    Ctx::new(RATE, block * BLOCK as u64)
}

fn inst(k: usize) -> Instrument {
    let node: Box<dyn Node> = match k % 3 {
        0 => Box::new(Poly::new(RATE)),
        1 => Box::new(Fm::new(RATE)),
        _ => Box::new(Keys::new(RATE)),
    };
    Instrument::new(node, RATE, BLOCK)
}

/// A busy phrase on six slots: chords on every slot, a swap (the old one
/// rings out), a frozen track and its thaw, STOP and play again. Each
/// block's tracks, in order.
fn phrase(lanes: usize) -> (Vec<Vec<f32>>, TrackHost) {
    let mut host = TrackHost::new(6, RATE, BLOCK);
    for s in 0..6 {
        host.swap(s, Some(inst(s))).unwrap();
    }
    if lanes > 0 {
        let setups = host.set_lanes(vec![Config::default(); lanes]);
        assert_eq!(setups.len(), lanes);
    }
    let mut spare = Some(inst(4));
    let mut out = Split::new(6);
    let mut blocks = Vec::new();
    for b in 0..600u64 {
        if b % 40 == 0 {
            for s in 0..6 {
                let root = 40 + ((b / 40) as u8 * 5 + s as u8 * 3) % 24;
                for (k, d) in [0u8, 4, 7].iter().enumerate() {
                    host.note_off(s, 0, root.wrapping_sub(5) + d);
                    host.note_on(s, (k * 9) as u32, root + d, 90 + k as u8 * 10);
                }
            }
        }
        if b == 100 {
            host.swap(2, spare.take()).unwrap();
        }
        out.resting = if (200..230).contains(&b) { 1 << 3 } else { 0 };
        if b == 300 {
            host.stop();
        }
        out.clear();
        host.render(&ctx(b), BLOCK, &mut out);
        host.settle(Instant::now() + Duration::from_secs(1));
        blocks.push(
            out.tracks
                .iter()
                .flat_map(|(l, r)| l.iter().chain(r).copied())
                .collect(),
        );
    }
    (blocks, host)
}

#[test]
fn slots_on_lanes_sound_the_same_to_the_bit() {
    let (here, _) = phrase(0);
    assert!(here.iter().flatten().any(|&x| x.abs() > 0.01), "it plays");
    for lanes in [1, 2] {
        let (split, host) = phrase(lanes);
        for (b, (a, c)) in here.iter().zip(&split).enumerate() {
            let same = a.iter().zip(c).all(|(x, y)| x.to_bits() == y.to_bits());
            assert!(same, "{lanes} lane(s): block {b} differs");
        }
        assert_eq!(host.lanes(), lanes);
        // Every slot with an instrument has a group, and the lanes had work.
        let used: Vec<usize> = (0..6).filter_map(|s| host.group_of(s)).collect();
        assert!(used.iter().any(|&g| g > 0), "{used:?}");
        for k in 0..lanes {
            let c = host.lane_counts(k).unwrap();
            assert!(c.periods > 500, "{c:?}");
            assert_eq!((c.late, c.waits), (0, 0));
        }
    }
}

/// An instrument that takes `ms` milliseconds a block and plays a steady
/// 0.25 while it has a note.
struct Slow {
    ms: u64,
    on: bool,
}

static SLOW: NodeInfo = NodeInfo {
    id: "test.slow",
    name: "Slow",
    kind: Kind::Instrument,
};

impl Node for Slow {
    fn info(&self) -> &'static NodeInfo {
        &SLOW
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        0.0
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {
        self.on = false;
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        for e in events {
            match e.ev {
                NodeEvent::NoteOn { .. } => self.on = true,
                NodeEvent::NoteOff { .. } | NodeEvent::AllOff => self.on = false,
                _ => {}
            }
        }
        // Busy, as a costly engine is (a sleep would give the core away).
        let until = Instant::now() + Duration::from_millis(self.ms);
        while Instant::now() < until {
            std::hint::spin_loop();
        }
        if self.on {
            left.fill(0.25);
            right.fill(0.25);
        }
    }
}

fn slow(ms: u64) -> Instrument {
    Instrument::new(Box::new(Slow { ms, on: false }), RATE, BLOCK)
}

#[test]
fn a_late_lane_leaves_its_slots_out_and_gives_them_back() {
    let mut host = TrackHost::new(2, RATE, BLOCK);
    host.swap(0, Some(slow(20))).unwrap();
    host.swap(1, Some(slow(0))).unwrap();
    host.set_lanes(vec![Config::default()]);
    // Placed in turn as nothing is timed yet, the lane first: the slow one
    // on the lane.
    let lane_slot = 0;
    assert_eq!(host.group_of(0), Some(1));
    assert_eq!(host.group_of(1), Some(0));
    host.note_on(0, 0, 60, 100);
    host.note_on(1, 0, 60, 100);
    let mut out = Split::new(2);
    // The deadline long before the lane can finish: its slot is left out.
    host.set_deadline(Some(Instant::now()));
    host.render(&ctx(0), BLOCK, &mut out);
    assert_eq!(out.peak(lane_slot), 0.0, "left out");
    assert_eq!(out.peak(1 - lane_slot), 0.25, "the other plays");
    assert_eq!(host.lane_counts(0).unwrap().late, 1);
    assert!(host.instrument(lane_slot).is_none(), "still on the lane");
    // Settled after the write: back, the note still held.
    host.settle(Instant::now() + Duration::from_secs(1));
    assert!(host.instrument(lane_slot).is_some());
    assert!(host.holds_note(lane_slot, 60));
    host.set_deadline(None);
    out.clear();
    host.render(&ctx(1), BLOCK, &mut out);
    assert_eq!(out.peak(lane_slot), 0.25, "plays again");
    // Late again and not settled: the next render takes it back first,
    // and counts the wait.
    host.set_deadline(Some(Instant::now()));
    host.render(&ctx(2), BLOCK, &mut out);
    host.set_deadline(Some(Instant::now()));
    out.clear();
    host.render(&ctx(3), BLOCK, &mut out);
    let c = host.lane_counts(0).unwrap();
    assert_eq!(c.late, 3, "{c:?}");
    assert!(c.waits >= 1, "{c:?}");
    host.settle(Instant::now() + Duration::from_secs(1));
    assert!(host.instrument(lane_slot).is_some());
}

#[test]
fn two_costly_slots_on_one_core_are_parted() {
    let mut host = TrackHost::new(4, RATE, BLOCK);
    for (s, ms) in [1, 0, 0, 1].into_iter().enumerate() {
        host.swap(s, Some(slow(ms))).unwrap();
    }
    host.set_lanes(vec![Config::default(); 2]);
    // Nothing timed yet: placed in turn, the last lane first, so the two
    // costly ones (0 and 3) share the second lane's core.
    let (a, b) = (0, 3);
    assert_eq!(host.group_of(a), Some(2));
    assert_eq!(host.group_of(b), Some(2));
    let mut out = Split::new(4);
    for blk in 0..(2 * mpfree_track::PLACE_EVERY as u64) {
        out.clear();
        host.render(&ctx(blk), BLOCK, &mut out);
        host.settle(Instant::now() + Duration::from_secs(1));
    }
    assert!(host.cost_ns(a) > 500_000.0, "timed: {}", host.cost_ns(a));
    assert_ne!(host.group_of(a), host.group_of(b), "parted");
    for k in 0..2 {
        assert_eq!(host.lane_counts(k).unwrap().late, 0);
    }
}
