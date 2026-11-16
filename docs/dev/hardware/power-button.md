# The power button: clean shut down and restart

**As of 2026-11-15** · **Status:** built, waiting for its first on-device check (below) · **Asked for by** Jay, 15 Nov 22:10Z: "I want the power button to work like it does on the MPC" · **Card approved as drawn** 22:24Z

A short press of the MPC One's power button opens MPFree's **power card** over any page: Shut down, Restart or Cancel, with a save first over unsaved work. Holding the button down is still the hard power-off. This page explains how the press reaches MPFree, how the pad player (which runs without root) gets the MPC switched off or restarted, and what a restart starts on the soft-boot stick. The musician's side is in the manual ([Switching off and restarting](../../user/src/getting-started/before-you-start.md#switching-off-and-restarting)); the card's layout is in the [layout spec](../design/layout-spec.md#6-the-buttons-everywhere).

## The button

The power button is not on the controller's USB MIDI like the other buttons. It is a GPIO key: the kernel's `gpio-keys` input device (`/dev/input/event1` on the reference unit), whose only key is `KEY_POWER` (116) ([m07 validation record](../milestones/m07-validation-record.md#actual-installed-ili2117-input-driver-and-multitouch-bits)). Under Akai's system both Akai's app and the separate `az01-power-button.service` read it.

Before this change nothing on the stick acted on a short press, so the only way off was holding the button. Who does the hard power-off on a hold is **not measured**: it is either the PMIC's own long press or Akai's `az01-pwrbtn` (Akai's unit is not masked on the stick, and is described in the validation record as its emergency shutdown handler). It doesn't matter for this design, which leaves the hold alone, but the on-device check below records which units are running.

`rust/mpfree-app/src/power.rs`:

- finds the device by **name and key**, not by number: a `gpio-keys` device whose `capabilities/key` bitmap has bit 116, read in the kernel's word width;
- reads it on a thread of its own (`mpfree-power`), **read-only and never grabbed** (no `EVIOCGRAB`), so anything else reading the device still sees every press. If the device isn't there yet it looks again every 2 s;
- counts a press as **short** when the button comes back up within 1.5 s of going down, by the kernel's own timestamps. A hold of any length is never a press, so the hard power-off can't also open the card. Auto-repeat events are ignored, and an up with no down (the app started with the button held) is not a press;
- sends `ToUi::PowerPress` to the screen for each short press.

The button is read only when the app is started with `--power-request` (the stick's `start-app.sh` passes it). A run without it, such as a developer's run under Akai's system, never opens `event1`.

## The card

The screen (`gui.rs`) shows the card as a page of its own over the page it was opened from, as the number pad is, and tells the app it is an overlay so the wheel, – / + and the push reach it. Its steps (`power::Step`):

| Step | Shows | Buttons |
|---|---|---|
| Ask | "Shut down the MPC?", the project (unsaved, saved, nothing to save, or no data folder), what Restart starts | Cancel · Restart · Shut down |
| Unsaved | "Save "Song" first?" | Cancel · Don't save · Save and shut down (or restart) |
| Saving | "Saving "Song"…" | none |
| SaveFailed | "…wasn't saved", the library's reason | Cancel · Shut down (or Restart) without saving |
| Going | "Shutting down…" or "Restarting…" | none; stays until the app is stopped |
| Unavailable | "Can't shut down from here", why, and the hard power-off | OK |

**The save** is the project Save (SHIFT + BROWSE): the screen asks the audio loop for its parts and the library's worker writes them, whole or not at all ([browser and projects](../design/browser.md)). The card waits until the library says the project is saved and its worker is idle. If the worker answered and the work is still unsaved, or nothing started within 10 s (the parts never came, say while a project opens), it shows SaveFailed with the library's status line. A save in progress has no time limit. **Don't save** goes on; autosave's last copy is still set aside at the next start, as after any stop with unsaved work ([saving](../../user/src/making-music/saving.md)).

While the card saves or goes, the power button, a page change and every other button leave it on the screen.

## Handing it to the system

The pad player runs as the `mpfree` user without capabilities on an `--app-user` stick, and as root otherwise; either way **it never shuts down by itself**, so the same route serves both. Shut down or Restart:

1. `sync()` (in `sys.rs`, the only module with C calls), so a save that has just landed is on the disk;
2. writes one word, `poweroff` or `restart`, to `power-request.new` beside the request file, `fsync`s it and renames it over `/run/mpfree-app/power-request` (in RAM, the app's own folder);
3. shows Going.

On the stick (`tools/mpc-one/offline-kexec-stick.py`, every build with `--app`):

- **`mpfree-test-power.path`** (root, in `sysinit.target`, `DefaultDependencies=no`) has `PathExists=/run/mpfree-app/power-request` and starts
- **`mpfree-test-power.service`** (root, oneshot), which runs **`/etc/mpfree-test/power.sh`**:
  - takes the word only from a regular file that is not a link, reading one line with the shell's `read` (the file is in the pad player's folder, so it is never followed and its content is never printed), and removes the file;
  - acts on exactly `poweroff` (`poweroff.target`) or `restart` (`reboot.target`); anything else is ignored with a line in the kernel log;
  - logs `mpfree-test-power: the pad player asked for WORD` to the kernel log, and writes `power.txt` (the word and the uptime) and a last `journal.txt` into this boot's folder on the stick's log partition, if the log service has it mounted;
  - `sync`s, then `systemctl --no-block --job-mode=replace-irreversibly start poweroff.target` (or `reboot.target`): what `systemctl poweroff` and `systemctl reboot` do, without asking logind, as the boot hook starts `soft-reboot.target`.

systemd then stops the pad player (SIGTERM; its crash mark is removed, so the next start is not a safe start), the log service and every other unit, unmounts partition 4, and powers off or restarts. Nothing here writes outside `/run` and the stick's log partition; the eMMC and SD card stay switched off to the end.

## What Restart starts

A restart is a full one: `reboot.target` restarts the hardware, and the MPC goes through U-Boot and Akai's kernel and system as at power-on. With the stick in, the soft-reboot hook on Akai's data partition starts the stick's system again ([how MPFree boots from the stick](soft-reboot-boot.md)), so MPFree comes back after about the power-on time. With the stick out, Akai's system starts. On the opt-in `--kexec-file` route the same happens, since the hook makes the same choice at every boot.

**Assumption, checked by the on-device run:** a plain restart from Linux on this kernel is an ordinary restart for Akai's U-Boot (no recovery or update mode). The kernel's restart writes only a reboot-mode register, not storage, and `power.sh` never passes a reboot argument.

A restart *into Akai's system with the stick in* (a one-time "skip the stick" flag the hook would read) is not built: it would change the hook on Akai's data partition, which needs its own approval.

## Tests

- `power.rs`: short and long presses, auto-repeat, the key bitmap in both word widths, finding the device by name and key in a fake sysfs, the request file, each step's buttons and the outline, taps.
- `gui.rs`: a short press opens the card over the page, the wheel and push choose, a second press cancels; Shut down and Restart write their word through `--power-request`, the card stays while going, and without the option it says it can't.
- `test_offline_kexec_stick.py`: the units, link and owners in a built stick; `power.sh` under dash and BusyBox's sh with fake `systemctl` and `journalctl`: each word starts its target after a sync and leaves its note; other content, an empty file, a link and a missing file do nothing.
- `test_app_mpc.py`: the request file is the app's only new write, reviewed; `sync` is the only new C call.

## On-device check

Short, read-only except for the power-offs and restarts themselves. It needs the boot hook installed (as for [the targeted app run](app-targeted-run.md)) and the stick from earlier rounds. It can follow another round in the same MPFree boot, from P3 on.

**P1: build the stick from the pull request's branch** (workstation only; nothing contacts the MPC):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin claude/project-thread-5r23co && git switch --no-track -C power-check FETCH_HEAD && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_app_mpc.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app' && python3 tools/mpc-one/results.py run build-stick -- python3 tools/mpc-one/offline-kexec-stick.py --splash rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-startup --app rust/target/armv7-unknown-linux-gnueabihf/release/mpfree --app-user
```

The checks end with `LOCAL-CHECKS-OK`, and the build report has the line `power button: a short press opens the pad player's power card`. `--app-user` is there so the check covers the harder case, the pad player without root.

**P2: write the stick** (**state-changing, the stick only**; keeps partition 4; asks for `WRITE`). Undo: write an older build the same way.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- python3 tools/mpc-one/write-test-stick.py
```

**P3: on the MPC** (**state-changing**: the power-offs and the restart are what is tested; nothing is written to the eMMC or SD card, which stay switched off). Switch on with the stick in and wait for MPFree. Then, noting what you see:

1. Press the power button briefly. The card "Shut down the MPC?" opens. Tap **Cancel**: back on the page.
2. Press briefly again, then briefly once more: the second press closes it.
3. Change something the project keeps (turn a knob on the Synth tab). Press briefly: the card says the project has changes that aren't saved. Tap **Shut down**, then **Save and shut down**. Expected: "Saving…", then "Shutting down…", and the MPC switches itself off within about a minute. Note how long it took.
4. Switch on again with the stick in. In **BROWSE** > Projects, the project you saved in step 3 is there. Press briefly and tap **Restart**: "Restarting…", then the MPC logo and MPFree again, as at power-on.
5. Press briefly and tap **Shut down** (nothing unsaved: no save question). The MPC switches off.

If anything hangs for more than a minute, hold the power button to switch off, as before.

**P4: what happened** (read-only; the stick in the workstation):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py zip
```

Upload the zip with your notes. **Passes when** each step did what it says, and the stick logs show three boots with a `power.txt`: `asked: poweroff`, `asked: restart` and `asked: poweroff`, each with `mpfree-test-power: the pad player asked for …` in its `dmesg.txt`, and `units.txt` from the first boot shows whether `az01-power-button.service` runs on the stick.

| Block | Changes | Undo |
|---|---|---|
| P1 | Build output under `~/mpfree-work` and in the clone | Delete them |
| P2 | The stick's partitions 1 to 3 | Write an older build |
| P3 | Power-offs and a restart; partition 4 gets the saved project | Delete the project in BROWSE |
| P4 | A zip in `~/mpfree-results` | Delete it |
