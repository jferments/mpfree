//! M16: four-corner touch calibration, for all eight panel mountings.

use mpfree_touch::corners::{
    orientation_line, parse_orientation_line, CalibrationError, Corner, CornerCalibrator,
    TapOutcome,
};
use mpfree_touch::pointer::{AxisMap, Orientation};
use mpfree_touch::profile::MPC_ONE;

/// Raw points near the four raw corners (10% in from each edge).
const RAW_CORNERS: [(i32, i32); 4] = [(205, 205), (1843, 205), (1843, 1843), (205, 1843)];

/// The raw point an owner would touch for `corner` if the panel were mounted
/// as `o`.
fn raw_for(o: Orientation, corner: Corner) -> (i32, i32) {
    let map = AxisMap::new(MPC_ONE.raw_x, MPC_ONE.raw_y, o, 1280, 800).unwrap();
    let want = match corner {
        Corner::TopLeft => (false, false),
        Corner::TopRight => (true, false),
        Corner::BottomRight => (true, true),
        Corner::BottomLeft => (false, true),
    };
    *RAW_CORNERS
        .iter()
        .find(|&&(x, y)| {
            let (lx, ly) = map.map(x, y);
            (lx > 640.0, ly > 400.0) == want
        })
        .unwrap()
}

#[test]
fn every_mounting_is_found_from_four_taps() {
    for o in Orientation::ALL {
        let mut c = CornerCalibrator::new(MPC_ONE);
        assert_eq!(c.next_corner(), Some(Corner::TopLeft));
        let mut last = TapOutcome::Ignored;
        for corner in Corner::ORDER {
            let (x, y) = raw_for(o, corner);
            last = c.tap(x, y);
        }
        assert_eq!(last, TapOutcome::Done, "{o:?}");
        let cal = c.result().unwrap();
        assert_eq!(cal.orientation, o);
        assert_eq!(cal.matches_profile, o == MPC_ONE.orientation);
        assert_eq!(cal.profile(&MPC_ONE).orientation, o);
        // The taps land in their logical corners.
        let (tl, br) = (cal.corners[0], cal.corners[2]);
        assert!(
            tl.0 < 300.0 && tl.1 < 200.0 && br.0 > 980.0 && br.1 > 600.0,
            "{o:?} {cal:?}"
        );
    }
}

#[test]
fn the_measured_unit_calibrates_to_its_profile() {
    // M12's corner touches (corners_logical in the profile JSON) converted back
    // to raw units with the measured orientation: raw X runs top to bottom and
    // raw Y right to left, so top-left is low X, high Y.
    let mut c = CornerCalibrator::new(MPC_ONE);
    for (x, y) in [(133, 1974), (179, 107), (1971, 80), (1956, 1965)] {
        c.tap(x, y);
    }
    let cal = c.result().unwrap();
    assert!(cal.matches_profile, "{cal:?}");
}

#[test]
fn central_and_repeated_taps_are_asked_again() {
    let mut c = CornerCalibrator::new(MPC_ONE);
    assert_eq!(c.tap(1024, 1024), TapOutcome::TooCentral);
    assert_eq!(c.tap(1100, 200), TapOutcome::TooCentral); // central on X only
    let tl = raw_for(MPC_ONE.orientation, Corner::TopLeft);
    assert_eq!(c.tap(tl.0, tl.1), TapOutcome::Next(Corner::TopRight));
    assert_eq!(
        c.tap(tl.0 + 20, tl.1 - 20),
        TapOutcome::SameAsEarlier(Corner::TopLeft)
    );
    assert_eq!(c.next_corner(), Some(Corner::TopRight));
    assert_eq!(c.result(), Err(CalibrationError::Incomplete));
}

#[test]
fn corners_in_the_wrong_order_fit_nothing() {
    let o = MPC_ONE.orientation;
    let mut c = CornerCalibrator::new(MPC_ONE);
    for corner in [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomLeft,
        Corner::BottomRight,
    ] {
        let (x, y) = raw_for(o, corner);
        c.tap(x, y);
    }
    assert_eq!(c.result(), Err(CalibrationError::NoFit(0)));
    assert_eq!(c.tap(1, 1), TapOutcome::Ignored);
    c.reset();
    assert_eq!(c.next_corner(), Some(Corner::TopLeft));
}

#[test]
fn orientation_lines_round_trip_and_reject_damage() {
    for o in Orientation::ALL {
        assert_eq!(parse_orientation_line(&orientation_line(o)), Some(o));
    }
    assert_eq!(
        orientation_line(MPC_ONE.orientation),
        "touch-orientation: swap_xy=1 flip_x=1 flip_y=0"
    );
    for bad in [
        "touch-orientation: swap_xy=1 flip_x=1",
        "touch-orientation: swap_xy=1 flip_x=1 flip_y=2",
        "touch-orientation: swap_xy=1 flip_x=1 flip_y=0 flip_y=1",
        "touch-orientation: swap_xy=1 flip_x=1 flip_z=0",
        "orientation: swap_xy=1 flip_x=1 flip_y=0",
        "",
    ] {
        assert_eq!(parse_orientation_line(bad), None, "{bad}");
    }
}
