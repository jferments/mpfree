# MPC One 3.6.0 — read-only hardware survey, 2026-11-09 UTC

> **Evidence class:** OBSERVED on one owner's MPC One, reported UI firmware 3.6.0; report from the repository's read-only SSH inventory collected on 2026-11-09 UTC.
>
> **Original local evidence SHA-256:** `4172d29e33359eee106cfcf03665d3a94cd4e4deed75d009a1fe1065ea53850b`
>
> **Privacy:** Original 32,826-byte output is **NOT** in the repository. It contained a device-unique serial number and locally identifying network information. This report paraphrases technical observations, retaining only generic board/device identifiers. Do not upload the original raw report into Git.

The collector ran over the already-established USB-autorun temporary SSH on TCP 2222. It made no deliberate persistent device writes, service changes, device opens for playback/capture or firmware modifications. Some utility calls returned no useful output because this OpenEmbedded image uses BusyBox and does not ship the desktop utilities assumed by the first collector. Those failures are marked UNKNOWN, not "absent feature".

## 1. Board, SoC, CPU and toolchain

### Raw evidence (redacted)

~~~text
/proc/device-tree/model: Akai Professional MPC One
/proc/device-tree/compatible: inmusic,acva inmusic,az01 rockchip,rk3288
Hardware: Rockchip (Device Tree)
processor: 0, 1, 2, 3
CPU implementer: 0x41
CPU architecture: 7
CPU part: 0xc0d
CPU revision: 1
Features: half thumb fastmult vfp edsp neon vfpv3 tls vfpv4 idiva idivt vfpd32 lpae
Linux 6.6.95-az01-2025-06-27-rt55 PREEMPT_RT armv7l
Compiler string: arm-poky-linux-gnueabi-gcc 13.3.0
Linker string: GNU Binutils 2.42.0.20240723
Distributor: inMusic az0x 5.0.8 (scarthgap)
~~~

**OBSERVED:** The device tree identifies `rockchip,rk3288`. Four logical CPUs were listed. This is a 32-bit ARMv7 target, not AArch64. NEON + VFPv4 are advertised, but these flags alone do not establish the exact userspace floating-point calling convention, `ld-linux` interpreter or glibc version; inspect actual ELF metadata before choosing a sysroot.

**IMPORTANT CPU identity nuance:** `0xC0D` is an RK3288-specific MIDR part number. Rockchip markets the RK3288 CPU as Cortex-A17, but the open-source `cpuinfo` mapping comments that A12/A17 nomenclature is ambiguous for part `0xC0D` and treats it as Cortex-A12 for microarchitecture selection. Do not use `0xC0E` (ordinary A17 ID) as a required match for this board. Source: https://github.com/pytorch/cpuinfo/blob/main/src/arm/uarch.c ; manufacturer: https://www.rock-chips.com/a/en/products/RK32_Series/2014/0504/484.html . These are **external references** distinct from the MPC's observed hardware report.

**UNVERIFIED:** actual frequencies/governor, `/proc/device-tree` revisions, CPU affinity of audio thread, ABI loader, complete library inventory, and GPU device ID / shader-core topology.

### RAM

~~~text
MemTotal:       2067188 kB        (~2019 MiB)
MemAvailable:    882876 kB        (~862 MiB at survey time)
Mlocked:        1039452 kB        (~1015 MiB)
Unevictable:    1051660 kB        (~1027 MiB)
HighTotal:      1310720 kB        (1280 MiB)
LowTotal:        756468 kB        (~739 MiB)
SwapTotal:            0 kB
~~~

**OBSERVED:** ~2 GiB physical RAM accessible to Linux, with no swap and substantial locked/unevictable memory at sampling time. More than 1 GiB reported as `Mlocked` should be investigated: **do not assume which process locked it**. On a 32-bit ARM system with highmem, direct kernel allocations, DMA and contiguous-memory assumptions require specific attention.

**DESIGN CONSEQUENCE:** MPFree must be budgeted for a constrained, realtime device rather than a desktop computer. Start with small preallocated audio buffers and bounded sample cache; prefer streaming large samples when the filesystem/audio performance is measured.

## 2. eMMC topology — a newly confirmed partition-label map

The first survey provides **kernel PARTNAME evidence**, not merely guessed partition roles:

