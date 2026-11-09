# M8 offscreen display test card: prepare a reversible GUI without touching factory DRM

**Status:** Host-only, deterministic Rust raster/coordinate prototype in `rust/mpfree-display`. The code contains **no** DRM/KMS, framebuffer, evdev, I²C, ALSA, GPIO, network, systemd or factory launch/stop APIs. It is **not** a GUI shown on the MPC and has no permission/means to acquire factory display or input devices.

## Why the display rehearsal is based on real hardware evidence

The owner's physical MPC One `--touchscreen` survey confirmed:
- Native DSI connector `card0-DSI-1` runs 800×1280, `/dev/fb0` has a virtual 800×3840 framebuffer at 32bpp, stride 3200 bytes.
- Its physical screen is used in landscape; logical GUI dimensions proposed are **1280×800**. We do **not** yet know whether clockwise or counterclockwise is correct.
- The ILI2117 `ili2117` kernel driver exposes `EV_ABS`, BTN_TOUCH, ABS_X/Y and multitouch slots/position/tracking IDs. Real EVIOCGABS min/max and calibration/rotation are **still unknown**.
- Rockchip RK3288 / Mali T760 + Panfrost; original factory MPC opens `/dev/dri/card0` and `/dev/input/event0`.

The independent service `DeviceControlServer.bin` and power-button handler are still active alongside the main factory MPC. A later stop of **only acvs** may leave them intact, but is not tested and remains blocked on recovery proof.

### API boundaries

Rust `mpfree-display` separates:
- `RgbCanvas`: finite memory-bounded logical RGB888 image in user RAM, with safe drawing and FNV checksum. This **does not assume the DRM framebuffer format** is RGB888. A future KMS adapter will convert pixels to the **queried actual** DRM format; never directly copy RGB bytes to /dev/fb0 without proof.
- `QuarterTurn`: either clockwise or counterclockwise 90° rotation from logical landscape to native portrait; no guessed default selected for physical hardware.
- `map_raw_touch`: requires caller-supplied raw X/Y axis minimum/maximum and orientation, rejecting out-of-range values, zero-span axes and invalid input. The synthetic unit-test ranges are **fixtures, not measured ILI2117 ranges**.
- `render_test_card`: four different brightly colored corner markers and an MPFREE TEST label, plus grid and crosshair. The distinct colors make rotation/mirroring recognizable once safely displayed on a physical panel.

No real touch events, fingers, click targets or GUI window managers are part of this proof.

## Owner workstation rehearsal (after CI and merge)

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-display

# Deterministic in-memory test: NO file writes or hardware access.
./rust/target/release/mpfree-display --self-test

# Explicitly save *new* PPM previews to the workstation (create_new prevents overwrite).
./rust/target/release/mpfree-display --preview-logical /tmp/mpfree-logical-preview.ppm
./rust/target/release/mpfree-display --preview-cw /tmp/mpfree-cw-800x1280.ppm
./rust/target/release/mpfree-display --preview-ccw /tmp/mpfree-ccw-800x1280.ppm
xdg-open /tmp/mpfree-logical-preview.ppm
~~~

Each output file is plain P6 PPM, no image dependencies. **Use a fresh output filename** if those files already exist; the CLI intentionally refuses overwriting. Logical preview is 1280×800, physical-orientation candidates are 800×1280. Some desktop image viewers will orient them as portrait because the pixel dimensions reflect the *native DSI frame*, not the physical MPC rotation.

This code can cross-build for ARMv7, but the GUI test has **no on-device launcher or permission to show anything on the screen** in this milestone.

## Later gates before live GUI takeover

1. Measure actual evdev ABS ranges and slots with a separately reviewed, non-grab, metadata-only ioctl (no event reading). Preserve factory evdev ownership.
2. Work out panel orientation and raw touch-to-GUI mapping empirically (nonexclusive, bounded gesture capture only with owner authorization). Build an input event backend that handles `ABS_MT_SLOT`, tracking IDs, `SYN_REPORT` and `SYN_DROPPED`; event0 is not a guaranteed stable name across boot, so resolve by sysfs device identity.
3. Independently demonstrate original MPC One boot/eMMC recovery; the official firmware updater is not proof of recovery from a damaged bootloader.
4. Build and test a separate independent supervised restoration controller with an out-of-band timeout armed **before** factory state change. Retain power-button, controller service, network and SSH. A pure Rust state machine on workstation already models failures, but hasn't controlled systemd.
5. **Separate future explicit approval** for acvs stop; verify no other process has DRM master; render a short KMS test card; close all DRM/input fds; restart stock and verify GUI, touch, controls and audio. No mask/disable/persistent overlay changes in first session.

A successful offscreen PPM render is preparation, **not evidence** of working LCD or touchscreen output, and successful `systemctl start` is not yet verified factory recovery.
