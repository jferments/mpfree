# Qualifying main on the installed MPC OS 3.10.1

**Status:** runbook, 2026-11-15, revised the same day for MPFree's boot hook (#370, [how MPFree boots from the stick](soft-reboot-boot.md)), and again for the pad player without root (#372) and the opt-in boot routes (#374, [boot routes](boot-routes.md)). This round keeps to the default: a soft-reboot stick, the pad player as root. Not run yet: every block that touches the MPC waits for the owner's go. **Scope:** the reference MPC One (Gen 1). The owner installed MPC OS 3.10.1 on it with Akai's updater on 2026-11-15 and switched developer mode on. Before that it ran 3.6.0 with [MOD-001 and MOD-002](archive/2026-11/device-modifications.md).

The owner wants to know that main still does everything MPFree needs on the installed 3.10.1: log in, survey, back up, start MPFree from the USB stick at power-on with the boot hook (the soft-boot USB), run the pad player on it, and come back. The [stick test](firmware-3.10.1-stick-test.md) answered most of it before the install, with 3.6.0 doing a kexec jump, and the [boot hook's runs](stick-autoboot.md#second-power-on-2026-11-15-zip-092220z-mpfree-started) showed the power-on start once. This round does all of it from main on the installed 3.10.1, in one sitting, with every step's undo below.

## What the update changed for the tools

- **Our own SSH (MOD-001) is gone.** The update rewrites the root filesystem, where its unit lived. MOD-002's folder `/data/mpfree/ssh` should still be on `/data`, which the update doesn't touch; nothing reads it now. The boot map lists the names in it.
- **The way in is Akai's developer mode:** root SSH on port 22. As Akai ships it, it needs no key and no password (`AuthenticationMethods none`). Akai's own lock-down is to empty `/data/ssh/sshd_config.d/allow-access.conf` and put a key in `/data/ssh/authorized_keys` ([what the image says](firmware-3.10.1-stick-test.md#phase-1-results-2026-11-13)).
- **Two ports in the tools.** [`collect-over-ssh.sh`](../../../tools/mpc-one/collect-over-ssh.sh) now reaches Akai's system on `MPC_PORT`, default 22, and the test build on the stick on `MPC_TEST_PORT`, default 2222. For a unit still on 3.6.0 with MOD-001, set `MPC_PORT=2222`.
- **New modes**, all in [survey tools](../guides/survey-tools.md):
    - `--devmode-trust`, `--devmode-status`, `--devmode-lock` and `--devmode-unlock` ([`host_devmode_login.py`](../../../tools/mpc-one/host_devmode_login.py)).
    - `--system-survey` ([`read-only-system-310.sh`](../../../tools/mpc-one/read-only-system-310.sh)).
    - `--hook-install`, `--hook-report` and `--hook-remove`: MPFree's boot hook, below ([`hook-install.sh`](../../../tools/mpc-one/hook-install.sh)).
- **MPFree starts from the stick by the boot hook, not by kexec.**
    - 3.10.1's LoadPin refuses the kexec jump (`kexec-image old-api-denied`, [the preflight of 2026-11-15](stick-autoboot.md#the-hook-step-3-the-kexec-version-replaced)), so `--kexec-boot` stops at its checks there and loads nothing.
    - Instead, `--hook-install` writes MPFree's boot hook to `/data`: three small files. At each power-on with the stick in, the hook checks the stick and soft-reboots into the stick's own system on Akai's running kernel, at about 6 s, before Akai's app starts. The stick's system switches the eMMC and SD card off before anything mounts. Without the stick, Akai's system starts as usual. How it works is in [how MPFree boots from the stick](soft-reboot-boot.md); how it was found and proven is in [the record](stick-autoboot.md).
    - **This round uses the default stick.** Q6 builds it without `--kexec-file`, `--chain` or `--app-user`, so the hook soft-reboots into it at every power-on and never runs `kexec` for it, and the pad player runs as root, as before #372. The opt-in routes are a restart into Akai's own kernel by `kexec_file`, and a chain from there to the stick's own kernel and device tree, which brings Ethernet up. They are compared in [boot routes](boot-routes.md). They and the pad player without root have their own owner blocks in [the record](stick-autoboot.md#the-kexec-file-route-step-6-opt-in) (steps 6 and 7) and are not part of this round.
    - The hook was installed on this MPC in the boot-hook runs of 2026-11-15, last by the opt-in rounds of #374, and Q3 shows whether it still is. Q7b installs main's version either way: it soft-reboots any stick without `--kexec-file`.
    - **So a power-on with the stick in starts MPFree, not Akai's system.** Every block that needs Akai's system runs with the stick out of the MPC, until Q7b.
    - The stick's system is built for the running kernel, `6.18.44-az01-2026-08-10-rt6`: the builder takes the release Q5 records, and the hook refuses a stick built for another release.
- **The backup's baseline is still 3.6.0's**, the only one recorded. The update rewrote `mmcblk0p6` (the root filesystem), so the backup is expected to end with a `BASELINE MISMATCH` for p6 and exit code 3. Any other partition in that list means stop. After this round, a 3.10.1 baseline is made from its boot map and backup.

## What this round answers

| Question | Answered by |
|---|---|
| Does developer mode answer as the image says: port 22, root, no key, no password? | Q1 |
| Does Akai's lock-down to a key work on the installed system? | Q2 |
| Which ports are open with developer mode on (Akai's web server too), and is Wi-Fi on? | Q3 |
| Did the update change any boot partition besides the root filesystem? | Q4 and the boot maps in Q7a and Q10 |
| Do developer mode, the lock and the SSH host key survive a restart? | Q7a, compared with Q3 |
| Does main's boot hook install, and does it accept the stick main builds? | Q7b |
| Does a power-on with the stick in start MPFree with no tap, with the eMMC and SD card switched off before anything reads them? | Q7c and the stick logs |
| Does the stick's system answer on port 2222 after the soft-reboot, and does the hardware work in it (Ethernet, screen, touch, pads, sound, partition 4)? | Q8 |
| Does the MPC come back to 3.10.1 with the eMMC's boot areas unchanged? | Q10 |

## You need

- **The MPC on the direct Ethernet cable only.** Until Q2 has locked it, anyone on any network the MPC is on can get a root shell with no password. If the MPC is on Wi-Fi, switch its Wi-Fi off in Akai's settings before Q1.
- Developer mode on: Menu > Preferences > Enhancements, switch on the developer panel; then hold SHIFT and press MENU, and choose Developer.
- About 8 GiB free in `~/mpfree-backups` for Q4, and a second disk for the copy the backup tool asks you to make.
- The USB stick from earlier rounds. Its partition 4 keeps the pad player's settings.
- **The stick out of the MPC at the start.** With the boot hook on the MPC, switching on with the stick in starts MPFree instead of Akai's system. The stick goes into the MPC in Q7b, while Akai's system runs.
- An SD card in the MPC's slot can stay. The stick's system switches the SD card off with the eMMC, and the stick logs show that nothing read it first.

Every block runs in a terminal on the workstation, from the clone at `/home/jesse/code/mpfree-hardware-survey`. Each step's output lands in one results folder, and Q10 packs it into one zip.

## The blocks

**Block Q0: update, check, build** (workstation only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C qual-3101 origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_devmode_login.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_kexec_preflight.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_host_emmc_backup.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && python3 tools/mpc-one/tests/test_write_test_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_boot_hook.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_app_mpc.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_read_stick_logs.py && python3 tools/mpc-one/tests/test_results.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app && cd rust/target/armv7-unknown-linux-gnueabihf/release && sha256sum mpfree-screen-log mpfree-startup mpfree'
```

It ends with three hashes. `results.py status` lists what is already in the results folder: if an earlier round left files there, run `python3 tools/mpc-one/results.py zip` first, so this round gets its own zip.

**Block Q1: record developer mode's host key** (read-only on the MPC; one line in `~/.ssh/known_hosts` after you type `TRUST`):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-trust -- bash tools/mpc-one/collect-over-ssh.sh --devmode-trust
```

It checks the cable, the route and the MPC's MAC first, then reads the SSH server's ed25519 host key on port 22 and prints its fingerprint. To check the key itself too (optional), type this in the developer panel's terminal on the MPC's screen. It only reads, and it should print the same fingerprint:

```sh
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
```

After `TRUST`, it tries two logins that run `id -u` only. Expect `no key, no password: logged in`: that is the open login Q2 closes.

**Block Q2: lock developer mode's SSH to your key** (**state-changing, on `/data`**: the eMMC's data partition, which survives a restart; you type `LOCK`):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-lock -- bash tools/mpc-one/collect-over-ssh.sh --devmode-lock
```

- **What it changes:**
    - It copies `allow-access.conf` and `authorized_keys` into `/data/ssh/mpfree-lock-backup`.
    - It adds your public key (`/home/jesse/.ssh/mpc_one_autorun_ed25519.pub`) to `/data/ssh/authorized_keys` and empties `allow-access.conf`.
    - It runs `systemctl reload sshd.service`, the step Akai's login banner gives.
    - It writes nothing else, and only under `/data/ssh`.
- **Before it changes anything,** it checks that this is an installed MPC OS with developer mode on: `/data` comes from the eMMC's p8, developer mode's overlay is on `/etc`, and no earlier lock of ours is there.
- **Passes when** it ends with `LOCKED: the login with no key is refused and your key gets in`. It also checks that the SSH server no longer accepts empty passwords.
- **If it fails halfway:** it keeps one SSH connection open for the whole step. If the new logins don't come out as expected, it puts both files back over that connection, reloads the server and checks that the open login works again. The lock was rehearsed against a real OpenSSH 9.6 server set up with Akai's files: no-key login refused after the reload, the key got in, the kept connection survived the reload, and the undo reopened it.
- **Undo:** block Q2-undo below.

**Block Q3: what the installed 3.10.1 looks like** (read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-status -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run system-survey -- bash tools/mpc-one/collect-over-ssh.sh --system-survey && python3 tools/mpc-one/results.py run hook-report-before -- bash tools/mpc-one/collect-over-ssh.sh --hook-report
```

- **The status** shows where `/` and `/data` come from and the `/etc` overlay. It shows the SSH server's settings as the server reads them, the lock, the kind of root's password field (never its value) and the open TCP ports. It also prints the host key's fingerprint and date, and how long the MPC has been on: Q7a prints them again after a restart. It ends `LOCKED: only your key gets in`. Root's password field still shows `empty`, because that comes from Akai's overlay. The server refuses it after the lock: its settings show `permitemptypasswords no` and `authenticationmethods any`.
- **The system survey** covers:
    - versions, CPU clocks and idle states, memory;
    - kexec and kernel security (`/usr/sbin/kexec`, `kexec_load_disabled`, lockdown, LoadPin);
    - network interfaces and open ports (Wi-Fi shows here);
    - sound and MIDI (cards, sequencer clients, `aconnect -l`, `amidi -l`), input devices, drives and the names at their top level;
    - screen and GPU devices, controller scripts, and Akai's app's thread count.
- **The hook report** shows whether the boot hook from the 2026-11-15 runs is still on the MPC: its three files and their hashes, and what it did at this power-on. With no stick in, expect `MPFree's stick in now: no` and nothing staged at `/run/nextroot`.

**Block Q4: a fresh full eMMC backup** (read-only on the MPC; about 7.3 GiB to `~/mpfree-backups`, about 4 minutes; keep the MPC idle):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run backup-3101 -- bash tools/mpc-one/collect-over-ssh.sh --backup
```

This becomes the return point for the installed 3.10.1, which the 3.6.0 backup no longer is.

- **What to expect:** it checks the disk layout against the baseline and copies nothing if the layout differs. It then copies the whole eMMC and both boot areas, and checks every image twice.
- **The expected ending:** `BASELINE MISMATCH (boot storage differs from the baseline, state MOD-001+MOD-002): mmcblk0p6`, then `STOP: …` and exit code 3. That is the root filesystem the update rewrote.
- **Stop if** the list names any other partition (p1, p3, p4, p5, boot0 or boot1), and send the zip before going on.
- Make the second copy it asks for.

**Block Q5: the running device tree for the stick** (read-only; copies `/sys/firmware/fdt` to a private folder under `~/mpfree-work`):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run kexec-preflight -- bash tools/mpc-one/collect-over-ssh.sh --kexec-preflight
```

The report compares the running tree with Akai's board files in `~/mpfree-work/boot-files/6.18.44-az01-2026-08-10-rt6/`, without values.

**Block Q6: build the stick** (workstation only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run build-stick -- python3 tools/mpc-one/offline-kexec-stick.py --splash rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-startup --app rust/target/armv7-unknown-linux-gnueabihf/release/mpfree
```

It takes the newest unpacked image (3.10.1) and the device tree from Q5. Its report must say:
- `running release: 6.18.44-az01-2026-08-10-rt6 (recorded by the preflight; the same kernel as the test build's)`;
- `pad player user: root (no --app-user)`;
- `for MPFree's soft-reboot hook: /boot/mpfree-softreboot on partition 1 (protocol mpfree-softreboot-1, kernel release 6.18.44-az01-2026-08-10-rt6)`, then `modules the hook loads first: none` and `route: soft-reboot only (no --kexec-file)`. A `--kexec-file:` or `--chain:` line there, or `pad player user: mpfree`, means the command was not the one above: stop and build again with it;
- under `units Akai's udev rules start when a device appears`, `no-reserved-blocks@…` marked `masked on the stick` (the unit that read the SD card on 2026-11-15). If it is not masked, or another unit of Akai's (a name starting `az0`) is marked `NOT masked`, stop after Q6 and send the results: every Akai unit that can touch a disk must be masked on the stick ([the boot's limits](soft-reboot-boot.md#limits-and-what-to-watch)). systemd's own units in that list may show `NOT masked`; the stick logs show whether any of them touched the eMMC or SD card (below, under "Passes when").

**Block Q6-write: write the stick** (**erases the stick's partitions 1 to 3**, keeps partition 4; you type `WRITE`; the stick in the workstation):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- python3 tools/mpc-one/write-test-stick.py
```

If the stick last held an opt-in build (steps 6 and 7 of the record), this replaces it with the default one. Partition 4 is kept, and its files may belong to the `mpfree` user from `--app-user`; the pad player, root on this stick, still reads and writes them.

**Block Q7a: restart without the stick** (read-only). Leave the stick out of the MPC. Switch the MPC off, wait 10 s, and switch it on. Wait for Akai's app. If developer mode is not on any more, note that, and switch it on again (SHIFT + MENU, Developer). Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run status-after-restart -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run boot-map-before -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

- **Expect `LOCKED: only your key gets in`** and the same host key fingerprint as in Q3.
- **If it stops with `did not show the host key recorded`,** the MPC made a new host key at the restart. That is a finding, and nothing was changed. Run Q1 again: it prints the new fingerprint and the exact `ssh-keygen -R` line that removes the old one. Compare the new fingerprint on the MPC's screen, as in Q1, then run the printed line and Q1 once more, then Q7a.
- **If the logins say `nothing listens on that port`,** developer mode is off. Switch it on and run Q7a again.

**Block Q7b: install main's boot hook, with the stick in the MPC** (**state-changing, on `/data`**: three small files; then a read-only check of the stick). First put the stick into the MPC's USB-A port while Akai's app runs. Answer No or Cancel to anything that offers to update, format or erase, and note what it asked. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-install -- env MPFREE_ACCEPT_HOOK=YES bash tools/mpc-one/collect-over-ssh.sh --hook-install
```

- **What it changes:** it writes `/data/mpfree-hook/hook.sh`, the udev rule `/data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules` and the modprobe line `/data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf`, in Akai's writable `/etc` layer. It writes nothing else. With an earlier version there, it says `an earlier version of the hook is in place; it is replaced`; if main's version is there already (the last opt-in round may have installed it), it says `already in place, unchanged` and writes nothing.
- **Then it checks the stick, read-only:** it reads partition 1 where Akai's system mounted it (or mounts it read-only itself, and unmounts it after), and checks it as the hook will at power-on. This stick asks for no `kexec_file`, so the check loads no kernel. It ends `CHECK-OK: at power-on with this stick in, the MPC would soft-reboot into it`, with nothing after `into it`. A line `this stick asks to start on Akai's own kernel (kexec_file)` means the stick is an opt-in build: take it out before the next power-on, and go back to Q6.
- **Passes when** it ends `RESULT: the hook is installed, and the stick checks out.`
- **If it says `MPFree's stick is not in the MPC`,** put the stick in, wait for Akai's app to settle, and run Q7b again. It writes nothing more.
- **If the check stops,** it ends `RESULT: the hook is installed, but the check stopped (above).` Take the stick out before you next switch the MPC on (without it the hook does nothing), skip to Q10 and send the zip.
- **If it is cut off halfway:** each file is written under a temporary name and renamed, the script first, so nothing points at a half-written script. Run it again to finish.
- **Undo:** block Q7b-undo below.

**Step Q7c: the power-on into MPFree** (**state-changing**: at about 6 s Akai's programs stop and MPFree's system runs from the stick on Akai's kernel; the eMMC and SD card are switched off in software and nothing is written to them; MPFree writes only RAM and the stick). Leave the stick in. Switch the MPC off, wait 10 s, and switch it on.

- **Expected:** the MPC logo, then MPFree's startup screen and the pad player, about 16 s after power-on, with Akai's app never drawing. Play a few pads. On 2026-11-15 this took no tap.
- **If Akai's app comes up instead, leave the stick in and the MPC on,** and run block Q7c-report first. The hook's notes are in RAM and last only until the power-off. Then go to Q10.
- **If the MPC switches itself off,** the stick's switch-off service refused to go on: the eMMC or SD card was still in use, or would not switch off. It saved its notes to the stick. Go to Q10.
- **If the screen stays on the logo for more than a minute,** hold the power button to switch off, and go to Q10.
- Don't touch the MPC's power while it shows an update screen.
- **Undo, always:** switch off, take the stick out, wait 10 s, switch on. Akai's system starts as before.

**Block Q7c-report: only if Akai's app came up in Q7c** (read-only; the stick still in):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-report-stick-in -- bash tools/mpc-one/collect-over-ssh.sh --hook-report
```

It shows what the hook decided at this power-on and why it stopped.

**Block Q8: the pad player on the test build** (read-only: the check starts, copies and stops nothing; each phase starts and ends with Enter):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-check -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_PHASES=1-3,9,16,19 bash tools/mpc-one/collect-over-ssh.sh --app
```

Six of the app check's nineteen phases ([the app guide](../guides/mpfree-app.md#the-live-test)), by their numbers there:

- 1: Map controls.
- 2: the Dev pages (Performance, CPU by engine, Processes), which show the audio loop's render time against its 2.9 ms period.
- 3: kits, pad banks, Full and Half Level.
- 9: the Synth tab's five engines.
- 16: Prefs, System info and saving.
- 19: menus, the console and STOP.

They show the screen, touch, pads, buttons, sound and partition 4 working after the soft-reboot from the installed 3.10.1. The check also prints when the pad player started playing, and the GPU facts it always reads.

- **This is the first login on port 2222 after a soft-reboot.** Before it reads anything, the check prints `started by: the boot hook's soft-reboot (asked at … s after power-on)`, `no MMC block device`, `kexec_load_disabled: 1` and `/: read-only`, and ends `CHECKS-OK`. The release is Akai's own kernel now, so the stick's host key and the missing MMC devices are what tell the stick's system from Akai's.
- **It also prints `privileges:` lines** (#372): on this stick, built without `--app-user`, the pad player runs as root (`Uid: 0`), as before. That is expected here.
- **Our reading, which the check shows:** after the soft-reboot `/` stays the stick's read-only partition 1, `/run` is Akai's tmpfs carried over and allows programs, and the stick's own sysctl file turns kexec off. If any of these is wrong, the check prints a `PROBLEM` line and stops before it reads anything else.
- **If it says `the test build did not answer on port 2222`,** note what the screen shows. The pad player can still be played by hand. Go to Q10: the stick logs show the network state.

**Q9 (optional): GPU page flips in the same boot.** The test build is running from Q7c, started at power-on by the boot hook's soft-reboot, and Q8 is done. Nothing else in this round depends on Q9, so skip to Q10 if you'd rather not. These are the GPU thread's page-flip blocks, rehearsed by that thread word for word at this round's commit. Inside the round, G0 fetches nothing, so the GPU blocks use Q0's commit even if main moves during the round.

**Block G0: check the clone** (workstation only, read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git status --short --branch | head -1 && git log --oneline -1 && grep -c '^def flip' tools/mpc-one/host_gpu_mpc.py
```

Expect three lines: `## qual-3101`, Q0's commit line, and `1`.

**Block G1: build the benchmark and the page writer** (workstation only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc cargo build --release --locked --manifest-path rust/mpfree-gpu/Cargo.toml --target armv7-unknown-linux-gnueabihf --bins && cargo build --release --locked --manifest-path rust/Cargo.toml -p mpfree-app
```

Expect `Finished` twice. The ARM benchmark and the workstation's `mpfree`, which writes the sample pages, must come from one build, because the page lists' format is versioned. This block builds both from the round's commit.

**Block G2: the GPU on the MPC, ending with page flips** (**state-changing, RAM only**, at step 7; press Enter at each of steps 3 to 7; about 10 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run gpu-flip -- env MPFREE_ACCEPT_GPU_BENCH=YES python3 -I tools/mpc-one/host_gpu_mpc.py --flip
```

It talks only to the test build that the boot hook started at Q7c: port 2222, with the stick's own host key.

What it changes, step by step:
- **1:** workstation only. It checks the benchmark (ARM, the MPC's C library) and writes the sample pages (80 at `8408aa36`) into a new folder under `~/mpfree-work`.
- **2:** read-only on the MPC. The test build's checks (they include `started by: the boot hook's soft-reboot …`), then the GPU's render node and libraries, and who holds `/dev/dri`. After the soft-reboot no driver can be loaded, so the GPU's must already be. If step 2 stops on `no /dev/dri/renderD*`, nothing was changed: go on to Q10 and tell the GPU thread (the likely fix is a stick built with `--preload-module panfrost`).
- **3:** RAM. It copies the benchmark and the pages into `/run/mpfree-gpu` and checks their hashes. It stops the pad player (it prints `pad player: was active, now inactive`), and starts it again at the end, even on a failure or Ctrl-C.
- **4 and 5:** these only read and compute; the screen keeps what it shows. Step 4 stops if Mesa answers with anything but the Mali (Panfrost).
- **6:** writes each page to `/dev/fb0` as the app does, 1.5 s each. No display setting changes.
- **7:** first a read-only check that no program has `/dev/dri/card*` open; if one does, it stops and the screen is not changed. Then the program draws the first page into a screen buffer and reads it back. If that isn't the expected picture, it stops before any change. Only then does it print `SCREEN CHANGES NOW`, take the display (DRM master) and point the lit CRTC at its own buffers, in the mode the screen already runs. No mode, timing, panel or stored setting changes.

**Undo:** automatic. Before it exits, the program puts the CRTC back as it found it, showing the framebuffer's picture, and prints `flip: the screen is back as it was`. It does this on a normal end, on Ctrl-C, on a closed SSH session and on a failure. If the program is killed outright, the kernel gives the screen back to the framebuffer when the display's last file closes. Anything left after that is cleared by Q10: switch off, take the stick out, wait 10 s, switch on (nothing is stored). With the hook installed, a power-cycle with the stick still in starts MPFree again.

**If it fails partway:**
- **Stopped before `SCREEN CHANGES NOW`:** the tool says "the screen was not changed" or "(before the screen changed)". Nothing on the display was touched. Go on to Q10.
- **Printed `SCREEN CHANGES NOW`, but never "the screen is back as it was":** this includes `flip: putting the screen back failed: …`. The tool then prints "The program did not say the screen is back." Go straight to Q10, even if the screen looks right.
- **The screen freezes or goes black and the tool hangs:** press Ctrl-C once and wait 10 s. If the screen doesn't come back, go to Q10.

**Expected output (the lines that matter):**

```
[2/7] ... started by: the boot hook's soft-reboot (asked at N s after power-on) ... CHECKS-OK ... GPU-OK
[3/7] ... copied the benchmark and 80 pages; every hash on the MPC matches
      pad player: was active, now inactive
[4/7] gpu: Mali-T760 (Panfrost) ...
      gpu: draws into screen buffers (dma-buf import): yes
[5/7] summary: pages 80; gpu whole page median ... ; gpu tick median ...
      summary: gpu draw calls per whole page median N worst N; atlas cleared N times, N masks drawn alone; N shapes worked out on the GPU, their atlas cleared N times
      summary: gpu against software: at most N levels apart in any channel; ... s in all
[6/7] summary: panel gpu: draw p50 ..., read back p50 ..., copy p50 ...; whole p50 ...
      summary: panel showed 80 pages from the GPU; atlas cleared N times, N masks drawn alone
[7/7] display: /dev/dri/card0
      CARD-FREE
      flip: /dev/dri/card0: connector N on CRTC N, mode 800x1280 at 60 Hz
      flip: SCREEN CHANGES NOW: 80 pages drawn on the GPU straight into the buffers the panel shows, 1500 ms each
      summary: flip gpu ...: wait for the flip p50 ..., draw p50 ..., gpu finish p50 ...; whole p50 ... ; CPU ... % of one core
      summary: flip software ...: draw p50 ..., turn p50 ..., copy into the screen buffer p50 ... (whole frame); whole p50 ...; CPU ... % of one core
      flip: the screen is back as it was (CRTC N showing framebuffer N); N page flips in all
      pad player: active (started again)
RESULT: the GPU benchmark ran (numbers above).
```

**What to watch, and to note in the zip's notes:**
- In steps 6 and 7, is each page the right way up and not garbled?
- In step 7, does any page tear or flicker?
- When step 7 ends, does the pad player come back on the screen?

**The numbers this is for** (live test 3, 13 Nov): the GPU's way to the panel took 63 ms a frame and software's 32 ms. Step 7's GPU `whole p50` is the number the page flips are meant to bring below software's. Its software line times the turn into the panel's layout and the copy apart: if the turn is most of software's time, drawing software in the panel's own layout is worth building.

**Block Q10: back to 3.10.1.** Switch the MPC off, take out the stick, wait 10 s and switch it on. The stick must be out: with the hook installed, a power-on with the stick in starts MPFree again. Wait for Akai's app, and switch developer mode on again if it is off. Then (read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run status-after -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run hook-report-after -- bash tools/mpc-one/collect-over-ssh.sh --hook-report && python3 tools/mpc-one/results.py run boot-map-after -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

The hook report should show main's hook files, with the hashes Q7b wrote, and `MPFree's stick in now: no`.

Then, with the stick in the workstation (read-only on the stick):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py zip
```

Upload the zip, with your notes from the MPC's screen. The reports quote Akai's files: upload them to this project only.

## Passes when

- Q1 records the host key; Q2 ends `LOCKED`; Q3, Q7a and Q10 end `LOCKED: only your key gets in`.
- Q4's backup is verified, and its baseline mismatch names only `mmcblk0p6`.
- Q6's report has the soft-reboot line, `route: soft-reboot only (no --kexec-file)` and `pad player user: root (no --app-user)`, `no-reserved-blocks@…` is `masked on the stick`, and no unit starting `az0` is `NOT masked`.
- Q7b ends `RESULT: the hook is installed, and the stick checks out.`
- In Q7c the MPC went from the logo to MPFree's pad player with no tap, and Akai's app never drew.
- Q8 ends its checks with `started by: the boot hook's soft-reboot` and `CHECKS-OK`, and its phases work as in the stick test's phase 2.
- The stick's `dmesg.txt` shows `mpfree-internal-off: switched off ff0f0000...` before any `sd` or `mmc` mount. Nothing reads the eMMC or SD card before `mpfree-internal-off: done`: no `no-reserved-blocks` unit and no `Buffer I/O error` on `mmcblk`.
- The boot maps from Q7a and Q10 have the same boot-area hashes, with 0 sectors written to p1 to p6 and boot0/1 since the power-on in Q10.
- The stick logs show no failed unit we didn't expect and no `mpfree-test: blocked` line we can't explain. Akai's `az0x-loadpin-trustpoints` fails after the soft-reboot, as on 2026-11-15; that failure is harmless.

## Undo, block by block

| Block | Changes | Undo |
|---|---|---|
| Q0, Q6 | New folders under `~/mpfree-work` and `~/mpfree-results`, and build output in the clone | Delete them |
| Q1 | One line for `10.42.0.141` in `~/.ssh/known_hosts` | `ssh-keygen -R 10.42.0.141 -f ~/.ssh/known_hosts` |
| Q2 | `/data/ssh` on the MPC | Block Q2-undo |
| Q4 | A new folder under `~/mpfree-backups` | Keep it: it is the return point for 3.10.1 |
| Q5 | A private folder under `~/mpfree-work` (it holds the serial number and MAC) | Delete it |
| Q6-write | The stick's partitions 1 to 3 | Rebuild and write an earlier stick; partition 4 stays |
| Q7b | The boot hook's three files on `/data`, in place of an earlier version | The hook stays installed after the round: it is how MPFree starts. To take it off the MPC, block Q7b-undo |
| Q7c, Q8, Q9 | RAM on the MPC; the stick's log partition and partition 4 | Switch off, take out the stick, wait 10 s, switch on |

**Block Q2-undo: unlock** (**state-changing, on `/data`**; you type `UNLOCK`). It logs in with your key, puts both files back from `/data/ssh/mpfree-lock-backup`, removes that folder, reloads the server, and checks that the login with no key works again, as Akai ships it:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-unlock -- bash tools/mpc-one/collect-over-ssh.sh --devmode-unlock
```

If SSH can't be reached at all, the developer panel's terminal on the MPC's screen is a root shell. These three lines put Akai's files back; Q2 prints them too:

```sh
cp /data/ssh/mpfree-lock-backup/allow-access.conf /data/ssh/sshd_config.d/allow-access.conf
cp /data/ssh/mpfree-lock-backup/authorized_keys /data/ssh/authorized_keys
systemctl reload sshd.service
```

When the backup folder holds a file named `absent` instead of `authorized_keys`, there was none before the lock: the second line is then `rm /data/ssh/authorized_keys`. Akai's files say that switching developer mode off deletes `/data/ssh` and `/data/root` and restarts the MPC, which removes the lock too. That has not been seen on this unit.

**Block Q7b-undo: remove the boot hook** (**state-changing, on `/data`**; Akai's system, the stick out). It deletes the hook's three files, and the folders its installs made, and nothing else. From the next power-on the MPC starts Akai's system, stick in or not:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-remove -- bash tools/mpc-one/collect-over-ssh.sh --hook-remove
```

It ends `RESULT: the hook is gone.` By hand, in the developer panel's terminal:

```sh
rm /data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf /data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules
rm -r /data/mpfree-hook
```

## What this round cannot show

- Whether developer mode, the lock, the host key and the boot hook survive a long power-off or Akai's next update. After an update, the stick must be rebuilt from a fresh Q5: the hook refuses a stick built for another kernel, and Akai's system starts.

## Rehearsal (2026-11-15)

Every block was run word for word from a clone at `/home/jesse/code/mpfree-hardware-survey`, with `HOME=/home/jesse`. Stand-ins played the cable and the MPC: `ip`, `ping`, `ssh` and `ssh-keyscan` answered as a fake installed 3.10.1 with developer mode on. Its remote scripts ran under BusyBox against fake system files, and the boot hook's check and jump under dash, with stand-ins for `mount`, `umount`, `systemctl`, `systemd-run` and `modprobe`. The answers were typed in a terminal only after each question appeared.

**The boot-hook round, at commit `6bf62081`.** The fake MPC started with an earlier version of the hook installed an hour before.

- **Q0 ran for real.** It fetched main and printed `LOCAL-CHECKS-OK`, with the boot hook's tests. The three ARM programs built (`Finished`, three hashes).
- **Q1** recorded the key and printed `no key, no password: logged in`. **Q2** printed `LOCKED: the login with no key is refused and your key gets in.`
- **Q3:** the status ended `LOCKED: only your key gets in.` The survey reached `END of read-only system survey`. The hook report showed the earlier hook's three files, `MPFree's stick in now: no` and `hook installed before this boot: yes`.
- **Q5** copied the fake running tree. **Q6** built a stick from a fake 3.10.1 image with Q0's real ARM programs. It printed the `running release` line, the soft-reboot line, `modules the hook loads first: none` and `no-reserved-blocks@%k.service: masked on the stick`.
- **Q7a**, after a restart without the stick, ended `LOCKED: only your key gets in.`
- **Q7b**, first with the stick left out:
    - It replaced the earlier hook (`an earlier version of the hook is in place; it is replaced`).
    - It stopped with `MPFree's stick is not in the MPC`.
- **Q7b again, with the stick in:**
    - The stick's partition 1 came from Q6's `stick.img`, mounted at `/media/<UUID>` as Akai's system mounts it.
    - The block said `already in place, unchanged` and read the stick from there.
    - It ended `CHECK-OK` and `RESULT: the hook is installed, and the stick checks out.`
    - The fake image has no `/usr/lib/systemd/systemd` or `os-release`; the real one has both. They were added to the copy. Without them, the check stops with `the stick has no /usr/lib/systemd/systemd to start`, as it should.
- **A power-on that falls through:**
    - The installed hook ran as at power-on: the modprobe trigger at 6.05 s, then the jump, with the stick plugged in but not yet mounted.
    - The stand-in `systemctl` refused the soft-reboot, so the jump stopped and left nothing at `/run/nextroot`.
    - **Q7c-report** then showed the hook's notes, from `MPFree's stick is in: modprobe starts the jump at 6.05 s` to `STOP: systemctl could not start soft-reboot.target` and `nothing was started: Akai's system starts as usual`.
- **Q10's first block,** with the stick out:
    - The status ended `LOCKED: only your key gets in.`
    - The hook report showed main's files, with Q7b's hashes, and `MPFree's stick in now: no`.
    - The boot map ran.
- **Q7b-undo** printed `RESULT: the hook is gone.` **Q2-undo** printed `UNLOCKED: Akai's files are back; a login with no key gets in again, as before the lock.`

**A new host key after the restart, at commit `8c5e2ab4`.** Q1 and Q7a have not changed since.
- Q7a stopped with `did not show the host key recorded`, and nothing changed.
- Q1 stopped with `already holds a different key` and printed `ssh-keygen -R '10.42.0.141' -f /home/jesse/.ssh/known_hosts`.
- After that line, Q1 recorded the new key, and Q7a passed.

**Q9** was rehearsed by the GPU thread at `b6160175` (earlier at `f2e36b7b` and `209f5a15`) against the fake test build from `tools/mpc-one/tests/test_host_gpu_mpc.py`, as after Q7c and Q8: the boot hook's marker `/run/mpfree-hook/soft-reboot` in place and the pad player running. G0 printed `## qual-3101`, the commit and `1`; G1 printed `Finished` twice. G2 step 2 printed `started by: the boot hook's soft-reboot (asked at 6.59 s after power-on)`, then `CHECKS-OK` and `GPU-OK`. G2 ended both ways: with "the screen is back as it was" and exit code 0, and, for a flip that fails without saying so, with "The program did not say the screen is back. …" and exit code 1. Both times the pad player started again. The GPU thread rehearsed it again at main `8408aa36` (train 53, with #368) merged with this pull request's head: same output, 80 pages.

**Not run here,** because they need the MPC or a real stick: Q4, Q6-write, the power-on in Q7c that soft-reboots into MPFree, Q8 and the stick logs. Q0 runs their tools' own tests against fakes under BusyBox:
- The backup test includes a mismatch on p6 only, with the lines Q4 quotes.
- The boot hook's tests include the jump that mounts the stick read-only at `/run/nextroot` and asks for `soft-reboot.target` once, and every way it stops.
- The app check's tests include a test build started by the boot hook (`started by: the boot hook's soft-reboot`).
- The soft-reboot itself started MPFree on this MPC on 2026-11-15 ([second power-on](stick-autoboot.md#second-power-on-2026-11-15-zip-092220z-mpfree-started)).

Later commits in the same pull request change only documentation.

## After the round

- A 3.10.1 baseline for the backup tool, `tools/mpc-one/baselines/mpc-one-3.10.1-reference.txt`, from the Q7a boot map and the Q4 backup.
- The [compatibility page](firmware-3.10.1-compatibility.md) and the [developer access guide](../guides/developer-access.md) updated with what the installed unit showed.
- The results here, as in the stick test's phase sections.
