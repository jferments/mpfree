# Looper: audio loops on the transport

**Status:** engine crate (roadmap step T4), 2026-11-11, on main. The pad player runs it, with a Looper tab and the line input ([below](#in-the-pad-player-round-6)), and round 7 (cc7f06d) ran the one-loop version on the MPC One, from the pads and from the line input (see [Line input](#line-input-on-the-mpc-one) below). Several loops, per-layer mixing and the faster playback described here have not run on the device yet.

Up to 8 stereo loops on the transport, recorded from the line input or the pad player's own sound, playing together and sharing one memory. Each follows the transport's bars, so it stays in time with the clips, the arp and the metronome.

## How it plays

- **The first pass** starts on the next bar line after Record, and ends after the set number of bars (2 by default), or, with free length, on the bar line after Record is pressed again. Its place in the song is a whole number of bars from `origin`.
- **Playback follows the song position.** The loop's position is the song position's place in the loop: `(tick - origin) mod len_ticks`, scaled to frames and read between frames. It stays on the bars through stops, locates and transport loops. At the tempo it was recorded at it plays sample for sample; at another tempo it plays faster or slower, like tape (no time-stretch yet).
- **Overdubs** are layers on top of the first pass, from the moment Overdub is pressed until it is pressed again, over as many passes as you like. Overdub pressed during the first pass starts the overdub exactly where the first pass ends. Undo fades out and removes the newest layer; Redo brings it back, until a new overdub starts.
- **Each layer has its own mix.** Level (0 to 2), pan (a balance, -1 to 1), mute and solo. While any layer is soloed, only soloed layers play.
- **Any layer can be deleted**, the first pass too; the last layer stays (Clear removes the loop). A delete fades the layer out, and Undo brings it back in its place. Undo and Redo walk one history of overdubs and deletes, newest first.
- **Layer ids.** A layer keeps its id while it exists, whatever comes and goes around it, so a screen can name it safely.
- **Silent while stopped or counting in.** Overdub pressed while stopped waits for Play. Stopping ends an overdub, with a fade.

## Several loops

- **8 loops** (`MAX_LOOPS`), each with its own first pass, length, origin, layers, history, level and mute. They all play at once, each on its own bars: a 1-bar loop and a 3-bar loop recorded later both stay on the song's bar lines.
- **The selected loop takes the commands.** `select_loop(n)` is queued with the other commands, in order, so `select_loop(2)` then `Record` arms loop 3 and nothing else. A selection waits behind a command that is waiting for a fade, so no command reaches the wrong loop. `status()`, `layer()` and `layer_info()` show the selected loop; `loop_status(n)`, `loop_layer(n, k)` and `loop_layer_info(n, k)` show any loop. A loop number past the last is counted in `Stats::no_layer`, and the selection stays.
- **One recording at a time.** The line input goes to one loop: when a loop arms, starts an overdub or waits to start one, the others stop recording. An overdub punches out (with its fade); a first pass ends on its next bar line, so a free-length pass keeps what was played; an armed loop disarms. Arming a loop while another records its first pass hands over on the bar line: the first pass ends where the next one starts. Starting an overdub on one loop while another records its first pass lets that pass run to its bar line, so both take the input until then.
- **One memory.** All loops share the memory `Looper::new` asks for, in blocks of 1024 frames (`BLOCK`, 8 KiB of stereo). A layer holds whole blocks: a first pass takes them as it records, an overdub takes a whole pass's worth when it starts. Clear gives a loop's blocks back once it has faded out, and so do Undo and Redo once what they could bring back is gone. The pool is a little larger than the seconds asked for: one block per layer of one loop more, so whole layers fill what the seconds promise.
- **The gauge.** `memory()` gives `used` and `capacity` in frames for all loops together; `Status::used` is what one loop holds, and `Status::capacity` is the shared total. A first pass that outgrows the free memory is thrown away and counted in `Stats::aborted`; an overdub that does not fit is refused and counted in `Stats::refused`. Neither affects the other loops.

## No clicks

| Where | What the looper does |
|---|---|
| The loop point | The last 256 frames crossfade into the input from just before the first pass, which flows on into the first frame. A sine that does not fit the loop would jump about 0.13 at a raw splice; the test holds it under 0.008. |
| Punch in and out | The overdub writer fades in and out over 256 frames (`FADE`, about 5.8 ms). After a full pass it adds, so the fade-in and the next pass join without a seam. An overdub that takes over from the first pass fades in over 32 frames (`HANDOFF_FADE`), because the input runs on without a break there; that keeps the downbeat's attack. |
| Start, stop, mute, clear | The loop's gain ramps over 256 frames. |
| Undo, redo and delete | The layer's gain ramps over 256 frames. |
| A layer's level, pan, mute and solo | Its left and right gains move one 256th of full scale a frame toward the settings. |
| A jump in the song position | A locate, a transport loop that is not a whole number of loops, or a restart right after a stop: the old place and the new one crossfade over 256 frames. |
| Commands during a fade | Record, Overdub, Undo, Redo and Delete that would cut a fade short wait for it (at most 256 frames), in order. Pressing Undo twice removes two layers. A new overdub waits while the last one's tail is written (the latency, then 256 frames), so that tail still fades out. |
| Stale audio | A reused layer slot only plays the frames the new overdub wrote. |

## Latency

The player hears the loop late (the output buffer) and their playing reaches the input late (the input buffer and the converters). `SetLatency(frames)` makes up for both: input that arrives `latency` frames after a moment belongs to that moment, so it is written that much earlier in the loop. The demo checks that the first pass lines up with what was played, to the frame, with 640 frames of latency.

On the MPC One the latency is not measured yet. The playback buffer is 384 frames and the capture buffer was 256 frames while Akai's service ran; the converters add an unknown amount. It will be measured with a cable from an output to an input, and that needs the owner's go (see below).

## The API

```rust
let mut looper = Looper::new(44_100, 60);   // 60 s of stereo for all loops: the only allocation
looper.command(Command::SetLatency(640));   // for every loop
looper.command(Command::Record);            // loop 1 armed: records from the next bar line
// in the audio callback, after the transport:
looper.process(&block, &line_input, &mut mix); // adds every loop to the mix
looper.select_loop(1);                      // the commands after this go to loop 2
looper.command(Command::Record);
let s = looper.status();                    // the selected loop: phase, layers, redo, position, memory, ...
let all = looper.memory();                  // the gauge: used and capacity, all loops
let one = looper.loop_status(0);            // any loop
let wave = looper.layer(0);                 // a layer's audio, in play order, for the waveform: wave[2 * frame + channel]
let info = looper.layer_info(1);            // its id, level, pan, mute, solo, audible, recording
looper.layer_command(info.unwrap().id, LayerCommand::Mute(true));
```

- `Command`: `Record`, `Overdub`, `Undo`, `Redo`, `Clear`, `Mute(bool)`, `SetBars(Option<u32>)`, `SetLatency(u32)`, `SetLevel(f32)` (0 to 2). Commands apply at the start of the next period.
- `LayerCommand`, sent with a layer's id: `Level(f32)` (0 to 2), `Pan(f32)` (-1 to 1), `Mute(bool)`, `Solo(bool)`, `Delete`. Deleting the overdub being recorded throws it away, as Undo does. A command for a layer that is not there is counted in `Stats::no_layer`.
- `LayerInfo` (`layer_info(k)`, in play order, the first pass first): `id`, `level`, `pan`, `muted`, `solo`, `audible` (not muted, not left out by a solo) and `recording`.
- `Phase`: `Empty`, `Armed`, `Recording`, `Playing`, `Overdubbing`.
- `layer(k)` and `loop_layer(n, k)` give a `LayerView`: index it like the slice it used to be (`wave[2 * frame + channel]`, `wave.len()` samples). Frames an overdub did not reach read as silence, so a waveform never shows stale audio from a reused slot.
- `Stats` counts what went wrong, never silently: commands that did not fit the queue, overdubs refused because the memory was full, first passes thrown away (the transport stopped or jumped during the first pass, or it outgrew the memory).
- The memory is shared by every loop and layer: a 2-bar loop at 120 BPM is 4 s, so 60 s holds a first pass and 14 overdubs, or 8 loops of a first pass and two overdubs at 2 s each. At most 16 layers a loop. `Status::used` is the memory a loop holds, in whole blocks: its layers and the ones Undo or Redo can bring back. A deleted layer keeps its memory while Undo can bring it back; when the memory is full, a new overdub takes the oldest deleted layer's memory, and the history up to that delete is forgotten.
- `process`, `command`, `select_loop` and `layer_command` never allocate, lock or panic.

## Cost

Measured with `mpfree-bench --filter looper` on the workstation (128-frame periods at 44.1 kHz, 2902 µs; the looper's budget is 3 % of a period, 87 µs, judged on the worst period on the MPC):

| Case | Before this change (mean / p99) | Now (mean / p99) |
|---|---|---|
| One 2-bar loop, layer after layer overdubbed | 11 to 15 µs / 31 to 45 µs | 7 to 9 µs / 16 to 23 µs |
| 8 one-bar loops of 3 layers, all playing, an overdub on one after another | about 100 µs / 165 to 210 µs (multi-loop build, before the speed-ups) | 29 to 34 µs / 53 to 74 µs (1.8 to 2.5 %) |

The workstation is several times faster than the MPC One, so these are for comparing, not for the budget: the device numbers come from the pad player's status lines and from `mpfree-bench` run on the MPC. Round 7 on the device (the one-loop version before this change): with the pads and three layers, looper mean 1.6 %, p99 3.1 %, worst 5.6 % of a period; with the line input, mean about 1.3 %, p99 4.0 %, worst 10.6 %. Both are over the budget on the worst period, which is why playback got cheaper first.

What made it cheaper, with the output unchanged to the bit (the demo's hash and the period-size test):

- **A still loop runs layer by layer.** While a loop plays at full gain with no writer, fade, mix change or jump, `process` works out each frame's position for a run of up to 128 frames, then adds each layer over the whole run. The sums happen in the same order as frame by frame, so the bits are the same. Anything that moves (a first pass, an overdub, a fade, a mute, a jump at a span's start) runs frame by frame as before.
- **Runs.** A period is cut into runs at bar lines and transport markers, so the loops run one after another inside a run, and each frame's song position is worked out once for all loops.
- **No `fmod` per frame.** The loop's position is `(tick - origin) mod length`; `f64::rem_euclid` calls `fmod`, a loop of integer steps in software, which was an eighth of the looper's time with 8 loops. `rem_ticks` gives the same bits with one division (tested against `rem_euclid` on a million values, on x86-64 and ARMv7).
- **A still mix costs nothing.** Each layer's gains are only stepped while one of them is moving.

## Tests

`cargo test -p mpfree-looper` (workstation, no device):

- `src/lib.rs`: `rem_ticks` against `rem_euclid`, bit for bit, on a million song positions and loop lengths.
- `tests/looper.rs` (27 tests, against a real transport): the first pass starts on the bar and plays back sample for sample; latency is made up for; the loop point does not click; overdubs stack, and undo and redo fade; Undo twice removes two layers; punches fade, and old audio in a reused slot stays silent; stop fades out and play fades back in; a locate crossfades; nothing plays during a count-in; free length ends on the bar after Record; a transport loop ends the first pass and the loop follows it; stopping or jumping during the first pass throws it away; overdubs are refused when the memory is full; Overdub pressed while stopped waits for Play; Clear then Record arms again; mute and level fade and scale; Overdub during the first pass takes over where it ends; each layer's level, pan, mute and solo, without clicks, and NaN or out-of-range values; a deleted layer fades out and comes back with Undo in its place, in one history with overdubs; deleting the overdub being recorded throws it away like Undo; a new overdub in a full memory takes the oldest delete's memory; a layer's waveform shows only what its overdub wrote; several loops play together, each in time, and commands go to the selected one; one loop records at a time (an overdub punches out, an armed loop disarms, a first pass ends on the bar line where the next one starts); the loops share one memory and Clear gives it back; a first pass that outgrows the memory is thrown away while the other loops play on; and two busy sessions, one loop (latency, a transport loop, a locate, a tempo change, stop and play, overdubs, undo, redo, layer pan, level, delete, solo and mute, clear) and three loops (handovers, overdubs, clear, undo, mute, a locate, stop and play), give bit-identical output at periods of 64 to 1024 frames.
- `tests/no_alloc.rs`: a counting allocator watches `command`, `layer_command`, `select_loop`, `process`, `status`, `loop_status`, `memory`, `stats`, `layer`, `loop_layer` and `layer_info` through a busy session at periods of 1 to 4096 frames, with a full memory, layer mutes, solos, pans, levels and deletes, and three loops: no allocation or free.
- The ARMv7 build (the MPC One's) passes the same tests under `qemu-arm`, and its demo gives the same hash.

## Listening

```text
cargo run --release -p mpfree-looper --example render -- [--period N] [--out FILE]
```

Writes `~/mpfree-work/looper-demo.wav` (45.5 s). A player plays into the line input at 96 BPM, 640 frames late. One bar of count-in, then a two-bar loop builds up: a distorted saw bass (the first pass), crushed drums (overdub), a saturated pulse lead (overdub), Undo (the lead fades out), Redo (it is back), distorted power-chord stabs on the off-beats (overdub), then a stop under a final live hit. The distortion is in the sounds, on purpose; the master is plain gain, peak -2.1 dBFS, no clipped samples. It prints each press and phase, and exits with an error if the first pass does not line up or anything clips.

## Line input on the MPC One

Round 7 (cc7f06d, the owner's run, reported by the pad player thread): the pad player opened `/dev/snd/pcmC1D0c` as S32_LE stereo at 44.1 kHz with 128-frame periods and a 256-frame buffer, and ran 46 188 periods (about 134 s) with no short reads, dropped frames, overruns or errors. The looper recorded a 2-bar first pass from the line input with 512 frames of latency, then an overdub. The owner recorded through the L jack; R is not checked yet.

Earlier read-only facts (from the owner's device reports while Akai's `acvs` service ran):

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

- Measure the round-trip latency on the MPC One, and check the R jack.
- The Looper tab (the owner approved the direction 10:16Z), engine side in this order: edit tools (half, double, reverse, nudge 1/16, trim, seam fade, merge) with an edit history; send to a pad, to a WAV and resample. The pad player thread builds the tab on this API.
- Run `mpfree-bench --filter looper` on the MPC One for the device cost of 8 loops.
- Time-stretch, so a loop keeps its pitch at another tempo; saving loops with the project.
