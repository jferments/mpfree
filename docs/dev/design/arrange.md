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
  - `clear(tracks, track, from, to)` takes a track's placements out of a stretch. Placements inside it go. Placements across an edge are cut there, and a cut at the start moves the offset on, so the rest plays as before (`Placement::from(tracks, at)` is the part after a cut).
  - `move_placement(tracks, i, track, start, copy)` moves placement `i`, or puts a copy of it, on a track from a tick, and returns its index. Onto another track a linked placement takes its own copy of the clip, since its slot number would name that track's clip. It refuses (`BadPlacement`) an overlap or a start before 0, and changes nothing then.
  - `set_length(i, length)` makes a placement longer or shorter, refused where it would run into the next one on its track.
  - `section(k, bar)` is marker `k` to the next, the last one to the song's end in whole bars from its marker. `move_section(tracks, k, gap, copy, bar)` moves section `k`, or puts a copy of it, before section `gap` (after the last for the markers' count), on every track: the sections are laid out again from the first marker in the new order, so nothing is written over, and what comes before the first marker stays. Placements across a section's edge are cut there first, in phase. The song list, the morph lane and the tempo points stay as they were. It refuses (`BadMoves`) while the song has morph moves or rows, or tempo points, which would not move with it.
- **The song's own lanes.** `Arrangement::lanes` holds lanes for parameters that are not on a track. For now that is the morph crossfader (`MORPH_LANE`, on `ParamAddr::morph_pos(0)`).
  - The lanes are on song ticks and never loop (`SONG_LANE_LEN`, about 72 hours at 120 BPM).
  - `Arrangement::morphs` holds the two rows the morph is between, from each tick on (`MorphRows { at, a, b }`).
  - `put_morph` writes recorded moves over what was there, and the rows the song had after them go on.
