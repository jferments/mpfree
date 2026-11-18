<!--
covers: rust/mpfree-app/src/time.rs rust/mpfree-app/src/time/link.rs rust/mpfree-app/src/time/audio.rs rust/mpfree-app/src/time/control.rs rust/mpfree-play/src/view.rs rust/mpfree-play/src/player.rs rust/mpfree-play/src/recorded.rs
checked: 2026-11-17 4ad54d3
-->

# The time engine on two threads

**Status:** steps 1 and 2 built, 2026-11-12. The time engine is two halves that speak only through two queues, with a facade (`TimeHost`) that still runs both on the audio thread, so `main.rs` has not changed. Step 3, the control half on the app thread's control thread, waits: live test 2 (13 Nov) found no slow period on the audio thread, so it is done only when a device run's `control: slow period` lines name MIDI in or the screen's actions ([concurrency.md](concurrency.md), plan step 2). Since 17 Nov a step edit no longer compiles (sequencer S1, [step edits in place](#step-edits-in-place)), and the status line gives what compiles and step edits cost, so the `step-speed` device check (Gate 0) measures both on the MPC. The app thread, which owns `main.rs` and the control thread ([concurrency.md](concurrency.md), plan step 2), agreed to the shape on 12 Nov (07:20Z) with three asks, which are rules 1 to 3 below. Nothing here has run on the MPC yet.

## Why

The pad player ran the whole time engine (`TimeHost` in `rust/mpfree-app/src/time.rs`) in its audio loop. Recording is cheap there, because it writes notes into the play view in place. Every other edit ran on the audio thread, inside a period:

- **A compile** of the `PlayView` on every launch, stop, cue that starts, follow action, undo, redo, arm, capture, mute, song edit and note editor edit. It allocates, and it costs time proportional to the notes playing. After the first step, a project of 128 clips of 512 notes with 16 playing still took 159 µs on the workstation. The MPC One's Cortex-A17 is several times slower, against a 128-frame period of 2.9 ms whose busiest round 7 period already used 44 %.
- **Song edits** clone the whole project (`TimeAction::ScenesToSong` and its kin keep a copy to put back on an error).
- **Capture** builds its clip and takes there.

The time this takes is unbounded, because it grows with the project. On the MPC it can make a launch in a big project cost a period's deadline, which is a click.

## The split

`TimeHost` is two halves with one facade, in `rust/mpfree-app/src/time/`:

