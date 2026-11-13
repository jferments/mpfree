# Audio tracks: recording on the song timeline

**Status:** engine crate `rust/mpfree-audiotracks`, 2026-11-12; in the pad player since 2026-11-13 (`rust/mpfree-app/src/audiotracks.rs`, see "In the pad player" below), workstation only. It has not run on the MPC One.

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

A region sits on song ticks, so it stays on its bar when the tempo before it changes. Its audio plays sample for sample from the region's start, the way a DAW plays audio that is not warped. At a tempo other than the one it was recorded at, it doesn't stretch by itself, so a region longer than a bar drifts against the bars until the take is fitted (below).

**Fitting a take to a new tempo** ([`stretch.rs`](../../../rust/mpfree-audiotracks/src/stretch.rs)) keeps its pitch:

1. A take remembers the tempo it was recorded at (`TakeInfo::bpm`), and `stretch::fit_ratio(recorded, now)` says how much longer it must be.
2. Off the audio thread, the host reads the take with `read_take`, runs a `StretchJob` (the `mpfree-stretch` phase vocoder that keeps hits sharp) in steps on a worker, and loads the result under `free_take_id()` with `load_take`. The new take keeps the old one's anchor, so its frame 0 stays on the same tick.
3. Once it has copied in, `Command::Swap { old, new, ratio }` plays it wherever the old take played, with each region's `src` scaled by the ratio, as one undo step. The old take stays until it is forgotten, so Undo goes back to it.

The stretch is one ratio over the whole take. That is right for a take recorded at one tempo and played at one tempo. A test records a 441 Hz tone at 240 BPM, halves the tempo and fits the take: the take then fills its bar again, still at 441 Hz.

Positions use the transport's tempo map (`process` takes it), so a tempo map with steps plays audio in the right place: `frames_from` and `tick_after` walk the map's segments.

## Memory

All takes share one pool of samples, asked for once in `AudioTracks::new(rate, seconds)`: `seconds` of stereo, or twice that of mono takes. Its pages are touched there, so the audio thread never waits for the system to map one in. 300 s of stereo at 44.1 kHz is 106 MB.

- The pool is cut into equal chunks of 32,768 samples (`CHUNK`, 128 KiB; 0.74 s of mono at 44.1 kHz). A take is any number of chunks, each in any free slot, found through a small table made once ([`pool.rs`](../../../rust/mpfree-audiotracks/src/pool.rs)). A recording takes one chunk at a time, so any number of tracks can start recording at any moment with no planning, and nothing is ever moved. The last chunk of a take is usually part-filled.
- A recording that runs out of memory stops there. What was written stays, with its region; the take is marked `cut_short` and counted in `Stats::full`. The track records again at the next punch in.
- `memory()` is the gauge (`used`, `capacity`, in samples).
- **Waveforms.** The pool keeps the lowest and highest sample of every block of 256 samples (`PEAK_BLOCK`) as audio is recorded or loaded. `peaks(id, from, to)` reads the table for whole blocks, so a screen column covering a second of audio costs about 170 reads instead of 44,100. It works while a take records, up to what has been written, and doesn't allocate. A chunk is whole blocks, so a block never mixes two takes. Chunks read back from the disk while streaming have no peaks here; the disk side keeps those (below). The host gives the screen waveforms (`wave`, below).

Without streaming, the pool is the whole limit, as a tape that ends. With streaming it is a cache in front of the disk; see "Streaming from the stick".

## Saved with the project

[`save.rs`](../../../rust/mpfree-audiotracks/src/save.rs) holds the audio tracks' part of a project (`parts/audiotracks.txt`): each track's settings and regions, and each take's place, with its audio named by file. The audio is a 32-bit float WAV a take, mono or stereo as recorded.

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

## In the app: the host

[`host.rs`](../../../rust/mpfree-audiotracks/src/host.rs) is what the app wires in. `AudioHost` owns the tracks (on the audio thread) and a worker thread, `mpfree-audiotracks`, that makes and drops every big buffer, so the audio loop never allocates audio memory.

