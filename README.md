# MPFree

A free, open-source music-production environment for the Akai MPC One.

MPFree aims to be an offline, hardware-first sampler, sequencer and arranger that runs on the MPC One's own Linux system. It keeps the tactile MPC workflow and adds flexible clip and timeline arrangement, polymeter, parameter locks, conditional triggers, better automation and a less menu-heavy UI. One design serves both beatmakers and live performers.

![MPFree's Play page in the middle of a jam: a three-layer loop playing, a beat in the pattern, the Stutter throw held and the master meter moving.](docs/images/play-all-loop.png)

> **There is no installable MPFree yet.** MPFree runs from a test USB stick that boots the MPC One into RAM with its internal storage hidden, so the factory system is never changed. This repository holds the app, its engines, the hardware research and the tools used to gather it.

## Documentation

- **Making music with MPFree:** the [user manual](docs/user/README.md), built as a website and a PDF. It starts from zero and notes where MPFree differs from the factory MPC software.
- **Building MPFree:** the [developer docs](docs/dev/README.md): architecture, a design page per engine, decisions, testing, device guides and hardware research.
- **AI agents working here:** [docs/agents](docs/agents/README.md) and [CLAUDE.md](CLAUDE.md).

## Status

Development happens on one MPC One running firmware 3.6.0, reached over a direct Ethernet link with root SSH.

**The app.** The pad player (`mpfree-app`, the program `mpfree`) is MPFree's first app. It runs on the MPC from the test stick with its own touchscreen GUI, the pads and their lights, the hardware buttons, and audio in and out through ALSA. Its tabs are Play, Pads, Arrange, Synth, Looper and Sample, with the mixer, the browser and Prefs opening over any page. On `main` today:

- **Playing:** drum kits on the pads with Full and 16 Levels, Note Repeat, Latch and an arpeggiator, the Play page's pattern, throws and XY pad (Perform FX), FILL and TRACK MUTE.
- **Time:** the transport (tap tempo, swing, following another device's MIDI clock), clips of any length, the launch grid with scenes and follow actions and up to 128 tracks, the step sequencer and note editor (piano roll, list, tracker, Timing Correct), Capture, and the Song view with Rec song.
- **Audio:** an eight-loop audio looper with layers, undo history and time stretch; audio tracks on the Song view's lanes, with take lanes for comping.
- **The sampler:** the Rec page (thresholds, Skip back, resampling the master or any channel), Wave, Warp, Chop, live chop, Play, Grain and FX pages, and the Pads tab's browser for loading sounds onto pads.
- **Sound:** nine native instruments (Poly, FM, Drums, Pluck, Wavetable, the Keys electric piano, the Drawbar organ, the Squelch acid bass and the Analog poly) on the Synth tab or as any track's own instrument; a native effect in place of every MPC effect, in per-channel insert chains, with macros, XY FX and the mixer with its I/O view; CLAP plugin effects and instruments, each run in its own sandboxed process, and a plugin pack of free plugins built for the MPC (Airwindows, Dexed, Surge XT).
- **MIDI:** keyboards on the 5-pin MIDI IN, MIDI Learn, clock in and out, MIDI tracks and CV/gate out (MIDI out and CV out are behind flags until their device round).
- **Projects:** New, Save and Open from the browser (BROWSE, SHIFT + BROWSE to save), with every engine's state, the samples and the audio takes saved in the project folder and the last ten saves kept; autosave every two minutes, and a safe start after a crash.
- **Export:** the song as a mixdown, as stems and as a MIDI file.
- **Import (on a computer):** an MPC keygroup program becomes an MPFree keygroup kit (`mpfree-mpc-import`, stage 1 of [MPC import](docs/dev/design/mpc-import.md)).

The main runs on the device so far are round 7 (2026-11-11: the looper from the pads and the line input with no underruns, about 8.7 ms from pad to sound) and live test 3 (2026-11-13: the app check's first five phases and the benchmarks, with 0 sectors written to the boot partitions). Most of the list above is built and tested on the workstation and in CI (unit tests, golden screens and GUI smoke tests) but not yet tried on the MPC; the [guide to the app on the test build](docs/dev/guides/mpfree-app.md) says what has run where, and each [design page](docs/dev/README.md#design) carries its own status.

**The foundations** (earlier milestones):

| Milestone | What it proves | Where it ran |
|---|---|---|
| M0–M3 | Click, groove sequencer, WAV sampler and PCM network framing in Rust (C++ reference kept for parity) | Workstation and CI |
| M5 | A native ARMv7 Rust binary runs from RAM alongside the factory app | MPC One |
| M6 | Groove and sampler engine renders through a null audio backend (65× real time) | MPC One |
| M7 | 690 paced 128-frame callbacks, 0 synthetic deadline misses (p99 66 µs) | MPC One |
| M8 | 1280×800 test card and panel-rotation/touch transforms, offscreen | Workstation |
| M9–M10 | ILI2117 touch metadata: X/Y 0–2048, 10 multitouch slots, read-only | MPC One |
| GPU | Surfaceless EGL/GLES 3.1 on Panfrost (Mali-T760), offscreen | MPC One |
| M11–M16 | Multitouch decoder, live touch capture and orientation, gestures, the Touch lab and touch calibration | MPC One, workstation and CI |

Not done yet: an installer, a proven recovery path from an unbootable unit (MaskROM), and replacing the factory app on the unit's own storage. The [roadmap](docs/dev/project/roadmap.md) lists the gates, and the [development log](docs/dev/project/development-log.md) has the history.

## Approach

- **Userspace first.** MPFree runs on top of the stock Linux system before any thought of a custom kernel, bootloader or installer.
- **Rust first,** with narrow C/C++ interfaces where mature libraries are needed ([ADR-0001](docs/dev/decisions/adr-0001-rust-first.md)).
- **Direct ALSA** for audio on the device; JACK and PipeWire only as desktop backends ([ADR-0002](docs/dev/decisions/adr-0002-direct-alsa-audio.md)).
- **Reversible by default.** Every on-device test runs from RAM, is opt-in, and leaves the factory app running.

## Build and test

On a Linux workstation with stable Rust and CMake:

~~~sh
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo test --manifest-path rust/Cargo.toml --workspace --release --all-targets --locked

# C++ reference engine and Rust/C++ WAV parity check
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel 4
ctest --test-dir build --output-on-failure
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-render
python3 tests/check_cpp_rust_wav_parity.py --cpp build/mpfree-render --rust rust/target/release/mpfree-render
~~~

CI runs all of this and more (clippy, the ARMv7 cross-build tested under qemu, the device tools against fakes) on every pull request, on self-hosted runners. See [Testing and CI](docs/dev/guides/testing-and-ci.md) for what each job checks, the rules for tests, coverage, and the plan for the regression suite.

None of this touches the MPC. To work with a device, start with [developer access](docs/dev/guides/developer-access.md), the [survey tools guide](docs/dev/guides/survey-tools.md) and the [guide to the app on the test build](docs/dev/guides/mpfree-app.md). To build the user manual, see [docs/user](docs/user/README.md).

## Repository layout

| Path | Contents |
|---|---|
| `rust/` | Cargo workspace: the app (`mpfree-app`) and its GUI toolkit (`mpfree-ui`); the engines (transport, sequencer, playback, looper, audio tracks, sampler, synths, effects and mixer, modulation, arp, MIDI) and their screens; the library and browser; the CLAP host and plugin sandbox; touch, display and GPU; profiling and the bench; on-device smoke tests and the handoff model |
| `src/`, `tests/` | C++ reference engine kept for parity, and Python/C++ tests |
| `tools/mpc-one/` | Workstation scripts that survey or test the MPC over SSH, plus the GPU qualification kit |
| `docs/` | `user/` the user manual, `dev/` developer docs, `agents/` rules for AI agents, `tools/` the docs checks and manual build, `images/` the screenshot above (redrawn by the manual build); see the [docs index](docs/README.md) |

## Safety

- There is no MPFree installer. Root SSH via the community USB-autorun route is a research bootstrap for tested setups, not a general jailbreak.
- The MPC's root filesystem is read-only, but `/etc` and `/var` are persistent overlays, so careless edits can survive a reboot or stop the unit from starting.
- During research, do not repartition, flash, remount read-write or change the boot chain.
- Never commit raw device logs, private keys, factory firmware, sample content or disk images. Raw survey output goes to the git-ignored `research/` folder.

## Legal

MPFree will not distribute Akai/AIR firmware, plugins, presets or factory sample packs, and will not bypass activation. Compatibility with content a user legitimately owns may be researched separately ([policy](docs/dev/project/factory-content-policy.md)). The factory system must always remain recoverable.

Licensed under [MIT](LICENSE). This covers MPFree's own code and documentation, not any Akai/AIR material or third-party projects referenced here. The fonts built into the GUI (Inter and JetBrains Mono) are under the SIL Open Font License 1.1; see [`rust/mpfree-ui/fonts`](rust/mpfree-ui/fonts/README.md).

## Related projects

- [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore)
- [TheKikGen/MPCLiveXplore-libs](https://github.com/TheKikGen/MPCLiveXplore-libs)

These are independent projects; check their licenses and hardware fit before reusing code.
