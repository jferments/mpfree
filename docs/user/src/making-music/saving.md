<!--
status: changing
covers: rust/mpfree-store rust/mpfree-library rust/mpfree-browser-ui rust/mpfree-app/src/app/saved.rs rust/mpfree-app/src/project.rs rust/mpfree-app/src/app/arp/part.rs
checked: 2026-11-16 d653b57
-->

# Saving your work

MPFree keeps everything you save on its own USB stick. The MPC's internal storage and its SD card are never written, so the factory software and your MPC projects are exactly as you left them.

To save the song you're working on, press **SHIFT + BROWSE** on any page. The rest of this chapter says what is kept where, and how to open, name and go back to your projects.

## Your song, and your setup

MPFree keeps two kinds of things, and keeps them apart:

- **The song's**, in its project: everything that makes the music sound the way it does. They change when you open another project, and go back to their defaults with New project.
- **Yours**, in Prefs: how you like to play, your MPC and what is plugged into it. They stay the same whichever project is open, and New project leaves them alone.

A good test: if you sent the project to a friend with another MPC, would they want it? Then it is the song's. If it is about your hands, your MPC or your desk, it is yours.

## What MPFree keeps for you

These are saved on the stick as soon as you change them, and are back the next time MPFree starts, whatever project you open:

| What | Where you set it |
|---|---|
| Your Prefs: pad velocity curve, how Latch keeps pads, the audio buffer, the loop memory, developer tools, the theme, what opens at start, how exports play, screen pad velocity | [Prefs](../reference/prefs.md) |
| The metronome on or off, and its level | **SHIFT + PLAY START**, or the Play page's options (**MENU**) |
| The Looper's level, and the line input's latency once you have set it by hand | The Looper tab's options (see [Recording loops](looping.md)) |
| Whether the Play page's Beats view shows the timing strip | The Play page's options (**MENU**) |
| The buttons MPFree has learned | Prefs > Map controls, or the button guide |
| Each MIDI port's input (Plays tracks, Learn only or Both) and thru | The mixer's I/O view, MIDI section (see [MIDI, sync and CV](midi-and-cv.md)) |
| Arpeggiator patterns you save, and your own scales | The pattern workspace's **Save** (see [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md#the-pattern-workspace)) |
| Recent and Favourites in the browser | The browser |

Two things are never kept, for safety: the line input is off every time MPFree starts (so a noisy input you forgot about is never suddenly loud, and the Looper records the pads until you turn it on), and the **Level** (the master volume) never comes from a project.

Saving is safe to interrupt. Each file is written whole to the side and only then swapped in, so if the power goes off in the middle of a save, the previous version is still there.

## Projects

A **project** is one song: everything you need to pick it up again where you left it.

| Kept in a project |
|---|
| The tempo and the swing |
| The clips and the song on Arrange, with their scenes |
| The kits, with every sound on their pads and each pad's settings |
| The synth sounds, and the Synth tab's options: the engine on the pads, the scale and its root, each pad bank's layout, In key rows, and the tuning with its root and A4 |
| Each track's own instrument (Poly, FM, a plugin synth...), with its sound or the plugin's settings |
| Each track's arpeggiator: its mode, rate (the rate Note Repeat plays at too), order, octaves, gate, swing, scale and the other options, and its pattern with your edits |
| The mixer: each strip's level, pan, sends and mutes, the effects and their settings (plugins' too), and the macros |
| The Looper's loops, with each layer's mix |
| The audio tracks on the Song view: every take, the regions comped from them, and each track's input, monitor, level, pan, mute and solo |
| The MIDI, sync and CV setup: what each track plays on, where the tempo comes from, clock out, the CV outputs and the controls you learned |

Latch isn't kept: it is how you play, not part of the song, so every arp lets go when a project opens. The pads stay on the bank and octave you were playing.

The **Level** (the master volume) is never taken from a project: it stays where you set it, so opening a project can't suddenly be loud.

