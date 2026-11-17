# Loop tracks: loops in the Clips grid

**Status:** sequencer roadmap S2, Jay's sequencer decision 3 (16 Nov 2026, "Loop tracks in the Clips grid, as designed"). Part 1 is built: the looper launches a loop on or off from a bar line, on the frame it falls on (`rust/mpfree-looper`, 2026-11-17). Parts 2 to 6 below are the plan. Design: the sequencer review (mpfree-claude `sequencer/rethink/README.md`, "Loop tracks: loops in the grid", mockup J).

## The rule

**A loop is a clip on a Loop track.** It launches, stops, sits in scenes and goes into the song like any clip, while its audio, layers, edits and undo stay in the looper ([looper.md](looper.md)). Nothing moves audio: the grid holds references to the looper's loops.

The grid plays one clip per track at a time, so loops sit on Loop tracks, as audio clips sit on Live's audio tracks: one loop plays per Loop track, and loops on different Loop tracks play together, as they do in the looper today.

## The model (part 2)

- **A track's kind.** `mpfree_seq::Track` gains a kind: the note tracks of today, or a Loop track with its input (Pads + synth, the looper's source today; Line in; Resample, the other loops as heard).
- **A loop cell is a clip** whose `looped` names a looper loop (0 to 7) and which has no notes; its length is the loop's length in ticks, kept in step when the loop's length changes (a first pass, Half, Double, Trim). Because it is a clip, the launches, cues, scenes, follow actions, Stop, Stop all and the song's blocks already work on it.
- **The time engine says when a loop goes on or off.** A launch, a cue and a scene give each Loop track the loop that plays from a tick: the cue's bar line while the transport rolls, at once while it is stopped. The engine hands these to the app as loop launches (loop, on or off, the tick), and the app gives them to the looper (`LoopHost::launch`, `Looper::launch`), which acts on the bar line's own frame (part 1). The cue's tick is known when the cue is made, so the looper hears it before the line comes.
- **Loops in no cell play as today**, so nothing changes for a project without Loop tracks until it is saved.
- **A loop recorded from Play's strip or the Looper tab** lands on the next Loop track with nothing playing (one is added if none is free), in the row of the scene playing, so loops still layer as they do today.
- **Saved projects.** A project saved before Loop tracks opens with each loop on its own Loop track, in row 1, all playing as before. A new project has one empty Loop track, Loops A.
- **The looper still holds the audio:** 8 loops in a project, one memory.

## Building it

1. **Launches in the looper** (built). `Looper::launch(n, on, at)`: on or off from a bar line's frame, or at once, fading as a mute does; an off loop keeps its place in the song, so a launch brings it back in step. `Status::on` and `Status::cue`.
2. **The model and launching:** the track kind, loop cells, the time engine's loop launches to the looper, loops recorded landing on Loop tracks, saving and opening (with the old projects' loops each on a Loop track). The Clips grid shows a loop cell as a clip named after its loop.
3. **The Clips grid:** Loop track columns with a wave icon and their input under the name, loop cells with the loop's wave, bars and layers and the usual playing, queued and armed marks, a gauge over the grid ("Loops 4 of 8 · memory 38 %"), and Loop track in + Track. Play's loop strip and the Looper tab name a loop by its place ("Loops B · Loop 2 · Verse"); the Looper tab's To a clip becomes Show in Clips.
4. **Recording from the grid** (the REC model, [rec-arm.md](rec-arm.md)): a tap on an empty Loop cell arms it ("REC → Loops C · Drop"); REC readies it, PLAY (stopped) or the next bar line (playing) starts the first pass, REC again closes it on the bar line, and it plays. OVERDUB adds layers to the loop playing on the armed Loop track. With 8 loops in use or the memory full, REC says why. A new loop records its Loop track's input, never the other loops unless that input is Resample.
5. **The song:** a lane per Loop track in the Song view, Rec song recording loop launches as blocks linked to their cells, song mode playing them, and a block keeping the place in the loop it started at (an origin per block, a small looper change: a launch from a given place). **Print to audio track** bounces a loop's blocks into regions on an audio track (the looper's bounce already renders a loop as heard).
6. **The same loop in more than one row:** COPY a loop cell to another row of its Loop track and both play the same loop (a scene change between them keeps it playing without a break); a copy onto another Loop track makes a new loop, a copy of the audio, if memory and a free loop allow.

## Edge cases

- **On the frame:** a launch lands on the bar line's exact frame (part 1); a looper command otherwise applies at the start of the next period.
- **While recording:** a loop recording its first pass can't be launched elsewhere or copied; its cell shows it recording.
- **Undo:** a launch isn't an undo step, as for clips; recordings and edits keep the looper's undo.
- **Another tempo:** prints keep the pitch, as today.
- **The mixer:** the Loop tracks share today's Loop strip. A strip per Loop track, with its own effects, needs the looper to send each loop out on its own: later engine work.

## Tests

- Part 1, `rust/mpfree-looper/tests/looper.rs`: `a_launch_lands_on_its_bar_line_and_a_stopped_loop_keeps_its_place` (untouched up to the line's frame against a loop never stopped, fading from it, silent after, back in step from a later line, at once with no tick, a first pass turning a loop on; the same output at periods of 100, 128 and 333 frames) and `a_launch_the_song_jumps_past_lands_on_the_next_line` (and a loop past the last counted). `tests/no_alloc.rs` watches `launch`.
