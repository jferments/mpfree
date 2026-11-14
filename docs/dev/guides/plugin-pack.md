<!--
covers: tools/plugin-pack rust/mpfree-clap/examples/clap_check.rs rust/mpfree-plughost/examples/rack_check.rs
checked: 2026-11-14
-->
# The plugin pack

**Status:** 2026-11-14. Built on the cloud workstation and played under qemu-arm, in-process and through the plugin process the app uses. **No plugin has been loaded on the MPC yet**; that comes with the plugin passports (P4).

The plugin pack is step P3 of the [plugin plan](../design/plugins.md#next): free plugins built for the MPC One's ARM chip as CLAP files, with recipes anyone can run again. Jay chose Airwindows, Surge XT and Dexed on 13 Nov and gave the go to build them on 14 Nov. The GPL-3 ones ship as separate `.clap` files with their sources, run in their own plugin process and are never linked into MPFree.

Nothing built goes in this repository: not the plugins, not their sources. The repository holds the recipe, [`tools/plugin-pack/`](../../../tools/plugin-pack), and the checks.

## What is in it

| File | What | License | Plugins |
| --- | --- | --- | --- |
| `airwindows.clap` | Airwindows, the 89 effects of Airwindows Consolidated's "Recommended" collection (reverbs, delays, consoles, filters, saturation, dither, utilities) | MIT | 89 effects |
| `dexed.clap` | Dexed, a DX7-style FM synth | GPL-3.0-or-later; JUCE 8 inside it is AGPL-3.0 | 1 instrument |
| `surge-xt.clap` | Surge XT, the synth | GPL-3.0-or-later (JUCE 7 inside it is GPL-3.0) | 1 instrument |
| `surge-xt-effects.clap` | Surge XT's effects as a plugin of their own | GPL-3.0-or-later | 1 effect |
| `SurgeXTData/` | Surge XT's factory presets (637 patches), wavetables, effect and modulator presets and tunings, 35 MB, which both Surge XT files read | GPL-3.0-or-later | |

92 plugins in all, under the rack's limit of 128. Each plugin's folder in the pack also has:

- `licenses/`: the license of everything inside the file;
- `source/`: a tarball of each source at the commit built, submodules included, and `mpfree-plugin-pack-recipe.tar.xz`, the recipe that built it (together, the complete corresponding source the GPL asks for);
- `BUILD.txt`: what was built, from which commits, with which tools, what each file needs from the MPC and its SHA-256;
- `SHA256SUMS` (every file, the data folders' too), and `PLAY.txt` when the pack was played (below).

## Building it

```sh
python3 tools/plugin-pack/build.py --work ~/pack-work --out ~/pack --play
python3 tools/plugin-pack/build.py --work ~/pack-work --out ~/pack airwindows   # one plugin
```

`--work` holds the sources, the builds and the logs (`logs/<plugin>.log`); `--out` gets the pack. Both must be outside the repository. On the 4-core cloud workstation the whole pack takes about 20 minutes (Airwindows 3, Dexed 6, Surge XT 9, and the source tarballs); the sources take about 2 GB and the builds about 4 GB. Two builds of Airwindows and of Dexed from the same commits gave the same bytes.

It needs Ubuntu 24.04 (noble) with:

- `cmake`, `ninja-build`, `git`, `xz-utils`, Python 3.11 or later;
- `g++-arm-linux-gnueabihf`: the cross compiler, which links glibc 2.39, the MPC's release;
- the armhf libraries JUCE builds against: `libfreetype-dev:armhf libfontconfig-dev:armhf libpng-dev:armhf zlib1g-dev:armhf libbrotli-dev:armhf libbz2-dev:armhf libexpat1-dev:armhf libx11-dev:armhf libxext-dev:armhf libxrandr-dev:armhf libxinerama-dev:armhf libxcursor-dev:armhf libxcomposite-dev:armhf libxrender-dev:armhf libasound2-dev:armhf`. FreeType and fontconfig (with what they need) are linked into the JUCE plugins; the rest are headers only (below);
- `libc6-dev-i386 lib32gcc-13-dev` for Surge XT's LuaJIT, whose build tools must be 32-bit like the target. **Don't install `gcc-multilib`**: on Ubuntu it removes the ARM cross compiler;
- for `--play`: `qemu-user`, Rust with the `armv7-unknown-linux-gnueabihf` target.

Each source is fetched at the commit in [`pack.toml`](../../../tools/plugin-pack/pack.toml) and checked against it; a checkout already at that commit and clean is reused. The build folder starts fresh each time. To move a plugin to a new release, change its `tag` and `commit` in `pack.toml`, build, play and read the new `BUILD.txt`.

## What each file must be

The MPC runs its own Linux, and a plugin there can rely on nothing but the C library ([ABI survey](../hardware/abi-handoff-survey.md)). [`armv7-mpc.cmake`](../../../tools/plugin-pack/armv7-mpc.cmake) builds for the MPC's CPU (ARMv7 with NEON and VFPv4, hard-float), links the C++ library and libgcc into each file and exports only `clap_entry` ([`clap-exports.map`](../../../tools/plugin-pack/clap-exports.map)). `build.py` then checks every file and stops on any of these:

- not a 32-bit ARM hard-float shared library;
- needs a library other than `libc.so.6`, `libm.so.6`, the loader `ld-linux-armhf.so.3` and glibc's empty stubs (`libpthread`, `libdl`, `librt`);
- needs a glibc symbol newer than 2.39;
- needs anything from the MPC's C++ library or libgcc;
- doesn't export `clap_entry`.

Ubuntu's armhf compiler turns on 64-bit time and file offsets by default, so the plugins call glibc's `*_time64` functions. Those are glibc's own since 2.34, versioned `GLIBC_2.34`, and the check above covers them. Dexed is built with 32-bit ones instead (JUCE 8's zlib doesn't compile with 64-bit file offsets on 32-bit ARM), as the app itself is.

The JUCE plugins (Dexed, Surge XT) need a few more things handled:

- JUCE links FreeType (and JUCE 8 fontconfig) to draw text in an editor. [`static-deps.cmake`](../../../tools/plugin-pack/static-deps.cmake), included after each of their CMake projects, answers JUCE's pkg-config search for those two with the armhf packages' `.a` files (position-independent), and adds what they need in turn, so they go inside the `.clap`. The C library stays shared.
- ALSA is left out: Dexed with `DEXED_NO_ALSA=ON` (its MIDI out to a real DX7, from its editor) and Surge XT with `SURGE_SKIP_STANDALONE=ON` (no standalone app, so no ALSA or JACK).
- JUCE opens its X11 libraries itself, only when it shows an editor, which MPFree never asks for; they aren't needed on the MPC.

## Airwindows: one file, 89 effects

Airwindows has no CLAP of its own, and Airwindows Consolidated's CLAP picks its effect in its editor, which MPFree doesn't show. So the pack wraps the effects itself: [`tools/plugin-pack/airwindows/`](../../../tools/plugin-pack/airwindows) is MPFree's own small CLAP wrapper, built against Airwindows Consolidated's registry (airwin2rack, MIT).

- Each effect of a collection ("Recommended" by default, `-DAW_COLLECTION`) is its own CLAP effect, with id `mpfree.airwindows.<name>` (such as `mpfree.airwindows.galactic`) and vendor Airwindows. They are listed by category, then by name.
- Stereo in and out. Parameters are Airwindows' own, 0 to 1, with its value text ("70.0000 hz", "HD 24").
- Parameter changes land on their frame. A reset swaps in a fresh copy of the effect made ahead of time, so it never allocates on the audio thread; the plugin's main-thread callback makes the next one.
- Its state is the effect's name and its values; a state for another effect is refused.

## Checking a build

Two examples play the files the way the app would, without the MPC:

- `clap_check` (in `rust/mpfree-clap`) loads one file in-process and plays every plugin in it for two seconds: a chord for an instrument, a tone for an effect. It fails a plugin that can't be made or activated, fails a block, gives a sample that isn't a finite number, stays silent as an instrument, or doesn't get its state back after a parameter is moved. `build.py --play` builds it for ARM and runs it under `qemu-arm` on each file, into `PLAY.txt`.
- `rack_check` (in `rust/mpfree-plughost`) does what the app does: the rack scans a folder, and each plugin runs in its own process, as on the MPC. Under qemu-user, name the emulator as the plugin process:

```sh
cargo build --release -p mpfree-plughost --bins --example rack_check --target armv7-unknown-linux-gnueabihf
T=target/armv7-unknown-linux-gnueabihf/release
qemu-arm -L /usr/arm-linux-gnueabihf $T/examples/rack_check ~/pack/all \
  --exe qemu-arm -L /usr/arm-linux-gnueabihf $T/mpfree-plughost --
```

Times under qemu say nothing about the MPC. What a plugin costs there is for the passports (P4).

## Putting it on the stick

Copy the `.clap` files into `files/plugins/` in MPFree's data folder on the test stick, with Surge XT's `SurgeXTData/` folder beside them (Surge XT looks beside its `.clap` file, then in the folders above it) ([storage](../design/storage-spec.md)); the app scans that folder at start ([plugins in the app](../design/plugins.md#in-the-app)). The plugins keep their own files (Surge XT's `.Surge XT` folder, Dexed's cartridges) in `files/plugin-home/` beside it, which MPFree gives them as their home ([plugins in the app](../design/plugins.md#in-the-app)). Keep each plugin's `licenses/` and `source/` with the files you give anyone. This writes only to the stick, never to the MPC's eMMC. Trying them on the MPC is a device step and waits for the owner's go.

## Not yet

- **Presets.** MPFree doesn't browse a plugin's own presets yet, so Surge XT starts on its init patch and Dexed on its first built-in voice; their parameters are all on the Synth tab. The pack already carries Surge XT's factory presets. Surge's third-party patches and wavetables (most of its 380 MB data folder) and its skins (no GUI here) are left out.
- **Surge XT on 32-bit Linux** is something Surge's own build calls untested (`SURGE_BUILD_32BIT_LINUX`). The pack's checks pass, but it is the plugin to watch on the MPC.
- **On the MPC:** loading, real-time cost per voice, NaN watch and state round trips there are the passports (P4).