### Saving

Press **SHIFT + BROWSE** on any page to save. The loops and the audio tracks keep playing while they are saved; if a loop or an audio track is recording, the save waits until it stops. The first save needs no typing: the project is named after the date and time (Project 2026-11-13 21.04), and you can rename it in the browser later. After that, SHIFT + BROWSE saves over the same project.

In the browser's **Projects** place you also have:

- **Save**, the same as SHIFT + BROWSE;
- **Save as**, which asks for a name and saves the work as a new project;
- **New project**, below;
- **Export**, which makes a mixdown, stems and a MIDI file of the song (see [Exporting your music](exporting.md)).

A save is all or nothing. The project's new files are written to the side first and only then swapped in, so if the power goes off during a save, the project opens as it was at the save before. A save that can't finish (a full stick, say) says so and leaves the last save as it was.

The last ten saves of each project are kept. Pick a project and press **Earlier saves** to see them by when they were saved, and open one to go back. Saving it then makes it the newest, and nothing is lost.

### Autosave

While you have changes you haven't saved, MPFree saves them every 2 minutes into a project of its own, **Autosave** in the Projects place, even while it plays. It never saves over your project: your project changes only when you save it, and your work still counts as unsaved. Autosave keeps only its latest copy, with no earlier saves, and nothing on the screen changes when it saves. If you have a project of your own called Autosave, it uses Autosave 2 instead.

