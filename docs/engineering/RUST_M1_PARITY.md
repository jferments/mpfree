# Rust M1 — first MPFree-owned core, desktop parity and ARMv7 cross-build

**Decision:** [ADR-0001 — ACCEPTED Rust-first, C/C++-compatible architecture](ADR-0001-RUST-FIRST-HYBRID.md), approved by the owner on 2026-11-08 Pacific.

**Current stage:** Rust metronome core and offline WAV renderer implemented. Host native unit tests and C++/Rust WAV comparison pass in CI. ARMv7-gnueabihf cross-linking requires the cross-compiler **and** its target libc development/startup objects; its result is tracked separately. **Nothing has been executed or installed on the MPC.**

## Why preserve the C++ implementation?

The initial C++ `mpfree-core` and `mpfree-render` were verified both by CI and by a physical developer workstation test: a four-second, 120 BPM WAV with 176,400 frames and eight beat triggers was audibly played. They are still checked by the existing CMake/CTest CI job.

Keeping them temporarily gives an independent oracle for Rust:
- Exact frame counts and beat-trigger positions at 120, 109 and 137.2 BPM.
- Output at 44,100 Hz stereo S32_LE, matching the **observed** stock ACVA stream container format.
- Bounded numerical differences in audio samples (libm `sin` and `exp` can vary slightly by toolchain), not bit-for-bit WAV identity.
- Same control-flow corner cases: invalid tempo/configuration, repeatability across arbitrary audio block boundaries, PCM clipping and non-finite input silence.

We are **not** committing to preserving C++ in the future MPFree DSP graph; we are preserving working evidence until Rust meets its contracts.

## Workspace layout

```text
rust/
  Cargo.toml                       # independent std-only workspace
  Cargo.lock                       # committed lockfile for reproducibility
  mpfree-core/
    Cargo.toml
    src/lib.rs                     # ClickEngine, ClickConfig, float_to_s32
  mpfree-render/
    Cargo.toml
    src/main.rs                    # host-only PCM WAV generator

tests/check_cpp_rust_wav_parity.py # compares compiled C++ and Rust binaries
.github/workflows/mpc-inventory.yml
                                  # native Rust debug+release, C++ parity,
                                  # ARMv7 hard-float cross-link and ELF checks
```

**Zero third-party crate dependencies** in M1. The Rust standard library is appropriate for this milestone; ALSA, DRM, MIDI, CLAP, audio decoders, UI and actual network sockets are not yet included.

## Audio/render contract

The Rust metronome precomputes a short waveform at configuration time. Its `render(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<...>` processes two same-length planar buffers.

**In render():**
- No heap allocation, `std::fs` calls, `std::net` calls, mutex, logging or `sin/exp`.
- Transport advances by sample frames, with absolute beat positioning rather than repeatedly adding a rounded integer period.
- Channel geometry mismatch returns a typed error and leaves transport unchanged.
- The S32 conversion saturates outside [-1,1], rounds within it and maps NaN/infinity to silence.

**Outside render():** non-RT `configure()` validates tempo and precomputes a 512-sample maximum click wavetable; the desktop WAV writer interleaves channels and writes little-endian signed 32-bit PCM.

Rust's memory safety does not guarantee worst-case runtime latency: future PCM callbacks will still require allocator detection, bounded workloads, memory locking where appropriate, backpressure, system-call avoidance and on-device xrun measurements.

## Developer commands — ML-tower only

**Precondition:** a recent Rust stable toolchain with Cargo installed. Check with `rustc --version` and `cargo --version`. Use the official Rust installation instructions if absent: https://www.rust-lang.org/tools/install .

```sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey

# Original C++ reference:
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel 4
ctest --test-dir build --output-on-failure

# Rust std-only workspace:
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo test --manifest-path rust/Cargo.toml --workspace --release --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-render

# Compare C++ vs Rust across 120, 109 and 137.2 BPM:
python3 tests/check_cpp_rust_wav_parity.py \
  --cpp build/mpfree-render \
  --rust rust/target/release/mpfree-render

# Create another WAV using the Rust executable only:
./rust/target/release/mpfree-render /tmp/mpfree-rust-metronome.wav 4 120
file /tmp/mpfree-rust-metronome.wav
xdg-open /tmp/mpfree-rust-metronome.wav
```

**No connection to the MPC is required.** The output WAV lives on ML-tower and can be listened to normally.

### ARMv7 compile and ELF inspection (on developer workstation)

```sh
rustup target add armv7-unknown-linux-gnueabihf

# On Ubuntu, install the cross compiler PLUS its target libc startup files.
sudo apt-get update
sudo apt-get install --no-install-recommends \
  gcc-arm-linux-gnueabihf binutils-arm-linux-gnueabihf libc6-dev-armhf-cross

CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-render

arm-linux-gnueabihf-readelf -h \
  rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-render
arm-linux-gnueabihf-readelf -l \
  rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-render
```

Expected architecture: `ELF32`, `EM_ARM`, EABI5 hard-float, consistent with the factory executable. **This does not establish that the linked runner-image glibc version, `PT_INTERP`, kernel ABI, sysroot, or dependencies are compatible with the MPC's az0x firmware. Do not copy/run this build on the MPC yet.** CI cross-builds with the GitHub runner's Ubuntu sysroot, not a verified target-image sysroot.

## CI gates

- Existing shell, reference USB safety checks and Python packet tests: unchanged.
- Existing C++ metronome + RIFF/PCM WAV smoke: unchanged.
- New Rust workspace: debug and release unit tests; cargo locked build.
- Cross-language parity: three tempos; exact PCM WAV header/data geometry, frames, beat counts, stereo symmetry, and ≤16,384 integer PCM units maximum sample difference. This corresponds to ~7.63×10⁻⁶ full-scale amplitude and intentionally tolerates tiny libm differences, **not** beat misalignment.
- ARMv7: cargo cross-compile `--target armv7-unknown-linux-gnueabihf` and ELF-header assertions. The first attempt lacked ARM C runtime `Scrt1.o` and `crti.o`; the CI setup now explicitly installs `libc6-dev-armhf-cross`.

**No artifact uploads or proprietary SDK usage.** Build products are generated in local ignored `rust/target/`, and GitHub CI does not archive them.

## Next Rust-owned work

1. Extract typed sample-frame and MIDI event representations from the click-specific engine into a small deterministic transport/sequencer crate.
2. Port the **host-only** network PCM packet format and bounded jitter/packet-drop simulation from Python to Rust, with property tests against existing Python vectors. Keep hardware/instrument acquisition out of this stage.
3. Establish a reproducible armv7 hard-float sysroot compatible with az0x: verify glibc symbols and loader path before on-device staging. No permanent installation.
4. Define minimal C ABI adapters for external C/C++ synth engines; no unsafe foreign-library code in the central render graph.
5. Later prototype the actual MPC ALSA HAL **only after** a proven rollback/resource-handoff plan and explicit approval. The factory `acvs.service` currently owns the PCM endpoints and has `Restart=always`.

**Safety boundary:** Rust-first does not imply flashing Rust firmware or replacing the vendor kernel. MPFree remains an independent userspace application on the existing hardware.
