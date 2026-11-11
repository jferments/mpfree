# Playback: clips on the transport

**Status:** roadmap step T2, 2026-11-11. The code is [`rust/mpfree-seq`](../../rust/mpfree-seq) (the model) and [`rust/mpfree-play`](../../rust/mpfree-play) (the playback engine). Both are host-only and tested. They are not yet wired to the audio thread, the pads or the arp. Nothing here has run on the MPC.

This builds on the [transport](transport.md). The full plan is the time engine design document. Its mockups and skill tree are in the mpfree-claude notes (`sequencer/`).

## The model (`mpfree-seq`)

A project holds up to 16 tracks. A track has a destination, an optional swing, a mute flag and 16 clip slots. A clip has:

- a length in its own ticks (from a 1/32 note to 256 bars)
- a speed (1/8 to 4 times)
- an optional swing and an optional quantise
- its notes, each with a tick, a length, a note or pad number and a velocity

These are plain values, edited on the control thread and checked with `check()`.

- **Everything is a clip.** A step pattern and a recorded phrase are the same thing. The step grid will be a view of a clip, not its own data.
- **Any length.** Clips of different lengths drift against each other (polymeter), and no clip is tied to 16 steps.
- **Quantise is non-destructive.** Notes keep their recorded ticks, and quantise (grid and strength) applies on playback, so it can be changed or undone later.
- **Swing:** the clip's own swing wins, then the track's, then the transport's.

Chance, trig conditions, ratchets, parameter locks, audio clips, scenes and the arrangement come in later steps. They add to this model without changing it.

## Compiling (`PlayView`)

The control thread compiles the project and the playing clips (`Launch { track, slot, origin }`) into a read-only `PlayView`. For each clip it:

- sorts the notes
- applies quantise and speed
- picks the swing to use

The audio thread only reads the view. After an edit, the control thread compiles a new one and swaps it in, the same pattern as the sampler's `EngineView`. Launching clips on a grid (T9) will produce the `Launch` values. For now a clip plays from `origin` on and loops.

## Playing (`Player::process`)

Each period the player walks the transport's spans and sends events in frame order to a `Sink`. Each event says which track it came from and which destination it is for. The host maps destinations to the sampler, the arp or MIDI out.

- **Clip positions come from the tick.** A clip's position is worked out from the song tick every time (origin, length, speed), never counted up. After a loop jump, a locate or a tempo change, every clip is in the right place.
- **Whole ticks.** A note's song position after quantise, speed and swing is rounded to the nearest tick. A tick is 1/960 of a beat, 0.26 ms at 120 BPM, the same resolution as the MPC. A note plays when the transport plays its tick, on that tick's frame. The transport plays every tick exactly once between jumps, whatever the period size, so no note is lost or doubled at a period edge or a tempo change.
- **Note-offs** come on the tick where the note ends.
  - A loop jump, a locate or a stop ends every sounding note on that frame, and the sound keeps its release tail.
  - STOP while stopped (panic) also sends `AllOff`.
  - If a note starts while the same note is still sounding on the same track, the old one ends first, on the same frame.
- **Count-in and mute.** Nothing plays during a count-in, or on a muted track.
- **Real-time safe.** There is no allocation, lock or panic in `process`. A full table is counted in `Stats::dropped`, never dropped silently: 256 sounding notes, 256 note starts per span.

## Tests

- `tests/play.rs` has 12 behaviour tests:
  - quarter notes on the beats, and loops
  - polymeter, speed, swing (and a clip overriding it), quantise, a launch origin
  - loop jumps ending notes, and clips restarting in place
  - locate, stop and panic, retriggering, mute and count-in, tempo changes
- `tests/period_size.rs` plays a busy session on five tracks:
  - the session has a count-in, a loop with a tempo segment inside, live tempo changes, punch in and out, locates, stop and panic, and the shortest loop
  - the tracks cover polymeter, 3/4 speed, double speed with its own swing, quantised overlapping notes, and a muted track

  It cuts the session into periods of 1 to 8192 frames and three random sequences, and every event must match. It also checks that each note ends before it starts again and that the final stop ends every note. With the swing look-back window removed, the test fails.
- `tests/no_alloc.rs`: a counting allocator watches 4000 periods of five tracks with loops, tempo changes, locates, stop and panic. It sees no allocation and no free.

## Listening

`cargo run --release -p mpfree-play --example render` plays a demo song through the transport, the player and the real sampler engine (`mpfree-sampler`), and writes `~/mpfree-work/play-demo.wav`. The song is 92 BPM with 1/16 swing, a four-bar loop played twice:

- drums
- hats at double speed with their own swing
- a bass line on gated pads, so note lengths matter
- a 7/16 pluck clip that drifts against the bar

All the sounds are synthesised in the tool. It prints an events hash, which is the same on every computer, and checks that the events are identical at periods of 1, 128 and 4096 frames. On the build host the WAV bytes are also identical at 128 and 1000 frames.

## Next

- Feed a track's notes through the arp (`Engine::sync` per span, PR #42) once #40 and #42 merge.
- Then the clip recorder (T3) and the looper (T4).
- Device work waits for the pad player's round 5 (PR #31).
