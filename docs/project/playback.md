# Playback: clips on the transport

**Status:** roadmap step T2 (merged in #47), a track through the arp (#51), the clip recorder (T3, #55), replace recording (#56) and take undo, 2026-11-11. The code is [`rust/mpfree-seq`](../../rust/mpfree-seq) (the model) and [`rust/mpfree-play`](../../rust/mpfree-play) (the playback engine). Both are host-only and tested. They are not yet wired to the audio thread or the pads. Nothing here has run on the MPC.

This builds on the [transport](transport.md). The full plan is the time engine design document. Its mockups and skill tree are in the mpfree-claude notes (`sequencer/`).

## The model (`mpfree-seq`)

A project holds up to 16 tracks. A track has a destination, an optional swing, a mute flag and 16 clip slots. A clip has:

- a length in its own ticks (from a 1/32 note to 256 bars)
- a speed (1/8 to 4 times)
- an optional swing and an optional quantise
- its notes, each with a tick, a length, a note or pad number, a velocity, a chance (1 to 100 %, 100 for a recorded note), a trig condition (`Always` for a recorded note) and a ratchet (1 to 8 hits, 1 for a recorded note)

These are plain values, edited on the control thread and checked with `check()`.

- **Everything is a clip.** A step pattern and a recorded phrase are the same thing. The step grid is a view of a clip, not its own data: see [steps.md](steps.md).
- **Any length.** Clips of different lengths drift against each other (polymeter), and no clip is tied to 16 steps.
- **Quantise is non-destructive.** Notes keep their recorded ticks, and quantise (grid and strength) applies on playback, so it can be changed or undone later.
- **Swing:** the clip's own swing wins, then the track's, then the transport's.

Parameter locks, trig conditions and ratchets are in the model too (see [steps.md](steps.md)). Audio clips, scenes and the arrangement come in later steps. They add to this model without changing it.

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

## Launching on the bar line (cues, T9)

A clip launched while the music plays does not cut in at once. It waits for the next bar line, the way the MPC's Next Sequence mode switches sequences at the end of a bar.

- **A cue.** `PlayView::compile_cued` takes the launches and the `Cue { track, slot }` values waiting on the tracks, one per track. `slot: None` is a stop.
- **The next bar line the playhead plays.** That is usually the next bar. In a loop whose end comes first, it is the loop start's bar line, right after the jump back. A cue made while stopped waits for the first bar line played after PLAY.
- **The switch.** On that tick the track's clip stops and the cued clip starts from its first tick. Notes of the old clip still sounding end on the same tick, before the new clip's notes start. A stop does the same with no new clip.
- **Then it keeps its place against the song.** A started clip's origin is `anchored`: the bar line, moved back by whole passes to before any position the song can reach (`EARLIEST`, about 12 days at 120 BPM). So after a loop jump back or a locate it plays on in phase, and doesn't go silent until it reaches the bar line again.
- **The control thread catches up.** `Player::started()` names each switch (track, slot, the tick, the new origin). The app applies it with `Started::apply`, which makes the clip the track's launch (starting on that tick) and removes the cue, and compiles again. Until that view arrives, the player and the recorder carry the switch themselves, so when the view arrives changes nothing that plays.
- **Recording.** On the armed track, notes held at the switch end on its bar line, in the clip that was playing. Notes played from that tick on go into the new clip. A switch ends a replace pass, and the take goes on as overdub.

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

## Through the arp (`ArpHost`)

A track whose destination is an arpeggiator plays its notes into an `mpfree-arp` engine instead of a sound. The held notes press and release keys on the arp, and the arp's own notes go on to the sampler (or any other destination).

- The host's sink passes the arp track's events to `ArpHost::input`, with their offsets, plus a panic's `AllOff` (sent to every destination). After `Player::process`, `ArpHost::process(block, out)` runs the engine over the same period and gives every note it plays, with its offset, in time order.
- Before every engine call the host gives the engine the span's clock with `Engine::sync` (the hook from #42): the span's anchor, tempo, play state, meter and swing. A loop jump or a locate at the start of a span says `jumped`, so the arp lands on the grid again. Transport swing above 50 swings the arp's steps. Straight transport swing leaves the arp's own swing setting in charge.
- The host splits a span at each note from the player, so a press lands on its exact frame. The engine's timing does not depend on how its calls are cut, so the arp plays the same notes on the same frames for any period size.
- Notes from several sources can come in any order in a period: the player's, and live pad notes for the track. They are kept in offset order. A note held by a pad and a clip at once is pressed on the engine once and released when the last of them lets go.
- No allocation: up to 256 player notes a period. `ArpHost::dropped()` counts player notes that did not fit and engine notes past its 128 per call. It should stay 0.

## One arp per track (`TrackArps`)

The owner's arp decisions of 2026-11-11 give each track its own arp, and each track a switch, "Clips through the arp" (`Track::through_arp`, off by default). `TrackArps` (the arp thread's [`arps.rs`](../../rust/mpfree-play/src/arps.rs)) holds 16 `ArpHost`s and sits between the player and the host's sink.

