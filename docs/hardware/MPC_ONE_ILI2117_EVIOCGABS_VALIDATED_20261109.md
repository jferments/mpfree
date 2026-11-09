# Original MPC One — M10 physical ILI2117 EVIOCGABS acceptance (2026-11-09)

**Status: owner-run M10 READ-ONLY TOUCH METADATA TEST PASSED on the physical original MPC One.**

This record is derived from the owner's `--touch-axes-check` and double-opt-in
`--touch-axes` report. Only reproducible hardware metadata is recorded;
the original private logs, device network identifiers, transient paths and
binary/log SHA-256 values **are intentionally not committed**.

## Exact measured touch metadata

Device identity: original Akai MPC One, factory az0x 5.0.8 and glibc 2.39;
`/dev/input/event0` identified as `ILI2117 Touchscreen`; Linux
`ili2117` driver on I²C bus 4/address 0x26.

| Linux ABS axis | Code | Reported minimum | Reported maximum | Fuzz | Flat | Resolution |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `ABS_X` | 0 | **0** | **2048** | 0 | 0 | 0 |
| `ABS_Y` | 1 | **0** | **2048** | 0 | 0 | 0 |
| `ABS_MT_SLOT` | 47 | **0** | **9** | 0 | 0 | 0 |
| `ABS_MT_POSITION_X` | 53 | **0** | **2048** | 0 | 0 | 0 |
| `ABS_MT_POSITION_Y` | 54 | **0** | **2048** | 0 | 0 | 0 |
| `ABS_MT_TRACKING_ID` | 57 | **0** | **65535** | 0 | 0 | 0 |

The kernel reports **10 multitouch slots** (`ABS_MT_SLOT` has inclusive
indices 0–9). This is a driver-advertised capacity, **not proof that the
physical sensor reliably tracks 10 fingers simultaneously**.

**Do not silently replace the measured 2048 axis maxima with 2047.**
The coordinates are reported using an inclusive range `0..=2048`, which
has 2049 possible integer codes. `resolution=0` means the driver did not
provide meaningful physical units/mm; it is not evidence that the physical
resolution is zero.

The `ABS_MT_TRACKING_ID` capability reports `0..=65535`. The negative
event value `-1`, used by Linux MT protocol to mark contact lift, is an
**event semantic**, not something that must appear in the capability's
min/max metadata. Actual contact lifecycles have not yet been captured.

## Experiment safety and result boundaries

1. The read-only `--touch-axes-check` preflight **passed**, verifying the
   device identity, matching ARM EABI5 hard-float ELF, target GLIBC2.39 vs
   highest required 2.34, the exact ILI2117 driver, factory service active,
   `/run` tmpfs and SCHED_OTHER constraints. It made **no evdev open**.
2. The separate `--touch-axes` run required both
   `MPFREE_ACCEPT_VOLATILE_EXEC=YES` and `MPFREE_ACCEPT_TOUCH_AXES=YES`.
   The ARMv7 executable was staged under a randomized `/run` directory
   after SHA-256 verification, and ran the **fixed** `--inspect` action.
3. The helper reported `access=evdev_RDONLY`,
   `events_consumed=0`, `exclusive_grab=no`; its six read-only
   `EVIOCGABS` calls succeeded and emitted the metadata above.
4. The in-process `MPFREE_TOUCH_IOCTL_WATCHDOG_5S_ARMED` marker was
   emitted. The shell reported the stock `acvs.service` was active at
   completion and its normal cleanup trap removed the temporary stage.
5. **Not proven by this test:** actual GUI responsiveness, touch response after
   the probe (requires owner manual check), physical orientation, inversion,
   ten-finger sensor performance, timestamp accuracy, event loss or
   `SYN_DROPPED` recovery, actual DRM scanout, factory app stop/restart or
   independent factory boot recovery. The cleanup marker is a report of the
   trap's normal execution, not a separate filesystem audit.

## How the results feed MPFree

The Rust `mpfree-display::map_raw_touch` function already accepts
caller-supplied `AxisRange`. A new **host-only unit test** passes the
physically observed `AxisRange { min: 0, max: 2048 }` for both axes and
verifies corner/midpoint mappings and rejection of values outside this range
under *both* possible quarter-turn orientations.

**Do not hardcode a rotation or silently apply this fixture to other MPC
models/firmware.** The 800×1280 DSI panel is physically landscape-mounted,
but the actual mapping from raw (x,y) to physical screen corners is still
unknown.

### Next separately authorized experiment

Implement a **bounded nonexclusive touch calibration** on the original MPC
while `acvs` remains active:

- First use host-only simulated Type-B event streams for
  `ABS_MT_SLOT`, `ABS_MT_TRACKING_ID`, `ABS_MT_POSITION_X/Y`,
  `SYN_REPORT` and `SYN_DROPPED`, plus per-contact state and lift handling.
- Only after safety review, build an exact-device-gated, explicitly opted-in
  5–10-second **non-grabbing** read-only gesture test, capturing only
  owner-selected corner/center touches needed for orientation. Do not send
  `EVIOCGRAB`; do not touch power-button event1.
- Test normal Akai operation manually before and after. Do not proceed with
  any factory display takeover until independent recovery is established.

For graphical feasibility, see
[`GPU_PANFROST_OFFSCREEN_20261109.md`](../engineering/GPU_PANFROST_OFFSCREEN_20261109.md):
the owner's separate native Panfrost EGL/GLES tests already proved **offscreen**
GPU rendering while Akai remained active; they **did not** obtain KMS master
or drive the actual LCD.
