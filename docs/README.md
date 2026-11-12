# MPFree documentation

Start with the [project README](../README.md) for what MPFree is and where it stands. This page lists every document, grouped by purpose.

## Project

- [Architecture](project/architecture.md): the proposed system design.
- [Roadmap](project/roadmap.md): milestones, phase gates and open decisions.
- [Development log](project/development-log.md): dated history of how the project got here.
- [Factory content policy](project/factory-content-policy.md): what MPFree will and won't do with Akai/AIR content and plugins.
- [USB installer architecture](project/usb-installer.md): design for a future one-USB, owner-consented installer with rollback.
- [Sampler design](project/sampler-design.md): how MPFree records, chops, edits, assigns and plays samples, including granular tools (proposal, 2026-11-10).
- [Sampler feature map](project/sampler-features.md): every MPC sampler feature rebuilt with a better UI, what other samplers add, and what makes a sampler great for production and performance (proposal, 2026-11-10).
- [Sampler architecture](project/sampler-architecture.md): crates, threads, data flow, real-time rules, algorithms and files for the sampler (proposal, 2026-11-10).
- [Arpeggiator, Latch and latch patterns](project/arpeggiator.md): research, design and plan for Note Repeat, Latch, the arpeggiator and custom patterns (engine in `rust/mpfree-arp`).
- [UI architecture](project/ui-architecture.md): MPFree's GUI toolkit (`mpfree-ui`): canvas, theme, fonts, widgets, one-line page layouts, and how the screen and audio threads talk.
- [Transport](project/transport.md): the time engine's clock that the sequencer, looper, arp and sampler follow: ticks, tempo map, loop, count-in, punch, and the same output for any period size (engine in `rust/mpfree-transport`, 2026-11-11).
- [Playback](project/playback.md): clips of any length on the transport: the model, compiling, sample-exact playback, note-offs, a track through the arp, the clip recorder with replace, overdub and take undo, and a WAV renderer (engine in `rust/mpfree-seq` and `rust/mpfree-play`, 2026-11-11).
- [Scenes and follow actions](project/scenes.md): a row of the launch grid launched together on the bar line, with its mutes switching there, and follow actions that chain clips and scenes (`rust/mpfree-perform`, 2026-11-11).
- [Capture](project/capture.md): keep what was just played, even without recording: the always-on history, the last phrase cut to whole bars, a new clip in place or notes into the playing clip, undo, the Capture button, and a WAV demo (`rust/mpfree-capture`, 2026-11-11).
- [Audio tracks](project/audio-tracks.md): recording the line input in a line along the song: takes, a take per pass of a loop recording, comping between takes with crossfades, punch in and out, latency, one memory, and a demo (`rust/mpfree-audiotracks`, 2026-11-12).
- [Synths](project/synths.md): the node interface every instrument and effect uses, and the native synths: Poly (subtractive), FM (DX7-style, with DX7 banks), Drums (a model per pad: kick, snare, clap, hats, toms, percussion, with choke groups and kits) and Pluck (Karplus-Strong strings and modal mallets), pad pressure, the Synth tab, demos and costs (`rust/mpfree-node`, `rust/mpfree-synth`, `rust/mpfree-synth-ui`, 2026-11-11).
- [Sound presets](project/presets.md): any instrument's sound saved as a plain-text preset file and loaded back to the bit, with settings by name, so old and new presets load (`rust/mpfree-instruments`, 2026-11-11).
- [Track host](project/track-host.md): each track's instrument fed by the sequencer, the pads and the arp and rendered into its mixer track: slots, routing, notes counted per source, sample-exact parameters, swaps that ring out, STOP, and no allocation on the audio thread (`rust/mpfree-track`, 2026-11-11).
- [Plugins: the CLAP host](project/plugins.md): CLAP instruments and effects as nodes on a track or in an insert chain: loading `.clap` files, parameters, sample-exact events, state, the main and audio threads, and our own test plugins (`rust/mpfree-clap`), and the sandbox that runs each plugin in its own process with a deadline each block and a restart after a crash (`rust/mpfree-plughost`, 2026-11-11).
- [Wavetable synth](project/wavetable.md): the third native synth: table oscillators scanned by Position, warps, band-limited tables, factory tables, WAV and sample import, memory and device cost estimates (`rust/mpfree-synth`, 2026-11-11).
- [Concurrency](project/concurrency.md): using every core of the MPC One: where threads run today, the core plan, and lanes, which run part of each audio period on another core with a deadline (`rust/mpfree-lanes`, 2026-11-11).
- [Offload](project/offload.md): render partitions on another computer over UDP, answered L periods later with the MPC as the only clock: the datagram, the server, the client, and `mpfree-offload measure` (`rust/mpfree-offload`, desktop prototype, 2026-11-12).

