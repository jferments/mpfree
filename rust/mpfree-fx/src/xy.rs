//! The XY pad's logic: a finger on the touchscreen plays two parameters at
//! once, and when it lifts the pad glides back to where it rests (or stays,
//! with Hold on). No audio here: the control thread calls this as touches
//! and frames come, and sends the values it gives to the node as parameter
//! events, the way a Q-Link turn goes.
//!
//! [`XyPad::perform`] is the Perform FX page's pad: across is the DJ filter
//! (low-pass to the left, high-pass to the right, off in the middle, where
//! it rests) and up is the echo (none at the bottom, where it rests).

use crate::perform::ids;

/// One direction of the pad: the parameter it plays, the values at the
/// pad's two edges, and where it rests (0 to 1 across the pad).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XyAxis {
    pub param: u16,
    pub name: &'static str,
    /// The value at position 0 (left or bottom) and at position 1.
    pub from: f32,
    pub to: f32,
    pub rest: f32,
}

impl XyAxis {
    /// The parameter's value at a position.
    pub fn value(&self, pos: f32) -> f32 {
        self.from + (self.to - self.from) * pos.clamp(0.0, 1.0)
    }
    /// The position of a value (for showing where the parameter is).
    pub fn position(&self, value: f32) -> f32 {
        if self.to == self.from {
            return self.rest;
        }
        ((value - self.from) / (self.to - self.from)).clamp(0.0, 1.0)
    }
}

/// An XY pad: where the finger is, or where the pad is gliding back to.
#[derive(Clone, Debug)]
pub struct XyPad {
    pub x: XyAxis,
    pub y: XyAxis,
    /// With Hold on, lifting the finger leaves the pad where it was.
    pub hold: bool,
    /// How fast it glides back: about this many ms to cover 63 % of the
    /// way (it lands within a hundredth in five times this).
    pub snap_ms: f32,
    pos: (f32, f32),
    down: bool,
}

impl XyPad {
    /// A pad on two parameters, resting where they rest.
    pub fn new(x: XyAxis, y: XyAxis) -> XyPad {
        XyPad {
            x,
            y,
            hold: false,
            snap_ms: 60.0,
            pos: (x.rest, y.rest),
            down: false,
        }
    }

    /// The Perform FX pad: the DJ filter across, the echo up.
    pub fn perform() -> XyPad {
        XyPad::new(
            XyAxis {
                param: ids::DJ_FILTER,
                name: "DJ filter",
                from: -100.0,
                to: 100.0,
                rest: 0.5,
            },
            XyAxis {
                param: ids::ECHO,
                name: "Echo",
                from: 0.0,
                to: 100.0,
                rest: 0.0,
            },
        )
    }

    /// A finger goes down or moves, at `x` and `y` from 0 to 1 (left to
    /// right, bottom to top).
    pub fn touch(&mut self, x: f32, y: f32) {
        self.down = true;
        self.pos = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    }

    /// The finger lifts: the pad glides back from here unless Hold is on.
    pub fn release(&mut self) {
        self.down = false;
    }

    /// Turns Hold on or off; off with no finger down glides back.
    pub fn set_hold(&mut self, on: bool) {
        self.hold = on;
    }

    /// Moves the glide on by `ms`; true while the pad moved (send the
    /// values again).
    pub fn tick(&mut self, ms: f32) -> bool {
        if self.down || self.hold || self.at_rest() {
            return false;
        }
        let k = 1.0 - (-ms.max(0.0) / self.snap_ms.max(1.0)).exp();
        let (x, y) = self.pos;
        let (rx, ry) = (self.x.rest, self.y.rest);
        let mut nx = x + (rx - x) * k;
        let mut ny = y + (ry - y) * k;
        if (nx - rx).abs() < 0.01 && (ny - ry).abs() < 0.01 {
            (nx, ny) = (rx, ry);
        }
        self.pos = (nx, ny);
        true
    }

    /// Back where it rests (and no finger on it).
    pub fn at_rest(&self) -> bool {
        !self.down && self.pos == (self.x.rest, self.y.rest)
    }

    /// Whether a finger is on the pad.
    pub fn touched(&self) -> bool {
        self.down
    }

    /// Where the pad is, 0 to 1 each way.
    pub fn position(&self) -> (f32, f32) {
        self.pos
    }

    /// The two parameter values to send, `(id, value)`.
    pub fn values(&self) -> [(u16, f32); 2] {
        [
            (self.x.param, self.x.value(self.pos.0)),
            (self.y.param, self.y.value(self.pos.1)),
        ]
    }
}
