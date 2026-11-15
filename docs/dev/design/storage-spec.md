# MPFree storage: where settings and user files are saved

**Status:** APPROVED by Jay on 2026-11-11 at 09:30Z ("Yes, kept across rounds"). Built in round 7; until a stick with partition 4 is written, nothing is saved.
**Owner:** the app thread (it was the Hello MPC thread), which owns the app shell. The touch thread (calibration) and the arp thread (patterns) use this; nothing else picks its own place.

## Where (rule zero)

- **On the test stick, never on the MPC.** The eMMC (p1–p6, the U-Boot environment, the rootfs) and the SD card are never written. On the test boot, the eMMC and SD controllers are disabled in the device tree anyway.
- **Root file system:** the stick's partition 1 stays read-only, and /data is RAM.
- **A new partition 4, "MPFREE-DATA":** ext4 with a journal, built by `offline-kexec-stick.py` at 1 GiB. It fills the rest of the stick (plan decision 1, approved 2026-11-12): `stick.img` holds a 64 MiB one, `write-test-stick.py` makes the partition run to the end of the stick, and the filesystem grows to fill it at boot ([how](../hardware/archive/2026-11/kexec-test-boot.md#saved-settings-partition-4)).
  - `start-app.sh` mounts it read-write at `/run/mpfree-data` (nosuid, nodev, noexec) and passes `--data /run/mpfree-data` to the pad player.
  - If the mount fails, the app runs without saving and logs why.
- **Kept across rounds:** `write-test-stick.py` leaves partition 4 where it is when the stick already has it at 1 GiB, and rewrites only partitions 1 to 3 and the partition table. A stick with the old 64 MiB layout has its partition 4 saved to the build folder first, then moved. `--fresh-data` starts empty; `--data-from FILE` writes a saved copy.
- **Undo:** rewrite the stick with `--fresh-data`, write a saved copy back with `--data-from`, or delete files on partition 4 from the workstation.
- **If a write is cut off:** each file is written whole to `NAME.part`, synced, then renamed over the old one. A cut-off write leaves the old file.

## Layout of the partition

```
/run/mpfree-data/
  FORMAT                 "mpfree-data 1"
  settings/<owner>.conf  key = value lines, one file per owner: app, touch, arp, synth, sampler, looper ...
  files/<owner>/<name>   whole files: arp/patterns.txt, touch/calibration.txt ...
```

## API: crate `mpfree-store`

- `Store::at(dir)` on the MPC; `Store::none()` in tests and previews (nothing is written).
- `store.settings(owner) -> Settings`: ordered `key = value`, with `get_*` / `set_*` helpers. `store.save_settings(owner, &settings)`.
- `store.read(owner, name) -> Option<Vec<u8>>`, `store.write(owner, name, bytes)`.
- `store.list(owner) -> Vec<String>`: the names of the owner's plain files, sorted (read only; at most `MAX_LIST`).
- **Rules:**
  - Owner and file names are `[a-z0-9._-]` only, so there are no paths and no `..`.
  - 1 MiB per file, and the partition's size caps the total.
  - Reads happen at start only.
  - Saves never happen on the audio thread. They go to one `mpfree-store` writer thread, which logs `saved: arp/patterns.txt (812 bytes)` or why it failed.
- **Static test:** `mpfree-store` and `mpfree-library` are the only code that writes files, each only under the folder it was given, and each write site is listed in `tools/mpc-one/tests/test_app_mpc.py`. `start-app.sh` is the only place that names that folder.

## What is saved (first version)

- **Prefs:** pad velocity curve, screen pad velocity, latch mode, audio buffer, show developer tools, theme (Appearance), sound cores, loop memory and At start (`at-start = new | last`). With them in the "app" settings, the system's other settings (Jay, 13 Nov: the system's, not the project's): the metronome (`metronome` and `metronome-level`, from SHIFT + PLAY START and the Play page's options, live test 3 fixes), the Looper's level (`loop-level`, whole dB) and the line input's latency once set by hand (`loop-latency`, frames). Line input is never saved and is Off at every start, so what the Looper records from isn't kept either (it follows the line input).
- **The buttons found with Map controls and the guide,** so the MPC is mapped once.
- **Touch tuning** (the touch thread, #145): `settings/touch.conf`, written by Save on Dev > Touch. Keys are `mpfree_touch::tuning::Param::key` (offset-x, offset-y, scale-x, scale-y, hold-filter-px, tap-slop-px, tap-max-ms, long-press-ms, swipe-min-px, swipe-min-speed). Each value is clamped to its bounds when read, so a damaged file can't make touch unusable ([M17](../milestones/m17-touch-tuning.md)).
- **MIDI settings** (the app): `files/midi/rig.txt`, each port's input and thru, kept for every project. The song's MIDI settings are the project's `midi` part, saved with the project (docs/dev/design/midi-io.md). A `files/midi/project.txt` left by an older build is no longer read.
- **Effect picker favourites and recents** (the app, from the FX coverage work): `files/fx/favourites.txt` and `files/fx/recent.txt`, one effect id per line (favourites in the order they were starred, at most 64; recent effects newest first, at most 12), written when a star or an added effect changes them. Ids this build doesn't know (a plugin that is missing today) are kept. Kept for every project, not saved with one ([fx.md](fx.md#the-effect-picker)).
- **Arp patterns** (the arp thread): `files/arp/patterns.txt`, the patterns and custom scales Save keeps. `--save-patterns` is gone; `--patterns FILE` only reads a library for one start.
- **Themes folder** (the themes thread): `files/themes/NAME.theme`, theme files the user adds; the app only reads them and offers them in Prefs > Appearance. The picked theme is `theme` in the app settings.
- **Synth tab options** are the project's since 13 Nov (Jay: a song's key and tuning are part of the song), in the project's `synth` part ([browser.md](browser.md#the-parts)), with the same words: `engine` (poly, fm, drums, pluck, wavetable), `scale` (off or the scale's name, built in or the owner's), `scale-root` (C to B), `in-key-rows` (sequence, fourths), `bank-a` to `bank-h` (chromatic, in-key, chords), `tuning` (12-tet, just, pythagorean, meantone, 24-tet, or `file NAME`), `tuning-root` and `a4` (380 to 480). An open puts them back over the defaults (`App::open_synth`); a word not understood, or a scale or tuning file that is gone, is left out with a log line and that option is its default. The bank playing and the octave are not kept. An older `settings/synth.conf` is no longer read.
- **Plugins' own files** (the CLAP thread; place agreed with this spec's owner, 14 Nov): each plugin process runs with its home and XDG folders in `files/plugin-home/`, so what a plugin keeps for itself (Surge XT's user folder, Dexed's cartridges) goes there; the app writes nothing there itself. The store makes the folder (`Store::dir`). Kept for every project, not saved with one. It has no size cap, so a plugin that writes a lot fills partition 4. To reset every plugin's own files, delete `files/plugin-home/` from the workstation. Without partition 4, the plugins' home is a folder of that run's own in the system's temp folder, never the real home.
- **Tunings folder** (the synth thread): `files/tunings/NAME.scl`, Scala scales the user adds, each with an optional keyboard map `NAME.kbm`; the app only reads them, at start, and offers them on the Synth tab's Tuning row. The pick is kept with the Synth tab's options (in the project), by file name.
