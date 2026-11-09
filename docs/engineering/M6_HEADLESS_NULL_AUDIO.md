# M6: independent headless MPFree engine with null audio

**Status:** Host-only integration written, physical MPC execution of **M6** not yet attempted. Original **M5** Rust on-target smoke is **proven successful** (owner-run, UTC 2026-11-09). The factory stock application remains untouched and owns physical audio/MIDI/display/input. This M6 implementation does **not** include a persistent daemon, an audio driver or a factory handoff.

## M5 completed on original MPC One — actual owner evidence

The first ARMv7 smoke test ran from a unique temporary directory under verified /run tmpfs using guarded Ethernet/SSH. The original Akai app remained active, as verified before and after execution.

- **Live target:** original Akai MPC One, az0x 5.0.8, glibc 2.39, ARM ELF32 EABI5 hard-float.
- **Executable:** 531,960 bytes, max GLIBC requirement 2.34, libraries libc.so.6/libgcc_s.so.1/libm.so.6.
- **Local binary SHA-256:** `65f2c2a10a6421b40561b0d0c868be600290a10ae8fb4e616bb0ca2e803bd1fe`
- **Host-side private report SHA-256:** `776de1db00440dc69e34260ecef9753ec97559dd1e742545a04477eaadf9a07d`.
- **On-target result:** `MPFREE_ARMV7_SMOKE_V1`, `os=linux`, `arch=arm`, `sample_rate_hz=44100`, `frames=4096`, `beats=1`, `nonzero_samples=153`, `checksum_fnv1a64=295b5aeef41825c6`, `status=PASS`.
- **Safety evidence:** `MPFREE_VOLATILE_STAGE_SHA256_OK`, `STAGE=RUST_EXECUTED`, `WATCHDOG=RUST_5S_ARMED`; launcher reported removal of the temporary file. Verification was done with factory app active. **This proves a short-lived application-space coexistence computation, not physical audio playback or worst-case real-time safety.**
- The full raw reports remain only under the owner's ignored `research/` folder. No device MAC, serial numbers, or raw session logs are committed to GitHub.

## M6 architecture

~~~text
              MPFree headless Rust binary
                        |
          finite CLI self-test + 5s watchdog
                        |
                HeadlessSession
               /       |       \
        GrooveEngine  Sampler    audio sink trait
         16 steps      WAV PCM   (AudioSink::write)
               \       /               |
                per-128-frame mix --> NullAudioBackend
                                        |
                     PCM checks, peak, frames, FNV checksum
                                        |
                                stdout metrics only
~~~

- A **real** `GrooveEngine` runs the default 16-step synthesized kick/snare/hats pattern, 120 BPM, 44.1 kHz for one bar (2.0 seconds of *audio frames*).
- The existing **WAV parser** and `Sampler` render a newly generated **synthetic PCM16 sample**, prepared once **outside render** and mapped to MIDI note 60. A note is triggered at each exact 22,050-frame interval (0, 0.5, 1.0, 1.5 seconds). MIDI/input hardware is never accessed.
- `HeadlessSession::render` handles variable callback sizes and splits blocks at sample-exact note timestamps; all audio buffers are allocated before entering the render path.
- `NullAudioBackend` accepts stereo float slices, validates finite values, counts frames/nonzero samples, tracks peak amplitude and hashes quantized stereo S32_LE samples. **It does not open any audio device, write a WAV, network socket or file, or acquire physical controller/display resources**.
- The executable's five-second Rust watchdog is for this **finite computation only**. There is no persistent daemon, async event loop, remote control API, production watchdog or real-time scheduler in M6. CPU performance measured for this offline render must not be interpreted as proven xrun-free hardware playback.

## Host build/test (no MPC involved)

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo run --manifest-path rust/Cargo.toml --release --locked \
  -p mpfree-headless -- --self-test
~~~

Expected key fields:

~~~text
MPFREE_HEADLESS_V1
os=linux
arch=x86_64
backend=null
sample_rate_hz=44100
frames=88200
groove_events=12
sampler_notes=4
groove_voice_steals=0
sampler_voice_steals=0
...
status=PASS
~~~

The process emits `MPFREE_HEADLESS_RUST_WATCHDOG_5S_ARMED` on stderr. The exact checksum and performance time are measured, not predetermined. Unit tests include variable-block partition equivalence, invalid buffer lengths, nonfinite PCM rejection and event counts.

## ARM cross-build (host-only)

~~~bash
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-headless

file rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-headless
~~~

CI validates ARM EABI5 hard-float ELF32, `/lib/ld-linux-armhf.so.3` and glibc requirements ≤2.39. **Cross-building M6 does not mean it has executed on the physical MPC**.

### Physical MPC acceptance must be a separate stage

The existing `--ram-smoke` launcher hardcodes `mpfree-device-smoke`, its output markers and watchdog string. It **must not** be repurposed via renaming the M6 binary or supplying arbitrary shell commands. A future M6-specific allowlisted mode can reuse its validated protections: exact device/SSH/ABI identity, live stock service, /run tmpfs, SHA-256, normal scheduling, POSIX CPU cap, explicit owner opt-in, remote stage markers, cleanup and independent host timeout.

Until that review is complete **do not attempt to upload or execute `mpfree-headless` on the MPC**. Host CI does not connect to or mutate the MPC.

## Road ahead

1. Validate the bounded M6 integration and cross-compile against ARMv7 (this PR).
2. Add a specifically qualified, opt-in and reversible **M6 on-device null-backend launcher**, then measure the device's runtime, RSS/PSS and CPU-time while stock runs.
3. Introduce a separately supervised, longer-lived process architecture: command queue/event loop, safety limits, null audio and explicit stop. Non-RT file/network loading stays outside the render path. Minimize privileges; avoid root for general application logic.
4. Design device HAL adapters (ALSA, MIDI, DRM/touch, pad controls) **behind hardware-ownership gates**. Do not open factory-owned endpoints or stop `acvs.service` until independent factory restoration has been verified.
5. Only in the long term explore our own MPC-specific Linux **distribution**, preserving proven recovery and current compatible kernel/drivers unless clear engineering evidence supports replacements.

**Core user goal:** replace the factory music application with MPFree — first coexistence, later a validated reversible handoff. No need to replace Linux itself.
