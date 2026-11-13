# Trying firmware 3.10.1 from the USB stick before installing it

**Status:** runbook, 2026-11-13. Phases 1 and 2 are done (results below): the firmware matches, developer mode's files have been read, and 3.10.1 runs the pad player from the stick with the eMMC untouched. Phase 3's tools are ready and its blocks are below; running it waits for the owner's go, since it is the first time Akai's app runs on the test build. **Scope:** the reference MPC One (Gen 1) on 3.6.0 with MOD-001 and MOD-002.

The owner wants to move MPFree's way in from our own persistent SSH ([MOD-001/002](archive/2026-11/device-modifications.md)) to the developer mode Akai added in 3.10.1 ([compatibility page](firmware-3.10.1-compatibility.md)), and to check first that 3.10.1 does everything we need. Installing it is the one step that cannot be undone safely: it rewrites the rootfs, may rewrite the bootloader partitions, and a downgrade to 3.6.0 has not been checked. So 3.10.1 is tried the way 3.10.0 was ([kexec test boot](archive/2026-11/kexec-test-boot.md)): 3.6.0 starts it by kexec, its root filesystem comes from the USB stick, and the eMMC and SD card are hidden from it. A power-cycle brings 3.6.0 back.

## What each phase answers

| Phase | Where it runs | Question | Changes |
|---|---|---|---|
| 1. Offline survey | Workstation only | What does 3.10.1 ship for developer mode (SSH server, its login rules, the script runner)? Does it ship any peripheral firmware the MPC's chips don't already have? Is the kernel the one we already boot? | New folders under `~/mpfree-work` and `~/mpfree-results` only |
| 2. Stick test, Akai's app off | MPC RAM, from the stick | Does 3.10.1's userspace boot from the stick with our kexec, and does the pad player run on it? Does the changed `az01-signed-fs` get in the way? | The stick (erased and rewritten, partition 4 kept); nothing on the MPC that survives a power-cycle |
| 3. Stick test, Akai's app on | MPC RAM, from the stick | Does developer mode open SSH and run `autorun.sh` the way the tools need: port, user, password rules, host key, which drives, as which user, when? Does Akai's lock-down to a key work? | As phase 2, plus a spare USB drive for the autorun test (optional) |

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

- The first survey takes the newest unpacked image, which is the one block 1.1 just made: its `source:` line names that folder. Section 9 shows the developer-mode pieces: SSH and script-runner units, `sshd_config`, whether root's password field is empty, which files mention `autorun.sh`, everything in `/usr/lib/az0x/user-access`, and what Akai's app says about developer mode. Section 8 copies `/boot` to `~/mpfree-work/boot-files/<release>/`; if it prints `WARNING: ... exists with different files`, 3.10.1's kernel or device trees differ from the copy already there, and phase 2 waits.
- [`offline-firmware-compare.py`](../../../tools/mpc-one/offline-firmware-compare.py) compares the two surveys' section 10, file by file: the touch firmware, the pad and button controller's firmware in `/usr/share/Akai/SME0/Firmware`, and anything else under the firmware folders. `VERDICT: SAME` means 3.10.1's updaters have nothing new to write to those chips. `VERDICT: STOP` lists what differs. The `;` before `zip` packs the results even when the comparison stops.
- Upload the one zip it prints. The reports quote Akai's files: upload them to this project only.

### Phase 1 results (2026-11-13)

Two runs, from the owner's zips `mpfree-results-20261113T130512Z` (clone 5f2d71c) and `mpfree-results-20261113T131028Z` (clone 90f8bfa, which compares the `SysEx` folders and prints developer mode's files). Both zips are owner-local.

