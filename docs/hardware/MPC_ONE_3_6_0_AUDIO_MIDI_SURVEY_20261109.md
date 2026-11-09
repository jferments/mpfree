# MPC One / az0x 3.6.0: audio and MIDI implementation survey

> **Evidence type:** owner-run `collect-over-ssh.sh --audio-midi` (read-only), 2026-11-09 22:31 UTC, factory app running (PID 317 in this boot). Report SHA-256 `84ffd15043b5c8c73708a35b2224adeb6e15f408e9f0178d0f8a56376a26fdc8` (owner-local, not committed). Kernel `6.6.95-az01-2025-06-27-rt55`.
> **Collector bug in this run:** the device-tree section printed nothing because `/proc/device-tree` is a symlink and BusyBox `find` did not descend into it. Fixed in the same PR (trailing slash); re-run to get codec/I2S device-tree properties.

## Audio

| Item | Observed | What it means for the MPFree engine |
| --- | --- | --- |
| Card / driver | Card 1 `ACVA`, platform driver `jp07-audio`; ASoC components `sound-codec`, `ff890000.i2s`; DAIs `inmusic,acva-audio-codec`, `ff890000.i2s` | Rockchip I2S (`ff890000`) with a vendor codec DAI. Plain ALSA PCM; no vendor ioctls needed to stream |
| Factory stream | S32_LE, 2 ch, 44,100 Hz, period 128, buffer 384 (play) / 256 (capture), RW_INTERLEAVED; both triggered within 11 µs of each other at 14.15 s after boot | Same profile as earlier survey; capture and playback start together (linked or back-to-back) |
| ALSA timers | PCM period timer 2902.494 µs; HR timer available | Confirms 128-frame periods at 44.1 kHz |
| PCM buffers | `prealloc` 512 KiB, `prealloc_max` 4 MiB; `snd_soc_core.prealloc_buffer_size_kbytes=512` | Room for larger buffers if we ever need them |
| Mixer | No codec controls listed (`controlC1` exists; no `amixer` on device) | Gain/routing is probably fixed or handled by hardware. Verify later with our own read-only control enumeration |
| Audio DMA IRQ | IRQ 45 `ffb20000.dma-controller`, **pinned to CPU 3**, thread `irq/45` SCHED_FIFO 50 on CPU 3; ~6.6 M interrupts | The factory design runs audio on core 3. Our audio thread should also live on CPU 3, and its priority must be chosen relative to `irq/45` (FIFO 50) |
| Controller USB IRQ | IRQ 56 `ff540000.usb` (DWC2, internal MIDI), **pinned to CPU 2**, FIFO 50 | Core 2 handles the control surface; core 3 audio |
| Other IRQ threads | Display, GPU, Ethernet, eMMC, I2C on CPU 0 (FIFO 50, eMMC FIFO 5) | Housekeeping stays on CPU 0 |
| Isolation | `isolated: 2-3`; `NO_HZ_FULL` not set; `HZ=1000`; `HIGH_RES_TIMERS=y`; `PREEMPT_RT=y` | Cores 2 and 3 are free of general scheduling but still take the 1 kHz tick |
| CPU frequency | Governor `performance`, all cores fixed at 1.608 GHz (ondemand also available); cpuidle driver `none` | No frequency or deep-idle wake-up latency to fight. Do not change the governor |
| RT permissions | `RT_GROUP_SCHED` not set (cgroup v2, `cpu` controller only); `acvs.service` RLIMIT_RTPRIO 0 and MEMLOCK 8 MiB, yet the app holds ~1 GiB locked and runs RT threads | It runs as root (CAP_SYS_NICE / CAP_IPC_LOCK bypass the limits). MPFree as root can use SCHED_FIFO + `mlockall`; as a non-root user it would need limits raised |
| alsa-lib | `libasound.so.2.0.0` dated 2018; no `/etc/asound.conf`; only `amidi` among ALSA tools | We can use the kernel PCM API through alsa-lib or directly; we must bring our own test tools |

## MIDI / control surface

| Item | Observed | Meaning |
| --- | --- | --- |
| USB device | `09e8:0046` "Akai Pro / MPC One MIDI", USB 2.0 full speed (12 Mbit/s), bus 1 device 2, bcdDevice 0x0010 | Internal controller is an ordinary USB device on the DWC2 host |
| Descriptors (decoded from the cached raw bytes) | Audio Control interface + MIDI Streaming interface; 3 embedded IN jacks and 3 embedded OUT jacks; bulk EP 0x01 OUT and 0x81 IN, 64-byte packets, each carrying 3 cables | **Standard USB-MIDI 1.0 class device.** Cables 0/1/2 = "Public", "Private", "MIDI Port". No vendor driver needed; messages are 4-byte USB-MIDI event packets |
| RawMIDI | `midiC0D0` with 3 substreams; factory app opens all of them (65,536-byte buffers) | Ports map 1:1 to cables |
| Sequencer | Client 16 (kernel, 3 ports), 128 "JUCE" (virtual), 129 "MPC"; only port 2 ("MIDI Port") is routed through the sequencer | Pads/buttons use RawMIDI directly, not the sequencer |
| Kernel config | `CONFIG_USB_MON=y`, `CONFIG_DEBUG_FS=y`, `DEBUG_FS_ALLOW_ALL=y`; debugfs already mounted at `/sys/kernel/debug`; `usb/usbmon/{0..3}{s,t,u}` present | **A passive USB capture is possible without loading modules or mounting anything** (`--midi-capture`) |
| Input devices | `ILI2117 Touchscreen` (event0) and `gpio-keys` (event1) with only `KEY_POWER` | Everything except touch and power goes through USB MIDI |

Combined with the [counter map](MPC_ONE_3_6_0_MIDI_ACTIVITY_20261109.md): all controls arrive on cable 1 ("Private"); pad-related output goes to cable 0 ("Public") and button-related output to cable 1.

## Prior art (hypothesis only)

TheKikGen's MPCLiveXplore-libs (LD_PRELOAD mapper for the factory app) lists the MPC One as product code `ACVA`, SysEx device ID `0x46`, with pad notes, button notes and an RGB pad SysEx `F0 47 7F <id> 65 00 04 <pad> <r> <g> <b> F7` sent on the Public port. The source headers say **CC BY-NC 4.0** (the repository LICENSE file says CC0), so **no code or tables are copied into MPFree**. We treat those values as hypotheses and confirm each one on this unit with `--midi-capture`.

## Next

1. Re-run `--audio-midi` (device-tree fix) to get the codec, I2S format and MCLK properties.
2. Run `--midi-capture` to record actual pad/button/knob messages and LED output from the factory app.
3. After the reversible-handoff gate: enumerate ACVA hardware constraints with our own tool, then the ADR-0002 xrun and latency tests.
