//! One shape for every controller's input.

/// The controller an input comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// The touchscreen.
    Screen,
    /// A learned MIDI control (Play · XY across and Play · XY up).
    Midi,
}

/// Where in its gesture an input is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// A finger landed (or a control left where the pad rests).
    Begin,
    Move,
    /// The finger lifted (or the control came back to rest).
    End,
    /// The gesture was taken away (a touch the screen cancelled, a
    /// controller unplugged): let go, as for End.
    Cancel,
}

/// One input from a controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XyInput {
    pub device: Device,
    /// Which touch on that device (a finger's id; 0 for a controller with
    /// one).
    pub touch: u64,
    /// Counts the device's inputs, so one that arrives out of order is
    /// known ([`crate::Gesture`] drops it).
    pub seq: u32,
    /// When it happened, in microseconds on the MPC's clock: the touch
    /// layer's kernel time for the screen, the arrival time for MIDI. For
    /// placing a recorded move at its own time within a period.
    pub at: u64,
    /// Across, 0 (left) to 1 (right).
    pub x: f32,
    /// Up, 0 (bottom) to 1 (top).
    pub y: f32,
    /// How hard, 0 to 1, when the controller knows.
    pub pressure: Option<f32>,
    pub phase: Phase,
}

impl XyInput {
    /// An input at `x`, `y` (each clamped to 0 to 1) with no pressure.
    pub fn new(
        device: Device,
        touch: u64,
        seq: u32,
        at: u64,
        phase: Phase,
        x: f32,
        y: f32,
    ) -> XyInput {
        XyInput {
            device,
            touch,
            seq,
            at,
            x: unit(x),
            y: unit(y),
            pressure: None,
            phase,
        }
    }
}

/// `v` clamped to 0 to 1 (not a number: 0).
pub(crate) fn unit(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}
