# The MPC plugin SDK and the factory OS

**Status:** research and planning, 2026-11-15. The ranked plan lives in the project's plan document; this page keeps the facts and the line we keep, for the repository.

Since MPC OS 3.10.1 the factory app loads third-party instruments and effects built with the [inMusic MPC Plugin Builder SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk) (MIT). The same firmware adds a developer mode with Akai's own SSH server. The reference MPC One runs 3.10.1 since 2026-11-15, with developer mode on (SSH on port 22, key login). Our earlier `mpfree-sshd` on port 2222 (MOD-001) did not survive the update.

## What an SDK plugin is

From the SDK's `docs/host-contract.md` and `docs/plugin-folder-format.md`:

- A Linux `.so` with the VST 2.4 binary interface (exports only `VSTPluginMain`): ARMv7 hard-float with NEON for the MPC One, glibc 2.39 or older, static C++ runtime. The same build rules as our CLAP pack.
- It runs **inside Akai's app, with no sandbox**. It can use files, sockets, shared memory and threads, and a crash in it takes Akai's app down.
- Instruments receive the track's notes, controllers, bend, aftertouch and sysex, sample-accurate. Effects receive no MIDI. MIDI a plugin sends is discarded.
- Every block carries the transport: tempo, beat position, bar start, time signature, playing, recording, loop points.
- Effects get the track's stereo audio in place; instruments may have more than two outputs.
- Fixed 44.1 kHz; blocks of 32 to 512 frames, the announced size being a maximum.
- State is an opaque chunk saved in the project (up to 128 MB).
- The screen comes from JSON skins (`Plugin Skins/TUI.json`, `Q-Links.json`); a plugin cannot draw.
- Installed as a folder in `VST2 Plugins/` at the root of any drive, with Preferences, Enhancements, External Plugin Support turned on. Deleting the folder undoes it.
- `tools/host-sim` in the SDK is a desktop host that copies the MPC host's call order; its `test` command checks a plugin.

## Ideas, ranked

1. Use 3.10.1's developer SSH for our device tools (now done by the update; the tools are being ported).
2. A host probe: the timing log in the FX quality work's capture plugin (FXProbe Tap, `tools/fxprobe/`) records each process call's time, frames and CPU, the audio thread's scheduling, and every host call in order.
3. A "Switch to MPFree" plugin that asks the handoff supervisor to start as its own systemd unit. Only after the Tier 1 rehearsal in [reversible handoff](reversible-handoff.md), and after checking inMusic's terms.
4. MPFree's engines as MPC plugins, through a Rust VST2-ABI shim, so Akai projects that use them import into MPFree with every setting. Started: [MPFree engines as MPC plugins](../guides/mpc-plugins.md).
5. Bridge plugins that pass audio, notes and transport over shared memory between Akai's app and an MPFree process.
6. Drawing MPC plugin skins in MPFree when it hosts MPC plugins.
7. What 3.10.1's remote screen and audio enhancement speaks.

## The line we keep

Our plugins use only what the SDK documents: the host interface, their own folder, and ordinary Linux services. We never read or patch Akai's memory, look up symbols in `/usr/bin/MPC`, hook its functions, touch activation or licence data, answer a licence check in Akai's place, or copy Akai's files. Anything that changes device state follows the device safety rule in [the agents' README](../../agents/README.md).

## First device facts

`collect-over-ssh.sh --plugin-host` ([survey tools](../guides/survey-tools.md)) reads, without changing anything: SSH servers and ports, how `acvs.service` runs Akai's app (user, limits, sandbox properties), what the app process can do (capabilities, `NoNewPrivs`, seccomp, memory lock limit), its threads' CPUs and real-time priorities, the plugin folders on each drive, plugin settings lines, security modules, signed-filesystem units, mounts and free memory. Results go here once reviewed.
