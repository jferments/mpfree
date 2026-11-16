# The targeted app run on the soft-boot stick

**Status:** runbook, 2026-11-15, revised that night for [targeted device checks](device-checks.md): R5 names its checks instead of the app check's numbered phases, and R1b runs the app's own tests. Not run yet: every block that touches the MPC waits for the owner's go. **Scope:** the reference MPC One with Akai's MPC OS 3.10.1 installed and developer mode on. **Boots by:** the soft-boot stick, the default route ([how it works](soft-reboot-boot.md)): switch on with the stick in and the boot hook starts MPFree. There is no kexec block run over SSH; on the installed 3.10.1 the kernel refuses it. The opt-in routes on `main` up to #374 (the pad player without root, `--app-user`; Akai's own kernel by kexec_file, `--kexec-file`; and the chain to the stick's own device tree, `--chain`) are covered in [On the opt-in routes](#on-the-opt-in-routes); see [boot routes](boot-routes.md) for what each one is.

After live test 3 the owner asked for targeted runs: only what has a reason to run, not every step every time. On 15 Nov he asked again: ask only about what a round tests, and leave to the app's own UI smoke tests whatever they can check. This run tries on the MPC what has merged since live test 3 and what that test never reached, on a stick built from `main`, and runs the app's tests for everything else.

## What it tests

The [device checks](device-checks.md) this run names, in the order R5 runs them (`python3 tools/mpc-one/app_checks.py show NAME` prints one in full):

