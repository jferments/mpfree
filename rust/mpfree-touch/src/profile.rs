//! The measured MPC One touch profile, as Rust constants.
//!
//! The source of truth is `docs/hardware/data/mpc-one-touch-profile.json`
//! (M12, one unit, firmware 3.6.0). The test below parses that file and fails
//! if these constants drift from it, so the GUI never carries its own copy.

use crate::pointer::{AxisMap, MapError, Orientation, RawRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchProfile {
    pub raw_x: RawRange,
    pub raw_y: RawRange,
    /// `ABS_MT_SLOT` maximum + 1.
    pub slots: usize,
    pub orientation: Orientation,
    /// Logical (landscape) screen size the orientation was measured against.
    pub logical: (u32, u32),
}

/// MPC One, ILI2117 on I2C 4-0026, measured by M12 on 2026-11-10.
pub const MPC_ONE: TouchProfile = TouchProfile {
    raw_x: RawRange { min: 0, max: 2048 },
    raw_y: RawRange { min: 0, max: 2048 },
    slots: 10,
    orientation: Orientation {
        swap_xy: true,
        flip_x: true,
        flip_y: false,
    },
    logical: (1280, 800),
};

impl TouchProfile {
    /// Raw device units to logical pixels for a screen of `width`×`height`.
    pub fn axis_map(&self, width: u32, height: u32) -> Result<AxisMap, MapError> {
        AxisMap::new(self.raw_x, self.raw_y, self.orientation, width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = include_str!("../../../docs/hardware/data/mpc-one-touch-profile.json");

    /// The text right after `"key":` (first occurrence), trimmed.
    fn after(key: &str) -> &'static str {
        let pat = format!("\"{key}\":");
        let at = JSON
            .find(&pat)
            .unwrap_or_else(|| panic!("{key} missing from the profile JSON"));
        JSON[at + pat.len()..].trim_start()
    }

    fn pair(key: &str) -> (i64, i64) {
        let text = after(key);
        let body = &text[1..text.find(']').unwrap()];
        let v: Vec<i64> = body.split(',').map(|n| n.trim().parse().unwrap()).collect();
        assert_eq!(v.len(), 2, "{key}");
        (v[0], v[1])
    }

    fn int(key: &str) -> i64 {
        let text = after(key);
        let end = text.find(|c: char| !c.is_ascii_digit()).unwrap();
        text[..end].parse().unwrap()
    }

    fn flag(key: &str) -> bool {
        let text = after(key);
        if text.starts_with("true") {
            true
        } else {
            assert!(text.starts_with("false"), "{key} is not a boolean");
            false
        }
    }

    #[test]
    fn constants_match_the_measured_profile_json() {
        assert_eq!(int("schema_version"), 1);
        let p = MPC_ONE;
        assert_eq!(
            pair("raw_x"),
            (i64::from(p.raw_x.min), i64::from(p.raw_x.max))
        );
        assert_eq!(
            pair("raw_y"),
            (i64::from(p.raw_y.min), i64::from(p.raw_y.max))
        );
        assert_eq!(int("slots"), p.slots as i64);
        assert_eq!(
            pair("logical"),
            (i64::from(p.logical.0), i64::from(p.logical.1))
        );
        assert_eq!(flag("swap_xy"), p.orientation.swap_xy);
        assert_eq!(flag("flip_x"), p.orientation.flip_x);
        assert_eq!(flag("flip_y"), p.orientation.flip_y);
    }

    #[test]
    fn the_profile_maps_the_corners_the_owner_touched() {
        // M12's corners, in logical pixels, were derived from raw readings by
        // this same orientation, so raw extremes must land in those corners.
        let m = MPC_ONE.axis_map(1280, 800).unwrap();
        let (x, y) = m.map(0, 2048); // raw X low = top, raw Y high = left
        assert!(x < 1.0 && y < 1.0, "({x}, {y})");
        let (x, y) = m.map(2048, 0);
        assert!(x > 1278.0 && y > 798.0, "({x}, {y})");
    }
}
