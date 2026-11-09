# MPC hardware reverse-engineering research matrix

**Objective:** a reproducible, legally distributable, evidence-based map of MPC One firmware/hardware interfaces that other developers can use without proprietary firmware copies. This is a work queue and evidence schema. Nothing marked UNKNOWN should be interpreted as a published hardware specification.

Reference baseline: [MPC One 3.6.0](MPC_ONE_3_6_0_BASELINE.md). Latest redacted evidence: [initial 2026-11-09 survey](MPC_ONE_3_6_0_SURVEY_20261109.md) and [focused hardware-ownership survey](MPC_ONE_3_6_0_RESOURCES_20261109.md). Read-only collection: [developer access](DEVELOPER_ACCESS.md).

## Evidence procedure

For each finding, capture:
- `device_model`, `hardware_revision` (if visible), `firmware_ui_version`, `uname`, `os_release`, UTC date;
- exact command or code/fixture used and SHA-256 of the raw local report;
- redacted, minimal evidence excerpt (no unique IDs, secrets, or licensed content);
- state `OBSERVED`, `DERIVED` or `UNVERIFIED`;
- a reproducible validation plan and a falsification condition;
- how the result affects the MPFree hardware-abstraction interface;
- differences between firmware versions, MPC One vs One+ vs Live/Force rather than assuming parity.

Organize evidence into per-firmware notes and cross-version comparison tables. **Do not** upload raw partition images or proprietary executables.

## Mapping matrix and gates

| ID | Subsystem | Current knowledge | Next safe question | Required evidence | Implementation output |
| --- | --- | --- | --- | --- | --- |
| SYS-01 | Board/SoC | **OBSERVED:** device tree `rockchip,rk3288`, `inmusic,acva`, `inmusic,az01` | Confirm SoC revision and binding of active kernel drivers | Device-tree `compatible`, driver symlink metadata | Stable platform identification |
| SYS-02 | CPU/RAM | **OBSERVED:** four ARMv7 RK3288 CPUs, NEON/VFPv4, **performance @ 1,608 MHz**; ~2 GiB RAM, ~1 GiB `Mlocked` (owner unknown) | Sample thread priority, runtime CPU affinity, memory lock owner | `/proc/<pid>/task`, `/proc/<pid>/status`, `/sys/devices/system/cpu` | ARMv7 RT/memory performance budget |
| SYS-03 | Kernel/modules | **OBSERVED:** 6.6.95 PREEMPT_RT; I²S/JP07 ASoC modules, Panfrost IRQ; sched RT 950000/1000000 | Audio thread priorities, IRQ affinity, true kernel-config features | `/proc/<pid>/task`, `/proc/interrupts`, sysfs | Real-time tuning evidence |
| BOOT-01 | Internal eMMC | **OBSERVED:** 7.3 GiB eMMC with PARTNAME labels `uboot-spl`, `env`, `uboot`, `splash`, `recoverysplash`, `rootfs`, `content`, `data`; boot0/boot1 also present | Read-only boot-chain metadata and validated restoration study | `/sys/class/block/mmcblk0p*/uevent`, partition metadata | Partition reference map **without** writes |
| BOOT-02 | Boot/update chain | UNKNOWN | Is there A/B switching, U-Boot and recovery without Linux? | Non-destructive vendor documentation, read-only boot metadata | Proven recovery prerequisite |
| FS-01 | Overlayfs | `/etc` and `/var` persistent upperdirs observed | Which paths are mutable and survive reboot? | `mount`, `/proc/mounts`, optional benign isolated tests **only after consent** | Runtime/installer path constraints |
| FS-02 | USB execution | Research USB showed `noexec` mount | Which deployment paths support ARM ELF? | `/proc/mounts`, loader inspection; executable tests only in isolated phase | Portable loader strategy |
| APP-01 | Factory GUI | **OBSERVED:** `acvs.service` active; dynamic MainPID has DRM, ALSA PCM playback/capture, control, seq, triple RawMIDI, touch and gpio-key fds | Which threads own audio, GPU, MIDI and what are service dependencies? | `systemctl show acvs.service`, `/proc/<pid>/task`, fd symlink metadata | Safe takeover supervisor design |
| APP-02 | Libraries/plugins | UNKNOWN | What libraries load and which formats are public? | dependency and path metadata; separate local legal review | Clean-room API design |
| AUD-01 | Audio devices | **OBSERVED:** ACVA card1 PCM playback/capture, one subdevice each, **0 available while stock app runs** | Negotiated format/rate/period from procfs, then codec routing and capabilities | `/proc/asound/card1/pcm0{p,c}/sub0/hw_params`, no opens | ACVA driver backend prototype |
| AUD-02 | Audio ownership | **OBSERVED:** `acvs.service` MainPID holds `/dev/snd/pcmC1D0p`, `pcmC1D0c`, `controlC1`, `/dev/snd/seq`, three raw-MIDI fds | Is it exclusive? How to restore exact normal ownership after a later test? | Existing `/proc/<pid>/fd`, ALSA subdevice availability; later opt-in trial | Arbitration/ownership policy |
| AUD-03 | Realtime performance | UNKNOWN | Rate, periods, xruns, latency? | Controlled **opt-in** audio test with factory app idle/stopped safely | Audio engine budgets |
| MIDI-01 | Controller protocol | **OBSERVED:** USB `09e8:0046` full-speed 12 Mbit/s Akai MPC One MIDI (two interfaces), RawMIDI 3 subports, stock app opens `/dev/snd/midiC0D0` three times | Map USB interface descriptors, then physical pad/encoder event semantics | USB sysfs interface descriptors; later controlled passive capture | Versioned MIDI mapping table |
| MIDI-02 | LED/output | UNKNOWN | LED/control feedback protocol? | Later active opt-in experiments, no unsolicited writes | Bidirectional controller backend |
| UI-01 | Display | **OBSERVED:** DRM `card0-DSI-1` enabled `800x1280`, framebuffer virtual `800x3840` @ 32 bpp, Mali-T760-compatible Panfrost GPU | Physical panel orientation, transforms, compositor ownership | DRM sysfs, `/proc/<pid>/fd`, later GUI isolation tests | DRM/KMS display backend |
| UI-02 | Touchscreen | **OBSERVED:** ILI2117 `event0` opened by factory app; touch capability bitmasks captured | Absolute ranges, calibration/rotation, multi-touch tracking | sysfs/device tree; later non-interfering opt-in capture | Touch adapter |
| UI-03 | Hardware keys | **OBSERVED:** `gpio-keys` `event1` opened by factory app; no separate pad evdev observed | Determine GPIO keys and internal MIDI controller mapping | Read-only node metadata; later consented event capture | Button/encoder adapter |
| IO-01 | External MIDI/USB | **OBSERVED:** Akai USB `09e8:0046` internal MIDI; PNY `154b:1006` removable research USB; DWC/EHCI/OHCI controllers | Inspect interface subclass/protocol; distinguish control and external data ports | `/sys/bus/usb/devices/*`, `/proc/asound` | I/O backend |
| NET-01 | Ethernet | **OBSERVED:** `end0`, direct IPv4; stmmac-like PHY IRQ label | Driver details, link config, factory default discovery | `/sys/class/net/end0`, `/proc/interrupts` | Discovery/network API |
| CONTENT-01 | Factory content | p7 content mounted; formats UNKNOWN | User content paths/formats and provenance? | filenames/metadata only, local user-authorized inspection | Local-only importer |

