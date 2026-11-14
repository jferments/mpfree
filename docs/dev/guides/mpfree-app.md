# mpfree on the kexec test build

The app is called mpfree. Phase 1 called it Hello MPC, and its crate was `mpfree-hello` until 2026-11-11; it is now [`rust/mpfree-app`](../../../rust/mpfree-app), and the program is `mpfree`.

**Status:** round 3 ran on 2026-11-10, and every Phase 1 part now works on the test build: the screen, touch, pads, Q-Links, wheel, buttons, pad lights, and sound from the pads. A factory drum kit played from the pads with 0 underruns, and Akai's setup, replayed, moved the controls to the Private port with relative Q-Links ([results](#round-3-results-2026-11-10)). Round 4 ran [the pad player](#round-4-the-pad-player-and-mpfrees-first-gui), MPFree's first GUI: 323 hits in 120 s with 0 underruns and 8.7 ms from pad to sound ([results](#round-4-results-2026-11-10)). Round 5, [straight into the pad player at power-on](#round-5-straight-into-the-pad-player), is ready to run. Rounds 8 to 10 are built and tested on the workstation but have not run on the MPC yet ([what they add](#rounds-8-to-10-the-app-as-it-stands)).

Phase 1 is the first time MPFree's own code drives the MPC One's hardware: the screen, the touchscreen, the pads and the audio output. It runs on the [kexec test build](../hardware/archive/2026-11/kexec-test-boot.md): Akai's 3.10.0 system started from the USB stick, with Akai's app blocked and the eMMC and SD card hidden from it. A power-cycle brings back stock 3.6.0.

## What runs, and what it changes

The program is `mpfree` (crate [`mpfree-app`](../../../rust/mpfree-app/src/main.rs)), a static ARM binary. `collect-over-ssh.sh --app` runs [`host_app_mpc.py`](../../../tools/mpc-one/host_app_mpc.py), which:

1. checks the program on the workstation (a static ARM EABI5 hard-float executable);
2. connects **only to the test build**: it uses the test build's own SSH host key, so stock 3.6.0 is refused before anything runs. On the test build it checks, read-only, the release, that no MMC device exists, that kexec is off, that `/` is read-only, that `/run` allows programs and has room, and that Akai's app is not running. It also lists which programs hold the sound, MIDI, touch and screen devices;
3. copies the program to `/run/mpfree-app` (RAM) and checks its hash there. If the drums or play step is chosen, it copies the drum kits there too and checks their hash. When the only steps are `app` and `line`, nothing is copied;
4. runs one step at a time. Each step starts only when you press Enter. `s` skips a step and `q` stops. `MPFREE_APP_STEPS` picks the steps by name, comma-separated (default `inventory,screen,touch,pads,click`); they always run in the order of the table. `MPFREE_APP_PHASES` picks the app step's phases by number, with ranges (`9-11,16-19`): the others are skipped without a prompt, and the start-up check and the summary still read the whole session. The app step may run up to three hours;
5. after the lights, drums, setup and play steps, checks read-only that the controller still has its normal USB id, `09e8:0046`, and stops if it does not.

The first step that draws on the screen stops our own log display (`mpfree-test-screen`) or pad player (`mpfree-test-app`, on a stick built with `--app`), whichever runs, since both would draw on `/dev/fb0`; it is started again at the end.

| Step | What the program does | Writes to |
|---|---|---|
| app | nothing new runs: it reads the log of the pad player that started at power-on, phase by phase ([round 5](#round-5-straight-into-the-pad-player)). Without other steps nothing is copied | nothing |
| line | **state-changing, never in the default steps.** Nothing new runs either: after a read-only look at the ACVA card's capture device in `/proc/asound`, and only once you type `LINE`, you turn Line input on in the pad player, which opens the capture device; phases for its meter, a loop and turning it off, each followed by the same read-only look ([round 6](#the-line-input-step-state-changing)) | the pad player opens the ACVA capture device (RAM state only) |
| inventory | lists sound cards, PCM and MIDI devices, input devices, the framebuffer, panel rotation, real-time limits | nothing but its report |
| screen | stops our own log display (`mpfree-test-screen`, started again at the end), draws the mpfree card | `/dev/fb0` and its sysfs `pan` file |
| touch | shows your touches for 10 s; reads the ILI2117 device without grabbing it | the screen |
| pads | listens to all three of the controller's MIDI inputs (Public, Private, MIDI Port) for 15 s and shows every message in hex, with its meaning (pad, button, Q-Link, wheel) where known; then lists the kernel's MIDI byte counters | the screen |
| pads, second try | only if no pad hit arrived: listens again while sending Akai's keep-alive message every 5 s on the Public port (the only message this step sends) | the controller's Public MIDI port |
| lights | sends Akai's startup pad colours (orange on pad 1 to teal on pad 16) and the 58 button lights the app sets, holds them 4 s, then sends the owner's project's pad colours; for 15 s, each pad lights brighter while held. No session is asked for | the controller's Public port (pad colours) and Private port (button lights) |
| drums | plays the first factory drum kit (made on the workstation, see [below](#the-factory-drum-kits)) from the pads for 30 s through ACVA, at the click's stream settings; pads with a sound are lit, the others dark | the controller's Public port (pad colours), the ACVA playback device |
| setup | after you type `SEND`: asks for a session as Akai's app does (identity request and session ping every 4 s, up to 20 s). Only once the controller accepts our token, it sends Akai's setup messages byte for byte and in the app's order, the lights again, then listens for 20 s with a ping every 4 s. If the session is not accepted, it sends nothing more | the controller's Public and Private ports |
| play | the pad player, after you type `SEND`: the same session and setup as the setup step (with the first kit's pad colours), then up to 120 s with every kit. The pads or the screen's pads play; the wheel or a tap on the kit list picks the kit (its pad colours are sent); Q-Link 1 or the slider sets the level, from −40 dB to −12 dB, starting at −18 dB; STOP ends it. It pings the session every 4 s and times each hit. If the session is not accepted within 20 s, it plays without Akai's setup, sending only pad colours | the screen, the controller's Public and Private ports, the ACVA playback device |
| click | 120 BPM click for 10 s on the ACVA output, about 18 dB below full scale, at the stock app's stream settings (stereo S32_LE, 44.1 kHz, 128-frame periods, 3 periods); counts underruns and late periods | the ACVA playback device |

The program also appends every line to `/run/mpfree-app/report.txt`, which the tool copies into its report at the end. While the sound plays the lines are written by a thread of their own (`mpfree-report`), so the audio thread never waits on the file; if that thread falls behind by more than 2048 lines, the extra lines are dropped and the report says how many. A panic adds its line, `PANIC: thread 'name' at file:line:column: message`, written and synced before the program ends ([`crash.rs`](../../../rust/mpfree-app/src/crash.rs)). Beside the report, `/run/mpfree-app/running` holds the app's pid while it runs (`app` and `play`); it says `ended` when the run ends well and is removed by a stop (SIGTERM, SIGINT, SIGHUP). Nothing is written to the stick, and the test build cannot see the eMMC. Everything is gone after a power-cycle.

Every step draws on the screen with MPFree's GUI toolkit ([`mpfree-ui`](../../../rust/mpfree-ui), see [UI architecture](../design/ui-architecture.md)): a header and the step's log for most steps, and the pad player's Play and Debug pages for `play`. Every line still goes to the workstation's terminal as well.

**Turn the MPC's volume knob most of the way down before the drums, play and click steps.** The levels are capped in the program (−12 dB: the click peaks about −18 dBFS, each drum sound at most −12 dBFS at full velocity, though hits that overlap add up), but the analogue volume knob sets what you hear.

The lights, setup and play steps send the controller only messages Akai's app sent it in the [round 2 capture](../hardware/controller-protocol.md#akais-app-at-startup-hello-mpc-round-2-2026-11-10), plus pad colours made by the app's own rule for a held pad. What they change lives in the controller's RAM: the lights and, inferred, its port mode. Switching the MPC off clears it, and Akai's app sends its own setup at every start.

## The program

[`rust/mpfree-app`](../../../rust/mpfree-app) talks to ALSA directly with ioctls, as [ADR-0002](../decisions/adr-0002-direct-alsa-audio.md) chose, without alsa-lib. The parameter blocks are built as bytes in the 32-bit ARM layout. [`abi-check.c`](../../../rust/mpfree-app/abi-check.c) checks every size, offset, ioctl number and constant it uses against the kernel's own `<sound/asound.h>` and `<linux/input.h>`, compiled for ARM. CI compiles it on every change; a mismatch fails the build.

A few things it relies on, checked against the kernel source (6.18):

- **MIDI substreams.** The controller's three cables are substreams 0 (Public), 1 (Private) and 2 (MIDI Port) of `midiC<card>D0`. ALSA picks one through the "preferred subdevice" set on the card's control device by the same thread, as alsa-lib does. The program then asks the kernel which substream it got (`RAWMIDI_INFO`, which must name the direction) and stops unless the subdevice and name match.
- **Blocking writes.** The PCM is opened non-blocking, so a busy device fails at once instead of waiting. Then it is switched to blocking; `PREPARE` copies that flag into the stream.
- **Underruns** show as `EPIPE` from a write. The program counts them, prepares the stream again and refills it with silence.
- **Pad-to-sound timing** (play step): after each write that carries a new hit, the program asks the kernel how many frames are still queued (`SNDRV_PCM_IOCTL_DELAY`). A hit's time is from reading its MIDI message to the end of the period it starts in leaving the card's buffer: the time until the write returned, plus the queued frames beyond that period. Not counted: up to one period (2.9 ms) before the message is read, the controller and USB, and the codec.

It locks its memory (`mlockall`) and runs the click at `SCHED_FIFO` priority 70; both are reported, and it carries on without them if refused. In the play step, the screen has its own thread at normal priority, started before the audio thread raises its own; the audio thread only hands it messages and never waits for it.

## Running it

Build the program on the workstation (the same flags as the screen log program):

```bash
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" \
cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-app
```

Then, with the test build running (`--kexec-boot`, see [the kexec test boot](../hardware/archive/2026-11/kexec-test-boot.md)):

```bash
MPFREE_ACCEPT_APP=YES bash tools/mpc-one/collect-over-ssh.sh --app
```

For the drums and play steps, first make the kits on the workstation (read-only on the backup, [below](#the-factory-drum-kits)), and choose the steps:

```bash
python3 tools/mpc-one/offline-factory-kit.py
MPFREE_ACCEPT_APP=YES MPFREE_APP_STEPS=screen,play bash tools/mpc-one/collect-over-ssh.sh --app
```

The host tool takes the newest `~/mpfree-work/factory-kit-*/kits.bin`, or the file named by `MPFREE_APP_KIT`. Without one it leaves the drums and play steps out. A round 3 `kit.bin` is not used.

Wrap every command in `python3 tools/mpc-one/results.py run NAME -- ...` so the output lands in the round's results folder. Afterwards: switch the MPC off, take out the stick, wait 10 s, switch it on, and run the boot-map survey to confirm nothing on the eMMC changed.

`mpfree --preview DIR` draws the screens with sample data into new PNG files in `DIR` (`console`, `loading`, `loading-failed`, `play`, `debug`, `syslog`, `prefs`, `options` and `guide`, each `.png`) on the workstation, with no devices; CI runs it for ARM under qemu. `--font`, `--bold-font` and `--mono-font FILE` replace the built-in fonts with any TTF or OTF file, on the workstation or the MPC.

## Round 1 results (2026-11-10)

From the owner's results zip (clone at `da8a614`):

- **Storage:** after the test build, the hashes of `p1`–`p6` and both boot areas match the baseline, and those partitions show zero writes since boot.
- **Screen:** the card was drawn the right way up. **Touch:** one and two contacts showed where they were touched.
- **Click:** 3445 periods at the stock stream settings with 0 underruns and 0 late periods. The gap between periods averaged 2.903 ms (one period is 2.902 ms), and the worst was 2.939 ms. It ran with `mlockall` and `SCHED_FIFO` 70.
- **Pads:** nothing arrived on the Private input in 15 s. With the keep-alive, the controller answered (three replies, the first different from the usual one), but still sent no control data. Round 2 showed why: the controls were on the Public port. Details in [the controller protocol](../hardware/controller-protocol.md#without-akais-app-hello-mpc-rounds-1-and-2-2026-11-10).
- Ethernet came up at 20 s on that boot without the retry.

## Round 2 results (2026-11-10)

From the owner's results zip (clone at `13ee8d0`):

- **Storage:** after both the app restart on stock 3.6 and the test build, the hashes of `p1`–`p6` and both boot areas match the baseline, with zero writes to those partitions.
- **The controller was never silent.** Listening on all three inputs, 216 messages arrived on Public in 15 s: 14 pad hits with pressure, Q-Link 1's touch and turns, the wheel and many buttons. Only buttons 73, 81 (STOP) and 82 (PLAY) arrived on Private. Round 1 had listened on Private only. Without Akai's app, Q-Link turns are **absolute** (0–127) rather than relative steps. Nothing lit up, since nothing was sent. The mapping is in [the controller protocol](../hardware/controller-protocol.md#without-akais-app-hello-mpc-rounds-1-and-2-2026-11-10).
- **Akai's app at startup:** the capture recorded every message from the restart on: a session handshake, 18 setup messages, then pad colours and button lights. The app came back as a new process and the controller was the same USB device afterwards. The sequence, with the serial number left out, is in [the controller protocol](../hardware/controller-protocol.md#akais-app-at-startup-hello-mpc-round-2-2026-11-10).
- **Galaxy splash on the test build:** 85 frames in 6.0 s (about 14 frames a second); the stick had no font, so it used its built-in letters.
- The test build answered SSH after 34 s; Ethernet needed its retry this time.

### How round 2 captured Akai's startup

Two measurements, in one round:

1. **On the test build**, the pads step now opens all three MIDI inputs and prints every byte, so a reply or message the program doesn't know is no longer dropped. The keep-alive is still the only thing it sends.
2. **On stock 3.6**, [`host_app_start_capture.py`](../../../tools/mpc-one/host_app_start_capture.py) (`collect-over-ssh.sh --app-start-capture`) records what Akai's app sends to the controller as it starts:
   - Read-only checks first: stock 3.6's kernel release, Akai's app running, card 0 is the controller in its normal MIDI mode (`09e8:0046`), no USB drive attached, a usbmon reader for its bus. It notes the controller's USB address and firmware version.
   - It starts a passive usbmon capture of that bus. It prefers the binary `/dev/usbmonN`, which keeps whole transfers, over the debugfs text file, which keeps 32 bytes of each. The reader stops by itself within 240 s.
   - After you type `RESTART`, it runs `systemctl restart acvs.service`. **This is the state change:** Akai's app quits and starts again, as at every power-on, and may save its usual settings to `/data`. systemd does the restart, so a dropped SSH session cannot leave it half done.
   - You press Enter when the app's main screen is back, then hit pads and turn a Q-Link and the wheel for 8 s.
   - Afterwards, read-only, it checks that the app runs again as a new process and that the controller is the same USB device (same address, ids and firmware version).

   The report lists every message between the app and the controller from the restart on, decoded and in order, and every captured transfer in hex.

Why the restart cannot flash the controller (from [the updater notes](../hardware/archive/2026-11/akai-updater-offline.md); the second point is an inference): the app's launcher flashes the controller only if it is already in its firmware-update mode, which the checks rule out (`09e8:0046` is its normal MIDI mode). And this same app starts at every power-on of this unit, with the same controller firmware, without re-flashing it: with the app running, the controller was still USB device 2 on its bus in the 2026-11-09 capture, so it had connected only once since power-on, and a flash would have reconnected it in another mode. If Akai's app does not come back, switch the MPC off and on.

Messages are sent to the controller only after they have been seen in this capture and listed for the owner's OK (round 3).

## Round 3: lights, factory drums and Akai's setup

The owner chose to try the lights alone first, then Akai's setup, each started by Enter (the setup by typing `SEND`):

1. **lights**, without a session: shows whether the controller takes pad colours and button lights while it is in its no-app mode.
2. **drums**: a factory drum kit played from the pads, for timing and sound. It runs before the setup, in the no-app mode where round 2 saw the pads working.
3. **setup**: the session and Akai's setup, replayed. It shows whether this moves the controls to the Private port with relative Q-Links, as under Akai's app (inferred). The step's report counts pad hits on each port. It runs last of the three, so whatever it changes cannot affect the others.

The program listens on all three inputs in every step, so it works whichever port the controls use.

### Round 3 results (2026-11-10)

From the owner's results zip (clone at `2db957c`):

- **Storage:** before the jump, the test build's preflight on 3.6 counted 0 sectors written to `p1`–`p6` since boot. After the power-cycle, the hashes of `p1`–`p6` and both boot areas are the same as in round 2 (which matched the baseline), again with 0 sectors written to `p1`–`p6`.
- **Pad lights work without a session.** The pads lit up, and each got brighter while held (the owner's words: "the pads were lit and got brighter when pressed"). The button lights were not reported either way.
- **Factory drums work.** The kit tool picked a 16-pad drum program from a factory demo project, all 16 samples found by pad number (44.1 and 96 kHz, 16- and 24-bit, mono and stereo). In 30 s the owner played 42 hits, up to 10 sounds at once, peak −7.0 dBFS with no clipping. ACVA ran 10335 periods with 0 underruns and 0 late periods; the gap between periods averaged 2.903 ms (one period is 2.902 ms) and was at most 2.947 ms. Two runs of the kit tool made byte-identical kits. Pad-to-sound latency was not measured; from the settings it should be about 12 ms or less after the MIDI message arrives (up to one 2.9 ms period of polling plus the 8.7 ms output buffer), an estimate only.
- **Akai's setup moves the controls.** With nothing holding the controller, it accepted our token at once (status `02`), echoed the setup as it did for Akai's app, and pinged back `01` every 4 s. From then on the pads, Q-Link 1 (now relative: +1, +2, −1 … −3 per message), the wheel and the buttons all arrived on **Private**; Public got only the two identity replies. The setup step plays no sound, so the pads were silent in it, as expected.
- The test build answered SSH after 24 s; Ethernet needed its retry again.

## Round 4: the pad player and MPFree's first GUI

The owner chose the pad player (2026-11-10): one program that holds Akai's session, lights the pads, plays factory kits and lets you pick them with the wheel. The owner also asked for a GUI for everything from now on, modular enough that the layout can change completely as features arrive, with a debug mode for MIDI and system events, while the terminal keeps the full log. The new [`mpfree-ui`](../../../rust/mpfree-ui) crate and the [UI architecture](../design/ui-architecture.md) are the start of that.

What round 4 runs (`MPFREE_APP_STEPS=screen,play`):

1. **screen**: the mpfree card, now drawn by the GUI toolkit. It shows whether the new drawing code lands the right way up on the panel.
2. **play** (typed `SEND`): the session and Akai's setup, as in round 3, then up to 120 s of the pad player:
   - **Play page:** the 16 pads laid out as on the MPC, lit in the kit's colour with each sound's name, brighter while held; the kit list; the level slider; timing (pad-to-sound delay, underruns, voices, peak level).
   - **Debug page:** every MIDI message in and out with its meaning, the program's events, and the same timing.
   - The tabs at the top switch pages. The pads, the wheel and Q-Link 1 work on either page.
   - Since the time engine (round 6), a Clips page sits between Play and Debug.
   - The kits file holds up to 8 factory kits ([below](#the-factory-drum-kits)).

It sends the controller nothing new: the session, Akai's setup and pad colours, all as in round 3. Only the colours differ, since each kit's pads rest in one of the seven colours seen from Akai's app.

The report gives the pad-to-sound time (minimum, mean, maximum), underruns, and the hits from the pads and the screen.

### Round 4 results (2026-11-10)

From the owner's results zip (clone at `a0b136f`):

- **Storage:** the hashes of `p1`–`p6`, the U-Boot environment and both boot areas match the baseline after the power-cycle, with 0 sectors written.
- **Sound:** 323 hits in 120 s from the pads, with kits 1, 3, 4, 5 and 6, up to 16 sounds at once, peak −9.0 dBFS, no clipping. ACVA ran 41343 periods with 0 underruns; the gap between periods averaged 2.903 ms and was at most 2.973 ms.
- **Pad to sound:** mean 8.68 ms (8.63 to 8.70 ms), from reading the MIDI message to the end of its period leaving the card's buffer. Up to one 2.9 ms period before the message is read is not counted.
- **Screen:** 1958 frames, 3.7 ms each on average, the slowest 38.5 ms. The owner saw the log display at the screen's edges, behind the pad player. The cause, from the kernel source: a write of exactly one whole native row gives the framebuffer emulation a zero-width damage rectangle, so those rows were never flushed. `FbTarget` now writes at least two whole rows at a time, and the app's pages fill the whole screen.
- **Touch:** one touch registered in 120 s (not yet explained; round 5 asks).
- **Velocity:** the pads used the whole range, 1 to 127, but 91 of the 323 hits (28 %) arrived as exactly 127, so firm hits saturate in the controller before any curve of ours sees them (inferred from the numbers; Akai's own pad sensitivity settings are not decoded).
- **Buttons:** one unknown button, number 11, was pressed five times. Round 5's button guide learns which buttons MENU, NOTE REPEAT, TAP TEMPO, − and + are.

## Round 5: straight into the pad player

The owner asked (2026-11-10) to go from the loading screen to the app without running a command and without the console on screen, with a progress bar and plain messages while it starts, a button to switch to the raw console, pad velocity, Full Level and Half Level, Latch with Note Repeat, and MENU and SHIFT + MENU for options and Prefs.

**At power-on.** A stick built with `offline-kexec-stick.py --app` ([the kexec test boot](../hardware/archive/2026-11/kexec-test-boot.md#tools-in-order)) starts the pad player instead of the log display: the startup screen for 6 s (Synthwave's, from `mpfree-startup`; the galaxy on sticks built before 12 Nov), then a loading page over it with a progress bar and what it is doing (loading the drum kits, finding the pads and buttons, finding the sound card, connecting, lighting the pads, starting the sound), then the pad player. If something fails, the loading page says what, and a tap shows the console. The drum kits are on the stick's partition 3, mounted read-only. If the program crashes or exits with an error, systemd starts it again after 2 s (`Restart=on-failure`, on sticks built after 14 Nov); after the fourth failure within a minute the log display starts instead. A stop (`systemctl stop`, as `host_app_mpc.py` does) is not a failure, so the app stays stopped.

**Controls** (logic in [`app.rs`](../../../rust/mpfree-app/src/app.rs), with tests):

| Control | What it does |
|---|---|
| Pads | Play, through the velocity curve (Prefs: soft, linear or hard) |
| FULL LEVEL | Every hit at 127; lit while on. SHIFT + FULL LEVEL: Half Level, every hit at 64 |
| NOTE REPEAT (held) | Held pads play through the arp: Repeat on the grid until the Arp panel picks Arp or Pattern. A slim strip at the top of the right-hand column shows the mode, the rate with − and +, and Latch (nothing else moves); the wheel, − / + and Q-Links 1-4 set the arp (see the [arpeggiator](../design/arpeggiator.md) for every control) |
| NOTE REPEAT twice, or SHIFT + NOTE REPEAT | Latch: pads keep playing after you let go; new pads replace them. NOTE REPEAT once more turns it off. The two presses must come within 0.4 s. Uses the [`mpfree-arp`](../design/arpeggiator.md) engine |
| NOTE REPEAT + MENU, or the arp chip | The Arp panel beside the pads (mode, rate, order, pattern, Q-Links), and from it the pattern workspace |
| MENU | The current page's options (on the Arp panel: accent level, scale, scale root, strum, new notes restart, swing from); again to close. SHIFT + MENU: Prefs |
| In a menu | The wheel or a tap picks a row; − / +, SHIFT + wheel or the arrows change it. The wheel's push (`Job::DataPush`, found with Map controls) opens the row: the wheel then changes it, and a second push sets it; on a row that opens something it opens it |
| TAP TEMPO | Sets the tempo; its light blinks with it |
| Wheel, − / + | The kit (outside menus) |
| Q-Link 1, the slider | The level |
| STOP | Silences everything and turns Latch off. While the transport plays, STOP stops it first and a second STOP silences everything. SHIFT + STOP (round 9 on): stop, silence and back to bar 1, as on the MPC |
| SHIFT + ERASE + TAP TEMPO | The console (the system log, raw) and back: hold SHIFT, hold ERASE, press TAP TEMPO (round 9 on; SHIFT + STOP before). ERASE works once Map controls has found it; the Dev tab's Console page works without it |

MENU, NOTE REPEAT, TAP TEMPO, − and + were measured in round 5 and are built in. At power-on a **button guide** asks only for buttons not known yet: REC, OVERDUB, UNDO and PLAY START, for the time engine. STOP skips one. The numbers go in the log for next time. Prefs > Find the buttons again asks for all nine.

**The time engine** (for round 6, not yet tried on the MPC; see [playback](../design/playback.md#in-the-pad-player-timehost)). A transport bar sits under the header on the Play page and the new Clips page:

| Control | What it does |
|---|---|
| PLAY, the bar's ▶ | Plays from where the transport is |
| PLAY START | Plays from bar 1; while playing, jumps back to bar 1 |
| REC, the bar's ● | Records into the armed track, from now or after the count-in; again to stop recording and play on |
| OVERDUB, the bar's mode | Overdub (add notes) or Replace (notes played over erase the old ones) |
| UNDO, the bar's Undo | Takes back the last take. SHIFT + UNDO: redo |
| Clips page: a clip | Plays it from the next bar, or stops it |
| Clips page: an empty slot (+) | Makes a 2-bar clip there (or one as long as the loop), arms its track and records |
| Clips page: a track's name | Arms that track, or disarms it |
| Clips page: the wheel | Steps the record setting last touched (the armed track to begin with); SHIFT + wheel scrolls the slots |

Clips play the current kit's pads. A click plays while counting in and recording. The status line also gives the transport, the takes and any notes or parameter locks lost.

Every 30 s the app writes a status line to the system log: hits, underruns, pad-to-sound timing, and the note engine's dropped events and overflow. A `status: mod: ...` line gives the mod engine's targets (mixer controls a clip has locked), the values it sent, any it had no place for and any events dropped.

**The check** (`MPFREE_APP_STEPS=app`, **read-only**): it copies, starts and stops nothing. It shows what the app wrote since power-on, then seven phases, each started by Enter and ended by Enter when you are done: the button guide; velocity with Full and Half Level; Note Repeat; Latch (two pads latched, let go, a new pad takes over, NOTE REPEAT stops it); the menus; the console, Debug page and touch; STOP. After each it shows the app's log lines from that time (events, pads with their velocities, buttons, wheel) and asks what you saw. Every line goes in the report.

### Round 5 results (2026-11-11)

From the owner's results zip (clone at `38905af`). The owner's summary: "That worked beautifully!"

- **Storage:** the hashes of `p1`–`p6`, the U-Boot environment and both boot areas match the baseline before the test and after the power-cycle, with 0 sectors written to `p1`–`p6`.
- **Start-up:** the test build answered SSH 24 s after the jump (eth0 needed its usual retry). The galaxy, then the loading page with its progress bar and messages, then the pad player filling the whole screen, with no log at the edges (the owner's answer). No unit failed.
- **Button guide:** **MENU is button 123, NOTE REPEAT is 11, TAP TEMPO is 53, − is 55 and + is 54** (notes on channel 1 of the Private port). All five are in Akai's startup light list. Round 4's unknown button 11 was NOTE REPEAT, and the guess for TAP TEMPO was right. Two more buttons were pressed once each and are still unnamed: 52 and 73.
- **Sound:** about 7.5 minutes of play: 409 pad hits plus 209 Note Repeat hits, kits 1 to 5, up to 24 sounds at once (the voice limit), peak −10.8 dBFS, 0 periods clipped, **0 underruns in 155145 periods**.
- **Pad to sound:** mean 8.67 ms (8.56 to 8.70 ms) over 358 timed hits, the same as round 4.
- **Note Repeat and Latch** (through `mpfree-arp`): held pads repeated, up to 16 hits a second on two pads. SHIFT + NOTE REPEAT turned Latch on, pads 1 and 2 stayed latched after release, and STOP stopped everything. The note engine dropped 0 events, with 0 overflow.
- **Velocity:** 1 to 127 again, and 72 of the 409 hits (18 %) arrived as exactly 127 (28 % in round 4). 184 hits (45 %) were below 32.
- **Menus and console:** MENU was pressed 8 times, SHIFT + MENU (Prefs) once, and SHIFT + STOP (console) twice. The app doesn't log page changes, so these rest on the owner's summary.
- **Not tested this round:** FULL LEVEL (button 39) was never pressed, so Full and Half Level are untested. No screen pad hit or kit tap was recorded ("0 from the screen"), and the owner skipped the check's paced phases, so touch in the pad player is still unconfirmed.

## Round 6: the looper, the synth and the sampler in one pad player

The owner asked (2026-11-11) for the next round to bring together what the other threads built: the looper (the time engine's clips, record, overdub and undo), the synth and the sampler, all in the GUI. Then also for a debug mode in Prefs with tools for measuring performance and for mapping the remaining buttons at his own pace, and for pad banks A-H with all the transport buttons.

**New in the pad player** (none of it tried on the MPC yet):

- **The sampler engine** ([`mpfree-sampler`](../../../rust/mpfree-sampler)) plays the kits, plus a Grains kit (the first kit as grain clouds; pressure changes them by row). STOP fades everything out.
- **The synth** ([`mpfree-synth`](../design/synths.md)): the last entry in the kit list, "Synth: Poly", plays notes from the pads (pad 1 = C3), and the **Synth** tab edits it (Q-Links 1-4 on the marked knobs, the wheel on the selected knob, − / + for pages). SHIFT + wheel on Play moves the pads by octaves.
- **The time engine** (the table under Round 5): the transport bar, the **Clips** tab, recording, overdub and undo. Clips recorded while the synth has the pads play the synth back.
- **Debug mode** (Prefs, off by default) adds the **Debug** tab, with a strip for its three pages; SHIFT + wheel steps through them, and − / + on the first two:
  - **MIDI**: the MIDI log, events and timing (the round 5 Debug page).
  - **Performance**: CPU per thread and for the whole program, memory, load, temperature and clock (read-only from `/proc` and `/sys` by the screen thread every 2 s; [`perf.rs`](../../../rust/mpfree-app/src/perf.rs)), and the audio loop's render time for the sampler and the synth against the 2.9 ms period, the worst period, steals, dropped events and grains, with the last minute drawn. While debug mode is on, a `perf:` line goes to the system log every 30 s.
  - **Map controls** ([`app/mapper.rs`](../../../rust/mpfree-app/src/app/mapper.rs)): every button of the MPC One in a list, found, skipped or still to find, with a count of each. Tap one (or pick it with the wheel and press +) and press it on the MPC: `map: REC = button 73` goes to the log. While MPFree listens, buttons do nothing else; STOP stops listening and − skips a control that isn't there. A channel 1 CC that isn't the wheel or a Q-Link is taken too, for knobs. Other 1-10 hold buttons the list doesn't name. Leaving the page logs a `map summary:` line. Prefs > Map controls opens it directly, so it works without touch. The names in the list come from the MPC One's panel as MPFree expects it, so they are assumptions until each is found.
- **REC, OVERDUB, UNDO, PLAY START and the PAD BANK buttons** work as soon as the mapper finds them. The button guide no longer runs at power-on (Prefs > Find the buttons again still has it).
- **The audio looper** ([`mpfree-looper`](../design/looper.md)) has its own **Looper** tab, after Synth: one stereo loop on the transport, with the loop's waveform and bar lines, Record, Overdub, Undo, Redo, Clear, Mute and Monitor, a level meter and the looper's state. By default it records what the pads and the synth play (after the master level, before the click), which opens nothing on the MPC. REC, OVERDUB and UNDO (SHIFT + UNDO: redo) work the looper while its tab shows. MENU there has its options: Record from (Pads and synth, or Line input), Line input, Monitor, the first pass's length (1, 2, 4 or 8 bars, or free), the latency to make up for and the loop's level. A `looper:` line goes to the log for each press and change, and every 30 s a `status: looper: ...` line with its render time.
- **The line input** (Prefs, before System info and the Touch screen section, and the Looper options): **Off at every start, and never saved as On.** Turning it on opens the ACVA card's capture device (stereo S32_LE, 44.1 kHz, 128-frame periods, a 256-frame buffer, as Akai's app used it), read without waiting so the outputs never wait on it, and the looper then records it. If the card refuses, the log says why and it stays Off. Which jacks it reads is not checked yet (line in L and R is the likely reading). It is the `line` step below, after its own go.
- **Pad banks A-H**: banks A-H are kits 1-8 (from kit 9 they start again at A). PAD BANK A/E to D/H pick A-D; SHIFT, or pressing the button of the bank playing again, picks E-H (an assumption from other MPCs). The kit chip shows the bank, the bank playing is lit (when Akai's app was seen lighting that button), and `bank: B (kit 2: …)` goes to the log. Clips play whichever bank is playing; a bank of its own per clip note is next, with 128-pad programs.

**The check** (`MPFREE_APP_STEPS=app`, still **read-only**) has ten phases: debug mode and Map controls; Performance; kits, pad banks, Full and Half Level; Grains; the synth; Note Repeat and the arp; clips (record, overdub and undo); the audio looper on the pads; the Touch lab; menus, console and STOP. At the end it reads the whole session from power-on, so a skipped phase or free play still counts, and it shows the touch interrupt count, the latest three status lines and the latest `perf:` line.

#### The line input step (state-changing)

Last, and only with the owner's go: `MPFREE_APP_STEPS=line`. It is the first time MPFree opens the MPC's capture device. The tool itself still only reads: the pad player started at power-on opens the device when you turn Line input on.

1. **Read-only first.** It shows whether the pad player runs and the ACVA card's capture device: the `/dev/snd` node and `/proc/asound/cardN/pcm0c/info`, `sub0/status` and `sub0/hw_params` (`closed` while nothing records), next to the playback side's settings. Without the pad player it stops there.
2. **The change, the undo and what can go wrong,** then it waits until you type `LINE` (`s` skips the step, `q` stops):
   - *Change*: the pad player opens the capture device and starts it. Nothing goes to the eMMC, the U-Boot environment or the mixer settings, and nothing is saved.
   - *Undo*: Line input Off (Looper options or Prefs) closes it. Leaving the pad player, or switching the MPC off (the test build lives in RAM), closes it too.
   - *If it fails*: if the card refuses to open or start the input, the log says why and the row stays Off; after more than 3 read errors in a row the pad player closes it itself and says so, and the looper goes back to recording the pads. Playback does not wait on the input. Not known yet: whether Akai's driver records on the test build, which jacks the two channels are, and the input's gain.
3. **Three phases**, each started by Enter and ended by Enter, each followed by the same read-only look: Line input on and its meter (and which jack is L or R); a loop of the line input with an overdub, undo and Monitor; Line input off. At the end it shows every `line input:` and `looper:` line of the session.

## Round 7: saved settings, the Dev tab and the Sample tab

Round 7 puts the pad player on the layout the owner approved (2026-11-11, [layout-spec.md](../design/layout-spec.md)) and keeps its settings between starts ([storage-spec.md](../design/storage-spec.md), approved 09:30Z). None of it has been tried on the MPC yet.

- **The top bar:** the tabs are Play, Clips, Synth, Looper and Sample, and Dev comes after them while Prefs > "Show developer tools" is on. The bar shows no MPFree name (the startup screen and Prefs > System info do), and Prefs is not a tab: SHIFT + MENU opens it. Every crate's preview draws the same bar (`mpfree_ui::widgets::APP_TABS`, `Header::app`).
- **The Dev tab** (the old Debug tab) has six pages, stepped through with SHIFT + wheel: MIDI, Performance, **CPU by engine** (each audio stage, from period, sampler and synth to looper, time and mix, against its budget from `mpfree-prof`, then control: the rest of the period, which is not sound), **Processes** (every process on the MPC, CPU and memory, read from `/proc` only while the page shows), Map controls and Console.
- **Saved settings.** The test stick gets a fourth partition, `MPFREE-DATA` (GPT name `mpfree-test-data`, 64 MiB, ext4 with a journal). The test build mounts it at `/run/mpfree-data` (`rw,nosuid,nodev,noexec,noatime`) and starts the pad player with `--data /run/mpfree-data`. If it does not mount, the pad player starts without saving and the kernel log says so.
  - Kept: the Prefs rows pad velocity curve, screen pad velocity, Latch keeps pads, audio buffer and Show developer tools (`settings/app.conf`), and every control found with Map controls or the button guide (`settings/map.conf`). Line input is never kept.
  - All writes go through `mpfree-store`'s one writer thread: a `.part` file, flushed, then renamed, then the folder flushed. It never follows a link, never writes outside the folder and refuses a folder that is not its own. At start the log says `saved settings: …`, `saved buttons: …` and `saving: …`.
  - The eMMC and the SD card are never written. `write-test-stick.py` keeps partition 4 from the stick it rewrites. Since 2026-11-12 it sits at 1 GiB and fills the rest of the stick: it is left in place when it is already there, or saved to the build folder and moved from the old 64 MiB layout ([details](../hardware/archive/2026-11/kexec-test-boot.md#saved-settings-partition-4)). `--fresh-data` starts it empty, and `--data-from FILE` writes a saved copy back.
- **The Sample tab** (#75, #78): the sampler's pad editor on the pad hit last, with the Wave and Chop tabs. Edits stay in memory.
- **Capture** (#80): the Clips tab's Capture button keeps the phrase just played as a take, even when nothing was recording.
- **The arp:** the Arp panel's MENU has its options, including scales, chords and the scale run. Its MIDI out and MIDI clock are built as bytes only; no MIDI port is opened yet. NOTE REPEAT pressed twice quickly latches, as SHIFT + NOTE REPEAT does.
- **System info** (Prefs): the program's version, the system and the hardware.

**The check** has twelve read-only phases now: developer tools and Map controls; the Dev pages; kits, banks and pad levels; Grains; the synth; Note Repeat and the arp; clips, overdub, undo and Capture; the audio looper; the Sample tab; Prefs, System info and saving; the Touch lab and Calibrate touch (both now in Prefs > Touch screen); menus, console and STOP. The line input step and the benchmark step each come after their own go, and a second boot shows whether the settings came back.

### Dev > Touch (#145)

With the developer tools on, the Dev strip has a **Touch** page between Map controls and Console. It tests the touchscreen, measures it and tunes it ([M17](../milestones/m17-touch-tuning.md)).

- **The test area** draws each finger with its trail, its untuned position and the panel's raw reading, and names the gestures the current thresholds see. Its bar has Free, Targets (nine rings over the whole page), Swipes and Clear.
- **The tuning list** shows each value now and suggested, with the reason. The wheel picks a value and - / + changes it at once. Use suggested, Defaults and Save work with wheel and + as well, so they never need the screen.
- **MENU** opens Calibrate touch and the Touch lab.
- **Save** writes `settings/touch.conf` to the data partition, which is read at the next start. The shift, stretch and hold filter apply to every touch. The gesture values apply to the Touch page and the Touch lab.

### The factory drum kits

[`offline-factory-kit.py`](../../../tools/mpc-one/offline-factory-kit.py) runs **on the workstation only** and never contacts the MPC. It reads the owner's [eMMC backup](../hardware/archive/2026-11/emmc-backup.md):

1. It finds the newest `~/mpfree-backups/mpc-one-emmc-*/mmcblk0.img`, reads its GPT (checking both CRCs) and finds the partition named `content` (p7, `/media/acvs-content` on the MPC).
2. It lists that ext4 file system with `debugfs`, which opens the image read-only (no `-w`) at the partition's offset. Nothing is mounted and no `sudo` is needed.
3. It reads the drum programs (`.xpm` files whose XML says `type="Drum"`). Kit 1 is the first, by path, whose pads 1 to 16 name at least 8 samples it can find. It tries two readings of which sample a pad plays (by pad number, and by the pad's note) and reports which it used; the `.xpm` layout is inferred from the files, not documented. `--program TEXT` picks the first program whose path contains `TEXT`. If no program fits, it takes the folder with the most drum-named WAV files.
4. Kits 2 to 8 (`--kits N` asks for 1 to 16) are the next programs by path with at least 12 samples found, one per name family (the part of the file name before `-Kit`, or its first word), so the list is not eight versions of one kit. A kit that would take the file over 48 MiB is left out.
5. It copies each kit's WAV files out with `debugfs` into a temporary folder, converts them to 44.1 kHz stereo 16-bit (at most 10 s each), and deletes the copies.
6. It writes a new folder `~/mpfree-work/factory-kit-<UTC>-XXXXXXXX/` holding `kits.bin` (every kit with its name; the format is in [`kit.rs`](../../../rust/mpfree-app/src/kit.rs)) and `report.txt` (file and sample names, formats and kit 1's program XML; no audio).

The kits are Akai's copyrighted content, under the [factory content policy](../project/factory-content-policy.md): it stays in `~/mpfree-work` and in the test build's RAM. They are never committed or shared, and `results.py` copies only `report.txt` into a round's zip.

## Rounds 8 to 10: the app as it stands

None of rounds 8 to 10 has run on the MPC yet; they are tested on the workstation (unit tests, the golden screens from `mpfree --preview DIR`, and the bench). The [layout spec](../design/layout-spec.md) has every page in detail; this is the short list of what to try.

- **Round 8 (#106): the Play page.** No big pads: the transport bar with the view switch (All, Beats, Loop, Drum), the loop strip, the pattern with a speaker per lane, eight throws, the XY pad, the thin master bar and the Q-Link column on the right. Clips became **Arrange**. Its mini pads came off after the 12 Nov live test (the owner's pick A): the pattern, loop or timing panel takes their room.
- **Round 9 (#136): the Looper tab and the control map.** The Looper tab edits and sets up the loops (mockup J): the loops and their layers, the wave, the tools row, Q-Links for the layer's level and pan. Every mode button goes to the page with the same job (MAIN, STEP SEQ, NEXT SEQ, SAMPLER, SAMPLE EDIT, PROG EDIT), SHIFT + STOP goes back to bar 1, SHIFT + REC is Capture, SHIFT + PLAY START the metronome, SHIFT + 16 LEVEL the synth pads' Chromatic | In key. The Console is hold SHIFT + hold ERASE + TAP TEMPO. Buttons without a measured note work once Map controls (Dev tab) has found them.
- **Round 10 (#186 and the PR after it):**
  - **The FX chain editor** opens from a mixer strip's insert row (or its "+", with the effect picker up), in the mixer's place; Back returns to the mixer. The Q-Links turn the effect's first four knobs.
  - **Q-LINK** steps the Q-Link column through Macros 1-4 and Macros 5-8 on Play and in the chain editor. Holding a macro card still for half a second opens the **macros panel**.
  - **The Looper tab's tools work:** ½ length, × 2, Reverse, the nudges, the seam fade and Merge layers; **Undo history** lists every step and goes back or forward to any of them; **trim handles** on the wave cut the loop by 1/16 notes. Q3 is Fade old and Q4 the seam fade. After recording (play or overdub) and Use the loop > Resample work.
  - **FILL:** PLAY while the transport plays is FILL while held; pressed twice it latches until PLAY again or STOP.
  - **Perform FX on Play:** the throws and the XY pad play `mpfree_fx::Perform` on the master (the XY pad is its DJ filter across and its echo up, and glides home when let go). The FX mix Q-Link is Perform's mix.
  - **TRACK MUTE:** the pads become mute keys over any page: pads 1-8 mute tracks 1-8 (green plays, red muted). SHIFT + TRACK MUTE does the same for the kit's pads. Press it again to play the pads.
- **After the 12 Nov live test: the Pads tab** (the owner's pick, layout spec section 4e). A top-level tab between Play and Arrange: the bank's pads on the left and a **Browse | Pad** switch. Browse is the browser beside the pads (with `--data`): Samples, Recordings, Recent and Favourites, Load (onto the pad picked), Load to pad (onto the next hit), Audition and Favourite; the wheel, its push, - and + and PLAY work it, PLAY hears the sound picked through the preview path (`preview.rs`, mixed into the main out). Pad is the old Sample > Pads page. The Sample tab gains **Send to pad** and **Chop to pads** beside its bar; PROG EDIT opens the Pads tab's Pad view. At start the library makes `library/` and its folders under `--data` when they are missing (the stick's partition 4 only; undo: delete the folder from the workstation, or rewrite the stick with `--fresh-data`).

- **Safe start after a crash** ([`crash.rs`](../../../rust/mpfree-app/src/crash.rs)). A start that finds `/run/mpfree-app/running` still holding a pid knows the last run in this boot crashed (a panic, whose line is in that file and in the report; a crash outside Rust; or the out-of-memory killer). It says so in the report (`safe start: the last run crashed (...)`), keeps the new, empty project even when Prefs > At start is Last project, and says on the hint line which project wasn't opened. With the stick's restart this is what follows a crash a few seconds later, so a project that crashes the app as it opens cannot do it at every start. `host_app_mpc.py` keeps `PANIC: ` and `safe start: ` lines in its summary.

Still to come in the app: hold MAIN + pad to pick a track, SHIFT + NEXT SEQ for a Perform FX page, the Looper's bounce rows (To a pad, To a clip, Save loop), Sample > FX, the browser overlay. The audio tracks record onto the Song view's lanes ([audio tracks](../design/audio-tracks.md#in-the-pad-player)); their panel, take lanes and saving come next.

### The live test

The app check (`MPFREE_APP_STEPS=app` with `collect-over-ssh.sh --app`, read-only) has nineteen phases, in this order, because later phases use the buttons the first one maps. The 12 Nov run skipped most of them, and the 13 Nov run stopped at phase 1, so the next run repeats them all with what has merged since (the live test 2 fixes in four phases, 4, 9, 11 and 14, and the retest of the Warp crash in 14):

1. Developer tools and Map controls: REC, OVERDUB, PLAY, PLAY START, UNDO, ERASE, TAP TEMPO, the PAD BANK buttons, TRACK MIX, TRACK MUTE, Q-LINK, DATA DIAL PUSH and the six mode buttons first.
2. The Dev pages.
3. Kits, banks, Full and Half Level.
4. The Play page's four views without pads, the loop strip's symbols, the Q-Link cards (no flicker), the pattern, the master bar and the mode buttons (PROG EDIT opens the Pads tab), and STOP's light (PLAY, REC and OVERDUB stay dark until Akai's messages for them are captured).
5. Throws, the XY pad on Perform FX, and FILL.
6. Arrange: clips, takes, Capture, the note editor (Grid, List, Tracker, Timing Correct, the Euclid rows set with the wheel, - / + and the push), the Song view, the morph bar and Rec song.
7. TRACK MUTE and SHIFT + TRACK MUTE.
8. Grains.
9. The Synth tab's five engines (Poly, FM, Drums, Pluck, Wavetable), the wheel picking tiles, Drive turned up on Grit Bass (grittier, not quieter), and Tuning and A4 in its options.
10. Note Repeat and the arp.
11. The Looper tab: the live wave, STOP stopping the loop, layers, the tools, the undo history, trim, several loops, Resample, a loop card's speaker (mute), Clear loop, and × on the last layer.
12. The mixer, the chain editor, the I/O view (Audio, MIDI, Sync, CV; MIDI out is off in this build), the Q-LINK banks and the macros panel.
13. The Pads tab: the pads, Browse, Pad (the wheel picks a row), Copy, Paste, Clear, and Map to keys.
14. The Sample tab: the wheel on its rows and SHIFT + - / + zoom, the Warp retest on a very short sound (the 13 Nov crash), Wave, Chop, live chop, Stops, Also plays, Latch and Send to pad.
15. The Sample tab's scenes (Store A, Store B, the crossfader, Return to A) and FX (the chain editor on the pad's channel and on Drums).
16. Prefs (every row, the wheel's push, Loop memory set to 10 min for the second boot), System info and saving.
17. A MIDI keyboard on the 5-pin MIDI IN, if there is one: kit pads and the synth from the keys. Never a USB keyboard while the stick holds the running system.
18. Touch: the Touch lab, Calibrate touch, and Dev > Touch (Save writes only the stick's partition 4).
19. Menus, the Console chord, TAP TEMPO and STOP.

At the start the tool also reads, read-only, partition 4's size and free space and the first lines of `/run/mpfree-data-grow.txt` (`partition 4: …` and `partition 4 grow: …`), which shows whether 3.10's `resize2fs` grew it to fill the stick. Then it reads, read-only, what the GPU work needs (`gpu: …` lines, for the profiling thread's GPU plan): the EGL, GLES, GBM and DRM libraries and `/usr/lib/dri`, the release from `/etc/os-release`, the `/dev/dri` nodes, each display connector's status and modes, which programs hold `/dev/dri` open, whether the Panfrost driver is bound, and the display state from debugfs only where debugfs is already mounted. It opens no device node, mounts nothing, and puts the debugfs lines in the report only. The interrupt table comes from the core probe after phases 2 and 11. It prints `start-up: the app was playing N s after power-on`, read from the log line that starts with `playing ` (Perform FX and the wavetables allocate about 18 MB at startup). The line input step (state-changing, its own go) now checks the left and the right jack one at a time; round 7 checked only the left.

### Live test 3 (2026-11-13, main c556b49)

The owner ran steps 1 to 5, the benchmark and the GPU benchmark, the boot map after and the stick logs (zip `mpfree-results-20261113T114411Z`, owner-local). The line input, the stick speed test and the second boot's app check weren't run; the owner saw the saved settings come back on the second boot. The boot partitions' hashes matched before and after, with 0 sectors written to p1-p6 and boot0/1.

- **Akai's transport lights** were captured first, on stock 3.6.0 ([controller protocol](../hardware/controller-protocol.md#akais-transport-lights-live-test-3-2026-11-13)). The MPC One's STOP button has no LED.
- **The app check ran in two parts.** The workstation's terminal closed after phase 5. The check is read-only and its session summary reads the app's whole log since power-on, so it was started again under another name (`results.py run app-check-resume -- …` with the same command) and phases 1 to 5 were skipped with `s`. Before that, the newest `research/mpc-one-app-*.txt` was copied into the results folder, because `results.py` writes the log through a buffer and loses up to 8 KB of its end when the terminal goes; the report in `research/` is written line by line.
- **The app check stops itself after an hour:** `collect-over-ssh.sh --app` runs `host_app_mpc.py` under `timeout --foreground 3600`. All nineteen phases take longer than that when every one is tried. Raise the limit, or let the check start at a given phase, before the next run.
- **What the run found** (causes traced in the code, no fixes yet): the chain editor's taps never reach the app (its widget's `touch` returns no event, so `gui.rs` never takes its actions); BROWSE has no job in the mapper; the Grain page's Mode row can't step to Loop, so it never reaches Granular; In key without a scale only writes a line in the Dev log; and the Beats view never pages past the first 16 steps. The full list, with Jay's requests after the run and the answers to his questions, is in mpfree-claude `handoffs/2026-11-13-app-thread.md`.