- **Each period:** `period(block, map, input, out)` runs the tracks on the transport's block, then tends the save, the fit and the buffers handed back by loads.
- **Save:** `save()` waits until nothing records or loads, has the worker make room for every take, then reads the takes at `READ_SPEED` (64 frames for each frame that plays). `saving()` says how far it has got. `take_saved()` gives the result, ready for `save::to_text` and `save::wav`.
- **Open:** `load(saved)` puts the takes and tracks back. Each take copies in over the next periods. Anything that can't be put back is named in the error, and the rest loads.
- **Fit to tempo:** `fit(id, bpm)` reads the take, stretches it on the worker, loads the result under a free id and swaps it in as one undo step. `fitting()` names the take while it runs. `fits` and `fits_failed` count the outcomes, so a failed fit is shown, not hidden.
- **Waveforms:** `wave(id, columns)` reads a take's peak table into up to `columns` columns (at most `MAX_COLUMNS`, 4096) at `WAVE_SPEED` (1024 frames of the take for each frame that plays). `take_wave()` gives a `TakeWave`: the lowest and highest sample of each column. A column covers whole blocks of the table, so a short take gets fewer columns. The pool lives with the tracks on the audio thread, so this is how the screen sees it. For a take that is recording, ask again as it grows.
- **Tests** (`tests/host.rs`): a save made across periods opens in a fresh host and plays back sample for sample; a save asked for while recording waits for the recording to end; a fit at half the tempo fills the bar, and Undo goes back; a waveform matches the take's samples column by column, and asking again replaces a wave not yet done.

## Streaming from the stick

Jay chose (2026-11-12) to stream takes from the stick's data partition rather than hold them all in RAM. The memory becomes a cache: recorded chunks are written out behind the recording, and the chunks the next seconds of the song need are read ahead of the playhead. The audio thread only ever touches memory, so streaming adds no latency.

**The engine** (`AudioTracks::stream(true)`, only while no take is there):

- Each chunk is **dirty** (recorded or loaded here, not yet on the disk), **clean** (the same as the disk has) or **loading** (waiting for the disk). Only clean chunks can make room for others, the one wanted longest ago first, so nothing recorded is ever lost to make room.
- A full chunk of a recording, and the last part-filled one when it ends, is asked to be written (`Io::Write`, then `Io::Finish` for the file). A load (a fitted take) is written out the same way as it copies in, a chunk at a time, so it needs only a little memory.
- Every 1,024 frames, and at once after a start, stop, locate, loop jump or tempo change, a planning round holds the chunks every track plays in the next 4 s (`AHEAD_SECONDS`; the first second for all tracks before the rest), then the first second of the loop when one is set (`HEAD_SECONDS`), then the first second of each region, nearest the playhead first, up to a quarter of the memory. Held chunks not in memory are asked for (`Io::Read`), at most 64 a round. The host answers with `fill_part`, a part a period.
- A chunk that isn't in memory when it should play is silence, counted in `Stats::missed` (frames). Disk errors are counted in `read_errors` and `write_errors`. Like every fallback, these are shown, not hidden.
- `add_take` puts back a saved take whose audio is in its file; its chunks are read as the song needs them.

**The host** (`AudioHost::streaming(rate, seconds, dir)`) adds a disk thread, `mpfree-audiotracks-disk` ([`disk.rs`](../../../rust/mpfree-audiotracks/src/disk.rs)):

- **Files.** Each take recorded here gets a new file name, `<stamp>-<n>.wav`, never reused, so a save that names a file always finds that audio. The file is a 32-bit float WAV, written a chunk at a time at its place and finished (header lengths, fsync) when the take ends, with a small `.peaks` file beside it holding the lowest and highest sample of every 1,024 samples.
- **Copies.** Each period copies at most `COPY_SAMPLES` (8,192 samples, 32 KiB) into or out of the tracks' memory, read chunks first. That is about 94 chunks a second at 128-frame periods, against about 60 a second for 8 stereo tracks recording while 8 more play. It keeps the audio thread's cost small and steady. Reads go out as soon as a buffer is free; writes and finishes keep their order. The 32 chunk buffers are made once.
- **Save.** `save()` waits until nothing records, loads or is unwritten. `take_saved()` then gives the text with each take's file name and no audio, because the audio is already in the files in `dir`.
- **Open.** `load(saved)` opens and checks each take's file on the disk thread: it must be there, at the right rate and channel count, and long enough. The waveform table comes from the `.peaks` file, or from reading the take if that file is missing. Each take is put back once its file checks out. `opening()` counts the takes still being checked. A take whose file is missing or wrong is named in `open_errors`; its regions stay and play nothing.
- **Fit.** The take's audio is read from its file, stretched on the worker, and loaded. The load streams into a new file, then the result is swapped in. Takes over `FIT_MAX_SAMPLES` (about 6 minutes of stereo at 44.1 kHz) are refused, because the stretch holds the whole take in memory.
- **Waveforms.** `wave` asks the disk thread, which builds the columns from its table. The end of a recording that isn't on the disk yet comes from the tracks' own table.
- `disk_stats()` gives the reads, writes and errors, and the longest single call. The test hook `delay_us` makes the disk slow or stalling.

