# Testing and CI

How MPFree is tested, where CI runs, what it checks, and how the suite is
meant to grow. Status as of 2026-11-11.

## The test tiers

| Tier | What it proves | Where it runs | Who starts it |
| --- | --- | --- | --- |
| Unit and crate tests | Each crate's logic: engines, DSP, parsers, UI layout and drawing | `cargo test` in `rust/`; CI on every PR | Anyone, CI |
| GUI smoke | Every control on every screen does something; random and scripted sequences of controls keep the screen and the engines together (below) | `cargo test -p mpfree-app` (`smoke`); CI on every PR | Anyone, CI |
| Host integration | Whole programs with no hardware: headless engine with null audio, session offline check, groove and sampler WAV renders, display previews, C++/Rust WAV parity | CI's `core` and `rust` jobs | CI |
| ARMv7 under qemu | The device build: static ARM binaries, ABI layouts against the kernel's headers, the app and UI tests as 32-bit ARM (the control sweep and song workflows only by hand), preview screens | CI's `rust-arm` job | CI |
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

- **Draft pull requests skip every job** (they report as skipped). A PR handed
  to the merge desk is marked draft: its commits are tested in the desk's
  train, and its own runs would only hold the runners while the train waits.
  Marking it ready runs CI in full; so does any push to a ready PR.
- **`rust` and `rust-arm` wait for `instructions`.** The gate takes about two
  minutes, so a rise over 5 % fails the run before the two long jobs start;
  when the gate fails they are skipped.
- **A numbers-only fix merges without a second full run.** When a run's only
  red job is the instruction gate and the fix changes only
  `rust/mpfree-bench/instructions.txt` (setting rows to what CI measured on the
  same build), no other job reads that file, so their results stand.