A short press of the power button asks whether to save before the MPC switches off (see [Switching off and restarting](../getting-started/before-you-start.md#switching-off-and-restarting)). If MPFree stops while you have unsaved changes (a crash, or the MPC switched off by holding the power button), the next start keeps what autosave had as a project of its own, named after when it was autosaved (Unsaved 2026-11-14 21.04), and for a few seconds the hint line at the bottom of the screen says where it is (the Events log keeps it). Open it from **BROWSE** like any project: it opens as unsaved work, and **SHIFT + BROWSE** saves it as a new project. Delete it in the browser once you don't need it. The changes from the last 2 minutes or so before the stop may be missing, so save often all the same.

### Opening a project

Press **BROWSE**, pick **Projects** on the left, pick a project and push the data wheel (or tap it again). Playback stops, and the project's clips, kits, synth sounds, tracks' instruments and mixer come in together. A plugin the project used that isn't in `files/plugins/` on this stick is left out, and the Events log names its file so you can put it back: a missing plugin effect leaves its strip without it, and a track whose plugin synth is missing plays the kit. The loops you had fade out, and the project's loops come in a moment later. The same goes for the audio tracks: a recording ends, and the project's takes come in a moment later. If the project's takes don't all fit in **Audio track memory** ([Prefs](../reference/prefs.md)), the Events log says which were left out. A part MPFree can't read (from a newer version, say) is left as it is, and the Events log says which.

### Starting a new project

**New project** in the Projects place starts again from nothing: an empty song with 8 tracks, every track on the kit, no loops, no audio tracks, the factory kits, every arp at its default, the MIDI, sync and CV settings at their defaults, the synth sounds and mixer as they are when MPFree starts, and the synth pads chromatic in C, tuned to A4 = 440 Hz. Your Prefs and the other things MPFree keeps for you stay as they are. Nothing is saved until you save, and the first save names a new project.

If you have changed anything since you last saved or opened, opening a project or New project asks first, so you never lose work by accident. Playing the pads, changing page or the Level doesn't count as a change.

### At start

MPFree starts with a new, empty project. To have it open the project you last saved or opened instead, set **At start** in [Prefs](../reference/prefs.md) to **Last project**: once the sound is going, the project opens as if you had opened it from the browser. If it has been deleted since, the Events log says so and the new project stays.

**After a crash**, MPFree starts again by itself a few seconds later, and this time it opens nothing: the new, empty project stays even if At start says **Last project**, and for a few seconds the hint line at the bottom of the screen says which project wasn't opened, and where your unsaved work is if autosave had any (the Events log keeps it). Open it from **BROWSE** when you're ready. This way a project that makes MPFree crash as it opens can't do it again at every start. Switching the MPC off and on starts as usual.

## The browser

{{shot: browser | The browser in Projects, with Night drive picked: its tempo and when it was saved on the right, with Earlier saves.}}

**BROWSE** opens the browser over the page you are on, and pressing it again goes back to that page. It is a full file manager for MPFree's stick:

- **Places** on the left: Projects, Samples, Kits, Instruments, Patterns and Recordings, plus Recent, Favourites, the Trash, and Factory (read-only kits). **Plugins** finds and installs MPC plugins (see [Getting plugins](plugins.md)).
- **The list** in the middle, folders first, in natural order (Kick 2 before Kick 10), with filters for audio, projects and kits. Each row has a **star** at its right end: tap it to add the item to Favourites, and again to take it out.
- **The selected item** on the right: a sound's length and waveform, a project's tempo and when it was saved, and buttons to Load, Load to pad, Audition, Rename, Copy, Cut and Delete.

**Opening things.** Tap a folder to open it. Tap anything else to pick it (a sound plays once), and tap it again to open it (a project) or load it (a sound, a kit, a preset). Pushing the data wheel does the same as that second tap.

**Kits.** A kit is kept as a folder with its sounds and a `kit.txt` file listing every pad and its settings, so it reads on any computer. Kits show with a green icon wherever they are. Loading one puts it in place of the kit the pads play; **+** opens it to show its sounds. **Save kit** at the top of the **Kits** place saves the kit the pads play there, named after it. MPFree's own drum kits are in **Kits > MPFree**.

A folder with thousands of sounds lists straight away; the sizes fill in a moment later.

Moving onto a sound plays it once, so you can audition as you scroll. Delete always asks first and moves things to the Trash, which you empty yourself (that asks again).

**Load** puts a sound on the pad you are editing, and **Load to pad** on the next pad you hit. WAV, AIFF and FLAC files load, mono or stereo, at 8 to 192 kHz, whatever their name says. A file that was cut short, by a recorder that lost power say, loads the part it has. A FLAC file with a damaged part doesn't load at all, rather than play it as noise. A pad you exported from the Sample tab loads with its settings: load its sound, and the settings file beside it brings back its trim, loop, layers and slices (see [Exporting a pad as files](sampling.md#exporting-a-pad-as-files)).

The data wheel moves through the list and its push opens or loads; the other buttons are in [Buttons and controls](../reference/controls.md#the-browser). On the test build, BROWSE works once **Map controls** in [Prefs](../reference/prefs.md) has learned it.

### Pickers

Where you need one kind of file, a narrower **picker** shows the same browser with only what fits. A picker only loads; renaming and deleting are in the full browser.

The Pads tab's sound picker stays open beside the pads, with Load, Load to pad and Audition, the stars, and the Kits place with **Save kit**. See [Sampling](sampling.md#the-pads-tab-sounds-onto-pads). A preset picker on the Synth tab (each turn of the wheel lets you play the next preset on the pads before you choose it) comes later.

### Your own synth presets

Synth presets you save go in **Instruments**, in a folder for each synth, and show up in that synth's preset list and picker.

## Getting files on and off the stick

Later versions will share MPFree's library over the network cable and by USB, so you can copy samples in and projects out from a computer. Until then, the files are on a Linux-format part of the stick that Windows and macOS can't open without extra software.

> [!MPC]
> On the MPC, projects are saved to the internal drive, an SD card or a USB drive, with the samples in a folder beside the project file. MPFree saves only to its own stick, keeps every sample inside the project folder, and keeps the last ten versions of each project. MPFree can't open MPC projects (`.xpj`) yet; a tool to bring them across is being looked at.
