# M13: live touch reader

**Status:** built and tested on the workstation and in CI with a fake device. Not yet run on the MPC. Nothing in this milestone touches the device by itself; it is a library that the GUI (`mpfree-hello`, later the main app) will use.

## Why

The GUI's first touch reader (`TouchIn` in `mpfree-hello`, PR #31) opens `/dev/input/event0` and feeds the M11 decoder. Two gaps were left open in the M12 handoff:

1. **The current slot at open is unknown.** The kernel sends `ABS_MT_SLOT` only when the slot changes, and filters repeated per-slot values. A reader that opens fresh assumes slot 0 and empty slots, so if the kernel last selected another slot, the next contact is filed under the wrong slot; and a contact that shares a coordinate with the slot's previous contact is held back until the other coordinate changes.
2. **An overflow cancels every finger.** On `SYN_DROPPED` the reader used `resync_without_snapshot`, which is safe but drops held fingers.

## What it does

- `mpfree_touch::profile::MPC_ONE` holds the M12 profile (raw ranges 0 to 2048, 10 slots, `swap_xy=true, flip_x=true, flip_y=false`, logical 1280×800). A unit test parses `docs/hardware/data/mpc-one-touch-profile.json` and fails if the constants drift from it.
- `MtDecoder::seed(&MtSnapshot, time)` seeds a decoder with the kernel's current slot and each slot's last tracking id and position. Fingers already on the screen at open are **not** published: their slots are ignored until they lift, so a resting finger can't press whatever lies under it when the GUI starts. Stale slots now keep recording positions (never publishing them), because the kernel won't resend a value a later contact happens to share.
- The new crate `mpfree-touch-evdev`:
  - `EvdevFile` opens the node read-only and non-blocking. **No `EVIOCGRAB`**, so Akai's app keeps every event when it runs alongside.
  - At open it checks the device's axis ranges and slot count against the profile (`EVIOCGABS`), drains anything queued, reads a snapshot (`EVIOCGABS(ABS_MT_SLOT).value` plus `EVIOCGMTSLOTS` for tracking id, X and Y) and seeds the decoder. If the snapshot can't be read, open fails rather than guessing.
  - On `SYN_DROPPED` it drops the rest of what is buffered and queued (all of it predates the snapshot, and replaying it could end a contact that's still down), reads a fresh snapshot and resyncs, so held fingers keep their identity. Only if the snapshot fails does it cancel every finger.
  - `TouchReader<S: TouchSource>` is generic over the source, so all of the above is tested with a fake device.
  - `ReaderStats` (slot and fingers at open, resyncs, fallbacks, drained events, contacts) is for the GUI's Debug page and the workstation log.

## Safety

The only unsafe code is two `_IOR` ioctls in `rust/mpfree-touch-evdev/src/sys.rs`: `EVIOCGABS` and `EVIOCGMTSLOTS`. They read device state, change nothing, consume no events and don't affect other readers. CI asserts there is no `EVIOCGRAB`, `EVIOCSABS`, write, create or process call in the crate, and exactly two ioctl calls. The request numbers are checked against the Linux UAPI in a unit test (`0x8018456f` for `EVIOCGABS(ABS_MT_SLOT)`, `0x802c450a` for a 10-slot `EVIOCGMTSLOTS`, both confirmed against `<linux/input.h>` with gcc on the workstation). The M12 capture tool is unchanged and still makes no ioctl at all.

## Not yet verified

- The ioctls have not run against the real ILI2117. The workstation has no evdev device, so only bad-fd error paths ran for real. The first device check should open the reader beside Akai's app (read-only, no grab) and log `ReaderStats` plus a few touches. That will be its own owner-paced step with full instructions.
- The snapshot is four separate ioctls, not one atomic read. A contact that changes between them is corrected by the next events; at worst a finger that lifts and lands again in that instant shows as an extra Up and Down.

## Next

`mpfree-hello`'s `TouchIn` switches to `EvdevTouch` once PR #31 (Hello MPC round 5) has merged, so the round 5 build Jay is testing doesn't change under him.