| Node | PARTNAME (observed via sysfs) | Size | Observed mounted role | Danger |
| --- | --- | ---: | --- | --- |
| `/dev/mmcblk0p1` | `uboot-spl` | 64 KiB | not mounted | boot-critical; never write |
| `/dev/mmcblk0p2` | `env` | 8 KiB | not mounted | boot environment; never write |
| `/dev/mmcblk0p3` | `uboot` | 384 KiB | not mounted | bootloader; never write |
| `/dev/mmcblk0p4` | `splash` | 4 MiB | not mounted | not for experiments |
| `/dev/mmcblk0p5` | `recoverysplash` | 4 MiB | not mounted | **name is not proof of a recovery OS** |
| `/dev/mmcblk0p6` | `rootfs` | 500 MiB | `/`, ext4, read-only | vendor root filesystem |
| `/dev/mmcblk0p7` | `content` | 4 GiB | `/media/acvs-content`, ext4, rw | proprietary factory content |
| `/dev/mmcblk0p8` | `data` | ~2.8 GiB | `/data` AND `/media/az01-internal`, ext4, rw | persistent user/config data |

The kernel also reports a `mmcblk0` device of ~7.3 GiB and separate 4 MiB hardware boot regions `mmcblk0boot0` and `mmcblk0boot1`. These are **not ordinary partitions**; no assumptions about whether the boot ROM actually uses them.

### Storage diagram

~~~text
Internal eMMC: mmcblk0 (~7.3 GiB)
  p1   64 KiB   "uboot-spl"    [boot sensitive, unmounted]
  p2    8 KiB   "env"          [boot sensitive, unmounted]
  p3  384 KiB   "uboot"        [boot sensitive, unmounted]
  p4    4 MiB   "splash"       [unmounted]
  p5    4 MiB   "recoverysplash" [unmounted]
  p6  500 MiB   "rootfs"       -> /                 [ext4, ro]
  p7    4 GiB   "content"      -> /media/acvs-content [ext4, rw]
  p8  ~2.8 GiB  "data"         -> /data AND /media/az01-internal [ext4, rw]
  boot0 4 MiB   eMMC boot region — purpose unconfirmed
  boot1 4 MiB   eMMC boot region — purpose unconfirmed

Secondary MMC: mmcblk1 (~59.5 GiB)
  p1 exFAT -> /media/3339-6530 [device type not yet conclusively established]

Research USB: /dev/sda (~57.8 GiB)
  p1 FAT32 -> /media/USB321FD [USB autorun stick]
~~~

### Filesystem overlays — important installer risk

- `/` is an ext4 **read-only** mount from p6.
- `/etc` and `/var` are **writable overlayfs** whose upperdirs lie in persistent `/data/system/etc/overlay` and `/data/system/var/overlay`.
- `/srv` has a volatile overlay under `/var/volatile/srv`.
- `/run` and `/tmp` are RAM-backed.
- FAT32/exFAT removable mounts use `noexec` and `nosuid` options; standalone ELF execution is not guaranteed directly from USB.

Space at survey time:
- `/`: ~382.3 MiB usable filesystem, 96% used, read-only (filesystem size is less than 500 MiB partition size; not necessarily corruption).
- `/data`: ~2.7 GiB filesystem, ~201.6 MiB used, ~2.3 GiB available.
- `/media/acvs-content`: ~3.9 GiB filesystem, ~3.0 GiB used (77%).
- Research USB: ~57.7 GiB available, ~576 KiB occupied.

**UNVERIFIED:** boot ROM chain, offsets, backup format, boot-recovery switch, boot fallback, hardware reset path, and viable restoration when Linux cannot boot. Do not infer a working recovery partition solely from `recoverysplash`. Partition labels are not a license to overwrite any partition.

## 3. Audio codec and ALSA kernel devices

### Observed kernel card table

~~~text
0 [MIDI ]: USB-Audio - MPC One MIDI
             Akai Pro MPC One MIDI at usb-ff540000.usb-1, full speed

1 [ACVA ]: ACVA - ACVA

01-00: ACVA PCM inmusic,acva-audio-codec-0 :
       playback 1 : capture 1
~~~

### Observed ALSA nodes

| Node | Role indicated by kernel name |
| --- | --- |
| `/dev/snd/controlC0` | USB MIDI control |
| `/dev/snd/midiC0D0` | Raw USB MIDI character device |
| `/dev/snd/controlC1` | ACVA ALSA control |
| `/dev/snd/pcmC1D0p` | ACVA playback PCM |
| `/dev/snd/pcmC1D0c` | ACVA capture PCM |
| `/dev/snd/seq` | ALSA sequencer |
| `/dev/snd/timer` | ALSA timer |

