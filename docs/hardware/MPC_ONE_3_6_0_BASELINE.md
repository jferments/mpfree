# Akai MPC One — verified reference baseline (factory firmware 3.6.0)

> **Evidence status:** observed on one owner-authorized MPC One, 2026-11-08 (US Pacific). This is a research baseline, **not** a specification for every MPC model or firmware release. The displayed firmware version 3.6.0 was established in the prior device session; all `uname`, `os-release`, SSH and mount details below were captured from an interactive root shell. No partition image, proprietary payload, serial number, private key or factory assets are distributed here.

## Research principles

Every claim should carry one of these designations:

- **OBSERVED:** captured in terminal output on the reference unit; record the read-only command, date and version.
- **DERIVED:** inference from multiple observations; state what would disprove it.
- **UNVERIFIED:** a hypothesis requiring further device evidence or controlled testing.
- **PROHIBITED DURING DISCOVERY:** actions that can change the installed firmware, data or hardware resource ownership.

This document currently records OBSERVED facts and explicitly labels remaining unknowns. It is deliberately not a fictional SoC pinout or driver map.

## 1. Entry point and verified access

The reference workstation and MPC were connected directly by Ethernet; their addresses are laboratory-specific, not universal defaults.

| Field | Reference observation |
| --- | --- |
| Hardware | Akai MPC One (user-identified) |
| MPC firmware | 3.6.0 (user-reported firmware menu) |
| Access | USB autorun launches an OpenSSH server transiently; key-only authentication |
| SSH account | `root` (confirmed `id`: uid=0, gid=0) |
| SSH TCP port | 2222 |
| Reference MPC IPv4 | `10.42.0.141/24` on `end0` |
| Reference workstation | `10.42.0.1/24` on `eno1np0`, NetworkManager profile `mpc-direct` |
| SSH server banner | `OpenSSH_9.6` |
| Auth mechanism | Ed25519 public key, `publickey` accepted, no password authentication enabled by this bootstrap |

A successful SSH session **does not** mean a replacement OS was installed or that the factory system has been altered. The autorun launcher created runtime SSH files under `/run/mpc-one-autorun-ssh`, a RAM-backed mount, plus limited diagnostic files on the removable USB. It has not been demonstrated that autorun works on firmware newer than the upstream author-tested range.

The reproducibility and safety runbook is in [DEVELOPER_ACCESS.md](DEVELOPER_ACCESS.md). The bootstrap uses upstream [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore) static/manifest/validation files exactly as published at upstream commit `4955a83686227690495aaad68e3ba4e39f5afaec`. Their exact byte hashes are checked rather than regenerating an unofficial validation token. Do not modify those immutable upstream files.

## 2. Kernel and operating system

Observed with `uname -a` and `cat /etc/os-release`:

~~~text
Linux mpc-one 6.6.95-az01-2025-06-27-rt55 #1 SMP PREEMPT_RT Fri Jun 27 17:24:33 UTC 2025 armv7l GNU/Linux

ID=az01
NAME="az0x (inMusic Brands AZ0x base distribution)"
VERSION="5.0.8 (scarthgap)"
VERSION_ID=5.0.8
VERSION_CODENAME="scarthgap"
PRETTY_NAME="az0x (inMusic Brands AZ0x base distribution) 5.0.8 (scarthgap)"
CPE_NAME="cpe:/o:openembedded:az01:5.0.8"
~~~

**OBSERVED:** ARMv7 Linux userspace, Linux 6.6.95 vendor kernel with PREEMPT_RT, inMusic az0x built on an OpenEmbedded/Yocto Scarthgap-era base. This does **not** establish the exact SoC part number, CPU core count, floating-point ABI, kernel defconfig, bootloader type or RT tuning. Collect these separately before deciding on a cross-compiler/sysroot.

## 3. Storage and mount topology

Captured from `mount` on the running device; this is mount evidence, **not** the complete eMMC partition table.

| Device/source | Mountpoint | Type | Observed access | Caution |
| --- | --- | --- | --- | --- |
| `/dev/mmcblk0p6` | `/` | ext4 | `ro` | Factory root; never remount RW during exploration |
| `/dev/mmcblk0p8` | `/data` | ext4 | `rw` | Persistent data; do not change without verified backups |
| `/dev/mmcblk0p8` | `/media/az01-internal` | ext4 | `rw` | A second mount of the same underlying partition, **not** a second storage pool |
| `/dev/mmcblk0p7` | `/media/acvs-content` | ext4 | `rw` | Content partition; factory data may be present |
| `overlay` | `/etc` | overlayfs | `rw` | Persistent upperdir `/data/system/etc/overlay`; writes survive reboot |
| `overlay` | `/var` | overlayfs | `rw` | Persistent upperdir `/data/system/var/overlay`; writes survive reboot |
| `overlay` | `/srv` | overlayfs | `rw` | Volatile upperdir under `/var/volatile/srv` |
| `tmpfs` | `/run` | tmpfs | `rw` | Temporary memory-backed runtime |
| `tmpfs` | `/tmp` | tmpfs | `rw` | Temporary memory-backed scratch |
| `tmpfs` | `/media`, `/secure-media` | tmpfs | `rw` | Runtime mountpoint roots |
| `/dev/mmcblk1p1` | `/media/3339-6530` | exfat | `rw,nosuid,nodev,noexec` | Secondary device/media observed; identity not yet mapped |
| `/dev/sda1` | `/media/USB321FD` | vfat | `rw,nosuid,nodev,noexec,nosymfollow,showexec`, etc. | Research USB; do not assume ELF binaries can execute in place |

