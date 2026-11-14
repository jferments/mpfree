# Testing and CI

How MPFree is tested, where CI runs, what it checks, and how the suite is
meant to grow. Status as of 2026-11-11.

## The test tiers

| Tier | What it proves | Where it runs | Who starts it |
| --- | --- | --- | --- |
| Unit and crate tests | Each crate's logic: engines, DSP, parsers, UI layout and drawing | `cargo test` in `rust/`; CI on every PR | Anyone, CI |
| GUI smoke | Every control on every screen does something; random and scripted sequences of controls keep the screen and the engines together (below) | `cargo test -p mpfree-app` (`smoke`); CI on every PR | Anyone, CI |
| Host integration | Whole programs with no hardware: headless engine with null audio, session offline check, groove and sampler WAV renders, display previews, C++/Rust WAV parity | CI's `core` and `rust` jobs | CI |
| ARMv7 under qemu | The device build: static ARM binaries, ABI layouts against the kernel's headers, the app and UI tests as 32-bit ARM, preview screens | CI's `rust` job | CI |
| Device tools offline | Survey, backup, kexec and update tools against fake MPCs, fake images and BusyBox | CI's `syntax` job | CI |
| On the MPC | Real audio, pads, touch, screen | The reference MPC One, round by round with `tools/mpc-one/results.py` | Jay only |

The last tier never runs in CI. Every on-device step is paced by Jay and
must be reversible (the safety rules in the [README](../../../README.md) and
[developer access](developer-access.md)). A hardware-in-the-loop lane is not
planned until MaskROM recovery is proven on the unit.

## What CI runs

Workflow: [`.github/workflows/mpc-inventory.yml`](../../../.github/workflows/mpc-inventory.yml).
It runs on every pull request, every push to `main` and by hand, with no path
filters, so the check always reports.

