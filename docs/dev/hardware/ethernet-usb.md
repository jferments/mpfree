# MPC One 3.6.0 — Gigabit Ethernet, USB and network benchmark (2026-11-09)

> **Evidence source:** owner-run passive `--io` survey and explicitly opted-in `--netbench` over the established, identity-checked direct Ethernet link. This is a measurement of **one device, firmware UI-reported 3.6.0, in one wired setup**, not a cross-model Akai specification.
>
> **Passive report:** `mpc-one-io-20261109T060106Z-9slW9o.txt` (owner-local only), SHA-256 **`92687a46ab7d04d81ba6bf05486e29e6f67f73e66c0c9ea0ad7e136e20eee45d`**.
>
> **Active report:** `mpc-one-netbench-20261109T060116Z-uiNq7w.txt` (owner-local only), SHA-256 **`6d206eba95640a385b58ea76f36ea91723591b44fada7155a5fff9de14679d01`**.
>
> Both SHA-256 hashes were computed from the original user-uploaded files on 2026-11-09 UTC. Raw reports include the reference network IP/MAC and remain uncommitted. This document intentionally omits the device-specific MAC.

## Findings at a glance

| Item | Evidence status | Result |
| --- | --- | --- |
| MPC Ethernet `end0` negotiated link | **OBSERVED** | **1000 Mbit/s, full duplex**, carrier=1, MTU 1500 |
| Board Ethernet driver | **OBSERVED** | `rk_gmac-dwmac`, controller `ff290000.ethernet` |
| Ethernet PHY | **OBSERVED** | **Realtek RTL8211E Gigabit Ethernet**, PHY ID `0x001cc915`, RGMII, node `stmmac-0:01` |
| Ethernet error counters at passive survey | **OBSERVED snapshot** | 0 RX/TX errors, 0 RX/TX drops |
| 20 ICMP echoes, desktop ↔ MPC | **OBSERVED** | 20/20 received, min/avg/max/mdev 0.251/0.272/0.315/0.015 ms |
| SSH MPC→workstation | **OBSERVED short test** | 4 MiB in 0.265 s, **126.66 Mbit/s** |
| SSH workstation→MPC | **OBSERVED short test** | 4 MiB in 0.713 s, **47.05 Mbit/s** |
| External USB-A storage speed | **OBSERVED** | PNY `154b:1006` enumerated at USB **480 Mbit/s**, USB 2.0 high-speed transport |
| Internal MIDI link speed | **OBSERVED** | Akai `09e8:0046` USB-MIDI enumerated at **12 Mbit/s** on a different bus |
| USB root hubs | **OBSERVED** | DWC2 high-speed OTG host bus, EHCI high-speed host bus, OHCI full-speed host bus |
| USB device controller (UDC) | **OBSERVED** | `ff580000.usb`, driver `dwc2`, maximum `high-speed`, state `not attached`, current speed `UNKNOWN` |
| USB gadget ConfigFS path | **OBSERVED** | `/sys/kernel/config/usb_gadget` exists |
| USB Ethernet (ECM/NCM) | **UNVERIFIED** | Gadget function support, binding, host behavior and contention not tested |
| USB Audio gadget | **UNVERIFIED** | Kernel support, routes and audio timing not tested |
| Real-time UDP/PCM jitter or end-to-end latency | **UNVERIFIED** | Neither benchmark measured scheduled audio delivery or analog output |

## 1. Ethernet — verified gigabit, not merely a capable SoC

Selected kernel/sysfs evidence, identifying MAC/IP omitted:

~~~text
/sys/class/net/end0/operstate: up
/sys/class/net/end0/carrier: 1
/sys/class/net/end0/speed: 1000
/sys/class/net/end0/duplex: full
/sys/class/net/end0/mtu: 1500
/sys/class/net/end0/device -> ../../../ff290000.ethernet
/sys/class/net/end0/device/driver -> .../rk_gmac-dwmac
/sys/class/net/end0/phydev -> .../stmmac-0:01
/sys/bus/mdio_bus/devices/stmmac-0:01/phy_id: 0x001cc915
/sys/bus/mdio_bus/devices/stmmac-0:01/phy_interface: rgmii
/sys/bus/mdio_bus/devices/stmmac-0:01/driver -> .../RTL8211E Gigabit Ethernet
rx_errors: 0
tx_errors: 0
rx_dropped: 0
tx_dropped: 0
~~~

**OBSERVED:** this particular MPC One's integrated Gigabit MAC+PHY and wired link negotiated 1000BASE-T full duplex with ML-tower. It is NOT a 100BASE-T-only interface as might be inferred from older standalone product assumptions.

