# Recording: what a take keeps

**Status:** step sequencer refactor E4, 2026-11-22. This page is the rule every way of recording notes follows, and where each one keeps it. The code is in `rust/mpfree-app/src/midi_file.rs` (`clips`), `rust/mpfree-app/src/time/audio.rs` (`TimeAudio::heard`, `history`, `routed`, `erase_sound`), `time/control.rs` (`take_holds`, `sync_tracks`), [`rust/mpfree-capture`](../../../rust/mpfree-capture) (`Capture::input_as`, `input_on`, `keep`) and `main.rs` (`notes_tracks`, `pad_rows`). Convert to notes (step sequencer refactor E1, [below](#convert-to-notes)) turns an older clip of pad hits into notes. It is tested on the host. Nothing here has run on the MPC yet.

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
- **Trigger groups.** A step's chord rolls its chance once per note, so Convert refuses a hit with chance below 100 %. Trigger groups (the notes one hit made rolling together) will lift that; they need a short design of their own first.
- **Rows for notes no pad plays** on the Steps come in a later phase; the piano roll edits those notes meanwhile.

## Convert to notes

Rule 3 keeps an older clip of pad hits on a synth track as it is, so it plays whatever its pads play now, and a new scale, layout or octave moves its tune. Convert to notes makes one such clip a clip of notes, on purpose and one at a time, keeping exactly what it plays now (design doc rev 15, https://claude.ai/artifact/UtLWMnMM4rxP7uiBNqa4TU). The code is `rust/mpfree-seq/src/convert.rs` (`Clip::convert_to_notes`, `Refused`), `time/control.rs` (`convert_preview`, `convert`, `refused_words`) and the clip card's Melody row in `rust/mpfree-time-ui/src/view.rs` (`CLIP_MELODY`, `Converting`, `clips_card_hint`).

- **What each hit becomes:** the notes its pad plays now (the track's `PadRows`, the map playback and the step grid use, before the live transpose, so the transpose still applies once), a chord pad every note, root last. Each note copies the hit's tick, length, velocity, chance, condition, ratchet and ramp, tie or slide, on or off, and touch run (a chord's notes share the run, which `Takes::reclaim_touch` renumbers as one). A lock with a pad (`Lock::sound`) moves to the pad's root; a whole-step lock stays. The original Mutate keeps (`Clip::original`) is converted the same way, so Restore brings back notes. Moves and settings stay.
- **All or nothing.** It refuses, and lists every reason, the clip's first and each once: a hit with chance below 100 % (`Chance`: a chord's notes would each roll), a lock on a pad's own setting (`PadLock`, `OnTrack::Pad`: no note to follow), a hit or lock on a pad that plays nothing (`NoNote`), two pads' notes meeting on one pitch (`Overlap`, below), more than `MAX_NOTES` notes (`Room`), and two pads with one root and locks of one setting on one step (`LocksMeet`). A reason found only in the original is marked so. A loop's cell and a clip of notes are refused alone.
- **Two pads on one pitch.** The player cuts a sounding note when its pitch starts again on the track, and a tie or slide continues the pitch it meets, while two pads were two sounds. So where two pads' notes on one pitch overlap (on a pass or across the clip's end, which a note past it sounds over), or one ties or slides onto the other's end, the notes couldn't play the same: it refuses rather than merge or shorten them. Hits that are off never sound. One pad meeting itself is what it played already. This reason is not in the design doc's first list: it was found checking the player.
- **Preview, then Convert.** › on the Melody row sends `TimeAction::ConvertPreview`. The control half converts a copy, keeps the clip and the pads it read (`ConvertAsked`) and says what it would make, or why not, adding STOP while the transport rolls; `TimeState::converting` puts that line on the hint line (in place of `CLIPS_CARD_HINT`) and makes the card's buttons Cancel and, when it would convert and the transport is stopped, Convert. `TimeAction::Convert` checks the clip and the pads are as read and the transport stopped, then puts the clip in through `TimeControl::replace` as one take (`Replaced::Convert`), so UNDO and REDO swap it whole. Cancel, Convert, or any other action but the transport closes the preview; STOP leaves it open for its Convert. A kit's track (`take_holds`), a loop's cell or a clip of notes opens none.
- **A pad that plays nothing.** `PadRows::set` with no notes now marks a pad that plays nothing (its notes outside 0 to 127), so Convert refuses its hits. The step grid still shows its row on its own number (`PadRows::roots`), and a clip's hit on it into an arp that works in notes plays nothing, as on its instrument (it played the pad's number as a note before).
- **A pad's own setting on a clip of notes.** A new step lock on one (`OnTrack::Pad`) is refused on a clip of notes: it would end on a pitch, not the pad.
- **Saved states.** Reload brings a saved state back as it was saved, so one saved before converting follows the pads again; it is not converted with the clip.

## Tests

- `time::tests::a_second_armed_synth_track_records_the_notes_its_routed_pads_play`: a second armed synth track gets a new clip of notes, a routed key records its pad's chord, each note with the key's pressure, and a pad that plays no note there is counted, not recorded.
- `time::tests::capture_on_a_synth_track_keeps_what_a_take_there_would`: a chord pad played without recording and captured on a synth track lands as its notes in a new clip of notes with no clip playing, as its pad in a playing clip of pad hits (the line says so), and as its notes in a playing clip of notes.
- `time::tests::live_erase_on_a_clip_of_notes_takes_out_the_notes_its_pad_plays`: ERASE held on a chord pad takes out its notes, not the note with the pad's number.
- `midi_file::tests::on_a_track_that_plays_notes_a_file_keeps_its_notes`: a part on a track that plays notes is a clip of notes with every note, one on a kit track its pads.
- `tests::a_track_on_the_shared_synth_records_notes` (main.rs): a track on the shared synth and a track with its own synth play notes; the others, on kits, don't.
- mpfree-capture: `a_pad_and_a_note_of_one_number_are_two_notes`, `capture_over_a_playing_clip_keeps_that_clips_kind`, `a_touch_kept_only_as_the_other_kind_is_left_out_and_counted`, and the no-allocation test as before.
- The takes of 5c and 5d as before ([instruments.md](instruments.md#5c-as-built-takes-of-notes)).
- mpfree-seq `convert::tests`: each hit becomes its pad's notes and keeps the rest; two pads meeting on one note are refused, and apart they convert; locks move to their pad's root and whole-step locks stay; every reason is listed and nothing changes; chord pads past a clip's room are refused; the original Restore brings back is converted too.
- mpfree-play `tests/convert.rs`, the main test: a bar of hits (a chord pad with pressure, ratchets with a ramp, a tie, a slide, a condition, two pads on one note apart in time, a hit that is off, a note past the end, locks) and its conversion play the same note-ons, note-offs, pressure, locks and unlocks on the same frames over four passes, launched, in a song and saved and opened again, at periods of 64, 333 and 4,096 frames.
- `time::tests::convert_to_notes_previews_then_converts_in_one_undo`: the preview changes nothing, the pads changing or another action closes it, STOP first, Convert as one take, UNDO and REDO, Restore after it gives notes, no preview on a clip of notes or a kit's clip, every reason at once and only Cancel.
- `view::tests::the_clip_cards_melody_row_previews_convert_to_notes`: the row's words, › and its refusals, the preview's buttons and hint line.
