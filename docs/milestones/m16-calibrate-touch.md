# M16: "Calibrate touch", orientation from four corner taps

**Status:** the logic is built and tested on the workstation (`rust/mpfree-touch/src/corners.rs`). A setup page in the GUI comes after PR #31 merges. Nothing here touches the device, and nothing writes a settings file yet.

## Why

M12 measured how one MPC One's touch panel is mounted (`swap_xy=true, flip_x=true, flip_y=false`). Another unit or a replacement panel could be mounted differently, and with the wrong orientation every touch lands in the wrong place. The touchscreen handoff planned to ship M12's orientation check as a setup page, so an owner can fix this without a rebuild.

## How it works

1. The page draws a target in each logical corner in turn: top-left, top-right, bottom-right, bottom-left, as you face the MPC.
2. For each one, the GUI passes in the **raw** position where the finger lifted. It doesn't use the mapped position, because the mapping is the thing being found.
3. A tap within 15% of the middle on either raw axis is asked again (`TooCentral`). So is a tap in the same raw region as an earlier corner (`SameAsEarlier`).
4. After four taps, `result()` tries all eight orientations and keeps the one that puts each tap in its own logical corner. Four labelled corners always pick exactly one orientation, because each orientation is a different symmetry of the rectangle. Corners tapped in the wrong order fit none, so the page starts again.
5. The result says whether it matches the built-in profile. When it doesn't, `Calibration::profile` gives the profile to use.

`orientation_line` and `parse_orientation_line` define one settings line, `touch-orientation: swap_xy=1 flip_x=1 flip_y=0`. A damaged line reads as `None`, so the built-in profile is used instead. Where and when that line is stored is a later decision. On the kexec test build nothing is written; the result is only shown and logged.

## Tests

`tests/m16_corners.rs`:
- every one of the 8 mountings is found from four taps;
- M12's own corner touches, converted back to raw units, give the measured profile;
- central and repeated taps are asked again;
- the wrong order fits nothing;
- settings lines round-trip, and damaged ones are rejected.
