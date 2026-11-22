<!--
status: changing
covers: rust/mpfree-seq/src/scenes.rs rust/mpfree-seq/src/arrange.rs rust/mpfree-perform rust/mpfree-time-ui/src/song.rs rust/mpfree-time-ui/src/widgets.rs rust/mpfree-time-ui/src/list.rs
checked: 2026-11-19 79fc6f63
-->

# Arranging a song

The Arrange tab turns clips into a song in three ways. You can play it live, launching whole rows of clips (scenes) and morphing between their sounds. You can line scenes up in a song list ("Intro × 2, Verse × 4"), as on an MPC. Or you can lay the clips out on a timeline and let the song play itself. You can also do both at once: jam on the grid and record the jam as a song.

## Scenes

A row of the clip grid is a scene. Tap a row's number on the left and every clip in that row starts on the next bar line (or the line the Record panel's **Launch** sets, see [Recording a clip](clips.md#recording-a-clip)), so the whole part of the song changes at once.

{{shot: clips | The Arrange tab's clip grid in a jam: clips playing (a ring turning), two waiting for the next bar ("from 4.1"), Chop B recording, loop cells from the looper, and empty slots, with the morph bar under it.}}

- The square button above the row numbers stops every track on the next bar line.
- A scene can also switch which tracks are muted on its bar line (a track with no clip in the row too: in song mode it plays the song until the line, and its pad and name show the mute waiting, as TRACK MUTE's do), send your outside synths and drum machines a program change there, have a name, and move on by itself after some bars (to the next row, back to the first, or to a random one). Set these up on the Record panel's **Scene** tab (below). **Song → scenes** (below) also sets rows up to play one after another, and the [song list](#the-song-list) plays them in any order.

### Setting up a scene

Tap **Scene**, the third tab on the Record panel's title line, to the right of the grid. It shows the scene playing, or scene 1 when nothing plays. Launching a scene or picking a step of the song list brings that scene.

{{shot: clips-scene | The Record panel's Scene tab: scene 2, named Drop, moves on to the next scene after 4 bars four times in five, and mutes Track 2 when it starts.}}

Turn the wheel to pick a row, and press **–** or **+** to change it:

- **Scene**: which scene you are setting up. Its name shows after its number.
- **Follow**: what plays after this scene has played its bars: off (it plays until you launch another), the next scene, the previous one, the first or the last, any scene, any other scene, this one again, or Stop. Turning it on sets **After** to the scene's longest clip.
- **After**: how many bars the scene plays before Follow happens.
- **Chance**: how often Follow happens, in steps of 10 %.
- **Or**: what happens the rest of the time, when Chance is under 100 %. **Stay** plays the scene again for the same bars and then tries again.
- **Mutes**: "as they are" leaves the mutes alone when the scene starts. **+** gives the scene the mutes you have now.
- **Program**: the program change the scene sends the armed track's synth or drum machine when it starts, **1** to **128**, or **off** for none. Arm another track to set its own: each track can have one. It shows a dash when no track is armed.
- **Bank**: the bank select sent just before that program, **0/0** to **127/127** (the two numbers sent, as on the clip's **Bank**), or **off**. Hold **SHIFT** as you press **–** or **+** to step the first number.
- **Sent to**: the tracks this scene sends a program change to.

{{shot: clips-scene-program | Further down the Scene tab: scene 2 sends the armed track's synth program 12 when it starts, with no bank select.}}

A scene's program change goes out on the bar line the scene starts on, after the old clip's last notes and before the new clip's first, so the synth plays the new part with the new sound. A track with no clip in the row gets its program change too, which lets the scene switch a drum machine that plays its own patterns. If the clip that starts on a track has its own **Program** (see [Clip settings](clips.md#clip-settings)), the clip's goes out instead. They go out again when you start the music after stopping, but not on every loop. Only tracks that play on MIDI Out send them (their **Plays on**, with **Send MIDI and CV out** on in Prefs): see [MIDI and CV](midi-and-cv.md).

**Mutes now**, under the rows, also gives the scene the mutes you have now: mute the tracks you want silent in this part, then tap it. When the scene starts, those tracks mute and the others play. **Rename** opens the keyboard: type the scene's name and tap **Enter**. The name shows on the song list, and becomes the section's name when the scene goes into the song.

Every change plays at once, and **UNDO** takes it back. Turning one row many times is one undo. While a scene plays, new mutes wait until it starts again.

## The scene morph

The morph bar under the grid glides the sound from one scene to another, like a crossfader for your mix. Each row can store mixer settings: levels, pans, sends and effect controls.

1. Pick the two rows with the **A** and **B** chips. Tap a chip's left half to step its row down, its right half to step it up.
2. Move the crossfader. Every setting either row stores moves between A's value and B's.
3. Let go in one of two ways:
   - **Snap back** returns to the sound from before you started, including any knob you turned while morphing.
   - **Keep** leaves everything where the morph put it.
4. **Save A** or **Save B** stores the sound you have now in the row the crossfader is nearer.

**Reload clip**, at the right of the morph bar, brings the armed track's clip back to the state you saved with **Save state** (see [Save state and Reload](clips.md#save-state-and-reload)).

A knob you turn while morphing changes the scene the crossfader is nearer, and that change stays in the row.

## The song

The song is your clips laid out on a timeline, one lane per track. Tap **Song ›** above the clip grid to see it, and **Clips** to go back to the grid. **+ Track**, under the last track's lane, adds a track, as on the grid. With more tracks than fit, drag up or down on the names to scroll.

{{shot: arrange-song | The Song view: seven rows made into a song (Intro, Verse, Build and on), eight tracks of blocks under the scene names, and Rec song on.}}

- **Scenes → song** adds every row that has clips to the end of the song, one after another, with each scene's name above the bars where it starts. A scene's mutes carry on into the rows after it, as they do when the rows play, so a track a scene muted has no blocks until a scene unmutes it. A one-shot clip's block ends after its one pass.
- **Song on** plays the song. A track plays the song wherever you have not launched a clip on it. Launch a clip and it takes that track over. Stop it and the track goes back to the song. While clips play over the song, **Back to song** shows on the grid's title line: it stops them all on the next Launch line, and every track plays the song from there.
- **Song → scenes** goes the other way. It cuts the song into its parts and puts each part into empty rows, set to play one after another. It never writes over a row that has clips, or one you have set up without clips (a name, mutes, a follow action, morph values or a program change). A part that starts partway into a block gets its clip turned round to start there, so launching the first row plays what the song played. **UNDO** takes the whole thing back in one go, and **SHIFT + UNDO** puts it back.
- Every conversion tells you whether it came out **exact**. When something can't carry over, the message names it, with the track and the bar: a part off the bar line, a stretch of silence, the song's tempo changes or morph moves, a note held over a row change, a clip that plays in Random order or has notes with a chance (a copy rolls them its own way), or, going into the song, a scene's program changes, a clip's own follow action or a legato clip.
- Hold **SHIFT** and turn the wheel to scroll through the bars. Tap a block to select it. It is picked when you lift your finger, because you can also drag it.
- Under each track's name is what it plays: its kit (**Kit** for kit 1, then **Kit 2** and on: see [A kit for each track](pads.md#a-kit-for-each-track)), **Synth**, or its own instrument (see [A synth for each track](synths.md#a-synth-for-each-track)).
- **Loop tracks** have lanes too, and Song on plays their loops' blocks as it plays clips (see [Loops in the song](looping.md#loops-in-the-song)).

A block in the song is linked to its clip, so when you edit the clip, every block that uses it changes too. Moving the clip on the grid, to another row or track, doesn't change the song: its blocks play it wherever it is. **Make unique** gives the selected block its own copy, so you can change it without touching the others. A unique block has a small dot in its corner. **Delete** removes the selected block, and **UNDO** puts it back. In Perform, Delete takes a half-second hold (see [Make and Perform](playing-live.md#make-and-perform)).

### Moving blocks and sections

You can move the song around with your finger:

- **Drag a block** to move it. Once your finger moves, it lifts and follows you, and its new place is outlined, with a tag saying the bar it starts on. It snaps to the bar. Drag it up or down to put it on another track's lane (a loop's block stays on its Loop track's lane, and a clip's block stays off them). There it plays that track's sound and stays linked to the same clip, so editing the clip still changes both. Hold **SHIFT** as you lift to drop a copy instead, linked to the same clip. Lift it where it started to leave it be. A block can't land on top of another one.
- **Drag a picked block's end** to make it longer or shorter, a bar at a time. The picked block has a grip (two short bars) at its right end. Past the clip's end, the clip plays again from its top.
- **Drag a section's name** on the ruler to move the whole section, every track, from its marker to the next one. A line shows where it will go. The sections close up behind it and make room for it, so nothing is written over. Hold **SHIFT** as you lift to put a copy there instead, such as a second verse. Everything in the section goes with it: its blocks, the audio on its audio tracks, its marker, its tempo changes and its morph moves, so it sounds the same in its new place. A block that runs across a section's edge is cut there, and each part still plays as it did; where the parts of one block meet again in step, they stay one block. If a tempo ramp runs past the section's end, its last 1/16 holds steady so the section still ends on time.
- Hold a dragged block or section at the left or right edge and the bars scroll.
- With the wheel: pick a block, push the wheel to grab it, turn to move it a beat at a time, and push again to let go. Tap a section's name to pick it, then push and turn to move it a section at a time.
- **UNDO** takes back any of these, one at a time, and **SHIFT + UNDO** puts them back again.

> [!MPC]
> On the MPC, a song is a list of sequences played one after another. In MPFree, the song is a timeline of clips, and each track can leave the song to play a launched clip and come back.

### Markers and bars

The names on the ruler are markers: each one starts a section, which runs to the next marker. You can add your own and make room in the song.

{{shot: arrange-song-section | The Verse picked by a tap on its name: its grip just left of its marker, and + Bar, Rename, Remove marker and Delete bars at the end of the bottom row.}}

- **Tap the ruler** where no name is to add a marker on the nearest bar line. It is named with the next free letter (A, B, C…) and picked, ready to rename. Tap a bar that has a marker and it says so.
- **Tap a section's name** to pick it. The last four buttons on the bottom row change:
  - **+ Bar** puts an empty bar just before the section. The section and everything after it move a bar later, every track (audio tracks too), with their tempo changes and morph moves. Press it again for another bar; presses in a row are one **UNDO**. To make room at the end of a section, pick the one after it.
  - **Rename** opens the keyboard with the name in it. Type the new name and tap **Enter**.
  - **Remove marker** takes the marker away. Its bars join the section before, and nothing you hear changes.
  - **Delete bars** takes the section's bars out of the song, every track (audio tracks too), and closes the gap. Its tempo changes and morph moves go with it. In Perform it needs a half-second hold, like every delete. To take out some other stretch of bars, put markers around it first, delete it, then remove the markers.
- **Drag the grip** (two short bars just left of the picked section's marker) along the ruler to move where the section starts. It snaps to the bar, with a tag such as "Verse from bar 6". Only the marker moves, never what plays, and it can't land on another marker.
- **UNDO** takes back each of these, and the message line says what changed.

### The song's tempo

The **Tempo** lane under the ruler shows the song's tempo: a line across the bars, with a dot on each tempo point and its tempo beside it. Before the first point the song plays the project's tempo, the one on Play's transport bar.

{{shot: arrange-song-tempo | The Tempo lane: the project's 92 BPM, then 100 BPM from bar 3 ramping up to 128 at bar 6. The point at bar 3 is picked, so the last two buttons read Ramp (lit) and Delete.}}

- **Tap the lane** where there is no point to add one on the nearest bar line. It starts at the tempo playing there, and it is picked.
- **Tap a point** to pick it, and tap it again to let it go. Push the wheel to grab it, then turn: its tempo moves 1 BPM a click. Push again to let go.
- **Drag a point** along the lane to move it to another bar. A tag says its tempo and the bar it lands on.
- With a point picked, the last two buttons read **Ramp** and **Delete**. **Ramp** makes the tempo glide from this point to the next one's, arriving on the next point: the line slopes, and Ramp is lit. Tap it again to step instead. A ramp on the last point waits for a point after it.
- **UNDO** takes back any of these, one at a time. Turning the wheel on one point is one undo however far you turn it.

**Song on** plays the points. While it does, turning the tempo (the Tempo row, a tempo you type on Play, or a tempo knob you taught with MIDI Learn) changes the point in effect at the playhead, so what you hear is what you set; before the first point it adds a point on the playhead's bar. Inside a ramp it changes the point the ramp starts from, and the glide follows it. The project's tempo stays as it was, and **TAP TEMPO** still sets it. With Song off, the project's tempo plays throughout and the points wait.

A ramp plays as a quick staircase of tempos, one step every 1/16 note, each step landing where a smooth glide would put it. Effects that follow the tempo, such as a synced delay, and the synth follow it too. A MIDI file exported from the song has a tempo for every step of a ramp.

Points are at least a 1/16 apart, and a song has up to 31.

*Example:* tap the lane at bar 9, push the wheel and turn it to 140, then tap the lane at bar 5 and tap **Ramp**. With Song on, bars 1 to 4 play the project's tempo, bars 5 to 8 speed up to 140, and from bar 9 the song plays at 140.

### Audio in the song

Audio you record from the INPUTS goes on **audio tracks**, with lanes of their own under the clip lanes, on the same timeline. They play whenever the song plays. Recording them, editing their regions and picking the best parts of several takes is in [Recording audio tracks](audio-tracks.md).

When you move a section, put bars in or take them out, the audio moves with the song. A region across a section's edge is cut there, and each part plays what it played; parts that meet again in their new place are one region again.

- **UNDO** takes the audio back with the song, and **SHIFT + UNDO** puts both back.
- If you changed a region since (a split, a move, a fade), **UNDO** waits and says so. Take that change back first with the region's **Undo**, then press **UNDO**.
- While an audio track records, the song's bars can't move: the message line says to stop the recording first, and nothing changes.

## The song list

The song list plays scenes one after another, each so many times: "Intro × 2, Verse × 4, Drop × 4, Break × 1". It's the quick way to sketch a song's shape without laying out blocks, and a scene can come back as often as you like. Tap **List** at the top of the Song view to see it; in Perform, **MAIN** goes Clips, Song, List. **Clips** and **Song** at the top go back.

{{shot: arrange-list | The song list: seven steps with their times, bars and where each starts; the Drop playing its third pass of four.}}

- **+ Add a scene** puts a step after the one you picked (at the end if none is), playing the next row that has clips. Tap **‹** or **›** on a step's scene to change it, and **–** or **+** to change how many times it plays (1 to 99). **Bars** says how long the step lasts, and **Starts** the bar it starts on.
- Tap a step to pick it. **Play from here** plays the list from that step, starting on the next Launch line (at once while stopped). Each step's scene plays its times, and the next comes in on the bar line, so you can play pads or turn knobs over it. The step playing is lit green with "pass 3 of 4"; the one about to start is outlined amber.
- After the last step everything stops, unless **Loop the list** is on: then it starts again from the top.
- Launch a scene yourself, stop all, or tap **Back to song**, and the list stops leading; what plays carries on.
- **Remove** takes the picked step out (in Perform, hold it for half a second). Every change to the list is undoable with **UNDO**.
- **List → song** writes the whole list onto the end of the timeline, each step's clips for its bars, with the scene's name above where it starts, so you can carry on in the Song view. Mutes carry from step to step and a one-shot ends after its pass, as with Scenes → song, and the message says whether it came out exact.

A scene's length is its follow action's bars if it has one, else its longest clip, rounded up to whole bars. Moving a row on the grid takes the list's steps with it.

## Recording a performance into the song

**Rec song** records what you play on the clip grid into the song, so a jam becomes a song you can edit afterwards. The button sits above the clip grid and on the Song view, and it is red while it is on.

1. Tap **Rec song**.
2. Press play and jam: launch clips and rows, stop tracks, move the morph crossfader.
3. Press stop, or tap **Rec song** again. What you played goes into the song.

- Each clip goes into the song where it started and stopped, linked to its clip. It plays from the song just as it does from the grid, pad pressure included.
- Tracks that were muted are left out of the song for as long as they were muted.
- The morph crossfader's moves are recorded too, with the two rows it was between. If you leave the crossfader still for a moment, the song holds it there until your next move.
- A track you didn't touch keeps what the song already had there. Where you launched a clip, it records over the song, like recording over tape.
- With the loop on, each time round records over the last.
- A song holds up to 4,096 blocks. When a recording gets to 90 % of that, the message line warns you. If a launch would take it past the limit, Rec song stops recording on every track on that launch line (never halfway through a scene change) and turns off, the message says so, and everything recorded up to there goes into the song. The music plays on.
- Rec song records which clips played, not new notes. To record notes, use **REC** on a clip (see [Clips and patterns](clips.md)).
- **In Perform, REC brings Rec song with it**: each take you record (a clip or a loop) also records the grid into the song for as long as it lasts, and the REC chip says "+ Rec song". To leave it out of a take, tap the REC chip and turn off **Rec song with REC**; it's back for the take after. Rec song turned on by hand stays on as usual (see [Make and Perform](playing-live.md#make-and-perform)).

> [!NOTE]
> A track that is muted when you stop stays muted. Its silent parts are already left out of the song, so unmute it to hear the song as you played it.
