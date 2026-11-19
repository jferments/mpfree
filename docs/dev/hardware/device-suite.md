# The automated device suite: workflows and benchmarks on the MPC

**As of 2026-11-19** · **Asked for by** the owner, 19 Nov 06:46Z: test everything merged to `main` in the next device round with as little hand work as possible, through realistic workflows that use several tools together, and measure what it costs on the MPC. · **Code:** [`rust/mpfree-app/src/script.rs`](../../../rust/mpfree-app/src/script.rs) (the script player), [`smoke/scripts.rs`](../../../rust/mpfree-app/src/smoke/scripts.rs) (the same scripts in CI), [`tools/mpc-one/devtest/workflows/`](../../../tools/mpc-one/devtest/workflows/), [`host_devtest_mpc.py`](../../../tools/mpc-one/host_devtest_mpc.py), [`host_bench_mpc.py`](../../../tools/mpc-one/host_bench_mpc.py), [`devtest_report.py`](../../../tools/mpc-one/devtest_report.py).

A device round used to be a person working through [device checks](device-checks.md) one by one: a few steps each, read from the log. Most of those steps were a musician's moves that software can make just as well. Now mpfree plays them itself. A **workflow** is a musician's session written down as steps. `mpfree play --script FILE` plays it on the MPC, and CI's UI smoke session plays the same file on every pull request. A workflow that passes in CI does what it says. On the MPC the same steps show what they cost on its own cores, screen and sound card, and whether anything clicks.

What is still done by hand is what only the hardware or a person can show: that the real pads, buttons, wheel, Q-Links and touch panel reach the app, the lights, the MIDI and CV jacks with real gear, and one listening question. The [hardware thread's runbook](#what-stays-by-hand) has those.

## How the steps reach the app

On the MPC each step goes in where the hardware's would:

- **A button, a pad, a Q-Link or the wheel** becomes the controller's event, added to what the ports read in that period (`player_session` in `main.rs`). Everything after that is the app's own code.
- **A tap** goes to the screen thread, which finds the control by its widget, id and label on the page showing (`Player::script_taps_due` in `gui.rs`) and touches it as a finger would: a tap, a drag or a long press, as the control asks. If the control isn't there within 1.5 s, the run logs `script: line N: no control …` with what the page has.
- **MIDI from gear** goes in as bytes on the 5-pin MIDI IN.
- **A mark** names a part of the workflow. The app writes its status block right then, so each part's underruns, render times and late lanes can be read on their own.

The run logs each step as it plays (`script: line N at T s: …`) and stops by itself 3 seconds after the last step. In script mode STOP stops the transport and doesn't end the run, and the app takes buttons that have no number yet on this MPC (`map_unmapped`) from the free numbers, saying so in its log. With `--data`, the run gets its own data folder, so nothing touches partition 4.

In CI the same file plays in the smoke session (`smoke/scripts.rs`) on the same clock: a button takes 6 periods, a tap a quarter of a second, and so on. Each workflow is one test there, `workflow_<name>`.

## The script format

One step per line. `#` starts a comment. Each step takes its time, and then the next one starts.

```text
title A beat from scratch           the workflow's name in reports
covers rec-arm, step-pads           the device checks it stands in for
needs demo-songs                    the three demo songs in the run's library first
wait 1.5                            seconds
tap header tab Sequencer            widget, id, then the label exactly as the page has it
button PLAY                         a hardware button by the mapper's name
button SHIFT + NOTE REPEAT          the first held while the second is pressed
button-down ERASE / button-up ERASE one edge (for a chord with the pads)
hold 2 PLAY START                   held for 2 s
pad 1 100                           pad 1 (of 16) at velocity 100, 0.1 s
pad 1 100 0.5                       held 0.5 s
pad-down 1 100 / pad-up 1           one edge
qlink 1 +3                          Q-Link 1 (of 4) turned 3 steps
wheel -2                            the data wheel
wheel SHIFT + 1                     the wheel with SHIFT held
gear 90 3C 64                       MIDI bytes from a keyboard on MIDI IN
mark heavy                          a named part: the run logs its totals here
expect project: Demo is in          a line holding this text since the last mark
expect-not refused                  no line holding this since the last mark
expect-on-mpc track 1: plays FM     as expect, checked on the MPC only (see below)
```

A tap, an expect, a mark or the title may quote the app's own characters (`Speed 2×`, `‹ Back`, `TRACK MUTE · bank A`); everything else is ASCII. A script is at most 30 minutes long, and CI keeps each workflow at 10 minutes or less.

**What an expect sees.** In CI an expect sees the control side's lines and what the screen said (`Player::say` and the Sample page's lines) since the last mark. On the MPC, [`devtest_report.py`](../../../tools/mpc-one/devtest_report.py) checks each expect against the run's log from its mark to the next mark, because the app's threads write their lines a little after the step that caused them. The text is matched anywhere in a line, so `expect : End ` matches `sample: kit 1 (…) pad A5 …: End 4.891 s -> 4.868 s` whatever sound the kit has.

