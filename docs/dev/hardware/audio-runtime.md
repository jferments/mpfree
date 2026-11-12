# MPC One / az0x 3.6.0 — active ALSA, runtime and MIDI routing survey

> **Evidence type:** Observed on one owner-authorized Akai MPC One during a read-only session, 2026-11-09 ~05:30:31 UTC. Firmware version 3.6.0 was owner-reported from the UI. The kernel, device metadata and factory application state have been observed on-device.
>
> **Source (owner-local file; not committed):** `mpc-one-runtime-20261109T053031Z-8HGgjg.txt`
>
> **SHA-256:** `1ea9eb27f837e5602cd5548db71622adf2f8c6aef52553f2aafd22e035a34b76` (verified from uploaded bytes).
>
> **Collector:** `tools/mpc-one/read-only-runtime.sh`, invoked with the identity-gated `collect-over-ssh.sh --runtime`. No audio device opened, data sent to MIDI, services stopped, DRM master requested or internal storage written. This document is a **redacted and interpreted observation**, not a factory-specification claim.

Earlier research:
- [First board/eMMC/ALSA/display map](survey-2026-11-09.md)
- [Factory process device-file ownership map](resource-ownership.md)
- [Hardware evidence matrix](research-matrix.md)
- [Machine-readable device profile](data/mpc-one-3.6.0-observed.json)

## 1. Verified active audio stream settings

The original vendor process `/usr/bin/MPC` (systemd unit `acvs.service`, **PID 311 in this snapshot only**) owns the ALSA device **card 1, device 0** (`ACVA`). Both streams were RUNNING at inspection time.

### ALSA PCM negotiated hardware parameters (OBSERVED)

| ALSA procfs key | Playback: `/proc/asound/card1/pcm0p/sub0` | Capture: `/proc/asound/card1/pcm0c/sub0` |
| --- | --- | --- |
| `access` | `RW_INTERLEAVED` | `RW_INTERLEAVED` |
| `format` | `S32_LE` | `S32_LE` |
| `subformat` | `STD` | `STD` |
| `channels` | `2` | `2` |
| `rate` | `44100` | `44100` |
| `period_size` | `128` frames | `128` frames |
| `buffer_size` | `384` frames | `256` frames |
| `avail_min` (software param) | `128` frames | `128` frames |
| `stop_threshold` | `384` frames | `256` frames |
| `start_threshold` | `1610612736` | `1073741824` |
| `tstamp_mode` | `NONE` | `NONE` |
| `state` | `RUNNING` | `RUNNING` |
| `owner_pid` | `311` | `311` |
| `subdevices_count` | `1` | `1` |
| `subdevices_avail` | `0` | `0` |

Values above were read directly from each stream's `hw_params`, `sw_params`, `status` and `info` procfs nodes; no stream acquisition was attempted.

### Derived timing and memory model (DERIVED from OBSERVED parameters)

Define `f_s = 44,100` frames/s, `N = 128` frames/period, `C = 2` channels and `B = 4` bytes/channel-sample (S32_LE):

- Period duration: `T_period = N / f_s = 128 / 44100 s ≈ 2.902494 ms`.
- Playback buffer duration: `T_out = 384 / 44100 s ≈ 8.707483 ms`, equivalent to **3 periods**.
- Capture buffer duration: `T_in = 256 / 44100 s ≈ 5.804989 ms`, equivalent to **2 periods**.
- Bytes per interleaved frame: `C × B = 8 bytes`.
- Bytes per period: `N × C × B = 1024 bytes`.
- Playback ring data capacity: `384 × 8 = 3072 bytes`.
- Capture ring data capacity: `256 × 8 = 2048 bytes`.

These buffer durations are **NOT end-to-end latency measurements**, and are not even guaranteed to be the amount of audio waiting in the PCM at a given time. Additional device, driver, scheduling, DSP, monitoring, playback, capture and application buffering determine observed latency. The snapshot's playback `delay=308` frames (about 6.98 ms) and capture `delay=8` frames describe one instantaneous ALSA status, not a robust latency benchmark or an acoustic loopback measurement. Future performance tests must measure xruns and actual loopback latency under representative workload.

**Do not confuse `S32_LE` with "32-bit acoustic precision":** it is the ALSA *container* format. The effective number of valid converter bits, converter noise floor and any software sign/bit alignment have **not** been established.

