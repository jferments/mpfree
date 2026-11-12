<!--
status: changing
covers: rust/mpfree-store rust/mpfree-library rust/mpfree-browser-ui rust/mpfree-app/src/app/saved.rs
checked: 2026-11-12 3c5a580
-->

# Saving your work

MPFree keeps everything you save on its own USB stick. The MPC's internal storage and its SD card are never written, so the factory software and your MPC projects are exactly as you left them.

> [!IMPORTANT]
> In the current test build, **your music is not saved yet**. Patterns, clips, loops, samples you chop and synth edits last until you switch off. Saving projects comes with the browser, described below. Settings and arpeggiator patterns are already kept.

## What is kept today

These are saved on the stick as soon as you change them, and are back the next time MPFree starts:

| What | Where you set it |
|---|---|
| Your Prefs: pad velocity curve, how Latch keeps pads, the audio buffer, developer tools, screen pad velocity | [Prefs](../reference/prefs.md) |
| The buttons MPFree has learned | Prefs > Map controls, or the button guide |
| Arpeggiator patterns you save | The pattern workspace's **Save** (see [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md#the-pattern-workspace)) |

The line input is never kept on: it is off every time MPFree starts, so a noisy input you forgot about is never suddenly loud.

Saving is safe to interrupt. Each file is written whole to the side and only then swapped in, so if the power goes off in the middle of a save, the previous version is still there.

## Coming: projects and the browser

The next big step is saving and loading your music. It is designed and built, and is being connected to the rest of MPFree.

### Projects

A **project** is everything in one song: its tempo, patterns and clips, loops, kits and samples, synth sounds, mixer and effects.

- **SHIFT + BROWSE** saves the project from any page. The first save needs no typing: the project is named after the date and time, and you can rename it later.
- Each project is one folder that holds a copy of every sample it uses, so a project never goes missing a sound because a drive or a folder has gone.
- The last ten saves of each project are kept, so you can go back to an earlier version.
- The files are plain and readable on any computer.

### The browser

**BROWSE** opens the browser over the page you are on. It is a full file manager for MPFree's stick:

- **Places** on the left: Projects, Samples, Kits, Instruments, Patterns and Recordings, plus Recent, Favourites, the Trash, and Factory (read-only kits).
- **The list** in the middle, folders first, in natural order (Kick 2 before Kick 10), with filters for audio, projects and kits.
- **The selected item** on the right: a sound's length and waveform, a project's tempo and when it was saved, and buttons to Load, Load to pad, Audition, Favourite, Rename, Copy, Cut and Delete.

Moving onto a sound plays it once, so you can audition as you scroll. Delete always asks first and moves things to the Trash, which you empty yourself.

**Load** puts a sound on the pad you are editing, and **Load to pad** on the next pad you hit. WAV, AIFF and FLAC files load, mono or stereo, at 8 to 192 kHz, whatever their name says. A file that was cut short, by a recorder that lost power say, loads the part it has. A FLAC file with a damaged part doesn't load at all, rather than play it as noise.

The data wheel moves through the list, **SHIFT** and the wheel through the places, and pushing the wheel opens or loads. **–** and **+** go up a folder and into one.

### Pickers

Where you need one kind of file, a narrower **picker** opens in the same place: a preset picker on the Synth tab (each turn of the wheel lets you play the next preset on the pads before you choose it), a sample picker on the Sample tab, and a project picker for Open. A picker only loads; renaming and deleting are in the full browser.

### Your own synth presets

Synth presets you save go in **Instruments**, in a folder for each synth, and show up in that synth's preset list and picker.

## Getting files on and off the stick

Later versions will share MPFree's library over the network cable and by USB, so you can copy samples in and projects out from a computer. Until then, the files are on a Linux-format part of the stick that Windows and macOS can't open without extra software.

> [!MPC]
> On the MPC, projects are saved to the internal drive, an SD card or a USB drive, with the samples in a folder beside the project file. MPFree saves only to its own stick, keeps every sample inside the project folder, and keeps the last ten versions of each project. MPFree can't open MPC projects (`.xpj`) yet; a tool to bring them across is being looked at.
