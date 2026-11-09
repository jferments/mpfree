# MPC One external compute: Ethernet/USB research and network-audio architecture

**Status: research + instrumented diagnostics, not a working external-audio bridge.**
Reference device: owner's Akai MPC One, firmware UI-reported **3.6.0**, Rockchip RK3288, inMusic az0x 5.0.8, Linux 6.6.95 PREEMPT_RT. Known working direct SSH link **ML-tower `eno1np0` (10.42.0.1/24) ↔ MPC `end0` (10.42.0.141)**. These laboratory addresses are **not universal MPC defaults**.

## Executive decision

**Explore Ethernet first.** We already have working point-to-point Ethernet, root SSH for diagnostics and a known 44.1-kHz ALSA PCM profile. An auxiliary Linux workstation can compute synth voices, DSP effects, or sample streaming and return timestamped audio frames to the MPC. The hard problems will be clock discipline, deadlines, latency variation, flow control and recovery after missed packets—not the raw bandwidth of stereo PCM.

USB is also worth investigating, but **USB-A is a host port and a laptop USB-A/USB-C port normally acts as a host too**: never connect USB-A to USB-A using an ordinary cable. The MPC One's USB-B computer connection is device-side in Akai's controller-mode workflow, but whether the existing kernel exposes a usable UDC and can safely implement USB Audio or USB Ethernet *while the stock app is running* is **unverified**.

## 1. Source evidence and confidence labels

- **OBSERVED on actual MPC:** direct 10.42.0.x Ethernet link works and responds with low ICMP latency (previous short runs); USB MIDI device 09e8:0046 runs at **12 Mbit/s**; a separate removable research USB appears on another USB bus; active ACVA PCM runs **44,100 Hz, stereo, interleaved S32_LE, 128-frame period**, playback ring 384 frames and capture ring 256 frames. These data come from the first three surveys, **not** from the new link-speed tests.
- **UNKNOWN UNTIL NEXT SURVEY:** MPC `end0` negotiated line speed (10/100/1000), full/half duplex, PHY model, max advertised modes, error counters, actual USB removable-device speed, exposed USB host-controller max speed, USB-B UDC/gadget availability, TCP/UDP sustained goodput and latency/jitter under load.
- **EXTERNAL PRODUCT DOCUMENTATION:** Akai's original MPC One FAQ states **USB 2.0**, one USB Type-A and one Type-B port; see https://support.akaipro.com/en/support/solutions/articles/69000816149-akai-pro-mpc-one-frequently-asked-questions . Rockchip RK3288 **SoC** specifications support up to gigabit GMAC using RGMII and 10/100 using RMII, but that **does not prove the MPC One PCB's PHY supports 1 Gbit/s**; see https://rockchip.fr/RK3288%20TRM/rk3288-chapter-41-gmac-ethernet-interface.pdf .
- **KERNEL REFERENCE:** `/sys/bus/usb/devices/<device>/speed` is the *negotiated USB device speed in Mbit/s*, NOT guaranteed payload throughput. See https://kernel.org/doc/html/latest/admin-guide/abi-stable.html .

## 2. Safe diagnostics added to MPFree

### A. Passive/low-impact Ethernet and USB survey (first)

From **ML-tower**, while the existing network/USB SSH bootstrap is active:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash -n tools/mpc-one/collect-over-ssh.sh
sh -n tools/mpc-one/read-only-io.sh
bash tools/mpc-one/collect-over-ssh.sh --io
~~~

- No files/scripts are installed on MPC: the POSIX shell probe is sent by SSH stdin.
- The **existing wrapper** verifies host Ethernet adapter `eno1np0`, assigned address `10.42.0.1/24`, route, expected MPC MAC and SSH host key before running any probe.
- Probe reads `/sys/class/net/end0/{speed,duplex,carrier,mtu,statistics/*}`, MDIO PHY, interface driver, read-only `ethtool end0` and `ethtool -i end0` **only if available**, USB device `speed` / USB descriptors excluding serials, per-USB-interface drivers, platform host controllers and `/sys/class/udc` gadget capabilities.
- The command creates only workstation-local `research/mpc-one-io-*.txt` and `.sha256` files. Reports can include IP addresses and MACs; review before posting or making public. The source code does not read USB device serial fields.
- **Distinguish current USB device speed from port capability:** a flash drive enumerating at 480 Mbit/s does not prove the USB-B device-mode path can run at 480 Mbit/s, and an internal 12-Mbit/s MIDI device does not limit unrelated ports to 12 Mbit/s.
- **Absence of an installed command or a registered UDC is inconclusive** about electrically available interfaces; do not try to load drivers or change USB gadget functions during survey.

### B. OPTIONAL active Ethernet benchmark (run only when idle)

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
bash tools/mpc-one/collect-over-ssh.sh --netbench
~~~