| Half | Owns | Runs |
|---|---|---|
| **Control** (`TimeControl`, `control.rs`) | The project, the takes, the launches and cues, scenes and follow actions (`Perform`), live to song, the note editor, the morph's settings, the selection, the armed track, CAPTURE's history and the move log, the steps for the Play page, every compile, and the `TimeState` for the screens | At its own pace: on each screen or MIDI event, and on a timer of a few ms (the app thread's control thread, step 3). Never needed once per period. |
| **Audio** (`TimeAudio`, `audio.rs`) | The transport and its block, the follower of another device's clock (`follow.rs`, see [midi-clock.md](midi-clock.md#following-the-time-engine)), the player, the recorder, the view it plays, the last recorded changes (`Recent`), the click, the pad hits and the locks and lanes going out | Once per period, on the audio thread, with no allocation and no free |

They talk through two bounded queues (`link.rs`), one each way.

CAPTURE's history and the move log moved to the control half, not the audio half as first planned. The audio half sends each pad note and gesture it takes, with its offset, and each period's spans; the control half feeds them to the history and the log against a copy of the period's spans (`Block::start_copy`, `Block::copy_span`), so they keep the same phrase as before. Keeping a capture reads the whole history, so this way no phrase has to cross the queue in pieces.

### Rules

1. **The queue.** `link::link` makes a bounded `std::sync::mpsc::sync_channel`, the type `main.rs` already uses between the screen and the audio thread, with all its room. The two halves only `try_send` and `try_recv`, so nothing blocks. A full queue hands the message back and counts it, never a silent drop (the fallback rule). If the device numbers call for it, an SPSC ring goes in behind `link` in one place.
2. **The audio half allocates and frees nothing.** Every message is a fixed size, except a compiled view, which comes boxed. The audio half swaps the box in at the start of a period and sends the old one back, and the control half drops it. While the queue back is full, up to 8 views wait in the audio half; past that one is kept rather than freed there (counted; it should never happen). Messages from it carry no `Vec` or `String`.
3. **The control half keeps up at its own pace.** It may run a few ms late, so nothing it does may need to land within a period. The two places where time matters are below.

### Messages

**To audio** (`ToAudio`, 256 slots): a new view, which says what it includes; a transport command; the tracks' settings when they change: how many tracks and which is armed (`ToAudio::Tracks`), and, one message per track that changed, its "Clips through the arp", destination and "Record pad pressure" (`ToAudio::Track`); a track's MIDI effects, one place a message (`ToAudio::NoteFx`, and `ToAudio::NoteArp` for where its arp sits), and, the first time the track has one, a chain to run them in, made here and boxed (`ToAudio::Chain`: about 400 KB, it stays with the track's arp; one the audio half has no use for comes back to be freed, `ToControl::Chain`); and a step edit's changes, one a message (`ToAudio::Edit`, below). Both halves keep a table of all 128 tracks (`link::Tracks`), but a message carries only what changed, so it stays small; one that finds the queue full goes at the next poll.

**To control** (`ToControl`, 8192 slots, a busy second of periods), in the order things happened: each pad note played, each pad pressure value that changed (both for CAPTURE's history) and each gesture taken, with its offset; each cue that started (`Started`); each recorded change (`Rec`) with its number, after its pressure or gesture points in fixed chunks; a request to compile again; each of the period's spans; the clock follower's counts when they change; the period's end, with the transport as it left it and the audio half's counts; and each view to free.

The pad player's audio loop calls the audio half directly for what it hands the time engine each period: the pads, their pressure, the knobs, the gestures, FILL, the tempo it follows, the followed port's clock bytes, and the metronome. It can also queue a transport command on an exact frame of the coming period (`TimeHost::command_at`), where a command through the queue waits for the next period: the export stops the song on its range's last frame with it ([export.md](export.md#the-apps-part)).

### Where time matters

**Recording.** The audio half writes recorded and erased notes, locks and gestures into its view in place, numbers each one and keeps the last 1024 with their points (`Recent`), then sends it on. The control half makes them to the project and the takes in the same order. Each view it compiles says the last number it includes. When the audio half swaps in a view older than its last number, it first makes the newer ones to it in place. If the ring has run over, or the view has no room, it plays on with the view it has (which holds everything), counts it, and asks for a compile. Nothing recorded is lost or doubled.

**Cues and follow actions.** A launch made while the music plays already waits for the next bar line, and the player carries a switch itself until a view with it arrives. Each view says how many switches it includes. A view compiled before a switch that has since happened gets the switch put in by the audio half before it plays, the same way as recorded notes. Follow actions are cued for their bar line as soon as their clip or scene starts, a bar or more before they are due (`Perform::due` cues one within the bar before its point), so the control half has that long to compile.

### Step edits in place

A tap on a step, a Q-Link turned on a held step and a lock change one or two notes or locks of the clip playing. Compiling the whole view for each (every detent of a turn) cost the audio thread a compile per edit while both halves run on it, and a compile's time grows with the notes playing. So `TimeControl::step_action` makes the edit to the project and the takes as before, and then, when it can, sends its changes instead of compiling (`TimeControl::in_place`): `ToAudio::Edit { track, slot, change }`, one per change, with `link::InPlace` naming it (a note or lock added or erased), in the order the clip took them (`Clip::apply`: the notes erased, the locks erased, the notes added, the locks added). The audio half makes each to its view in place with `PlayView::add_note`, `remove_note`, `add_lock` and `remove_lock`, the calls recording uses, so the view holds them where a compile would, stamp and all. That allocates and frees nothing: the room is what each compile sets aside for recording (`RECORD_ROOM`, 1024 notes and 1024 locks in the armed track's clips).

They need no number and no catch-up, unlike recorded changes: they go through the same queue as the views, so a view sent before them was compiled without them and one sent after them has them. A change the view cannot make (no room left, or what it erases is not there) is counted (`Counts::missed`, "step edits compiled again" on the status line, which should stay 0) and asks for one compile until a new view comes; the compile brings it and every change after it.

It compiles, as before, when the edit makes a new pattern or launches the pattern on a track playing nothing, changes the clip's length, pressure or moves, has more than `MAX_IN_PLACE` (64) changes or would cost the audio half more than 8192 notes moved (a big erase or copy), in song mode (the song's placements play clips the view would also need changing), while recording or counting in (the audio half edits the view itself), while a view waits to be sent, on a track other than the armed one (no room set aside), or when the queue fills partway (the changes sent are made, then the compiled view takes their place). An edit to a clip neither playing nor cued sends nothing. UNDO and REDO of a step edit still compile.

**What it costs, on the status line.** The time engine's part of the 30 s status line ends with what compiles and step edits have cost the thread the control half runs on since the start (`control::Costs`): "compiles: 12 (slowest 0.21 ms, 96 notes and locks); step edits: 40 in place (slowest 0.02 ms), 3 compiled (slowest 0.25 ms)". A step edit is timed by the facade from the action to the audio half having made it (`TimeHost::step_action`), so it is what a tap or a turn costs the audio thread today. The `step-speed` device check reads it after a run of taps and a lock turn during playback: it passes when the edits went in place, each within a tenth of a 128-frame period (`IN_PLACE_MS`, 0.29 ms), and no `control: slow period` line named the pads or the screen. Its numbers, with the slowest compile and the notes it played, are Gate 0 of the sequencer roadmap: whether step 3 has to come before more step work.

### The facade

`TimeHost` keeps the API `main.rs` used. A call on the control half (an action, a step edit, a capture, a mute) is followed by the audio half taking what it sent, and a period by the control half taking the period's news and the audio half taking any view it compiled (a cue that started, a follow action, a recording with no room left). So on one thread nothing plays differently, and the old time tests pass with only their field paths changed (`h.control.project`, `h.audio.view`).

**Starting on a note.** `TimeHost::start_at(at, bpm, take)` starts the song on frame `at` for the looper's Starts: When I play ([looper.md](looper.md#when-i-play-starting-on-your-note)), so bar 1 is the note. It is the audio half's (`TimeAudio::start_at`), on the transport's own queue at `at`: back to the start of the bar the song stopped in (`Locate`), `SetTempo(bpm)` if given (before `Play`, so the whole song takes it: the tempo from the first loop, step 4c), then `Play`, with no count-in. A `take` ready (adding or replacing, as for `take_waits`) records from the same frame the way a followed clock's start does: `ToggleOverdub` if the mode differs, `Play`, then `Record`, a punch-in; the control half readies the clip it records into first (`take_started_on_note`), and with no clip to record into the song starts without the take (so no punch-in needs taking back) and only plays. Refused while the song plays or counts in, while another device's clock leads (its tempo arriving, or PLAY waiting for its Start), or when the transport's queue hasn't room for every command (`Transport::queue_room`: all go in or none, so a refused start leaves no `Locate` or tempo behind), and counted (`starts_refused`); the reason comes back for the screen (`TimeAudio::start_refusal` asks without counting). `TimeAudio` keeps the tempo last asked for (a `SetTempo` or `start_at`'s) with the frame it is due on until the transport plays at it (a change while rolling waits for the next 1/16 line; stopped, once its period has run), and `follow_tempo` compares the app's tempo with that, so a tempo on its way is asked for once.

