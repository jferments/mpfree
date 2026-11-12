# Roadmap and acceptance gates

This is a proposal, not a promise of parity or a timeline.

**Status (2026-11-11):** the milestone list below was last brought up to date at M12 (2026-11-10). Since then the pad player app has run on the MPC from the test stick through round 7 (touch GUI, pads, ALSA audio out, line input, looper, sampler, synths); the [README](../../../README.md#status) has the current summary and each [design page](../README.md#design) has its own status. The gates below (recovery tiers, no eMMC writes) still apply.

## Incremental host-only progress (not Phase 1/2 MPC acceptance)

- [x] **M0:** deterministic C++ sample-frame click and 44.1-kHz stereo S32_LE WAV; owner confirmed audible desktop playback.
- [x] **M1:** Rust-first metronome with native debug/release tests, three-tempo **zero-difference** WAV parity against C++, and ARMv7 EABI5 hard-float CI **cross-build**. This is not a runtime compatibility or MPC sound-device test.
- [x] **M2:** Rust 16-step pattern compiler, timestamped note events, synthetic kick/snare/hat source bank and bounded 16-voice playback; desktop-only generated groove WAV and CI validation.
- [x] **M3 host:** Rust `MPF1` framed PCM codec and bounded playout model ported from Python (no network sockets); Rust WAV PCM16/24/32/float32 sampler (note mapping, release envelope, 16-voice limit and 64 MiB bank cap); CI unit/synthetic WAV smoke.
- [x] **M3 safety tools:** read-only ABI/loader/library inspection `--abi`, read-only factory service/device ownership `--handoff-preflight`, and host-only GLIBC/ELF comparator.
- [x] **M3/M4 MPC observations:** owner successfully ran the revised `--abi` and `--rtthreads`: embedded GNU libc release banner explicitly confirms **2.39**; the factory has **38 threads across cores 0–3**, including audio processing on CPU3, ~1,016 MiB locked/1,019 MiB resident memory and exclusive ACVA PCM ownership. Core affinity is not CPU utilization.
- [x] **M4 host-only safety:** no-unsafe pure Rust handoff/watchdog state-machine simulation with gates for recovery/owner consent; CI tests crash/cancel/timeout/restore failure. **No actual systemd or device-control backend exists.**
- [ ] **M4 MPC next passive probes:** re-run fixed `--rtthreads` to read per-task SCHED policy/RT priority from `/proc/task/stat`; run two-second read-only `--capacity` at idle and under ordinary playback to quantify real CPU/memory pressure.
- [ ] **M4 actual recovery:** independent eMMC/boot restore **unproven**, real stock unit stop/restart/handoff prohibited until separately owner-approved and simulated.
- [x] **M5 host build/tooling:** independent safe Rust `mpfree-device-smoke` (4,096 in-memory frames, zero device opens), live MPC ABI/ELF checks and opt-in `/run`-only staging with SHA-256, Rust wall watchdog, CPU-time limit and cleanup. **Host CI and owner-run physical MPC execution both passed; this is not audio-hardware playback.**
- [x] **M5 actual ARM execution:** owner successfully ran guarded Rust ELF on physical MPC One, 4,096 frames, 1 beat, SHA256 verified, `status=PASS`, factory app active at pre/post checks, RAM-only cleanup reported; private raw logs excluded from GitHub.
- [x] **M6 integrated engine:** `mpfree-headless` combines real GrooveEngine + WAV Sampler with preallocated 128-frame mixing and `NullAudioBackend`; deterministic one-bar PCM hash and ARMv7 code now **verified on physical MPC**. Long-running event loop and production daemon remain unimplemented.
- [x] **M6 on-device acceptance:** owner ran `--headless-check` and double-opt-in `--headless-smoke` successfully on MPC One with stock app active: 88,200 frames, 12 groove events, 4 sampler notes, exact PCM hash `3dc4d6dabbb17fbd`, 30,565 µs elapsed (65.43× offline throughput), 1,792 KiB RSS/peak RSS, SHA-256 verified and cleanup reported. This proves null-audio coexistence computation, **not** hardware playback.
- [x] **M7 host foundation:** `mpfree-session` runs finite 128-frame synthetic wall-paced null-audio callbacks; separately launched HOST supervisor enforces timeout/cancel and output integrity. Physical M7 execution is now separately qualified and owner-validated below.
- [x] **M7 host validation:** owner's workstation passed all Rust tests, 690 paced callbacks, 0 synthetic deadline misses, p99=10µs/max=28µs, max wakeup lateness=81µs, PCM checksum unchanged. Not physical ALSA/xrun proof.
- [x] **M7 physical MPC acceptance:** owner ran `--session-check` and separately dual-opted-in to `--session-smoke` on the original MPC One. **PASS**: 690 paced callbacks at 128f/44.1k, 0 synthetic deadline misses, p99=66µs/max=165µs, max wake lateness=167µs, render total=26.859ms/2s, exact PCM hash; SHA256 guard and `/run` cleanup verified. **No ALSA, PCM opens or real driver deadlines tested**.
- [x] **GUI/handoff owner-run discovery:** read-only `--process-map` and `--touchscreen` both passed on original MPC One. Confirmed `DeviceControlServer.bin` independent in network MIDI service, separate power service reading `KEY_POWER` on event1, main MPC process owning DRM and touch/PCM/raw MIDI, direct ILI2117 I²C4/0x26 kernel driver, multitouch slots/tracking IDs, 800×1280 DSI and Panfrost GPU.
- [x] **M8 host-only GUI rehearsal:** new std-only `mpfree-display` Rust module creates distinct-color four-corner test card and checked RGB888 1280×800 logical → 800×1280 native CW/CCW previews with range-supplied touch coordinate transforms; no DRM/evdev I/O.
- [x] **M8 owner preview:** owner ran 40 native Rust workspace tests and confirmed 1280×800 logical and 800×1280 CW/CCW PPM generation with all tests passing; visual orientation on MPC remains unknown.
- [x] **M9 metadata-only host foundation:** new isolated Rust Linux EVIOCGABS bridge can read ABS_X/Y, MT_SLOT/POSITION_X/Y/TRACKING_ID capability min/max/slot count *after exact ILI2117 sysfs matching and local explicit opt-in*. No device deployment, event consumption, grabs, or systemd changes. Actual axis values still unmeasured.
- [x] **M10 physical touchscreen metadata PASSED (2026-11-09):** owner ran `--touch-axes-check` and double-opt-in `--touch-axes` on original MPC One. Actual ILI2117 `ABS_X/Y` and `ABS_MT_POSITION_X/Y` **0..=2048**, `ABS_MT_SLOT=0..=9` (**10 reported slots**), tracking IDs `0..=65535`; all fuzz/flat/resolution 0. Checked read-only fd, no event consumption/grab, ARM ELF+SHA/watchdog and active stock unit afterward. **Rotation, actual ten-finger reliability, and stock UI functionality after test remain unverified.**
- [x] **M10 owner hardware execution completed:** read-only preflight and volatile metadata query both returned PASS. Host-only M8 mapping regression now covers observed inclusive 0..=2048 range under both CW/CCW candidates; physical orientation **unknown**. See `docs/dev/milestones/m10-validation-record.md`.
- [ ] **M10 post-run functional confirmation:** owner manually verifies factory GUI, touchscreen, pads and power controls still behave normally (not established by service-active check alone).
- [x] **M11 host-only multitouch:** `mpfree-touch` gains a Type-B decoder (slots, tracking ids, SYN_REPORT frames, SYN_DROPPED resync with or without an `EVIOCGMTSLOTS` snapshot), an eight-orientation raw-to-logical pointer map and a toolkit-neutral tap/drag/long-press/two-finger recognizer, tested against synthetic streams. No device reads. See [M11](../milestones/m11-multitouch-decoder.md).
- [x] **M12 tooling:** read-only, non-grabbing, RAM-only ILI2117 capture paced by the owner (Enter, countdown, 9 s per step) (`mpfree-touch-capture`, no ioctl import), guarded `--touch-capture-check`/`--touch-capture` wrapper modes with eMMC p1–p6 write-counter checks, and a host analyzer that matches the touch protocol and picks the orientation. See [M12](../milestones/m12-touch-capture.md).
- [x] **M12 owner run:** capture on the physical MPC and analyzer PASS (2026-11-10; orientation swap X/Y + flip X).
- [x] **M12 profile:** reviewed touch profile in [`docs/dev/hardware/data/mpc-one-touch-profile.json`](../hardware/data/mpc-one-touch-profile.json).
- [x] **M13-M16 touch in the pad player** (workstation and CI): live non-grabbing reader with slot seeding and resync ([M13](../milestones/m13-live-touch-reader.md)), release velocity, swipes and flings ([M14](../milestones/m14-gestures.md)), the Touch lab ([M15](../milestones/m15-touch-lab.md)) and Calibrate touch ([M16](../milestones/m16-calibrate-touch.md)), both in Prefs > Touch screen.
- [x] **M17 touch tuning** (workstation and CI, #145): Dev > Touch measures fingers, fits a position correction to nine target taps, suggests a hold filter and gesture thresholds, and saves them in the store ([M17](../milestones/m17-touch-tuning.md)).
- [ ] **Touch on the device:** run Dev > Touch on the MPC (Targets, Swipes, a few holds), then Use suggested and Save. Have the app's widgets with fixed thresholds (wave TAP_SLOP, Q-Link detent) adopt the saved gesture values.
- [ ] **Later:** full factory GUI stop/restore remains blocked until the Tier 1 recovery gate passes.
- [ ] **Future experimental screen takeover:** requires the Tier 1 recovery gate and a separate supervised, owner-consented stock-app stop → timed DRM test-card → verified stock-app restore. Not implemented or authorized in current work.
- [x] Read-only mapping of Rockchip hardware, ACVA ALSA buffer settings, MIDI graph, storage layout, 1-Gbit Ethernet PHY, USB 2.0 high-speed port and Linux DWC2 device controller.
- [ ] Target-image-compatible ARMv7 sysroot and tested reversible **nonpersistent** custom executable launch.
- [ ] **Tiered recovery gate** (adopted by the owner 2026-11-09, replacing the single "unbootable-MPC recovery" gate; see [recovery and OS strategy](../research/recovery-and-os-strategy.md)). Tier 1, a RAM-only session: verified full eMMC backup, zero writes to `p1`–`p6`, stub stop → start rehearsal. Tier 2, persistent MPFree files on SD or `/data`: adds Akai Update Mode and `/data` restore rehearsals. Tier 3, anything boot-critical: adds MaskROM recovery proven on this unit. Volatile watchdog and tested service restoration are still required for any takeover.
- [x] Full eMMC backup over SSH, the first Tier 1 condition: read-only [`--backup`](../hardware/emmc-backup.md). Owner ran it on 2026-11-09: verified, boot partitions match the boot map, second copy verified on another disk.
- [x] **Persistent SSH** (owner-built, kept by owner decision 2026-11-09): early-boot `mpfree-sshd.service` in rootfs, keys under `/data/mpfree/ssh`; recorded as MOD-001/MOD-002 in [device modifications](../hardware/device-modifications.md).
- [x] Read-only `--boot-map` survey, owner-run twice on 2026-11-09: eMMC layout, U-Boot environment and boot flow, boot-partition hashes, zero writes to `p1`–`p6`, kexec support. Akai's Update Mode is U-Boot fastboot and does not need the rootfs. See the [boot map](../hardware/boot-map.md).
- [ ] Track B research: test a non-Akai kernel with **zero eMMC writes** (kexec, which is available; fastboot RAM boot, unverified; or MaskROM RAM boot) before any persistent boot change. SD boot is not zero-write, because U-Boot tries the eMMC first.
- [ ] Demonstrate controller input and safe ALSA playback **on actual MPC** after verifying factory service handoff and recovery.
- [ ] Port host-only Python network-audio datagram model to Rust; add bounded real packet pacing/clock-drift simulation before any MPC audio ingress.
- [ ] Implement user-provided sample importer and genuine sequencer editing, projects, MIDI and hardware HAL.

These engineering milestones deliberately do **not** mark the Phase 1 or Phase 2 gates complete. No MPFree software is installed persistently on the MPC, and the original `acvs.service` still owns PCM and display resources. **The reference unit is not stock, though:** persistent SSH added a unit to the rootfs and keys under `/data` (see [device modifications](../hardware/device-modifications.md)). For source and tests see [Rust M1](../milestones/m01-rust-metronome-parity.md) and [Rust M2](../milestones/m02-step-sequencer-sampler.md).

## Phase 0 — Evidence and risk reduction
- [x] Record exact target model and factory firmware version. (MPC One, 3.6.0: [baseline](../hardware/baseline-3.6.0.md))
- [x] Review licensing and choose a repository license. (MIT, 2026-11-09: [LICENSE](../../../LICENSE))
- [ ] Inventory kernel, audio devices, controller MIDI endpoints, touchscreen, graphics, storage, and recovery options. (Everything except recovery options is inventoried; see the [survey](../hardware/survey-2026-11-09.md). The boot chain is mapped in the [boot map](../hardware/boot-map.md), but no recovery path has been proven yet.)
- [x] Identify a reversible, offline, non-destructive way to launch a custom userspace application. (RAM-only `/run` execution over SSH, proven in M5–M7 and M10.)
- [ ] Define a desktop simulator for the hardware inputs and display. (Partial: M8 offscreen test card and M11 synthetic touch streams.)

**Gate:** We can document an accurate hardware map and a safe way back to the factory application.

## Phase 1 — Hello MPC
- [ ] Build a native ARM userspace executable.
- [ ] Display a custom full-screen interface.
- [ ] Receive at least one pad/button/encoder event.
- [ ] Generate and output a metronome click through a low-latency audio backend.
- [ ] Measure and log audio latency and xruns without blocking the audio callback.

**Gate:** A reproducible, non-destructive demonstration on real MPC One hardware.

## Phase 2 — Playable instrument
- [ ] Sample loading and pad mapping.
- [ ] Velocity response and choke groups.
- [ ] Transport and tempo.
- [ ] MIDI input/output and sync.
- [ ] Project save/load, undo, and crash recovery.

**Gate:** Record, save, reload, and perform a basic beat without a computer.

## Phase 3 — Distinctive sequencer
- [ ] Per-track polymeter and independent clip lengths.
- [ ] Microtiming, conditional triggers, probability, ratchets, parameter locks.
- [ ] Fast step editing using pads + encoders; touchscreen is optional for core operations.
- [ ] Clip launcher plus linear arranger with easy conversion between them.
- [ ] Nondestructive automation and modulation routing.

**Gate:** Produce a multi-section composition that would be awkward in a conventional MPC workflow.

## Phase 4 — Production workstation
- [ ] Audio tracks, overdubbing, resampling, and slicing.
- [ ] Flexible buses, sends, effects chains, sidechain routing.
- [ ] Instrument/effect plugin architecture.
- [ ] Browser with tags, auditioning, and content provenance.
- [ ] Export stems, MIDI, and project archive.

## Phase 5 — Compatibility and distribution
- [ ] Optional local import of supported user-owned content.
- [ ] Assess factory project import and legally compatible native plugin hosting.
- [ ] Package signed/reproducible releases with backup/recovery documentation.
- [ ] Test on MPC One, then separately evaluate One+ and other models.

## Future: firmware-proof installation

**Core principle (owner, 2026-11-10):** installing MPFree must be effortless for every MPC owner, in perpetuity, and must keep working when Akai ships new firmware. Nobody should have to repeat the reverse engineering done for 3.6.0 and 3.10.0. These two tasks are recorded now and scheduled later; neither is started, and neither may touch a device outside the [recovery tiers](../research/recovery-and-os-strategy.md).

- [ ] **Automated firmware analyzer.** One workstation command takes a new Akai update image and reports everything the MPFree installer needs to change. It chains the offline tools written for the 3.10.0 analysis (image format listing, extraction with hash checks, read-only rootfs survey, DTB, kernel config and kexec details, updater/flasher inventory; in PR #23, `tools/mpc-one/offline-*.py` and `docs/dev/hardware/akai-updater-offline.md`). Output: a human report, a diff against the last known-good firmware profile, and a machine-readable firmware profile that the installer and the firmware allowlist consume. It never runs on the MPC and never writes the image. Things it must flag: a changed image format, a bootloader record or signature tool (3.10.0 adds `az01-bootloader-sig` and LoadPin trust points, inferred from file names only), changed DTB compatibles or eMMC/PHY nodes, kexec removed from the kernel, and new boot-time flashers (touch firmware, controller MCU via `dfu-util`).
- [ ] **Guaranteed bootloader access.** Find a route to the boot chain that no official firmware can take away. The candidate is the RK3288 MaskROM, which is in the SoC's boot ROM rather than on the eMMC, entered over USB (the USB-B port is the likely MaskROM port; unconfirmed) by a button or eMMC test point (not yet found on this board). This is the Tier 3 work in the recovery strategy. To check, all unverified today: how MaskROM is forced on this board; whether the SoC's secure-boot fuses are or could be programmed so that only signed loaders run; and what Akai's new bootloader signature tool implies. The installer should read and report this state read-only and refuse to continue on firmware whose bootloader the analyzer hasn't cleared.

## Prioritization principles

Prioritize low menu depth, immediate physical control, reliable audio, reversible operations, and expressive sequencing. Do not let 'all factory features' delay a useful, independently playable first release.

## Open decisions

- Engine language: **DECIDED — Rust-first**, retaining C/C++ compatibility adapters for established instruments, DSP and plugin hosting; see [ADR-0001](../decisions/adr-0001-rust-first.md). Hardware real-time performance remains unqualified.
- UI: **DECIDED (owner, 2026-11-09)**: SDL3 and a custom Rust retained-mode UI on OpenGL ES 3. The custom UI is [`mpfree-ui`](../../../rust/mpfree-ui), drawing in software to the framebuffer until a GLES canvas is qualified; see [UI architecture](../design/ui-architecture.md).
- Audio: **DECIDED — direct ALSA on the device**, with JACK/PipeWire/CPAL as desktop backends behind an `AudioDevice` trait; see [ADR-0002](../decisions/adr-0002-direct-alsa-audio.md). On-device xrun and latency qualification still pending.
- Licensing: GPL-3.0-or-later, MPL-2.0, or other approved FOSS license.
- Plugin API: LV2, CLAP, custom internal graph, or combination; hardware and licensing constraints apply.