This **is not read-only traffic**: it deliberately sends 20 small ICMP echo requests and **4 MiB in each direction** via key-authenticated SSH. It makes **no persistent changes** on the MPC: downlink reads `/dev/zero`, uplink discards bytes into `/dev/null`; no iperf, netcat daemon or package installation. It sends **8 MiB total** plus SSH/ICMP overhead. It uses a hard 30-second SSH timeout per transfer, no compression, with the same MAC + SSH host-key checks. Do not benchmark during live playing, recording, saving or critical operations, as cipher CPU/network load may cause xrun/glitches.

Important measurement distinctions:
- `ethtool Speed` or sysfs `end0/speed` measures **negotiated physical line rate**; 100 Mbit/s is not 100 Mbyte/s.
- The measured transferred bytes divided by transfer duration is **end-to-end encrypted SSH goodput**, including handshake and encryption CPU; its value is generally *lower* than native TCP/UDP performance. Do not infer a PHY ceiling from it.
- ICMP ping RTT describes round-trip timing of small packets with no DSP; it does not measure one-way latency, packet delivery distribution or sample playout deadlines. ICMP may be deprioritized.
- A future **native UDP loopback/jitter probe** may better characterize clock delivery and scheduling, but must be a separately approved, small reversible userspace process with explicit rate limits and no permanent listeners.
- USB transfer performance cannot be measured accurately using sysfs alone. Actual throughput needs a **specific external device attached to the specified physical port** and an explicit opt-in test, never benchmark by writing data onto the owner's mounted USB stick or internal eMMC.

### Why not start with iperf?

We have not verified `iperf3` on the MPC, and shouldn't install a daemon or change the stock rootfs just to benchmark. Using the already working SSH channel lets us establish a **conservative end-to-end baseline** without new services, although its cipher cost may substantially limit ARMv7 throughput. If needed, implement a purpose-built, ephemeral rate-limited network diagnostic later and validate it on desktop first.

## 3. The bandwidth calculation

For uncompressed PCM, define:

- `f_s`: audio frames per second (e.g. 44,100);
- `C`: channels per frame;
- `B`: bytes per channel sample (e.g. 4 for S32_LE);
- `R`: required **payload** bitrate in bits per second.

Then:

`R = f_s × C × B × 8`.

| Stream format | Audio payload | Bytes per 128-frame block |
| --- | ---: | ---: |
| 44.1 kHz, 2 channels, S32_LE | 2.8224 Mbit/s | 1,024 B |
| 48 kHz, 2 channels, S32_LE | 3.072 Mbit/s | 1,024 B |
| 44.1 kHz, 8 channels, S32_LE | 11.2896 Mbit/s | 4,096 B |
| 44.1 kHz, 16 channels, S32_LE | 22.5792 Mbit/s | 8,192 B |
| 44.1 kHz, 32 channels, S32_LE | 45.1584 Mbit/s | 16,384 B |

For reference a 100-Mbit/s link's **nominal physical serialization** for a 1,024-byte payload, ignoring all headers and inter-frame gaps, is `1024×8/100e6 = 81.92 μs`. **This is not end-to-end latency**. At 1 Gbit/s that bare serialization drops to 8.192 μs. If the link negotiates 100 Mbps, even stereo bidirectional raw PCM (~5.64 Mbit/s application payload) fits comfortably in nominal bandwidth. Actual usable bandwidth will be lower due to Ethernet/IP/UDP headers, pacing, competing traffic, hardware interrupts and scheduling.

The stock MPC's **128-frame audio period = 128/44100 ≈ 2.9025 ms**, while its **384-frame playback ring ≈ 8.7075 ms**. These numbers are configuration observations, not a guaranteed 2.9-ms processing callback latency or 8.7-ms round-trip audio latency.

## 4. Proposed MPFree Auxiliary Compute architecture

~~~text
HOST / LAPTOP                                   MPC One
┌──────────────────────────────────┐            ┌────────────────────────────┐
│ MPFree Compute Server            │ Ethernet   │ MPFree userspace          │
│ - sample library & virtual synth │◄──────────►│ - controls/sequencer      │
│ - expensive reverb/FX            │  framed    │ - local sound + mixer     │
│ - renders timestamped PCM blocks │   data     │ - network receive worker │
│ - rate/tempo/sample clock aware  │            │ - jitter buffer          │
└──────────────────────────────────┘            │ - ALSA ACVA (later)       │
                                                └─────────────┬──────────────┘
                                                              │
                                                          analog out
~~~

### Recommended workload split

**Best early applications:** trigger MIDI notes remotely and receive synthesized stereo audio; offload sample-heavy instruments, long impulse-response reverbs, spectral analysis, convolution or pre-rendered patterns. This keeps the MPC's limited RAM free for UI, sequencing, sample cache and low-latency local mixing.

