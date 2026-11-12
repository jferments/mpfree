# MIDI I/O, sync, MIDI Learn and CV/gate (`rust/mpfree-midi`)

The plan for MPFree's external MIDI and CV, and the contract of the host-first crate `mpfree-midi`. Scope:

- MIDI in from the 5-pin MIDI In jack and from USB keyboards on the USB-A socket;
- MIDI tracks out of the MIDI Out jack;
- clock sync in and out;
- MIDI Learn;
- the CV/Gate outputs.

The MPC's own pads, knobs and buttons are not part of this. They come through the controller protocol (`mpfree-controller`) and the app's mapper.

Device safety (rule zero) applies to every step below that touches the MPC. Each such step is labelled read-only or state-changing, and waits for the owner's go.

## What we know about the hardware

| Fact | Status | Source |
|---|---|---|
| One 5-pin MIDI In, one 5-pin MIDI Out, one USB-A (MIDI controllers allowed), one USB-B | OBSERVED (documented) | MPC 3.6 user guide, MPC One rear panel and specifications |
| **4** CV/Gate jacks, 3.5 mm TRS: a TS cable carries one signal, a TRS-to-dual-TS breakout two, so up to **8 signals** | OBSERVED (documented) | the same pages |
| The CV/Gate voltage range of the MPC One | **UNKNOWN**: the guide does not give it, and it warns that V/oct scaling differs between devices | guide glossary, "CV" |
| The internal controller is USB `09e8:0046`, a standard USB-MIDI 1.0 device with three cables: 0 "Public", 1 "Private", 2 "MIDI Port". It has no audio streaming interface | OBSERVED | [audio-midi-survey](../hardware/audio-midi-survey.md) |
| Akai's app routes only cable 2 through the ALSA sequencer, and every captured physical control is on cables 0 and 1 | OBSERVED | the same, and [midi-activity-map](../hardware/midi-activity-map.md) |
| Cable 2 carries the DIN jacks | **DERIVED, not observed**: nothing was plugged into the jacks during any capture | — |
| Akai's app holds no spidev, gpiochip or IIO device, so it drives the CV jacks through the controller, over USB-MIDI | **INFERENCE** | [resource-ownership](../hardware/resource-ownership.md) |
| A USB keyboard on USB-A shows up as another ALSA card (snd-usb-audio is in the kernel, since card 0 uses it) | DERIVED, untested | — |

### The read-only check that settles the DIN cable and the CV path

`MPFREE_ACCEPT_USBMON=YES MPFREE_CAPTURE_PHASES=midi-cv bash tools/mpc-one/collect-over-ssh.sh --midi-capture`. It is **read-only on the MPC** (the opt-in passive usbmon reader, see [audio-midi-diagnostics](../guides/audio-midi-diagnostics.md)) and runs on stock 3.6 with Akai's app.

The owner sets up a MIDI track (output MIDI Out) and a CV track, without saving anything. The tool then walks through six phases:

1. idle;
2. notes into MIDI In;
3. pads on the MIDI track;
4. pads on the CV track;
5. one held CV note;
6. one CV slider.

The report counts messages per cable against idle, which shows:

- which cable carries DIN In and Out under Akai's app;
- whether CV moves anything on USB, and in what form.

