<!--
covers: rust/mpfree-audiotracks/src/lib.rs rust/mpfree-audiotracks/src/clips.rs rust/mpfree-audiotracks/src/save.rs rust/mpfree-audiotracks/tests/tempo.rs rust/mpfree-stream/src/cursor.rs rust/mpfree-seq/src/audio.rs rust/mpfree-seq/src/arrange.rs rust/mpfree-app/src/audiotracks.rs rust/mpfree-app/src/time/audio_wait.rs rust/mpfree-fxlab/src/readers.rs rust/mpfree-bench/src/cases.rs rust/mpfree-stretch/src/sched.rs rust/mpfree-stretch/src/lib.rs
checked: 2026-11-25 53005cf4
-->
# Audio following the tempo

**Status:** phase 5a of the audio tracks rethink, built 2026-11-25 (the mapping, Follow tempo, Repitch, saving, the one history, the read's measures and bench rows), with no screen yet: phase 5c draws it. Stretch plays as Repitch until phase 5b makes its copies; 5b.1 built the scheduler they will be made on, with no client yet. Workstation only; it has not run on the MPC One.

Until phase 5 a region of an audio track, and an audio clip's region, played its recording at the rate it was recorded at, whatever the tempo ([audio-tracks.md, "Time"](audio-tracks.md#time)). A region recorded at 120 BPM and played at 90 stayed on its bar's start and ran on past its bars. Phase 5 lets a region follow the song's tempo instead, so it stays on its beats. The plan is [phase 5's](https://claude.ai/artifact/4hTk7JTozq8jiWPMzTvhrd) (mpfree-claude `notes/audio-tracks/07-phase-5-plan-clips-follow-the-tempo.md`), and its build steps note 16.

## The setting

Every region, of the song or of a clip, has three new fields (`Region` in `mpfree-audiotracks`, `ClipRegion` in `mpfree-seq` and in the track player):

- **`follow`** (`FollowTempo`): **Off** (the default, as before), **Repitch** (read faster or slower with the tempo: on its beats, its pitch moving with the speed) or **Stretch** (on its beats at its own pitch, from a stretched copy; phase 5b. Until then it plays as Repitch).
- **`keep_pitch`**: on Stretch, whether to keep the pitch while its copy isn't ready (Keep pitch) or to play on time meanwhile (Play on time). Saved now, used from 5b.
- **`bpm`**: the tempo its audio was made at. A song region carries it resolved: its take's (`TakeInfo::bpm`) unless set by hand, or 0 for none known. A clip region says 0 for its recording's and a tempo only when it has one of its own, so a clip from before phase 5, followed later, plays at its recording's tempo; the track player (`takes_for`), Song → scenes (`SongSound::tempos`) and the cues that wait for their audio (the time engine's `rec_tempos`, which the audio tracks hand over when they change) resolve the 0.

A take whose tempo changed while it recorded has no one tempo (`TakeInfo::varied`, saved as `tempo=varies`). Its regions get tempo 0, and a region with tempo 0 plays Off whatever it says.

An audio clip has no setting of its own: each region has its own, as a song region does, so Song → scenes and To a cell carry the song's exactly. The clip card's one row (5c) sets every region (`AudioClip::set_follow`) and shows "mixed" when they differ (`AudioClip::follow`).

## The mapping

`Region::frames_to` stays what it was: the frames on the song from the region's start, counted along the tempo map. Fades, joins, cut counts (`into`, `left`) and a region's length on the song are measured in those.

A new position, the plan's `x(t)`, says which frame of the recording plays at song tick `t` (`Region::src_pos`): its `phase` plus

- Off: the frames along the tempo map from its start, as before;
- following the tempo: `(t − start) × 60 × rate / (bpm × PPQ)`, the ticks at its own tempo (`Region::per_tick`), whatever the song's tempo.

It is summed in f64 from the ticks every period, never added up a period at a time, so a region plays the same frames from any period size, after a locate and through a ramp. Every place that worked out a recording's frame now asks for it: the track player's read, a cut's `src` and `phase` (`cut`; Song → scenes' `SongSound::cut`), joins and seams (a region runs on into the next only on the same mapping), a clip voice's ring, the streaming heads and read ahead (in the recording's frames at the speed the song gives it now), the Song view's waves and a cell's small wave, To a cell's `last_frame` and the cues waiting for their audio.

