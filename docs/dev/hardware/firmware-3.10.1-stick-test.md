# Trying firmware 3.10.1 from the USB stick before installing it

**Status:** runbook, 2026-11-13. Phase 1 is ready to run. Phase 2 uses the tools live test 3 used and runs only after the phase 1 results have been read. Phase 3 needs a stick builder option that is not written yet. **Scope:** the reference MPC One (Gen 1) on 3.6.0 with MOD-001 and MOD-002.

The owner wants to move MPFree's way in from our own persistent SSH ([MOD-001/002](archive/2026-11/device-modifications.md)) to the developer mode Akai added in 3.10.1 ([compatibility page](firmware-3.10.1-compatibility.md)), and to check first that 3.10.1 does everything we need. Installing it is the one step that cannot be undone safely: it rewrites the rootfs, may rewrite the bootloader partitions, and a downgrade to 3.6.0 has not been checked. So 3.10.1 is tried the way 3.10.0 was ([kexec test boot](archive/2026-11/kexec-test-boot.md)): 3.6.0 starts it by kexec, its root filesystem comes from the USB stick, and the eMMC and SD card are hidden from it. A power-cycle brings 3.6.0 back.

## What each phase answers

| Phase | Where it runs | Question | Changes |
|---|---|---|---|
| 1. Offline survey | Workstation only | What does 3.10.1 ship for developer mode (SSH server, its login rules, the script runner)? Does it ship any peripheral firmware the MPC's chips don't already have? Is the kernel the one we already boot? | New folders under `~/mpfree-work` and `~/mpfree-results` only |
| 2. Stick test, Akai's app off | MPC RAM, from the stick | Does 3.10.1's userspace boot from the stick with our kexec, and does the pad player run on it? Does the changed `az01-signed-fs` get in the way? | The stick (erased and rewritten, partition 4 kept); nothing on the MPC that survives a power-cycle |
| 3. Stick test, Akai's app on | MPC RAM, from the stick | Does developer mode open SSH and run `autorun.sh` the way the tools need: port, user, password rules, host key, which drives, as which user, when? | As phase 2. **Not ready:** needs a builder option, and phase 1 must pass first |

What the stick cannot answer: whether developer mode survives a reboot of an installed 3.10.1, whether 3.10.1's kernel can kexec into MPFree (the stick turns kexec off in the test build by design; the offline comparison found the 3.10.1 kernel unchanged from the 3.10.0 one we already boot into), and anything about the installer itself.

## Phase 1: offline survey (workstation only)

Everything here only reads Akai's image and the verified eMMC backup, and writes new folders under `~/mpfree-work` and `~/mpfree-results`. Nothing contacts the MPC.