- **The image:** build `SNAPSHOT-20261107130239`, the same eight boards as 3.10.0, and again one record only: the rootfs (xz, 453,051,392 bytes unpacked, its SHA-1 matches). No bootloader record.
- **Kernel and device trees:** release `6.18.44-az01-2026-08-10-rt6`, and the survey found `/boot` byte-identical to the 3.10.0 copy already in `boot-files/` (same hashes). The kernel and device trees phase 2 boots are the ones 3.10.0 booted.
- **Firmware (second run):** 35 of 38 files match 3.6.0. They include every board's `SME0/Firmware/<board>/update.dfu` and the 14 files in their `SysEx` folders (the MPC One's board is `ACVA`), the touch firmware (`ilitek/ili2117.bin`) and the Wi-Fi and Bluetooth firmware. Three differ, and no chip is flashed with any of them:
    - `regulatory.db` and `regulatory.db.p7s`: the Wi-Fi regulatory database, which the kernel reads.
    - `SME0/Firmware/update.sh`: one more sound-device path (`platform-fc840000.usb-usb-0:1:1.0`), for another board.
- **The verdict:** the tool says STOP because of those three files. It stops on any difference by design, so the owner and the thread read each one by hand. With these three explained, 3.10.1's updaters have nothing new to write to the MPC's chips, which is the condition phase 3 needs.
- **SSH as shipped:** `/etc/ssh/sshd_config.d/10-az0x.conf` allows root by key only (`PermitRootLogin prohibit-password`, `PasswordAuthentication no`, host key `/etc/ssh/ssh_host_ed25519_key`), and root's password field is locked (`*`). No port is set, so it is 22.
- **What developer mode changes** (files in `/usr/lib/az0x/user-access/`):
    - `enable.sh` copies `data-skel/` into `/data` without overwriting (`cp -an`), mounts an overlay of `etc-overlay/` on `/etc`, and reloads sshd. Akai's app names this script.
    - The overlay's `shadow` gives root an empty password. Its `ssh/sshd_config.d/00-user.conf` includes `/data/ssh/sshd_config.d/*.conf` and sets `AuthorizedKeysFile /data/ssh/authorized_keys`.
    - `data-skel/ssh/sshd_config.d/allow-access.conf` sets `PermitRootLogin yes`, `PermitEmptyPasswords yes`, `PasswordAuthentication yes` and `AuthenticationMethods none`: **root SSH with no authentication at all**. Its own comment says how to lock it down: empty the file and put your key in `/data/ssh/authorized_keys` (empty as shipped).
    - The login banner says "SSH is currently configured with no authentication" and points to that file.
    - `disable.sh` runs `rm -rf /data/root /data/ssh` and reboots.
    - **Inference:** the overlay mount lives only in RAM, while `/data/ssh` stays on the eMMC. So whether developer mode is on again after a reboot depends on Akai's app running `enable.sh` again at start. Only an installed 3.10.1 can show this.
- **The script runner** is inside Akai's app (`/usr/bin/MPC`: `ScriptRunner`, "contains an autorun script. Do you want to run it now?", "unsigned autorun.sh"). It asks on the screen before running a script. The app also has `StartSSHD` and `StopSSHD`. 3.6.0's app already had `ScriptRunner` and `StartSSHD`, but not `user-access`. The separate `az01-script-runner.service` is still started at boot, as in 3.6.0, and the stick masks it.
- **Units new since 3.6.0:**
    - `az0x-info.service`: logs system information through `/usr/bin/az0x-info`.
    - `az0x-loadpin-trustpoints.service`: known since 3.10.0.
    - `media-acvs\x2dcontent.mount`: the eMMC content partition, hidden on the stick.
- None of the new units flashes a chip by its unit file. Nothing in the stick builder's mask or stub lists needs to change for phase 2, and the stick's own SSH server reads only its own config (`sshd -f /etc/mpfree-test/sshd_config`), so Akai's new `sshd_config.d` files don't touch it.

**What happens next.** Phase 2 runs with the blocks given in the thread. Phase 3 is possible now that the firmware matches.

**One side effect.** After block 1.1 the newest unpacked image is 3.10.1, so `offline-kexec-stick.py` builds 3.10.1 sticks by default from then on, for every thread. To build a 3.10.0 stick again, pass `--image` with the 3.10.0 folder's `rootfs.img`.

## Phase 2: stick test with Akai's app off

