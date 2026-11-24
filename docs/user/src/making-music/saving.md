<!--
status: changing
covers: rust/mpfree-store rust/mpfree-library rust/mpfree-browser-ui rust/mpfree-app/src/app/saved.rs rust/mpfree-app/src/project.rs rust/mpfree-app/src/app/arp/part.rs
checked: 2026-11-22 f8d9d6fe
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
| Your Prefs: pad velocity curve, how Latch keeps pads and whether the sustain pedal latches, Quick advance, the audio buffer, the loop memory, developer tools, the theme, what opens at start, how exports play, screen pad velocity | [Prefs](../reference/prefs.md) |
| The metronome on or off, and its level | **SHIFT + PLAY START**, or the Play page's options (**MENU**) |
| The Looper's level, and the line input's latency once you have set it by hand | The Looper tab's options (see [Recording loops](looping.md)) |
| Whether the Sequencer's Steps view shows the timing strip | The Sequencer's or the Play page's options (**MENU**) |
| The buttons MPFree has learned | Prefs > Map controls, or the button guide |
| Each MIDI port's input (Plays tracks, Learn only or Both), what it ignores, its gear's delay, its clock's nudge and its device profile | The mixer's I/O view, MIDI section and the port's page (see [MIDI, sync and CV](midi-and-cv.md)) |
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
| The synth sounds, and the Synth tab's options: the engine on the pads, the scale and its root, each pad bank's layout, In key rows, and the song's tuning with its root and A4, and each instrument's own |
| Each track's own instrument (Poly, FM, a plugin synth...), with its sound or the plugin's settings |
| Which kit, or the synth, each other track plays |
| Each track's arpeggiator: its mode, rate (the rate Note Repeat plays at too), order, octaves, gate, swing, scale and the other options, and its pattern with your edits |
| The mixer: each strip's level, pan, sends and mutes, the effects and their settings (plugins' too), and the macros |
| The Looper's loops, with each layer's mix |
| The audio tracks on the Song view: every take, the regions comped from them, and each track's input, monitor, level, pan, mute and solo |
| The MIDI, sync and CV setup: what each track plays on, where the tempo comes from, clock out, the CV outputs and the controls you learned |

Latch isn't kept: it is how you play, not part of the song, so every arp lets go when a project opens. The pads stay on the bank and octave you were playing.

The **Level** (the master volume) is never taken from a project: it stays where you set it, so opening a project can't suddenly be loud.

### Saving

