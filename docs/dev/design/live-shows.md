# Live shows: patches, keygroups and set lists

The plan for playing whole shows from one MPC: songs in a set, the next song ready before you need it, keyboard patches that scenes recall, keys that play keygroups, and a Show view in Perform. The full plan, its research, two reviews and the twelve decisions are in the Claude Doc [Live shows: patches, keygroups and set lists](https://claude.ai/artifact/PBWnfRy2bfrJwhgS1eVjf6), approved as written by the owner on 22 Nov 2026. This page says what is built, as it is built.

The rules it keeps: one track list per project; instruments make the sound, tracks record, pads and keys play instruments; no instrument swaps on a track per scene. It uses what the other redesigns make and never its own copy: the arranger's one scheduler, the stream layer ([Disk streaming](disk-streaming.md)), the instrument list ([Instruments](instruments.md)) and the audio tracks' recordings ([Audio tracks](audio-tracks.md)).

## Roadmap

| Phase | What lands | State |
|---|---|---|
| A1 | All-or-nothing opens: the parts gate is the audio tracks rethink's phase 0.1 ([Project safety](project-safety.md)); A1 adds a refusal when sounds, takes or loops would be left out, and Open what reads | built ([Project safety](project-safety.md#a-project-that-cant-open-whole-asks-first-live-shows-a1-as-built)) |
| A2 | The engines an open replaced always go back to the screen thread, never freed on the audio thread | built ([Project safety](project-safety.md#the-engines-an-open-replaced-go-back-live-shows-a2-as-built)) |
| A3 | Lasting project ids (Save As gives a new one, a rename keeps it, twins are noticed) and a manifest per project | built ([The browser and projects](browser.md#projects)) |
| A4 | Real songs measured on the MPC (read-only, on the owner's go) | planned |
| K1 | Keys play keygroups by note on the sampler's single note path, with each kit's Keys play: Pads, Keygroups or Both | planned |
| B1, B2 | Sets, the Set editor and the Show view with cold changes; the set check as a preflight, Prepare show media and Soundcheck | B1 building: sets in the library, the Sets place, the Set editor, the Show view's drawing, Learn's Show card and Quick advance built ([below](#sets-b1-as-built)); the Show view on Arrange, Play set and cold changes next. Screens: [layout spec 4n](layout-spec.md#4n-sets-the-show-view-and-the-set-editor) |
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

**The browser** (`mpfree-browser-ui`: `sets.rs` draws, `set_place.rs` is the logic, a child of `state` so it shares the browser's worker, keyboard and questions): Sets is a place after Projects. The list of sets, a set's card, the Set editor (a song's Starts, Ends, Fade, Needs, Notes; Move up and down; Remove from set; Open in Make) and Add songs are drawn as layout spec 4n says. The set is saved after every change (`Job::SaveSet`); the place's list is updated at once from the editor's copy. While a check runs, the card's buttons are faint and the editor takes no changes, so the results land on the same songs they checked. The app gives the browser the rig (`Browser::set_rig`: the MIDI ports the setup knows and the ones plugged in, by the names the musician gave them; learned controls join with the Show card in Learn) and gets `BrowseAction::PlaySet` from Play set; the Show view takes it next.

**The set player's rules** (`mpfree-app/src/show.rs`, tested, wired with the Show view): Next song arms the song after NOW and a second press goes (with Quick advance on and the transport stopped, one press goes); Previous song arms the one before; a tap on NOW or the armed song does nothing; a second press of the same control within 50 ms is the same press; while a change is under way every press is ignored except Go during FINISHING (Go now); after a failed open only arming works (Try again, Reopen); a song that isn't ready never goes.

**The Show view** (`mpfree-time-ui/src/show.rs`, `ShowView`): drawn to layout spec 4n at 1116 × 610, the place of the grid and the Record panel. It draws only what it is given (the set column's rows with their marks, the song band and its `Pill`, the parts as tiles, the `NextCard`, the notes, the keyboards and the click) and answers taps: `Select(k)` arms song `k`, on the lift so a drag scrolls a long set, and never for NOW or the armed song; `Toggle(SHOW_*)` is Clips, Song, List, Back to the set order, Go, Cancel next, Click, and Try again and Reopen after a failed open. Faint buttons answer nothing. The column follows NOW and the armed song when either changes. The card's and the pill's words come from their constructors (`NextCard::cold`, `finishing`, `saving_takes`, `opening`, `blocked`, `last_song`, `failed`, `not_armed`; `Pill::playing`, `loaded`, ...), so a state reads the same wherever it is made. `show::sample()` is the approved drawing's state. It goes on Arrange as the fourth view (MAIN, the title lines' Show) once the arranger's phase 3 is in.

**Footswitches** (the Learn section's Show card, layout spec 4d): `Owner::Show` (`show.N`, ids in `mpfree_param::show`, a saved format) for Next song, Previous song, Go, Cancel next, Back to the set order, Panic and Click; learned in the rig ([MIDI I/O](midi-io.md)). `remote::Remote::show_write` gives each press once as it goes down. Until the Show view is on Arrange, a press says on the hint line that no set is open (`remote::NO_SET`). **Quick advance** (Prefs, `Settings::quick_advance`, kept as `quick-advance`) is what `show::Show::press` takes as `quick`.
