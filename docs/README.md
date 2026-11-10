# MPFree documentation

Start with the [project README](../README.md) for what MPFree is and where it stands. This page lists every document, grouped by purpose.

## Project

- [Architecture](ARCHITECTURE.md): the proposed system design.
- [Roadmap](ROADMAP.md): milestones, phase gates and open decisions.
- [Development log](DEVELOPMENT_LOG.md): dated history of how the project got here.
- [Factory content policy](FACTORY_CONTENT.md): what MPFree will and won't do with Akai/AIR content and plugins.
- [USB installer architecture](USB_INSTALLER_ARCHITECTURE.md): design for a future one-USB, owner-consented installer with rollback.

## Decisions

- [ADR-0001: Rust first, C/C++ compatible](engineering/ADR-0001-RUST-FIRST-HYBRID.md) (accepted 2026-11-08).
- [ADR-0002: direct ALSA audio on the MPC One](engineering/ADR-0002-DIRECT-ALSA-AUDIO.md) (accepted 2026-11-09).

## Working with the device

- [Developer access](hardware/DEVELOPER_ACCESS.md): getting root SSH over direct Ethernet, and the safety runbook.
- [Survey tools](hardware/SURVEY_TOOLS.md): running read-only surveys and milestone tests from a workstation.
- [USB autorun reference](hardware/USB_AUTORUN_REFERENCE.md): the exact lab tooling used to first get SSH. Not a general installer.
- [Audio and MIDI diagnostics](engineering/AUDIO_MIDI_DIAGNOSTICS.md): the `--audio-midi`, `--midi-activity` and `--midi-capture` modes, what each reads and why.
- [Device modifications](hardware/DEVICE_MODIFICATIONS.md): every persistent change made to the reference MPC One (currently persistent SSH, MOD-001/MOD-002), with verification and undo steps.

## Milestones

| | Report | Ran on |
|---|---|---|
| M0 | [C++ audio core: metronome and WAV](engineering/FIRST_AUDIO_CORE.md) | Workstation |
| M1 | [Rust metronome, C++ parity, ARMv7 cross-build](engineering/RUST_M1_PARITY.md) | Workstation, CI |
| M2 | [Rust step sequencer and 16-voice sampler](engineering/RUST_M2_GROOVE.md) | Workstation, CI |
| M3 | [Network PCM codec and WAV sampler](engineering/RUST_M3_NETWORK_SAMPLER.md) | Workstation, CI |
| M5 | [First RAM-only Rust binary on the MPC](engineering/ARMV7_VOLATILE_SMOKE.md) | MPC One |
| M6 | [Headless engine with null audio](engineering/M6_HEADLESS_NULL_AUDIO.md) | MPC One |
| M7 | [Synthetic timing and host supervisor](engineering/M7_SYNTHETIC_TIMING_AND_HOST_SUPERVISOR.md), [on-device acceptance](engineering/M7_ARM_NULL_TIMING_ACCEPTANCE.md), [validation record](hardware/MPC_ONE_M7_PROCESS_TOUCH_VALIDATED_20261109.md) | MPC One |
| M8 | [Offscreen display test card](engineering/M8_DISPLAY_OFFSCREEN_TESTCARD.md) | Workstation |
| M9 | [Touch ABS metadata interface](engineering/M9_TOUCH_ABS_METADATA.md) | Workstation |
| M10 | [Guarded touch-axis test](engineering/M10_GUARDED_TOUCH_AXES.md), [validation record](hardware/MPC_ONE_ILI2117_EVIOCGABS_VALIDATED_20261109.md) | MPC One |
| M11 | [Multitouch decoder and gestures](engineering/M11_MULTITOUCH_DECODER.md) | Workstation, CI |
| M12 | [Live touch capture and orientation calibration](engineering/M12_TOUCH_CAPTURE.md) | MPC One |
| GPU | [Panfrost offscreen GLES qualification](engineering/GPU_PANFROST_OFFSCREEN_20261109.md), [kit](../tools/mpc-one/gpu-qualification/README.md) | MPC One |

M4 was host-only safety tooling and the [handoff state-machine simulator](../rust/mpfree-handoff-model/src/lib.rs); it has no separate report. See the [roadmap](ROADMAP.md).

## Research and strategy

- [Recovery tiers and the path to an MPFree OS](engineering/RECOVERY_AND_OS_STRATEGY.md): the proposed three-tier recovery gate, what an Akai firmware update overwrites, and how to test another OS with zero eMMC writes.
- [Hardware ownership strategy](engineering/HARDWARE_OWNERSHIP_STRATEGY.md): win back hardware resources in userspace first, custom Linux later.
- [Reversible factory handoff](engineering/REVERSIBLE_HANDOFF.md): safety design for stopping and restoring the factory app.
- [Factory GUI handoff research](engineering/FACTORY_GUI_HANDOFF_RESEARCH.md): factory processes and what a screen takeover needs.
- [ARMv7 userspace compatibility](engineering/ARMV7_USERSPACE_COMPATIBILITY.md): matching the device's glibc, loader and ABI.
- [Auxiliary compute over Ethernet and USB](engineering/AUXILIARY_COMPUTE_AND_IO.md): offloading synths and effects to another computer.

## Hardware findings (MPC One, firmware 3.6.0)

- [Baseline](hardware/MPC_ONE_3_6_0_BASELINE.md): kernel, distribution, storage and known unknowns (2026-11-08).
- [Hardware survey](hardware/MPC_ONE_3_6_0_SURVEY_20261109.md): SoC, eMMC partitions, audio, MIDI, display, touch.
- [Resource ownership](hardware/MPC_ONE_3_6_0_RESOURCES_20261109.md): which factory process holds PCM, MIDI, DRM and input devices.
- [Audio runtime](hardware/MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md): the live ALSA stream, threads and MIDI routing.
- [ABI and handoff survey](hardware/MPC_ONE_3_6_0_ABI_HANDOFF_SURVEY_20261109.md): userspace ABI, CPU affinity and locked memory.
- [Ethernet and USB](hardware/MPC_ONE_3_6_0_ETHERNET_USB_20261109.md): gigabit link, USB roles and measured throughput.
- [Audio and MIDI implementation survey](hardware/MPC_ONE_3_6_0_AUDIO_MIDI_SURVEY_20261109.md): device tree, ALSA, IRQs, RT limits and USB MIDI descriptors (2026-11-09).
- [MIDI activity map](hardware/MPC_ONE_3_6_0_MIDI_ACTIVITY_20261109.md): which internal MIDI port carries pads, knobs and buttons.
- [Controller protocol](hardware/MPC_ONE_3_6_0_CONTROLLER_PROTOCOL_20261109.md): the internal controller's USB-MIDI messages, observed with passive usbmon.
- [Touchscreen and display](hardware/MPC_ONE_TOUCHSCREEN_ILI2117_RESEARCH.md): ILI2117 driver, evdev, DSI panel and Panfrost.
- [Boot chain and recovery map](hardware/MPC_ONE_3_6_0_BOOT_MAP_20261109.md): eMMC layout, U-Boot environment and boot order, boot-partition hashes, and kexec support (2026-11-09).
- [Research matrix](hardware/RESEARCH_MATRIX.md): subsystem-by-subsystem mapping plan and safety gates.
- [Observed data (JSON)](hardware/data/mpc-one-3.6.0-observed.json): redacted, machine-readable observations.
