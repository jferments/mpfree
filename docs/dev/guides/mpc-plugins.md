# MPFree engines as MPC plugins

**Status:** desktop and qemu only, 2026-11-15. Nothing has been copied to an MPC yet.

Since MPC OS 3.10.1, Akai's app loads third-party plugins. [`rust/mpfree-mpcplug`](../../../rust/mpfree-mpcplug) wraps any MPFree node (an effect from `mpfree-fx`, an instrument from `mpfree-synth`) in the binary interface that host speaks: VST 2.4, one exported symbol, `VSTPluginMain`. Why we want it, and the line we keep, are in [the plugin SDK research](../research/mpc-plugin-sdk.md).

## How a plugin is made

A plugin is a `cdylib` that names itself with a `Spec` and calls `export_mpc_plugin!`. There are two examples: [`mpfree_delay`](../../../rust/mpfree-mpcplug/examples/mpfree_delay.rs) (the Delay effect) and [`mpfree_squelch`](../../../rust/mpfree-mpcplug/examples/mpfree_squelch.rs) (the Squelch instrument). The wrapper:

- shows the node's writable parameters, mapped to the host's 0 to 1 range by each parameter's own curve, with value text such as `375 ms` and choice names;
- keeps parameters in atomics and hands them to the node at the start of each block, because the MPC writes parameters from other threads while audio runs;
- processes in place, and cuts calls longer than the announced block size into pieces;
- turns the host's MIDI (note on and off, pitch bend) into node events for instruments;
- reads tempo, beat position and play state from the host every block;
- saves state as text, `id=value` per parameter by its stable node id, so later versions load old projects;
- has no screen of its own yet. The MPC shows its generic page of 16 sliders until we generate `Plugin Skins`.

## Building for the MPC One

```sh
bash tools/mpc-plugins/build-arm32.sh
```

[`build-arm32.sh`](../../../tools/mpc-plugins/build-arm32.sh) builds the examples for 32-bit ARM and checks each `.so` against the SDK's rules: an ELF32 hard-float shared object that exports only `VSTPluginMain`, needs only libc, libm and the loader, and asks for glibc 2.39 at most. Rust's std asks for `libgcc_s.so.1` by name, so the script points that name at GCC's static unwinder, as the SDK's C++ builds do with `-static-libgcc`. CI runs this script on every run of the MPC inventory workflow. The host-contract tests (`cargo test -p mpfree-mpcplug`) run with the workspace tests.

## Checking with the SDK's host simulator

The [inMusic SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk)'s `tools/host-sim` is a desktop host that follows the MPC host's call order: it loads the plugin, prepares it, renders, changes block sizes, sweeps parameters, round-trips state and processes in place. To run it by hand at SDK commit `febd2ca5`:

```sh
c++ -std=c++17 -O2 -o host_sim mpc-plugin-builder-sdk/tools/host-sim/host_sim.cpp -ldl
cargo build --manifest-path rust/Cargo.toml -p mpfree-mpcplug --examples
./host_sim rust/target/debug/examples/libmpfree_delay.so test --wav delay.wav
```

For the ARM build, compile `host_sim.cpp` with `arm-linux-gnueabihf-g++` and run it under `qemu-arm -L /usr/arm-linux-gnueabihf`. On 2026-11-15 both examples passed all 19 checks natively and as ARMv7 under qemu. The SDK's `validate.py` passed the Delay's folder except for a missing `file64`, because we build only the 32-bit file that the MPC One loads.

## A plugin folder

The MPC wants a folder named `<vendor> - VST - <name>`, for example `MPFree - VST - Delay/`, holding the `.so` and a `plugin.json` whose `name`, `manufacturer` and `uid` match the `Spec` (see the SDK's `docs/plugin-folder-format.md`). The folder goes in `VST2 Plugins/` at the root of a drive. Deleting the folder removes the plugin. Putting one on the MPC writes to that drive, so it is a labelled step that needs the owner's go.

## Not done yet

- Skins generated from our parameter groups, so the MPC shows named pages and maps the Q-Links.
- A 64-bit build for newer MPCs.
- Presets as programs.
- A first run on the MPC itself, with CPU measured there.
