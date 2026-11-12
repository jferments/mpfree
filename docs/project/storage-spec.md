# MPFree storage: where settings and user files are saved

**Status:** APPROVED by Jay on 2026-11-11 at 09:30Z ("Yes, kept across rounds"). Built in round 7; until a stick with partition 4 is written, nothing is saved.
**Owner:** the Hello MPC thread, which owns the app shell. The touch thread (calibration) and the arp thread (patterns) use this; nothing else picks its own place.

## Where (rule zero)

- **On the test stick, never on the MPC.** The eMMC (p1–p6, the U-Boot environment, the rootfs) and the SD card are never written. On the test boot, the eMMC and SD controllers are disabled in the device tree anyway.
- **Root file system:** the stick's partition 1 stays read-only, and /data is RAM.
- **A new partition 4, "MPFREE-DATA":** ext4 with a journal, built by `offline-kexec-stick.py` at 1 GiB. It fills the rest of the stick (plan decision 1, approved 2026-11-12): `stick.img` holds a 64 MiB one, `write-test-stick.py` makes the partition run to the end of the stick, and the filesystem grows to fill it at boot ([how](../hardware/kexec-test-boot.md#saved-settings-partition-4)).
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
- **Static test:** `mpfree-store` is the only code that writes files, and only under the folder it was given. `start-app.sh` is the only place that names that folder.

## What is saved (first version)

- **Prefs:** pad velocity curve, screen pad velocity, latch mode, audio buffer, show developer tools, theme (Appearance). Line input is never saved and is Off at every start.
- **The buttons found with Map controls and the guide,** so the MPC is mapped once.
- **Touch tuning** (the touch thread, #145): `settings/touch.conf`, written by Save on Dev > Touch. Keys are `mpfree_touch::tuning::Param::key` (offset-x, offset-y, scale-x, scale-y, hold-filter-px, tap-slop-px, tap-max-ms, long-press-ms, swipe-min-px, swipe-min-speed). Each value is clamped to its bounds when read, so a damaged file can't make touch unusable ([M17](../milestones/m17-touch-tuning.md)).
- **Arp patterns** (the arp thread): `files/arp/patterns.txt`, the patterns and custom scales Save keeps. `--save-patterns` is gone; `--patterns FILE` only reads a library for one start.
- **Themes folder** (the themes thread): `files/themes/NAME.theme`, theme files the user adds; the app only reads them and offers them in Prefs > Appearance. The picked theme is `theme` in the app settings.
