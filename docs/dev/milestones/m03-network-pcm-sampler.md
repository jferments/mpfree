# MPFree M3 — Rust network-audio protocol, bounded WAV sampler, ARM ABI preflight

**Status:** Implemented on the Rust host/workstation test path and passing automated build/tests. The factory MPC is untouched. This is **not** an ALSA backend, production network listener, MPC playback executable, or functioning factory-app handoff.

## 1. Rust network audio protocol

Source: `rust/mpfree-network/src/lib.rs`. The Rust implementation matches the earlier [Python MPF1 reference](../../../tools/mpc-one/host_audio_packet.py). A 28-byte big-endian header is followed by *interleaved S32_LE PCM payload*:

| Field | Width | Encoding |
| --- | ---: | --- |
| Magic `MPF1` | 4 bytes | ASCII |
| Version / format | 1 + 1 | `1` / `1=S32_LE` |
| Reserved flags | 2 | Must be zero |
| Stream ID / sequence | 4 + 4 | Big-endian unsigned |
| Start sample frame | 8 | Big-endian unsigned |
| Frames / channels | 2 + 2 | Big-endian unsigned |
| PCM | frames × channels × 4 | Little-endian signed-32-bit |

Total packet limit: **1,472 B UDP payload**, consistent with a standard IPv4 MTU 1500 without IP options. Maximum audio bytes: 1,444 B; 128-frame stereo audio occupies 1,024 B plus the 28-byte header. The parser rejects incorrect magic/version/flags, invalid or incomplete geometry, payload-length mismatch, and oversized packets.

`AudioPacket` exposes borrowed PCM bytes and encodes into a **caller-owned output buffer**. `BoundedPlayout` owns a setup-time allocated fixed-capacity ring with an upper bound of **64 blocks** and no allocation in `ingest` or `playout_into`. The receiver accepts out-of-order frames within the window, rejects duplicates/late/too-far-ahead packets, and writes exactly one block of **silence** on a missed deadline. Counters describe the outcome.

### Critical limitations

This is a **host-only deterministic transport model**:
- It opens **no UDP sockets** and does not move audio across real networks yet.
- It does not authenticate peers, control replay, validate session ownership, synchronize independent device clocks, or implement clock-drift compensation.
- Setup allocates slots. Incoming packet parsing/ingestion must eventually occur in a non-RT network thread, and a verified SPSC bridge must separate that thread from the audio render callback.
- An 8-block 128-frame jitter window at 44.1 kHz spans up to ~23.22 ms; **this is a capacity limit, not a measured or recommended buffer latency**.
- No production userspace plugin should be allowed to call systemd actions through this protocol.

See [measured Ethernet/USB hardware](../hardware/ethernet-usb.md): actual Ethernet link is **1 Gbit/s full-duplex**, short ping RTT averaged 0.272 ms, but **real audio network jitter and worst-case end-to-end delay are not measured**.

## 2. User-owned WAV sampler

Source: `rust/mpfree-sampler/src/lib.rs`. CLI: `rust/mpfree-sampler/src/main.rs`.

**Implemented:**
- Bounded, read-only, setup-time local WAV loading of uncompressed **RIFF WAVE** PCM **16/24/32-bit** or **IEEE float32**, mono or stereo. Parsed little-endian, with input header, size and block-alignment validation.
- Mono samples are promoted to stereo; nonfinite float32 input is silenced; values outside ±1 are clamped.
- **64 MiB maximum source file** and **64 MiB total prepared sample bank**. At most 128 samples and 5,000,000 frames per sample. If any budget is exceeded, the load fails closed; no streaming from the disk/audio thread.
- Sample rate must match the engine; the standalone CLI uses **44,100 Hz**. **No resampler yet**; attempts to load a 48-kHz file into the 44.1-kHz bank fail with an explicit error rather than silently playing at the wrong pitch.
- MIDI note → sample mapping, velocity, note-off with fixed 64-frame linear release, fixed 16-voice pool and basic 32-frame ramp on voice stealing, with prepared buffers retained by the bank.
- `Sampler::render` produces planar float stereo, does not explicitly allocate, lock or access filesystem, and clamps mixed output to ±1.
- Both new Rust crates use `#![forbid(unsafe_code)]` and the existing `mpfree-core::float_to_s32` to produce an offline 44.1-kHz signed-32-bit WAV.

