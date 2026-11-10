# Hello MPC (Phase 1) on the kexec test build

**Status:** ready for its first run (2026-11-10). Nothing has run on the MPC yet.

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
| pads | listens to the controller's Private MIDI port for 15 s and shows pads, buttons, Q-Links and the wheel | the screen |
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
