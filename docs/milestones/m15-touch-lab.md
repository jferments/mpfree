# M15: Touch lab

**Status:** the steps, the judging and the journal lines are built and tested on the workstation (`rust/mpfree-touch/src/touchlab.rs`), and so is the on-screen page in the pad player (`rust/mpfree-hello/src/touchlab_page.rs`, PR #53). The lab rides in the round after round 5 as one more phase of the app check; the Hello MPC runbook will carry the commands. Nothing here touches the device by itself: the page only reads the touchscreen the pad player already has open (no grab) and prints lines.

## What the lab is for

- **First real-device check of the M13 reader:** whether it was seeded at open, the slot and fingers at open, and any resyncs or fallbacks.
- **Owner feedback on gestures,** with numbers behind every decision, so the thresholds (slop 12 px, long press 500 ms, swipe 80 px at 500 px/s and 2:1, fling half-life 0.25 s) can be tuned from evidence.
- **Jitter,** measured properly: 10 s holds, not M12's 3 s.

It only reads `/dev/input/event0` (no grab) and logs to the journal. The host side reads the journal, so the host check stays read-only.

## Steps

Each step is shown on screen with its instruction. Jay taps **Start** on the screen, a 3-2-1 countdown runs, and then the window opens. When it closes the page shows the result, and **Redo** or **Next** moves on. **Skip** passes over a step without running it.

| Step | Window | Jay does | Passes when |
|---|---|---|---|
| `reader` | 3 s | Doesn't touch the screen | No touches (the reader line is the point) |
| `taps` | 10 s | Taps each of 5 targets once | At least 5 taps, no drags |
| `long_press` | 8 s | Holds the target 2 s, lifts; twice | At least 2 long presses |
| `drag_stop` | 10 s | Drags slowly, stops a second, lifts; twice | At least 2 drags, no swipes |
| `swipes` | 12 s | Swipes quickly left, right, up, down | At least one swipe each way |
| `fling` | 10 s | Flicks the list up and down | At least 2 releases at swipe speed or faster |
| `hold_centre` | 12 s | Holds the centre dot 10 s | One contact held at least 5 s; jitter recorded |
| `hold_corner` | 12 s | Holds the top-left dot 10 s | As above |
| `pad_and_swipe` | 10 s | Holds a screen pad and swipes the page with the other hand | At least one swipe and no two-finger gesture |

A "fail" isn't a device problem. It says the gesture settings and Jay's hand disagreed, and the gesture lines show by how much.

## In the pad player

**Opening it.** Prefs (SHIFT + MENU), then the last row, **Touch lab**: turn the wheel to it and press +, or tap its arrows. Both work without touch, so the lab opens even when the touchscreen seems dead, and its top line then shows why (the reader line, or `touchscreen: not available yet (...)`).

**Leaving it.** The **Exit** button at the top right, or any hardware button that changes the page (MENU closes Prefs, which closes the lab too). A step in progress is closed and logged first. Opening the lab again starts over at the first step.

**The page.** A top bar with the step and the latest reader line (refreshed every 30 s), an instruction bar that turns into the countdown and then `GO (N s)`, and a work area with the step's targets (5 tap circles, the long-press target, the hold dots, the pad in `pad_and_swipe`, list rows in `fling`). Fingers show as green dots, and the last gesture is written at the bottom of the area. After the last step a list shows each step's result.

**What is measured.** Only fingers that land in the work area while a window is open count. A finger already down when a window opens or closes is left out, and so are the buttons. In `pad_and_swipe`, the finger on the pad is recorded but kept out of gesture recognition, as a real pad would be. Gestures and speeds use the kernel's own event times, so a slow frame doesn't distort them. While the lab is open, touches go only to the lab, never to the app.

## Journal lines

All plain ASCII `key=value` pairs separated by spaces, emitted on the app's stdout (journal unit `mpfree-test-app.service`):

```text
touch-reader: seeded=1 slot=3 fingers=0 resyncs=0 fallbacks=0 drained=0 contacts=12 events=480
touch-lab-open: steps=9
touch-lab-step: step=swipes seconds=12
touch-gesture: step=swipes kind=swipe dir=left speed=1180 t_ms=4521
touch-gesture: step=fling kind=drag-end vx=0 vy=-1136 cancelled=0 t_ms=2210
touch-lab: step=swipes result=pass downs=4 taps=0 long=0 drags=4 swipes=1,1,1,1 two_finger=0 flings=4 note=ok
touch-lab: step=hold_centre result=pass downs=1 taps=0 long=1 drags=0 swipes=0,0,0,0 two_finger=0 flings=0 note=ok hold_n=37 hold_hz=3.6 hold_std_x=0.84 hold_std_y=1.10 hold_max_dev=2.40
touch-lab-exit
```

`touch-lab-open` is printed each time the lab opens, `touch-lab-step` when a step's window opens, the `touch-lab` summary when it closes, and `touch-lab-exit` when the lab is left. A step that was skipped has no lines.

`events` counts the raw input events the reader has read. The app logs the reader line when the touchscreen opens, then every 30 s while that count is changing. If Jay touched the screen and `events` stayed at 0, the events are stopping at or below the driver, not in MPFree's code.

Gesture kinds are `tap`, `long`, `drag-start`, `drag-end` (with `vx`, `vy`, `cancelled`), `swipe` (with `dir`, `speed`), `2f-start` and `2f-end`. Positions and speeds are in logical pixels (1280×800). `parse_line` reads any of these lines back, and skips any timestamp or unit prefix in front.

## Next

Jay runs the lab in the next round and reports how each step felt. The thresholds then get tuned from the `touch-gesture` lines, and the `GestureRouter` hook moves into `mpfree-ui` so other pages get gestures the same way.