## Repitch

A run (a stretch of a period with one tempo; the track player already cut runs at tempo changes) of a region following the tempo at a speed other than 1 reads through the streaming cursor's band-limited read, `Read::Sinc8`, on frames the track gathers itself (`Reader::add_pitched`): the frames under the run with the read's neighbours either side (`mpfree_stream::cursor::reach_of`), from the pool, the take's file or a part, then `mpfree_stream::cursor::read_gathered` on that slice. The cursor's `Cursor::play` calls the same function with its own fields, bit for bit as before.

- **One read at every speed.** `Sinc8` meets every row of the [pitched reads](pitched-reads.md) standard, so a region never changes read as the tempo moves and needs no fade between two, and a song region stays stateless. The pads' cheaper two-read scheme would need a 256-frame crossfade when the read changes, and so state a song region hasn't got; it is the fallback if the MPC's bench rows (5a.2) show `Sinc8` costs too much.
- **At speed 1** (the song at the region's own tempo) it takes Off's path, bit for bit, from the same frames (`at_its_own_tempo_a_region_following_the_tempo_plays_as_off_frame_for_frame`).
- **Range.** Past `MAX_SPEED`, four times (the cursor's), a run is silent and counted in `Stats::too_fast`, for the screens to say in 5c.
- **The real-time rules hold.** The gathered frames and the read's output have room made at start (`gathered`, `pitched`), and the kernels are built before the sound starts (`prepare_reads`); `no_alloc.rs` covers regions following the tempo through a tempo change.
- **Measured** through the whole track player as one of the [pitched reads](pitched-reads.md) (fxlab's `Reader::Tracks`: a take made at 100 BPM, the song at the speed's tempo): every row is the streaming cursor's band-limited read's.
- **Cost.** 8 tracks of comped takes repitched cost about 0.60 M instructions a 128-frame block at 0.9x and 0.78 M at 1.3x on the workstation, all of the track player, against 37 k played as recorded (the bench rows `Audio tracks: 8 playing comped takes, repitched to 0.9x` and `1.3x`). Their time on the MPC is on the next device round's list; if 8 don't fit beside a full project there, the pads' two-read scheme above is the fallback.

## Saving

- **The audio tracks' part:** a region line ends with `follow=repitch` or `follow=stretch`, `keep=pitch` and `bpm=` when its tempo isn't its take's, each only when not the default; a take line has `tempo=varies` when its tempo changed as it recorded.
- **The clips part:** a clip's `region` line ends with the same words, after where between frames it starts, `bpm=` only when it has a tempo of its own.
- **The gate.** A project with any of them says it needs `audio-tempo` ([project-safety.md](project-safety.md)), so a build from before phase 5 refuses it rather than play the audio at its recorded rate and drop the words at its next save. A project with none is written as before and needs nothing new.

## One history

- A song region's Follow tempo and tempo are region edits (`Command::SetFollow`, `Command::SetTempo`; the app's `AudioAction::Follow` and `AudioAction::Tempo`), each a step UNDO takes back, as Gain and Fades are.
- A clip's Follow tempo, and one of its region's tempos, go through the clip card's edit (`AudioSet::Follow`, `AudioSet::Tempo`): one take on the arrangement's history each.

## Tests

`rust/mpfree-audiotracks/tests/tempo.rs`: a 441 Hz tone made at 120 BPM and played at 90 fills its 2 bars at 330.75 Hz; a 4-bar recording with a click on every beat played across a tempo change halfway, a ramp and a locate into bar 3, every click on its beat's frame, none twice or missing, each at its own level; a clip launched with an offset plays its beats on the song's beats; a comp of takes made at 120 and 100 BPM plays on its beats at 110; at its own tempo a region following the tempo plays as Off, frame for frame; a take recorded across a tempo change plays Off. Besides: `save.rs` `follow_tempo_and_a_regions_own_tempo_come_back`, `mpfree-seq`'s `follow_tempo_is_set_for_the_whole_clip_and_each_region_keeps_its_tempo` and `a_region_following_the_tempo_is_cut_where_its_recording_is_at_its_tempo`, and in the app the needs gate, the region edits, the clip card's takes and a waiting cue's frames; fxlab's `the_tracks_repitch_reads_as_the_streaming_cursors_band_limited_read` and the tracks' rows in `quality/readers.tsv`. The scheduler's own tests (`mpfree-stretch` `sched`): a 2-bar job asked for while a long one runs finishes with only its own steps, the long one waiting; the order is priority, then deadline, then first asked; a newer job with a key replaces the old, which never finishes; one told to stop in its step stops at the step's end and never steps again; one with no memory for it is refused with why and never steps; retain and reorder; steps follow their time within their bounds; jobs left when the scheduler goes are told. Off plays bit for bit as before: every test from before passes unchanged, and so do the streaming cursor's tests and fxlab's readers and listening ratchets.

## Making copies ahead: one scheduler

Stretch plays a copy of its recording made at the tempo (phase 5b). A copy is cheap to keep and slow to make, so copies are made before they are needed, and `mpfree_stretch::sched` (5b.1) is where: one worker for every stretch MPFree makes ahead, the tracks' copies first, and in time the sampler's Warps and the looper's tempo copies, so they never compete for the cores. The plan's "Preparing ahead: one scheduler" says why; mpfree-claude note 18 has the order of the 5b pull requests.

- **Steps.** A job is `sched::Work`, done a step at a time (`Work::step(frames)`; a stretch is `mpfree_stretch::Job::run`). A step asks for about 10 ms of work (`Config::step`), its frames set from how long the last steps took, within 256 and 8192 frames (`Config::max_frames`; the sampler's Warp promises a copy stops within 8192 frames of being told to). After every step the most urgent job goes next, so a 64-bar backing track's stretch holds up a launch's 2-bar copy by one step at most.
- **The order:** the plan's four priorities (`Priority::Urgent`, a launch waiting; `Protect`, what something playing needs next; `Prepared`, what is set up to come; `Speculative`, the rest), then the nearest deadline (`Job::deadline`; a job with none after those with one), then the first asked. `Sched::reorder` moves a job when its use changes (a cell queued after it was asked for).
- **Keys.** A job has a `Key`: what it makes, for whom. A newer job with the key of one waiting or part done takes its place; one in its step stops at the step's end. The old one is told (`Work::cancelled`) and never finishes. `Sched::cancel` and `Sched::retain` stop jobs the same way. A client files each result under what made it as well, so one that comes after its clip changed is thrown away.
- **Memory.** A job says what it holds (`Job::needs`) and starts only with a promise of that much (`Config::promise`), kept until it ends; without one it is refused, counted, and told why (`Work::refused`). 5b.3 makes the app's promise the shared memory check's, with one budget for all derived audio.
- **Where it runs.** The worker (`mpfree-stretch`) starts with the first job and runs `Config::on_start` first on its own thread; 5b.3 gives it the sampler's sound lane's (off the sound's cores, at idle priority). If it can't start, the jobs wait and `Counts::spawn_failed` says so. `Sched::shared` is the process's one scheduler, configured at the program's start (`sched::configure`); a test makes its own, or a manual one it steps itself (`Sched::manual`, `Sched::step`).
- **Counts** (`Counts`): done, replaced, cancelled, refused, couldn't start, steps, for the Dev > Debug lines.

The shape is the sampler's job service's (`mpfree-sampler`'s `jobs.rs`: keys, the promise, `on_start`, one a process), with steps and priorities added, so moving Warps onto it (5b.3) changes how its jobs are run, not what they are.

## Not built yet

- The screens (5c): the clip card's Follow tempo row, the region's, and saying a region past `MAX_SPEED`.
- Stretch's copies in the track player (5b.2), with the stretch's timings (a 2-bar loop and a 30 s take at 0.8x and 1.25x, on the MPC in a device round), the one budget for derived audio (5b.3), and Keep pitch's launches that wait (5b.4).
