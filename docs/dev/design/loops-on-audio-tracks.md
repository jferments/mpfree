# Loops on audio tracks: the contracts (phase 4)

**Status:** designed 23 Nov 2026, before 4b's code, as the [phase 4 plan](https://claude.ai/artifact/HA92ecH36LM6UDwiLCwJHa) asks (approved by Jay on 23 Nov; its build steps are in [audio-tracks.md](audio-tracks.md#loops-joining-audio-tracks-phase-4)). Built: 4a, each loop on its own output ([looper.md](looper.md#each-loop-on-its-own-output)), and of 4b: loop ids, loop cells on audio tracks and one track a loop (the model step), the arranger's loop answer, the track player's loops with one crossfade at the track, and the looper playing a loop where it is told, the app's wiring behind Dev's switch, and the mixer's answer for a recording's input (each marked built below). Transaction ids and recording are still to come. Everything else here is 4b to 4e, behind Dev's switch until 4e. The interfaces it builds on are phase 2's (audio clips, `Player::audio`) and Arrange 4b's (one song) as they merged; where this page names one of their types, it is that type, not a copy.

Three contracts, so 4b, 4c and 4d each follow one written interface: the identities, the track player, and the transactions behind one UNDO.

## Who decides what

**The arranger decides when, the track player manages the switch, the looper and the region player render, the mixer processes.** Nothing else schedules a loop.

```text
ARRANGER (mpfree-play): each period, each audio track's answer:
   what plays at the start, up to 4 changes on their frames, each clip's origin
     |
TRACK PLAYER (one an audio track, the audio thread)
     |-- region player (mpfree-audiotracks): take clips and song regions
     |-- looper (mpfree-looper): loop clips, at the places it is told
     |
     one crossfade and the track's mute, the Loop group's macros on the loop source
     |
TRACK'S MIXER CHANNEL (mpfree-fx) -> master
```

## The identities

Five, kept apart. A slot is never an identity, and a place in a history is never one.

| Identity | Type | What it is | Made by | Lasts |
| --- | --- | --- | --- | --- |
| **Clip** | `ClipId(u32)` (as now) | a cell's clip, or one of the song's own: its settings, its content, the blocks that name it | the project (`nextclip`) | saved |
| **Loop** | `LoopId(u128)`, new, in `mpfree-seq` beside `RecId` | an editable layered composition: its layers, each layer's view of its recording, its length, its level, its version | the app's loop host when a first pass is armed, from the audio tracks' id maker (a key drawn at random, a count through SplitMix64's finish), never 0, never given twice | saved, as 32 hex digits |
| **Recording** | `RecordingId(u128)` (phase 1c) | audio that never changes once finished | the audio tracks | saved |
| **Slot** | `Slot(u8)`, 0 to 7: today's loop number | where the looper holds a loop in memory to play and record it | the loop host's table, `LoopId` to `Slot` | the session only |
| **Playback** | no id: the track player's state for a track | what a track plays now: the clip, its origin, its voices or slot, its fades | the track player | the session only: one a track, two during a crossfade |

**A clip's content** is one of two kinds, and an audio track's clip has exactly one (`Clip::is_audio_clip`):

