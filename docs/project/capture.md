# Capture: keep what you just played

**Status:** roadmap step T3's last part, 2026-11-11. The engine is [`rust/mpfree-capture`](../../rust/mpfree-capture). The Clips page's Capture button is in [`rust/mpfree-time-ui`](../../rust/mpfree-time-ui), and take undo for captures is in [`rust/mpfree-play`](../../rust/mpfree-play) (`Takes`). The pad player runs it in `TimeHost` (`rust/mpfree-app/src/time.rs`). All of it is tested on the host. Nothing here has run on the MPC yet.

You play the pads without recording, then decide you liked it. CAPTURE keeps it. The MPC's own Retro Rec does the same thing into the selected track (MPC 3.6 user guide, page 67).

## What it keeps

- **Always listening.** Every note the pads play goes into a history, whether the transport rolls or not, and whether or not anything records. On a track through the arp, the history gets the arp's notes, which is what you heard.
- **The last phrase.** A phrase is what was played since the last silence of at least one bar. A phrase is either all played while the transport rolled (in time) or all played while it was stopped (free). Starting or stopping the transport starts a new phrase.
- **Whole bars.**
  - **In time:** the clip starts on the bar line at or before the first note. Each note keeps its place in the bar.
  - **Free:** the clip starts on the first note, at the current tempo.
  - **Either way:** the clip ends on the bar line after the last note starts.
- **Up to 32 bars.** A longer phrase keeps its last 32 bars. A clip holds at most 8192 notes, the last ones played.
- **Lengths.** A note ends where its pad was released. A note still held ends at the moment of the capture.
- **Exact timing.** Notes land on the nearest tick to the frame they were played on. Nothing is quantised. The clip's quantise setting can still be turned on afterwards, because quantise never changes the stored notes.

## Where it goes

CAPTURE puts the phrase in the armed track (track 1 if none is armed), as a take of its own:

- **Over the playing clip:** if the phrase was played in time while the track's clip was playing, its notes go into that clip, each on the clip tick where it was played. It is as if you had been recording in overdub.
- **Otherwise, a new clip:** it goes in the track's first empty slot, named "Capture 1", "Capture 2" and so on, and it is launched in place of the track's clip.
  - A phrase played in time keeps its place against the song. If you played it over bars 3 and 4, bar 5 is its first bar again, so it plays on in time straight away.
  - A free phrase starts on the next bar line (when stopped, on the current bar's line).
- **FX moves go with it.** The Perform FX moves made over a phrase played in time (the XY pad, the DJ knob, a pad's Hold, kept always in a move log while the transport rolls) go into the same clip as moves, on the ticks where they were made, as part of the same capture (`mpfree_capture::moves_for_clip`, `Takes::capture_moves`). The log line counts them ("capture 2: 6 notes and 1 move, ..."). A free phrase keeps its notes only. See [playback.md](playback.md#fx-moves).
- **Undo.** UNDO takes a capture back like any take. Notes come out of the clip, or the new clip leaves its slot and stops. REDO puts it back. After an undo, the same phrase can be captured again.
- **Nothing new.** After a capture, the Capture button goes quiet until another note is played, so pressing it twice does not make two copies.
- **Refused.** It is refused, and nothing changes, when nothing was played, the track does not exist, or all 16 of the track's slots hold clips.

## The clock

The history puts every note on a clock that never goes back.

- **While the transport rolls,** the clock counts song ticks and keeps the song's bar lines. A loop of whole bars, or a jump from one bar line to another, goes on without a seam, so a phrase can run across a loop jump. A jump that is not on the bar grid moves the clock forward to the next point with the same place in the bar.
- **While it is stopped,** the clock counts ticks at the current tempo.

Each note also keeps the song tick it was played on. That is how notes over a playing clip find their clip ticks, even across loop jumps.

## On the screen

- **The Takes panel** on the Clips page has three buttons: Capture, Undo and Redo.
  - Capture shows what it would keep ("Capture 2 bars") with an accent outline. When there is nothing new to keep, it is greyed out.
  - Captures show in the list as "Capture 1 · Chords / 2", and "new +6" when the capture made a new clip.
- **Undo and Redo** name a capture "capture 1", and a recorded take "take 3", on the buttons and on the transport bar.
- **The sample screens** (`mpfree-time-preview`) show the button before and after a capture (`clips-capture-ready`, `clips-captured`).
- **SHIFT + REC** as CAPTURE is proposed for the pad player, on every page. It is a shared button, so it waits for the owner's approval.

## Real time

`Capture::input`, `Capture::process` and `Capture::pending` run on the audio thread and never allocate. The history is a fixed ring:

- 16384 note starts and ends: 8192 notes, about 8 minutes of 1/16 notes on two pads at 120 BPM;
- 256 notes a period.

`Capture::dropped()` counts notes lost to a full input table. It should stay 0.

`Capture::phrase` and `keep` allocate. In the pad player they run when the button is pressed, like the other edits.

## Tests

- `src/tests.rs` (13 tests):
  - a phrase in time starts on its bar and keeps its place; a free phrase starts on its first note;
  - a bar of silence, or the transport starting, ends a phrase; less than a bar does not;
  - a held note ends now, and a pad pressed again ends the held note;
  - a long phrase keeps its last 32 bars;
  - a phrase goes on across a loop of whole bars, and each note keeps its song tick;
  - every period size (1, 64, 128, 1000 and 4096 frames, with a tempo change) gives the same phrase, rolling or stopped;
  - the button goes quiet once a phrase is kept, until the next note; the history is a ring;
  - `keep`: a new clip in time plays on in its place (in a loop too); a free one starts on the next bar; notes over the playing clip go into it; refusals change nothing.
- `tests/no_alloc.rs`: a counting allocator watches the capture follow a busy transport long enough for the history to wrap, at periods of 1, 128 and 4096 frames. It sees no allocation and no free.
- `mpfree-play`'s `takes.rs`: a captured clip is undone and redone as a take, and a slot filled meanwhile keeps its clip. Recorded notes still join their own take when a capture has the same number. Captured notes go in all together or not at all.
- `mpfree-time-ui`: the button, the take rows, and a sample session that captures a riff as a new clip in place, captures over a playing clip, and undoes both.

## Listening

`cargo run --release -p mpfree-capture --example capture_demo` writes `~/mpfree-work/capture-demo.wav` (34.5 s). The tool synthesises every sound itself.

1. A two-bar beat plays at 90 BPM.
2. In bars 3 and 4, a chord stab riff is played on the pads, a little off the grid, without recording.
3. CAPTURE in bar 4 makes it a clip that plays on from bar 5 with the same timing.
4. In bar 7, clanks are played on the off-beats over the clip.
5. CAPTURE in bar 8 puts them into the clip, so bars 9 and 11 have them.

The tool checks every bar (what played live and what played from the clip, each within a tick of where it was played). It also undoes both captures on a copy. It prints an events hash, and it checks that the hash is the same at periods of 1, 128 and 4096 frames.
