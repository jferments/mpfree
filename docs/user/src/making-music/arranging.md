<!--
status: changing
covers: rust/mpfree-seq/src/scenes.rs rust/mpfree-seq/src/arrange.rs rust/mpfree-perform rust/mpfree-time-ui/src/song.rs rust/mpfree-time-ui/src/widgets.rs rust/mpfree-time-ui/src/list.rs rust/mpfree-time-ui/src/scene_card.rs rust/mpfree-time-ui/src/add_view.rs rust/mpfree-time-ui/src/chord_strip.rs
checked: 2026-11-23 2bc73d12
-->

# Arranging a song

The Arrange tab turns clips into a song in three ways. You can play it live, launching whole rows of clips (scenes) and morphing between their sounds. You can line scenes up in a song list ("Intro × 2, Verse × 4"), as on an MPC. Or you can lay the clips out on a timeline and let the song play itself. You can also do both at once: jam on the grid and record the jam as a song.

## Scenes

A row of the clip grid is a scene. Tap a row's number on the left and every clip in that row starts on the next bar line (or the line the Record panel's **Launch** sets, see [Recording a clip](clips.md#recording-a-clip)), so the whole part of the song changes at once.

{{shot: clips | The Arrange tab's clip grid in a jam: clips playing (a ring turning), two waiting for the next bar ("from 4.1"), Chop B recording, loop cells from the looper, and empty slots, with the morph bar under it.}}

- The square button above the row numbers stops every track on the next bar line.
- A scene can also switch which tracks are muted on its bar line (a track with no clip in the row too: in song mode it plays the song until the line, and its pad and name show the mute waiting, as TRACK MUTE's do), send your outside synths and drum machines a program change there, have a name, and move on by itself after some bars (to the next row, back to the first, or to a random one). A scene can also change the tempo on its line, and let a track with no clip in the row keep playing through the change. Set these up on the row's **scene card** (below). **Song → scenes** (below) also sets rows up to play one after another, and the [song list](#the-song-list) plays them in any order.

### Setting up a scene

Hold a row's number for half a second and its **scene card** opens over the grid, which dims behind it. On the [song list](#the-song-list), hold a part's number for its scene's card. A quick tap on the number still launches the row. Tap **Done**, or anywhere off the card, to close it.

{{shot: clips-scene-card | The scene card for row 2, Verse: it moves on to the next scene after 4 bars four times in five, sets 95 BPM on its line, mutes Hats and sends Stab's synth program 12. Chords and Dust keep playing through it; the row's other empty cells stop their tracks.}}

At the top is the scene's name, with **‹** and **›** either side to go to the card of the scene before or after. A row with no name is called by its number ("Row 2"). Tap the name to rename the scene: the keyboard opens, type the name and tap **Enter**. The name shows on the song list, and becomes the section's name when the scene goes into the song.

Under it are the scene's settings, in two columns. Tap **‹** or **›** on a setting to change it, or tap its name and turn the wheel or press **–** and **+**:

- **Follow**: what plays after this scene has played its bars: off (it plays until you launch another), the next scene, the previous one, the first or the last, any scene, any other scene, this one again, or Stop. Turning it on sets **After** to the scene's longest clip.
- **After**: how many bars the scene plays before Follow happens.
- **Chance**: how often Follow happens, in steps of 10 %.
- **Or**: what happens the rest of the time, when Chance is under 100 %. **Stay** plays the scene again for the same bars and then tries again.
- **Tempo**: "as it is" leaves the tempo alone. Tap the value to give the scene the tempo playing now, and tap it again to go back to as it is. **‹** and **›** step it 1 BPM, or 0.1 BPM with **SHIFT** held. The tempo changes on the line the scene's clips start on, so the new part starts at its own tempo.
- **Mutes**: "as they are" leaves the mutes alone when the scene starts. Tap the value to give the scene the mutes you have now: mute the tracks you want silent in this part, then tap it. When the scene starts, those tracks mute and the others play.
- **Program**: the program change the scene sends the armed track's synth or drum machine when it starts, **1** to **128**, or **off** for none. Arm another track to set its own: each track can have one. It shows a dash when no track is armed.
- **Bank**: the bank select sent just before that program, **0/0** to **127/127** (the two numbers sent, as on the clip's **Bank**), or **off**. Hold **SHIFT** as you step it to step the first number.
- **Sent to**: the tracks this scene sends a program change to.

A scene's program change goes out on the bar line the scene starts on, after the old clip's last notes and before the new clip's first, so the synth plays the new part with the new sound. A track with no clip in the row gets its program change too, which lets the scene switch a drum machine that plays its own patterns. If the clip that starts on a track has its own **Program** (see [Clip settings](clips.md#clip-settings)), the clip's goes out instead. They go out again when you start the music after stopping, but not on every loop. Only tracks that play on MIDI Out send them (their **Plays on**, with **Send MIDI and CV out** on in Prefs): see [MIDI and CV](midi-and-cv.md).

**Empty cells** has a box for each track, eight at a time (**‹** and **›** show the rest). A track with a clip in the row shows the clip's name: it starts with the scene. A track with no clip in the row shows **Stop** or **Keep**. **Stop**, as it always was, stops the track when the scene starts. **Keep** lets whatever the track plays carry on through the change, so a bass line or a loop can run on under several scenes. Tap the box to turn it over. Scenes → song lengthens a kept track's block through the row, so the song plays it the same way.

Along the card's foot:

- **Capture** copies the clips playing now into the first empty row under the scene playing (from row 1 when no scene plays), as clips of their own. In song mode a track playing the song gives the song's block under the playhead. What plays carries on, and the card moves to the new row. In Perform the grid's title line has the same button, **Capture scene**, so you can keep a combination without opening the card (see [Make and Perform](playing-live.md#make-and-perform)).
- **Copy row** puts a copy of this row, its clips and its scene, under it. The rows under it move down one.
- **Insert row** puts an empty row under it, and the rows under it move down one.
- **Delete row** takes the row out, its clips and its scene with it, and the rows under it move up one. Its clips stop if they were playing, and its parts leave the song list. A clip of the row that the song plays stays in the song. In Perform it needs a half-second hold, like every delete.

Copy row and Insert row need the grid's last row to be empty, since every row under this one moves down.

Every change plays at once, and **UNDO** takes it back. Turning one setting many times is one undo. While a scene plays, new mutes wait until it starts again.

### Adding a part to a row

**Add a part**, on the grid's title line before **+ Track**, makes a new part for a row that plays with what is already there: a bass line under your beat and tune, chords, a second melody, or drums. It listens to the row's clips first, so the part follows their key, chords and groove.

{{shot: clips-add-part | Add a part open on row 5: it hears the row's Break, Ride and Solo, a Bass part on the Bass track plays in its cell (dashed, "try 2"), the row's chords (Cm, Ab, Bb, Cm, Ab) under the row, and Keep ready.}}

1. Pick the row on the **Scene** tab, or let it use the row playing. Tap **Add a part**. The row gets an outline, and the panel takes the place of the Record panel and the takes.
2. **Heard** lists what it listens to in the row. Tap a part: **Bass**, **Chords**, **Melody** or **Drums**. A first try starts at once.
3. The try plays in its cell with the row, dashed in the track's colour, from the next bar line while the row plays (press **PLAY** and launch the row to hear it).
4. Change it until you like it:
   - **Role** says how it plays with the others: **Support** (with the beat, under the tune), **Answer** (where the tune rests), **Interlock** (in the beat's gaps) or **Stay out** (sparse, out of the way). The words under it say what the role does.
   - **Style** (Minimal, Groove, Jazz-ish), and **Bars**: the row's length, or 1, 2, 4, 8 or 16 bars.
   - **A phrase as the part:** step **Style** on past Jazz-ish and it runs on into the arp's phrases of the part's kind, MPFree's and then yours (for Bass the bass lines, for Chords the chords, for a melody the riffs and arp lines; drums take none). The row then reads **Phrase**, and the part is the phrase played over the row's chords, moved to each chord's root in the row's key, as the arp moves it from a note you hold. A phrase plays one way, so **Try again** is faint and the Q-Links read "–"; step back to a style for a part the composer writes. See [Phrases](note-repeat-and-arp.md#phrases).
   - **Track**: the track whose empty cell gets the part. It starts on a track whose name says the part (a "Bass" track for Bass), else the first with room. **Sound** says what plays it.
   - **New track**, the Track row's last choice: the part goes on a track **Keep** adds, named for the part ("Bass", or "Bass 2" when you have one). **Sound** then picks the synth it plays: for Bass the Bass synth first, then Squelch, Analog, Poly, FM and Pluck; for Chords Poly, Keys, Strings, Drawbar, Choir, Analog and FM; for a melody FM, Pluck, Analog, Wind, Poly, Keys, Strings and Squelch; drums play the Drums synth. The status says "Making the Bass synth…" for a moment before the first try. When every track has a clip in the row, the panel opens on New track.
   - The Q-Links: **Similarity**, **Variation**, **Activity** and **Range**, as on Continue.
   - **Try again** makes another. **‹ Try 1** goes back to an earlier one (the last eight are kept).
5. **Keep** puts the part in its cell: one take, "a new part", and one **UNDO** takes it back. If the song plays the row, its parts get it too, on the same take. On New track, Keep adds the track with the part and its synth, and one **UNDO** takes both away. **Back** or **×** closes the panel and changes nothing (New track's synth goes too).

{{shot: clips-add-part-phrase | Add a part on row 5 with Style stepped past Jazz-ish to the phrase Night drive: the Bass try plays Night drive over the row's chords, Try again is faint and the Q-Links read "–".}}

Adding a track with **+ Track** while New track is trying moves the new track one along: the tries go ("A track was added: Try again"), and **Try again** makes one for the track after it.

While **Rec song** is on, Add a part is faint and the hint says why: a try playing in its cell would be recorded into the song. A part for drums plays the track's kit, on the pads its kit names as kick, snare and hats.

### The row's chords

While Add a part or the **Chords** tab is open, the row's chords show as a strip under the row, one row high: the row's bars across the grid, a chip for each chord as long as it lasts. Chords it heard in your clips are solid, suggested ones (where nothing says which chord) are dashed, and chords you set are blue. The green line is where the row is playing. A row with no notes reads "no chords yet": tap a bar to set one.

{{shot: clips-chord-picker | The chord picker open on bar 4: heard Bb, Ab7 picked, the 7 lit; Split, Join next, As heard and Use Ab7.}}

Tap a chip to change its chord. The picker opens under it, saying what was heard and what you're picking ("heard G7 → Ab7"). Tap a root, then a kind (major, minor, 7, maj7, m7, sus4, 6, m6, dim, dim7, m7♭5 or power), then **Use**. **Split** makes two chords of one, **Join next** makes one of it and the next, and **As heard** gives the row its own chord back. **×** closes it with nothing changed.

- With Add a part open, the part you're trying plays the new chord at once, in the same rhythm and shape, and **Keep** keeps the part and its chords as one **UNDO** (**Back** drops both).
- From the Chords tab, **Use** is a take of its own, "row 5's chords".

### Clips that don't fit the new chords

Your clips are never rewritten behind your back. When a row's chords change, a kept clip with notes off the new chords is marked in its cell: an amber dashed outline, and "doesn't fit" in place of its length. Drums and audio are never marked.

{{shot: clips-chords | The Chords tab after bar 4 became Ab7: two clips don't fit, both ticked, with how many notes would move, and Fit to the new chords.}}

Tap **Chords**, the third tab on the Record panel, to deal with them. It shows the row (the one you picked; tap **‹** or **›** for another), what changed, and each clip that doesn't fit, ticked, with how many of its notes would move. **Fit to the new chords** moves the ticked ones: their rhythm and shape stay, and only notes under a changed chord move. It is one take, "the new chords", and one **UNDO**. Untick a clip to leave it as it is (your tune, say).

An empty clip in the Sequencer's note editor offers the same panel: see [Editing notes](editing-notes.md#an-empty-clip).

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

The song is your clips laid out on a timeline, one lane per track. Tap **Song** on the chip at the top left of the Arrange page (**Clips | List | Song**) to see it, and **Clips** to go back to the grid; the part lit is the view you're on. **+ Track**, under the last track's lane, adds a track, as on the grid. With more tracks than fit, drag up or down on the names to scroll.

{{shot: arrange-song | The Song view: seven rows made into a song (Intro, Verse, Build and on), eight tracks of blocks under the scene names, and Rec song on.}}

- **Scenes → song** adds every row that has clips to the end of the song, one after another, with each scene's name above the bars where it starts. It plays the rows behind the scenes, much faster than real time, and records what they play, so the song plays just what the rows played: a scene's mutes carrying on into the rows after it, a one-shot clip stopping after its pass, a clip's follow action moving to its next clip, and a legato clip picking up where the clip before it had got to. A row that changes the tempo gets a tempo point where it starts, so with **Song on** the song changes tempo where the rows did. A follow action with a chance rolls the same way each time you convert, so the same rows always make the same song.
- **Song on** plays the song. A track plays the song wherever you have not launched a clip on it. Launch a clip and it takes that track over. Stop it and the track goes back to the song. While clips play over the song, **Back to song** shows on the grid's title line: it stops them all on the next Launch line, and every track plays the song from there.
- **What each track plays** shows on the grid while the song is on. A track the song has something for has a **SONG** tag on its name: filled green while it plays the song, outlined while a clip you launched plays over it. A clip about to give the track back says when, in amber ("to song 6.1"). On the Song view, a clip you launched is drawn over its track's lane, the song's blocks hatched under it while they wait, and a dashed amber line marks where the song comes back.

{{shot: clips-perform-song | Song on in Perform: SONG tags on the tracks, filled where they play the song and outlined where a launched clip plays over it, and the glitch going back to the song at 6.1.}}

{{shot: arrange-song-over | The same on the Song view: the launched clips drawn over their lanes, the glitch ending at the dashed line where the song comes back, and the chords waiting for bar 6.}}
- **Song → scenes** goes the other way. It cuts the song into its parts and puts each part into empty rows, set to play one after another. It never writes over a row that has clips, or one you have set up without clips (a name, mutes, a follow action, morph values or a program change). A part that starts partway into a block gets its clip turned round to start there, so launching the first row plays what the song played. A tempo point on a part's first bar becomes its row's tempo: launching the row changes the tempo on its line, exactly where its clips start. What an audio track recorded comes too: each part's stretch of its lane becomes an audio clip in the part's row, named after the track and its bars ("Vocal 5–8"), playing exactly what the song played there, fades and all. A part with more than 16 recordings on one audio track can't be one clip, so nothing changes and the message line says where ("Vocal, bars 9 to 16: 21 regions, and a clip holds 16"); split the part with a marker, or bounce it first. **UNDO** takes the whole thing back in one go, and **SHIFT + UNDO** puts it back.
- Every conversion tells you whether it came out **exact**. An exact one says so in place of the hint line ("Song → scenes: rows 9 to 11, exact. UNDO takes it back."). When something can't carry over, a card opens over the song with one row for each difference: the track (or **Song** when it's the whole song's), the bar, and what's different. That can be a part off the bar line, a stretch of silence, a tempo change inside a part or a tempo that glides, the song's morph moves, a note held over a row change, a clip that plays in Random order or has notes with a chance (a copy rolls them its own way), or, going into the song, a scene's program changes (the one thing the song doesn't play yet). Five rows show at a time; drag the list or turn the wheel for the rest.
- On the card, **UNDO** (or the MPC's UNDO) takes the whole conversion back, and **Done** closes the card and keeps the result. A tap anywhere outside the card, **MENU** or another page also closes it and keeps the result; that tap does nothing else. Once the card is closed, the details are still in the log in Dev.

{{shot: arrange-report | Song → scenes with four differences: the card over the Song view, each difference by track and bar, with UNDO and Done.}}
- Hold **SHIFT** and turn the wheel to scroll through the bars. Tap a block to select it. It is picked when you lift your finger, because you can also drag it.
- Under each track's name is what it plays: its kit (**Kit** for kit 1, then **Kit 2** and on: see [A kit for each track](pads.md#a-kit-for-each-track)), **Synth**, or its own instrument (see [A synth for each track](synths.md#a-synth-for-each-track)).
- **Loop tracks** have lanes too, and Song on plays their loops' blocks as it plays clips (see [Loops in the song](looping.md#loops-in-the-song)).

A block in the song is linked to its clip, so when you edit the clip, every block that uses it changes too. Moving the clip on the grid, to another row or track, doesn't change the song: its blocks play it wherever it is. The exception is a part of a linked song list that plays a row: it follows its row (see [The song list](#the-song-list)). **Make unique** gives the selected block its own copy, so you can change it without touching the others. A unique block has a small dot in its corner. **Delete** removes the selected block, and **UNDO** puts it back. In Perform, Delete takes a half-second hold (see [Make and Perform](playing-live.md#make-and-perform)).

### Split and Duplicate

- **Split** (the first button under the lanes) cuts at the playhead. With a block picked, it cuts that block in two, and the part after is picked, so another Split cuts it again. With nothing picked, it cuts every block the playhead is on, on every track. The song plays exactly as it did; you just have two blocks to move, trim or delete. If the playhead isn't on a block, it says so.
- **Duplicate** puts a copy of the picked block right after it, linked to the same clip, and picks the copy, so pressing it again builds a run. If another block is in the way, it says so and changes nothing.
- A split doesn't make a part of a linked song list the song's own: the part plays just what it did. A trim, a slide or a duplicate inside the part does.

### Moving blocks and sections

You can move the song around with your finger:

- **Drag a block** to move it. Once your finger moves, it lifts and follows you, and its new place is outlined, with a tag saying the bar it starts on. It snaps to the bar. Drag it up or down to put it on another track's lane (a loop's block stays on its Loop track's lane, and a clip's block stays off them). There it plays that track's sound and stays linked to the same clip, so editing the clip still changes both. Hold **SHIFT** as you lift to drop a copy instead, linked to the same clip. Lift it where it started to leave it be. A block can't land on top of another one.
- **Drag a picked block's end** to make it longer or shorter, a bar at a time. The picked block has a grip (two short bars) at its right end. Past the clip's end, the clip plays again from its top.
- **Drag a picked block's start** to trim it from the left, a bar at a time: the grip at its left end. What it plays stays where it was in the song, so a later start leaves out the clip's first bars, and an earlier one brings in the bars before (from the clip's end, as it loops). It stops at the block before it on its lane, and keeps its last bar.
- **Slide what a block plays:** pick the block, hold your finger still on it for a moment, then drag. The block stays where it is and the clip moves inside it, a 1/16 at a time, with a tag saying how far ("slide +1 beat"). Drag right to play it later, left to play it earlier.
- **Drag a section's name** on the ruler to move the whole section, every track, from its marker to the next one. A line shows where it will go. The sections close up behind it and make room for it, so nothing is written over. Hold **SHIFT** as you lift to put a copy there instead, such as a second verse. Everything in the section goes with it: its blocks, the audio on its audio tracks, its marker, its tempo changes and its morph moves, so it sounds the same in its new place. A block that runs across a section's edge is cut there, and each part still plays as it did; where the parts of one block meet again in step, they stay one block. If a tempo ramp runs past the section's end, its last 1/16 holds steady so the section still ends on time.
- Hold a dragged block or section at the left or right edge and the bars scroll.
- With the wheel: pick a block, push the wheel to grab it, turn to move it a beat at a time, and push again to let go. Tap a section's name to pick it, then push and turn to move it a section at a time.
- **UNDO** takes back any of these, one at a time, and **SHIFT + UNDO** puts them back again.

### Several blocks at once

- **SHIFT + tap** a block to add it to what's picked, or to take it out again. A tap without SHIFT picks just that block.
- **Drag across empty space** to draw a box: every block it touches is picked. Hold **SHIFT** as you lift to add them to what's picked. A tap on empty space picks nothing.
- Each picked block is outlined. **Drag any of them** and they all go with it, keeping their places to each other, each one's new place outlined (red where it would run into a block that isn't picked). Hold **SHIFT** as you lift for copies. If one can't go there, none moves, and the hint line says why.
- **Split** cuts the picked blocks that the playhead is on, and picks the parts after. **Duplicate** puts copies of them all right after the last one ends, and picks the copies, so pressing it again repeats them on. **Delete** removes them all (in Perform, with the half-second hold). **Make unique** takes one block at a time.
- The wheel moves them all a beat at a time: push to grab them, turn, push again to let go.
- Each of these is one step that **UNDO** takes back.

{{shot: arrange-song-picks | Four blocks picked on the Bass and Lead lanes, each outlined; Make unique is faint, and the hint line says what they do together.}}

{{shot: arrange-song-box | A box dragged from bar 15 on the Keys lane back to bar 7 on the Hats lane: the five blocks it touches are outlined.}}

### Zoom

The **Bars** chip on the Song view's title line shows **4**, **8**, **16**, **32** or **64** bars across the lanes; the one showing is lit. Up close (4 or 8 bars) a faint line marks each beat, and dragged blocks, ends and starts snap to the beat instead of the bar. Further out, the bar numbers on the ruler thin out to every 4th or 8th bar, and the wheel and a block held at the edge scroll more bars at a time. The zoom stays as you set it while you switch views and pages; a new session starts at 16 bars.

{{shot: arrange-song-zoom-4 | The Song view at 4 bars, from bar 5: a faint line on every beat.}}

> [!MPC]
> On the MPC, Song Mode plays a list of sequences one after another, and Arrange Mode records tracks onto a linear timeline. In MPFree, the song is a timeline of clips, and each track can leave the song to play a launched clip and come back.

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

Audio you record from the INPUTS goes on **audio tracks**, with lanes of their own under the clip lanes, on the same timeline. They play whenever the song plays, unless an audio clip launched on the grid plays in its place. Recording them, editing their regions and picking the best parts of several takes is in [Recording audio tracks](audio-tracks.md).

When you move a section, put bars in or take them out, the audio moves with the song. A region across a section's edge is cut there, and each part plays what it played, fades included; parts that meet again in their new place are one region again.

- **UNDO** takes the audio back with the song, and **SHIFT + UNDO** puts both back.
- If you changed a region since (a split, a move, a fade), **UNDO** takes those changes back first, then the song edit with its audio.
- While an audio track records, the song's bars can't move: the message line says to stop the recording first, and nothing changes.

## The song list

The song list is your song as a list of parts, each playing a scene so many times: "Intro × 2, Verse × 4, Drop × 4, Break × 1". It's the quick way to sketch a song's shape without laying out blocks, and a scene can come back as often as you like. Tap **List** on the chip at the top left of the Arrange page to see it; in Perform, **MAIN** goes Clips, Song, List.

In a new project the list and the song are one song shown two ways: each part on the list is a stretch of the song, from one of its markers to the next, and changing either changes the other. The title says so: "Linked to the song · 7 parts · 50 bars · 2:10". A project saved before this keeps a list of its own, apart from the song (see [A list of its own](#a-list-of-its-own)).

{{shot: arrange-list-linked | The song list linked to the song: the song's parts with the Song row first, the Verse playing its third pass of four, and the first Drop changed in the song and picked.}}

- **+ Add a scene** puts a part after the one you picked (at the end if none is), playing the next row that has clips. The song gets that row's clips in the part's place, and everything after it moves later.
- Tap **‹** or **›** on a part's scene to play another scene there; the part takes the scene's name, and the song after it moves to make room or closes up. Tap **–** or **+** to change how many times it plays (1 to 99): **+** adds a pass at its end and everything after moves later; **–** takes its last pass out and everything after closes up. **Bars** says how long the part lasts, and **Starts** the bar it starts on.
- To move a part, drag it by its **grip**, the two short bars at its left (or its number): a line shows where it will land, and it goes there when you let go, with everything in its bars. A drag that starts anywhere else on the row scrolls a long list.
- **Remove** takes the picked part out with everything in its bars, and the song closes up (in Perform, hold it for half a second).
- **Song**, in faint words at the top, is the song's bars before its first marker, when it has some. It can't be moved, given a scene or repeated; add a marker on the Song view's ruler to make those bars a part.
- Every change is one step for **UNDO**, the song's bars with it.

**A part plays what its row holds.** Record a clip into a part's row, paste or drag one there, drag one away or delete it, and every part that plays that row changes with it. The part keeps its bars: a longer clip is cut at the part's end, and a shorter row plays round as many times as fill it. One **UNDO** takes back the clip and the parts together, and the Takes list says "and the song". A part changed in the song keeps what it has.

The song changes the list too. Drag a section's name along the Song view's ruler and the part moves on the list; put in a marker and the list gets a new part. **Edit inside a part** on the Song view (move, delete, add or resize a block, make one unique, change its bars, or record over it with Rec song) and it becomes the song's own: the list marks it "● changed in the song", and its passes become plain bars (× 1). Moving or copying it whole doesn't count. **Back to the scene**, in List → song's place, puts the scene's clips back for the part's bars, as many passes as fit; it is lit when a changed part is picked. **Scenes → song** adds its rows after the last part.

**Playing.** Tap a part to pick it. **Play from here** plays the song from the part's first bar, with **Song on**: clips you launched over the song stop, and it starts playing if it was stopped. As each part starts, its scene's mutes, program changes and tempo come in, as launching the scene would, and a scene without mutes leaves them as they were. When the song has tempo points of its own, they set the tempo instead. The part playing is lit green with "pass 3 of 4". **Loop the list** loops the song from its first part to the end of its last; any loop you had set comes back when you turn it off. Launching a scene over the song, and **Back to song**, work as they always do.

On the Song view, a part that plays its scene more than once has its times after its name on the ruler ("Verse ×2"), and a faint dashed line marks where each pass starts, labelled "pass 2 of 2". Once the part is the song's own, its passes are plain bars and the lines go.

{{shot: arrange-song-passes | The Song view with the Verse playing its scene twice: "Verse ×2" on the ruler and a dashed line where its second pass starts.}}

### A list of its own

A project saved before the list was linked keeps a list of its own, apart from the song: its steps play scenes live, and the song doesn't change when the list does.

{{shot: arrange-list | The song list: seven steps with their times, bars and where each starts; the Drop playing its third pass of four.}}

- **+ Add a scene** puts a step after the one you picked (at the end if none is), playing the next row that has clips. Tap **‹** or **›** on a step's scene to change it, and **–** or **+** to change how many times it plays (1 to 99). **Bars** says how long the step lasts, and **Starts** the bar it starts on.
- To move a step, drag it by its **grip**, the two short bars at its left (or its number): a line shows where it will land, and it goes there when you let go. **UNDO** puts it back. A drag that starts anywhere else on the row scrolls a long list.
- Tap a step to pick it. **Play from here** plays the list from that step, starting on the next Launch line (at once while stopped). Each step's scene plays its times, and the next comes in on the bar line, so you can play pads or turn knobs over it. The step playing is lit green with "pass 3 of 4"; the one about to start is outlined amber.
- After the last step everything stops, unless **Loop the list** is on: then it starts again from the top.
- Launch a scene yourself, stop all, or tap **Back to song**, and the list stops leading; what plays carries on.
- **Remove** takes the picked step out (in Perform, hold it for half a second). Every change to the list is undoable with **UNDO**.
- **List → song** writes the whole list onto the end of the timeline, each step for its bars, with the scene's name above where it starts, so you can carry on in the Song view. Like Scenes → song, it plays the list behind the scenes and records it, so the song plays what the list played, follow actions, one-shots and carried mutes included. It plays the list once, even with **Loop the list** on. An exact result says so on the hint line; otherwise the card opens over the list, as above, usually with one row: a scene's program changes, which the song doesn't send yet.

A scene's length is its follow action's bars if it has one, else its longest clip, rounded up to whole bars. Moving a row on the grid takes the list's steps with it.

**Link** makes the list and the song one, as in a new project. It sits left of **Loop the list**, only on a list of its own. Tap it and choose:

{{shot: arrange-list-link | Link's two choices over a list of its own: Song from the list, and List from the song, each saying what it would do.}}

- **Song from the list**: the list's steps become the start of the song, each a part that plays its scene, and the song you had moves along after them. The song then plays what the list played.
- **List from the song**: the song stays as it is, and its marked parts become the list, each played once.

Each line says what it would do to this project before you choose, and either is one step for **UNDO**. Tap **Link** again, or anywhere off the choices, to close them without linking. Once the list is linked, Link goes.

## Recording a performance into the song

**Rec song** records what you play on the clip grid into the song, so a jam becomes a song you can edit afterwards. The button sits above the clip grid and on the Song view, and it is red while it is on.

1. Tap **Rec song**.
2. Press play and jam: launch clips and rows, stop tracks, move the morph crossfader.
3. Press stop, or tap **Rec song** again. What you played goes into the song, as one step **UNDO** takes back.

- Each clip goes into the song where it started and stopped, linked to its clip. It plays from the song just as it does from the grid, pad pressure included.
- Tracks that were muted are left out of the song for as long as they were muted.
- The morph crossfader's moves are recorded too, with the two rows it was between. If you leave the crossfader still for a moment, the song holds it there until your next move.
- A track you didn't touch keeps what the song already had there. Where you launched a clip, it records over the song, like recording over tape.
- With the loop on, each time round records over the last.
- An audio clip launched on an audio track goes into the song as a block too. Where it lands over something that track recorded, the block replaces it, and **UNDO** takes back both. The other way round, recording on an audio track over a block cuts the block there, and **UNDO** takes back both too (see [Audio clips in the song](audio-tracks.md#audio-clips-in-the-song)).
- A song holds up to 4,096 blocks. When a recording gets to 90 % of that, the message line warns you. If a launch would take it past the limit, Rec song stops recording on every track on that launch line (never halfway through a scene change) and turns off, the message says so, and everything recorded up to there goes into the song. The music plays on.
- Rec song records which clips played, not new notes. To record notes, use **REC** on a clip (see [Clips and patterns](clips.md)).
- **In Perform, REC brings Rec song with it**: each take you record (a clip or a loop) also records the grid into the song for as long as it lasts, and the REC chip says "+ Rec song". To leave it out of a take, tap the REC chip and turn off **Rec song with REC**; it's back for the take after. Rec song turned on by hand stays on as usual (see [Make and Perform](playing-live.md#make-and-perform)).

> [!NOTE]
> A track that is muted when you stop stays muted. Its silent parts are already left out of the song, so unmute it to hear the song as you played it.
