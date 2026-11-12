# MIDI I/O, sync, MIDI Learn and CV/gate (`rust/mpfree-midi`, `rust/mpfree-midi-ui`)

The plan for MPFree's external MIDI and CV, and the contract of the host-first crate `mpfree-midi`. Scope:

- MIDI in from the 5-pin MIDI In jack and from USB keyboards on the USB-A socket;
- MIDI tracks out of the MIDI Out jack;
- clock sync in and out;
- MIDI Learn;
- the CV/Gate outputs.

The MPC's own pads, knobs and buttons are not part of this. They come through the controller protocol (`mpfree-controller`) and the app's mapper.

The owner-facing design and architecture doc, with the research below in full and the open decisions, is [MPFree MIDI, sync and CV/gate: design and architecture](https://claude.ai/artifact/RxdZ76ve5hE5VptdkU3JMo); the proposed screens are drawn in [MPFree MIDI screens](https://claude.ai/artifact/5ygP92smCQVhu31qq2kfYZ). This page stays the source of truth for the hardware facts and the crate.

Device safety (rule zero) applies to every step below that touches the MPC. Each such step is labelled read-only or state-changing, and waits for the owner's go.

## What we know about the hardware

| Fact | Status | Source |
|---|---|---|
| One 5-pin MIDI In, one 5-pin MIDI Out, one USB-A (MIDI controllers allowed), one USB-B | OBSERVED (documented) | MPC 3.6 user guide, MPC One rear panel and specifications |
| **4** CV/Gate jacks, 3.5 mm TRS: a TS cable carries one signal, a TRS-to-dual-TS breakout two, so up to **8 signals** | OBSERVED (documented) | the same pages |
| The CV/Gate voltage range of the MPC One | **UNKNOWN**: the guide does not give it, and it warns that V/oct scaling differs between devices. Akai's pitch spans five octaves over the full 14-bit range, which fits 0 to 5 V at 1 V/oct (INFERENCE until a meter is on a jack) | guide glossary, "CV"; [controller-protocol](../hardware/controller-protocol.md#second-round-the-cv-scale-and-the-next-outputs-2026-11-12) |
| The internal controller is USB `09e8:0046`, a standard USB-MIDI 1.0 device with three cables: 0 "Public", 1 "Private", 2 "MIDI Port". It has no audio streaming interface | OBSERVED | [audio-midi-survey](../hardware/audio-midi-survey.md) |
| Akai's app routes only cable 2 through the ALSA sequencer, and every captured physical control is on cables 0 and 1 | OBSERVED | the same, and [midi-activity-map](../hardware/midi-activity-map.md) |
| The 5-pin MIDI In jack is cable 2 in (notes and the keyboard's clock), under Akai's app | **OBSERVED** 2026-11-12 | [controller-protocol](../hardware/controller-protocol.md#the-rear-jacks-under-akais-app-2026-11-12) |
| The 5-pin MIDI Out jack is cable 2 out, under Akai's app | **OBSERVED** 2026-11-12 (third round; rounds 1 and 2 sent nothing because the MIDI track's MIDI Out routing was not set) | [controller-protocol](../hardware/controller-protocol.md#third-round-the-midi-out-jack-2026-11-12) |
| Akai's app takes 5.1 to 8.1 ms from a note in to the same note out on cable 2 | **OBSERVED** 2026-11-12 (usbmon times on the host) | the same |
| Akai's app drives the CV/Gate jacks through the controller: CCs on cable 1, channel 16 (CC 0 to 3 for a CV track's pitch and gate) | **OBSERVED** 2026-11-12 | [controller-protocol](../hardware/controller-protocol.md#the-rear-jacks-under-akais-app-2026-11-12) |
| Each CV port is one 14-bit value as a CC pair, high half first: Akai's port 1 on CC 0 and 1, 2 on CC 2 and 3, 3 on CC 4 and 5, 4 on CC 6 and 7. The pair follows the port number, whichever use (pitch, gate) the track gives it | **OBSERVED** 2026-11-12 (ports 5 to 8 on CC 8 to 15: INFERENCE) | [controller-protocol](../hardware/controller-protocol.md#second-round-the-cv-scale-and-the-next-outputs-2026-11-12) |
| A CV track's pitch is `floor((note − 36) × 16383 / 60)`: 0 at note 36, five octaves to full scale. Its gate is full scale while held, 0 after | **OBSERVED** 2026-11-12, six notes | the same |
| Which jack, and tip or ring, each output number is | **UNKNOWN** | — |
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

**First result (2026-11-12):** MIDI In is cable 2 in, and the CV jacks are driven by CCs on cable 1, channel 16. Nothing was seen for MIDI Out. Details are in [controller-protocol](../hardware/controller-protocol.md#the-rear-jacks-under-akais-app-2026-11-12).

**Second round (2026-11-12):** `MPFREE_CAPTURE_PHASES=cv-scale`, read-only in the same way. The keyboard on MIDI In played known notes (C in three octaves, then four semitones) into the CV track, and the capture saw those note numbers on cable 2, so no pad layout was needed. Result: a CV track's pitch is `floor((note − 36) × 16383 / 60)`, and Akai's CV ports 3 and 4 are the next CC pairs (CC 4 to 7). A MIDI track again sent nothing on USB. Details are in [controller-protocol](../hardware/controller-protocol.md#second-round-the-cv-scale-and-the-next-outputs-2026-11-12), and the bytes are in `cv_wire`, below.

**Third round (2026-11-12):** `MPFREE_CAPTURE_PHASES=midi-out`, read-only in the same way. In rounds 1 and 2 the MIDI track had only its MIDI Output Ch set; the owner found that its MIDI Out routing also had to be set (to "MPC", 1), and a synth on the jack played. Result: the MIDI Out jack is **cable 2 out**. Notes leave on the track's channel 5 to 8 ms after they arrive, note-offs as `80 nn 00`, pad pressure as poly pressure. PLAY and STOP sent no clock. Details are in [controller-protocol](../hardware/controller-protocol.md#third-round-the-midi-out-jack-2026-11-12).

M0 is done: both 5-pin jacks are cable 2 of `midiC0D0`, and the CV jacks are CC pairs on cable 1. Clock out from Akai's app was not seen (Sync may have been off); it is not needed for MPFree's own clock out, which M4 writes to cable 2 itself. The CV volts per count need a meter on a jack while MPFree sends a known value, which is state-changing and belongs to M6.

**No MIDI port and no CV output is opened by MPFree until the owner has seen these results.**

## The crate: `mpfree-midi`

Bytes and volts only. It opens nothing, it has no unsafe code, and every part that can run on the audio thread or the MIDI reader thread is free of allocation after `new`, which `tests/no_alloc.rs` checks.

| Module | What it does |
|---|---|
| `port` | External ports. `DIN` is always id 0. A USB device gets its old id back when it is plugged in again, by name, so its track inputs, Learn mappings and clock settings follow it. There are 8 ports at most |
| `msg` | Channel messages as values, read from `mpfree_controller::midi::Parser` output. A note-on with velocity 0 is a note-off |
| `input` | `InputRouter`: every byte of every external port in, then track messages, `Seen` messages (for Learn, the monitor and the port lights) and clock bytes out. **Selected** mode (the default, as on Akai's MPC): any port and channel plays the track on screen. **Per-track** mode: each track has its own port, channel and transpose. A note-off always goes where its note-on went, even if the selection changed while the key was held. Unplugging a port ends its notes. Only the chosen clock source reaches the clock |
| `output` | `OutputRouter`: each MIDI track has a port and channel. A route change, stop or panic ends exactly the notes that are on, never 2048 blind note-offs. `OutQueue` holds one port's period, in frame order, with clock bytes first among messages at the same frame, and writes the bytes, with running status if asked. `DinPacer` models the 31250 bit/s wire: a note takes 0.96 ms and a 16-note chord about 15 ms. It reports how late each message and each clock pulse leaves |
| `learn` | `LearnMap`: arm a parameter (`mpfree_param::ParamAddr`), move a control, and it is learned. CC, note, pitch bend and channel pressure can be learned. Modes are absolute, relative (two's complement, offset 64, sign bit), toggle and momentary. Range and inversion are per mapping, and so is takeover: jump, or pickup, which waits until the control passes the value. `takes` says whether a message is Learn's (an armed press or move, a mapped control), so the app doesn't also play it; a key's release while armed is not, so a held note still ends. `observe` gives a parameter's value set elsewhere, for pickup, encoders and toggles. `Saved` is a mapping by device name, as a project keeps it (`MidiSetup::learn`, `learn = …` lines) |
| `cv` | `CvEngine`: 8 outputs. Each one is pitch (1 V/oct or Hz/V; last, low or high note priority; bend range), gate (with an optional retrigger gap), trigger (any note or one note, for drum modules), velocity, a CC, clock (any PPQN), run, reset, or fixed (the CV track's sliders). Pitch holds after the gate falls, so release tails stay in tune. Calibration (gain and offset) is per output. It emits a `CvChange` only when an output's voltage changes. **No range is assumed**: the hardware layer will clamp once the range is measured |
| `cv_wire` | The bytes Akai's app sends to set the jacks, as captured: the CC pair of each output on cable 1, channel 16 (`ccs`, `encode`, `decode`), Akai's pitch for a note (`akai_pitch_counts`), and counts from volts with the full-scale voltage as a parameter (`counts`), since it is not measured. Its tests hold every value the two captures saw. It sends nothing |

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
- every CV value the two captures saw, byte for byte, and the CC pair of each output;
- no allocation on the audio path.

## Steps

| Step | What | Touches the MPC | Waits on |
|---|---|---|---|
| M0 | The read-only `midi-cv`, `cv-scale` and `midi-out` captures above (all done 2026-11-12) | read-only (passive usbmon, stock 3.6) | done |
| M1 | `mpfree-midi`, host only | no | done: PR #177, merged 2026-11-12 (train 14, main `7ecb7e4`) |
| M2 | App wiring for input. The app reads the DIN In cable (from M0) and USB keyboards **for reading**, plays them on the armed track and logs them on the Dev tab's MIDI page (built, see [M2: keyboards in](#m2-keyboards-in)). The settings (ports, input mode, clock source) go in the mixer's I/O view, which waits on the FX and app threads | a device round on the test boot: opens MIDI inputs for reading | input built; device round and the I/O view to come |
| M3 | MIDI tracks out. A seq destination for MIDI out, and cable 2 out written with paced bytes from a writer thread that sleeps to each message's frame (sub-period timing; to be measured on the unit, aiming under Akai's 5 to 8 ms) | **state-changing, RAM only**: sends note bytes on cable 2 to the MIDI Out jack. Undo: power-cycle. No SysEx is ever sent there | built on the host behind `mpfree app --midi-out` (see [M3: tracks out](#m3-tracks-out)); the device round waits on the owner's go |
| M4 | Clock out (DIN, USB) from the transport, and clock in, with the transport following it | as M3 for out | clock out on the 5-pin jacks built on the host (see [M4: clock out](#m4-clock-out)); clock in wired to the time engine thread's follower (see [M4: clock in](#m4-clock-in)), read only on the MPC; the device round waits on the owner's go |
| M5 | MIDI Learn: the chain editor's Learn arms it, a control on a keyboard takes the knob, learned controls are the project's (`MidiSetup::learn`) and listed in the I/O view's MIDI section | as M2 (reads only) | built on the host (see [M5: MIDI Learn](#m5-midi-learn)); on the MPC it rides on the M2 round; the list on screen and keeping them with the project wait on the app thread wiring the I/O view and the `midi` part |
| M6 | CV out: a hardware layer that sends `cv_wire` CC pairs on cable 1 from `CvEngine`'s changes, then a first output test with a multimeter on one jack (known counts, read the volts, find which jack each output is), then a calibration page | **state-changing, RAM only**: sends control changes on cable 1 (channel 16, CC 0 to 7). Undo: power-cycle. No SysEx | CV 1 to 4 built on the host behind `mpfree app --cv-out` (see [M6: CV out](#m6-cv-out)); the meter round, CV 5 to 8 and tuning wait on the owner's go |
| Later | USB-B as a USB-MIDI device to a computer (the UDC gadget: rule zero, not before the boot work allows it); MIDI file import and export (the arp already writes MIDI files) | — | — |

## Defaults taken (the owner may change them)

1. Input goes to the selected track from any port and channel, as on Akai's MPC. Per-track listening is a mode.
2. MIDI Learn listens to external ports only. The Q-Links keep their own learn (`mpfree-mod`'s bindings).
3. A control moves one parameter: learning it again replaces the old mapping.
4. Learned CCs start as absolute with jump. Learned keys start as momentary.
5. Clock out is off and the clock source is none (internal) until set in the project's I/O view.
6. Running status on the DIN wire is up to the MPC's controller: MPFree hands it whole messages over USB (USB-MIDI packets carry no running status), so `DinPacer` estimates the wire time without it.

## A rule for the MIDI screens (the owner, 2026-11-12)

Akai's MIDI track has two settings with nearly the same name on the same screen, "MIDI Output Ch" and a "MIDI Out" routing, and both have to be set before a note leaves the jack. MPFree does not do that. Each track has **one** MIDI output setting, port and channel together, and no two settings on a screen share a name or look alike while doing different things. If a track would send nothing (no port, or a port that is unplugged), the field says so in place.

## What musicians ask for, and where Akai's MPC falls short (2026-11-12)

Forum posts are owners' reports, not tests. Each was read on 2026-11-12.

| Complaint about Akai's MPC | What owners report | MPFree's answer |
|---|---|---|
| The same note on two channels chokes | one channel's note cuts the other's; note-offs reach every channel ([MPC-Forums](https://www.mpc-forums.com/viewtopic.php?p=1860854)) | notes tracked per port, channel and note, in and out; a note-off goes only where its note-on went |
| Program changes get lost | external program changes not passed on or recorded; a track's own only sends after nudging the dial ([MPC-Forums](https://www.mpc-forums.com/viewtopic.php?p=1808235)) | program and bank are a track setting, sent at every start and every change, and an event that records and passes through |
| Clock while following | about 15 ms jitter, drift as leader, clock only while playing, no looper or audio recording while following ([MPC-Forums](https://www.mpc-forums.com/viewtopic.php?p=1737157), [Elektronauts](https://www.elektronauts.com/t/clock-sync-latency-and-jitter/256632?page=2)) | a follower that smooths tempo with every recorder still working; clock out per port, which can run while stopped; jitter measured and shown |
| No clock from the CV jacks | [MPC-Forums](https://www.mpc-forums.com/viewtopic.php?p=1782631), [Elektronauts](https://stage2.elektronauts.com/t/mimicking-clock-with-cv-mpc-one/127896) | any CV output can be clock (any PPQN), run or reset |
| MIDI Learn without ranges | a slider's top gives +6 dB, no mappings outside a project, mappings break when tracks move ([MPC-Forums](https://www.mpc-forums.com/viewtopic.php?p=1898047)) | a range and takeover per mapping with musical defaults; global and project mappings; a mapping follows its sound, not a track position |
| Learn that fails silently, and MIDI that breaks between updates | [MPC-Forums](https://www.mpc-forums.com/viewtopic.php?p=1890369) | Learn always shows what it caught and where it went; every routing case is a host test in CI; a MIDI log on the Dev tab |
| Two look-alike settings, both needed | the owner's own run (third capture above) | one output setting per track (see the rule above) |

The CV range is set by Akai's hardware (5 V at most, if the forum report holds). MPFree can show it, calibrate each output and offer Hz/V, but cannot raise it.

What musicians praise in other sequencers: clock they can trust as leader and follower, measured per port ([Elektronauts](https://www.elektronauts.com/t/clock-sync-latency-and-jitter/256632?page=2)); many ports with each lane routed freely, automation lanes to CC, pitch bend, aftertouch or CV, MIDI effects per track and MPE input ([Perfect Circuit, Hapax](https://www.perfectcircuit.com/signal/squarp-hapax-review)); CV outputs that can be LFOs or envelopes ([Perfect Circuit, OXI One](https://perfectcircuit.com/signal/oxi-one-sequencer-review)); USB host for several devices, CC lanes and microtuning through pitch bend ([Elektronauts](https://stage2.elektronauts.com/t/how-would-your-dream-sequencer-look-like/138255)).

## M2: keyboards in

Built 2026-11-12 in `rust/mpfree-app/src/midi_in.rs` and the app's `main.rs`. Read only: nothing is written to a MIDI port.

- **Cable 2 is music, not MPC controls.** Before M2 the app decoded cable 2's bytes with `mpc_one::decode`, as if they came from the MPC's own pads and buttons, so a keyboard on channel 1 pressed MPC buttons and one on channel 10 hit pads, and each clock tick printed a log line. Now `Ports::poll` hands cable 2's raw bytes to `MusicIn`, which parses them with `mpfree_midi::input::InputRouter` in Selected mode (any port, any channel).
- **USB keyboards.** A thread named `usb-midi-scan` lists the ALSA cards every 2 s, skips the MPC's own audio and controller cards, and opens the first rawmidi input of each new card for reading. Unplugging a keyboard lets go of its held keys.
- **Where keys go.** With a synth on the pads (`pad_layout` set), keys go by note to the armed track's arp engine through `App::key_down` and `App::key_up`, and share hold counts with the pads, so a note held on both ends when both let go. With a kit, keys whose note is one of `mpc_one::PAD_NOTES` become pad hits; other keys play nothing, and the first such key is logged once with the pad notes. Poly pressure reaches the synth.
- **The log.** Each channel message gets one line naming the port. Clock (F8) and active sensing (FE) are counted, not listed: the log says when clock starts and stops arriving. Start, continue, stop and reset are listed. SysEx is logged by length and ignored. The run's counts are printed at exit.

Not yet: CCs and pitch bend do nothing until learned (see [M5: MIDI Learn](#m5-midi-learn)), and program change waits for the track settings; clock-follow waits for M4; keys that don't land on a pad aren't recorded; only the first rawmidi input of a USB card is read. Not yet tried on the MPC: the read-only device round comes with the app thread's next live test.

## M3: tracks out

Built 2026-11-12 in `rust/mpfree-app/src/midi_out.rs`, on the time engine's `Pads::out` hook (PR #232). Host-tested only. **State-changing on the MPC**: note bytes leave the MIDI Out jack (cable 2), in RAM only, never SysEx. So the app opens the port only when started with `mpfree app --midi-out`, until the owner has run the device round.

- **Which tracks.** A track whose `TrackOut` names the 5-pin jacks (`DIN_NAME`) plays there and not on the pads. The pads still flash for its notes. A track set to a USB device plays on the pads for now, and its row says "sends nothing: no output to … yet". With MIDI out off (no `--midi-out`, or the port failed to open), every track plays on the pads and the MIDI section's banner says "MIDI out is off".
- **The hook.** The clips' events reach `SeqPads::out` first (`Pads::out`, #232). The arps' notes (the pads played live, a latched arp) reach `SeqPads::live`. For a track on MIDI Out, `live` sends the note with the pads' velocity curve, shows its pad and returns the pad to record, as it does for the pads. A panic's `AllOff` ends MIDI Out's notes and still stops the pads. Locks and lanes stay with the mixer.
- **Which notes.** A kit pad goes out as the MPC's own note for that pad (`mpc_one::PAD_NOTES`, pad 1 = C#2, note 37), as Akai's MPC sends it. A synth pad sends the notes the synth would play for it (`Synth::notes`, a chord too). An arp note with a synth pad layout is already a MIDI note. A pad's note-off and pressure go to the notes its note-on sent, even if the layout changed in between.
- **Bank and program** go out (CC 0, CC 32, then program change) when a track's output is set or changed, and again for every routed track when the song starts, ahead of that period's notes.
- **Ending notes.** `OutputRouter` counts the notes that are on per track. A changed channel or port ends them where they started; a panic and closing the port (`MidiOut::close`, at exit) send note-offs only for the notes that are on. If the app is killed or the MPC loses power, notes still on stay on in the other instrument: its own all-notes-off, or switching it off and on, ends them.
- **Timing.** The sound loop never writes to the port. `MidiOut::period_end` hands each message, in frame order, to the `midi-out` thread (`crate::port_writer`, shared with CV out) over a bounded queue (`WIRE_QUEUE`, 4096 messages, over a second of a full DIN wire), due at the loop's start time plus `MIDI_OUT_LEAD` plus its offset. `MIDI_OUT_LEAD` is the sound card's buffer, 3 periods of 128 frames at 44.1 kHz (8.7 ms), so the jack lines up with the sound. The thread sleeps until each message is due and writes it to the non-blocking port, waiting up to 500 ms for room before it drops and counts it. A message that finds the queue full is dropped and counted. A write error stops the thread; the loop logs it and lets the port go, and its tracks play on the pads again.
- **The log.** The Dev tab's MIDI page lists the first 64 messages sent ("+12.345 s MIDI Out: 99 25 64 = note on, ch 10, C#2, velocity 100"), then counts. At exit: messages written and dropped, how late the latest one left after its frame, and `DinPacer`'s estimate of the wire's own delay.
- **Sound path.** `sync`, `send` and `period_end` never allocate, wait or write: `rt_safety` plays a recorded clip on MIDI Out for 4,000 periods under the counting allocator.

Not yet: MIDI thru (`Built::thru`, its row says "thru does nothing yet"), USB devices as outputs, a per-track delay (a rig fact, for Prefs), CCs, pitch bend and pressure lanes from clips. The device round, with its exact steps and undo, goes to the owner first.

## M4: clock out

Built 2026-11-12 in `MidiOut::clock` (`rust/mpfree-app/src/midi_out.rs`), on the same `--midi-out` switch as M3, so it is state-changing in the same way and waits for the same device round.

- **The setting.** The jacks' "Clock out" in the I/O view's Sync section: Off (the default), While playing, or Always. USB devices as outputs come later.
- **From the transport.** Once a period, after `TimeHost::advance`, the app hands `TimeHost::block()` to `MidiOut::clock`, which drives `mpfree_midi_clock::ClockOut` in time order (a marker on a span's first frame comes before that span):
  - `Mark::Started { tick }`: Start (FA) at tick 0, else Song Position (F2) and Continue (FB) with pulses from the next sixteenth.
  - Playing and recording spans: a pulse (F8) on every 40-tick line (`Span::lines(PULSE_TICKS)`, 24 a quarter note), so pulses follow every tempo change.
  - `Mark::Located` and `Mark::Looped`: Stop, Song Position, Continue. `Mark::Rewound` (bar 1 while stopped): Song Position 0. `Mark::Stopped`: Stop (FC).
  - A count-in sends nothing. Its spans roll, but Start comes with `Started`, when playback begins.
  - Clock set while playing (or the port opened while playing) joins at the next sixteenth with Song Position and Continue.
  - Set to Off while playing: Stop. Closing the port (at exit) sends Stop too.
- **Always** also sends pulses while stopped, at the transport's tempo, from the moment it stops.
- **Order.** The clock is queued before the period's notes, and the queue is sorted by frame only (`OutQueue::sort_by_frame`), keeping the order pushed: Song Position must come before Continue, which `OutQueue::sort` (real-time bytes first) would swap.
- **Tests.** Start and 48 pulses a second at 120 BPM while playing, nothing while stopped; Always; a locate; a count-in; turned on and off while playing; and `rt_safety` plays a clip on MIDI Out with clock on for 4,000 periods with no allocation.

## M4: clock in

Built 2026-11-12 in `MusicIn` (`rust/mpfree-app/src/midi_in.rs`) and the app's loop, on the time engine thread's follower (`rust/mpfree-app/src/time/follow.rs`, described in [midi-clock.md](midi-clock.md)). It only reads, from the ports M2 already reads, so it needs no round of its own.

- **The setting.** The I/O view's Sync section: "Tempo from" MPFree or a port (by name, so a USB device plugged in later is found), and "Start and stop" and "Position" (both on by default). `MusicIn::sync` turns them into `time::Follow` on each generation of the model, and sets `Built::clock_in`, so the Sync section's rows no longer say following isn't built.
- **The leader's bytes.** `MusicIn::bytes` keeps the followed port's clock (F8), Start, Continue, Stop and Song Position with its two data bytes, in a 64-byte buffer for the period; anything else is left out, and a status byte that cuts a Song Position short is kept so the follower drops it too. Past 64 bytes in a period (more than any clock sends) bytes are counted at exit.
- **Each period**, before `TimeHost::advance`: `MusicIn::leader_clock` hands the options and the bytes to `TimeHost::follow` and `TimeHost::clock_in`, at `TimeHost::next_frame()` (read at the period's top, so up to a period late, about half a period on average). `follow_tempo` with the app's tempo is ignored while the clock arrives.
- **The tempo.** After `advance`, `TimeHost::followed_bpm()` becomes the app's tempo (`App::set_tempo`, rounded to 0.1 BPM, without its log line) when it has moved more than 0.08 BPM from it, so the tempo chip, Note Repeat, the arps and tempo-synced effects follow the leader without flicker. When the clock stops arriving the follower lets go and the transport keeps the last tempo.
- **The log.** "clock arriving …; MPFree follows it" (or "not followed: the tempo isn't from this port") once per port; the time engine's status line counts jumps to the leader's position and lost clocks.
- **Sound path.** No allocation: the bytes go through a fixed buffer, and `MusicIn::bytes` borrows the port's name for its log lines instead of building one.
- **Tests.** `the_followed_port_hands_its_clock_on` (what is kept, from which port, with which options) and `a_clock_on_the_jacks_sets_the_tempo_and_starts_the_song` (a 100 BPM clock with Start through `MusicIn` and the follower, as the loop runs it: 100 BPM within 0.1, and playing). The follower's own timing tests are the time engine's.
- **Not yet.** Thru; following from USB-B (a computer); the setting on the MPC's screen, which comes with the I/O view.

## M5: MIDI Learn

Built 2026-11-12. Host-tested only. On the MPC it only reads (the same inputs as M2), so it needs no round of its own.

- **Arming.** The chain editor's existing Learn button (mod thread code, `mpfree-chain-ui`) arms both learns: `ToUi::ChainLearn(addr)` also calls `MusicIn::arm(addr)`, and its armed text now reads "Turn a Q-Link or a MIDI control". A Q-Link turned first ends it as before (`FromUi::ChainLearned` disarms MIDI Learn); a MIDI control taken first sends `ToUi::ChainLearnDone`, which ends the editor's learn (`ChainPage::cancel_learn`). Back or closing the editor disarms it (`Panels::learning`).
- **Which ports learn.** A port whose input is "Learn only" or "Both" (the default). "Plays tracks" never learns, and its learned controls do nothing; the I/O view says so on the row.
- **What Learn takes doesn't play.** `MusicIn::bytes` hands each track message to `LearnMap::takes` first: an armed press or move, and every message of a learned control (a learned key's release too), go to Learn and not to the armed track. A key's release while armed still reaches the track, so a note held while arming ends.
- **Defaults.** A key learns as momentary ("Button: while held"), anything else as absolute ("Knob or fader"), over the whole range with jump takeover. Learning a control again (same port, channel and source) replaces its old mapping (`MidiSetup::learned`); a project keeps at most `MAX_LEARNED` (256), and one past that is counted at exit.
- **The project's.** A learned control is pushed to `MidiSetup::learn` at the next `sync` (by device name, so it finds the device again under another port number), which bumps the model's generation; `sync` rebuilds the `LearnMap` from the setup on every generation change, so the I/O view's "Works as" and Forget take effect at once. Until the app adds the `midi` part to the project store, learned controls last for the run.
- **Setting the value.** A learned control's 0 to 1 becomes the parameter's own value with `ParamInfo::from_unit` (a macro takes the 0 to 1 as it is), set with `Panels::set_param`, which is how a chain editor knob sets it (the mod engine's `hand` first, else the mixer) and refreshes the open panel. A parameter the app can't set from a control yet (a synth's: `modmix::describe` doesn't know it) is counted and logged once.
- **Where the value stands.** Every 50 ms `MusicIn::observe_learned` reads each learned parameter (`modmix::describe`) and, when something other than the control moved it, tells `LearnMap::observe`, so an endless encoder steps on from a value set on the screen and a button's next press flips from it. For 250 ms after a learned control set a parameter, and while the parameter holds what the control set (as the parameter rounds it, so a stepped parameter keeps an encoder's part steps), it is left alone.
- **On screen.** `mpfree-midi-ui`'s MIDI section ends with a "MIDI Learn" part: one row per learned control ("CC 74 · ch 1 · Keystep 37", `control_text`), what it moves (`Live::learn_names`, which the app fills from `modpanels::label` on each generation), **Works as** (the six modes) and a tap-only **Forget** (the wheel, - and + and the Q-Links skip it). A row says "not plugged in: moves …" or "does nothing: its port only plays tracks" when that is so.
- **The log.** "+1.234 s Keystep 37: MIDI Learn: CC 74 · ch 1 · Keystep 37 takes t1.fx3.2" on the Dev tab's MIDI page.
- **Sound path.** Learn runs in the sound loop with the rest of `MusicIn`: no waiting (the model is only try_locked), and no allocation once a control's parameter has been seen (`learn_sets` has room for every learned control from the start).
- **Not yet.** Learning from any value on the screen (only the chain editor's knobs arm it); a range per control and pickup on screen (the engine has both); a synth's parameters; global mappings outside a project.

## M6: CV out

Built 2026-11-12 in `rust/mpfree-app/src/cv_out.rs`. Host-tested only. **State-changing on the MPC**: control changes on the controller's cable 1 ("Private"), channel 16, the CC pairs Akai's app was seen sending (`mpfree_midi::cv_wire`), in RAM only, never SysEx. So the app opens CV out only when started with `mpfree app --cv-out`, until the owner has run a device round with a meter.

- **Which outputs.** Only CV 1 to 4 (`CHECKED_OUTS`, CC 0 to 7), whose pairs were seen in the owner's captures. CV 5 to 8 would be CC 8 to 15 by the pattern, but nobody has seen them, so they are never sent: the engine still follows them, and their rows say "sends nothing yet: not checked on the MPC". `Built::cv_outs` tells the CV section how many outputs this run drives (0 with CV out off, and every row with a job says "sends nothing: CV out is off").
- **Volts.** A change's volts become a 14-bit value with `cv_wire::counts`, taking full scale as 5 V (`MAX_VOLTS`). That is INFERENCE from Akai's own pitch values (60 semitones across the 14 bits): no meter has been on a jack. MPFree rounds where Akai's app truncates, so a note can be one count higher (C3: Akai 3276, MPFree 3277; a third of a millivolt if full scale is 5 V). Per-output tuning (`Calibration`) is in the engine but has no screen yet: it is a rig fact for Prefs, after the meter round.
- **What drives it.** The same hooks as MIDI Out, but CV only follows: a track's events still play wherever the track plays. `SeqPads::out` hands every clip event to `CvOut::send` before MIDI Out sees it; `SeqPads::live` hands every arp note on (with the pads' velocity curve). Notes are read as MIDI Out reads them (`Chords`, shared): a kit pad is the MPC's own pad note, a synth pad its layout's notes. A panic lets every gate fall. `CvOut::clock` takes the transport's period: Started (from tick 0: a reset pulse), Stopped, and each clock output's lines (`Span::lines(PPQ / ppqn)`) while playing or recording, never in a count-in. Opened while the song plays, run joins at once.
- **Gates that touch.** A gate that falls and rises on the same frame (one note ending as the next starts, which is how a sequence's notes meet) stays low for `GATE_GAP_MS` (2 ms) before it rises, so the module's envelope starts again. The pitch moves at once. 2 ms is a choice, longer than USB's 1 ms frames, not yet tried on a module.
- **Timing.** As MIDI Out: `CvOut::period_end` sorts the period's changes by frame (keeping their order within a frame), sends only the last change of each output on any one frame, and hands each change's two control changes, as one 6-byte write, to the `cv-out` thread (`crate::port_writer`, shared with MIDI Out), due at the loop's start plus `MIDI_OUT_LEAD` plus its offset. A 5 ms trigger's fall leaves about 5 ms after its rise. Past 120 changes in a period, each output's last value is sent and the period is counted.
- **The lights.** The app's button lights are 3-byte control changes on the same Private port, written by the sound loop. While CV out is open they go through its writer (`CvOut::light`) instead, so a light's bytes can never land inside a CV pair if the kernel's buffer takes only part of a write.
- **Open and close.** The first period after it opens, CV 1 to 4 go to their job's voltage (0 V with no job). Closing (at exit) sets all four to 0 V, so no gate is left up. If the app is killed or the MPC loses power, the jacks keep their last value until the controller resets.
- **The log.** The Dev tab's MIDI page lists the first 64 writes ("+12.345 s CV out: BF 00 19 BF 01 4D = CV 1 = 3277 (1.000 V if full scale is 5 V)"), then counts. At exit: writes, drops, and how late the latest one left.
- **Sound path.** `sync`, `send`, `clock` and `period_end` never allocate, wait or write: `rt_safety` plays a recorded clip into pitch, gate, a clock and run for 4,000 periods under the counting allocator.

The device round (the owner's go first, with exact steps and undo): with nothing plugged into the jacks, start `mpfree app --cv-out` with CV 1 set to Fixed at known volts, read each jack's tip and ring with a multimeter to learn which output is which and the volts at full scale; then a pitch and gate on a module. Until then, CV 5 to 8 and tuning wait.

## A project's settings and the I/O view's sections

Built 2026-11-12, not yet wired into the app (that waits on the app thread, which owns `gui.rs`):

- **`mpfree_midi::setup::MidiSetup`**: a project's MIDI, sync and CV settings. Ports are named, not numbered (`DIN_NAME`, "MIDI", for the 5-pin jacks), so a project names "Digitakt" and the app finds it when it is plugged in. Each track has one output setting, `TrackOut` (port and channel together, with program and bank). Each port has its input (Plays tracks, Learn only, Both; Both by default), clock out (Off, While playing, Always; Off by default) and thru (off by default). The tempo is MPFree's own or a port's clock, with follow start/stop and follow position. Each of the 8 CV outputs has a `CvAssign`. The text is `key = value` lines after `mpfree-midi 1`, only what differs from the defaults; a line that can't be read is skipped and named in a warning, never dropped silently.
- **`rust/mpfree-midi-ui`**: the MIDI, Sync and CV sections of the mixer's I/O view, through `mpfree_mixer_ui::IoSection`. They share an `IoModel`: the setup, `Live` facts the app fills in (ports plugged in and when each last received something, track names, clock arriving, CV volts) and `Built`, which says which parts of MIDI this build has. A setting whose part isn't working in this run is kept and says so on its own row ("sends nothing: MIDI out is off", "thru does nothing yet", "sends nothing: no output to Keystep 37 yet"), as does one for a device that isn't plugged in, following the owner's rule. A tap on a setting changes it (a choice steps, "Plays on" opens a picker with the port and the channel on one page); the wheel selects a row, - and + step the selected setting, and the Q-Links take the row's settings. `MidiPart` is the project's `midi` part (`parts/midi.txt`); lines it couldn't read are counted on the MIDI section and named in the log.
- **Previews**: `cargo run -p mpfree-midi-ui --bin mpfree-midi-preview -- --out DIR` draws the three sections inside the mixer's real I/O view, before and after the parts of MIDI are built.

## The screens (proposed, 2026-11-12)

The MIDI, Sync and CV sections above are built; the rest below is a proposal, drawn at 1280 × 800 in [MPFree MIDI screens](https://claude.ai/artifact/5ygP92smCQVhu31qq2kfYZ).

Where they live (the owner, 2026-11-12): MIDI and sync are **project** settings, not Prefs, since they change from project to project. A project's routing (audio, MIDI, sync and CV) is an I/O view inside the mixer, opened by a new icon beside the mixer icon. The FX thread owns the mixer and plans I/O as a fifth mixer view beside Tracks, Pads, Inputs and Submixes, with a section list (Audio, MIDI, Sync, CV); the [layout spec](layout-spec.md) names it the I/O icon, just left of the mixer icon at the top bar's right edge, and a tap on the tempo chip opens the same view at Sync (docs PR #221). The MIDI, Sync and CV sections are panels built against a small section interface that the mixer provides, so neither side edits the other's code. Facts about the rig rather than the project (CV tuning, port names, port delays) stay in Prefs. The mockups were drawn before this choice and still show a Prefs row.

| Screen | Where it opens | What is on it |
|---|---|---|
| MIDI and sync | the mixer's I/O view | tempo source, follow start/stop/position, what keys play, links to ports, Learn mappings and CV outputs, every note off |
| A port | I/O view › Ports | what its input does, clock out (off, while playing, always), start/stop out, offset, thru, ignored kinds; a device keeps its settings by name while unplugged. Its name and delay are rig facts in Prefs |
| CV outputs | I/O view › CV outputs | the 8 outputs with job, source, scale and a live value (built, see [M6: CV out](#m6-cv-out)); each output's tuning is a rig fact for Prefs and waits for the meter round |
| A track's output | Arrange, the track's settings | one "Plays on" field (port and channel together), program, bank, what pad pressure sends, delay; "Sends nothing: no port" under the track name when true |
| MIDI Learn | a value on any page | Learn, range, direction, takeover, where it is kept; a strip while waiting; STOP cancels. Built so far: the chain editor's Learn (see [M5: MIDI Learn](#m5-midi-learn)) |
| Learn mappings | I/O view › MIDI › MIDI Learn | every mapping with control, target, range and scope; greyed when the device is unplugged. Built: control, target, how it works, Forget, and a note when it does nothing |
| MIDI log | the Dev tab's MIDI page | in and out per port, filters by port and kind, clock timing and counts |

The six decisions in the design doc's "Decisions for Jay" table are settled: where the settings live changed as above, and the other five (clock out per port, Learn scopes, port input names, live note timing, thru default) were agreed as recommended.
