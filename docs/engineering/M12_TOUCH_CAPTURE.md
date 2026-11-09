# M12: live touch capture and orientation calibration

**Status:** implemented and tested on the workstation and in CI with synthetic captures. **Not yet run on the MPC.** Plan, approved by the owner on 2026-11-09: [M12 plan](https://claude.ai/artifact/7nmxioi3kAyo6RTR5id8QK).

M12 records real ILI2117 touch events while the factory app keeps running. You do one touch step at a time: read the step, press Enter on the workstation, wait for the 3-2-1 countdown, then do it within 6 seconds. The workstation then decodes them with the M11 code to find the panel orientation, how far the touch area reaches, how much a held finger jitters, the report rate, and how many fingers are actually tracked.

## What touches the MPC

| Step | Runs on | Effect on the MPC | How it is undone |
|---|---|---|---|
| Build, tests, analyzer | Workstation | None | Nothing to undo |
| `--touch-capture-check` | MPC over SSH | Read-only: sysfs, `/proc`, systemd state, eMMC p1–p6 write counters | Nothing to undo |
| `--touch-capture`: staging, once per step | MPC `/run` (tmpfs, RAM) | One SHA-256-verified binary in a random temporary directory | Shell trap deletes it, and the run checks it is gone. If not, a reboot empties `/run`, and the next preflight reports any leftover folder |
| `--touch-capture`: capture, once per step | MPC | Opens `/dev/input/event0` `O_RDONLY\|O_NONBLOCK`, reads events for 9 s | Descriptor closes when the process exits, aborts or is killed |
| `--touch-capture`: after each step | MPC over SSH | Read-only: factory app still active, p1–p6 write counters equal to the preflight's | Nothing to undo |

Your taps also reach the factory app, as they always do. Do the capture in a new, empty project and don't save it.

Never done: `EVIOCGRAB` or any other ioctl (the ARM binary doesn't even import `ioctl`; CI checks this), I²C access, opening `event1` or DRM, stopping or restarting services, writes outside `/run`, remounts.

Why reading is safe: Linux gives each evdev reader its own copy of every event, so the factory app still gets every touch. If our buffer overflows, only our reader sees `SYN_DROPPED`. The driver's open and close hooks run only for the first and last reader, and the factory app holds `event0` open throughout. If the factory app had an exclusive grab, we would get zero events. That makes the run fail closed, and says so.

## Parts

- `rust/mpfree-touch/src/bin/capture.rs` (`mpfree-touch-capture`). `#![forbid(unsafe_code)]`, no FFI. It does the same strict sysfs identity checks as M10 and opens only the hard-coded `event0`, read-only and non-blocking, polling every 10 ms. It parses 16-byte armhf `input_event` records and rejects any other layout. It prints `E <sec> <usec> <type> <code> <value>` lines between start and end markers. Limits: 9 s per run (one step), 100,000-event cap, and a separate 20 s watchdog thread that aborts. A separate binary keeps the M10 `mpfree-touch` guarantees and CI checks intact.
- `tools/mpc-one/read-only-touch-capture-env.sh`: read-only preflight. It checks the touch identity and that the factory app is active. It confirms mmcblk0 p1–p6 have the expected partition names before printing their write counters. It also counts leftover `/run/mpfree-touch-capture.*` folders from an interrupted run, without touching them.
- `tools/mpc-one/host_touch_capture.py`: preflight and run, reusing the M5–M10 identity, ABI and RAM checks. With `--execute`, it walks you through the steps. For each one it waits for Enter, stages the binary with `ulimit -c 0 -f 0 -t 15` under SCHED_OTHER, counts down 3-2-1, says GO, and says STOP when the 9 s run ends. It fails closed if the eMMC counters differ from the preflight's, the stage directory remains, or the factory app stops. If a step saw the wrong number of touches, it offers to redo it. It writes a step-labelled event log to `research/`. `nice` is not installed on the device image, so it isn't used.
- Ctrl-C stops the run at any point. The wrapper runs the helper with `timeout --foreground` so it gets Ctrl-C and can read Enter. The remote script ignores SIGPIPE, so when the SSH link closes, the capture ends within its 9 s and the shell's exit trap still deletes the `/run` copy. Before this fix, a closed link could kill the shell before the trap ran, leaving the copy in RAM until reboot. This was reproduced under BusyBox ash, and the fix was tested there.
- `rust/mpfree-touch/src/calibrate.rs` and `mpfree-touch-analyze`: host-only. They parse the step-labelled log, decode each step with a fresh `MtDecoder`, check each step did what it asked, and test all 8 orientations. They print the report and a proposed profile in JSON.

## Touch protocol

The terminal shows one step at a time. Read it, press Enter, wait for the 3-2-1 countdown, then do it after GO. You have 6 seconds, and the terminal says STOP when the window closes. "Top-left" means top-left as you face the MPC in its normal playing position.

| # | Step |
|---|---|
| 1 | One finger held still in the centre for 3 s |
| 2–5 | Corners, about 1 s each, as close to the edges as you can: top-left, top-right, bottom-right, bottom-left |
| 6 | Slow drag left to right across the middle |
| 7 | Slow drag top to bottom through the middle |
| 8 | Two fingers, hold 2 s |
| 9 | Five fingers (one flat hand), hold 2 s |
| 10 | Optional: ten fingers (both hands), hold 2 s. Type `s` and Enter to skip |

After each step the terminal shows how many touches it saw. If that isn't what the step asked for, press Enter to redo the step, or type `k` and Enter to keep it anyway.

## Pass and fail

PASS needs all of these:

- exactly one orientation fits all four corners and both drags;
- every corner touch lands within 15% of its edges;
- every step's run passed, with eMMC p1–p6 write counts equal to the preflight's, the stage directory removed, and the factory app active;
- you confirm the factory screen, pads and power button still behave normally.

Fail closed, with no retries and no loosened checks, on any of these:

- any identity, ABI or hash mismatch;
- zero events across all steps;
- a malformed record;
- the watchdog, the deadline or the event cap firing;
- a required step missing, or a step that didn't do what it asked (one finger for steps 1–7, a hold that stayed put, a drag across at least 30% of the screen, two or more fingers for steps 8–9);
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
# project first, and turn the volume down. Run it from an interactive
# terminal: it waits for Enter before each step. Ctrl-C stops it.
MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_TOUCH_CAPTURE=YES \
  bash tools/mpc-one/collect-over-ssh.sh --touch-capture

rust/target/release/mpfree-touch-analyze \
  "$(ls -t research/mpc-one-touch-capture-*.events.log | head -n 1)"
~~~

Reports and event logs stay in gitignored `research/`, each with a `.sha256` sidecar. After a PASS, the reviewed profile goes to `docs/hardware/data/mpc-one-touch-profile.json` in a follow-up PR.
