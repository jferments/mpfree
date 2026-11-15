# Playback: clips on the transport

**Status:** roadmap step T2 (merged in #47), a track through the arp (#51), the clip recorder (T3, #55), replace recording (#56) and take undo, 2026-11-11; pad pressure in clips, 2026-11-12. The code is [`rust/mpfree-seq`](../../../rust/mpfree-seq) (the model) and [`rust/mpfree-play`](../../../rust/mpfree-play) (the playback engine). Both are tested on the host. The pad player runs them in `TimeHost` (`rust/mpfree-app/src/time.rs`): the player and the recorder on the audio half, the project and every compile on the control half ([time-threads.md](time-threads.md)). The app's first live test on the MPC (12 Nov, main c8ef977) skipped most of its phases, so clips, recording and everything newer are still to be tried there.

This builds on the [transport](transport.md). The full plan is the time engine design document. Its mockups and skill tree are in the mpfree-claude notes (`sequencer/`).

## The model (`mpfree-seq`)

A project holds up to 128 tracks (`MAX_TRACKS`; Jay, 14 Nov 2026: "Up to 128, like Akai's MPC", raised from 16). A track has a destination, an optional swing, a mute flag and 16 clip slots. A clip has:

- a length in its own ticks (from a 1/32 note to 256 bars)
- a speed (1/8 to 4 times)
- an optional swing and an optional quantise
- its notes, each with a tick, a length, a note or pad number, a velocity, a chance (1 to 100 %, 100 for a recorded note), a trig condition (`Always` for a recorded note), a ratchet (1 to 8 hits, 1 for a recorded note) and its pad pressure, if any (`Note::touch`, see "Pad pressure" below)
- its pad pressure (`Clip::touch`): every note's pressure values, one run a note

These are plain values, edited on the control thread and checked with `check()`.

- **Everything is a clip.** A step pattern and a recorded phrase are the same thing. The step grid is a view of a clip, not its own data: see [steps.md](steps.md).
- **Any length.** Clips of different lengths drift against each other (polymeter), and no clip is tied to 16 steps.
- **Quantise is non-destructive.** Notes keep their recorded ticks, and quantise (grid and strength) applies on playback, so it can be changed or undone later.
- **Swing:** the clip's own swing wins, then the track's, then the transport's.

Parameter locks, trig conditions and ratchets are in the model too (see [steps.md](steps.md)). So are FX moves: a clip's `moves` holds a lane of points per parameter (the FX thread's `mpfree-moves`), see "FX moves" below. Scenes ([scenes.md](scenes.md)) and the arrangement ([arrange.md](arrange.md)) add to this model without changing it. Audio clips come in a later step.

## Compiling (`PlayView`)

The control thread compiles the project and the playing clips (`Launch { track, slot, origin, start }`) into a read-only `PlayView`. `start` is the tick the clip started on, which trig conditions count passes from: the origin, unless the origin was anchored back from a later bar line. For each clip it:

- sorts the notes
- applies quantise and speed
- picks the swing to use

The audio thread only reads the view. After an edit, the control thread compiles a new one and swaps it in, the same pattern as the sampler's `EngineView`. A clip plays from `origin` on and loops.

**Recording edits in place.** Recording is the exception, because it changes a clip on every note. A recorded or erased note or lock goes into the view in place, where compiling again would put it:

- `PlayView::add_note(track, slot, &note)` and `add_lock(track, slot, &lock, replaced)` for what `Takes::add` and `Takes::add_lock` put in (`add_lock` returns the lock it replaced);
- `remove_note` and `remove_lock` for what `Takes::erase` and `Takes::erase_lock` took out.

They update the clip playing and the clip cued, whichever shows that slot. `PlayView::make_room(notes, locks)` sets room aside in the armed track's clips after a compile, so these never allocate; each returns false when the view has no room left, or does not hold what is erased, and then the host compiles instead. A note goes after the notes at its place, as the clip's newest, so the order is a compile's (a stable sort), and a test compares the two after thousands of random edits.

**Recorded changes, numbered (`mpfree_play::recorded`).** For a time engine on two threads ([time-threads.md](time-threads.md)), the view and the project must each make a recorded change without asking the other. Each is a `Rec`: a note taken (with its pressure run beside it), a note erased, a lock taken, a lock erased, or a part of a gesture (with its points). `PlayView::record` makes it to the view in place and `Takes::record` to the project, and they decide alike, from the same rules in `mpfree-seq`:

- a note's pressure goes on the end of its clip's pressure when it fits (`touch_fits`: in order, and no further than `MAX_TOUCH_POINTS`), else the note has none; the run stays even if the note is then refused, as `Takes::add_touched` leaves it;
- a note goes in when the clip can hold it (`Note::fits`, and fewer than `MAX_NOTES`); one past the clip's end, say, is left out by both;
- a lock takes the place of the lock in its place (`Lock::same_place`: the same parameter, tick and row), else goes in as the newest when the clip can hold it (`Lock::fits`, fewer than `MAX_LOCKS`);
- an erase of a note or lock the view no longer holds needs nothing, since an edit took it out of the project too.

`PlayView::record` returns false only when there is no room in place, and then the host compiles. The pad player makes every recorded change this way, and numbers it.

**What a clip plays, stamped (`ClipView::stamp`).** Each compiled clip carries a 64-bit hash of what it plays: its length, speed, quantise, swing, and every note, lock, pressure value and moves lane, but not where it plays (slot, origin, start). It is a sum of a hash per note, lock, pressure value (with its place) and lane, so a recorded change in place adds or takes away its own share and the view still equals a fresh compile, stamp and all; a recorded gesture hashes its lane again before and after. `Player::now_playing(view, track, tick)` gives a host what a track plays now, as `NowPlaying { slot, origin, song_len, stamp, muted }`: the clip launched or switched in, else in song mode the placement under `tick`. The mixer's frozen strips follow it ([FX](fx.md), "In the app"): a new stamp thaws the strip, a new origin moves its take, and a mute holds it quiet.

**What a view includes.** A view can say what it was compiled with (`PlayView::set_includes(Includes { recorded, switches })`): the number of the last recorded change made to the project, and how many started cues were applied on each track. A view compiled a little late then catches up before it plays: `Recent` keeps the last `RECENT` (1024) changes with their points (`RECENT_TOUCH` pressure values and `RECENT_MOVES` move points, four full notes' or parts' worth), and `Recent::catch_up(view, recorded)` makes the ones after `recorded` to it in order. If the view needs changes `Recent` no longer holds (`Behind::TooOld`), or has no room for one (`Behind::NoRoom`), the host keeps playing the view it has, which holds everything, and compiles again.

## Launching on the bar line (cues, T9)

A clip launched while the music plays does not cut in at once. It waits for the next bar line, the way the MPC's Next Sequence mode switches sequences at the end of a bar.