## Research order

**Gate A — read-only identity and inventory (now):** all probes above are enumeration-only; collect report and redact locally. No `evtest`, `amidi -d` sending, ALSA playback/open, process stopping, filesystem write or kernel tracing that modifies runtime behavior.

**Gate B — safe backups:** investigate complete eMMC layout, collect carefully bounded read-only backup images to the workstation if technically appropriate, verify hashes, and establish a real **restoration path** before touching persistent state. A checksum of a backup alone does not prove recoverability.

**Gate C — passive hardware discovery:** inspect audio/MIDI and input owner processes; use demonstrably nonexclusive interfaces when available. Limit diagnostic windows and avoid degradation of live playback.

**Gate D — reversible hardware access:** explicitly opt in to session-only changes, stop factory services *gracefully* after identifying actual units, run a minimal test, and restore services on crash, timeout or cancellation. Confirm resource release and normal stock operation.

**Gate E — independent MPFree userspace:** split device-specific adapters from engine/core; test C++ or Rust code on workstation simulation and cross-compile for the observed device ABI; no bootloader changes.

**Gate F — one-USB installer:** implement only after A–E and proven rollback. Compatibility allowlist and cryptographic release validation required. See [installer architecture](../USB_INSTALLER_ARCHITECTURE.md).

## Recommended public artifact layout

~~~text
docs/hardware/
  MPC_ONE_3_6_0_BASELINE.md   observed reference
  DEVELOPER_ACCESS.md         verified onboarding/reconnection
  RESEARCH_MATRIX.md          UNKNOWN -> OBSERVED plan
  firmware/
    <future per-version redacted field reports>
tools/mpc-one/
  collect-over-ssh.sh         host-only, guarded SSH orchestration
  read-only-inventory.sh      no-write POSIX remote enumeration
research/                     local **untracked** raw reports, never publish
~~~

### Record management

Keep a machine-readable mapping alongside future human-readable docs once we have actual observations (proposed keys: firmware version, kernel, SoC compatible ID, ALSA card IDs, MIDI client ports, device-tree node, driver module, verification command, evidence hash and confidence). Do not infer missing values or merge measurements from distinct firmware builds.

Independent references such as [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore) and [MPCLiveXplore-libs](https://github.com/TheKikGen/MPCLiveXplore-libs) can guide experiments, but must not be re-labelled as measurements on this MPC One without reproducing them.
