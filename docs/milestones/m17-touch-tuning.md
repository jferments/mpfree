# M17: the Dev tab's Touch page, measuring and tuning touch

**Status:** built and tested on the workstation (`rust/mpfree-touch/src/tuning.rs`, `rust/mpfree-hello/src/touch_dev.rs`). Not yet tried on the MPC One.

## Why

M14 and M15 left the gesture thresholds as untuned starting points, and M16 only reports whether the panel's orientation matches. The owner asked for a graphical touch test under the developer tools, and tools that tune touch from what it measures.

## Opening it

Prefs (SHIFT + MENU) > **Show developer tools** on, then the **Dev** tab and **Touch** in its strip (between Map controls and Console). SHIFT + wheel also walks the Dev pages. MENU on the page offers the full-screen **Calibrate touch** (M16's corner check) and the **Touch lab** (M15).

## The page

**Left: the test area.** Every finger is drawn where the app sees it, after the tuning, with its trail. Its label gives that position, the position before the tuning, and the panel's raw reading. The bottom lines name the gestures the current thresholds make of each touch, count what was measured, and show the last lift. The bar on top has four entries:
- **Free:** touch anything. Taps, holds and strokes are measured.
- **Targets:** the area takes the whole page and lights nine rings in turn, three rows of three. Tap each centre as you would a button. A tap more than 110 px from the ring is not counted. After the ninth, the page comes back with the suggestions.
- **Swipes:** strokes made now count as swipes, for the swipe thresholds. Swipe as you would to turn a page, a few times.
- **Clear:** forget every measurement.

Every finger is measured **before** the tuning, so a tuning never hides what it is meant to fix.

**Right: the tuning list.** Each value now, what the measurements suggest (blue when it differs), and below the list what the picked row does and why it is suggested. The wheel picks a row. - / + or the row's own buttons change it, and the change applies at once. The three buttons, also reachable with the wheel and +:
- **Use suggested** takes every suggestion.
- **Defaults** goes back to the built-in values, which move no touch.
- **Save** keeps the values in the store (the `touch` settings, on the stick's data partition), read at the next start. Without a data folder the values stay until the power goes off, and a line says so.

## The values

| Value | What it does | Bounds | Suggested from |
|---|---|---|---|
| Shift across / down | Moves every touch | ±40 px | A line per axis fitted to the nine target taps |
| Stretch across / down | Spreads touches from the middle | 90-110 % | The same fit |
| Hold filter | Drops a held finger's moves smaller than this, and lifts it where it last was | 0-6 px | 9 in 10 holds' largest wander, rounded up (0 under 1 px) |
| Tap slop | How far a tap may move | 4-40 px | 19 in 20 taps' travel × 1.5 + 2, and at least twice the hold wander + 2 |
| Longest tap | Longer is not a tap | 100-1000 ms | 19 in 20 taps' length + 100 ms |
| Long press | Holding still this long | 250-1500 ms | 19 in 20 taps' length + 250 ms, at least 400 ms |
| Swipe distance / speed | A swipe's least length and lift speed | 30-400 px, 150-3000 px/s | Half the middle swipe's length and speed |

A value changes only with enough samples behind it: 5 target taps, 3 holds (still for 0.4 s or more), 8 taps, 4 swipes. The reason line says how many more are needed.

**Where they apply.** The shift, stretch and hold filter apply to every touch in the app: the screen thread passes each finger through `tuning::Filter` before anything sees it. The bounds keep every control reachable whatever is set, and Defaults (wheel to it, then +) works without the screen. The gesture thresholds apply to the gesture recognisers (this page and the Touch lab) and are kept in the store for widgets that adopt them.

## Journal lines

- `touch-tune-open`, `touch-tune-exit`, `touch-tune-mode: free|targets|swipes`, `touch-tune-clear`
- `touch-measure: tap ms=… travel_px=…`, `touch-measure: hold ms=… reports=… rate_hz=… std_px=…,… max_dev_px=…`, `touch-measure: stroke px=… speed=… ratio=… asked=swipe|free`
- `touch-target: n=… target=x,y at=x,y miss_px=…`
- `touch-tune-summary: taps=… holds=… strokes=… swipes=… rate_hz=… targets=… target_miss_px=mean/worst tuned_miss_px=mean/worst`
- `touch-tune: loaded|use-suggested|save …` with every value, `touch-tune: set key=value`, `touch-tune: defaults`

## Tests

- `mpfree-touch` `tuning::tests` (12): settings round trip and bounds, steps, the filter (identity by default, correction, hold filter), what a lift becomes, release speed, no suggestion without samples, a shifted and stretched panel fitted back to within 1.5 px, and thresholds from taps, holds and swipes.
- `mpfree-hello` `touch_dev::tests` (4) and `gui::tests::the_touch_page_tunes_every_touch_and_saves_on_request`: targets, swipes, the recogniser under new thresholds, the list's buttons, and the page end to end (step, save, Defaults by touch, Targets taking the whole page and coming back).
- Previews: `debug-touch.png`, `debug-touch-targets.png`.
