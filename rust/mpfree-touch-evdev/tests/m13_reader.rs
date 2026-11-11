//! M13 reader tests over a fake device. No device access.

use std::collections::VecDeque;
use std::io;

use mpfree_touch::mt::{MtSnapshot, Phase, SlotSnapshot, EV_ABS, EV_SYN, SYN_DROPPED, SYN_REPORT};
use mpfree_touch::profile::MPC_ONE;
use mpfree_touch_evdev::{parse_input_event, TouchReader, TouchSource, EVENT_SIZE};

const SLOT: u16 = 47;
const X: u16 = 53;
const Y: u16 = 54;
const TID: u16 = 57;

/// Bytes of one native `struct input_event`.
fn event(time_us: u64, kind: u16, code: u16, value: i32) -> Vec<u8> {
    let word = EVENT_SIZE / 2 - 4;
    let mut b = Vec::with_capacity(EVENT_SIZE);
    b.extend_from_slice(&(time_us / 1_000_000).to_ne_bytes()[..word]);
    b.extend_from_slice(&(time_us % 1_000_000).to_ne_bytes()[..word]);
    b.extend_from_slice(&kind.to_ne_bytes());
    b.extend_from_slice(&code.to_ne_bytes());
    b.extend_from_slice(&value.to_ne_bytes());
    assert_eq!(b.len(), EVENT_SIZE);
    b
}

#[derive(Default)]
struct Fake {
    /// Each entry is what one read(2) returns.
    reads: VecDeque<Vec<u8>>,
    snapshots: VecDeque<io::Result<MtSnapshot>>,
    log: Vec<&'static str>,
}

impl TouchSource for Fake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.reads.pop_front() {
            Some(chunk) => {
                self.log.push("read");
                assert!(chunk.len() <= buf.len());
                buf[..chunk.len()].copy_from_slice(&chunk);
                Ok(chunk.len())
            }
            None => Err(io::ErrorKind::WouldBlock.into()),
        }
    }

    fn snapshot(&mut self, slots: usize) -> io::Result<MtSnapshot> {
        assert_eq!(slots, 10);
        self.log.push("snapshot");
        self.snapshots.pop_front().expect("unexpected snapshot")
    }
}

fn snap(current_slot: i32, live: &[(usize, i32, i32, i32)]) -> MtSnapshot {
    let mut slots = vec![SlotSnapshot { tracking_id: -1, raw_x: 0, raw_y: 0 }; 10];
    for &(s, tracking_id, raw_x, raw_y) in live {
        slots[s] = SlotSnapshot { tracking_id, raw_x, raw_y };
    }
    MtSnapshot { current_slot, slots }
}

const SYN: (u16, u16, i32) = (EV_SYN, SYN_REPORT, 0);
const DROP: (u16, u16, i32) = (EV_SYN, SYN_DROPPED, 0);
fn slot(s: i32) -> (u16, u16, i32) { (EV_ABS, SLOT, s) }
fn id(t: i32) -> (u16, u16, i32) { (EV_ABS, TID, t) }
fn x(v: i32) -> (u16, u16, i32) { (EV_ABS, X, v) }
fn y(v: i32) -> (u16, u16, i32) { (EV_ABS, Y, v) }

fn reader(fake: Fake) -> TouchReader<Fake> {
    TouchReader::new(fake, &MPC_ONE, 1280, 800).unwrap()
}

fn phases(evs: &[mpfree_touch::pointer::PointerEvent]) -> Vec<Phase> {
    evs.iter().map(|e| e.phase).collect()
}

#[test]
fn events_round_trip_for_this_build() {
    let ev = parse_input_event(&event(3_000_042, EV_ABS, X, -7));
    assert_eq!((ev.time_us, ev.kind, ev.code, ev.value), (3_000_042, EV_ABS, X, -7));
}

fn chunk(events: &[(u16, u16, i32)]) -> Vec<u8> {
    events.iter().flat_map(|&(k, c, v)| event(1_000, k, c, v)).collect()
}

#[test]
fn open_drains_then_seeds_from_the_snapshot() {
    let mut fake = Fake::default();
    fake.reads.push_back(chunk(&[slot(5), id(1), x(1), y(1), SYN])); // older than the snapshot
    fake.snapshots.push_back(Ok(snap(3, &[(5, 1, 1, 1)])));
    let mut r = reader(fake);
    assert_eq!(r.source().log, vec!["read", "snapshot"]);
    assert_eq!(r.stats().slot_at_open, 3);
    assert_eq!(r.stats().fingers_at_open, 1);
    assert_eq!(r.stats().drained_events, 5);
    // A new finger with no ABS_MT_SLOT goes to slot 3, not slot 0.
    r.source_mut().reads.push_back(chunk(&[id(9), x(1024), y(1024), SYN]));
    let out = r.read().unwrap();
    assert_eq!(phases(&out), vec![Phase::Down]);
    assert_eq!(r.decoder().contacts()[0].slot, 3);
    assert_eq!((out[0].x, out[0].y), (639.5, 399.5));
    assert_eq!(r.stats().contacts, 1);
}

