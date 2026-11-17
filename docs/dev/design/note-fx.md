# MIDI effects per track (`rust/mpfree-notefx`)

**Status:** the engine, each track's effects around its arp, saved with the project, and the Notes stage in the chain editor that sets them, built and tested on the workstation, 17 Nov 2026 (the MIDI powerhouse thread, plan item 3.1); Strum, Latch and Randomize the same evening. Nothing here has run on the MPC.

The owner picked MIDI effects per track as what follows the MIDI powerhouse plan's phase 2 (card, 17 Nov 2026 18:35Z). The plan puts them in a **Notes** stage at the head of each track's chain, before its sound: board 7 of the [MIDI mockups](https://claude.ai/artifact/9W8R9NDy4KJUcScN9DoTh6), approved for direction on 16 Nov. The research behind the list of effects (Bitwig's Note FX, Logic's MIDI plug-ins, the Hapax, Midihub, the OXI One) is in the [plan](https://claude.ai/artifact/L5XFouNLEEP66wJ2tPALFv).

## The engine

Notes in, sample-timed notes out, as [`mpfree-arp`](arpeggiator.md) does. A `Chain` holds up to eight effects (`MAX_FX`) in a track's order; each is a `Slot` (the effect's settings, `Fx`, and whether it is on). Once a period the host hands it the period (`Period`: where it starts on the frame count, its length, the rate and the tempo) and that period's notes in time order (`Ev`: a frame and a note on or off), and `Chain::process` puts what leaves in that period into a `Notes` list. Notes that leave later (a strum, an echo, a note made longer) wait inside the stage that made them. `Chain::process_range` runs only some of the places, so a track's arp can sit between them (the effects above it, the arp, the effects below it).

It never allocates, locks or does I/O once made (`tests/no_alloc.rs` runs 4,000 periods through a full chain of eight, swaps Strum, Latch and Randomize in for three of them, and changes settings, cuts for a mute and panics, and counts no allocation), so it can run in the sound loop. A chain is about 400 KB, made on the heap: the host makes one for a track only when the track has an effect.

A track's settings are a `TrackFx`: its effects in their places and where its arp sits among them (`arp_at`: the places above the arp; board 7's Scale, Chord, Arp, Humanize is 2), and each effect's id on the track (`ids`), which it keeps when it moves, so what names one of its settings follows the effect as an insert's slot id does on the mixer. `Chain::apply` puts them in, changing only the places that changed.

### The effects

| Effect | What it does | Settings |
|---|---|---|
| **Transpose** | Every note moves | -48 to 48 semitones |
| **Scale** | Notes outside the scale go into it, or aren't played | any of the 49 scales of [`mpfree-scale`](scales.md) on any root; Down, Up, Nearest or Skip |
| **Chord** | Each note becomes a chord | Octave, Fifth, Major, Minor, 7th, m7, maj7, sus2, sus4, add9, dim, aug, In key (2 to 5 stacked thirds in a scale) or Custom (any notes up to two octaves); Close, Spread or Drop 2; inversion 0 to 3; a highest note (Keep below); the added notes' velocity; strum in ms, lowest first |
| **Velocity** | A fixed velocity, or the one played on a curve | fixed; curve -100 to 100; lowest and highest |
| **Note length** | Every note lasts as long, however long it is held | ms or a note value (1 bar to 1/32T) |
| **Echo** | Each note again after it, softer each time | time (ms or note value); 1 to 16 repeats; each repeat's velocity in % of the one before; a pitch step per repeat |
| **Humanize** | Each note a little late and its velocity a little off | up to 100 ms late; up to 64 either way |
| **Chance** | Each note plays with a chance | 0 to 100 % |
| **Strum** | Notes that start together start one after another | 1 to 200 ms between notes; Up (lowest first), Down or As played |
| **Latch** | Notes stay on after their keys let go | Next chord (the next key down after all were up ends them) or Toggle (a second press ends a note) |
| **Randomize** | Some notes move in pitch, velocities spread, notes last longer, never the same twice | up to 24 semitones either way; how often a note moves, 0 to 100 %; velocity up to 64 either way; longer by up to 1,000 ms |

Times in note values follow the tempo the period gives (960 ticks a quarter, as the arp and the sequencer count). Humanize only ever moves a note later: notes played live can't be moved earlier, and a clip's notes should feel the same as the keys. Strum works the same way, so it can only hold notes back.