- **On (melodic tracks).** The track's clip notes go into its arp, as the live pads do. Recording keeps the pads as pressed (the chord), so the clip is arpeggiated again on playback.
- **Off (drum tracks).** The clip notes skip the arp and play as recorded. Recording keeps what the arp played, so a roll becomes notes.
- Either way a part is never arpeggiated twice. CAPTURE follows the same rule.
- A through track's parameter locks skip its arp, which has no use for them. They reach the sink as the player sent them, ahead of the arp's notes on the same frame. A panic's `AllOff` clears every arp and passes through.
- Each period: `player.process(view, block, &mut arps.through_sink())`, then `arps.finish(block, sink)`, which runs every arp over the block and sends everything in frame order.
- **Cost.** Running 16 arps instead of one adds about 3,300 instructions a period on the bench's Time case (callgrind): 8,060 to 11,320 with no conditions, 7,580 to 10,860 with them. Most of that is the 15 arps with nothing to play, which still follow the transport each period. An idle arp could skip its period; that is the arp thread's call.

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

  It cuts the session into periods of 1 to 8192 frames and three random sequences, and every event must match. It also checks that each note ends before it starts again and that the final stop ends every note. With the swing look-back window removed, the test fails.
- `tests/arp.rs` plays a held chord through the arp:
  - it arpeggiates on the transport's 1/16s, with the gate, and the chord's end stops it
  - it follows the transport's swing
  - a busy session (a loop, swing, a tempo change, a locate, stop, panic and a restart) gives the same arp notes at periods of 1 to 8192 frames and two random sequences. With the span not split at the player's notes, period 8192 differs and the test fails.
