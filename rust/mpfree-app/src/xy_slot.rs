//! The screen's XY pad as one value the sound loop reads every period,
//! beside the queue of the screen's actions (the XY controller plan, phase
//! 1: "a held setting can't stay held").
//!
//! The screen sends the pad's moves and its lift as actions
//! (`PlayFx::Xy`), and a lift that finds the queue full waits for room
//! (gui.rs `Waiting`). This slot is the second guard: the screen writes
//! the pad's state here before it sends anything, and once a period the
//! sound loop reads it ([`crate::mixing::MixHost::screen_xy`]). If the mixer
//! still holds the pad for the screen but the slot says the finger is up,
//! it lets go there and then, even if the lift itself never came.
//!
//! One `AtomicU64`: written whole and read whole, so a read never sees half
//! of a state. No lock, no allocation; the sound loop only loads it.

use std::sync::atomic::{AtomicU64, Ordering};

/// The pad's state: which touch it is, whether the finger is down, and
/// where (0 to 1 each way, in steps of 1/65535).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XyState {
    /// Counts the screen's touches on the pad (wraps).
    pub gesture: u16,
    pub down: bool,
    pub x: f32,
    pub y: f32,
}

impl XyState {
    fn pack(self) -> u64 {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u64;
        q(self.x) | q(self.y) << 16 | u64::from(self.gesture) << 32 | u64::from(self.down) << 48
    }

    fn unpack(w: u64) -> XyState {
        let f = |v: u64| (v & 0xFFFF) as f32 / 65535.0;
        XyState {
            gesture: (w >> 32 & 0xFFFF) as u16,
            down: w >> 48 & 1 == 1,
            x: f(w),
            y: f(w >> 16),
        }
    }
}

/// Shared by the screen (which writes) and the sound loop (which reads).
#[derive(Debug, Default)]
pub struct XySlot(AtomicU64);

impl XySlot {
    pub fn write(&self, s: XyState) {
        self.0.store(s.pack(), Ordering::Release);
    }

    pub fn read(&self) -> XyState {
        XyState::unpack(self.0.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_reads_back_whole() {
        let slot = XySlot::default();
        assert_eq!(slot.read(), XyState::default(), "up, at the corner");
        let s = XyState {
            gesture: 65535,
            down: true,
            x: 0.25,
            y: 1.0,
        };
        slot.write(s);
        let r = slot.read();
        assert_eq!((r.gesture, r.down), (65535, true));
        assert!((r.x - 0.25).abs() < 1e-4 && r.y == 1.0, "{r:?}");
        // Out of range is the edge, never another field.
        slot.write(XyState {
            gesture: 7,
            down: false,
            x: -3.0,
            y: 9.0,
        });
        let r = slot.read();
        assert_eq!((r.gesture, r.down, r.x, r.y), (7, false, 0.0, 1.0));
    }
}