## Decisions

- [ADR-0001: Rust first, C/C++ compatible](decisions/adr-0001-rust-first.md) (accepted 2026-11-08).
- [ADR-0002: direct ALSA audio on the MPC One](decisions/adr-0002-direct-alsa-audio.md) (accepted 2026-11-09).

## Testing and CI

- [Testing and CI](guides/testing-and-ci.md): the test tiers, what each CI job checks, the self-hosted runners, rules for tests, coverage by crate, and the plan for the regression suite.

## Working with the device

- [Developer access](guides/developer-access.md): getting root SSH over direct Ethernet, and the safety runbook.
- [Survey tools](guides/survey-tools.md): running read-only surveys and milestone tests from a workstation.
- [USB autorun reference](guides/usb-autorun-reference.md): the exact lab tooling used to first get SSH. Not a general installer.
- [mpfree on the test build](guides/mpfree-app.md): the app on the kexec test build (Phase 1 called it Hello MPC): our own program on the screen, touchscreen, pads, pad lights and audio output, with factory drum kits read from the backup, and the pad player with MPFree's first GUI, one step at a time (`--app`).
- [Audio and MIDI diagnostics](guides/audio-midi-diagnostics.md): the `--audio-midi`, `--midi-activity` and `--midi-capture` modes, what each reads and why.
- [Device modifications](hardware/device-modifications.md): every persistent change made to the reference MPC One (currently persistent SSH, MOD-001/MOD-002), with verification and undo steps.

## Milestones