- `tests/arp.rs` also checks that live pad notes fed after the player's notes give the same arp output as notes fed in order, and that a note held by a pad and a clip waits for both.
- `tests/arps.rs` (the arp thread's) checks one arp per track: a through track arpeggiates its clip and a drum track plays as recorded, a latched arp keeps running while the pads play another track, the same notes for any period size with no allocation, a panic clears every arp, and a through track's locks go around its arp.
- In `mpfree-hello`'s `time.rs`, a through track records a chord as pressed and plays it back through its arp one note per 1/16, and a track with the switch off records and plays back what the app's arp let be heard.
- `tests/trigs.rs` has 17 tests of trig conditions and ratchets, through the host's own steps (cues applied with `Started::apply` and compiled again):
  - A:B and 1ST count passes from the clip's start; passes before the start count backwards; a pass is one pass at the clip's own speed
  - a locate lands on the pass of the song position, and a locate back to the start plays 1ST again; a transport loop plays the same passes each time; a relaunch counts again from its new start
  - FILL turns on exactly on a step's frame and off one frame after another, at periods of 64, 128 and 1000 frames, and a held FILL stays on
  - PRE and NEI follow the last conditional note, PRE and NEI notes do not change it, NEI on the same tick reads the track to the left decided first, and the first track has no neighbour; chance sets the last result too; a locate resets it and a loop jump keeps it
  - a ratchet gives the same events as the plain notes it stands for, also with swing, quantise and double speed; it plays whole or not at all at 50 % with its hits spread over many periods; a switch and a locate drop its later hits
  - a note left out leaves out its own lock, and a whole-step lock on a chord goes only with every note
  - a busy session (four tracks, every condition, chance, ratchets, locks, swing, quantise, 3/2 speed, FILL switched four times, a loop, a locate and a cue) gives the same events and stats at periods of 1 to 1024 frames

  Making a loop jump reset PRE, sending every lock, or keeping a ratchet's hits past a switch each make a test fail.
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
- `tests/cue.rs` has 9 tests, each run at periods of 1, 64, 128, 1000 and 4096 frames, with the switch applied by the control thread at once, 3 or 40 periods later, or never. Every way must give the same events.
  - a launch waits for the bar line and starts the clip from its top; a note held over the bar line ends on it; a stop waits too
  - cues on two tracks start on the same bar line
  - in a loop, a cue made in its last bar starts at the loop start; a clip started in a loop's first bar plays on in place after the jump back
  - a cue made while stopped waits for the first bar line played
  - notes held over the switch go into the old clip, and notes played after it (in the same period too) into the new one
  - a switch on the armed track ends the replace pass

  With the player forgetting a switch before the view shows it, or the recorder switching after the notes played in the same period, the tests fail.
- `tests/no_alloc.rs`: a counting allocator watches 4000 periods of five tracks, one through the arp at 1/32s and one recording live pads and knob moves with replace recording, with loops, tempo changes, locates, punch in and out, stop and panic. A launch and a stop are cued, and the view is never compiled again, so the player carries both switches itself. Some notes play by chance, some have trig conditions and ratchets, FILL turns on and off, and every track has locks (the instrument's, a pad's, the strip's). It sees no allocation and no free.
- `tests/in_place.rs` (3): thousands of random recorded and erased notes and locks, with and without quantise and speed, into the playing clip, a cued one, and the playing clip cued again, put in place leave the view equal to a fresh compile; a view with little room asks for a compile and the result is the same; with room set aside, recording in place allocates nothing once the take has begun.
- `mpfree-hello`'s `recording_neither_compiles_nor_allocates_on_every_note`: the pad player records sixteen notes a second over a replace pass for 5.5 bars without an allocation or a free, its view equals a fresh compile, and the screens are told at most four times a second.

## Listening

`cargo run --release -p mpfree-play --example render` plays a demo beat through the transport, the player, the arp and the real sampler engine (`mpfree-sampler`), and writes `~/mpfree-work/play-demo.wav`. The beat is 84 BPM with heavy 1/16 swing: a four-bar loop played three times, then a final hit that rings out. The sounds are saturated and bit-crushed:

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

- **The transport bar** goes under the header on the Play and Clips pages. It has Stop, Play and Rec buttons, the record mode (Overdub or Replace), bar.beat.tick, BPM, the meter, swing and the loop, and an Undo button that only answers when there is a take to undo. Chips sit on the right; the arp's chip moves here.
- **The Clips page** is a launch grid of 8 tracks by 16 slots, 6 slots at a time. A ring shows one pass of a playing clip, amber marks a clip waiting for the next bar, and red marks recording. Tapping an empty slot makes a clip there and records into it. Tapping a track's name arms the track. While the music plays:
  - tapping a clip cues it. Its cell shows where it starts ("from 6.1"), and the track's playing clip shows "until 6.1".
  - tapping the playing clip cues a stop ("stops 6.1").
  - tapping what is cued takes the cue back.

  While stopped, a tap launches or stops at once. A clip launched while stopped plays as if it had started on the bar line before the position, wherever the song goes after. `clips_after` works out the launches and cues after a tap.
- **The record settings** set the armed track, the record mode, the count-in (0 to 4 bars), the loop (off, 1, 2, 4, 8 or 16 bars from bar 1) and the tempo.
- **The takes list** shows the newest take first and dims undone takes, with Undo and Redo.

`TimeState::from_engine` reads what they show from the transport, the project, the launches and the takes (the app sets the cues and Capture's label), and `action()` turns touches into `TimeAction`s. `mpfree-time-preview` draws sample screens from a real transport, player, recorder and take undo as PNG files.

## In the pad player (`TimeHost`)

`mpfree-hello`'s [`time.rs`](../../rust/mpfree-hello/src/time.rs) runs the engine in the audio loop:

1. Each period the loop hands `TimeHost::pad` the pad notes it is about to play, after the arp, at their frame offsets. Once the app plays the pads through the per-track arps, it also hands `TimeHost::pad_pressed` the pads as pressed, before any arp. The armed track records the pressed pads when its clips play through its arp, and the heard notes otherwise. The capture history keeps the same side.
2. `TimeHost::period` runs the transport and the player. The clips play the current kit's pads through the sampler (`hit_at`, `release_at`); a clip's note numbers are pad numbers, 0 to 15. `TimeHost::period_arps(frames, pads, arps)` does the same with the per-track arps between the player and the pads. The app owns the `TrackArps` with its arp panel and lends it for the period. Each period every arp first takes its track's destination and switch. `TimeHost::set_through_arp(track, on)` turns the switch. Nothing calls `period_arps` or `set_through_arp` yet: the arp thread moves the app's arp onto the per-track arps.
3. The armed track records, and the taken and erased notes, and the recorded locks, go into the clips and the take history. `TimeHost::knob(offset, addr, value)` takes a knob move for recording; no knob calls it yet. `TimeHost::set_fill(on, offset)` turns FILL on or off for the trig conditions; no button calls it yet (the pad player picks the FILL button).
4. While counting in and recording, a click plays on each beat: 1760 Hz on the bar and 1320 Hz on the other beats, at about −12 dBFS.

The player's `Lock` and `Unlock` events are ignored for now: they need the mod resolver, which the pad player does not run yet.

It starts with 8 empty tracks, with track 1 armed. A clip made from an empty slot is as long as the loop, or 2 bars without one. The tempo belongs to the app, so TAP TEMPO, the menus and the Clips page all set it, and the transport follows. Only the position (about 30 times a second) and real changes go to the screen.

Cues that start become launches right after the period they started in (`Started::apply`). Edits (launches, cues, arming, undo and redo, a capture) compile a new `PlayView` in the audio loop. That allocates, but only on an edit, not every period. Recording does not compile: each recorded or erased note or lock goes into the clip and into the view in place, in room set aside at each compile (`RECORD_ROOM`, 1024 notes and 1024 locks in the armed track's clips; a new take's lists have room for `TAKE_ROOM`, 256). Once a take has begun, a recording period allocates nothing, and the screens hear about the recorded notes at most four times a second. It is also far cheaper: on the workstation, with 8 clips of 500 notes playing, compiling after each recorded note took 66 to 90 µs (2 to 3 % of a 128-frame period, and several times that on the MPC One); putting the note in place takes 0.3 µs. Moving the other edits to a control thread is a later step. The status line every 30 s adds the transport state, the takes and any notes lost to full tables (should stay 0).

## Next

- Try the time engine on the MPC (round 6), with REC, OVERDUB, UNDO and PLAY START learned by the button guide.
- Move the other edits (launches, arming, undo, capture) out of the audio loop, and the screen's state with them.
- CAPTURE (keep what was just played, even without recording) is built: see [capture.md](capture.md).
- Launching on the bar line (cues) is built. Still to come for T9: a launch quantise setting (1 beat to 8 bars, or none), scenes (a row of clips launched together), and a new clip that waits for its bar line without stopping the track's clip first.
- The step sequencer's model is built (steps with velocity, length, nudge, chance, trig conditions, ratchets and parameter locks, 16 to a page and as many pages as the clip holds, chords, undoable edits): see [steps.md](steps.md). Its screen waits for the Play page layout.
- The looper (T4).
