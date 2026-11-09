# M9: exact ILI2117 evdev ABS axis metadata — isolated Rust interface

**Status:** M9 host implementation and tests passed on owner ML-tower (48 Rust workspace tests); M10 now adds a separate exact-identity-gated `--touch-axes-check` read-only ABI+ILI2117 preflight and double-opt-in `--touch-axes` temporary read-only EVIOCGABS mode. **Not executed on the MPC**. Details, safety and commands: [M10 guarded touch axes](M10_GUARDED_TOUCH_AXES.md). No touch gestures or display handoff.

## Why a new interface is needed

The owner's ILI2117 touchscreen is bound to the factory Linux `ili2117` driver at I²C bus 4/address 0x26. It exposes `/dev/input/event0` and capability bitmap `ABS=2608000 3`, which includes:
- `ABS_X` (0), `ABS_Y` (1);
- `ABS_MT_SLOT` (47);
- `ABS_MT_POSITION_X` (53), `ABS_MT_POSITION_Y` (54), `ABS_MT_TRACKING_ID` (57).

It also has BTN_TOUCH and EV_SYN/EV_KEY/EV_ABS. The real panel reports DRM `800x1280` portrait, while MPFree uses logical `1280x800` landscape. **The device's exact coordinate min/max, axis resolutions, physical orientation and number of multitouch slots are not yet established by the read-only sysfs report.** Upstream driver code identifying a maximum of ten contacts does not prove that the factory-supplied vendor driver actually exposes ten slots.

The relevant Linux UAPI is `EVIOCGABS(axis)`, a **read-only ioctl request**, returning `struct input_absinfo` with `value`, `minimum`, `maximum`, `fuzz`, `flat` and `resolution`. M9 intentionally **never prints `value`**, which could describe the user's current touch location. Reading event stream messages is a separate interaction and NOT performed.

## Rust crate structure

- `rust/mpfree-touch/src/lib.rs`: `#![forbid(unsafe_code)]`. Pure metadata representation and validation, inclusive slot count, unit tests on synthetic values.
- `rust/mpfree-touch/src/main.rs`: the *only* small, separately audited `unsafe` interface is the Linux C ABI `ioctl`. The code uses a six-`i32` `#[repr(C)]` structure (24 bytes), `_IOR('E', 0x40 + axis, struct input_absinfo)` on Linux ARM and x86-64, verified against numeric requests in tests. It opens a **fixed** `/dev/input/event0` with `O_RDONLY | O_NONBLOCK` only after validating real sysfs `name`, I²C BUS_I2C, `4-0026` driver and `ilitek,ili2117` compatible string. No arbitrary device path, fd, command, SSH destination or process controls are accepted.
- Direct `--inspect` mode has a second explicit local `MPFREE_ACCEPT_TOUCH_IOCTL=YES` environment guard. This still is **NOT enough for on-device authorization by itself**; there is no MPC launch/deployment mechanism.
- No `read`, `write`, `EVIOCGRAB`, `EVIOCSABS`, framebuffer, DRM master, I²C transaction, network socket, systemctl or firmware API. No factory processes or services are stopped or signalled.
- Metadata currently returned if run on a separately approved device: min/max, fuzz/flat/resolution for six known axes and the inclusive ABS_MT_SLOT count. An unsupported range or unexpected driver fails closed.
- This is a Linux input *device read-only open*; it still interacts with a character node, unlike the prior purely sysfs shell survey. Shared read-only evdev descriptors ordinarily do not steal events, but this has **not been independently demonstrated on the owner's firmware**.

## Owner workstation tests (safe; NO on-device access)

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-touch
rust/target/release/mpfree-touch --self-test

# Optional cross-build for future ARMv7 qualification, but DO NOT copy
# the executable onto MPC or run --inspect on the device yet.
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-touch
~~~

`--self-test` performs no sysfs or `/dev/input` access, and prints a synthetic metadata fixture with exactly 10 slots. **That is a test fixture, not a physical-MPC result**.

`--inspect` is **not authorized for direct MPC execution** in this milestone. We should not give unguarded root SSH snippets that bypass the existing physical device identity gates.

## Future separate owner consent / deployment plan

1. **Implemented in M10; hardware not yet run.** Add a narrowly allowlisted `--touch-axes-check` read-only preflight to the existing exact Ethernet/MAC/SSH guard. Verify original MPC One, expected az0x/glibc/kernel, factory `acvs.service` still active, ILI2117 exact sysfs identity and current USB/SSH route.
2. Add an explicitly authorized `--touch-axes` mode with **two independent opt-ins**; only upload the exact cross-compiled, verified-hash ARMv7 `mpfree-touch` helper to volatile `/run`; set bounded CPU/wall deadline and separate watcher; run *only* the compiled `--inspect` action; remove staging directory even on failure. No arbitrary paths or commands.
3. Validate output as one `MPFREE_TOUCH_ABS_METADATA_V1` with exact driver/bus/access markers, all six axis range fields, no live touch values, and no unexpected error/duplicate markers. Verify the factory app remained active and the touchscreen/buttons still work after the test. **If there is any unexpected change, halt and diagnose.**
4. Import only these measured ranges and slot count into the M8 `map_raw_touch` calibration model. Raw physical-to-logical rotation still requires separately authorized touch/corner sampling; axis limits alone cannot determine physical orientation.
5. Only then investigate a non-grabbing, bounded gesture sampler. Keep factory GUI operating. Full DRM master, factory service stop/restart and graphics takeover remain on the *separate* recovery-gated roadmap.

The factory main MPC process currently opens `/dev/input/event0` and `event1`. Independent `az01-pwrbtn` also reads `event1`, so never grab the power-button device. These metadata investigations do not create a replacement for verified independent factory boot recovery.
