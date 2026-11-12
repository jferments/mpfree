<!--
status: changing
covers: rust/mpfree-app/src/app.rs rust/mpfree-controller docs/dev/design/layout-spec.md
checked: 2026-11-12 b9b7477
-->

# Buttons and controls

This is the one place the manual lists exactly what every button and combination does. Other chapters link here instead of repeating it, so if a button changes, only this page changes.

"SHIFT + MENU" means hold SHIFT, then press MENU.

On the current test build, some buttons (REC, OVERDUB, UNDO, PLAY START, TRACK MIX, the PAD BANK buttons) work once **Map controls** in [Prefs](prefs.md) has learned them; the button guide walks you through it. The **When** column says whether a control works in the current test build, arrives in the next one, or is planned.

{{shot: guide | The button guide shows what the buttons do on the page you are on.}}

## Everywhere

| Control | What it does | When |
|---|---|---|
| **MENU** | Opens the options for the page you are on. In Prefs' System info or Touch screen rows, goes back to the main Prefs rows. | Works |
| **SHIFT + MENU** | Opens Prefs. | Works |
| **STOP** | Stops playback; sounds ring out and latched pads keep going. Press again to silence everything and go back to bar 1. | Works |
| **SHIFT + STOP** | Stops, silences everything and goes back to the start of bar 1. | Next build |
| **PLAY** | Starts playback from where it stopped. | Works |
| **PLAY** while playing | Hold for a fill; press twice to keep the fill on, and again (or STOP) to end it. See [Playing live](../making-music/playing-live.md#fills). | Next build |
| **PLAY START** | Starts playback from the beginning. | Works |
| **REC** | On the Looper tab, records a loop. Everywhere else, records a clip. | Works |
| **OVERDUB** | Records on top of what is already there (a clip or a loop). | Works |
| **UNDO** | Undoes the last recording or edit on the page you are on (the Looper tab: the last loop layer; the Sample tab: the last edit; elsewhere: the last clip take). **SHIFT + UNDO** redoes it. | Works |
| **SHIFT + REC** | Capture: keeps what you just played as a clip, even if you weren't recording. | Next build |
| **SHIFT + PLAY START** | Metronome on or off. | Next build |
| **TAP TEMPO** | Tap four times in time to set the tempo. | Works |
| **TRACK MIX** | Opens or closes the mixer over the page you are on. **SHIFT + TRACK MIX** opens the pad mixer. | Works |
| **PAD BANK A–D** | Switches the pads to bank A, B, C or D. **SHIFT** + a bank button, or a second press, gives banks E–H. | Works |
| **FULL LEVEL** | Every pad plays at full velocity, however hard you hit it. Press again to turn it off. **SHIFT + FULL LEVEL** is Half Level (every hit at half velocity). | Works |
| **16 LEVEL** | Planned. **SHIFT + 16 LEVEL** switches the synth pads between Chromatic and In key. | Next build (SHIFT) |
| **Q-Link knobs 1–4** | Turn the four parameters shown in the Q-Link column at the right of the screen. What they control depends on the page. | Works |
| **Data wheel** | Changes the selected value; on the Play page, picks the kit. | Works |
| **–** and **+** | Step the selected menu row down or up. | Works |

## Repeat and arpeggiator

| Control | What it does | When |
|---|---|---|
| **NOTE REPEAT** (hold) | While held, pads repeat in time at the repeat rate. | Works |
| **NOTE REPEAT** twice, or **SHIFT + NOTE REPEAT** | Latch: repeat keeps going after you let go. Press again to stop. | Works |
| **NOTE REPEAT + MENU** | Opens the Arp panel. | Works |

## Arrange tab

| Control | What it does | When |
|---|---|---|
| **Data wheel** | Changes the picked row of the Record settings, or of the Edit options while the note editor is open. | Works |
| **SHIFT + data wheel** | Scrolls the clip grid, the song, or the note editor's roll or list. | Works |

## Mode buttons

The mode buttons along the top of the MPC go to the MPFree page that does the same job.

| Button | Goes to | When |
|---|---|---|
| **MAIN** | The Play page. Pressed on the Play page, the next view (All, Beats, Loop, Drum). | Next build |
| **STEP SEQ** | The Play page's Beats view. | Next build |
| **NEXT SEQ** | The Arrange tab. | Next build |
| **SAMPLER** | The Sample tab. **SHIFT + SAMPLER** goes to the Looper tab. | Next build |
| **SAMPLE EDIT** | The Sample tab's Wave page. | Next build |
| **PROG EDIT** | The sound the pads play: the Synth tab while the synth is on the pads, otherwise the Sample tab's Pads page. | Next build |
| **TRACK MUTE** | The pads become mute keys over any page: pads 1-8 mute and unmute tracks 1-8. **SHIFT + TRACK MUTE**: each pad mutes and unmutes itself in the kit. Press the same button again to play the pads. See [Playing live](../making-music/playing-live.md#muting-tracks-and-pads). | Next build |
| **Q-LINK** | Steps the Q-Links through Macros 1-4, Macros 5-8 and the page's own knobs (the Play page and the chain editor). See [Mixing and effects](../making-music/mixing-and-effects.md#macros). | Next build |
| **ERASE** | Live erase while recording. | Planned |
| **BROWSE** | Opens or closes the browser. **SHIFT + BROWSE** saves the project. See [Saving your work](../making-music/saving.md). | Planned |
| **COPY**, **TC**, the data wheel's push | Not assigned yet. | Planned |

## For developers

The Console, a text view of what MPFree is doing, opens with a chord nothing else uses: hold **SHIFT**, hold **ERASE**, then press **TAP TEMPO** (the same again closes it). On the current test build it is **SHIFT + STOP**. The Dev tab appears when Prefs > Show developer tools is on.

> [!MPC]
> Most buttons do what their label says, as on the MPC. The differences: **MAIN** cycles the Play page's views, **STEP SEQ** opens the Beats view rather than a separate step editor, **SHIFT + REC** is Capture rather than Automation, and **SHIFT + MENU** opens MPFree's Prefs.
