# Akai MPC One 3.6.0 — focused service, device-ownership and graphics survey

> **Evidence class:** OBSERVED on one owner-authorized MPC One. Firmware 3.6.0 is the owner's UI report, not a value re-read in this survey. The actual survey began at approximately **2026-11-09 05:21:06 UTC**.
>
> **Original, locally retained report SHA-256:** `d25be98aea6e90cc0328c0af4d29c505106dcd26704da7dc1f6f5bd59cec317e`
>
> **Source collector:** `tools/mpc-one/read-only-detail.sh` at branch commit `b2632ae603d844c92968eaec9f2634d2ae2e880a`. The report is **not** in GitHub: it includes a workstation-specific IP/MAC. This file includes only redacted, non-unique device values. Do not publish factory binaries or a raw filesystem/USB dump.

Companion baseline: [first hardware survey](archive/2026-11/survey-2026-11-09.md). The device's kernel is `6.6.95-az01-2025-06-27-rt55`, OpenEmbedded-derived az0x `5.0.8`, ARMv7 on Rockchip RK3288. Neither survey modified internal partitions, services, PCM streams, raw-MIDI streams, or DRM output.

## 1. Most important finding — actual live factory device usage

The factory user-facing application is `acvs.service`. Systemd reports:

~~~text
acvs.service
  Type=simple
  MainPID=311
  ExecMainPID=311
  ExecStart=/usr/bin/az01-launch-MPC
  FragmentPath=/usr/lib/systemd/system/acvs.service
  ActiveState=active
  SubState=running
~~~

The process `/proc/311/comm` appeared as `MPC Main Thread`. **PID 311 is a transient session value** and must never be hardcoded in tooling. Obtain the current PID from `systemctl show acvs.service -p MainPID`.

### File-descriptor ownership map

Observed by `readlink /proc/<acvs-MainPID>/fd/*`, **without opening any hardware device**:

| fd on this session | Kernel node | Function of node | What this proves |
| ---: | --- | --- | --- |
| 11 | `/dev/dri/card0` | DRM/KMS display card | Factory process has a display-device handle |
| 22 | `/dev/snd/seq` | ALSA sequencer | Factory process opens ALSA sequencing |
| 34, 35, 36 | `/dev/snd/midiC0D0` | Raw USB-MIDI | Factory process opens the same MIDI character node three times |
| 42 | `/dev/snd/controlC1` | ACVA ALSA controls | Factory process can query/control ACVA mixer/card |
| 52 | `/dev/snd/pcmC1D0p` | ACVA playback PCM | Factory process has a playback handle |
| 54 | `/dev/snd/pcmC1D0c` | ACVA capture PCM | Factory process has a capture handle |
| 7 | `/dev/input/event0` | ILI2117 touchscreen | Factory process receives touch evdev |
| 8 | `/dev/input/event1` | GPIO key events | Factory process receives GPIO key evdev |

The simultaneous `/proc/asound/card1/pcm0p/info` and `pcm0c/info` each showed `subdevices_count: 1` and `subdevices_avail: 0`. This is consistent with the vendor application occupying the only exposed PCM subdevice for each direction at that moment. **Do not assume sharing is possible, and do not open either PCM device concurrently** until a planned opt-in test.

**Important limitations:**
- A present fd does **not** prove exclusive ownership or document all ioctls/buffer settings used.
- The three fd entries for `midiC0D0` do **not** identify which is Public/Private/MIDI Port; all are links to a shared device node. Correlate subdevice selection by later approved diagnostics.
- No pad traffic, button event data, PCM samples, GPU buffers, or vendor API calls were captured.
- The stock application was not paused, stopped, restarted, or replaced.

### Implication for an independent userspace

These observations justify **prototyping** an MPFree Linux hardware abstraction over ALSA (PCM/control/seq/rawmidi), evdev, and DRM/KMS, while **reusing** the kernel and board drivers. They do not yet prove that a new app can reproduce all vendor routing, control surface feedback, or graphics composition.

Avoid a dangerous oversimplification: the vendor MIDI/private control interface and JP07 audio hardware may still require reverse-engineered message layouts or exact ALSA control setup. Standard device nodes do not automatically imply a documented, portable hardware protocol.

## 2. Other running services and lifecycle dependencies

~~~text
az01-network-midi.service
  MainPID=302
  ExecStart=/usr/bin/start-remote-midi.sh

az01-script-runner.service
  MainPID=309
  ExecStart=/usr/bin/az01-script-runner
~~~

The process list also contained `DeviceControlSe` (truncated kernel `comm` name); its identity and relationship to network MIDI **are not yet proven**. Do not stop network MIDI or the script runner merely because they sound optional.