**Strum** takes notes that start within 40 ms of the first (`STRUM_TOGETHER_MS`) as one chord; the next note after that starts a new one. Each note waits its place in the chord times the time: the second starts one time after the first, the third two, up to 16 places (`STRUM_MOST`). Up and Down sort the notes that start on the same frame (a chord from a clip, the pads, or a Chord effect above it with no strum of its own) lowest or highest first; keys played by hand a few ms apart come in the order they were played, whatever the order. Each note's off waits as long as its on, so it keeps its length. Chord's own strum is still there for a chord that strums by itself.

**Latch** sits ahead of the rest of its stage: while it is on, a key down turns its note on and a key up ends nothing. With Next chord, the first key down after every key was let go ends the notes held and plays the new one, so a chord played while another key is still down adds to the one held. With Toggle, a key on a note held ends it. Switched off, changed to another effect or taken out, it ends what it holds at the start of the next period, and the keys still down from it end nothing more. A key that went down before Latch came ends as it would have.

**Randomize** moves a note it picks (How often, in %) by up to Pitch semitones either way, the same chance up or down (a move off the keyboard leaves the note put); spreads every note's velocity by up to Velocity either way, as Humanize does; and makes each note last up to Longer by more, its off coming later than its key's. A setting at 0 leaves that part alone.

### A key's notes end where they went