| Job | Checks | Typical time |
| --- | --- | --- |
| `syntax` | Shell syntax; static no-write checks on the read-only scripts; about 20 Python test files for the device tools against fakes (BusyBox, offline images, DTBs, the RK3288 ROM tools); `python3 -m unittest discover -s tests` (12 more) | ~1 min |
| `core` | C++ reference engine build, `ctest`, WAV format and size | ~10 s |
| `lint` | `cargo clippy --workspace --all-targets` with warnings as errors, and `cargo fmt --check`, on a pinned toolchain | ~1 min |
| `rust` | All workspace tests in debug and release; the `mpfree-bench` no-allocation check; host self-tests and fixed-output renders; C++/Rust WAV parity; the ARMv7 hard-float cross-build, ELF checks, and ARM tests and previews under qemu | ~2-3 min warm |
| `splash-gl` | SDL3/GLES startup screen built from source and rendered headless with Mesa | ~1 min warm |
| `gpu` | The GPU renderer (`rust/mpfree-gpu`, OpenGL ES through EGL) with Mesa's software GPU: clippy, then its frames against the software path's for the MPC's screen and every panel turn, and a compute round trip against the CPU; then `mpfree-gpu-bench` built for the MPC (ARMv7) and checked as `host_gpu_mpc.py` checks it | ~2 min warm |
| `instructions` | Instructions per 128-frame block for every `mpfree-bench` case under callgrind, against `rust/mpfree-bench/instructions.txt`; fails when a case rises more than 5 % (see [profiling](../design/profiling.md#instruction-counts-the-instructions-ci-job)) | ~2 min warm |
| `coverage` | Line coverage per crate (report only, never fails a PR); on `main` and by hand | ~5 min |

The coverage table appears in the run's summary page, and the lcov file is
kept as an artifact for 7 days. If the account's artifact storage is full,
the upload fails without failing the run.

### Where it runs

GitHub-hosted Actions minutes ran out on 2026-11-11. Since then every job runs
on four self-hosted runners, `mpfree-ci` to `mpfree-ci-4` (label `mpfree-ci`),
inside the CI VM `project-after-ci` that also runs Project After's and
PlantGen's CI. The setup, resource limits, job guard and undo are documented in
jferments/ci-infrastructure `docs/mpfree-runner.md`.

Things that differ from GitHub's hosts, and that the workflow handles:

- The runner account has no sudo. The packages the jobs need are installed
  once on the VM by `mpfree_runner.py`, and the workflow's `apt-get` lines
  run only on GitHub's hosts. A job that needs a new package needs it added
  to `PACKAGES` in that script and installed on the VM first.
- `~/.cargo/bin` isn't on the service's PATH, so each Rust job adds it.
- Rust is chosen with `RUSTUP_TOOLCHAIN`, never `rustup default`, because
  the account is shared with other projects.
- Cargo targets are kept between runs at
  `$RUNNER_WORKSPACE/cargo-target/<job>`, outside the checkout. When they
  pass 15 GiB together, the job's own target is wiped first.
- Each runner has its own `/tmp` that lasts between jobs. Every job starts
  by deleting the runner's `/tmp/mpfree-*` files, because many steps write
  fixed `/tmp/mpfree-*` names, and some create them with `create_new`.
- Only Jay's own pushes and PRs from this repository's branches run. The
  VM's job guard refuses forks and other actors.

Four runners means at most four jobs at once; the rest queue. A run waiting
for a runner is waiting, not failed. GitHub's list of runs can show a run as
queued while its jobs are already running: to see what the runners are doing,
look at the jobs of each run, not the run's status.

If the account's artifact storage is full, the coverage job's lcov upload
fails without failing the run (the table is in the run summary anyway).

## Running it locally

On a Linux workstation with stable Rust and CMake (see the
[README](../../../README.md#build-and-test) for the C++ parity commands):

~~~sh
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- \
  -D warnings -A clippy::needless_range_loop -A clippy::should_implement_trait
python3 -m unittest discover -s tests -p 'test_*.py'
~~~

Coverage, with `cargo install cargo-llvm-cov` and
`rustup component add llvm-tools-preview`:

~~~sh
cd rust && cargo llvm-cov --workspace --all-targets --locked --no-report
cargo llvm-cov report --json --summary-only --output-path /tmp/cov.json
python3 ../tools/ci/coverage_summary.py /tmp/cov.json
~~~

## GUI smoke tests

Jay asked on 13 Nov 2026 for every button to do something when touched,
and for the sequences and combinations of controls musicians use to be
tested, not only single presses. Three kinds of test in
`rust/mpfree-app/src/smoke/` do that, all without a device. They press
controls as a finger would, through the same touch path as the
touchscreen, using the list of controls each widget gives
(`Widget::targets`, see [UI architecture](../design/ui-architecture.md#controls-and-the-smoke-tests)).

- **The sweep** (`smoke.rs`, `every_control_on_every_screen_does_something`)
  opens every sample screen and presses every control on it: a tap, a
  drag or a long press, as the control says. A press passes when it sends
  an action to the app or the audio thread, changes the screen, or says
  something on the hint line. A drag that does nothing is tried the other
  way (a slider at its end), and a press that does nothing after the ones
  before it is tried again on the screen as it first was. A control drawn
  faint, or already lit (the tab or row picked), may do nothing; any other
  that does nothing fails the test by name. `catalogue::KNOWN_DEAD` lists
  controls that may do nothing for now, each with who builds it; it is
  empty. `MPFREE_AUDIT_OUT=<file>` writes the whole report, one row per
  control. The sweep paints each screen before listing its controls,
  because some widgets (the top bar's chips) measure them when they draw.
  A press it cannot bring the page back from gets a fresh copy of the
  screen from a spare set. The spares share one copy of the fonts, and at
  most three sets' worth are kept, the screens swept soonest first, so
  the sweep fits in the 32-bit ARM test's memory (it peaks at about
  1.3 GB on x86-64, and takes about 3.5 minutes in a debug build).
- **The catalogue check** (`every_swept_control_is_in_the_catalogue`)
  reads the same sweep (it runs once for both tests) and fails on any
  `widget/id` the [controls catalogue](controls.md) doesn't explain. A new
  kind of control needs its entry there in the same pull request.
- **Random walks** (`smoke/walk.rs`) start a whole session (the screen,
  the app, the mixer, the time engine, the looper, the chain editor) and
  take seeded random steps: a control on the page, a hardware button
  (alone or with SHIFT), the wheel, a pad, or time passing. After each step
  the checks in `walk::check` must hold: the page the screen shows is the
  app's, the transport and tempo on screen are the time engine's, the
  master bar shows the Level, the step grid shows the clip and the mutes,
  the chain editor shows the mixer's chain, and more. A failure names the
  seed and every step. `MPFREE_WALK_SEED=<n>` walks one seed again,
  `MPFREE_WALKS` and `MPFREE_WALK_STEPS` walk more or longer, and
  `MPFREE_WALK_OUT=<file>` writes the presses that did nothing. One walk
  covers every page; the other stays on the mixer's FX pages.
- **Workflows** (`smoke/workflows.rs`) script what a musician does, step by
  step, and check each step against the engine, not only the screen: an
  insert added on the mixer, bypassed, moved and removed; a knob given to a
  macro; steps tapped on Beats playing in time; REC and Undo; mutes from
  the Play page, the pad mixer and TRACK MUTE; the synth staying on the pads;
  the tempo tapped; the master level from both ends; the kit from the
  wheel and from the kit chip; Full level turned off from its chip. A new
  feature a musician uses gets a workflow here.

`Session` (`smoke/session.rs`) is the test harness: the screen and every
engine wired together as `main.rs` wires them, run in simulated time
(`run_for`), with helpers to press a control by widget, id and label
(`press`), press a hardware button (`button`, `button_with`), turn the wheel
or a Q-Link, and hit a pad. The routing from screen to engines is shared
with `main.rs` (`desk.rs`), so the tests drive the real code.

## Rules for changes

- **Never skip, ignore or loosen a test to get green.** A test that is wrong
  gets fixed in the same PR, with the reason in the commit.
- **New behaviour comes with tests** in the crate that owns it. Engine code
  gets unit tests. Audio output gets a fixed-output or golden check. A parser
  gets tests on bad input as well as good.
- **A sound or screen that changes on purpose gets new golden references** in the
  same PR, with the reason in the commit:
  `MPFREE_GOLDEN_BLESS=1 cargo test -p mpfree-bench golden`. The golden
  test fingerprints each bench case per eighth of its render and channel
  (level in dB, zero crossings) and allows 0.5 dB and 3 % of crossings, so
  another CPU or libm passes and a changed preset, tuning or filter fails.
  A new bench case needs a reference too. A screen that changes on purpose
  is the same: `MPFREE_GOLDEN_BLESS=1 cargo test -p mpfree-app --test cli`
  (references are x86-64; the ARMv7 run under qemu checks only that the
  pictures are valid).
- **Audio-thread code doesn't allocate or block.** Each engine proves this
  with a counting allocator (`tests/no_alloc.rs`), CI runs
  `mpfree-bench --check-allocs` over the profiled engines, and
  `mpfree-app/src/rt_safety.rs` runs the pad player's whole sound path. A
  new audio path gets a no-allocation test, and a new step in the pad
  player's period goes into `rt_safety.rs` too.
- **A program gets a `tests/cli.rs`** that runs it whole through
  `env!("CARGO_BIN_EXE_<name>")`: its arguments, exit codes and output files.
  Tests write only under `env!("CARGO_TARGET_TMPDIR")`, never a fixed `/tmp`
  name, so parallel tests and the self-hosted runners don't collide.
- **No test touches a device.** Tests that need device-like input use
  fixtures, fakes (`tools/mpc-one/tests/fake_mpc_ssh.py`) or qemu.
- **Clippy is pinned** (`MPFREE_CLIPPY_TOOLCHAIN` in the workflow), so a new
  Rust release can't turn every PR red at once. To bump it, change the pin,
  fix the new warnings, and merge that as one PR. Two lints are allowed
  workspace-wide: `needless_range_loop` (index loops are clearer in much of
  the DSP) and `should_implement_trait` (`mpfree-sampler`'s `next`).
- **rustfmt is enforced** by the `lint` job (`cargo fmt --check`, with the
  pinned toolchain's rustfmt). Run `cargo fmt --manifest-path rust/Cargo.toml
  --all` before pushing. Source checks that read Rust files (the workflow's
  static checks, `test_app_mpc.py`'s write audit) compare what the code
  says, not how rustfmt wraps it.

## Merging

PRs are merged through the merge desk (trains of ready PRs tested together),
and code PRs need Jay's go. CI is meant to be a required status check on
`main`, so nothing merges red. That's a repository setting Jay turns on
(Settings, Branches, a rule for `main`, "Require status checks to pass"
with the jobs above).

## Coverage today

Line coverage of `cargo test` across the workspace, measured 2026-11-11 on
`main` (`59fd00d`) plus the whole-program tests: **87.3%** (37,654 of 43,113
lines). Programs with a `tests/cli.rs` (render, session, device-smoke,
screen-log, display) are run whole by `cargo test` through
`CARGO_BIN_EXE_*`, and the coverage run counts those runs too.

| Thinnest crates | Lines covered |
| --- | ---: |
| `mpfree-bench` | 69% |
| `mpfree-splash` | 69% (its `main.rs` drives the device framebuffer) |
| `mpfree-touch-evdev` | 71% |
| `mpfree-app` | 73% |
| `mpfree-headless` | 76% |
| `mpfree-groove` | 80% |
| `mpfree-touch` | 81% |

The engines are well covered: `mpfree-synth`, `-fx`, `-grain`, `-analysis`,
`-arp`, `-sampler`, `-play`, `-transport`, `-ui` are all above 90%. The
`coverage` job's summary on `main` has the current table.

## Plan: growing the regression suite

In order. Each step is its own PR.

1. **Gates** (done): clippy, the orphaned ROM tool tests, coverage, no path
   filters, the no-allocation bench check (#92), and rustfmt after the
   one-time reformat. Then the required check on `main`.
2. **Raise the thin crates** in the table above. Done first: whole-program
   tests for render, session, device-smoke, screen-log and display. Next is
   `mpfree-app`, the program that actually runs on the MPC.
3. **Golden regressions.** Audio is done: `mpfree-bench`'s golden test
   renders every bench case (each Poly preset, the drum kits, the plucks,
   every effect and reverb, the sampler, the mixer, the time engine and arp,
   the looper, a whole song) and compares a fingerprint with
   `rust/mpfree-bench/golden/cases.txt`. Screens too: `mpfree-app`'s
   `tests/cli.rs` draws all its preview screens and compares each with
   `rust/mpfree-app/golden/screens.txt` (the mean brightness of every
   20×20 cell, within 1 level; renaming one tab label trips it).
4. **Real-time safety everywhere.** Every engine has its own no-allocation
   test (`tests/no_alloc.rs` in synth, sampler, fx, grain, looper, play,
   transport, capture, track), and the `rust` job runs
   `mpfree-bench --quick --check-allocs`, which fails if any bench case
   allocates inside its audio callback. On top of those, `mpfree-app`'s
   `src/rt_safety.rs` runs the program's whole sound path the way its audio
   loop does, period by period: pads and pressure into the arp, the
   transport and a playing clip, the sampler or the synth, the click and the
   looper. Half a minute of that may not allocate or free once. Recording
   into a clip is left out: each finished note is an edit, and the clips are
   compiled again on that thread. Bench timings are not a gate, because
   wall-clock speed on a shared VM varies from run to run. CPU cost is gated
   by instruction counts instead: the `instructions` job counts every bench
   case's instructions per block under callgrind and fails when one rises
   more than 5 % above its committed baseline.
5. **Damaged input** (done): every reader of files and device input has a
   `tests/robust_*.rs` that feeds it thousands of damaged copies of good
   input (bits flipped, fields set to edge values, cut short, grown,
   repeated) and checks the answer is an error or something safe to use,
   never a panic: WAV in the sampler and wavetables, DX7 sysex (then played
   in the FM engine), kits files, the plugin process's wire messages and
   parameter ranges, touchscreen events, MIDI from the controller, arp
   patterns and libraries (they must write back to what they read), and
   settings files. The damage comes from `rust/test-support/mutate.rs`,
   which needs no crate and no nightly toolchain, and the same seed gives
   the same cases everywhere. A new reader gets the same kind of test.
6. **End-to-end headless scenarios** under qemu ARMv7: scripted pad and touch
   input in, audio and screen fingerprints out.
7. **Nightly long run:** soak tests and sanitizers. This needs the VM's job
   guard to allow `schedule` for jferments/mpfree (ci-infrastructure
   `runner_job_guard.py`).
