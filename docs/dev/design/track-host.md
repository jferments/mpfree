# Track host: each track's instrument into its mixer track

**Status:** 2026-11-11, host-only and tested. The code is [`rust/mpfree-track`](../../../rust/mpfree-track). Nothing calls it yet: the pad player still holds its synth itself ([Synths](synths.md), "In the pad player"). Wiring it into the pad player is the next step and is shared with the Hello MPC work, which owns the app.

The track host sits between the notes and the mixer. The sequencer, the pads and the arp send it notes; it plays each track's instrument and adds the sound into that track's mixer input. Effects aren't in it: every mixer strip has its own insert chain ([FX](fx.md)).

```
sequencer (mpfree-play Sink) ─┐
pads, arp, Q-Links ───────────┼─> TrackHost ─> slot 0: Poly ─> mixer track 0 ─> inserts ─> ...
mod matrix, automation ───────┘               slot 1: FM   ─> mixer track 1 ─> inserts ─> ...
                                              slot 2: (empty)
```

## Slots and routing

- A host has a fixed number of **slots**, set when it is made. The sequencer's tracks name their instrument by a destination number (`mpfree_seq::DestId`, "the host owns the list"); slot `d` is destination `d`.
- A slot holds one **instrument**: any `mpfree_node::Node` that adds its sound into the buffers (Poly, FM, later a drum synth, a wavetable synth or a sandboxed plugin). `Instrument::new(node, rate, max_block)` prepares the node and gives it a timing probe. It allocates, so it runs on the control thread.
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

- `tests/host.rs`: a slot plays into its own track and only there, and routing moves it; a note held by two sources ends with the last of them; the same events give the same samples in blocks of 1, 31, 64, 128 and 1000 frames; a swapped-out instrument rings out, goes quiet and comes back to be dropped; STOP silences every slot within 10 ms; a node that outputs NaN is reset and counted; a full queue counts what it drops, and long blocks are counted; Poly and FM go through the real mixer and out of the master.
- `tests/no_alloc.rs`: 3000 periods of notes, sequencer notes, parameters, swaps, `AllOff` and STOP with Poly and FM into the real mixer, under a counting allocator. The audio thread's part makes no allocation and no free.

## Next

- Wire it into the pad player with the Hello MPC work: the Synth tab's instrument moves into a slot, the pads and the arp send to it, and the sequencer's tracks get their destinations.
- New engines (the drum and pluck synths, the wavetable synth) arrive as instruments in slots; the mod matrix and macros drive them through `param`.
- Plugin hosting: a CLAP plugin, in-process or sandboxed, in a slot ([Plugins](plugins.md)).
