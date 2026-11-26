<!--
covers: rust/mpfree-audiotracks/src/lib.rs rust/mpfree-audiotracks/src/clips.rs rust/mpfree-audiotracks/src/save.rs rust/mpfree-audiotracks/tests/tempo.rs rust/mpfree-stream/src/cursor.rs rust/mpfree-seq/src/audio.rs rust/mpfree-seq/src/arrange.rs rust/mpfree-app/src/audiotracks.rs rust/mpfree-app/src/time/audio_wait.rs rust/mpfree-fxlab/src/readers.rs rust/mpfree-bench/src/cases.rs
checked: 2026-11-25 77de55c0
-->
# Audio following the tempo

**Status:** phase 5a of the audio tracks rethink, built 2026-11-25 (the mapping, Follow tempo, Repitch, saving, the one history, the read's measures and bench rows), with no screen yet: phase 5c draws it. Stretch plays as Repitch until phase 5b makes its copies. Workstation only; it has not run on the MPC One.

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

`rust/mpfree-audiotracks/tests/tempo.rs`: a 441 Hz tone made at 120 BPM and played at 90 fills its 2 bars at 330.75 Hz; a 4-bar recording with a click on every beat played across a tempo change halfway, a ramp and a locate into bar 3, every click on its beat's frame, none twice or missing, each at its own level; a clip launched with an offset plays its beats on the song's beats; a comp of takes made at 120 and 100 BPM plays on its beats at 110; at its own tempo a region following the tempo plays as Off, frame for frame; a take recorded across a tempo change plays Off. Besides: `save.rs` `follow_tempo_and_a_regions_own_tempo_come_back`, `mpfree-seq`'s `follow_tempo_is_set_for_the_whole_clip_and_each_region_keeps_its_tempo` and `a_region_following_the_tempo_is_cut_where_its_recording_is_at_its_tempo`, and in the app the needs gate, the region edits, the clip card's takes and a waiting cue's frames; fxlab's `the_tracks_repitch_reads_as_the_streaming_cursors_band_limited_read` and the tracks' rows in `quality/readers.tsv`. Off plays bit for bit as before: every test from before passes unchanged, and so do the streaming cursor's tests and fxlab's readers and listening ratchets.

## Not built yet

- The screens (5c): the clip card's Follow tempo row, the region's, and saying a region past `MAX_SPEED`.
- Stretch's copies (5b), with the stretch's bench rows (a 2-bar loop and a 30 s take at 0.8x and 1.25x), which measure the job 5b runs.
