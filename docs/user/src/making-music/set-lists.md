<!--
status: changing
covers: rust/mpfree-browser-ui/src/sets.rs rust/mpfree-browser-ui/src/set_place.rs rust/mpfree-library/src/sets.rs rust/mpfree-midi-ui/src/learn_section.rs
checked: 2026-11-22 befc4266
-->

# Set lists

A **set** is a list of songs to play in order, for a gig or a rehearsal. Each song is one of your projects. A set only points at its songs and never copies them, so a song can be in many sets, and a change you save to the song is in every set that has it. If you rename a song or move it to another folder, the set still finds it.

You make and change sets in BROWSE's **Sets** place. Playing a set from start to finish, with the next song loading while you play, comes with the Show view, which is still being built: for now **Play set** only says so.

{{shot: browser-sets | The Sets place: three sets, each with how many songs it has and how its last check went; Friday at the Lantern picked, with the song that isn't ready and the one that plays with limits.}}

## Making a set

1. Press **BROWSE** and pick **Sets**, under Projects.
2. Tap **New set**, type its name (up to 32 characters) and tap **Done**. The new set opens, empty.
3. Tap **Add songs**. Your projects are listed, each with a **+** at its right. Tap **+** on each song you want, in the order you'll play them. Each one goes after the song picked, or at the end. A song can be in a set more than once.
4. Tap **Done**, or press **-**, to go back to the set.

The set saves itself after every change, so there's nothing to remember. Tap **‹** by the set's name to go back to your sets.

## Setting up each song

Tap a song in the set to pick it. Its card on the right sets how it plays in the show:

{{shot: browser-set-song | Editing Friday at the Lantern with song 4, Night drive, picked: how it starts and ends, the keyboard it needs and its notes.}}

| Row | What it does |
|---|---|
| **Starts** | **Stopped** (it waits for PLAY), **Playing** (it plays as soon as it's in), or **Count-in then play**. |
| **Ends** | How the song before it ends when this one takes over: **Let it finish** (no new notes, and what's still ringing rings out), **Fade**, or **Cut**. |
| **Fade** | How long the fade takes, 1 to 30 seconds, when Ends is Fade. |
| **Needs** | The keyboards this song needs. Tap it to see the keyboards MPFree knows, then tap one to mark it **Needed** (the song isn't ready without it), **Nice to have** (it plays without it, with limits) or not needed. Tap **Done** to go back. |
| **Notes** | Anything you want to see on stage: the key, the cues, who starts. Tap the box to type, up to 240 characters. |

Tap the left or right half of a row to step it back or on. You can also press the wheel to go through Starts, Ends and Fade and turn the wheel to change the one outlined.

**Move up** and **Move down**, or **-** and **+**, move the picked song in the set. **Remove from set** asks again on the button before it takes the song out; the song itself stays in Projects. **Open in Make** opens the song as a normal project, so you can work on it.

## Checking a set

**Check set** looks at every song: that it opens, that every sound and recording it uses is there and hasn't changed since it was saved, and that the keyboards it needs are plugged in. Each song then shows how it went:

- **READY**: everything is there.
- **LIMITS**: it plays, but without something it would like, such as a keyboard marked Nice to have that isn't plugged in.
- **NOT READY**: something it needs is missing. The set's card says what, for example "5 Glass harbour: the file samples/strings.wav is missing."
- **NOT CHECKED**: it hasn't been checked yet, or it was saved by an older version of MPFree. Saving the song again lets the check look at its files.

The set keeps what the check found, so you see it next time too. Run it again before the show, after you've plugged everything in. **Check every file**, on the set's card, reads every file in full instead of only looking at its size and date. It takes longer, so it's for the day of the show.

## Footswitches

Footswitches or buttons on a MIDI controller can arm the next song, go, cancel, stop everything and turn the click on and off. Learn them on the Show card in **Learn**: see [Footswitches for a set](midi-and-cv.md#footswitches-for-a-set).

## Renaming, copying and deleting a set

Pick a set in the Sets place and use the buttons on its card. **Copy** makes a copy beside it ("Friday 2") that you can change for another night. **Delete** asks first, then moves the set to the Trash; its songs stay in Projects.

| Control in the Sets place | Does |
|---|---|
| Wheel | Picks a set, or a song in the set |
| Wheel push | Edit set (in a set: goes through Starts, Ends and Fade; adding songs: adds the one picked) |
| **-** / **+** | In a set: move the picked song up or down. Adding songs: **-** is Done, **+** adds |
| **ERASE** | Delete the set (asks first), or in a set, Remove from set (asks again) |
