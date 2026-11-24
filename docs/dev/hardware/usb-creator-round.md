# The USB creator round: a new musician's install, from developer mode off

**Status:** runbook, 2026-11-16. Not run yet: every block that touches the MPC waits for the owner's go. Run it from `main` once the pull request that adds this page has merged. **Scope:** the reference MPC One (Gen 1) on MPC OS 3.10.1, as the [3.10.1 device round](firmware-3.10.1-qualification.md) of 2026-11-16 and the [second 3.10.1 round](firmware-3.10.1-round-2.md) of 2026-11-19 leave it: developer mode on and not locked (the 18 Nov notes record it unlocked, and the second round leaves its lock out), MPFree's boot hook installed.

The USB creator's earlier rounds made sticks on Linux, installed the boot hook with one tap and backed the MPC up, but always from a lab MPC: developer mode was already on, and the last round installed the hook over SSH. This round follows a new musician's path from the start, with the window's own steps and nothing else: the MPC with no hook and developer mode off, a new stick made in the window, developer mode switched on, the stick put in, one tap. Since 2026-11-16 the setup script ends by switching developer mode off with Akai's own off script ([decision 10](../design/usb-creator.md#the-owners-decisions-2026-11-15), [developer mode afterwards](../design/usb-creator.md#developer-mode-afterwards)), so the round also shows whether the boot hook survives that, and whether developer mode stays off.

## What this round answers

| Question | Answered by |
|---|---|
| Does Akai's menu switch developer mode off as Akai's files say (deleting `/data/ssh` and `/data/root`, then restarting), and what does "off" look like from the network? | C3 |
| With developer mode switched on from off and the stick put in afterwards, does Akai's script runner offer `autorun.sh`? If not, does a power-on with the stick in? | C5 |
| Does one tap install the hook, switch developer mode off with Akai's `disable.sh`, and restart into MPFree, with no other step? | C5, C6, C9's `setup-log` |
| Does the first start make a fresh eMMC backup with `mpfree copy-image` through the read-only window, and does it match the last SSH backup where it must? | C6, C9 |
| Does the boot hook survive developer mode going off, at a restart and at a cold power-on? | C6, C7 |
| Does developer mode stay off at the next start of Akai's system, or does Akai's app switch it on again? | C8 |
| Does the window offer to copy the backup when the stick comes back to the computer? | C9 |
| Do the free CC0 kits (Big Rusty Drums, Swirly Drums, VCSL Percussion) play on every pad, start on time and sound right, and are they in BROWSE? | C6 |
| With the backup left out in the window, does the first start read Akai's kits and samples straight from the MPC, with the eMMC's write and discard counters still, and are they the kits a backup gives? Do Prefs' new Backup and Akai kits rows show it? | C11 |
| Optional: do the Windows and macOS writers make a stick that works? Does a Windows creator built and signed on the workstation run on Windows? | W0, W1a, W1, W2 |

## The round at a glance

| Part | Blocks | Where | About |
|---|---|---|---|
| Builds and the setup script's tests | C0 | Workstation | 15 min |
| Make the MPC a new musician's: the hook off, developer mode off | C1, C2, C3 | MPC on 3.10.1, the stick out | 10 min |
| The stick, made in the window | C4 (or W0, W1a and W1; or W2) | Workstation (W1 and W2 on a Windows PC or a Mac) | 10 min (W0's first run adds 15) |
| One tap, and the first start | C5, C6 | MPC | 35 min |
| A second power-on, then Akai's system | C7, C8 | MPC | 5 min |
| The window's reminder, the logs, the backup and the zip | C9 | Workstation | 20 min |
| Only if the next round needs developer mode's SSH | C10 | MPC on 3.10.1 | 5 min |
| Akai's kits without a backup | C11 | Workstation and MPC | 30 min (50 with its optional backup) |

The round asks for your password twice: once in the window in C4, and once in C9.

## You need

- **The MPC on the direct Ethernet cable only, with its Wi-Fi off.** From C5's switch-on until the tap, developer mode lets anyone on the MPC's network in as root with no password, as it does for any musician.
- **A spare USB stick of 16 GB or more**, which C4 erases. The first start's backup needs about 8 GB on it. If you have no spare, use the stick from 16 Nov and choose **Erase everything and start over** in C4: that erases its partition 4 too, with its copy of the 01:44Z backup (a copy is in `~/mpfree-backups`), the buttons Map controls found, and its projects and plugins.
- Akai's 3.10.1 update file in `~/mpfree-work/akai-update-files`, as in earlier rounds.
- About 8 GiB free in `~/mpfree-backups`, for C9's copy.
- **The stick out of the MPC until C5.** C2 takes the hook off, but until then a power-on with an MPFree stick in starts MPFree.

Every block runs in a terminal on the workstation, from the clone at `/home/jesse/code/mpfree-hardware-survey`. Each step's output lands in one results folder, and C9 packs it into one zip.

## The blocks

**Block C0: update, check, build** (workstation only; no sudo; about 15 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C creator-round origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_stick_setup.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app && cd rust/target/armv7-unknown-linux-gnueabihf/release && sha256sum mpfree-screen-log mpfree-startup mpfree' && python3 tools/mpc-one/results.py run build-helpers -- bash -c 'CARGO_ZIGBUILD_PYTHON_PATH=/home/jesse/.local/share/mpfree-zig/bin/python3 cargo zigbuild --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf.2.34 -p mpfree-mpcstore --bin mpfree-fetch -p mpfree-plughost --bin mpfree-plughost && sha256sum rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-fetch rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-plughost' && python3 tools/mpc-one/results.py run build-kits -- bash -c 'cargo run --manifest-path rust/Cargo.toml --release --locked -p mpfree-factory --bin mpfree-free-kits -- --cache /home/jesse/.cache/mpfree-free-kits --out rust/target/armv7-unknown-linux-gnueabihf/release/free-kits && sha256sum rust/target/armv7-unknown-linux-gnueabihf/release/free-kits' && python3 tools/mpc-one/results.py run build-creator -- bash -c 'cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-stick && cargo build --manifest-path rust/mpfree-creator/Cargo.toml --release --locked && sha256sum rust/target/release/mpfree-stick rust/mpfree-creator/target/release/mpfree-creator'
```

- `results.py status` lists what is already in the results folder. If an earlier round left files there, run `python3 tools/mpc-one/results.py zip` first, so this round gets its own zip.
- `test_stick_setup.py` runs the setup script's tests, among them the new ending: Akai's off script, the restart if it returns, and a developer's stick keeping developer mode on. It ends `LOCAL-CHECKS-OK`.
- `build-kits` makes the free CC0 kits as a release does: it fetches about 15 MB of samples from GitHub at the commits `rust/mpfree-factory/free-kits.txt` pins (once; they are kept in `~/.cache/mpfree-free-kits`), checks every file's SHA-256 and each library's CC0 licence, and lists each pad with its length. It ends with the kits file's hash.
- Then eight hashes: the three programs for the MPC, `mpfree-fetch` and `mpfree-plughost` (the Plugins place's download helper and the process each plugin runs in, both linked dynamically with `cargo zigbuild`; they need the 3.10.1 round's [block G1a](firmware-3.10.1-qualification.md) tools, installed on the workstation on 17 Nov. If C0 stops at `build-helpers` with `no such command: zigbuild`, run G1a, then C0 again), the free kits, `mpfree-stick` (the creator's command line, for C9) and the window. A developer's build of the window carries no programs inside it; it finds the five just built, and the free kits, in `rust/target/armv7-unknown-linux-gnueabihf/release`.
- **Undo:** none needed. The builds stay in the clone, and the fetched samples in `~/.cache/mpfree-free-kits` (about 15 MB; delete the folder if you like, and the next build fetches them again).

**Block C1: Akai's system as the last round left it** (read-only; the stick out; Akai's app up, developer mode on):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run status-before -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run hook-report-before -- bash tools/mpc-one/collect-over-ssh.sh --hook-report
```

- **Expect `OPEN: a login with no key and no password gets in`** (developer mode has been unlocked since 16 Nov; `LOCKED: only your key gets in` is fine too), and a hook report listing the hook's three files with `MPFree's stick in now: no`.
- **If it stops with `could not read the running system's state` and `Connection refused` above it,** developer mode is off already. Switch it on in the menu (SHIFT + MENU > Developer) and run C1 again. It then ends `OPEN`, which is fine for C2: C3 closes it again.
- **If it stops with `did not show the host key recorded`,** run the 3.10.1 round's [Q1](firmware-3.10.1-qualification.md) first, then C1 again.

**Block C2: take the boot hook off, so the MPC is a new musician's** (**state-changing, on `/data`**: deletes the hook's three files and the folders its install made, and nothing else; the stick out):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-remove -- bash tools/mpc-one/collect-over-ssh.sh --hook-remove
```

- **Passes when** it ends `RESULT: the hook is gone.` From now on a power-on starts Akai's system, stick in or not.
- **Undo:** C5's tap puts main's hook back. Without the tap, the 3.10.1 round's Q7b-ssh (`--hook-install`) does, over developer mode's SSH.

**Block C3: developer mode off, the way Akai's menu does it** (**state-changing, Akai's own switch**: per Akai's files it deletes `/data/ssh`, with developer mode's SSH settings in it (and the lock, if C1 showed one), and `/data/root`, then restarts the MPC; nothing of MPFree's is on the MPC now). As far as the records show, the switch hasn't been used on this MPC since 3.10.1 went on.

1. Hold **SHIFT** and press **MENU**, choose **Developer**, and switch developer mode off.
2. Note what the MPC does: whether it restarts by itself, and after how long. Wait for Akai's app.
3. Open SHIFT + MENU > Developer again and note what the switch shows.
4. Then, from the workstation (read-only on the MPC: it reads the SSH host key, if anything answers, and tries two logins that run `id -u` only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-off-menu -- bash tools/mpc-one/collect-over-ssh.sh --devmode-trust
```

What it can end with, and what each means:

| It ends | Meaning |
|---|---|
| `STOP: ssh-keyscan got no answer…` or `expected one ed25519 host key from 10.42.0.141, got 0` | Nothing answers on port 22: developer mode is off, and no SSH server runs |
| `no key, no password: refused` and `your key: refused` | An SSH server answers with Akai's own key-only settings, and developer mode's open login is gone: off |
| `no key, no password: logged in` | Developer mode is still on. Note it; switch it off in the menu again and run this block once more |
| `STOP: … already holds a different key` | Something answers on port 22 with a new host key. Don't remove the old key now; note it and go on. C10 sorts the key out if a later round needs it |

The `STOP` lines are answers here, not failures: the block is only asking whether developer mode is off.

**Block C4: the stick, made in the window** (**state-changing, on the stick only**: erases the whole stick; the stick in the workstation; one password prompt). Put only the spare stick in. Then open the window:

```bash
cd /home/jesse/code/mpfree-hardware-survey && rust/mpfree-creator/target/release/mpfree-creator
```

1. Step 1, **Akai's update file**: type `/home/jesse/mpfree-work/akai-update-files` in the box and press Enter. Expect a green line naming MPC 3.10.1 and Akai's build.
2. Step 2, **Your USB stick**: choose the spare stick. Leave **Preferences > Developer options** alone: this stick is a musician's, with no developer SSH and no developer tools. Step 3 must not say "Developer SSH is on".
3. Step 3: click **Make the stick**. The window asks "Erase this stick?": tick **Yes, erase this stick**, click **Erase and make the stick**, and give your password once. (With the 16 Nov stick, step 3 is **Update MPFree on this stick**: choose **Erase everything and start over**. The button then says **Make the stick**, and the same question follows.)
4. **Expect "The stick is ready"** after a few minutes, with six steps for the MPC. Steps 5 and 6 should read "A few seconds later the script adds MPFree's start-up helper, switches developer mode off again and restarts the MPC…" and "The next time you use Akai's software (stick out), check that developer mode is off". Note anything in the window that was unclear.
5. Close the window and eject the stick in the file manager.

- **If it says `This copy of the creator is missing MPFree's programs`,** main's stick needs a program C0 didn't build. Stop and send the results.
- **Undo:** the stick is the round's; make it again in the window, or erase it.

**Block C5: one tap, from developer mode off** (**state-changing, on `/data`**: the hook's three files; then Akai's off script deletes `/data/root` and `/data/ssh` and restarts the MPC). Follow the window's steps 3 to 5, as a musician would. Akai's app is up from C3, with the stick out.

1. The developer panel is already on (MENU > Preferences > Enhancements). Hold **SHIFT**, press **MENU**, choose **Developer** and switch developer mode on. From here until the tap, the MPC's root login is open with no password: keep going.
2. Put the stick into the MPC's USB-A port. **Note whether Akai's app asks to run `autorun.sh`, and after how many seconds.**
3. If it hasn't asked after a minute, switch the MPC off and on again with the stick in. With the hook off, Akai's system starts. Note whether it asks once Akai's app is up.
4. **Tap Yes.** Answer No or Cancel to anything else that offers to update, format or erase, and note what it asked.
5. **Expect,** a few seconds after the tap: the screen goes dark and the MPC restarts by itself (Akai's off script restarts it, about 5 s after the tap), then the MPC logo and MPFree's startup screen, not Akai's app. Note the time from the tap to the restart. Leave the stick in: C6 follows by itself.

- **If it never asks, even after the power-on,** take the stick out, switch developer mode off in the menu (it is open with no password), and go to C9: the setup log will be empty, and the missing offer is the finding.
- **If the MPC doesn't restart within a minute of the tap,** the script stopped and changed nothing, or the off script didn't restart. Wait 30 s more (the script restarts plainly 20 s after the off script returns), then take the stick out, switch developer mode off in the menu if it is on, and go to C9: `setup-log` says which.
- **Undo:** Q7b-undo of the 3.10.1 round removes the hook; developer mode is switched on again in Akai's menu.

**Step C6: the first start** (**state-changing**: at about 6 s after the restart Akai's programs stop and MPFree's system runs from the stick on Akai's kernel; the eMMC and SD card are switched off in software. MPFree writes only RAM and the stick, and its first start opens a read-only window on the eMMC to back it up onto partition 4; about 30 minutes). It is the 3.10.1 round's [Q7c](firmware-3.10.1-qualification.md), on a fresh partition 4:

- **Expect:** the startup screen about 16 s after the restart, with a card saying the stick's space is being made (seconds), then "Backing up your MPC, so you can always go back", "Checking the backup on the stick" and "Extracting Akai drum kits and samples", with one bar and the time left. Keep the stick in and the MPC on, and don't touch the power. Note the time each part started. Then the pad player opens with the MPC's own kits and samples first. Play a few pads.
- **The free kits:** pick each of the last three kits in the kit list, after MPFree's own (`MPFree Init Kit` to `MPFree Electro Kit`): **Big Rusty Drums**, **Swirly Drums** and **VCSL Percussion**. Play all 16 pads of each, softly and hard. Note any pad that is silent, clicks, starts late, is cut off early, is far louder or quieter than the rest, or doesn't sound like its name (each kit's pads are listed in [the design page](../design/usb-creator.md#the-free-cc0-kits-as-built)). Then open BROWSE > **Kits** > **MPFree**: the three are there as kit folders, with MPFree's five own kits.
- **That MPFree started at all shows the hook survived** Akai's off script: the files C5 wrote were there at the restart it made.
- **If Akai's app comes up instead,** the hook didn't start MPFree. Developer mode is off and its SSH lock gone, so the hook's notes can't be read now: note what the screen showed, then switch off, take the stick out, and go to C8. `setup-log` in C9 shows whether the hook was installed.
- **If the setup says it failed,** note its words. The pad player still opens, and the next start tries again (C7).
- **If the MPC switches itself off,** the stick's switch-off service or the backup's window refused to go on, and saved its notes to the stick. Go to C8.
- **If the screen stays on the logo for more than a minute,** hold the power button to switch off, and go to C8.
- **Undo, always:** switch off, take the stick out, wait 10 s, switch on. Akai's system starts. The backup, kits and samples stay on partition 4.

**Step C7: a second power-on with the stick in** (RAM on the MPC, and the stick; about 3 minutes):

1. Press the power button briefly and tap **Shut down** on MPFree's power card. Note how long the MPC takes to switch itself off.
2. Wait 10 s, and switch on with the stick still in.
3. **Expect** MPFree's startup screen and then the pad player, in about 16 s, with no setup this time. This is a cold power-on after developer mode went off: the hook still works without it.
4. Press the power button briefly and tap **Shut down** again.

**Block C8: Akai's system, the stick out: is developer mode still off?** (read-only) Take the stick out, wait 10 s, and switch on. Wait for Akai's app.

1. Open SHIFT + MENU > **Developer** and note what the switch shows. Don't change it yet.
2. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-after -- bash tools/mpc-one/collect-over-ssh.sh --devmode-trust
```

- Its endings mean what they meant in C3. **Off, as hoped:** the menu shows it off, and the block ends with no answer on port 22, or with both logins refused.
- **If it shows `no key, no password: logged in`, or the menu shows developer mode on,** Akai's app switched it on again at its start: Akai's off script alone isn't enough, and the musician's step 5 stays a real step. Switch it off in the menu now (the MPC is open with no password), and note what the menu showed. That decides whether the setup script keeps calling the off script.

**Block C9: the window's reminder, the stick's logs, the backup copied and compared, and the zip** (read-only on the stick; a new folder in `~/mpfree-backups`, about 7.3 GiB; one password prompt; about 20 minutes). Put the stick into the workstation.

1. Open the window again (it changes nothing until you click):

    ```bash
    cd /home/jesse/code/mpfree-hardware-survey && rust/mpfree-creator/target/release/mpfree-creator
    ```

    **Expect a card at the top of the first page,** "Keep a copy of your MPC's backup on this computer", with a **Copy it now** button: the window offers the copy because `~/MPFree backups` holds no backup. If an earlier round copied one there, the card stays hidden, which is right too; say which. Don't click it (the next step copies the backup to where the comparison finds it), and close the window.

2. Then the reads, the copy, the comparison and the zip:

    ```bash
    cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-read -- sudo sh -c 'C=rust/target/release/mpfree-stick; $C setup-log --device mpfree; $C inspect --device mpfree; $C logs --device mpfree; $C copy-backup --device mpfree --to /home/jesse/mpfree-backups' ; python3 tools/mpc-one/results.py run compare-backups -- python3 tools/mpc-one/compare-emmc-backups.py --dir /home/jesse/mpfree-backups ; python3 tools/mpc-one/results.py zip
    ```

- `setup-log` prints what `autorun.sh` did in C5. Expect `RESULT: MPFree's startup hook is installed. Restarting the MPC in 5 s; leave the stick in.`, then `switching developer mode off with Akai's /usr/lib/az0x/user-access/disable.sh, which restarts the MPC`. A line `… disable.sh ended (exit 0); restarting in 20 s if it has not already` after it is expected too: Akai's script asks for the restart and returns while the MPC shuts down.
- `inspect` shows the stick the window made (`made: MPFree stick, made by MPFree USB creator 0.1.0; …; developer SSH: off`), `setup mode: install`, and `backup: mpc-one-emmc-<UTC>` from C6.
- `logs` prints every boot's files from the stick's log partition, with MAC addresses and USB serial numbers hidden.
- `copy-backup` copies that backup into a `.part` folder in `~/mpfree-backups`, checks every image against its `SHA256SUMS`, then names the folder and makes it yours. It ends `done the backup is in /home/jesse/mpfree-backups/mpc-one-emmc-…`.
- `compare-emmc-backups.py --dir` compares it with the newest backup before it (the 3.10.1 round's Q4, made over SSH). The partition table, `uboot-spl`, `uboot`, the splashes, `rootfs` and both boot areas must be the same, and `env` normally is; `content` and `data` differ, because Akai's system writes them in use, and C2, C3 and C5 changed files on `data`. It exits 0 when everything that must be the same is.
- Then the zip. Upload it, with your notes from the MPC's screen. The backups hold your projects and the MPC's identifiers: they stay on your disks and never go into the zip or a repository.

**Block C10: only if the next round needs developer mode's SSH** (**state-changing, on `/data`**: the 3.10.1 round's Q1 and Q2 again). After this round developer mode is off and its lock is gone. Device rounds reach Akai's system over developer mode's SSH, so before the next one: stick out, Akai's app up, switch developer mode on in the menu (open with no password until the lock), then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-trust -- bash tools/mpc-one/collect-over-ssh.sh --devmode-trust && python3 tools/mpc-one/results.py run devmode-lock -- bash tools/mpc-one/collect-over-ssh.sh --devmode-lock && python3 tools/mpc-one/results.py run hook-report-after -- bash tools/mpc-one/collect-over-ssh.sh --hook-report ; python3 tools/mpc-one/results.py zip
```

- `--devmode-trust` says `already recorded: nothing to add` if the host key is the one from before. If it stops with `already holds a different key`, developer mode made a new host key: compare it on the MPC's screen as in [Q1](firmware-3.10.1-qualification.md), run the `ssh-keygen -R` line it prints, then the block again.
- `--devmode-lock` asks you to type `LOCK` and ends `LOCKED: the login with no key is refused and your key gets in`.
- The hook report lists the three files C5 wrote, with `MPFree's stick in now: no`.
- A stick made with developer tools (`--dev-tools`) or with a `keep-developer-mode` file on its `MPFREE` drive leaves developer mode on when its setup script runs; a musician's stick switches it off again.

**Block C11: Akai's kits without a backup, read straight from the MPC** (**state-changing, on the stick only**: erases it. On the MPC, a read-only window on the eMMC like C6's, opened for the content partition alone and checked by its write and discard counters; about 30 minutes; one password prompt in the window, one for the reads). It tries the window's new choice to leave the backup out (Jay, 17 Nov), and the new use of the read-only window that comes with it: MPFree reads Akai's kits and samples from the MPC itself, for about ten minutes, instead of from a backup on the stick ([the design](../design/usb-creator.md#leaving-the-backup-or-akais-kits-out)). Run it after C9, whose `stick-read` holds C6's kits lines for the comparison. The MPC has the hook from C5, so developer mode stays off.

1. **The stick.** A second spare stick of 16 GB or more if you have one. Otherwise the round's stick, but only after C9's `copy-backup` ended `done`: erasing it deletes the backup on it, whose checked copy is in `~/mpfree-backups`.
2. **In the window,** as in C4: step 1 the update, step 2 the stick, step 3 **Make the stick** (with the round's stick, **Update MPFree on this stick** shows: choose **Erase everything and start over**). Under "On the first start", **untick Back up my MPC** and leave **Bring in Akai's drum kits and samples** ticked. Make it. **Expect** "The stick is ready", with "MPFree copies your MPC's drum kits and samples into MPFree, reading them from the MPC without changing it. It takes about 10 minutes, once." Eject the stick.
3. **On the MPC:** switched off, put the stick in, switch on. **Expect** MPFree's startup screen, the card for the stick's space (seconds), then the setup: "Switching on the MPC's internal storage, read-only", then "Extracting Akai drum kits and samples" with one bar and the time left, about 10 minutes, and **no** "Backing up your MPC". Note when each started and when the pad player opened. Then play a few pads of two of Akai's kits, and open BROWSE > **Samples** > **Akai factory**.
4. **Prefs:** SHIFT + MENU, then the **System** tab's **Backup and Akai kits** card (before the Prefs redesign of 24 Nov: the row just above Touch screen). **Expect** "Backup of your MPC" to say **Left out** and "Akai's kits and samples" **In BROWSE**. Push the wheel on the backup row: it says **At the next start**, with a line saying so and that the power button's Restart starts it now. Push again: **Left out**. Push the wheel on the Akai row: a line says they are in already.
5. **Optional, about 20 minutes more: the backup asked for in Prefs** (**state-changing, on the stick only**: a backup on partition 4, through C6's window). On the backup row, push once (**At the next start**), then press the power button briefly and tap **Restart**. **Expect** the setup to run again with "Backing up your MPC" and "Checking the backup on the stick" only, no kits step, then the pad player; Prefs then shows "Made" and today's date. Skip it if C6's backup is enough for today.
6. Press the power button briefly and tap **Shut down**. Take the stick out, put it into the workstation, and read it:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-read-direct -- sudo sh -c 'C=rust/target/release/mpfree-stick; $C inspect --device mpfree; $C logs --device mpfree' ; grep -H -e 'first start: kits: .*SHA-256' -e 'first start: kit folders: .* layout ' -e 'mpfree-setup-backup: content read' -e 'mpfree-setup-backup: STOP' ~/mpfree-results/*/[0-9]*-stick-read.log ~/mpfree-results/current/[0-9]*-stick-read-direct.log ; python3 tools/mpc-one/results.py zip
```

- `inspect` shows the stick made by the window with `left out of the first start: the backup`, and `backup: none` (or the one from step 5).
- **The comparison:** `grep` prints the two lines C6's first start logged (from C9's `stick-read`) and the same two from this one: `first start: kits: N made, B bytes, SHA-256 …` and `first start: kit folders: N sounds, B bytes, layout …`. **They must be the same**, apart from the time at the start of each line: the same Akai content, read straight from the MPC, gives the same kits file and the same sounds in the same folders. Akai's system ran in C8 and C10, but it doesn't change its factory kits. A difference is a finding: send the zip, and the direct read stays off for musicians until it is understood.
- **The window:** `mpfree-setup-backup: content read in about N s; no write or discard on the eMMC` from this first start (in its `dmesg.txt`, and so in `stick-read-direct`), and no `STOP:` line. With step 5, also the backup's lines, as in C6.
- **If the setup says it failed,** note its words: with "the MPC's storage could not be read" or "was not opened for reading in time" the window never opened; with "closing the MPC's storage: …" it opened and the service found something wrong when it closed. The pad player still opens, and the next start tries again. Read the stick as in step 6 either way.
- **Undo:** the stick is the round's; make it again in the window with both boxes ticked, or erase it. Nothing on the MPC changes: the hook stays, as after C5.

## Optional: the Windows and macOS writers

These replace C4 when you have a Windows PC or a Mac to hand: the stick they make goes through C5 to C9 like the window's, which tests that writer to the end. Copy Akai's 3.10.1 update file from `~/mpfree-work/akai-update-files` into that computer's **Downloads** first, any way but on the stick you're about to erase.

The creators come from one of two places:

- **The draft release**, from the [release workflow](../design/usb-creator.md#releases): on GitHub, **Actions > USB creator release > Run workflow** on `main`, with the Windows and macOS boxes ticked. That uses GitHub's hosted runners, about 180 counted minutes of the account's monthly allowance, which ran out on 2026-11-11 and returns with the next billing month. The draft, `creator-v0.1.0`, is under **Releases**. Its Windows creator is unsigned.
- **For Windows, block W0 on the workstation**, with no GitHub minutes. It cross-builds the Windows creator with zig and signs it with a test certificate. It is the same Rust code as the release's, linked with zig's MinGW C runtime instead of Microsoft's, so it tests the creator, not the release job. There is no such route for the Mac: it needs the draft release.

**Block W0a: the signing tools and a test certificate, once** (workstation only; one sudo prompt). It installs `osslsigncode`, adds Rust's Windows target and makes a self-signed code-signing certificate in `~/.local/share/mpfree-signing`, unless one is there already. openssl asks twice for a passphrase, which locks the key. The key never leaves the workstation. The build also needs the 3.10.1 round's [block G1a](firmware-3.10.1-qualification.md) tools, as C0 does.

```bash
sudo apt install osslsigncode && rustup target add x86_64-pc-windows-gnu && mkdir -p -m 700 /home/jesse/.local/share/mpfree-signing && cd /home/jesse/.local/share/mpfree-signing && (test -e test-signing.key || openssl req -x509 -newkey rsa:3072 -sha256 -days 365 -keyout test-signing.key -out test-signing.crt -subj "/CN=MPFree test signing" -addext "extendedKeyUsage=codeSigning" -addext "keyUsage=critical,digitalSignature" -addext "basicConstraints=critical,CA:FALSE") && openssl x509 -in test-signing.crt -noout -subject -enddate -ext extendedKeyUsage
```

Expect `subject=CN=MPFree test signing`, an end date a year after the certificate was made, and `Code Signing`.

**Block W0b: build the Windows creator** (workstation only; no sudo; a few minutes). Run it after C0, which builds the programs it carries:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run build-windows -- bash -c 'set -e; rel=rust/target/armv7-unknown-linux-gnueabihf/release; progs=/home/jesse/mpfree-work/windows-creator/stick-programs; mkdir -p "$progs" && cp -f "$rel/mpfree" "$rel/mpfree-startup" "$rel/mpfree-screen-log" "$rel/mpfree-fetch" "$rel/mpfree-plughost" "$rel/free-kits" "$progs/" && sha256sum "$progs"/* && CARGO_ZIGBUILD_PYTHON_PATH=/home/jesse/.local/share/mpfree-zig/bin/python3 MPFREE_CREATOR_PROGRAMS="$progs" RUSTFLAGS="-C target-feature=+crt-static" cargo zigbuild --manifest-path rust/mpfree-creator/Cargo.toml --release --locked --target x86_64-pc-windows-gnu && exe=rust/mpfree-creator/target/x86_64-pc-windows-gnu/release/mpfree-creator.exe && file "$exe" && objdump -p "$exe" | grep "DLL Name" | sort -u && sha256sum "$exe"'
```

- Expect six hashes that match C0's, then `PE32+ executable for MS Windows 6.00 (GUI), x86-64`.
- The DLL list should be all Windows' own, with no libgcc or libwinpthread. On 17 Nov it was kernel32, ntdll, ole32, opengl32, shell32, user32 and uxtheme: the C runtime is linked in (`crt-static`), so no `api-ms-win-crt-…` either.
- The `.exe`'s hash comes last.

**Block W0c: sign it** (workstation only; asks for the key's passphrase):

```bash
cd /home/jesse/code/mpfree-hardware-survey && out=/home/jesse/mpfree-work/windows-creator/MPFree-USB-Creator-0.1.0-windows-x64.exe && tmp=$(mktemp -u /home/jesse/mpfree-work/windows-creator/signing-XXXXXX.exe) && osslsigncode sign -certs /home/jesse/.local/share/mpfree-signing/test-signing.crt -key /home/jesse/.local/share/mpfree-signing/test-signing.key -askpass -h sha256 -n "MPFree USB Creator" -i https://github.com/jferments/mpfree -in rust/mpfree-creator/target/x86_64-pc-windows-gnu/release/mpfree-creator.exe -out "$tmp" && mv -f "$tmp" "$out" && python3 tools/mpc-one/results.py run sign-windows -- bash -c "osslsigncode verify -CAfile /home/jesse/.local/share/mpfree-signing/test-signing.crt -in $out && sha256sum $out"
```

Expect `Signature verification: ok`, `Number of verified signatures: 1` and `Succeeded`, then the signed file's hash, which W1a checks on Windows. `Timestamp is not available` is expected too: the test signature has no timestamp, so Windows treats it as valid only while the certificate is (a year). osslsigncode won't write over a file, hence the temporary name. A wrong passphrase stops it with nothing written: run it again. Copy `~/mpfree-work/windows-creator/MPFree-USB-Creator-0.1.0-windows-x64.exe` into the PC's **Downloads**.

**W0 outside a round.** When the clone is on another round's branch, build in a separate folder at `main`, with its own results folder, so that branch and its results stay as they are. Start with:

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && (test -d /home/jesse/code/mpfree-windows-test || git worktree add --detach /home/jesse/code/mpfree-windows-test origin/main) && cd /home/jesse/code/mpfree-windows-test && git switch --detach origin/main && git log --oneline -1 && export MPFREE_RESULTS_DIR=/home/jesse/mpfree-results-windows
```

Then, in the same terminal, run C0's `build-arm`, `build-helpers`, `build-kits` and `build-creator` steps, W0b, W0c and, after W1, the inspect step below, each with `cd /home/jesse/code/mpfree-windows-test` in place of the clone's folder. The results go to `~/mpfree-results-windows`, and `results.py zip` packs them there. Undo: `cd /home/jesse/code/mpfree-hardware-survey && git worktree remove --force /home/jesse/code/mpfree-windows-test`.

**Block W1a: check the file on Windows** (read-only; PowerShell):

```powershell
cd $HOME\Downloads; Get-FileHash .\MPFree-USB-Creator-0.1.0-windows-x64.exe; Get-AuthenticodeSignature .\MPFree-USB-Creator-0.1.0-windows-x64.exe | Format-List Status, StatusMessage, SignerCertificate
```

- The hash should be W0c's, or the draft's `SHA256SUMS` line.
- W0's file: `Status: UnknownError`, with "…root certificate which is not trusted", signer `CN=MPFree test signing`. Windows read the signature and found it intact, but doesn't trust a test certificate. `NotSigned` or `HashMismatch` means the file changed on the way: stop there.
- The draft release's file: `NotSigned`, until the release is signed.

Then put the stick in and run `& .\MPFree-USB-Creator-0.1.0-windows-x64.exe --cli disks 2>&1 | Out-String`. The creator is a windowed program, so PowerShell shows its output only through `Out-String`. Expect the stick with its size, then `done 1 USB disk(s)`.

**Block W1: Windows** (**state-changing, on the stick only**: erases it; one "Do you want to allow this app to make changes?" prompt). Double-click `MPFree-USB-Creator-0.1.0-windows-x64.exe`. Windows shows "Windows protected your PC": click **More info**, then **Run anyway**. Follow C4's steps 1 to 4; step 1 should find the update in Downloads by itself. Note any console window that flashes up, and anything that stops. Then **Eject** the stick in Explorer.

**Block W2: macOS** (**state-changing, on the stick only**: erases it; one password prompt). Download `MPFree-USB-Creator-0.1.0-macos.zip`, open it, and move **MPFree USB Creator** to Applications. Open it; macOS refuses the first time. Open **System Settings > Privacy & Security**, click **Open Anyway** next to its name and enter your password. Follow C4's steps 1 to 4. If it says it could not unmount the stick, close any Finder window showing it and try again. Then eject the stick in Finder.

**Then, on the workstation** (read-only on the stick; one password prompt), before C5:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-made-elsewhere -- sudo rust/target/release/mpfree-stick inspect --device mpfree
```

It should list partitions 1 to 5, `made: MPFree stick, made by MPFree USB creator 0.1.0; …; developer SSH: off` and `setup mode: install`. A stick that inspects cleanly here goes on to C5.

**Seen on 2026-11-17** (W0 outside a round, at `main` `a96a6f5`, then W1 on the owner's Windows 11 Home PC, 64-bit, AMD Ryzen 5). C0's `build-fetch` first stopped at `no such command: zigbuild`, as its note says; G1a's tools (cargo-zigbuild 0.24.1, ziglang 0.17.1) fixed it. W0b's five programs matched C0's hashes, the creator was `PE32+ … (GUI), x86-64` with only Windows' own DLLs, and W0c's signature verified against the test certificate. W1 made the stick on Windows with no other step; `stick-made-on-windows` (the inspect step) showed the creator, developer SSH off and `setup mode: install`; and the stick's first start backed the MPC up (`mpc-one-emmc-20261117T173519Z`) and started MPFree. W1a's PowerShell output wasn't kept. Not tried yet: a release draft's own Windows build, and the Mac (W2).

## Passes when

- C0 ends `LOCAL-CHECKS-OK` and prints eight hashes.
- C2 ends `RESULT: the hook is gone.`
- C3 and C8 end with no answer on port 22 or with both logins refused, and the menu shows developer mode off.
- C4 ends with "The stick is ready" and the new steps 5 and 6.
- In C5 Akai's app offered `autorun.sh` after the stick went in (or, failing that, at the power-on with it in), and one tap restarted the MPC into MPFree with no other step.
- C6's first start ran to its end: the backup, its check, the kits and samples, then the pad player.
- C6: the three free kits play on all 16 pads, and BROWSE's **Kits > MPFree** holds them. A pad that sounds wrong doesn't fail the round: note it, and it is changed in `free-kits.txt`.
- C7's cold power-on started MPFree with no setup.
- C9's `setup-log` has the `RESULT: … installed` and `switching developer mode off` lines; `copy-backup` ends `done`; `compare-emmc-backups.py` exits 0; the window showed its **Copy it now** card (or had a copy already).
- The first start's `dmesg.txt` in the stick logs has the backup's window in order, as the 3.10.1 round lists, with `mpfree copy-image` doing the copy.
- C11's first start brought Akai's kits in with no backup step, its kits and layout lines match C6's, the kernel log says `content read in about N s; no write or discard on the eMMC` with no `STOP:` line, and Prefs showed **Left out** and **In BROWSE**.

## Undo, block by block

| Block | Changes | Undo |
|---|---|---|
| C0 | Build output in the clone; a new `~/mpfree-results/current` | Delete them |
| C2 | The hook's three files on `/data`, deleted | C5 puts them back; or the 3.10.1 round's Q7b-ssh |
| C3 | Akai's switch: `/data/ssh` (your lock and key) and `/data/root` deleted, developer mode off | Switch developer mode on in the menu; C10 locks it again |
| W0a | `osslsigncode`; Rust's Windows target; the test certificate and its key in `~/.local/share/mpfree-signing` | `sudo apt remove osslsigncode && rustup target remove x86_64-pc-windows-gnu && rm -r /home/jesse/.local/share/mpfree-signing` |
| W0b, W0c | `~/mpfree-work/windows-creator`; build output in `rust/mpfree-creator/target/x86_64-pc-windows-gnu` | Delete them; on Windows, delete the `.exe` from Downloads |
| C4, W1, W2 | The whole stick | Make it again, or erase it |
| C5 | The hook's three files on `/data`; developer mode off again; `setup-log.txt` on the stick's `MPFREE` drive | The hook stays: it is how MPFree starts. To take it off, the 3.10.1 round's Q7b-undo, or a removal stick from the window's **Take MPFree off my MPC** tab |
| C6 | `backup/mpc-one-emmc-<UTC>/` (about 7.3 GiB), the Akai kits and samples, and `settings/setup.conf` on the stick's partition 4. Nothing on the MPC | Keep them: the backup is the MPC's |
| C6, C7 | RAM on the MPC; the stick's log partition and partition 4 | Switch off, take the stick out, wait 10 s, switch on |
| C9 | A new folder in `~/mpfree-backups` | Keep it with the others, or delete it: the stick keeps its own |
| C10 | `/data/ssh` locked to your key again | The 3.10.1 round's Q2-undo |
| C11 | The whole stick (erased and made again); on its partition 4, Akai's kits and samples, the settings and, with step 5, a backup. Nothing on the MPC | Make the stick again with both boxes ticked, or erase it. A backup of the MPC is in `~/mpfree-backups` from C9 |

## What this round cannot show

- Other MPC Ones, or another release of Akai's software: the stick, the hook and the setup script refuse anything but 3.10.1 on an MPC One.
- Whether the hook's files survive Akai's next update. Either way the hook refuses a stick made for another release, and Akai's system starts.
- What a musician's MPC on Wi-Fi does while developer mode is on: this one is on the direct cable only.
- The Windows and macOS writers, unless W1 or W2 ran. A W0 build doesn't show the release's own Windows build, which is linked with Microsoft's compiler on GitHub, or how Windows treats a trusted signature.
