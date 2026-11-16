# The MPC plugin SDK and the factory OS

**Status:** research and planning, 2026-11-15; idea 4 started (#361, #367). The ranked plan lives in the project's plan document; this page keeps the facts and the line we keep, for the repository.

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
4. MPFree's engines as MPC plugins, through a Rust VST2-ABI shim, so Akai projects that use them import into MPFree with every setting. Built for Delay and Squelch (#361, #367), untried on the MPC: [MPFree engines as MPC plugins](../guides/mpc-plugins.md).
5. Bridge plugins that pass audio, notes and transport over shared memory between Akai's app and an MPFree process.
6. Drawing MPC plugin skins in MPFree when it hosts MPC plugins.
7. What 3.10.1's remote screen and audio enhancement speaks.

## The line we keep

Our plugins use only what the SDK documents: the host interface, their own folder, and ordinary Linux services. We never read or patch Akai's memory, look up symbols in `/usr/bin/MPC`, hook its functions, touch activation or licence data, answer a licence check in Akai's place, or copy Akai's files. Anything that changes device state follows the device safety rule in [the agents' README](../../agents/README.md).

## First device facts

`collect-over-ssh.sh --plugin-host` ([survey tools](../guides/survey-tools.md)) reads, without changing anything: SSH servers and ports, how `acvs.service` runs Akai's app (user, limits, sandbox properties), what the app process can do (capabilities, `NoNewPrivs`, seccomp, memory lock limit), its threads' CPUs and real-time priorities, the plugin folders on each drive, plugin settings lines, security modules, signed-filesystem units, mounts and free memory. Results go here once reviewed.

## Running community MPC plugins in MPFree

The [Open MPC catalog](https://open-mpc.github.io/) lists community plugins for this format (255 entries in `catalog.json` on 2026-11-15: 136 instruments, 107 effects, 12 add-ins). The SDK documents the binary interface and the host's call order exactly, and `tools/host-sim` checks a host against it, so MPFree can **load these binaries as they are** rather than rebuild each one: about 184 source repositories with different wrappers, against one host built once. The binaries already fit the MPC One (ARMv7 hard-float with NEON, glibc 2.38 or older, static C++ runtime, 44.1 kHz). Full analysis: the project document "Running the Open MPC plugins in MPFree".

What the catalog needs from a host (each plugin counted once):

| Group | Count | Needs |
|---|---|---|
| Plain | 187 | Only the documented contract: audio in place, notes, transport, parameters, state chunk, skins |
| Sends MIDI | 27 | Its own ALSA sequencer port (the MPC host discards MIDI out); the host routes that port to tracks |
| Network | 4 | Network access from the plugin process |
| Companion pairs | 3 | Two plugins sharing one process |
| ROM | 1 | A ROM file the user supplies |
| Extra outputs | 1 | Output pairs past the first routed to mixer channels |
| 64-bit only | 14 | An arm64 MPC; cannot run on the MPC One |
| Add-ins | 12 | Not audio plugins: they extend Akai's app |
| Built on Akai internals | 6 | Checked one by one; we never load Akai code |

What MPFree would add, in the order it unlocks plugins:

1. A VST2-ABI plugin format in `mpfree-plughost`, next to CLAP, reusing `rust/mpfree-mpcplug/src/abi.rs` and copying the host contract (`open` twice, the parameter 0 nudge, in-place buffers, 32 to 512 frame blocks, the host's `canDo` answers and vendor strings, the transport through `audioMasterGetTime`). `host_sim` is the reference. This is the same piece as the synths work's VST2-ABI host, so it is built once.
2. Finding plugins: `VST2 Plugins/<folder>/plugin.json` on each drive and the older community `Synths` layout (the SDK's `legacy_convert.py` shows the mapping); copy each into its plugin home, since drives mount noexec.
3. Drawing `Plugin Skins/TUI.json` and `Q-Links.json` as the SDK's `ui-reference.md` describes; generic parameter pages until then.
4. ALSA sequencer ports from plugin processes routed to tracks, and `audioMasterProcessEvents` accepted.
5. One sandbox process for a companion pair; 6. extra outputs; 7. optionally a catalog browser that downloads on the user's request and checks the sha256.

Licences (MIT 136, GPL-3.0 89, AGPL 8, PolyForm Noncommercial 7, others): hosting binaries the user installs, in a separate process, puts no duty on MPFree's MIT code, the same line as the CLAP pack. MPFree images never bundle GPL, AGPL or noncommercial plugins.

Next step, desktop only: run five MIT catalog plugins under `host_sim` on 32-bit ARM (qemu) in CI, then the same five in MPFree once the format exists.
