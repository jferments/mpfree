# Akai's own instruments: guest mode, the laptop route and MPC-format plugins

**Status:** 2026-11-16, plan only; no guest-mode code exists. Written at the close of the thread "Make synths and plugins credible" (15 to 16 Nov). The two read-only checks it asks for are built and tested on fakes, and have not run on Jay's MPC yet: [`tools/mpc-one/offline-akai-scan.py`](../../../tools/mpc-one/offline-akai-scan.py) and [`tools/mpc-one/read-only-akai-app.sh`](../../../tools/mpc-one/read-only-akai-app.sh). MPC-format plugins (VST2-ABI `.so` files) load in the sandbox since #385 ([plugins](plugins.md)). The full plan, with the reasoning behind each option, is the doc "Synths and plugins: making them credible" (https://claude.ai/artifact/MRFtqAaD2eMNJAQM8Emvcy; a copy is in mpfree-claude `synths/`).

Akai's instruments (Hype, TubeSynth, Bassline, Electric, Mellotron, Solina, Odyssey, Drumsynth, and the bought Fabric, Organ, Stage, Studio Strings and OPx-4) are already activated on the owner's MPC. The question is how to play them from MPFree on the same machine without copying Akai's code, shipping it, or answering Akai's licence check ourselves.

## What we know

