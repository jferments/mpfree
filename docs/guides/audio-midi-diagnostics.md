# Audio and MIDI diagnostics (read-only, phase 1)

**Goal:** collect what the direct-ALSA engine ([ADR-0002](../decisions/adr-0002-direct-alsa-audio.md)) and the internal-controller MIDI backend need, **without stopping or disturbing the factory app**. Everything here is read-only. Nothing opens `/dev/snd/*`, sends MIDI, mounts filesystems, loads modules, or changes services, scheduling, CPU frequency or files on the MPC.

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

## `--midi-capture`: passive USB-MIDI capture (opt-in, interactive)

Remote script [`passive-usbmon-capture.sh`](../../tools/mpc-one/passive-usbmon-capture.sh), host tool [`host_midi_capture.py`](../../tools/mpc-one/host_midi_capture.py). Requires `MPFREE_ACCEPT_USBMON=YES`.

This is not strictly read-only, so it is opt-in. It reads the kernel's usbmon text interface for the controller's USB bus. While the file is open, the kernel copies a summary of each USB transfer on that bus to a buffer for our reader. USB traffic itself is not created, changed, delayed or blocked; a slow reader just loses events. It only runs if debugfs is **already** mounted and usbmon is **already** built in (both confirmed on 3.6.0). It never mounts, loads modules, writes files or sends MIDI. The remote side stops itself after 240 s at most, and ends sooner once the host disconnects. The kernel removes the hook when the reader closes.

The host tool walks through ten phases (single pad soft/hard, pads 1-16 in order, pressure, Q-Links, data wheel, SHIFT, FULL LEVEL, play/stop) and decodes USB-MIDI packets and SysEx for each phase. usbmon text shows at most 32 data bytes per transfer, so long LED bursts are marked as truncated.

`MPFREE_CAPTURE_PHASES=midi-cv` swaps in six phases about the rear jacks instead: idle, notes into the 5-pin MIDI In jack, pads on a MIDI track whose output is the MIDI Out jack, pads on a CV track, one held note on the CV track, and one CV slider. Before anything runs on the MPC, the tool lists what to set up in Akai's app (a new MIDI track and a new CV track, nothing saved, Preferences unchanged). The report adds, for each phase, the messages per direction and cable next to the idle phase's count. It answers which USB-MIDI cable carries the DIN jacks under Akai's app, and whether the CV/Gate jacks are driven over USB-MIDI at all. Nothing needs to be plugged into the CV/Gate jacks.

```bash
MPFREE_ACCEPT_USBMON=YES MPFREE_CAPTURE_PHASES=midi-cv bash tools/mpc-one/collect-over-ssh.sh --midi-capture
```

## What comes next (not in this phase)

These need separate, explicit opt-in steps, each with its own undo plan:

1. **Stopping the factory app** through the reversible handoff to free the ACVA card, followed by a capability probe (rates, formats, period limits), the xrun soak test and loopback latency test from ADR-0002, and finally restarting the factory app.
