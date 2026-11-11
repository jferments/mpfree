# MPFree

A free, open-source music-production environment for the Akai MPC One.

MPFree aims to be an offline, hardware-first sampler, sequencer and arranger that runs on the MPC One's own Linux system. It keeps the tactile MPC workflow and adds flexible clip and timeline arrangement, polymeter, parameter locks, conditional triggers, better automation and a less menu-heavy UI. One design serves both beatmakers and live performers.

> **There is no installable MPFree yet.** This repository holds the engine prototypes, the hardware research and the tools used to gather it. Nothing here replaces the factory firmware.

## Status

Development happens on one MPC One running firmware 3.6.0, reached over a direct Ethernet link with root SSH.

| Milestone | What it proves | Where it ran |
|---|---|---|
| M0–M3 | Click, groove sequencer, WAV sampler and PCM network framing in Rust (C++ reference kept for parity) | Workstation and CI |
| M5 | A native ARMv7 Rust binary runs from RAM alongside the factory app | MPC One |
| M6 | Groove and sampler engine renders through a null audio backend (65× real time) | MPC One |
| M7 | 690 paced 128-frame callbacks, 0 synthetic deadline misses (p99 66 µs) | MPC One |
| M8 | 1280×800 test card and panel-rotation/touch transforms, offscreen | Workstation |
| M9–M10 | ILI2117 touch metadata: X/Y 0–2048, 10 multitouch slots, read-only | MPC One |
| GPU | Surfaceless EGL/GLES 3.1 on Panfrost (Mali-T760), offscreen | MPC One |
| M11 | Multitouch decoder, pointer mapping and gesture model | Workstation and CI |
| M12 | Live touch capture: panel orientation (swap X/Y, flip X), full-screen reach, 10 fingers, ~113 Hz reports; zero eMMC writes | MPC One |

Not done yet: real ALSA playback, on-screen display, pad/button/encoder input, and a proven recovery path for the factory system. The [roadmap](docs/project/roadmap.md) lists the gates, and the [development log](docs/project/development-log.md) has the history.

## Approach

- **Userspace first.** MPFree runs on top of the stock Linux system before any thought of a custom kernel, bootloader or installer.
- **Rust first,** with narrow C/C++ interfaces where mature libraries are needed ([ADR-0001](docs/decisions/adr-0001-rust-first.md)).
- **Direct ALSA** for audio on the device; JACK and PipeWire only as desktop backends ([ADR-0002](docs/decisions/adr-0002-direct-alsa-audio.md)).
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

CI runs all of this and more (clippy, the ARMv7 cross-build tested under qemu, the device tools against fakes) on every pull request, on self-hosted runners. See [Testing and CI](docs/guides/testing-and-ci.md) for what each job checks, the rules for tests, coverage, and the plan for the regression suite.

None of this touches the MPC. To work with a device, start with [developer access](docs/guides/developer-access.md) and then the [survey tools guide](docs/guides/survey-tools.md).

## Repository layout

| Path | Contents |
|---|---|
| `rust/` | Cargo workspace: engine crates (`mpfree-core`, `-groove`, `-sampler`, `-headless`, `-session`, `-network`), display and touch (`mpfree-display`, `mpfree-touch`), on-device smoke tests and the handoff model |
| `src/`, `tests/` | C++ reference engine kept for parity, and Python/C++ tests |
| `tools/mpc-one/` | Workstation scripts that survey or test the MPC over SSH, plus the GPU qualification kit |
| `docs/` | Architecture, decisions, milestone reports and hardware research; see the [docs index](docs/README.md) |

## Safety

- There is no MPFree installer. Root SSH via the community USB-autorun route is a research bootstrap for tested setups, not a general jailbreak.
- The MPC's root filesystem is read-only, but `/etc` and `/var` are persistent overlays, so careless edits can survive a reboot or stop the unit from starting.
- During research, do not repartition, flash, remount read-write or change the boot chain.
- Never commit raw device logs, private keys, factory firmware, sample content or disk images. Raw survey output goes to the git-ignored `research/` folder.

## Legal

MPFree will not distribute Akai/AIR firmware, plugins, presets or factory sample packs, and will not bypass activation. Compatibility with content a user legitimately owns may be researched separately ([policy](docs/project/factory-content-policy.md)). The factory system must always remain recoverable.

Licensed under [MIT](LICENSE). This covers MPFree's own code and documentation, not any Akai/AIR material or third-party projects referenced here. The fonts built into the GUI (Inter and JetBrains Mono) are under the SIL Open Font License 1.1; see [`rust/mpfree-ui/fonts`](rust/mpfree-ui/fonts/README.md).

## Related projects

- [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore)
- [TheKikGen/MPCLiveXplore-libs](https://github.com/TheKikGen/MPCLiveXplore-libs)

These are independent projects; check their licenses and hardware fit before reusing code.