Press **SHIFT + BROWSE** on any page to save. The loops and the audio tracks keep playing while they are saved; if a loop or an audio track is recording, the save waits until it stops. The first save needs no typing: the project is named after the date and time (Project 2026-11-13 21.04), and you can rename it in the browser later. After that, SHIFT + BROWSE saves over the same project, unless the project opened with something left out (see [Opening a project](#opening-a-project)). Right after you open a project, a save waits until its kits, loops and audio tracks have come in.

In the browser's **Projects** place you also have:

- **Save**, the same as SHIFT + BROWSE;
- **Save as**, which asks for a name and saves the work as a new project;
- **Export**, which makes a mixdown, stems and a MIDI file of the song (see [Exporting your music](exporting.md)).

Each project also carries a hidden id that stays with it when you rename it, so a set list will find the song wherever it is. **Save as** gives the new project an id of its own. If you copy a project's folder on a computer, the copy still carries the original's id at first; the first time you save the copy, MPFree tells them apart and says so: *Saved Night drive copy. It was a copy of Night drive, so from now on it counts as a song of its own*. The original keeps its id.

A save is all or nothing. The project's new files are written to the side first and only then swapped in, so if the power goes off during a save, the project opens as it was at the save before. A save that can't finish (a full stick, say) says so and leaves the last save as it was.

The last ten saves of each project are kept. Pick a project and press **Earlier saves** to see them by when they were saved, and open one to go back. Saving it then makes it the newest, and nothing is lost.

Audio track takes you no longer use, in the song or in any of those ten earlier saves, go to the **Trash** when you save, so they stop taking room in the project, and the status line says how many (*Saved Night drive, and moved 2 takes it no longer uses to the Trash*). A take you erased stays as long as Undo or an earlier save can bring it back. Put one back from the Trash if you want it; emptying the Trash frees the space.

### Autosave

While you have changes you haven't saved, MPFree saves them every 2 minutes into a project of its own, **Autosave** in the Projects place, even while it plays. It never saves over your project: your project changes only when you save it, and your work still counts as unsaved. Autosave keeps only its latest copy, with no earlier saves, and nothing on the screen changes when it saves. If you have a project of your own called Autosave, it uses Autosave 2 instead.

A short press of the power button asks whether to save before the MPC switches off (see [Switching off and restarting](../getting-started/before-you-start.md#switching-off-and-restarting)). If MPFree stops while you have unsaved changes (a crash, or the MPC switched off by holding the power button), the next start keeps what autosave had as a project of its own, named after when it was autosaved (Unsaved 2026-11-14 21.04), and for a few seconds the hint line at the bottom of the screen says where it is (the Events log keeps it). Open it from **BROWSE** like any project: it opens as unsaved work, and **SHIFT + BROWSE** saves it as a new project. Delete it in the browser once you don't need it. The changes from the last 2 minutes or so before the stop may be missing, so save often all the same.

### Opening a project

Press **BROWSE**, pick **Projects** on the left, pick a project and push the data wheel (or tap it again). Playback stops, and the project's clips, kits, synth sounds, tracks' instruments and mixer come in together. A plugin the project used that isn't in `files/plugins/` on this stick is left out, and the Events log names its file so you can put it back: a missing plugin effect leaves its strip without it, and a track whose plugin synth is missing plays the kit. The loops you had fade out, and the project's loops come in a moment later, in their cells on the Loop tracks, waiting for a launch as the clips do. A project saved before Loop tracks opens with each loop on a Loop track of its own, in row 1, all playing as they did (see [Loops in the Clips grid](looping.md#loops-in-the-clips-grid)). The same goes for the audio tracks: a recording ends, and the project's takes come in a moment later. If the project's takes don't all fit in **Audio track memory** ([Prefs](../reference/prefs.md)), the Events log says which were left out.

**A project that can't open whole asks first.** Before anything changes, MPFree checks that every file the project needs is there and unharmed, and that it can make everything in it. If not, the project you have open stays as it is and the browser asks, for example *Night drive can't open whole: a file it needs is missing. Open what reads?* **Cancel** changes nothing. **Open what reads** opens everything else; what was left out is listed in the Events log, and saving then goes into a new project beside it (below). When the project is opened at start (Prefs > At start), nobody is asked: it doesn't open, and the screen says why. A project saved by an older MPFree, and not saved since, isn't checked first (its files have no list yet): the browser says so, and saving it once makes the list. Two things can only be found as the project opens, because they depend on how much memory is free: takes that don't fit in Audio track memory and kit sounds the sampler has no room for. Those are said as it opens, and the save goes beside it too.

**A project that opened with something left out is never saved over.** If anything of the project couldn't come in (a plugin that isn't on this stick, a sound, take or loop whose file is missing, a take that doesn't fit in Audio track memory), saving over it would leave that out, and only an earlier save would still have it. So **SHIFT + BROWSE** (or Save, or saving before switching off) saves your work into a new project beside it, named like it with a number (Night drive 2), and says why, for example *Saving as Night drive 2: Night drive opened without the effect clap:org.example.reverb on Master, so it stays as it was*. Night drive keeps everything it had, ready for when the plugin or file is back; you go on working in Night drive 2, and later saves go there. To save over the original anyway, use **Save as** and type its name (Save as offers the new name first).

If MPFree can't read the whole project (it was saved by a newer version of MPFree, or one of its files is damaged), it doesn't open it at all: the project you had open stays exactly as it was, still saved where it was, and the screen says why, for example *Didn't open Song: it was saved by a newer MPFree (project format 5; this one opens up to 4). Update MPFree to open it. The project you had open is unchanged*. MPFree is early in its development, so it doesn't open projects saved by an older MPFree either, and doesn't convert them: *Didn't open Song: it was saved by an older MPFree, which this one doesn't open (project format 3; this one opens 4). The project you had open is unchanged*. Once a project has an audio track in its track list, an older MPFree won't open it. If you open projects one after another faster than MPFree can put them in (a few milliseconds apart), the third waits: *the last two projects are still going in; try again in a moment*.

### Starting a new project

**New project**, at the bottom of the browser's left column under Trash (it is there whichever place the browser shows), starts again from nothing: an empty song with 8 tracks, every track on the kit, no loops, no audio tracks, the factory kits, every arp at its default, the MIDI, sync and CV settings at their defaults, the synth sounds and mixer as they are when MPFree starts, and the synth pads chromatic in C, tuned to A4 = 440 Hz. Your Prefs and the other things MPFree keeps for you stay as they are. Nothing is saved until you save, and the first save names a new project.

If you have changed anything since you last saved or opened, opening a project or New project asks first, so you never lose work by accident. Playing the pads, changing page or the Level doesn't count as a change.

### At start

MPFree opens the project you last saved or opened: once the sound is going, it opens as if you had opened it from the browser. If it has been deleted since, the Events log says so and you start with a new, empty project. To always start with a new project instead, set **At start** in [Prefs](../reference/prefs.md) to **New project**.

**After a crash**, MPFree starts again by itself a few seconds later, and this time it opens nothing: the new, empty project stays even if At start says **Last project**, and for a few seconds the hint line at the bottom of the screen says which project wasn't opened, and where your unsaved work is if autosave had any (the Events log keeps it). Open it from **BROWSE** when you're ready. This way a project that makes MPFree crash as it opens can't do it again at every start. Switching the MPC off and on starts as usual.

## The browser

{{shot: browser | The browser in Projects, with Night drive picked: its tempo and when it was saved on the right, with Earlier saves.}}

**BROWSE** opens the browser over the page you are on, and pressing it again goes back to that page. The folder at the right end of the top bar does the same, and the Pads tab's **Browse for** opens it to find a sound for the pad picked. It is a full file manager for MPFree's stick:

- **Places** on the left: Projects, Sets, Samples, Kits, Instruments, Patterns and Recordings, plus Recent, Favourites, the Trash, Factory (read-only kits) and the **SD card** in the MPC's slot (read only: see [The SD card](#the-sd-card)). **Sets** holds your set lists (see [Set lists](set-lists.md)). **Plugins** finds and installs MPC plugins (see [Getting plugins](plugins.md)).
- **The list** in the middle, folders first, in natural order (Kick 2 before Kick 10), with filters for audio, projects and kits. Each row has a **star** at its right end: tap it to add the item to Favourites, and again to take it out.
- **The selected item** on the right: a sound's length and waveform, a project's tempo and when it was saved, and buttons to Load (for a sound, **Load on A5**: the pad picked), Next pad hit, Audition, Rename, Copy, Cut and Delete. Under a sound, the kit's pads show what each pad holds; a finger on one picks it and plays it (see [Finding sounds](pads.md#finding-sounds-browse-for)).

**Opening things.** Tap a folder to open it. Tap anything else to pick it (a sound plays once), and tap it again to open it (a project) or load it (a sound, a kit, a preset, a MIDI file). Pushing the data wheel does the same as that second tap.

**MIDI files.** A `.mid` file loads as clips, from the armed track on: see [Bringing in a MIDI file](clips.md#bringing-in-a-midi-file). The MIDI file an export makes (in Recordings) loads back the same way.

**Kits.** A kit is kept as a folder with its sounds and a `kit.txt` file listing every pad and its settings, so it reads on any computer. Kits show with a green icon wherever they are. Loading one (its button names the selected track: **Load on Drums**) puts it in place of the kit the pads play (or in a new kit for the selected track, when other tracks play that kit); **New track with this kit** puts it on a new track of its own instead. **+** opens it to show its sounds. **Save kit** at the top of the **Kits** place saves the kit the pads play there, named after it. MPFree's own drum kits and its free kits of recorded drums ([Sampling](sampling.md)) are in **Kits > MPFree**.

A folder with thousands of sounds lists straight away; the sizes fill in a moment later.

Moving onto a sound plays it once, so you can audition as you scroll. Delete always asks first and moves things to the Trash, which you empty yourself (that asks again).

**Load on A5** puts a sound on the pad picked, named on the button: hit another pad and the button follows it. **Next pad hit** puts it on the next pad you hit. WAV, AIFF and FLAC files load, mono or stereo, at 8 to 192 kHz, whatever their name says. A file that was cut short, by a recorder that lost power say, loads the part it has. A FLAC file with a damaged part doesn't load at all, rather than play it as noise. A pad you exported from the Sample tab loads with its settings: load its sound, and the settings file beside it brings back its trim, loop, layers and slices (see [Library: saving the pad and its slices](sampling.md#library-saving-the-pad-and-its-slices)).

The data wheel moves through the list and its push opens or loads; the other buttons are in [Buttons and controls](../reference/controls.md#the-browser).

### The SD card

{{shot: browser-sd-card | The SD card place: an exFAT card's folders and a sound, read only, with Copy to put it in the library.}}

**SD card** in the places shows the card in the MPC's slot. MPFree only reads it: nothing on the card is changed, added or deleted, so it is safe with a card your MPC software uses too. Tap **SD card** and it opens in a few seconds; the top of the list says what the card is, such as **SD card · exFAT, 63.9 GB · read only**. MPFree reads FAT32 and exFAT cards.

Sounds on the card play, load and go on pads as anywhere else. To keep one in your library, pick it, tap **Copy**, then go to a place such as Samples and tap **Paste**. There is no Rename, Cut, Delete, New folder or star on the card.

If the card wasn't safely removed last time (the power went off, or it was pulled out while something was saving to it), the line under the list says **This card wasn't safely removed last time: check it on a computer**. It still opens to read. The card keeps that mark until a disk check clears it, so it may be from long ago. Before saving anything to it from your computer or the MPC software, check it with your computer's disk tool (on Windows, the drive's Properties > Tools > Check; on a Mac, Disk Utility > First Aid).

With no card in the slot, the place says so: put one in and tap **SD card** again. The card is put away 30 seconds after you close the browser, and opens again when you come back to it.

### Pickers

Where you need one kind of file, a narrower **picker** shows the same browser with only what fits. A picker only loads; renaming and deleting are in the full browser.

The Pads tab's **Browse for** opens the full browser, not a picker: see [Sampling](pads.md#finding-sounds-browse-for). Tapping the preset's name on the Synth tab opens a preset picker, where each turn of the wheel plays the next preset: see [Trying and saving presets](synths.md#trying-and-saving-presets).

### Your own synth presets

Synth presets you save (**Save preset** in the preset picker) go in **Instruments**, in a folder for each synth, and show up under **Yours** when you open that synth's presets. Loading one from **Instruments** in the full browser puts it on the Synth tab's sound, if it was made on the same synth. XY pad layouts you save (**Save to browser** in a layout's Setup) go in **Instruments › XY pad**; loading one puts it in the song as one of [your XY layouts](xy-layouts.md#using-a-layout-in-another-song).

## Getting files on and off the stick

Later versions will share MPFree's library over the network cable and by USB, so you can copy samples in and projects out from a computer. Until then, the files are on a Linux-format part of the stick that Windows and macOS can't open without extra software.

Pictures of the screen go there too: hold all four **PAD BANK** buttons together and MPFree saves what the screen shows in `files/screens/`, named by the date and time. So do reports from **System status**'s **Save a report** button, in `files/reports/` ([When something goes wrong](../reference/when-something-goes-wrong.md#the-system-status-page)).

> [!MPC]
> On the MPC, projects are saved to the internal drive, an SD card or a USB drive. MPFree saves only to its own stick, keeps every sample inside the project folder, and keeps the last ten versions of each project. MPFree doesn't open MPC projects (`.xpj`): bring your kits and patterns across as [Moving your beats from the MPC](moving-from-the-mpc.md) says.