| Job | Checks | Typical time |
| --- | --- | --- |
| `syntax` | Shell syntax; static no-write checks on the read-only scripts; about 20 Python test files for the device tools against fakes (BusyBox, offline images, DTBs, the RK3288 ROM tools); `python3 -m unittest discover -s tests` (12 more) | ~1 min |
| `core` | C++ reference engine build, `ctest`, WAV format and size | ~10 s |
| `lint` | `cargo clippy --workspace --all-targets` with warnings as errors, and `cargo fmt --check`, on a pinned toolchain; the same for the USB creator and its build command (`rust/mpfree-creator-build`, whose unit tests run here too) | ~1 min |
| `rust` | After `instructions`: all workspace tests, natively in debug; the `mpfree-bench` no-allocation check; the FX quality ratchet; host self-tests and fixed-output renders; C++/Rust WAV parity | ~12 min |
| `rust-arm` | After `instructions`: the ARMv7 hard-float cross-build, ELF checks, the NEON kernels against their plain twins under qemu (`mpfree-simd`'s tests, Poly's Super presets, every Strings and Combo preset with the kernels on and off, then `tools/simd_arm_cost.py --gate` on the supersaw's saws and the Choir's formants: the same samples, and fewer ARMv7 instructions than each twin; [NEON kernels](../design/simd.md)), the app's and UI's tests as ARM and previews under qemu, the example plugins and the plugin sandbox. Split from `rust` on 17 Nov so the two run side by side on two runners. Under qemu it leaves out the app's smoke module (the control sweep, workflows and song workflows, about 32 minutes there; they run natively every time): **Run workflow** with the `full_arm` box ticked runs them as ARM too | ~9 min |
| `splash-gl` | SDL3/GLES startup screen built from source and rendered headless with Mesa | ~1 min warm |
| `gpu` | The GPU renderer (`rust/mpfree-gpu`, OpenGL ES through EGL) with Mesa's software GPU: clippy, then its frames against the software path's for the MPC's screen and every panel turn, and a compute round trip against the CPU; then `mpfree-gpu-bench` built for the MPC (ARMv7) and checked as `host_gpu_mpc.py` checks it | ~2 min warm |
| `instructions` | Instructions per 128-frame block for every `mpfree-bench` case under callgrind, against `rust/mpfree-bench/instructions.txt`; fails when a case rises more than 5 % (see [profiling](../design/profiling.md#instruction-counts-the-instructions-ci-job)) | ~2 min warm |
| `coverage` | Line coverage per crate (report only, never fails a PR); on `main` and by hand | ~5 min |
| `soak` | The `mpfree-bench` song for 2^21 periods (about 101 minutes of sound), failing on any allocation in any block (see [profiling](../design/profiling.md#the-soak-the-soak-ci-job)); only by hand, with **Run workflow**'s Soak box ticked | ~15 min |

The coverage table appears in the run's summary page. The lcov file and the
JSON summary are kept on the CI VM, in `/srv/mpfree-store/coverage` (the newest
30 runs; see [Artifacts](#artifacts)). If the store can't take them, the run
still passes.

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

### Artifacts

MPFree's workflows upload nothing to GitHub's artifact storage. That storage
is one allowance for the whole GitHub account (500 MB on Free, 1 GB on Pro,
shared with GitHub Packages and every private repository), billed by the hour
over the month, and when the month's share is used every upload on the account
fails ("Artifact storage quota has been hit", MPFree's runs since 2026-11-11).
Deleting artifacts doesn't give back what the month has used. Project After
and PlantGen avoid it the same way: their builds stay on the CI VM.

What a run makes is kept **on the CI VM instead**, in `/srv/mpfree-store`, a
folder the four mpfree-ci runners share. jferments/ci-infrastructure's
`mpfree_runner.py install` makes it (owned by the runner account, readable by
the VM's other accounts), and its `docs/mpfree-runner.md` ("The store") has
the `scp` line to copy files out. A job keeps files with
[`tools/ci/store.py`](../../../tools/ci/store.py):

~~~sh
python3 tools/ci/store.py keep KIND PATH...
~~~

| Folder | What | Kept |
|---|---|---|
| `kernel/runs/<run>.<attempt>/` | `kernel.yml`: `zImage`, the modules, the config, `System.map`, `build-info.txt`, `SHA256SUMS`, from every run (pull requests too) | 14 days, the newest 30 runs |
| `coverage/runs/<run>.<attempt>/` | `mpc-inventory.yml`'s coverage job: `lcov.info` and `coverage.json` | 14 days, the newest 30 runs |
| `creator/runs/<run>.<attempt>/` | `release-creator.yml`: the programs for the MPC and the free kits' credits, then the finished `release/` (the creators and `SHA256SUMS`) | 14 days, the newest 10 runs |

Each run's folder has a `run.json` (the run's link, ref, commit, and each
file's sha256 and size), and `<KIND>/main`, `<KIND>/pr-<number>` and so on
point at the newest run for that ref; the newest run on `main` is always
kept. The script refuses to replace a file a run already kept, and keeps
nothing when the VM's disk would be left with less than 10 GiB free (After's
and PlantGen's builds share it). A keep step is `continue-on-error`: when
the store can't take the files, the run still passes, with a warning, and
its summary still has the hashes.

GitHub's own machines (the USB creator's Linux, Windows and macOS builds)
can't reach the store, so the release workflow hands files over through its
draft release ([`tools/ci/release_assets.js`](../../../tools/ci/release_assets.js),
below). A new workflow keeps its files with `store.py`, never with
`actions/upload-artifact` or `actions/cache`.

The tests: `python3 tools/ci/test_store.py`, and
`node --test tools/ci/test_release_assets.js` (against a fake GitHub), both in
the `syntax` job.

### The USB creator's release workflow

[`.github/workflows/release-creator.yml`](../../../.github/workflows/release-creator.yml) is not CI: it runs only by hand (**Run workflow**) or on a `creator-vX.Y.Z` tag, and makes a draft release of the USB creator for Linux, Windows and macOS ([Releases](../design/usb-creator.md#releases)). MPFree's programs for the MPC, its free drum kits (`mpfree-free-kits`, which fetches the samples `rust/mpfree-factory/free-kits.txt` pins from GitHub and keeps them in the runner's tool cache, `_work/_tool/mpfree-free-kits`) and the draft itself use the self-hosted runners; the creator is built on GitHub's own Ubuntu 22.04, Windows and macOS runners, which count against the account's Actions minutes (about 180 counted minutes a release). Each build runs the creator's tests on its own system and lists that machine's USB disks, the only place the Windows and macOS disk code runs before a real stick. Between releases, [`cargo usb-creator`](build-usb-creator.md) builds a creator with the same programs on a developer's own Linux, macOS or Windows computer.

### The USB creator build on three systems

[`.github/workflows/usb-creator-build.yml`](../../../.github/workflows/usb-creator-build.yml) runs `cargo usb-creator` from a fresh clone on GitHub's own Ubuntu 24.04, macOS 15 (Apple silicon) and Windows Server 2025 machines, free for a public repository: the build command's tests, the whole build, the creator's `--cli disks`, and a second run as after a `git pull`. Each machine's programs' SHA-256s go in the run's summary; nothing is uploaded. On Linux it also builds `mpfree` and `mpfree-startup` the release's way (Ubuntu's cross compiler) and checks, under qemu, that both builds draw the same app screens and startup frames byte for byte, and that the download helper starts. It runs on pull requests that touch the build (the creator, the build command, `stick.rs`, the free kits, `.cargo/config.toml`, `.gitattributes`), on `main`, and by hand; it isn't a required check. The build command's unit tests and clippy also run in `lint`.

### MPFree's kernel build

[`.github/workflows/kernel.yml`](../../../.github/workflows/kernel.yml) builds MPFree's own kernel for the MPC One ([own-kernel step 2](../hardware/own-kernel.md)) with [`tools/kernel/build.sh`](../../../tools/kernel/build.sh). It is not a required check: it runs on a pull request or a push to `main` that changes `tools/kernel/` or the workflow, or by hand. It runs the build script's own tests, prints what the runner has (system, CPUs, memory, disk, container engines, host tools, which hosts it reaches), then builds Linux 6.18 with the PREEMPT_RT patch.

- **The compiler** is kernel.org's crosstool gcc 14 for 32-bit ARM (gcc and binutils, the toolchain kernel developers use, made to run on any x86-64 Linux), never the machine's own, so the kernel comes out the same wherever it is built.
- **Where it runs:** in the pinned container ([`tools/kernel/Dockerfile`](../../../tools/kernel/Dockerfile): Debian pinned by digest, packages from one day's snapshot.debian.org) where Docker or Podman works; otherwise on the machine itself, as its user, never as root. The mpfree-ci runners have neither engine (first run, 2026-11-17), so they build on the runner. A host tool the runner lacks (flex, bison, m4, patch, bc, depmod, the OpenSSL headers) is fetched with `apt-get download`, which checks it against the runner's own signed package lists, and unpacked into the run's folder; nothing is installed. These tools only build the kernel's own helper programs, not the kernel.
- **Checks:** the kernel, the RT patch and the compiler are taken by the sha256 pinned in [`versions.env`](../../../tools/kernel/versions.env). With nothing pinned, the newest 6.18 RT patch and its kernel and the newest gcc 14 crosstool are taken after kernel.org's PGP signatures check out (the kernel's must be Greg Kroah-Hartman's or Linus Torvalds's key, the compiler's Arnd Bergmann's, who builds the crosstool compilers), and `build-info.txt` lists the lines to pin. [`check-config.py`](../../../tools/kernel/check-config.py) stops the build if any setting in [`mpfree.config`](../../../tools/kernel/mpfree.config) did not take, or the eMMC and SD drivers ended up built in.
- **Result:** the run's summary shows `build-info.txt` (what was built, from what, how it was checked) and `SHA256SUMS`. The job log lists every setting of the built config, compactly, with a hash on each line ([`config-digest.py`](../../../tools/kernel/config-digest.py)): an agent who can read the log only as text copies it out, and `config-digest.py read` checks the copy and prints the set lines for comparing with another kernel's config. Every run keeps `zImage`, the modules, the config, `System.map`, `build-info.txt` and `SHA256SUMS` on the CI VM, in `/srv/mpfree-store/kernel` (see [Artifacts](#artifacts)); nothing goes to GitHub's artifact storage. When the store can't take them the step only warns ("Kernel not kept"), because the kernel itself built. Downloads live in the runner's tool cache (`_work/_tool/mpfree-kernel`; the runners' home folder is read-only to jobs); each run builds in its own folder there, removed when the run ends, so two builds at once don't meet.

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
  wheel and from the list of kits; Full level turned off from its chip. A new
  feature a musician uses gets a workflow here.
- **Song workflows** (`smoke/song_workflows.rs`) do the same with three
  whole demo songs written in code (`smoke/songs.rs`), saved into a
  library in a temporary folder and opened from BROWSE as a musician
  opens them:
  - *Night Market* (92 BPM, swung): five scenes chained by follow
    actions, scene mutes, fill-only and first-pass trig conditions, and a
    44-bar arrangement with markers.
  - *Glass Garden* (124 BPM): clips of 3, 5, 7, 12 and 16 steps looping
    against each other, ratchets, per-pass conditions, pan locks, a
    half-speed lead, and scene values for the morph bar.
  - *Tidal Suite* (76 BPM): clip and scene follow actions with chances
    (Next or Again, Next or Previous, Last), neighbour conditions, and no
    arrangement yet.

  The workflows check each song opens as written; plays its sections by
  follow actions; FILL; the polymeters, ratchets and conditions step by
  step; song mode following the arrangement; Scenes → song; TRACK MUTE
  mid-section; the morph crossfader; a REC overdub and a CAPTURE with
  their UNDO; and an edit (tempo, a step, a fader) saved with SHIFT +
  BROWSE, a New project, and the song opened again as edited.
  Exporting: Night Market's arrangement and 8 bars of Tidal Suite are
  exported from BROWSE in fast mode (`Session::make_sounds` gives the
  pads a kit of tones, so the files carry sound), and the mixdown,
  stems and MIDI file are read back and checked against the song. The
  pattern legend for writing more songs is on `hits` in `songs.rs`.

- **Workflow scripts** (`smoke/scripts.rs`) play the files in
  `tools/mpc-one/devtest/workflows/` (a beat from scratch, loops, every
  synth, effects, the demo songs, the Sample page, a heavy load) in the
  test session, on the same timing as the MPC, where a device round plays
  the same files in the real app (`mpfree play --script`). Each file's
  `expect` lines are checked here on every pull request, so a workflow
  that would fail on the MPC fails in CI first. The format and how to add
  one: [the automated device suite](../hardware/device-suite.md).

- **Device phases** (`smoke/phases.rs`) check, without the MPC, what the
  device run used to ask Jay to try by hand (his rule of 15 Nov: automate
  every check the smoke tests can do, and ask only about what a round
  tests). One module per area, so a round can run one area alone
  (`cargo test -p mpfree-app --bins smoke::phases::looper`):
  - `looper`: REC records from the bar line and REC again closes it on
    the bar line; REC and PLAY record until STOP, which keeps the take
    (silent to the bar line); a pad recorded with the Play page's echo in
    the loop; OVERDUB adds a layer; STOP stops the loop and its playhead
    at once (live test 3 found
    it playing on for minutes; the test fails on that build); every tool
    while it plays, UNDO and SHIFT + UNDO, the undo history back and
    forward, trim; a layer row's speaker, S, level and Q-Links; a layer
    dragged onto another loop and onto + New loop, with one UNDO taking a
    move back out of both loops; a loop's speaker through STOP and PLAY;
    Clear; x down to the last layer; Starts: When I play, a loop waiting
    with the song stopped until the first pad starts both, and one
    starting on the bar line before a pad with the song playing; Tempo
    from the first loop, a free first pass from REC whose close sets the
    tempo (exactly, in the app too) and starts the song.
  - `mixer`: TRACK MIX to the Mix tab and back to the page it came from,
    SHIFT + TRACK MIX on the pads; UNDO and SHIFT + UNDO on strip changes;
    a Room reverb with Damping and Early learned on Q-Links 2 and 3,
    switched to Plate (shared knobs kept, as far as the Plate's range
    goes: its Size runs 50 to 150 %), Q-Link 2 still on Damping and
    Early's Q-Link freed, and back to Room as it was left.
  - `track_mute`: with 24 tracks, TRACK MUTE's pads past track 16 by PAD
    BANK, the pads staying mute keys on another tab, and SHIFT + TRACK
    MUTE silencing a pad in the kit.
  - `device_log`: the log lines the device checks are judged on
    (`tools/mpc-one/app_checks.py`): a step done here as on the MPC must
    write its line in the shape the check matches, so a changed line fails
    here before a round.

  The rest of what those phases asked is in tests that already ran on
  every PR: Arrange in `time.rs`, `mpfree-play` and the song workflows
  (recording, takes, Capture, clip and row drops, the note editor, the
  Song view and its drags, the morph, Rec song); the Synth tab in
  `app.rs`, `synth.rs`, `mpfree-synth` and `mpfree-synth-ui` (engines,
  presets, the wheel on its tiles, tuning and A4, Drive at a level);
  Note Repeat and the arp in `app.rs` and `gui.rs` (the strip, - and +
  while held, Latch, the Arp panel); the Mix tab's Effects look, chain
  drag, I/O view and macros in `smoke/workflows.rs`. What the device
  still has to show is that the buttons, pads, Q-Links and touch reach
  the app, the lights, the sound, and how it holds up under load.

`Session` (`smoke/session.rs`) is the test harness: the screen and every
engine wired together as `main.rs` wires them, run in simulated time
(`run_for`), with helpers to press a control by widget, id and label
(`press`), press a hardware button (`button`, `button_with`,
`hold`), turn the wheel or a Q-Link, and hit a pad. `Session::with_library`
gives it a library, so saves and opens run as `main.rs` runs them;
`hits.played` keeps every note played with its track and time, and
`said()` reads the screen's events log. The routing from screen to engines is shared
with `main.rs` (`desk.rs`), so the tests drive the real code. Each period it
also does what `main.rs` does around the engines: it shows the Looper tab
the loop and its wave, tells the app the track count, and tells the screen
when an effect's mode switch moved the controls a Q-Link had learned.

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
- **A test that waits for a worker thread fails when the wait runs out,
  never carries on.** The coverage job's instrumented build on a busy runner
  is many times slower than `cargo test`: a sound's analysis that takes under
  a second here took over 10 s there, and tests that carried on after a
  silent 10 s wait chopped half-analysed sounds. The Sample tab's waits
  (`gui::settle_views`, the tests' `tick_until`) share one long limit,
  `gui::WORKER_WAIT` (120 s), and panic with a message when it runs out.
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
