//! M16: "Calibrate touch", the four-corner setup step.
//!
//! M12 measured one unit's panel orientation with a workstation analyzer. A
//! product can't assume every MPC One's panel is mounted the same way, so the
//! setup screen asks the owner to tap four corners in order (top-left,
//! top-right, bottom-right, bottom-left, as they face the MPC) and works out
//! the orientation from the raw readings alone.
//!
//! Four labelled corners pick exactly one of the eight orientations: each
//! orientation is a symmetry of the rectangle and sends the four raw corners
//! to a different arrangement. So no drags are needed, only that each tap
//! lands in its own corner region.
//!
//! The calibrator works on **raw** contact positions (`Contact::raw_x/raw_y`
//! from the decoder), never on mapped ones, because the mapping is what it is
//! finding. The GUI draws the target in the logical corner and passes in the
//! raw position where the finger lifted.

use crate::pointer::{AxisMap, Orientation, RawRange};
use crate::profile::TouchProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    pub const ORDER: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft];

    /// (right half, bottom half) in logical space.
    fn quadrant(self) -> (bool, bool) {
        match self {
            Corner::TopLeft => (false, false),
            Corner::TopRight => (true, false),
            Corner::BottomRight => (true, true),
            Corner::BottomLeft => (false, true),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "top-left",
            Corner::TopRight => "top-right",
            Corner::BottomRight => "bottom-right",
            Corner::BottomLeft => "bottom-left",
        }
    }
}

/// What a tap did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapOutcome {
    /// Recorded; ask for this corner next.
    Next(Corner),
    /// All four recorded: call [`CornerCalibrator::result`].
    Done,
    /// Too close to the middle of the screen on some axis; ask again.
    TooCentral,
    /// In the same raw region as an earlier corner; ask again.
    SameAsEarlier(Corner),
    /// Already done; the tap was ignored.
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibration {
    pub orientation: Orientation,
    /// The four taps in logical pixels, in [`Corner::ORDER`].
    pub corners: [(f32, f32); 4],
    /// True when it matches the profile the calibrator started from.
    pub matches_profile: bool,
}

impl Calibration {
    /// The starting profile with this orientation.
    pub fn profile(&self, base: &TouchProfile) -> TouchProfile {
        TouchProfile { orientation: self.orientation, ..*base }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationError {
    /// Fewer than four corners recorded.
    Incomplete,
    /// The four taps fit no orientation (or, impossibly, several). Usually a
    /// tap in the wrong corner: start again.
    NoFit(usize),
}

#[derive(Debug, Clone)]
pub struct CornerCalibrator {
    base: TouchProfile,
    taps: Vec<(i32, i32)>,
}

/// A tap must be at least this far from the middle on both raw axes, as a
/// fraction of the axis (0.5 would be the very edge).
const MIN_OFF_CENTRE: f64 = 0.15;

impl CornerCalibrator {
    /// `base` gives the raw ranges and logical size; its orientation is only
    /// used to report whether the result matches.
    pub fn new(base: TouchProfile) -> Self {
        Self { base, taps: Vec::with_capacity(4) }
    }

    /// The corner to ask for now, or `None` when all four are in.
    pub fn next_corner(&self) -> Option<Corner> {
        Corner::ORDER.get(self.taps.len()).copied()
    }

    pub fn reset(&mut self) {
        self.taps.clear();
    }

    /// Record the raw position where a finger lifted.
    pub fn tap(&mut self, raw_x: i32, raw_y: i32) -> TapOutcome {
        if self.next_corner().is_none() {
            return TapOutcome::Ignored;
        }
        let (u, v) = (norm(raw_x, self.base.raw_x), norm(raw_y, self.base.raw_y));
        if (u - 0.5).abs() < MIN_OFF_CENTRE || (v - 0.5).abs() < MIN_OFF_CENTRE {
            return TapOutcome::TooCentral;
        }
        let region = (u > 0.5, v > 0.5);
        for (i, &(x, y)) in self.taps.iter().enumerate() {
            if (norm(x, self.base.raw_x) > 0.5, norm(y, self.base.raw_y) > 0.5) == region {
                return TapOutcome::SameAsEarlier(Corner::ORDER[i]);
            }
        }
        self.taps.push((raw_x, raw_y));
        match self.next_corner() {
            Some(c) => TapOutcome::Next(c),
            None => TapOutcome::Done,
        }
    }

    pub fn result(&self) -> Result<Calibration, CalibrationError> {
        if self.taps.len() < 4 {
            return Err(CalibrationError::Incomplete);
        }
        let (w, h) = self.base.logical;
        let fitting: Vec<(Orientation, AxisMap)> = Orientation::ALL
            .iter()
            .filter_map(|&o| {
                let map = AxisMap::new(self.base.raw_x, self.base.raw_y, o, w, h).ok()?;
                let ok = self.taps.iter().zip(Corner::ORDER).all(|(&(x, y), c)| {
                    let (lx, ly) = map.map(x, y);
                    (lx > w as f32 / 2.0, ly > h as f32 / 2.0) == c.quadrant()
                });
                ok.then_some((o, map))
            })
            .collect();
        match fitting.as_slice() {
            [(orientation, map)] => {
                let mut corners = [(0.0, 0.0); 4];
                for (slot, &(x, y)) in corners.iter_mut().zip(&self.taps) {
                    *slot = map.map(x, y);
                }
                Ok(Calibration {
                    orientation: *orientation,
                    corners,
                    matches_profile: *orientation == self.base.orientation,
                })
            }
            other => Err(CalibrationError::NoFit(other.len())),
        }
    }
}

fn norm(v: i32, r: RawRange) -> f64 {
    ((f64::from(v) - f64::from(r.min)) / (f64::from(r.max) - f64::from(r.min))).clamp(0.0, 1.0)
}

/// One settings line for a stored orientation, e.g.
/// `touch-orientation: swap_xy=1 flip_x=1 flip_y=0`.
pub fn orientation_line(o: Orientation) -> String {
    format!(
        "touch-orientation: swap_xy={} flip_x={} flip_y={}",
        u8::from(o.swap_xy),
        u8::from(o.flip_x),
        u8::from(o.flip_y)
    )
}

/// Read [`orientation_line`] back. Anything malformed is `None`, so a damaged
/// settings file falls back to the built-in profile.
pub fn parse_orientation_line(line: &str) -> Option<Orientation> {
    let rest = line.trim().strip_prefix("touch-orientation:")?;
    let mut o = (None, None, None);
    for kv in rest.split_whitespace() {
        let (k, v) = kv.split_once('=')?;
        let b = match v {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let slot = match k {
            "swap_xy" => &mut o.0,
            "flip_x" => &mut o.1,
            "flip_y" => &mut o.2,
            _ => return None,
        };
        if slot.replace(b).is_some() {
            return None;
        }
    }
    Some(Orientation { swap_xy: o.0?, flip_x: o.1?, flip_y: o.2? })
}