The read-only probe found no `ethtool` program installed on the MPC. `/sys/class/net/end0/speed` is a current negotiated speed; **supported/advertised speed capabilities** and EEE/PTP/offload features are NOT completely established without additional read-only driver and PHY metadata.

Zero network errors/drops is a useful instantaneous baseline, not a lifetime reliability guarantee or a demonstration of zero errors under sustained traffic.

### Real benchmark: 20 ping requests

~~~text
20 packets transmitted, 20 received, 0% packet loss
rtt min/avg/max/mdev = 0.251/0.272/0.315/0.015 ms
~~~

ICMP was 56-byte payload, about 0.2 s spacing. The recorded **mdev is the ping utility's statistic**, not a percentile latency SLA or jitter upper bound. Peak seen RTT in this **20-sample test** was 0.315 ms; under high CPU/DSP/USB load, longer tail delays are unknown. One-way latency, interrupt scheduling behavior and end-to-end audio latency were **not measured**.

### Real benchmark: 4 MiB per direction over SSH

| Direction | Payload | Elapsed | Reported goodput |
| --- | ---: | ---: | ---: |
| MPC → ML-tower | 4,194,304 B | 0.265 s | 126.66 Mbit/s |
| ML-tower → MPC | 4,194,304 B | 0.713 s | 47.05 Mbit/s |

**These are small-sample, application-level, encrypted, no-compression SSH measurements**, including startup and CPU cryptography. They are not the negotiated 1-Gbit/s PHY limit, UDP performance, safe simultaneous duplex performance, or expected realtime network-audio throughput. The asymmetry can involve cipher implementation, CPU load, host→MPC receive path, SSH buffering, packet processing and short transfer duration. The test cannot separate these factors.

Future performance experiments should observe CPU consumption and network packet pacing at fixed rates without disturbing the factory app, preferably first on desktop. Do **not** conclude USB is preferable to Ethernet because of the 47-Mbit/s SSH test.

### What the measured link means for audio

The stock ALSA stream is 44,100 frames/s × 2 channels × 4 B/channel-sample = **352,800 B/s** = **2.8224 Mbit/s** per uncompressed S32_LE stereo stream.

A **128-frame stereo** audio block is **1,024 B**, generated every **2.9025 ms**, at an average rate of **344.53 blocks/s**. With a conceptual 32- or 40-byte application header plus 28-byte IPv4/UDP header, a stereo block fits within a 1500-byte standard MTU. This is a design sketch, not a packet capture or confirmed receive-window capacity.

At 8/16/32 channels the 128-frame payload grows to **4096/8192/16384 B**, **above standard MTU**: split into smaller packets or use a different framing layout; never rely on IP fragmentation for real-time multichannel delivery.

**Physical serialization only** for 1024 bytes at the observed 1 Gbit/s link is ~8.192 μs, ignoring Ethernet/IP headers and inter-frame spacing. This does **not** predict kernel scheduling, transfer, CPU load or play-out latency.

The actual MPC already owns ACVA ALSA PCM under `acvs.service` with no available subdevices; no one has routed network audio to its analog output. This milestone is interface confirmation, not networked audio playback.

## 2. USB host-side: confirmed USB 2.0 high-speed operation

Observed USB topology:

~~~text
Bus 1: DWC OTG Controller (dwc2_hsotg), root hub high-speed
  1-1: Akai Pro MPC One MIDI 09e8:0046
       negotiated speed 12 Mbit/s (USB full speed), 2 interfaces
Bus 2: EHCI Host Controller, root hub high-speed
  2-1: PNY USB storage 154b:1006
       negotiated speed 480 Mbit/s (USB 2.0 high speed)
Bus 3: OHCI Host Controller, root hub full-speed (12 Mbit/s)
~~~

The PNY device advertises itself as `USB 3.2.1 FD`, but **its negotiated link is USB 2.0 high speed**, not 5 Gbit/s or above. The device name is not its connection speed.

Device `2-1:1.0` was attached to `usb-storage` with class `08`, subclass `06` and protocol `50` (bulk-only mass storage). No storage reads/writes or filesystem benchmarking were performed, so **effective payload throughput and per-port contention remain unknown**.

The internal controller device's 12-Mbit/s USB MIDI speed does not restrict the external port to 12 Mbit/s. Distinct root hubs have different devices and capabilities.

## 3. USB device-side: registered DWC2 USB gadget controller

Observed:

~~~text
/sys/class/udc/ff580000.usb/state: not attached
/sys/class/udc/ff580000.usb/current_speed: UNKNOWN
/sys/class/udc/ff580000.usb/maximum_speed: high-speed
/sys/class/udc/ff580000.usb/is_otg: 0
/sys/class/udc/ff580000.usb/device/driver -> .../dwc2
/sys/kernel/config/usb_gadget: [directory exists]
~~~

