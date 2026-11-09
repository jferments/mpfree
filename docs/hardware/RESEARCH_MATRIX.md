# MPC hardware reverse-engineering research matrix

**Objective:** a reproducible, legally distributable, evidence-based map of MPC One firmware/hardware interfaces that other developers can use without proprietary firmware copies. This is a work queue and evidence schema. Nothing marked UNKNOWN should be interpreted as a published hardware specification.

Reference baseline: [MPC One 3.6.0](MPC_ONE_3_6_0_BASELINE.md). Latest redacted evidence: [initial survey](MPC_ONE_3_6_0_SURVEY_20261109.md), [focused ownership survey](MPC_ONE_3_6_0_RESOURCES_20261109.md), and [44.1-kHz audio/runtime survey](MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md). Read-only collection: [developer access](DEVELOPER_ACCESS.md).

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
| SYS-02 | CPU/RAM | **OBSERVED:** four ARMv7 RK3288 CPUs, NEON/VFPv4, **performance @ 1,608 MHz**; ~2 GiB RAM, ~1 GiB locked largely accounted for by `/usr/bin/MPC` (`VmLck=1040052 kB`); 38 threads | Sample thread priority, runtime CPU affinity, memory lock owner | `/proc/<pid>/task`, `/proc/<pid>/status`, `/sys/devices/system/cpu` | ARMv7 RT/memory performance budget |
| SYS-03 | Kernel/modules | **OBSERVED:** 6.6.95 PREEMPT_RT; I²S/JP07 ASoC modules, Panfrost IRQ; sched RT 950000/1000000 | Audio thread priorities, IRQ affinity, true kernel-config features | `/proc/<pid>/task`, `/proc/interrupts`, sysfs | Real-time tuning evidence |
| BOOT-01 | Internal eMMC | **OBSERVED:** 7.3 GiB eMMC with PARTNAME labels `uboot-spl`, `env`, `uboot`, `splash`, `recoverysplash`, `rootfs`, `content`, `data`; boot0/boot1 also present | Read-only boot-chain metadata and validated restoration study | `/sys/class/block/mmcblk0p*/uevent`, partition metadata | Partition reference map **without** writes |
| BOOT-02 | Boot/update chain | UNKNOWN | Is there A/B switching, U-Boot and recovery without Linux? | Non-destructive vendor documentation, read-only boot metadata | Proven recovery prerequisite |
| FS-01 | Overlayfs | `/etc` and `/var` persistent upperdirs observed | Which paths are mutable and survive reboot? | `mount`, `/proc/mounts`, optional benign isolated tests **only after consent** | Runtime/installer path constraints |
| FS-02 | USB execution | Research USB showed `noexec` mount | Which deployment paths support ARM ELF? | `/proc/mounts`, loader inspection; executable tests only in isolated phase | Portable loader strategy |
| APP-01 | Factory GUI | **OBSERVED:** `acvs.service` (`Restart=always`) runs `/usr/bin/MPC` with 38 threads; it has DRM, ALSA PCM playback/capture, control, seq, triple RawMIDI, touch and gpio fds | Map each audio thread policy/affinity and safely restorable lifecycle | `/proc/<pid>/task`, systemd properties; no service changes | Safe takeover supervisor design |
| APP-02 | Libraries/plugins | UNKNOWN | What libraries load and which formats are public? | dependency and path metadata; separate local legal review | Clean-room API design |
| AUD-01 | Audio devices | **OBSERVED:** ACVA playback & capture actively configured **44.1 kHz, stereo, S32_LE, interleaved, 128-frame periods**, with 384-frame output and 256-frame input buffer | Full driver capability matrix, physical codec, measured xrun/latency | `/proc/asound` for stock state; later consented exclusive tests | S32 PCM HAL profile and offline engine tests |
| AUD-02 | Audio ownership | **OBSERVED:** `acvs.service` MainPID holds `/dev/snd/pcmC1D0p`, `pcmC1D0c`, `controlC1`, `/dev/snd/seq`, three raw-MIDI fds | Is it exclusive? How to restore exact normal ownership after a later test? | Existing `/proc/<pid>/fd`, ALSA subdevice availability; later opt-in trial | Arbitration/ownership policy |
| AUD-03 | Realtime performance | UNKNOWN | Rate, periods, xruns, latency? | Controlled **opt-in** audio test with factory app idle/stopped safely | Audio engine budgets |
| MIDI-01 | Controller protocol | **OBSERVED:** USB `09e8:0046` AudioControl/MIDIStreaming interfaces, three RawMIDI ports; kernel counters show output0 6,036,856B, output1 29,775B, input1 49,837B; JUCE/MPC sequencer graph mapped | Which substreams carry pad/encoder activity? | Short read-only counter deltas during physical user presses (no device opens) | Versioned MIDI mapping table |
| MIDI-02 | LED/output | UNKNOWN | LED/control feedback protocol? | Later active opt-in experiments, no unsolicited writes | Bidirectional controller backend |
| UI-01 | Display | **OBSERVED:** DRM `card0-DSI-1` enabled `800x1280`, framebuffer virtual `800x3840` @ 32 bpp, Mali-T760-compatible Panfrost GPU | Physical panel orientation, transforms, compositor ownership | DRM sysfs, `/proc/<pid>/fd`, later GUI isolation tests | DRM/KMS display backend |
| UI-02 | Touchscreen | **OBSERVED:** ILI2117 `event0` opened by factory app; touch capability bitmasks captured | Absolute ranges, calibration/rotation, multi-touch tracking | sysfs/device tree; later non-interfering opt-in capture | Touch adapter |
| UI-03 | Hardware keys | **OBSERVED:** `gpio-keys` `event1` opened by factory app; no separate pad evdev observed | Determine GPIO keys and internal MIDI controller mapping | Read-only node metadata; later consented event capture | Button/encoder adapter |
| IO-01 | USB host / device mode | **OBSERVED:** internal MIDI 12 Mbit/s on DWC2 host; PNY USB storage 480 Mbit/s on EHCI; OHCI host, and DWC2 UDC `ff580000.usb` registered high-speed, `not attached`; ConfigFS gadget path exists | Which actual gadget functions are compiled and safely assignable? Is UDC wired to USB-B? | Read-only UDC driver/config paths and vendor docs; **no gadget binding** | Optional USB gadget/audio network backend |
| NET-01 | Ethernet + DSP offload | **OBSERVED:** `end0` **1000 Mbit/s full-duplex**, Realtek RTL8211E PHY/RGMII, `rk_gmac-dwmac`, MTU 1500; ICMP 0.272 ms avg (20/20), SSH 126.66 Mbit/s MPC→host / 47.05 Mbit/s host→MPC (short test) | Native network audio deadline jitter, clock drift and sustainable CPU load; **do not equate SSH throughput with PHY rate** | Redacted [Ethernet/USB measured report](MPC_ONE_3_6_0_ETHERNET_USB_20261109.md); future host-only stream tests | Bounded, authenticated auxiliary-compute transport and jitter buffer |
| CONTENT-01 | Factory content | p7 content mounted; formats UNKNOWN | User content paths/formats and provenance? | filenames/metadata only, local user-authorized inspection | Local-only importer |

## Current implemented software milestone

**M0 (host-only) is implemented and owner-validated on ML-tower:** [Deterministic audio core and WAV renderer](../engineering/FIRST_AUDIO_CORE.md). A C++17 sample-frame click engine with a bounded, precomputed wavetable and saturating float-to-S32 converter; offline WAV demo and CI tests. Does not access MPC devices. This is a software foundation, not a proof of audio-driver takeover or ROM recovery.

[Ethernet/USB capability and auxiliary compute design](../engineering/AUXILIARY_COMPUTE_AND_IO.md) covers device-role constraints, link/USB speed probes, optional small benchmark and raw PCM bandwidth/latency model. **Both on-device `end0` gigabit PHY and USB-A 480 Mbit/s **are now confirmed**, as are 20 short ICMP RTT probes and bounded SSH goodput. **Actual native UDP streaming deadlines, audio latency, and USB gadget functionality remain unmeasured**.**

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
