# Capture: keep what you just played

**Status:** roadmap step T3's last part, 2026-11-11. The engine is [`rust/mpfree-capture`](../../../rust/mpfree-capture). The Clips page's Capture button is in [`rust/mpfree-time-ui`](../../../rust/mpfree-time-ui), and take undo for captures is in [`rust/mpfree-play`](../../../rust/mpfree-play) (`Takes`). The pad player runs it in `TimeHost` (`rust/mpfree-app/src/time.rs`), on its control half ([time-threads.md](time-threads.md)), fed the pad notes and gestures the audio half takes. All of it is tested on the host. Nothing here has run on the MPC yet.

You play the pads without recording, then decide you liked it. CAPTURE keeps it. The MPC's own Retro Rec does the same thing into the selected track (MPC 3.6 user guide, page 67).

## What it keeps

- **Always listening.** Every note the pads play goes into a history, whether the transport rolls or not, and whether or not anything records. On a track through the arp, the history gets the arp's notes, which is what you heard.
- **Routed keys too** (18 Nov 2026, the MIDI powerhouse handoff's item 7). Keys a MIDI route plays on a track that isn't armed (the I/O view's Routing, [midi-io.md](midi-io.md#routes-keyboards-on-several-tracks)) go into the same history, named by their track (`Capture::input_on`); the pads' notes are the armed track's (`ARMED`). Each track holds its own notes, so the same note on two tracks is two notes, and a track's All off ends only its own. One phrase covers them all: it ends on a bar of silence on every track.
- **The last phrase.** A phrase is what was played since the last silence of at least one bar. A phrase is either all played while the transport rolled (in time) or all played while it was stopped (free). Starting or stopping the transport starts a new phrase.
- **Whole bars.**
  - **In time:** the clip starts on the bar line at or before the first note. Each note keeps its place in the bar.
  - **Free:** the clip starts on the first note, at the current tempo.
  - **Either way:** the clip ends on the bar line after the last note starts.
- **Up to 32 bars.** A longer phrase keeps its last 32 bars. A clip holds at most 8192 notes, the last ones played.
- **Lengths.** A note ends where its pad was released. A note still held ends at the moment of the capture.
- **Exact timing.** Notes land on the nearest tick to the frame they were played on. Nothing is quantised. The clip's quantise setting can still be turned on afterwards, because quantise never changes the stored notes.
- **Pad pressure.** Each note keeps the pressure played on it, thinned as recording thins it ([playback.md](playback.md#pad-pressure-goes-on-its-note)): a value 2 or more steps from the last one kept at once, a one-step value once it has lasted 10 ms, a value the same as the last kept never, a new press starting from 0. Values come in through `Capture::input` as `Event::Pressure` (the pad and its raw value) and are kept only while their pad's note is held. They are timed in ticks from their note's start (`Phrase::touch`, each note's `touch` naming its run), on the same clock as the notes, so a phrase is the same for any period size, rolling or stopped.

## Where it goes

CAPTURE puts the pads' notes in the armed track (track 1 if none is armed), and each routed track's notes in that track (`Phrase::split`: the armed track's part first, then the others in track order; a key routed to the armed track itself joins its part). Every part keeps the phrase's length, bar line and song start, so their clips play together as they were played. It is one take for them all: the first part's capture, which the others join (`Takes::joining`), so one UNDO takes every track's notes back. Each track's part goes in as follows:

- **Over the playing clip:** if the phrase was played in time while the track's clip was playing, its notes go into that clip, each on the clip tick where it was played. It is as if you had been recording in overdub.
- **Otherwise, a new clip:** it goes in the track's first empty slot, named "Capture 1", "Capture 2" and so on, and it is launched in place of the track's clip.
  - A phrase played in time keeps its place against the song. If you played it over bars 3 and 4, bar 5 is its first bar again, so it plays on in time straight away.
  - A free phrase starts on the next bar line (when stopped, on the current bar's line).
- **FX moves go with it,** with the armed track's part (their addresses are the armed track's). The Perform FX moves made over a phrase played in time (the XY pad, the DJ knob, a pad's Hold, kept always in a move log while the transport rolls) go into the same clip as moves, on the ticks where they were made, as part of the same capture (`mpfree_capture::moves_for_clip`, `Takes::capture_moves`). The log line counts them ("capture 2: 6 notes and 1 move, ..."). A free phrase keeps its notes only. When the armed track's part isn't kept (only routed keys played, or its track refused), the moves stay out and stay in the log, and the line counts them ("1 move left out: it goes with the armed track's notes"). See [playback.md](playback.md#fx-moves).
- **Pressure goes with it** when the track records pressure (`Track::record_touch`, the Record list's Pressure row). In a new clip the clip's pressure is the phrase's. Over the playing clip, each value moves to the clip's ticks as the note lengths do (twice as far at double speed), and the runs go after the clip's own pressure (`Takes::capture_notes`, given the runs). A playing clip with no room for them takes the notes without their pressure; `Kept::touch_lost` counts the values left off, the log line says so, and they add to the pressure losses the status line shows. The log line counts the values kept ("capture 1: 1 notes with 3 pressure values, ..."). A track with Pressure off keeps the notes without it.
- **Undo.** UNDO takes a capture back like any take. Notes come out of the clip, or the new clip leaves its slot and stops. REDO puts it back. After an undo, the same phrase can be captured again.
- **Nothing new.** After a capture, the Capture button goes quiet until another note is played, so pressing it twice does not make two copies.
- **Refused.** It is refused, and nothing changes, when nothing was played, the track does not exist, or all 16 of the track's slots hold clips. With several tracks, a track refused is left out and the log line says why ("Bass left out: every slot holds a clip"); the others go in. Only when every track is refused does nothing change.
- **The log line** names each track's part: "capture 1: 1 notes, 2 bars, new clip in slot 1 of Track 1; 2 notes, new clip in slot 1 of Track 3". Undo names the first part's track and slot.

## The clock

The history puts every note on a clock that never goes back.

- **While the transport rolls,** the clock counts song ticks and keeps the song's bar lines. A loop of whole bars, or a jump from one bar line to another, goes on without a seam, so a phrase can run across a loop jump. A jump that is not on the bar grid moves the clock forward to the next point with the same place in the bar.
- **While it is stopped,** the clock counts ticks at the current tempo.

Each note also keeps the song tick it was played on. That is how notes over a playing clip find their clip ticks, even across loop jumps.

## On the screen

- **The Takes panel** on the Clips page has three buttons: Capture, Undo and Redo.
  - Capture shows what it would keep ("Capture 2 bars") with an accent outline. When there is nothing new to keep, it is greyed out.
  - Captures show in the list as "Capture 1 · Chords / 2", and "new +6" when the capture made a new clip.
- **Undo and Redo** name a capture "capture 1", and a recorded take "take 3", on the buttons and on the transport bar.
- **The sample screens** (`mpfree-time-preview`) show the button before and after a capture (`clips-capture-ready`, `clips-captured`).
- **SHIFT + REC** as CAPTURE is proposed for the pad player, on every page. It is a shared button, so it waits for the owner's approval.

## Real time

`Capture::input`, `Capture::process` and `Capture::pending` run on the audio thread and never allocate. The history is a fixed ring:

- 16384 note starts and ends: 8192 notes, about 8 minutes of 1/16 notes on two pads at 120 BPM;
- 256 notes a period;
- a ring of its own for pressure: 32768 thinned values (`MAX_TOUCH_HITS`), each with the number of note starts and ends before it, so each value goes on the note its pad held then. A flood of pressure pushes out old pressure, never notes. A phrase whose first notes' values were pushed out says so (`Phrase::touch_cut`), and the log line adds "the first notes' pressure was older than the history keeps";
- 512 pressure values a period (`MAX_TOUCH_INPUT`), in a queue of their own.

`Capture::dropped()` counts notes and pressure values lost to a full input table. It should stay 0.

In the pad player the audio half sends each pad's pressure to the history as it sends the notes (`ToControl::Played`), whatever the armed track records, but only a value that changed since the last one it sent for that pad (a press sends the next one again).

Routed keys reach the history from the sound loop (`main.rs`, where `play_routed` plays them): `midi_in::RoutedPads` turns each key into the pad that plays its note alone on its track, as the track's clips play (`midi_file::pad_on_track`: the track's own instrument's pad map, the synth's pad layout, or the kit pad that sends the note, in any bank), and remembers it per track and note, so the key's end and pressure go to that pad even if the pads change while it is held. A note no pad plays alone (an instrument's or the synth's pads) plays but isn't kept, as with the armed track's keys. `TimeHost::routed_played` sends it as `ToControl::PlayedOn` (track, offset, event). A track's held notes are a voice: the armed track's is voice 0, and up to 31 routed tracks hold notes at once (`MAX_VOICES` = 32, each 128 notes); a note on one more is left out and counted in `dropped()`. A voice is free again once its track holds nothing. A routed key's end comes even when the key isn't let go on the keyboard: unplugging it, or taking its route away, lets go of its held keys through the router (`MusicIn::unplugged`, `set_routes`), and those ends reach the history as any other. `routed_played` also takes `AllOff`, which ends every note the track's voice holds, so nothing can hold a phrase open for good.

`Capture::phrase` and `keep` allocate. In the pad player they run when the button is pressed, like the other edits.

## Tests

- `src/tests.rs` (21 tests):
  - a phrase in time starts on its bar and keeps its place; a free phrase starts on its first note;
  - a bar of silence, or the transport starting, ends a phrase; less than a bar does not;
  - a held note ends now, and a pad pressed again ends the held note;
  - a long phrase keeps its last 32 bars;
  - a phrase goes on across a loop of whole bars, and each note keeps its song tick;
  - every period size (1, 64, 128, 1000 and 4096 frames, with a tempo change) gives the same phrase, rolling or stopped;
  - the button goes quiet once a phrase is kept, until the next note; the history is a ring;
  - `keep`: a new clip in time plays on in its place (in a loop too); a free one starts on the next bar; notes over the playing clip go into it; refusals change nothing;
  - pressure goes on the note its pad held, thinned as recording thins it (a one-step value given way, one kept 480 frames after it came, one kept as the pad lets go, a repeat, a value after letting go and one on a pad not held), the same at 1, 64, 128, 1000 and 4096 frames, rolling or stopped;
  - the pressure ring is its own: 36000 values over 120 notes keep every note and the last 32768 values, the first ten notes' runs cut and the phrase saying so;
  - kept notes keep their pressure: in a new clip, left out on a track with Pressure off, on a double-speed playing clip's ticks after its own pressure, and left off and counted when that clip is full;
  - routed tracks: their notes are held apart (the same note on the pads and on a routed track is two notes) and part by track, armed first, every part as long as the phrase; a bar of silence on every track ends the phrase; a track's All off ends only its own notes; pressure goes on its own track's note and stays with it when the phrase parts; a note on one track more than can hold is left out and counted; parts kept with `Takes::joining` are one take that one UNDO takes back; a routed track's All off ends a note whose end never came, so the next note after the quiet starts a phrase of its own.
- `tests/no_alloc.rs`: a counting allocator watches the capture follow a busy transport long enough for the history to wrap, at periods of 1, 128 and 4096 frames, with pressure on the held pads every 8 frames and keys routed to 40 tracks in turn. It sees no allocation and no free.
- `mpfree-play`'s `takes.rs`: a captured clip is undone and redone as a take, and a slot filled meanwhile keeps its clip. Recorded notes still join their own take when a capture has the same number. Captured notes go in all together or not at all, their pressure after the clip's own, or without it in a clip with no room. Captures made `joining` another are one take, and can't join one that isn't the latest take or has a take undone since.
- `mpfree-app`'s `time.rs`: `capture_keeps_the_pressure_played_on_its_notes` plays a pad squeezed harder and let up without recording, captures it, and hears the pressure play back with the note every bar; with Pressure off the capture keeps the note alone. `capture_keeps_routed_keys_in_their_own_tracks_as_one_take` plays the pads and keys routed to two tracks and one that doesn't exist: three new clips, the log line naming each and the track left out, one UNDO and one REDO for all. `capture_of_routed_keys_alone_leaves_the_moves_out_and_says_so`; `a_routed_all_off_ends_a_held_note_for_capture`. `midi_in`'s `capture_keeps_a_routed_key_on_the_pad_its_note_went_to` and `a_routed_key_let_go_by_an_unplug_or_a_route_change_ends_for_capture`, and `rt_safety::routed_keys_play_their_track_where_its_clips_play` (no allocation, with CAPTURE's path).
- `mpfree-time-ui`: the button, the take rows, and a sample session that captures a riff as a new clip in place, captures over a playing clip, and undoes both.

## Listening

`cargo run --release -p mpfree-capture --example capture_demo` writes `~/mpfree-work/capture-demo.wav` (34.5 s). The tool synthesises every sound itself.

1. A two-bar beat plays at 90 BPM.
2. In bars 3 and 4, a chord stab riff is played on the pads, a little off the grid, without recording.
3. CAPTURE in bar 4 makes it a clip that plays on from bar 5 with the same timing.
4. In bar 7, clanks are played on the off-beats over the clip.
5. CAPTURE in bar 8 puts them into the clip, so bars 9 and 11 have them.

The tool checks every bar (what played live and what played from the clip, each within a tick of where it was played). It also undoes both captures on a copy. It prints an events hash, and it checks that the hash is the same at periods of 1, 128 and 4096 frames.
