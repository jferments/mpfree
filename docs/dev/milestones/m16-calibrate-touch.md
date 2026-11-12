# M16: "Calibrate touch", orientation from four corner taps

**Status:** the logic (`rust/mpfree-touch/src/corners.rs`) and a page in the pad player (`rust/mpfree-app/src/calibrate_page.rs`) are built and tested on the workstation. The page reports and logs the result only: nothing writes a settings file, and the mapping in use doesn't change.

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

## In the pad player

**Opening it.** Prefs (SHIFT + MENU), the last row **Touch screen**, then **Calibrate touch** (just above Touch lab): turn the wheel to each and press +, or tap its arrows. **Leaving it:** Exit in the middle of the screen, or MENU.

**The page.** A target in one corner at a time, in order: top-left, top-right, bottom-right, bottom-left, as the owner faces the MPC. Each tap counts where the finger lifts. A tap too near the middle, or in a corner that is already done, is asked again with a note. After the fourth tap, the page says one of three things:
- the panel matches the built-in mapping;
- it is mounted differently (the orientation is shown, and logged, but not applied);
- or the taps fit no mapping, usually because a corner was tapped out of order.

**Start again** clears the taps. The buttons sit in the middle of the screen, where no corner tap counts. The middle stays the middle under every orientation, so they stay reachable even when the mapping is wrong.

**Raw readings.** The page only sees screen positions, so it turns each one back into the raw reading through the built-in mapping (`AxisMap::unmap`, exact to within 2 raw units). That gives the calibrator what the panel reported, whatever the panel's real orientation.

**Journal lines** (stdout, as with the Touch lab):

```text
touch-calibrate-open
touch-calibrate-tap: corner=top-left x=70 y=70 raw_x=179 raw_y=1936 outcome=ok
touch-calibrate-tap: corner=top-right x=640 y=60 raw_x=153 raw_y=1024 outcome=too_central
touch-calibrate: result=match
touch-orientation: swap_xy=1 flip_x=1 flip_y=0
touch-calibrate-exit
```

`outcome` is `ok`, `too_central`, `same_as_<corner>` or `ignored`. `result` is `match`, `differs` or `no_fit fits=N`. The `touch-orientation:` line is the settings line format, logged here so a round shows what would be stored.

## The touch screen section in Prefs

Prefs keeps the touch screen's settings in their own section (Jay, 2026-11-11): its last row, **Touch screen**, opens a list with **Back to Prefs**, **Screen pad velocity**, **Calibrate touch** and **Touch lab**. MENU, or the Back row, returns to Prefs' main rows on the Touch screen row. Prefs always opens on its main rows. Future touch settings (gesture thresholds, a stored orientation) go in this section.

## Next

Storing the result (a settings file, which needs a decision on where settings live), and applying a stored orientation when the touchscreen opens.
