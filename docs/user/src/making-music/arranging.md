<!--
status: changing
covers: rust/mpfree-seq/src/scenes.rs rust/mpfree-seq/src/arrange.rs rust/mpfree-perform rust/mpfree-time-ui/src/song.rs rust/mpfree-time-ui/src/widgets.rs
checked: 2026-11-14 b76c3c7
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

Audio you record from the INPUTS goes on **audio tracks**, with lanes of their own under the clip lanes, on the same timeline. They play whenever the song plays. Recording them, editing their regions and picking the best parts of several takes is in [Recording audio tracks](audio-tracks.md).

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
