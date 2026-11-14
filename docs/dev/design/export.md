<!--
covers: rust/mpfree-export rust/mpfree-fx/src/bounce.rs rust/mpfree-midi/src/smf.rs rust/mpfree-browser-ui/src/export.rs rust/mpfree-app/src/export.rs
checked: 2026-11-14 97a6b00
-->

# Export: mixdown, stems and MIDI file

**Status:** 2026-11-14. Built and tested: the engine, [`rust/mpfree-export`](../../../rust/mpfree-export) (the pass, Fast or Listen, the files), on `mpfree_fx::bounce` ([fx.md](fx.md#bouncing-the-exports-mixdown-and-stems)) and `mpfree_midi::smf` ([midi-io.md](midi-io.md)); and the app's part (the Export button and panel in BROWSE, Prefs > Export, the pass in the sound loop, the notes for the MIDI file), which the FX thread wrote with the app thread's agreement (14 Nov). Listen runs in a test of the sound loop's steps; Fast, quiet stops and restarts the app's own sound card stream, which has not run on the MPC yet. The plan Jay reviewed is the doc "Export plan: mixdown, stems and MIDI file". The musician's page is [Exporting your music](../../user/src/making-music/exporting.md).

## What an export makes

One pass over the song makes three things, lined up to the sample, in one new folder:

| File | What is in it | Format |
|---|---|---|
| Mixdown | The master after Master safety: what the speakers get | Stereo WAV, 24-bit, the session's rate, not normalised, not dithered |
| Stems | One file per engine track (Drums, Synth, Loop, Audio), per track instrument in use and per return, after its fader, pan and mute, without Master safety. A stem that never went above about -90 dBFS is left out (the mixdown never is) | One stereo WAV each, all the same start and length |
| MIDI file | The notes and controls each track played in that pass, as the player sent them (quantise, swing, chance and ratchets as they came out), with the tempo map and meter | Standard MIDI File type 1, 960 ticks a quarter |

- **Range.** The arrangement from its start to its end, or, with no arrangement, the clips playing now for a number of bars (8 by default).
- **Tail.** After the range, the pass plays on until every file has been silent (below about -90 dBFS) for 2 s, so a slow delay's next echo isn't lost, and at most 10 s. That silence isn't kept: the files are cut where the last sound ends, never inside the range.
- **Where.** A new folder in the library's Recordings, named after the project and the time ("Night drive 2026-11-14 01.32", UTC as the project's own names are; " 2" and on when taken), with the MIDI file named after the project. Names are made safe for a FAT or exFAT stick: no `/ \ : * ? " < > |`, and no two files that differ only in case (a second "Drums" is "Drums 2").
- **Cancel or failure** (Cancel, STOP before the range's end, the stick fills or can't keep up, or a capture point's latency changes because a look-ahead effect went in or out) stops the export and removes its files; the panel says why.

## Fast or Listen

Jay's decision (14 Nov 2026, 01:32Z): build the listening mode first, then the fast quiet mode, make fast and quiet the default, let Prefs switch between them, and switch to listening by itself when the song needs outside gear.

- **Fast, quiet** (the default): the sound loop runs the song as fast as the processor allows, the speakers quiet. Estimated, not yet measured on the MPC One: about 2 to 3 times faster than real time for a typical song, close to real time for one that already works it hard.
- **Listen**: the song plays once in real time and you hear it as it records, faders and all.
- **When an export listens anyway** (`mpfree_export::choose`, from a `Rig` the sound loop fills in): a track plays outside gear over MIDI, the line input is on, the CV or gate jacks are in use, MIDI clock goes out, the tempo follows MIDI clock coming in, part of the sound is made on another computer (offload), or the song has plugins (until the plugin host can wait for a late block instead of dropping it). The panel says which, in words (`Why::words`).

## The engine

[`rust/mpfree-export`](../../../rust/mpfree-export):

- **`Plan`**: the folder (new or empty), the song's name, the streams (a mixer capture point and a WAV file each: `Stream::mixdown()`, `Stream::stem(name, strip)`), the MIDI file's track names, tempo map, meter and end (`MidiPlan`), the mode, the rate, the range's length when known (for the time left), and the first of the mixer's capture slots the export may use.
- **`start(plan)`**, off the sound thread: makes the folder and the files, the bounce's rings (2 s a file) and a writer thread (`mpfree-export`, emptying the rings every 20 ms and syncing the files to the stick at the end). It gives back two halves.
- **`Pass`**, the sound thread's, which never allocates:
  - `attach(&mut mixer)` sets the export's capture points and takes their latencies (once the mixer has played, as the app's always has). A refused capture point fails the pass at once and leaves none set.
  - `period(&mixer, frames, ended)` after the mixer each period: checks every capture point's latency against the one `attach` took (a change fails the pass with `Failure::Latency`, since the stems would no longer line up), then copies every capture point's block into its ring. `ended` says the range is over; the pass then plays the tail and says `Done` once the range's last frames have reached every stream and the tail is silent or 10 s long. `Cancelled` when Cancel was pressed (`Job::cancel`, or `Pass::cancel` from the sound thread); `Failed` when blocks were lost or a capture point was refused.
  - `note(tick, track, msg)` logs a message the player sent (`mpfree_midi::smf::NoteLog`, fixed room, the overflow counted).
  - `wait_for_room(frames, most)`, in fast mode only, before each block: waits for the writer, so a fast pass never loses a block. A pass in real time is paced by the sound card and never waits.
  - `detach(&mut mixer)` frees the capture points; the pass then goes back to the screen.
- **`Job`**, the screen's: `progress()` ("1:12 of 3:50", "3:52 · the tail"), `cancel()`, and `finish(pass)`, which waits for the writer, cuts the tail's silence, removes the stems that never sounded (`BounceOut::heard`), writes the MIDI file (only when notes played) and gives the `Outcome` with a line for the panel.

## The app's part

- **Prefs > Export** (`Settings::export_mode`, saved as `export fast|listen`): Fast, quiet | Listen, a row before Touch screen (layout spec section 5, Jay's ask of 14 Nov 01:32Z).
- **The Export button and panel** ([`mpfree-browser-ui/src/export.rs`](../../../rust/mpfree-browser-ui/src/export.rs), layout spec section 3, Jay's pick on 14 Nov 01:39Z): Export in the Projects place sends `BrowseAction::ExportAsked`; the panel ticks Mixdown, Stems and MIDI file, picks the range (Arrangement, or 1 to 999 bars), says how it will play and where, and Start sends `ExportStart(choice)`. While it runs it shows the time done with Cancel (`ExportCancel`); when it is over, the outcome with Show in BROWSE. The panel remembers the last choice while the app runs. BROWSE hides it and a running export carries on.
- **The facts and the plan** ([`mpfree-app/src/export.rs`](../../../rust/mpfree-app/src/export.rs)): `FromUi::ExportAsk` and `FromUi::Export` are the audio loop's own (`Desk::screen` hands them back, since they need MIDI out, CV out and the card). On `ExportAsk` the sound loop fills in the `Rig` and calls `Facts::gather` (the rate, the arrangement's end, the playhead, the meter and tempo map, the track names and the stem strips, from the time engine and the mixer), and sends the facts to the screen, which opens the panel. The smoke tests' session can call `Facts::gather` with `Rig::default()`. On Start the screen makes the `Plan` (`Facts::plan`: the streams, the MIDI file's tracks and tempo map from the range's start, the range's length), calls `mpfree_export::start` and sends the `Pass` to the sound loop in a `Run`.
- **The pass in the sound loop** (`export::Host`): `begin` attaches the pass, remembers the transport (song mode, loop, position), stops it, clears the loop, sets song mode for the arrangement, locates to the range's start and plays. Each period after the mixer, `notes` logs what the player sent (`Tap`, below) at its tick from the range's start, and `period` hands the block to the pass with `ended` once the playhead reaches the range's end. In the period before the range ends, the host forecasts the frame of the range's end from the block's last span (`Span::frame_of`) and queues STOP on it (`TimeHost::command_at`), so the song stops exactly there: the clips play nothing after the range and the MIDI file gets nothing after it, while what still sounds (a release, an echo, a reverb) rings out as the tail. A transport that doesn't roll within 400 periods, or stops before the end (STOP), cancels the pass. When it is over the host detaches it, puts the transport back as it was and sends the `Run` to the screen, which calls `Job::finish` and shows the outcome. A second export while one runs is refused.
- **Fast mode in the sound loop:** while a Fast pass runs, the sound card stream is dropped (`Playback::pause`, so no writes and no underruns), each period waits for the writer's room instead of the card (`Pass::wait_for_room`, at most 5 s, with a 1 ms rest every 64 periods so the screen and MIDI threads get the cores), and the lanes on other cores are given 250 ms instead of the card's deadline, so a slow lane is waited for rather than dropped. When the pass is over the stream is prepared and refilled with silence (`Playback::resume`), as the underrun recovery does.
- **The MIDI file's notes** (`export::Tap`): each event the player sends a track (a clip's note, a pad as its note, the arp and live keys) goes through a `midi_out::Chords` as MIDI Out would send it, into a log with fixed room (1024 events a period), which `notes` hands to `Pass::note`.

