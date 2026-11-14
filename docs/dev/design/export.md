<!--
covers: rust/mpfree-export rust/mpfree-fx/src/bounce.rs rust/mpfree-midi/src/smf.rs
checked: 2026-11-14 203b908
-->

# Export: mixdown, stems and MIDI file

**Status:** 2026-11-14. The engine is built and tested: [`rust/mpfree-export`](../../../rust/mpfree-export) (the pass, Fast or Listen, the files), on `mpfree_fx::bounce` ([fx.md](fx.md#bouncing-the-exports-mixdown-and-stems)) and `mpfree_midi::smf` ([midi-io.md](midi-io.md)). The app's part (the Export button and panel in BROWSE, Prefs > Export, the pass in the sound loop, the notes for the MIDI file) is being built by the FX thread, which the app thread agreed on 14 Nov. The plan Jay reviewed is the doc "Export plan: mixdown, stems and MIDI file".

## What an export makes

One pass over the song makes three things, lined up to the sample, in one new folder:

| File | What is in it | Format |
|---|---|---|
| Mixdown | The master after Master safety: what the speakers get | Stereo WAV, 24-bit, the session's rate, not normalised, not dithered |
| Stems | One file per track that plays, after its fader, pan and mute, and one per return, without Master safety | One stereo WAV each, all the same start and length |
| MIDI file | The notes and controls each track played in that pass, as the player sent them (quantise, swing, chance and ratchets as they came out), with the tempo map and meter | Standard MIDI File type 1, 960 ticks a quarter |

- **Range.** The arrangement from its start to its end, or, with no arrangement, the clips playing now for a number of bars (8 by default).
- **Tail.** After the range, the pass plays on until every file has been silent (below about -90 dBFS) for 2 s, so a slow delay's next echo isn't lost, and at most 10 s. That silence isn't kept: the files are cut where the last sound ends, never inside the range.
- **Where.** A new folder in the library's Recordings, named after the project and the time. Names are made safe for a FAT or exFAT stick: no `/ \ : * ? " < > |`, and no two files that differ only in case (a second "Drums" is "Drums 2").
- **Cancel or failure** (the stick fills or can't keep up) stops the export and removes its files; the panel says why.

## Fast or Listen

Jay's decision (14 Nov 2026, 01:32Z): build the listening mode first, then the fast quiet mode, make fast and quiet the default, let Prefs switch between them, and switch to listening by itself when the song needs outside gear.

- **Fast, quiet** (the default): the sound loop runs the song as fast as the processor allows, the speakers quiet. About 2 to 3 times faster than real time for a typical song on the MPC One, close to real time for one that already works it hard.
- **Listen**: the song plays once in real time and you hear it as it records, faders and all.
- **When an export listens anyway** (`mpfree_export::choose`): a track plays outside gear over MIDI, the line input is on, the CV or gate jacks are in use, MIDI clock goes out, part of the sound is made on another computer (offload), or the song has plugins (until the plugin host can wait for a late block instead of dropping it). The panel says which, in words (`Why::words`).

## The engine

[`rust/mpfree-export`](../../../rust/mpfree-export):

- **`Plan`**: the folder (new or empty), the song's name, the streams (a mixer capture point and a WAV file each: `Stream::mixdown()`, `Stream::stem(name, strip)`), the MIDI file's track names, tempo map, meter and end (`MidiPlan`), the mode, the rate, the range's length when known (for the time left), and the first of the mixer's capture slots the export may use.
- **`start(plan)`**, off the sound thread: makes the folder and the files, the bounce's rings (2 s a file) and a writer thread (`mpfree-export`, emptying the rings every 20 ms and syncing the files to the stick at the end). It gives back two halves.
- **`Pass`**, the sound thread's, which never allocates:
  - `attach(&mut mixer)` sets the export's capture points and takes their latencies (once the mixer has played, as the app's always has). A refused capture point fails the pass at once and leaves none set.
  - `period(&mixer, frames, ended)` after the mixer each period: copies every capture point's block into its ring. `ended` says the range is over; the pass then plays the tail and says `Done` once the range's last frames have reached every stream and the tail is silent or 10 s long. `Cancelled` when Cancel was pressed; `Failed` when blocks were lost or a capture point was refused.
  - `note(tick, track, msg)` logs a message the player sent (`mpfree_midi::smf::NoteLog`, fixed room, the overflow counted).
  - `wait_for_room(frames, most)`, in fast mode only, before each block: waits for the writer, so a fast pass never loses a block. A pass in real time is paced by the sound card and never waits.
  - `detach(&mut mixer)` frees the capture points; the pass then goes back to the screen.
- **`Job`**, the screen's: `progress()` ("1:12 of 3:50", "3:52 · the tail"), `cancel()`, and `finish(pass)`, which waits for the writer, cuts the tail's silence, writes the MIDI file (only when notes played) and gives the `Outcome` with a line for the panel.

## The app's part (being built)

- **Prefs > Export**: Fast, quiet | Listen, a row before Touch screen, saved with the Prefs (layout spec section 5, Jay's ask of 14 Nov 01:32Z).
- **The Export button** in the BROWSE overlay next to Save and New project, and its panel over the browser: Mixdown, Stems and MIDI file ticked, the range, Start; while it runs the time done and left and Cancel; when done the folder and Show in BROWSE (layout spec section 3, Jay's pick on 14 Nov 01:39Z).
- **The pass in the sound loop** (`main.rs`): the screen sends the pass; the loop attaches it, puts the transport at the range's start and plays; after the mixer each period it calls `period`; when the pass is over it stops the transport, detaches it and sends it back. Edits that change a latency wait while it runs.
- **Fast mode in the sound loop**: the sound card is paused (no writes, so no underruns are counted), the lanes on other cores are waited for instead of dropped, and the loop waits for room before each block; at the end the card is prepared and plays again.
- **The MIDI file's notes**: each event the player sends a track goes to `Pass::note` as MIDI Out would send it (a pad as its note).

## Tests

- `mpfree-export/tests/export.rs`, against a real mixer with 300 frames of look-ahead on a track: the mixdown is the mixer's output sample for sample from the song's first frame, and each stem its track alone; the files reach into the tail and stop where it has died away; either mode and any block size make the same files; a silent end keeps the whole range; a tail that never dies stops 10 s after the range; a cancel leaves no file or folder; a folder with files in it is refused, as is an export with nothing to make; a mixer without a strip the plan asks for fails at once; the MIDI file has the conductor and a track for each that played, and a held note is let go at the end.
- `mpfree-export/tests/no_alloc.rs`: attach, a pass's periods with notes past the log's room, waiting for room and detach never allocate on the sound thread.
- The mode rule (each reason makes an export listen, outside gear is named first, Listen chosen is Listen), names a stick takes and the progress words, in the crate's unit tests.