| | Report | Ran on |
|---|---|---|
| M0 | [C++ audio core: metronome and WAV](milestones/m00-cpp-audio-core.md) | Workstation |
| M1 | [Rust metronome, C++ parity, ARMv7 cross-build](milestones/m01-rust-metronome-parity.md) | Workstation, CI |
| M2 | [Rust step sequencer and 16-voice sampler](milestones/m02-step-sequencer-sampler.md) | Workstation, CI |
| M3 | [Network PCM codec and WAV sampler](milestones/m03-network-pcm-sampler.md) | Workstation, CI |
| M5 | [First RAM-only Rust binary on the MPC](milestones/m05-ram-only-rust-smoke.md) | MPC One |
| M6 | [Headless engine with null audio](milestones/m06-headless-null-audio.md) | MPC One |
| M7 | [Synthetic timing and host supervisor](milestones/m07-synthetic-timing.md), [on-device acceptance](milestones/m07-on-device-acceptance.md), [validation record](milestones/m07-validation-record.md) | MPC One |
| M8 | [Offscreen display test card](milestones/m08-offscreen-test-card.md) | Workstation |
| M9 | [Touch ABS metadata interface](milestones/m09-touch-metadata.md) | Workstation |
| M10 | [Guarded touch-axis test](milestones/m10-guarded-touch-axes.md), [validation record](milestones/m10-validation-record.md) | MPC One |
| M11 | [Multitouch decoder and gestures](milestones/m11-multitouch-decoder.md) | Workstation, CI |
| M12 | [Live touch capture and orientation calibration](milestones/m12-touch-capture.md) | MPC One |
| M13 | [Live touch reader: seeded at open, resync after overflow](milestones/m13-live-touch-reader.md) | Workstation, CI (device check pending) |
| M14 | [Swipes, release velocity, fling and hold jitter](milestones/m14-gestures.md) | Workstation, CI (device tuning pending) |
| M15 | [Touch lab: owner-paced touch steps and journal lines](milestones/m15-touch-lab.md) | Workstation, CI; page in the pad player (Prefs, Touch lab) |
| M16 | ["Calibrate touch": orientation from four corner taps](milestones/m16-calibrate-touch.md) | Workstation, CI; page in the pad player (Prefs, Calibrate touch) |
| M17 | [The Dev tab's Touch page: measuring and tuning touch](milestones/m17-touch-tuning.md) | Workstation, CI; page in the pad player (Dev, Touch) |
| GPU | [Panfrost offscreen GLES qualification](milestones/gpu-panfrost-offscreen.md), [kit](../tools/mpc-one/gpu-qualification/README.md) | MPC One |

M4 was host-only safety tooling and the [handoff state-machine simulator](../rust/mpfree-handoff-model/src/lib.rs); it has no separate report. See the [roadmap](project/roadmap.md).

## Research and strategy

- [Recovery tiers and the path to an MPFree OS](research/recovery-and-os-strategy.md): the proposed three-tier recovery gate, what an Akai firmware update overwrites, and how to test another OS with zero eMMC writes.
- [Hardware ownership strategy](research/hardware-ownership.md): win back hardware resources in userspace first, custom Linux later.
- [Reversible factory handoff](research/reversible-handoff.md): safety design for stopping and restoring the factory app.
- [Factory GUI handoff research](research/factory-gui-handoff.md): factory processes and what a screen takeover needs.
- [ARMv7 userspace compatibility](research/armv7-userspace-compat.md): matching the device's glibc, loader and ABI.
- [Auxiliary compute over Ethernet and USB](research/auxiliary-compute.md): offloading synths and effects to another computer.
- [Samplers: what people like, what they don't, and what to borrow](research/sampler-landscape.md): MPC owners' praise and complaints, other samplers, granular synthesis, and MIT-compatible libraries (2026-11-10).

## Hardware findings (MPC One, firmware 3.6.0)

- [Baseline](hardware/baseline-3.6.0.md): kernel, distribution, storage and known unknowns (2026-11-08).
- [Hardware survey](hardware/survey-2026-11-09.md): SoC, eMMC partitions, audio, MIDI, display, touch.
- [Resource ownership](hardware/resource-ownership.md): which factory process holds PCM, MIDI, DRM and input devices.
- [Audio runtime](hardware/audio-runtime.md): the live ALSA stream, threads and MIDI routing.
- [ABI and handoff survey](hardware/abi-handoff-survey.md): userspace ABI, CPU affinity and locked memory.
- [Ethernet and USB](hardware/ethernet-usb.md): gigabit link, USB roles and measured throughput.
- [Audio and MIDI implementation survey](hardware/audio-midi-survey.md): device tree, ALSA, IRQs, RT limits and USB MIDI descriptors (2026-11-09).
- [MIDI activity map](hardware/midi-activity-map.md): which internal MIDI port carries pads, knobs and buttons.
- [Controller protocol](hardware/controller-protocol.md): the internal controller's USB-MIDI messages, observed with passive usbmon.
- [Touchscreen and display](hardware/touchscreen-display.md): ILI2117 driver, evdev, DSI panel and Panfrost.
- [Boot chain and recovery map](hardware/boot-map.md): eMMC layout, U-Boot environment and boot order, boot-partition hashes, and kexec support (2026-11-09).
- [eMMC backup](hardware/emmc-backup.md): a read-only copy of the whole eMMC to the workstation, checked against the boot map (2026-11-09).
- [Reading Akai's updater offline](hardware/akai-updater-offline.md): what Akai's updater and the 3.10.0 update image write and check, read on the workstation only (2026-11-10).
- [Test-booting another firmware version by kexec](hardware/kexec-test-boot.md): booting 3.10.0 from a USB stick with the eMMC hidden from it; booted three times, storage unchanged, Ethernet and SSH work after one retry (2026-11-10).
- [MaskROM over USB](hardware/maskrom-usb.md): how the RK3288 boot ROM's USB recovery mode could be reached on the MPC One, what it can do RAM-only, secure-boot risk, and the first checks (research, 2026-11-10).
- [Research matrix](hardware/research-matrix.md): subsystem-by-subsystem mapping plan and safety gates.
- [Observed data (JSON)](hardware/data/mpc-one-3.6.0-observed.json): redacted, machine-readable observations.
- [Touch profile (JSON)](hardware/data/mpc-one-touch-profile.json): touch orientation and ranges measured by M12 (2026-11-10).