**Do not confuse the configured 44.1 kHz stream with a driver capability matrix:** the supported rates, 16-bit vs 24-bit container formats, channel arrangements and period limits remain UNVERIFIED. A future controlled opt-in ALSA probe will enumerate capabilities only after the vendor application is safely released.

### Software thresholds need interpretation

- The unusually large `start_threshold` values correspond to ALSA boundary-sized values, consistent with an application that manually controls stream start/trigger rather than relying on the usual automatic start when buffered frames reach a modest threshold. We have **not** traced syscalls to prove how the factory application starts/restarts the PCM.
- Both directions have `avail_min=128`, which is consistent with work in a 128-frame cadence. It does **not** prove that every DSP node is processed in exactly 128-frame chunks or every callback is synchronized one-to-one with the kernel period.
- Both have `tstamp_mode=NONE`. Do not report the `tstamp` field as a reliable nanosecond clock. For our own engine, we should separately assess whether hardware timestamps and suitable monotonic scheduling clocks are available.

### Reproduction (read-only, MPC connected as previously configured)

~~~sh
# Execute on the Linux workstation, NOT the MPC:
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --runtime
~~~

The host wrapper verifies the known Ethernet interface, route, neighbor MAC and SSH host key before sending the read-only script to `sh -s`. The generated report stays only in local, gitignored `research/`.

## 2. Actual factory memory footprint and CPU affinity