**Critical implementation detail:** a read-only `/` does **not** imply an immutable `/etc` or `/var`. Their overlay upperdirs are on writable persistent `/data`. The installer must treat edits to these mounts as persistent changes, requiring transactional backup and rollback.

**UNVERIFIED:** meaning of eMMC partitions 1–5, bootloader environment, update slots, recovery mode, GPT/MBR details, boot selection, exact capacity, block-device wear, storage restore procedure. The next inventory is limited to read-only metadata (`/proc/partitions`, `lsblk` and `/sys/class/block/...`).

## 4. Networking

`ip addr` observed the MPC interface `end0` as UP, with IPv4 `10.42.0.141/24` and a link-local IPv6 address. A direct link returned three successful ICMP replies (approximately 0.27–0.30 ms in this session).

**UNVERIFIED:** exact Ethernet MAC controller, PHY, kernel driver, USB networking modes, default factory network configuration, DHCP arrangement for other users. The listed addresses belong only to the developer's laboratory profile.

## 5. Audio, MIDI, display, controls and application

**No validated hardware map yet.** The following are investigation targets rather than claims:

| Subsystem | Questions | Non-invasive evidence |
| --- | --- | --- |
| Audio hardware | ALSA cards/codecs, playback/capture PCM, sample formats, rates, buffer constraints, routing | `/proc/asound/*`, `aplay -l`, `arecord -l`, `/dev/snd`; **do not** start audio playback during stock app use |
| MIDI + control surface | Pad velocity/aftertouch, pad scan path, transport, Q-Link encoders, buttons, LEDs, DIN/USB MIDI | `/proc/asound/seq/clients` if present, `amidi -l`, `aconnect -l`, `/dev/input`; do not open exclusive devices |
| Display/touch | Display controller, framebuffer/DRM or compositing process, touch controller, input event nodes | `/proc/fb`, `/sys/class/graphics`, `/dev/dri`, `/proc/bus/input/devices` |
| DSP/latency | Which process owns audio, realtime scheduler usage, interrupt distribution, XRUN behavior | `ps`, `/proc/interrupts`, `/proc/sys/kernel/sched_rt_*`, read-only process metadata |
| Factory app | Actual executable, service name, startup dependencies, plugin and sample paths | `command -v`, `systemctl list-units`, limited `find` for path names only |
| Device tree | Board compatible string, SoC model, loaded drivers | `/proc/device-tree/{model,compatible}`, `/sys/bus`, `/proc/modules` |

Do **not** copy proprietary executables, embedded assets, activation data, license files, vendor firmware or complete factory image blobs into this repository. Use metadata, independently developed probes, diagrams and original code.

## 6. Safety boundary and next evidence

1. Do not write internal `/dev/mmcblk*`, remount `/` read-write, modify `/data/system`, alter boot variables or flash update images.
2. Preserve original service ownership until device and audio interfaces are mapped. Reading device enumeration is not the same as acquiring exclusive handles or sniffing private protocols.
3. Store raw inventory files **locally**, review and redact identifiers and potentially licensed paths before uploading or publishing.
4. Only after inventory: design read-only images of critical partitions, cryptographic verification, and **independently verified restoration** before persistent experiments.
5. Use the [read-only collector](../../tools/mpc-one/collect-over-ssh.sh) from the workstation; add new OBSERVED findings, exact commands and dates here after review.

## 7. Confidence and scope

| Claim | Confidence |
| --- | --- |
| Working root SSH using autorun on this particular firmware/device | OBSERVED |
| Linux/az0x versions, ARMv7, real-time kernel | OBSERVED |
| Mounts and persistent overlay upperdirs | OBSERVED |
| Complete eMMC/bootloader/firmware/recovery layout | UNVERIFIED |
| Pad and encoder protocol/LED control | UNVERIFIED |
| Audio driver, exclusive ownership and performance | UNVERIFIED |
| Compatibility on other MPC One firmware or models | UNVERIFIED |
| Ability to boot independent kernel or an installer without risks | UNVERIFIED |

Updates to this document must not silently upgrade an inference to a confirmed fact.
