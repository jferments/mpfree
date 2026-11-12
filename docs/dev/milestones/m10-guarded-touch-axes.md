# M10 — guarded real MPC ILI2117 multitouch axis metadata experiment

**Status: IMPLEMENTED AND OWNER-RUN ON PHYSICAL MPC — PASS (2026-11-09).** M9 desktop tests passed (48 total); the M10 exact-original-MPC read-only preflight and double-opt-in RAM-only EVIOCGABS query both passed on the original MPC One. **The driver reported 0..=2048 on X/Y and MT_X/Y, 10 available MT slots (0–9) and tracking ID 0..=65535.** This is not evidence of correct orientation or responsive factory GUI after the test. See [sanitized physical acceptance](m10-validation-record.md). **Nothing stops or restarts Akai, grabs touch events, performs I2C commands, sets calibration, acquires DRM master or modifies persistent storage.**

This is a narrow experimental user-space interaction: after a successful preflight, the test briefly opens the already kernel-managed **/dev/input/event0** O_RDONLY|O_NONBLOCK and performs read-only EVIOCGABS metadata ioctls. This is a new class of access compared to earlier sysfs surveys, and the owner must authorize it explicitly.

## Hardware evidence before M10

- Original MPC One, az0x 5.0.8, GLIBC 2.39, kernel 6.6.95-az01, 32-bit hard-float ARM.
- Factory MPC app owns its DRM, touchscreen event0, GPIO event1, ALSA PCM and raw MIDI. az01-pwrbtn also reads event1; **M10 never touches event1**.
- Kernel binding: *ILI2117 Touchscreen*, I²C bus 4 device 0x26, driver **ili2117**, compatible **ilitek,ili2117**. Linux Type-B-style MT slot, tracking ID and coordinate event bitmaps confirmed.
- The display is native 800×1280 (logical MPFree landscape 1280×800). **Raw min/max and the driver's MT slot capacity are now measured; orientation, raw-to-screen calibration and physical ten-finger reliability remain unknown.**

## Security and safety gates

### Read-only --touch-axes-check
- Uses the existing host wrapper **collect-over-ssh.sh** to check exact local network interface, route, ARP/MAC and pinned SSH host key.
- Runs the same read-only ABI survey and RAM safety prerequisite check as M5–M7: *original MPC One*, exact az0x 5.0.8 / glibc2.39 / expected kernel, live factory acvs.service MainPID, /run tmpfs, inherited SCHED_OTHER, available ulimit CPU cap.
- Runs additional read-only **read-only-touch-axes-env.sh** to ensure the exact ILI2117 event0 sysfs name, I²C address, bound driver and compatible, and stock factory active.
- Checks the cross-built ARMv7 hard-float ELF dynamic loader / versioned glibc symbols, requires embedded M10 sentinel and five-second Rust watchdog marker, caps executable <=2MiB and hashes bytes. **No on-device upload or evdev open.**

### Separate owner opt-in --touch-axes
**Only run after the read-only preflight succeeds and factory project has been saved.**

Requires *both* MPFREE_ACCEPT_VOLATILE_EXEC=YES and MPFREE_ACCEPT_TOUCH_AXES=YES.

- Repeats identity/ABI/device preflights, creates *one random temporary directory under /run* and transmits the exact hashed executable.
- Re-verifies hash with SHA-256 **on the MPC** before running.
- Restricts to one hardcoded executable and fixed --inspect CLI action. Child gets only MPFREE_ACCEPT_TOUCH_IOCTL=YES for that invocation.
- Kernel CPU time max 3s, zero core dumps, zero regular file size, inherited SCHED_OTHER, 18-second host SSH deadline; a **separate 5s Rust watchdog thread** calls process abort if metadata inspection hangs (releasing its fd).
- Rust checks event0's actual **/sys/class/input/event0/device** resolved ancestry includes 4-0026, matches name and BUS_I2C, reads physical 4-0026 driver link and device-tree compatible NUL-delimited bytes; then it opens **only /dev/input/event0 O_RDONLY|O_NONBLOCK**.
- It queries EVIOCGABS for ABS_X (0), ABS_Y (1), ABS_MT_SLOT (47), ABS_MT_POSITION_X (53), ABS_MT_POSITION_Y (54), and ABS_MT_TRACKING_ID (57). **No read() / write() of events; no EVIOCGRAB; no EVIOCSABS; no I2C command or DRM.**
- It *never prints* the current touch-value member of input_absinfo, only min/max/fuzz/flat/resolution. Host requires complete, unique and sane values for 6 axes, internally consistent inclusive slot count (1–64), correct fixed identifiers, and a unique watchdog marker.
- After tool exits it closes the evdev descriptor, checks acvs.service remains active, cleans up temporary staging via shell trap. A PASS means **only metadata read succeeded and stock unit remained active**, not proven GUI/pad functionality or independent recovery.

Unexpected driver, kernel, process state, ELF, missing firmware metadata, ioctl rejection, timeout, checksum, output fields or watchdog -> **fail closed**; do not retry by weakening gates. The factory UI should be confirmed manually after test.

## Owner workstation acceptance

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main

cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked

CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-touch

ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
# FIRST: only read-only SSH + ABI/sysfs metadata preflight.
bash tools/mpc-one/collect-over-ssh.sh --touch-axes-check
~~~

**Do not proceed** if the above preflight fails. Save factory project, stop recording/playback, verify normal touch/UI and power button work before authorizing the following optional physical experiment:

~~~bash
MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_TOUCH_AXES=YES \
  bash tools/mpc-one/collect-over-ssh.sh --touch-axes
~~~

Reports are stored **only on ML-tower** in gitignored research/mpc-one-touch-axes-*.txt and hashed sidecars. Review before sharing; never commit original device reports and identifiers. Device touch events/current positions are **not collected**.

## Interpret the results

Successful results should include:

- MPFREE_TOUCH_ABS_METADATA_V1, driver=ili2117, device=event0, bus=i2c-4-0026, access=evdev_RDONLY;
- events_consumed=0 and exclusive_grab=no;
- <axis>_min, <axis>_max, <axis>_fuzz, <axis>_flat, <axis>_resolution for six supported ABS axes;
- multitouch_slots=N, status=PASS, temporary /run stage cleanup report;
- stock GUI is still active and owner confirms UI/touch works.

A metadata-only ioctl **does not identify rotation**: the axes may be swapped or inverted by driver/device tree. On this MPC the measured axes are `0..=2048` (inclusive), `ABS_MT_SLOT` is `0..=9` and each axis resolution field is 0. We added a host-only M8 regression for these real coordinate limits with both candidate 90° orientations; the actual physical orientation still requires separately authorized touch/corner calibration.

## Next after physical M10 acceptance

- [x] Add an M8 host-only coordinate regression using the **actual measured 0..=2048 limits**, testing both CW/CCW while leaving physical rotation unset. `ABS_MT_SLOT` reports 10 available slots, but a live ten-finger sensor test has not been conducted.
- Design a **separate** finite non-grabbing gesture capture with explicit permission and no event logging outside selected calibration points, and validate that the factory GUI remains interactive.
- Physical MPFree GUI/factory application stop-and-restart experiments remain **blocked** on independent stock boot recovery proof and a tested separate restoring supervisor. M10 does not change those gates.