#[test]
fn open_fails_if_the_snapshot_cannot_be_read() {
    let mut fake = Fake::default();
    fake.snapshots.push_back(Err(io::ErrorKind::PermissionDenied.into()));
    assert!(TouchReader::new(fake, &MPC_ONE, 1280, 800).is_err());
}

#[test]
fn the_resting_finger_at_open_is_ignored_and_the_next_one_is_not() {
    let mut fake = Fake::default();
    fake.snapshots.push_back(Ok(snap(0, &[(0, 4, 100, 100)])));
    let mut r = reader(fake);
    r.source_mut().reads.push_back(chunk(&[x(120), SYN, id(-1), SYN, id(5), x(200), SYN]));
    let out = r.read().unwrap();
    assert_eq!(phases(&out), vec![Phase::Down]);
    assert_eq!(r.decoder().contacts()[0].tracking_id, 5);
    // Y was never resent: it comes from the resting finger's last value.
    assert_eq!(r.decoder().contacts()[0].raw_y, 100);
}

#[test]
fn overflow_resyncs_and_a_held_finger_keeps_going() {
    let mut fake = Fake::default();
    fake.snapshots.push_back(Ok(snap(0, &[])));
    let mut r = reader(fake);
    r.source_mut().reads.push_back(chunk(&[slot(0), id(1), x(10), y(10), slot(1), id(2), x(20), y(20), SYN]));
    assert_eq!(phases(&r.read().unwrap()), vec![Phase::Down, Phase::Down]);
    let held = r.decoder().contacts()[0].id;

    // Overflow; finger 2 lifted during it, finger 1 moved. After the post-drop
    // SYN_REPORT there are two stale events in the same read and one more
    // queued: all older than the snapshot, so all dropped.
    r.source_mut().reads.push_back(chunk(&[DROP, x(999), SYN, slot(1), id(-1)]));
    r.source_mut().reads.push_back(chunk(&[slot(0), x(11), SYN]));
    r.source_mut().snapshots.push_back(Ok(snap(0, &[(0, 1, 15, 16)])));
    let out = r.read().unwrap();
    assert_eq!(r.source().log[r.source().log.len() - 3..], ["read", "read", "snapshot"]);
    // Finger 1 moved (same identity), finger 2 is up. No cancels.
    assert_eq!(phases(&out), vec![Phase::Move, Phase::Up]);
    assert_eq!(out[0].id, held);
    assert_eq!(r.decoder().contacts().len(), 1);
    assert_eq!((r.decoder().contacts()[0].raw_x, r.decoder().contacts()[0].raw_y), (15, 16));
    let st = r.stats();
    assert_eq!((st.resyncs, st.resync_fallbacks), (1, 0));
    assert_eq!(st.drained_events, 2 + 3);

    // Events after the resync are applied normally.
    r.source_mut().reads.push_back(chunk(&[x(30), SYN]));
    let out = r.read().unwrap();
    assert_eq!(phases(&out), vec![Phase::Move]);
    assert_eq!(out[0].id, held);
}

#[test]
fn overflow_without_a_snapshot_cancels_every_finger() {
    let mut fake = Fake::default();
    fake.snapshots.push_back(Ok(snap(0, &[])));
    let mut r = reader(fake);
    r.source_mut().reads.push_back(chunk(&[id(1), x(10), y(10), SYN]));
    r.read().unwrap();
    r.source_mut().reads.push_back(chunk(&[DROP, SYN]));
    r.source_mut().snapshots.push_back(Err(io::ErrorKind::Other.into()));
    let out = r.read().unwrap();
    assert_eq!(phases(&out), vec![Phase::Cancel]);
    assert_eq!((r.stats().resyncs, r.stats().resync_fallbacks), (0, 1));
}

#[test]
fn a_full_buffer_is_followed_by_another_read() {
    let mut fake = Fake::default();
    fake.snapshots.push_back(Ok(snap(0, &[])));
    let mut r = reader(fake);
    // 64 events fill the read buffer exactly; the frame ends in the next read.
    let mut first = vec![id(1)];
    for i in 0..63 {
        first.push(x(100 + i));
    }
    r.source_mut().reads.push_back(chunk(&first));
    r.source_mut().reads.push_back(chunk(&[y(5), SYN]));
    let out = r.read().unwrap();
    assert_eq!(phases(&out), vec![Phase::Down]);
    assert_eq!(r.decoder().contacts()[0].raw_x, 162);
}