`acvs.service` resolves to process 311 in this run, whose `/proc/311/exe` symlink resolves to `/usr/bin/MPC` (rather than merely the systemd launcher's text path).

Selected `/proc/<pid>/status` evidence:

~~~text
Name:               MPC Main Thread
VmSize:             1044072 kB
VmLck:              1040052 kB
VmRSS:              1043768 kB
VmHWM:              1047696 kB
Threads:            38
Cpus_allowed_list:  0
~~~

**OBSERVED:** approximately 1,015.7 MiB of virtual memory locked by the main MPC process and approximately 1,019.3 MiB resident; 38 threads. This directly accounts for much of the ~1 GiB system-wide locked memory previously observed. Note that `VmLck` counts locked *virtual address space* associated with the process, and must not be presented as a complete explanation for each physical kernel accounting category. The difference between RSS, VmSize and VmLck is small at this moment.

**OBSERVED:** the **main thread** was allowed to execute on CPU 0. **UNVERIFIED:** affinity of other threads, whether the audio thread is pinned to specific cores, whether the application uses sched FIFO/RR, and how memory is pre-faulted/locked. The third survey printed 38 thread names but failed to extract the requested `policy`/`prio` fields from `/proc/<tid>/sched`; absence of these fields in this report is a **collector limitation**, not evidence that the policies are SCHED_OTHER.

Selected thread names (kernel `comm` names can be truncated to 15 bytes):

~~~text
MPC Main Thread
Audio Processin
AudioWorker0
AudioWorker1
AudioWorker2
AudioWorker3
AudioWatchdog
JUCE MIDI Input
JUCE Message Th
MIDI Out: MPC O
MIDI Out: JUCE
MIDI
FileIO
File Management
DirectoryWatche
JUCE FileBrowse
~~~

**OBSERVED:** the process has multiple named audio workers and JUCE-named threads. **DERIVED:** portions of the application use JUCE-style MIDI/application infrastructure; the names alone do not prove that all DSP rendering uses JUCE, that every worker is realtime, or that workers are always simultaneously active.

**Future RT probe:** read `/proc/<pid>/task/<tid>/{stat,sched,status}` with robust kernel-format parsing, and read per-task `Cpus_allowed_list` / `voluntary_ctxt_switches` / `nonvoluntary_ctxt_switches`. A later repeated sample with low frequency could identify CPU placement; any sampling tool must have a fixed maximum run time and no intrusive tracing. Test the parser on desktop fixtures before publishing. Do not change task affinity or scheduling.

## 3. ALSA RawMIDI activity and actual sequencer graph

Observed kernel RawMIDI statistics in `/proc/asound/card0/midi0`:

| Direction/substream | Bytes since counter origin | Owner | Interpretation |
| --- | ---: | --- | --- |
| Output 0 | **6,036,856 Tx bytes** | PID 311 | Significant ongoing or cumulative output traffic |
| Output 1 | **29,775 Tx bytes** | PID 311 | Less output traffic |
| Output 2 | **0 Tx bytes** | PID 311 | No bytes observed in this counter snapshot |
| Input 0 | **0 Rx bytes** | no owner shown | No counted input |
| Input 1 | **49,837 Rx bytes** | PID 311 | Input traffic, zero overruns reported |
| Input 2 | **0 Rx bytes** | PID 311 | Opened input; no counted bytes |

**Important: these are cumulative counters at one point**, not bytes/sec, messages/sec, or packets correlated with key presses. Do not conclude "Output 0 is LEDs" or "Input 1 is pads" without targeted read-only counter deltas during deliberate physical actions. The earlier `amidi -l` enumerated subdevices 0=Public, 1=Private, 2=MIDI Port, but **the message meanings remain unknown**.

USB MIDI device identity (OBSERVED):
- On-board Akai device `09e8:0046`, reported `MPC One MIDI`, full-speed 12 Mbit/s.
- Interface `1-1:1.0`: audio interface class `01`, audio-control subclass `01`, no endpoints (control interface).
- Interface `1-1:1.1`: audio class `01`, MIDI streaming subclass `03`, two endpoints; kernel bound to `snd-usb-audio`.

The ALSA sequencer `/proc/asound/seq/clients` shows four clients:
- `0` — System (kernel)
- `16` — MPC One MIDI (kernel), ports 0 Public, 1 Private, 2 MIDI Port
- `128` — JUCE (userspace), virtual MIDI ports 0–3
- `129` — MPC (userspace), ports 0–5

### Observed sequencer connections (use current dynamic client numbers, never hardcode)

~~~text
16:2 -> 129:0     (onboard MPC One MIDI Port -> MPC userspace)
129:2 -> 16:2    (MPC userspace -> onboard MIDI Port)
128:3 -> 129:1   (JUCE Virtual MIDI Input 2 -> MPC)
128:2 -> 129:4   (JUCE Virtual MIDI Input 1 -> MPC)
129:3 -> 128:1   (MPC -> JUCE Virtual MIDI Output 2)
129:5 -> 128:0   (MPC -> JUCE Virtual MIDI Output 1)
~~~

Directions are taken from `Connecting To` / `Connected From`, not guessed from confusing strings such as "virtual input" in the port name.

**DERIVED:** The factory application exposes an ALSA sequencer graph bridging onboard MIDI Port 2 with its own user-level MIDI ports and a JUCE component. This does not show all paths used for physical pad/encoder or private control-surface events, because those may use opened RawMIDI devices directly and bypass the sequencer graph.

## 4. Factory ELF ABI (64-byte header, independently decoded)

The first 64 bytes of `/usr/bin/MPC` were read with BusyBox-compatible `dd if=/usr/bin/MPC bs=64 count=1 | od -b`. This reads the original file without copying the binary to the research repository.

Decoding the observed octal bytes as an ELF header:

| Field | Actual value | Meaning |
| --- | --- | --- |
| `e_ident[0:4]` | `7f 45 4c 46` | ELF |
| `EI_CLASS` | `1` | **ELF32** |
| `EI_DATA` | `1` | Little-endian |
| `e_type` | `3` | ET_DYN (shared-object/PIE-compatible executable form) |
| `e_machine` | `40` / `EM_ARM` | 32-bit ARM |
| `e_version` | `1` | Current ELF version |
| `e_flags` | `0x05000400` | ARM EABI version 5 plus ABI hard-float flag |

`EF_ARM_EABI_VER5 = 0x05000000` and `EF_ARM_ABI_FLOAT_HARD = 0x00000400` (see e.g. standard Linux `elf.h` constants). This is stronger evidence than finding a hard-float loader pathname. **OBSERVED:** the *factory executable's header itself* declares the ARM EABI5 hard-float ABI. The installed loader name `/lib/ld-linux-armhf.so.3` is consistent with that.

**Still unknown:** actual executable `PT_INTERP` contents, glibc minimum symbol versions, linked JUCE/Qt/plugin dependencies, target CPU tuning, and dependencies necessary for independently compiled MPFree. Because `e_type=ET_DYN` alone is compatible with both shared objects and PIE, avoid asserting PIE solely from this field; the actual process runs as an executable.

### Correct build target hypothesis

- ISA **ARMv7-A / 32-bit, little-endian**, with NEON/VFPv4 support present.
- Linux **gnueabihf** / hard-float calling convention, ARM EABI5.
- Reuse stock kernel and drivers at first, but build an independent open-source userspace.
- Start with independently compiled portable engine code on the workstation; select the deployment sysroot/toolchain only after confirming `PT_INTERP` and minimum library symbol dependencies.
- No binary builds or factory libraries should be committed from the device to the public repository without clear redistribution rights.

## 5. What MPFree should implement first

### Baseline audio configuration profile (OBSERVED)

~~~text
device family:      akai-mpc-one / inMusic az0x
platform:           rockchip,rk3288 / armv7l / gnueabihf
ALSA device:        ACVA, card 1 device 0
playback:           stereo S32_LE, RW_INTERLEAVED
capture:            stereo S32_LE, RW_INTERLEAVED
sample rate:        44100 frames/s
period:             128 frames
playback ring:      384 frames
capture ring:       256 frames
stock ownership:    acvs.service
runtime state:      read-only survey (no device takeover permitted)
~~~

The profile should store **observed working values**, not a hardcoded list of every supported hardware capability. A future adapter should be able to negotiate and validate device parameters and fail safely if a supported profile does not match. It must never silently change the stock app's PCM settings.

### First independent software milestone: deterministic offline metronome engine

Implement the **engine core and a host-only test harness** before deploying to the MPC. Good starter scope:
1. Sample-frame counter as the sole source of audio scheduling truth.
2. Musical timing (`tempo`, beat boundary/phase, bar position) converted to frame positions deterministically.
3. No locks, filesystem I/O, heap allocation, or logging in the render method.
4. Workstation dummy/null backend and short WAV export for verification. It must not touch MPC hardware.
5. Verified conversion from internal float audio to stereo S32_LE with explicit clipping, no overflow.
6. Tests at 44100 Hz/128 frames plus non-divisor tempos and long-running drift/phase boundaries.
7. A later **opt-in** ALSA backend should use the measured device profile *after* proven reversible stock handoff.

### Realtime design caveats

- ARMv7 + 2 GiB + ~1 GiB locked by stock MPC: do **not** promise concurrent playback from MPFree. The factory app holds both PCM subdevices and must remain untouched during survey.
- A 128-frame ALSA period is an **observed configuration**, not a mandate that every core DSP graph node use 128-frame buffers in perpetuity. Design block-length as a runtime parameter with deterministic tests.
- Use explicit bounded maximum tracks/voices/samples according to actual memory measurements rather than hardcoding a 1 GiB memory lock like the stock app.
- Keep UI/graphics, disk streaming, plugin management, and non-realtime event handling off audio processing threads.

## 6. Remaining safety gates and research

**Not yet established:**
- Mechanism to recover from a failed eMMC boot. A ROM/USB recovery procedure is not proven.
- Exactly how `acvs.service` startup/stop affects hardware dependencies and whether autorestart must be managed; current unit reports `Restart=always`.
- Which raw MIDI bytes correspond to pads, Q-Links, buttons, or LEDs. Do not send undocumented SysEx.
- Playback/capture hardware rate/format support beyond current observed settings.
- Actual end-to-end monitor/output latency and worst-case CPU audio callback time.
- Scheduling policies/affinities of DSP worker threads (collector did not display requested values).
- Correct touchscreen rotation and coordinate scaling.
- Vendor plugin binary compatibility (no proprietary libraries in repository).

The most useful **next read-only MPC experiment** is a bounded, counter-only comparison of the `/proc/asound/card0/midi0` statistics while the owner presses **one identified pad** (or turns a Q-Link once) and the factory software continues normal operation. This can identify a likely substream *without opening the raw MIDI device*. It cannot decode the event bytes: later passive capture would require an explicit nonexclusive observation mechanism and consent. Also useful: a small thread-affinity/scheduler summary from `/proc/<pid>/task/<tid>/status` and `stat`, with strict output caps.

**Do not conflate research with installation.** The eventual owner-friendly USB installer can package a separate MPFree userspace and authorized user-content import tools, but the proven recovery + rollback gates remain blocking for persistent internal changes.