## Tests

- `mpfree-export/tests/export.rs`, against a real mixer with 300 frames of look-ahead on a track: the mixdown is the mixer's output sample for sample from the song's first frame, and each stem its track alone; the files reach into the tail and stop where it has died away; either mode and any block size make the same files; a silent end keeps the whole range; a tail that never dies stops 10 s after the range; a cancel leaves no file or folder; a folder with files in it is refused, as is an export with nothing to make; a mixer without a strip the plan asks for fails at once; the MIDI file has the conductor and a track for each that played, and a held note is let go at the end.
- `mpfree-export/tests/no_alloc.rs`: attach, a pass's periods with notes past the log's room, waiting for room and detach never allocate on the sound thread.
- The mode rule (each reason makes an export listen, outside gear is named first, Listen chosen is Listen), names a stick takes and the progress words, in the crate's unit tests. Also in `tests/export.rs`: a silent return's stem is left out and named; a look-ahead put in during the pass fails it with no files left; `Pass::cancel` on the sound thread cancels.
- `mpfree-browser-ui`: the panel's range, bar steps and last choice (`src/export.rs`), and in `tests/browser.rs` the Export button asks the app, the panel takes the screen's input and answers taps; sample screens 18 to 20.
- `mpfree-app/src/export.rs`, with the time engine, the mixer and the player's steps as the sound loop runs them: a 1-bar export from bar 3 with a loop on plays the range, stops the song on the range's last frame, makes the mixdown and the synth's stem at the right length, puts the MIDI note on its exact tick, and puts the transport and loop back; STOP during the range cancels it; the setup, plan, mode line and folder names. `rt_safety` runs the player with the tap on and checks it never allocates. `app/saved.rs` checks the Prefs row saves and reads back.