Observed loaded modules: `snd_soc_inmusic_jp07`, `snd_soc_rockchip_inmusic_jp07`, `snd_soc_rockchip_i2s`. This is strong evidence of a vendor-specific Linux ASoC audio interface built around Rockchip I²S, but the exact physical codec, audio pin routing, DMA buffer properties, active owner and control interface remain **UNVERIFIED**.

**Collector limitation:** `aplay`, `arecord` and `aconnect` did not produce usable output; the image only positively located `amidi`. Missing commands are **not** evidence that playback/capture or ALSA sequencing cannot work.

**NEXT:** enumerate `/proc/asound/card1` and `/sys/class/sound` metadata, identify the owning application and loaded PCM library, then — only in an explicitly approved non-concurrent test — determine exact sample formats, rates, buffer layout, capture/playback routing and minimum stable latency. Do not open `/dev/snd/pcmC1D0p` merely for reconnaissance while the MPC application owns it.

## 4. Internal USB MIDI and control surfaces

`amidi -l` enumerated on this **MPC One**:

~~~text
Dir Device    Name
IO  hw:0,0,0  MPC Studio Live Public
IO  hw:0,0,1  MPC Studio Live Private
IO  hw:0,0,2  MPC Studio Live MIDI Port
~~~

These names are manufacturer labels on internal USB MIDI endpoints. The names **do not** mean the physical device is an MPC Live. Whether each carries pad input, Q-Link encoder values, lights, transport control, or external MIDI is still **UNVERIFIED**.

Observed USB controller path for MIDI: `ff540000.usb`; ALSA by-id symlink `usb-Akai_Pro_MPC_One_MIDI-00`. There are three USB buses listed in sysfs. USB IDs and descriptors were not yet obtained because `lsusb` supplied no meaningful output.

**NEXT:** read USB descriptors (`/sys/bus/usb/devices/*/{idVendor,idProduct,manufacturer,product,bInterfaceClass}`) and ALSA rawmidi metadata without issuing messages. Later, use an opt-in **passive, nonblocking, short-duration capture** and record controlled button/pad press timestamps; investigate whether the factory app can share the stream safely before opening it.

## 5. Display, GPU and touchscreen

### Display/GPU (observed)

~~~text
/proc/fb: 0 rockchipdrmfb
/dev/fb0
/dev/dri/card0
/dev/dri/card1
/dev/dri/renderD128

/sys/class/drm/card0-DSI-1
/sys/class/drm/card1 -> .../platform/ffa30000.gpu/...
/sys/class/drm/renderD128 -> .../platform/ffa30000.gpu/...
/sys/class/backlight/mipi-backlight
/proc/interrupts: panfrost-gpu, panfrost-mmu, panfrost-job
~~~

**OBSERVED:** the display subsystem has a DSI connector, a DRM framebuffer, and a separate GPU render device. The kernel has Panfrost GPU-related interrupts. **DERIVED:** use DRM/KMS, or an existing compositor if one can be identified, for a future UI; hardware acceleration via EGL/GLES may be viable. **UNVERIFIED:** actual GPU SKU/driver binding details, DRM mode, pixel format, rotation, compositor state and rendering performance. Do not take DRM master or modeset during the stock MPC session.

Manufacturer RK3288 documentation describes a Mali T76x GPU family, but the board-specific GPU configuration remains unverified. Do not hardcode a particular model until sysfs reports a compatible ID.

### Touch/input (observed)

~~~text
I: Bus=0018
N: Name="ILI2117 Touchscreen"
S: Sysfs=/devices/platform/ff160000.i2c/i2c-4/4-0026/input/input0
H: Handlers=event0

I: Bus=0019
N: Name="gpio-keys"
H: Handlers=kbd event1
~~~

- `/dev/input/event0` — ILI2117 touchscreen on I²C bus 4, address 0x26 (derived from `4-0026`).
- `/dev/input/event1` — gpio-keys (likely power/system key; exact key code unverified).

No separate evdev pad/encoder devices were enumerated. This is **consistent with** control-surface input arriving over internal MIDI, but doesn't prove it.

**NEXT:** inspect the readonly capability/range files under `/sys/class/input/event0/device` and DRM connector mode files, then plan a later opt-in event stream capture without conflicting with the factory GUI.

## 6. Application processes and service management

