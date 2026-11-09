# MPC One 3.6.0 boot chain and recovery map (owner-run, 2026-11-09)

**Source:** `bash tools/mpc-one/collect-over-ssh.sh --boot-map` run by the owner on
the reference MPC One (persistent SSH) at 2026-11-09 22:04 UTC. Raw report
SHA-256: `a2b295c37baecf8e088e239380323556fdba82289b073c361faab834554b9a4a`
(kept privately on the workstation, not committed). Read-only: the report itself
shows zero sectors written to `p1`–`p6` since boot.

PARTUUIDs, the MAC address, and the `p2` hash (the environment may hold
per-device values) are left out of this document on purpose.

## eMMC layout (512-byte sectors)

| Partition | Name | Start | Sectors | Size | Notes |
| --- | --- | ---: | ---: | --- | --- |
| `mmcblk0p1` | `uboot-spl` | 64 | 128 | 64 KiB | **Starts at sector 64, the RK3288 boot ROM's idbloader offset.** This is what the boot ROM loads first |
| `mmcblk0p2` | `env` | 192 | 16 | 8 KiB | U-Boot environment; `/etc/fw_env.config` points at a partuuid with offset 0, size `0x2000` |
| `mmcblk0p3` | `uboot` | 256 | 768 | 384 KiB | U-Boot proper |
| `mmcblk0p4` | `splash` | 1024 | 8192 | 4 MiB | |
| `mmcblk0p5` | `recoverysplash` | 9216 | 8192 | 4 MiB | |
| `mmcblk0p6` | `rootfs` | 17408 | 1024000 | 500 MiB | ext4, mounted ro **without a journal** |
| `mmcblk0p7` | `content` | 1041408 | 8388608 | 4 GiB | |
| `mmcblk0p8` | `data` | 9430016 | 5843551 | ~2.8 GiB | |
| `mmcblk0boot0/1` | (hardware boot areas) | – | 8192 each | 4 MiB each | **Both all zeros** (SHA-256 equals that of 4 MiB of `0x00`), `force_ro=1`, never read. **Unused**: the device boots from the user area |

eMMC: `DG4008`, 7.28 GiB, manfid `0x45`. SD slot: `mmc1` = `dwmmc@ff0c0000`
(64 GB SDXC card present, exFAT).

## Boot-critical hashes (baseline, state `MOD-001+MOD-002`)

| Partition | SHA-256 |
| --- | --- |
| `p1` uboot-spl | `aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec` |
| `p3` uboot | `7b6c8d091e7fcaecb89e633bacc1b541f4c494844b23daae6dc9a3c564721976` |
| `p4` splash | `6b4b891d664e59c933716d5babf99711526921e228376a19ace130322fe49a3a` |
| `p5` recoverysplash | `9a06a00806729dda0bfc44b3193720b5ba91e3c166b80d8b21a7d4ee312a11e2` |
| `p6` rootfs (modified) | `fa2ca12bfac82f5ca69238f8b85c1f1df18994486b385086ebdda26ae61b2350` |
| `boot0`, `boot1` | `bb9f8df61474d25e71fa00722318cd387396ca1736605e1248821cc0de3d3af8` (4 MiB of zeros) |
| `/opt/ramfs-update.squashfs` | `39db4a08bc988d8cb8ce24fb40e29ed6289c1484753e07c52c842906ad24fadb` |

Future `--boot-map` runs should reproduce `p1`, `p3`–`p6` exactly. Any change
there means something wrote boot-critical storage.

## Kernel and boot script live in the rootfs

`/boot` on `p6` holds `zImage-6.6.95-az01-2025-06-27-rt55` (5.6 MiB), a 535-byte
U-Boot script `boot.scr.uimg`, and 15 board DTBs (`rk3288-az01-*`,
`rk3288-az05-*`). There is no separate kernel partition. **Confirmed:** U-Boot loads
the kernel from `p6`. **Still an inference:** Akai's Update Mode also depends on
`p6`. The script's text (printed by the revised `--boot-map`) should settle this.

Kernel command line: `root=PARTUUID=… rootwait ro rfkill.default_state=0 fsck.repair=yes isolcpus=2-3`.
Device tree: model `Akai Professional MPC One`, compatible
`inmusic,acva inmusic,az01 rockchip,rk3288`.

## kexec is available (zero-eMMC-write path to another kernel)

`CONFIG_KEXEC=y`, `CONFIG_KEXEC_FILE=y`, `CONFIG_IKCONFIG_PROC=y`, and
`/usr/sbin/kexec` is installed. Nothing is loaded (`kexec_loaded=0`). The
running kernel can therefore load and jump to another kernel in RAM without
touching storage. A power-cycle returns to the stock boot. `/proc/config.gz`
gives us the exact vendor kernel config to start an MPFree kernel from.

## USB OTG controller (MaskROM port candidate)

`usb@ff580000` (`rockchip,rk3288-usb`, `snps,dwc2`) is the RK3288's OTG
controller, the one the boot ROM uses for MaskROM. It is configured
`dr_mode = peripheral`, status `okay`, and the UDC is `not attached`. The other DWC2
(`ff540000`) runs as a host. **Inference:** the USB-B port on the back is wired
to the OTG controller, which makes it the MaskROM port. This needs physical
confirmation.

## Update path

- `/usr/bin/az01-update` exists (Akai's updater, not yet inspected).
- `/opt/ramfs-update.squashfs` (3,551,232 bytes) lives on `p6`.
- Akai also ships boot-time firmware updaters for peripherals:
  `touch-fw-update.service` (`/usr/bin/touch-fw-upd`, Ilitek ILI2116/2117) and
  `xmos-update.service` (`xmosdfu --auto`, only when no I²S sound node exists).
  An MPFree distribution must keep or deliberately replace these.

## Survey defects fixed after this run

- BusyBox `od` on az0x has no `-A`/`-t`. Loader bytes and the squashfs magic
  printed usage errors. Fixed to use `od -x`/`od -c`.
- The raw `p2` environment printed nothing. This was most likely a
  `tr '[:print:]'` class that this BusyBox doesn't support. Fixed to avoid
  character classes, with `fw_printenv` (read-only) as a second source.
- Added the text of `/boot/boot.scr.uimg` and a partuuid → partition listing.