**Get the image.** From [Akai's MPC firmware page](https://www.akaipro.com/downloads-and-support/downloads/firmware/mpc/), save the 3.10.1 update image for the first-generation standalones (the 3.10.0 one was `MPC-3.10.0-Gen1-update.img`) into `~/mpfree-work/akai-update-files/`. The block below stops unless exactly one file there has both `3.10.1` and `Gen1` in its name.

**Block 1.1: list and unpack the image** (read-only on the image; writes a new `akai-unpacked-<UTC>-*` folder):

```bash
cd /home/jesse/code/mpfree-hardware-survey && N="$(find ~/mpfree-work/akai-update-files -maxdepth 1 -type f -name '*3.10.1*Gen1*.img' | wc -l)" && { [ "$N" -eq 1 ] || { echo "STOP: expected one *3.10.1*Gen1*.img in ~/mpfree-work/akai-update-files, found $N"; false; }; } && IMG="$(find ~/mpfree-work/akai-update-files -maxdepth 1 -type f -name '*3.10.1*Gen1*.img')" && echo "image: $IMG" && python3 tools/mpc-one/results.py run fw3101-list -- python3 tools/mpc-one/offline-update-image.py "$IMG" && python3 tools/mpc-one/results.py run fw3101-extract -- python3 tools/mpc-one/offline-update-extract.py "$IMG"
```

The listing shows which partitions the update writes (3.10.0 had only a rootfs record). The unpack checks the image's own hashes and writes `rootfs.img` only if they match.

**Block 1.2: survey both root filesystems and compare their firmware** (read-only; the backup survey copies the 3.6.0 rootfs out of the backup into its work folder, checks it against the boot map, and deletes the copy at the end):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fw3101-survey -- bash tools/mpc-one/offline-rootfs-survey.sh && python3 tools/mpc-one/results.py run fw360-survey -- bash tools/mpc-one/offline-rootfs-survey.sh --from-backup && python3 tools/mpc-one/results.py run fw-compare -- python3 tools/mpc-one/offline-firmware-compare.py ; python3 tools/mpc-one/results.py zip
```

- The first survey takes the newest unpacked image, which is the one block 1.1 just made: its `source:` line names that folder. Section 9 shows the developer-mode pieces (SSH and script-runner units, `sshd_config`, whether root's password field is empty, and which files mention `autorun.sh`). Section 8 copies `/boot` to `~/mpfree-work/boot-files/<release>/`; if it prints `WARNING: ... exists with different files`, 3.10.1's kernel or device trees differ from the copy already there, and phase 2 waits.
- [`offline-firmware-compare.py`](../../../tools/mpc-one/offline-firmware-compare.py) compares the two surveys' section 10, file by file: the touch firmware, the pad and button controller's firmware in `/usr/share/Akai/SME0/Firmware`, and anything else under the firmware folders. `VERDICT: SAME` means 3.10.1's updaters have nothing new to write to those chips. `VERDICT: STOP` lists what differs. The `;` before `zip` packs the results even when the comparison stops.
- Upload the one zip it prints. The reports quote Akai's files: upload them to this project only.

**What happens next.** The phase 1 reports are read before phase 2 is offered: in particular any 3.10.1 unit that the stick builder does not yet mask and that could write to a chip, and section 9. If the comparison says STOP, phase 3 is off until each difference is understood; phase 2 can still run, because the stick keeps Akai's app and every flashing program off.

**One side effect.** After block 1.1 the newest unpacked image is 3.10.1, so `offline-kexec-stick.py` builds 3.10.1 sticks by default from then on, for every thread. To build a 3.10.0 stick again, pass `--image` with the 3.10.0 folder's `rootfs.img`.

## Phase 2: stick test with Akai's app off

This is live test 3's boot with 3.10.1's root filesystem in place of 3.10.0's. The tools, their checks and their undo are the ones in the [kexec test boot](archive/2026-11/kexec-test-boot.md#tools-in-order) and [the app guide](../guides/mpfree-app.md#running-it); nothing in them changes for 3.10.1. The exact blocks are given in the thread with the clone's commit once phase 1 has been read, and rehearsed word for word first.

In order: build `mpfree-screen-log`, `mpfree-startup` and `mpfree` for ARM; `offline-kexec-stick.py --splash … --app …` (it takes the newest unpacked image, 3.10.1); `write-test-stick.py` (**erases the stick's partitions 1 to 3**, keeps partition 4, asks for `WRITE`); `collect-over-ssh.sh --boot-map`; `MPFREE_ACCEPT_KEXEC_BOOT=YES collect-over-ssh.sh --kexec-boot` (**state-changing, RAM only**: you type `BOOT 3.10`); play the pad player for a few minutes; switch off, take the stick out, wait 10 s, switch on; `--boot-map` again; `read-stick-logs.py` with the stick in the workstation; `results.py zip`.

**Passes when:** the kexec boot ends with `RESULT: the test build is running from the stick, and it cannot see the eMMC or SD card.`; the pad player starts and plays; the stick logs show no failed unit we didn't expect and no `mpfree-test: blocked` call we can't explain; and the boot maps before and after match, with 0 sectors written to p1 to p6 and boot0/1.

**Undo:** switch off, take the stick out, wait 10 s, switch on: U-Boot boots 3.6.0 from the eMMC as before. The stick's old partitions 1 to 3 are gone (rebuild a 3.10.0 stick to go back); partition 4, the pad player's saved settings, stays.

## Phase 3: developer mode on the stick (not ready)

Developer mode is switched on from inside Akai's app (hold SHIFT + MENU, choose Developer; Akai's [release notes](https://cdn.inmusicbrands.com/Software/MPC/MOPN4PCOPEN/MPC%203.10.1%20-%20Release%20Notes.pdf)), so this phase needs Akai's app running on the test build. Today's sticks mask it, and stub its launcher, because it can flash the pad and button controller ([updater findings](archive/2026-11/akai-updater-offline.md)).

The plan, to be built only after phase 1:

- A builder option that leaves `acvs.service` and its launcher in place, keeps every other mask and stub (`dfu-util`, `midifirmup`, `touch-fw-upd`, `xmosdfu`, `fw_setenv`, `az01-update`, `kexec` and the rest), keeps the eMMC and SD hidden and `/data` in RAM. Whether the script runner's unit is unmasked depends on what section 9 shows.
- It runs only if phase 1's comparison said SAME, so even if a blocked flashing program were reached some other way, it would have nothing new to write.
- On the MPC: hold SHIFT + MENU, choose Developer. From the workstation, a read-only check records the SSH port, the login it accepts, and the host key's fingerprint. A harmless `autorun.sh` that only writes one file onto the stick shows when the script runner starts it, as which user, and from which drive. Akai's notes say the SSH server is open to the network while developer mode is on, and a community FAQ says it takes root with no password ([Open MPC FAQ](https://open-mpc.github.io/faq.html)), so the MPC stays on the direct Ethernet cable only.

## After the test

If 3.10.1 passes, installing it is a separate decision with its own gate: a fresh full eMMC backup, Akai's updater on mains power, then deleting MOD-002 by hand (the update removes MOD-001). The host tools then get a developer-mode login in place of port 2222, and the kexec jump stays as it is. A later step can start MPFree at power-on without Akai's app: a start-up unit in Akai's Linux that, before the app starts, checks the stick and jumps to it. That is a lasting change to Akai's files, like MOD-001, and needs its own go.