- `/usr/bin/MPC` exists, observed size **106,115,596 bytes** (~101.2 MiB), executable.
- `/usr/bin/az01-launch-MPC` exists by filename.
- Running `acvs.service` is labeled **"InMusic MPC Application"** — this is the service to investigate, **not** hypothetical `inmusic-mpc.service`.
- Other observed services include `az01-network-midi.service`, `az01-script-runner.service`, `az01-power-button.service`, `az01-unsupported-device-daemon.service` and `az01-usbsata-fixer.service`.
- `/opt/ramfs-update.squashfs` exists (3,551,232 bytes): **do not mount, overwrite or assume it is a complete recovery environment** until investigated.
- Directory paths under `/usr/share/Akai/Content/Synths` reveal numerous factory sound/synth/effect labels, **not** reusable plugin ABIs or licenses.

**Collector bug:** `ps -eo pid,ppid,comm | head` emitted nothing without triggering the fallback because a pipeline can exit successfully when the command before `head` fails; similarly `ip -br addr` is not supported by the BusyBox `ip` applet. Therefore the absence of process output is a **collector limitation**. New targeted read-only probes should use `/proc/[0-9]*/comm`, standard `ps` only when supported, `systemctl show acvs.service`, `ip addr` and `ip route`.

**NEXT:** inspect actual unit metadata and `/proc/<acvs-main-pid>/exe` symlink (not binary contents), process scheduling policy, loaded-library *names* and open-device link targets **without opening those devices**. Preserve application ownership; do not stop or restart `acvs.service` during discovery.

## 7. Kernel scheduling and interrupts

~~~text
sched_rt_runtime_us = 950000
sched_rt_period_us  = 1000000
~~~

Linux default RT bandwidth values (95% per 1-second window). This **does not** establish the scheduling policy or priority of the audio thread. The IRQ table reported the Rockchip VOP/display, `panfrost`, I²C, USB, touch, storage, `end0` Ethernet, watchdog, DMA and audio-adjacent components, but no explicit ALSA scheduling information. Investigate `/proc/<pid>/task/<tid>/sched` and thread priorities with read-only sampling before changing tuning.

## 8. Development implications and immediate gates

### What is now well supported

1. **Target architecture:** cross-compiled **32-bit ARMv7** binaries with NEON/VFPv4 if ABI and compiler flags match. Build on Linux workstation, not on the limited MPC.
2. **Reuse vendor kernel initially:** existing PREEMPT_RT, DRM, evdev and ALSA enable a potential independent userspace without a new kernel.
3. **Proposed hardware adapter split:** `AudioBackend::ACVA`, `MidiBackend::InternalUSB`, `TouchBackend::EvdevILI2117`, `DisplayBackend::RockchipDRM`, `StorageBackend::Az01MountedVolumes`. These are *proposed module labels*, not confirmed implementations.
4. **Treat stock `acvs.service` as owner** until actual process tree and device handles are mapped.
5. **Strict installer requirements:** no writes to `mmcblk0`, firmware, bootloader, vendor content, or persistent overlays until a proven off-device backup and independent restore mechanism exists.

### Next read-only evidence tasks

- Fix BusyBox utility detection; harvest process names, stock service unit metadata, per-process schedulers and open-device symlink *metadata*.
- Record audio sysfs and DRM connector properties without opening hardware for playback, capture, or modesetting.
- Record USB controller/interface descriptor IDs and touchscreen absolute-axis capability ranges.
- Inspect the existing MPC executable's *dynamic linker and ABI metadata* without copying binary code into GitHub.
- Create a cautious rescue/backup **design** before any request to read raw partitions; a hashed image without validated restoration is not sufficient.

### Remaining unknowns

Physical codec silicon and register map; audio sample formats/rates; IRQ-to-audio mapping; MIDI message encoding; pad velocity mapping; proprietary/plugin host ABI; DRM supported display modes; GUI service lifecycle; full boot ROM and recovery path; ABI loader and glibc; per-process memory budget; restore-from-brick procedure; other models/firmware compatibility.

## 9. Provenance / reproducibility

Collector source: `tools/mpc-one/read-only-inventory.sh` at initial captured branch head `b8f39393ec84d4315e564b487bc1be6284dc7cbb`; report was generated over the known direct Ethernet SSH link. Filename local only: `research/mpc-one-20261109T050046Z-Lb4Yxp.txt`. Fingerprint in opening note allows verifying the original private evidence without distributing it.

**Redaction policy:** The original `/proc/cpuinfo` contained an MPC-specific hardware serial. It has been intentionally omitted. No raw partition bytes, factory instrument binaries, sound libraries, or user samples are included. Third-party source references, where used, are clearly distinguished from observations on the MPC.
