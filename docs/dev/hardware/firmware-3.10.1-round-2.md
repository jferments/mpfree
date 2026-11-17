# The second 3.10.1 round: what merged since 16 Nov, the GPU flips and FXProbe

**Status:** runbook, 2026-11-16, brought up to main on 2026-11-17 (`68709b47`) with what merged that night: step locks and step speed, MIDI ports, the sampler's clean pitching, the live-run fixes (#429) and round 2 of the FX quality work (Drive, Crush, Amp). Rehearsed word for word on 2026-11-17 with the device-rounds kit (a stand-in MPC on 3.10.1 and a stick image, as the user `jesse`): every block ran and ended as this page says, Q8 with all 27 checks and Q8b with both earlier builds. A container has no GPU and no USB disk, so G2 stopped at its read-only check (no render node, nothing changed), and Q11's log read stopped at "the stick's log partition is not on this computer"; `read-stick-logs.py`'s own tests cover the rest. Not run yet: every block that touches the MPC waits for the owner's go. **Scope:** the reference MPC One (Gen 1) on MPC OS 3.10.1 with developer mode on and MPFree's boot hook installed, as the [3.10.1 device round](firmware-3.10.1-qualification.md) left it on 16 Nov. **Follows:** that round, whose blocks this one reuses word for word where nothing changed.

The 16 Nov round showed main working on the installed 3.10.1: the boot hook, the USB creator's stick, its first start and backup, and most of the app's checks. It also found bugs, which have been fixed since: Play's steps that didn't play (#418), the Looper's STOP and what it records (#428), the Looper tab's lights (#419), the plugin checks' missing Instrument row and the catalog's "Insufficient randomness" (#428). Three parts didn't run at all: the GPU page flips (Q9, the benchmark needed a newer glibc than the MPC's; fixed in #419), FXProbe (F1: it came last, and by then the MPC wasn't in Akai's system with developer mode on), and the second boot (S1 to S4, skipped). This round tries the fixes on the MPC, adds the checks for what merged since (#421 to #424, #426, #430 to #432, #434, #441, #445, #450, #452, and #428's rename of the EQ's Listen to Solo band), times what FX PR C and round 2's Drive, Crush and Amp changed against builds from just before them (Q8b), and runs Q9 and FXProbe. The stick also gets #429's download helper, `mpfree-fetch`, which the Plugins place downloads through. Nothing that passed on 16 Nov and hasn't changed is asked again.

## The round at a glance

| Part | Blocks | Where | About |
|---|---|---|---|
| Builds, and the app's tests | Q0, Q0b, Q0c, Q0d, G1a, G1 | Workstation | 60 min (G1a's first run adds 10) |
| The stick | Q6a which stick, Q6 the USB creator writes it, Q6b the plugins | Workstation | 10 min |
| Akai's system, before MPFree | F0 developer mode on, Q2 the lock (optional), F1 to F7 FXProbe | MPC on 3.10.1 | 35 min |
| MPFree | Q7 switch on with the stick in, Q8 this round's checks, Q8b what the engines cost, Q9 GPU flips, Q10 shut down | MPC on the stick | 150 min |
| Akai's system, after MPFree | Q10b back to 3.10.1: status, hook report, boot map | MPC on 3.10.1 | 5 min |
| Measure, logs and the zip | F8, Q11 | Workstation | 15 min |

The round can stop after any block that leaves Akai's system or MPFree running: Q10 and then Q11 end it from anywhere. F0, Q2 and the F blocks can be left out: then go from Q6b to Q7. They can also run on another day with this round's stick.

**FXProbe comes before MPFree.** It needs Akai's system with developer mode on, which is how the MPC starts the round. On 16 Nov it came last, after about two and a half hours; by then the MPC wasn't in that state, and none of it ran. Coming first also gets the FX thread its measurements of Akai's effects before anything later can cut the round short. Its captures wait on partition 4 until F8 measures them at the end, so the stick makes no extra trip.

## What this round leaves out, and why

- **Q1, Q3 and Q4.** Developer mode's host key was recorded on 16 Nov, and F0 and Q10b say if it changed. Nothing changed on Akai's side since. Q4's backup of 16 Nov (`mpc-one-emmc-20261116T122336Z-hlv5nch1`, verified, and the same as the first start's in every region that must be) stays the return point: nothing in this round writes the eMMC's boot areas, U-Boot, its environment or the root filesystem. Q2 writes only `/data/ssh`, and F2 only two folders on the internal drive, each with its undo.
- **Q7a, Q7b and Q7c's first start.** The boot hook on the MPC is main's: `hook-install.sh` hasn't changed since the 16 Nov round (main `f9824ce2`), so the hook stays as it is. The USB creator has changed (#427, #429), and Q6 writes its new partitions 1, 2, 3 and 5. The first-start setup runs once for each partition 4, and this stick's ran on 16 Nov, so #429's card with the backup's age doesn't show: Q6 would need `--new-backup` for that, a 20 to 30 minute backup. Q10b's hook report shows the hook's files.
- **The free CC0 kits (#437)** stay off this stick: Q0 doesn't build them, so the kit list is the one the checks were written for. The [USB creator round](usb-creator-round.md) tries them.
- **S1 to S4 (the second boot and the hour).** Not asked for this round. They still haven't run on the MPC: say if you want them, and they come back in.
- **Checks that passed on 16 Nov and haven't changed since:** `first-start`, `basics`, `load`, `note-repeat`, `console`, `fx-add` and `power` (Q10's shut down still uses the power card). `wheel` and `midi-in` were skipped on 16 Nov and haven't changed; type their names into Q8's list if you want them.

## This round's checks

The app step asks only about these ([device checks](device-checks.md); `python3 tools/mpc-one/app_checks.py show NAME` prints one in full). For each, it shows a few steps, reads the app's log while you do them, says what it saw, and asks at most one question, about what only you can see or hear.

