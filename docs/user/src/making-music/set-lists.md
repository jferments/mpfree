<!--
status: changing
covers: rust/mpfree-browser-ui/src/sets.rs rust/mpfree-browser-ui/src/set_place.rs rust/mpfree-library/src/sets.rs rust/mpfree-midi-ui/src/learn_section.rs rust/mpfree-time-ui/src/show.rs rust/mpfree-app/src/set_play.rs rust/mpfree-app/src/show.rs rust/mpfree-app/src/song_end.rs
checked: 2026-11-22 cc300da5
-->

# Set lists

A **set** is a list of songs to play in order, for a gig or a rehearsal. Each song is one of your projects. A set only points at its songs and never copies them, so a song can be in many sets, and a change you save to the song is in every set that has it. If you rename a song or move it to another folder, the set still finds it.

You make and change sets in BROWSE's **Sets** place, and play them on Arrange's **Show** view: it shows the song playing, what comes next and whether it's ready, and moves through the set with a footswitch, the wheel or a tap.

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

## Playing a set

Pick a set and tap **Play set** on its card. If the song open has changes you haven't saved, it asks first. MPFree switches to Perform, Arrange shows the **Show** view, and the set's first song opens. Nothing asks you anything from here on: a song that can't open whole doesn't open, and the screen says why.

{{shot: arrange-show | The Show view: Friday at the Lantern's nine songs on the left, Night drive playing part 4 of its song list, and Paper moons armed as the next song.}}

- **The set** runs down the left: songs already played are grey, the one playing says **NOW**, the one armed to go next says **NEXT** with what Go will do, and a red dot marks a song that isn't ready.
- **The song playing** is in the middle: its name, whether it's **PLAYING** or **LOADED** (open, waiting for PLAY), its tempo, the part of its song list playing and the time left, and its parts as tiles.
- **The next-song card** says which song is armed and what Go does with it. Under it are the song's **notes** from the set, the keyboards you have and whether they're plugged in (one this song needs and isn't plugged in is red), and the **click** switch.

### Moving to the next song

Changing song takes two steps, so nothing changes by accident: **arm** the next song, then **Go**.

1. Arm a song: press **Next song** on a footswitch (or **Previous song**), turn the wheel or press **-** / **+**, or tap the song in the set. **Back to the set order** arms the song after the one playing. A tap on the song playing or the one armed does nothing, so a double tap is safe.
2. Go: press **Go** on the card or a footswitch, press the wheel, or press **Next song** again.

With the transport stopped, Next song still needs two presses unless you turn on **Quick advance** in [Prefs](../reference/prefs.md). **Cancel next** drops the armed song and leaves the music alone. A song that isn't ready can be armed, so you can read why on its card, but Go does nothing.

When you Go, the song playing ends the way the next song's **Ends** says:

- **Let it finish**: no new notes start, notes you hold sound until you let go, and what's ringing rings out. Once it's quiet (or after 10 seconds at most) it fades out over a moment. The pill counts down **FINISHING**. Press Go again to go now.
- **Fade**: it fades out over the song's Fade time.
- **Cut**: it stops at once.

If you changed the song while it played (a new take, a knob turned), its changes and takes are kept in a **recovery copy** beside it before the next song opens: the card says **SAVING TAKES**. The song in the set stays as you saved it. Then the next song opens (**OPENING**) and starts as its **Starts** says. Autosave waits while a song changes, and saving waits too.

The next song opens when you Go, so there is a short pause between songs while it loads. Loading the next song while you play comes later.

### When a song doesn't open

If the next song can't open (a file is missing, or it was deleted), nothing plays and the card turns red with what went wrong. **Try again** opens it again; **Reopen** goes back to the song you were playing, as you left it. You can also tap another song to arm it, then Try again.

### After the show

Songs that changed on stage say **changed on stage** in the Set editor. Pick one: its card lists the recovery copies under **Changed on stage**. **Keep the changes** puts the newest copy into the song (the song keeps its old version, so you can go back to it) and moves the older copies to the Trash. **Throw them away** moves the copies to the Trash and leaves the song as it was.

### The Show view's controls

| Control | Does |
|---|---|
| Tap a song | Arms it as the next song |
| **Go** | Changes to the armed song; during Let it finish, goes now |
| **Cancel next** | Drops the armed song |
| **Back to the set order** | Arms the song after the one playing |
| Wheel, **-** / **+** | Arms the song after or before |
| Wheel push | Go |
| **Clips**, **Song**, **List** | Arrange's other views; **Show** on their title lines comes back |
| **MAIN** | Show, Clips, Song, List and round (from another page in Perform, the Show view) |
| **MENU** | **Open the Set editor**, **Check the set** (the quick check on every song), **Close the set** (the song playing stays open as a normal project and the Show view goes) |

Opening another project from BROWSE, or a new project, also closes the set.

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