This is live test 3's boot with 3.10.1's root filesystem in place of 3.10.0's. The tools, their checks and their undo are the ones in the [kexec test boot](archive/2026-11/kexec-test-boot.md#tools-in-order) and [the app guide](../guides/mpfree-app.md#running-it); nothing in them changes for 3.10.1. The exact blocks are given in the thread with the clone's commit once phase 1 has been read, and rehearsed word for word first.

In order: build `mpfree-screen-log`, `mpfree-startup` and `mpfree` for ARM; `offline-kexec-stick.py --splash … --app …` (it takes the newest unpacked image, 3.10.1); `write-test-stick.py` (**erases the stick's partitions 1 to 3**, keeps partition 4, asks for `WRITE`); `collect-over-ssh.sh --boot-map`; `MPFREE_ACCEPT_KEXEC_BOOT=YES collect-over-ssh.sh --kexec-boot` (**state-changing, RAM only**: you type `BOOT 3.10`); play the pad player for a few minutes; switch off, take the stick out, wait 10 s, switch on; `--boot-map` again; `read-stick-logs.py` with the stick in the workstation; `results.py zip`.

**Passes when:** the kexec boot ends with `RESULT: the test build is running from the stick, and it cannot see the eMMC or SD card.`; the pad player starts and plays; the stick logs show no failed unit we didn't expect and no `mpfree-test: blocked` call we can't explain; and the boot maps before and after match, with 0 sectors written to p1 to p6 and boot0/1.

**Undo:** switch off, take the stick out, wait 10 s, switch on: U-Boot boots 3.6.0 from the eMMC as before. The stick's old partitions 1 to 3 are gone (rebuild a 3.10.0 stick to go back); partition 4, the pad player's saved settings, stays.

### Phase 2 results (2026-11-13, clone 236de3f)

From the owner's zip `mpfree-results-20261113T135046Z` (owner-local). **Passed.**

- **The boot:** the stick was built from the 3.10.1 image (`akai-unpacked-20261113T130442Z-*`). After `BOOT 3.10` the test build answered SSH in 24 s and the boot step printed its `RESULT` line: release `6.18.44-az01-2026-08-10-rt6`, no MMC device, kexec off, `/` the stick's partition 1 read-only, `/data` tmpfs. Ethernet came up without the retry (`eth0 is up at 20 s; no retry needed`).
- **Nothing failed or was blocked:** 0 failed units, no masked Akai unit active, and no `mpfree-test: blocked` line in the boot step or the journal.
- **The pad player** was playing about 15 s after the jump and ran for about 8 minutes: 1,319 hits, up to 24 sounds at once, 0 underruns in 165,447 periods, pad to sound 8.65 ms on average (8.45 to 8.68), the same as live test 3 on 3.10.0. The saved settings and found buttons came back from partition 4. The app's processes used 20 to 44 % of one core; its memory peaked at 141 MB.
- **3.10.1's new units:**
    - `az0x-info.service` logged hardware and software details (exited normally).
    - `az0x-loadpin-trustpoints.service` printed `empty list of trusted verity digests: Protocol error` and finished; the kernel's LoadPin pinned the filesystem of `/usr/lib/az0x/trusted-verity-digests`, which on the stick is partition 1.
    - `media-acvs\x2dcontent.mount` bind-mounts `/storage` on `/media/acvs-content`. On the stick `/storage` is an empty folder in the read-only root, so nothing of the eMMC is involved.
    - `az01-signed-fs` did not get in the way: nothing about it appears in the journal.
- **The eMMC:** the boot maps before and after have the same 8 boot-area hashes, with 0 sectors written to p1 to p6 and boot0/1 since the power-on after the test. The p7 and p8 writes are stock 3.6.0's own use, as in every round.

So 3.10.1's userspace runs MPFree's test build as 3.10.0's did. What it cannot show is developer mode, which needs Akai's app: that is phase 3.

## Phase 3: developer mode on the stick

Developer mode is switched on from inside Akai's app (Akai's [release notes](https://cdn.inmusicbrands.com/Software/MPC/MOPN4PCOPEN/MPC%203.10.1%20-%20Release%20Notes.pdf)), so this phase runs Akai's app on the test build for the first time. Earlier sticks mask it and stub its launcher, because it can flash the pad and button controller ([updater findings](archive/2026-11/akai-updater-offline.md)).

**What the stick holds.** `offline-kexec-stick.py --akai-app` leaves `acvs.service` and its launcher `/usr/bin/az01-launch-MPC` as Akai ships them, and keeps the rest of phase 2:

- Every other mask and stub stays: `dfu-util`, `midifirmup`, `touch-fw-upd`, `xmosdfu`, `fw_setenv`, `az01-update`, `kexec`, `az01-script-runner.service` and the rest.
- The eMMC and SD card stay hidden, `/data` is RAM and kexec is off.
- The launcher runs `dfu-util -l` to look for a controller waiting for an update. The stub finds none, so the launcher never runs `SME0/Firmware/update.sh` and starts the app. One `mpfree-test: blocked /usr/bin/dfu-util -l` line in the journal is expected.
- Phase 1 found no firmware the MPC's chips don't already have. So even a flashing program reached some other way would have nothing new to write.
- There is no pad player or startup screen. Partitions 3 and 4 are made as with `--app`, so the stick's saved settings stay in place.
- Akai's firewall units stay masked, so port 22 can be reached on the stick. The build report quotes the rules they would load on an installed 3.10.1, which decide whether port 22 can be reached there.

**The two new tools:**

- [`host_devmode_check.py`](../../../tools/mpc-one/host_devmode_check.py), run as `collect-over-ssh.sh --devmode-check`. **Read-only** unless you type `LOCK`.
    - Over our port 2222 (the build's own host key only), it reads what developer mode changed: the `/etc` overlay, `/data/ssh`, the SSH server's settings as it reads them (`sshd -T`), its host key, the listening ports, the kind of root's password field (never its value), mounted drives, the journal and the autorun result.
    - It then logs in on port 22 from the workstation with no key and no password, and again with your key, and runs only `id -u`. It accepts only the host key it just read over port 2222.
    - After a typed `LOCK` it does Akai's own lock-down. This is **state-changing, RAM only**. It copies both files to `/run/mpfree-devmode`, empties `/data/ssh/sshd_config.d/allow-access.conf`, adds your public key to `/data/ssh/authorized_keys` and reloads Akai's SSH server. Then the login with no key must be refused and your key must get in.
    - It checks that `/data` is RAM before it changes anything. A power-cycle undoes it, and our port 2222 is never affected.
    - The login with no key and the lock-down were rehearsed against a real OpenSSH 9.6 server set up with Akai's files: the no-key login got in before the lock-down and was refused after it, and the key got in.
- [`write-autorun-drive.py`](../../../tools/mpc-one/write-autorun-drive.py), on the workstation only. It adds a harmless `autorun.sh` to the one mounted FAT32 or exFAT USB drive that is not the test stick, after a typed `WRITE`.
    - On the MPC the script writes one kernel log line and `mpfree-autorun-result.txt` next to itself (or in `/run` if the drive is read-only). It records the time, user, script path, parent process and cgroup, and changes nothing else.
    - The tool refuses a drive that already holds an `autorun.sh` or result file it did not write.
    - Undo: delete the two files from the drive.

**You need:**

- The MPC on the direct Ethernet cable only: while developer mode is on, anyone on its network can get a root shell with no password.
- For the autorun test (optional), a spare FAT32 or exFAT USB drive and the MPC's second USB-A port.
- 3.6.0 has its own script runner, and whether it would run this script has not been checked. So the spare drive goes into the MPC only while the test build runs.

**Block 3.1: update, check, build** (workstation only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin claude/project-thread-r73rb5:refs/remotes/origin/claude/project-thread-r73rb5 && git switch --no-track -C review-3101 origin/claude/project-thread-r73rb5 && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && python3 tools/mpc-one/tests/test_write_test_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_kexec_boot.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_devmode_check.py && python3 tools/mpc-one/tests/test_write_autorun_drive.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_read_stick_logs.py && python3 tools/mpc-one/tests/test_results.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log && sha256sum rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-screen-log' && python3 tools/mpc-one/results.py run build-stick -- python3 tools/mpc-one/offline-kexec-stick.py --akai-app
```

The build report says `akai app: on (--akai-app)`, lists `dfu-util` among the stubs and no stub for `az01-launch-MPC`, and quotes Akai's firewall rules.

**Block 3.2 (optional): the autorun drive** (workstation only; plug in the spare drive and let the desktop mount it first; asks for `WRITE`):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run autorun-drive -- python3 tools/mpc-one/write-autorun-drive.py
```

Eject the drive afterwards and keep it out of the MPC for now.

**Block 3.3: write the stick** (**erases the stick's partitions 1 to 3**, keeps partition 4, asks for `WRITE`):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- python3 tools/mpc-one/write-test-stick.py
```

Put the stick into the MPC and switch it on as usual. 3.6.0 starts as in every round.

**Block 3.4: boot map before** (read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run boot-map-before -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

**Block 3.5: start the test build** (**state-changing, RAM only**; you type `BOOT 3.10`):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run kexec-boot -- env MPFREE_ACCEPT_KEXEC_BOOT=YES bash tools/mpc-one/collect-over-ssh.sh --kexec-boot
```

It says the build runs Akai's app, and its checks on the test build print `Akai's app (acvs.service, expected in this build): active`. Every other masked unit must still be inactive.

**On the MPC, after block 3.5:**

1. Wait for Akai's app. It starts with empty settings, as on a new MPC, because `/data` is RAM. **Answer No or Cancel to anything that offers to update, format or erase**, and note what it asked.
2. Menu > Preferences > Enhancements: switch on the developer panel.
3. Hold SHIFT and press MENU, then choose Developer. Note what the screen shows.
4. Optional: plug the spare drive into the MPC's free USB-A port. If the MPC asks whether to run the drive's autorun script, answer yes. Note its exact words, and whether it asked at all.

**Block 3.6: the developer mode check** (read-only, unless you type `LOCK`: then **state-changing, RAM only**). Typing `LOCK` is recommended, since it checks that Akai's lock-down works:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-check -- bash tools/mpc-one/collect-over-ssh.sh --devmode-check
```

Then switch the MPC off, take out the stick and the spare drive, wait 10 s and switch it on. 3.6.0 starts from the eMMC.

**Block 3.7: boot map after** (read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run boot-map-after -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

**Block 3.8: stick logs and the zip** (workstation only; the stick in the workstation, read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py zip
```

Upload the zip, with your notes from the MPC's screen. The reports quote Akai's files: upload them to this project only.

**Passes when:**

- The kexec boot ends with its `RESULT` line, with Akai's app active and no other masked unit active.
- The developer mode check prints `CHECKS-OK` and records what port 22 accepts. With `LOCK`, it ends with `Akai's lock-down works`.
- The only `mpfree-test: blocked` line is the launcher's `dfu-util -l`, or each other one is explained.
- The boot maps before and after match, with 0 sectors written to p1 to p6 and boot0/1.
- The autorun test, if run, shows a result file or a kernel line, or the MPC's notes say why not.

**What it cannot show:**

- Whether developer mode is still on after a reboot of an installed 3.10.1. The app's settings live in `/data`, which is RAM here, so developer mode is off at every start of the test build.
- For the same reason, whether the script runner offers a drive that is already in at power-on.
- Whether 3.10.1's kernel can kexec into MPFree, since the test build turns kexec off.
- Whether Akai's firewall lets port 22 through on an installed 3.10.1. The build report quotes its rules for reading.

**Undo:** as phase 2. Developer mode, the lock-down and the autorun result in `/run` live in RAM only. To go back to the pad player stick, run phase 2's build and write blocks again.

## After the test

If 3.10.1 passes, installing it is a separate decision with its own gate: a fresh full eMMC backup, Akai's updater on mains power, then deleting MOD-002 by hand (the update removes MOD-001). The host tools then get a developer-mode login in place of port 2222, and the kexec jump stays as it is. A later step can start MPFree at power-on without Akai's app: a start-up unit in Akai's Linux that, before the app starts, checks the stick and jumps to it. That is a lasting change to Akai's files, like MOD-001, and needs its own go.
