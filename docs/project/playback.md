# Playback: clips on the transport

**Status:** roadmap step T2 (merged in #47), a track through the arp (#51), the clip recorder (T3, #55), replace recording (#56) and take undo, 2026-11-11. The code is [`rust/mpfree-seq`](../../rust/mpfree-seq) (the model) and [`rust/mpfree-play`](../../rust/mpfree-play) (the playback engine). Both are host-only and tested. They are not yet wired to the audio thread or the pads. Nothing here has run on the MPC.

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

## Through the arp (`ArpHost`)

A track whose destination is an arpeggiator plays its notes into an `mpfree-arp` engine instead of a sound. The held notes press and release keys on the arp, and the arp's own notes go on to the sampler (or any other destination).

- The host's sink passes the arp track's events to `ArpHost::input`, with their offsets, plus a panic's `AllOff` (sent to every destination). After `Player::process`, `ArpHost::process(block, out)` runs the engine over the same period and gives every note it plays, with its offset, in time order.
- Before every engine call the host gives the engine the span's clock with `Engine::sync` (the hook from #42): the span's anchor, tempo, play state, meter and swing. A loop jump or a locate at the start of a span says `jumped`, so the arp lands on the grid again. Transport swing above 50 swings the arp's steps. Straight transport swing leaves the arp's own swing setting in charge.
- The host splits a span at each note from the player, so a press lands on its exact frame. The engine's timing does not depend on how its calls are cut, so the arp plays the same notes on the same frames for any period size.
- Notes from several sources can come in any order in a period: the player's, and live pad notes for the track. They are kept in offset order. A note held by a pad and a clip at once is pressed on the engine once and released when the last of them lets go.
- No allocation: up to 256 player notes a period. `ArpHost::dropped()` counts player notes that did not fit and engine notes past its 128 per call. It should stay 0.

## Recording (`Recorder`, T3)

Notes played while the transport records go into the armed track's playing clip.

- **Feeding it.** The control thread arms a track in the view (`PlayView::set_armed`) before swapping it in. Each period the host passes the track's live pad notes to `Recorder::input` with their offsets. On a track whose arp output should be kept, it passes the arp's notes instead. Then `Recorder::process(view, block)` runs after the player and the arp. Notes held while the arm changes are not recorded.
- **Exact ticks.** A press lands on the nearest whole tick to its frame (a tick is 0.26 ms at 120 BPM). The tick maps into the clip by its origin, speed and length, so a note played in the clip's fifth pass lands where that pass is. The clip keeps that tick. The clip's quantise applies on playback, so it can change later.
- **Lengths.** A note ends when its pad is released. A punch out, a loop jump, a locate or a stop ends every held note on that tick. A pad pressed again while held ends the held note first. Nothing is recorded while playing without recording, during a count-in, before the clip's origin, on an unarmed track, or with no clip playing.
- **Handing over.** Finished notes come out of `Recorder::taken()` with their track and slot, in the clip's own ticks. The control thread adds each with `Takes::add`, which calls `Clip::add_note` (checked, at most 8192 notes) and remembers the note for undo, and compiles a new view.
- **No double notes.** Quantise or swing can move a note later than the moment it was played. Such a note waits until the playhead has passed the place where it will play, so the player never plays it again straight after it was heard live. It plays from the next pass on.
- **Period size.** Every tick comes from the span the note fell in, and waiting notes come out in a fixed order. The recorded notes are therefore the same, in the same order, for any period size. When the control thread swaps in the new view can depend on the period size.
- **Real-time safe.** Fixed tables, with no allocation: 256 inputs a period, and 256 notes waiting or handed over. `Recorder::dropped()` counts what did not fit. It should stay 0.

### Replace and overdub

With the transport's overdub on, a take only adds notes. With overdub off (the default, like REC on an MPC), the take replaces:

- **One pass.** The first time in a take that the transport records with overdub off, the player erases the armed track's notes as the playhead reaches them, until the clip wraps. A take is everything from a punch in to the next punch out. A loop jump, a locate, a stop, a punch out or overdub turning on also ends the pass. After that the take goes on as overdub, so a looping clip never erases what was just played into it. A punch in a moment before the clip wraps erases only that moment.
- **Erased notes.** They are not played. `Player::erased()` names them after each period with their track and slot, as the clip stores them, in the order the playhead reached them. The control thread removes each with `Takes::erase`, which calls `Clip::remove_note` (it removes one equal note), then adds the recorded ones, and compiles a new view. Naming notes by value, not by position, keeps this right when notes are added in the same period. Until the new view is swapped in, an erased note is still in the view, but the playhead has passed it for this pass.
- **The new notes are safe.** A note recorded in the replace pass is handed over only after the playhead has passed where it plays (see "No double notes"), so the pass never erases it.
- **Muted tracks.** A muted armed track is still replaced; nothing on it plays.
- **Real-time safe.** Up to 256 erased notes a period; the rest are counted in `Stats::dropped`. `Stats::erased` counts erased notes.

### Take undo (`Takes`)

A take is undone and redone as a whole: the notes it recorded come out and the notes it erased go back.

- **Take numbers.** Each record-on starts the next take, from 1. The recorder and the player count the same record-ons, so every `Taken` and `Erased` note carries its take's number (`Recorder::take()` is the latest).
- **The history.** `Takes` lives on the control thread. `Takes::add` and `Takes::erase` change the clip and remember the note with its take. `Takes::undo` and `Takes::redo` return the take's number, or None. A new take ends redo. It keeps the last 32 takes by default (`Takes::new(limit)`); older ones stay as they are.
- **Late notes.** A note that quantise moves past a punch out waits for the playhead, so it can arrive after the next take has begun. Its number puts it in its own take.
- **By value.** Notes are named as the clip stores them, so editing other notes between takes does not disturb undo. When undo or redo can't apply a note, it skips it. That happens when the clip is gone, or no longer has room or reach for the note (it was shortened). A take undone while it is still recording goes on as a new take, and redo can no longer bring back the undone part.

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
- `tests/arp.rs` plays a held chord through the arp:
  - it arpeggiates on the transport's 1/16s, with the gate, and the chord's end stops it
  - it follows the transport's swing
  - a busy session (a loop, swing, a tempo change, a locate, stop, panic and a restart) gives the same arp notes at periods of 1 to 8192 frames and two random sequences. With the span not split at the player's notes, period 8192 differs and the test fails.
- `tests/arp.rs` also checks that live pad notes fed after the player's notes give the same arp output as notes fed in order, and that a note held by a pad and a clip waits for both.
- `tests/record.rs` has 13 tests:
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
  - a busy take (300 random notes, punch in and out, a loop, a tempo change, a locate, a stop, overdub off then on, a quantised clip at 3/4 speed) gives the same recorded notes, the same erased notes and the same clip at periods of 1 to 8192 frames and two random sequences. Placing notes by the span's start instead of their frame makes it fail.
- `src/takes.rs` unit tests: a new take ends redo, old takes are forgotten past the limit, and missing clips and notes change nothing.
- `tests/no_alloc.rs`: a counting allocator watches 4000 periods of five tracks, one through the arp at 1/32s and one recording live pads with replace recording, with loops, tempo changes, locates, punch in and out, stop and panic. It sees no allocation and no free.

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

## Screens (`mpfree-time-ui`)

The time engine's screens, as [`mpfree-ui`](ui-architecture.md) widgets:

- **The transport bar** goes under the header on the Play and Clips pages. It has Stop, Play and Rec buttons, the record mode (Overdub or Replace), bar.beat.tick, BPM, the meter, swing and the loop, and an Undo button that only answers when there is a take to undo. Chips sit on the right; the arp's chip moves here.
- **The Clips page** is a launch grid of 8 tracks by 16 slots, 6 slots at a time. A ring shows one pass of a playing clip, amber marks a clip waiting for the next bar, and red marks recording. Tapping a clip launches it from the next bar, or stops it if it is playing. Tapping an empty slot makes a clip there and records into it. Tapping a track's name arms the track.
- **The record settings** set the armed track, the record mode, the count-in (0 to 4 bars), the loop (off, 1, 2, 4, 8 or 16 bars from bar 1) and the tempo.
- **The takes list** shows the newest take first and dims undone takes, with Undo and Redo.

`TimeState::from_engine` reads what they show from the transport, the project, the launches and the takes, and `action()` turns touches into `TimeAction`s. `mpfree-time-preview` draws sample screens from a real transport, player, recorder and take undo as PNG files.

## In the pad player (`TimeHost`)

`mpfree-hello`'s [`time.rs`](../../rust/mpfree-hello/src/time.rs) runs the engine in the audio loop:

1. Each period the loop hands `TimeHost::pad` the pad notes it is about to play, after the arp, at their frame offsets.
2. `TimeHost::period` runs the transport and the player. The clips play the current kit's pads through the sampler (`hit_at`, `release_at`); a clip's note numbers are pad numbers, 0 to 15.
3. The armed track records, and the taken and erased notes go into the clips and the take history.
4. While counting in and recording, a click plays on each beat: 1760 Hz on the bar and 1320 Hz on the other beats, at about −12 dBFS.

It starts with 8 empty tracks, with track 1 armed. A clip made from an empty slot is as long as the loop, or 2 bars without one. The tempo belongs to the app, so TAP TEMPO, the menus and the Clips page all set it, and the transport follows. Only the position (about 30 times a second) and real changes go to the screen.

Edits (launches, arming, takes and recorded notes) compile a new `PlayView` in the audio loop. That allocates, but only on an edit, not every period. Moving the edits to a control thread is a later step. The status line every 30 s adds the transport state, the takes and any notes lost to full tables (should stay 0).

## Next

- Try the time engine on the MPC (round 6), with REC, OVERDUB, UNDO and PLAY START learned by the button guide.
- Move edits out of the audio loop.
- CAPTURE (SHIFT+REC keeps what was just played, even without recording).
- The looper (T4).
