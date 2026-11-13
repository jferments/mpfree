# MPC One Hardware Documentation

**Canonical developer reference:** [MPC One Linux and Boot Architecture](mpc-one-boot-architecture.md) — the complete source for RK3288 ROM/SPL/U-Boot, eMMC, filesystem mounts, Linux rights and kexec, USB, firmware 3.10.1, direct-boot possibilities, and installer/recovery design.

## Start here

1. [Comprehensive Boot and Linux Architecture](mpc-one-boot-architecture.md) — current technical specification and evidence matrix.
2. [Firmware 3.10.1 Compatibility](firmware-3.10.1-compatibility.md) — official developer facilities, image comparison and outstanding concerns.
3. [Installer and Recovery Contract](installer-recovery-contract.md) — gates for no-write experiments versus persistent boot changes.
4. [Development Access Guide](../guides/developer-access.md) — device communication and commands (check firmware-specific applicability).
5. [Dated Hardware Evidence Archive](archive/2026-11/README.md) — preserved original measurements and development-session notes.

## Specialized hardware interfaces

| Interface | Detailed reference |
|---|---|
| ARM ABI, systemd, and handoff | [ABI/handoff](abi-handoff-survey.md) |
| ALSA / MIDI device inventory | [Audio and MIDI](audio-midi-survey.md) |
| Audio engine operation | [Audio runtime](audio-runtime.md) |
| Pad and controller interfaces | [Controller protocol](controller-protocol.md) |
| Ethernet and USB controllers | [Network and USB](ethernet-usb.md) |
| MIDI events | [MIDI activity](midi-activity-map.md) |
| Factory hardware ownership | [Resource ownership](resource-ownership.md) |
| Display, touchscreen and GPU | [Touchscreen/display](touchscreen-display.md) |
| Structured machine-readable board facts | [Observed data](data/) |

## Historical technical records

The previous top-level research/session files remain addressable as landing pages and their **entire original contents** are archived unchanged. For primary facts, interpretations and dates see [the archive](archive/2026-11/README.md).

| Topic | Original file |
|---|---|
| Reference 3.6.0 Linux baseline | [baseline-3.6.0.md](baseline-3.6.0.md) |
| GPT/eMMC, loader and boot script | [boot-map.md](boot-map.md) |
| Full eMMC backup | [emmc-backup.md](emmc-backup.md) |
| Three 3.10.0 temporary boots | [kexec-test-boot.md](kexec-test-boot.md) |
| ROM and USB boot modes | [maskrom-usb.md](maskrom-usb.md) |
| Owner first-stage recovery | [owner-rom-recovery-plan.md](owner-rom-recovery-plan.md) |
| Factory updater | [akai-updater-offline.md](akai-updater-offline.md) |
| Reference unit persistent modifications | [device-modifications.md](device-modifications.md) |
| Dated detailed survey | [survey-2026-11-09.md](survey-2026-11-09.md) |
| Original evidence backlog | [research-matrix.md](research-matrix.md) |

## Editorial and evidence policy

New developers should not add standalone timestamped reports to this directory. Keep the **current interpretation** in the architecture and focused reference documents, and retain raw run/session material in a dated archive. Date findings and label them *observed on device*, *verified in image*, *public platform behavior*, *inferred*, *proposed*, or *unknown*. Do not promote a successful VM or static program test into a physical-device claim. Do not put private full eMMC images, proprietary firmware distributions, hardware identifiers, SSH keys or factory samples in GitHub.

A verified backup is **not** a proven restore. Boot-critical persistent writes remain prohibited until independent recovery is demonstrated.
