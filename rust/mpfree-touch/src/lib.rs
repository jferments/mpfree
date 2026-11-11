//! Safe Linux evdev metadata representation. No system calls or device I/O.
//! The narrowly audited Linux ioctl boundary resides exclusively in main.rs.
//!
//! M11 adds three pure, host-only layers over already-read events:
//! [`mt`] (Type-B multitouch decoder), [`pointer`] (raw-to-logical mapping)
//! and [`gesture`] (toolkit-neutral recognizer). M12 adds [`calibrate`],
//! host-side analysis of a recorded capture. M13 adds [`profile`], the measured
//! MPC One profile as constants checked against its JSON. M14 adds swipes,
//! release velocity and [`gesture::Fling`], [`route`] (a recognizer per
//! on-screen owner) and [`jitter`] for hold analysis. M15 adds [`touchlab`],
//! the owner-paced Touch lab steps and their journal lines. M16 adds
//! [`corners`], the four-corner "calibrate touch" setup step. None of them read
//! devices; the live reader is the separate `mpfree-touch-evdev` crate.
#![forbid(unsafe_code)]

pub mod calibrate;
pub mod corners;
pub mod gesture;
pub mod jitter;
pub mod mt;
pub mod pointer;
pub mod profile;
pub mod route;
pub mod touchlab;

pub const ABS_X: u8 = 0;
pub const ABS_Y: u8 = 1;
pub const ABS_MT_SLOT: u8 = 47;
pub const ABS_MT_POSITION_X: u8 = 53;
pub const ABS_MT_POSITION_Y: u8 = 54;
pub const ABS_MT_TRACKING_ID: u8 = 57;

/// An input_absinfo without its *current value*, intentionally omitted to
/// avoid sampling/reporting live touch position. Values represent capability
/// bounds and noise/resolution metadata only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisInfo {
    pub minimum: i32,
    pub maximum: i32,
    pub fuzz: i32,
    pub flat: i32,
    pub resolution: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataError {
    InvalidRange,
    InvalidNoise,
    InvalidSlotCount,
}

impl AxisInfo {
    pub fn check(self) -> Result<Self, MetadataError> {
        if self.maximum < self.minimum {
            return Err(MetadataError::InvalidRange);
        }
        if self.fuzz < 0 || self.flat < 0 || self.resolution < 0 {
            return Err(MetadataError::InvalidNoise);
        }
        Ok(self)
    }

    pub fn slot_count(self) -> Result<u32, MetadataError> {
        self.check()?;
        // Inclusive Linux ABS_MT_SLOT indices; actual slot count may differ
        // between factory/driver versions and must be directly measured.
        if self.minimum != 0 || self.maximum > 63 {
            return Err(MetadataError::InvalidSlotCount);
        }
        u32::try_from(i64::from(self.maximum) + 1).map_err(|_| MetadataError::InvalidSlotCount)
    }
}

/// Synthetic-only validation used in unit tests and the self-test binary.
/// These values are *not* values observed on the owner's MPC.
pub fn validate_fixture() -> Result<u32, MetadataError> {
    let synthetic_slot = AxisInfo {
        minimum: 0,
        maximum: 9,
        fuzz: 0,
        flat: 0,
        resolution: 0,
    };
    synthetic_slot.slot_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_linux_evdev_axis_ids() {
        assert_eq!(
            [
                ABS_X,
                ABS_Y,
                ABS_MT_SLOT,
                ABS_MT_POSITION_X,
                ABS_MT_POSITION_Y,
                ABS_MT_TRACKING_ID
            ],
            [0, 1, 47, 53, 54, 57]
        );
    }

    #[test]
    fn inclusive_slot_count_is_not_derived_from_pixel_dimensions() {
        assert_eq!(validate_fixture(), Ok(10));
        let single = AxisInfo {
            minimum: 0,
            maximum: 0,
            fuzz: 0,
            flat: 0,
            resolution: 0,
        };
        assert_eq!(single.slot_count(), Ok(1));
        let bad = AxisInfo {
            minimum: 1,
            maximum: 9,
            ..single
        };
        assert_eq!(bad.slot_count(), Err(MetadataError::InvalidSlotCount));
        let unbounded = AxisInfo {
            maximum: i32::MAX,
            ..single
        };
        assert_eq!(unbounded.slot_count(), Err(MetadataError::InvalidSlotCount));
    }

    #[test]
    fn reject_invalid_raw_ranges_without_touch_events() {
        let zero = AxisInfo {
            minimum: 0,
            maximum: 0,
            fuzz: 0,
            flat: 0,
            resolution: 0,
        };
        assert!(zero.check().is_ok());
        assert_eq!(
            AxisInfo {
                minimum: 30,
                maximum: 5,
                ..zero
            }
            .check(),
            Err(MetadataError::InvalidRange)
        );
        assert_eq!(
            AxisInfo { fuzz: -1, ..zero }.check(),
            Err(MetadataError::InvalidNoise)
        );
    }
}
