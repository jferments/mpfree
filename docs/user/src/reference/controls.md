<!--
status: changing
covers: rust/mpfree-app/src/app.rs rust/mpfree-controller docs/dev/design/layout-spec.md
checked: 2026-11-14 b76c3c7
-->

# Buttons and controls

This is the one place the manual lists exactly what every button and combination does. Other chapters link here instead of repeating it, so if a button changes, only this page changes.

"SHIFT + MENU" means hold SHIFT, then press MENU.

On the current test build, some buttons (REC, OVERDUB, UNDO, PLAY START, TRACK MIX, the PAD BANK buttons) work once **Map controls** in [Prefs](prefs.md) has learned them; the button guide walks you through it. The **When** column says whether a control works in the build this manual describes (see [Which version this describes](../index.md#which-version-this-describes)) or is planned.

{{shot: guide | The button guide shows what the buttons do on the page you are on.}}

## Everywhere

| Control | What it does | When |
|---|---|---|
| **MENU** | Opens the options for the page you are on. In Prefs' System info or Touch screen rows, goes back to the main Prefs rows. | Works |
| **SHIFT + MENU** | Opens Prefs. | Works |
| **STOP** | Stops playback; sounds ring out and latched pads keep going. Press again to silence everything and go back to bar 1. | Works |
| **SHIFT + STOP** | Stops, silences everything and goes back to the start of bar 1. | Works |
| **PLAY** | Starts playback from where it stopped. | Works |
| **PLAY** while playing | Hold for a fill; press twice to keep the fill on, and again (or STOP) to end it. See [Playing live](../making-music/playing-live.md#fills). | Works |
| **PLAY START** | Starts playback from the beginning. | Works |
| **REC** | On the Looper tab and on the Play page's All and Loop views, records a loop (as the loop's own Record button). On the Play page's Beats view and everywhere else, records a clip. | Works |
| **OVERDUB** | Records on top of what is already there: the loop where REC records a loop, otherwise the clip. | Works |
| **UNDO** | Undoes the last recording or edit on the page you are on, and the line at the bottom of the screen says what it took back. The Looper tab and the Play page's Loop view: the last loop layer. The Play page's Beats view and the Arrange tab: the last take (a recording, a step edit or a deleted clip). The Play page's All view: whichever of the two changed last. The Sample and Pads tabs: the last edit. The Synth tab: the last knob you turned. The mixer: the last fader, knob or button you changed on a strip. A run of turns of the same knob or fader is one UNDO. Other pages say they have nothing to undo (the FX chain editor and the macros panel get theirs with the effects overhaul). **SHIFT + UNDO** redoes it. **Hold UNDO** for a moment to see the page's history as a list: tap a step (or turn the wheel to it and push) to go straight back, or forward, to just after it. | Works |
| **SHIFT + REC** | Capture: keeps what you just played as a clip, even if you weren't recording. | Works |
| **SHIFT + PLAY START** | Metronome on or off (it starts off; MPFree remembers your choice). The click plays while counting in and recording. Its level is in the Play page's options (**MENU**). | Works |
| **TAP TEMPO** | Tap four times in time to set the tempo. Tapping the tempo on the Play page does the same. | Works |
| **TRACK MIX** | Opens or closes the mixer over the page you are on. **SHIFT + TRACK MIX** opens the pad mixer. | Works |
| **PAD BANK A–D** | Switches the pads to bank A, B, C or D. **SHIFT** + a bank button, or a second press, gives banks E–H. | Works |
| **FULL LEVEL** | Every pad plays at full velocity, however hard you hit it. Press again to turn it off. **SHIFT + FULL LEVEL** is Half Level (every hit at half velocity). | Works |
| **16 LEVEL** | 16 Levels: the pad you hit last plays on all 16 pads, each a step of velocity, tune, cutoff, decay or start, and the [16 Levels panel](../making-music/playing-live.md#16-levels) opens on Play. Press again to turn it off. **SHIFT + 16 LEVEL** switches the synth pads between Chromatic and In key (with no scale picked yet, In key picks C Major; the line at the bottom of the screen says which). | Works |
| **Q-Link knobs 1–4** | Turn the four parameters shown in the Q-Link column at the right of the screen. What they control depends on the page. | Works |
| **Data wheel** | On a page with settings (the menus, Prefs, the Sample tab, the Pads tab's Pad view, the Synth tab, Arrange): picks a row. On the Play page, picks the kit. | Works |
| **–** and **+** | Step the picked row's value down or up. | Works |
| **The data wheel's push** | Opens the picked row: its value is outlined and the wheel changes it. Push again to go back to picking rows. On a row that opens something, opens it. | Works |

**The transport lights** light as Akai's own software lights them: **PLAY** stays lit while playing. **REC** and **OVERDUB** blink while they wait for PLAY, with PLAY blinking alongside, and keep blinking while recording or overdubbing. **UNDO** blinks while you record, and otherwise is bright when the page you are on has something to undo and dim when it has nothing. **TAP TEMPO** blinks on the beat. The MPC One's STOP button has no light of its own. REC, OVERDUB and UNDO light once **Map controls** knows them.

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

While the Pads tab shows its **Browse** view, the controls work the browser beside the pads. On its **Pad** view they work as on the Sample tab (below): the wheel picks a row, **–** and **+** change it, and **SHIFT** + wheel steps through the pads with a sound. See [Sampling](../making-music/sampling.md#the-pads-tab-sounds-onto-pads).

| Control | What it does | When |
|---|---|---|
| **Data wheel** | Moves through the list. | Works |
| **SHIFT + data wheel** | Moves through the places on the left (Samples, Recordings, Recent, Favourites). | Works |
| **The data wheel's push** | Opens the folder picked, or loads the sound picked onto the pad picked. | Works |
| **–** | Back up one folder. | Works |
| **+** | Into the folder picked. | Works |
| **PLAY** | Hears the sound picked (press again to stop it). PLAY doesn't start playback while Browse shows. | Works |
| **Q-Link knobs 1–4** | 1: moves through the list. 3: the level you hear sounds at. 4: the places. | Works |

## The browser

While the browser shows (**BROWSE**), the controls work it. See [Saving your work](../making-music/saving.md#the-browser).

| Control | What it does | When |
|---|---|---|
| **Data wheel** | Moves through the list. | Works |
| **SHIFT + data wheel** | Moves through the places on the left. | Works |
| **The data wheel's push** | Opens the folder or project picked, loads anything else, or answers yes to the question on screen. | Works |
| **–** | Back up one folder. | Works |
| **+** | Into the folder picked. | Works |
| **PLAY** | Hears the sound picked. | Works |
| **ERASE** | Moves the item picked to the Trash (asks first). | Works |
| **Q-Link knobs 1–4** | 1: moves through the list. 2: the filter. 3: the level you hear sounds at. 4: the places. | Works |

While the Export panel shows, the data wheel sets the number of bars, its push presses **Start** (and **Close** once the export is over), and **–** closes the panel. See [Exporting your music](../making-music/exporting.md).

## Sample and Synth tabs

| Control | What it does | When |
|---|---|---|
| **Data wheel** | Picks a row of the settings on the right (on the Synth tab, a tile). | Works |
| **–** and **+** | Change the row or tile picked. | Works |
| **The data wheel's push** | Opens the row or tile picked: the wheel changes it until you push again. | Works |
| **SHIFT + data wheel** | Sample: the previous or next pad with a sound. Synth: the previous or next page. | Works |
| **SHIFT + –** and **SHIFT + +** | Wave, Warp and Chop pages: zoom out and in. Synth: the previous or next page. | Works |

## Mode buttons

The mode buttons along the top of the MPC go to the MPFree page that does the same job.

| Button | Goes to | When |
|---|---|---|
| **MAIN** | The Play page. Pressed on the Play page, the next view (All, Beats, Loop). | Works |
| **STEP SEQ** | The Play page's Beats view. | Works |
| **NEXT SEQ** | The Arrange tab. | Works |
| **SAMPLER** | The Sample tab. **SHIFT + SAMPLER** goes to the Looper tab. | Works |
| **SAMPLE EDIT** | The Sample tab's Wave page. | Works |
| **PROG EDIT** | The sound the pads play: the Synth tab while the synth is on the pads, otherwise the Pads tab's Pad view. | Works |
| **TRACK MUTE** | The pads become mute keys over any page: pads 1-16 mute and unmute tracks 1-16, and **PAD BANK** picks the next 16 (B: 17-32, up to H: 113-128). **SHIFT + TRACK MUTE**: each pad mutes and unmutes itself in the kit. Press the same button again to play the pads. See [Playing live](../making-music/playing-live.md#muting-tracks-and-pads). | Works |
| **Q-LINK** | Steps the Q-Links through Macros 1-4, Macros 5-8 and the page's own knobs (the Play page, Arrange and the chain editor). See [Effects](../making-music/effects.md#macros). | Works |
| **ERASE** | Hold **ERASE** and tap something on the screen to delete it: a clip on the Arrange tab, a pattern row's name on the Play page (that sound's steps), or a layer on the Looper tab. **UNDO** brings it back. The hint line says what you can tap while you hold it. A clip used in the song, a whole loop, effects and macros can't be deleted this way yet. In the browser, **ERASE** moves the item picked to the Trash (asking first). Works once Map controls has found ERASE. Live erase while recording is planned. | Works |
| **BROWSE** | Opens the browser over the page, and goes back to the page. **SHIFT + BROWSE** saves the project, on any page. See [Saving your work](../making-music/saving.md). Works once Map controls has found it. | Works |
| **The data wheel's push** | See [Everywhere](#everywhere). Works once Map controls has found it. | Works |
| **COPY**, **TC** | Not assigned yet. | Planned |

## For developers

The Console, a text view of what MPFree is doing, opens with a chord nothing else uses: hold **SHIFT**, hold **ERASE**, then press **TAP TEMPO** (the same again closes it). The Dev tab appears when Prefs > Show developer tools is on.

> [!MPC]
> Most buttons do what their label says, as on the MPC. The differences: **MAIN** cycles the Play page's views, **STEP SEQ** opens the Beats view rather than a separate step editor, **SHIFT + REC** is Capture rather than Automation, and **SHIFT + MENU** opens MPFree's Prefs.
