# The arrangement and the Song view (T13)

**Status:** built and tested on the workstation, 2026-11-12. Nothing here has run on the MPC.

The launch grid plays clips as you trigger them. The arrangement puts them on a timeline, so the same project can also play as a fixed song. A placement in the song is linked to its slot, so an edit to the slot's clip shows wherever the song uses it, until the placement is made unique. Scenes and the song convert both ways.

The model is in `rust/mpfree-seq/src/arrange.rs`, song-mode playback in `rust/mpfree-play`, and the Song view in `rust/mpfree-time-ui/src/song.rs`.

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
- **Real-time safe.** Song mode allocates nothing in `process`. `tests/no_alloc.rs` plays a track from placements, linked and unique, under the counting allocator.

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

## In the pad player (`TimeHost`)

- `act(SongView)` swaps the view, and `act(SongMode)` turns song mode on and compiles.
- `act(ScenesToSong)`, `SongToScenes`, `MakeUnique` and `DeletePlacement` edit the project, and compile. If the edit or the compile fails, the project goes back to what it was, and the log says why.
- Every compile calls `PlayView::set_song`.

## Tests

- `mpfree-seq` unit tests cover placing and refusing overlaps, a placement's origin with an offset, make unique and unlink, Scenes → song, and Song → scenes (sections, markers splitting them, follow actions and no room).
- `mpfree-play/tests/song.rs` plays the song through the transport and the player at periods of 1, 7, 64, 128, 1000 and 4096 frames. Every period size must give the same events. It checks:
  - placements on their ticks, looping and from their offset;
  - a note held over a placement's end, ending on it;
  - a launched clip taking a track over, and the song playing on after its stop;
  - the exact frames after a locate and a loop jump;
  - a lane chasing into a placement;
  - song mode off playing only the launches.
- `mpfree-play/tests/no_alloc.rs` plays placements in song mode with no allocation.
- `mpfree-time-ui` tests cover the Song view's blocks, buttons and empty space, and swapping between the grid and the Song view.
- `mpfree-hello`'s `rows_become_a_song_that_plays_and_goes_back_into_rows` runs it through `TimeHost` with the pads: Scenes → song, song mode, a launched clip taking a track over and the song playing on after its stop, Song → scenes, make unique and delete.

## Not yet

- Moving, resizing and copying placements by touch, and adding or naming markers on screen. The model has them; the Song view only selects, makes unique and deletes for now.
- Undo for song edits.
- Recording into the song. Recording goes into the session's clips. Live to song (T11) records launches, mutes and morphs into the arrangement.
- Song mode is not saved with the project yet: the project has no file format so far.
