# Qualifying main on the installed MPC OS 3.10.1

**Status:** runbook, 2026-11-15. Not run yet: every block that touches the MPC waits for the owner's go. **Scope:** the reference MPC One (Gen 1). The owner installed MPC OS 3.10.1 on it with Akai's updater on 2026-11-15 and switched developer mode on. Before that it ran 3.6.0 with [MOD-001 and MOD-002](archive/2026-11/device-modifications.md).

The owner wants to know that main still does everything MPFree needs on the installed 3.10.1: log in, survey, back up, boot the test build from the USB stick by kexec, run the pad player on it, and come back. The [stick test](firmware-3.10.1-stick-test.md) answered most of it before the install, with 3.6.0 doing the jump. This round does the same from the installed 3.10.1, in one sitting, with every step's undo below.

## What the update changed for the tools

- **Our own SSH (MOD-001) is gone.** The update rewrites the root filesystem, where its unit lived. MOD-002's folder `/data/mpfree/ssh` should still be on `/data`, which the update doesn't touch; nothing reads it now. The boot map lists the names in it.
- **The way in is Akai's developer mode:** root SSH on port 22. As Akai ships it, it needs no key and no password (`AuthenticationMethods none`). Akai's own lock-down is to empty `/data/ssh/sshd_config.d/allow-access.conf` and put a key in `/data/ssh/authorized_keys` ([what the image says](firmware-3.10.1-stick-test.md#phase-1-results-2026-11-13)).
- **Two ports in the tools.** [`collect-over-ssh.sh`](../../../tools/mpc-one/collect-over-ssh.sh) now reaches Akai's system on `MPC_PORT`, default 22, and the test build on the stick on `MPC_TEST_PORT`, default 2222. For a unit still on 3.6.0 with MOD-001, set `MPC_PORT=2222`.
- **New modes**, all in [survey tools](../guides/survey-tools.md):
    - `--devmode-trust`, `--devmode-status`, `--devmode-lock` and `--devmode-unlock` ([`host_devmode_login.py`](../../../tools/mpc-one/host_devmode_login.py)).
    - `--system-survey` ([`read-only-system-310.sh`](../../../tools/mpc-one/read-only-system-310.sh)).
- **The kexec test boot from an installed 3.10.1.**
    - The running kernel, `6.18.44-az01-2026-08-10-rt6`, is the one the test build boots. The stick builder accepts that: one `boot-files` folder serves both.
    - The boot step no longer requires 3.6.0's release on the MPC: it checks the release the stick was built against.
    - The jump uses the `kexec_load` system call (`kexec --load --kexec-syscall`), which takes the kernel from memory. 3.10.1's LoadPin pins files the kernel reads itself, such as `kexec_file_load`'s. That is our reading of the kernel; the round shows whether it holds.
- **The backup's baseline is still 3.6.0's**, the only one recorded. The update rewrote `mmcblk0p6` (the root filesystem), so the backup is expected to end with a `BASELINE MISMATCH` for p6 and exit code 3. Any other partition in that list means stop. After this round, a 3.10.1 baseline is made from its boot map and backup.

## What this round answers

| Question | Answered by |
|---|---|
| Does developer mode answer as the image says: port 22, root, no key, no password? | Q1 |
| Does Akai's lock-down to a key work on the installed system? | Q2 |
| Which ports are open with developer mode on (Akai's web server too), and is Wi-Fi on? | Q3 |
| Is kexec allowed on the installed 3.10.1 (`/usr/sbin/kexec`, `kexec_load_disabled`, lockdown)? | Q3, then the checks in Q7b |
| Did the update change any boot partition besides the root filesystem? | Q4 and the boot maps in Q7a and Q10 |
| Do developer mode, the lock and the SSH host key survive a restart? | Q7a, compared with Q3 |
| Does the jump from the installed 3.10.1 work, and does the hardware come up in the test build after it (Ethernet, screen, pads, sound)? | Q7b, Q8 |
| Does the MPC come back to 3.10.1 with the eMMC unchanged? | Q10 |

## You need

- **The MPC on the direct Ethernet cable only.** Until Q2 has locked it, anyone on any network the MPC is on can get a root shell with no password. If the MPC is on Wi-Fi, switch its Wi-Fi off in Akai's settings before Q1.
- Developer mode on: Menu > Preferences > Enhancements, switch on the developer panel; then hold SHIFT and press MENU, and choose Developer.
- About 8 GiB free in `~/mpfree-backups` for Q4, and a second disk for the copy the backup tool asks you to make.
- The USB stick from earlier rounds. Its partition 4 keeps the pad player's settings.

Every block runs in a terminal on the workstation, from the clone at `/home/jesse/code/mpfree-hardware-survey`. Each step's output lands in one results folder, and Q10 packs it into one zip.

## The blocks

**Block Q0: update, check, build** (workstation only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C qual-3101 origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_devmode_login.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_kexec_preflight.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_host_emmc_backup.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && python3 tools/mpc-one/tests/test_write_test_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_kexec_boot.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_app_mpc.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_read_stick_logs.py && python3 tools/mpc-one/tests/test_results.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app && cd rust/target/armv7-unknown-linux-gnueabihf/release && sha256sum mpfree-screen-log mpfree-startup mpfree'
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
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-status -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run system-survey -- bash tools/mpc-one/collect-over-ssh.sh --system-survey
```

- **The status** shows where `/` and `/data` come from and the `/etc` overlay. It shows the SSH server's settings as the server reads them, the lock, the kind of root's password field (never its value) and the open TCP ports. It also prints the host key's fingerprint and date, and how long the MPC has been on: Q7a prints them again after a restart. It ends `LOCKED: only your key gets in`.
- **The system survey** covers:
    - versions, CPU clocks and idle states, memory;
    - kexec and kernel security (`/usr/sbin/kexec`, `kexec_load_disabled`, lockdown, LoadPin);
    - network interfaces and open ports (Wi-Fi shows here);
    - sound and MIDI (cards, sequencer clients, `aconnect -l`, `amidi -l`), input devices, drives and the names at their top level;
    - screen and GPU devices, controller scripts, and Akai's app's thread count.

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

It takes the newest unpacked image (3.10.1) and the device tree from Q5. Its report must say `running release: 6.18.44-az01-2026-08-10-rt6 (recorded by the preflight; the same kernel as the test build's)`.

**Block Q6-write: write the stick** (**erases the stick's partitions 1 to 3**, keeps partition 4; you type `WRITE`; the stick in the workstation):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- python3 tools/mpc-one/write-test-stick.py
```

**On the MPC, after Q6-write:** switch the MPC off and put the stick into its USB-A port. Switch it on and wait for Akai's app.

- Answer No or Cancel to anything that offers to update, format or erase, and note what it asked.
- If developer mode is not on any more, note that, and switch it on again (SHIFT + MENU, Developer).

**Block Q7a: after the restart** (read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run status-after-restart -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run boot-map-before -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

- **Expect `LOCKED: only your key gets in`** and the same host key fingerprint as in Q3.
- **If it stops with `did not show the host key recorded`,** the MPC made a new host key at the restart. That is a finding, and nothing was changed. Run Q1 again: it prints the new fingerprint and the exact `ssh-keygen -R` line that removes the old one. Compare the new fingerprint on the MPC's screen, as in Q1, then run the printed line and Q1 once more, then Q7a.
- **If the logins say `nothing listens on that port`,** developer mode is off. Switch it on and run Q7a again.

**Block Q7b: start the test build** (**state-changing, RAM only**; you type `BOOT 3.10`):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run kexec-boot -- env MPFREE_ACCEPT_KEXEC_BOOT=YES bash tools/mpc-one/collect-over-ssh.sh --kexec-boot
```

- **Before it asks,** it checks Akai's system, read-only: the release the stick was built against, kexec allowed, lockdown, the stick plugged in and on USB, at least 64 MiB free in `/run`, and no unit or shutdown hook that reacts to kexec other than systemd's own. It records the eMMC write counters.
- **Then** it copies the kernel and device tree to `/run` and loads them (`kexec --load`), and asks you to type `BOOT 3.10`. Anything else unloads them and stops.
- **After `BOOT 3.10`,** Akai's system shuts down cleanly and jumps.
- **Passes when** it ends `RESULT: the test build is running from the stick, and it cannot see the eMMC or SD card.` The pad player starts on the screen after the startup screen.
- **If the jump fails,** systemd restarts Akai's system normally, and the step says the MPC is on Akai's system again. If nothing answers, the step says so: switch off, take out the stick, wait 10 s, switch on.

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

They show the screen, touch, pads, buttons, sound and partition 4 working after a jump from the installed 3.10.1. The check also prints when the pad player started playing, and the GPU facts it always reads.

**Q9 (optional): measuring in the same boot.** The benchmark and the GPU page-flip blocks need the test build running, and nothing else in this round depends on them. They come from their own threads' runbooks, rehearsed at this round's commit, and are given with the round.

**Block Q10: back to 3.10.1.** Switch the MPC off, take out the stick, wait 10 s and switch it on. Wait for Akai's app, and switch developer mode on again if it is off. Then (read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run status-after -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run boot-map-after -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

Then, with the stick in the workstation (read-only on the stick):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py zip
```

Upload the zip, with your notes from the MPC's screen. The reports quote Akai's files: upload them to this project only.

## Passes when

- Q1 records the host key; Q2 ends `LOCKED`; Q3, Q7a and Q10 end `LOCKED: only your key gets in`.
- Q4's backup is verified, and its baseline mismatch names only `mmcblk0p6`.
- Q7b ends with its `RESULT` line for a running test build, with no `PROBLEM` line.
- Q8's phases work as in the stick test's phase 2.
- The boot maps from Q7a and Q10 have the same boot-area hashes, with 0 sectors written to p1 to p6 and boot0/1 since the power-on in Q10.
- The stick logs show no failed unit we didn't expect and no `mpfree-test: blocked` line we can't explain.

## Undo, block by block

| Block | Changes | Undo |
|---|---|---|
| Q0, Q6 | New folders under `~/mpfree-work` and `~/mpfree-results`, and build output in the clone | Delete them |
| Q1 | One line for `10.42.0.141` in `~/.ssh/known_hosts` | `ssh-keygen -R 10.42.0.141 -f ~/.ssh/known_hosts` |
| Q2 | `/data/ssh` on the MPC | Block Q2-undo |
| Q4 | A new folder under `~/mpfree-backups` | Keep it: it is the return point for 3.10.1 |
| Q5 | A private folder under `~/mpfree-work` (it holds the serial number and MAC) | Delete it |
| Q6-write | The stick's partitions 1 to 3 | Rebuild and write an earlier stick; partition 4 stays |
| Q7b, Q8, Q9 | RAM on the MPC | Switch off, take out the stick, wait 10 s, switch on |

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

## What this round cannot show

- Whether developer mode, the lock and the host key survive a long power-off or Akai's next update.
- Anything about installing MPFree to start at power-on: that is a lasting change to Akai's files with its own gate ([stick test, after the test](firmware-3.10.1-stick-test.md#after-the-test)).

## After the round

- A 3.10.1 baseline for the backup tool, `tools/mpc-one/baselines/mpc-one-3.10.1-reference.txt`, from the Q7a boot map and the Q4 backup.
- The [compatibility page](firmware-3.10.1-compatibility.md) and the [developer access guide](../guides/developer-access.md) updated with what the installed unit showed.
- The results here, as in the stick test's phase sections.
