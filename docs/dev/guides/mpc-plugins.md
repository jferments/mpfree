# MPFree engines as MPC plugins

**Status:** desktop and qemu only, 2026-11-15. Complete plugin folders build and pass the SDK's checks; nothing has been copied to an MPC yet.

Since MPC OS 3.10.1, Akai's app loads third-party plugins. [`rust/mpfree-mpcplug`](../../../rust/mpfree-mpcplug) wraps any MPFree node (an effect from `mpfree-fx`, an instrument from `mpfree-synth`) in the binary interface that host speaks: VST 2.4, one exported symbol, `VSTPluginMain`. Why we want it, and the line we keep, are in [the plugin SDK research](../research/mpc-plugin-sdk.md). The other direction, MPC plugins running inside MPFree, is [MPC plugins in MPFree](../design/mpc-plugin-host.md).

## How a plugin is made

A plugin is a `cdylib` that calls `export_mpc_plugin!` with a `Spec`: its name, vendor, four-character id and how to make the node. The specs are written once, in [`examples/common/plugins.rs`](../../../rust/mpfree-mpcplug/examples/common/plugins.rs). So far there are two: [`mpfree_delay`](../../../rust/mpfree-mpcplug/examples/mpfree_delay.rs) (the Delay effect) and [`mpfree_squelch`](../../../rust/mpfree-mpcplug/examples/mpfree_squelch.rs) (the Squelch instrument). Adding another engine is a new `Spec`, a line in `ALL` and a three-line example. The wrapper:

- shows the node's writable parameters, mapped to the host's 0 to 1 range by each parameter's own curve, with value text such as `375 ms` and choice names;
- keeps parameters in atomics and hands them to the node at the start of each block, because the MPC writes parameters from other threads while audio runs;
- processes in place, and cuts calls longer than the announced block size into pieces;
- turns the host's MIDI (note on and off, pitch bend) into node events for instruments;
- reads tempo, beat position and play state from the host every block;
- saves state as text, `id=value` per parameter by its stable node id, so later versions load old projects;
- names parameters uniquely, because the MPC's skins find a parameter by its name: a name several groups share gets its group's tag in front ("B1 Freq" for the EQ's first band). A test checks every MPFree effect and synth for this.

## The screen

The MPC draws a plugin's screen from `Plugin Skins/TUI.json` and `Q-Links.json`. [`skin.rs`](../../../rust/mpfree-mpcplug/src/skin.rs) writes the SDK's short `ui.json` from the node's own parameter groups. Each group gets a page named after it, such as TIME, REPEATS and OUTPUT for the Delay. Each page holds up to 16 controls in the MPC's 4 x 4 grid, so the Q-Links work in blocks of four. Switches become switches, choices become sliders with a menu, and ranges that cross zero get a centred knob. The SDK's `gen_ui.py` turns that into the skin files. A test checks that every MPFree effect and synth fits the MPC's limits: at most 32 pages, 16 controls a page and 8 letters a page name.

## Building

```sh
bash tools/mpc-plugins/build.sh arm32      # MPC One and the other 32-bit MPCs
bash tools/mpc-plugins/build.sh arm64      # MPC Live III, MPC XL and later
bash tools/mpc-plugins/make-folders.sh     # complete folders in rust/target/mpc-plugins/
```

[`build.sh`](../../../tools/mpc-plugins/build.sh) builds the plugins for one CPU family and checks each `.so` against the SDK's rules. It must be a shared object for the right CPU that exports only `VSTPluginMain`, needs only libc, libm and the loader, and asks for glibc 2.39 at most. Rust's std asks for `libgcc_s.so.1` by name, so the script points that name at GCC's static unwinder, as the SDK's C++ builds do with `-static-libgcc`. CI runs `build.sh arm32`. The host-contract and catalogue tests (`cargo test -p mpfree-mpcplug`) run with the workspace tests.

[`make-folders.sh`](../../../tools/mpc-plugins/make-folders.sh) does the rest. It fetches the SDK at a pinned commit and builds both CPU families. It writes each folder's `plugin.json` and `version.xml` (from `examples/make_folders.rs`) and dumps the parameters with the SDK's host simulator. It then makes the skins with `gen_ui.py` and runs the simulator's `test` and `validate.py --strict` on each folder. On 2026-11-15 both folders passed with no warnings.

## Checking with the SDK's host simulator

The [inMusic SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk)'s `tools/host-sim` is a desktop host that follows the MPC host's call order: it loads the plugin, prepares it, renders, changes block sizes, sweeps parameters, round-trips state and processes in place. To run it by hand at SDK commit `febd2ca5`:

```sh
c++ -std=c++17 -O2 -o host_sim mpc-plugin-builder-sdk/tools/host-sim/host_sim.cpp -ldl
cargo build --manifest-path rust/Cargo.toml -p mpfree-mpcplug --examples
./host_sim rust/target/debug/examples/libmpfree_delay.so test --wav delay.wav
```

For the ARM build, compile `host_sim.cpp` with `arm-linux-gnueabihf-g++` and run it under `qemu-arm -L /usr/arm-linux-gnueabihf`. On 2026-11-15 both examples passed all 19 checks natively and as ARMv7 under qemu. 

## A plugin folder

The MPC wants a folder named `<vendor> - VST - <name>`, for example `MPFree - VST - Delay/`, holding the `.so` and a `plugin.json` whose `name`, `manufacturer` and `uid` match the `Spec` (see the SDK's `docs/plugin-folder-format.md`). The folder goes in `VST2 Plugins/` at the root of a drive. Deleting the folder removes the plugin. Putting one on the MPC writes to that drive, so it is a labelled step that needs the owner's go.

## Not done yet

- Presets as programs.
- A first run on the MPC itself, with CPU measured there.
