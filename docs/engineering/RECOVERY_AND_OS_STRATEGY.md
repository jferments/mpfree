# Recovery tiers, firmware updates and the path to an MPFree OS

**Status:** proposed plan, 2026-11-09. Owner direction is recorded below. The
tiered gate replaces the single "independent unbootable-device recovery"
blocker in [REVERSIBLE_HANDOFF.md](REVERSIBLE_HANDOFF.md) and
[HARDWARE_OWNERSHIP_STRATEGY.md](HARDWARE_OWNERSHIP_STRATEGY.md) **once the owner
adopts it**. Until then, those documents' gates still apply.

## Owner direction (Jay, 2026-11-09)

1. **Keep persistent SSH.** It is the primary development path, and it is recorded in
   [DEVICE_MODIFICATIONS.md](../hardware/DEVICE_MODIFICATIONS.md).
2. **Product goal: one-click USB install.** The user inserts a USB stick in the
   back of the MPC. It installs the MPFree OS/userspace and sets up an SSH
   server the user can log into over Ethernet.
3. **Long term: our own Linux distribution**, so MPFree can update the
   underlying OS and control the device fully. This must be done with
   **certainty that it cannot brick the device**. That requires understanding the
   existing bootloader well enough to boot and test another OS **without
   damaging the existing one**.
4. **Keep developing the custom userspace on the stock OS** in parallel.

## Two tracks

| | Track A: MPFree userspace on stock az0x | Track B: MPFree Linux distribution |
| --- | --- | --- |
| Kernel/drivers | Akai's 6.6 PREEMPT_RT kernel | Ours (start from vendor config/DT, then decide) |
| Boot chain | Untouched | Must be understood first; tested without eMMC writes |
| Delivery | USB autorun → install to SD/`/data` → SSH | USB/SD image, eventually an installer |
| Gate tier | Tier 1 → Tier 2 | Tier 3 |
| Next step | Tier 1 session handoff | Read-only `--boot-map` survey |

Track A delivers a usable instrument early. Track B starts as read-only
research and runs alongside it.

## Recovery tiers

A gate should match what an experiment can actually damage.

| Tier | Scope | What it unlocks | Recovery path | Gate (all must hold) |
| --- | --- | --- | --- | --- |
| 1 | Session only: RAM-resident MPFree, `systemctl stop acvs.service` for one run, no persistent writes | Phase 1: own display, pads, audio | Supervisor restarts `acvs`, else power-cycle | Full off-device eMMC backup taken; per-partition write counters show zero writes to `p1`–`p6` across the session; stub stop → start rehearsal passes (no MPFree code) |
| 2 | Persistent MPFree files on SD/USB ext4 or a dedicated `/data/mpfree` directory; opt-in launch | Installed MPFree, saved projects, the one-click installer | Remove media; Akai Update Mode reinstalls `p6`; restore `/data` from backup | Tier 1, plus: Update Mode rehearsed once (reinstall 3.6.0 over itself); `/data` restore rehearsed; update scripts read offline |
| 3 | Anything on `p1`–`p6`, `boot0/1`, U-Boot env, or another kernel | MPFree distribution | RK3288 MaskROM USB reflash from the backup | Tier 2, plus: MaskROM entry found on this board; full eMMC **read-back** over MaskROM matches the SSH backup |

Tier 1 does not need MaskROM because nothing on the eMMC changes, and the
kernel's write counters (`/sys/class/block/mmcblk0pN/stat`, field 7 =
sectors written) let us *measure* that instead of assuming it. The other
hard prerequisites in REVERSIBLE_HANDOFF.md (allowlist, consent, quiescence,
independent supervisor, tested fake restoration) still apply in full.

**Note:** MOD-001 (persistent SSH in rootfs) was a Tier 3 write made before
this plan existed. See DEVICE_MODIFICATIONS.md.

## Recovery paths by failure layer