**OBSERVED:** a DWC2 kernel USB device controller is registered, with a stated maximum **high-speed** capability. At survey time, it was not attached to a USB host. The ConfigFS USB gadget directory exists. This is meaningful evidence for a possible USB-B MPC↔laptop device-mode transport.

**NOT PROVEN:** the physical connector wiring for that particular UDC, whether gadget ECM/NCM/RNDIS or UAC1/UAC2 functions are built/enabled, how the stock controller-mode system owns the UDC, endpoint/clock behavior, whether the laptop sees a stable class-compliant device, or whether gadget activation is reversible without interfering with normal MPC operation. **Do not bind/unbind, write a USB gadget configuration, or connect host-to-host USB-A ports during this discovery stage.**

For Ethernet-equivalent over USB, **ECM or NCM gadget networking** could provide the same higher-level packet transport on a USB 2.0 link, but there is no current proof of a working function driver. USB Audio Class gadget mode could also provide direct class-based audio to a laptop, but converting the MPC into a slave audio peripheral is not the same goal as making it a standalone computer with auxiliary DSP offload. Any future USB test needs user consent, physical connector identification and a reversible stock-state procedure.

## 4. Architecture and engineering decisions

### Recommended transport priority

1. **Gigabit Ethernet** first, because it is measured, independent of USB bootstrapping and already has a direct working link. Reuse the stock 1 Gbit/s path for a laptop-assisted DSP prototype, initially entirely off-device.
2. **USB device-mode networking** later, if non-intrusive evidence supports ECM/NCM and a demonstrably safe stock-to-custom configuration transition.
3. **USB-A host attached external devices** for MIDI controllers, audio I/O, storage, and eventually special-purpose plug-in compute accessories acting as USB peripherals, not host-to-host laptop connections.

### Key risks to measure rather than assume away

- **Network jitter and worst-case latency:** real audio requires deadline compliance, not low average ping RTT. Need timestamped UDP packet delivery distributions and CPU contention tests.
- **Clock drift:** nominally identical 44.1-kHz host/MPC clocks can drift by ppm, eventually causing underruns/overruns. Use sample-frame timestamps, bounded adaptive jitter buffers, and ASRC or playout-rate discipline.
- **Time alignment for sequencer events:** send scheduled notes **ahead** of intended render frame, track event ordering and host processing deadline, do not base note triggering on packet arrival time.
- **MPC CPU/RAM:** network receives, decoding, mixing and sample conversion still cost CPU; all audio-thread memory and CPU budgets must be finite.
- **Reliability:** implement local audio fallback and bounded concealment/fade when the auxiliary computer disconnects or packets are late.
- **Security:** no remote shell command execution as part of audio transport; explicit pairing, authorization, packet size/sequence checks and rate limits.
- **Reversibility:** stock `acvs.service` has `Restart=always` and owns audio/graphics/controller devices. No PCM playback on MPC until a verified recovery strategy and consented one-session handoff exists.

## 5. Next safe stages

| Stage | Work | Safety and validation |
| --- | --- | --- |
| IO-1 | Passive Ethernet, PHY, USB speed, UDC inventory | **Done on MPC**. No persistent device writes. |
| IO-2 | 20 ICMP replies and 4+4 MiB SSH goodput | **Done on MPC**, explicitly active. Short benchmark; no driver-level saturation claims. |
| IO-3 | **Host-only** packet transport prototype + loss/jitter/drift simulation | Can proceed now; no MPC access required. |
| IO-4 | Optional, short, rate-limited native UDP transport timing | Only after isolated test program security/realtime review, separate user consent; no active devices. |
| IO-5 | ARM hard-float userspace network receive-only program | Cross-compile and check ABI; test without opening ALSA, no persistent writes. |
| IO-6 | MPC ACVA playback with network streams | **Blocked:** restoration and stock service handoff unproven. |
| IO-7 | Optional USB gadget networking/audio | **Blocked:** gadget driver capabilities and safe role handoff unproven. |
| IO-8 | General installer/plugin pairing for end users | **Blocked:** firmware compatibility matrix, signatures, rollback and recovery. |

## 6. Exact replication commands

Run on the owner's **ML-tower workstation**, never inside MPC shell:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519

# Passive sysfs/PHY and USB survey
bash tools/mpc-one/collect-over-ssh.sh --io

# Optional active SSH throughput/ICMP baseline: NOT during music playback/recording
bash tools/mpc-one/collect-over-ssh.sh --netbench
~~~

Reports go into untracked `research/`; never commit raw logs, device MACs, private keys or proprietary blobs. No bootloader, partitions, service or ALSA/DRM resource changes are required.
