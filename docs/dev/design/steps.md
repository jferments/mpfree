# The step sequencer: a grid over a clip

**Status:** roadmap step T7, the model only, 2026-11-11. The code is [`rust/mpfree-seq/src/steps.rs`](../../../rust/mpfree-seq/src/steps.rs) (the grid), chance in [`rust/mpfree-play`](../../../rust/mpfree-play)'s player, and step edits as takes in `Takes::edit`. It is host-only and tested. There is no screen for it yet: where step editing goes (the Play page) waits for Jay's pick of a mockup, through the layout owner. Nothing here has run on the MPC.

This builds on [playback](playback.md): clips, the player and take undo.

## Steps are a view of a clip

A step sequencer here is not a second kind of pattern. It is the clip's own notes, shown on a grid. So:

- notes recorded live show up on the steps, and a step entered on the grid plays like any recorded note (the clip's quantise and swing still apply on playback);
- a clip of any length works, and clips of different lengths still drift against each other;
- a step pattern can be recorded over, captured, launched on the bar line and undone like any clip.

## The grid (`Grid`)

A `Grid` cuts a clip into steps of one size: 1/4, 1/8, 1/8 triplet, 1/16, 1/16 triplet or 1/32 (`SIZES`, in ticks), 16 to a page (`PAGE`). The last step may be cut short by the clip's end.

## Pages

There is no fixed number of steps. A clip has as many steps as its length holds, and as many pages of 16 as those steps need, as on the KeyStep Pro or the Digitakt:

- **A new step clip is one page**, 16 steps (`Grid::page_length`: one bar of 1/16s).
- **`add_page(clip, copy)`** adds 16 steps. They are empty, or with `copy` they repeat the page before them (a clip shorter than a page repeats whole). Four pages make 64 steps; a clip can go on to 256 bars (`Grid::max_steps`, 4096 steps of 1/16).
- **`remove_page(clip)`** takes the last 16 steps off, with their notes. A clip keeps at least one page.
- **`resize(clip, steps, copy)`** sets any number of steps, such as 13 against a 16-step track for polymeter.
- **`copy_page(clip, from, to)`** copies one page over another, step for step.

A note that plays early on step 0 (just before the clip comes round) stays just before the end when the clip grows or shrinks.

A step of a sound (a pad, or a note number) holds the notes of that sound nearest the step's line:

- A note belongs to step `i` when its tick is from half a step before `i * size` to just under half a step after it. How far it is from the line is its **nudge**: negative is early, positive is late.
- A note half a step or more past the last step's line belongs to step 0, early. It plays just before the clip comes round again.
- `Grid::row(clip, sound)` gives every step of a sound, None where it is empty. A step (`Step`) shows its first note's **velocity**, **length**, **nudge** and **chance**, and how many notes it holds. A roll recorded live puts several notes in one step.

Every tick of every clip belongs to exactly one step, and every nudge a step allows lands back on that step. The tests check this for every size and odd clip lengths.

## Editing (`toggle`, `change`, `Edit`)

Edits never change the clip themselves. They look at the clip and return an `Edit`: the notes to take out and the notes to put in.

- **`toggle(clip, sound, i, velocity)`** is a tap on a step. A step with notes is cleared. An empty step gets one note on its line, a step long, at that velocity, always played.
- **`enter(clip, i, keys)`** is step entry of a held chord, as on the KeyStep Pro: the sounds or notes held (with their velocities) go on step `i` in place of what it held. No keys clears the step.
- **`change(clip, sound, i, Change)`** sets one thing on every note of a step:
  - `Velocity` (1 to 127)
  - `Length` in ticks
  - `Nudge` in ticks from the line. It is kept within the step, and the step's notes move together, so the first one lands there.
  - `Chance`, 1 to 100 %.

  Values out of range are clamped. An empty step, or one already set, gives an empty edit. `change_all(clip, i, Change)` does the same for every sound in the step, so a chord's velocity or nudge changes in one go.
- **`Clip::apply(&Edit)`** applies an edit all or nothing: if a note to take out is not there (`MissingNote`), the clip is not the length the edit expects or a note would be past its new end (`ClipLength`), or a new note does not fit, the clip stays as it was. An edit that changes pages carries the length from and to.

## Chords (polyphony)

Every sound, or every note number on a melodic track, has its own row of steps, so polyphony is built in: a chord is several notes on the same step. `Grid::chord(clip, i)` reads a step across every sound, lowest first, which is how a KeyStep-style lane shows a melodic step. `enter` puts a held chord on a step, and `change_all` edits the chord as one.

## Undo (`Takes::edit`)

The control thread applies an edit with `Takes::edit(project, track, slot, &edit, join)`. Each edit is a take of its own, numbered on its own ("Edit 5" in the takes list), so UNDO and REDO take it back like a recorded take.

Turning a Q-Link through many values must not leave dozens of undo steps. `join` names what is being turned, such as `Change::Velocity(v).key(sound, i)` (or `key_all(i)` for a whole chord). An edit with the same key on the same clip joins the latest edit, as long as nothing has been undone since. One UNDO then takes the whole turn back. A note put in and taken out again within a turn leaves no trace.

Adding, taking off or copying a page is one edit, so one UNDO takes the page back, notes and length. A length change never joins a turn.

## Chance

`Note.chance` is 1 to 100 % (100 for every recorded note). A note below 100 % rolls each time it comes round. The roll is a hash of the note (its track, slot, tick and sound) and the absolute frame it would play on. So:

- it is the same for any period size, and on every computer;
- it differs on every pass of the clip, transport loops included.

A note left out is counted in `Stats::skipped`. It does not cut the same note still sounding.

## Parameter locks (`Lock`)

The owner chose the Elektron way on 11 Nov (12:21Z; design https://claude.ai/artifact/9gEoqcU5D4K6B47u7a3wgR). A step can set any parameter of its track for that step: the instrument's, a sampler pad's own, the track strip's, or an insert effect's. Parameters are named with `mpfree-param`'s `RelAddr` (the mod thread's crate, #102), relative to the clip's track, so a clip copied to another track locks that track.

- **The model.** `Clip.locks` holds `Lock { tick, sound, addr, value }`, at most 8192 a clip (`MAX_LOCKS`). `tick` is the tick of the step's note, so the lock moves with the note's nudge; a step with no note puts it on the step's line (a trigless lock). `sound` is the row it shows on: a pad or note number, or None for the whole step (a chord on a melodic track). `value` is in the parameter's own units and must be finite. `Clip::add_lock` and `remove_lock` are checked like notes.
- **How long it holds.** A lock plays on its tick, before the note there starts, and holds until the track plays a note that has no lock on that parameter on its own tick. Then the parameter goes back to where it was. A sampler pad's parameter (`OnTrack::Pad`) only ends at that pad's next note; anything else ends at any note of the track. A trigless lock holds until the next note in the same way. On a poly synth a lock sets the whole instrument, as on an Elektron. See [playback.md](playback.md#parameter-locks).
- **Editing.** `Grid::lock(clip, sound, i, addr, value)` returns an `Edit` that locks `addr` on step `i` of row `sound` (None for the whole step), in place of a lock of the same parameter there. It sits on the row's first note in the step (the chord's earliest note for the whole step), or on the line if the step is empty. `unlock` takes one parameter off and `clear_locks` all of a step's. `Grid::locks(clip, sound, i)` reads a step's locks, and `lock_row(clip, sound)` counts each step's locks for the dots on the step screen.
- **Locks go with their steps.** Toggling a step off, entering a chord over it, nudging it, or adding, copying or taking off a page carries the step's locks along with its notes, in the same `Edit` (`erase_locks`, `add_locks`). A nudge moves them with the note.
- **Undo.** Lock edits are takes like note edits. A Q-Link turned on a held step passes `lock_key(sound, i, addr)` as the join key, so the whole turn is one UNDO.
- **Recording.** Knob moves while recording become locks on the step they belong to. See [playback.md](playback.md#knob-moves-become-locks).

## Tests

- `mpfree-seq` `steps` unit tests (15), three of them for locks: a lock sits on its step's note (a chord's earliest) or on the line, and replaces the one there; locks go with their steps when they are toggled off, nudged, entered over or copied with a page; a lock out of range is refused. The others: every tick has one step and comes back from it, for every size and odd clip lengths; a row shows the first note of a step and counts the rest; toggle on and off; changes and their limits, nudging a group of notes together; nudges reaching back over the clip's end on step 0; `Clip::apply` is all or nothing, length changes too; pages added (empty and copied, up to 64 steps and on), taken off and set to 13 steps, with an early note staying before the end; a short clip repeating to fill a copied page; a page copied over another; a held chord entered on a step, read back and nudged as one.
- `mpfree-play` `takes` unit tests: step edits are takes; a turn of velocity joins one edit; an edit after an undo starts a new one; a recorded note never joins an edit; pages added and taken off undo and redo to the clip exactly as it was; locks undo and redo with their steps.
- `mpfree-play` `tests/locks.rs` (9) checks how locks play, and `tests/record.rs` how knob moves are recorded: see [playback.md](playback.md#tests).
- `tests/play.rs`: a note at 50 % is left out on some passes and not others, differently on every pass and the same at every period size.
- `tests/period_size.rs` and `tests/no_alloc.rs` include notes with chance below 100 %.

## Listening

`cargo run --release -p mpfree-play --example steps_demo` programs a beat at 96 BPM only through the step grid, each tap and change an undoable edit, and writes `~/mpfree-work/steps-demo.wav`:

- a new clip of one page: kicks on 1/16 steps; snares on 2 and 4, nudged 24 ticks late so they sit back; hats on every 1/16, the off-beats quieter and every fourth one at 50 %;
- a copied page makes bar 2, and one of its kicks moves;
- one hat turned down from 100 to 70 in a single edit;
- a rising ghost-snare roll on 1/16 triplets, and a clank at 35 % on the last 16th.

It prints the grid, checks where every note was heard on every pass, checks on a copy that UNDO takes the edits back newest first, the whole velocity turn in one step and the copied page in another, and checks that the events are identical at periods of 1, 128 and 4096 frames.

## Next

- The step screen, once Jay picks where it goes: pads or a lane per sound, 16 steps a page with page buttons, Q-Links for velocity, length, nudge and chance.
- Parameter locks reach the sound once the mod thread's resolver (`mpfree-mod`, #102) takes the player's `Event::Lock` and `Event::Unlock`. The pad player ignores them until then. The step screen shows them as dots (`lock_row`) and edits a held step's locks on the Q-Links.
- Trig conditions (every 2nd pass, first pass only) and ratchets. They add fields to the note, like chance.
- Ties: a held key in step entry stretching a note over the next steps.