Before attempting a reversible UI/audio takeover, map unit dependencies (`Requires`, `After`, `PartOf`, `Restart`), the true main executable's `/proc/<pid>/exe` symlink, and any active helper processes. An immediate service stop can interrupt normal playback, saving, control surface feedback, or power handling.

## 3. ALSA audio ownership, capabilities and unknowns

~~~text
Card 0: USB-Audio — MPC One MIDI
Card 1: ACVA — ACVA
01-00: ACVA PCM inmusic,acva-audio-codec-0
       playback 1 : capture 1
~~~

`/proc/asound/card1/pcm0p/info`:

~~~text
card: 1
device: 0
subdevice: 0
stream: PLAYBACK
subdevices_count: 1
subdevices_avail: 0
~~~

`/proc/asound/card1/pcm0c/info` correspondingly reported `CAPTURE`, a single subdevice, and no available subdevices.

**Next evidence, without opening ALSA:** read `/proc/asound/card1/pcm0{p,c}/sub0/hw_params` and `/proc/asound/card1/pcm0{p,c}/sub0/status` if they exist and are readable. The procfs `hw_params` may reveal the **currently negotiated** sample format, rate, channels, period/buffer sizes while the factory application is using the PCM. That is a snapshot of the active session, **not** the set of *all* formats/rates supported by the driver. Do not change mixer controls or invoke playback/capture tools during enumeration.

The prior survey identified vendor ASoC modules `snd_soc_inmusic_jp07`, `snd_soc_rockchip_inmusic_jp07`, and `snd_soc_rockchip_i2s`. The external codec silicon and routing remain unverified.

## 4. Display and GPU: actual modes, not assumptions

Device node and connector metadata:

~~~text
DRM connector: card0-DSI-1
  status: connected
  enabled: enabled
  modes: 800x1280

Framebuffer:
  /proc/fb: 0 rockchipdrmfb
  virtual_size: 800,3840
  bits_per_pixel: 32

Backlight:
  mipi-backlight, max_brightness: 255
  observed brightness: 106
~~~

GPU driver binding in `/sys/class/drm/card1/device/uevent`:

~~~text
DRIVER=panfrost
OF_COMPATIBLE_0=rockchip,rk3288-mali
OF_COMPATIBLE_1=arm,mali-t760
~~~

**OBSERVED:** Mali-T760-compatible GPU with Panfrost, DRM render node `renderD128`, an enabled DSI output at `800x1280` and 32-bit framebuffer. The virtual buffer height is **3 × 1280**, which is *consistent with* multiple frame buffers, but not proof of a specific buffering algorithm.

**UNVERIFIED:** panel orientation relative to physical landscape placement, how the application rotates/transforms frames and touch coordinates, actual EGL/GLES stack, active DRM master, video memory footprint, whether modesetting is safe. Do not obtain DRM master or perform a modeset while the stock UI is active.

For desktop simulator parity, the device profile should store the **reported KMS mode (800x1280)** separately from an unverified **logical UI orientation**. Do not hardcode `1280x800` as the Linux connector mode.

## 5. Touchscreen and input capabilities

~~~text
/dev/input/event0 — ILI2117 Touchscreen; bustype 0018 (I²C)
  capabilities/ev: b
  capabilities/key: 400 ...
  capabilities/abs: 2608000 3

/dev/input/event1 — gpio-keys; bustype 0019
  capabilities/ev: 3
  capabilities/key: 100000 ...
~~~

**OBSERVED:** the touchscreen and GPIO keys use Linux evdev and the stock process has both handles open. Exact ABS axis ranges, multitouch tracking slots and coordinate transform are **not** determined by these capability bitmaps. Later, read the relevant evdev `abs` ioctl data only in a controlled non-disruptive test, or infer from read-only kernel driver/device-tree properties if exposed.

Physical pads and encoders still have **no positively identified evdev node** in the current survey. The internal USB MIDI interface remains the stronger candidate, not a confirmed message-level protocol.

## 6. Internal USB MIDI — USB ID and bus topology

The kernel enumerated this actual device:

~~~text
Path:        /sys/bus/usb/devices/1-1
Vendor ID:   09e8
Product ID:  0046
Manufacturer: Akai Pro
Product:      MPC One MIDI
bcdDevice:    0010
Speed:        12 Mbit/s (USB full speed)
Interfaces:   2
~~~

The manufacturer's interface names from the earlier `amidi -l` were:

~~~text
hw:0,0,0  MPC Studio Live Public
hw:0,0,1  MPC Studio Live Private
hw:0,0,2  MPC Studio Live MIDI Port
~~~

The research FAT32 USB was independently identified as a different device, PNY `154b:1006`, on the second USB bus. This helps separate the embedded controller from removable media in future investigations. Other exposed root hub/controller IDs were Linux `1d6b` entries (DWC OTG, EHCI, OHCI).

