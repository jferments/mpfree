# The targeted app run on the soft-boot stick

**Status:** runbook, 2026-11-15. Not run yet: every block that touches the MPC waits for the owner's go. **Scope:** the reference MPC One with Akai's MPC OS 3.10.1 installed and developer mode on. **Boots by:** the soft-boot stick ([how it works](stick-autoboot.md#the-soft-reboot-hook-step-4)): switch on with the stick in and the boot hook starts MPFree. There is no kexec block; on the installed 3.10.1 the kernel refuses a kexec.

After live test 3 the owner asked for targeted runs: only the phases of the app check that have a reason to run, not every step every time. This run redoes the phases live test 3 skipped by accident and tries what has merged since, on a stick built from `main`.

## What it tests

The app check's phases ([the app guide](../guides/mpfree-app.md#the-live-test)), by their numbers there:

| Phase | Why it runs |
|---|---|
| 6: Arrange | Drag and drop on the clip grid and the Song view (#355); a clip now launches when the finger lifts |
| 7: TRACK MUTE | Pads 1-16 mute tracks 1-16, and PAD BANK pages through up to 128 tracks (#332) |
| 9: the Synth tab | Skipped in live test 3 |
| 10: Note Repeat and the arp | Skipped in live test 3 |
| 11: the Looper tab | Skipped in live test 3; also dragging a layer to another loop (#355) |
| 12: the mixer and effects | The Mix tab (#353), the effect page (#354), effects with modes (#329), and a learned Q-Link following a mode switch (#360) |
| 16: Prefs, System info and saving | Skipped in live test 3 |
| 17: a MIDI keyboard on MIDI IN | Skipped in live test 3; `s` skips it without a keyboard and a 5-pin cable |
| 18: Touch | Skipped in live test 3: the Touch lab, then two throws held at once with two fingers |
| 19: menus, the console and STOP | Skipped in live test 3 |

Phases 6, 7, 11 and 12 describe the new features in the app check's own text, so the terminal says what to try.

## You need

- The MPC on the direct Ethernet cable only, with developer mode on (Menu > Preferences > Enhancements switches on the developer panel; then SHIFT + MENU, Developer).
- Developer mode's SSH host key in `~/.ssh/known_hosts`. If it isn't there yet, run [Block Q1](firmware-3.10.1-qualification.md) once first (`--devmode-trust`, read-only on the MPC). Without it, block R3 stops before it runs anything on the MPC and says so.
- The USB stick from earlier rounds. Its partition 4 keeps the pad player's settings, among them the buttons Map controls found. If the buttons that phase 1 lists aren't green in Map controls, add phase 1 to the list in R5 (`MPFREE_APP_PHASES=1,6,7,9-12,16-19`).
- Turn the MPC's volume knob down before R4.

Every block runs in a terminal on the workstation, from the clone at `/home/jesse/code/mpfree-hardware-survey`. Each step's output lands in one results folder, and R6 packs it into one zip.

## The blocks

**Block R1: update, check, build** (workstation only; nothing contacts the MPC):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C app-targeted origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_boot_hook.py && python3 tools/mpc-one/tests/test_write_test_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_app_mpc.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app' && python3 tools/mpc-one/results.py run build-stick -- python3 tools/mpc-one/offline-kexec-stick.py --splash rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-startup --app rust/target/armv7-unknown-linux-gnueabihf/release/mpfree
```

This is block 4.1 of [the boot record](stick-autoboot.md#blocks-for-the-owner) with the app check's own tests added. `results.py status` lists what is already in the results folder: if an earlier round left files there, run `python3 tools/mpc-one/results.py zip` first, so this run gets its own zip. The checks end with `LOCAL-CHECKS-OK`. The build report says `for MPFree's soft-reboot hook: /boot/mpfree-softreboot on partition 1 (protocol mpfree-softreboot-1, kernel release 6.18.44-az01-2026-08-10-rt6)`, and its list of the units Akai's udev rules start shows none that is NOT masked and could touch a disk.

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

**Block R5: the app check's phases** (read-only on the MPC: the check starts, copies and stops nothing; it talks only to the stick's system on port 2222, and Akai's system, with its own host key, is refused; each phase starts and ends with Enter):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-targeted -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_PHASES=6,7,9-12,16-19 bash tools/mpc-one/collect-over-ssh.sh --app
```

Step 2 of the check must end with `CHECKS-OK` (it reads `kexec_load_disabled: 1`, `no MMC block device` and `/: read-only` on the stick's system). Then each phase prints what to try, waits for Enter, and asks what you saw. `s` skips a phase. If the terminal closes, run the same block again under another name (`app-targeted-resume` in place of `app-targeted`) and skip the phases already done with `s`: the check only reads, and its summary reads the app's whole log since power-on.

**Block R6: back to Akai's system, and what happened** (read-only). Switch the MPC off, take the stick out, wait 10 s and switch it on. With the hook installed, switching on with the stick still in starts MPFree again, so the stick must come out first. Wait for Akai's app, and switch developer mode on again if it is off. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-report -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --hook-report ; python3 tools/mpc-one/results.py run boot-map-after -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

Then, with the stick in the workstation (read-only on the stick):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py zip
```

This is block 4.5. Upload the zip, with your notes from the MPC's screen. The reports quote Akai's files: upload them to this project only.

### In the same boot as the qualification round

If the [qualification round](firmware-3.10.1-qualification.md) is run in the same sitting, R5 can run in its MPFree boot, after its Q8 (and Q9) and before its Q10, instead of R1 to R4. The round's Q8 already covers phases 9, 16 and 19, so the list is shorter:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-targeted -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_PHASES=6,7,10-12,17,18 bash tools/mpc-one/collect-over-ssh.sh --app
```

Then carry on with the round's Q10, which packs this block's log into the round's zip.

## Passes when

- R3 ends with `RESULT: the hook is installed, and the stick checks out.`
- In R4 the MPC goes from the logo to MPFree with no tap, and Akai's app never draws.
- R5's step 2 ends with `CHECKS-OK`, and each phase does what its text says, or your answer says what it did instead.
- The boot maps from R3 and R6 have the same boot-area hashes, with 0 sectors written to p1 to p6 and boot0/1.
- The stick's `dmesg.txt` shows `mpfree-internal-off: switched off ff0f0000...` before any `sd` or `mmc` mount, and no `Buffer I/O error` on `mmcblk`.

## Undo, block by block

| Block | Changes | Undo |
|---|---|---|
| R1 | New folders under `~/mpfree-work` and `~/mpfree-results`, and build output in the clone | Delete them |
| R2 | The stick's partitions 1 to 3 | Write an earlier build the same way; partition 4 stays |
| R3 | The boot hook's three files on the MPC's data partition | `--hook-remove` (R3 gives the line) |
| R4, R5 | Akai's programs stopped for this boot; MPFree runs in RAM from the stick; what you save in MPFree goes to the stick's partition 4 | Switch off, take the stick out, wait 10 s, switch on |

## Rehearsal

- The app check's own tests (`test_app_mpc.py`, run by R1) include this run's phase list on the stick's system as a soft-reboot leaves it: the stick's partition as a read-only root mounted `ro,noload`, Akai's `/run` carried over, `kexec_load_disabled` set by the stick's `sysctl.d`. The checks pass, only the chosen phases ask, and nothing is copied, started or stopped.
- R2, R3, R4 and R6 are blocks 4.2 to 4.5 of [the boot record](stick-autoboot.md#blocks-for-the-owner), word for word; the owner ran them for the power-on of 2026-11-15 that started MPFree (zip 092220Z). R1 is block 4.1 with the results-folder check and the app check's tests added.
- On that power-on the wrapper's identity check passed against the stick's system at the MPC's address (the hook report then found port 22 closed, as expected while MPFree runs), so R5's way in is the one the kexec test build had: port 2222 and the stick's own host key.
