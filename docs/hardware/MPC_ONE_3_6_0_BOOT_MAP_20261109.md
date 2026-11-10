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
the kernel from `p6`. Update Mode does **not** depend on `p6`. It lives in U-Boot
(see the next section).

Kernel command line: `root=PARTUUID=… rootwait ro rfkill.default_state=0 fsck.repair=yes isolcpus=2-3`.
Device tree: model `Akai Professional MPC One`, compatible
`inmusic,acva inmusic,az01 rockchip,rk3288`.

## U-Boot environment and boot flow (second run, 22:13 UTC)

Raw report SHA-256 of the second run:
`cfb92f8d1dd8af0a2d50b96c276c2f2d57852e9b2d2d62ce7956fc3f605342db`. All hashes
matched the first run, and `p1`–`p6` again showed zero sectors written. The raw
`p2` read and `fw_printenv` agree. `ethaddr` and `serial#` are redacted, and
partition UUIDs are left out here.

```text
bootcmd=run az01_bootcmd
az01_bootcmd=if test x${dorecovery} = x1; then run az01_recovercmd; fi;
             run distro_bootcmd; echo 'Boot failed, resetting to bootloader...'; reset_bootloader
az01_recovercmd=echo 'Starting fastboot...'; fastboot 0
boot_targets=mmc0 mmc1
boot_prefixes=/ /boot/
boot_scripts=boot.scr.uimg boot.scr
fastboot.partition-type:{rootfs,content,data}=ext4
bootdelay=0   stdin=serial,gpio-keys   stdout=serial (DT: serial2 115200n8)
partitions=<full GPT template: uboot-spl, env, uboot, splash, recoverysplash,
            rootfs (bootable), content, data>
```

`/boot/boot.scr.uimg` loads `zImage` and `${fdtfile}` from the same
partition. It applies `rk3288-az01b.dtbo` only when `board_name=az01b` (here it
is `az01`), appends `fsck.repair=yes`, and runs `bootz`. `fdtfile` and
`dorecovery` are not stored in the environment, so U-Boot's board code sets
them at runtime (inference).

What this establishes:

1. **Akai's Update Mode is U-Boot fastboot, stored in `p1`/`p3`.** When
   `dorecovery=1` (most likely set by the documented button combination),
   U-Boot runs `fastboot 0` before it touches `p6`. The fastboot partition types
   name `rootfs`, `content` and `data`. **So a damaged or unbootable `p6` can
   still be reflashed by Akai's updater, as long as `p1`–`p3` are intact.**
   This corrects the earlier inference that Update Mode depended on `p6`.
2. **Boot order is eMMC first, then the SD slot.** `distro_bootcmd` scans `mmc0`
   (eMMC) and then `mmc1`. The Linux DT aliases map `mmc1` to the SD
   controller `dwmmc@ff0c0000`; U-Boot's numbering is assumed to match.
   On each device it checks `extlinux/extlinux.conf`, then `boot.scr.uimg`,
   under `/` and `/boot/`. A healthy eMMC always wins, so **an SD card is only
   booted if the eMMC boot fails.** Making the SD card boot first would need an
   environment write (Tier 3).
3. **When every target fails, U-Boot calls `reset_bootloader`** rather than
   hanging. What that command does on this board is not known.
4. **The bootloader carries a full GPT template** (`partitions=`), which is what
   `gpt write` and fastboot use. This suggests the partition table itself can be
   rebuilt from Update Mode (inference).
5. **The serial console runs on UART2 at 115200 baud**, but `bootdelay=0`. A
   UART header on the board, if one exists, would give a non-destructive
   U-Boot and kernel console.

### Loader headers

- `p1` / whole-disk sector 64 starts `3b 8c dc fc be 9f 9d 51 eb 30 34 ce 24 51 1f 98`.
  That is the standard RC4-scrambled Rockchip ID-block header, so a normal
  Rockchip idbloader sits at sector 64.
- `p3` starts `27 05 19 56`, the legacy U-Boot uImage magic, so U-Boot proper is
  stored as a uImage.
- `boot0`/`boot1`: zeros.

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
to the OTG controller, which makes it the MaskROM port. This is now stronger:
Akai's Update Mode is U-Boot `fastboot 0`, which runs over a USB device-mode
controller, and the desktop updater connects through the USB-B port. Physical
confirmation is still needed.

## Update path

- ~~`/usr/bin/az01-update` exists~~. Corrected 2026-11-10: it does not exist in `p6`, and neither boot-map run shows it. See [reading Akai's updater offline](akai-updater-offline.md).
- `/opt/ramfs-update.squashfs` (3,551,232 bytes) lives on `p6`. It is a signed-image updater that runs from RAM; findings are in [reading Akai's updater offline](akai-updater-offline.md).
- Akai also ships boot-time firmware updaters for peripherals:
  `touch-fw-update.service` (`/usr/bin/touch-fw-upd`, Ilitek ILI2116/2117) and
  `xmos-update.service` (`xmosdfu --auto`, only when no I²S sound node exists).
  An MPFree distribution must keep or deliberately replace these.

## Survey defects found in the first run (fixed, confirmed by the second run)

- BusyBox `od` on az0x has no `-A`/`-t`. Loader bytes and the squashfs magic
  printed usage errors. Fixed to use `od -x`/`od -c`.
- The raw `p2` environment printed nothing. This was most likely a
  `tr '[:print:]'` class that this BusyBox doesn't support. Fixed to avoid
  character classes, with `fw_printenv` (read-only) as a second source.
- Added the text of `/boot/boot.scr.uimg` and a partuuid → partition listing.