Each stage keeps, for each of the 128 notes coming in, what that note became when its key went down (the notes, each one's delay, which take the velocity played) until the key is let go. The key's off ends exactly those notes. So a setting changed, an effect switched off or taken out while a key is down never leaves a note on: the old chord ends as the old chord. A key played again while still held (two sources on one note) reuses what it became, and each of its offs ends one. A key that went down before its effect was there ends as it was played.

What each effect does with it:

- **Note length** puts each note's off in with its on; the key's off ends nothing.
- **Echo** puts the repeats' ons in with the key's first on, and their offs when the key's last off comes, each a repeat's time after it, so every repeat is as long as the note played.
- **Chance** and a **Scale** set to Skip make nothing of the note, and its key's off sends nothing.
- **Strum** and **Humanize** keep each note's delay with it, so its off comes as late as its on did.
- **Randomize** keeps the note it moved to and how much longer it lasts, and ends that note.
- **Latch** keeps which keys it took and which notes it holds (a cut ends those too).
- A panic (`Chain::reset`) forgets everything held and waiting; the host sends all notes off after it, as it does for the arps.
- A mute landing (`Chain::cut`, at a frame): nothing waiting plays from that frame (echoes, strummed and humanized notes not yet started), every note the chain has on ends there, and the keys down then end nothing. Notes played after it go through as usual. Each stage counts the notes it has sent on and not off, so it knows what to end.

Events at the same frame keep the order they were put in, so a note's off never comes before its own on, even for a note as short as one frame.

### When a list is full

Each list holds 2,048 events (`MAX_EVENTS`): a four-note chord echoed three times is 28, so a busy part's keys fit while their echoes wait. Ons stop being put in while half a stage's list waits, so the offs of notes already on always have room: a flood drops new notes, never an off, and a key whose notes were dropped sends no offs. `Chain::dropped` counts them; it should stay 0.

### Random, but the same each time

Humanize, Chance and Randomize draw from one small seeded generator per chain (`Chain::reseed`), in the order notes come in, so the same notes give the same result. The result doesn't depend on how the periods are cut either: a test plays the same part in periods of 64, 127, 1,000 and 4,096 frames and gets the same notes on the same frames.

### As a saved project keeps them

Each effect is one line of words: on or off, the effect, then its settings as `name=value` (`on chord shape=m7 voicing=drop2 inv=0 below=- vel=90 strum=8 scale=-`; a scale is `root:up:down`, the notes as bits; a time is `120ms` or `480t`, in ticks). The newer three are `strum ms=30 order=up` (up, down or played), `latch mode=chord` (chord or toggle) and `random pitch=5 often=30 vel=0 longer=0`. A setting a build doesn't know is skipped and one left out keeps the effect's first value, so a newer build's file still opens; a number out of its range is refused (`Slot::to_words`, `Slot::from_words`).

## On a track

Each track's effects are `Track::note_fx` (`mpfree-seq`), saved in the project's clips part after the track's `arp` and `rectouch` lines, one `notefx <place> <id> <words>` line an effect (its place, its id on the track, its words) and `notearp <n>`:

```text
track Keys
dest 2
mute 0
arp 1
rectouch 1
notefx 0 1 on scale scale=0:2741:2741 snap=down
notefx 1 2 on chord shape=m7 voicing=close inv=0 below=- vel=90 strum=0 scale=-
notefx 2 3 off humanize ms=6 vel=8
notearp 2
```

A place past the last, an effect twice in one place, an id of 0 or one another effect on the track has, or a number out of range refuses the part, as a bad line of a clip does; a setting the build doesn't know is skipped.

`TrackArps` (`mpfree-play`, [playback.md](playback.md#one-arp-per-track-trackarps)) runs a track's chain around its arp: `Chain::process_range` over the places above the arp on the notes going into it (the live pads, and the clips when "Clips through the arp" is on), the arp, then the places below it on what the arp played. So a part is never put through them twice: with the switch off a clip plays as recorded, and recording keeps what the effects and the arp played; with it on a clip goes through them again, and recording keeps the pads as pressed. Notes go out on MIDI ports (a track's Plays on, Routing) after them.

The chain is made off the sound loop: the time engine's control half sends one the first time a track has an effect (`ToAudio::Chain`), and its effects one place a message ([time-threads.md](time-threads.md)). A chained track runs every period, so its echoes leave on time while nothing is played. A panic, or the track's switch changing, cuts its chain where it lands; `TrackArps::cut` does the same for the sequencer's mute to call where it lands (sequencer S4).

## On screen: the Notes stage

Board 7, approved by the owner for the [layout spec](layout-spec.md#4b-the-fx-chain-editor-and-macros) (section 4b) on 17 Nov 2026, 19:19Z. A note track's chain in the chain editor (`mpfree-chain-ui`) has a NOTES group above its SOUND group, with a Plays on row between them; returns, submixes, the master and Loop tracks have none.

- **State:** `ChainState::notes` is a `NotesState` (`mpfree-chain-ui/src/notes.rs`): the track, its `TrackFx`, the arp's line and whether it plays, the row whose page shows, whether the MIDI effect picker is up, and the Plays on text. The app fills it in `ChainHost::dress_notes` (`mpfree-app/src/modpanels.rs`) from the project's track (`NotesSource::of`); the screen fills Plays on from the MIDI set-up's Tracks out (`Player::plays_on`). While a Notes row's page or the picker shows, no sound effect's page does (`ChainState::without_page`).
- **Touches** come back as `ChainAction::Notes(NoteAction)`: select a row, its on pill, move a row (the arp too), remove one (not the arp), the picker, add an effect (above the arp), set one setting, Plays on and Open the arp. `ChainHost::note_action` applies them to a copy of the track's `TrackFx`, and `TimeHost::set_note_fx` puts it in the project and sends what changed to the sound loop. Plays on opens the I/O view on its MIDI section; Open the arp arms the track (unless a take records on another) and opens the arp panel.
- **Q-Links:** a MIDI effect's page puts the four settings of its plan (`Fx::qlinks`) on the Q-Links, as `ParamView`s whose address is `Owner::NoteFx(track, id)` (`t3.nfx2.4` as text): the effect's id on the track, so the address follows it through moves. A Choice setting is a `Curve::Choice`, a number a `Curve::Stepped`. A Q-Link turn comes back as `ChainAction::Set` on that address, which the app turns into `NoteAction::Set`; focus and release are ignored. While the transport records, the turn records as a lock on that address too ([below](#moved-by-the-mod-engine)).
- **Pages** use the effect page frame: the header with the effect's name, its place and what it does; chips for a choice of many (Chord's shape, Scale's root) with ‹ › when they don't fit; a keyboard card with what goes in and what comes out (Transpose, Scale, Chord); then up to three section cards, a choice of four or fewer as segments and the rest as – value +. The Modulate row says which settings the Q-Links turn. A finger resting on a setting's row (the chips, or a card's row) puts it under the data wheel, a step a detent (`ChainView::held`, `notes::setting_at`, `notes::step_setting`), and the page shows each new value at once. The arp's page says what it plays and opens its panel. The picker shows the eleven effects as tiles, four a row, over the page and the Modulate row.

## Moved by the mod engine

A MIDI effect setting is a parameter like any other (`Owner::NoteFx(track, id)`, its param the setting's number), so the mixer's mod engine (`MixHost::modu`, [modulation.md](modulation.md)) can lock it on a step, map it to a macro, aim a matrix row (an LFO) at it, and play back a Q-Link turn recorded as a lock. What it works out goes to the track's chain, not the mixer:

- **Knowing the setting.** The engine describes a target from the tracks' effects as the sound loop has them (`ModMix::set_note_fx`, a copy of the time engine's audio half's table, taken in the audio loop whenever it changes, and from the project as it opens, `MixLoad::note_fx`, so the saved set-up's lines on MIDI effects load). Its range and steps are the setting's (`mpfree_chain_ui::notes::info`): a choice moves through its list, a number in whole steps.
- **The value's way.** `ModMix::process` keeps each value it sends on a MIDI effect setting (`ModMix::take_note_values`); the audio loop hands them to `TimeHost::modulate_note_fx` before the next period. The time engine's audio half keeps up to 16 a track (`NOTE_MODS`) on top of the track's own effects, and `dress_arps` puts the two together into the arps' chain each period (`TrackFx::set_value`, kept in range, never allocating). The project keeps the track's own value, which is what the page shows and what is saved.
- **The engine's base follows the screen.** A setting the engine holds takes a change made on the page as its own value (`ModMix::set`), so a macro or an LFO moves it from there; an effect taken out of the track is forgotten (`ModMix::forget`), and its values in the sound loop go when another effect (or another id) takes its place. The sound loop now gets each effect's id with it (`ToAudio::NoteFx`), to name its settings.
- **When.** One period after the engine works it out (about 3 ms), and a held note keeps what its key started as, as with any change to an effect.
- **On screen.** A setting touched on a MIDI effect's page (a tap on its chips, segments, – + or value) becomes the editor's focus (`ChainAction::Focus` with its `NoteFx` address), and the Modulate row under the page is then the usual one: macros 1 to 8, Learn, + LFO and + Follower (not for a choice, as for a sound's switch), and the link to its rows. `ChainHost::dress_notes` takes each setting's macro, ring and lock from the engine, and the focus's rows; `ModMix::note_label` names it on the macro cards ("Chord inversion (Track 3)"). Until a setting is touched the row says which settings the Q-Links turn.

## Tests

`cargo test -p mpfree-notefx`: each effect (board 7's C3 m7 becomes C3 Eb3 G3 Bb3 with the added notes at 90 %; voicings, inversions, Keep below and In key; strum; velocity curves; note length; echo with climbing repeats cut at the top of the keyboard; humanize's range and its notes' lengths; chance's share; Strum's three orders keeping each note's length, keys close together as one chord, and a Chord above it strummed; Latch holding to the next chord, Toggle, taken out while a key is down, a key from before it, and a cut; Randomize's range, share, velocities and lengths, and at 0 leaving notes as they were), the order of effects, an effect changed or taken out while a key is down, a key played twice while held, the period size, a flood, a mute's cut (what waits is dropped, what sounds ends at the cut, a note after it plays), every effect's words read back as written, and a run of random parts with effects changed and cuts between periods in which every note that starts ends. `tests/no_alloc.rs` checks the sound loop never allocates. On a track: `mpfree-play`'s `tests/arps.rs` (effects above the arp change what it plays and those below what it played; a drum track's clip skips them while its pads go through; a track without a chain plays as it is; a mute cuts what waits and a panic ends what the effects added; the same notes for any period size, without allocating), `mpfree-seq`'s save tests (board 7's list reads back with its ids; a bad place, a doubled one, a missing or doubled id, a number out of range or an effect this build lacks refuse the part) and the app's `a_tracks_midi_effects_reach_the_pads_from_the_project` (the project's effects and chain cross to the sound loop, an edit from the screen follows, and a Loop track takes none). On screen: `mpfree-chain-ui`'s notes tests (the list's rows, scrolling and Plays on; each page's chips, segments, steppers and keyboard; the picker; board 7's Chord as drawn) and sample screens (`chain-notes-chord`, `chain-notes-arp`, `chain-notes-add`; touches become `NoteAction`s, a scrolled SOUND group keeps its effect in view, a held setting turns with the wheel), the app's `the_notes_stage_edits_the_tracks_midi_effects` and `plays_on_opens_the_io_view`, the smoke walks (the Notes stage the screen shows matches the project's), and the `chain-notes` golden screen.

## Next

- **More effects** from the plan: Harmonize to a track, Split, and the controller effects (LFO to CC, CC smooth and scale, Channel map) for MIDI out. (Strum, Latch and Randomize are in.)
- **Live transpose** from the pads is the sequencer thread's: it shifts a track's notes where pads become notes (after the pad layout), not in this chain, since a clip's events there are pad numbers.
