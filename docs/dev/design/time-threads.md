<!--
covers: rust/mpfree-app/src/time.rs rust/mpfree-play/src/view.rs rust/mpfree-play/src/player.rs
checked: 2026-11-12 ea72d6c
-->

# The time engine on two threads (plan)

**Status:** plan, 2026-11-12. Step 1 is built (a compile checks only what it plays, see [playback.md](playback.md#in-the-pad-player-timehost)), and so is step 2a (recorded changes made alike to the view and the project, and the catch-up); the split itself is not. The app thread, which owns `main.rs` and will build the control thread ([concurrency.md](concurrency.md), plan step 2), agreed to the shape on 12 Nov (07:20Z) with three asks, which are rules 1 to 3 below.

## Why

The pad player runs the whole time engine (`TimeHost` in `rust/mpfree-app/src/time.rs`) in its audio loop. Recording is cheap there, because it writes notes into the play view in place. Every other edit still runs on the audio thread, inside a period:

- **A compile** of the `PlayView` on every launch, stop, cue that starts, follow action, undo, redo, arm, capture, mute, song edit and note editor edit. It allocates, and it costs time proportional to the notes playing. After the first step, a project of 128 clips of 512 notes with 16 playing still took 159 µs on the workstation. The MPC One's Cortex-A17 is several times slower, against a 128-frame period of 2.9 ms whose busiest round 7 period already used 44 %.
- **Song edits** clone the whole project (`TimeAction::ScenesToSong` and its kin keep a copy to put back on an error).
- **Capture** builds its clip and takes there.

The time this takes is unbounded, because it grows with the project. On the MPC it can make a launch in a big project cost a period's deadline, which is a click.

## The split

`TimeHost` becomes two halves with one facade, so `main.rs` doesn't change until the control thread exists:

| Half | Owns | Runs |
|---|---|---|
| **Control** (`TimeControl`) | The project, the takes, the launches and cues, scenes and follow actions (`Perform`), live to song's keep, the note editor, the morph's settings, the selection, the armed track, capture's keep, the steps for the Play page, every compile, and the `TimeState` for the screens | At its own pace: on each screen or MIDI event, and on a timer of a few ms (the app thread's control thread). Never once per period. |
| **Audio** (`TimeAudio`) | The transport and its block, the player, the recorder, the play view, the capture history and the move log, the click, the pad hits and the locks and lanes going out | Once per period, on the audio thread, with no allocation and no free |

They talk through two bounded queues (`Link`), one each way.

### Rules

1. **The queue.** `Link<T>` wraps a bounded `std::sync::mpsc::sync_channel`, the type `main.rs` already uses between the screen and the audio thread. The two halves only `try_send` and `try_recv`, so nothing blocks. A full queue is counted and logged, never a silent drop (the fallback rule). If the device numbers call for it, an SPSC ring goes in behind `Link` in one place.
2. **The audio half allocates and frees nothing.** Messages to it are fixed-size, except a compiled view, which comes boxed. The audio half swaps the box in at the start of a period and sends the old one back, and the control half drops it. Messages from it carry no `Vec` or `String`.
3. **The control half keeps up at its own pace.** It may run a few ms late, so nothing it does may need to land within a period. The two places where time matters are below.

### Messages

**To audio:** a new view (with what it includes, see below); a transport command; FILL on or off at an offset; the metronome; the tracks' arp settings (through the arp, destination) as a fixed array; capture's "kept" and "forget kept"; a request for the capture phrase.

**To control:** each cue that started (`Started`); each note or lock that was recorded or erased, and each finished part of a gesture (the recorder's points copied into fixed-size chunks); the transport's info when it changes (state, position at most every few ms, meter, loop, tempo); capture's pending phrase when it changes, and the phrase in fixed-size chunks when asked; the pad hits for the Play page's steps; counts of anything dropped.

### Where time matters

**Recording.** The audio half keeps writing recorded and erased notes into its view in place, as now, and numbers each one. The control half applies them to the project and the takes in order. Each view it compiles says the last number it includes. When the audio half swaps in a view older than its last number, it first writes the newer ones into it in place, from a ring of the last `RECORD_ROOM` items it keeps for this. If that ring has run over, it asks for a compile, counts it and logs it, and the old view plays until the new one comes. Nothing recorded is lost or doubled. A test records at every period size with the control half running late by random amounts, and checks that every view the audio half plays equals a fresh compile of the project as it will be.

**Cues and follow actions.** A launch made while the music plays already waits for the next bar line, and the player carries a switch itself until a view with it arrives. Each view says how many switches it includes. A view compiled before a switch that has since happened gets the switch put in by the audio half before it plays, the same way as recorded notes. Follow actions are cued when their clip starts, a bar or more before they are due, so the control half has that long to compile. A follow action due within the control half's slack (a 1-bar clip with a 1-bar follow, at a fast tempo) is cued at once from the `Started` that set it off. A test runs every follow action at 300 BPM with the control half late by up to 20 ms, and checks that each lands on its bar line.

### What stays the same

- The player, the recorder and the view (`mpfree-play`) don't change, apart from the numbering and the in-place catch-up.
- The facade calls the control half once a period on the audio thread until the control thread exists. That changes nothing that plays, and the existing tests keep passing unchanged. Then the app thread moves the control half onto its thread.

## Steps

1. Done: a compile checks only what it plays.
2. In two parts:
   - **2a, built:** recorded changes numbered (`mpfree_play::Rec`) and made to the view and the project alike (`PlayView::record`, `Takes::record`), views that say what they include (`Includes`), the switch count, and the catch-up (`Recent`), with their tests in `mpfree-play` (`tests/catch_up.rs`, and the late view that cues the same clip again in `tests/cue.rs`). The pad player already makes every recorded change this way and sets what each view includes, so every time test runs through it. See [playback.md](playback.md#compiling-playview).
   - **2b:** `Link`, the messages and the two halves behind the facade, still on one thread, and the tests above. The existing time tests pass unchanged.
3. The app thread runs the control half on its control thread. Round tests on the MPC compare the audio thread's worst period with launches, undo and capture in a big project, before and after.
4. Song edits and capture lose their project clones, since the control half can afford them.

Step 2 waits until the open time engine PRs (#206, #209, #211, #212) have merged, because it moves most of `time.rs`.
