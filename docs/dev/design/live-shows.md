# Live shows: patches, keygroups and set lists

The plan for playing whole shows from one MPC: songs in a set, the next song ready before you need it, keyboard patches that scenes recall, keys that play keygroups, and a Show view in Perform. The full plan, its research, two reviews and the twelve decisions are in the Claude Doc [Live shows: patches, keygroups and set lists](https://claude.ai/artifact/PBWnfRy2bfrJwhgS1eVjf6), approved as written by the owner on 22 Nov 2026; a copy is in jferments/mpfree-claude `live-shows/plan.md`, with the approved Show view and Set editor screens in `live-shows/mockups/`. This page says what is built, as it is built.

The rules it keeps: one track list per project; instruments make the sound, tracks record, pads and keys play instruments; no instrument swaps on a track per scene. It uses what the other redesigns make and never its own copy: the arranger's one scheduler, the stream layer ([Disk streaming](disk-streaming.md)), the instrument list ([Instruments](instruments.md)) and the audio tracks' recordings ([Audio tracks](audio-tracks.md)).

## Roadmap

| Phase | What lands | State |
|---|---|---|
| A1 | All-or-nothing opens: the parts gate is the audio tracks rethink's phase 0.1 ([Project safety](project-safety.md)); A1 adds a refusal when sounds, takes or loops would be left out, and Open what reads | built ([Project safety](project-safety.md#a-project-that-cant-open-whole-asks-first-live-shows-a1-as-built)) |
| A2 | The engines an open replaced always go back to the screen thread, never freed on the audio thread | built ([Project safety](project-safety.md#the-engines-an-open-replaced-go-back-live-shows-a2-as-built)) |
| A3 | Lasting project ids (Save As gives a new one, a rename keeps it, twins are noticed) and a manifest per project | built ([The browser and projects](browser.md#projects)) |
| A4 | Real songs measured on the MPC (read-only, on the owner's go) | planned |
| K1 | Keys play keygroups by note on the sampler's single note path, with each kit's Keys play: Pads, Keygroups or Both | built by the sampler refactor as its step 15a2 (agreed 23 Nov): Keys play is the kit's setting, used by the armed track's keys and by routes alike |
| B1, B2 | Sets, the Set editor and the Show view with cold changes; the set check as a preflight, Prepare show media and Soundcheck | B1 built: sets in the library, the Sets place, the Set editor, the Show view on Arrange, Play set, cold changes with each song's Ends, recovery copies, Learn's Show card and Quick advance ([below](#sets-b1-as-built), [playing a set](#playing-a-set-b1-as-built)). Screens: [layout spec 4o](layout-spec.md#4o-sets-the-show-view-and-the-set-editor). B2 planned |
| C1 to C4 | Patches: zones, the note owner table, scenes and parts picking patches, REC with zones, knobs and pads per patch | planned |
| D1, D2 | The next song prepared in the background; the switch at once or on its frame; Let it finish, Fade and Cut | planned |
| E1, E2, V1, F1 | Click, cues and the Split output; Next, Repeat and Hold part; the track view; Overlap | planned |

## Sets (B1, as built)

A set is a list of songs in order, each a project in Projects. It points at its songs and never copies them, so one song can be in many sets, and a song can be in one set more than once.

**The file** (`mpfree-library/src/sets.rs`): `Sets/<name>.set`, plain text.

```text
mpfree-set 1
song = <the project's lasting id, or -> <its folder in Projects at the last save>
starts = stopped | playing | count-in
ends = finish | fade <1 to 30 seconds> | cut
needs = <a keyboard or control's name in the rig>
nice = <one it can do without>
notes = <240 characters at most, with \n for a new line and \\ for a backslash>
checked = <seconds since 1970> <the tempo then>
not-ready = <why, from that check>
limit = <what it plays without, from that check>
unchecked = yes
```

- Each song's lines follow its `song` line; values left out are the defaults (Stopped, Let it finish). Lines a build doesn't know are kept with the song they follow, so a newer build's lines survive an older build's save. A song line with no name is left out with its lines. At most 200 songs and 256 KiB.
- **Saved whole:** `Library::save_set` writes `<name>.set.part`, syncs it and renames it over the set, then syncs the folder, so a power cut leaves the set as it was or as it is now. Only `.set` files in Sets can be written this way.
- **Finding a song:** `Library::find_song` looks in the folder the set names; if the project there doesn't have the song's id, it looks for the id among the other projects (`Library::find_project`, A3), so a renamed or moved song is still found, and the set takes its new name at its next check. A song saved before ids is found by its folder alone. A song whose id is nowhere (deleted, in the Trash) is gone.
- **The check** (`Library::check_song`, the open's quick check from A1): the project reads; every file its manifest lists is there with the same size and time (Check every file reads each one in full, `check_song_with(.., deep)`); and the keyboards it needs are connected. A missing or changed file, a project that doesn't read or isn't there, or a needed keyboard not connected makes it NOT READY; a nice-to-have keyboard not connected, or a part changed since its last save, gives it LIMITS; a project with no manifest yet is NOT CHECKED (saving it makes one). Each song keeps its last check in the set (`checked` and the lines after it), so the Sets place shows it after a restart.

**The worker's jobs** (`mpfree-library/src/worker.rs`): `Sets` (every set, read, for the place's list), `Songs` (the projects a set can hold, not autosave's), `NewSet` (a set under a name not taken), `LoadSet`, `SaveSet`, `CheckSet` (with the names of the rig's things connected now, and `deep` for Check every file). The browser never reads or writes a set on the screen thread.

**The browser** (`mpfree-browser-ui`: `sets.rs` draws, `set_place.rs` is the logic, a child of `state` so it shares the browser's worker, keyboard and questions): Sets is a place after Projects. The list of sets, a set's card, the Set editor (a song's Starts, Ends, Fade, Needs, Notes; Move up and down; Remove from set; Open in Make) and Add songs are drawn as layout spec 4o says. The set is saved after every change (`Job::SaveSet`); the place's list is updated at once from the editor's copy. While a check runs, the card's buttons are faint and the editor takes no changes, so the results land on the same songs they checked. The app gives the browser the rig (`Browser::set_rig`: the MIDI ports the setup knows and the ones plugged in, by the names the musician gave them; learned controls join with the Show card in Learn) and gets `BrowseAction::PlaySet` from Play set; the Show view takes it next.

**The set player's rules** (`mpfree-app/src/show.rs`, tested, wired with the Show view): Next song arms the song after NOW and a second press goes (with Quick advance on and the transport stopped, one press goes); Previous song arms the one before; a tap on NOW or the armed song does nothing; a second press of the same control within 50 ms is the same press; while a change is under way every press is ignored except Go during FINISHING (Go now); after a failed open only arming works (Try again, Reopen); a song that isn't ready never goes.

**The Show view** (`mpfree-time-ui/src/show.rs`, `ShowView`): drawn to layout spec 4o at 1116 × 610, the place of the grid and the Record panel. It draws only what it is given (the set column's rows with their marks, the song band and its `Pill`, the parts as tiles, the `NextCard`, the notes, the keyboards and the click) and answers taps: `Select(k)` arms song `k`, on the lift so a drag scrolls a long set, and never for NOW or the armed song; `Toggle(SHOW_*)` is Clips, Song, List, Back to the set order, Go, Cancel next, Click, and Try again and Reopen after a failed open. Faint buttons answer nothing. The column follows NOW and the armed song when either changes. The card's and the pill's words come from their constructors (`NextCard::cold`, `finishing`, `saving_takes`, `opening`, `blocked`, `last_song`, `failed`, `not_armed`; `Pill::playing`, `loaded`, ...), so a state reads the same wherever it is made. `show::sample()` is the approved drawing's state. It goes on Arrange as the fourth view (MAIN, the title lines' Show) once the arranger's phase 3 is in.

**Footswitches** (the Learn section's Show card, layout spec 4d): `Owner::Show` (`show.N`, ids in `mpfree_param::show`, a saved format) for Next song, Previous song, Go, Cancel next, Back to the set order, Panic and Click; learned in the rig ([MIDI I/O](midi-io.md)). `remote::Remote::show_write` gives each press once as it goes down. With no set open, a press says on the hint line that no set is open (`remote::NO_SET`); with one open, Panic and Click are the app's own (`App::show_press`: Panic is SHIFT + STOP's stop with StopAndRewind, Click turns the metronome) and the other five go to the screen's set player (`ToUi::ShowPress`). **Quick advance** (Prefs, `Settings::quick_advance`, kept as `quick-advance`) is what `show::Show::press` takes as `quick`.

## Playing a set (B1, as built)

B1 changes song cold: the next song opens when Go is pressed, so there is a pause while it reads. Preparing it in the background is D1 and D2.

**Who does what.** The screen thread owns the set being played: `mpfree-app/src/set_play.rs` (`SetPlay`, pure and tested) holds the set, the set player's rules (`show::Show`), where a change stands (`Step`: Starting, Idle, Ending, Saving, Opening) and draws the `ShowView` from them; gui.rs feeds it presses, the browser's answers and the audio loop's news, and does what it answers (`set_play::Do`). The audio loop runs the endings (`song_end.rs`). The app thread (app.rs) knows only that a set is open and the Show view shows (`FromUi::ShowState`), for MENU's rows and the wheel.

**Play set** (`BrowseAction::PlaySet`, after Confirm::PlaySet over unsaved work): `Browser::set_playing` tells the Sets place which set plays, so its changes come back as `BrowseAction::SetChanged`; `check_set_now` checks it; the time page's `set_open` and `show` put the Show view up (`SHOW_LAYOUT`); Perform turns on (layout spec 4o) and Arrange shows; the first song opens. A song that doesn't open there (`Do::NotStarted`) ends the set before it starts.

**A song opens without asking.** `Browser::open_set_song` (a tracked job, `Job::OpenSetSong`) finds the song by its id or folder (`Library::find_song`) and runs the open's quick check: a missing or damaged file fails it with why (`SetSongFailed`), never the Open what reads question. Otherwise `SetSongOpened` comes first and the next `ProjectOpened` is the set's: `Player::open_project(.., stage: true)` refuses, rather than asks, a song that would leave something out. The song is in once its engines are back (`ProjectBack`), its loops and takes are read and its kits are in (`set_play_tick`); it then starts as its Starts says: nothing (LOADED), `Command::Play`, or `Command::CountInPlay` (the transport's count-in that ends in Playing, not Recording). Perform turns on again if the song was saved in Make.

**A cold change**, from Go:

1. **Ending** (`FromUi::SongEnd(End)`): the audio loop presses the transport's STOP (no new notes; held notes go when let go; a take recording ends and is kept) unless it was stopped, and `SongEnd` gains the output after the mixer, before Sample Recall and the click, so the project's master fader is never touched. Let it finish waits until the output's peak stays under -60 dBFS for 300 ms (a rise over -54 dB starts the 300 ms again), at most 10 s from Go, then fades over 50 ms; Fade falls over its seconds; Cut over 10 ms; Go again during Let it finish goes now (`End::Now`). The output stays silent until the next open resets it (`FromUi::OpenProject`) or Reopen does (`SongEndReset`). News: `ToUi::SongEnd(News::Finishing(secs))` each second, then `Ended`. The quiet is measured on the mixer's output, so line monitoring counts; the 10 s cap bounds it.
2. **Saving takes**, if the song changed since it opened (the browser's unsaved mark): its parts are asked for as a save (`FromUi::SaveProject`), and `Browser::on_stage_copy_with` (`Job::OnStageCopy`, `Library::on_stage_copy`) writes them, takes and all, into a new project beside the song, "<song> on stage" (a number added if taken), marked as autosave's so it has no lasting id and isn't listed as a song. `Browser::mark_on_stage` adds it to the song's `changed` lines in the set (at most 16; `SetSong::on_stage`). The song itself stays as saved. Each pass that changed the song gets its own copy, so every pass's takes are kept.
3. **Opening**: the next song opens as above.

Nothing asks on stage and nothing saves during a change: `autosave_tick` waits while a change is under way, a save asked for then says so on the hint line, and the Show view says "autosave waiting" if autosave is due meanwhile. A copy or an open that fails turns the card red (`NextCard::failed`): **Try again** runs that step again; **Reopen** gives back the song that was playing. An open is all or nothing and the song before is never closed first, so Reopen only gives the sound back (`SongEndReset`).

**After the show**, the Set editor marks a song that changed on stage and lists its copies under **Changed on stage**: **Keep the changes** saves the newest copy into the song (`Job::KeepOnStage`, `Library::keep_on_stage`: the song keeps a version of what it was, the copy's sound files that the song lacks are copied in, the manifest is made again, and the copy goes) and moves older copies to the Trash; **Throw them away** moves every copy to the Trash.

**Controls on the Show view**: a tap on a song arms it (`Press::Arm`); Go, Cancel next, Back to the set order, Try again and Reopen on the view; the wheel and - / + arm the song after or before (`app::ShowKey::Step`, SHIFT + wheel still scrolls) and the wheel's push is Go (`ShowKey::Push`); the Click switch is the learned Click's (`FromUi::ShowControl`). MENU gives Open the Set editor (`Browser::edit_set`, then BROWSE), Check the set and Close the set (`app::SetMenu`); Close, another project opened from BROWSE, or New project ends the set and leaves the song open. Each control has its own id for the 50 ms bounce (`set_play::TAP`, `WHEEL`, `PUSH`, `FOOT` + a learned control's id). MAIN with a set open goes to the Show view from another page in Perform, and on Arrange goes Show, Clips, Song, List; Show on the Clips grid's, the Song view's and the song list's title lines comes back.

**Built with these details for the owner to confirm:** a song counts as changed by the browser's unsaved mark, which clips launched in Perform set too, so a song only played can get a recovery copy; one copy per pass rather than one per song per show, and Keep keeps the newest; the keyboards card says "connected" or "not connected" rather than what each plays; the Show view's hint line words; Let it finish's quiet is measured on the whole output before the click.
