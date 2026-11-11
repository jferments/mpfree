# Looper: audio loops on the transport

**Status:** engine crate (roadmap step T4), 2026-11-11, on main. Round 6 wires it into the pad player with a Looper tab and the line input ([below](#in-the-pad-player-round-6)). Nothing here has run on the MPC yet, and MPFree has never opened the MPC One's capture device (see [Line input](#line-input-on-the-mpc-one) below).

One stereo loop on the transport, recorded from the line input or the pad player's own sound. It follows the transport's bars, so it stays in time with the clips, the arp and the metronome.

## How it plays

- **The first pass** starts on the next bar line after Record, and ends after the set number of bars (2 by default), or, with free length, on the bar line after Record is pressed again. Its place in the song is a whole number of bars from `origin`.
- **Playback follows the song position.** The loop's position is the song position's place in the loop: `(tick - origin) mod len_ticks`, scaled to frames and read between frames. It stays on the bars through stops, locates and transport loops. At the tempo it was recorded at it plays sample for sample; at another tempo it plays faster or slower, like tape (no time-stretch yet).
- **Overdubs** are layers on top of the first pass, from the moment Overdub is pressed until it is pressed again, over as many passes as you like. Overdub pressed during the first pass starts the overdub exactly where the first pass ends. Undo fades out and removes the newest layer; Redo brings it back, until a new overdub starts.
- **Silent while stopped or counting in.** Overdub pressed while stopped waits for Play. Stopping ends an overdub, with a fade.

## No clicks

| Where | What the looper does |
|---|---|
| The loop point | The last 256 frames crossfade into the input from just before the first pass, which flows on into the first frame. A sine that does not fit the loop would jump about 0.13 at a raw splice; the test holds it under 0.008. |
| Punch in and out | The overdub writer fades in and out over 256 frames (`FADE`, about 5.8 ms). After a full pass it adds, so the fade-in and the next pass join without a seam. An overdub that takes over from the first pass fades in over 32 frames (`HANDOFF_FADE`), because the input runs on without a break there; that keeps the downbeat's attack. |
| Start, stop, mute, clear | The loop's gain ramps over 256 frames. |
| Undo and redo | The newest layer's gain ramps over 256 frames. |
| A jump in the song position | A locate, a transport loop that is not a whole number of loops, or a restart right after a stop: the old place and the new one crossfade over 256 frames. |
| Commands during a fade | Record, Overdub, Undo and Redo that would cut a fade short wait for it (at most 256 frames), in order. Pressing Undo twice removes two layers. |
| Stale audio | A reused layer slot only plays the frames the new overdub wrote. |

## Latency

The player hears the loop late (the output buffer) and their playing reaches the input late (the input buffer and the converters). `SetLatency(frames)` makes up for both: input that arrives `latency` frames after a moment belongs to that moment, so it is written that much earlier in the loop. The demo checks that the first pass lines up with what was played, to the frame, with 640 frames of latency.

On the MPC One the latency is not measured yet. The playback buffer is 384 frames and the capture buffer was 256 frames while Akai's service ran; the converters add an unknown amount. It will be measured with a cable from an output to an input, and that needs the owner's go (see below).

## The API

```rust
let mut looper = Looper::new(44_100, 60);   // room for 60 s of stereo: the only allocation
looper.command(Command::SetLatency(640));
looper.command(Command::Record);            // armed: records from the next bar line
// in the audio callback, after the transport:
looper.process(&block, &line_input, &mut mix); // adds the loop to the mix
let s = looper.status();                    // phase, layers, redo, position, ...
let wave = looper.layer(0);                 // a layer's audio, for the waveform
```

- `Command`: `Record`, `Overdub`, `Undo`, `Redo`, `Clear`, `Mute(bool)`, `SetBars(Option<u32>)`, `SetLatency(u32)`, `SetLevel(f32)` (0 to 2). Commands apply at the start of the next period.
- `Phase`: `Empty`, `Armed`, `Recording`, `Playing`, `Overdubbing`.
- `Stats` counts what went wrong, never silently: commands that did not fit the queue, overdubs refused because the memory was full, first passes thrown away (the transport stopped or jumped during the first pass, or it outgrew the memory).
- The memory is shared: a 2-bar loop at 120 BPM is 4 s, so 60 s holds the first pass and 14 overdubs. At most 16 layers.
- `process` and `command` never allocate, lock or panic.

## Tests

`cargo test -p mpfree-looper` (workstation, no device):

- `tests/looper.rs` (18 tests, against a real transport): the first pass starts on the bar and plays back sample for sample; latency is made up for; the loop point does not click; overdubs stack, and undo and redo fade; Undo twice removes two layers; punches fade, and old audio in a reused slot stays silent; stop fades out and play fades back in; a locate crossfades; nothing plays during a count-in; free length ends on the bar after Record; a transport loop ends the first pass and the loop follows it; stopping or jumping during the first pass throws it away; overdubs are refused when the memory is full; Overdub pressed while stopped waits for Play; Clear then Record arms again; mute and level fade and scale; Overdub during the first pass takes over where it ends; and a busy session (latency, a transport loop, a locate, a tempo change, stop and play, overdubs, undo, redo, clear) gives bit-identical output at periods of 64 to 1024 frames.
- `tests/no_alloc.rs`: a counting allocator watches `command`, `process`, `status`, `stats` and `layer` through a busy session at periods of 1 to 4096 frames, with a full memory: no allocation or free.

## Listening

```text
cargo run --release -p mpfree-looper --example render -- [--period N] [--out FILE]
```

Writes `~/mpfree-work/looper-demo.wav` (45.5 s). A player plays into the line input at 96 BPM, 640 frames late. One bar of count-in, then a two-bar loop builds up: a distorted saw bass (the first pass), crushed drums (overdub), a saturated pulse lead (overdub), Undo (the lead fades out), Redo (it is back), distorted power-chord stabs on the off-beats (overdub), then a stop under a final live hit. The distortion is in the sounds, on purpose; the master is plain gain, peak -2.1 dBFS, no clipped samples. It prints each press and phase, and exits with an error if the first pass does not line up or anything clips.

## Line input on the MPC One

Read-only facts so far (from the owner's device reports while Akai's `acvs` service ran):

- The audio device is ALSA card 1, device 0, stereo S32_LE at 44.1 kHz, 128-frame periods. Playback has a 384-frame buffer; capture had a 256-frame buffer.
- **Not verified:** which physical jacks the two capture channels are (line in L/R is the likely reading, not checked), and that playback channels 1 and 2 are the main L/R outputs (inferred from MPFree's sound coming out of them, not traced). Akai's routing, gain staging and any mixer controls on the capture path are unknown.

The plan, with rule zero (nothing state-changing on the device without the owner's go):

1. A read-only probe: the capture device's info in `/proc/asound`, its device node, and its hardware limits, with nothing opened for capture.
2. Line input off by default in the pad player, behind a Prefs row ("Line input: off/on"), never saved as on across boots. Opening capture is a state change: it is a separate step that waits for the owner's go, with the exact change, the undo (switch it off, or quit) and what a partial failure looks like.
3. Then an input meter, then the loop.

## In the pad player (round 6)

[`looping.rs`](../../rust/mpfree-hello/src/looping.rs) runs the looper in the pad player's audio loop, after the master level, next to the click, and [`looper_page.rs`](../../rust/mpfree-hello/src/looper_page.rs) is its **Looper** tab ([the guide](../guides/hello-mpc.md#round-6-the-looper-the-synth-and-the-sampler-in-one-pad-player)).

- **What it records.** By default the pads and the synth, after the master level and before the click and the loop, so nothing on the MPC is opened and no latency needs making up. With the line input on, the line input, with the latency estimate below.
- **The line input.** `alsa.rs` opens the ACVA card's capture device without waiting (`O_NONBLOCK`) at the settings Akai's app used (stereo S32_LE, 44.1 kHz, 128-frame periods, a 256-frame buffer), starts it, and reads whatever has arrived each period into a FIFO of 8 periods. A period with too little input gets silence and is counted (after the first 4); more than 3 periods waiting are dropped to 2, counted; an overrun prepares and starts it again, counted; more than 3 read errors in a row close it. It is off at every start; the row comes back Off whenever it closes.
- **Monitor** adds the line input to the outputs (about 9 ms late at 3 periods).
- **Latency estimate**: the output buffer plus one input period, `(periods + 1) × 128` frames (512 at 3 periods). The converters add an unknown amount: measure it with a cable from an output to the input and set it in the Looper options.
- **Logging**: a `looper:` line for each press and change (with the song position), `line input:` lines when it opens, closes or fails, and every 30 s `status: looper: ...` with the render time.
- **Tests**: `cargo test -p mpfree-hello looping` (the FIFO, the pads recorded to the frame, a missing device, and no allocation in a steady period) and `looper_page`.

## Next

- The `line` step on the MPC (round 6, after the owner's go): does Akai's driver record on the test build, which jack is L and which R, and the input's level.
- Measure the round-trip latency on the MPC One.
- Time-stretch, so a loop keeps its pitch at another tempo; more than one loop; saving loops with the project.