**What CI's session doesn't do,** so a workflow checks these with `expect-on-mpc` (or not at all):

- It builds no track instruments, so `track 1: plays FM` only shows on the MPC.
- It keeps no master recording, so Sample Recall's Keep last 4 bars says `nothing kept` in CI.
- It doesn't play notes that arrive as MIDI from gear (`Session::midi_from_gear` takes only remote control).
- Lines the screen prints directly (`steps held: …`, `q-link bank: …`) aren't captured.
- A Sample page edit's line is written when the next edit starts, on the MPC as in CI. Put its expect after that next edit.

## The workflows

About 7.5 minutes in all, plus heavy-load once more on each of the four sound-core settings (about 6 minutes). `covers` names the [device checks](device-checks.md) each one stands in for.

| Workflow | What a musician does | Stands in for | About |
|---|---|---|---|
| `beat-from-scratch` | Steps on the pads with STEP SEQ, a step erased, a held step turned; a recorded take; mutes that wait for the bar in Perform | `rec-arm`, `step-pads`, `step-hold`, `step-speed`, `track-mute`, `mute-queue`, `live-erase` | 26 s |
| `loops` | A loop recorded from Play and overdubbed; Speed, Reverse, mute and a second loop on the Looper tab; Capture; loops launched, stopped and played as a scene from Arrange | `looper`, `rec-arm`, `loop-tracks`, `looper-deck`, `loop-cell` | 38 s |
| `every-synth` | Each of the twelve engines on Track 1 in turn: eight notes held, then a run; the arp latched on Poly | `load`, `note-repeat`, and each engine's cost at eight voices | 2 min |
| `effects` | A beat playing while a compressor, a drive, a delay and a reverb go on the drums, turned, run for 10 s and taken off | `fx-add`, `fx-busy`, `comp-moves` | 27 s |
| `sample-page` | A loop's end moved while it sounds, ping-pong, Slice, Warp (stretch and repitch), Grain with its LFO and presets, Keep last 4 bars, UNDO | `sample-live-loop`, `sample-warp`, `grain-movement`, `grain-cloud-view`, `grain-presets` | 31 s |
| `songs` | The three demo songs opened from BROWSE, played through two scenes, a track muted on the bar, the tempo moved, saved, and the first opened again | `mute-queue`, `reload-on-the-bar`, `sample-r3-kit-per-track` | 2 min |
| `heavy-load` | A full song, a loop recorded over it, the pads rolling, and a reverb, delay and chorus on the drums: half a minute of everything at once | `load`, `fx-busy` | 1.5 min |

## The round's order

The hardware thread builds the stick and writes the round's runbook. These are the blocks for the software part. Each runs through `results.py`, so the zip keeps its log.

1. **Builds** (workstation): the `build-arm` and `build-bench` blocks as in the [second 3.10.1 round](firmware-3.10.1-round-2.md), from the commit the stick is made from. `build-arm` makes `rust/target/armv7-unknown-linux-gnueabihf/release/mpfree`, the program `host_devtest_mpc.py` copies.
2. **The stick boots** on the hardware thread's blocks, and its checks pass.
3. **The benchmark** (**RAM only**, about 50 minutes): `host_bench_mpc.py` as in Q8b. Since this change the default build also runs `--starts` (what a note costs as it starts) and `--plain --json default-plain.json` (every engine with the NEON kernels off, for the keep-or-drop A/B).

   ```bash
   cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run bench -- env MPFREE_ACCEPT_BENCH=YES python3 tools/mpc-one/host_bench_mpc.py
   ```

