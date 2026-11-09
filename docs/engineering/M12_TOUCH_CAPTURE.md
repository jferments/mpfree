# M12: live touch capture and orientation calibration

**Status:** implemented and tested on the workstation and in CI with synthetic captures. **Not yet run on the MPC.** Plan, approved by the owner on 2026-11-09: [M12 plan](https://claude.ai/artifact/7nmxioi3kAyo6RTR5id8QK).

M12 records about 60 seconds of real ILI2117 touch events while the factory app keeps running. The workstation then decodes them with the M11 code to find the panel orientation, how far the touch area reaches, how much a held finger jitters, the report rate, and how many fingers are actually tracked.

## What touches the MPC

| Step | Runs on | Effect on the MPC | How it is undone |
|---|---|---|---|
| Build, tests, analyzer | Workstation | None | Nothing to undo |
| `--touch-capture-check` | MPC over SSH | Read-only: sysfs, `/proc`, systemd state, eMMC p1–p6 write counters | Nothing to undo |
| `--touch-capture`: staging | MPC `/run` (tmpfs, RAM) | One SHA-256-verified binary in a random temporary directory | Shell trap deletes it, and the run checks it is gone. If not, a reboot empties `/run` |
| `--touch-capture`: capture | MPC | Opens `/dev/input/event0` `O_RDONLY\|O_NONBLOCK`, reads events for at most 60 s | Descriptor closes when the process exits, aborts or is killed |
| `--touch-capture`: after | MPC over SSH | Read-only: factory app still active, p1–p6 write counters unchanged | Nothing to undo |

Your taps also reach the factory app, as they always do. Do the capture in a new, empty project and don't save it.

Never done: `EVIOCGRAB` or any other ioctl (the ARM binary doesn't even import `ioctl`; CI checks this), I²C access, opening `event1` or DRM, stopping or restarting services, writes outside `/run`, remounts.

Why reading is safe: Linux gives each evdev reader its own copy of every event, so the factory app still gets every touch. If our buffer overflows, only our reader sees `SYN_DROPPED`. The driver's open and close hooks run only for the first and last reader, and the factory app holds `event0` open throughout. If the factory app had an exclusive grab, we would get zero events. That makes the run fail closed, and says so.

## Parts

- `rust/mpfree-touch/src/bin/capture.rs` (`mpfree-touch-capture`). `#![forbid(unsafe_code)]`, no FFI. It does the same strict sysfs identity checks as M10 and opens only the hard-coded `event0`, read-only and non-blocking, polling every 10 ms. It parses 16-byte armhf `input_event` records and rejects any other layout. It prints `E <sec> <usec> <type> <code> <value>` lines between start and end markers. Limits: 60 s capture, 100,000-event cap, and a separate 70 s watchdog thread that aborts. A separate binary keeps the M10 `mpfree-touch` guarantees and CI checks intact.
- `tools/mpc-one/read-only-touch-capture-env.sh`: read-only preflight. It checks the touch identity and that the factory app is active. It confirms mmcblk0 p1–p6 have the expected partition names before printing their write counters.
- `tools/mpc-one/host_touch_capture.py`: preflight and run, reusing the M5–M10 identity, ABI and RAM checks. With `--execute`, it stages the binary with `ulimit -c 0 -f 0 -t 15` under SCHED_OTHER. It streams the event log to `research/`, prints timed prompts, and fails closed if the eMMC counters change, the stage directory remains, the factory app stops, or zero events arrive. `nice` is not installed on the device image, so it isn't used.
- `rust/mpfree-touch/src/calibrate.rs` and `mpfree-touch-analyze`: host-only. They parse the log, decode it with `MtDecoder`, match the touch protocol, and test all 8 orientations. They print the report and a proposed profile in JSON.

## Touch protocol

The terminal prompts you. Lift your fingers between steps. "Top-left" means top-left as you face the MPC in its normal playing position.

| Seconds | Step |
|---|---|
| 0–3 | Get ready |
| 3–8 | One finger held still in the centre for 3 s |
| 8–24 | Corners, about 1 s each: top-left, top-right, bottom-right, bottom-left |
| 24–32 | Slow drag left to right across the middle, then top to bottom |
| 32–37 | Two fingers, hold 2 s |
| 37–43 | Five fingers (one flat hand), hold 2 s |
| 43–50 | Optional: ten fingers (both hands), hold 2 s |
| 50–60 | Hands off |

## Pass and fail

PASS needs all of these:

- exactly one orientation fits all four corners and both drags;
- every corner touch lands within 15% of its edges;
- the capture itself passed, with an unchanged eMMC p1–p6 write count, the stage directory removed, and the factory app active;
- you confirm the factory screen, pads and power button still behave normally.

Fail closed, with no retries and no loosened checks, on any of these:

- any identity, ABI or hash mismatch;
- zero events;
- a malformed record;
- the watchdog, the deadline or the event cap firing;
- a step missing from the protocol;
- zero or several fitting orientations.

The drop count and the ten-finger result are recorded but don't affect PASS.

## Commands

From an up-to-date checkout of this branch on the workstation:

~~~sh
cargo test --manifest-path rust/Cargo.toml --locked -p mpfree-touch
python3 -m unittest discover -s tests -p 'test_*.py'
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-touch
rust/target/release/mpfree-touch-analyze --self-test
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-touch

# Read-only preflight. Nothing is uploaded.
bash tools/mpc-one/collect-over-ssh.sh --touch-capture-check

# Only after the preflight passes. Save your work and open a new empty
# project first, and turn the volume down.
MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_TOUCH_CAPTURE=YES \
  bash tools/mpc-one/collect-over-ssh.sh --touch-capture

rust/target/release/mpfree-touch-analyze \
  "$(ls -t research/mpc-one-touch-capture-*.events.log | head -n 1)"
~~~

Reports and event logs stay in gitignored `research/`, each with a `.sha256` sidecar. After a PASS, the reviewed profile goes to `docs/hardware/data/mpc-one-touch-profile.json` in a follow-up PR.
