# The 3.10.1 device round: qualification, GPU flips, performance and FX tools

**Status:** runbook, 2026-11-15, revised the same day for MPFree's boot hook (#370, [how MPFree boots from the stick](soft-reboot-boot.md)), and again for the pad player without root (#372) and the opt-in boot routes (#374, [boot routes](boot-routes.md)). This round keeps to the default: a soft-reboot stick, the pad player as root. Revised once more for the whole round the owner asked for on 2026-11-15: round 8's benchmark (Q8b) and second boot (the S blocks), the GPU page flips (Q9), the app's targeted run, and the FX thread's FXProbe (the F blocks), which the stick now carries as developer tools for Akai's system (`--dev-tools`). Revised again that night for [targeted device checks](device-checks.md): the app step asks only about this round's checks, confirms each step from the app's log, and leaves to the app's own tests (Q0d) everything they already check. Not run yet: every block that touches the MPC waits for the owner's go. **Scope:** the reference MPC One (Gen 1). The owner installed MPC OS 3.10.1 on it with Akai's updater on 2026-11-15 and switched developer mode on. Before that it ran 3.6.0 with [MOD-001 and MOD-002](archive/2026-11/device-modifications.md).

The owner wants to know that main still does everything MPFree needs on the installed 3.10.1: log in, survey, back up, start MPFree from the USB stick at power-on with the boot hook (the soft-boot USB), run the pad player on it, and come back. The [stick test](firmware-3.10.1-stick-test.md) answered most of it before the install, with 3.6.0 doing a kexec jump, and the [boot hook's runs](stick-autoboot.md#second-power-on-2026-11-15-zip-092220z-mpfree-started) showed the power-on start once. This round does all of it from main on the installed 3.10.1, with every step's undo below. Since then the owner asked for more in the same round: the GPU page flips, the performance plan's round 8, the app's targeted run, and the FX measuring tools on the same stick, run from inside Akai's system.

## The round at a glance

| Part | Blocks | Where | About |
|---|---|---|---|
| Builds, and the app's tests | Q0, Q0b, Q0c, Q0d | Workstation | 30 min |
| Akai's system, the stick out | Q1 to Q5: trust, lock, survey, backup, device tree | MPC on 3.10.1 | 20 min |
| The stick | Q6, Q6-write | Workstation | 5 min |
| Akai's system, then the stick in | Q7a restart, Q7b the boot hook | MPC on 3.10.1 | 10 min |
| MPFree, first boot | Q7c power-on, Q8 this round's checks, Q8b benchmark; Q9 GPU flips (optional) | MPC on the stick | 60 to 90 min |
| MPFree, second boot (optional) | S1 to S4: Sound cores on cores 3 and 2, the load again, an hour of play | MPC on the stick | 80 min |
| Akai's system, the stick out, then in | Q10 back to 3.10.1; F1 to F8 FXProbe (optional) | MPC on 3.10.1 | 10 min, F 30 min |
| Logs and the zip | Q11 | Workstation | 5 min |

The optional parts can be left out, and the round can stop after any block that leaves Akai's system or MPFree running: Q10 and then Q11 end it from anywhere. Q1 to Q5 have to come first, because Q4's backup is the way back for everything after it.

## This round's checks

The app step asks only about these ([device checks](device-checks.md); `python3 tools/mpc-one/app_checks.py show NAME` prints one in full). For each, it shows a few steps, reads the app's log while you do them, says what it saw, and asks at most one question, about what only you can see or hear.

| Check | Block | Why it's in this round | You do | About |
|---|---|---|---|---|
| `transport` | Q8, first | #387: REC, OVERDUB, UNDO and PLAY START work and light from the start, with no Map controls | Eight presses on Play and the Looper tab | 2 min |
| `basics` | Q8 | The first MPFree boot on the installed 3.10.1: pads, PAD BANK, FULL LEVEL, the wheel, the Q-Links (also on the Mix tab and in the chain editor) and touch | Play and turn things | 3 min |
| `load` | Q8, S2 | Round 8: the worst case for the cores, on Shared in Q8 and on cores 3 and 2 in S2 | Supersaw and the Grains kit with 8 pads | 2 min |
| `track-mute` | Q8 | #332: pad colours as track mutes, and PAD BANK past 16 tracks | TRACK MUTE and the pads | 3 min |
| `wheel` | Q8 | #339 and #355: the outline and the push on the real wheel, and a Song block moved by wheel | Turn and push | 3 min |
| `touch` | Q8 | The real panel: Touch lab, Calibrate touch, two fingers, and a drag past the 12 px lift | Follow the Touch lab, drag a clip twice | 6 min |
| `note-repeat` | Q8 | Never reached on the MPC in live test 3 | NOTE REPEAT, Latch, the Arp panel | 2 min |
| `looper` | Q8 | Live test 3's loop that kept playing after STOP | Record 2 bars, STOP | 2 min |
| `console` | Q8 | Button chords on the real panel, and STOP's silence | The console chord, TAP TEMPO, STOP | 1 min |
| `midi-in` | Q8, last | The MPC's real MIDI IN and its port in the I/O view; `s` skips it without a keyboard and a 5-pin cable | Play the keyboard | 3 min |
| `kept` | S2 | Sound cores, set in S1, comes back at the next start and runs | Nothing: it reads the log and partition 4 | 0 |
| `hour` | S3 | #273's underruns at 25 and 51 minutes | Nothing: it reads the status lines | 0 |

**Checked by the app's tests instead (Q0d), not on the MPC:** Arrange (clips, drag and drop, Capture, the Song view, the morph), Play's views, steps, throws, XY pad and FILL, the Mix tab with its chains, effect modes, Learn and macros, the Looper tab's layers, tools and drags, the Pads and Sample tabs, the Synth, Grains and Prefs, and the sweeps over every control and the demo songs. Q0d runs the app's whole test suite and reports it by check and by area.

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
| Does the app, at this round's commit, pass its own tests, the UI smoke workflows among them? | Q0d |
| Do REC, OVERDUB, UNDO and PLAY work and light as soon as MPFree starts, with no Map controls (#387)? | Q8: `transport` |
| Does the stick's system answer on port 2222 after the soft-reboot, and does the hardware work in it (Ethernet, screen, touch, pads, buttons, sound, partition 4)? | Q8: its checks, then `basics` and `touch` |
| What does each engine cost on the MPC, and how many voices fit in each budget (round 8 of the performance plan)? | Q8b |
| Does what has merged in the app since live test 3 work on the MPC? The app's tests check it in Q0d; on the MPC, what only the MPC shows: 128 tracks on TRACK MUTE's pads, the real wheel, a real drag | Q0d; Q8: `track-mute`, `wheel`, `touch` |
| How do the heaviest sounds run on the MPC's cores, on Shared and on cores 3 and 2? | Q8 and S2: `load` |
| Does Prefs > Sound cores > Cores 3 and 2 come back at the next start and put the audio thread on core 3 and its lanes on core 2, and does a song play for an hour with no underrun? | S2: `kept`; S3: `hour` |
| Does the MPC come back to 3.10.1 with the eMMC's boot areas unchanged? | Q10 |
| Does Akai's app load the FXProbe plugins from the internal drive, and do they capture AIR effects at 0 and -12 dB? | F1 to F8 |

## You need

- **The MPC on the direct Ethernet cable only.** Until Q2 has locked it, anyone on any network the MPC is on can get a root shell with no password. If the MPC is on Wi-Fi, switch its Wi-Fi off in Akai's settings before Q1.
- Developer mode on: Menu > Preferences > Enhancements, switch on the developer panel; then hold SHIFT and press MENU, and choose Developer.
- About 8 GiB free in `~/mpfree-backups` for Q4, and a second disk for the copy the backup tool asks you to make.
- The USB stick from earlier rounds. Its partition 4 keeps the pad player's settings.
- **The stick out of the MPC at the start.** With the boot hook on the MPC, switching on with the stick in starts MPFree instead of Akai's system. The stick goes into the MPC in Q7b, while Akai's system runs.
- An SD card in the MPC's slot can stay. The stick's system switches the SD card off with the eMMC, and the stick logs show that nothing read it first.

Every block runs in a terminal on the workstation, from the clone at `/home/jesse/code/mpfree-hardware-survey`. Each step's output lands in one results folder, and Q11 packs it into one zip.

## The blocks

**Block Q0: update, check, build** (workstation only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C qual-3101 origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_devmode_login.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_kexec_preflight.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_host_emmc_backup.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && python3 tools/mpc-one/tests/test_write_test_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_boot_hook.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_app_mpc.py && python3 tools/mpc-one/tests/test_app_checks.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_read_stick_logs.py && python3 tools/mpc-one/tests/test_results.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app && cd rust/target/armv7-unknown-linux-gnueabihf/release && sha256sum mpfree-screen-log mpfree-startup mpfree'
```

It ends with three hashes. `results.py status` lists what is already in the results folder: if an earlier round left files there, run `python3 tools/mpc-one/results.py zip` first, so this round gets its own zip.

**Block Q0b: build the benchmark** (workstation only). These are the two builds Q8b runs: the plain one, and one tuned for the MPC's Cortex-A17 cores, in a folder of its own:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run build-bench -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc && RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-bench && RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static -C target-cpu=cortex-a17" cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf --target-dir rust/target/a17 -p mpfree-bench && sha256sum rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-bench rust/target/a17/armv7-unknown-linux-gnueabihf/release/mpfree-bench'
```

It ends with two hashes.

**Block Q0c: build the FX thread's measuring kit** (workstation only; the first time it fetches Akai's plugin SDK from GitHub, and the SDK fetches its Zig compiler). This is the folder of developer tools Q6 puts on the stick for the F blocks: FXProbe, two plugins made with Akai's [plugin SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk) at the commit FXProbe is tested at, the test programmes and `fxprobe.sh`, which runs on the MPC under Akai's system ([fxlab and FXProbe](../design/fxlab.md)). Skip Q0c, and leave `--dev-tools` out of Q6, if you won't do the F blocks:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run build-fxprobe -- bash -c 'set -e; sdk=/home/jesse/code/mpc-plugin-builder-sdk; [ -d "$sdk" ] || git clone https://github.com/inmusic-software/mpc-plugin-builder-sdk "$sdk"; git -C "$sdk" checkout --detach febd2ca5; (cd rust && cargo build --release --locked -p mpfree-fxlab); kit=/home/jesse/mpfree-work/dev-tools-$(git rev-parse --short=12 HEAD); mkdir -p "$kit"; python3 tools/fxprobe/stage.py --sdk "$sdk" --out "$kit/fxprobe"'
```

It ends with the kit's 20 files, about 23 MB in all, and `STAGE-OK`. The kit's folder is named after Q0's commit. If it stops with `… exists; give a new folder`, an earlier Q0c at the same commit made it already: go on to Q0d.

**Block Q0d: the app's own tests** (workstation only, read-only: it builds into `rust/target`; about 6 minutes). This is what the device rounds used to ask you to tap through on the MPC. The app's whole test suite runs at Q0's commit, the UI smoke workflows among them, which drive the screen and the engines as a finger, the buttons and the wheel would:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-tests -- python3 tools/mpc-one/app_checks.py smoke
```

It prints how many tests ran and passed, then the software side of each device check and each area that is no longer checked on the MPC, and ends `RESULT: every test passed.` If a test fails, it names it: go on with the round, and say so with the zip.

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
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run build-stick -- python3 tools/mpc-one/offline-kexec-stick.py --splash rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-startup --app rust/target/armv7-unknown-linux-gnueabihf/release/mpfree --dev-tools /home/jesse/mpfree-work/dev-tools-$(git rev-parse --short=12 HEAD)
```

It takes the newest unpacked image (3.10.1) and the device tree from Q5. Its report must say:
- `running release: 6.18.44-az01-2026-08-10-rt6 (recorded by the preflight; the same kernel as the test build's)`;
- `pad player user: root (no --app-user)`;
- `developer tools for Akai's system (--dev-tools /home/jesse/mpfree-work/dev-tools-…): 20 files, … bytes, on partition 3 under /mpfree-dev with SHA256SUMS and MPFREE-DEV.txt; …`, then a `dev tool /mpfree-dev/fxprobe/…` line with its hash for each file. If you skipped Q0c, leave `--dev-tools` and what follows it out of the command; the report then says `developer tools: none (no --dev-tools)`;
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

**Block Q8: this round's checks** (read-only on the MPC: the tool starts, copies and stops nothing and only reads the app's log; you press buttons and pads, the loops stay in RAM, and anything MPFree saves goes to the stick's partition 4; about 30 minutes). These are the first boot's checks in [this round's checks](#this-rounds-checks), in that order. `transport` comes first, so REC, OVERDUB, UNDO and PLAY are tried before anything else uses them (#387). Turn the volume down a little before you start.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-checks -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_CHECKS=transport,basics,load,track-mute,wheel,touch,note-repeat,looper,console,midi-in bash tools/mpc-one/collect-over-ssh.sh --app
```

- **This is the first login on port 2222 after a soft-reboot.** Before it reads anything, the tool prints `started by: the boot hook's soft-reboot (asked at … s after power-on)`, `no MMC block device`, `kexec_load_disabled: 1` and `/: read-only`, and ends `CHECKS-OK`. The release is Akai's own kernel now, so the stick's host key and the missing MMC devices are what tell the stick's system from Akai's.
- **Our reading, which the tool shows:** after the soft-reboot `/` stays the stick's read-only partition 1, `/run` is Akai's tmpfs carried over and allows programs, and the stick's own sysctl file turns kexec off. If any of these is wrong, it prints a `PROBLEM` line and stops before it reads anything else.
- **Then the start-up, from the log, with no question:** `start-up: playing … s after power-on; startup screen: …`, the saved settings and buttons the pad player started with, and `no PANIC or safe start lines`. Its `privileges:` lines show `Uid: 0`: on this stick, built without `--app-user`, the pad player runs as root, as expected (#372).
- **If it lists buttons that Map controls has not found yet** (this round's checks use PAD BANK A/E and B/F, TRACK MIX, MAIN, TRACK MUTE, the wheel's push, SAMPLER and ERASE), it runs `map-controls` first. Find only the buttons it lists, then press MENU. REC, OVERDUB, UNDO and PLAY START already have numbers, and `transport` checks those. Partition 4 keeps what you find, so a later round doesn't ask again. If you skip it with `s`, the steps that use those buttons show `NOT SEEN`.
- **Each check** shows its steps. Press Enter to start (`s` skips the check, `q` stops here), do the steps, and press Enter when you are done. It then prints `what the log showed:`, with `seen`, `NOT SEEN` or `unclear` for each thing it looked for, and asks at most one question, about what only you can see or hear. Answer in a few words, with the step's number if one went wrong.
- **`transport`:** a mapping saved on the stick in an earlier round still applies, and wins over the built-in numbers. If REC does nothing at all (`button 73 never reached the app`), open Prefs > Map controls after the check and read the REC row: a number other than `button 73` is the saved one. Tap the row and press REC to set it again, and say so in your answer. If OVERDUB, UNDO or PLAY START does nothing while REC works, write down the number its Map controls row shows: 80, 67 and 83 are inferred from Akai's light messages, so that number corrects the default.
- **`load`** waits up to 40 s after your Enter for the app's next status line, so it can say whether the worst case underran, then prints the cores, the clock and the threads.
- **`midi-in`** is last: type `s` at its prompt if no keyboard and 5-pin cable are at hand.
- **At the end** it lists each check with how many of its lines were seen and your answer, then the session summary since power-on and the app's latest status.
- **If it says `the test build did not answer on port 2222`,** note what the screen shows. The pad player can still be played by hand. Go to Q10: the stick logs show the network state.
- **Undo:** none needed. The loops are in RAM, and the power-off in Q10 clears them. Buttons that Map controls finds stay in `settings/map.conf` on partition 4, where they are meant to be.

**Block Q8b: what each engine costs on the MPC** (**RAM only**: it copies the two benchmark builds into `/run/mpfree-bench` and stops the pad player while it runs; press Enter at each of steps 2 to 5; about 30 minutes). This is round 8's first ask in the [performance plan](../design/profiling.md#mpfree-bench): real voice counts for every engine, Analog at 8 and 16 voices among them, and the sampler after its 31 % pass.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run bench -- env MPFREE_ACCEPT_BENCH=YES python3 tools/mpc-one/host_bench_mpc.py
```

- **What it changes:** it talks only to the test build (port 2222, the stick's own host key) and runs the same checks as Q8 first, with the same `started by:` line. It copies both builds into the stick system's RAM and checks their hashes there. For a clean run it stops our pad player (`mpfree-test-app`, RAM state only) and starts it again at the end, also when a run fails or you press `q`. Akai's services are not running and are never touched. The benchmark only computes: it opens no device, writes only its JSON files into `/run/mpfree-bench`, and runs as an ordinary process.
- **Each build runs** `--calibrate`, `--json`, `--afford` (what a voice costs and how many fit in each budget) and `--lanes --lane-cores 3,2` (the same work on one core and split over two, pinned to cores 3 and 2).
- **Passes when** it ends `RESULT: both builds ran (numbers above).` and prints `pad player: active (started again)`.
- **If it is cut off halfway** (the cable, or `q`), the pad player may stay stopped: it starts again at the next power-on. Nothing is left but RAM.
- **Undo:** none needed. The power-off in Q10 clears `/run`.

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

**S (optional): a second boot, with the sound on cores 3 and 2, playing a song for an hour** (about 80 minutes). Round 8 of the [performance plan](../design/profiling.md#mpfree-bench) asks three things that need a boot of their own. Does Prefs > Sound cores > Cores 3 and 2 come back at the next start and put the audio thread on core 3 and its lanes on core 2? How does the worst case run there? And does a song play for an hour with no underrun? In live test 3 (#273) the pad player had two underruns, at periods 524,292 and 1,048,580 (25.4 and 50.7 minutes). Q8b and Q9 stopped the pad player in this boot, so the hour needs a fresh start. Nothing after the S blocks depends on them: to skip them, go to Q10. The song's steps are the app thread's, checked against main's code and manual.

**Block S1: Sound cores to Cores 3 and 2** (**state-changing, on the stick only**: one key, `sound-cores`, in `settings/app.conf` on partition 4; the MPC isn't touched; it takes effect at the next start). Still in this boot, on the MPC:

1. Press **SHIFT + MENU** to open Prefs.
2. Turn the data wheel to **Sound cores**, the sixth row, just below Show developer tools. You can also tap it.
3. Press **+** twice. The row goes Shared, then Core 3 (next start), then **Cores 3 and 2 (next start)**. Stop there: a third press gives Cores 3, 2 and 1, a test setting, and **–** comes back from it.
4. Wait two seconds, then press **MENU** to close. The setting is written within about a second of the press.

- **Undo:** block S4 sets it back to Shared. If the round stops before S4, do S4 in the next MPFree boot: until then each start runs on cores 3 and 2, because partition 4 is kept.

**Block S2: the second boot: Sound cores kept, the load, then the song** (a read-only check first; then **state-changing, on the stick only**: autosave writes the song to `Projects/Autosave` on partition 4). Switch the MPC off, **leave the stick in**, wait 10 s and switch it on. The boot hook starts MPFree again, as in Q7c. Before you play anything:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-cores-32 -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_CHECKS=kept,load bash tools/mpc-one/collect-over-ssh.sh --app
```

- It prints the same checks as Q8 (`started by: the boot hook's soft-reboot …`, `CHECKS-OK`).
- **`kept`** asks nothing. It reads `settings/app.conf` on partition 4, read-only, and compares it with the settings the app started with and the cores it runs on. Expect `seen` with `sound cores two`. `NOT SEEN` with `it runs as …` means the setting came back but the sound didn't move.
- **`load`** is Q8's worst case again, now on cores 3 and 2; its numbers compare with Q8's, which ran on Shared. It comes before the song because it changes kits.
- Its session summary at the end has `sound cores: two: the audio thread on core 3, lanes on core 2`. In Prefs the row reads **Cores 3 and 2**, with no "(next start)".

Then build a song on the Play page. It takes about three minutes and needs only built-in buttons:

1. Tap the **Play** tab. Kit 1 is on the pads (`load` put it back). Tap **Beats** in the view switch, on the right of the transport bar. If the pattern isn't empty (an earlier round may have set At start to Last project, so a project opened), skip steps 2 and 3, press PLAY on what's there, and say so in your notes.
2. Tap squares to light them. Each row is named after its sound. Light the kick on steps 1, 5, 9 and 13, the snare on 5 and 13, and a hi-hat on every odd step.
3. Tap **All** in the view switch. Turn the fourth Q-Link, whose card says **Delay send**, to about half. The drums now also play through the delay on Return A.
4. Press **PLAY**. The pattern plays round and round until STOP is pressed.
5. Turn the **MASTER VOL** knob on the back down if you don't want to hear it for an hour. The test doesn't need it loud.

Don't use REC or the Arrange tab's song, and from here on don't play the pads or change the kit: a kit change swaps the song's sound and load partway through the hour. Note the time you pressed PLAY.

**Block S3: after an hour** (read-only). Leave the song playing. Nothing in MPFree stops it: there is no screen sleep or dimming, the pad player has no time limit, and autosave never stops play. When the song has played for about **65 minutes**:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-hour -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_CHECKS=hour bash tools/mpc-one/collect-over-ssh.sh --app
```

- **`hour`** asks nothing. It reads the app's status lines since the start. An hour is 1,240,312 periods, counted from the pad player's start, not from PLAY. It shows `seen` with more than that, `0 underruns`, no `underrun at period` line and no restart.
- **`unclear` with `NOT YET an hour: run this check again in N min`** means the hour isn't up: wait that long and run the block again. The name can repeat; each run gets its own log.
- **A crash shows as** `NOT SEEN` with `the period count went back, so the pad player restarted`, with play stopped and an empty project; the hint line says so. That is a fail worth reporting. Every status line and every `underrun at period N` line of the boot is also in the stick logs that Q11 reads.
- An underrun in S2's `load` counts here too. Its `underrun at period N` has a small N, from before PLAY, and S2's `load` result shows it.

**Block S4: Sound cores back** (**state-changing, on the stick only**: one key, `sound-cores`, in `settings/app.conf` on partition 4, back to Shared). This is the undo of S1:

1. Press **STOP**. Press **SHIFT + MENU**, then go to **Sound cores**.
2. Press **–** twice. The row goes Core 3 (next start), then **Shared (next start)**. It stops at Shared.
3. Wait two seconds, then press **MENU**.

This boot keeps running on cores 3 and 2 until it ends; the next start runs Shared. Nothing else needs undoing: Cores 3 and 2 changes no other setting, project or file. At the next MPFree start, the autosave of the song is kept as a project named `Unsaved 2026-11-15 HH.MM` in BROWSE > Projects, and the hint line says so for a few seconds. It is harmless; delete it in the browser whenever you like.

**Block Q10: back to 3.10.1.** Switch the MPC off, take out the stick, wait 10 s and switch it on. The stick must be out: with the hook installed, a power-on with the stick in starts MPFree again. Wait for Akai's app, and switch developer mode on again if it is off. Then (read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run status-after -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run hook-report-after -- bash tools/mpc-one/collect-over-ssh.sh --hook-report && python3 tools/mpc-one/results.py run boot-map-after -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

The hook report should show main's hook files, with the hashes Q7b wrote, and `MPFree's stick in now: no`. Then go on to the F blocks, or straight to Q11.

**F (optional): FXProbe, the FX thread's measuring plugins, under Akai's system.** FXProbe lets fxlab measure the AIR effects on the MPC exactly as it measures MPFree's ([fxlab and FXProbe](../design/fxlab.md)). This is its first time on an MPC, so the first question is whether Akai's app loads the two plugins at all. The kit came onto the stick's partition 3 in Q6, under `/mpfree-dev/fxprobe`. Akai's system mounts every partition of a stick noexec, so the kit's script runs with `sh`, and the plugins are copied once to the MPC's internal drive, where the app can load them. Captures go to the stick's partition 4. Skip to Q11 if you'd rather not: nothing else depends on F. The F blocks can also run on another day, with the stick from this round. They need **External Plugin Support** on in Akai's app (Menu > Preferences > Enhancements).

Every F block runs the kit's script on the MPC over developer mode's SSH, with your key, from the workstation. The stick goes in only while Akai's app is up: with the hook installed, a power-on with the stick in starts MPFree.

**Block F1: what is where** (read-only). With Akai's app up, put the stick into the MPC's USB-A port. Answer No or Cancel to anything that offers to update, format or erase, and note what it asked. Wait 10 s. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-check -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh check'
```

It checks the kit against its `SHA256SUMS` and shows the kits and data partitions, the internal drive's mount options, whether the plugins are installed, and the work folder. It ends `CHECK-OK`, and says `FORMAT: mpfree-data 1` for partition 4. If it says `can't open '/media/*/mpfree-dev/fxprobe/fxprobe.sh'`, Akai's system has not mounted the stick: take it out, put it in again, wait 10 s and run F1 again.

**Block F2: install the plugins** (**state-changing, on the MPC's internal drive**: two folders, about 1.1 MB, into `/media/az01-internal/VST2 Plugins`; Akai's app keeps running):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-install -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh install'
```

- **What it changes:** it copies `MPFree - VST - FXProbe Source` and `MPFree - VST - FXProbe Tap` to `.fxprobe-staging` at the internal drive's top, checks the copy against the kit's hashes, then moves each folder into `VST2 Plugins` (it makes that folder if there is none), so the app never sees half a plugin. Each folder gets a pointer file, `workfolder.txt`, that finds the work folder on the stick. It refuses a drive that is not mounted read-write or is noexec, and a folder of the same name that isn't FXProbe's. It ends `INSTALL-OK`, and its lines name what changed.
- **If it stops partway:** before the moves, nothing in `VST2 Plugins` changed and the staging folder is gone. Between the two moves, run F2 again, or the undo.
- **Undo:** block F-undo's `remove` deletes exactly those two folders, and `VST2 Plugins` if F2 made it and it is empty. The plugins can also stay for later measuring rounds.

**Block F3: restart Akai's app, so it lists the plugins** (read-only on the MPC). Switch the MPC off first, so the stick's file systems are left clean, then take the stick out. Wait 10 s and switch on. Wait for Akai's app, and put the stick in again once it is up. Wait 10 s.

**Block F4: the work folder** (**state-changing, on the stick only**: `FXProbe/` on partition 4, with the test programmes and the marker the plugins look for):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-setup -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh setup'
```

It ends `SETUP-OK`. Undo: F-undo's `clean`, which deletes `FXProbe/` from partition 4, captures included.

**Step F5: three plugin tracks in Akai's app** (on the MPC: an Akai project in RAM; don't save it unless you want to). These taps follow the plugin SDK's description and have **not been checked on 3.10.1 yet**: note where the screens differ. FXProbe Source plays sines at 0 dBFS, so turn **MASTER VOL** down first, and set each Tap's **Output** to **Mute** before anything plays. Finish each track before you start the next, because tracks made in order get slots 1, 2 and 3:

- **Track 1, the reference:** a new instrument track, as a plugin program, and browse to External Plugins > **FXProbe Source**. In insert 1, **FXProbe Tap**, with Slot **1** and Output **Mute** on its screen. This proves the route is a wire.
- **Track 2:** the same, with **AIR Distortion** at its defaults in insert 1, and FXProbe Tap in insert 2, Slot **2**, Output **Mute**.
- **Track 3:** the same, with **AIR Spring Reverb** at its defaults in insert 1, and FXProbe Tap in insert 2, Slot **3**, Output **Mute**.

Leave Source's own Slot and Trigger at their defaults. If the app doesn't list FXProbe under External Plugins, note what it lists, skip to F-undo or Q11, and tell the FX thread: that answers this round's first question.

**Block F6: do the plugins report?** (read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-status -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh status'
```

Three Sources and three Taps, slots 1 to 3, each `idle`.

**Block F7: the first capture** (**state-changing, on the stick only**: `control.txt` and new files in `FXProbe/captures/` on partition 4; about 2 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-capture -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh capture round1-defaults --programme standard --gains 0,-12 --timing'
```

It plays the standard programme through every slot, once at 0 dB and once at -12 dB, and records what reaches each Tap. It prints `slots: 1 2 3; 2 capture(s) of 52.639 s each` and `loaded standard.wav in every Source` (each Source reads the 17.5 MB programme from the stick before the first gain, so none starts late), then `captured round1-defaults-g0 from 3 slot(s)` and the same for `round1-defaults-gm12`, and ends `CAPTURE-OK`. If a Source can't read the programme, it stops with `STOP: source-s1: cannot read programmes/standard.wav; …` before anything is recorded. It checks every report for overruns, dropped frames and samples that aren't numbers; a problem shows as `…; PROBLEMS: …` and exit code 1. `--timing` also logs the audio thread's scheduling and block sizes. AIR Distortion is measured against MPFree's Drive (aliasing), and the Spring Reverb against MPFree's spring (its metallic tail).

**Block F8: measure the captures** (workstation only; read-only on the stick). Switch the MPC off, take the stick out, and put it into the workstation. The desktop mounts its data partition at `/media/jesse/MPFREE-DATA`; if it doesn't, mount it the way the file manager does, with `udisksctl mount -b /dev/disk/by-label/MPFREE-DATA`. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-analyse -- bash -c 'set -e; w=/home/jesse/mpfree-work/fxprobe-round1; mkdir -p "$w"; sudo cp -r /media/jesse/MPFREE-DATA/FXProbe/captures /media/jesse/MPFREE-DATA/FXProbe/programmes "$w/"; sudo chown -R "$(id -u):$(id -g)" "$w"; rust/target/release/fxlab analyse "$w"/captures/round1-defaults-g0-s*.wav --schedule "$w/programmes/standard.tsv" --gain 0 --reverb --out "$w/g0"; rust/target/release/fxlab analyse "$w"/captures/round1-defaults-gm12-s*.wav --schedule "$w/programmes/standard.tsv" --gain -12 --reverb --out "$w/gm12"; tar -czf /home/jesse/mpfree-results/current/fxprobe-round1.tar.gz --exclude="*.wav" -C "$w" .'
```

It asks for your sudo password: Akai's system wrote `FXProbe/` as root. The kit and the plugins make its folders 0755 and its files 0644, but this is their first time on the MPC, so the copy reads as root anyway and then makes the copy yours. It copies the captures and programmes into `~/mpfree-work/fxprobe-round1`, measures each gain with fxlab (built in Q0c), and packs everything but the WAV files into `fxprobe-round1.tar.gz` in the results folder, so Q11's zip carries the numbers. The WAV files stay on your workstation: they are Akai's effects' output, so they never go into the repository.

The plugins and `FXProbe/` can stay: without the marked folder the plugins do nothing, and the next measuring rounds need them. Their undo is block F-undo, in the undo section.

**Block Q11: the stick logs and the zip.** If the stick is in the MPC (after F), switch the MPC off first, then take the stick out. Put it into the workstation. Then (read-only on the stick):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py zip
```

Upload the zip, with your notes from the MPC's screen. The reports quote Akai's files: upload them to this project only.

## Passes when

- Q0d ends `RESULT: every test passed.`
- Q1 records the host key; Q2 ends `LOCKED`; Q3, Q7a and Q10 end `LOCKED: only your key gets in`.
- Q4's backup is verified, and its baseline mismatch names only `mmcblk0p6`.
- Q6's report has the soft-reboot line, `route: soft-reboot only (no --kexec-file)` and `pad player user: root (no --app-user)`, `no-reserved-blocks@…` is `masked on the stick`, and no unit starting `az0` is `NOT masked`.
- Q7b ends `RESULT: the hook is installed, and the stick checks out.`
- In Q7c the MPC went from the logo to MPFree's pad player with no tap, and Akai's app never drew.
- Q8 ends its checks with `started by: the boot hook's soft-reboot` and `CHECKS-OK`, and each of its checks has every line `seen` and an answer that says it looked and sounded right. A `NOT SEEN` line, or an answer that says otherwise, is a finding: the report has the app's log for that check's time.
- Q8b ends `RESULT: both builds ran (numbers above).` with the pad player started again.
- In S2, `kept` and `load` have every line `seen`, and the session summary has `sound cores: two: the audio thread on core 3, lanes on core 2`. S3's `hour` is `seen`: more than 1,240,312 periods, `0 underruns` and no restart, or at least no new underrun near periods 524,292 and 1,048,580. The stick logs of the second boot have no `underrun at period` line.
- F1 ends `CHECK-OK`, F2 `INSTALL-OK`, F4 `SETUP-OK`; F6 lists three Sources and three Taps, `idle`; F7 ends `CAPTURE-OK` with three slots at each gain; F8 writes `fxprobe-round1.tar.gz` into the results.
- The stick's `dmesg.txt` shows `mpfree-internal-off: switched off ff0f0000...` before any `sd` or `mmc` mount. Nothing reads the eMMC or SD card before `mpfree-internal-off: done`: no `no-reserved-blocks` unit and no `Buffer I/O error` on `mmcblk`.
- The boot maps from Q7a and Q10 have the same boot-area hashes, with 0 sectors written to p1 to p6 and boot0/1 since the power-on in Q10.
- The stick logs show no failed unit we didn't expect and no `mpfree-test: blocked` line we can't explain. Akai's `az0x-loadpin-trustpoints` fails after the soft-reboot, as on 2026-11-15; that failure is harmless.

## Undo, block by block

| Block | Changes | Undo |
|---|---|---|
| Q0, Q0b, Q0c, Q0d, Q6 | New folders under `~/mpfree-work` and `~/mpfree-results`, build output in the clone, and (Q0c) the SDK's clone at `~/code/mpc-plugin-builder-sdk` with its build folders | Delete them |
| Q1 | One line for `10.42.0.141` in `~/.ssh/known_hosts` | `ssh-keygen -R 10.42.0.141 -f ~/.ssh/known_hosts` |
| Q2 | `/data/ssh` on the MPC | Block Q2-undo |
| Q4 | A new folder under `~/mpfree-backups` | Keep it: it is the return point for 3.10.1 |
| Q5 | A private folder under `~/mpfree-work` (it holds the serial number and MAC) | Delete it |
| Q6-write | The stick's partitions 1 to 3 | Rebuild and write an earlier stick; partition 4 stays |
| Q7b | The boot hook's three files on `/data`, in place of an earlier version | The hook stays installed after the round: it is how MPFree starts. To take it off the MPC, block Q7b-undo |
| Q7c, Q8, Q8b, Q9 | RAM on the MPC; the stick's log partition and partition 4 | Switch off, take out the stick, wait 10 s, switch on |
| Q8 (`map-controls`, if it ran) | The buttons found, in `settings/map.conf` on the stick's partition 4 | Keep them: they are this MPC's own numbers. To start again from the built-in numbers, delete `settings/map.conf` on partition 4 with the stick in the workstation |
| S1 | `sound-cores` in `settings/app.conf` on the stick's partition 4 | Block S4 |
| S2 | `Projects/Autosave` on partition 4, kept as an `Unsaved …` project at the next start | Delete it in BROWSE > Projects, if you like |
| F2 | Two plugin folders in `/media/az01-internal/VST2 Plugins` on the MPC's internal drive | Block F-undo's `remove` (they can stay) |
| F4, F7 | `FXProbe/` on the stick's partition 4, the captures in it | Block F-undo's `clean` (copy the captures off first) |
| F8 | `~/mpfree-work/fxprobe-round1`, and `fxprobe-round1.tar.gz` in the results | Delete them |

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

**Block F-undo: take FXProbe off** (**state-changing**: `remove` on the MPC's internal drive, `clean` on the stick's partition 4; Akai's app up, the stick put in once it is up). Each line can run alone. `remove` deletes exactly the two plugin folders F2 copied, and `VST2 Plugins` if F2 made it and it is empty; it ends `REMOVE-OK`. Restart Akai's app afterwards, stick out, so it stops listing them. `clean` deletes `FXProbe/` from the stick's partition 4, captures included, so copy them off first (F8 does); it ends `CLEAN-OK`, and F4 makes the folder again:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-remove -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh remove'
```

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-clean -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh clean'
```

Without the stick, by hand in the developer panel's terminal: `rm -r "/media/az01-internal/VST2 Plugins/MPFree - VST - FXProbe Source" "/media/az01-internal/VST2 Plugins/MPFree - VST - FXProbe Tap"`.

## What this round cannot show

- Whether developer mode, the lock, the host key and the boot hook survive a long power-off or Akai's next update. After an update, the stick must be rebuilt from a fresh Q5: the hook refuses a stick built for another kernel, and Akai's system starts.

## Rehearsal (2026-11-15)

Every block was run word for word from a clone at `/home/jesse/code/mpfree-hardware-survey`, with `HOME=/home/jesse`. Stand-ins played the cable and the MPC: `ip`, `ping`, `ssh` and `ssh-keyscan` answered as a fake installed 3.10.1 with developer mode on. Its remote scripts ran under BusyBox against fake system files, and the boot hook's check and jump under dash, with stand-ins for `mount`, `umount`, `systemctl`, `systemd-run` and `modprobe`. The answers were typed in a terminal only after each question appeared.

**The whole round, at this pull request's first commits merged with #384 (FX thread, head `00266194`)**, Q0 to Q11, the S blocks and the F blocks, word for word, as an ordinary user at the owner's paths:

- **Q0b** built both benchmark builds (two hashes). **Q0c** cloned the plugin SDK, checked out `febd2ca5`, fetched its Zig compiler, built fxlab and both plugins (arm32, arm64, the SDK's validator), and printed the kit's 20 files, 23.3 MB, and `STAGE-OK`. Run again at the same commit, it stopped with `… exists; give a new folder`, as Q0c says.
- **Q6** printed the developer tools line with 20 files and a `dev tool /mpfree-dev/fxprobe/…` line for each.
- **Q8b, Q9b, S2 and S3** ran against the fake test build behind port 2222, after the soft-reboot: each printed `started by: the boot hook's soft-reboot`, `CHECKS-OK` and the phases asked for. Q8b ended `RESULT: both builds ran`. The song, the Prefs checks and their numbers can't be rehearsed off the MPC: those steps are the app thread's, checked against main's code and manual.
- **F1 to F7 and F-undo** ran `fxprobe.sh` under BusyBox against a fake MPC root. Partition 3 came from the stick Q6 built, mounted noexec under a UUID name as Akai's disk service does. The FX thread's stand-in played live Source and Tap instances in slots 1 to 3 and wrote real float captures. F1 printed `files match SHA256SUMS`, `FORMAT: mpfree-data 1` and `CHECK-OK`. F2 printed `INSTALL-OK`, F4 `SETUP-OK`, and F6 three Sources and three Taps `idle`. F7 printed `loaded standard.wav in every Source`, both `captured … from 3 slot(s)` lines and `CAPTURE-OK`. F-undo printed `REMOVE-OK` and `CLEAN-OK`. Under a 077 umask, the work folder first came out 0700 with files 0600. A workstation would then have needed root to read the stick, so the FX thread now sets its modes (0755 and 0644), and F8 copies as root anyway.
- **F8** measured both gains with fxlab (`wrote 3 cards`, twice) and packed the results without any WAV. **Q11**'s zip carried it.

**Again after #372 and #374, at commit `7a3984ef`** (main `d58eb513` merged in), Q0 to Q10 word for word, this time as an ordinary user, as on the owner's workstation (earlier rounds ran as root):
- **Q0** printed `LOCAL-CHECKS-OK` and built the three ARM programs (`Finished`, three hashes). Four tests that need root to make a device node or a loop mount said `skipped`, as they will on the workstation.
- **Q6** printed `pad player user: root (no --app-user)`, the soft-reboot line, `modules the hook loads first: none` and `route: soft-reboot only (no --kexec-file)`, and no `--kexec-file:` or `--chain:` line.
- **Q7b** replaced the earlier hook, stopped once with `MPFree's stick is not in the MPC`, then, with the stick in, said `already in place, unchanged`, `CHECK-OK: at power-on with this stick in, the MPC would soft-reboot into it` and `RESULT: the hook is installed, and the stick checks out.`
- The power-on that falls through, **Q7c-report**, **Q10**, **Q7b-undo** and **Q2-undo** printed the same lines as the boot-hook round below. Q1, Q2, Q3, Q5 and Q7a too.

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

Later commits in the same pull request change only documentation, or merge `main`.

## After the round

- A 3.10.1 baseline for the backup tool, `tools/mpc-one/baselines/mpc-one-3.10.1-reference.txt`, from the Q7a boot map and the Q4 backup.
- The [compatibility page](firmware-3.10.1-compatibility.md) and the [developer access guide](../guides/developer-access.md) updated with what the installed unit showed.
- The results here, as in the stick test's phase sections.
