# FXProbe: record the factory MPC's effects unattended

Two plugins made with Akai's [MPC plugin SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk) and a workstation script, so fxlab can measure the AIR effects on a real MPC exactly as it measures ours. Design, file formats and safety rules: [fxlab and FXProbe](../../docs/dev/design/fxlab.md).

| Path | What |
|---|---|
| `FXProbeSource/` | Instrument: plays an fxlab programme (from `FXProbe/programmes/`) on command, on a note, or on transport start |
| `FXProbeTap/` | Insert effect: records what reaches it to `FXProbe/captures/<name>-s<slot>.wav` with a report; optional timing and host-call logs |
| `common/` | The control file, status files, lock-free ring, WAV writer and slot numbering both share |
| `fxprobe.sh` | The on-device tool, run on the MPC from the kit on the test stick (BusyBox sh): `check`, `install`, `remove`, `setup`, `status`, `capture`, `clean` |
| `stage.py` | Makes the kit: both plugins built with the SDK, fxlab's programmes, `fxprobe.sh`, `VERSION` and `SHA256SUMS`, in one folder for the stick |
| `fxprobe.py` | The workstation controller: `drives`, `setup`, `status`, `capture`, `pull`, `clean`, over SSH or on a local folder |
| `test/chain_sim.cpp` | A small VST2 host that runs plugins in series like an MPC track, for desktop tests |
| `test/run_chain_test.py` | The end-to-end desktop test |
| `test/test_fxprobe_sh.py` | `stage.py` and `fxprobe.sh` under BusyBox against a fake MPC and stand-in plugins (CI) |

**Status (2026-11-15):** both plugins pass the SDK's host simulator and validator (arm32 and arm64), and the end-to-end desktop test passes, including the kit installed and run as on the stick. Nothing here has run on an MPC yet.

## Build

Needs a checkout of the SDK (tested at commit `febd2ca5`) and Python 3. The SDK's build script fetches the Zig cross compiler the first time.

```sh
git clone https://github.com/inmusic-software/mpc-plugin-builder-sdk /path/to/sdk
cd /path/to/sdk
python3 tools/build.py /path/to/mpfree/tools/fxprobe/FXProbeSource
python3 tools/build.py /path/to/mpfree/tools/fxprobe/FXProbeTap
```

Each build leaves an installable folder in `tools/fxprobe/<Plugin>/build/` (`MPFree - VST - FXProbe Source`, `MPFree - VST - FXProbe Tap`; ignored by git) and runs the SDK's validator. `build.py ... --target native` alone is enough for the host simulator: `.sdk-tools/host_sim tools/fxprobe/FXProbeTap/build/native/fxprobe-tap.so test`.

### The kit for the test stick

```sh
(cd rust && cargo build --release -p mpfree-fxlab)
python3 tools/fxprobe/stage.py --sdk /path/to/sdk --out ~/mpfree-work/fxprobe-kit
```

`stage.py` builds both plugins as above (it stops if the SDK is not at `febd2ca5`), checks each `.so` is ARM of the right width, renders the short and standard programmes with fxlab and writes the kit, about 23 MB in 20 files:

```
fxprobe.sh  README.txt  VERSION  SHA256SUMS
plugins/MPFree - VST - FXProbe Source/   plugins/MPFree - VST - FXProbe Tap/
programmes/short.wav  short.tsv  standard.wav  standard.tsv
```

The plugins are in `plugins/`, not `VST2 Plugins/`, because Akai's system mounts every partition of a USB stick noexec, and the kernel refuses to load a library from a noexec mount. `fxprobe.sh install` copies them to the MPC's internal drive instead. The stick build puts the kit at `/fxprobe` on the stick's kits partition (partition 3), and the work folder goes on its data partition (partition 4).

## Test on a desktop

```sh
(cd rust && cargo build --release -p mpfree-fxlab)
MPC_SDK=/path/to/sdk python3 tools/fxprobe/test/run_chain_test.py
```

It builds both plugins and `chain_sim` with the system C++ compiler and checks, in about a minute: a wire capture holds the programme bit for bit at the frame `fxlab cut` finds; a capture through the SDK's StereoDelay example is complete, at the gain asked for, with timing and host-call logs and the output muted; `fxprobe.py` setup, capture at two gains and pull read back through `fxlab analyse` as a flat wire; and the stick route: a kit from `stage.py` (native builds), `fxprobe.sh install` to a fake internal drive, the installed plugins finding the work folder on a fake data partition through their pointer, captures that read back as a flat wire, the stick coming back under another mount name without the old command running again, and `remove` leaving the drive as it was.

