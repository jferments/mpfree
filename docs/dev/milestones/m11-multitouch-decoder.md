# M11: host-only multitouch decoder, pointer map and gestures

**Status:** implemented and tested on the workstation and in CI against synthetic event streams. Nothing in M11 reads a device or runs on the MPC.

M10 measured the ILI2117's capabilities (X/Y `0..=2048`, 10 multitouch slots, tracking ids `0..=65535`). M11 builds the software that will consume its events, so that the first live capture (M12) only has to feed real events into code that is already tested.

## Layers

All three live in `rust/mpfree-touch` under `#![forbid(unsafe_code)]`, with no I/O. They are independent of any GUI toolkit.

| Module | Input | Output |
|---|---|---|
| `mt` | `InputEvent { time_us, kind, code, value }` triples, as read from evdev | Per-frame `ContactEvent`s (Down / Move / Up / Cancel) and a snapshot of live contacts |
| `pointer` | `ContactEvent` in raw units | `PointerEvent` in logical pixels, via `AxisMap` (raw ranges + one of 8 orientations + logical size) |
| `gesture` | One frame of `PointerEvent`s | `Gesture`s: Tap, LongPress, DragStart/Drag/DragEnd, TwoFingerStart/TwoFinger/TwoFingerEnd |

Independent simultaneous controls (faders, pads, XY areas) should consume `PointerEvent`s directly: every contact is its own pointer with a stable `ContactId`. The gesture layer is for navigation.

## Decoder rules (`mt`)

- `ABS_MT_SLOT` selects the slot; it persists across frames because the kernel only sends it on change. Slot 0 is selected initially.
- `ABS_MT_TRACKING_ID >= 0` starts a contact, `-1` ends it. A slot switching directly from one id to another produces Up then Down in the same frame.
- Positions persist per slot, across contacts, because the kernel drops repeated values. A new finger at the same X as the previous one only sends Y.
- Nothing is published until `SYN_REPORT`. A contact that starts and ends within one frame is never published.
- `ContactId` is assigned by the decoder and never repeats, so the 16-bit kernel tracking id can wrap safely.
- `BTN_TOUCH`, single-touch `ABS_X/ABS_Y` and other codes are redundant with the MT stream. `BTN_TOUCH` is only checked for agreement (`stats().btn_touch_mismatches`).
- Out-of-range or negative slots disable MT updates until a valid slot is selected (`stats().out_of_range_slots`).

### Sync loss (`SYN_DROPPED`)

1. On `SYN_DROPPED` the pending frame is rolled back, and every event up to and including the next `SYN_REPORT` is discarded.
2. That `SYN_REPORT` returns `Step::ResyncRequired`.
3. The caller then either:
   - reads `EVIOCGABS(ABS_MT_SLOT)` and `EVIOCGMTSLOTS` for TRACKING_ID/POSITION_X/POSITION_Y and calls `resync(&MtSnapshot)`. Contacts whose tracking id survived keep their `ContactId`; others get Up, Up+Down, or Down. **This is the preferred path.**
   - or calls `resync_without_snapshot()`. Every live contact is **Cancelled**, the current slot becomes unknown, and each slot is ignored until it reports a fresh tracking id (a contact missing a coordinate is held back until both are known).
4. If the caller does neither, the next pushed event triggers the no-snapshot path automatically, so touch can never stall.

The gesture layer never turns a Cancel into a Tap; open drags and two-finger gestures end with `cancelled: true`.

## Orientation (`pointer`)

The panel's physical orientation relative to the 1280×800 logical GUI has not been measured. `Orientation { swap_xy, flip_x, flip_y }` covers all 8 axis-aligned possibilities (`Orientation::ALL`), applied as swap, then mirror X, then mirror Y. M12 will touch the corners and centre of the real panel and pick one. Raw ranges are passed in from metadata rather than hard-coded.

## Gestures (`gesture`)

Defaults: 12 px slop, 300 ms tap limit, 500 ms long-press. Long-presses fire from `tick(now)` or the next event's timestamp, so tests are deterministic.

To avoid stealing touches in a performance UI:

- A two-finger gesture starts only when exactly two contacts are down and both are still undecided (inside slop, before long-press). A finger already dragging a fader stays a drag, and a second finger on a pad still taps.
- `two_finger: false` turns two-finger recognition off entirely.
- One `TwoFinger` update per decoder frame carries centroid pan (`dx`, `dy`) and spread `scale` relative to the start, so pinch and two-finger pan come from the same event.
- "Scroll" is the widget's interpretation of `Drag` or `TwoFinger` deltas, not a separate recognizer.

## Tests

`rust/mpfree-touch/tests/m11_multitouch.rs` (22 tests) and unit tests in `pointer.rs` cover: single and multiple contacts, slot persistence, coordinate inheritance on slot reuse, id-to-id switches, contacts born and ended in one frame, tracking-id wraparound, malformed slots and unknown codes, missing `SYN_REPORT`, `BTN_TOUCH` disagreement, `SYN_DROPPED` with snapshot (identity preserved), with no snapshot, and with the resync ignored; all 8 orientations and clamping; tap, slow release, long-press, drag deltas, two-finger pan and pinch, drag not hijacked by a second finger, two-finger disabled, and cancellation after sync loss.

~~~sh
cargo test --manifest-path rust/Cargo.toml --locked -p mpfree-touch
~~~

## Not in M11

- Reading `/dev/input/event0`. The existing M9/M10 ioctl boundary in `main.rs` is unchanged and still never reads events or grabs the device.
- Calibration and choosing an orientation (M12).
- Any GUI toolkit binding. The GUI stack is being decided separately; these types are plain data.