`TimeHost::split` hands over the two halves, already linked, for step 3: the audio loop keeps `TimeAudio` and calls it as the facade does (the pads' input, `advance`, `dress_arps` and `run`, then the hits, locks and click), and the control thread keeps `TimeControl` and calls `poll` every few ms and after each screen or MIDI event. The queue to the audio half holds 256 messages and the one back 8192 (`link::TO_AUDIO`, `link::TO_CONTROL`). Neither half ever waits: a full queue hands the message back to its sender, which counts it (`Counts::lost` for the audio half's, `TimeControl::lost` for the control half's; a view waits and is sent again).

What passing between the halves lost goes in the status line when it is not 0 (`TimeHost::describe`): messages lost to a full queue (either way, and spans past a copy's room), late views compiled again, views kept unfreed, and transport commands the transport's queue refused.

### Tests

- `recording_with_the_control_half_late_loses_and_doubles_nothing` (time.rs) records the pads, their pressure and a knob into a looping clip at periods of 16 to 4096 frames, with the control half waking 0 to 20 ms apart at random and half the times compiling a view from what it knew when it last woke. Every view the audio half swaps in plays just like the one it replaces, every time the control half has caught up the view playing is a fresh compile of its project, and the clip, the takes, the notes played back and the pressure are just what they are with the halves in step. About 30 views a run catch up recorded changes; nothing is lost, sent back or compiled again.
- `follow_actions_land_on_their_bar_lines_with_the_control_half_late` runs every follow action (stay, again, next, previous, first, last, any, other, stop) on 1-bar clips at 300 BPM, at 64, 128 and 512 frames, with the control half up to 20 ms late: each lands on its bar line, playing just what it plays in step.
- `the_halves_split_onto_two_threads_play_what_the_control_half_launches` splits the host and runs the control half on a real thread, polling every millisecond: the clip it launches plays from the first period the commands reach, on the bar lines, and nothing is lost.
- `mpfree-play`'s `tests/catch_up.rs` and `tests/cue.rs` test the catch-up and the switches alone.
- `step_edits_go_into_the_playing_view_in_place_as_a_compile_would` (time.rs) taps, nudges, changes, locks (with and without a note), copies and erases steps on the pattern playing: after each, the audio half took the changes without allocating or freeing, its view equals a fresh compile (notes on one tick in a compile's order too), and nothing compiled; through the facade the status line counts them. `step_edits_that_need_more_than_the_playing_view_compile` covers a new pattern, the length, a big erase and recording; `a_step_edit_the_view_cannot_take_compiles_once_and_is_counted` a change the view can't make.

## Steps

1. Done: a compile checks only what it plays.
2. Done, in two parts:
   - **2a:** recorded changes numbered (`mpfree_play::Rec`) and made to the view and the project alike (`PlayView::record`, `Takes::record`), views that say what they include (`Includes`), the switch count, and the catch-up (`Recent`). See [playback.md](playback.md#compiling-playview).
   - **2b:** the queues, the messages and the two halves behind the facade, still on one thread, with the tests above.
3. The app thread runs the control half on its control thread. `main.rs` has none yet (the controller poll, the app's effects and the logging run in the audio loop), so it first gets one, which takes `TimeControl` from `TimeHost::split` with them, and the audio loop keeps the engines, `TimeAudio` and the queues. Round tests on the MPC compare the audio thread's worst period with launches, undo and capture in a big project, before and after.
4. Song edits and capture lose their project clones, since the control half can afford them.