**Higher-risk applications:** use laptop as an insert effect with audio captured on the MPC, processed remotely, then sent back. This adds capture buffering, two network legs, DSP latency, jitter buffering and playback. Tight live performance monitoring will be substantially harder than playing a synth from timestamped notes.

**Timing and sound quality:** data must be frame-indexed, not reliant on raw packet arrival. Remote host and MPC use different oscillator clocks; a drift-control loop or asynchronous sample-rate conversion (ASRC) is required for sustained gapless audio even if both negotiate 44.1 kHz.

### Initial protocol sketch (not implemented)

Control plane on authenticated TCP/TLS or a mutually authenticated local control channel; data plane uses **sequence-numbered, timestamped UDP** when appropriate, not unbounded blocking reads from TCP on the audio thread.

Conceptual audio message:

~~~text
magic/version | stream_id | sequence | starting_sample_frame |
frame_count | format_id | channel_count | flags | audio_samples
~~~

The receiver should:
1. Check packet length, version, bounded frame count, stream authorization and sequence number before touching the ring buffer.
2. Map packet `starting_sample_frame` onto the MPC's local playout timeline, track clock offset/drift and reorder within a bounded window.
3. Copy samples using a preallocated SPSC ring and **never call recv(), malloc, mutex or logger from the real-time ALSA callback**.
4. Produce bounded silence/ramp or hold-last-safe audio on underrun; report missed deadlines outside the audio thread.
5. Separate network ingress/egress task priorities from real-time output callback. Inbound buffers should have finite memory limits and cancellation paths.
6. Negotiate format and endianness explicitly; for the current MPC reference profile use 44,100 Hz stereo S32_LE or an internal float transport with deterministic conversion.

### Latency budget model

For *laptop-generated instrument audio* delivered to MPC, define:
- `L_ctrl`: scheduling/control delivery time (can be hidden by lookahead for timestamped notes);
- `L_render`: laptop audio generation time for scheduled frames;
- `L_net`: laptop→MPC network transport + scheduling latency;
- `L_jitter`: intentional receive buffering needed to tolerate late packets;
- `L_play`: MPC output queue/driver/DAC/analog path delay.

`L_total ≈ L_ctrl + L_render + L_net + L_jitter + L_play`, with overlap/advance scheduling reducing the effective serial sum. For MPC-captured audio processed remotely, add capture-side buffering, MPC→laptop transport and capture-to-render delay. **No end-to-end latency has been measured on this device.** A direct link with a good ICMP RTT is not enough to promise playable live roundtrip monitoring.

### Resource and safety rules

- MPFree software should remain usable without an auxiliary computer: when disconnected, local-only processing and projects still work.
- External devices may be Linux laptop/desktop, SBC, or a dedicated USB-network gadget; isolate by device-aware drivers and signed/authenticated protocol.
- No arbitrary network message can trigger shell execution, system updates, unsigned plugins or untrusted file writes.
- Treat the dedicated direct Ethernet link as an untrusted network endpoint; use explicit pairing, per-device credentials and narrow firewall/service scope.
- Set finite connection, jitter-buffer, stream-count, packet-size and CPU quotas; packet loss must degrade audio, **not** destabilize the MPC or corrupt projects.
- All timing and routing experiments stay on host simulation until a **reversible, consented** hardware takeover and tested recovery have been established.
- Documentation distinguishes kernel/SoC *capabilities* from the exact MPC One **physical ports and negotiated connections**.

## 5. Integration roadmap

| Gate | Result | Current status |
| --- | --- | --- |
| IO-1 | Read-only Ethernet PHY/link rate, duplex, MTU, error stats, USB port speed/gadget map | **Tool committed, measurements pending** |
| IO-2 | Optional 4+4 MiB SSH encrypted goodput and 20-ping RTT baseline | **Tool committed, opt-in only** |
| IO-3 | Native short UDP timestamp/pacing and jitter diagnostics (no audio devices) | Planned, requires independent process-safety review |
| IO-4 | Desktop-to-desktop framed PCM relay and ring-buffer tests | Planned; no MPC modifications |
| IO-5 | Audio stream receiver on MPC **without** opening ACVA, validating parsing/clock drift only | Blocked on compatible loader and defined noninterference |
| IO-6 | Reversible, explicitly approved ACVA playback through MPFree | Blocked on genuine recovery and service-handoff proof |
| IO-7 | Plug-and-play discovery, pairing, per-user secure auxiliary DSP registry | Planned |
| IO-8 | Optional USB gadget mode with USB Audio Class or ECM/NCM | Unverified kernel UDC/role support; cannot assume |

**One-USB installer implication:** nothing in this plan requires an initial MPC bootloader change. The end-user installer can eventually ship a signed MPFree userspace and a separate optional companion app to other computers, after safety and compatibility gates.
