//! M11: platform-neutral pointer model.
//!
//! Maps raw decoder contacts into logical GUI coordinates. The physical
//! orientation of the ILI2117 relative to the 1280x800 logical GUI has not
//! been measured yet (that is M12), so every swap/flip combination is
//! expressible and none is assumed. Nothing here depends on a GUI toolkit.

use crate::mt::{ContactEvent, ContactId, Phase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawRange {
    pub min: i32,
    pub max: i32,
}

/// One of the eight axis-aligned orientations, applied in this order:
/// swap raw X/Y, then mirror the resulting X, then mirror the resulting Y.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Orientation {
    pub swap_xy: bool,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl Orientation {
    pub const ALL: [Orientation; 8] = {
        let mut all = [Orientation {
            swap_xy: false,
            flip_x: false,
            flip_y: false,
        }; 8];
        let mut i = 0;
        while i < 8 {
            all[i] = Orientation {
                swap_xy: i & 4 != 0,
                flip_x: i & 2 != 0,
                flip_y: i & 1 != 0,
            };
            i += 1;
        }
        all
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    InvalidRange,
    InvalidSize,
}

/// Raw device axes to logical pixels. Output spans `0.0..=width-1` and
/// `0.0..=height-1`; raw values outside the declared range are clamped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisMap {
    raw_x: RawRange,
    raw_y: RawRange,
    orientation: Orientation,
    width: u32,
    height: u32,
}

impl AxisMap {
    /// `raw_x`/`raw_y` should come from EVIOCGABS metadata (0..=2048 on the
    /// measured MPC One per M10) rather than being hard-coded by callers.
    pub fn new(
        raw_x: RawRange,
        raw_y: RawRange,
        orientation: Orientation,
        width: u32,
        height: u32,
    ) -> Result<Self, MapError> {
        if raw_x.max <= raw_x.min || raw_y.max <= raw_y.min {
            return Err(MapError::InvalidRange);
        }
        if width < 2 || height < 2 {
            return Err(MapError::InvalidSize);
        }
        Ok(Self {
            raw_x,
            raw_y,
            orientation,
            width,
            height,
        })
    }

    pub fn map(&self, raw_x: i32, raw_y: i32) -> (f32, f32) {
        fn norm(v: i32, r: RawRange) -> f64 {
            let t = (f64::from(v) - f64::from(r.min)) / (f64::from(r.max) - f64::from(r.min));
            t.clamp(0.0, 1.0)
        }
        let (mut u, mut v) = (norm(raw_x, self.raw_x), norm(raw_y, self.raw_y));
        if self.orientation.swap_xy {
            std::mem::swap(&mut u, &mut v);
        }
        if self.orientation.flip_x {
            u = 1.0 - u;
        }
        if self.orientation.flip_y {
            v = 1.0 - v;
        }
        (
            (u * f64::from(self.width - 1)) as f32,
            (v * f64::from(self.height - 1)) as f32,
        )
    }

    /// The inverse of [`AxisMap::map`]: the raw reading that lands on logical
    /// pixel (`x`, `y`), to within one raw unit. A page that only sees mapped
    /// positions uses it to get the raw ones back (the corner calibrator needs
    /// raw readings). Positions off the screen are clamped to its edge.
    pub fn unmap(&self, x: f32, y: f32) -> (i32, i32) {
        let mut u = (f64::from(x) / f64::from(self.width - 1)).clamp(0.0, 1.0);
        let mut v = (f64::from(y) / f64::from(self.height - 1)).clamp(0.0, 1.0);
        if self.orientation.flip_y {
            v = 1.0 - v;
        }
        if self.orientation.flip_x {
            u = 1.0 - u;
        }
        if self.orientation.swap_xy {
            std::mem::swap(&mut u, &mut v);
        }
        let raw = |t: f64, r: RawRange| {
            (f64::from(r.min) + t * (f64::from(r.max) - f64::from(r.min))).round() as i32
        };
        (raw(u, self.raw_x), raw(v, self.raw_y))
    }

    pub fn pointer(&self, ev: &ContactEvent) -> PointerEvent {
        let (x, y) = self.map(ev.contact.raw_x, ev.contact.raw_y);
        PointerEvent {
            id: ev.contact.id,
            phase: ev.phase,
            x,
            y,
            time_us: ev.time_us,
        }
    }
}

/// One pointer transition in logical coordinates. Every contact is its own
/// pointer, so independent simultaneous controls (faders, pads, XY areas)
/// can be driven directly from this stream without the gesture layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    pub id: ContactId,
    pub phase: Phase,
    pub x: f32,
    pub y: f32,
    pub time_us: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: RawRange = RawRange { min: 0, max: 2048 };

    #[test]
    fn rejects_degenerate_configuration() {
        let o = Orientation::default();
        assert_eq!(
            AxisMap::new(RawRange { min: 5, max: 5 }, R, o, 1280, 800),
            Err(MapError::InvalidRange)
        );
        assert_eq!(AxisMap::new(R, R, o, 1, 800), Err(MapError::InvalidSize));
    }

    #[test]
    fn eight_distinct_orientations_hit_the_expected_corner() {
        let mut corners = Vec::new();
        for o in Orientation::ALL {
            let m = AxisMap::new(R, R, o, 1280, 800).unwrap();
            // Raw (max, 0): the raw "top-right" corner.
            corners.push((o, m.map(2048, 0)));
        }
        let expect = |o: Orientation| {
            let (mut u, mut v) = (1.0f32, 0.0f32);
            if o.swap_xy {
                std::mem::swap(&mut u, &mut v);
            }
            if o.flip_x {
                u = 1.0 - u;
            }
            if o.flip_y {
                v = 1.0 - v;
            }
            (u * 1279.0, v * 799.0)
        };
        for (o, got) in &corners {
            assert_eq!(*got, expect(*o), "{o:?}");
        }
        for (i, a) in Orientation::ALL.iter().enumerate() {
            assert!(Orientation::ALL[i + 1..].iter().all(|b| b != a));
        }
    }

    #[test]
    fn unmap_undoes_map_in_every_orientation() {
        let r = RawRange { min: 0, max: 2048 };
        for o in Orientation::ALL {
            let m = AxisMap::new(r, r, o, 1280, 800).unwrap();
            for (rx, ry) in [
                (0, 0),
                (2048, 0),
                (0, 2048),
                (2048, 2048),
                (1024, 512),
                (37, 1999),
            ] {
                let (x, y) = m.map(rx, ry);
                let (bx, by) = m.unmap(x, y);
                assert!(
                    (bx - rx).abs() <= 2 && (by - ry).abs() <= 2,
                    "{o:?} {rx},{ry} -> {x},{y} -> {bx},{by}"
                );
            }
        }
    }

    #[test]
    fn clamps_out_of_range_and_maps_centre() {
        let m = AxisMap::new(R, R, Orientation::default(), 1280, 800).unwrap();
        assert_eq!(m.map(-50, 9000), (0.0, 799.0));
        assert_eq!(m.map(1024, 1024), (639.5, 399.5));
    }
}
