# Looper: audio loops on the transport

**Status:** engine crate (roadmap step T4), 2026-11-11, on main. The pad player runs it, with a Looper tab and the line input ([below](#in-the-pad-player-round-6)), and round 7 (cc7f06d) ran the one-loop version on the MPC One, from the pads and from the line input (see [Line input](#line-input-on-the-mpc-one) below). Several loops, per-layer mixing, the edits, merge and the faster playback described here have not run on the device yet.

Up to 8 stereo loops on the transport, recorded from the line input or the pad player's own sound, playing together and sharing one memory. Each follows the transport's bars, so it stays in time with the clips, the arp and the metronome.

## How it plays

- **The first pass** starts on the next bar line after Record, and ends after the set number of bars (2 by default), or, with free length, on the bar line after Record is pressed again. Its place in the song is a whole number of bars from `origin`.
- **Playback follows the song position.** The loop's position is the song position's place in the loop: `(tick - origin) mod len_ticks`, scaled to frames and read between frames. It stays on the bars through stops, locates and transport loops. At the tempo it was recorded at it plays sample for sample; at another tempo it plays faster or slower, like tape, until a print for that tempo is in ([Prints](#prints-another-tempo-the-same-pitch)): then it keeps its pitch.
- **Overdubs** are layers on top of the first pass, from the moment Overdub is pressed until it is pressed again, over as many passes as you like. Overdub pressed during the first pass starts the overdub exactly where the first pass ends. Undo fades out and removes the newest layer; Redo brings it back, until a new overdub starts.
- **After recording** (`Command::SetAfter`, the Looper tab's setting): `After::Play` (the default) plays the loop when its first pass ends; `After::Overdub` goes straight into an overdub, as if Overdub had been pressed during the first pass. Overdub during the first pass cancels it for that pass. A resample's first pass never goes into an overdub.
- **Fade old** (`Command::SetFadeOld(keep)`, 0 to 1, the default 1 fades nothing): each time the loop comes round while overdubbing, the older layers keep `keep` of their sound (0.88 keeps 88 % a pass), and the layer being recorded keeps that share of its own earlier passes, so old material dies away as new comes in, like a tape loop's feedback. The older layers turn down by a gain of their own, gliding over a few milliseconds at the loop point; their level knobs stay as set, and `LayerInfo::faded` shows how far each is down. Undo of the overdub turns them back up to where they were, and Redo down again. Below -80 dB a layer is silent.
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

## The history

`history()` (or `loop_history(n)`) gives what the Looper tab's undo history shows: `done()`, the steps Undo takes back, oldest first, and `redo()`, the steps Redo brings back, the next one first. Each `Step` is `Overdub(id)`, `Delete(id)`, `Edit(edit)` or `Merge(id)`, with layers named by id as in `LayerInfo`. It holds the last `HISTORY` (64) steps of a loop. The first pass is not a step: Clear takes it away. It is a plain value with no allocation, so the audio loop can send it to the screen.

## Edits

The edits change how a loop plays, never its audio:

| Edit | What it does |
|---|---|
| `Half` | Keeps the first half (down to a 1/16 note). |
| `Double` | Plays the loop twice. Overdubs can then make the two halves differ. |
| `Reverse` | Plays the loop backwards. |
| `Nudge(n)` | Moves the audio by `n` 1/16 notes, later (above 0) or earlier, going round the loop. |
| `Trim { from, to }` | Keeps ticks `from..to` of the loop. That becomes the loop, and it stays where it was in the song (`origin` moves by `from`). |
| `SeamFade(frames)` | The crossfade at every cut the edits make, up to `MAX_SEAM` (8192). 0 is a straight cut; 256 (`FADE`) until changed. |

- **How they work.** Each layer plays its audio through a view (`src/view.rs`): up to 8 pieces of its audio, each forwards or backwards. An edit makes new views for every layer and keeps the old shape (length, place, seam, views) for Undo. So an edit takes no memory and no time, and Undo brings back exactly what was there. Edits go into the same history as overdubs and deletes.
- **The loop around an edit.** Every layer of the loop is edited together. Overdubs recorded after an edit fit the edited loop. An edit, its Undo and its Redo crossfade from the loop before over 256 frames.
- **Seams.** Where playback jumps from one piece of audio to another (a trimmed loop's loop point, a nudge inside a trimmed loop, a halved loop's end), the last `seam` frames of the piece crossfade into the audio just before the next piece, which flows on into it. That is the same trick as the first pass's loop point.
- **When.** Edits apply while the loop plays and nothing records. Otherwise, or when an edit cannot be made (shorter than a 1/16 note, or more than 8 pieces in a view), they are counted in `Stats::refused`. Like Overdub, an edit waits for a fade or an overdub's tail.

## Merge

`Command::Merge` bounces the loop's layers into one layer, as heard: through their edits, at their levels, pans, mutes and solos. The loop's own level and mute stay outside.

- **In the background.** It takes the memory for one layer and renders `MERGE_SPEED` (2) frames for each frame that plays, while the loop plays on unchanged. A 2-bar loop at 120 BPM (4 s) is merged in 2 s. `Status::merging` gives the progress, 0 to 1.
- **Taking over.** The merged layer takes the layers' place on the frame by which it is surely rendered: the period the merge started in, plus half the loop. That frame is the same at any period size. It sounds the same, so the change is instant. The test compares the whole session against one without the merge: the output differs by less than 0.00001.
- **Undo** crossfades back to the layers, in their places with their mixes, and Redo crossfades to the merged layer again. Undo while it renders stops it and gives the memory back.
- **While it renders,** the loop's other commands wait (Record, Overdub, Redo, edits and every layer command), so the layers it reads cannot change under it. Undo, Clear, Mute and the loop's level still apply.
- **Refused** (counted in `Stats::refused`) when the loop is not playing, is recording, has nothing to merge (one plain layer at its own level), or there is no memory for one more layer.
- **Room.** A loop plays at most 15 layers (`MAX_LAYERS` - 1), so a merge always has a slot. When every slot is held (the merged layer, and the layers its Undo would bring back), a new overdub forgets the history up to the oldest delete or merge.

## Bounce and resample

Step (d) of the Looper tab: a loop can leave the looper as audio, for a pad or a WAV file, and a new loop can record the others.

- **Bounce.** `Looper::bounce(n, buf)` renders one pass of loop `n` as heard (its layers through their edits and mixes, at the loop's level, not muted) into `buf`, in the background at `MERGE_SPEED` like a merge. The app makes `buf` off the audio thread with room for the loop (twice its frames, stereo) and moves it in; the looper never grows it, so nothing allocates. `bounce_progress()` gives 0 to 1. When it is done, `take_bounce()` moves the `Bounce` out: `audio`, `frames`, the loop's `ticks` and `origin`, and `complete` (false if Clear stopped it, with silence after what was rendered). Off the audio thread, `Bounce::wav(rate)` makes a 32-bit float stereo WAV, which the sampler's `parse_wav` reads, so the app can put it on a pad or save it with `mpfree-store`.
- **While it renders** the loop plays on, unchanged, and its commands wait (Mute and the loop's level still apply; Clear stops it). One bounce at a time. `bounce` gives `buf` back when the loop is not playing, is recording or merging, `buf` is too small, or a finished bounce has not been taken.
- **Resample.** `Command::Resample` on an empty loop is Record, but the first pass records the other loops as they are heard (after their levels and mutes), sample for sample, instead of the input. There is no latency to make up for, because the audio never leaves the box. The loop then plays and takes overdubs like any other. Resample again during the first pass acts as Record; on a loop that has a first pass it is refused.
- **Tests.** A bounce matches the layers as heard, through their mixes and at the loop's level (to 0.000001), and its WAV has the sampler's format (a WAV from `Bounce::wav` was also read back through the sampler's `PreparedSample::parse_wav`, bit for bit, once by hand); a resampled loop matches what the other loops played, sample for sample; a resample runs through the busy three-loop session at every period size; the random-press test bounces and resamples; the no-alloc test bounces again and again, with its buffer going round.

## No clicks

| Where | What the looper does |
|---|---|
| The loop point | The last 256 frames crossfade into the input from just before the first pass, which flows on into the first frame. A sine that does not fit the loop would jump about 0.13 at a raw splice; the test holds it under 0.008. |
| Punch in and out | The overdub writer fades in and out over 256 frames (`FADE`, about 5.8 ms). After a full pass it adds, so the fade-in and the next pass join without a seam. An overdub that takes over from the first pass fades in over 32 frames (`HANDOFF_FADE`), because the input runs on without a break there; that keeps the downbeat's attack. |
| Start, stop, mute, clear | The loop's gain ramps over 256 frames. |
| Undo, redo and delete | The layer's gain ramps over 256 frames. |
| A layer's level, pan, mute and solo | Its left and right gains move one 256th of full scale a frame toward the settings. |
| A jump in the song position | A locate, a transport loop that is not a whole number of loops, or a restart right after a stop: the old place and the new one crossfade over 256 frames. |
| Edits | An edit, its Undo and its Redo crossfade from the loop before over 256 frames. Each cut an edit makes has a seam fade (`SeamFade`). |
| Merge | The merged layer takes over without a fade (it is the same sound). Undo and Redo of a merge crossfade over 256 frames. |
| Resample | The first pass records the other loops after their own fades, and its loop point crossfades like any first pass. |
| Commands during a fade | Record, Overdub, Undo, Redo, edits, merges and Delete that would cut a fade short wait for it (at most 256 frames), in order. Pressing Undo twice removes two layers. A new overdub waits while the last one's tail is written (the latency, then 256 frames), so that tail still fades out. |
| Stale audio | A reused layer slot only plays the frames the new overdub wrote. |

## Prints: another tempo, the same pitch

A loop follows the song position, so at another tempo it would play faster or slower, like tape. A **print** keeps its pitch: one pass of the loop's layers as heard, stretched to the loop's length at the new tempo with `mpfree-stretch` (the sampler's phase vocoder, which keeps hits sharp).

- **Made on a worker.** `Looper::bounce_print(n, buf)` renders the layers as heard, without the loop's own level and mute, at `PRINT_SPEED` (8 frames for each frame that plays, so a 2 s loop is ready in a quarter of a second). `print::PrintJob` (or `print::print`) stretches the `Bounce` to `print::frames_at(ticks, bpm, rate)` frames on a worker thread, in steps. A loop goes round, so the job stretches it with two stretch windows wrapped on before and after and keeps the middle; the print's start is crossfaded over `print::SEAM` (1024) frames from what the stretch made after its end, so it joins up at its loop point with no click.
- **Handed in.** `Looper::print(n, Some(print))` hands it to loop `n` without allocating; it takes over once the print playing (if any) has faded out. It plays in place of the layers, crossfading in and out over `FADE` frames, while the loop is still as it was when bounced: the same `Status::version` (which moves on with every overdub, undo, edit, delete or layer mix change), length in ticks and start. Any change stops it until a new one comes. The print it replaces waits in `take_print(n)` for the host to drop away from the audio thread. `Status::printed` says a print plays; `print_info(n)` gives its frames, ticks and version.
- **Where it plays** is worked out from the song position, as the loop's is: `(tick - origin) mod ticks`, scaled to the print's frames. So it stays on the bars through locates and transport loops, and a jump crossfades as the layers' does.
- **In the app**, `LoopHost` runs a print worker (the `mpfree-print` thread): each period it looks at the tempo, and a playing loop whose length at that tempo is more than a frame off its own (within the stretch's 10x) gets a print made and handed in. A print that comes back for a loop that has changed meanwhile is dropped (counted as "too late"). Buffers are made and dropped on the worker, so the audio loop never allocates. `describe()` counts prints made, too late and failed.

## Fit

Overdubbing over a print would record at the new tempo onto layers made at the old one. So **Overdub (or Record) while a print plays first fits the loop to the tempo**: the print becomes the loop's one layer, at the print's length (the loop is then sample for sample at the new tempo), and the overdub starts on the frame it takes over. The copy runs at `FIT_SPEED` (64 frames for each frame that plays, a plain copy), so the overdub waits about a 64th of the loop; a second press while it copies calls the overdub off. `Command::Fit` fits without overdubbing. Undo takes the fit back as one step (`Step::Fit`): the loop dips out over `FADE` frames, its layers come back as they were, and it fades in again; Redo fits again. `Status::fitting` gives the copy's progress. A fit is refused when no print plays.

## Saved with the project

A loop goes into a project as its settings, each layer's mix, and each layer's audio as it plays (through the loop's edits and their seam fades, at gain 1): [`save.rs`](../../../rust/mpfree-looper/src/save.rs).

- **Saving.** `Looper::loop_file(n)` gives a `LoopFile` (frames, ticks, origin, level, mute, bars, After recording, Fade old, and each layer's level, pan, mute, solo and Fade old turn-down) for a loop that is playing with nothing changing its layers (no first pass or overdub being written, merge, fit or load). It allocates the list of layers. `Looper::read_layer(n, k, from, out)` reads a piece of layer `k` without allocating, and gives the loop's version, so a save read across many periods can tell that the layers changed.
- **The files.** `save::to_text` and `save::from_text` are the looper's part (`parts/looper.txt`): `key=value` lines, one a loop and one a layer, with each layer's audio named by its file. Unknown keys and lines are skipped, so a newer build's text still loads. `save::wav` and `save::from_wav` write and read each layer as a 32-bit float stereo WAV, which the sampler reads too.
- **Loading.** `Looper::load(n, file)` puts a saved loop back into an empty loop. It takes its memory at once (refused if there is not enough), sets the settings, and copies the layers in at `LOAD_SPEED` (64 frames for each frame that plays) while the loop stays silent; then the loop fades in, in time with the song, on a frame that does not depend on the period size. Its commands wait while it copies, except Clear (which calls it off and gives the memory back), Mute and the settings. `Status::loading` gives the progress. The `LoopFile` waits in `take_loaded(n)` for the host to drop away from the audio thread. A loaded loop has no history: Undo cannot go back past it.
- **In the app.** `LoopHost::save()` waits until no loop is recording, overdubbing, merging, fitting or loading, has the worker make room for every layer, then reads the layers as they play at `SAVE_SPEED` (64 frames for each frame that plays), with no glitch; a loop that changes while it is read starts the save again (`save_restarts`). `take_saved()` gives the result: the project keeps its text (`save::to_text`, each layer naming its file) and a WAV a layer (`save::wav`). Opening goes back through `save::from_text` and `save::from_wav`, and `LoopHost::load` loads each loop into its place. Nothing here writes a file: the project browser writes the part and the WAVs into the project through its own reviewed save (`gui.rs` `save_project`, each layer named `loop-<id>.wav` for its audio, see [browser.md](browser.md#the-parts)).
- **Open and New.** `LoopHost::clear_all` stops a save under way and queues Clear for every loop (once the command queue has room for all eight). The opened project's loops come from a reader thread (`project::loops_open`) through `LoopHost::open`, and `LoopHost::tend_open`, between periods, loads them once every loop is empty and the clears have applied (`settle`: a load before then would be cleared with them). New project only clears.

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
wave.read(from, &mut frames);               // or many frames at once, a block at a time
let so_far = looper.first_pass(0);          // the first pass as far as it has recorded (Status::recorded frames)
let info = looper.layer_info(1);            // its id, level, pan, mute, solo, audible, recording
looper.layer_command(info.unwrap().id, LayerCommand::Mute(true));
looper.bounce(0, buf);                      // loop 1 out to a pad or a WAV: buf made off the audio thread
if let Some(b) = looper.take_bounce() { /* send b off the audio thread: b.wav(44_100) */ }
looper.bounce_print(0, buf);                // for a print: the layers as heard, without the loop's level
let p = print::print(&bounce, 44_100, print::frames_at(ticks, bpm, 44_100))?; // on a worker
looper.print(0, Some(p));                   // plays while the loop is as it was; take_print(0) to drop the old one
let file = looper.loop_file(0);             // saving: settings and mixes (allocates the list)
looper.read_layer(0, k, from, &mut piece);  // a layer as it plays, a piece at a time
looper.load(2, file);                       // into an empty loop; take_loaded(2) once in
```

- `Command`: `Record`, `Overdub`, `Undo`, `Redo`, `Clear`, `Mute(bool)`, `SetBars(Option<u32>)`, `SetAfter(After)`, `SetFadeOld(f32)` ([How it plays](#how-it-plays)), `SetLatency(u32)`, `SetLevel(f32)` (0 to 2), `Edit(Edit)` ([Edits](#edits)), `Merge` ([Merge](#merge)), `Resample` ([Bounce and resample](#bounce-and-resample)), `Fit` ([Fit](#fit)). Commands apply at the start of the next period.
- `Status` also has `seam` (the seam fade in frames), `merging` (a merge's progress, 0 to 1, while it renders), `resampling` (armed or recording a resample), `after`, `fade_old`, `version` (moves on whenever the layers' sound changes), `printed`, `fitting` and `loading`.
- `history()` and `loop_history(n)`: [The history](#the-history).
- `LayerCommand`, sent with a layer's id: `Level(f32)` (0 to 2), `Pan(f32)` (-1 to 1), `Mute(bool)`, `Solo(bool)`, `Delete`. Deleting the overdub being recorded throws it away, as Undo does. A command for a layer that is not there is counted in `Stats::no_layer`.
- `LayerInfo` (`layer_info(k)`, in play order, the first pass first): `id`, `level`, `pan`, `muted`, `solo`, `audible` (not muted, not left out by a solo), `recording` and `faded` (Fade old's turn-down; the layer plays at `level * faded`).
- `Phase`: `Empty`, `Armed`, `Recording`, `Playing`, `Overdubbing`.
- `layer(k)` and `loop_layer(n, k)` give a `LayerView`: index it like the slice it used to be (`wave[2 * frame + channel]`, `wave.len()` samples). It reads through the layer's edits, so frame `f` is what plays at frame `f` (without the seam fades). Frames an overdub did not reach read as silence, so a waveform never shows stale audio from a reused slot. `read(from, out)` gives the same values for many frames at once, copying whole blocks where the layer plays unedited and fully written; a reader of many frames (the waveform) uses it, since indexing frame by frame looks up the block for every sample.
- `first_pass(n)` gives loop `n`'s first pass as far as it has recorded while it records (`Status::recorded` frames, 0 otherwise), so the screen can draw it as it grows; None before its first frame and once it ends.
- **Stopped:** once a loop has faded out after a stop, its position waits at the song position's place in it, so the playhead stops with the transport and shows where it will start again (a second STOP takes the song, and the playhead, back to the start). Before, the position ran on while stopped.
- `Stats` counts what went wrong, never silently: commands that did not fit the queue, overdubs, edits and merges refused, first passes thrown away (the transport stopped or jumped during the first pass, or it outgrew the memory).
- The memory is shared by every loop and layer: a 2-bar loop at 120 BPM is 4 s, so 60 s holds a first pass and 14 overdubs, or 8 loops of a first pass and two overdubs at 2 s each. The app sizes it from Prefs "Loop memory" (1, 2, 5, 10 or 20 minutes, 5 by default; `LoopMemory` in `looping.rs`), made once at the start and locked in RAM with the rest of the process, so a new size applies from the next start. `bytes_for(rate, seconds)` gives what a size costs (the sound plus the block tables, about 8.4 KB a block: 110 MB for 5 minutes at 44.1 kHz). Block numbers are `u16`, so the most a looper holds is 65 535 blocks, about 25 minutes at 44.1 kHz; 20 minutes fits at 48 kHz too. At most 15 layers a loop (16 slots, one kept for a merge). `Status::used` is the memory a loop holds, in whole blocks: its layers and the ones Undo or Redo can bring back. A deleted layer keeps its memory while Undo can bring it back; when the memory is full, a new overdub takes the oldest deleted layer's memory, and the history up to that delete is forgotten.
- `process`, `command`, `select_loop`, `layer_command`, `history`, `bounce`, `bounce_progress`, `take_bounce`, `print`, `take_print`, `read_layer`, `load` and `take_loaded` never allocate, lock or panic. `Bounce::wav`, `loop_file`, the `print` and `save` modules allocate and belong off the audio thread.

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

**Edits and merges.** The workstation's timings drift by a third during the day on the cloud VM, so these were compared by counting instructions (callgrind, 2000 periods of 128 frames, 8 one-bar loops of 3 layers):

| Case | Instructions per period |
|---|---|
| 8 loops, no edits, before the edits existed | 277 000 |
| 8 loops, no edits, with the edits built in | 292 000 (+5 %) |
| 8 loops, half of them edited (a quarter reversed, a quarter trimmed and nudged: two pieces and a seam) | 431 000 |

An edited layer finds its pieces with a cursor that starts from the last one (one compare a frame, not a search). It reads each audio frame once where one frame's second sample is the next one's first, and a frame's place in the audio needs no division. The first version of the edits cost 609 000 for the edited case. A merge adds two frames of every layer per frame while it renders: about as much again as playing the loop, for half its length. The bench has an "8 loops of 3 layers playing, half edited" case.

**History, After recording and Fade old.** The history is a fixed list read by the screen, so it costs nothing while playing. Fade old adds one multiply to each layer's level and, once a pass, one to each frame the overdub writes over; the bench's "8 loops of 3 layers playing" case went from 526.7 to 532.8 million instructions for the whole run (+1.2 %), with the demo's hash unchanged.

## Tests

`cargo test -p mpfree-looper` (workstation, no device):

- `src/lib.rs`: `rem_ticks` against `rem_euclid`, bit for bit, on a million song positions and loop lengths.
- `src/view.rs`: views against a plain list of frames through thousands of random cuts, doublings, rotations and reverses; where the cuts are; the pieces stay few; the division-free wrap against `rem_euclid`; the piece cursor against a search from the start.
- `tests/looper.rs` (49 tests, against a real transport): the first pass starts on the bar and plays back sample for sample; the first pass reads as it records; latency is made up for; the loop point does not click; overdubs stack, and undo and redo fade; Undo twice removes two layers; punches fade, and old audio in a reused slot stays silent; stop fades out and play fades back in; the playhead stops with the transport (and a second STOP takes it to the start); a locate crossfades; nothing plays during a count-in; free length ends on the bar after Record; a transport loop ends the first pass and the loop follows it; stopping or jumping during the first pass throws it away; overdubs are refused when the memory is full; Overdub pressed while stopped waits for Play; Clear then Record arms again; mute and level fade and scale; Overdub during the first pass takes over where it ends; each layer's level, pan, mute and solo, without clicks, and NaN or out-of-range values; a deleted layer fades out and comes back with Undo in its place, in one history with overdubs; deleting the overdub being recorded throws it away like Undo; a new overdub in a full memory takes the oldest delete's memory; a layer's waveform shows only what its overdub wrote, and read in bulk it is what indexing gives (whole, a partial overdub, reversed and nudged); several loops play together, each in time, and commands go to the selected one; one loop records at a time (an overdub punches out, an armed loop disarms, a first pass ends on the bar line where the next one starts); the loops share one memory and Clear gives it back; a first pass that outgrows the memory is thrown away while the other loops play on; Half keeps the first half in time and Undo brings back the rest; Double plays the loop twice and an overdub can fill either half; Reverse plays backwards and twice is as recorded; Nudge moves the audio by 1/16 notes round the loop; Trim keeps a range where it was in the song; the seam fade smooths the cut a trim makes; an edit crossfades from the loop before it; edits undo and redo in one history with overdubs; edits wait for play and refuse what cannot be made; a merge sounds the same (against the same session without it) and Undo brings the layers back; a merge holds the other commands and Undo stops it; merging a full loop makes room for more overdubs; a bounce renders the loop as heard, for a pad or a WAV; resample records the other loops sample for sample; the history lists each overdub, delete, edit and merge, and what Redo would bring back; set to overdub after recording, the first pass goes straight into one (and Overdub during the pass cancels it); Fade old turns the older layers down each time round while overdubbing, gliding at the loop point, and Undo turns them back up; Fade old keeps its share of the overdub's own earlier passes; random presses on three loops, with every edit, merges, resamples and bounces, never break a loop; and two busy sessions, one loop (latency, a transport loop, a locate, a tempo change, stop and play, overdubs, undo, redo, layer pan, level, delete, solo and mute, clear) and three loops (handovers, overdubs, Fade old on one of them, clear, undo, mute, a locate, stop and play, edits, merges rendering through a stop, a merge undone, a resample), give bit-identical output at periods of 64 to 1024 frames.
- `tests/print.rs`: a print plays at another tempo with its pitch kept, loops sample for sample and joins up with no click at its loop point; a layer change stops it; Overdub over a print fits the loop first and opens the overdub on the frame the fit takes over, Undo takes the fit back with a dip and Redo brings it again; prints and fits give the same output at periods of 1 to 1024 frames; and what `print` accepts and gives back.
- `tests/save.rs`: a loop with two layers, a nudge (a cut, so a seam fade) and a mix is saved through its text and a WAV a layer, loaded into another looper, and plays the same to 0.00001 a bar on, in time with the song; a load comes in on the same frame at periods of 1 to 1000 frames; Clear calls a load off and gives the memory back; a load is refused into a loop that is not empty, past the last loop, with a short or no layer, past the memory, or while the last one waits to be collected; nothing is saved from an overdub in progress. `src/save.rs` tests the text (round trip, unknown words skipped, bad lines refused with their number) and the WAV.
- `tests/no_alloc.rs`: a counting allocator watches `command`, `layer_command`, `select_loop`, `process`, `status`, `loop_status`, `memory`, `stats`, `layer`, `loop_layer` and `layer_info` through a busy session at periods of 1 to 4096 frames, with a full memory, Fade old, overdub after recording, the history, layer mutes, solos, pans, levels and deletes, edits, merges with undo and redo, resamples, bounces with their buffer going round, and three loops: no allocation or free. `bounce`, `bounce_progress` and `take_bounce` are watched too, and a second test watches `read_layer`, `load`, the copy and a load called off with Clear.
- The ARMv7 build (the MPC One's) passes the same tests under `qemu-arm`, and its demo gives the same hash.

## Listening

```text
cargo run --release -p mpfree-looper --example looper_render -- [--period N] [--out FILE]
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

[`looping.rs`](../../../rust/mpfree-app/src/looping.rs) runs the looper in the pad player's audio loop: the loop plays into the mixer's Loop track and the line input into its input strip, which is heard while Monitor is on ([`mixing.rs`](../../../rust/mpfree-app/src/mixing.rs)), and [`looper_page.rs`](../../../rust/mpfree-app/src/looper_page.rs) is its **Looper** tab ([the guide](../guides/mpfree-app.md#round-6-the-looper-the-synth-and-the-sampler-in-one-pad-player)).

- **What it records.** By default the pads and the synth as the engines make them, before the mixer (the loop plays back through the mixer, so the Level is applied once), and never the click or the loop, so nothing on the MPC is opened and no latency needs making up. With the line input on, the line input, with the latency estimate below.
- **The line input.** `alsa.rs` opens the ACVA card's capture device without waiting (`O_NONBLOCK`) at the settings Akai's app used (stereo S32_LE, 44.1 kHz, 128-frame periods, a 256-frame buffer), starts it, and reads whatever has arrived each period into a FIFO of 8 periods. A period with too little input gets silence and is counted (after the first 4); more than 3 periods waiting are dropped to 2, counted; an overrun prepares and starts it again, counted; more than 3 read errors in a row close it. It is off at every start; the row comes back Off whenever it closes.
- **Monitor** adds the line input to the outputs (about 9 ms late at 3 periods).
- **Latency estimate**: the output buffer plus one input period, `(periods + 1) × 128` frames (512 at 3 periods). The converters add an unknown amount: measure it with a cable from an output to the input and set it in the Looper options.
- **Logging**: a `looper:` line for each press and change (with the song position), `line input:` lines when it opens, closes or fails, and every 30 s `status: looper: ...` with the render time.
- **Prints and saves**: the print worker and the save reader ([Prints](#prints-another-tempo-the-same-pitch), [Saved with the project](#saved-with-the-project)).
- **The waveform** (`LoopHost::wave_step`, on the audio thread, once a period). The Looper tab and the Play page's strip draw the selected loop from a `LoopWave`: min, max and RMS of the layers summed and each layer's peak, in 600 columns. It is built a slice a period: `WAVE_BUDGET` (2 048) layer-frames shared by the layers, read with `LayerView::read` and folded into the columns run by run. That is about 6 µs a period on the build workstation, roughly 1 to 2 % of a period on the MPC; a two-bar loop's wave takes a quarter of a second, and a minute of loop under 4 s (one loop filling all 20 minutes of the largest Loop memory: about 80 s, the wave growing as it goes, never costing more a period). It starts again when the loop's length, layers or phase change, or another loop is picked, and every `LOOP_WAVE_PERIODS` while overdubbing. While the selected loop's first pass records, a live wave takes its place: the frames recorded since the last period go into columns drawn against the pass's length (its bars at the tempo, or two bars for a free length, doubling each time the pass gets there), and a wave goes to the screen every 12 periods (about 35 ms) with `recorded` set, so the strip and the tab draw it growing and put the playhead at its end. The waves come from a pool of four made at the start: a wave is filled in place once the screen has let go of it (`Arc::get_mut`), so making one never allocates or frees on the audio thread. If the screen holds all four, the wave waits a period, counted in `describe()` (it should stay 0). Before this, the builder indexed every sample of every layer, 8 192 frames a period, and on the MPC that cost more than a period: the device log from the 12 Nov live test shows underruns climbing from 6 to 100 while the looper was in use, which is the choppy, clicking playback heard then.
- **Tests**: `cargo test -p mpfree-app looping` (the FIFO, the pads recorded to the frame, a missing device, no allocation in a steady period, the wave built a small slice a period and matching the layers frame for frame, the first pass's wave growing as it records and widening at a free length, the wave emptying on Clear or a thrown-away pass, no allocation making waves, and a loop at another tempo getting a print and keeping its pitch, then fitting on Overdub), loops saved as the project keeps them (text and WAVs), read back and loaded into a fresh host that then plays the same, and a loop changing while it is read starting the save again, and `looper_page`.

## Next

- Measure the round-trip latency on the MPC One, and check the R jack.
- The Looper tab (the owner approved the direction 10:16Z): the engine side is done (layers, several loops, edits, merge, bounce and resample, the undo history, After recording and Fade old). The tab has the layers, the edit tools, merge, the trim handles, the undo history, After recording, Fade old and the seam fade on Q3 and Q4, and Resample (round 10). Still to wire: a bounce to a pad (the sampler), to a WAV (`mpfree-store`) and to a clip.
- Run `mpfree-bench --filter looper` on the MPC One for the device cost of 8 loops.
- Show `printed`, `fitting`, `loading` and the save's progress on the Looper tab.
- A loop to an audio clip on Arrange.