**Not implemented:** streaming large samples, multi-sample velocity layers, playback-rate/pitch changes, looping, multisample/sfz files, 24-bit-in-32 WAV extensible headers, RF64, sample-rate conversion, envelopes other than basic note release, declick-quality assessment, effects and panning, all-in-one soundfont/SFZ instruments or MIDI device input. Do not call this a production sampler.

### Try your own WAV without touching the MPC

The input must be an uncompressed 44,100 Hz mono/stereo RIFF WAV in one of the formats above.

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-sampler

# Replace the input with any local, user-owned 44.1-kHz compatible WAV:
./rust/target/release/mpfree-sampler \
  /path/to/your/sample.wav /tmp/mpfree-sampler-preview.wav
file /tmp/mpfree-sampler-preview.wav
xdg-open /tmp/mpfree-sampler-preview.wav
~~~

The demo triggers MIDI note 60 with velocity 110, renders **2 seconds** to host-local stereo S32_LE WAV, and sends note-off at exactly frame **44,100** (1 second), using a short release fade. Short samples may finish naturally earlier. It does **not** talk to the MPC or contact the network. It is strictly an independent host test.

CI generates its own short WAV sine fixture and validates the sampler output, avoiding any proprietary or owner-specific assets.

## 3. ARMv7 ABI and reversible handoff preparation

These are separate **read-only hardware surveys**, not actions that stop stock services or run MPFree on MPC:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519

# Investigate actual az0x glibc/interpreter and installed ALSA/DRM/EGL libs:
bash tools/mpc-one/collect-over-ssh.sh --abi

# Investigate factory systemd service/dependency map and resource holders:
bash tools/mpc-one/collect-over-ssh.sh --handoff-preflight
~~~

The reports are saved under local gitignored `research/`. They **have not been run against the MPC yet** and may contain private device/network/boot metadata. Review/redact before sharing; never commit raw logs.

Follow-up host-only ELF comparison after `--abi`:
- [ARMv7 userspace compatibility and checker instructions](../research/armv7-userspace-compat.md), using `tools/mpc-one/check-arm-abi.py`.
- [Reversible handoff design and hard recovery gates](../research/reversible-handoff.md).

**A successful Linux armv7 cross-build is not yet a compatible MPC build.** The target GNU libc version, dynamic loader, installed dependencies and safe binary staging remain unverified. **We have not demonstrated a reversible handoff**, and the recovery-gate remains blocking for any stock service stop or MPFree PCM/DRM/MIDI takeover.

## 4. Validation and milestones

| Milestone | Current state |
| --- | --- |
| M0/M1 metronome and C++↔Rust WAV parity | Completed, user+CI verified |
| M2 16-step Rust groove engine, 16 voices | Completed, owner confirmed audible desktop demo |
| M3 Rust MPF1 packet encoding, bounded playout | Implemented; unit-tested, *no live sockets* |
| M3 Rust WAV sampler with local sample input and bounded memory | Implemented; unit-tested, CI-generated fixture |
| M3 cross-build all new Rust code for ARMv7 gnueabihf | CI builds; target-runtime compatibility pending |
| MPC az0x loader and libc version discovery | New read-only survey; awaiting owner run |
| Factory service reversible handoff | Safety/runbook and read-only inventory; **live handoff blocked** |
| Networked DSP exchange with laptop | Planned (host-only UDP loopback and clock-drift simulation first) |
| Playing samples on MPC ACVA | **Blocked** by independent recovery and stock-service handoff |

## 5. Engineering priorities after this merge

1. Decode the read-only ABI and handoff reports from the actual target, preserving report SHA-256 and documenting redacted evidence; no silent assumptions about model support.
2. Build a reproducible target-compatible ARMv7 sysroot with open sources; test a tiny nonpersistent *harmless* binary only after a documented, separately approved safety review.
3. Give the sampler a typed timestamped event queue and dynamically prepared pad mappings; keep disk loading off the audio thread, enforce total sample-bank budgets.
4. Build **host-to-host UDP loopback** to measure the actual MPF1 sender/receiver, lost/deadline packets, and clock drift in controlled conditions.
5. Simulate factory stop/restore and watchdog behavior in a fake systemd environment and prove independent recovery before any real-device handoff.

The long-term goal remains an entirely new open-source MPFree userspace over the MPC One's existing kernel and hardware drivers, with optional user-owned factory-content import utilities and a separately validated single-USB installer.