**Next evidence:** inspect only USB *interface descriptors* such as `bInterfaceClass`, `bInterfaceSubClass`, `bInterfaceProtocol`, and read-only ALSA RawMIDI metadata. Do not send SysEx or other MIDI bytes until the command format and risk are understood.

## 7. CPU clocks and runtime ABI

All four CPU sysfs frequency domains reported:

~~~text
scaling_governor: performance
scaling_cur_freq: 1608000   # kHz, i.e. ~1.608 GHz
cpuinfo_max_freq: 1608000   # kHz
~~~

This is a **momentary** runtime observation, not a validated full-speed DSP budget under thermal load. The clock could throttle, and the heavy locked-memory footprint remains unassigned to a specific process.

The root filesystem exposes:

~~~text
/lib/ld-linux-armhf.so.3
/lib/libc.so.6
/usr/lib/libasound.so.2
~~~

**DERIVED:** a compatible ARMv7 Linux **hard-float** sysroot is the likely toolchain choice. Verify the `/usr/bin/MPC` ELF interpreter/flags or a small independent read-only ELF header before treating that as confirmed. A loader pathname does not establish the full vendor application ABI.

**Collector bug:** BusyBox `head` rejected `-c` and BusyBox `od` rejected `-An`. The intended ELF header was **not** captured; do not claim the header was inspected. Next collector must use known-supported BusyBox tools such as `dd if=/usr/bin/MPC bs=64 count=1` piped into `od -b` (read-only; no `of=`). The report should contain only the 64-byte header, not executable contents.

## 8. Proposed architecture boundaries (not implemented yet)

~~~text
MPFree engine, independent of device
  |-- Audio engine: realtime graph, no allocation in callback
  |-- Sequencer/clock: samples/ticks with explicit sync conversion
  |-- Sample manager: fixed memory budget and streaming
  |-- UI model: normalized logical coordinates and hardware event pipeline
  |
  +-- HAL implementations
       |-- Az01AlsaAudio: ACVA card 1 PCM+control
       |-- Az01Midi: card 0 RawMIDI/ALSA sequencer, per-port verification pending
       |-- Az01Touch: evdev event0, calibration/rotation pending
       |-- Az01Display: DRM/KMS card0 + Panfrost rendering, rotation pending
       |-- Az01Storage: read-only root, persistent data/overlays (no writes in survey)
       +-- StockProcessSupervisor: can detect acvs.service and resource usage
               but CANNOT stop or change the factory service without opt-in
~~~

The eventual deployment needs an explicit **resource ownership state machine**:

~~~text
READ_ONLY_SURVEY
  -> VERIFIED_STOCK_IDLE (can enumerate and inspect only)
  -> BACKUP_AND_RECOVERY_VERIFIED (hard prerequisite)
  -> USER_CONSENT_AND_KNOWN_SERVICE_MAP
  -> SESSION_ONLY_TAKEOVER
  -> RESTORE_STOCK (must work on crash/timeout/cancel)
~~~

**Current permitted state is READ_ONLY_SURVEY.** A working SSH connection is not a reason to start overwriting internal storage or to kill the stock GUI. The future USB installer must never assume its ability to obtain root means the target version supports a safe persistent installation.

## 9. What was and was not verified

| Claim | Classification |
| --- | --- |
| `acvs.service` main process opens PCM playback+capture, RawMIDI, ALSA sequencer, DRM and evdev | OBSERVED |
| One capture and one playback subdevice; neither available at that snapshot | OBSERVED |
| Actual current PCM negotiated sample rate/format | UNVERIFIED (procfs follow-up) |
| MIDI port names, USB VID/PID and 12 Mbit/s bus speed | OBSERVED |
| Which MIDI bytes represent pads or Q-Link/LED operations | UNVERIFIED |
| DSI connector 800x1280; DRM virtual buffer 800x3840; 32 bpp | OBSERVED |
| Mali-T760-compatible GPU with Panfrost driver | OBSERVED |
| Physical LCD orientation and application presentation transform | UNVERIFIED |
| Four cores at ~1.608 GHz on performance governor | OBSERVED at survey time |
| Hard-float dynamic loader path exists | OBSERVED; ELF ABI matching still UNVERIFIED |
| Process exclusivity and reversible service lifecycle | UNVERIFIED |
| A reliable recovery flash path if eMMC boot fails | UNVERIFIED |

### Reproduction

On the **workstation**, not on the device:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --detail
~~~

The wrapper verifies the known interface, IP, ARP MAC and SSH host key and saves the report locally under `research/`. After the follow-up fix, future surveys should separate ABI and thread/PCM metadata into a third, similarly read-only mode.

Do not commit the original report; preserve its hash above as provenance.
