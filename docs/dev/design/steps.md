# The step sequencer: a grid over a clip

**Status:** roadmap step T7, the model only, 2026-11-11. The code is [`rust/mpfree-seq/src/steps.rs`](../../../rust/mpfree-seq/src/steps.rs) (the grid), chance in [`rust/mpfree-play`](../../../rust/mpfree-play)'s player, and step edits as takes in `Takes::edit`. It is tested on the host. Its screens are the Play page's step grid (the app thread's, in `rust/mpfree-app/src/play_page.rs`) and the note editor's tracker lens and Euclidean fill ([note-editor.md](note-editor.md)). Nothing here has run on the MPC yet.

This builds on [playback](playback.md): clips, the player and take undo. STEP SEQ puts one sound's steps on the pads: [STEP SEQ: the pads as steps](step-pads.md).

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
- **`copy_steps(clip, sound, from, to, count)`** copies one sound's steps over others (COPY with STEP SEQ on: a step, or a page of 16), each keeping its nudge and its row's locks; an empty step copied clears its target.
- **On the Play page's pattern** (`mpfree-app`, `play_widgets::StepGrid`), the head's ‹ and › turn the pages shown, and its + and – send `StepAction::Length { steps, copy }`: + doubles the steps up to 128 (`MAX_STEPS`) with `copy`, – halves them down to 16 (`MIN_STEPS`) without. `TimeControl` makes it `resize`, one take, so UNDO brings back the steps and notes – took off.

A note that plays early on step 0 (just before the clip comes round) stays just before the end when the clip grows or shrinks.

A step of a sound (a pad, or a note number) holds the notes of that sound nearest the step's line:

- A note belongs to step `i` when its tick is from half a step before `i * size` to just under half a step after it. How far it is from the line is its **nudge**: negative is early, positive is late.
- A note half a step or more past the last step's line belongs to step 0, early. It plays just before the clip comes round again.
- `Grid::row(clip, sound)` gives every step of a sound, None where it is empty. A step (`Step`) shows its first note's **velocity**, **length**, **nudge** and **chance**, and how many notes it holds. A roll recorded live puts several notes in one step.
- `Grid::shown(clip)` is the whole grid a screen shows (`Shown`): every pad's row (0 to 15), then any other note the clip holds, then the whole line, with each step's lock count and each row's own lock values. `TimeControl::steps` hands it to the Play page and the step pads after every edit, and the bench times it on a full clip (refactor B1, [profiling.md](profiling.md#mpfree-bench)).

Every tick of every clip belongs to exactly one step, and every nudge a step allows lands back on that step. The tests check this for every size and odd clip lengths.

## Editing (`toggle`, `change`, `Edit`)

Edits never change the clip themselves. They look at the clip and return an `Edit`: the notes to take out and the notes to put in.

- **`toggle(clip, sound, i, velocity)`** is a tap on a step. A step with notes is cleared. An empty step gets one note on its line, a step long, at that velocity, always played.
- **`enter(clip, i, keys)`** is step entry of a held chord, as on the KeyStep Pro: the sounds or notes held (with their velocities) go on step `i` in place of what it held. No keys clears the step. `enter_steps(clip, steps, keys)` enters the same keys on several steps as one edit (a keyboard played while step pads are held), and `enter_key(steps)` names it, so the keys of a chord played one after another join one undo.
- **`change(clip, sound, i, Change)`** sets one thing on every note of a step:
  - `Velocity` (1 to 127)
  - `Length` in ticks
  - `Nudge` in ticks from the line. It is kept within the step, and the step's notes move together, so the first one lands there.
  - `Chance`, 1 to 100 %.
  - `Cond`, a trig condition (see below). One out of range leaves the step as it is.
  - `Ratchet`, 1 to 8 hits.
  - `Ramp`, a ratchet's velocity ramp, -126 to 126 (see "Ratchets").
  - `On`, on or off (see "A step that is off").
  - `Join`, its own attack, a tie or a slide (see "Ties and slides").
  - `Note`, a melodic step's note: the step's notes move so the lowest is this sound (a pad, or a note number) and the others keep their distance above it, the rows' locks on the step going with them (the step's own stay). On one row, it is refused onto a row the step already sounds; anywhere, past 127.

  Values out of range are clamped. An empty step, or one already set, gives an empty edit. `change_all(clip, i, Change)` does the same for every sound in the step, so a chord's velocity or nudge changes in one go, and `change_steps(clip, sound, steps, Change)` for several steps of one sound (step pads held together), joining one undo for a whole turn under `Change::key_steps`.