4. **The workflows** (**RAM only**, about 15 minutes, three presses of Enter):

   ```bash
   cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run devtest -- env MPFREE_ACCEPT_DEVTEST=YES python3 tools/mpc-one/host_devtest_mpc.py
   ```

   - **What it changes:** it talks only to the test build (port 2222, the stick's own host key) and runs the same read-only checks as the app step first, plus that the stick's kits are at `/run/mpfree-kits/kits.bin`. It copies `mpfree` and the workflows into `/run/mpfree-devtest` (RAM) and checks their hashes there. It stops our pad player and log display (`mpfree-test-app`, `mpfree-test-screen`: RAM state only; Akai's services are not running and are never touched) and starts whichever was running again at the end, also when a run fails or you press `q` or Ctrl-C. Each workflow runs `mpfree play --script … --kits /run/mpfree-kits/kits.bin --data /run/mpfree-devtest/data-NAME --gain-db -12`: its saves go to that RAM folder, so partition 4 is never opened, and the kits are only read.
   - **Hands off the MPC** while it plays: the workflows press the buttons and pads themselves. Keep the volume where you can hear it. The terminal shows each part (`mark` lines) as it starts. At the end of the default pass it asks one question: did anything click, drop out, stick or sound wrong.
   - **Then heavy-load on each sound-core setting** (`--sound-cores shared`, `core3`, `two`, `three`), the same steps each time. `s` at that prompt skips it.
   - **Passes when** it ends `RESULT: 11 runs played.` and prints `pad player: active (started again)`. A workflow that fails is listed on a `FAILED:` line, and the next one still plays.
   - **If it is cut off halfway** (the cable, `q`, Ctrl-C), it ends a workflow still running from `/run/mpfree-devtest` and starts the pad player again; if the cable is gone, the pad player starts at the next power-on. Nothing is left but RAM.
   - **Undo:** none needed. Switching the MPC off clears `/run`.

5. **The hands checks that remain** (the app step with `MPFREE_APP_CHECKS`): only those the workflows can't stand in for. See [what stays by hand](#what-stays-by-hand).
6. **The report** (workstation, reads files only):

   ```bash
   cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/devtest_report.py ~/mpfree-results/current/*-bench.log ~/mpfree-results/current/*-devtest.log --out ~/mpfree-results/current/devtest-report.md
   ```

   It prints one page: each workflow's checks, the steps it couldn't do, and each part's underruns, periods, slow periods, late lanes and render means; heavy-load on each sound-core setting, with the setting the numbers favour; the NEON verdict; each asked-about engine's cost per voice and the voices that fit; `GRAIN_BUDGET` and `VOICE_COST` to set; and the presets whose note starts go over the synth budget. It exits 0 when every workflow ran with every check met and no underrun.

## What stays by hand

The workflows can't show that the hardware reaches the app, so the round keeps these [device checks](device-checks.md), run by the app step: `basics` (pads, PAD BANK, FULL LEVEL, the wheel, the Q-Links and touch), `wheel`, `touch` (Touch lab, Calibrate touch and a real drag), `power`, `map-controls` (only if a button had no number: the run's log says `script: … has no button number`), and the checks with real gear (`midi-in`, `midi-learn`, `midi-thru`, `clip-program`, `cc-lanes`, the CV meter, and the USB-B survey). The Sample page's three checks from step 6d (`sample-audio-track`, `sample-keep-take`, `sample-edit-pad`) also stay by hand this round: each asks what only a person sees (the regions on the audio lane, the take being what was just played, Edit pad opening and closing). Pad and button lights, and a check's question about what only a person sees or hears, stay with the check that has them.

## Adding a workflow

1. Write `tools/mpc-one/devtest/workflows/NAME.mpw`: a `title`, a `covers` line, and `mark`s around the parts worth timing.
2. Play it in CI's session while you write it. A tap that finds nothing lists the page's controls:

   ```bash
   MPFREE_SCRIPT=$PWD/tools/mpc-one/devtest/workflows/NAME.mpw MPFREE_SCRIPT_LINES=1 cargo test --manifest-path rust/Cargo.toml -p mpfree-app --bin mpfree the_script_mpfree_script_names -- --nocapture
   ```

   `MPFREE_SCRIPT_LINES=1` prints every line the session wrote, to write the `expect`s from.
3. Add it to the `workflows!` list in `smoke/scripts.rs` (`every_workflow_file_has_its_test` fails until you do).
4. The device round plays every file in the folder. `MPFREE_DEVTEST_ONLY=NAME` plays just one.

## What comes next

The owner wants this to become a general scripting mode (19 Nov): commands sent to the MPC over USB or Ethernet, musicians' own workflow automations, and the desktop companion. That is on the [roadmap](../project/roadmap.md#future-scripting-and-automation), for after the device round: the same step lines over a live channel, then a small sandboxed scripting language in its own program that sends them.
