# M14: swipes, release velocity, fling and hold jitter

**Status:** built and tested on the workstation and in CI with synthetic input at the panel's measured rate (~113 Hz). Thresholds are starting points and haven't been tuned on the device yet. Nothing here touches the device.

## What it adds to `mpfree-touch`

- **Release velocity.** `Gesture::DragEnd` now carries `vx`, `vy` in px/s, measured over the moves in the last 80 ms before the lift. The panel sends nothing while a finger is still (M12), so a finger that pauses longer than that before lifting has zero velocity. That's what makes "drag, stop, lift" not fling.
- **Swipes.** `Gesture::Swipe { direction, speed }` follows a `DragEnd` when the stroke travelled at least 80 px, was moving at least 500 px/s at release, and its main axis dominated by 2:1 in both distance and velocity. A widget that only scrolls can ignore it. Page switching and dismissing use it.
- **`Fling`** for kinetic scrolling: content keeps the release velocity and slows exponentially (half-life 0.25 s), stopping below 20 px/s. `remaining()` says where it will stop, so a list can snap to a row.
- **`jitter::hold_stats`** summarises one held finger: report count and rate, per-axis standard deviation, and the largest distance from the median. The next device round uses it to decide on smoothing.

## How the GUI should use it

One `GestureRecognizer` per widget that wants gestures, fed only the fingers that widget owns. `route::GestureRouter` does this: a widget registers with its own `GestureConfig`, the GUI tags each pointer event with its owning widget, and the router feeds each owner's recognizer and ticks them all (`mpfree-ui` already gives each finger to the widget it first touched). Then a pad press and a fader drag in different places can never pair up into a pinch, and plain widgets (pads, buttons, faders) keep using raw down/move/up with no gesture delay. Long press needs `tick(now)` from the UI loop, because a still finger sends no events.

Planned wiring, after PR #31 (which carries `mpfree-ui` and `mpfree-hello`) and M13 merge:

1. `TouchIn` in `mpfree-hello` switches to `mpfree_touch_evdev::EvdevTouch`.
2. `mpfree-ui` gets an optional per-widget gesture hook (`Widget::gesture`) fed by a `GestureRouter` the `Ui` keeps, keyed by widget, with ticks from the UI loop.
3. First users: swipe left and right between pages, a drag-scroll list with fling (kit list), and long press for a context action on a pad. The GUI's Debug page and the workstation log record every gesture decision with its numbers (distance, velocity, hold time).

## Next device round (planned, needs Jay's go)

This runs on the kexec RAM-only test build, where MPFree owns the screen, as owner-paced steps (step shown, Enter, 3-2-1, a few seconds, STOP, counts shown, redo offer). Each step is wrapped in `results.py run`, and the round ends in one zip.

1. Reader check: `ReaderStats` at open (slot, fingers already down), touch count, no fallbacks.
2. Taps on small and large targets, long press, and a drag that stops before lifting.
3. Swipes in all four directions, slow and fast. The log shows which were swipes and why the others weren't.
4. Scroll and fling the kit list.
5. Holds: 10 s still at the centre and the four corners, for `hold_stats`.
6. A pad held while another finger swipes, to confirm they stay independent.

Jay's feedback plus the logged numbers then set the slop, long-press time and swipe thresholds.
