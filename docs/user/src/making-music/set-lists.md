<!--
status: changing
covers: rust/mpfree-browser-ui/src/sets.rs rust/mpfree-browser-ui/src/set_place.rs rust/mpfree-library/src/sets.rs rust/mpfree-library/src/preflight.rs rust/mpfree-app/src/set_checks.rs rust/mpfree-midi-ui/src/learn_section.rs rust/mpfree-time-ui/src/show.rs rust/mpfree-app/src/set_play.rs rust/mpfree-app/src/show.rs rust/mpfree-app/src/song_end.rs
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
| **Ends** | How this song ends when you Go to the next one: **Let it finish** (no new notes, and what's still ringing rings out), **Fade**, or **Cut**. |
| **Fade** | How long the fade takes, 1 to 30 seconds, when Ends is Fade. |
| **Needs** | The keyboards this song needs. Tap it to see the keyboards MPFree knows, then tap one to mark it **Needed** (the song isn't ready without it), **Nice to have** (it plays without it, with limits) or not needed. Tap **Done** to go back. |
| **Notes** | Anything you want to see on stage: the key, the cues, who starts. Tap the box to type, up to 240 characters. |

Tap the left or right half of a row to step it back or on. You can also press the wheel to go through Starts, Ends and Fade and turn the wheel to change the one outlined.

**Move up** and **Move down**, or **-** and **+**, move the picked song in the set. **Remove from set** asks again on the button before it takes the song out; the song itself stays in Projects. **Open in Make** opens the song as a normal project, so you can work on it.

## Checking a set

**Check set** opens the set's **check view**: every song in the set, each with how it went and its numbers. You can run it from the set's card, from the title line while you edit a set, and from the Show view's **MENU** while a set plays (the set plays on behind it).

{{shot: browser-sets-check | Check set on Friday at the Lantern: nine songs with their chips and numbers, and song 5, Glass harbour, picked: a sound is missing, so it isn't ready, and the Selected column says what every check found.}}

Each song shows one of these:

- **READY**: everything is there.
- **LIMITS**: it plays, but without something it would like, such as a keyboard marked Nice to have that isn't plugged in, or a stick that's nearly full.
- **NOT READY**: something it needs is missing, or it's too big to open. It can't go on stage until that's fixed. The row says why in red, for example "samples/strings.wav is missing".

Under each song's name are its numbers: how long it takes to open and how much memory it uses. Until you soundcheck it they're estimates from the song's files ("about 5 s · about 460 MB · not soundchecked yet"); after, they're what [Soundcheck](#soundcheck) measured ("opens in 3.1 s · 540 MB · CPU 71 % in Chorus 2"). The line at the top counts the songs ("7 ready, 1 with limits, 1 not ready") and says when the set was checked.

Turn the wheel or tap a song to see every check it ran, each with a mark (✓ fine, **i** a note, **!** limits, **×** not ready) and what it found:

| Check | What it looks at |
|---|---|
| **Files** | Every sound and recording the song uses is there and hasn't changed since it was saved. |
| **Parts** | Every part of the song reads. |
| **Plugins** | Each plugin the song uses is on this stick. One that isn't makes the song NOT READY, named by its file. |
| **Keyboards** | The keyboards the song **Needs** are plugged in. |
| **Outputs** | Where the song plays: the stereo mix on Main out, as the MPC One has it. |
| **Memory** | The memory the song uses at its peak, against what's free for songs. A song too big on its own is NOT READY. If it fits but not together with the next song, a note says so: that change will always have a pause while the next song loads. |
| **Stick** | Room on the stick for takes and recovery copies. Under 1 GB free is LIMITS; under 100 MB, NOT READY. |
| **Streams** | Audio track takes that play from the stick as the song plays. |
| **Soundcheck** | What the last soundcheck measured, or "not soundchecked yet". |

**Soundcheck this song** soundchecks just the picked song (see below). **Open in Make** opens the picked song as a normal project, so you can fix it. **Check every file**, at the top right, reads every file in full instead of only looking at its size and date. It takes longer, so it's for the day of the show. Tap **‹** by the title, or press **-**, to go back.

The set keeps what the check found, so the chips show on the set's card and in the Set editor next time too. Run it again before the show, after you've plugged everything in. A song saved by an older version of MPFree shows a note under Files until you save it again, so the check can look at its files.

## Soundcheck

**Soundcheck** plays the set through on this MPC and measures each song for real: how long it takes to open from cold, how much memory it uses, how hard its busiest part works the MPC, and how long the change into the next song takes. Do it once you've set up for the show, with everything plugged in.

Tap **Soundcheck** on the set's card (or **Soundcheck this song** in the check view, for one song and its change into the next). It asks first, in the right-hand column: what it's going to do and about how long it takes ("about 6 minutes" for nine songs), and **Sound**:

- **Out loud** (the first choice) plays through Main out and the headphones, so turn the PA down.
- **Muted** measures exactly the same with nothing coming out.

If the song open now has changes you haven't saved, it says so: they're saved first, and that song opens again at the end. A song that was never saved has to be saved before you can start. Tap **Start soundcheck**, or **Cancel**.

{{shot: browser-sets-soundcheck | Soundcheck on Friday at the Lantern: songs 1 and 2 are done and show what was measured, song 3, Undertow, is playing its busiest part, Chorus 2, at bar 41, with the CPU now and at its worst, and the rest wait.}}

For each song, in turn:

1. **Opened cold**: the song opens just as it would on stage, timed, with its memory at its peak.
2. **Its busiest part**: the step of the song's list with the most playing at once (audio tracks and plugins count double) plays for up to 16 bars. A song with no song list plays its first 16 bars. The box shows the CPU (now, and a mark at its worst), the longest a clip waited for its audio from the stick, stream misses and xruns.
3. **The change into the next song**: the song ends the way its **Ends** says, then the next song opens cold, and the pause is timed. The last song has no change.

The song being checked is outlined with a **SOUNDCHECK** chip, the songs done show their new numbers, and the rest say "waiting". The line at the top says how far it's got and about how long is left.

- **Skip this song** stops its part where it is and goes on to the change into the next song.
- **Stop soundcheck** (or **STOP**) stops it. Everything measured so far is kept.

At the end it says how it went: how many songs and how long it took, the busiest song, any stream misses or xruns (that song may crackle there), and the longest pause between songs. The sound comes back, and the song you had open before opens again. Tap **Done**.

What it measured goes into each song's line in the set, so the check uses real numbers from now on, and the Show view's COLD LOAD card says how long the silence will be while the next song opens ("about 4 s of silence while it opens"). If you change a song after its soundcheck, the check says so: soundcheck it again for its numbers now.

While a set is playing, Soundcheck is faint: close the set first. While Soundcheck runs, Play set waits for it.

## Playing a set

Pick a set and tap **Play set** on its card. If the song open has changes you haven't saved, it asks first. MPFree switches to Perform, Arrange shows the **Show** view, and the set's first song opens. Nothing asks you anything from here on: a song that can't open whole doesn't open, and the screen says why.

{{shot: arrange-show | The Show view: Friday at the Lantern's nine songs on the left, Night drive playing part 4 of its song list, and Paper moons armed as the next song.}}

- **The set** runs down the left: songs already played are grey, the one playing says **NOW**, the one armed to go next says **NEXT** with what Go will do, and a red dot marks a song that isn't ready.
- **The song playing** is in the middle: its name, whether it's **PLAYING** or **LOADED** (open, waiting for PLAY), its tempo, the part of its song list playing and the time left, and its parts as tiles.
- **The next-song card** says which song is armed and what Go does with it. Under it are the song's **notes** from the set, the keyboards you have and whether they're plugged in (one this song needs and isn't plugged in is red; with [patches](patches.md#on-the-show-view), the patch each one plays and a strip of its zones), and the **click** switch.

### Moving to the next song

Changing song takes two steps, so nothing changes by accident: **arm** the next song, then **Go**.

1. Arm a song: press **Next song** on a footswitch (or **Previous song**), turn the wheel or press **-** / **+**, or tap the song in the set. **Back to the set order** arms the song after the one playing. A tap on the song playing or the one armed does nothing, so a double tap is safe.
2. Go: press **Go** on the card or a footswitch, press the wheel, or press **Next song** again.

With the transport stopped, Next song still needs two presses unless you turn on **Quick advance** in [Prefs](../reference/prefs.md). **Cancel next** drops the armed song and leaves the music alone. A song that isn't ready can be armed, so you can read why on its card, but Go does nothing.

When you Go, the song playing ends the way its own **Ends** says:

- **Let it finish**: no new notes start, notes you hold sound until you let go, and what's ringing rings out. Once it's quiet (or after 10 seconds at most) it fades out over a moment. The pill counts down **FINISHING**. Press Go again to go now.
- **Fade**: it fades out over the song's Fade time.
- **Cut**: it stops at once.

If you changed the song while it played (a new take, a knob turned), its changes and takes are kept in a **recovery copy** beside it before the next song opens: the card says **SAVING TAKES**. The song in the set stays as you saved it. Then the next song opens (**OPENING**) and starts as its **Starts** says. Autosave waits while a song changes, and saving waits too.

The next song opens when you Go, so there is a short pause between songs while it loads. Loading the next song while you play comes later. The Events log says how long each change took, from Go to the next song in (*Glass Garden is in (Playing), 4.80 s after Go*), and how long its open took, part by part (see [Opening a project](saving.md#opening-a-project)).

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
| **MENU** | **Open the Set editor**, **Check the set** (opens the check view over the Show view while the set plays on), **Close the set** (the song playing stays open as a normal project and the Show view goes) |

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