- `Clip::audio: Option<AudioClip>`: a take clip, as phase 2 built it.
- `Clip::loop_clip: Option<LoopClip>`: a loop cell, `LoopClip { id: LoopId }` (built, 4b's model step). The clip is as long as its loop (`loops::length`, as a Loop cell is: `Clip::new_audio_loop`), holds nothing a note clip holds, and its loop's own level stays in the looper. Both at once is `BadAudioClip`; a loop cell with notes, or naming `LoopId::NONE`, is `BadLoop`.

Two fields rather than one enum, so phase 2's take-clip code keeps reading `Clip::audio` unchanged; `Clip::check` holds the either-or. Loop tracks' `Clip::looped: Option<u8>` stays, for Loop tracks while Dev's switch is off, and goes at 4e with them.

**The rules**, enforced in the model, not left to the renderer:

1. **A loop belongs to one track** (built). Cells of that track's rows may name it, and so may blocks on its lane, of a cell's clip or of the song's own. `Project::check` refuses a `LoopId` named on two tracks (`ModelError::BadLoop`), so a loop block moved to another track's lane is refused too. `Project::loop_places` lists every place a loop is named, `Project::loop_track(id)` gives its track, and `Project::loops_named` the loops that need a slot. A track plays one clip at a time, so one playback a loop is enough.
2. **A copy onto another track is a new loop:** a new `LoopId`. From 4c its layers name the same recordings (nothing copied on the stick); in 4b it copies the audio, as a Loop track's copy does today. It takes a slot of its own either way.
3. **Live or stored.** A loop is in a slot while a cell or a song clip names it. One nothing names is stored: its views and recordings are kept for UNDO, and it holds no slot and no memory. `LoopHost::slot_of(id) -> Option<Slot>`; bringing a stored loop back takes a free slot first (`LoopHost::bring_back(id) -> Result<Slot, Full>`), and is refused with the reason when all 8 are in use.
4. **Later, not phase 4:** several read-only playbacks of one loop at once (the same loop on two tracks). The identities above are what that needs; phase 4 keeps one playback a slot.

## The track player

### What the arranger says

The answer stays phase 2's (`AudioAnswer`: what plays at the start, up to `MAX_AUDIO_CHANGES` changes on their frames, the mute with each, the ring). It gains one case, in `mpfree-play` and, as `Plays::Loop(LoopPlay)`, in the engine's own types in `mpfree-audiotracks` (the app copies one to the other, as for clips):

```rust
AudioPlays::Loop {
    id: LoopId,     // the loop the clip names
    origin: f64,    // the song tick its pass 0 starts on
    len: Tick,      // the clip's length, the loop's
    clip: ClipId,
    stamp: u64,     // the clip's, as for a take clip
}
```

- **The origin is the clip's, by the same rules as every clip** (`Switches::playing_where`, `TrackView::region_at`): a launch's line, a legato switch's old clip's origin, a block's start less its offset (`Placement::offset`). So a 3-bar loop (bars A, B, C) in a block at bar 6 plays A at bar 6, and with the block's start trimmed by a bar it plays B from bar 7. See the open point below for launches.
- **The rest is as for a take clip:** the changes land on the frame a note on that tick lands on; more than four in a period keep the last; a change's `muted` is the track's mute from it on.
- **A ring** (a once loop clip's own end) ends the loop at its pass end with the usual fade: what follows a loop's end is its own start, so there is nothing to ring on to.
- **Gone for loops on audio tracks:** `TimeControl::loop_plan`, `LoopPlan`, `LoopHost::follow_plan` and `Looper::launch`. They stay for Loop tracks until 4e, then go.

### What the looper is told

Before `Looper::process_each` each period, the track player gives each slot that sounds its runs (built: `mpfree-looper` `on_track.rs`, [looper.md](looper.md#a-loop-on-an-audio-track)):

```rust
pub struct PlayRun {
    pub from: u32,      // the first frame of the period it plays
    pub to: u32,        // the frame after its last
    pub origin: f64,    // the song tick the loop's pass 0 starts on
}
looper.play(slot, &runs)   // at most MAX_PLAY_RUNS (6) runs a slot a period
```

The track player's side is `LoopPlayer::plan` then `LoopPlayer::runs(track)` (`mpfree-audiotracks` `loop_play.rs`, built): each `LoopRun` names its loop by id, and the host gives each loop's runs to its slot.

- **The place** in the loop is the song position's, counted from the run's origin: `(tick - origin)` modulo the loop's own ticks, as a loop's own `origin` is used today. So locates, transport loops, stops and tempo changes behave as they do now (a jump crossfades inside the looper over its `FADE`), and a loop made shorter repeats sooner inside the same block.
- **Two origins, kept apart** (loop-tracks.md's "an origin per launch"): the loop's own `origin` stays where its first pass was recorded, for `KEEP_BEFORE`, Snap and its history; the run's origin is where this playback counts from.
- **The same loop with a new origin** (a launch from another row naming the same loop, a block meeting a block of the same loop with another offset) is still one playback: the looper crossfades from the old place to the new, as for a locate.
- **A slot told nothing is silent and isn't run** (4a's quiet loops). An outgoing loop's last run goes on past its change by the track's crossfade, then its slot is told nothing.
- **No launches, fades, mutes or solos of the looper's own** for a slot on an audio track: `Looper::launch` and the loop's `Command::Mute` and `Command::Solo` are refused for it. Layer mute and solo stay inside the loop.

### One crossfade, at the track

Built in `loop_play.rs` (`LoopPlayer::mix_into`, from `AudioTracks::process_with_loops`), apart from the Loop group's macros, Listen and the Loop strip's going.

The track player sums the slot's buffer (4a's `LoopOuts`) and the region player's voices into the track's channel input, each source's gain applied once: `y[n] = gL[n]·L[n] + gT[n]·T[n]`. The fades are phase 2's: equal power over the audio tracks' `FADE` (64 frames), not the looper's 256. A switch between two loop clips on one track runs both slots through the fade. The channel's effects hear one signal.

- **A loop is in one voice at a time.** A loop that comes back while its last voice still fades out (a switch away and back inside `FADE`) takes that voice again, its gain rising from where the fade had it, so the looper is only ever asked for one place of a loop. A third loop inside a fade takes the voice that is closing, and is counted (`LoopPlayer::stolen`).
- **The track's mute and solo are the only ones a loop has.** The answer's mute ramps the track's sum over `FADE`, as for clips; the panel's Mute and Solo are the engine's, as now. A muted track's loop is still told its runs, so it comes back where the transport says; the looper may skip rendering it while its gain is 0, since its place comes from the song position, not from running.
- **The Loop group's Level and Filter** act on the loop source only, before the sum (a gain and a DJ filter a track's loop source; at rest, a check). The outgoing loop keeps them until its fade ends; a take clip never has them. Delay is an offset on the channel's Send A while it plays a loop.
- **Listen** (a loop alone, while stopped) is a run the track player makes itself, through the track's channel.
- **The Loop strip goes** (`mixing::LOOP` and its `LEFT_OUT` entry) at 4e; until then, with the switch off, Loop tracks play through it as now.

### In the app (built, behind Dev's switch)

Dev's "Loops on audio tracks" (`Settings::loops_on_tracks`, kept as `loops-on-tracks`, from the next start: `Settings::loops_on`) turns it on. Each period, with it on, in the audio loop (`main.rs`):

1. **Before the looper:** the audio tracks follow the track list (`TrackHost::sync`), then `TrackHost::plan_loops` reads what the arranger says each audio track plays (`TimeHost::audio`, the same answer the tracks play) and plans its loops (`LoopPlayer::plan`; only a loop or not matters there, so no regions are copied).
2. **Ids:** a loop with sound and no lasting id gets one from the audio tracks' id maker (`LoopHost::give_loop_id`, `TrackHost::fresh_loop_id`), one a period at most. Ids go with the loops into the looper part, come back with an open, and are forgotten for a New project.
3. **Telling the looper:** each slot whose loop a loop cell or block names (looked at every 8 periods, `Project::loop_track`) is told its runs from every track (`LoopHost::tell_tracks`, `OnTracks::tell`); a slot named no more leaves its track.
4. **The looper runs** (`LoopHost::run`): a loop on a track renders only where told, into its own buffer, and is left out of the Loop strip's mix (`LoopOuts::on_track`).
5. **The tracks run** with the loops (`TrackHost::period_with_loops`, `AudioHost::period_with_loops`, `AudioTracks::process_with_loops`): each loop's buffer by its id (`LoopHost::track_loop`), mixed into its track before the track's level, pan, mute and solo, and so into the track's own channel.

The app's test `a_loop_cell_plays_its_loop_in_its_track_and_not_on_the_loop_track` (`loops_on_tracks.rs`) runs these five steps as `main.rs` does, on a loop recorded from the pads: with the cell named and playing, the loop is heard in its audio track's channel and the Loop strip is silent; with the cell gone the loop plays on the Loop strip again and the track is silent. It fails if the loop's buffer doesn't reach the track, or if the Loop strip's mix keeps a loop on a track.

With the switch off nothing here runs: a loop cell's answer reaches the region player as nothing, and Loop tracks play through the Loop strip as before. Still to come in 4b: making a loop cell (recording into an audio track's cell, boards 2 to 6), the transaction ids, and the app taking its recordings' input from `capture_for` (built in the mixer, below).

### Recording

- **One loop records at a time** (a first pass or an overdub), as `others_stop_recording` does now. A stated limit of phase 4.
- **An overdub records at the place it is heard,** from its run's origin, so a layer recorded during a block with an offset lands where it was heard. Layers keep their places from the loop's own top, so an offset never shifts a layer.
- **A change that takes the track off a recording loop** never drops the recording: an overdub punches out on the change's frame (`Looper::punch_out(slot, frame)`), its layer kept; a first pass closes on whole bars at the change's line, as STOP keeps it. Each is its own transaction, before the launch.
- **A first pass into a cell:** REC on the armed empty cell mints the `LoopId`, takes a free slot (refused with "8 loops · clear one" when there is none) and records the track's input. When the first pass closes, the time engine puts `AudioContent::Loop` in the cell and launches it with the loop's own origin, so it plays straight on with no jump.
- **The input** is the mixer's answer (built): `Mixer::capture_for(source, track) -> Result<CaptureFrom, CaptureRefusal>`, one call on its graph in place of the rules in the looper and the tracks ([fx.md](fx.md#the-mixer), "A recording's input, one answer"). `source` is a strip at a point; the answer is that strip, or a submix or the master on its way in without the track (`CaptureFrom::without`), or why not (not in the mix, the track itself, the track sends to a return that plays in, it gets there through a submix, or past the strip's way in). Master without the recording track is a second sum the mixer adds while the capture is in use (`Mixer::set_capture_from`), not a subtraction afterwards: the take is what the master would get with the track silent, to the sample. The host asks again each period, so a route changed mid-take is caught; an echo still ringing after its send came down stays the host's to wait out, as `MixHost::loop_take` does for the Loop track today. A loop takes its track's input when its first pass starts; each overdub takes the input it has then.

## Transactions: one UNDO

Every UNDO takes back the newest step anywhere. The history is made of **project transactions**; each engine keeps its own record of its part. 4b puts a transaction id on every change; 4d builds the one history on them.

### Ids and parts

- **`TransactionId(u64)`:** given by the time engine's control half, which owns UNDO, from 1, only ever up, never given twice in a run. Not saved: the history isn't. Every change to the project from one action takes one before anything changes, in every engine it touches.
- **The take numbers stay** what the screens show (takes, captures, edits, song edits, each counted on its own). The transaction id is what ties the engines: it replaces the song edit number `Command::TieStep`, `Command::SongUndo` and `Command::SongRedo` carry today (Arrange 3.4).
- **Each engine's part, under the id:**
  - the time engine: what `Takes` keeps now (notes, clips and their content, the song, rows and scenes);
  - the audio tracks: their steps (regions, takes), as the song-tied steps are kept now;
  - the looper: for each loop touched, its `LoopId`, its version before and after, and its `Step`. The per-loop history (`HISTORY`, 64 steps) becomes how the looper keeps its parts, not a history of its own.
- **The one history** lives in the control half: `done` and `redo` lists of entries, each its id, the engines it touched and its words for the screen ("UNDO: overdub on Vocal · row 2").

### Prepare, then commit

All or nothing, in four moves:

1. **Prepare**, on the control thread, from what each engine publishes for each of its parts: its version and whether it is busy (recording, loading, a command on its way). The answer is Ready, Wait (audio it needs isn't in memory: the loads start and the screen says "bringing back Loop 3's layers"), or Refused with the reason. Nothing is sent unless every engine is Ready.
2. **Commit:** the control half sends one message to the audio thread, `Commit { txn, undo or redo, each audio part, each part's version expected }`, with the play view compiled from the time engine's part as it will be.
3. **At the next period's start** the audio thread checks every expected version before it changes anything. All as expected: it applies every audio part and takes the new view in that period, and answers Committed. Any not: nothing changes, the old view stays, and it answers Refused with which and why.
4. **The control half** makes its own part real on Committed (it held it aside until then) and lets it go on Refused, saying why. Until the answer comes (a period or two) the next UNDO waits.

This is Arrange 3.4's way (ask first, then apply in the first period the edit plays), with the audio side's check moved into the period it applies in, so nothing between the two can leave a step half done. Apart (`--time-thread`), the same messages go over the link.

### Old steps and redo

- **Retired only from the oldest end.** An engine that must let go of its part of transaction T (the looper's 64 steps a loop or its room for layers, the audio tracks' history) says so first; the control half then retires T and every older entry in every engine at once (`RetireThrough(T)`), and the project as it is becomes the baseline. UNDO stops at the oldest kept entry and says older ones were let go. Nothing is cut from the middle.
- **A new step after UNDOs** drops every redo entry in every engine (the looper's redo steps of every loop too), in the same message as the new step's commit. Their audio goes once nothing else names it.
- **Not transactions:** launches, cues, scenes played, mutes, the transport, the Loop group, Listen. The synth's and the mixer's own undo lists stay their own; Freeze, Flatten and Bounce join in phase 6.

## Saved

- **The clips part** (built): a loop cell writes `loopclip <loop id>` (32 hex digits, not 0, once) in place of phase 2's `audioclip` and `region` lines (`mpfree-seq` `save.rs`), and the project needs `audio-loops` (built: `project::AUDIO_LOOPS`, in `KNOWN_NEEDS`), so a build from before phase 4 refuses it instead of opening an empty clip. The project format stays 3.
- **The looper part** names each loop by its `LoopId` (built: `id=` and 32 hex digits on its `loop=` line, `LoopFile::id`, which the looper only keeps for its host; the loop host's table, `loops_on_tracks::OnTracks`, gives and takes it) and, from 4c, each layer by its recording, with its view's lines.
- **Until 4e** a project saved with Dev's switch on is a development file and may be refused later. From 4e a project with a Loop track is refused when opened, with the reason; old projects are not converted.

## What 4b's tests pin

Built so far:

- `mpfree-seq`'s `save.rs` `loop_cells_read_back_and_a_loop_keeps_to_one_track` (two rows of one track name a loop and read back; the `loopclip` line's refusals; a loop cell on a note track, with a note, with regions or naming no loop refused; the loop's cell or block on another track refused, a copy with a loop of its own not; a block made the song's own still names it).
- `mpfree-play`'s `a_loop_cell_plays_its_loop_from_its_clips_origin` (the arranger answers a loop cell with `AudioPlays::Loop`, its id and its clip's origin).
- `mpfree-audiotracks`' `tests/loop_play.rs`: a loop opens on its frame and the looper is told from there; a take clip and a loop cross over once, with equal power, on the arranger's frame; the answer's mute ramps a loop and it comes back in place; two loops cross over in two voices and a third takes the closing one (counted); the same loop from another origin stays one voice; a loop back inside its own fade reopens its voice from the gain it had; stopped, nothing plays. Each for periods of 1 to 4096 frames. `tests/no_alloc.rs`'s `loops_through_the_track_player_never_allocate`.
- `mpfree-looper`'s `tests/on_track.rs` ([looper.md](looper.md#a-loop-on-an-audio-track)): only where told, from the run's origin, at full gain; a new origin crossfades the place; after silence no crossfade; its own launch, mute and solo refused.
- `mpfree-fx`'s `tests/mixer.rs`: `the_master_without_a_track_is_a_second_sum_not_a_subtraction` (the master's way in without track 1 is, to the sample, what the master gets with track 1 silent, through a submix move and a look-ahead insert, and the mix itself is unchanged) and `capture_for_answers_on_the_graph` (each answer and refusal, and the mixer takes every answer it gives); `tests/no_alloc.rs` asks it each block.

To come:

- A loop recorded into a cell of Audio 1 and overdubbed is heard only on Audio 1's channel, to the sample.
- A `LoopId` named on two tracks is refused by `Project::check`; a copy onto another track gets a new one.
- A switch from a loop to a take clip fades once, at the track, on the arranger's frame, for any period size.
- A block with an offset plays the bar its offset names, to the frame; an overdub during it lands where it was heard.
- A launch that cuts an overdub keeps its layer, as a transaction of its own before the launch.
- A muted track's loop comes back in place after a switch to a take clip and back.
- Every change carries a transaction id, the same in every engine it touched.
- The 8-loops worst case for real time on the MPC: the longest period, the 99.9th percentile and missed deadlines, with 8 loops on 8 tracks, each with its own effects. Its engine half is counted already (below).

## Cost

`mpfree-bench`'s "Looper (60 s) on audio tracks: 8 loops of 3 layers, half launched again every bar" is the worst the arranger can ask of the engines: the looper's 8 loops of three layers (half of them edited, as "half edited" has them), each on an audio track of its own, the track player planning every track each period, the looper rendering each loop only where it is told, and the tracks mixing them in; every bar, half the tracks launch their loop again a beat on, mid period, so the looper crossfades those loops' places. Counted under callgrind (rustc 1.97.0, 128-frame periods):

| Case | Instructions a period |
|---|---|
| The 8 loops on the Loop track ("half edited") | 425 007 |
| The same loops on 8 audio tracks | 494 618 (+16 %) |

The 16 % is the track player's plan (33 000), the tracks' mix (32 000) and telling the looper its runs (11 000); the looper's own work is the same. The first count was 1 018 146: the plan cleared all its gains each period (8 tracks, two voices, a 4 096-frame period each: 256 KiB), and now clears only the period's frames. The same bench checked that a loop launched again on its track sounds, after the crossfade, exactly as the loop placed there from the start (the difference is silence).

## Where a launched loop starts (settled)

A clip launched on its own starts from its top; Legato keeps the place of the clip it takes over. Loop tracks don't: an off loop keeps its place in the song, so a launch brings it back in step with where it was recorded. With the arranger deciding, the plan's "one rule for both kinds of clip" gives loop cells the clip rule (a loop launched on bar 3 of its 4 plays its bar 1 from there; Legato, or a launch quantise of the loop's length, keeps it in step). Jay chose this on 23 Nov ("From its top, like any clip"), and it is what 4b builds: a loop cell's launch is a clip launch, with no rule of its own.
