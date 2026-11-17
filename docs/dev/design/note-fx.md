# MIDI effects per track (`rust/mpfree-notefx`)

**Status:** the engine, built and tested on the workstation, 17 Nov 2026 (the MIDI powerhouse thread, plan item 3.1). Nothing plays through it yet: putting it on a track and its screen come next (below). Nothing here has run on the MPC.

The owner picked MIDI effects per track as what follows the MIDI powerhouse plan's phase 2 (card, 17 Nov 2026 18:35Z). The plan puts them in a **Notes** stage at the head of each track's chain, before its sound: board 7 of the [MIDI mockups](https://claude.ai/artifact/9W8R9NDy4KJUcScN9DoTh6), approved for direction on 16 Nov. The research behind the list of effects (Bitwig's Note FX, Logic's MIDI plug-ins, the Hapax, Midihub, the OXI One) is in the [plan](https://claude.ai/artifact/L5XFouNLEEP66wJ2tPALFv).

## The engine

Notes in, sample-timed notes out, as [`mpfree-arp`](arpeggiator.md) does. A `Chain` holds up to eight effects (`MAX_FX`) in a track's order; each is a `Slot` (the effect's settings, `Fx`, and whether it is on). Once a period the host hands it the period (`Period`: where it starts on the frame count, its length, the rate and the tempo) and that period's notes in time order (`Ev`: a frame and a note on or off), and `Chain::process` puts what leaves in that period into a `Notes` list. Notes that leave later (a strum, an echo, a note made longer) wait inside the stage that made them. `Chain::process_range` runs only some of the places, so a track's arp can sit between them (the effects above it, the arp, the effects below it).

It never allocates, locks or does I/O once made (`tests/no_alloc.rs` runs 4,000 periods through all eight effects, with settings changed, a mute's cut and a panic, and counts no allocation), so it can run in the sound loop. A chain is about 400 KB, made on the heap: the host makes one for a track only when the track has an effect.

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

Times in note values follow the tempo the period gives (960 ticks a quarter, as the arp and the sequencer count). Humanize only ever moves a note later: notes played live can't be moved earlier, and a clip's notes should feel the same as the keys.

### A key's notes end where they went

Each stage keeps, for each of the 128 notes coming in, what that note became when its key went down (the notes, each one's delay, which take the velocity played) until the key is let go. The key's off ends exactly those notes. So a setting changed, an effect switched off or taken out while a key is down never leaves a note on: the old chord ends as the old chord. A key played again while still held (two sources on one note) reuses what it became, and each of its offs ends one. A key that went down before its effect was there ends as it was played.

What each effect does with it:

- **Note length** puts each note's off in with its on; the key's off ends nothing.
- **Echo** puts the repeats' ons in with the key's first on, and their offs when the key's last off comes, each a repeat's time after it, so every repeat is as long as the note played.
- **Chance** and a **Scale** set to Skip make nothing of the note, and its key's off sends nothing.
- A panic (`Chain::reset`) forgets everything held and waiting; the host sends all notes off after it, as it does for the arps.
- A mute landing (`Chain::cut`, at a frame): nothing waiting plays from that frame (echoes, strummed and humanized notes not yet started), every note the chain has on ends there, and the keys down then end nothing. Notes played after it go through as usual. Each stage counts the notes it has sent on and not off, so it knows what to end.

Events at the same frame keep the order they were put in, so a note's off never comes before its own on, even for a note as short as one frame.

### When a list is full

Each list holds 2,048 events (`MAX_EVENTS`): a four-note chord echoed three times is 28, so a busy part's keys fit while their echoes wait. Ons stop being put in while half a stage's list waits, so the offs of notes already on always have room: a flood drops new notes, never an off, and a key whose notes were dropped sends no offs. `Chain::dropped` counts them; it should stay 0.

### Random, but the same each time

Humanize and Chance draw from one small seeded generator per chain (`Chain::reseed`), in the order notes come in, so the same notes give the same result. The result doesn't depend on how the periods are cut either: a test plays the same part in periods of 64, 127, 1,000 and 4,096 frames and gets the same notes on the same frames.

### As a saved project keeps them

Each effect is one line of words: on or off, the effect, then its settings as `name=value` (`on chord shape=m7 voicing=drop2 inv=0 below=- vel=90 strum=8 scale=-`; a scale is `root:up:down`, the notes as bits; a time is `120ms` or `480t`, in ticks). A setting a build doesn't know is skipped and one left out keeps the effect's first value, so a newer build's file still opens; a number out of its range is refused (`Slot::to_words`, `Slot::from_words`).

## Tests

`cargo test -p mpfree-notefx`: each effect (board 7's C3 m7 becomes C3 Eb3 G3 Bb3 with the added notes at 90 %; voicings, inversions, Keep below and In key; strum; velocity curves; note length; echo with climbing repeats cut at the top of the keyboard; humanize's range and its notes' lengths; chance's share), the order of effects, an effect changed or taken out while a key is down, a key played twice while held, the period size, a flood, a mute's cut (what waits is dropped, what sounds ends at the cut, a note after it plays), every effect's words read back as written, and a run of random parts with effects changed and cuts between periods in which every note that starts ends. `tests/no_alloc.rs` checks the sound loop never allocates.

## Next

In this order, each its own pull request:

1. **On a track.** Each track's chain beside its arp in `TrackArps` (`mpfree-play`), so live keys and, with the track's "Clips through the arp" switch, its clips go through it; the arp becomes a place in the chain. The chains are saved with the project. Agreed with the sequencer thread, which owns the time engine.
2. **The Notes stage on screen.** Board 7: the chain editor's Notes list above the sound's effects, + MIDI effect, each effect's page with what goes in and what comes out, and the Q-Links. Agreed with the FX thread, which owns the chain editor, and proposed for the [layout spec](layout-spec.md).
3. **More effects** from the plan: Strum and Latch on their own, Randomize (pitch and length), Harmonize to a track, Split, and the controller effects (LFO to CC, CC smooth and scale, Channel map) for MIDI out.
