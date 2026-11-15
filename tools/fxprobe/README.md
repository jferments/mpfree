# FXProbe: record the factory MPC's effects unattended

Two plugins made with Akai's [MPC plugin SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk) and a workstation script, so fxlab can measure the AIR effects on a real MPC exactly as it measures ours. Design, file formats and safety rules: [fxlab and FXProbe](../../docs/dev/design/fxlab.md).

| Path | What |
|---|---|
| `FXProbeSource/` | Instrument: plays an fxlab programme (from `FXProbe/programmes/`) on command, on a note, or on transport start |
| `FXProbeTap/` | Insert effect: records what reaches it to `FXProbe/captures/<name>-s<slot>.wav` with a report; optional timing and host-call logs |
| `common/` | The control file, status files, lock-free ring, WAV writer and slot numbering both share |
| `fxprobe.py` | The controller: `drives`, `setup`, `status`, `capture`, `pull`, `clean`, over SSH or on a local folder |
| `test/chain_sim.cpp` | A small VST2 host that runs plugins in series like an MPC track, for desktop tests |
| `test/run_chain_test.py` | The end-to-end desktop test |

**Status (2026-11-15):** both plugins pass the SDK's host simulator and validator (arm32 and arm64), and the end-to-end desktop test passes. Nothing here has run on an MPC yet.

## Build

Needs a checkout of the SDK (tested at commit `febd2ca5`) and Python 3. The SDK's build script fetches the Zig cross compiler the first time.

```sh
git clone https://github.com/inmusic-software/mpc-plugin-builder-sdk /path/to/sdk
cd /path/to/sdk
python3 tools/build.py /path/to/mpfree/tools/fxprobe/FXProbeSource
python3 tools/build.py /path/to/mpfree/tools/fxprobe/FXProbeTap
```

Each build leaves an installable folder in `tools/fxprobe/<Plugin>/build/` (`MPFree - VST - FXProbe Source`, `MPFree - VST - FXProbe Tap`; ignored by git) and runs the SDK's validator. `build.py ... --target native` alone is enough for the host simulator: `.sdk-tools/host_sim tools/fxprobe/FXProbeTap/build/native/fxprobe-tap.so test`.

## Test on a desktop

```sh
(cd rust && cargo build --release -p mpfree-fxlab)
MPC_SDK=/path/to/sdk python3 tools/fxprobe/test/run_chain_test.py
```

It builds both plugins and `chain_sim` with the system C++ compiler and checks, in about a minute: a wire capture holds the programme bit for bit at the frame `fxlab cut` finds; a capture through the SDK's StereoDelay example is complete, at the gain asked for, with timing and host-call logs and the output muted; and `fxprobe.py` setup, capture at two gains and pull read back through `fxlab analyse` as a flat wire.

## On the MPC (draft: not yet rehearsed on a device)

Needs MPC OS 3.10.1 or later with **External Plugin Support** on (Preferences → Enhancements) and developer mode on (root SSH). These steps follow the [safety rules](../../docs/agents/README.md#1-safety-comes-first); they become a runbook only after they have been rehearsed word for word on the unit, so **don't run them from this page yet**.

1. **Install the plugins** (state-changing; undo: delete the two folders). Copy both build folders into `VST2 Plugins` at the top of an SD card or USB stick, put it in the MPC and start the MPC, so the plugin list is read at boot without restarting the app over SSH. Nothing goes on internal storage.
2. **Find the drive** (read-only): `python3 tools/fxprobe/fxprobe.py --ssh MPC drives` lists mounted drives and which has `VST2 Plugins`.
3. **Make the work folder** (state-changing; undo: `fxprobe.py ... clean` deletes `FXProbe/` and nothing else): `fxlab programme --short --out /tmp/prog`, then `fxprobe.py --ssh MPC --drive DRIVE setup --programme /tmp/prog/short.wav`.
4. **Set up a project** (on the MPC, once per batch): a plugin track per slot, FXProbe Source as its instrument, the AIR effect under test in insert 1 and FXProbe Tap in insert 2, Tap's Output on Mute. Tracks created in order get slots 1, 2, 3...
5. **Check** (read-only): `fxprobe.py ... status` shows every Source and Tap as idle.
6. **Capture** (state-changing on the drive only: writes `FXProbe/control.txt` and new files in `captures/`): `fxprobe.py ... capture flanger-presets --programme-file /tmp/prog/short.wav --gains 0,-12`. One capture per gain of every slot as it is set now, checked for overruns and dropped frames.
7. **Copy back and measure** (read-only on the MPC): `fxprobe.py ... pull --out ~/fxprobe-captures`, then `fxlab analyse ~/fxprobe-captures/*-s1.wav --schedule /tmp/prog/short.tsv --out ~/fxprobe-cards`.

Captures of AIR effects are Akai's output: keep them out of this repository ([factory content policy](../../docs/dev/project/factory-content-policy.md)).

If a plugin misbehaves, the MPC app may stop; restarting the MPC recovers it, and removing the card (or the two folders) removes the plugins.

## License

The plugins use the SDK's framework headers (MIT, inMusic Brands) from the SDK checkout; nothing from the SDK is copied here. FXProbe itself is MIT.