**Tests:**
- `tests/stream.rs` (a fake disk with a chosen delay):
  - 40 s recorded into 6 s of memory is kept whole and plays back sample for sample;
  - a disk 2 s slow misses nothing in steady play;
  - one 8 s slow is heard as counted misses;
  - three comped loop passes stream exactly as they play in memory;
  - a loop over the middle of a long take plays every pass.
- `tests/host_stream.rs` (real files):
  - the long recording lands in its WAV whole and plays back from it;
  - a save opens in a fresh host and plays the same;
  - a missing file is reported;
  - a 5 s stall never holds up the periods and is heard as counted misses;
  - a fit and a waveform work from the files.
- `tests/no_alloc.rs` also covers a streaming session.

**Cost.** The bench case "Audio tracks: 8 streaming comped takes" is the in-memory case with 60 s of memory: three passes in play on every track are 24 streams at once, the worst case for the cache. It costs 58,151 instructions a block against about 44,400 fully in memory, which is the planning plus the copies. Looking chunks up costs the in-memory cases 1.8 %.

**Limits.**
- The memory has to hold the read-ahead for every stream in play: tracks × takes playing at once × 4 s. With 60 s of stereo memory that is about 7 s of audio per stream for 8 tracks of 3 passes each.
- After a jump to a place whose chunks aren't in memory, past the first second of each region and the loop's start, play is silent until the disk answers. That silence is counted.
- Files of forgotten takes stay in the folder. Clearing out files no save names is not built yet.

### Measuring the disk first

Jay chose (2026-11-12) to stream takes from the stick's data partition rather than hold them all in RAM: a small RAM buffer in front of the take files, a disk thread reading ahead of the playhead and writing behind a recording, and the first second or so of every region kept in RAM so jumps between comped takes and locates play at once. The audio thread only ever touches RAM, so streaming adds no latency. The risk is the stick pausing: a pause longer than the buffer would lose audio, which is counted and shown.

How big the buffer must be depends on the stick, so `mpfree-disktest` measures it ([`src/bin/disktest.rs`](../../../rust/mpfree-audiotracks/src/bin/disktest.rs)). Every I/O is 128 KiB. It runs a sequential write, a write paced at 3.1 MB/s (8 stereo tracks recording) for 60 s with fdatasync every second, reads with the file's pages dropped from the cache (sequential, then 8 paced streams), and 4 writing and 4 reading streams at once. For each it gives p50, p99 and max time per call, the calls over 50, 100, 250 and 500 ms, and how far behind schedule it fell, which is the buffer a recording or playback would need.

- **Safety (rule zero).** With no arguments it runs only on `/run/mpfree-data`, and only if that is the mount of an ext4 file system labelled `MPFREE-DATA` on partition 4 of a USB disk, with no MMC device present (it reads the superblock, read-only, for the label). It writes only inside `mpfree-disktest/` there, with files it creates itself (`O_EXCL`, `O_NOFOLLOW`): at most 448 MiB at once and at most half the free space, refusing under 64 MiB free. It deletes the folder at the end, after an error, on Ctrl-C, or when the ssh session closes, and a later run clears a folder left by a power cut (only its own file names). Free space is printed before and after.
- **On the MPC:** [`tools/mpc-one/host_disktest_mpc.py`](../../../tools/mpc-one/host_disktest_mpc.py) copies the static armv7 build to `/run/mpfree-disktest` (RAM), checks its hash and runs it, as `host_bench_mpc.py` does for the benchmark. It needs `MPFREE_ACCEPT_DISKTEST=YES`, a terminal, and Jay's go for the step. Build: `RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" cargo build --release --target armv7-unknown-linux-gnueabihf -p mpfree-audiotracks --bin mpfree-disktest`.
- **On a desktop:** `mpfree-disktest --desktop-test DIR [--quick]`. Tests: `tests/disktest.rs` (a quick run leaves nothing, a folder holding other files is left alone, a leftover folder of its own is cleared, no run off the stick without the flag) and `tools/mpc-one/tests/test_host_disktest_mpc.py` (against a fake test build under BusyBox).

## In the pad player

[`rust/mpfree-app/src/audiotracks.rs`](../../../rust/mpfree-app/src/audiotracks.rs) holds `TrackHost`, which owns the `AudioHost` in the app's audio loop.

