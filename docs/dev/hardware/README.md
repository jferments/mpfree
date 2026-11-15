# MPC One hardware and operating-system documentation

The original MPC One is an RK3288-based embedded Linux computer. The documents here describe the actual board, its factory operating system, its bootloader and the interfaces that MPFree must control to run as a separate music-production environment. They are written for developers building and testing MPFree on **hardware they own**.

## Operating system and boot

**[MPC One operating-system architecture](mpc-one-boot-architecture.md)** is the complete introduction. It follows the machine from power-on through Linux, explains the eMMC and filesystem layout, developer access, realtime audio and hardware ownership, and documents how an independent kernel has already been booted. It ends with the installer and recovery architecture required for MPFree to start without factory Linux.

**[MPC One bootloader](bootloader.md)** is the dedicated, detailed guide to the **RK3288 BootROM, Rockchip ID block, SRAM first stage, U-Boot, persistent environment, boot scripts and USB Fastboot**. It explains how the stages execute, where their images are stored, how U-Boot finds a kernel, which pre-Linux launch mechanisms appear usable for MPFree, and which recovery functions still need physical verification. This is the recommended reference for anyone implementing the installer or boot selector.

**[Firmware 3.10.1 compatibility and developer access](firmware-3.10.1-compatibility.md)** describes the official developer mode (terminal, SSH and USB script runner), the plugin SDK and the results and limits of the offline firmware comparison.

**[Trying firmware 3.10.1 from the USB stick](firmware-3.10.1-stick-test.md)** is the runbook for testing 3.10.1, and its developer mode, by kexec from the USB stick before installing it (superseded by the install on 2026-11-15).

**[How MPFree boots from the stick on MPC OS 3.10.1](soft-reboot-boot.md)** explains the boot system that works today: with the stick in at power-on, a hook on Akai's data partition hands over to the stick's system by a systemd soft-reboot on Akai's running kernel, and the stick switches the eMMC and SD card off before anything can touch them. Start here to understand or change the boot.

**[Boot routes on MPC OS 3.10.1](boot-routes.md)** compares the ways to take over after power-on: the soft-reboot, the opt-in kexec_file restart into Akai's own kernel, a chain to MPFree's own kernel and the U-Boot route. It explains LoadPin, the kernel policy that decides between them, and which external USB devices each route can drive.

**[Starting MPFree from the stick at power-on](stick-autoboot.md)** is the record of how that boot was found and proven: what can start the jump on an installed 3.10.1, what is ruled out, the canary, the kexec hook that LoadPin refused, both soft-reboot power-ons, the pad player without root (`--app-user`), the opt-in kexec_file route and its chain to the stick's own device tree, and the owner's blocks.

**[The 3.10.1 device round](firmware-3.10.1-qualification.md)** is the runbook for the reference unit since the owner installed 3.10.1 (2026-11-15): developer mode's SSH locked to a key, a fresh eMMC backup, main's boot hook and the power-on into MPFree from the stick, the pad player on it, the benchmark, GPU page flips, a second boot with an hour-long song, FXProbe under Akai's system from the same stick, and the way back.

**[The targeted app run on the soft-boot stick](app-targeted-run.md)** is the runbook for the app check's phases that have a reason to run: the ones live test 3 skipped, and the features merged since (the Mix tab, drag and drop, effects with modes). Switching on with the stick in starts MPFree; there is no kexec step.

**[Installer and recovery contract](installer-recovery-contract.md)** specifies the minimum evidence required for temporary kernel boots, persistent MPFree application installation, direct boot and boot-critical storage changes.

## Hardware interfaces

These are the active, specialized developer references for programming the board. They retain their original technical detail rather than being replaced with navigation-only pages.

| Area | Reference | What it answers |
|---|---|---|
| Processor ABI and factory service lifecycle | [ARM ABI and service handoff](abi-handoff-survey.md) | Which binaries run on the device, what systemd starts and how hardware resources are owned |
| Audio and MIDI | [Audio/MIDI inventory](audio-midi-survey.md) and [audio runtime](audio-runtime.md) | Which ALSA cards and PCM/MIDI interfaces are present, and what the realtime runtime must handle |
| Pads and controls | [Controller protocol](controller-protocol.md) and [MIDI activity map](midi-activity-map.md) | How physical controls are represented in Linux and MIDI |
| Networking and USB controllers | [Ethernet and USB hardware](ethernet-usb.md) | Gigabit link, USB host/peripheral controllers and measurable I/O behavior |
| Touchscreen and graphics | [Display and touchscreen](touchscreen-display.md) | DSI/DRM/Panfrost interfaces, panel properties and touch input |
| Service/resource ownership | [Resource ownership](resource-ownership.md) | Which factory processes hold PCM, MIDI, DRM and input devices |
| Structured board measurements | [Observed hardware data](data/) | Recorded machine-readable hardware profiles and touchscreen properties |

For hands-on work, see the [developer access guide](../guides/developer-access.md) and the [MPFree application test guide](../guides/mpfree-app.md). Exact firmware, device revision and test date matter: never assume that an observation made on the original MPC One applies to all related products.

## Earlier engineering records

The original dated reports on firmware 3.6.0, eMMC acquisition, the boot map, the three 3.10.0 temporary boots, factory updates and independent-ROM recovery have been **moved entirely into the [November 2026 archive](archive/2026-11/README.md)**. They are preserved byte-for-byte, including their measurements, historical interpretations and detailed command transcripts. They are not duplicated here as placeholder documents. Use the current architecture and bootloader pages for an integrated, up-to-date explanation.

### Keeping the documentation current

When a physical test resolves an open question, update the relevant current reference and identify the tested hardware and firmware. Preserve the original evidence in the dated archive. Distinguish what was observed on the MPC from what was verified in an image or proposed for a future installer. In particular, a complete eMMC backup is not yet a demonstrated independent restore procedure for a damaged first-stage loader.