`test/test_fxprobe_sh.py` (no SDK or Rust; CI runs it with `MPFREE_REQUIRE_BUSYBOX=1`) runs `stage.py` with stand-in builds and `fxprobe.sh` under BusyBox applets only, against a fake MPC root and stand-in plugin instances: install, reinstall and remove, the refusals (noexec, not mounted, someone else's folder, a damaged kit), setup, capture at two gains, a stale done status not ending a wait, a capture with an overrun, and clean.

## On the MPC, from the test stick (draft: not yet rehearsed on a device)

Needs MPC OS 3.10.1 or later with **External Plugin Support** on (Preferences → Enhancements) and developer mode on (root SSH), and the test stick with the kit. These steps follow the [safety rules](../../docs/agents/README.md#1-safety-comes-first); they become a runbook only after they have been rehearsed word for word on the unit, so **don't run them from this page yet**. Every command runs in the developer-mode shell with Akai's app up and the stick in (put in once the app is up: with MPFree's boot hook installed, switching on with the stick in starts MPFree instead). `/media/*/fxprobe/fxprobe.sh` is the kit, whatever name Akai's system mounted the kits partition under.

1. **Check** (read-only): `sh /media/*/fxprobe/fxprobe.sh check` checks the kit against its `SHA256SUMS` and shows the kits and data partitions, the internal drive's mount options, whether the plugins are installed and the same as the kit's, and the work folder.
2. **Install** (state-changing, the internal drive): `sh /media/*/fxprobe/fxprobe.sh install` copies the two plugin folders (about 1.1 MB) into `/media/az01-internal/VST2 Plugins`, the MPC's internal drive, with a pointer file in each. It refuses a drive that is not a read-write mount or is noexec, and a folder of the same name that isn't FXProbe's. It copies to `.fxprobe-staging` at the drive's top first and checks the copy against the kit's hashes, then moves each folder into place, so the app never sees half a plugin. Undo: `remove`. If it fails before the moves, nothing in `VST2 Plugins` changed and the staging folder is gone; between the two moves, run `install` again, or `remove`.
3. **Restart the app** with the stick out: switch the MPC off, take the stick out, switch it on. The app lists plugins when it starts. Put the stick in once the app is up.
4. **Make the work folder** (state-changing, the stick only): `sh /media/*/fxprobe/fxprobe.sh setup` makes `FXProbe/` on the stick's data partition (found as the partition after the kit's, whatever it is mounted as), copies the programmes into it and writes the marker the plugins look for. It needs MPFree's `FORMAT` file on that partition, so MPFree must have started from this stick once: MPFree's store refuses a data partition that holds other files and no `FORMAT`. Undo: `clean`.
5. **Set up a project** (on the MPC, once per batch): a plugin track per slot, FXProbe Source as its instrument, the AIR effect under test in insert 1 and FXProbe Tap in insert 2, Tap's Output on Mute. Tracks created in order get slots 1, 2, 3...
6. **Check the instances** (read-only): `sh /media/*/fxprobe/fxprobe.sh status` shows every Source and Tap as idle.
7. **Capture** (state-changing, the stick only: `control.txt` and new files in `captures/`): `sh /media/*/fxprobe/fxprobe.sh capture flanger-presets --gains 0,-12` makes one capture per gain of every slot as it is set now (`--programme standard` for the long programme, `--slots 1,2`, `--tail SECONDS`, `--timing`). It waits for each, checks every report for overruns, dropped frames and non-finite samples, and ends with `CAPTURE-OK`.
8. **Read the captures** (read-only on the MPC): switch the MPC off, take the stick out, and on the workstation read `FXProbe/captures/` on its data partition: `fxlab analyse CAPTURE.wav --schedule FXProbe/programmes/short.tsv --gain 0 --out ~/fxprobe-cards`.
9. **Tidy up** (state-changing): `clean` deletes `FXProbe/` from the stick, captures included, so copy them off first. `remove` deletes exactly the two plugin folders from the internal drive (and `VST2 Plugins` if `install` made it and it is empty).

Each mode's last line is `CHECK-OK`, `INSTALL-OK`, `REMOVE-OK`, `SETUP-OK`, `CAPTURE-OK` or `CLEAN-OK` when it did all it should; a line starting `STOP:` says why it stopped.

How the plugins find the work folder: `install` writes `workfolder.txt` into each installed plugin folder, holding `/media/*/FXProbe`. A plugin with a pointer that has a `*` uses the one folder that matches it and holds the marker `.fxprobe-work`; none, or more than one, and it stays idle. It looks again at most once a second while there is none, so the stick can go in after the plugins load and come back under another mount name; a command already in `control.txt` when the folder appears is ignored.

### Over SSH from a workstation

`fxprobe.py` does the same from a workstation for plugins on a drive the app can load them from, with the work folder beside them (`FXProbe/` next to that drive's `VST2 Plugins`): `fxprobe.py --ssh MPC drives`, then `--drive DRIVE setup --programme short.wav`, `status`, `capture NAME --programme-file short.wav --gains 0,-12`, `pull --out DIR` and `clean`. On the factory MPC, USB and SD drives are mounted noexec, so this route fits the internal drive only, and puts the captures there too; the stick route above keeps them on the stick.

Captures of AIR effects are Akai's output: keep them out of this repository ([factory content policy](../../docs/dev/project/factory-content-policy.md)).

If a plugin misbehaves, the MPC app may stop; restarting the MPC recovers it, and `fxprobe.sh remove` (or deleting the two folders from `VST2 Plugins` on the internal drive) removes the plugins.

## License

The plugins use the SDK's framework headers (MIT, inMusic Brands) from the SDK checkout; nothing from the SDK is copied here. FXProbe itself is MIT.
