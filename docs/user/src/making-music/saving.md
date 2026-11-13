<!--
status: changing
covers: rust/mpfree-store rust/mpfree-library rust/mpfree-browser-ui rust/mpfree-app/src/app/saved.rs rust/mpfree-app/src/project.rs
checked: 2026-11-13 e5c172f
-->

# Saving your work

MPFree keeps everything you save on its own USB stick. The MPC's internal storage and its SD card are never written, so the factory software and your MPC projects are exactly as you left them.

> [!IMPORTANT]
> Saving projects arrives in the next build. Until then your music lasts until you switch off; settings and arpeggiator patterns are already kept.

## What is kept today

These are saved on the stick as soon as you change them, and are back the next time MPFree starts:

| What | Where you set it |
|---|---|
| Your Prefs: pad velocity curve, how Latch keeps pads, the audio buffer, the loop memory, developer tools, screen pad velocity | [Prefs](../reference/prefs.md) |
| The buttons MPFree has learned | Prefs > Map controls, or the button guide |
| Arpeggiator patterns you save | The pattern workspace's **Save** (see [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md#the-pattern-workspace)) |
| The Synth tab's options: the engine on the pads, the scale and its root, each pad bank's layout, In key rows, and the tuning with its root and A4. The pads still start on bank A, in their usual octave. | MENU on the Synth tab (see [Synths](synths.md)) |

The line input is never kept on: it is off every time MPFree starts, so a noisy input you forgot about is never suddenly loud.

Saving is safe to interrupt. Each file is written whole to the side and only then swapped in, so if the power goes off in the middle of a save, the previous version is still there.

## Projects

A **project** is one song: everything you need to pick it up again where you left it.

| Kept in a project | Not yet |
|---|---|
| The tempo | The Looper's loops |
| The clips and the song on Arrange, with their scenes | The arpeggiator's settings on each track |
| The kits, with every sound on their pads and each pad's settings | |
| The synth sounds and the Synth tab's options | |
| The mixer: each strip's level, pan, sends and mutes, the effects and their settings, and the macros | |
| The MIDI, sync and CV setup | |

The **Level** (the master volume) is never taken from a project: it stays where you set it, so opening a project can't suddenly be loud.

### Saving

Press **SHIFT + BROWSE** on any page to save. The first save needs no typing: the project is named after the date and time (Project 2026-11-13 21.04), and you can rename it in the browser later. After that, SHIFT + BROWSE saves over the same project.

In the browser's **Projects** place you also have:

- **Save**, the same as SHIFT + BROWSE;
- **Save as**, which asks for a name and saves the work as a new project;
- **New project**, below.

A save is all or nothing. The project's new files are written to the side first and only then swapped in, so if the power goes off during a save, the project opens as it was at the save before. A save that can't finish (a full stick, say) says so and leaves the last save as it was.

The last ten saves of each project are kept. Pick a project and press **Earlier saves** to see them by when they were saved, and open one to go back. Saving it then makes it the newest, and nothing is lost.

### Opening a project

Press **BROWSE**, pick **Projects** on the left, pick a project and push the data wheel (or press **Open**). Playback stops, and the project's clips, kits, synth sounds and mixer come in together. A part MPFree can't read (from a newer version, say) is left as it is, and the Events log says which.

### Starting a new project

**New project** in the Projects place starts again from nothing: an empty song with 8 tracks, the factory kits, and the synth sounds and mixer as they are when MPFree starts. Nothing is saved until you save, and the first save names a new project.

If you have changed anything since you last saved or opened, opening a project or New project asks first, so you never lose work by accident. Playing the pads, changing page or the Level doesn't count as a change.

## The browser

**BROWSE** opens the browser over the page you are on, and pressing it again goes back to that page. It is a full file manager for MPFree's stick:

- **Places** on the left: Projects, Samples, Kits, Instruments, Patterns and Recordings, plus Recent, Favourites, the Trash, and Factory (read-only kits).
- **The list** in the middle, folders first, in natural order (Kick 2 before Kick 10), with filters for audio, projects and kits.
- **The selected item** on the right: a sound's length and waveform, a project's tempo and when it was saved, and buttons to Load, Load to pad, Audition, Favourite, Rename, Copy, Cut and Delete.

Moving onto a sound plays it once, so you can audition as you scroll. Delete always asks first and moves things to the Trash, which you empty yourself (that asks again).

**Load** puts a sound on the pad you are editing, and **Load to pad** on the next pad you hit. WAV, AIFF and FLAC files load, mono or stereo, at 8 to 192 kHz, whatever their name says. A file that was cut short, by a recorder that lost power say, loads the part it has. A FLAC file with a damaged part doesn't load at all, rather than play it as noise. A pad you exported from the Sample tab loads with its settings: load its sound, and the settings file beside it brings back its trim, loop, layers and slices (see [Exporting a pad as files](sampling.md#exporting-a-pad-as-files)).

The data wheel moves through the list and its push opens or loads; the other buttons are in [Buttons and controls](../reference/controls.md#the-browser). On the test build, BROWSE works once **Map controls** in [Prefs](../reference/prefs.md) has learned it.

### Pickers

Where you need one kind of file, a narrower **picker** shows the same browser with only what fits. A picker only loads; renaming and deleting are in the full browser.

The Pads tab's sound picker stays open beside the pads, with Load, Load to pad, Audition and Favourite. See [Sampling](sampling.md#the-pads-tab-sounds-onto-pads). A preset picker on the Synth tab (each turn of the wheel lets you play the next preset on the pads before you choose it) comes later.

### Your own synth presets

Synth presets you save go in **Instruments**, in a folder for each synth, and show up in that synth's preset list and picker.

## Getting files on and off the stick

Later versions will share MPFree's library over the network cable and by USB, so you can copy samples in and projects out from a computer. Until then, the files are on a Linux-format part of the stick that Windows and macOS can't open without extra software.

> [!MPC]
> On the MPC, projects are saved to the internal drive, an SD card or a USB drive, with the samples in a folder beside the project file. MPFree saves only to its own stick, keeps every sample inside the project folder, and keeps the last ten versions of each project. MPFree can't open MPC projects (`.xpj`) yet; a tool to bring them across is being looked at.
