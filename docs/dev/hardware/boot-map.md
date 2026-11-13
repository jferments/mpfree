# MPC One 3.6.0 boot chain and recovery map (owner-run, 2026-11-09)

> **Archived technical record.** The full November 2026 source has been retained **byte-for-byte** at [archive/2026-11/boot-map.md](archive/2026-11/boot-map.md). The current integrated engineering interpretation is [MPC One Linux and Boot Architecture](mpc-one-boot-architecture.md); for installation requirements see [Installer and Recovery Contract](installer-recovery-contract.md). The source's observations are specific to the then-tested device and firmware.

**Coverage:** Exact reference GPT/eMMC partition geometry, boot environment, image headers and hashes.

**Original Git blob:** `18aa2bef84e4d60cca45088bdcfb403e2a0b2a1c` — its exact contents remain both in Git history and in the evidence archive. No original measurements or notes were discarded.

## Original section navigation

## eMMC layout (512-byte sectors)

Full content: [original section](archive/2026-11/boot-map.md#emmc-layout-512-byte-sectors).

## Boot-critical hashes (baseline, state `MOD-001+MOD-002`)

Full content: [original section](archive/2026-11/boot-map.md#boot-critical-hashes-baseline-state-mod-001mod-002).

## Kernel and boot script live in the rootfs

Full content: [original section](archive/2026-11/boot-map.md#kernel-and-boot-script-live-in-the-rootfs).

## U-Boot environment and boot flow (second run, 22:13 UTC)

Full content: [original section](archive/2026-11/boot-map.md#u-boot-environment-and-boot-flow-second-run-2213-utc).

### Loader headers

Full content: [original section](archive/2026-11/boot-map.md#loader-headers).

## kexec is available (zero-eMMC-write path to another kernel)

Full content: [original section](archive/2026-11/boot-map.md#kexec-is-available-zero-emmc-write-path-to-another-kernel).

## USB OTG controller (MaskROM port candidate)

Full content: [original section](archive/2026-11/boot-map.md#usb-otg-controller-maskrom-port-candidate).

## Update path

Full content: [original section](archive/2026-11/boot-map.md#update-path).

## Survey defects found in the first run (fixed, confirmed by the second run)

Full content: [original section](archive/2026-11/boot-map.md#survey-defects-found-in-the-first-run-fixed-confirmed-by-the-second-run).