- **A cue.** `PlayView::compile_cued` takes the launches and the `Cue { track, slot }` values waiting on the tracks, one per track. `slot: None` is a stop.
- **The next bar line the playhead plays.** That is usually the next bar. In a loop whose end comes first, it is the loop start's bar line, right after the jump back. A cue made while stopped waits for the first bar line played after PLAY.
- **The switch.** On that tick the track's clip stops and the cued clip starts from its first tick. Notes of the old clip still sounding end on the same tick, before the new clip's notes start. A stop does the same with no new clip.
- **Then it keeps its place against the song.** A started clip's origin is `anchored`: the bar line, moved back by whole passes to before any position the song can reach (`EARLIEST`, about 12 days at 120 BPM). So after a loop jump back or a locate it plays on in phase, and doesn't go silent until it reaches the bar line again.
- **The control thread catches up.** `Player::started()` names each switch (track, slot, the tick, the new origin). The app applies it with `Started::apply`, which makes the clip the track's launch (starting on that tick) and removes the cue, and compiles again. Until that view arrives, the player and the recorder carry the switch themselves, so when the view arrives changes nothing that plays. A view that says what it includes stops the switch once it shows as many switches on the track as the player made (`Includes::switches`); one that doesn't stops it once its cue is gone. The count matters on two threads: a late view can both apply the switch and cue the same clip again, and the player must take that cue for a new one.
- **Recording.** On the armed track, notes held at the switch end on its bar line, in the clip that was playing. Notes played from that tick on go into the new clip. A switch ends a replace pass, and the take goes on as overdub.
- **Song mode.** `PlayView::set_song(project, on)` adds the arrangement's placements as regions on each track. A track with no clip launched plays the song, and a launched clip takes the track over until it stops: see [arrange.md](arrange.md).
- **A mute with the cue.** A cue can carry its track's mute (`PlayView::set_cue_mute`, for a scene's mutes). The player switches the track's mute on the cue's bar line, and `Started::mute` tells the app. Scenes and follow actions use this: see [scenes.md](scenes.md).

## Playing (`Player::process`)

Each period the player walks the transport's spans and sends events in frame order to a `Sink`. Each event says which track it came from and which destination it is for. The host maps destinations to the sampler, the arp or MIDI out.

- **Clip positions come from the tick.** A clip's position is worked out from the song tick every time (origin, length, speed), never counted up. After a loop jump, a locate or a tempo change, every clip is in the right place.
- **Whole ticks.** A note's song position after quantise, speed and swing is rounded to the nearest tick. A tick is 1/960 of a beat, 0.26 ms at 120 BPM, the same resolution as the MPC. A note plays when the transport plays its tick, on that tick's frame. The transport plays every tick exactly once between jumps, whatever the period size, so no note is lost or doubled at a period edge or a tempo change.
- **Note-offs** come on the tick where the note ends.
  - A loop jump, a locate or a stop ends every sounding note on that frame, and the sound keeps its release tail.
  - STOP while stopped (panic) also sends `AllOff`.
  - If a note starts while the same note is still sounding on the same track, the old one ends first, on the same frame.
- **Count-in and mute.** Nothing plays during a count-in, or on a muted track.
- **Chance.** A note below 100 % rolls each time it comes round, on a hash of the note and the frame it would play on: the same for any period size, different on every pass. Notes left out are counted in `Stats::skipped`.
- **Trig conditions.** A note's condition is decided before its chance; `Stats::unmet` counts the notes it leaves out. A:B and 1ST count passes from the clip's `start` by song position, so they are right after a locate or a loop jump. `Player::set_fill(on, offset)` turns FILL on or off on an exact frame of the coming period. Each track keeps its last result for PRE, and NEI reads the track to its left. The player decides a span's notes in the order they play (tick, then track, then the clip's order), so the results are the same for any period size. A track's result starts again when its clip starts, and on a stop or a locate, but not on a loop jump. See [steps.md](steps.md#trig-conditions-cond).
- **Ratchets.** A ratchet's first hit is decided like any note; if it plays, its later hits wait in a table (256 for all tracks) and play on their ticks, after the note-offs and locks there and before the new notes. A stop, a locate or a loop jump drops them, a switch drops those from its bar line on, and a muted track drops its own.
- **What conditions cost.** Counted in instructions with callgrind on the bench's Time case (8 clips of 16ths with 3-note chords and 2 locks a step, through the arp): 7,380 instructions a period before conditions existed, 8,060 with them built in and none used (+9 %), and 7,580 with a condition on every note, a ratchet of 3 on every fourth step and FILL switching (fewer notes play). A note with no condition and full chance skips the deciding.
- **Real-time safe.** There is no allocation, lock or panic in `process`. A full table is counted in `Stats::dropped`, never dropped silently: 256 sounding notes, 256 note starts per span.

## Parameter locks

The player sends a clip's locks as `Event::Lock { addr, value }`, with `addr` the full `ParamAddr` on the clip's track, and ends them with `Event::Unlock { addr }`. The mod thread's resolver (`mpfree-mod`) applies them: a lock sets the parameter, and an unlock lets it go back to its knob, automation and modulation.

- **Before the note.** A lock is sent on its tick's frame, after the note-offs there and before the note-ons, so the step's note starts with the locked value.
- **Until the next note without one.** The lock holds until the track plays a note with no lock on that parameter on its own tick; the `Unlock` comes on that note's frame, before it. A sampler pad's parameter is only ended by that pad's notes. A trigless lock (no note on its tick) holds the same way. A note its condition or chance leaves out ends nothing.
- **A left-out note leaves out its locks.** The locks on its tick and row are not sent, and a whole-step lock is not sent when every note on its tick is left out. Trigless locks always play. The player decides the span's notes before it sends anything, so it knows this when it reaches the lock.
- **Jumps and switches.** A stop, a locate, a transport loop jump or a panic unlock everything, after the notes end. When a cue switches a track's clip, the old clip's locks end on the switch, before the new clip's start. A muted track, or one with nothing playing, lets its locks go at once and sends none.
- **Every Unlock follows a Lock.** The player keeps what it has locked (up to 256 held locks, 256 lock starts a span; more is counted in `Stats::dropped`). `Stats::locks` counts locks sent. A pass being replaced sends none of its locks: it erases them (see "Replace and overdub").

## FX moves

A clip can hold moves: Perform FX gestures (the XY pad, the DJ knob, a pad's Hold) recorded as automation, one lane per parameter (`Clip::moves`, the FX thread's `mpfree-moves`). A lane's points sit on the clip's own ticks: a ramp glides to the next point, a step holds, an end lets the parameter go. A lane on a parameter of the clip's own track is track-relative (a copied clip moves its new track); any other (the master's Perform FX, a macro) is fixed.