| Check | Why it runs | About |
|---|---|---|
| `transport` | #387: REC, OVERDUB, UNDO and PLAY START work and light from the start, with no Map controls | 2 min |
| `basics` | The soft-boot route: pads, PAD BANK, FULL LEVEL, the wheel, the Q-Links (on Play, the Mix tab and the chain editor) and touch reach the app | 3 min |
| `track-mute` | Pads 1-16 mute tracks 1-16 with their colours, and PAD BANK pages through up to 128 tracks (#332) | 3 min |
| `wheel` | The outline and the push on the real wheel (#339), and a Song block moved by wheel (#355) | 3 min |
| `touch` | The Touch lab, Calibrate touch, two throws at once, and a real drag on the clip grid (#355) | 6 min |
| `note-repeat` | Never reached on the MPC in live test 3 | 2 min |
| `looper` | Live test 3's loop that kept playing after STOP | 2 min |
| `console` | The console chord, TAP TEMPO and STOP on the real panel | 1 min |
| `midi-in` | Never reached in live test 3; `s` skips it without a keyboard and a 5-pin cable | 3 min |

The rest of what live test 3 skipped or what has merged since is software the app's tests drive as a finger, the buttons and the wheel would, and R1b runs them: Arrange and its drag and drop, the Synth tab's engines, the Looper tab's layers and drags, the Mix tab with the effect page, effects with modes and a learned Q-Link that follows a mode switch (#353, #354, #329, #360), and Prefs with its saving.

## You need

- The MPC on the direct Ethernet cable only, with developer mode on (Menu > Preferences > Enhancements switches on the developer panel; then SHIFT + MENU, Developer).
- Developer mode's SSH host key in `~/.ssh/known_hosts`. If it isn't there yet, run [Block Q1](firmware-3.10.1-qualification.md) once first (`--devmode-trust`, read-only on the MPC). Without it, block R3 stops before it runs anything on the MPC and says so.
- The USB stick from earlier rounds. Its partition 4 keeps the pad player's settings, among them the buttons Map controls found. If a check uses a button it hasn't found yet, R5 says so and runs `map-controls` first.
- Turn the MPC's volume knob down before R4.

Every block runs in a terminal on the workstation, from the clone at `/home/jesse/code/mpfree-hardware-survey`. Each step's output lands in one results folder, and R6 packs it into one zip.

## The blocks

**Block R1: update, check, build** (workstation only; nothing contacts the MPC):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C app-targeted origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_boot_hook.py && python3 tools/mpc-one/tests/test_write_test_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_app_mpc.py && python3 tools/mpc-one/tests/test_app_checks.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app' && python3 tools/mpc-one/results.py run build-stick -- python3 tools/mpc-one/offline-kexec-stick.py --splash rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-startup --app rust/target/armv7-unknown-linux-gnueabihf/release/mpfree
```

This is block 4.1 of [the boot record](stick-autoboot.md#blocks-for-the-owner) with the app step's own tests added. `results.py status` lists what is already in the results folder: if an earlier round left files there, run `python3 tools/mpc-one/results.py zip` first, so this run gets its own zip. The checks end with `LOCAL-CHECKS-OK`. The build report says `for MPFree's soft-reboot hook: /boot/mpfree-softreboot on partition 1 (protocol mpfree-softreboot-1, kernel release 6.18.44-az01-2026-08-10-rt6)`, and its list of the units Akai's udev rules start shows none that is NOT masked and could touch a disk. This is the default stick: the soft-reboot, with the pad player as root. To run the checks with the pad player as `mpfree` instead (`--app-user`, #372, opt-in), add ` --app-user` at the end of the command, as [block 5.2](stick-autoboot.md#the-pad-player-without-root---app-user-opt-in) does; the report then also says `pad player user: mpfree (--app-user)`, and R2 to R6 stay as they are.

**Block R1b: the app's own tests** (workstation only, read-only: it builds into `rust/target`; about 6 minutes). The app's whole test suite at R1's commit, the UI smoke workflows among them:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-tests -- python3 tools/mpc-one/app_checks.py smoke
```

It reports the software side of each device check and of each area checked only by the tests, and ends `RESULT: every test passed.` If a test fails, it names it: go on with the run, and say so with the zip.

**Block R2: write the stick** (**state-changing, the stick only**: erases its partitions 1 to 3, keeps partition 4 with the pad player's settings; asks for `WRITE`; the stick in the workstation). Undo: write an older build to the stick the same way.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- python3 tools/mpc-one/write-test-stick.py
```

This is block 4.2.

**Block R3: boot map before, then the boot hook and its check of the new stick** (the boot map is read-only; the install is **state-changing, the data partition only**: it writes the hook's three files, or finds them already in place, then checks the stick read-only). Switch the MPC on without the stick, wait for Akai's app, then put the stick in the MPC. Run it even if the hook is already installed: its last step checks this stick the way the hook will at power-on.

- Undo: `cd /home/jesse/code/mpfree-hardware-survey && MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --hook-remove`. It deletes the three files and the folders the installs made. By hand: `rm /data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf /data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules; rm -r /data/mpfree-hook`.
- If it is cut off halfway: each file is written under a temporary name and renamed, the script first, so nothing points at a half-written script. Run it again to finish.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run boot-map-before -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --boot-map && python3 tools/mpc-one/results.py run hook-install -- env MPC_PORT=22 MPFREE_ACCEPT_HOOK=YES bash tools/mpc-one/collect-over-ssh.sh --hook-install
```

This is block 4.3. It must end with `RESULT: the hook is installed, and the stick checks out.` If it stops instead, take the stick out before the next power-on and send the results.

**Step R4: switch on into MPFree** (**state-changing**: at about 6 s Akai's programs are stopped and MPFree runs from the stick; the eMMC and SD card are switched off in software, and nothing is written to them). Switch the MPC off, wait 10 s, and switch it on with the stick in. Expected: the MPC logo, then MPFree's startup screen and the pad player, about 16 s after you switch on, with Akai's app never drawing. Play a few pads.

- **If Akai's app comes up instead, leave the stick in and the MPC on**, and run this read-only block first. The hook's notes are in RAM and only last until the power-off:

  ```bash
  cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-report-stick-in -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --hook-report
  ```

  Then go to R6; R5 needs MPFree running.
- If the MPC switches itself off, the switch-off service refused to go on. Go to R6.
- If the screen stays on the logo for more than a minute, hold the power button to switch off.
- Don't touch the MPC's power while it shows an update screen.

This is step 4.4.

**Block R5: the checks** (read-only on the MPC: the tool starts, copies and stops nothing and only reads the app's log; it talks only to the stick's system on port 2222, and Akai's system, with its own host key, is refused; about 25 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-targeted -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_CHECKS=transport,basics,track-mute,wheel,touch,note-repeat,looper,console,midi-in bash tools/mpc-one/collect-over-ssh.sh --app
```

Step 2 of the tool must end with `CHECKS-OK` (it reads `kexec_load_disabled: 1`, `no MMC block device` and `/: read-only` on the stick's system). Step 3 reports the start-up from the log, with no question, and its `privileges:` lines say who the pad player runs as: `Uid: 0` on the default stick, `Uid: 960` with `--app-user`. Then each check prints its steps and waits for Enter (`s` skips it, `q` stops here), reads the app's log while you do them, prints `seen`, `NOT SEEN` or `unclear` for each thing it looked for, and asks at most one question. If the terminal closes, run the same block again under another name (`app-targeted-resume` in place of `app-targeted`) with only the checks not done yet: the tool only reads, and its summary reads the app's whole log since power-on.

**Block R6: back to Akai's system, and what happened** (read-only). Switch the MPC off, take the stick out, wait 10 s and switch it on. With the hook installed, switching on with the stick still in starts MPFree again, so the stick must come out first. Wait for Akai's app, and switch developer mode on again if it is off. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-report -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --hook-report ; python3 tools/mpc-one/results.py run boot-map-after -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

Then, with the stick in the workstation (read-only on the stick):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py zip
```

This is block 4.5. Upload the zip, with your notes from the MPC's screen. The reports quote Akai's files: upload them to this project only.

### On the opt-in routes

R5 does not depend on how MPFree was started: it needs the stick's system answering SSH on port 2222, and its read-only checks passing. So it also runs on a stick from the opt-in blocks of [the boot record](stick-autoboot.md), built from `main`, in place of R1 to R4:

| Stick | Built and started by | R5 |
|---|---|---|
| `--app-user` | Blocks 5.2 to 5.4 | Runs; its `privileges:` lines show `Uid: 960` |
| `--app-user --kexec-file --chain` | Blocks 7.1 to 7.4 | Runs, once the second restart is done (about half a minute after power-on) |
| `--kexec-file` without `--chain` | Blocks 6.1 to 6.5 | Doesn't: on that route Ethernet doesn't come up (zip 192320Z), so the check can't reach the stick |

After R5, finish with R6 on every route: switch off, take the stick out, switch on into Akai's system, then the reports and the zip. On the chain route block 7.5 is the same as R6, so run one of them, not both.

### In the same boot as another round

The [3.10.1 device round](firmware-3.10.1-qualification.md#this-rounds-checks) runs all of these checks in its Q8, so it needs nothing from here. To add checks to another round's MPFree boot, run R5 before that round's last power-off, naming only the checks it didn't run; that round's zip then packs R5's log.

## Passes when

- R3 ends with `RESULT: the hook is installed, and the stick checks out.`
- In R4 the MPC goes from the logo to MPFree with no tap, and Akai's app never draws.
- R1b ends `RESULT: every test passed.`
- R5's step 2 ends with `CHECKS-OK`, and each check has every line `seen` and an answer that says it looked and sounded right; a `NOT SEEN` line or an answer that says otherwise is a finding.
- The boot maps from R3 and R6 have the same boot-area hashes, with 0 sectors written to p1 to p6 and boot0/1.
- The stick's `dmesg.txt` shows `mpfree-internal-off: switched off ff0f0000...` before any `sd` or `mmc` mount, and no `Buffer I/O error` on `mmcblk`.

## Undo, block by block

| Block | Changes | Undo |
|---|---|---|
| R1, R1b | New folders under `~/mpfree-work` and `~/mpfree-results`, and build output in the clone | Delete them |
| R2 | The stick's partitions 1 to 3 | Write an earlier build the same way; partition 4 stays |
| R3 | The boot hook's three files on the MPC's data partition | `--hook-remove` (R3 gives the line) |
| R4, R5 | Akai's programs stopped for this boot; MPFree runs in RAM from the stick; what you save in MPFree goes to the stick's partition 4, and buttons `map-controls` finds go to `settings/map.conf` there | Switch off, take the stick out, wait 10 s, switch on; keep the buttons, or delete `settings/map.conf` on partition 4 to start again from the built-in numbers |

## Rehearsal

- The app step's own tests (`test_app_mpc.py` and `test_app_checks.py`, run by R1) include named checks on the stick's system as a soft-reboot leaves it: the stick's partition as a read-only root mounted `ro,noload`, Akai's `/run` carried over, `kexec_load_disabled` set by the stick's `sysctl.d`. The checks pass, only the named checks ask, each judges its lines from a fake log, and nothing is copied, started or stopped.
- R2, R3, R4 and R6 are blocks 4.2 to 4.5 of [the boot record](stick-autoboot.md#blocks-for-the-owner), word for word as they are on `main` after #374. The owner ran them for the power-on of 2026-11-15 that started MPFree (zip 092220Z). R1 is block 4.1 with the results-folder check and the app check's tests added.
- R1b and R5 were rehearsed word for word at the owner's paths, as an ordinary user, at `ec8b3531` merged with #384: R1b is the round's Q0d (`630 tests ran: 630 passed`, `RESULT: every test passed.`), and R5 against the fake test build put `map-controls` first for the buttons it lacked, ran the nine checks with their one question each, and ended `RESULT: all steps ran`.
- R5's command is block 5.1's (`MPFREE_APP_STEPS=app` with `--app`) with this run's checks. On the MPC that command passed its checks and read the pad player's state after a soft-reboot with `--app-user` (zip 183259Z, `privileges-after`) and on the chain route (zip 201107Z, `privileges-chain`), so the way in, port 2222 with the stick's own host key, works on both.