| Check | Why it's in this round | You do | About |
|---|---|---|---|
| `transport` | #419: on the Looper tab REC made OVERDUB and PLAY blink and STOP didn't let go (step 8); #428: REC closes the free first pass (step 3) | Eight presses on Play and the Looper tab | 2 min |
| `looper` | #428: REC + PLAY records until STOP, STOP keeps the take, the pads are recorded with their echo | Record about a bar and a half, STOP, play it back | 2 min |
| `step-pads` | #426: STEP SEQ makes the pads the steps of one sound, lit by velocity, with a white playhead | Steps on pads 1, 5, 9 and 13, PLAY, ERASE, PAD BANK, pick another sound | 3 min |
| `step-hold` | #431: a step pad held takes the Q-Links, a later one makes a tie, COPY copies steps | Clear the kick's steps first (see Q8), then hold, tie, COPY and UNDO | 3 min |
| `step-locks` | #434: a held step's Sound bank locks the kick's tune and decay on that step only; SHIFT holds an empty step; ERASE takes locks off | Four kicks, two locks, one lock with no note, ERASE and UNDO | 3 min |
| `step-speed` | #441: step taps and turns go into the playing clip in place, with no compile for each | 20 s of fast step taps and a held turn while it plays | 2 min |
| `track-mute` | #418: Play's steps didn't play on 16 Nov ("No audio from the sequencer track") | TRACK MUTE and the pads, over the kick `step-pads` lit | 3 min |
| `eq` | #428: the EQ page's Listen is now Solo band, and the hint line says what it does | "+ EQ" on the Drums row, drag a dot, UNDO, Solo band | 2 min |
| `return-wet` | #422: a reverb added to a send return starts at Mix 100 % | Reverb on Return B, the Drums strip's B send | 1 min |
| `mix-curve` | #423: a delay's or reverb's Mix curve; Equal keeps the level as Mix rises | A latched snare through a Hall, Mix up on Equal, then on Linear | 2 min |
| `mix-curve-click` | #423: a change of Mix curve glides, with no click | Step the Mix curve while the snare plays | 1 min |
| `room-level` | #423: the Room was about 4 dB quieter than the other reverbs | Switch between Room and Plate | 1 min |
| `tape-first-echo` | #430: the Tape delay's first echo has been round the tape once, so it already sounds like tape | A Tape delay on the Drums row, single snare hits | 2 min |
| `dub-echo-dies` | #430: the Tape's Dub echo preset rang on before; now its repeats die away | The Dub echo preset, one snare hit | 1 min |
| `runaway-howls` | #430: the Tape's Runaway preset still grows into a howl on its own | The Runaway preset, one hit, then Feedback down and Remove | 1 min |
| `crush-rate` | #450: Crush keeps exact time at every rate, with band-limited steps | Crush on the Drums row, a latched pad, Rate from 22 kHz to 1 kHz and back | 2 min |
| `crush-filters` | #450: Crush's Filters switch with a crossfade | The 12-bit classic preset, Filters through its four choices, Remove | 1 min |
| `drive-modes` | #445: a change of Drive's mode fades over 10 ms | A Supersaw chord through Drive at about 18 dB, the six modes stepped through | 2 min |
| `amp-turns` | #452: the Amp's Gain changes the grit, not the level, and its knobs glide | The Modern lead preset on a held note, Gain 0 to 10, Treble flicked | 2 min |
| `amp-switches` | #452: the Amp's Model and Cabinet crossfade | Model and Cabinet through their choices on a held note | 1 min |
| `amp-gate` | #452: the Amp's Gate shuts hiss in the rests and doesn't clip a chord's start | The Tight rhythm preset, short chords with rests, Remove | 1 min |
| `fx-busy` | #445: Drive and Character run oversampled; a busy project on the MPC's cores | Character's Pushed tape on four tracks, Drive's Hard clip on two, a minute of play, then all six out | 5 min |
| `midi-monitor` | #421: the I/O view's Monitor, fed from the real MIDI IN, says where each message went; `s` skips it without a keyboard and a 5-pin cable | Play the keyboard, turn a knob | 3 min |
| `midi-now` | #424: the Monitor's Now view shows what a keyboard holds, and Save log keeps the list on the stick; `s` skips it too | Hold a chord, move the mod wheel and the bend, Save log | 2 min |
| `midi-port` | #432: a port's row opens its page, and each MIDI port of a USB interface is listed as a port of its own; `s` skips it too | Open › on the MIDI row, play a key; a USB interface only if you have one | 2 min |
| `mpc-plugin` | #428: the check's step names the row that picks a plugin (16 Nov's named one that isn't there) | Pick [SYN] Mono Voice on the Synth tab, play pads | 2 min |
| `mpc-plugin-install` | #428: the catalog's OpenSSL checked and mended; #419: the install line says where the plugin is | BROWSE > Plugins, two installs, the plugin's screen | 4 min |