| Fact | Source |
|---|---|
| Akai's app is `/usr/bin/MPC`, a JUCE program started by `az01-launch-MPC` from `acvs.service` | [Factory GUI handoff](../research/factory-gui-handoff.md), [audio runtime](../hardware/audio-runtime.md) |
| It holds about 1,019 MiB resident and 1,016 MiB locked of about 2,019 MiB, with no swap (on 3.6.0) | [Hardware ownership](../research/hardware-ownership.md) |
| It owns `/dev/dri/card0`, the touch and GPIO inputs, the ACVA PCMs and raw MIDI to the pads, and publishes ALSA sequencer ports (JUCE virtual MIDI in and out) | [Reversible handoff](../research/reversible-handoff.md), [audio runtime](../hardware/audio-runtime.md) |
| Third-party MPC plugins (OS 3.10.1 and later) are Linux `.so` files with the VST 2.4 binary interface (`VSTPluginMain`), with skin JSON and `Q-Links.json`; inMusic's SDK is MIT with a clean-room header | [inMusic SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk), [open-mpc notes](https://github.com/open-mpc/mpc-vst-plugins) |
| Akai's included instruments are reportedly built into `/usr/bin/MPC`, not plugin files | open-mpc notes; **unverified**: the scan answers it |
| Activation is tied to the owner's inMusic profile and kept by Akai's app | [Akai support](https://support.akaipro.com/en/support/solutions/articles/69000864882-akai-pro-mpc-series-troubleshooting-activation-issues-on-mpc-standalone-hardware) |
| Akai licenses the instruments on computers too: desktop MPC software, Hype, Solina, Mellotron and Odyssey need no activation there, expansions may be active on up to 3 devices | [Akai support](https://support.akaipro.com/en/support/solutions/articles/69000816306) (written for the Key 61; check for the MPC One) |
| MPFree boots by a soft-reboot from a stick that **unbinds the eMMC and SD controllers** before anything mounts, so Akai's rootfs and `/data` are out of reach while MPFree runs | [Soft-reboot boot](../hardware/soft-reboot-boot.md) |

## The options

| Option | Verdict |
|---|---|
| **A. Host MPC-format plugins** in the sandbox | Built (#385). Covers everything made with inMusic's SDK. A plugin that asks Akai's host for a licence through private calls is reported as unsupported, never answered by us. |
| **B. Guest mode**: Akai's unmodified app runs as a child of MPFree, as a background sound engine | **Jay's pick (15 Nov)**, with resource use as his main concern. Details below. |
| **C. Load Akai's instrument code into MPFree** | No. Brittle per firmware, and replacing Akai's licence check with ours is the step that becomes circumvention, valid licence or not. |
| **D. Boot back into Akai's OS** (stick out) | Works today; the fallback until B exists. |
| **E. Import the sounds** (keygroup programs, expansion samples on the owner's drive) | Started: MPC import stage 1 (#382) and the private factory-keygroups tool. |
| **F. Akai's desktop software on a laptop**, over the [offload](offload.md) link | Yes, next to B. Native speed, Akai's own activation, none of Akai's memory on the MPC. If the instruments load as VST3/AU outside the MPC app, the offload server hosts them; if not, it runs Akai's desktop app as a sound module. First step: Jay checks his laptop's plugin folders. |
| **G. Akai's OS in a QEMU VM on a laptop** | No. QEMU has no model of the board, emulated ARM runs at about the MPC's speed, and the activation would only pass if the VM pretended to be the owner's MPC. |

## Guest mode

MPFree starts `/usr/bin/MPC` with stand-in devices:

- **Audio:** if the app links `libasound` dynamically, a private `asound.conf` in its own mount namespace points it at an ALSA plugin of ours: a ring buffer in shared memory, clocked by MPFree's period (one extra period, about 3 ms at 128 frames). Otherwise a small `LD_PRELOAD` shim on `snd_pcm_open`. One stereo return at first.
- **Notes:** over its existing ALSA sequencer ports. Each MPFree track plays a track of a "rack" project in Akai's app.
- **Screen and touch (later):** the kernel's `vkms`, if Akai's kernel has it, or a shim on the DRM calls; touches back through `uinput` or the same shim. Pads and buttons forwarded through a virtual raw MIDI port while an "MPC" window has focus (MPFree already decodes the controller protocol).
- **Per-track returns (3.10.1):** a small SDK instrument and effect pair inside Akai's app could hand MPFree each track's audio, the notes Akai's sequencer plays and the exact transport over shared memory, through the SDK's documented interface only (idea from the plugin SDK thread).

**The first design question** is storage. Guest mode needs Akai's rootfs (p6) and `/data` (p8: settings and activation) while MPFree runs, and the soft-reboot stick switches the eMMC off on purpose. Options: bind the eMMC again with p6 and p8 mounted read-only (Akai's app writes settings to `/data`, so it may need a RAM overlay there), or a private copy on the owner's own stick. Either changes the boot's safety design, so it is Jay's call, with the [safety rule](../../agents/README.md) applied.

**Keeping it light** (Jay's concern), cheapest first: start it only when a project uses an Akai instrument and stop it when none does; one copy for every track; print Akai tracks to audio and stop it; start it without the right to lock memory (no `CAP_IPC_LOCK`, a small `RLIMIT_MEMLOCK`), so untouched memory stays unallocated and code pages can be dropped (risk: page faults on its audio thread); a smaller sample pool if its settings have one; its own cgroup with a CPU share and core set; or option F.

**What the first guest-mode test measures:** resident and locked memory with an empty rack and with Hype playing, idle CPU, CPU per Hype voice, start time, and whether the app starts at all without its real screen and controller.

## The two read-only checks

- `python3 tools/mpc-one/offline-akai-scan.py [--backup FOLDER] [--rootfs-image FILE|newest]`, on the workstation. Reads the backup's rootfs, content and data with debugfs (read-only, nothing mounted), or a rootfs unpacked from an Akai update instead of the backup's. Reports: Akai's app (ELF type, `DT_NEEDED`, whether it imports `mlockall`, `snd_pcm_open`, DRM and EGL calls, instrument names in its strings and symbols); plugin-like files and folders on all three partitions and whether they export `VSTPluginMain`; the **names** of files that look like licence or activation data (never their contents) and strings in the app that name such paths; the launcher, `acvs.service`, ALSA plugins, the kernel's config for `vkms`, `snd-aloop`, `snd-virmidi`, `uinput`, namespaces and cgroups; and the app's memory, voice and plugin settings (keys that suggest accounts, keys, serials or activation are left out; e-mail addresses are masked). Tests: `tools/mpc-one/tests/test_offline_akai_scan.py`.
- `ssh -o StrictHostKeyChecking=yes root@<mpc> 'sh -s' < tools/mpc-one/read-only-akai-app.sh > akai-app-empty.txt`, on the MPC in a **stock** boot (stick out) with developer mode on, transport stopped. Reads `/proc` only. Run it with an empty project and again with Hype and Mellotron loaded: memory and locked memory, memlock limit, cgroup, per-thread CPU and real-time policy, mapped files (a separate instrument library appears once loaded), open devices, sound and input devices, kernel features. The host key must already be trusted ([3.10.1 qualification](../hardware/firmware-3.10.1-qualification.md), `--devmode-trust`). Tests: `tools/mpc-one/tests/test_read_only_akai_app.py`.

## Next

1. Run the two checks (an optional add-on to the next device round).
2. Jay checks whether Akai's desktop instruments appear as VST3/AU plugins on his laptop.
3. Jay decides how guest mode reaches Akai's rootfs and `/data` while MPFree runs.
4. A guest-mode prototype, sound only: our ALSA plugin, the sequencer ports, a rack project, one stereo return, and the measurements above.
5. Read inMusic's licence terms on running Akai's app under another OS on the same MPC, and get a lawyer's view before guest mode is published.
