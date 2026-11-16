<!--
covers: rust/mpfree-mpchost rust/mpfree-mpchost-testplug rust/mpfree-plughost/src/child.rs rust/mpfree-plughost/src/rack.rs tools/mpc-plugins/catalog-check.sh tools/mpc-plugins/catalog-check.txt
checked: 2026-11-16 d653b57
-->

# MPC plugins in MPFree

**Status:** 2026-11-15. Built and tested on the workstation and on ARMv7 under qemu: MPFree's plugin sandbox loads MPC-format plugins next to CLAP ones, and CI runs five MIT-licensed plugins from the Open MPC catalog as their real 32-bit ARM releases. Nothing has run on an MPC yet. An instrument's skin shows on the Synth tab ([MPC plugin skins](mpc-plugin-skins.md)); effects in the chain editor still get the generic list, and a plugin's MIDI output is dropped.

MPC OS 3.10.1 loads third-party instruments and effects: Linux shared libraries that speak the VST 2.4 binary interface and export one symbol, `VSTPluginMain`, each in a plugin folder with its skins. The [Open MPC catalog](https://open-mpc.github.io) lists a few hundred of them, most with source. [`rust/mpfree-mpchost`](../../../rust/mpfree-mpchost) loads them as `mpfree_node::Node`s, and the [sandbox](plugins.md#p2-the-sandbox) runs each in its own process, as it runs CLAP plugins. Nothing is ported or rebuilt: the same `.so` an MPC loads is what MPFree loads. This is the other direction from [MPFree engines as MPC plugins](../guides/mpc-plugins.md) (`rust/mpfree-mpcplug`), and both use the same clean-room description of the interface (`mpfree_mpcplug::abi`, written from the inMusic SDK's MIT `mpc_vst2_abi.h`; no Steinberg header).

MPFree never bundles these plugins. People pick the ones they want: BROWSE's Plugins place downloads and installs them from the catalog, or from zips copied to the stick ([Installing MPC plugins](mpc-plugin-store.md)), or they copy folders to `files/plugins/` by hand (Jay, 15 Nov). Hosting a binary the user installed puts no licence duty on MPFree.

## Finding plugins

[`folder.rs`](../../../rust/mpfree-mpchost/src/folder.rs) reads a plugin folder from the first description it has:

| File | Who writes it | Library |
|---|---|---|
| `plugin.json` | inMusic's SDK, for MPC OS 3.10.1 | `file32` on 32-bit ARM, `file64` on 64-bit ARM, else `file` |
| `plugin-meta.xml` | the catalog's "portable" folders: the `<PLUGIN/>` line older MPC OS versions keep in their plugin list | `file="%payload-path%/<folder>/x.so"` |
| `mpc-plugin.json` | the catalog's own manifest | `so` |

With none of them, a folder holding exactly one `.so` is that plugin. MPC plugins are always folders: a loose `.so` could be a CLAP plugin (MPFree's own CLAP test plugin is one), so it is left to CLAP. The library's path must stay inside the folder, as the MPC requires. A JSON reader of our own ([`json.rs`](../../../rust/mpfree-mpchost/src/json.rs)) reads the top-level strings, numbers and booleans; no new dependency.

`find(dir)` looks where people unpack plugins: plugin folders straight in `files/plugins/`, the folders inside a copied `VST2 Plugins/` or `Synths/` folder, and a catalog release folder's `portable/`. Nothing deeper. The rack ([`rack.rs`](../../../rust/mpfree-plughost/src/rack.rs)) lists them after the `.clap` files, each through `mpfree-plughost --list` so a plugin that crashes on load costs only that process. `Rack::add` lists a folder installed while MPFree runs the same way and offers it at once (the Plugins place calls it after an install).

## Ids

An MPC plugin's id in MPFree is its four-character unique id (as letters when it is letters and digits, else eight hex digits), a dot, and its name in lowercase letters, digits and dashes: `AwXn.density2`. The node id is `mpc:` and that: `mpc:AwXn.density2`. Projects keep it. The plugin process checks that the folder it was given holds that plugin, so a project never gets a different plugin that happens to sit in the same folder name.

## Loading and running one

[`host.rs`](../../../rust/mpfree-mpchost/src/host.rs) follows the call order of the inMusic SDK's `docs/host-contract.md`, so a plugin sees what it sees on an MPC:

- **Load.** `dlopen` (never closed), `VSTPluginMain` with our callback, the magic number checked. Sample rate (44.1 kHz) and block size, `open`, "can be automated" for every parameter, program 0, the names. The plugin process sets the current folder to the plugin folder and `MPC_PLUGIN_FOLDER` to it first (the SDK's plugins look there).
- **Kind.** An instrument if the library flags itself a synth or says its category is synth, or its folder says instrument; otherwise an effect. An effect asking for more than two inputs is refused, as the MPC skips it.
- **Prepare.** Sample rate and block size again (at least 128 announced, as on the MPC), resume, start. Scratch buffers are made here, never on the audio thread.
- **Process.** The transport goes in the time info every block (tempo, position, playing, a 4/4 bar: MPFree has no time signature yet). An instrument gets its MIDI list every block, even an empty one: note on and off, poly pressure, pitch bend (14 bits), and all notes off plus all sound off for `AllOff`. Effects get no MIDI, as on the MPC. Parameter events are set just before the block. An effect is processed in place (the same buffers in and out, as the MPC does); an instrument renders into scratch buffers and adds them into the track's. A mono output goes to both sides; outputs past two are rendered and not used yet. No allocation, lock or system call on the audio thread (a test checks with the counting allocator).
- **Reset.** Suspend and resume, plus all notes off for an instrument.
- **Parameters.** Every parameter is 0 to 1, as the host sees it on the MPC. Names come from the plugin; text comes from its display text for the current value (a VST 2 plugin can't write text for any other value). For a skin, the GUI asks for every value and its text in one round trip (`ToChild::Values`, answered by `ToHost::Values`; `Rack::values`), a few times a second while the skin shows; a CLAP plugin answers nothing.
- **Programs.** The plugin's built-in programs are its presets, listed by number (`--presets`) and loaded with `effSetProgram`, as the MPC's preset list does.
- **State.** `MPCS`, a version byte, the program number, then the plugin's own chunk (bank chunk, if it keeps one) or every parameter's value: what the MPC saves.

What the host answers a plugin: version 2400, sample rate, block size, the time info, its folder (`getDirectory`), "MPFree" as vendor and product, English, and yes to sending and receiving MIDI and time info, as the MPC does. It answers nothing private to Akai's app. A plugin that needs something private (a licence check, Akai's own symbols) is not supported, and the route to Akai's own instruments is guest mode, never answering for Akai.

## Tests

- [`mpfree-mpchost-testplug`](../../../rust/mpfree-mpchost-testplug) is one `.so` that is MPFree's Delay or Squelch made into an MPC plugin by `mpfree-mpcplug`, picked by the name of the folder it is loaded from (it asks the host for its folder while it is being made). [`tests/host.rs`](../../../rust/mpfree-mpchost/tests/host.rs) loads it in-process: an effect and an instrument sound the same as the engine run in-process, sample for sample; state and programs cross instances; the audio thread never allocates.
- [`mpfree-plughost/tests/mpc.rs`](../../../rust/mpfree-plughost/tests/mpc.rs) does the same through a real plugin process from catalog-style folders, checks the rack finds `VST2 Plugins/` and release folders, and refuses a folder holding a different plugin.
- **Real catalog plugins.** [`tools/mpc-plugins/catalog-check.sh`](../../../tools/mpc-plugins/catalog-check.sh) downloads the releases pinned in [`catalog-check.txt`](../../../tools/mpc-plugins/catalog-check.txt) (checked against their SHA-256, cached, never committed), builds `mpfree-plughost` and `rack_check` for ARMv7 and runs them under qemu-arm. Each plugin must play two seconds, give finite sound, and hand its state to a second copy. The five are all MIT: Density2 and Tidal Verb (effects), [SYN] Mono Voice, [SYN] Modal Voice and Dub Force Siren (instruments). All pass (15 Nov). CI runs this on every pull request; it takes about a minute.

## What the catalog needs

From the catalog review of 15 Nov (255 entries): 187 need only this loader; 36 need one more host feature each, most of them the 27 sequencers that send MIDI out through their own ALSA port, the rest a ROM file, network access, a companion plugin pair or extra outputs; 32 can't run in MPFree (14 are 64-bit ARM only, 12 are add-ins for Akai's app, 6 rely on Akai's internals). Plugins that draw straight onto the framebuffer (the Tuner) are in the last group in practice.

## Next

1. **Skins in the chain editor.** The Synth tab draws an instrument's `Plugin Skins/TUI.json` and follows its `Q-Links.json` ([MPC plugin skins](mpc-plugin-skins.md)); effects need the insert to know its plugin instance first.
2. **MIDI out.** Route what a plugin sends (`audioMasterProcessEvents`) to MPFree's tracks or an ALSA port, for the sequencer plugins.
3. **Extra outputs** as stereo pairs on the mixer.
4. **On the MPC.** The sandbox already runs ARM; a device test is a labelled run on the soft-boot stick, on Jay's go.