**Checked by the app's tests instead (Q0d), not on the MPC:** everything the 16 Nov round listed there, and since then the EQ's finger-to-sound tests, a removed effect's page shutting, touches keeping their order, LATCH keeping the velocity, long option lists scrolling (#419), the Mix curve's own levels (`mpfree-fx`'s `mix_curve` tests, which CI runs), Drive's, Crush's and the Amp's measured targets (fxlab's gates in CI), and the sampler's clean pitching (`mpfree-sampler`'s `clean_pitch` tests, in CI). Q8b times the last two on the MPC.

## You need

- **The MPC on the direct Ethernet cable only.** Developer mode's SSH is still open with no key until Q2 locks it: anyone on any network the MPC is on can get a root shell with no password. If the MPC is on Wi-Fi, switch its Wi-Fi off in Akai's settings first.
- **The stick from the 16 Nov round, out of the MPC at the start.** With the boot hook on the MPC, switching on with the stick in starts MPFree. Q6 keeps its partition 4, with the settings, the buttons Map controls found, the first start's backup, the MPC's kits and samples, and the plugins.
- Akai's 3.10.1 update file in `~/mpfree-work/akai-update-files`, where the earlier rounds put it. The USB creator reads it in Q6.
- For `midi-monitor`, `midi-now` and `midi-port`, a MIDI keyboard with a mod wheel and a pitch bend, and a 5-pin cable; for `midi-port`'s step 2, a USB MIDI interface with two or more MIDI ports if you have one (it is optional); for `mpc-plugin-install`'s catalog, a route out to the internet on the MPC's cable (see Q8).
- For the F blocks, **External Plugin Support** on in Akai's app (Menu > Preferences > Enhancements).

Every block runs in a terminal on the workstation, from the clone at `/home/jesse/code/mpfree-hardware-survey`. Each step's output lands in one results folder, and Q11 packs it into one zip.

## The blocks

**Block Q0: update, check, build** (workstation only). Clone first, only if the clone isn't there yet:

```bash
[ -d /home/jesse/code/mpfree-hardware-survey ] || git clone https://github.com/jferments/mpfree.git /home/jesse/code/mpfree-hardware-survey
```

Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C qual-3101 origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_devmode_login.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_kexec_preflight.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_host_emmc_backup.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && python3 tools/mpc-one/tests/test_write_test_stick.py && python3 tools/mpc-one/tests/test_stick_plugin.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_stick_setup.py && python3 tools/mpc-one/tests/test_creator_recipe.py && python3 tools/mpc-one/tests/test_compare_emmc_backups.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_boot_hook.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_app_mpc.py && python3 tools/mpc-one/tests/test_app_checks.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_read_stick_logs.py && python3 tools/mpc-one/tests/test_results.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app && cd rust/target/armv7-unknown-linux-gnueabihf/release && sha256sum mpfree-screen-log mpfree-startup mpfree' && python3 tools/mpc-one/results.py run build-fetch -- bash -c 'CARGO_ZIGBUILD_PYTHON_PATH=/home/jesse/.local/share/mpfree-zig/bin/python3 cargo zigbuild --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf.2.34 -p mpfree-mpcstore --bin mpfree-fetch && sha256sum rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-fetch' && python3 tools/mpc-one/results.py run build-creator -- bash -c 'cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-stick && sha256sum rust/target/release/mpfree-stick'
```

It ends with five hashes: the three programs for the MPC, then `mpfree-fetch`, then the USB creator's command line, `mpfree-stick`, which makes the stick in Q6. `mpfree-fetch` is the Plugins place's download helper (#429): the pad player is linked statically and can't open the system's libcurl itself, so this one is linked dynamically, with `cargo zigbuild` against glibc 2.34, and Q6 puts it beside the pad player ([the download helper](../design/mpc-plugin-store.md#the-download-helper-on-the-mpc)). It uses block G1a's tools, installed on 16 Nov. If Q0 stops at `build-fetch` with `no such command: zigbuild`, run G1a, then Q0 again. `results.py status` lists what is already in the results folder: if the 16 Nov round left files there, run `python3 tools/mpc-one/results.py zip` first, so this round gets its own zip.

**Block Q0b: build the benchmark** (workstation only; about 20 minutes). These are the builds Q8b runs: as on 16 Nov the plain one, and one tuned for the MPC's Cortex-A17 cores, in a folder of its own; and, new this round, two built the plain way from earlier commits, so Q8b can time Drive, Character, Crush and the Amp just before round 2 changed them:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run build-bench -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc && RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-bench && RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static -C target-cpu=cortex-a17" cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf --target-dir rust/target/a17 -p mpfree-bench && sha256sum rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-bench rust/target/a17/armv7-unknown-linux-gnueabihf/release/mpfree-bench' && python3 tools/mpc-one/results.py run build-bench-before -- bash -c 'set -e; export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static"; out=/home/jesse/mpfree-work/bench-before; mkdir -p "$out"; for c in 11347cb0efa4bd094b9b29ade8747c2f19e18b6c c0c9bce42fcabc94de15682f25db56d7b42bd78f; do src=/home/jesse/mpfree-work/bench-before-src-${c:0:8}; [ -d "$src" ] || git worktree add --detach "$src" "$c"; git -C "$src" log --oneline -1; cargo build --manifest-path "$src/rust/Cargo.toml" --release --locked --target armv7-unknown-linux-gnueabihf --target-dir "$out/target" -p mpfree-bench; cp "$out/target/armv7-unknown-linux-gnueabihf/release/mpfree-bench" "$out/mpfree-bench-${c:0:8}"; done; sha256sum "$out"/mpfree-bench-11347cb0 "$out"/mpfree-bench-c0c9bce4'
```

It ends with two hashes, then the two earlier commits' lines and two more hashes:

- `11347cb0` is main just before #445 (train 64): Drive, Character and Crush as they were before round 2 (#445 didn't touch Crush).
- `c0c9bce4` is #452's first commit: main after #450 with only the two Amp bench cases added, so its Amp is the old one.
- Each earlier commit is checked out once as a git worktree in `~/mpfree-work` (the clone's own folder and branch stay as Q0 left them), and both build into one folder of their own, `~/mpfree-work/bench-before/target`, so the main builds aren't touched.

**Block Q0c: build the FX thread's measuring kit** (workstation only; the SDK's clone from the last round is reused). This is the folder of developer tools Q6 puts on the stick for the F blocks ([fxlab and FXProbe](../design/fxlab.md)). Skip Q0c, and leave `--dev-tools` and the folder after it out of Q6, if you won't do the F blocks:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run build-fxprobe -- bash -c 'set -e; sdk=/home/jesse/code/mpc-plugin-builder-sdk; [ -d "$sdk" ] || git clone https://github.com/inmusic-software/mpc-plugin-builder-sdk "$sdk"; git -C "$sdk" checkout --detach febd2ca5; (cd rust && cargo build --release --locked -p mpfree-fxlab); kit=/home/jesse/mpfree-work/dev-tools-$(git rev-parse --short=12 HEAD); mkdir -p "$kit"; python3 tools/fxprobe/stage.py --sdk "$sdk" --out "$kit/fxprobe"'
```

It ends with the kit's 20 files, about 23 MB in all, and `STAGE-OK`. The kit's folder is named after Q0's commit. If it stops with `… exists; give a new folder`, an earlier Q0c at the same commit made it already: go on.

**Block Q0d: the app's own tests** (workstation only, read-only: it builds into `rust/target`; about 6 minutes). The app's whole test suite at Q0's commit, the UI smoke workflows among them:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-tests -- python3 tools/mpc-one/app_checks.py smoke
```

It prints how many tests ran and passed, then the software side of each device check and each area that is no longer checked on the MPC, and ends `RESULT: every test passed.` If a test fails, it names it under `Failed:` with the first lines of what it printed: go on with the round, and say so with the zip.

**Block G1a: the tools for the GPU programs' pinned build, once** (workstation only; it installs `cargo-zigbuild` into `~/.cargo/bin` and zig into `~/.local/share/mpfree-zig`, and changes nothing else). The GPU programs are linked dynamically against the MPC's C library, so they must not ask for a newer glibc than the MPC's 2.39. Your distribution's ARM cross compiler links against its own glibc (2.43 on 16 Nov), which is why Q9 stopped in the last round. cargo-zigbuild links through zig against glibc 2.34's symbols instead:

```bash
rustup target add armv7-unknown-linux-gnueabihf && cargo install --locked cargo-zigbuild && python3 -m venv /home/jesse/.local/share/mpfree-zig && /home/jesse/.local/share/mpfree-zig/bin/pip install ziglang==0.17.1 && /home/jesse/.local/share/mpfree-zig/bin/python3 -m ziglang version
```

Expect `0.17.1` last. If `python3 -m venv` says `ensurepip is not available`, install your distribution's `python3-venv` package and run the block again. Undo: `cargo uninstall cargo-zigbuild && rm -r /home/jesse/.local/share/mpfree-zig`.

**Block G1: build the GPU benchmark and the page writer** (workstation only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && CARGO_ZIGBUILD_PYTHON_PATH=/home/jesse/.local/share/mpfree-zig/bin/python3 cargo zigbuild --release --locked --manifest-path rust/mpfree-gpu/Cargo.toml --target armv7-unknown-linux-gnueabihf.2.34 --bins && cargo build --release --locked --manifest-path rust/Cargo.toml -p mpfree-app
```

Expect `Finished` twice. G2's step 1 checks that the benchmark asks for nothing newer than glibc 2.34. The ARM benchmark and the workstation's `mpfree`, which writes the sample pages, must come from one build, because the page lists' format is versioned; this block builds both from Q0's commit.

**Block Q6a: which stick** (read-only on the stick; the stick in the workstation; sudo asks for your password). Put only the round's stick in, and leave the MPC's SD card and any other MPFree stick out:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-before -- sudo rust/target/release/mpfree-stick inspect --device mpfree
```

- **Go on when** it lists `partition 4: mpfree-test-data` at `1024 MiB`, says `partition 4: MPFREE-DATA ext4`, and lists the backup from 16 Nov (`backup: mpc-one-emmc-20261116T014409Z`). That is the partition Q6 keeps.
- **Stop if** partition 4 starts anywhere but 1024 MiB or isn't `MPFREE-DATA ext4`: Q6 would then start partition 4 afresh. Send the results instead.
- `no MPFree stick found among the USB disks` or `more than one MPFree stick is in`: put in only the round's stick, and run Q6a again.

**Block Q6: the USB creator writes the stick** (**erases the stick's partitions 1, 2, 3 and 5 and its partition table**, keeps partition 4; the stick in the workstation; sudo; about 5 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- sudo rust/target/release/mpfree-stick write /home/jesse/mpfree-work/akai-update-files --device mpfree --yes-erase mpfree --programs rust/target/armv7-unknown-linux-gnueabihf/release --ssh-key /home/jesse/.ssh/mpc_one_autorun_ed25519.pub --dev-tools /home/jesse/mpfree-work/dev-tools-$(git rev-parse --short=12 HEAD) --work /home/jesse/mpfree-work --record /home/jesse/mpfree-work
```

- **What it does:** as on 16 Nov. It finds the disk Q6a did and names it, checks Akai's update file and unpacks its system into `~/mpfree-work` (about 1.5 GB, deleted at the end), makes partitions 1, 2, 3 and 5 with Q0's programs, writes them, reads them back and compares, then writes the partition table. Partition 4 is not written. With `--record` it saves its report and the stick's SSH host key in a new `~/mpfree-work/creator-stick-<UTC>`, where the round's tools find them.
- **Its report must say:** `note the one MPFree stick that is in: /dev/sd…` (the disk Q6a showed), `note supported: MPC 3.10.1 (MPC One, Gen 1)`, `note kernel release: 6.18.44-az01-2026-08-10-rt6`, `note developer SSH on: root, key only, port 2222; …`, `note partition 4: this stick's saved settings, files and backups are kept`, `note download helper: mpfree-fetch (… bytes, sha256 …): the Plugins place downloads through it` with the hash Q0 printed, the developer tools line with 20 files (or `developer tools: none (no --dev-tools)` if you left Q0c out), `note checked by reading back: SHA-256 …` and `note recorded in /home/jesse/mpfree-work/creator-stick-…`, and it ends `done the stick is ready: put it in the MPC and follow the creator's next steps`. Its next steps are for a new stick: this MPC has the hook already, so follow Q7 instead.
- **If it stops before `Writing the stick`,** nothing on the stick changed. If it is cut off while writing, run Q6 again: partition 4 is kept as long as the partition table can still be read.
- **Undo:** run Q6 from the 16 Nov commit (`f9824ce2`) to write that round's system back; partition 4 stays either way.

**Block Q6b: the two Open MPC plugins, checked on the stick** (the stick still in the workstation after Q6; sudo asks for your password; under a minute). The 16 Nov round put [SYN] Mono Voice (unzipped) and Tidal Verb's zip in partition 4's `files/plugins/`, and Q6 kept them. This finds them, and writes them again only if one is missing:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-plugin -- python3 tools/mpc-one/stick-plugin.py add mono-voice && python3 tools/mpc-one/results.py run stick-plugin-zip -- python3 tools/mpc-one/stick-plugin.py add tidal-verb --as-zip
```

- **Expect** `already on the stick; nothing written` twice. If one was missing, it downloads the release, checks it against the SHA-256 CI pins in `tools/mpc-plugins/catalog-check.txt`, puts it in, and prints `PLUGIN-OK: …`.
- **Undo:** `cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/stick-plugin.py remove mono-voice && python3 tools/mpc-one/stick-plugin.py remove tidal-verb --as-zip`, the stick in the workstation; or keep them for the next rounds.

**Block F0: Akai's system, developer mode on** (read-only; for Q2 and the F blocks). Leave the stick out of the MPC: with the boot hook installed, switching on with it in starts MPFree. Switch the MPC on and wait for Akai's app. **Switch developer mode on** if it is off (hold SHIFT and press MENU, then Developer), and check that **External Plugin Support** is on (Menu > Preferences > Enhancements). Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run status-before -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status
```

- **The status** answers on port 22 and ends `OPEN: a login with no key and no password gets in.` until Q2 locks it.
- **If the logins say `nothing listens on that port`,** developer mode is off: switch it on and run F0 again.
- **If it stops with `did not show the host key recorded`,** the MPC made a new host key. Nothing was changed. Run the 3.10.1 device round's [block Q1](firmware-3.10.1-qualification.md) to record the new one, comparing its fingerprint on the MPC's screen, then F0 again.

**Block Q2 (optional, the owner's choice): lock developer mode's SSH to your key** (**state-changing, on `/data`**: the eMMC's data partition, which survives a restart; you type `LOCK`). It closes the open root login with no key that developer mode ships with. If the [USB creator round](usb-creator-round.md) follows this one (see the end), leave Q2 out: its C3 switches developer mode off, which deletes the lock, and its C10 makes it again. The 16 Nov try changed nothing and undid itself, because the tool turned your SSH agent off; #417 fixed that. **First**, so the tool can use your key without a question:

```bash
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519 && ssh-add -l
```

Expect your key's fingerprint in the list. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devmode-lock -- bash tools/mpc-one/collect-over-ssh.sh --devmode-lock
```

- **What it changes:** it copies `allow-access.conf` and `authorized_keys` into `/data/ssh/mpfree-lock-backup`, adds your public key (`/home/jesse/.ssh/mpc_one_autorun_ed25519.pub`) to `/data/ssh/authorized_keys`, empties `allow-access.conf`, and runs `systemctl reload sshd.service`, the step Akai's login banner gives. It writes nothing else, and only under `/data/ssh`.
- **Before it changes anything,** it checks that your key can be used without a question, and that this is an installed MPC OS with developer mode on and no earlier lock of ours.
- **Passes when** it ends `LOCKED: the login with no key is refused and your key gets in`.
- **If it fails halfway:** it keeps one SSH connection open for the whole step. If the new logins don't come out as expected, it puts both files back over that connection, reloads the server and checks that the open login works again.
- **Undo:** block Q2-undo below.

**F (optional): FXProbe, the FX thread's measuring plugins, under Akai's system.** FXProbe lets fxlab measure the AIR effects on the MPC exactly as it measures MPFree's ([fxlab and FXProbe](../design/fxlab.md)). This is its first time on an MPC, so the first question is whether Akai's app loads the two plugins at all. The kit came onto the stick's setup partition (5, FAT32, which Akai's system mounts at `/media/MPFREE`) in Q6, under `/mpfree-dev/fxprobe`. Akai's system mounts every partition of a stick noexec, so the kit's script runs with `sh`, and the plugins are copied once to the MPC's internal drive, where the app can load them. Captures go to the stick's partition 4, where they wait for F8 at the end. Skip to Q7 if you'd rather not.

Every F block runs the kit's script on the MPC over developer mode's SSH, with your key, from the workstation; F0 showed developer mode on. The stick goes in only while Akai's app is up. **Each time the stick goes in, Akai's app offers to run its `autorun.sh`: tap No.** Yes would install the hook again (unchanged) and restart the MPC into MPFree. Since #427 a stick's setup script also switches developer mode off with Akai's own script, which deletes `/data/ssh` and Q2's lock with it, unless the stick has developer tools; this one has them (Q0c), so it would leave developer mode on.

**Block F1: what is where** (read-only). With Akai's app up, put the stick into the MPC's USB-A port. Tap No when it offers to run `autorun.sh`, answer No or Cancel to anything that offers to update, format or erase, and note what it asked. Wait 10 s. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-check -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh check'
```

It checks the kit against its `SHA256SUMS` and shows the kits and data partitions, the internal drive's mount options, whether the plugins are installed, and the work folder. It ends `CHECK-OK`, and says `FORMAT: mpfree-data 1` for partition 4. If it says `can't open '/media/*/mpfree-dev/fxprobe/fxprobe.sh'`, Akai's system has not mounted the stick: take it out, put it in again, wait 10 s and run F1 again. If it says `Connection refused`, developer mode is off: switch it on and run F1 again.

**Block F2: install the plugins** (**state-changing, on the MPC's internal drive**: two folders, about 1.1 MB, into `/media/az01-internal/VST2 Plugins`; Akai's app keeps running):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-install -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh install'
```

- **What it changes:** it copies `MPFree - VST - FXProbe Source` and `MPFree - VST - FXProbe Tap` to `.fxprobe-staging` at the internal drive's top, checks the copy against the kit's hashes, then moves each folder into `VST2 Plugins` (it makes that folder if there is none), so the app never sees half a plugin. Each folder gets a pointer file, `workfolder.txt`, that finds the work folder on the stick. It refuses a drive that is not mounted read-write or is noexec, and a folder of the same name that isn't FXProbe's. It ends `INSTALL-OK`, and its lines name what changed.
- **If it stops partway:** before the moves, nothing in `VST2 Plugins` changed and the staging folder is gone. Between the two moves, run F2 again, or the undo.
- **Undo:** block F-undo's `remove`. The plugins can also stay for later measuring rounds.

**Block F3: restart Akai's app, so it lists the plugins** (read-only on the MPC). Switch the MPC off first, so the stick's file systems are left clean, then take the stick out. Wait 10 s and switch on. Wait for Akai's app, check that developer mode is still on, and put the stick in again once the app is up. Tap No when it offers to run `autorun.sh`. Wait 10 s.

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

It plays the standard programme through every slot, once at 0 dB and once at -12 dB, and records what reaches each Tap. It prints `slots: 1 2 3; 2 capture(s) of 52.639 s each` and `loaded standard.wav in every Source`, then `captured round1-defaults-g0 from 3 slot(s)` and the same for `round1-defaults-gm12`, and ends `CAPTURE-OK`. If a Source can't read the programme, it stops with `STOP: source-s1: cannot read programmes/standard.wav; …` before anything is recorded. A problem in a report shows as `…; PROBLEMS: …` and exit code 1.

The F blocks end here, with Akai's app up and the stick in. **Turn MASTER VOL back up** to where you play (F5 turned it down), then go to Q7.

**Step Q7: switch on into MPFree** (**state-changing**: at about 6 s Akai's programs stop and MPFree's system runs from the stick on Akai's kernel, with the eMMC and SD card switched off in software; MPFree writes only RAM and the stick). After the F blocks, Akai's app is up with the stick in: switch the MPC off, leave the stick in, and switch it on. Without them, the MPC is off: put the stick into its USB-A port, and switch it on.

- **Expected:** the MPC logo, then MPFree's startup screen, about 16 s after power-on, with Akai's app never drawing. No first-start setup: partition 4 had it on 16 Nov. The pad player opens on Play. Play a few pads.
- **If Akai's app comes up instead, leave the stick in and the MPC on,** and run block Q7-report first: the hook's notes are in RAM and last only until the power-off. Then go to Q10b.
- **If the MPC switches itself off,** the stick's switch-off service refused to go on: the eMMC or SD card was still in use, or would not switch off. It saved its notes to the stick. Go to Q11.
- **If the screen stays on the logo for more than a minute,** hold the power button to switch off, and go to Q11.
- **Undo, always:** switch off, take the stick out, wait 10 s, switch on. Akai's system starts as before.

**Block Q7-report: only if Akai's app came up in Q7** (read-only; the stick still in):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-report-stick-in -- bash tools/mpc-one/collect-over-ssh.sh --hook-report
```

It shows what the hook decided at this power-on and why it stopped.

**Block Q8: this round's checks** (read-only on the MPC: the tool starts, copies and stops nothing and only reads the app's log; you press buttons and pads, the loops stay in RAM, and anything MPFree saves goes to the stick's partition 4; about 75 minutes). Turn the volume down a little before you start.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-checks -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_CHECKS=transport,looper,step-pads,step-hold,step-locks,step-speed,track-mute,eq,return-wet,mix-curve,mix-curve-click,room-level,tape-first-echo,dub-echo-dies,runaway-howls,crush-rate,crush-filters,drive-modes,amp-turns,amp-switches,amp-gate,fx-busy,midi-monitor,midi-now,midi-port,mpc-plugin,mpc-plugin-install bash tools/mpc-one/collect-over-ssh.sh --app
```

- **First, the stick's system,** as on 16 Nov: `started by: the boot hook's soft-reboot (asked at … s after power-on)`, `no MMC block device`, `kexec_load_disabled: 1` and `/: read-only`, ending `CHECKS-OK`. If any is wrong it prints a `PROBLEM` line and stops before it reads anything else. Then the start-up from the log, with no question, and `privileges:` lines with `Uid: 960` and `CapEff: 0000000000000000`.
- **Map controls.** Most buttons these checks use were found on 16 Nov and kept on partition 4. If it lists any as not found yet (Q-LINK, which `step-hold`, `step-locks` and `step-speed` use, may be one), it runs `map-controls` first: find only the buttons it lists, then press MENU.
- **Each check** shows its steps. Press Enter to start (`s` skips the check, `q` stops here), do the steps, and press Enter when you are done. It then prints `what the log showed:`, with `seen`, `NOT SEEN` or `unclear` for each thing it looked for, and asks at most one question. Answer in a few words, with the step's number if one went wrong.
- **`transport`** is first, so REC, OVERDUB, UNDO and PLAY are tried before anything else uses them. Step 3 changed: the first pass now runs until you press REC again, and closes at the end of that bar. Step 8 is 16 Nov's bug: REC on the Looper tab should make OVERDUB and PLAY blink, and STOP should put both out.
- **`looper`** starts by tapping **Clear loop 1** on the Looper tab, because `transport` left a loop there.
- **`step-pads`** (#426), **`step-hold`** (#431), **`step-locks`** (#434) and **`step-speed`** (#441) are STEP SEQ's, and come before `track-mute`, whose step 1 then finds a clip and goes straight to PLAY. All of them come after `looper`, so the loop you judge there has no kick under it; the loop plays along with them, which their questions about the lights don't mind.
- **Between `step-pads` and `step-hold`,** before you press Enter at `step-hold`'s prompt, clear the kick's steps: on Play, hit pad 1 (the kick) and press STEP SEQ. Hold ERASE and hit pads 1 and 13: they go dark, and no pad stays lit. Let go of ERASE and press STEP SEQ: the pads play again. (`step-pads` leaves steps 1 and 13 lit, and `step-hold` starts from an empty kick lane. ERASE on a step that is already empty only says so.)
- **`step-locks`** clears the kick's steps itself in its step 1, so it needs no line before it. **`step-speed`** leaves a few of the kick's steps lit, which `track-mute` plays, and `fx-busy` later plays under its effects. It waits for the next status line, which counts the step edits that went in place.
- **`track-mute`** is #418's first time on the MPC: with the kick lit on Beats, PLAY should play it.
- **`eq`** is asked again for one change: the EQ page's Listen is now **Solo band**, and while it is on the hint line says what it does. It leaves an EQ on the Drums row, which the effect checks after it add theirs behind.
- **`return-wet`, `mix-curve`, `mix-curve-click` and `room-level`** follow each other: the last three share one latched snare and one Hall reverb on the Drums track, and `room-level` takes them out again. `mix-curve` waits up to 40 s after your Enter for the app's next status line, so it can say whether the Hall underran.
- **`tape-first-echo`, `dub-echo-dies` and `runaway-howls`** (#430, FX PR C) share one Tape delay on the Drums row, played by single snare hits, and `runaway-howls` takes it out again. Runaway gets loud on purpose: turn the volume down before its step 1, and Feedback below 100 % stops it. It waits for the next status line too, to say whether the howl underran.
- **`crush-rate` and `crush-filters`** (#450) share one Crush on the Drums row, played by a latched pad 2 of kit 1, and `crush-filters` takes it and Latch out again.
- **`drive-modes`, `amp-turns`, `amp-switches` and `amp-gate`** (#445, #452) play the synth: `drive-modes` turns the wheel to the last kit, "Synth: Poly", and picks its Supersaw preset, and the pads play that until `fx-busy` turns the wheel back to kit 1. Pads 1, 5 and 8 make a chord. Each puts its effect on the Synth row and takes it out again; the three Amp checks share one Amp. `drive-modes` and `amp-turns` wait for the next status line, to say whether the effect underran.
- **`fx-busy`** (#445) is the busy project: Character's Pushed tape (as Drive · Tape) on the Drums, Synth, Loop and Audio rows and Drive's Hard clip (Drive · Hard) on the Drums and Synth rows, then PLAY for the kick and a latched chord for a minute, then all six out. An insert runs whether its track has sound or not, so the quiet Loop and Audio rows cost what they would with sound. It waits for the next status line, so its underrun count covers the whole minute.
- **`midi-monitor`, `midi-now` and `midi-port`:** type `s` at each prompt if no keyboard and 5-pin cable are at hand. `midi-monitor` turns Show developer tools on and off again; none of them sends anything to MIDI OUT. `midi-now`'s Save log writes one file into `files/midi-logs/` on partition 4. `midi-port`'s step 2 is only for a USB MIDI interface with two or more MIDI ports: without one, leave the step out.
- **`mpc-plugin`** changes the synth track's instrument, so it comes near the end.
- **`mpc-plugin-install`** is last. Its step 3 downloads from GitHub through the MPC's Ethernet, which needs a route out to the internet. On the direct cable that is there only if the workstation shares its connection on that port (NetworkManager's "Shared to other computers", which gives the 10.42.0.x addresses this round uses). Don't change your network for it: without a route, the catalog's line says why, and that is the answer. Its first look reads the Plugins place's first line, which now ends with libcurl's OpenSSL: `OpenSSL … ready`, `ready (its random numbers didn't start: …; MPFree loaded its default provider)`, or `not ready: …` with why. Any of these answers 16 Nov's "Insufficient randomness", even without a route out. Tidal Verb was installed from its zip on 16 Nov, so step 2 now says `Updated` instead of `Installed`; both pass.
- **At the end** it lists each check with how many of its lines were seen and your answer, then the session summary since power-on and the app's latest status. It ends `RESULT: all steps ran` and a `NEXT:` line about switching off: leave the MPC on and the stick in, and go on to Q8b.
- **Undo:** none needed. The loops are in RAM, and the power-off in Q10 clears them. Buttons that Map controls finds stay in `settings/map.conf` on partition 4, where they are meant to be.

**Block Q8b: what the engines cost on the MPC** (**RAM only**: it copies the four benchmark builds into `/run/mpfree-bench` and stops the pad player while it runs; press Enter at each of steps 3 to 6; about 45 minutes). The 16 Nov round's Q8b, run again because FX PR C (#430) changed all three delays, round 2 of the FX work changed Drive, Character, Crush and the Amp (#445, #450, #452), and the sampler now pitches up cleanly (#439). It times every engine on the MPC's cores, so the FX quality thread can set the instruction counts CI gates on beside real time on the MPC ([FX quality](../design/fx-quality.md)): the `Delay`, `Tape delay` and `Multi-tap delay` cases (the Delay 70.9k a block, the Tape 87.9k), and `Drive (Eco 2x)`, `Drive (HQ 4x)`, the four `Character` modes, `Chain: drive, filter, delay, comp`, `Crush`, `Amp (Crunch)` and `Amp (Lead, Gain 10)`, each against the same case in the build from before its change. The sampler's cost is in the same run: `Sampler, 20 pads` in the JSON, and in `--afford`, `Sampler, pads held pitched up (clean read)` beside `Sampler, looping pads held` (the read it replaces above two semitones).

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run bench -- env MPFREE_ACCEPT_BENCH=YES python3 tools/mpc-one/host_bench_mpc.py --before 11347cb0=/home/jesse/mpfree-work/bench-before/mpfree-bench-11347cb0 --before c0c9bce4=/home/jesse/mpfree-work/bench-before/mpfree-bench-c0c9bce4
```

- **What it changes:** as on 16 Nov. It talks only to the test build (port 2222, the stick's own host key) and runs the same checks as Q8 first. It copies the four builds into the stick system's RAM and checks their hashes there. For a clean run it stops our pad player (`mpfree-test-app`, RAM state only) and starts it again at the end, also when a run fails or you press `q`. Akai's services are not running and are never touched. The benchmark only computes: it opens no device, writes only its JSON files into `/run/mpfree-bench`, and runs as an ordinary process.
- **Each build runs** `--calibrate`, `--json`, `--afford` and `--lanes --lane-cores 3,2`; then, at step 6, the two earlier builds run `--calibrate` and `--json` only (`11347cb0.json` and `c0c9bce4.json`, about 5 minutes each). Today's numbers are in step 4's `default.json`, and the earlier ones are what they are read against. If you are short of time, type `s` at step 5 to skip the build tuned for the Cortex-A17; step 6 still runs.
- **Passes when** it shows `----- 11347cb0.json -----` and `----- c0c9bce4.json -----`, ends `RESULT: both builds ran (numbers above).` and prints `pad player: active (started again)`.
- **If it is cut off halfway** (the cable, or `q`), the pad player may stay stopped: it starts again at the next power-on. Nothing is left but RAM.
- **Undo:** none needed. The power-off in Q10 clears `/run`.

**Q9 (optional): GPU page flips in the same boot.** These are the GPU thread's page-flip blocks, the ones the last round couldn't run. G1a and G1 built what they need. Nothing else in this round depends on Q9, so skip to Q10 if you'd rather not.

**Block G0: check the clone** (workstation only, read-only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git status --short --branch | head -1 && git log --oneline -1 && grep -c '^def flip' tools/mpc-one/host_gpu_mpc.py
```

Expect three lines: `## qual-3101`, Q0's commit line, and `1`.

**Block G2: the GPU on the MPC, ending with page flips** (**state-changing, RAM only**, at step 7; press Enter at each of steps 3 to 7; about 10 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run gpu-flip -- env MPFREE_ACCEPT_GPU_BENCH=YES python3 -I tools/mpc-one/host_gpu_mpc.py --flip
```

It talks only to the test build that the boot hook started at Q7: port 2222, with the stick's own host key. What it changes, step by step, as in the [3.10.1 device round's Q9](firmware-3.10.1-qualification.md):

- **1:** workstation only. It checks the benchmark (ARM, the MPC's C library, nothing newer than glibc 2.34) and writes the sample pages into a new folder under `~/mpfree-work`.
- **2:** read-only on the MPC: the test build's checks, then the GPU's render node and libraries, and who holds `/dev/dri`. If it stops on `no /dev/dri/renderD*`, nothing was changed: go on to Q10 and tell the GPU thread.
- **3:** RAM. It copies the benchmark and the pages into `/run/mpfree-gpu` and checks their hashes. It stops the pad player (`pad player: was active, now inactive`), and starts it again at the end, even on a failure or Ctrl-C.
- **4 and 5:** these only read and compute; the screen keeps what it shows. Step 4 stops if Mesa answers with anything but the Mali (Panfrost).
- **6:** writes each page to `/dev/fb0` as the app does, 1.5 s each. No display setting changes.
- **7:** first a read-only check that no program has `/dev/dri/card*` open; if one does, it stops and the screen is not changed. Then it draws the first page into a screen buffer and reads it back, and stops before any change if that isn't the expected picture. Only then does it print `SCREEN CHANGES NOW`, take the display (DRM master) and point the lit CRTC at its own buffers, in the mode the screen already runs. No mode, timing, panel or stored setting changes.

**Undo:** automatic. Before it exits, the program puts the CRTC back as it found it and prints `flip: the screen is back as it was`, on a normal end, Ctrl-C, a closed SSH session or a failure. If the program is killed outright, the kernel gives the screen back to the framebuffer when the display's last file closes. Anything left after that is cleared by Q10.

**If it fails partway:**
- **Stopped before `SCREEN CHANGES NOW`:** the screen was not changed. Go on to Q10.
- **Printed `SCREEN CHANGES NOW`, but never "the screen is back as it was":** go straight to Q10, even if the screen looks right.
- **The screen freezes or goes black and the tool hangs:** press Ctrl-C once and wait 10 s. If the screen doesn't come back, go to Q10.

It ends `RESULT: the GPU benchmark ran (numbers above).` and `pad player: active (started again)`. **What to note in the zip's notes:** in steps 6 and 7, is each page the right way up and not garbled? In step 7, does any page tear or flicker? When step 7 ends, does the pad player come back on the screen? Step 7's GPU `whole p50` is the number the page flips are meant to bring below software's (live test 3: the GPU's way to the panel 63 ms a frame, software's 32 ms).

**Block Q10: shut down.** In MPFree, press the power button briefly and tap **Shut down** (#389). If the card asks to save first, tap **Don't save**, or **Save and shut down** to keep the round's project on the stick. Expect "Shutting down…" and the MPC switching itself off within about a minute; note how long it took. If it hangs for more than a minute, or Q9 sent you here with a black or frozen screen, hold the power button to switch off. **Then take the stick out.** It must stay out while Akai's system starts: with the hook installed, switching on with the stick in starts MPFree again.

**Block Q10b: back to 3.10.1, developer mode on** (read-only). With the stick out, switch the MPC on and wait for Akai's app. **Switch developer mode on** if it is off (hold SHIFT and press MENU, then Developer), and note whether it was off: no round has shown yet whether it stays on across MPFree's boot. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run status-after -- bash tools/mpc-one/collect-over-ssh.sh --devmode-status && python3 tools/mpc-one/results.py run hook-report-after -- bash tools/mpc-one/collect-over-ssh.sh --hook-report && python3 tools/mpc-one/results.py run boot-map-after -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

- **The status** answers on port 22 and ends `OPEN: a login with no key and no password gets in.`, or `LOCKED: only your key gets in` if Q2 locked it.
- **If the logins say `nothing listens on that port`,** developer mode is off: switch it on and run Q10b again.
- **If it stops with `did not show the host key recorded`,** the MPC made a new host key. Nothing was changed. Run the 3.10.1 device round's [block Q1](firmware-3.10.1-qualification.md) to record the new one, comparing its fingerprint on the MPC's screen, then Q10b again.
- **The hook report** shows main's three hook files with the hashes of 16 Nov, and `MPFree's stick in now: no`. **The boot map** shows 0 sectors written to p1 to p6 and boot0/1 since this power-on.

**Block F8: measure the captures** (workstation only; read-only on the stick; only if F7 ran). Switch the MPC off, and put the stick (out since Q10) into the workstation. The desktop mounts its data partition at `/media/jesse/MPFREE-DATA`; if it doesn't, mount it the way the file manager does, with `udisksctl mount -b /dev/disk/by-label/MPFREE-DATA`. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-analyse -- bash -c 'set -e; w=/home/jesse/mpfree-work/fxprobe-round1; mkdir -p "$w"; sudo cp -r /media/jesse/MPFREE-DATA/FXProbe/captures /media/jesse/MPFREE-DATA/FXProbe/programmes "$w/"; sudo chown -R "$(id -u):$(id -g)" "$w"; rust/target/release/fxlab analyse "$w"/captures/round1-defaults-g0-s*.wav --schedule "$w/programmes/standard.tsv" --gain 0 --reverb --out "$w/g0"; rust/target/release/fxlab analyse "$w"/captures/round1-defaults-gm12-s*.wav --schedule "$w/programmes/standard.tsv" --gain -12 --reverb --out "$w/gm12"; tar -czf /home/jesse/mpfree-results/current/fxprobe-round1.tar.gz --exclude="*.wav" -C "$w" .'
```

It asks for your sudo password: Akai's system wrote `FXProbe/` as root. It copies the captures and programmes into `~/mpfree-work/fxprobe-round1`, measures each gain with fxlab (built in Q0c), and packs everything but the WAV files into `fxprobe-round1.tar.gz` in the results folder, so Q11's zip carries the numbers. The WAV files stay on your workstation: they are Akai's effects' output, so they never go into a repository.

**Block Q11: the stick logs, and the zip** (read-only on the stick; sudo asks for your password). If the stick is in the MPC, switch the MPC off first, then take the stick out and put it into the workstation (after F8 it is there already):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py run stick-after -- sudo rust/target/release/mpfree-stick inspect --device mpfree ; python3 tools/mpc-one/results.py zip
```

- `read-stick-logs.py` finds the log partition by the PARTUUID in Q6's record, copies it, and reports every boot's files, MAC addresses and serial numbers hidden.
- `inspect` lists what partition 4 holds after the round.
- `zip` prints `UPLOAD THIS ONE FILE: …zip`. Upload it, with your notes from the MPC's screen. The reports quote Akai's files: upload them to this project only.

## Passes when

- Q0d ends `RESULT: every test passed.`
- Q6a shows partition 4 at 1024 MiB with `MPFREE-DATA ext4`. Q6 names the disk Q6a found, says `supported: MPC 3.10.1 (MPC One, Gen 1)` and `partition 4: this stick's saved settings, files and backups are kept`, and ends `done the stick is ready`. Q6b finds both plugins.
- F0 answers on port 22 with the host key of 16 Nov.
- Q2, if run, ends `LOCKED: the login with no key is refused and your key gets in`.
- F1 ends `CHECK-OK`, F2 `INSTALL-OK`, F4 `SETUP-OK`; F6 lists three Sources and three Taps, `idle`; F7 ends `CAPTURE-OK` with three slots at each gain; F8 writes `fxprobe-round1.tar.gz` into the results.
- In Q7 the MPC went from the logo to MPFree with no tap, and Akai's app never drew.
- Q8 ends its checks with `started by: the boot hook's soft-reboot` and `CHECKS-OK`, and each of its checks has every line `seen` and an answer that says it looked and sounded right. A `NOT SEEN` line, or an answer that says otherwise, is a finding: the report has the app's log for that check's time. For `mpc-plugin-install`, the catalog's line without a route out is an answer, not a failure.
- Q8b shows the two earlier builds' JSON and ends `RESULT: both builds ran (numbers above).` (it says so even when you typed `s` at step 5: that step then shows `answer: s`), and the pad player started again.
- G2 ends `RESULT: the GPU benchmark ran (numbers above).`, with `flip: the screen is back as it was` and the pad player started again.
- Q10's shut down switched the MPC off within a minute, and the stick logs have a `power.txt` with `asked: poweroff`.
- Q10b answers on port 22 with the host key of 16 Nov, the hook report shows main's hook files and `MPFree's stick in now: no`, and the boot map shows 0 sectors written to p1 to p6 and boot0/1 since that power-on.
- The stick's `dmesg.txt` shows `mpfree-internal-off: switched off ff0f0000...` before any `sd` or `mmc` mount, and nothing reads the eMMC or SD card before `mpfree-internal-off: done`. The stick logs show no failed unit: since #429 the stick masks Akai's `az0x-loadpin-trustpoints`, which failed harmlessly after the soft-reboot on 15 and 16 Nov, so on this stick any failed unit is a finding. The app's journal lines read `mpfree[…]:` (#429's `SyslogIdentifier=mpfree`), not `sh[…]:`.

## Undo, block by block

| Block | Changes | Undo |
|---|---|---|
| Q0, Q0b, Q0c, Q0d, G1, Q6 | New folders under `~/mpfree-work` (Q6's `creator-stick-<UTC>` record among them) and `~/mpfree-results`, build output in the clone | Delete them |
| G1a | `cargo-zigbuild` in `~/.cargo/bin`, zig in `~/.local/share/mpfree-zig` | `cargo uninstall cargo-zigbuild && rm -r /home/jesse/.local/share/mpfree-zig` |
| Q0b | Two git worktrees, `~/mpfree-work/bench-before-src-11347cb0` and `-c0c9bce4`, known to the clone, and their builds in `~/mpfree-work/bench-before` | `cd /home/jesse/code/mpfree-hardware-survey && git worktree remove --force /home/jesse/mpfree-work/bench-before-src-11347cb0 && git worktree remove --force /home/jesse/mpfree-work/bench-before-src-c0c9bce4 && rm -r /home/jesse/mpfree-work/bench-before` |
| Q6 | The stick's partitions 1, 2, 3 and 5 and its partition table | Q6 again from an earlier commit; partition 4 stays |
| Q6b | Nothing if both plugins were there; else the missing one on partition 4 | `stick-plugin.py remove`, as in Q6b |
| Q7, Q8, Q8b, Q9 | RAM on the MPC; the stick's log partition and partition 4 | Switch off, take out the stick, wait 10 s, switch on |
| Q8 (`mpc-plugin-install`) | Tidal Verb updated from its zip, and with a route out Modal Voice from the catalog, as folders in `files/plugins/` on partition 4; the catalog in `files/plugins/.catalog.json` and the download in `files/plugins/.downloads/` | BROWSE > Plugins: Remove on each installed plugin; or keep them |
| Q8 (`midi-now`) | One file in `files/midi-logs/` on partition 4 | Delete it in BROWSE, or keep it |
| Q10 (if you saved) | The project, in `Projects` on partition 4 | Delete it in BROWSE > Projects, if you like |
| Q2 | `/data/ssh` on the MPC | Block Q2-undo |
| F2 | Two plugin folders in `/media/az01-internal/VST2 Plugins` on the MPC's internal drive | Block F-undo's `remove` (they can stay) |
| F4, F7 | `FXProbe/` on the stick's partition 4, the captures in it | Block F-undo's `clean` (copy the captures off first) |
| F8 | `~/mpfree-work/fxprobe-round1`, and `fxprobe-round1.tar.gz` in the results | Delete them |

The boot hook stays installed: it is how MPFree starts. To take it off the MPC, use the 3.10.1 device round's [block Q7b-undo](firmware-3.10.1-qualification.md#undo-block-by-block).

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

When the backup folder holds a file named `absent` instead of `authorized_keys`, there was none before the lock: the second line is then `rm /data/ssh/authorized_keys`.

**Block F-undo: take FXProbe off** (**state-changing**: `remove` on the MPC's internal drive, `clean` on the stick's partition 4; Akai's app up, the stick put in once it is up, and No to its `autorun.sh`). Each line can run alone. `remove` deletes exactly the two plugin folders F2 copied, and `VST2 Plugins` if F2 made it and it is empty; it ends `REMOVE-OK`. Restart Akai's app afterwards, stick out, so it stops listing them. `clean` deletes `FXProbe/` from the stick's partition 4, captures included, so copy them off first (F8 does); it ends `CLEAN-OK`:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-remove -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh remove'
```

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run fx-clean -- ssh -T -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i /home/jesse/.ssh/mpc_one_autorun_ed25519 root@10.42.0.141 'sh /media/*/mpfree-dev/fxprobe/fxprobe.sh clean'
```

Without the stick, by hand in the developer panel's terminal: `rm -r "/media/az01-internal/VST2 Plugins/MPFree - VST - FXProbe Source" "/media/az01-internal/VST2 Plugins/MPFree - VST - FXProbe Tap"`.

## Where the results go

| Part | Goes to |
|---|---|
| `transport`, `looper`, `eq`, `mpc-plugin`, `mpc-plugin-install`, `return-wet` | The app thread (#418, #419, #422, #428) |
| `step-pads`, `step-hold`, `step-locks`, `step-speed`, `track-mute` | The sequencer thread (#426, #431, #434, #441, #418) |
| `mix-curve`, `mix-curve-click`, `room-level`, `tape-first-echo`, `dub-echo-dies`, `runaway-howls`, `crush-rate`, `crush-filters`, `drive-modes`, `amp-turns`, `amp-switches`, `amp-gate`, `fx-busy`, Q8b's delay, Drive, Character, Crush and Amp cases with the earlier builds', F1 to F8 | The FX quality thread (#423, #430, #445, #450, #452) |
| Q8b's `Sampler, 20 pads` and `Sampler, pads held pitched up (clean read)` | The sampler thread (#439) |
| `midi-monitor`, `midi-now`, `midi-port` | The MIDI thread (#421, #424, #432) |
| Q9 (G2) | The GPU thread: G2's `whole p50` for the panel against software's |
| F0, Q2, Q10b, the stick logs | The device rounds' notes |

## After this round: the USB creator round (optional)

The [USB creator round](usb-creator-round.md) can follow in the same sitting, from its C0, once Q11 has made this round's zip. It follows a new musician's install from developer mode off, and the MPC is then where its C1 expects it once you switch it on with the stick out: Akai's system, developer mode on, the boot hook installed. It needs:

- **A spare stick of 16 GB or more**, which its C4 erases. With only this round's stick, C4's **Erase everything and start over** also erases its partition 4: the plugins, the buttons Map controls found, FXProbe's work folder (F8 copied the captures off first) and the copy of the 16 Nov backup (another copy is in `~/mpfree-backups`).
- **About 90 minutes.** It ends with developer mode off, which deletes `/data/ssh` and Q2's lock with it; its C10 switches developer mode on and locks it again for the round after.

It makes its own zip in C9.