- **The song's tempo** (sequencer roadmap, tempo ramps). `Arrangement::tempo` holds `TempoPoint { at, bpm, ramp }`s in order, at least a 1/16 apart, at most `MAX_TEMPO_POINTS` (31, the tempo map's 32 segments less one for the project's tempo); `Arrangement::check` refuses anything else (`BadTempo`). From a point on the song plays its tempo, and with `ramp` it glides to the next point's tempo, arriving on that point; a ramp on the last point waits for a point after it. No points: the project's tempo throughout.
  - `tempo_map(project_bpm)` makes the transport's map: the project's tempo until the first point (unless that is on bar 1), then a segment a point, ramping where the point ramps ([transport.md](transport.md#tempo-ramps)).
  - `set_tempo(at, bpm, ramp)` puts a point in its place, or changes the one on that tick (keeping its ramp when `ramp` is None); refused (`BadTempo`) before the start, out of range (20 to 400 BPM), within a 1/16 of another point, or one too many. `remove_tempo(at)` takes one away.
  - Saved in the clips part as `tempo <tick> <bpm>[ ramp]` lines after the markers (`save.rs`); an older build skips them and plays the project's tempo.
  - The Song view's tempo lane (`mpfree-time-ui` `SongLanes::tempo`, `tempo_base`, `tempo_pick`; layout spec, the tempo lane): under the ruler, the song's tempo as a line with a dot on each point. A tap on the lane reports `Place { index: NONE, row: TEMPO_ROW, at }` (a new point on the nearest bar), a tap on a point `Cell { row: TEMPO_ROW, col }` (pick it), a drag `Place { index, row: TEMPO_ROW, at }` (move it to a bar). With a point picked the last two buttons read Ramp (lit while it ramps) and Delete.
  - `TimeAction::SongTempo(TempoEdit)` is what the lane's events ask for: `Add(at)`, `Pick(k)`, `Move { index, at }`, `Bpm { index, bpm }` (the wheel, with the point grabbed), `Ramp { index, on }`, `Delete(k)` and `AtPlayhead(bpm)` (the tempo turned in song mode). `TimeState::tempo_points`, `tempo_base` (the project's tempo), `tempo_pick` and `tempo_here` (the tempo of the point in effect at the playhead while song mode plays the points, which the Tempo row steps from: a ramp's start, not where it has glided to) carry them to the screen. See "In the pad player" below.
- **An emptied slot.** A placement linked to a slot that becomes empty (an undone capture, say) stays in the song and plays nothing, until the slot has a clip again. `Project::check` lets it pass.
- `Arrangement::check` keeps the values in range (`ModelError::BadPlacement`), and `Project::check` runs it.

## Scenes and the song

- **Scenes → song (`scenes_to_song`).** Rows play one after another, each as a linked placement on every track that has a clip in the row, with a marker named after the scene ("Scene 3" when it has no name).
  - Each row is as long as `scene_length`: its follow action's bars, or else its longest clip rounded up to whole bars.
- **Song → scenes (`song_to_scenes`).** The song is cut into sections wherever anything starts or ends, and at every marker. Sections next to each other with the same placements are merged, unless a marker splits them, and silent sections are left out.
  - Each section becomes a row in the first run of empty rows long enough, with its clips copied in and the marker's name. A row is empty by `Project::row_empty`, the one test every feature that looks for room uses (a copied row too): no clip on any track and a plain scene, so a row with only a name, mutes, a follow, morph values or a program change is never taken (`song_to_scenes_never_takes_a_row_set_up_without_clips`).
  - The rows are chained by follow actions: Next after the section's bars, and Stop on the last. Launching the first row plays the song.
  - With no run of empty rows long enough, it changes nothing and says so (`ModelError::NoRoom`).
  - A section that starts part way into a placement gets the whole clip, from its top.
- **The song list → song (`list_to_song`, sequencer S4).** The song list's steps (`Arrangement::list`) laid out as Scenes → song lays rows, each step for its span (`list_spans`: its scene's `scene_length` times its times), so a scene in the list twice is in the song twice. The list itself plays on its own: [make-perform.md](make-perform.md#the-song-list-as-built-the-sixth-part).

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

- **The lanes.** Each track is a lane, its name on the left with what it plays under it when the lane is at least 34 px tall (`SongLanes::sounds`, from `TrackInfo::sound`; with take lanes open the names stand alone). Each placement is a block in the track's colour, with its clip's name. A unique placement has a small dot in its corner, and the selected one has an accent outline. After the last track's lane, "+ Track" adds a track (`Tab(song::ADD_TRACK)`, `TimeAction::AddTrack`, as on the grid) while the project has fewer than 128 (`SongLanes::can_add_track`); the audio lanes and "+ Audio" come after it.
- **The ruler** shows the bar numbers and the markers' names in amber, and the playhead is a line over every lane (green while song mode is on).
- **Scrolling** moves the timeline a bar a step. It shows 16 bars, and the title line says how long the song is.
- **Tapping a block** selects it as the finger lifts, and tapping an empty lane selects nothing.
- **Drag and drop** (Jay, 14 Nov; layout spec 4h). A block, the picked block's end grip or a section's name on the ruler is held as the finger goes down (`SongLanes::grip`) and lifts once it moves `LIFT_PX` (12 px); a tap acts on the lift.
  - A block follows the finger along its lane, its start snapped to the nearest bar, or up and down to another MIDI lane. Its old place is outlined, where it lands is outlined in the accent colour with a "bar 9" tag, and the block is drawn under the finger. Let go: `Place { index, row: track, at: start }`, and nothing where it started or off the MIDI lanes. `TimeAction::MovePlacement`, with `copy` from SHIFT held as it lifts (the app sets it).
  - The picked block's end grip (two short bars at its right end, `end_rect`): its new end snapped to the bar, a bar line past its start at least, with an "N bars" tag. `Stretch { index, to: length }`, `TimeAction::StretchPlacement`.
  - A section's name (`marker_rect`): a tap is `Tab(SECTION_TAP + k)`, `TimeAction::PickSection` (a second tap lets it go). Dragged, its span is outlined on every lane, a line marks the section edge nearest its start, and a chip with its name and length follows the finger, kept clear of the line. `Drop { from: k, to: gap }`, `TimeAction::MoveSection`; a move to where it is changes nothing.
  - Held within 28 px of the left or right edge, the bars scroll one way, every 400 ms while it moves there: `Tab(BARS_BACK)` or `Tab(BARS_ON)`, which `TimePage::page_tracks` takes.
  - The wheel. Arrange has no outline (the wheel picks the record row), so with a block or a section picked the wheel's push grabs it instead of opening the row (`Player::song_grab`): a turn moves the block a beat (`MovePlacement`), or the section a place (`MoveSection`), and the next push lets go.
  - The hint line follows the pick (`view::song_hint`).
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
- **Take lanes.** While `TimeState::audio_takes` names a track with a lane, its takes (`AudioLane::takes`, from `song::take_lanes`) show as thin lanes (`TAKE_H`, 28 px) under it, newest first, each named in the name column ("take 2", with "· rec" while it records). A take lane shows the take's stretch with its waveform, brighter where the track plays it (`TakeLane::used`). The other lanes share what is left (32 px at least, then they scroll), so lane heights now differ: `lane_rect` adds them up from the first lane shown, and `max_first` is the last lane that can be first. A finger down on a take lane (past the names) starts a swipe, drawn as an outline, which ends on the lift as `Span { index: take, from, to }` in song ticks (`tick_at`), either way round; a lift within 8 px of where it went down is a tap, `Span` with `from == to`. Sample screen: `arrange-song-takes`.
- **Scrolling the lanes.** When the lanes don't fit at 32 px each, a drag up or down the name column scrolls them, and a thin bar at the left edge shows where you are. The scroll is the widget's own (`SongLanes::first_lane`), kept across updates.
- **Touches** (`view::action`), as `TimeAction::Audio(AudioAction)`:
  - a region: `Pick(Some(RegionId { track, index }))` (an index into `regions(track)`) as the finger goes down; dragged sideways more than 8 px and let go (an outline shows where it would land), `Move { region, to, fine: false }` with its new start (never before 0), from the widget's `Marker { index: track * REGION_SLOTS + index, frame: to }`; the app sets `fine` while SHIFT is held;
  - the arm dot: `Arm { track, on }`; the name or the chip: `Panel(track)` (on the lift, unless the finger scrolled);
  - "+ Audio": `Add`;
  - a take lane: a tap `UseTake(take)`, a swipe `Comp { take, from, to, fine: false }` (the app sets `fine` while SHIFT is held);
  - the panel, which asks for its track (`audio_panel`): Close `ClosePanel`, Arm `Arm`, Mute and Solo `Mute { track, on }` and `Solo { track, on }` (on is the opposite of the lane's), Takes `Takes(track)`, an input `Input { track, input }`, a monitor mode `Monitor { track, monitor }`, and the level's and pan's steps `Turn { track, region, knob: 0 or 1, ticks: -1 or 1, fine: false }` (region: the picked region, if it is the track's);
  - with a region picked, the bottom row is Split · Erase · Fades · Gain · Takes · Undo: `Split(r)`, `Erase(r)`, `Fades(r)`, `Gain(r)`, `Takes(track)` and `Undo`. Rec song still works, and empty space picks nothing.
- **The selection has a kind.** `TimeState::pick()` gives `Pick::Placement(i)` or `Pick::Region(r)`; `TimeHost` keeps them apart (picking a region drops the placement, and `SelectPlacement` drops the region). The app's Q-Links follow it.
- **Waveforms are copied only when they change.** `song_lanes` leaves them out, and `TimePage::update` copies `st.waves` only when a take, its length or its detail differs (`song::same_waves`), as they can be large.
- **Sample screen:** `arrange-song-audio` (`sample::song_audio_state`) records real takes into a real `AudioTracks`: a guitar over bars 1 to 4, a vocal over bars 1 to 8 and again over 5 to 8 (that region picked, with fades), the guitar armed.

## In the pad player (`TimeHost`)

- `act(SongView)` swaps the view, and `act(SongMode)` turns song mode on and compiles.
- `act(ScenesToSong)`, `SongToScenes`, `MakeUnique`, `DeletePlacement`, `MovePlacement`, `StretchPlacement` and `MoveSection` edit the project, and compile. If the edit or the compile fails, the project goes back to what it was, and the log says why (the screen says it in place of the hint for a drag or a delete).
- **Undo.** Every one of those but Song → scenes (which changes the rows) is an edit take holding the song before and after it (`Takes::song_edit`, `Take::song` with a `SongChange` naming it: "Move 5 · Bass, to bar 9" in the takes list). UNDO puts the song back as it was and redo as it became, while nothing else has changed the song in between; otherwise they leave it. A moved block stays picked at its new index, and a moved section stays picked; an undo or redo of a song edit drops the pick.
- **The song's tempo.** `act(SongTempo(..))` edits the song's tempo points (`TimeControl::edit_tempo`): a tap on the lane adds a point on the bar with the tempo playing there (rounded to 0.1 BPM) and picks it; a point dragged onto a bar with a point, or within a 1/16 of one, is refused with why. Each is a song edit UNDO takes back (`SongChange::Tempo { at }`, "Tempo 4 · bar 9" in the takes list); the wheel's turns on one point and the tempo's turns at the playhead are one undo (`Takes::song_edit_joining`, keyed by the point's tick, while nothing else changed the song). Tempi are kept to the app's range, 30 to 300 BPM.
  - **Song mode plays them.** Every compile puts `Arrangement::tempo_map(project tempo)` in the view while song mode is on and the song has points (`PlayView::set_tempo_map`), so the map reaches the audio half with the song it goes with. Each period `TimeAudio::follow_tempo` hands a new map to the transport (`Transport::change_tempo_map`: on the next 1/16 while rolling, at once while stopped) and, when the view has none, puts the app's tempo back throughout (a map left in the transport is flattened too). A map on its way counts as the transport's, so it is asked for once; a tempo asked for after it (a followed clock's) forgets it. While a followed clock arrives its tempo wins, as before. The looper's start on a note (`start_at`) leaves out its tempo while the song's points play: they lead.
  - **The project's tempo** is the app's (`App::settings.bpm`: Play's tempo with no points, TAP TEMPO, a project opened), and plays before the first point. `TimeHost::follow_tempo` gives it to the control half each period (`TimeControl::project_tempo`), which compiles again only when it changed while song mode plays points after bar 1.
  - **The tempo turned in song mode** (Jay's card, 18 Nov 10:17Z, "Set the tempo at the playhead"): while song mode plays the points (`TimeHost::song_tempo_plays`), the app turns `SetTempo` from the Tempo row, a typed tempo or a learned tempo knob into `SongTempo(AtPlayhead(bpm))`: the point in effect at the playhead takes it, or before the first point a new one on the playhead's bar. The project's tempo stays. TAP TEMPO still sets the project's tempo.
  - **What follows the transport's tempo.** The synth's tempo and the effects' (`mixing::ctx`) are the transport's (`TimeHost::info().bpm`), so a synced delay and the synth's synced LFOs follow the points and ramps; warped pads already did. Note Repeat and an arp free-running while stopped keep the app's tempo; playing, the arps follow the transport's clock.
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
- `mpfree-seq` unit tests cover moving and copying a placement along and across lanes, a length up to the next block, and moving and copying sections (cut in phase, what comes first kept, refused with morph moves).
- `mpfree-time-ui` tests cover the Song view's blocks, buttons and empty space, swapping between the grid and the Song view, and the drags: a wobble still a tap, a block to the bar and to another lane, the end grip, sections picked and dropped, the edge scroll and the hint.
- `mpfree-app`'s `song_drags_move_blocks_and_sections_and_undo_takes_them_back` runs the drags through `TimeHost` with their messages, a refusal, and undo and redo back to the start; `song_drags_copy_with_shift_and_the_wheel_moves_the_pick` checks SHIFT and the wheel's grab on the screen.
- `mpfree-app`'s `rows_become_a_song_that_plays_and_goes_back_into_rows` runs it through `TimeHost` with the pads: Scenes → song, song mode, a launched clip taking a track over and the song playing on after its stop, Song → scenes, make unique and delete.
- `mpfree-app`'s `rec_song_records_the_grid_into_the_song` arms Rec song, launches two rows and stops. The song holds what was played and plays it back the same.
- `mpfree-app`'s `rec_song_keeps_the_morph_bar_moves_made_on_the_mixer` moves the morph bar on the real mixer's mod engine while Rec song records, changes rows, and checks the song's morph lane and rows.
- `mpfree-app`'s `song_mode_plays_the_songs_tempo_points` edits points through `TimeHost` (add, the wheel's turns as one undo, Ramp), plays them in song mode (the project's tempo, then 32 steps up a two-bar ramp, then the last point's), turns the tempo at the playhead (one undo, heard on the next 1/16), changes the project's tempo before the first point, adds a point on bar 1 from there, and turns song mode off (the project's tempo throughout, the points kept). `mpfree-play`'s `a_tempo_point_turned_on_the_wheel_is_one_undo` checks the joining; `mpfree-time-ui`'s `song_drags_ask_to_move_and_the_hint_follows_the_pick` the lane's events, Ramp, Delete and the hint. Preview screen `arrange-song-tempo` (a ramp picked); the control sweep presses the lane, its points and their Ramp and Delete there.

## Not yet

- Audio lanes: dragging a region to another track. The app wires the engine (`rust/mpfree-app/src/audiotracks.rs`, see [audio-tracks.md](audio-tracks.md)).

- Adding or naming markers on screen.
- Moving sections with morph moves in the song (their lanes would need cutting and shifting with them).
- Recording notes straight into the song. REC records into the session's clips, and live to song records which clips played.
- Hand mutes in live to song. The pad player has no mute button for the time engine yet, so only a scene's mutes are recorded there.
- The morph's rows in song mode. The song lane plays the crossfader on the mixer's mod engine, but the rows the song keeps are not loaded at their ticks yet, so it morphs between the rows loaded at the time.
- Song mode itself (on or off) is not saved: the project keeps the song (placements, markers, morphs, song lanes, the song list) and the scenes, in the `clips` part (`mpfree_seq::save`, format `mpfree-clips 1`; see [browser.md](browser.md#the-parts)), and opens with song mode off.
