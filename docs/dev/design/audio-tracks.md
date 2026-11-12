# Audio tracks: recording on the song timeline

**Status:** engine crate `rust/mpfree-audiotracks`, 2026-11-12, workstation only. Nothing in the pad player uses it yet, and it has not run on the MPC One. The arrangement it sits beside (T13, PR #160) and live to song (T11, PR #164) are not on main yet.

The MPC's factory OS has audio tracks: linear recordings that play in the sequence next to the MIDI tracks. MPFree's looper (T4) records loops; this crate records in a line along the song, the way a tape machine or a DAW does, and adds what the MPC does not have: a take for every pass of a loop recording, and comping between takes.

## How it records

- **Arm, then record.** Up to 8 audio tracks (`MAX_TRACKS`). An armed track records while the transport records: Record from stopped (after the count-in), or a punch in while playing. It stops at the punch out, at Stop, or when the track is disarmed. Several tracks can record at once.
- **Input.** Each track takes the left jack, the right jack, both summed (mono takes) or both as a stereo take (`Command::SetInput`). Round 7 found the MPC One's line input records on the left jack, so `Input::Left` is the default.
- **A take is one recording:** its audio and the song tick its first frame belongs to (its `anchor`). A new recording puts its region over whatever the track had there; the older audio is covered, not lost, and stays available to comp from.
- **Loop recording.** While the transport loops, each pass ends its take at the loop end and starts the next at the loop start, at the same frame. The passes share a `group` and are numbered by `pass`; the newest pass plays. A locate while recording does the same: the take before it ends where the song left off, and a new one starts where it lands.
- **Latency.** Input that arrives `latency` frames after a moment belongs to that moment (`Command::SetLatency`, for all tracks). The take's anchor is placed that much earlier, and a recording keeps writing that long after its punch out. It then writes `FADE` frames more, so its end can crossfade into what follows. Round 7 used a latency of 512 frames; the right number should be measured with a cable from out to in.
- **Monitoring.** `Monitor::Auto` (the default) plays the input on an armed track while stopped, counting in or recording, and plays the track's regions while just playing. `Monitor::In` always plays the input as well as the regions; `Monitor::Off` never plays the input. While a track records, its regions are silent (as in a DAW's loop recording, the earlier passes do not play over the new one).

## Regions, comping and edits

What a track plays is its list of regions, sorted and never overlapping. A region is a stretch of song ticks (`start..end`) that plays one take from frame `src`, at the rate it was recorded at.

| Command | What it does |
|---|---|
| `Comp { take, from, to }` | Plays the take on its track from `from` to `to`, over what was there (clipped to where the take has audio). A swipe across a take lane. |
| `Split { track, at }` | Cuts the region playing at `at` in two. |
| `Move { track, at, to }` | Moves the region playing at `at` to start at `to`, over what is there. |
| `Erase { track, from, to }` | Silences a stretch; regions across its edges are cut there. |
| `SetGain { track, at, gain }` | The region's level, 0 to 4. |
| `SetFades { track, at, fade_in, fade_out }` | Fades in frames from the region's start and to its end (each at most half the region). |
| `Undo`, `Redo` | One history of every edit and recording, for all tracks (32 steps, `HISTORY`). Refused while recording. |
| `Forget(take)` | Gives a take's memory back. Refused while a region, or a step Undo or Redo could bring back, plays it (`in_use`); `ClearHistory` first. |

An edit that names a track or region that is not there, or would not fit (256 regions a track, `MAX_REGIONS`), changes nothing and is counted in `Stats::refused`.

## No clicks

Every region fades in over `FADE` (64) frames from its start and plays on `FADE` frames past its end while fading out, both equal power. Two regions that meet therefore crossfade over the same 1.5 ms. A punch in fades from the older take into the new one, a punch out back again, and a comp boundary between two passes of the same part is seamless. The demo's comp boundaries show no larger sample-to-sample jump than the music around them.

## Time

A region sits on song ticks, so it stays on its bar when the tempo before it changes. Its audio plays sample for sample from the region's start, the way a DAW plays audio that is not warped. At a tempo other than the one it was recorded at, it does not stretch, so a region longer than a bar drifts against the bars. Time-stretching a take to a new tempo is a later step, using `mpfree-stretch` (phase vocoder on a worker, already on main) to print a stretched copy.

Positions use the transport's tempo map (`process` takes it), so a tempo map with steps plays audio in the right place: `frames_from` and `tick_after` walk the map's segments.

## Memory

All takes share one pool of samples, asked for once in `AudioTracks::new(rate, seconds)`: `seconds` of stereo, or twice that of mono takes. Its pages are touched there, so the audio thread never waits for the system to map one in. 300 s of stereo at 44.1 kHz is 106 MB.

- A recording writes into the largest free stretch. When two record at once, a new one takes half of the stretch another is still filling, whichever is larger, so tracks can start recording at any moment with no planning. A take holds up to 32 stretches (`MAX_EXTENTS`), and nothing is ever moved.
- When a recording ends, the part of its last stretch it did not fill goes back, joined to free neighbours.
- A recording that runs out of memory stops there. What was written stays, with its region; the take is marked `cut_short` and counted in `Stats::full`. The track records again at the next punch in.
- `memory()` is the gauge (`used`, `capacity`, in samples).
- **Waveforms.** The pool keeps the lowest and highest sample of every block of 256 samples (`PEAK_BLOCK`) as audio is written or loaded. `peaks(id, from, to)` reads the table for whole blocks, so a screen column covering a second of audio costs about 170 reads instead of 44,100. It works while a take records, up to what has been written, and doesn't allocate. Every stretch of the pool starts and ends on a block edge, so a block never mixes two takes. Memory is therefore handed out in blocks of 256 samples (5.8 ms of mono), and the pool is cut down to whole blocks. The host shares the peaks with the screen thread the way it shares the rest of the state, as the looper's waveform does.

