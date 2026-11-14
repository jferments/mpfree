# Track host: each track's instrument into its mixer track

**Status:** 2026-11-13, in the app: each sequencer track can play its own instrument, a copy of a built-in engine or a plugin synth (see [In the app](#in-the-app)). The code is [`rust/mpfree-track`](../../../rust/mpfree-track), and the app's side is [`rust/mpfree-app/src/instruments.rs`](../../../rust/mpfree-app/src/instruments.rs). The pads' shared synth ([Synths](synths.md), "In the pad player") still plays as before for a track without one.

The track host sits between the notes and the mixer. The sequencer, the pads and the arp send it notes; it plays each track's instrument and adds the sound into that track's mixer input. Effects aren't in it: every mixer strip has its own insert chain ([FX](fx.md)).

```
sequencer (mpfree-play Sink) ─┐
pads, arp, Q-Links ───────────┼─> TrackHost ─> slot 0: Poly ─> mixer track 0 ─> inserts ─> ...
mod matrix, automation ───────┘               slot 1: FM   ─> mixer track 1 ─> inserts ─> ...
                                              slot 2: (empty)
```

## Slots and routing

- A host has a fixed number of **slots**, set when it is made. The sequencer's tracks name their instrument by a destination number (`mpfree_seq::DestId`, "the host owns the list"); slot `d` is destination `d`.
- A slot holds one **instrument**: any `mpfree_node::Node` that adds its sound into the buffers (Poly, FM, Wavetable, Pluck, Drums, the sampler as `mpfree_sampler::node::SamplerNode` (see [Sampler architecture](sampler-architecture.md#as-an-instrument-node-noders)), or a sandboxed plugin). `Instrument::new(node, rate, max_block)` prepares the node and gives it a timing probe. It allocates, so it runs on the control thread.
- Slot `i` feeds mixer track `i` until `route(slot, Some(track))` moves it; `route(slot, None)` keeps it playing into nothing. Two slots may feed one track (a layer). `track_of(slot)` and `slot_of_track(track)` look the routing up both ways; `slot_of_track` skips empty slots.

## Notes and parameters in

- **The sequencer** sends through `mpfree_play::Sink`: `Out::dest` picks the slot, and `AllOff` to `ALL` reaches every slot.
- **Everything else** calls `note_on(slot, offset, note, velocity)`, `note_off`, `param(slot, offset, id, value)` or `event(slot, Timed)` for pressure, bend and the rest. The offset is the frame in the next block.
- **Notes are counted per slot.** When a clip and a pad (or the arp) play the same note on the same instrument, the note-off reaches the instrument only when the last of them lets go, so one never cuts the other short. A note-off for a note nobody holds is ignored and counted. `AllOff` ends every note and clears the counts.
- **Parameters** land on their exact frame, for automation, parameter locks and the mod matrix. `params(slot)` lists the instrument's parameters and `param_value(slot, id)` reads one back.
- Each slot takes up to 256 events a block (`MAX_EVENTS`). More are dropped and counted; a dropped note-off leaves its count, so a later note-off or `AllOff` still ends the note.

## Sound out

- `render(ctx, frames, out)` runs once per audio period, before the mixer. Each slot's instrument renders into the slot's own buffer, which is then added into its track's input. `out` is anything that implements `Outputs`: the mixer (`Mixer::track_input`), or `Sum`, which adds every track into one stereo pair for a host with no mixer.
- A block with a non-finite sample is dropped, the instrument reset and the reset counted, as the node rules ask.
- Events are sorted by frame; one late for the block plays on its last frame. A block longer than `max_block` is cut to it and the missing frames are counted (`clipped_frames`).

## Swapping an instrument

`swap(slot, Some(new))` puts a new instrument in the slot at once. The old one isn't cut off: it gets `AllOff` and rings out into the same track until it has been below -100 dBFS for 4096 frames (`QUIET_FRAMES`, about 93 ms), or its own `tail_frames` and that margin have passed, at most 20 s. Then it comes back through `pop_retired()` for the control thread to drop, because freeing memory on the audio thread isn't allowed. Swapping again while one is still ringing retires that one at once. `swap(slot, None)` empties the slot the same way.

`swap` never allocates. It refuses only when there is no such slot, or when an instrument is still ringing and the retired list is full; it hands the new instrument back in both cases.

## Tuning

Each slot has its own tuning: how its instrument turns notes into pitch ([Synths](synths.md), "Tunings and per-note expression"). `set_tuning(slot, &tuning)` gives the slot a tuning, and `tuning(slot)` reads it back. It starts as 12-TET at A4 = 440 Hz.

- **The slot keeps it.** An instrument swapped in later takes the slot's tuning as `swap` puts it in, so changing a track's sound doesn't drop back to 12-TET. An instrument still ringing out after a swap keeps the tuning it had.
- **Live changes.** A change applies from the next block. Sounding notes move to the new tuning, smoothed where the engine smooths pitch.
- **Real time.** Like the rest of the host, it never allocates. A `Tuning` is about 1 KB of plain data, copied into the slot and into the node (`Node::set_tuning`), so it may run on the audio thread. Instruments without notes ignore it.

## STOP

`stop()` ends every note and fades every slot out over 10 ms (`STOP_FADE`, 441 frames at 44.1 kHz). When the fade ends, every instrument is reset and instruments still ringing out are retired, so nothing rings on after STOP.

## Real-time rules and counters

- `TrackHost::new` and `Instrument::new` allocate. Everything else never allocates, frees, locks or panics: the event queues, the render buffers and the retired list are all made in `new`.
- The output is the same, sample for sample, for any block size. The one exception is the moment a ringing-out instrument is retired, which falls on a block boundary; it is silent by then.
- `stats(slot)` gives notes started, events dropped, stray note-offs, resets and swaps. With `set_profiling(true)` each instrument's render is timed into its probe (`probe_mut(slot)`, `mpfree-prof`), so the pad player can log each instrument's share of the period as it does for Poly today.

## Cost

`mpfree-bench` has two cases for the host (`--filter "track host"`):

- **Track host alone, 16 tracks, 8 events each per block:** sixteen instruments that make no sound, each given four notes on and four off in every block, out of order. This is the host's own work: the queues, the sorting, the per-track sums and the clip guard.
- **Track host, 4 Poly tracks of 4 voices:** four Poly (Init) tracks, each with a chord changing every half second, a beat apart. Most of this is Poly's own work, the same as in the Poly cases.

On the 4-core cloud workstation VM (release build, 2026-11-11), the host alone took 3.3 us per block on average and 7 us at the 99th percentile, 0.25 % of the 2.9 ms period. The four Poly tracks took 71 us on average and 148 us at the 99th percentile, 5 % of the period. Neither allocates (`--check-allocs`). The MPC's numbers come from the next bench run on the device.

## Tests

`cargo test --locked -p mpfree-track` (from `rust/`):

- `tests/host.rs`: a slot plays into its own track and only there, and routing moves it; a note held by two sources ends with the last of them; the same events give the same samples in blocks of 1, 31, 64, 128 and 1000 frames; a swapped-out instrument rings out, goes quiet and comes back to be dropped; STOP silences every slot within 10 ms; a slot's tuning reaches its instrument and the next one swapped in (bit-identical to a Poly tuned directly, the other slot unchanged), and a change while a note sounds retunes it; a node that outputs NaN is reset and counted; a full queue counts what it drops, and long blocks are counted; Poly and FM go through the real mixer and out of the master.
- `tests/no_alloc.rs`: 3000 periods of notes, sequencer notes, parameters, tunings, swaps, `AllOff` and STOP with Poly and FM into the real mixer, under a counting allocator. The audio thread's part makes no allocation and no free.

## In the app

[`rust/mpfree-app/src/instruments.rs`](../../../rust/mpfree-app/src/instruments.rs) wraps a `TrackHost` with one slot per sequencer track (`mpfree_seq::MAX_TRACKS`). Jay's call (13 Nov): every track holds its own instrument, picked on the Synth tab for the selected (armed) track, rather than one plugin engine on the shared synth.

- **Routing.** Slot `t` plays sequencer track `t` into mixer track `TRACKS + t`, after Drums, Synth, Loop and Audio (`mixing.rs`). The mixer is made with all of them; `MixHost::poll_state` shows a track's channel ("Track 1") only while its slot holds or rings out an instrument (`Instruments::in_use`).
- **Notes in.** `SeqPads` (`main.rs`) sends a track's notes to its slot when the slot holds one: clip events in `out` (`clip_own`: pads are mapped to notes with the slot's own pad map; pressure goes to every note the pad plays) and the hardware pads and per-track arps in `live` (`live_own`). A pad's notes are remembered as they start, so a pad lifted after the layout changed ends what it started. A track without an instrument goes to the sampler or the shared synth as before. STOP and `AllOff` reach every slot.
- **Picking.** The Synth tab's top options row, "Track N instrument" (`app/synth_pads.rs`), steps Kit, the five engines (`Engine::node_id`), then the plugin synths the rack found (`Rack::instruments`), and gives `Effect::TrackSound`. An engine starts as a copy of the shared synth's sound for that engine (`Synth::sound_of`, `synth::fresh_node`); a plugin is made by the rack, in its own process.
- **Off the audio thread.** The builder thread makes the instrument (`control::make_instrument`, `Instrument::new` allocates and a plugin waits for its process), and the audio loop puts it in with `swap` between periods (`MixHost::poll_instruments`). The old one rings out; `pop_retired` hands it back and it is dropped on the builder thread. `MadeInstrument` carries a plain `Tuning`, so nothing the audio loop drops frees memory. If the builder is busy, the pick is refused, said on screen in place of the hint (`ToUi::Tell`) and in the log, and the row is set back.
- **The Synth tab** shows and edits the selected track's own instrument (`Synth::state_of`, labelled "Track N"). A knob goes straight to the slot (`Instruments::param`). A preset is loaded into a scratch copy (`Synth::preset_values`) and given to the track as a new instrument, so the old sound rings out. The pad layout, octave and tuning follow the shared Synth options for every slot.
- **Saved with the project.** The project's `instruments` part (`project::instruments_text`, format in [browser.md](browser.md#the-parts)) names each track's instrument with its parameters and, for a plugin, its state. An open makes them on the screen thread (`project::instruments_load`, a plugin through the rack) and the audio loop puts them in (`FromUi::OpenProject` in `main.rs`, with the slot's tuning set first); a track the project doesn't name is emptied, and New empties every slot. One that can't be made is left out with a line in the log and its track plays the kit; one the host refuses (the old sound still ringing out) is said, and the track keeps what it had. The log says how many tracks play their own instrument. After a restart, Prefs > At start on Last project opens the last project with its instruments (the rack's plugins reach the screen thread before the start's open, `ToUi::Plugins`); otherwise every track starts on the kit.
- **Counted.** The 30 s status adds `status: tracks' own instruments: track 1 Poly (N notes, N dropped, N resets), ...` and how many swaps were refused. A plugin that can't be made is said on screen and in the log, and the track keeps what it had.
- **Tests.** `instruments.rs` (an empty track plays nothing; an instrument plays into its own channel; a pad ends its notes after the map changed; a clip and a held pad end with the last; a track takes a copy of the Synth tab's sound; emptying rings out and hands the instrument back), `project.rs` (the instruments part reads back into the same tracks, engines and sounds; a missing plugin, a track that isn't there or is named twice, and an engine this build hasn't got are left out and said; no part means every track plays the kit), and `rt_safety.rs` (Poly on track 1 plays a clip with no allocation on the audio thread, and the mixer shows its channel with a peak; live pads on a track with an instrument play it, not the sampler).

Not yet:

- The Synth tab's meters and the knob log line follow the shared synth, not a track's own instrument.
- The slots render on the main audio core, not the sound cores.
- CV out for a track with its own instrument uses the shared pad notes.

## Next

- Show and change a track's instrument beside it on the Play and Arrange pages (agreed with Jay, 13 Nov).
- Per-step locks on a track instrument's parameters through `param`, the mod matrix and macros.
- Move the slots onto the sound cores when their cost needs it.
