# Hello MPC (Phase 1) on the kexec test build

**Status:** round 1 ran on 2026-11-10: the screen, touch and click work; the controller sent no pad, button, Q-Link or wheel data. Round 2 looks for the reason ([below](#round-2-why-the-pads-were-silent)).

Phase 1 is the first time MPFree's own code drives the MPC One's hardware: the screen, the touchscreen, the pads and the audio output. It runs on the [kexec test build](../hardware/kexec-test-boot.md): Akai's 3.10.0 system started from the USB stick, with Akai's app blocked and the eMMC and SD card hidden from it. A power-cycle brings back stock 3.6.0.

## What runs, and what it changes

The program is [`mpfree-hello`](../../rust/mpfree-hello/src/main.rs), a static ARM binary. `collect-over-ssh.sh --hello` runs [`host_hello_mpc.py`](../../tools/mpc-one/host_hello_mpc.py), which:

1. checks the program on the workstation (a static ARM EABI5 hard-float executable);
2. connects **only to the test build**: it uses the test build's own SSH host key, so stock 3.6.0 is refused before anything runs. On the test build it checks, read-only, the release, that no MMC device exists, that kexec is off, that `/` is read-only, that `/run` allows programs and has room, and that Akai's app is not running. It also lists which programs hold the sound, MIDI, touch and screen devices;
3. copies the program to `/run/mpfree-hello` (RAM) and checks its hash there;
4. runs one step at a time. Each step starts only when you press Enter. `s` skips a step and `q` stops.

| Step | What the program does | Writes to |
|---|---|---|
| inventory | lists sound cards, PCM and MIDI devices, input devices, the framebuffer, panel rotation, real-time limits | nothing but its report |
| screen | stops our own log display (`mpfree-test-screen`, started again at the end), draws the Hello MPC card | `/dev/fb0` and its sysfs `pan` file |
| touch | shows your touches for 10 s; reads the ILI2117 device without grabbing it | the screen |
| pads | listens to all three of the controller's MIDI inputs (Public, Private, MIDI Port) for 15 s and shows every message in hex, with its meaning (pad, button, Q-Link, wheel) where known; then lists the kernel's MIDI byte counters | the screen |
| pads, second try | only if no pad hit arrived: listens again while sending Akai's keep-alive message every 5 s on the Public port (the only message it ever sends to the controller) | the controller's Public MIDI port |
| click | 120 BPM click for 10 s on the ACVA output, about 18 dB below full scale, at the stock app's stream settings (stereo S32_LE, 44.1 kHz, 128-frame periods, 3 periods); counts underruns and late periods | the ACVA playback device |

The program also appends every line to `/run/mpfree-hello/hello.txt`, which the tool copies into its report at the end. Nothing is written to the stick, and the test build cannot see the eMMC. Everything is gone after a power-cycle.

**Turn the MPC's volume knob most of the way down before the click step.** The level is fixed in the program (−12 dB, peak about −18 dBFS), but the analogue volume knob sets what you hear.

## The program

[`rust/mpfree-hello`](../../rust/mpfree-hello) talks to ALSA directly with ioctls, as [ADR-0002](../decisions/adr-0002-direct-alsa-audio.md) chose, without alsa-lib. The parameter blocks are built as bytes in the 32-bit ARM layout. [`abi-check.c`](../../rust/mpfree-hello/abi-check.c) checks every size, offset, ioctl number and constant it uses against the kernel's own `<sound/asound.h>` and `<linux/input.h>`, compiled for ARM. CI compiles it on every change; a mismatch fails the build.

A few things it relies on, checked against the kernel source (6.18):

- **MIDI substreams.** The controller's three cables are substreams 0 (Public), 1 (Private) and 2 (MIDI Port) of `midiC<card>D0`. ALSA picks one through the "preferred subdevice" set on the card's control device by the same thread, as alsa-lib does. The program then asks the kernel which substream it got (`RAWMIDI_INFO`, which must name the direction) and stops unless the subdevice and name match.
- **Blocking writes.** The PCM is opened non-blocking, so a busy device fails at once instead of waiting. Then it is switched to blocking; `PREPARE` copies that flag into the stream.
- **Underruns** show as `EPIPE` from a write. The program counts them, prepares the stream again and refills it with silence.

It locks its memory (`mlockall`) and runs the click at `SCHED_FIFO` priority 70; both are reported, and it carries on without them if refused.

## Running it

Build the program on the workstation (the same flags as the screen log program):

```bash
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" \
cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-hello
```

Then, with the test build running (`--kexec-boot`, see [the kexec test boot](../hardware/kexec-test-boot.md)):

```bash
MPFREE_ACCEPT_HELLO=YES bash tools/mpc-one/collect-over-ssh.sh --hello
```

Wrap both in `python3 tools/mpc-one/results.py run NAME -- ...` so the output lands in the round's results folder. Afterwards: switch the MPC off, take out the stick, wait 10 s, switch it on, and run the boot-map survey to confirm nothing on the eMMC changed.

`mpfree-hello --preview FILE.ppm` draws a sample card into a new file on the workstation, with no devices.

## Round 1 results (2026-11-10)

From the owner's results zip (clone at `da8a614`):

- **Storage:** after the test build, the hashes of `p1`–`p6` and both boot areas match the baseline, and those partitions show zero writes since boot.
- **Screen:** the card was drawn the right way up. **Touch:** one and two contacts showed where they were touched.
- **Click:** 3445 periods at the stock stream settings with 0 underruns and 0 late periods. The gap between periods averaged 2.903 ms (one period is 2.902 ms), and the worst was 2.939 ms. It ran with `mlockall` and `SCHED_FIFO` 70.
- **Pads:** nothing arrived on the Private input in 15 s. With the keep-alive, the controller answered (three replies, the first different from the usual one), but still sent no control data. Details in [the controller protocol](../hardware/controller-protocol.md#without-the-factory-app-hello-mpc-round-1-2026-11-10).
- Ethernet came up at 20 s on that boot without the retry.

## Round 2: why the pads were silent

Two measurements, in one round:

1. **On the test build**, the pads step now opens all three MIDI inputs and prints every byte, so a reply or message the program doesn't know is no longer dropped. The keep-alive is still the only thing it sends.
2. **On stock 3.6**, [`host_app_start_capture.py`](../../tools/mpc-one/host_app_start_capture.py) (`collect-over-ssh.sh --app-start-capture`) records what Akai's app sends to the controller as it starts:
   - Read-only checks first: stock 3.6's kernel release, Akai's app running, card 0 is the controller in its normal MIDI mode (`09e8:0046`), no USB drive attached, a usbmon reader for its bus. It notes the controller's USB address and firmware version.
   - It starts a passive usbmon capture of that bus. It prefers the binary `/dev/usbmonN`, which keeps whole transfers, over the debugfs text file, which keeps 32 bytes of each. The reader stops by itself within 240 s.
   - After you type `RESTART`, it runs `systemctl restart acvs.service`. **This is the state change:** Akai's app quits and starts again, as at every power-on, and may save its usual settings to `/data`. systemd does the restart, so a dropped SSH session cannot leave it half done.
   - You press Enter when the app's main screen is back, then hit pads and turn a Q-Link and the wheel for 8 s.
   - Afterwards, read-only, it checks that the app runs again as a new process and that the controller is the same USB device (same address, ids and firmware version).

   The report lists every message between the app and the controller from the restart on, decoded and in order, and every captured transfer in hex.

Why the restart cannot flash the controller (from [the updater notes](../hardware/akai-updater-offline.md); the second point is an inference): the app's launcher flashes the controller only if it is already in its firmware-update mode, which the checks rule out (`09e8:0046` is its normal MIDI mode). And this same app starts at every power-on of this unit, with the same controller firmware, without re-flashing it: with the app running, the controller was still USB device 2 on its bus in the 2026-11-09 capture, so it had connected only once since power-on, and a flash would have reconnected it in another mode. If Akai's app does not come back, switch the MPC off and on.

Messages are sent to the controller only after they have been seen in this capture and listed for the owner's OK (round 3).
