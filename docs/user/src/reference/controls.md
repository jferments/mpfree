<!--
status: changing
covers: rust/mpfree-app/src/app.rs rust/mpfree-controller docs/dev/design/layout-spec.md
checked: 2026-11-19 79fc6f63
-->

# Buttons and controls

This is the one place the manual lists exactly what every button and combination does. Other chapters link here instead of repeating it, so if a button changes, only this page changes.

"SHIFT + MENU" means hold SHIFT, then press MENU.

Every button works as soon as MPFree starts, with nothing to learn: the developers measured the whole panel. The **When** column says whether a control works in the build this manual describes (see [Which version this describes](../index.md#which-version-this-describes)) or is planned.

{{shot: guide | The button guide shows what the buttons do on the page you are on.}}

## Everywhere

| Control | What it does | When |
|---|---|---|
| **Power button**, a short press | Opens the power card: **Shut down**, **Restart** or **Cancel**, and over unsaved changes, whether to save first. Another short press closes it. See [Switching off and restarting](../getting-started/before-you-start.md#switching-off-and-restarting). | Works |
| **Power button**, held | Switches the MPC off at once, without asking or saving. | Works |
| **MENU** | Opens the options for the page you are on. In Prefs' System info, Backup and Akai kits or Touch screen rows, goes back to the main Prefs rows. | Works |
| **SHIFT + MENU** | Opens Prefs. | Works |
| **STOP** | Stops playback; sounds ring out and latched pads keep going. Press again to silence everything and go back to bar 1. | Works |
| **SHIFT + STOP** | Stops, silences everything and goes back to the start of bar 1. | Works |
| **PLAY** | Starts playback from where it stopped. | Works |
| **PLAY** while playing | Hold for a fill; press twice to keep the fill on, and again (or STOP) to end it. See [Playing live](../making-music/playing-live.md#fills). | Works |
| **PLAY START** | Starts playback from the beginning. | Works |
| **REC** | Records what is armed, on every page: the notes of the track the pads play (into its clip) or a loop, as the REC chip on the transport bar names. Stopped, the notes get ready and **PLAY** (or **PLAY START**, from bar 1) starts the take; REC again or **STOP** lets go. Playing, REC punches in; recording, it punches out. The Sequencer tab and picking a track arm the notes; the Looper tab and the **Arm** pill on Play's loop row arm the loop, where REC is the loop's own Record. With both armed (SHIFT + a tap on an Arm pill), REC records the notes and the loop in one pass. SHIFT + a tap on a track's row in the REC chip's list records that track too, from its keyboard's route, in the same take (see [Record several tracks at once](../making-music/sequencer.md#record-several-tracks-at-once)). Hold a pad with no sound and press REC to record one take onto that pad with the Sample tab's recorder (REC → Pad A5); REC again or STOP ends it, UNDO throws it away, and REC then records what it did before (see [Sampling](../making-music/sampling.md#hold-an-empty-pad-and-press-rec-one-take-from-any-page)). | Works |
| **OVERDUB** | As REC, but the take adds to what is already there: stopped, ready to add; playing, punches in adding; recording, switches between replacing and adding. With a loop armed, the loop's Overdub. | Works |
| **UNDO** | While a recording is going, throws it away: the take so far goes and the music plays on. Otherwise undoes the last recording or edit on the page you are on, and the line at the bottom of the screen says what it took back. The Looper tab: the last loop layer. The Sequencer tab and the Arrange tab: the last take (a recording, a step edit or a deleted clip). The Play page: whichever of the two changed last. The Sample and Pads tabs: the last edit. The Synth tab: the last knob you turned. The mixer: the last fader, knob or button you changed on a strip. A run of turns of the same knob or fader is one UNDO. Other pages say they have nothing to undo (the FX chain editor and the macros panel get theirs with the effects overhaul). **SHIFT + UNDO** redoes it. **Hold UNDO** for a moment to see the page's history as a list: tap a step (or turn the wheel to it and push) to go straight back, or forward, to just after it. | Works |
| **SHIFT + REC** | Capture: keeps what you just played as a clip, even if you weren't recording. On the Looper tab, the last bars you played become a loop. | Works |
| **Hold a pad + SHIFT + REC** | [Sample Recall](../making-music/sampling.md#sample-recall-keep-what-just-played-from-any-page): the last bars of everything you heard onto that pad (or the next empty one), from any page. While it is open, **PLAY** hears the bracket, **REC** keeps it and **STOP** closes it; the music plays on. | Works |
| **SHIFT + PLAY START** | Metronome on or off (it starts off; MPFree remembers your choice). The click plays while counting in and recording. Its level is in the Play page's options (**MENU**). | Works |
| **TAP TEMPO** | Tap four times in time to set the tempo. Tapping the tempo on the Play page does the same. | Works |
| **TRACK MIX** | Opens the mixer (the **Mix** tab) on the tracks; pressed again, back to the page you were on. **SHIFT + TRACK MIX** opens it on the pads. | Works (the Mix tab: needs a device check) |
| **PAD BANK A–D** | Switches the pads to bank A, B, C or D. **SHIFT** + a bank button, or a second press, gives banks E–H. With a kit on the pads, each bank is 16 of its 128 pads: A is pads A1–A16, B is B1–B16, up to H; the lit button is the bank on the pads, and it stays when you select another track. To change the selected track's kit, use the list of kits (**PADS PLAY** on Play, or **Kit** on the Pads tab) or **SHIFT** + the data wheel on the Play page. With a synth on the pads, they pick its pad layout. While **STEP SEQ** has the pads, they turn its pages of steps; while **TRACK MUTE** has them, they pick which 16 tracks; while the pads are the [Perform set](../making-music/looping.md#more-pages-the-perform-set) (SHIFT + SAMPLER on the Looper tab), or the Pads tab [shows it](../making-music/pads.md#the-perform-set-on-the-pads-tab), they pick its page: A the loop keys, B Play's throws. | Works |
| **FULL LEVEL** | Every pad plays at full velocity, however hard you hit it. Press again to turn it off. **SHIFT + FULL LEVEL** is Half Level (every hit at half velocity). | Works |
| **16 LEVEL** | 16 Levels: the pad you hit last plays on all 16 pads, each a step of velocity, tune, cutoff, decay or start, and the [16 Levels panel](../making-music/playing-live.md#16-levels) opens on Play. Its last kind, **Transpose**, makes the pads shift the armed track's notes live, pad 9 as written; with the synth on the pads, 16 LEVEL opens it at once (see [Transpose](../making-music/playing-live.md#transpose)). Press again to turn it off. **SHIFT + 16 LEVEL** switches the synth pads between Chromatic and In key (with no scale picked yet, In key picks C Major; the line at the bottom of the screen says which). | Works |
| **Q-Link knobs 1–4** | Turn the four parameters shown in the Q-Link column at the right of the screen. What they control depends on the page. | Works |
| **Data wheel** | On a page with settings (the menus, Prefs, the Sample tab, the Pads tab, the Synth tab, Arrange): picks a row. On the Play page, the Looper tab, the mixer and the FX chain editor: moves the outline from control to control (see [The wheel's outline](../getting-started/the-screen.md#the-wheels-outline)). It no longer changes the kit. With a finger resting on a value, turns that value instead (see [Hold and turn](../getting-started/the-screen.md#hold-and-turn)). | Works (the outline and hold and turn: need a device check) |
| **–** and **+** | Step the picked row's value down or up. With the outline on a value, step it; on anything else, move the outline. With a finger resting on a value, step that value. | Works (the outline: needs a device check) |
| **The data wheel's push** | Opens the picked row: its value is outlined and the wheel changes it. Push again to go back to picking rows. On a row that opens something, opens it. With the outline on a control, does what a tap does; on a value, lets the wheel turn it until you push again. | Works (the outline: needs a device check) |

**The transport lights** light as Akai's own software lights them: **PLAY** stays lit while playing. **REC** and **OVERDUB** blink while they wait for PLAY, with PLAY blinking alongside, and keep blinking while recording or overdubbing; **STOP** lets go of a looper Record or Overdub still waiting for PLAY, and the lights go dark (see [Looping](../making-music/looping.md)). **UNDO** blinks while you record, and otherwise is bright when the page you are on has something to undo and dim when it has nothing. **TAP TEMPO** blinks on the beat. The MPC One's STOP button has no light of its own.

## Repeat and arpeggiator

| Control | What it does | When |
|---|---|---|
| **NOTE REPEAT** (hold) | While held, pads repeat in time at the repeat rate. | Works |
| **NOTE REPEAT** twice, or **SHIFT + NOTE REPEAT** | Latch: repeat keeps going after you let go. Press again to stop. | Works |
| **NOTE REPEAT + MENU** | Opens the Arp panel. | Works |

## Arrange tab

| Control | What it does | When |
|---|---|---|
| **Data wheel** | Picks a row of the Record settings, or of the Edit options while the note editor is open. **–** and **+** change it; push the wheel and the wheel changes it. | Works |
| **SHIFT + data wheel** | Scrolls the clip grid, the song, or the note editor's roll, list or tracker. | Works |
| **Q-Link knobs 1-4** | The same knobs as on the Play page, as the cards on the right show; Q-LINK steps to the macro banks. See [Clips and patterns](../making-music/clips.md#the-q-links-on-arrange). | Works |

## Pads tab

On the Pads tab the controls work as on the Sample tab (below): the wheel picks a row, **–** and **+** change it, and **SHIFT** + wheel steps through the pads with a sound. **Browse for** at the top right opens the browser (next section). See [Sampling](../making-music/pads.md#the-page).

## The browser

While the browser shows (**BROWSE**, the folder at the right end of the top bar, or the Pads tab's **Browse for**), the controls work it. See [Saving your work](../making-music/saving.md#the-browser).

| Control | What it does | When |
|---|---|---|
| **Data wheel** | Moves through the list. | Works |
| **SHIFT + data wheel** | Moves through the places on the left. | Works |
| **The data wheel's push** | Opens the folder or project picked, loads anything else (a sound onto the pad picked, a whole kit for a kit), or answers yes to the question on screen. | Works |
| **–** | Back up one folder. | Works |
| **+** | Into the folder picked, or into the kit picked to see its sounds. | Works |
| **PLAY** | Hears the sound picked (press again to stop it). PLAY doesn't start playback while the browser shows. | Works |
| **ERASE** | Moves the item picked to the Trash (asks first). | Works |
| **Q-Link knobs 1–4** | 1: moves through the list. 2: the filter. 3: the level you hear sounds at. 4: the places. | Works |

When the browser is open on a synth's presets (a tap on the preset's name on the Synth tab), the data wheel plays each preset as it reaches it, its push keeps the one you're on and goes back to the Synth tab, **–** and **+** move between Factory, Yours, Recent and Favourites, and **BROWSE** puts back the sound you had, as **Cancel** does. See [Trying and saving presets](../making-music/synths.md#trying-and-saving-presets).

In the **Plugins** place, the data wheel moves through the plugins and its push shows the next picture; **+** installs the one picked (or updates it, or installs its zip), **–** takes back a Remove you tapped or clears a search, and Q-Link 2 changes the All, Instruments and Effects chips. See [Getting plugins](../making-music/plugins.md).

While the Export panel shows, the data wheel sets the number of bars, its push presses **Start** (and **Close** once the export is over), and **–** closes the panel. See [Exporting your music](../making-music/exporting.md).

## Sample and Synth tabs

| Control | What it does | When |
|---|---|---|
| **Data wheel** | Picks a row of the settings on the right (on the Synth tab, a tile). | Works |
| **–** and **+** | Change the row or tile picked. | Works |
| **The data wheel's push** | Opens the row or tile picked: the wheel changes it until you push again. | Works |
| **SHIFT + data wheel** | Sample: the previous or next pad with a sound. Synth: the previous or next page. | Works |
| **SHIFT + –** and **SHIFT + +** | Wave and Chop pages: zoom out and in. Synth: the previous or next page. | Works |

## Mode buttons

The mode buttons along the top of the MPC go to the MPFree page that does the same job.

| Button | Goes to | When |
|---|---|---|
| **MAIN** | The Play page (it has one view, so pressed there it stays). In [Perform](../making-music/playing-live.md#make-and-perform), the Arrange tab's Clips grid; pressed there, the Song view, then the [song list](../making-music/arranging.md#the-song-list), then the grid again. | Works |
| **SHIFT + MAIN** | Switches between [Make and Perform](../making-music/playing-live.md#make-and-perform), as the MAKE or PERFORM chip on the top bar does. | Works |
| **STEP SEQ** | The pads become steps of one sound over any page, and the Sequencer tab shows its Steps (closing the note editor): hit an empty step to put a note there, a lit one to take it off. Press it again to play the pads. Hold **STEP SEQ** and hit a pad to pick the sound. **PAD BANK** turns the pages of steps; **SHIFT + STEP SEQ** turns on Follow. Hold a step pad and turn the Q-Links to change that step, or lock its sound on the Q-Links' Sound bank; hold it and hit a later one to make the note last to there. **SHIFT** + an empty step holds it with no note, for a lock alone. With a synth on the pads, the pads are the steps of its whole line: hold **STEP SEQ** and play a pad or a chord to pick what new steps play; hold a step and turn **Q-Link 1** to move its notes, or play a keyboard to put its keys on it. See [Your first beat](../making-music/sequencer.md#the-pads-as-steps-step-seq) and [Steps on the synth](../making-music/synths.md#steps-on-the-synth). | Works (new: needs a device check) |
| **NEXT SEQ** | The Arrange tab. | Works |
| **SAMPLER** | The Sample tab. **SHIFT + SAMPLER** goes to the Looper tab; pressed again there, the pads become the [loop keys](../making-music/looping.md#the-pads-as-loop-keys), or the [Perform set](../making-music/looping.md#more-pages-the-perform-set)'s page you left it on (again: they play). | Works (Perform pages new: needs a device check) |
| **SAMPLE EDIT** | The Sample tab's Wave page. | Works |
| **PROG EDIT** | The sound the pads play: the Synth tab while the synth is on the pads, otherwise the Pads tab. | Works |
| **TRACK MUTE** | The pads become mute keys over any page: pads 1-16 mute and unmute tracks 1-16, and **PAD BANK** picks the next 16 (B: 17-32, up to H: 113-128). **SHIFT + TRACK MUTE**: each pad mutes and unmutes itself in the kit. Press the same button again to play the pads. In Perform a mute waits for the launch line (the pad blinks amber), and SHIFT + the pad mutes at once; in Make it's the other way round. **Hold TRACK MUTE + a pad** to solo that track (lit blue), or over the kit's pads, that pad. See [Playing live](../making-music/playing-live.md#muting-tracks-and-pads). | Works (the timing and solo: need a device check) |
| **Q-LINK** | Steps the Q-Links through Macros 1-4, Macros 5-8 and the page's own knobs (the Play page, the Sequencer, Arrange and the chain editor). See [Effects](../making-music/effects.md#macros). While you hold a step pad with **STEP SEQ** on, it steps the Q-Links through the step's velocity, length, nudge and chance, its condition, ratchet and fine nudge, and its sound's locks (tune, decay, filter and level, or the synth's first four knobs). With a step picked on the Sequencer's Steps, it steps between the step's velocity, length, nudge and chance and its condition, ratchet and fine nudge. | Works |
| **ERASE** | Hold **ERASE** and tap something on the screen to delete it: a clip on the Arrange tab, a pattern row's name on the Play page (that sound's steps), or a layer on the Looper tab, or a loop's ring there (the whole loop). With **STEP SEQ** on, **ERASE** + a step pad clears that step, **ERASE** + a **PAD BANK** button clears that page of steps, and **ERASE** pressed while you hold step pads takes their locks off and keeps their notes. **UNDO** brings it back. The hint line says what you can tap while you hold it. A clip used in the song, effects and macros can't be deleted this way yet. In the browser, **ERASE** moves the item picked to the Trash (asking first). While you record, hold **ERASE** and hold a pad: that pad's hits come out of the clip as the playhead passes them, and the pad makes no sound (see [Erasing while you record](../making-music/clips.md#erasing-while-you-record)). | Works (live erase new: needs a device check) |
| **BROWSE** | Opens the browser over the page, and goes back to the page. **SHIFT + BROWSE** saves the project, on any page. See [Saving your work](../making-music/saving.md). | Works |
| **The data wheel's push** | See [Everywhere](#everywhere). | Works |
| **COPY** | With **STEP SEQ** on: hold **COPY**, hit a step pad, then hit the steps it should go to. Or hold **COPY**, press a **PAD BANK** button, then the **PAD BANK** button it should go to, to copy a whole page of the sound's steps. **UNDO** takes a copy back. See [Your first beat](../making-music/sequencer.md#the-pads-as-steps-step-seq). On the Arrange tab: hold **COPY** and tap a clip to copy it, then hold **COPY** and tap an empty slot for a copy there (see [Clips and patterns](../making-music/clips.md#tracks-and-slots)). | Works (new: needs a device check) |
| **TC** | Not assigned yet. | Planned |

## For developers

The Console, a text view of what MPFree is doing, opens with a chord nothing else uses: hold **SHIFT**, hold **ERASE**, then press **TAP TEMPO** (the same again closes it). The Dev tab appears when Prefs > Show developer tools is on.

> [!MPC]
> Most buttons do what their label says, as on the MPC. The differences: **MAIN** in Perform cycles the Arrange tab's views, **STEP SEQ** keeps the pads as steps on every page until you press it again, **SHIFT + REC** is Capture rather than Automation, and **SHIFT + MENU** opens MPFree's Prefs.