Recording to the storage partition instead of RAM, so tracks can be as long as the disk allows, needs a writer thread and the store; it is not built.

## Saved with the project

[`save.rs`](../../rust/mpfree-audiotracks/src/save.rs) holds the audio tracks' part of a project (`parts/audiotracks.txt`): each track's settings and regions, and each take's place, with its audio named by file. The audio is a 32-bit float WAV a take, mono or stereo as recorded.

- **Saving.** `save_file()` gives the settings, regions and takes. It returns None while a track records or a take loads, leaves out tracks with nothing to keep, and allocates. `read_take(id, from, out)` reads a take's audio a piece at a time without allocating, so the app can read across periods. `save::to_text` and `save::wav` make the files. A take never changes once recorded, so a save read slowly can't tear.
- **Opening.** Takes and tracks come back through `save::from_text` and `save::from_wav`, then:
  - `load_take(file)` puts each take back under its own id. It takes the memory at once, then copies the audio in at `LOAD_SPEED` (64 frames for each frame that plays). A loading take plays nothing until it's done.
  - `load_track(&file)` puts back the settings and regions, and forgets the undo history.
  - `take_returned()` hands the loaded audio back for the host to drop away from the audio thread.
  - A load is refused, with the take given back, if the id is in use, the audio is the wrong length or the memory is short.
- **Unknown words and lines are skipped,** so a newer build's text still loads. Regions must be in order and must name takes of their own track.
- **Test:** a session (three comped loop passes, a stereo punch, gains and mixes) is saved, opened in fresh tracks, and plays back the same, sample for sample.

## The audio thread

`command` and `process` never allocate, lock or panic, and give the same output for any period size (a test compares 1, 7, 128, 999 and 4096 frames). `tests/no_alloc.rs` checks a busy session with a counting allocator: recording on three tracks, punches, loop recording, locates, a tempo change, comping, every edit, undo, redo, forgetting, a full memory and stops.

`process(block, map, input, out)` adds every track to `out`. `track_out(track, frames)` gives each track's own sound after its level, pan, mute and solo, so the host can feed each audio track into its own mixer strip instead.

## Cost

`mpfree-bench` has two cases, counted against the looper's budget (3 % of a period) until there is a line of its own:

| Case | Instructions per 128-frame block (callgrind) | Desktop mean / p99 |
|---|---|---|
| Audio tracks: 8 playing comped takes | 43,917 | 2.9-4.9 us / 8-10 us |
| Audio tracks: 6 playing comped takes, 2 recording | 43,619 | 3.0-4.4 us / 9-10 us |
| For comparison: Looper, 8 loops of 3 layers playing | 298,470 | |

Each case is an 8-bar song loop with every track comped so that two takes crossfade on every beat. Using round 7's calibration (the MPC is roughly 6.6 to 10 times slower), 8 tracks should cost about 1 % of a period on average and about 3 % at the 99th percentile. That is an estimate, not a device number. The worst blocks on the cloud VM vary from run to run.

## Joining the rest of MPFree

- **The arrangement.** Audio tracks keep their own regions on song ticks, next to the arrangement's MIDI placements (PR #160), so the two are independent. The Song view can draw each audio track as one more lane, with its take lanes under it for comping. `SetActive(false)` silences the tracks when the song is not playing (clip launch mode), if that is what the time engine wants.
- **Live to song (PR #164)** records the launch grid into the arrangement. Recording audio at the same time is just an armed audio track while the transport records.
- **The line input** comes from the pad player's capture (`alsa.rs`), as for the looper. Taking a mixer capture point instead (`Mixer::set_capture`) would let a track record the pads or a bus.
- **The screen.** `regions`, `takes`, `take`, `peaks` (waveform), `recording`, `armed`, `history`, `memory` and `stats` are what a page needs. Where audio tracks go in the layout is the app thread's call, and needs Jay's approval.
- **Export.** `take_wav(id)` writes a take as a 32-bit float WAV for the store (it allocates, so not on the audio thread).

## Demo

`cargo run --release -p mpfree-audiotracks --example demo [OUT.wav]` records a bass line on track 1, then three loop passes of a melody on track 2 (the second with a wrong note), comps bars 1-2 from pass 1 and bars 3-4 from pass 3, and plays the song back twice round. On the workstation: 19.2 s, peak -4.0 dBFS, 4 takes, track 2 regions 2, audio hash `8127531820478b39`.

## Not built yet

- Recording to disk, for tracks longer than the RAM pool.
- Time-stretch to a new tempo (with `mpfree-stretch`).
- The pad player's wiring and the Song view's audio lanes and take lanes. Saving is built in the engine (above); the project browser writes the files once it is in the app.
- Device runs. Opening the line input on the MPC needs Jay's go, step by step (rule zero).
