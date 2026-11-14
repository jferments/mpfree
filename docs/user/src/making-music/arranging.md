<!--
status: changing
covers: rust/mpfree-seq/src/scenes.rs rust/mpfree-seq/src/arrange.rs rust/mpfree-perform rust/mpfree-time-ui/src/song.rs rust/mpfree-time-ui/src/widgets.rs
checked: 2026-11-13 f8cc558
-->

# Arranging a song

The Arrange tab turns clips into a song in two ways. You can play it live, launching whole rows of clips (scenes) and morphing between their sounds. Or you can lay the clips out on a timeline and let the song play itself. You can also do both at once: jam on the grid and record the jam as a song.

## Scenes

A row of the clip grid is a scene. Tap a row's number on the left and every clip in that row starts on the next bar line, so the whole part of the song changes at once.

{{shot: clips | The Arrange tab's clip grid, with the morph bar under it.}}

- The square button above the row numbers stops every track on the next bar line.
- A scene can also switch which tracks are muted on its bar line, and move on by itself after some bars (to the next row, back to the first, or to a random one). Setting these up on screen is coming. For now a row plays until you launch another. **Song → scenes** (below) sets rows up to play one after another.

## The scene morph

The morph bar under the grid glides the sound from one scene to another, like a crossfader for your mix. Each row can store mixer settings: levels, pans, sends and effect controls.

1. Pick the two rows with the **A** and **B** chips. Tap a chip's left half to step its row down, its right half to step it up.
2. Move the crossfader. Every setting either row stores moves between A's value and B's.
3. Let go in one of two ways:
   - **Snap back** returns to the sound from before you started, including any knob you turned while morphing.
   - **Keep** leaves everything where the morph put it.
4. **Save A** or **Save B** stores the sound you have now in the row the crossfader is nearer.

A knob you turn while morphing changes the scene the crossfader is nearer, and that change stays in the row.

## The song

The song is your clips laid out on a timeline, one lane per track. Tap **Song ›** above the clip grid to see it, and **Clips** to go back to the grid. **+ Track**, under the last track's lane, adds a track, as on the grid. With more tracks than fit, drag up or down on the names to scroll.

{{shot: arrange-song | The Song view: five rows made into a song, a drum block selected, and Rec song on.}}

- **Scenes → song** adds every row that has clips to the end of the song, one after another, with each scene's name above the bars where it starts.
- **Song on** plays the song. A track plays the song wherever you have not launched a clip on it. Launch a clip and it takes that track over. Stop it and the track goes back to the song.
- **Song → scenes** goes the other way. It cuts the song into its parts and puts each part into empty rows, set to play one after another. It never writes over a row that has clips.
- Hold **SHIFT** and turn the wheel to scroll through the bars. Tap a block to select it.
- Under each track's name is what it plays: **Kit**, or its own instrument (see [A synth for each track](synths.md#a-synth-for-each-track)).

A block in the song is linked to its clip, so when you edit the clip, every block that uses it changes too. **Make unique** gives the selected block its own copy, so you can change it without touching the others. A unique block has a small dot in its corner. **Delete** removes the selected block.

> [!MPC]
> On the MPC, a song is a list of sequences played one after another. In MPFree, the song is a timeline of clips, and each track can leave the song to play a launched clip and come back.

### Audio in the song

Audio you record from the line input goes on **audio tracks**, which have their own lanes under the clip lanes, on the same timeline. A lane shows once its track is armed or has audio on it. Audio tracks play in song mode only: with song mode off they stay silent, though an armed track still lets you hear its input.

{{shot: arrange-song-audio | The Song view with two audio lanes: a vocal recorded twice, its newer part picked, and a guitar armed.}}

To record onto an audio track:

1. Turn on **Prefs > Line input** (SHIFT + MENU). The input stays off until you turn it on.
2. On Arrange, open the Song view and turn song mode on.
3. Tap **+ Audio** at the bottom of the lanes to add a track, then tap its **arm dot** so it turns red.
4. Press **REC**, then **PLAY**, and play. Press **STOP** when you're done. The take shows as a region with its waveform.

- Each recording is a **region**: a block in the track's colour with its waveform. When an older take plays, the block says which one ("take 1"). A fade shows as a slope.
- While a track records, a red block grows from where it started.
- On the left of each lane: the **arm dot** (red while armed: tap it to arm or disarm), the track's name, its input (**L** by default: the MPC One's line input records on the left jack) and a thin level bar along the bottom. The bar shows what the track plays, and while it is armed, the input it records, so you can check your level before you press REC. It turns red near full scale.
- **+ Audio** at the bottom adds the next audio track, up to 8.
- Tap a track's name (or its input) to open its **track panel** over the lanes. Tap the name again, or **Close**, to shut it.
  - **Arm**, **Mute** and **Solo**. Solo works among the audio tracks: a soloed track silences the other audio tracks, not the drums or the synth.
  - **Input**: what the track records from. **L** (the left jack, the default), **R**, **L+R** (both jacks mixed into one) or **ST** (stereo). The input can't change while the track records.
  - **Monitor**: when you hear the input through the track. **Auto** (the default) plays it while the track is armed, except during playback. **In** always plays it, armed or not. **Off** never does.
  - **Level** and **Pan**, with a step either way. Q1 and Q2 turn them too while the panel is open, and Q3 and Q4 turn a region's gain and fades once you pick one of the track's regions.
  - **Takes** opens the track's take lanes (below).

{{shot: arrange-song-panel | The guitar's track panel: armed, soloed and set to stereo, with its level and pan on the Q-Links.}}

- Tap a region to pick it. The buttons along the bottom become **Split**, **Erase**, **Fades**, **Gain**, **Takes** and **Undo** for that region. Tap empty space to go back to the song's own buttons.
  - **Split** cuts the region at the playhead, so put the playhead inside it first.
  - **Erase** takes the region out. **Undo** brings it back, and **SHIFT + Undo** redoes what Undo took back.
  - **Drag** a region sideways to move it. It snaps to sixteenth notes; hold SHIFT to move it freely. It stays picked where it lands, and **Undo** puts it back.
  - While a region is picked, the four Q-Links on the right turn the track's **Level** (Q1) and **Pan** (Q2), and the region's **Gain** (Q3) and **Fades** (Q4, the fade in and the fade out together). Hold SHIFT for fine steps. **Gain** and **Fades** in the bottom row point you to those knobs.
- **Takes** (in the track panel, or in the bottom row with a region picked) opens the track's **take lanes**: a thin lane for each recording under the track's lane, newest first. Each take lane shows that take's waveform, brighter where the track plays it now. Tap **Takes** again to close them.
  - **Swipe** across a take lane to play that stretch of the take on the track, in place of what was there. The swipe snaps to sixteenth notes; hold SHIFT for a free swipe.
  - **Tap** a take lane to play that take everywhere it has audio.
  - This is called comping: record a part a few times, then keep the best bits of each. **Undo** takes each swipe back.

{{shot: arrange-song-takes | The vocal's take lanes: take 2 plays over bars 5 to 8 and take 1 everywhere else.}}

- When there are more lanes than fit, drag up or down on the names to scroll them.
- All the audio tracks play through one **Audio** strip in the mixer (TRACK MIX).
- Audio tracks make up for the line input's delay with **Line input latency** in [Prefs](../reference/prefs.md), so a take lines up with the beat you played it against. It's the same setting as the Looper's **Latency to make up for** (MENU on the Looper tab): change either and both follow.
- The audio tracks share one minute of stereo memory; **Audio track memory** in [Prefs](../reference/prefs.md) sets 30 seconds to 10 minutes, from the next start. The Song view's legend says how much is free ("Audio 0:42 of 1:00 free"). A recording that runs out of memory stops there and keeps what it got.

- The audio tracks are saved with the project (**SHIFT + BROWSE**): every take, the regions you comped from them, and each track's input, monitor, level, pan, mute and solo. If a track is recording when you save, the save waits until it stops. Opening a project ends any recording and replaces the audio tracks with the project's; New project empties them.

## Recording a performance into the song

**Rec song** records what you play on the clip grid into the song, so a jam becomes a song you can edit afterwards. The button sits above the clip grid and on the Song view, and it is red while it is on.

1. Tap **Rec song**.
2. Press play and jam: launch clips and rows, stop tracks, move the morph crossfader.
3. Press stop, or tap **Rec song** again. What you played goes into the song.

- Each clip goes into the song where it started and stopped, linked to its slot. It plays from the song just as it does from the grid, pad pressure included.
- Tracks that were muted are left out of the song for as long as they were muted.
- The morph crossfader's moves are recorded too, with the two rows it was between. If you leave the crossfader still for a moment, the song holds it there until your next move.
- A track you didn't touch keeps what the song already had there. Where you launched a clip, it records over the song, like recording over tape.
- With the loop on, each time round records over the last.
- Rec song records which clips played, not new notes. To record notes, use **REC** on a clip (see [Clips and patterns](clips.md)).

> [!NOTE]
> A track that is muted when you stop stays muted. Its silent parts are already left out of the song, so unmute it to hear the song as you played it.

## Coming

- Moving, stretching and copying blocks by touch, and naming the places in the song.
- Undo for changes to the song.
- Playing the morph's rows back from the song. The song plays the crossfader's moves, between whichever two rows are picked.
