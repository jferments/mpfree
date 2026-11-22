# Recording: what a take keeps

**Status:** step sequencer refactor E4, 2026-11-22. This page is the rule every way of recording notes follows, and where each one keeps it. The code is in `rust/mpfree-app/src/midi_file.rs` (`clips`), `rust/mpfree-app/src/time/audio.rs` (`TimeAudio::heard`, `history`, `routed`, `erase_sound`), `time/control.rs` (`take_holds`, `sync_tracks`), [`rust/mpfree-capture`](../../../rust/mpfree-capture) (`Capture::input_as`, `input_on`, `keep`) and `main.rs` (`notes_tracks`, `pad_rows`). It is tested on the host. Nothing here has run on the MPC yet.

When you play, three things could be kept: what you touched (a pad, a key), the note it made, or the sound. Before E4 the answer depended on the path: REC on the armed track kept notes on a synth track, but CAPTURE, a second armed track and the shared synth's tracks kept the pad you touched, so a clip on a synth track could hold a pad's number where you heard a pitch. Every path now follows one rule.

## The rule

1. **A track whose instrument plays notes records the notes.** That is a track's own synth, the one it plays of another track's, or the shared synth (the track's kit is the Synth kit), unless the instrument is the Drums engine. The notes are as written: a pad's chord is its notes, root last, after the pad layout, scale, chord and octave, and before the live transpose (S4) moves them. A new clip holds notes (`Holds::Notes`).
2. **A track whose instrument plays pads records the pads.** That is the Drums engine and a kit. A new clip holds pad hits (`Holds::Hits`).
3. **A clip never holds both.** A take into a clip that is already there records in that clip's kind, so an old clip of pad hits on a synth track stays one, and its takes record the pads. CAPTURE does the same (below).
4. **The sound is never a note take.** The looper, the sampler's pad takes and the audio tracks keep sound.

Which tracks play notes is worked out once, in the main loop: `notes_tracks` in main.rs (each pass) gives `Instruments::note_tracks`, plus each track with no instrument of its own that plays the shared synth while the synth isn't the Drums engine. `TimeHost::set_notes_tracks` hands the bits to the control half, whose `TimeControl::take_holds` gives a new clip's kind for every path that makes one. The audio half needs no list of its own: a take records in the kind of the clip it goes into (`TimeAudio::takes_notes`), which the control half made or found before recording. The notes each track's 16 pads play (`TimeHost::set_pad_rows`, a `PadRows` each) reach the audio half too (`ToAudio::Pads`), so a pad on any track can be turned into its notes there.

## Each path

| Path | Track that plays notes | Track that plays pads | Code |
|---|---|---|---|
| REC and OVERDUB, the armed track's pads and keys | the notes (a chord pad's every note), each with the pad's pressure | the pad | `TimeAudio::heard` |
| Through the track's arp | what the arp played, which is notes ([5d](instruments.md#5d-as-built-one-path-through-the-arp)) | the arp's pads | the recorder |
| Pads as pressed ("Clips through the arp") | the notes the pad plays | the pad | `TimeAudio::pad_pressed` |
| A second armed track (SHIFT + its row), its routed keys | the notes the key's pad plays on that track, each end and pressure on the notes it started | the pad | `TimeAudio::routed`, `routed_heard` |
| Pad layers and route synths (the instrument redesign) | as a second armed track: they arrive on `routed` | as a second armed track | `TimeAudio::routed` |
| CAPTURE, the armed track's part (track 1 when none is armed) | the notes, with their pressure | the pad | `TimeAudio::history`, `mpfree_capture::keep` |
| CAPTURE, a routed track's part | the notes the key's pad plays there | the pad | `TimeAudio::routed`, `mpfree_capture::keep` |
| Live ERASE (ERASE held on a pad while playing or recording) | on a clip of notes, every note the pad plays on the armed track, following the pads if they change while it is held | the pad's notes | `TimeAudio::erase_notes` |
| ERASE + a pad while stopped | every note the pad plays | the pad's notes | `StepAction::ClearPad` through `on_notes` |
| A step tapped on the grid | the pad's notes ([the step grid](instruments.md#5c-as-built-the-step-grid)) | the pad | `TimeControl::step_action` |
| A MIDI file opened from BROWSE (not a take, but it follows the rule) | a clip of notes, every note as it is | each note on the pad that plays it (`pad_on_track`); a note no pad plays is counted | `midi_file::clips` |

"A track that plays notes" in the table is the clip's kind where a clip is already there: a take into a clip of pad hits records pads, as rule 3 says.

### CAPTURE

CAPTURE keeps what you played before it knows which clip it will go in, so its history keeps each touch both ways where it has both: the pad, and the notes it played, each marked as paired (`Took`, sent by `TimeAudio::history` and `routed`). A pad and a note of one number are two different notes (kept apart in each voice, `key`). `keep` then takes one kind for each track's part, as a take there would record: over the clip playing on the track, that clip's kind (an overdub's rule); else the kind a new clip there holds, which the control half says (`TimeControl::take_holds`). A touch the history has only as the other kind is left out, and the log line counts it: a pad that plays no note there ("1 pad hit left out: no note played there"), or a note that came with no pad, such as the arp's ("2 notes left out: the clip holds pads, and no pad played them"). A part with nothing of its kind changes nothing (`KeepError::OtherKind`), and the line says why.

### Pressure

A pad's pressure goes on each note the pad started, whichever path took them: `pad_notes` keeps the armed track's (for a take and for CAPTURE's history), and `routed_notes` keeps each routed track's, so a key's end and pressure reach the notes it started even if the pads change while it is held. CAPTURE's history sends a value only when it changed for that pad, as before; a new press sends the next one again.

## Not yet

- **A routed key whose note no pad plays alone on its track isn't kept**, on any track (`midi_in::RoutedPads`, `midi_file::pad_on_track`): routed keys still reach the time engine as pads, and the audio half turns them back into notes. A keyboard routed to a synth track records only the notes its 16 pads play alone. The armed track's own keys don't have this gap. Passing a routed key's note as it is waits for the instrument redesign's route work to land, since it changes the same loop in main.rs.
- **Convert to notes** will turn an old clip of pad hits on a synth track into a clip of notes, on the Clip tab; it comes next in the step sequencer refactor.

## Tests

- `time::tests::a_second_armed_synth_track_records_the_notes_its_routed_pads_play`: a second armed synth track gets a new clip of notes, a routed key records its pad's chord, each note with the key's pressure, and a pad that plays no note there is counted, not recorded.
- `time::tests::capture_on_a_synth_track_keeps_what_a_take_there_would`: a chord pad played without recording and captured on a synth track lands as its notes in a new clip of notes with no clip playing, as its pad in a playing clip of pad hits (the line says so), and as its notes in a playing clip of notes.
- `time::tests::live_erase_on_a_clip_of_notes_takes_out_the_notes_its_pad_plays`: ERASE held on a chord pad takes out its notes, not the note with the pad's number.
- `midi_file::tests::on_a_track_that_plays_notes_a_file_keeps_its_notes`: a part on a track that plays notes is a clip of notes with every note, one on a kit track its pads.
- `tests::a_track_on_the_shared_synth_records_notes` (main.rs): a track on the shared synth and a track with its own synth play notes; the others, on kits, don't.
- mpfree-capture: `a_pad_and_a_note_of_one_number_are_two_notes`, `capture_over_a_playing_clip_keeps_that_clips_kind`, `a_touch_kept_only_as_the_other_kind_is_left_out_and_counted`, and the no-allocation test as before.
- The takes of 5c and 5d as before ([instruments.md](instruments.md#5c-as-built-takes-of-notes)).