- **Each period**, right after the looper (which reads the line input), `TrackHost::period` runs the tracks on the time engine's block and tempo map (`TimeHost::block`, `TimeHost::tempo_map`) with the looper's copy of the line input, and gives their sound to the mixer's fourth track, **Audio** (`mixing::AUDIO`). It sends `SetActive` as song mode turns on and off (`TimeHost::song_on`), so the tracks play and record in song mode only (layout spec 4c); monitoring goes on either way. REC records an armed track as the transport records.
- **Latency:** `TrackHost::follow_latency` sends the looper's line input latency (`Settings::loop_latency`, "Latency to make up for", estimated from the audio buffer until set by hand) to the tracks as `SetLatency` whenever it changes, every period from the loop. Layout spec 4c asks for one line input latency shared with the looper; this is it.
- **Memory:** 60 s of stereo (`audiotracks::SECONDS`, about 21 MB), set aside at start. Prefs "Audio track memory" (layout spec 4c) comes with the track panel. Takes stay in RAM: streaming from the stick waits for the stick speed test, which needs Jay's go (rule zero).
- **The Song view:** `TrackHost::fill` puts the lanes (`AudioLane`, built with `song::audio_blocks`), the waveforms, `audio_free` and `audio_counts` (refused, dropped, cut short, fits failed: shown while not zero) into the `TimeState` the app sends. `TrackHost::take_changed` checks a fingerprint of the lanes every 8 periods without allocating, so the state goes out again when a region, an arm, a recording or a counter changes.
- **Waveforms** are read a take at a time with `AudioHost::wave` (one column per block of the peak table, at most 4096); a recording take is read again about every quarter second as it grows. Replaced waves go back to the host's worker to be freed (`AudioHost::drop_wave`), so the audio thread never frees.
- **Touches** (`AudioAction`): Arm (with a note when the line input is off), "+ Audio" (shows the next track's lane), Split at the playhead, Erase (the picked region), Undo. Panel, Fades, Gain and Takes answer that they aren't built yet. Each answer is a log line. A pick stays with the time engine.
- **Tests:** `audiotracks.rs` records a tone in song mode onto an armed track, checks the lane, the red block, the waveform and the playback, and that the tracks are silent out of song mode; Split, Erase and Undo, and each refusal's reason; steady play with the waveform read neither allocates nor frees. `rt_safety.rs` runs the tracks (one armed) in the sound path under the counting allocator.

## Joining the rest of MPFree

- **The arrangement.** Audio tracks keep their own regions on song ticks, next to the arrangement's MIDI placements (PR #160), so the two are independent. The Song view can draw each audio track as one more lane, with its take lanes under it for comping. `SetActive(false)` silences the tracks when the song is not playing (clip launch mode), if that is what the time engine wants.
- **Live to song (PR #164)** records the launch grid into the arrangement. Recording audio at the same time is just an armed audio track while the transport records.
- **The line input** comes from the pad player's capture (`alsa.rs`), as for the looper. Taking a mixer capture point instead (`Mixer::set_capture`) would let a track record the pads or a bus.
- **The screen.** `regions`, `takes`, `take`, `peaks` (waveform), `recording`, `armed`, `history`, `memory` and `stats` are what a page needs. Where audio tracks go in the layout is the app thread's call, and needs Jay's approval.
- **Export.** `take_wav(id)` writes a take as a 32-bit float WAV for the store (it allocates, so not on the audio thread).

## Demo

`cargo run --release -p mpfree-audiotracks --example demo [OUT.wav]` records a bass line on track 1, then three loop passes of a melody on track 2 (the second with a wrong note), comps bars 1-2 from pass 1 and bars 3-4 from pass 3, and plays the song back twice round. On the workstation: 19.2 s, peak -4.0 dBFS, 4 takes, track 2 regions 2, audio hash `8127531820478b39`.

## Not built yet

- The app's wiring for streaming: the folder per project (agreed with the project thread: the library's work folder `.work/audiotracks/` while a project is unsaved, the project's `samples/` once saved), the memory size, and showing `missed` and the disk errors on screen.
- In the pad player: the track panel, Fades, Gain, take lanes and comping, dragging regions, the header's level bar, Prefs "Audio track memory" (the line input latency is the looper's setting, shared), and saving with the project.
- Clearing out take files that no save names.
- Fitting takes on its own when the tempo changes. The host fits one take when asked; deciding when to ask is the app's call.
- Device runs. Opening the line input on the MPC needs Jay's go, step by step (rule zero).
