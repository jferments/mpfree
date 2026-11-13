# The arrangement, the Song view and live to song (T13, T11)

**Status:** built and tested on the workstation, 2026-11-12: the arrangement and the Song view (T13), then live to song (T11), then the audio tracks' lanes in the Song view (layout spec 4c). Nothing here has run on the MPC.

The launch grid plays clips as you trigger them. The arrangement puts them on a timeline, so the same project can also play as a fixed song. A placement in the song is linked to its slot, so an edit to the slot's clip shows wherever the song uses it, until the placement is made unique. Scenes and the song convert both ways, and live to song records what the grid plays into the song.

The model is in `rust/mpfree-seq/src/arrange.rs`, song-mode playback in `rust/mpfree-play`, live to song in `rust/mpfree-perform/src/song.rs`, and the Song view (with the audio tracks' lanes) in `rust/mpfree-time-ui/src/song.rs`.

## The model (`mpfree-seq`)

- **`Placement { track, start, length, offset, source }`** puts a clip on a track from song tick `start` for `length` ticks.
  - `source` is `Source::Slot(s)`, linked to slot `s` of its track, or `Source::Own(clip)`, its own copy.
  - A placement longer than its clip loops it.
  - `offset` starts it part way into the clip, in the clip's own ticks.
- **`Marker { at, name }`** names a place in the song, such as "Verse" or "Drop".
- **`Arrangement { placements, markers }`** is `Project::arrangement`. The placements are kept in order by track, then start, and never overlap on a track. Up to 4096 placements and 256 markers.
  - `place` puts a placement in its place, and refuses one that overlaps or is empty.
  - `remove`, `on_track`, `at(track, tick)` (the placement playing there) and `end`.
  - `make_unique(tracks, i)` gives placement `i` its own copy of its clip.
  - `unlink(tracks, track, slot)` gives every placement linked to a slot its own copy, before the slot is emptied.
  - `mark(at, name)` adds a marker.
  - `clear(tracks, track, from, to)` takes a track's placements out of a stretch. Placements inside it go. Placements across an edge are cut there, and a cut at the start moves the offset on, so the rest plays as before.
- **The song's own lanes.** `Arrangement::lanes` holds lanes for parameters that are not on a track. For now that is the morph crossfader (`MORPH_LANE`, on `ParamAddr::morph_pos(0)`).
  - The lanes are on song ticks and never loop (`SONG_LANE_LEN`, about 72 hours at 120 BPM).
  - `Arrangement::morphs` holds the two rows the morph is between, from each tick on (`MorphRows { at, a, b }`).
  - `put_morph` writes recorded moves over what was there, and the rows the song had after them go on.
- **An emptied slot.** A placement linked to a slot that becomes empty (an undone capture, say) stays in the song and plays nothing, until the slot has a clip again. `Project::check` lets it pass.
- `Arrangement::check` keeps the values in range (`ModelError::BadPlacement`), and `Project::check` runs it.

## Scenes and the song

- **Scenes → song (`scenes_to_song`).** Rows play one after another, each as a linked placement on every track that has a clip in the row, with a marker named after the scene ("Scene 3" when it has no name).
  - Each row is as long as `scene_length`: its follow action's bars, or else its longest clip rounded up to whole bars.
- **Song → scenes (`song_to_scenes`).** The song is cut into sections wherever anything starts or ends, and at every marker. Sections next to each other with the same placements are merged, unless a marker splits them, and silent sections are left out.
  - Each section becomes a row in the first run of empty rows long enough, with its clips copied in and the marker's name.
  - The rows are chained by follow actions: Next after the section's bars, and Stop on the last. Launching the first row plays the song.
  - With no run of empty rows long enough, it changes nothing and says so (`ModelError::NoRoom`).
  - A section that starts part way into a placement gets the whole clip, from its top.

## Song mode (`mpfree-play`)

`PlayView::set_song(project, on)` compiles the arrangement into each track's regions (`RegionView { end, clip }`), and turns song mode on or off. Own copies get slot numbers past the 16 real slots, so they never mix with a slot's clip.

- **A track with no clip launched plays the song.** The player plays the region at each tick, from the placement's start, looping, from its offset.
- **A launched clip takes the track over.** It plays in place of the song until it stops, and then the song plays on there from where it has got to.
- **Edges.** At a placement's end, notes still held end on that tick, the clip's locks end, and ratchet hits after it are dropped. Trig conditions count passes from each placement's start.
- **Moves chase.** A placement that starts part way into a lane starts it at the lane's value there.
- **Jumps.** A locate or a loop jump lands in whichever placement is at the new tick, at the right place in its clip.
- **The song's own lanes** play in song mode as events on track `SONG_TRACK` (one past the last track), after every track's. They chase after a locate. When song mode goes off, their parameters are let go (`Unlock`).
- **Real-time safe.** Song mode allocates nothing in `process`. `tests/no_alloc.rs` plays a track from placements, linked and unique, and the morph lane, under the counting allocator.

## Live to song (T11, `mpfree-perform`)

**Rec song** records what the launch grid plays into the song, so a jam becomes a song to edit. `SongRecorder` is control-side code. In the pad player, `TimeHost` feeds it from its period, which runs in the audio loop until edits move to a control thread. A period where nothing starts or stops allocates nothing (`tests/song_no_alloc.rs`), while a clip starting or stopping and writing the song at a stop allocate, like any edit there today. While it is armed and the transport plays, it follows each track:

- **Launches and stops.** A clip that starts on a track becomes a placement linked to its slot, from that tick and at the right place in the clip. That covers a launch, a scene and a follow action. The placement ends where the track switches clip or stops.
- **Mutes.** A muted stretch is left out of the song. The placement ends where the track is muted, and a new one starts where it is heard again, in phase with the clip. A scene's mutes come with its cue. A mute made by hand comes through `SongRecorder::mute`.
- **The morph.** Crossfader moves go into the song's morph lane, with the rows they morph between. A pause of a 16th note or more (`MORPH_HOLD`) is held, so a move after it starts from where the crossfader sat instead of ramping across the pause. A drag's moves come closer together, so a drag stays one ramp. A pass with no move leaves the lane as it was.
- **Tracks playing the song are not recorded.** On a track with no clip launched, the song stays as it is. Where a launched clip played a track, what the song had there is replaced, like recording over tape.
- **Passes.** A pass ends where the transport stops or jumps (a locate or a loop jump), and a new one starts where it lands. Each loop pass records over the last.
- **When it goes in.** Nothing goes into the project until the transport stops or Rec song is turned off (`SongRecorder::finish`). That way the song does not change under the player while it plays.
  - On an error (a full song), nothing changes and the recording is kept.
- **The song plays back what was heard.** `tests/live.rs` jams with launches, scenes with mutes, stops, a launch over the song and a mid-clip mute, then plays the song alone. The same notes come out on the same frames, at any period size.
- **Tracks muted at the end stay muted.** In the song their gaps already hold the mutes, so turn the mutes off to hear the song as it was played.

## The Song view (`mpfree-time-ui`)

On the Clips page, the **Song ›** button at the right of the grid's title swaps the grid for the Song view, and its **Clips** button swaps back.

- **The lanes.** Each track is a lane, its name on the left. Each placement is a block in the track's colour, with its clip's name. A unique placement has a small dot in its corner, and the selected one has an accent outline.
- **The ruler** shows the bar numbers and the markers' names in amber, and the playhead is a line over every lane (green while song mode is on).
- **Scrolling** moves the timeline a bar a step. It shows 16 bars, and the title line says how long the song is.
- **Tapping a block** selects it, and tapping an empty lane selects nothing.
- **The buttons along the bottom:**
  - **Clips** goes back to the grid.
  - **Song on/off** turns song mode on or off.
  - **Scenes → song** adds every row with a clip to the end of the song, from the next bar line after it.
  - **Song → scenes** puts the song's sections into empty rows.
  - **Make unique** and **Delete** act on the selected placement, and are dim when nothing is selected.
- **Rec song** sits at the right of the title line on the Song view, and beside the Song › button on the grid. It is red while armed.

### Audio lanes (layout spec 4c)

The audio tracks (`rust/mpfree-audiotracks`, the audio tracks thread's engine: see [audio-tracks.md](audio-tracks.md)) get lanes under the MIDI lanes, as Jay approved on 12 Nov (mockups A to C in `/mnt/project-files/audiotracks/mockup/`). The Song view draws them; the app wires the engine (`AudioHost`), the track panel, the region tools and the Q-Links.

- **What the app gives.** `TimeState::audio` is a list of `AudioLane`s, one for each audio track that has a region or is armed, in order: the track, its name and colour (`view::audio_color`), armed, its input (`mpfree_audiotracks::Input`, shown as the chip "L", "R", "L+R" or "ST": `song::input_chip`), the level for the header's bar (0 to 1), its level and pan, monitor mode, mute and solo (for the panel and the Q-Links), its blocks and, while it records, the punch in. `song::audio_blocks(track, regions, takes, map, rate)` makes the blocks from the engine's `regions(track)` and `takes()`: each keeps its ticks, take, source frame and fades, the frames it spans along the tempo map (`frames_from`), and a "take N" tag when it plays an older take than the track's newest (N counts the track's takes from 1, oldest first by group and pass). `TimeState::waves` holds the `TakeWave`s read so far (`AudioHost::wave`). `TimeState::audio_free` (seconds free and in all) goes on the legend as "Audio 0:34 of 1:00 free", and `TimeState::audio_counts` (the engine's refused, dropped and full, and the streaming misses and errors) are added to the legend while not zero, in the warning colour: a loss is never hidden.
- **The lanes.** Under the MIDI lanes, after a thin line, with the same ruler, playhead, bar scrolling and lane height (56 px at most, 32 px at least). The last lane is "+ Audio" while there are fewer than 8. A region is a block in the track's colour with the take's waveform across it (each pixel column shows the lowest and highest sample of the frames under it), its name (with the tag) over its top, and each fade as a slope with the part above it dimmed. While a track records, a red block runs from its punch in to the playhead.
- **The header** (the 96 px name column): an arm dot (filled red while armed), the name in the track's colour, the input chip, and a 3 px level bar along the bottom edge (red from 95 %).
- **The track panel.** While `TimeState::audio_panel` names a track with a lane, the Song view draws its panel over the lanes, right of the names: under the track's lane if it fits, else over it, else on the side with more room (`SongLanes::panel_parts`). It has the name with "armed" or "recording", Close, Arm, Mute, Solo, Takes, the input (`song::INPUTS`), the monitor mode (`song::MONITORS`), and the level and pan, each with a step down and up. The header whose panel is open has an accent outline. A touch on the panel goes to it alone; the names stay live, so another header opens its own panel. Sample screen: `arrange-song-panel`.
- **Scrolling the lanes.** When the lanes don't fit at 32 px each, a drag up or down the name column scrolls them, and a thin bar at the left edge shows where you are. The scroll is the widget's own (`SongLanes::first_lane`), kept across updates.
- **Touches** (`view::action`), as `TimeAction::Audio(AudioAction)`:
  - a region: `Pick(Some(RegionId { track, index }))` (an index into `regions(track)`);
  - the arm dot: `Arm { track, on }`; the name or the chip: `Panel(track)` (on the lift, unless the finger scrolled);
  - "+ Audio": `Add`;
  - the panel, which asks for its track (`audio_panel`): Close `ClosePanel`, Arm `Arm`, Mute and Solo `Mute { track, on }` and `Solo { track, on }` (on is the opposite of the lane's), Takes `Takes(track)`, an input `Input { track, input }`, a monitor mode `Monitor { track, monitor }`, and the level's and pan's steps `Turn { track, region, knob: 0 or 1, ticks: -1 or 1, fine: false }` (region: the picked region, if it is the track's);
  - with a region picked, the bottom row is Split · Erase · Fades · Gain · Takes · Undo: `Split(r)`, `Erase(r)`, `Fades(r)`, `Gain(r)`, `Takes(track)` and `Undo`. Rec song still works, and empty space picks nothing.
- **The selection has a kind.** `TimeState::pick()` gives `Pick::Placement(i)` or `Pick::Region(r)`; `TimeHost` keeps them apart (picking a region drops the placement, and `SelectPlacement` drops the region). The app's Q-Links follow it.
- **Waveforms are copied only when they change.** `song_lanes` leaves them out, and `TimePage::update` copies `st.waves` only when a take, its length or its detail differs (`song::same_waves`), as they can be large.
- **Sample screen:** `arrange-song-audio` (`sample::song_audio_state`) records real takes into a real `AudioTracks`: a guitar over bars 1 to 4, a vocal over bars 1 to 8 and again over 5 to 8 (that region picked, with fades), the guitar armed.

## In the pad player (`TimeHost`)

- `act(SongView)` swaps the view, and `act(SongMode)` turns song mode on and compiles.
- `act(ScenesToSong)`, `SongToScenes`, `MakeUnique` and `DeletePlacement` edit the project, and compile. If the edit or the compile fails, the project goes back to what it was, and the log says why.
- Every compile calls `PlayView::set_song`.
- `act(RecSong)` arms or disarms live to song. After every period the recorder sees the transport's block and the cues that started. When the transport stops, the recording goes into the song and compiles, and the log says how many placements and morph points went in (`TimeHost::take_song_note`).
- While recording, the morph bar's moves go into the song as they reach the mixer's mod engine (`TimeHost::morph`, see [scenes.md](scenes.md#the-scene-morph-t10-mpfree_performmorph)): each crossfader position, and new rows chosen while morphing.
- **Saved and opened with the project.** `mpfree_seq::save::to_text` writes the whole `Project` (tracks, clips, scenes, the song) as the `clips` part, and `from_text` reads it back, checked (`Project::check`), skipping lines it doesn't know. An open builds a new `TimeHost` from it on the screen thread (`TimeHost::with_project`) and the audio loop swaps it in with the transport stopped; the old one is freed on the screen thread. See [browser.md](browser.md#projects-in-the-pad-player).

## Tests

- `mpfree-seq` unit tests cover placing and refusing overlaps, a placement's origin with an offset, make unique and unlink, Scenes → song, and Song → scenes (sections, markers splitting them, follow actions and no room).
- `mpfree-play/tests/song.rs` plays the song through the transport and the player at periods of 1, 7, 64, 128, 1000 and 4096 frames. Every period size must give the same events. It checks:
  - placements on their ticks, looping and from their offset;
  - a note held over a placement's end, ending on it;
  - a launched clip taking a track over, and the song playing on after its stop;
  - the exact frames after a locate and a loop jump;
  - a lane chasing into a placement;
  - song mode off playing only the launches.
  - the song's morph lane chasing after a locate, and let go when song mode goes off.
- `mpfree-play/tests/no_alloc.rs` plays placements and the morph lane in song mode with no allocation.
- `mpfree-seq` unit tests also cover `clear` (cuts in phase) and `put_morph` (rows kept after a second pass).
- `mpfree-perform/tests/live.rs` records through the transport and the player and plays the song back. It checks:
  - launches, scenes with mutes, stops and a launch over the song: placed as played, and played back with the same notes on the same frames at periods of 1, 7, 128, 1000 and 4096;
  - a loop pass recording over the last;
  - a mute in the middle of a two-bar clip leaving a gap, and the clip coming back in phase;
  - morph moves going into the song's lane with their rows, held across a pause, and playing back.
- `mpfree-perform/tests/song_no_alloc.rs` records 200 periods where nothing starts, at periods of 1 to 4096 frames, under a counting allocator: none of them allocates.
- `mpfree-time-ui` tests cover the Song view's blocks, buttons and empty space, and swapping between the grid and the Song view.
- `mpfree-app`'s `rows_become_a_song_that_plays_and_goes_back_into_rows` runs it through `TimeHost` with the pads: Scenes → song, song mode, a launched clip taking a track over and the song playing on after its stop, Song → scenes, make unique and delete.
- `mpfree-app`'s `rec_song_records_the_grid_into_the_song` arms Rec song, launches two rows and stops. The song holds what was played and plays it back the same.
- `mpfree-app`'s `rec_song_keeps_the_morph_bar_moves_made_on_the_mixer` moves the morph bar on the real mixer's mod engine while Rec song records, changes rows, and checks the song's morph lane and rows.

## Not yet

- Audio lanes: the take lanes for comping (swiping a take) and dragging a region to move it. The app wires the engine (`rust/mpfree-app/src/audiotracks.rs`, see [audio-tracks.md](audio-tracks.md)).

- Moving, resizing and copying placements by touch, and adding or naming markers on screen. The model has them; the Song view only selects, makes unique and deletes for now.
- Undo for song edits.
- Recording notes straight into the song. REC records into the session's clips, and live to song records which clips played.
- Hand mutes in live to song. The pad player has no mute button for the time engine yet, so only a scene's mutes are recorded there.
- The morph's rows in song mode. The song lane plays the crossfader on the mixer's mod engine, but the rows the song keeps are not loaded at their ticks yet, so it morphs between the rows loaded at the time.
- Song mode itself (on or off) is not saved: the project keeps the song (placements, markers, morphs, song lanes) and the scenes, in the `clips` part (`mpfree_seq::save`, format `mpfree-clips 1`; see [browser.md](browser.md#the-parts)), and opens with song mode off.