- **Lane events.** The player sends a lane's values as `Event::Lane { addr, value }` on the clip's own ticks: every point, and inside a ramp every 8th clip tick (about every 4 ms at 120 BPM). A value on song tick x sounds on x rounded half up, so every value lands on the same frame at any period size. An end sends `Event::Unlock`. A value equal to the one held is not sent again. `Stats::lanes` counts values sent. The mod resolver applies them (`ModEvent::Lane`, the mod thread's #129), and a hand on the control beats the lane until it lets go (`Touch`), which needs nothing from the player.
- **A step's lock wins its step.** Lanes and step locks share one held slot per parameter. A step lock on a parameter a lane is moving takes it, and the lane's later values wait under the lock. When the lock ends (the track's next note without one), the lane takes the parameter back with its value there (`Lane`, not `Unlock`). A lane that ends under a lock leaves the parameter free when the lock ends. Moves are sent after the locks of the same tick, so a lock wins a tie. Notes never end a lane.
- **Chase.** When a clip's moves did not just play up to this span (a start, a launch, a jump, a loop, a switch, an unmute), every lane's state on the first tick comes first: its value, or nothing where it is free. A lane moving at the end of a pass goes on from the start of the next.
- **Jumps, switches, mutes.** A stop, a locate, a loop jump or a panic unlock everything (the chase brings back what the new place has). A switch ends the old clip's moves before the new clip's start. A muted track sends no moves and lets go of its own.
- **Real-time safe.** No allocation (`Moves::play` and `chase` walk fixed tables of 64 lanes). Lock starts per span went from 256 to 1024 to make room for lane values.
- **Cost.** In the bench's time case, which has no moves, the player and recorder cost 3.5 % more instructions (395 a period, about 0.3 µs on the MPC One). The moves case of the FX thread's bench plays 16 lanes.

## Through the arp (`ArpHost`)

A track whose destination is an arpeggiator plays its notes into an `mpfree-arp` engine instead of a sound. The held notes press and release keys on the arp, and the arp's own notes go on to the sampler (or any other destination).

- The host's sink passes the arp track's events to `ArpHost::input`, with their offsets, plus a panic's `AllOff` (sent to every destination). After `Player::process`, `ArpHost::process(block, out)` runs the engine over the same period and gives every note it plays, with its offset, in time order.
- Before every engine call the host gives the engine the span's clock with `Engine::sync` (the hook from #42): the span's anchor, tempo, play state, meter and swing. A loop jump or a locate at the start of a span says `jumped`, so the arp lands on the grid again. Transport swing above 50 swings the arp's steps. Straight transport swing leaves the arp's own swing setting in charge.
- The host splits a span at each note from the player, so a press lands on its exact frame. The engine's timing does not depend on how its calls are cut, so the arp plays the same notes on the same frames for any period size.
- Notes from several sources can come in any order in a period: the player's, and live pad notes for the track. They are kept in offset order. A note held by a pad and a clip at once is pressed on the engine once and released when the last of them lets go.
- No allocation: up to 256 player notes a period. `ArpHost::dropped()` counts player notes that did not fit and engine notes past its 128 per call. It should stay 0.

## One arp per track (`TrackArps`)

The owner's arp decisions of 2026-11-11 give each track its own arp, and each track a switch, "Clips through the arp" (`Track::through_arp`, off by default). `TrackArps` (the arp thread's [`arps.rs`](../../../rust/mpfree-play/src/arps.rs)) holds an `ArpHost` for each of the 128 tracks a project can have (about 1.5 MB, made once at start) and sits between the player and the host's sink.

- **On (melodic tracks).** The track's clip notes go into its arp, as the live pads do. Recording keeps the pads as pressed (the chord), so the clip is arpeggiated again on playback.
- **Off (drum tracks).** The clip notes skip the arp and play as recorded. Recording keeps what the arp played, so a roll becomes notes.
- Either way a part is never arpeggiated twice. CAPTURE follows the same rule.
- A through track's parameter locks skip its arp, which has no use for them. They reach the sink as the player sent them, ahead of the arp's notes on the same frame. A panic's `AllOff` clears every arp in use and passes through. Each arp ends only the notes it has sounding, so a panic costs what they sound. Before, every arp ended all 128 notes: 2 049 events in one period, more than the arps' table (1 024) and the time engine's (512) hold, so the 12 Nov live test counted 1 025 events and 512 notes lost on every STOP while stopped (`time::tests::a_panic_through_the_arps_loses_nothing` reproduces those numbers with the old panic).
- Each period: `player.process(view, block, &mut arps.through_sink())`, then `arps.finish(block, sink)`, which runs every arp over the block and sends everything in frame order.
- **Cost.** A period visits only the tracks up to the highest one anything has asked for (`TrackArps::used`: a note, a pad, the arp's switch or destination, or its engine), so a project of 8 tracks pays for 8 arps, not 128. An idle arp with nothing given to it skips its period (`Engine::skip`). Settings changed on every engine at once (the tempo, a project's arps) go through `TrackArps::engines_mut`, which wakes none: settings alone never start an engine. An engine no period has visited has never been given anything, so it is as a new one, and its first sync lines it up with the transport. On the bench (callgrind), the Time case costs about 8,100 instructions a period and 4 latched arps about 3,100, a little less than with 16 arps always visited.

## Recording (`Recorder`, T3)

Notes played while the transport records go into the armed track's playing clip.

- **Feeding it.** The control thread arms a track in the view (`PlayView::set_armed`) before swapping it in. Each period the host passes the track's live pad notes to `Recorder::input` with their offsets. On a track whose arp output should be kept, it passes the arp's notes instead. Then `Recorder::process(view, block)` runs after the player and the arp. Notes held while the arm changes are not recorded.
- **Exact ticks.** A press lands on the nearest whole tick to its frame (a tick is 0.26 ms at 120 BPM). The tick maps into the clip by its origin, speed and length, so a note played in the clip's fifth pass lands where that pass is. The clip keeps that tick. The clip's quantise applies on playback, so it can change later.
- **Lengths.** A note ends when its pad is released. A punch out, a loop jump, a locate or a stop ends every held note on that tick. A pad pressed again while held ends the held note first. Nothing is recorded while playing without recording, during a count-in, before the clip's origin, on an unarmed track, or with no clip playing.
- **Handing over.** Finished notes come out of `Recorder::taken()` with their track and slot, in the clip's own ticks. The control thread adds each with `Takes::add`, which calls `Clip::add_note` (checked, at most 8192 notes) and remembers the note for undo, and compiles a new view.
- **No double notes.** Quantise or swing can move a note later than the moment it was played. Such a note waits until the playhead has passed the place where it will play, so the player never plays it again straight after it was heard live. It plays from the next pass on.
- **Period size.** Every tick comes from the span the note fell in, and waiting notes come out in a fixed order. The recorded notes are therefore the same, in the same order, for any period size. When the control thread swaps in the new view can depend on the period size.
- **Real-time safe.** Fixed tables, with no allocation: 256 inputs a period, and 256 notes waiting or handed over. `Recorder::dropped()` counts what did not fit. It should stay 0.

### Knob moves become locks

While the transport records, the host passes the armed track's knob moves to `Recorder::input` as `Event::Lock { addr, value }`, in frame order with the pads. Each lands on a step, the Elektron way:

- **On the note being played.** If a pad is held, the lock goes on its note. Otherwise it goes on the note that started last, less than a 1/16 (`LOCK_REACH`, 240 ticks) before the move: one the clip holds, or one this take just recorded that the clip does not hold yet. A sampler pad's parameter looks only at that pad's notes. Old notes the replace pass has erased are not locked.
- **Otherwise trigless.** With no note that close, the lock goes on the nearest step line (the clip's quantise grid, or a 1/16).
- **Waiting like a note.** A recorded lock waits until the playhead has passed where it plays, as a recorded note does (see "No double notes"). So a replace pass never erases it, even when quantise puts it a little ahead of the playhead.
- **A turn is one lock.** A knob sends many values. While a lock waits, each later value for its step and parameter replaces the one it holds. Then `Recorder::taken_locks()` hands it over. The control thread puts each in with `Takes::add_lock`, in place of a lock of the same parameter on the same tick and row. A later value of a lock the same take recorded replaces it in the take too, so the take holds one lock for the whole turn, and UNDO puts back what was there before the take. The clip ends up with the same locks in the same order for any period size.
- **Only the armed track.** A knob on another track's parameter, or a project parameter, is not recorded (`Recorder::stray_locks()` counts them). Nothing is recorded while not recording.
- **Real-time safe.** Up to 64 recorded locks a period after joining; more are counted in `Recorder::dropped()`.

### Perform FX gestures become moves

The host gives the recorder the same Perform FX gestures it gives the move log: `Recorder::input_move(offset, Input::Move { addr, value, map })` while a control moves, `Input::End { addr }` when it lets go. While the transport records:

- **On the clip's ticks.** Each gesture lands on the armed track's playing clip, from where the clip was when it began, at the clip's speed. A gesture that goes on past the clip's end keeps counting on; writing it splits it at the end (`Moves::put`).
- **Where it ends.** A gesture ends where its control lets go, or at a punch out, a stop or a switch of the armed track's clip. After a loop jump or a locate it goes on from where the playhead landed, as a new part.
- **Handing over.** `Recorder::taken_moves()` names each finished gesture (or each part of 1024 values of a long one) with its track, slot, target, map and take; `Recorder::move_points(k)` gives its points. The control thread writes each with `Takes::put_move`, which replaces what the lane had over the ticks the gesture covers, and with `PlayView::put_move` into the view in place.
- **Which lane.** A parameter on the armed track gets a track-relative lane, any other a fixed one. Knob moves are not gestures: they still become parameter locks (the owner's decision, and an opt-in setting to record knobs as moves can come later).
- **Undo.** `Takes::put_move` keeps what each write replaced (`Moves::put_saving`), so UNDO puts the lane back exactly and REDO writes the gesture again. Step edits that move lanes (a page copied, a clip made longer or shorter) keep the lanes before and after.
- **Real-time safe.** 256 gesture inputs a period, 16 gestures at once, 32 handed over a period, in tables made when the recorder is made. `Recorder::dropped()` counts what did not fit.

### Pad pressure goes on its note

The pads send pressure (polyphonic aftertouch, 0 to 127) while they are held. While the armed track records and its "Pressure" switch is on (`Track::record_touch`, on for a new track; the Record list's sixth row), the host gives the recorder each value: `TimeHost::pad_pressure(offset, pad, value)`, which feeds `Recorder::input(offset, Event::Pressure { note, value })`. The decision (the owner's, 12 November, from the synth thread's "Poly aftertouch for MPFree synths" doc, part 7) is thinned pressure, a switch per track, and raw values played back through each instrument's own pressure curve.

- **Thinning.** A value 2 or more steps from the last one kept is kept at once (`TOUCH_STEP`). A value one step away is kept once it has lasted 10 ms (`TOUCH_SETTLE_MS`), on the frame that time is up. So a slow drift is kept about every 10 ms, and a finger's one-step jitter is not kept at all. A value one step away that ends before 10 ms (it goes back, or the pad lets go) is dropped. A pressed note starts at 0, as the instruments do.
- **On its note.** The values a note keeps are timed in clip ticks from the note's own start (`TouchPoint { at, value }`, at the clip's speed). The recorded note names its run in the clip (`Note::touch`, a `Touch { first, len }` into `Clip::touch`). Quantise, swing and note edits that move the note (`..note` copies) take its pressure with it. A split leaves it on the first part only; a shorter note stops sending what falls past its end. Half and double speed (`notes::half_speed`, `notes::double_speed`) stretch each note's pressure with it: the edit carries the notes' runs again with their times doubled or halved (`Edit::touch`, added after the clip's pressure, which is `Edit::touch_at` long when the edit is made), and the new notes name them. The clip's old runs stay, so undo puts back notes that still name theirs. A clip with no room for the new runs refuses the whole edit; an edit made on a clip whose pressure has grown since is refused (`BadTouch`) and changes nothing.
- **The same for any period size.** Every time comes from the frame the value came on, and a one-step value's 10 ms is checked against later values and at each span's end, inside the span where it falls.
- **Its own queue.** Pressure values have their own input queue (`MAX_TOUCH_INPUT`, 512 a period), so a flood of pressure never crowds out notes. A value on the same offset as a note comes after it.
- **Buffers.** Each note's values wait in one of 32 buffers of 2048 values (`TOUCH_BUFS`, `TOUCH_POINTS`, made when the recorder is made) until the note is handed over. The taken note's `touch` then names them in the recorder, `Recorder::touch_points(&note)` gives them, and the buffer is free from the next period on. A full queue or buffer, or a note that finds no free buffer, is counted in `Recorder::touch_dropped()` (it should stay 0) and shown in the status line.
- **Into the clip.** `Takes::add_touched(project, &taken, points)` adds the run to the clip first (`Clip::add_touch`), then the note naming it. A clip's pressure only grows (at most `MAX_TOUCH_POINTS`, 65536), so undo and redo of the note never touch it; pressure a full clip can't take is left off the note and counted. `PlayView::add_touch` puts the run in the view in place, before `PlayView::add_note`, in room set aside at each compile (`TOUCH_ROOM`, 8192 values in the armed track's clips).
- **Playback.** A note with pressure sends `Event::Pressure { note, value }` while it plays: each value on the song tick its time from the note's start lands on (the note's start as played, after quantise and swing, plus the value's clip ticks at the clip's speed, rounded to a tick). It stops at the whole note's end (a ratchet's later hits keep it going) or wherever the note is cut. A new note on the same sound and pitch ends the old note's values. A stop, a locate or a loop jump ends them with the notes; a switch ends the track's on its bar line; a track going quiet drops its own. On a tick, a value comes before the note-offs there. 64 notes send pressure at once (`MAX_TOUCHING`); past that, it counts in `Stats::dropped`. `Stats::pressure` counts the values sent.
- **Where it goes.** The raw value goes to the pads (`Pads::pressure_at`): the synth's `pressure_at` (every note the pad plays, on its frame) or the sampler's. On a track whose clips play through its arp, the arp's repeats follow it, and the sound hears it too. Each instrument shapes it with its own pressure curve (`mpfree_node::Pressure`), so changing a preset's curve changes the playback too. `mpfree-track`'s host sends it to the node as `NodeEvent::Pressure`.
- **CAPTURE** keeps it too: its history has a ring of thinned values of its own, and a kept phrase's notes carry them (see [capture.md](capture.md)).
- **Live to song** places the clips it heard, linked to their slots, and a placement's own copy is a whole clip: either plays its notes' pressure (`tests/pressure.rs`, `a_song_placement_plays_its_clips_pressure`).
- **The note editor** draws it on the roll ([note-editor.md](note-editor.md)).
- **Not yet.** Editing pressure in the note editor, and FX-move-style lanes of pressure.

### Replace and overdub

With the transport's overdub on, a take only adds notes. With overdub off (the default, like REC on an MPC), the take replaces:

- **One pass.** The first time in a take that the transport records with overdub off, the player erases the armed track's notes as the playhead reaches them, until the clip wraps. A take is everything from a punch in to the next punch out. A loop jump, a locate, a stop, a punch out or overdub turning on also ends the pass. After that the take goes on as overdub, so a looping clip never erases what was just played into it. A punch in a moment before the clip wraps erases only that moment.
- **Erased notes.** They are not played. `Player::erased()` names them after each period with their track and slot, as the clip stores them, in the order the playhead reached them. The control thread removes each with `Takes::erase`, which calls `Clip::remove_note` (it removes one equal note), then adds the recorded ones, and compiles a new view. Naming notes by value, not by position, keeps this right when notes are added in the same period. Until the new view is swapped in, an erased note is still in the view, but the playhead has passed it for this pass.
- **The new notes are safe.** A note recorded in the replace pass is handed over only after the playhead has passed where it plays (see "No double notes"), so the pass never erases it.
- **Locks too.** The pass erases the old parameter locks it reaches in the same way, trigless ones included, so the new take does not play the old take's locks. `Player::erased_locks()` names them and `Takes::erase_lock` removes them; UNDO puts them back. The control thread applies erased locks before the recorded ones.
- **Muted tracks.** A muted armed track is still replaced; nothing on it plays.
- **Real-time safe.** Up to 256 erased notes a period; the rest are counted in `Stats::dropped`. `Stats::erased` counts erased notes.

### Take undo (`Takes`)

A take is undone and redone as a whole: the notes it recorded come out and the notes it erased go back.

- **Take numbers.** Each record-on starts the next take, from 1. The recorder and the player count the same record-ons, so every `Taken` and `Erased` note carries its take's number (`Recorder::take()` is the latest).
- **The history.** `Takes` lives on the control thread. `Takes::add` and `Takes::erase` change the clip and remember the note with its take. `Takes::undo` and `Takes::redo` return the take's number, or None. A new take ends redo. It keeps the last 32 takes by default (`Takes::new(limit)`); older ones stay as they are.
- **Late notes.** A note that quantise moves past a punch out waits for the playhead, so it can arrive after the next take has begun. Its number puts it in its own take.
- **By value.** Notes are named as the clip stores them, so editing other notes between takes does not disturb undo. When undo or redo can't apply a note, it skips it. That happens when the clip is gone, or no longer has room or reach for the note (it was shortened). A take undone while it is still recording goes on as a new take, and redo can no longer bring back the undone part.
- **A drop on the Clips grid** (Jay, 14 Nov: drag and drop; `mpfree_play::takes::drops`). `Takes::drop_clip(project, from, to, copy)` moves a clip into an empty slot, swaps it with the clip there, or copies it into an empty slot (a copy onto a clip is refused: `NoRoom`). `Takes::drop_row(project, from, gap, copy)` moves a row (every track's clip in it) to a gap, the rows between shifting one, or copies it in with the rows under it moving down (refused while the last row is in use). Each is one edit take holding its clips in `removed` and `made` and the drop in `Take::dropped` (a `ClipDrop`), named "Move N", "Swap N" or "Copy N" in the takes list. The song's blocks linked to a clip that moves follow it to its new slot and lane (refused with `BadPlacement` if they would overlap there), and a row's scene and the morph's rows follow the row. Undo and redo work these out again from `ClipDrop::before` and `ClipDrop::after` rather than from a copy, so a song edited since is kept. The control half moves the launches and cues the same way (`TimeControl::follow_launches`): a playing clip plays on where it lands, on another track only if that track plays nothing, and a waiting launch follows its clip on its own track.
- **A deleted clip** (ERASE + a clip on Arrange). `Takes::remove_clip(project, track, slot)` takes the clip out of its slot as an edit take of its own ("Delete N" in the takes list) that holds the clip. Undo puts it back if the slot is still empty, and redo takes it out again. The control half refuses a clip a song placement plays (`Source::Slot`), since the block would then play nothing, and compiles afterwards, which drops the clip's launch and cues.
- **A Song view edit** (a block moved, copied, made longer or deleted, a section moved, Scenes → song, Make unique). `Takes::song_edit(project, change, before)` keeps the song before it and after it as an edit take of its own (`Take::song`, a `SongEdit` with a `SongChange`). Undo puts `before` back only while the song is still `after`, and redo the other way, so a song changed since (live to song, say) is left alone. See [arrange.md](arrange.md#in-the-pad-player-timehost).
- **What UNDO says.** The control half names the take it undid or redid as the list does ("take 3 · Drums / 1"); the app shows it in place of the hint ("Undid take 3 · Drums / 1").

## Tests

- `tests/play.rs` has 13 behaviour tests:
  - quarter notes on the beats, and loops
  - polymeter, speed, swing (and a clip overriding it), quantise, a launch origin
  - loop jumps ending notes, and clips restarting in place
  - locate, stop and panic, retriggering, mute and count-in, tempo changes
  - chance: a note at 50 % is left out on some passes and not others, differently on every pass
- `tests/period_size.rs` plays a busy session on five tracks:
  - the session has a count-in, a loop with a tempo segment inside, live tempo changes, punch in and out, locates, stop and panic, and the shortest loop
  - the tracks cover polymeter, 3/4 speed, double speed with its own swing, quantised overlapping notes with chance below 100 %, and a muted track
  - FX moves ride on the slow, fast and muted clips: ramps, holds and ends every 97 ticks, a gesture over the clip's end, one lane under the fast clip's locks and one fixed

  It cuts the session into periods of 1 to 8192 frames and three random sequences, and every event must match. It also checks that each note ends before it starts again and that the final stop ends every note. With the swing look-back window removed, the test fails.
- `tests/arp.rs` plays a held chord through the arp:
  - it arpeggiates on the transport's 1/16s, with the gate, and the chord's end stops it
  - it follows the transport's swing
  - a busy session (a loop, swing, a tempo change, a locate, stop, panic and a restart) gives the same arp notes at periods of 1 to 8192 frames and two random sequences. With the span not split at the player's notes, period 8192 differs and the test fails.
- `tests/arp.rs` also checks that live pad notes fed after the player's notes give the same arp output as notes fed in order, and that a note held by a pad and a clip waits for both.
- `tests/arps.rs` (the arp thread's) checks one arp per track: a through track arpeggiates its clip and a drum track plays as recorded, a latched arp keeps running while the pads play another track, the same notes for any period size with no allocation, a panic clears every arp, and a through track's locks go around its arp.
- In `mpfree-app`'s `time.rs`, a through track records a chord as pressed and plays it back through its arp one note per 1/16, and a track with the switch off records and plays back what the app's arp let be heard.
- `tests/trigs.rs` has 17 tests of trig conditions and ratchets, through the host's own steps (cues applied with `Started::apply` and compiled again):
  - A:B and 1ST count passes from the clip's start; passes before the start count backwards; a pass is one pass at the clip's own speed
  - a locate lands on the pass of the song position, and a locate back to the start plays 1ST again; a transport loop plays the same passes each time; a relaunch counts again from its new start
  - FILL turns on exactly on a step's frame and off one frame after another, at periods of 64, 128 and 1000 frames, and a held FILL stays on
  - PRE and NEI follow the last conditional note, PRE and NEI notes do not change it, NEI on the same tick reads the track to the left decided first, and the first track has no neighbour; chance sets the last result too; a locate resets it and a loop jump keeps it
  - a ratchet gives the same events as the plain notes it stands for, also with swing, quantise and double speed; it plays whole or not at all at 50 % with its hits spread over many periods; a switch and a locate drop its later hits
  - a note left out leaves out its own lock, and a whole-step lock on a chord goes only with every note
  - a busy session (four tracks, every condition, chance, ratchets, locks, swing, quantise, 3/2 speed, FILL switched four times, a loop, a locate and a cue) gives the same events and stats at periods of 1 to 1024 frames

  Making a loop jump reset PRE, sending every lock, or keeping a ratchet's hits past a switch each make a test fail.
- `tests/moves.rs` has 8 tests of how FX moves play: a ramp sends a value every 8th tick and an unlock at its end, each pass; a step lock beats the lane on its step and hands back at the next note; a lane ending under a lock leaves the parameter free; a locate into a gesture chases its value; a switch ends the old clip's moves; a fixed target; a muted track lets go and sends none; and every value on the same frame at period sizes 1 to 4096, with a locate, a loop, a tempo change and a 3/4-speed clip.
- `tests/record_moves.rs` has 7 tests of recording gestures: a gesture becomes a lane and undoes and redoes with its take; a punch out ends a held gesture there; a gesture over the clip's end runs on into the next pass; nothing is recorded while only playing; a parameter off the track gets a fixed lane; a long gesture is handed over in parts, and only a new lane compiles; and the same moves at any period size. The view written in place equals a fresh compile after every period.
- `tests/locks.rs` has 9 tests of how locks play: a lock comes before its note-on and ends at the track's next note without one; a trigless lock holds until the next note; a pad's lock ends only at that pad's next note; a chord keeps its lock for every note; a note its chance leaves out ends nothing; a stop and a locate unlock everything after the note-offs; a switch ends the old clip's locks before the new one's; a muted track lets its locks go and sends none; a clip locks the track it plays on.
- `tests/record.rs` has 20 tests:
  - notes land on the ticks they were played on, with their lengths
  - clip speed, origin and length map song ticks to clip ticks
  - punch out, a loop jump and a stop end held notes, and nothing is recorded between punch out and punch in
  - a pad pressed again ends the held note
  - nothing is recorded unless the track is armed and the transport records (not while playing, not during a count-in, not without a clip)
  - a recorded note that quantise and swing move later is not played again in the pass it was played in. Without the hold-back, it plays twice and the test fails.
  - replace recording erases the old notes the playhead reaches after the punch in, for one pass of the clip, and does not play them; overdub keeps them, and turning overdub on ends the pass
  - a loop jump ends the replace pass, the take goes on as overdub, and a new take replaces again. Without starting a new take on punch in, the test fails.
  - a muted armed track is still replaced
  - a take undoes and redoes as a whole, back to the clip exactly as it was before recording
  - a note that quantise moves past a punch out arrives after the next take has begun and erased a note, and still joins its own take. Tagging it when it is handed over instead of when it ends, or adding it to the latest take, makes the test fail.
  - a busy take (300 random notes and 400 knob moves, punch in and out, a loop, a tempo change, a locate, a stop, overdub off then on, a quantised clip at 3/4 speed) gives the same recorded notes, the same erased notes and the same clip, locks included, at periods of 1 to 8192 frames and four random sequences. Placing notes by the span's start instead of their frame makes it fail, and so does putting a replaced lock at the end of the clip's list.
  - knob moves: a knob turned while a pad is held locks its note, and the turn is one lock in the take, undone with its note; a turn just after a note locks it, and later on the nearest line; a pad's parameter looks only at that pad's notes; a knob on another track, a project parameter, a value that is not a number, or a move while not recording records nothing; a recorded lock replaces the one there and UNDO puts it back; a replace pass does not lock the old notes it erased; a replace pass erases the old locks (on a note, trigless, a pad's own) in the order it reaches them, plays none of them, and UNDO brings them back; a lock recorded just before its quantised note is not erased by the replace pass. Without the lock waiting for the playhead, the last of these fails, and so do two others.
- `src/takes.rs` unit tests: a new take ends redo, old takes are forgotten past the limit, missing clips and notes change nothing, and step edits are takes (a Q-Link turn joins one).
- `tests/cue.rs` has 10 tests. Nine run at periods of 1, 64, 128, 1000 and 4096 frames, with the switch applied by the control thread at once, 3 or 40 periods later, or never. Every way must give the same events.
  - a launch waits for the bar line and starts the clip from its top; a note held over the bar line ends on it; a stop waits too
  - cues on two tracks start on the same bar line
  - in a loop, a cue made in its last bar starts at the loop start; a clip started in a loop's first bar plays on in place after the jump back
  - a cue made while stopped waits for the first bar line played
  - notes held over the switch go into the old clip, and notes played after it (in the same period too) into the new one
  - a switch on the armed track ends the replace pass

  With the player forgetting a switch before the view shows it, or the recorder switching after the notes played in the same period, the tests fail.

  The tenth sends a late view that applies a switch and cues the same clip again: said how many switches it shows, the player starts the clip again from its top on the next bar line; without that, it never does.
- `tests/no_alloc.rs`: a counting allocator watches 4000 periods of five tracks, one through the arp at 1/32s and one recording live pads and knob moves with replace recording, with loops, tempo changes, locates, punch in and out, stop and panic. A launch and a stop are cued, and the view is never compiled again, so the player carries both switches itself. Some notes play by chance, some have trig conditions and ratchets, FILL turns on and off, and every track has locks (the instrument's, a pad's, the strip's) and FX moves (ramps, holds and ends, track-relative and fixed), while Perform FX gestures are recorded. It sees no allocation and no free.
- `tests/in_place.rs` (3): thousands of random recorded and erased notes and locks, with and without quantise and speed, into the playing clip, a cued one, and the playing clip cued again, put in place leave the view equal to a fresh compile; a view with little room asks for a compile and the result is the same; with room set aside, recording in place allocates nothing once the take has begun.
- `view.rs`'s `now_playing_names_the_clip_and_its_stamp_follows_edits_only` (a launch elsewhere keeps the stamp; a note's velocity, the length and the swing each change it) and `a_stamp_kept_up_while_recording_in_place_is_the_one_a_compile_makes` (a note with pressure, a lock, a lock in its place, a gesture and erases: after each, the view equals a fresh compile and the stamp is new).
- `tests/catch_up.rs` (3): thousands of random recorded changes (notes with and without pressure, notes past the clip's end, erases, locks in the place of others) made by `PlayView::record` and `Takes::record` leave the view equal to a fresh compile, with and without quantise and speed, playing or cued; with the control side late by up to 11 changes, every view compiled catches up before it plays and then equals a fresh compile with every change made; a view further behind than `RECENT`, or one with no room, is refused and the view playing stays whole.
- `tests/pressure.rs` has 8 tests of pad pressure, each at periods of 1, 7, 64, 128, 333 and 4096 frames: values are thinned onto their note (a big move kept at once, a one-step jitter dropped, a one-step value kept when its 10 ms are up, one let go before 10 ms dropped, pressure on a pad not held ignored); values are timed in the clip's own ticks from the note's start (a double-speed clip); a note's values play after quantise moved it, and none past its end; a ratchet keeps its pressure across its hits; a loop jump ends a note's values and the next pass starts them again; what was recorded plays back on the next pass on the ticks it was played on; a flood of 600 values in one period records both notes and counts the 88 the queue could not take; and a note keeps at most 2048 values and counts the rest. `tests/no_alloc.rs` also plays pressure on every note and records it on the armed track.
- `mpfree-app`'s `pad_pressure_while_recording_goes_on_its_note_and_plays_back`: the pad player records a pad's pressure onto its note, the view written in place equals a fresh compile, the next pass plays the values back to the pads a bar later (to a tick), and with the track's switch off a take keeps no pressure.
- `mpfree-app`'s `recording_neither_compiles_nor_allocates_on_every_note`: the pad player records sixteen notes a second over a replace pass for 5.5 bars without an allocation or a free, its view equals a fresh compile, and the screens are told at most four times a second.

## Listening

`cargo run --release -p mpfree-play --example play_render` plays a demo beat through the transport, the player, the arp and the real sampler engine (`mpfree-sampler`), and writes `~/mpfree-work/play-demo.wav`. The beat is 84 BPM with heavy 1/16 swing: a four-bar loop played three times, then a final hit that rings out. The sounds are saturated and bit-crushed:

- drums, with ghost snares at low velocity
- hats at double speed with their own swing, and a 1/32 roll
- a distorted bass line on gated pads, so note lengths matter
- a 7/16 glitch clip that drifts against the bar
- dark chords held for a bar each, played through the arp in swung 1/16s on gated pads
- vinyl dust
- two live takes on an empty one-bar clip that quantises to 1/16:
  - **Take 1.** The transport records the first pass. A distorted chord stab is played on the pads a little off the grid, one or two hits a bar. From the next bar on, each hit plays from the clip on the grid, so the stab riff builds up. The transport punches out at the loop's end.
  - **Take 2.** In the second pass, bars 5 and 6 play the riff. Bar 7 punches in with overdub off. The stabs are erased as the playhead reaches them, so they stop, while a metal clank is played on the off-beats. Bar 8 and the third pass play the clanks alone.
- the end: the transport stops on the last bar line, and a kick, stab and crash ring out. The tool then undoes take 2 on a copy of the song and checks that the stab riff is back (take undo).

All the sounds are synthesised in the tool. It prints the take track bar by bar from the notes the player and the pads actually sent (which steps played from the clip, which were played live, which were erased). It checks that the stabs play from the clip only in bars 2 to 6, the clanks only from bar 8, and every erased note falls in bar 7. It prints an events hash for the player's and the arp's notes, the recorded notes and the erased notes. That hash is the same on every computer, and the tool checks that the events are identical at periods of 1, 128 and 4096 frames. On the build host the WAV bytes are also identical at 128 and 1000 frames. Every sound fades out at its end, so none ends with a click. The master is a plain gain with about 1.5 dB of headroom. The tool counts samples that clip at the master and warns if there are any. Drive and bit crushing inside a sound are deliberate and are not counted.

`cargo run --release -p mpfree-play --example steps_demo` plays a beat programmed only through the step grid; see [steps.md](steps.md#listening).

## Screens (`mpfree-time-ui`)

The time engine's screens, as [`mpfree-ui`](ui-architecture.md) widgets:

- **The transport bar** goes under the header on the Arrange page. It has Stop, Play and Rec buttons, the record mode (Overdub or Replace), bar.beat.tick, BPM, the meter, swing and the loop, and an Undo button that only answers when there is a take to undo. It can carry status chips on the right (`TimeState::chips`), but on Arrange it has none: the app's header carries the arp's chip. (The Play page has its own transport card, `play_page`.)
- **Arrange's frame** (layout spec 4c, Jay's option E, 12 Nov): Play's Q-Link column (168 px, `QLINKS` = `"qlinks"`, the app's one `QLinkColumn` widget) runs down the right edge from under the header to the hint line, at x 1100 to 1268 as on Play, in all three views (Clips, Song, Notes). The bar ends at x 1088, and the panel beside the grid, the song or the note editor (Record or Edit, over Takes) is `SIDE_W` = 250 px (x 838 to 1088), so the grid, the song and the editor keep their width. `TimePage::add_to` adds an empty column only when the `Ui` has none, so the app's is never replaced. The app fills the cards on Arrange and routes the knobs there. At 250 px the option lists draw narrow (`mpfree_ui::widgets::OptionList::narrow`: 15 px text, a value box a little over half the row, thin arrows, a tap on the box's left or right third steps it); the record rows are named Armed, Mode, Count-in, Loop and Tempo to fit.
- **The Clips page** (Arrange's Clips view) is a launch grid of 8 tracks at a time by 16 slots, 6 slots at a time. A ring shows one pass of a playing clip, amber marks a clip waiting for the next bar, and red marks recording. Tapping an empty slot makes a clip there and records into it. Tapping a track's name arms the track, on the lift (`ClipGrid::press`), so a swipe along the names of 48 px or more pages instead. Under the name is what the track plays (`GridTrack::sound`, from `TrackInfo::sound`: the app writes "Kit" or the track's own instrument, `gui::fill_sounds`), and the header grows from 28 to 44 px for it; a tap on that line gives `Tab(SOUND_TAP + track)`, which the app turns into `FromUi::TrackInstrument` (the track armed, the Synth tab's MENU on its Instrument row). A clip and a row's number act on the lift, since they can be dragged (`ClipGrid::grip`): moved 12 px they lift, follow the finger, and drop as `EventKind::Drop` (a clip: cell to cell; a row: `ROW_DROP + row` to a gap); at the grid's edge they page the tracks or scroll the rows (`Toggle(SLOTS + PAGE_* / ROWS_*)`, again every 0.6 s held there). **+ Track** on the title line (`Toggle(SLOTS + ADD_TRACK)`, dim at 128) is `TimeAction::AddTrack`: `Takes::add_track` pushes "Track N" with the lowest destination no track uses, as a take of its own (`Take::track_added`), so UNDO takes it away and REDO brings it back with the takes made on it (undo is last first, so the track's own takes go before it). The control arms the new track. With more than 8 tracks the pager "‹ 9–16 of 24 ›" takes the legend's place; its arrows and the swipe are the page's own (`TimePage::page_tracks`, `Toggle(SLOTS + PAGE_BACK / PAGE_ON)`), 8 tracks a step, and a track newly armed off the page brings its page (`TimePage::update`). The Song view has "+ Track" after the MIDI lanes (`song::ADD_TRACK`). While the music plays:
  - tapping a clip cues it. Its cell shows where it starts ("from 6.1"), and the track's playing clip shows "until 6.1".
  - tapping the playing clip cues a stop ("stops 6.1").
  - tapping what is cued takes the cue back.

  - tapping a row's number launches the row as a scene, and the square over the row numbers stops every track ([scenes.md](scenes.md)).
  - the Song › button swaps the grid for the Song view, the arrangement on a timeline ([arrange.md](arrange.md)).

  While stopped, a tap launches or stops at once. A clip launched while stopped plays as if it had started on the bar line before the position, wherever the song goes after. `clips_after` works out the launches and cues after a tap.
- **The record settings** set the armed track, the record mode, the count-in (0 to 4 bars), the loop (off, 1, 2, 4, 8 or 16 bars from bar 1) and the tempo.
- **The takes list** shows the newest take first and dims undone takes, with Undo and Redo.

`TimeState::from_engine` reads what they show from the transport, the project, the launches and the takes (the app sets the cues and Capture's label), and `action()` turns touches into `TimeAction`s. `mpfree-time-preview` draws sample screens from a real transport, player, recorder and take undo as PNG files.

## In the pad player (`TimeHost`)

`mpfree-app`'s [`time.rs`](../../../rust/mpfree-app/src/time.rs) runs the engine in the audio loop, in two halves behind one facade since 12 Nov: an audio half that plays and records, and a control half that holds the project and compiles ([time-threads.md](time-threads.md)).

1. Each period the loop hands `TimeHost::pad` the pad notes it is about to play, after the arp, at their frame offsets. Once the app plays the pads through the per-track arps, it also hands `TimeHost::pad_pressed` the pads as pressed, before any arp. The armed track records the pressed pads when its clips play through its arp, and the heard notes otherwise. The capture history keeps the same side.
2. `TimeHost::period` runs the transport and the player. The clips play the current kit's pads through the sampler (`hit_at`, `release_at`); a clip's note numbers are pad numbers, 0 to 15. `TimeHost::period_arps(frames, pads, arps)` does the same with the per-track arps between the player and the pads. The app owns the `TrackArps` with its arp panel and lends it for the period. Each period every arp first takes its track's destination and switch. `TimeHost::set_through_arp(track, on)` turns the switch. Nothing calls `period_arps` or `set_through_arp` yet: the arp thread moves the app's arp onto the per-track arps.
3. The armed track records, and the taken and erased notes, the recorded locks and the recorded moves go into the clips and the take history. `TimeHost::gesture(offset, input)` takes a Perform FX gesture: the recorder records it while recording, and a move log (`mpfree_moves::MoveLog`) keeps it while rolling, so CAPTURE keeps the moves made over its phrase as part of the same capture. Nothing calls it yet: the Perform FX page calls it once the pad player runs it. `TimeHost::knob(offset, addr, value)` takes a knob move for recording; no knob calls it yet. `TimeHost::set_fill(on, offset)` turns FILL on or off for the trig conditions; no button calls it yet (the pad player picks the FILL button).
4. While counting in and recording, a click plays on each beat when the metronome is on: 1760 Hz on the bar and 1320 Hz on the other beats. The metronome is off by default and plays at −24 dBFS (Settings `metronome` and `metronome_db`, −36 to −6 dB; `TimeHost::metronome` and `click_gain`). It was on, at about −12 dBFS, until live test 3 ("far too loud").

Pad pressure goes to `TimeHost::pad_pressure` as it comes from the pads (main.rs, before the live instrument hears it), and the player's `Pressure` events go back to the pads with `Pads::pressure_at`. The Record list's "Pressure" row (`TimeAction::RecordPressure`) flips the armed track's switch; the row scrolls into view when a lower row is picked. `TimeHost::touch_dropped()` counts lost values, in the status line.

The player's `Lock`, `Lane` and `Unlock` events go to the mixer's mod engine: `TimeHost::locks` and `TimeHost::lanes` each period, then `MixHost::lock` and `MixHost::lane`. A move lane's value holds its parameter as a lock does, but a hand on the control beats it.

A track that plays somewhere other than the pads, such as a track on MIDI OUT (the MIDI thread's M3), is offered each event the clips play first, through `Pads::out(&Out) -> bool`: notes, a panic's `AllOff`, pressure, locks and lanes. When it returns true, neither the pads nor the mod engine get the event. When it returns false, the event goes on as usual, which is what an `AllOff` should do so the pads stop too. By default it takes nothing. The arps' notes, the live pads among them, are not offered to it: they go to `Pads::live`, which sees the whole event (track included) and says what the armed track records, so the pad player sends a MIDI OUT track's live notes from there. The pad player's `SeqPads` answers both from the track's "Plays on" setting.

It starts with 8 empty tracks, with track 1 armed. A clip made from an empty slot is as long as the loop, or 2 bars without one. REC when the armed track has no clip playing launches the clip its pattern shows, or makes one in its first empty slot (`TimeControl::ready_to_record`), so REC on a fresh project keeps what the pads play. The tempo belongs to the app, so TAP TEMPO, the menus and the Clips page all set it, and the transport follows. Only the position (about 30 times a second) and real changes go to the screen.

Cues that start become launches right after the period they started in (`Started::apply`). Every view the pad player compiles says what it includes: the recorded changes made so far and the cues started on each track (`PlayView::set_includes`). Edits (launches, cues, arming, undo and redo, a capture) compile a new `PlayView` on the control half, which still runs in the audio loop until the control thread takes it. That allocates, but only on an edit, not every period. Recording does not compile: each recorded or erased note, lock or gesture is a numbered change (`Rec`) made to the view in place (`PlayView::record`) and to the clip (`Takes::record`), deciding alike, in room set aside at each compile (`RECORD_ROOM`, 1024 notes and 1024 locks in the armed track's clips, and room for two parts of a gesture in each of their lanes; a new take's lists have room for `TAKE_ROOM`, 256). A gesture on a parameter the clip has no lane for yet compiles once. Once a take has begun, a recording period allocates nothing, and the screens hear about the recorded notes at most four times a second. It is also far cheaper: on the workstation, with 8 clips of 500 notes playing, compiling after each recorded note took 66 to 90 µs (2 to 3 % of a 128-frame period, and several times that on the MPC One); putting the note in place takes 0.3 µs.

**A compile costs what plays.** `PlayView::compile_cued` checks the tracks (`Project::check_tracks`: how many, their swing) and then each clip it compiles, launched or cued, as it compiles it; `set_song` checks the arrangement and each placed clip once, with song mode on. It no longer checks every clip in the project, which was more than half of a compile's time and grew with clips that weren't playing. A clip out of range is refused when it is launched, cued or placed (`CompileError::Model`), so the rest of the project still plays. On the workstation (release build, other jobs running), a project of 64 clips of 256 notes with 8 playing compiled in 49 µs before and 17 µs after; 128 clips of 512 notes with 16 playing, 307 µs before and 159 µs after. What is left grows with the notes playing, and on the MPC One it can still take a good part of a period, so the other edits move to a control thread: the two halves are built, and the control thread is next ([time-threads.md](time-threads.md)). The status line every 30 s adds the transport state, the takes and any notes or parameter locks lost to full tables (should stay 0).

## Next

- Try clips, recording, CAPTURE, scenes, the Song view, the note editor and pad pressure on the MPC, in the app thread's next live test round.
- Run the control half (launches, arming, undo, capture, the screen's state) on the control thread: [time-threads.md](time-threads.md) step 3.
- CAPTURE (keep what was just played, even without recording) is built: see [capture.md](capture.md).
- Launching on the bar line (cues), scenes and follow actions are built ([scenes.md](scenes.md)). Still to come for T9: a launch quantise setting (1 beat to 8 bars, or none), an editor for scene names and follow actions, and a new clip that waits for its bar line without stopping the track's clip first.
- The step sequencer's model is built (steps with velocity, length, nudge, chance, trig conditions, ratchets and parameter locks, 16 to a page and as many pages as the clip holds, chords, undoable edits): see [steps.md](steps.md). The Play page's step grid edits it (the app's `play_page.rs`), and the note editor's tracker lens shows it as rows ([note-editor.md](note-editor.md)).
- Pad pressure: editing it in the note editor, and showing it in the list and the tracker ([note-editor.md](note-editor.md)). Buttons for half and double speed, which stretch the pressure with the notes.
- The looper (T4) is built: [looper.md](looper.md).
- Audio tracks run in the app on the transport's block and tempo map, after the looper, in song mode only (`TimeHost::tempo_map`, `TimeHost::song_on`): [audio-tracks.md](audio-tracks.md#in-the-pad-player).
