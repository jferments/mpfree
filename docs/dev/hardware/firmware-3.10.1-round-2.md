# The second 3.10.1 round: everything merged since 16 Nov, played on the MPC by the app itself

**Status:** runbook, 2026-11-16, rewritten on 2026-11-19 for a hands-off round (the owner, 19 Nov: "minimal manual involvement except where it is absolutely necessary"): the app plays seven workflows on the MPC by itself, the benchmark times every engine with the NEON kernels on and off ([the automated device suite](device-suite.md), #692), and only what a person has to do stays by hand. Rehearsed word for word on 19 Nov at `771af5c4` (train 116's main with this page; this line aside) with the device-rounds kit, as the owner's user at the owner's paths against a stand-in MPC: every block passed except the optional G2, which stops at its read-only GPU check (a container has no GPU). The first run found that Q8e always failed the effects workflow, fixed in the same pull request. Not run yet: every block that touches the MPC waits for the owner's go. **Scope:** the reference MPC One (Gen 1) on **MPC OS 3.10.1**, the firmware it has run since 15 Nov and the one all MPFree work targets (the owner, 19 Nov), with developer mode on and MPFree's boot hook installed, as the [3.10.1 device round](firmware-3.10.1-qualification.md) left it on 16 Nov. **Follows:** that round, whose blocks this one reuses word for word where nothing changed.

The 16 Nov round showed main working on the installed 3.10.1: the boot hook, the USB creator's stick, its first start and backup, and most of the app's checks. Since then the sequencer, the looper, the one Sample page with its Slice, Warp and Grain tools, the synth engines, the rebuilt effects, MIDI, Perform and the NEON kernels have merged. Checking them one by one by hand would take over three hours: round 2's list had grown to about 70 checks. Instead the app plays them. Seven workflows, each a musician's session that uses several tools at once, press the buttons and pads, turn the Q-Links and tap the screen by themselves, check the app's log as they go, and report every part's underruns and render times; the heavy one plays again on each sound-core setting. CI plays the same files on every pull request, so a broken workflow shows up there before the round. The benchmark times every engine with the NEON kernels on and off, and what each note costs as it starts. One report page at the end answers keep or drop NEON, the best sound-core setting, `GRAIN_BUDGET` and `VOICE_COST`, and what the new engines cost per voice. By hand stays only what software can't press or judge: the real pads, wheel, touch panel and button chords, the Sample page's new checks, and the two plugin checks.

Nothing of the workflows or the new bench runs has run on ARM or on an MPC yet, so this round is their first real run. That is why they run in RAM only, and why a workflow that fails doesn't stop the others.

## The round at a glance

| Part | Blocks | Where | About |
|---|---|---|---|
| Builds, and the app's tests | Q0, Q0b, Q0d | Workstation | 50 min |
| The stick | Q6a which stick, Q6 the USB creator writes it, Q6b the plugins | Workstation | 10 min |
| MPFree | Q7 switch on with the stick in, Q8 what the engines cost, Q8b the workflows (hands off), Q8c the checks by hand, Q8e the report, Q10 shut down | MPC on the stick | 100 min, about 30 of them by hand |
| Akai's system, after MPFree | Q10b back to 3.10.1: status, hook report, boot map | MPC on 3.10.1 | 5 min |
| The logs and the zip | Q11 | Workstation | 10 min |
| Optional, left out unless asked for | Q8d checks with MIDI gear; Q0c and F0 to F8, FXProbe under Akai's system; G1a, G1 and Q9, the GPU page flips; Q2, the lock | | |

The round can stop after any block that leaves Akai's system or MPFree running: Q10 and then Q11 end it from anywhere. Without the optional blocks, go from Q6b straight to Q7, and from Q8e to Q10. They can also run on another day with this round's stick.

**If you do FXProbe, it comes before MPFree.** It needs Akai's system with developer mode on, which is how the MPC starts the round, and its captures wait on partition 4 until F8 measures them at the end. It needs about 35 minutes of taps in Akai's app, so this hands-off round leaves it out unless the FX thread asks for it.

## What this round leaves out, and why

- **The 24 device checks the workflows play.** `load`, `track-mute`, `step-pads`, `step-hold`, `step-speed`, `rec-arm`, `loop-tracks`, `loop-cell`, `looper`, `looper-deck`, `live-erase`, `mute-queue`, `reload-on-the-bar`, `note-repeat`, `sample-live-loop`, `sample-edit-pad`, `sample-r3-kit-per-track`, `sample-warp`, `grain-movement`, `grain-cloud-view`, `grain-presets`, `fx-add`, `comp-moves` and `fx-busy`: each workflow names the checks it stands in for (its `covers` line), plays their steps and judges their log lines. Their questions about what a person sees or hears are replaced by one listening question at the end of the workflows.
- **The effects' listening checks** (`eq-quality`, the `*-rebuilt` reverbs and modulation, `crush-*`, `comp-style`, `filter-*`, `fuzz-*`, `lofi-*`, `limiter-*`, `gate-bass`, `transient-snap`, `deesser-voice`, `strip-switches`, `ducker-pump`, `tremolo-lfo`, `midi-effects-heard` and the rest of the effect chain). They hear the same code as fxlab's renders and CI's quality gates. What the MPC adds, its own load and timing, is in the workflows and the benchmark.
- **The other device checks,** for the same reason: CI's smoke tests judge their log lines on every pull request (each check lists its tests), and Q0d runs all of them at this round's commit.
- **The effects' before-and-after timings.** The earlier round 2 plan built the benchmark at up to eight earlier commits, so Q8 could time each effect just before its rewrite. CI's instruction counts already gate each change against the commit before it, and each earlier build would add about 5 minutes to build and 5 on the MPC. Q8 times today's engines.
- **Q1, Q3 and Q4.** Developer mode's host key was recorded on 16 Nov, and Q10b says if it changed. Nothing changed on Akai's side since. Q4's backup of 16 Nov (`mpc-one-emmc-20261116T122336Z-hlv5nch1`, verified, and the same as the first start's in every region that must be) stays the return point: nothing in this round writes the eMMC's boot areas, U-Boot, its environment or the root filesystem.
- **Q7a, Q7b and Q7c's first start.** The boot hook on the MPC is main's: `hook-install.sh` hasn't changed since the 16 Nov round (main `f9824ce2`), so the hook stays as it is. The USB creator has changed, and Q6 writes its new partitions 1, 2, 3 and 5. The first-start setup runs once for each partition 4, and this stick's has run already. Q10b's hook report shows the hook's files.
- **The free CC0 kits (#437)** stay off this stick: Q0 doesn't build them. The [USB creator round](usb-creator-round.md) tries them.
- **Q2, the lock.** The USB creator round follows in the same sitting, and its C3 switches developer mode off, which deletes the lock; its C10 makes it again.
- **`power`:** Q10's shut down opens the power card from the real button. **`first-start`, `kept` and `hour`:** this stick's first start has run, and S1 to S4 (the second boot and the hour) are not asked for this round; say if you want them, and they come back in.

## What the app plays, and what stays by hand

**Q8b: the workflows** ([the automated device suite](device-suite.md#the-workflows); `tools/mpc-one/devtest/workflows/`). About 7.5 minutes, then `heavy-load` again on each of the four sound-core settings. Hands off the MPC while they play.

| Workflow | What a musician does | Stands in for | About |
|---|---|---|---|
| `beat-from-scratch` | Steps on the pads with STEP SEQ, a step erased, a held step turned; a recorded take; mutes that wait for the bar in Perform | `rec-arm`, `step-pads`, `step-hold`, `step-speed`, `track-mute`, `mute-queue`, `live-erase` | 26 s |
| `loops` | A loop recorded from Play and overdubbed; Speed, Reverse, mute and a second loop on the Looper tab; Capture; loops launched, stopped and played as a scene from Arrange | `looper`, `rec-arm`, `loop-tracks`, `looper-deck`, `loop-cell` | 38 s |
| `every-synth` | Each of the twelve engines on Track 1 in turn: eight notes held, then a run; the arp latched on Poly | `load`, `note-repeat`, and each engine's cost at eight voices | 2 min |
| `effects` | A beat playing while a compressor, a drive, a delay and a reverb go on the drums, turned, run for 10 s and taken off | `fx-add`, `fx-busy`, `comp-moves` | 27 s |
| `sample-page` | A loop's end moved while it sounds, ping-pong, Slice, Warp, Grain with its LFO and presets, Keep last 4 bars, UNDO, then Edit pad from the Pads tab's row and from PROG EDIT | `sample-live-loop`, `sample-warp`, `grain-movement`, `grain-cloud-view`, `grain-presets`, `sample-edit-pad` | 33 s |
| `songs` | The three demo songs opened from BROWSE, played through two scenes, a track muted on the bar, the tempo moved, saved, and the first opened again | `mute-queue`, `reload-on-the-bar`, `sample-r3-kit-per-track` | 2 min |
| `heavy-load` | A full song, a loop recorded over it, the pads rolling, and a reverb, delay and chorus on the drums: half a minute of everything at once | `load`, `fx-busy` | 1.5 min |

**Q8c: the checks by hand** ([device checks](device-checks.md); `python3 tools/mpc-one/app_checks.py show NAME` prints one in full). For each, the app step shows a few steps, reads the app's log while you do them, says what it saw, and asks at most one question, about what only you can see or hear.

| Check | Why it's by hand | You do | About |
|---|---|---|---|
| `basics` | The real pads, PAD BANK, FULL LEVEL, the wheel, the Q-Links and touch reaching today's app (the workflows put their presses in after the controller) | Every pad soft and hard, PAD BANK, FULL LEVEL, the wheel, the Q-Links, a few taps | 3 min |
| `wheel` | The real wheel and its push; skipped on 16 Nov | The outline, a Q-Link card, a block on the Song view | 3 min |
| `touch` | The real panel's jitter and report rate, which the workflows' exact taps don't have; touch tuning has never been tried on the MPC | Touch lab's 9 steps, Calibrate touch, two throws at once, two clip drags | 6 min |
| `console` | Button chords on the real panel | The Console chord, TAP TEMPO, STOP | 2 min |
| `sample-r1-reach` | Sampler R1's worker while real pads play; the sampler thread asked for it before the next two | Normalize, New sound while pads play, a Chop, Export, a layer | 4 min |
| `sample-audio-track` | The one Sample page's Put on Audio track (#673): CI's session has too little audio track memory to place the slices | Four slices to an audio track, one Undo | 2 min |
| `sample-keep-take` | Keep last 4 bars (#612, #662): CI's session keeps no master, so there is no take to chop | Ten seconds of pads, Keep, Chop to pads, UNDO | 2 min |
| `mpc-plugin` | #428: the row that picks a plugin instrument, not yet seen working on the MPC | Pick [SYN] Mono Voice on the Synth tab, play pads | 2 min |
| `mpc-plugin-install` | #428: the catalog's OpenSSL, 16 Nov's "Insufficient randomness"; #419: the install line says where the plugin is | BROWSE > Plugins, two installs, the plugin's screen | 4 min |

**Q8d (optional): the checks with MIDI gear,** only with the gear at hand; `s` skips any check you don't have the gear for. The workflows send no MIDI, because CI's session drops notes from gear.

| Check | You need | About |
|---|---|---|
| `midi-in` | A MIDI keyboard and a 5-pin cable into MIDI IN | 3 min |
| `midi-learn` | A keyboard or controller with a knob on MIDI IN | 3 min |
| `midi-thru` | A keyboard on MIDI IN and a synth on MIDI OUT | 2 min |
| `clip-program` | A synth on MIDI OUT that takes program changes | 3 min |
| `cc-lanes` | A synth on MIDI OUT whose cutoff follows a CC | 3 min |

**Checked by the app's tests instead (Q0d), not on the MPC:** the whole app test suite at Q0's commit, CI's smoke session with the same seven workflows among it.

## You need

- **The MPC on the direct Ethernet cable only.** Developer mode's SSH is open with no key (this round leaves Q2's lock out): anyone on any network the MPC is on can get a root shell with no password. If the MPC is on Wi-Fi, switch its Wi-Fi off in Akai's settings first.
- **The round's stick, out of the MPC at the start:** the stick from the 16 Nov round, or the one the Windows creator made on 17 Nov if you used that one since. With the boot hook on the MPC, switching on with the stick in starts MPFree. Q6 keeps its partition 4, with the settings, the buttons Map controls found, the first start's backup, the MPC's kits and samples, and the plugins.
- Akai's 3.10.1 update file in `~/mpfree-work/akai-update-files`, where the earlier rounds put it. The USB creator reads it in Q6.
- **The volume where you can hear it** during Q8b: its one question is whether anything clicked, dropped out, stuck or sounded wrong.
- For `mpc-plugin-install`'s catalog, a route out to the internet on the MPC's cable (see Q8c); without one its catalog line says why, and that is an answer too.
- Only for Q8d, the MIDI gear its table names. Only for the F blocks, **External Plugin Support** on in Akai's app (Menu > Preferences > Enhancements).

Every block runs in a terminal on the workstation, from the clone at `/home/jesse/code/mpfree-hardware-survey`. Each step's output lands in one results folder, and Q11 packs it into one zip.

## The blocks

**Block Q0: update, check, build** (workstation only). Clone first, only if the clone isn't there yet:

```bash
[ -d /home/jesse/code/mpfree-hardware-survey ] || git clone https://github.com/jferments/mpfree.git /home/jesse/code/mpfree-hardware-survey
```

Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C qual-3101 origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py status && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_devmode_login.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_kexec_preflight.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_host_emmc_backup.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && python3 tools/mpc-one/tests/test_write_test_stick.py && python3 tools/mpc-one/tests/test_stick_plugin.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_stick_setup.py && python3 tools/mpc-one/tests/test_creator_recipe.py && python3 tools/mpc-one/tests/test_compare_emmc_backups.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_boot_hook.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_app_mpc.py && python3 tools/mpc-one/tests/test_app_checks.py && MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_read_stick_logs.py && python3 tools/mpc-one/tests/test_results.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_host_devtest_mpc.py && python3 tools/mpc-one/tests/test_devtest_report.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_host_bench_mpc.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app && cd rust/target/armv7-unknown-linux-gnueabihf/release && sha256sum mpfree-screen-log mpfree-startup mpfree' && python3 tools/mpc-one/results.py run build-helpers -- bash -c 'CARGO_ZIGBUILD_PYTHON_PATH=/home/jesse/.local/share/mpfree-zig/bin/python3 cargo zigbuild --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf.2.34 -p mpfree-mpcstore --bin mpfree-fetch -p mpfree-plughost --bin mpfree-plughost && sha256sum rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-fetch rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-plughost' && python3 tools/mpc-one/results.py run build-creator -- bash -c 'cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-stick && sha256sum rust/target/release/mpfree-stick'
```

It ends with six hashes: the three programs for the MPC, then `mpfree-fetch` and `mpfree-plughost`, then the USB creator's command line, `mpfree-stick`, which makes the stick in Q6. The pad player is linked statically, so two helpers beside it are linked dynamically, with `cargo zigbuild` against glibc 2.34, and Q6 puts both beside it. `mpfree-fetch` is the Plugins place's download helper (#429), since the pad player can't open the system's libcurl itself ([the download helper](../design/mpc-plugin-store.md#the-download-helper-on-the-mpc)). `mpfree-plughost` is the process each plugin runs in: in the 19 Nov round the pad player served as its own plugin process and [SYN] Mono Voice crashed while loading at every start ([plugins](../design/plugins.md#in-the-app)). They use block G1a's tools (cargo-zigbuild and zig), which were on the workstation for the 17 Nov Windows run. If Q0 stops at `build-helpers` with `no such command: zigbuild`, run G1a, then Q0 again. The local checks now include the workflow runner's, the report's and the benchmark tool's tests (#692). `results.py status` lists what is already in the results folder: if an earlier run left files there, run `python3 tools/mpc-one/results.py zip` first, so this round gets its own zip.

**Block Q0b: build the benchmark** (workstation only; about 15 minutes). These are the builds Q8 runs, as on 16 Nov: the plain one, and one tuned for the MPC's Cortex-A17 cores, in a folder of its own. The plain one also runs Q8's two new parts, the note starts and every engine with the NEON kernels off (#692):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run build-bench -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc && RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-bench && RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static -C target-cpu=cortex-a17" cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf --target-dir rust/target/a17 -p mpfree-bench && sha256sum rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-bench rust/target/a17/armv7-unknown-linux-gnueabihf/release/mpfree-bench'
```

It ends with two hashes. No earlier commits are built this round (see what it leaves out).

**Block Q0c (optional, only for the F blocks): build the FX thread's measuring kit** (workstation only; the SDK's clone from the last round is reused). This is the folder of developer tools that Q6's second command puts on the stick for the F blocks ([fxlab and FXProbe](../design/fxlab.md)). Leave Q0c out if you won't do the F blocks:

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

**Block G1 (optional, only for Q9): build the GPU benchmark and the page writer** (workstation only):

```bash
cd /home/jesse/code/mpfree-hardware-survey && CARGO_ZIGBUILD_PYTHON_PATH=/home/jesse/.local/share/mpfree-zig/bin/python3 cargo zigbuild --release --locked --manifest-path rust/mpfree-gpu/Cargo.toml --target armv7-unknown-linux-gnueabihf.2.34 --bins && cargo build --release --locked --manifest-path rust/Cargo.toml -p mpfree-app
```

Expect `Finished` twice. G2's step 1 checks that the benchmark asks for nothing newer than glibc 2.34. The ARM benchmark and the workstation's `mpfree`, which writes the sample pages, must come from one build, because the page lists' format is versioned; this block builds both from Q0's commit.

**Block Q6a: which stick** (read-only on the stick; the stick in the workstation; sudo asks for your password). Put only the round's stick in, and leave the MPC's SD card and any other MPFree stick out:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-before -- sudo rust/target/release/mpfree-stick inspect --device mpfree
```

- **Go on when** it lists `partition 4: mpfree-test-data` at `1024 MiB`, says `partition 4: MPFREE-DATA ext4`, and lists a backup: the one from 16 Nov (`backup: mpc-one-emmc-20261116T014409Z`) on the 16 Nov round's stick, or the one from 17 Nov (`backup: mpc-one-emmc-20261117T173519Z`) on the stick the Windows creator made. That is the partition Q6 keeps.
- **Stop if** partition 4 starts anywhere but 1024 MiB or isn't `MPFREE-DATA ext4`: Q6 would then start partition 4 afresh. Send the results instead.
- `no MPFree stick found among the USB disks` or `more than one MPFree stick is in`: put in only the round's stick, and run Q6a again.

Not sure which stick is the round's? This checks the one that is in, and the backup on it (read-only on the stick; sudo asks for your password; it copies the backup into a new `~/mpfree-work/stick-check-<UTC>/`, about 7.3 GiB, and compares it with Q4's; about 20 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-check -- bash tools/mpc-one/check-stick-backup.sh
```

- **Go on when** it ends `RESULT: this is the round's stick.`
- `RESULT: NOT the stick the round's notes name (...), but its backup is intact and this MPC's`: this stick can be the round's too. Say so before Q6.
- Any other `RESULT` says what it found and what to do. One with `NOT` means not this stick: put in another and run it again.

**Block Q6: the USB creator writes the stick** (**erases the stick's partitions 1, 2, 3 and 5 and its partition table**, keeps partition 4; the stick in the workstation; sudo; about 5 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- sudo rust/target/release/mpfree-stick write /home/jesse/mpfree-work/akai-update-files --device mpfree --yes-erase mpfree --programs rust/target/armv7-unknown-linux-gnueabihf/release --ssh-key /home/jesse/.ssh/mpc_one_autorun_ed25519.pub --work /home/jesse/mpfree-work --record /home/jesse/mpfree-work
```

Only if you do the F blocks (after Q0c), run this one instead, which also puts Q0c's kit on the stick:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- sudo rust/target/release/mpfree-stick write /home/jesse/mpfree-work/akai-update-files --device mpfree --yes-erase mpfree --programs rust/target/armv7-unknown-linux-gnueabihf/release --ssh-key /home/jesse/.ssh/mpc_one_autorun_ed25519.pub --dev-tools /home/jesse/mpfree-work/dev-tools-$(git rev-parse --short=12 HEAD) --work /home/jesse/mpfree-work --record /home/jesse/mpfree-work
```

- **What it does:** as on 16 Nov. It finds the disk Q6a did and names it, checks Akai's update file and unpacks its system into `~/mpfree-work` (about 1.5 GB, deleted at the end), makes partitions 1, 2, 3 and 5 with Q0's programs, writes them, reads them back and compares, then writes the partition table. Partition 4 is not written. With `--record` it saves its report and the stick's SSH host key in a new `~/mpfree-work/creator-stick-<UTC>`, where the round's tools find them.
- **Its report must say:** `note the one MPFree stick that is in: /dev/sd…` (the disk Q6a showed), `note supported: MPC 3.10.1 (MPC One, Gen 1)`, `note kernel release: 6.18.44-az01-2026-08-10-rt6`, `note developer SSH on: root, key only, port 2222; …`, `note partition 4: this stick's saved settings, files and backups are kept`, `note download helper: mpfree-fetch (… bytes, sha256 …): the Plugins place downloads through it` and `note plugin process: mpfree-plughost (… bytes, sha256 …): each plugin runs in it`, each with the hash Q0 printed, `developer tools: none (no --dev-tools)` (or, from the second command, the developer tools line with 20 files), `note checked by reading back: SHA-256 …` and `note recorded in /home/jesse/mpfree-work/creator-stick-…`, and it ends `done the stick is ready: put it in the MPC and follow the creator's next steps`. Its next steps are for a new stick: this MPC has the hook already, so follow Q7 instead.
- **If it stops before `Writing the stick`,** nothing on the stick changed. If it is cut off while writing, run Q6 again: until the end the stick's table shows partition 4 alone, so Q6 finds it and keeps it. (Earlier creators cleared the table while they wrote, and a second run marked partition 4 new: with one of those, send the output instead of running Q6 again.)
- **Undo:** run Q6 from an earlier commit, such as the 16 Nov round's (`f9824ce2`), to write that system back; partition 4 stays either way.

**Block Q6b: the two Open MPC plugins, checked on the stick** (the stick still in the workstation after Q6; sudo asks for your password; under a minute). The 16 Nov round put [SYN] Mono Voice (unzipped) and Tidal Verb's zip in partition 4's `files/plugins/`, and Q6 kept them; a stick the Windows creator made on 17 Nov has neither yet. This finds them, and writes one only if it is missing:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-plugin -- python3 tools/mpc-one/stick-plugin.py add mono-voice && python3 tools/mpc-one/results.py run stick-plugin-zip -- python3 tools/mpc-one/stick-plugin.py add tidal-verb --as-zip
```

- **Expect** `already on the stick; nothing written` twice on the 16 Nov round's stick. If one was missing, it downloads the release, checks it against the SHA-256 CI pins in `tools/mpc-plugins/catalog-check.txt`, puts it in, and prints `PLUGIN-OK: …`.
- **Undo:** `cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/stick-plugin.py remove mono-voice && python3 tools/mpc-one/stick-plugin.py remove tidal-verb --as-zip`, the stick in the workstation; or keep them for the next rounds.

**Block F0 (optional, for Q2 and the F blocks): Akai's system, developer mode on** (read-only). Leave the stick out of the MPC: with the boot hook installed, switching on with it in starts MPFree. Switch the MPC on and wait for Akai's app. **Switch developer mode on** if it is off (hold SHIFT and press MENU, then Developer), and check that **External Plugin Support** is on (Menu > Preferences > Enhancements). Then:

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

- **Expected:** the MPC logo, then MPFree's startup screen, about 16 s after power-on, with Akai's app never drawing. No first-start setup: this partition 4 has had it. The pad player opens on Play. Play a few pads.
- **If Akai's app comes up instead, leave the stick in and the MPC on,** and run block Q7-report first: the hook's notes are in RAM and last only until the power-off. Then go to Q10b.
- **If the MPC switches itself off,** the stick's switch-off service refused to go on: the eMMC or SD card was still in use, or would not switch off. It saved its notes to the stick. Go to Q11.
- **If the screen stays on the logo for more than a minute,** hold the power button to switch off, and go to Q11.
- **Undo, always:** switch off, take the stick out, wait 10 s, switch on. Akai's system starts as before.

**Block Q7-report: only if Akai's app came up in Q7** (read-only; the stick still in):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-report-stick-in -- bash tools/mpc-one/collect-over-ssh.sh --hook-report
```

It shows what the hook decided at this power-on and why it stopped.

**Block Q8: what the engines cost on the MPC** (**RAM only**: it copies the two benchmark builds into `/run/mpfree-bench` and stops the pad player while it runs; press Enter at each of steps 3 to 5; about 40 minutes, and the MPC can stay as it is). As on 16 Nov it times every engine on the MPC's cores, and the plain build runs two new parts (#692): `--starts`, what a note costs as it starts on every preset, and `--plain --json default-plain.json`, every engine again with the NEON kernels off (their plain Rust twins instead), for the keep-or-drop A/B. Q8e reads them.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run bench -- env MPFREE_ACCEPT_BENCH=YES python3 tools/mpc-one/host_bench_mpc.py
```

- **What it changes:** as on 16 Nov. It talks only to the test build (port 2222, the stick's own host key) and runs the app step's read-only checks first. It copies the two builds into the stick system's RAM and checks their hashes there. For a clean run it stops our pad player and log display (`mpfree-test-app`, `mpfree-test-screen`: RAM state only) and starts whichever was running again at the end, also when a run fails or you press `q`. Akai's services are not running and are never touched. The benchmark only computes: it opens no device, writes only its JSON files into `/run/mpfree-bench`, and runs as an ordinary process.
- **Each build runs** `--calibrate`, `--json`, `--afford` and `--lanes --lane-cores 3,2`; the plain build (step 4, about 15 to 25 minutes) then also `--starts` and `--plain`. Step 5 is the build tuned for the Cortex-A17 (10 to 15 minutes): if you are short of time, type `s` there to skip it.
- **Passes when** it shows `----- default.json -----` and `----- default-plain.json -----`, ends `RESULT: both builds ran (numbers above).` and prints `pad player: active (started again)`.
- **If it is cut off halfway** (the cable, or `q`), the pad player may stay stopped: it starts again at the next power-on. Nothing is left but RAM.
- **Undo:** none needed. The power-off in Q10 clears `/run`.

**Block Q8b: the workflows, played by the app** (**RAM only**: it copies `mpfree` and the seven workflows into `/run/mpfree-devtest` and stops the pad player while they play; **hands off the MPC**; three presses of Enter and one answer; about 15 minutes). This is the round's main part ([the automated device suite](device-suite.md)):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devtest -- env MPFREE_ACCEPT_DEVTEST=YES python3 tools/mpc-one/host_devtest_mpc.py
```

- **What it changes:** it talks only to the test build (port 2222, the stick's own host key), runs the app step's read-only checks and checks that the stick's kits are at `/run/mpfree-kits/kits.bin`, then copies Q0's `mpfree` and the workflows into `/run/mpfree-devtest` and checks every hash there. It stops our pad player and log display (RAM state only) and starts whichever was running again at the end, also on a failure, `q` or Ctrl-C. Each workflow runs `mpfree play --script … --kits /run/mpfree-kits/kits.bin --data /run/mpfree-devtest/data-NAME --gain-db -12`: its saves go to that RAM folder, partition 4 is never opened, and the kits are only read.
- **What happens:** step 3 copies (Enter). At step 4 (Enter) the seven workflows play one after the other on the default sound-core setting: the screen changes, the pads light and the MPC sounds by itself, and the terminal shows each part as it starts. **Keep your hands off the MPC** and listen. At the end it asks one question: answer in a few words whether anything clicked, dropped out, stuck or sounded wrong, and in which part. At step 5 (Enter) `heavy-load` plays again on each of `shared`, `core3`, `two` and `three`; `s` there skips them.
- **Passes when** it ends `RESULT: 11 runs played.` and shows `pad player: active (started again)`. A workflow that fails is listed on a `FAILED:` line and the next one still plays: go on with the round, and Q8e says what failed. A workflow's log line `script: … has no button number` names a button this MPC hasn't mapped yet; the run uses a free number for it and goes on.
- **If it is cut off halfway** (the cable, `q`, Ctrl-C): it ends a workflow still running from `/run/mpfree-devtest` and starts the pad player again; if the cable is gone, the pad player starts at the next power-on. Only RAM was used.
- **Undo:** none needed. The power-off in Q10 clears `/run`.

**Block Q8c: the checks by hand** (read-only on the MPC: the tool starts, copies and stops nothing and only reads the app's log; you press buttons and pads and touch the screen; the pad player keeps your project open in RAM, and anything MPFree saves goes to the stick's partition 4; about 30 minutes):

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-checks -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_CHECKS=basics,wheel,touch,console,sample-r1-reach,sample-audio-track,sample-keep-take,mpc-plugin,mpc-plugin-install bash tools/mpc-one/collect-over-ssh.sh --app
```

- **First, the stick's system,** as on 16 Nov: `started by: the boot hook's soft-reboot (asked at … s after power-on)`, `no MMC block device`, `kexec_load_disabled: 1` and `/: read-only`, ending `CHECKS-OK`. If any is wrong it prints a `PROBLEM` line and stops before it reads anything else. Then the start-up from the log, with no question, and `privileges:` lines with `Uid: 960` and `CapEff: 0000000000000000`.
- **Map controls.** If it lists buttons these checks use that Map controls hasn't found yet (likely on a stick the Windows creator made on 17 Nov, whose partition 4 is new), it runs `map-controls` first: find only the buttons it lists, then press MENU.
- **Each check** shows its steps. Press Enter to start (`s` skips the check, `q` stops here), do the steps, and press Enter when you are done. It then prints `what the log showed:`, with `seen`, `NOT SEEN` or `unclear` for each thing it looked for, and asks at most one question. Answer in a few words, with the step's number if one went wrong.
- **`basics`** comes first, so the real pads, buttons, wheel, Q-Links and touch are known to reach the app before anything else uses them.
- **`wheel` and `touch`** each say "with no clip yet, light the kick on Play's Beats view first": the pad player opens your last project at start (Prefs > At start), so there may be one already. Touch lab's 9 steps take about 3 minutes; Calibrate touch only reports and changes no setting.
- **`sample-r1-reach`:** "the pad from the last check" is any pad of kit 1, such as pad 1. Its step 1 still says "On the Wave page": since the one Sample page there is no separate Wave page, and **Normalize to 0 dB** is on the Sample page, under the Gain and fades knobs. Its Export writes a new folder, `Samples/Exports/<pad name>`, on partition 4 (delete it in BROWSE, or keep it).
- **`sample-audio-track` and `sample-keep-take`** come right after it, as the sampler thread asked; each takes back what it did (one Undo, UNDO).
- **`mpc-plugin`** changes the synth track's instrument, so it comes near the end.
- **`mpc-plugin-install`** is last. Its step 3 downloads from GitHub through the MPC's Ethernet, which needs a route out to the internet. On the direct cable that is there only if the workstation shares its connection on that port (NetworkManager's "Shared to other computers", which gives the 10.42.0.x addresses this round uses). Don't change your network for it: without a route, the catalog's line says why, and that is the answer. Its first look reads the Plugins place's first line, which ends with libcurl's OpenSSL: `OpenSSL … ready`, `ready (its random numbers didn't start: …; MPFree loaded its default provider)`, or `not ready: …` with why. Any of these answers 16 Nov's "Insufficient randomness", even without a route out. If Tidal Verb is installed from its zip already, step 2 says `Updated` instead of `Installed`; both pass.
- **At the end** it lists each check with how many of its lines were seen and your answer, then the session summary since power-on and the app's latest status. It ends `RESULT: all steps ran` and a `NEXT:` line about switching off: leave the MPC on and the stick in, and go on to Q8e (or Q8d first).
- **Undo:** none needed. Buttons that Map controls finds stay in `settings/map.conf` on partition 4, where they are meant to be. `mpc-plugin-install` leaves Tidal Verb updated and, with a route out, Modal Voice installed in `files/plugins/` (BROWSE > Plugins: Remove on each, or keep them); `sample-r1-reach` leaves its export folder.

**Block Q8d (optional): the checks with MIDI gear** (read-only on the MPC, as Q8c; only with the gear in its table at hand; about 15 minutes). Plug the gear in first. `s` at a check's prompt skips one you don't have the gear for:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run app-checks-midi -- env MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=app MPFREE_APP_CHECKS=midi-in,midi-learn,midi-thru,clip-program,cc-lanes bash tools/mpc-one/collect-over-ssh.sh --app
```

`midi-thru`, `clip-program` and `cc-lanes` turn Prefs' **Send MIDI and CV out** on and take it off again in their last step, and send to the synth on MIDI OUT only; nothing is saved. It ends `RESULT: all steps ran`, as Q8c.

**Block Q8e: the report** (workstation only, reads files only; a few seconds). One page from Q8's and Q8b's logs, saved in the results folder so Q11's zip carries it:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/devtest_report.py /home/jesse/mpfree-results/current/*-bench.log /home/jesse/mpfree-results/current/*-devtest.log --out /home/jesse/mpfree-results/current/devtest-report.md
```

It prints each workflow's checks met and the steps it couldn't do; per part, the underruns, periods, slow periods, late lanes and render means; the worst period; the screen's frame times; `heavy-load` on each sound-core setting and the one the numbers favour; the NEON verdict (keep or drop); each asked-about engine's fixed cost, cost per voice and the voices that fit; `GRAIN_BUDGET` and `VOICE_COST` to set; and the presets whose note starts go over the synth budget. It exits 0 when every workflow ran with every check met and no underrun, 1 otherwise (the page says why), and 2 if it found nothing to read. The glob also matches Q0b's `build-bench` log, which it ignores. Either way, go on to Q10.

**Q9 (optional): GPU page flips in the same boot.** These are the GPU thread's page-flip blocks, the ones the 16 Nov round couldn't run. G1a and G1 (optional above) build what they need. Nothing else in this round depends on Q9, so skip to Q10 if you'd rather not.

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

- Q0's local checks end `LOCAL-CHECKS-OK`, and Q0d ends `RESULT: every test passed.`
- Q6a shows partition 4 at 1024 MiB with `MPFREE-DATA ext4`. Q6 names the disk Q6a found, says `supported: MPC 3.10.1 (MPC One, Gen 1)` and `partition 4: this stick's saved settings, files and backups are kept`, and ends `done the stick is ready`. Q6b finds or puts in both plugins.
- In Q7 the MPC went from the logo to MPFree with no tap, and Akai's app never drew.
- Q8 shows `default.json` and `default-plain.json`, ends `RESULT: both builds ran (numbers above).` (it says so even when you typed `s` at step 5: that step then shows `answer: s`), and the pad player started again.
- Q8b ends `RESULT: 11 runs played.` with no `FAILED:` line, and the pad player started again. Your answer to its question says nothing clicked, dropped out, stuck or sounded wrong.
- Q8c ends its checks with `started by: the boot hook's soft-reboot` and `CHECKS-OK`, and each of its checks has every line `seen` and an answer that says it looked and sounded right. A `NOT SEEN` line, or an answer that says otherwise, is a finding: the report has the app's log for that check's time. For `mpc-plugin-install`, the catalog's line without a route out is an answer, not a failure. Q8d, if run, the same for each check you didn't skip.
- Q8e exits 0: every workflow ran with every check met and no underrun. Its numbers (the favoured sound-core setting, the NEON verdict, the costs per voice, `GRAIN_BUDGET` and `VOICE_COST`) are answers whatever they are.
- Q10's shut down switched the MPC off within a minute, and the stick logs have a `power.txt` with `asked: poweroff`.
- Q10b answers on port 22 with the host key of 16 Nov, the hook report shows main's hook files and `MPFree's stick in now: no`, and the boot map shows 0 sectors written to p1 to p6 and boot0/1 since that power-on.
- The stick's `dmesg.txt` shows `mpfree-internal-off: switched off ff0f0000...` before any `sd` or `mmc` mount, and nothing reads the eMMC or SD card before `mpfree-internal-off: done`. The stick logs show no failed unit: since #429 the stick masks Akai's `az0x-loadpin-trustpoints`, which failed harmlessly after the soft-reboot on 15 and 16 Nov, so on this stick any failed unit is a finding. The app's journal lines read `mpfree[…]:` (#429's `SyslogIdentifier=mpfree`), not `sh[…]:`.
- Only if run: F0 answers on port 22 with the host key of 16 Nov; Q2 ends `LOCKED: the login with no key is refused and your key gets in`; F1 ends `CHECK-OK`, F2 `INSTALL-OK`, F4 `SETUP-OK`, F6 lists three Sources and three Taps, `idle`, F7 ends `CAPTURE-OK` with three slots at each gain, and F8 writes `fxprobe-round1.tar.gz` into the results; G2 ends `RESULT: the GPU benchmark ran (numbers above).`, with `flip: the screen is back as it was` and the pad player started again.

## Undo, block by block

| Block | Changes | Undo |
|---|---|---|
| Q0, Q0b, Q0c, Q0d, G1, Q6, Q8e | New folders under `~/mpfree-work` (Q6's `creator-stick-<UTC>` record among them) and `~/mpfree-results` (Q8e's `devtest-report.md` in it), build output in the clone | Delete them |
| Q6a's stick check (only if run) | A new `~/mpfree-work/stick-check-<UTC>/` with the stick's backup in it, kept only when `~/mpfree-backups` has no copy of that backup; nothing on the stick | Delete the folder its `RESULT` line names |
| G1a | `cargo-zigbuild` in `~/.cargo/bin`, zig in `~/.local/share/mpfree-zig` | `cargo uninstall cargo-zigbuild && rm -r /home/jesse/.local/share/mpfree-zig` |
| Q6 | The stick's partitions 1, 2, 3 and 5 and its partition table | Q6 again from an earlier commit; partition 4 stays |
| Q6b | Nothing if both plugins were there; else the missing one on partition 4 | `stick-plugin.py remove`, as in Q6b |
| Q7, Q8, Q8b, Q8c, Q8d, Q9 | RAM on the MPC (Q8 and Q8b copy into `/run/mpfree-bench` and `/run/mpfree-devtest`, and the workflows save only in `/run/mpfree-devtest`); the stick's log partition and partition 4 | Switch off, take out the stick, wait 10 s, switch on |
| Q8c (`mpc-plugin-install`) | Tidal Verb updated from its zip, and with a route out Modal Voice from the catalog, as folders in `files/plugins/` on partition 4; the catalog in `files/plugins/.catalog.json` and the download in `files/plugins/.downloads/` | BROWSE > Plugins: Remove on each installed plugin; or keep them |
| Q8c (`sample-r1-reach`) | The export's new folder, `Samples/Exports/<pad name>`, on partition 4 | Delete it in BROWSE, or keep it |
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

Send the zip (with Q8e's page in it) to the MPFree project's coordinator, who routes each part:

| Part | Goes to |
|---|---|
| A workflow's failed check or missed step (Q8b, Q8e) | The area's owner thread |
| Underruns per part, `heavy-load` on each sound-core setting, the setting the numbers favour, the screen's frame times | The performance work (the sound-core default) |
| The NEON verdict (Q8's `default.json` against `default-plain.json`) | The NEON kernels thread |
| `GRAIN_BUDGET` and `VOICE_COST` | The granular and Sample page tools owners |
| Wind, Keys, the drawbar organ and FM per voice; the presets over the synth budget as a note starts | The synth library thread |
| Q8b's listening answer | The coordinator, with the workflow that was playing |
| `basics`, `wheel`, `touch`, `console`, `mpc-plugin`, `mpc-plugin-install` | The app thread |
| `sample-r1-reach`, `sample-audio-track`, `sample-keep-take` | The sampler thread |
| Q8d's checks | The MIDI thread and the sequencer thread (`clip-program`, `cc-lanes`) |
| Q9 (G2), if run | The GPU thread: G2's `whole p50` for the panel against software's |
| F1 to F8, if run | The FX quality thread |
| Q10b, the stick logs | The device rounds' notes |

## After this round: the USB creator round

The owner's plan (17 Nov) is to run the [USB creator round](usb-creator-round.md) right after this one, in the same sitting, from its C0, once Q11 has made this round's zip. It follows a new musician's install from developer mode off, and the MPC is then where its C1 expects it once you switch it on with the stick out: Akai's system, developer mode on, the boot hook installed. It needs:

- **A spare stick of 16 GB or more**, which its C4 erases. With only this round's stick, C4's **Erase everything and start over** also erases its partition 4: the plugins, the buttons Map controls found, FXProbe's work folder if the F blocks ran (F8 copied the captures off first) and its copy of the MPC's backup (another copy is in `~/mpfree-backups`).
- **About 90 minutes.** It ends with developer mode off, which deletes `/data/ssh`; its C10 switches developer mode on and locks it to your key for the round after.

It makes its own zip in C9.