If the CV phases show no USB traffic, the CV is driven some other way. The next read-only step is then the device tree (`offline-dtb.py show` on the owner's saved tree) for a DAC or SPI node.

**No MIDI port and no CV output is opened by MPFree until the owner has seen this result.**

## The crate: `mpfree-midi`

Bytes and volts only. It opens nothing, it has no unsafe code, and every part that can run on the audio thread or the MIDI reader thread is free of allocation after `new`, which `tests/no_alloc.rs` checks.

| Module | What it does |
|---|---|
| `port` | External ports. `DIN` is always id 0. A USB device gets its old id back when it is plugged in again, by name, so its track inputs, Learn mappings and clock settings follow it. There are 8 ports at most |
| `msg` | Channel messages as values, read from `mpfree_controller::midi::Parser` output. A note-on with velocity 0 is a note-off |
| `input` | `InputRouter`: every byte of every external port in, then track messages, `Seen` messages (for Learn, the monitor and the port lights) and clock bytes out. **Selected** mode (the default, as on Akai's MPC): any port and channel plays the track on screen. **Per-track** mode: each track has its own port, channel and transpose. A note-off always goes where its note-on went, even if the selection changed while the key was held. Unplugging a port ends its notes. Only the chosen clock source reaches the clock |
| `output` | `OutputRouter`: each MIDI track has a port and channel. A route change, stop or panic ends exactly the notes that are on, never 2048 blind note-offs. `OutQueue` holds one port's period, in frame order, with clock bytes first among messages at the same frame, and writes the bytes, with running status if asked. `DinPacer` models the 31250 bit/s wire: a note takes 0.96 ms and a 16-note chord about 15 ms. It reports how late each message and each clock pulse leaves |
| `learn` | `LearnMap`: arm a parameter (`mpfree_param::ParamAddr`), move a control, and it is learned. CC, note, pitch bend and channel pressure can be learned. Modes are absolute, relative (two's complement, offset 64, sign bit), toggle and momentary. Range and inversion are per mapping, and so is takeover: jump, or pickup, which waits until the control passes the value. Mappings save as `learn = …` lines for `mpfree-store`, by device name |
| `cv` | `CvEngine`: 8 outputs. Each one is pitch (1 V/oct or Hz/V; last, low or high note priority; bend range), gate (with an optional retrigger gap), trigger (any note or one note, for drum modules), velocity, a CC, clock (any PPQN), run, reset, or fixed (the CV track's sliders). Pitch holds after the gate falls, so release tails stay in tune. Calibration (gain and offset) is per output. It emits a `CvChange` only when an output's voltage changes. **No range is assumed**: the hardware layer will clamp once the range is measured |

MIDI clock bytes are `mpfree-midi-clock`'s ([midi-clock.md](midi-clock.md)). `InputRouter` feeds `ClockIn` from the clock source. `ClockOut`'s messages go into an `OutQueue` as `Timed::realtime` or `Timed::raw`. How the transport follows another device's clock is the time engine's call.

Tests: `cargo test -p mpfree-midi`. They cover:

- a keyboard on the selected track, with a held note surviving a selection change;
- per-track splits by port and channel, and unplugging a keyboard;
- 120 BPM clock with Song Position from DIN while another port's clock is ignored;
- two tracks out on two channels;
- learning a knob, pickup, encoders, a toggle pedal and a momentary key;
- saving and loading by device name;
- CV pitch and gate on a mono line with retrigger and bend;
- priority, Hz/V and calibration;
- clock, run, reset and triggers;
- DIN pacing of a 16-note chord against a clock pulse;
- no allocation on the audio path.

## Steps

| Step | What | Touches the MPC | Waits on |
|---|---|---|---|
| M0 | The read-only `midi-cv` capture above | read-only (passive usbmon, stock 3.6) | the owner's run |
| M1 | `mpfree-midi`, host only | no | the owner's go to merge (code PR) |
| M2 | App wiring for input. The app opens the DIN In cable (from M0) and USB keyboards **for reading**, routes them to the selected track, and shows a MIDI monitor on the Dev page and a MIDI page in Prefs (ports, input mode, clock source). The layout goes through the app thread | a device round on the test boot: opens MIDI inputs for reading | M0, the owner's go, the app thread |
| M3 | MIDI tracks out. A seq destination for MIDI out, and the DIN Out cable written with paced bytes from a writer thread that sleeps to each message's frame (sub-period timing; to be measured on the unit) | **state-changing, RAM only**: sends note bytes on the DIN cable. Undo: power-cycle. No SysEx is ever sent there | M0 showing the out cable, the owner's go |
| M4 | Clock out (DIN, USB) from the transport, and clock in, with the transport following it | as M3 for out | the time engine thread, the owner's go |
| M5 | MIDI Learn on screen: hold a parameter and pick Learn, mappings in `mpfree-store` | as M2 | the app thread |
| M6 | CV out: a hardware layer from what M0 shows, then a first output test with a multimeter on one jack, then a calibration page | **state-changing**, gated separately, after M0 | M0, the owner's go |
| Later | USB-B as a USB-MIDI device to a computer (the UDC gadget: rule zero, not before the boot work allows it); MIDI file import and export (the arp already writes MIDI files) | — | — |

## Defaults taken (the owner may change them)

1. Input goes to the selected track from any port and channel, as on Akai's MPC. Per-track listening is a mode.
2. MIDI Learn listens to external ports only. The Q-Links keep their own learn (`mpfree-mod`'s bindings).
3. A control moves one parameter: learning it again replaces the old mapping.
4. Learned CCs start as absolute with jump. Learned keys start as momentary.
5. Clock out is off and the clock source is none (internal) until set in Prefs.
6. Running status is on for DIN out, which saves a byte on each repeated status.