| Damaged | Recovery | Depends on | Proven? |
| --- | --- | --- | --- |
| Running session | Restart `acvs` / power-cycle | Nothing persistent changed | No |
| `/data` (`p8`) | Restore from off-device backup; factory reset as last resort | Booting `p6` | No |
| `p6` rootfs | Akai Update Mode: FULL LEVEL + BROWSE + PAD BANK C/G at power-on ([Akai walkthrough](https://support.akaipro.com/en/support/solutions/articles/69000816160-akai-pro-mpc-series-firmware-update-walkthrough)) | **Probably** `p1`–`p3` **and** `p6` (see inference below) | No |
| `p1`–`p5`, `boot0/1` | RK3288 MaskROM + `rkdeveloptool` writing the backup image back | USB-B port reaching the RK3288 OTG controller; a way to force MaskROM | No |

**Confirmed by `--boot-map` (2026-11-09):** the kernel, DTBs and U-Boot's boot script live in `p6:/boot`, and `boot0/boot1` are empty, so the device boots from the user-area loader at sector 64. See [boot map](../hardware/MPC_ONE_3_6_0_BOOT_MAP_20261109.md). **Still to verify:** whether Update Mode also boots from `p6`. If so, Akai's Update Mode
can reinstall a rootfs but **cannot** revive a unit whose `p6` is unreadable,
and MaskROM is the only true brick recovery. `--boot-map` checks this.

## Testing another OS without touching the existing one

These options boot a non-Akai kernel while leaving every eMMC byte
untouched. A power-cycle returns the stock OS. All three still need to be checked on this
unit, in order of increasing effort:

1. **kexec from the running stock Linux. Available:** the vendor kernel has
   `CONFIG_KEXEC=y` and `CONFIG_KEXEC_FILE=y`, and `/usr/sbin/kexec` is installed. It can load our kernel and initramfs into
   RAM and jump to it. There are no eMMC writes, and a reboot returns to stock. Risk: drivers left
   in odd states by the first kernel. `--boot-map` reports kexec support.
2. **U-Boot scanning removable media.** If the vendor U-Boot environment
   (`p2`) already checks SD/USB for `extlinux.conf` or `boot.scr` before eMMC
   (generic distro boot), an SD card alone boots our OS. `--boot-map` prints
   the environment as text.
3. **MaskROM RAM boot.** `rkdeveloptool db` loads a Rockchip loader into RAM
   over USB without writing flash. From there a RAM-loaded U-Boot can boot
   our kernel. This is also the Tier 3 recovery path, so proving it pays twice.

Only when one of these works do we consider any persistent boot change, and
then only under the Tier 3 gate: for example an A/B arrangement that always falls back
to the untouched Akai slot.

## What an official Akai firmware update does

| Area | Rewritten? | Effect |
| --- | --- | --- |
| `p6` rootfs (kernel, DT, glibc, `acvs.service`) | Yes, almost certainly | Removes MOD-001 (persistent SSH). New ABI/units: MPFree must re-qualify. Our `p6` backup becomes the older version |
| `p1`–`p3` bootloader, `p2` env | Unknown | An interrupted flash here is the main brick risk of an update; recovery would be MaskROM |
| `p4`/`p5` splash | Possibly | None |
| `p7` content | Expansion installs only | None |
| `p8` data, `/etc`/`/var` overlays | Probably kept, possibly pruned | Tier 2 files on `/data` could vanish; stale overlay files could clash |
| SD card / USB | No | Anything MPFree keeps there survives |

Risks for MPFree: USB autorun can be patched out (it is the bootstrap for the
one-click installer, so this matters); glibc/library changes (prefer static
or musl ARMv7 builds); service-layout changes (re-run `--handoff-preflight`);
kernel interface changes (ACVA, ILI2117, DRM); a possible future downgrade
lock.

Policy:
- **Reference unit stays on 3.6.0** with updates off until the new version's
  update scripts have been read and the read-only surveys re-run.
- **Every MPFree release carries a firmware allowlist** (version, kernel
  string, rootfs/unit hashes). On anything else it refuses and leaves stock
  running.
- **Persistent MPFree files live on SD/USB or `/data/mpfree`**, never in `p6`
  or the `/etc` overlay, so updates can't break them and removing the media
  gives back a stock MPC.
- Keep private copies of every official update image MPFree is tested
  against. Never commit them.

## Next steps

Steps 1–5 are read-only or offline. Step 6 changes device state and needs the owner's go-ahead.

1. ~~Run `--boot-map`~~ **Done 2026-11-09** ([boot map](../hardware/MPC_ONE_3_6_0_BOOT_MAP_20261109.md)). Re-run the revised version to capture the U-Boot environment and `boot.scr` text that the first run missed.
2. Copy `/opt/ramfs-update.squashfs` to the workstation (a read on the device), `unsquashfs` it there,
   and do the same with Akai's public 3.6.0 update file. Document which partitions an update writes
   and whether it checks `p6` before writing. Commit findings, never binaries.
3. Full eMMC + `boot0/boot1` backup streamed over SSH, with per-partition SHA-256 and two private copies.
   Label it `MOD-001+MOD-002`.
4. Host-side write-counter diff tool: fails if `p1`–`p6` show writes between two `--boot-map` snapshots.
5. Board photos to locate a MaskROM button or eMMC clock test point. Opening the case is the owner's decision.
6. **Owner go-ahead required:** first Tier 1 rehearsal. Stop and start `acvs.service` with no MPFree
   code, confirm UI, pads and audio come back, and confirm zero writes.