- **`Clip::apply(&Edit)`** applies an edit all or nothing: if a note to take out is not there (`MissingNote`), the clip is not the length the edit expects or a note would be past its new end (`ClipLength`), or a new note does not fit, the clip stays as it was. An edit that changes pages carries the length from and to.

## Chords (polyphony)

Every sound, or every note number on a melodic track, has its own row of steps, so polyphony is built in: a chord is several notes on the same step. `Grid::chord(clip, i)` reads a step across every sound, lowest first, which is how a KeyStep-style lane shows a melodic step. `enter` puts a held chord on a step, and `change_all` edits the chord as one.

**The whole line** (`LINE`, a row number no note has) is every sound of a step as one row: the step pads on a melodic track ([step-pads.md](step-pads.md#on-the-synth-the-line-and-the-pen)). `Grid::line(clip)` shows each step's notes as one `Step` (the first note, as `row` picks it, with how many there are, and its lowest and highest note, `low` and `high`), and `lock_line` counts every lock on each step. `clear_line(clip, from, to)` and `copy_line(clip, from, to, count)` clear and copy whole steps, every note and lock on them, as `clear_steps` and `copy_steps` do for one sound; `change_line(clip, steps, Change)` changes several whole steps as one edit.

## Undo (`Takes::edit`)

The control thread applies an edit with `Takes::edit(project, track, slot, &edit, join)`. Each edit is a take of its own, numbered on its own ("Edit 5" in the takes list), so UNDO and REDO take it back like a recorded take.

Turning a Q-Link through many values must not leave dozens of undo steps. `join` names what is being turned, such as `Change::Velocity(v).key(sound, i)` (or `key_all(i)` for a whole chord). An edit with the same key on the same clip joins the latest edit, as long as nothing has been undone since. One UNDO then takes the whole turn back. A note put in and taken out again within a turn leaves no trace.

Adding, taking off or copying a page is one edit, so one UNDO takes the page back, notes and length. A length change never joins a turn.

`Grid::clear_row(clip, sound)` is ERASE + a lane's name on the Play page: one edit that takes out every note and lock of that sound, so one UNDO brings the lane back.

**One action is one transaction** (step sequencer refactor A1, 20 Nov 2026), for the clip and for what plays:

- **The first step on a track with no clip** makes the clip off to the side (`Clip::new`, one page), works out the edit on it and, only if the edit changes something, puts the clip in with its notes as one take, `Takes::new_clip` (the clip in `made`, `Take::launched` set), then launches it. UNDO takes the clip away whole, which stops it. REDO puts it back, and `TimeControl` launches it again unless its track has another clip playing or waiting by then. An empty edit (a step past the clip's end) makes nothing, not even the clip.
- **If the play view can't be compiled after an edit**, `TimeControl::step_action` takes the edit back with `Takes::withdraw`. A take of its own goes, and what its push put out comes back: the oldest take of a full list, and the takes redo could bring. An edit that joined a turn comes out of that take alone, by the edit the other way round under the same join key, so the turn's earlier detents stay. The launches and cues go back as they were, the view as it was compiles again, and the error ends "the edit was taken back".

## Chance

`Note.chance` is 1 to 100 % (100 for every recorded note). A note below 100 % rolls each time it comes round. The roll is a hash of the note (its track, slot, tick and sound) and the absolute frame it would play on. So:

- it is the same for any period size, and on every computer;
- it differs on every pass of the clip, transport loops included.

A note left out is counted in `Stats::skipped`. It does not cut the same note still sounding.

## Trig conditions (`Cond`)

The Elektron set (design: https://claude.ai/artifact/S18hXQ3hSfJk1GaYnrwDZK). `Note.cond` says when a note plays; `Always` is the default, so recorded notes and older clips do not change. The step screen shows it as `cond.to_string()`.

| Condition | Plays when |
|---|---|
| `Always` (shown as nothing) | every pass |
| `Every { a, b }` ("A:B", B from 2 to 8, A from 1 to B) | on pass A of every B: 1:2 on passes 1, 3, 5; 4:4 on the fourth of every four |
| `Fill` / `NotFill` ("FILL", "!FILL") | FILL is on / off |
| `Pre` / `NotPre` ("PRE", "!PRE") | the track's last conditional note played / did not |
| `Nei` / `NotNei` ("NEI", "!NEI") | the last conditional note of the track to the left played / did not (the first track has none: NEI never plays, !NEI always does) |
| `First` / `NotFirst` ("1ST", "!1ST") | on the clip's first pass / on every other pass |

- **With chance.** Chance stays its own setting. The condition is decided first, and chance rolls only if it holds, so a 1:2 note at 50 % plays on half of the odd passes. `Stats::unmet` counts notes their condition left out, `Stats::skipped` those their chance did.
- **Conditional notes.** A note whose condition is A:B, FILL, 1ST or an opposite, or that is `Always` with a chance below 100 %, is conditional (`Cond::sets_last`): whether it played is its track's last result, which PRE and NEI read. Notes on PRE or NEI read it but do not set it, as on Elektron boxes, so a chain of them all follow the same note.
- **Passes** count from the tick the clip started on (`Launch::start`: the bar line it launched on, or its origin if it was launched while stopped), by song position, not by counting wraps. A locate or a transport loop jump lands on the right pass, the same at any period size, and a locate back to the start plays 1ST again. Passes before the start count 0, -1 and so on, and A:B wraps the same way. A relaunch counts again from its new start. A pass is one pass at the clip's own speed: a 1-bar clip at half speed has a 2-bar pass.
- **FILL** is one switch for the whole project: `Player::set_fill(on, offset)` (`TimeHost::set_fill` in the pad player) turns it on or off on an exact frame, so a fill can start mid-bar. The button is the pad player's: hold for a fill, double-press to latch (as NOTE REPEAT). FILL is not recorded.
- **Stepping.** A wheel or an arrow steps a condition along `Cond::all`: always, 1:2 to 8:8 (by the number of passes, then the pass), then FILL, !FILL, PRE, !PRE, NEI, !NEI, 1ST and !1ST, stopping at either end (`Cond::step`). The note editor's Condition row uses it.
- **Reset.** A track's last result starts at "not played" when its clip starts, and after a stop or a locate. A transport loop jump keeps it, so a PRE chain carries round a loop.

## Ratchets (`Note.ratchet`)

A ratchet of N (1 to 8; 1 is a plain note) plays the note N times, evenly spread over its length, each hit lasting 1/N of it. A 1/16 note with ratchet 4 is four 1/64 hits; stretching the note spreads them out.

- The condition and chance are decided once, on the first hit, so a ratchet plays whole or not at all.
- Its locks come once, before the first hit.
- **The velocity ramp** (`Note.ramp`, sequencer S6, Elektron's retrig velocity curve): the last hit's velocity less the first's, -126 to 126. The hits between step evenly (`mpfree_seq::ramped`, rounded), each kept to 1 to 127, so a ramp of -60 on four hits at 100 plays 100, 80, 60, 40. A ramp of 0, the default, gives every hit the note's velocity; on a note with no ratchet it does nothing, and it is kept for when the ratchet comes back.
- Swing and quantise move the first hit; the others keep their spacing from it. Hits less than a tick apart merge.
- A switch drops the hits from its bar line on; a stop, a locate or a loop jump drops the rest.

## A step that is off (`Note.on`)

Sequencer S6, for the redesign's Play page (a step grid of the armed track's pads; agreed with the redesign thread, 18 Nov). Since Jay's device run of 20 Nov 2026 a tap takes a step out (`toggle`) and only the step inspector's Off switches it off; a tap on a step that is off switches it back on. A step that is off keeps its notes and everything they hold: velocity, length, nudge, chance, condition, ratchet, ramp, join and its locks. The player leaves it out as it leaves out a note whose chance fails, with its own locks, but it sets no last result for PRE and NEI, and `Stats::off` counts it. `Change::On(false)` turns it off and `Change::On(true)` back on, one undoable edit each; `Step::on` shows it. Turning a step off is not a tap that clears it (`toggle`): nothing is lost.

## Ties and slides (`Note.join`)

Sequencer S6, the 303 way: a step can be joined to the note before it on its track instead of starting with its own attack (`Join::Attack`, the default). Both are set on the step that is joined, as the Elektron boxes set a slide on the trig it slides to.

- **`Join::Tie`** continues the same note: if its track has that note (pitch or pad) sounding as the tie starts, or ending on its tick, started before it, there is no new attack and that note lasts to the tie's end. A run of tied steps is one long note. The locks it holds stay through the tie, and the tie's own locks still play on its tick. With nothing to continue (a gap, or another pitch), it plays as any note.
- **`Join::Slide`** plays legato into its note: the track's notes sounding as it starts, or ending on its tick, end one tick after its note-on instead, so the two overlap. A synth in mono or legato mode with glide (the Analog engine's Glide, the Squelch engine's Slide, an outside synth's portamento over MIDI) glides there with no new envelope; a poly synth hears the overlap. A slide onto the same pitch ties. A chord of slides starting together doesn't end itself.
- Both are decided like any note: a tie or slide its condition or chance leaves out joins nothing, and the note before it ends where it would have.
- **Not STEP SEQ's tie.** Holding a step and tapping a later one stretches the held step's note (`Change::Length`, [step-pads.md](step-pads.md)). A tied step reaches the same long note from the other end, and also over the clip's seam (the last step tied into the first, from the second pass on), and it keeps a step of its own with its own locks, so a lock on a tied step changes the sound in the middle of the note.

## Parameter locks (`Lock`)

The owner chose the Elektron way on 11 Nov (12:21Z; design https://claude.ai/artifact/9gEoqcU5D4K6B47u7a3wgR). A step can set any parameter of its track for that step: the instrument's, a sampler pad's own, the track strip's, or an insert effect's. Parameters are named with `mpfree-param`'s `RelAddr` (the mod thread's crate, #102), relative to the clip's track, so a clip copied to another track locks that track.

- **The model.** `Clip.locks` holds `Lock { tick, sound, addr, value }`, at most 8192 a clip (`MAX_LOCKS`). `tick` is the tick of the step's note, so the lock moves with the note's nudge; a step with no note puts it on the step's line (a trigless lock). `sound` is the row it shows on: a pad or note number, or None for the whole step (a chord on a melodic track). `value` is in the parameter's own units and must be finite. `Clip::add_lock` and `remove_lock` are checked like notes.
- **How long it holds.** A lock plays on its tick, before the note there starts, and holds until the track plays a note that has no lock on that parameter on its own tick. Then the parameter goes back to where it was. A sampler pad's parameter (`OnTrack::Pad`) only ends at that pad's next note; anything else ends at any note of the track. A trigless lock holds until the next note in the same way. On a poly synth a lock sets the whole instrument, as on an Elektron. See [playback.md](playback.md#parameter-locks).
- **Editing.** `Grid::lock(clip, sound, i, addr, value)` returns an `Edit` that locks `addr` on step `i` of row `sound` (None for the whole step), in place of a lock of the same parameter there. It sits on the row's first note in the step (the chord's earliest note for the whole step), or on the line if the step is empty. `unlock` takes one parameter off and `clear_locks` all of a step's. `Grid::locks(clip, sound, i)` reads a step's locks, and `lock_row(clip, sound)` counts each step's locks for the dots on the step screen.
- **Locks go with their steps.** Toggling a step off, entering a chord over it, nudging it, or adding, copying or taking off a page carries the step's locks along with its notes, in the same `Edit` (`erase_locks`, `add_locks`). A nudge moves them with the note.
- **Several steps at once** (STEP SEQ's held step pads, sequencer S1 locks): `Grid::lock_steps(clip, sound, steps, addr, value)` locks `addr` on each step listed (a value of None takes it off them), with a note or not, as one `Edit`; `Grid::clear_step_locks(clip, sound, steps)` takes every lock of row `sound` off those steps and leaves their notes.
- **Undo.** Lock edits are takes like note edits. A Q-Link turned on a held step passes `lock_key(sound, i, addr)` as the join key, so the whole turn is one UNDO; on several held steps, `lock_key_steps(sound, steps, addr)` (the steps folded into the key) does the same.
- **Recording.** Knob moves while recording become locks on the step they belong to. See [playback.md](playback.md#knob-moves-become-locks).
- **FX moves go with the pages.** A clip's moves (Perform FX automation, see [playback.md](playback.md#fx-moves)) are on its ticks too. Copying a page copies the moves over its ticks; making a clip longer with copy repeats the last pages' moves, and making it shorter cuts them. The `Edit` keeps the lanes before and after (`Edit::moves`, a `MovesEdit`), so UNDO and REDO put them back, and `Clip::apply` refuses an edit whose `before` is not the clip's moves. A step's lock beats a lane on its step when they play.
- **Lock lanes on the screen** (the Play, Pads and Sequencer redesign's step 5, 18 Nov). The Sequencer tab's Steps show a kit's locks as lanes: one per (row, `RelAddr`) the clip locks, plus the lanes added with **+ Lane** (kept by the screen per track and clip slot until their lane is taken off), each under its sound, the whole step's (`LINE`, the track's mix and FX) after all the sounds. They read `StepClip.lock_values` and write only the existing `StepAction::Lock` (`Grid::lock_steps`): a tap locks a step at the setting's own value or takes the lock off, a drag or the held wheel turns it, the lane's × sends one `Lock { value: None }` over every step it holds (one take). No engine change. A sound's lock on a step with no note of its row is a trigless lock, which this page's rules already play. The app's side: `play_page.rs` (`LockLane`, `lock_lanes`, `tap_lock`, `turn_lock`, `clear_lock_lane`, `lane_choices`), `play_widgets.rs` (`LockCells`, `paint_lock_lane`, the drag), and the Lanes list in `gui.rs`; see [controls.md](../guides/controls.md) (`steps/lock`, `lanes/row`).
- **Synth and gear lanes** (step 5b, 18 Nov; board 4). A synth track's Steps (scale rows) show its lock lanes under the notes, as many as leave three notes, the rest a swipe away (`StepGrid::pitched_parts`). Their settings are the synth's first Q-Link set and any synth parameter with a lane (`OnTrack::Inst`, on the line). A track with a Tracks out (MIDI section) plays its gear instead of its own sound, so its lanes are the gear's controls only (`OnTrack::Midi`, ids from `mpfree_param::midi`: CC 0 to 119, `BEND`, `PRESSURE`), named by the port's device profile, else by `common_cc_name`; the Lanes card's Another CC takes any CC by number. These locks are the existing `StepAction::Lock` on the line; the mod engine already sends `Owner::Midi` values out with the step (`modmix::midi_param`, `midi_out.rs`), so no engine change. The app's side: `Player::lock_knobs` (`pad_lane_knobs`, `synth_lane_knobs`, `gear_lane_knobs`), `SoundKind::Midi` and `PlayPage::gear`.
- **With conditions and chance.** A note its condition or chance leaves out leaves out its own locks too: the locks on its tick and row, and a whole-step lock only when every note on its tick is left out. So a 1:2 step with a filter lock opens the filter only on the passes it plays. Trigless locks always play. A left-out note ends nothing, so the lock before it keeps holding.

## Euclidean fill (`Euclid`, `Grid::euclid`)

Euclidean rhythms spread a number of hits as evenly as they go over a number of steps: 3 of 8 is x..x..x., 4 of 16 a hit every beat, 5 of 8 the cinquillo. They are the Digitakt's Euclidean mode, which is the reference users know. The note editor writes them from its Edit options ([note-editor.md](note-editor.md#euclidean-fill)).

- **The pattern.** `Euclid { hits, steps, rotate }`: up to 64 steps (`MAX_EUCLID_STEPS`, four pages), hits from 0 to the steps, and the whole pattern turned `rotate` steps later. Step `j` before the rotation has a hit when `j * hits mod steps < hits`. That is Bresenham's line: it starts on a hit and its gaps differ by at most one step, so it is the same rhythm as the Euclidean (Bjorklund) algorithm's, possibly turned. `Euclid::fit` keeps the three in range, and `pattern` gives it as text for the log.
- **Writing it.** `Grid::euclid(clip, sound, euclid, velocity)` returns one `Edit` that makes the sound's row the pattern, repeated from the clip's first step to its last. A step with a hit keeps the notes it has (their velocity, nudge, chance and locks), or gets a note on its line, a step long, at `velocity`. A step without one loses its notes and their locks. Other sounds are left alone. A pattern of 12 steps on a 16-step clip runs on into the next bar's start, for polymeter against the other rows.
- **Undo.** It is one take. The note editor passes one join key for its three rows, so a run of turns is one UNDO.

## Mutate with restore (`mpfree_seq::mutate`)

Sequencer S6, the Torso T-1's mutate and the KeyStep's Mutate and Original: a variation of a clip's notes that never loses the clip it came from. The clip card's Mutate row drives it (layout spec section 4j).

- **The original.** `Clip::original: Option<Box<Original>>` holds the clip's notes, locks, length and moves as they were before its first Mutate, and `variation` (how many Mutates it has had). `Clip::check` checks it as the clip's own (its notes name pressure in the clip's `touch`, which only grows); a loop's cell has none.
- **Mutate.** `Clip::mutated(seed, amount, pitches)` varies the original (the clip itself before the first Mutate), never the variation, so edits made since are dropped and a second Mutate is a new take on the same source. About one note in four varies (`Amount::Little`), one in two with SHIFT (`Amount::More`), at least one; steps that are off stay. Each picks one of the ways that apply to it: velocity 10 to 30 up or down; chance (100 % to 50 or 75 %, a chance back to 100 %); ratchet (a plain note to 2 or 3 hits of at least `MIN_HIT` 30 ticks, a ratchet back to 1); pitch, only on a melodic track (`Pitches::Own`: the next pitch up or down among those the clip plays, so it stays in the clip's key; `Pitches::Key` takes a scale's notes; `Pitches::Pads` never moves a pad). A tied or slid note and the note a tie continues keep their pitch and hits, so the join still holds; a lock on a moved note's row moves with it. The rest (length, other locks, moves) is the original's, the clip's settings its own. It is a pure function of the original, the seed and the amount: the control thread seeds it with the edit count, the variation and the slot, so each Mutate sounds new and a test repeats.
- **Restore.** `Clip::restored()` puts the original back and keeps none.
- **Undo.** Both are takes of their own, `Takes::replace_clip` with `Replaced::Mutate` or `Replaced::Restore` (Reload clip's swap, which is `Replaced::Reload`): the clip before in `removed`, the new one in `made`, so UNDO puts back the clip with the original it had. The Takes list says "Mutate 4 · variant 2" and "Restore 5 · the original". `TimeControl::mutate` and `restore` refuse while recording, as Reload does, and act at once (a Mutate is an edit like a step's, not a pattern change that waits for a line).
- **Saved.** As `original <length> <variation>` and the original's notes, locks, lanes and points, each a clip line with `original` before it, after the clip's own lines (`save.rs`). An older build skips them and keeps the variation playing.

## Tests

- `mpfree-seq` unit tests for the step's own settings (S6): a ratchet's ramp steps its hits' velocity and stays in range, and a clip refuses one past 126; a step turns off keeping its notes' every value and its locks, on every note of its sound, a chord together, and back on as it was; ramp and join changes, each its own undo; `off`, `ramp:` and `tie`/`slide` read back from the saved text, an older note takes the defaults, a newer build's word is skipped and a bad ramp refuses the part.
- `mpfree-seq` unit tests: each condition on passes before and after the start, FILL, PRE and NEI; which notes are conditional; how conditions show; the order a wheel steps them in (`Cond::all`, `Cond::step`); a clip refuses a condition or ratchet out of range. `steps`: a step takes a condition and a ratchet on every note of its sound, out of range is refused or clamped, and each joins its own undo.
- `mpfree-play` `tests/trigs.rs` (21) checks how conditions, ratchets, steps that are off, ramps, ties and slides play: see [playback.md](playback.md#tests).
- `mpfree-seq` `steps` unit tests (26), three of them for locks: a lock sits on its step's note (a chord's earliest) or on the line, and replaces the one there; locks go with their steps when they are toggled off, nudged, entered over or copied with a page; a lock out of range is refused. The others: every tick has one step and comes back from it, for every size and odd clip lengths; a row shows the first note of a step and counts the rest; toggle on and off; changes and their limits, nudging a group of notes together; nudges reaching back over the clip's end on step 0; `Clip::apply` is all or nothing, length changes too; pages added (empty and copied, up to 64 steps and on), taken off and set to 13 steps, with an early note staying before the end; a short clip repeating to fill a copied page; a page copied over another; a held chord entered on a step, read back and nudged as one; the whole line, a chord shown as one step and cleared, copied and changed whole with its locks; a step's Note moving its chord (and one row) with the rows' locks, refused past 127 and onto a row the step sounds, and the keys' undo key; Euclidean patterns (named ones, rotation, range, and for every hits-of-steps up to 32, exactly that many hits with gaps at most one step apart); a Euclidean fill writing one row, keeping a step's note, taking off a step's note with its lock, leaving other rows, and undoing.
- `mpfree-play` `takes` unit tests: step edits are takes; a turn of velocity joins one edit; an edit after an undo starts a new one; a recorded note never joins an edit; pages added and taken off undo and redo to the clip exactly as it was; locks undo and redo with their steps; a first step's clip is one edit that undo takes away whole and that remembers its launch; a withdrawn edit leaves the clip and the takes as they were (a full list's oldest take and the redo list back, a joined detent out of its turn alone).
- `mpfree-app` `time::tests::the_first_step_its_pattern_and_its_launch_are_one_take`: a step past the end makes nothing; UNDO after the first step leaves the slot empty and stopped, REDO brings it back playing and it plays; a view that can't compile takes back a first step and a length change, launches and all.
- `mpfree-bench` (refactor B1): `a_tap_goes_in_place_and_a_detent_compiles_on_a_full_clip` (on a full 8,192-note clip a tap's note goes out and back in place, the view following the clip, and every lock detent compiles, the first too) and `the_grid_case_shows_every_step_of_the_full_clip` (65 rows, 8,320 steps filled); `mpfree-seq`'s `the_grid_shown_has_every_pad_then_other_notes_then_the_line` (pads 0 to 15, then other notes, then the line; a whole step's lock counted on every row). Their speed is in the instruction gate ([profiling.md](profiling.md#mpfree-bench)).
- `mpfree-play` `tests/locks.rs` (9) checks how locks play, and `tests/record.rs` how knob moves are recorded: see [playback.md](playback.md#tests).
- `mpfree-seq` `mutate` unit tests (7): a Mutate varies some notes and nothing else, the same for the same seed, and Restore puts them back; More varies about twice as many; a second Mutate varies the original whatever was edited since; every way stays in range; a melodic note moves in its key or the clip's own pitches, its lock with it, while ties hold and pads never move; steps that are off stay, and an empty clip or a loop's cell has nothing to mutate; the clip's check covers its original. `save::a_mutated_clips_original_reads_back`: read back whole, skipped by an older build, refused when broken. `takes::mutate_and_restore_are_takes_undo_takes_back`; in the app, `time::tests::mutate_varies_the_clip_and_restore_brings_it_back` and the time-ui's `the_clip_cards_mutate_row_mutates_and_restores`.
- `tests/play.rs`: a note at 50 % is left out on some passes and not others, differently on every pass and the same at every period size.
- `tests/period_size.rs` and `tests/no_alloc.rs` include notes with chance below 100 %; `tests/no_alloc.rs` and `tests/trigs.rs` also trig conditions, ratchets and FILL.

## Listening

`cargo run --release -p mpfree-play --example steps_demo` programs a beat at 96 BPM only through the step grid, each tap and change an undoable edit, and writes `~/mpfree-work/steps-demo.wav`:

- a new clip of one page: kicks on 1/16 steps; snares on 2 and 4, nudged 24 ticks late so they sit back; hats on every 1/16, the off-beats quieter and every fourth one at 50 %;
- a copied page makes bar 2, and one of its kicks moves;
- one hat turned down from 100 to 70 in a single edit;
- a rising ghost-snare roll on 1/16 triplets, and a clank at 35 % on the last 16th.

It prints the grid, checks where every note was heard on every pass, checks on a copy that UNDO takes the edits back newest first, the whole velocity turn in one step and the copied page in another, and checks that the events are identical at periods of 1, 128 and 4096 frames.

## Next

- On the Play page's step grid (the app thread's): Q-Links for a held step's velocity, length, nudge, chance, condition and ratchet, and a FILL button.
- Parameter locks reach the mixer's strips and inserts through the pad player's mod engine (`rust/mpfree-app/src/modmix.rs`). A lock on a pad's sound or an instrument is counted and dropped until those engines take parameter events. The step grid can show locks as dots (`lock_row`) and edit a held step's locks on the Q-Links.
- A velocity ramp for ratchets (Elektron's retrig velocity curve), as a second setting.
- Ties: a held key in step entry stretching a note over the next steps.
