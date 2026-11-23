//! Where the pad is.

use crate::input::unit;
use crate::layout::Lift;

/// The pad's place: under the finger, gliding back to where it rests after
/// a lift ([`Lift::GoBack`]), or staying ([`Lift::Stay`]). No values here:
/// a [`crate::Compiled`] layout turns the place into its settings' values.
#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    rest: (f32, f32),
    lift: Lift,
    pos: (f32, f32),
    down: bool,
}

impl Pad {
    /// A pad resting at `rest` (0 to 1 each way), doing `lift` when the
    /// finger lifts.
    pub fn new(rest: (f32, f32), lift: Lift) -> Pad {
        let rest = (unit(rest.0), unit(rest.1));
        Pad {
            rest,
            lift,
            pos: rest,
            down: false,
        }
    }

    /// A finger goes down or moves, at `x` and `y` from 0 to 1 (left to
    /// right, bottom to top).
    pub fn touch(&mut self, x: f32, y: f32) {
        self.down = true;
        self.pos = (unit(x), unit(y));
    }

    /// The finger lifts: the pad glides back from here, or stays.
    pub fn release(&mut self) {
        self.down = false;
    }

    /// What the pad does when the finger lifts.
    pub fn lift(&self) -> Lift {
        self.lift
    }

    /// Changes what the pad does when the finger lifts; going back with no
    /// finger down glides back from where it is.
    pub fn set_lift(&mut self, lift: Lift) {
        self.lift = lift;
    }

    /// Whether the pad stays where the finger lifts.
    pub fn stays(&self) -> bool {
        self.lift == Lift::Stay
    }

    /// Moves the glide back on by `ms`; true while the pad moved (send the
    /// values again).
    pub fn tick(&mut self, ms: f32) -> bool {
        let Lift::GoBack { snap_ms } = self.lift else {
            return false;
        };
        if self.down || self.at_rest() {
            return false;
        }
        let k = 1.0 - (-ms.max(0.0) / snap_ms.max(1.0)).exp();
        let (x, y) = self.pos;
        let (rx, ry) = self.rest;
        let mut nx = x + (rx - x) * k;
        let mut ny = y + (ry - y) * k;
        if (nx - rx).abs() < 0.01 && (ny - ry).abs() < 0.01 {
            (nx, ny) = (rx, ry);
        }
        self.pos = (nx, ny);
        true
    }

    /// Back where it rests, with no finger on it.
    pub fn at_rest(&self) -> bool {
        !self.down && self.pos == self.rest
    }

    /// Whether a finger is on the pad.
    pub fn touched(&self) -> bool {
        self.down
    }

    /// Where the pad is, 0 to 1 each way.
    pub fn position(&self) -> (f32, f32) {
        self.pos
    }

    /// Where it rests.
    pub fn rest(&self) -> (f32, f32) {
        self.rest
    }
}
