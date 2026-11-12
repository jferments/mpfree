//! Robustness: whatever the touchscreen sends (out-of-range slots and
//! positions, tracking ids reused or never released, dropped events, a
//! snapshot that disagrees, a read cut mid-event), the reader never panics
//! and every pointer event it hands the GUI is finite and on the screen.

#[path = "../../test-support/mutate.rs"]
mod mutate;

use std::collections::VecDeque;
use std::io;

use mpfree_touch::mt::{MtSnapshot, SlotSnapshot, EV_ABS, EV_SYN, SYN_DROPPED, SYN_REPORT};
use mpfree_touch::profile::MPC_ONE;
use mpfree_touch_evdev::{TouchReader, TouchSource, EVENT_SIZE};
use mutate::Mutator;

const SLOT: u16 = 47;
const X: u16 = 53;
const Y: u16 = 54;
const TID: u16 = 57;
const EV_KEY: u16 = 1;
const BTN_TOUCH: u16 = 330;
const W: u32 = 1280;
const H: u32 = 800;

fn event(time_us: u64, kind: u16, code: u16, value: i32) -> Vec<u8> {
    let word = EVENT_SIZE / 2 - 4;
    let mut b = Vec::with_capacity(EVENT_SIZE);
    b.extend_from_slice(&(time_us / 1_000_000).to_ne_bytes()[..word]);
    b.extend_from_slice(&(time_us % 1_000_000).to_ne_bytes()[..word]);
    b.extend_from_slice(&kind.to_ne_bytes());
    b.extend_from_slice(&code.to_ne_bytes());
    b.extend_from_slice(&value.to_ne_bytes());
    b
}

struct Fake {
    reads: VecDeque<Vec<u8>>,
    m: Mutator,
}

impl TouchSource for Fake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.reads.pop_front() {
            Some(chunk) => {
                let n = chunk.len().min(buf.len());
                buf[..n].copy_from_slice(&chunk[..n]);
                Ok(n)
            }
            None => Err(io::ErrorKind::WouldBlock.into()),
        }
    }

    fn snapshot(&mut self, slots: usize) -> io::Result<MtSnapshot> {
        if self.m.below(8) == 0 {
            return Err(io::Error::other("snapshot failed"));
        }
        let m = &mut self.m;
        let value = |m: &mut Mutator| -> i32 {
            match m.below(4) {
                0 => [i32::MIN, -1, 0, 1, 2048, 4096, i32::MAX][m.below(7)],
                _ => m.below(2200) as i32 - 50,
            }
        };
        let n = if m.below(6) == 0 {
            m.below(2 * slots + 1)
        } else {
            slots
        };
        Ok(MtSnapshot {
            current_slot: value(m).rem_euclid(slots as i32 + 3) - 1,
            slots: (0..n)
                .map(|_| SlotSnapshot {
                    tracking_id: if m.below(2) == 0 { -1 } else { value(m) },
                    raw_x: value(m),
                    raw_y: value(m),
                })
                .collect(),
        })
    }
}

/// A burst of touch traffic: mostly plausible, with edge values mixed in.
fn traffic(m: &mut Mutator, t0: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut t = t0;
    for _ in 0..m.below(40) {
        t += m.below(9000) as u64;
        let edge = [i32::MIN, -1, 0, 2048, 2049, 65_535, i32::MAX];
        let v = |m: &mut Mutator, plain: i32| {
            if m.below(6) == 0 {
                edge[m.below(edge.len())]
            } else {
                m.below(plain as usize) as i32
            }
        };
        let ev = match m.below(12) {
            0..=1 => (EV_ABS, SLOT, v(m, 12).wrapping_sub(1)),
            2 => (EV_ABS, TID, if m.below(3) == 0 { -1 } else { v(m, 50) }),
            3..=5 => (EV_ABS, X, v(m, 2100)),
            6..=7 => (EV_ABS, Y, v(m, 2100)),
            8..=9 => (EV_SYN, SYN_REPORT, 0),
            10 => {
                if m.below(4) == 0 {
                    (EV_SYN, SYN_DROPPED, 0)
                } else {
                    (EV_KEY, BTN_TOUCH, m.below(2) as i32)
                }
            }
            _ => (m.below(32) as u16, m.below(64) as u16, v(m, 100)),
        };
        out.extend(event(t, ev.0, ev.1, ev.2));
    }
    out
}

#[test]
fn any_touch_traffic_gives_finite_events_on_the_screen() {
    let mut m = Mutator::new(9);
    let mut events = 0usize;
    for case in 0..2_000 {
        let mut reads = VecDeque::new();
        for k in 0..1 + m.below(6) {
            let mut chunk = traffic(&mut m, (case * 10 + k) as u64 * 1_000_000);
            // Now and then a read that stops mid-event, or junk bytes.
            if m.below(10) == 0 && !chunk.is_empty() {
                chunk.truncate(m.below(chunk.len()));
            }
            if m.below(20) == 0 {
                chunk = m.mutate(&chunk);
            }
            reads.push_back(chunk);
        }
        let fake = Fake {
            reads,
            m: Mutator::new(case as u64),
        };
        let mut r = TouchReader::new(fake, &MPC_ONE, W, H).unwrap();
        let mut out = Vec::new();
        for _ in 0..4 {
            r.read_into(&mut out).unwrap();
        }
        for e in &out {
            assert!(e.x.is_finite() && e.y.is_finite(), "case {case}: {e:?}");
            assert!(
                (0.0..=W as f32).contains(&e.x) && (0.0..=H as f32).contains(&e.y),
                "case {case}: off the screen: {e:?}"
            );
        }
        events += out.len();
    }
    assert!(events > 1_000, "only {events} pointer events");
}
