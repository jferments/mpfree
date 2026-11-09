# Audio and MIDI diagnostics (read-only, phase 1)

**Goal:** collect what the direct-ALSA engine ([ADR-0002](ADR-0002-DIRECT-ALSA-AUDIO.md)) and the internal-controller MIDI backend need, **without stopping or disturbing the factory app**. Everything here is read-only. Nothing opens `/dev/snd/*`, sends MIDI, mounts filesystems, loads modules, or changes services, scheduling, CPU frequency or files on the MPC.

Both modes run through `tools/mpc-one/collect-over-ssh.sh`, so the usual direct-Ethernet, route, MAC and SSH host-key checks run first. Reports stay in the local, gitignored `research/` directory.

## `--audio-midi`: one-shot survey

Script: [`read-only-audio-midi.sh`](../../tools/mpc-one/read-only-audio-midi.sh). It takes a few seconds.

| What it reads | Why the engine needs it |
| --- | --- |
| `/proc/asound/*` cards, PCM info, current ACVA `hw_params`/`sw_params`/`status`, timers | Confirm the stream profile and buffer layout we must match |
| `/sys/class/sound`, device-tree nodes for I2S/codec/DMA/sound card | Codec identity, DAI format, MCLK ratio, DMA channels; hints at supported rates |
| Loaded sound/USB modules and their parameters | Driver behavior, `snd_usb_audio` options |
| Selected kernel config symbols (if `/proc/config.gz` exists) | `HZ`, `PREEMPT_RT`, `RT_GROUP_SCHED`, `USB_MON`, `DEBUG_FS`, ALSA options |
| debugfs ASoC lists, **only if debugfs is already mounted** | DAI/component names |
| usbmon availability (module present or built in; nothing loaded) | Whether a later passive USB capture can decode pad and LED messages |
| `/proc/interrupts`, affinity of I2S/DMA/USB IRQs, priorities of threaded-IRQ kthreads | Choosing the audio core and SCHED_FIFO priority relative to the I2S/DMA IRQ threads |
| cpufreq governor/frequencies, cpuidle states | Frequency scaling and deep idle are common xrun causes |
| RT limits, cgroup layout, `limits.conf` RT/memlock lines | Whether our process can get SCHED_FIFO and `mlockall` |
| alsa-lib version, `asound.conf`, presence of ALSA/JACK tools (looked up, never run) | What userspace we can rely on |
| RawMIDI counters, sequencer clients, USB MIDI descriptors (cached copy in sysfs) and endpoints | MIDI port layout and endpoint packet sizes |
| `/proc/bus/input/devices` | Which controls are evdev rather than MIDI |

## `--midi-activity`: guided counter map (interactive)

Host tool [`host_midi_activity.py`](../../tools/mpc-one/host_midi_activity.py) walks the owner through eight short phases (idle, pads, pad pressure, Q-Links, data wheel, a few harmless buttons, play/stop, master volume). For each phase it reads the kernel's cumulative RawMIDI byte counters before and after an 8-second window, using [`read-only-midi-counters.sh`](../../tools/mpc-one/read-only-midi-counters.sh), and prints which internal MIDI substream changed. That answers research-matrix item MIDI-01 (which port carries pads, knobs and buttons) without opening any MIDI device. Any phase can be skipped.

The button phase only uses SHIFT and FULL LEVEL (pressed twice, so it ends where it started). Play/stop plays the currently loaded sequence briefly. Nothing is saved, loaded or deleted.

## What comes next (not in this phase)

These need separate, explicit opt-in steps, each with its own undo plan:

1. **Passive USB capture with usbmon**, if the survey shows it is available, to decode pad, knob and LED message formats while the factory app runs.
2. **Stopping the factory app** through the reversible handoff to free the ACVA card, followed by a capability probe (rates, formats, period limits), the xrun soak test and loopback latency test from ADR-0002, and finally restarting the factory app.
