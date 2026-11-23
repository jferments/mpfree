<!--
status: changing
covers: rust/mpfree-time-ui/src/editor.rs rust/mpfree-time-ui/src/editor/zoom.rs rust/mpfree-time-ui/src/editor/gesture.rs rust/mpfree-seq/src/notes.rs rust/mpfree-seq/src/save.rs rust/mpfree-app/src/time.rs rust/mpfree-app/src/hearing.rs
checked: 2026-11-22 75ce108d
-->

# Editing notes

The note editor shows a clip's notes so you can fix what you played: pull notes into time, move them, change how hard they hit, draw new ones and erase others. It is the **Sequencer** tab's **Notes** view: tap **Notes** at the right end of the Sequencer's transport bar, or **Edit ›** over the clip grid on Arrange, which takes you there. It opens the clip the Sequencer's Steps show, which is the armed track's playing clip, or its first clip.

{{shot: sequencer-notes | The note editor's piano roll across the screen, the overview strip over it. Two long melody notes show the pressure they were played with.}}

The Notes view has the Arrange tab's transport bar along the top (**Stop**, **Play** and **Rec**, the record mode, the position, the tempo, the time signature, the launch line, **Undo** and the REC chip), the roll across the screen, and a row of buttons under the roll. The **Edit** options and the clip's **Takes** are in a drawer at the right: **Edit**, **TC** or pushing the data wheel opens it, and the roll makes room.

| Button | What it does |
|---|---|
| **‹ Steps** | Back to the Steps view (so does **STEP SEQ**). |
| **Grid** | Which view shows: Grid, List or Tracker. Tap it for the next. |
| **Draw** | Draw on (lit) or off. With Draw on, the roll works like Steps: tap notes in and out. With it off, a tap picks a note so you can change it. |
| **Auto · 1/16** | The snap: the grid that moves and new notes land on. **Auto** follows your zoom (see [Snap](#snap)); tap for Auto triplets, then a fixed grid from a bar to 1/64, then off. |
| **Select** | Opens the Select menu: pick every note, none, the others, or notes by pitch, time, loudness and more (see below). |
| **Duplicate** | Copies the picked notes to just after them. |
| **Delete** | Takes the picked notes out (in Perform, hold it). |
| **Edit** | Opens the drawer with the Edit options (Timing Correct and more) and the clip's Takes; lit while it shows. Tap it again to close it. |

## The piano roll

The notes are bars on a piano roll: higher notes higher up, time running left to right, longer notes longer. A drum clip shows a row for each of the kit's pads instead of keys (see [Drum clips](#drum-clips)); a clip of notes on a synth track shows the keys even while it is empty. Under the roll, a lane shows how hard each note plays, how likely it is to play, how many times it repeats, or the picked notes' pressure (see [The lane under the roll](#the-lane-under-the-roll)). A note played with [pad pressure](clips.md#pad-pressure) shows it as a lighter shape along its bottom: it rises where you pressed harder and falls where you let up.

Notes are drawn as they will play. A note whose step is off (tapped off on Steps) is only an outline: it keeps its place and settings and plays nothing. A note that repeats (a ratchet) shows each hit, lighter or darker as its velocity ramps, and each hit shows in the velocity lane too. A tied note has a short level mark just before it, and a sliding note a short slanting one.

- **Draw on: tap a beat in.** With **Draw** lit, the roll works like the Steps view. Tap an empty spot to put a note there, one snap long and as loud as the last note you used. Tap a short note (a step long or less) to take it out again. A longer note is picked instead, so a quick tap never loses a long note. Drag along an empty row to draw a note as long as your drag. Hold a note for a moment to pick it without taking it out.
- **Draw off: pick and change.** Tap a note to pick it, just that one. A tap just beside a note, in its row, picks it too, so you don't have to hit a short note exactly. Tap an empty spot to let the picks go, or tap it twice quickly to put a note there.
- **Pick a group.** With Draw off, drag across an empty spot to draw a box: when you lift, every note sounding inside it is picked instead of the ones before. Or hold one note (with Draw on or off) and, still holding it, tap other notes: each tap adds a note to the picks, or takes it out if it was picked, as holding steps does on the step pads.
- **The Select menu.** Tap **Select** for a menu over the roll. **All** picks every note, **None** lets them go, and **Invert** picks the notes that weren't picked instead of the ones that were. **Fit to picked** zooms the roll to the picked notes. The rest pick from the whole clip: **Same pitch** (every note at the pitch of the picked ones, or of the sound you last worked on), **Pitch range** (every note from the lowest picked note to the highest), **Time range** (every note starting from the first picked note to the end of the last; for both ranges, pick a few notes first), **Quiet notes** and **Loud notes** (softer, or harder, than halfway between the clip's softest and loudest), **With a condition**, **With repeats** (a ratchet), **Overlapping** (notes that sound over another note of the same pitch) and **Off the scale** (notes off the clip's scale, see [Scale](#scale)). Tap anywhere else to close the menu.
- **Add to pick.** In the Select menu, **Add to pick** is a switch: while it is on, it and **Select** stay lit. Then a tap on a note adds it to the picks or takes it out (with Draw on, a short note still comes out), a box adds the notes in it, and the menu's choices add to the picks you have, so you can gather notes of several kinds: **Same pitch**, then a box somewhere else. Tap it again to turn it off. **All**, **None** and **Invert** always replace the picks.
- **Nothing happens until you lift.** A finger on a note is a tap when it lifts, a hold once it has stayed still for 0.4 seconds, or a drag once it moves about 3 mm. Nothing goes in or comes out before your finger lifts.
- **Erase.** Hold **ERASE** and tap a note to take it out, with Draw on or off. To take out several, hold **ERASE** and drag across them: each note your finger passes over is marked, with a count above your finger, and they all come out when you lift it, so one **UNDO** brings them all back. Drag along a row of hi-hats to clear it, or zigzag over a messy fill. A second finger on the roll calls it off.
- **Copy and paste.** Hold **COPY** and tap a note: the picked notes are copied, or just that note if it isn't picked. Then hold **COPY** again and tap an empty spot, in this clip or in another track's clip: the copy goes in starting where you tap, each note keeping its pitch and its place after the first. The notes bring their pad pressure and their locks. A lock on the instrument comes into another track's clip when that track plays the same kind of instrument the lock was made for, so it turns the same control there; a clip's instrument locks are all for one instrument, so it doesn't come into a clip whose instrument locks are for another. A lock on a kit pad, an insert or MIDI out comes only into another clip of the same track, since another track may play another kit or send to other gear. The hint line says how many locks were left out and what they were on, and for instrument locks which instrument they were made for ("made for Poly, Lead plays FM"). Pad hits paste only into a clip of pad hits, and notes into a clip of notes. A chord whose notes play together or not at all (a chord pad's) stays one when you copy all of it, and stays apart from a chord already where it lands; copy only some of its notes and each of them rolls its own chance. The pasted notes are picked, and one **UNDO** takes the paste back. The copy stays until you copy again, so you can paste it as many times as you like. In the tracker, tap an empty line to paste there.
- **Move them.** Drag a note to move it, along time and up or down in pitch. If it is picked, every picked note moves with it; if not, it moves on its own and is picked after. With **Auto** snap a move goes by whole grid steps and each note keeps its feel: a note played a little late stays a little late. With a fixed snap, the note you drag lands on a line. While you drag, the notes show where they will land, an outline shows where they were, and a box above your finger says where the note you hold lands ("2.3.000 · D3").
- **Change their length.** A picked note has a handle at each end, just outside it; on a note too short to grab, the two handles hang just under it. Drag the right handle to change where the picked notes end, or the left one to change where they start (their ends stay put). The box above your finger shows the new length or start, and it all goes in when you lift. Handles show while up to 64 picked notes are on the screen; for more, use the **Length** option.
- **Velocity, chance and repeats.** Draw along the lane under the roll: each note you pass takes the height under your finger (see below).
- **One finger edits, two move the view.** A drag belongs to the finger that started it. If a second finger touches the roll while you drag, the drag stops, the notes stay where they were, and the two fingers zoom and scroll instead (see below).
- **Duplicate** copies the picked notes to just after them. **Delete** takes them out (in Perform, hold it for half a second).
- **Grid, List, Tracker.** The second button says which view is showing (**Grid** to begin with) and switches to the next. **List** shows the notes as a list, one row per note, with its position, note, velocity, length and chance. An off note's row is faint and its note says "off"; a tied or sliding note's says "tie" or "slide". **Tracker** shows every step, described below.

{{shot: sequencer-notes-list | The same notes as a list, the picked ones lit.}}

### The lane under the roll

Under the roll, each place where notes start has a bar in the lane, as tall as their value, with the number over it when there is room. The chip at the lane's left says which lane shows; tap it to step through them:

- **Vel** (velocity): how hard each note plays, 1 to 127. A note that repeats also shows each later hit as a thin bar beside it, at the velocity its ramp gives it.
- **Chance**: how likely each note is to play on a pass, 1 to 100 %. A note that never plays is an off note (tapped off on Steps), not 0 %. A chord whose notes play together or not at all (a chord pad's) has one chance, so drawing across it sets the whole chord to the last value you drew on it.
- **Repeats**: how many times each note plays (a ratchet), 1 to 8. An arrow after the number says its ramp rises (↑) or falls (↓).
- **Pressure**: the [pad pressure](clips.md#pad-pressure) of the picked notes, as a line along each one that rises where the pressure grows. Pick the notes first: with none picked the lane says "Pick notes to draw their pressure". See [Drawing pressure](#drawing-pressure).
- **A setting's locks** ("Cutoff", "Tune"): one lane after Pressure for each [lock lane](sequencer.md#lock-lanes) the clip has on Steps, named for its setting. See [Drawing locks](#drawing-locks).

Draw with one finger along the lane: every bar you pass takes the height under your finger, so a crescendo or a fade is one stroke. The bars follow your finger, and nothing changes until you lift it; then the stroke is one change, and one **UNDO** takes it back. A second finger on the screen calls the stroke off.

- **Notes that start together** (a chord, or a strum close together) share one bar, and a stroke sets them all. To set just one of them, pick it first: then only the picked notes there change.
- **The ramp.** Hold **SHIFT** as your finger goes down on the Repeats lane to draw each note's ramp instead: up from the middle line makes its later hits louder, down makes them softer. Only notes that repeat take a ramp.
- **Room.** Drag the lane's top edge (the short grip over it) up to make it taller, or down to make it shorter. Tap the **▾** under the chip to fold the lane away and give the roll its room; tap the chip to open it again. The lane stays as you leave it, for every clip.

#### Drawing pressure

Pick the notes whose pressure you want to shape, then tap the lane's chip until it says **Pressure**. The lane shows each picked note's pressure as a line under it; where picked notes sound together it shows the highest.

{{shot: sequencer-notes-pressure | The Pressure lane under two picked notes that were played with pressure: each swell is a line under its note.}}

Draw along the lane with one finger and the line follows it: a swell, a fade, or pressure on a note that had none. Before and after your stroke each note keeps the pressure it had. Nothing changes until you lift your finger; then the stroke is one change to every picked note it crossed, and one **UNDO** brings their old pressure back. The roll's own pressure shape on each note shows the result. Pressure goes to your instrument as the pads' pressure does, so a synth or a sample program that listens to it plays the new line.

#### Drawing locks

A lock is a setting's own value on one step, such as a filter that closes on one chord and opens on the next. The lanes you add on **Steps** with **+ Lane**, and every setting the clip already locks, show here too: tap the lane's chip past **Pressure** until it names the setting.

{{shot: sequencer-notes-locks | The Cutoff lane under a bar of chords: three steps locked, the second chord picked and its lock shared by its three notes; the locked notes and a 1:2 note carry marks.}}

- **What it shows.** Each place a lock can sit has a bar, as tall as its lock. A bar with no lock is faint, at what that step plays without one. The lane's top right says the value of the picked note's bar ("2.6 kHz"), or of the bar under your finger while you draw.
- **Draw** along the lane as on the others: each bar you pass gets a lock at the height under your finger, faint ones too, and the stroke is one change, so one **UNDO** takes it back. A bar you only cross keeps its lock as it was.
- **Take one off.** Hold **ERASE** and tap a bar: its lock comes off, and the step plays the setting's own value again.
- **Shared locks.** A lock belongs to a step, as on Steps, so notes that start in the same step share it: a chord has one lock, not one per note. Changing it changes all of them, and the lane's top right says so ("2.6 kHz · shared by 3 notes"). A pad's setting, such as a kit pad's tune, has its own lane for that pad, and only the notes on that pad show on it.
- **These are Steps' locks.** A lock you draw here is on Steps too, under the same lane, and the other way round. To lock a setting with no lane yet, add its lane on Steps with **+ Lane**.

#### Marks on notes

A note with a lock where it plays has a small square in its top right corner. A note with a condition ("1:2") or a chance under 100 % has a small ring there. On a note wide enough, tap that corner to open the lane its mark is on: the first lock lane with a lock there, or **Chance**. The tap also picks the note, so its bar is outlined; it never takes a note out, even with **Draw** on.

### Change notes with the Q-Links

While notes are picked, the four Q-Links change them, and the Q-Link column says so: its top reads **Note edit** with how many notes are picked (and the chord they make, see [Chords](#chords)), and its four cards, in the same colour, show what each knob changes and the value now, or the range across the picked notes ("80–104").

- **Q1 Position** moves the picked notes a snap each step of the knob. With **Auto** snap each note keeps its feel; with a fixed snap they land on lines. Hold **SHIFT** to move them 4 ticks a step, off the grid.
- **Q2 Length** makes them a snap longer or shorter each step; with **SHIFT**, 4 ticks.
- **Q3 Semitones** moves them up or down a semitone each step. If one would go past the lowest or the highest note, nothing moves, so a chord keeps its shape. With a [scale](#scale) on the clip it reads **Scale steps** and moves them a scale note each step; hold **SHIFT** for semitones.
- **Q4 Velocity** makes them louder or softer, each keeping its difference from the others: the loudest stops at 127 and the softest at 1, and the rest stop with them. With **SHIFT** it widens or narrows the gap between the softest and the loudest, the notes in between keeping their places.
- **One turn, one UNDO.** The Q-Links feel your fingers: from the first knob you touch to the last you let go is one turn, however many knobs you turn, and one **UNDO** takes it all back. Touching the screen, hitting a pad, pressing a button or 3 seconds without turning ends a turn too.
- **Q-LINK** still shows Play's macro banks, over the note knobs; press it until the banks come round again to get **Note edit** back.
- With no notes picked, the Q-Links do what they do on Play.

### – / + and the data wheel

While the drawer is closed:

- **–** and **+** move the picked notes a snap earlier or later, as **Q1** does (the MPC's Nudge). A run of presses is one take. With no notes picked they open the drawer.
- **SHIFT + – / +** zoom the roll in time, around the cursor when it is on screen.
- **The data wheel** moves the **cursor**, a line across the roll in the note edit colour with a mark on the ruler, a snap at a time and round the clip; the roll scrolls to keep it in view. In the tracker the cursor's step is outlined and stays near the top. The cursor marks where the pads write notes with **Pads write** on (see below). **SHIFT + wheel** still scrolls the rows, and pushing the wheel opens the drawer. On the list the wheel opens the drawer, as before.

With the drawer open, the wheel, – and + work the Edit options as before.

### Write notes from the pads

**Pads write**, just left of **Follow** over the roll (at the top right on the list and the tracker), lets you play notes in where the cursor is, one step at a time. It is off whenever you open a clip; tap it and it lights up like **REC**.

- **Press a pad** and its notes go in at the cursor, one snap long, as hard as you hit it. A chord pad writes its whole chord, and on a drum clip a pad writes its hit.
- **The cursor steps on** a snap when you let go of the pad, ready for the next one. Turn the data wheel to move it anywhere first, for a rest or to go back.
- **Hold pads together** for a chord: they all go in on the same step, the cursor steps on once when you let go of the last one, and one **UNDO** takes the whole chord back.
- A pad pressed where the same note already sits takes its place, with the new length and velocity.
- **A chord pad's chord plays whole.** With a chance or a condition, the chord a pad writes plays all together or not at all, just as when you record the pad. Pressed again on the same step, its notes join that chord and keep its chance. Pads held together still decide one by one, as they do when recorded.
- The pads still play as always while you write, so you hear what goes in. Draw on or off makes no difference.
- While the transport records, the pads go into the recording instead, and Pads write leaves them be.

{{shot: sequencer-notes-pads | Pads write lit beside Follow over the roll, and the Sound chip set to Stopped only; the cursor sits on the second bar's last beat, where the next pad writes.}}

With **Pads write** off, the pads just play. On a drum clip, a pad also picks every note it plays in the clip, so you can change all the snares at once with the Q-Links. Hold **ERASE** and press a pad to take out all its notes, as before.

*Try it:* turn the snap to 1/8, tap **Pads write**, then play C, E and G one after another and C, E and G together: three notes up and a chord, each an eighth apart.

### Hear the notes

When you pick a note, draw one or move one, you hear it through the track's instrument, so you can tell what you've got without pressing Play. A note sounds for its own length, up to a second; pick a chord and you hear the chord. Drag a note up or down and you hear each note it passes. What you hear is never recorded and never cuts off a note the clip is playing.

The **Sound** chip, left of **Pads write**, says when you hear them: **Sound: On** (always), **Sound: Stopped only** (only while the transport is stopped, so playing back stays clean) or **Sound: Off**. Tap it for the next. It stays as you set it for every clip. A track that sends to MIDI out plays nothing here.

### Other clips behind

**Show**, just left of **Pads write** over the roll, puts up to three other clips faintly behind the one you're editing, each in its track's colour, so you can write a bass line against the chords or check a new take against the old one. Tap it to open its menu:

- **Plays with** marks the clips that play along with this one: on the same row of the clip grid, or playing now. They come first. Every other clip says **Reference**. A clip of notes offers other clips of notes, and a drum clip other drum clips.
- **Tap a clip** to show it and again to take it off. **None** takes them all off. With three showing, take one off before you pick another. The chip says how many are showing, as in **Show: 2**, and a label at the top right of the roll names each one.
- Each one sits **where it sounds** against this clip. When both are playing, they line up as they do right now. A clip that isn't playing lines up as if you'd launched it with this one, and when this clip isn't playing, both start from their tops. A shorter clip repeats across this one, and a clip at another speed is stretched to match.
- **Tap a faint note** (with Draw off) to open its clip. The clip you were on goes behind it, so you can go back and forth with a tap. Where one of your own notes sits on top, the tap picks yours.
- Hold **COPY** and tap a faint note to paste there, as on empty space. **ERASE** leaves faint notes alone: they aren't this clip's.

Faint clips show on the piano roll only, not on the list or the tracker. Each time you open a clip from the Steps view, it starts with none showing.

{{shot: sequencer-notes-ghosts | The Keys clip with Show: 2: the stab track's Pluck faintly in red between the chords, which plays with it, and Keys take 1 faintly behind the chords, for reference; the labels at the top right name both.}}

{{shot: sequencer-notes-show | The Show menu open: None, then the bass track's two lines and Pluck marked Plays with, and Keys take 1 marked Reference; the two showing are lit.}}

*Try it:* open an empty bass clip on the same row as your chords, tap **Show** and pick the chords (marked **Plays with**). Draw a note under the lowest note of each chord, then tap one of the chords' faint notes to open the chords with the bass behind them.

### Drum clips

A clip of pad hits, the kind a drum track records, shows a row for each pad of the kit in place of the keys: **A1** at the bottom up to **H16**, all 128 pads in their 8 banks. Each row says which pad it is and the kit's name for it, as in **A1 Kick** or **A2 Snare**, and a line marks where each bank starts. A track that plays a synth shows its notes on the keys as before; the clip's own kind decides, not the notes in it.

- **PAD BANK** takes the roll to that bank's first pad, so **B** shows B1 and up, just as the pads go to bank B. Opening an empty drum clip starts on the bank the pads are on.
- **Fold** on **Rows with notes** shows only the pads the clip uses, so a beat on pads all over the kit fits in a few rows.
- Pinch to the smallest zoom (rows 24 pixels tall) to see more pads at once. For tapping in a beat step by step, **Steps** is still the quicker view.
- A drum clip has no scale and no chords, so the **Scale** and **Chord** chips aren't there.
- Everything else works as on the keys: draw, pick, drag a hit to another pad, change velocities in the lane, and write from the pads with **Pads write**.

{{shot: sequencer-notes-drums | A bar of drums: rows A1 Kick up to A6 Rim, named after the kit's sounds, with the kick, snare, hats, clap and rim drawn on them.}}

*Try it:* open a drum clip, press **PAD BANK B**, draw a hit on B1, then press **PAD BANK A** to go back to the kick and snare.

### Zoom and scroll

Rows open at least 40 pixels tall, big enough for a fingertip, and the roll opens with the clip's notes filling them. A short clip opens whole; a long one opens on two bars from its first note.

- **Pinch** with two fingers to zoom, around the point between them. Spread them across to zoom in time, up and down to zoom in pitch, or on a slant for both. The way your fingers first move decides, and it stays that way until you lift them, so a pinch across never drifts in pitch. Rows go from 24 to 96 pixels; time goes from the whole clip to 1/64 notes 40 pixels wide.
- **Two fingers moved together** scroll the roll, any way.
- **One finger on the ruler** (the bar numbers) scrolls in time, and **on the keys** in pitch, so you can get round with one hand on the Q-Links.
- **The overview** over the ruler shows the whole clip with the part on screen outlined. Tap it to jump there, drag along it to move the window, or tap it twice quickly to see the whole clip.
- **Fit**, left of the overview, fits the picked notes on screen, or every note when none are picked.
- **Follow**, at the overview's right end, pages the roll along with the playhead while it is lit. Scrolling by hand turns it off.
- **SHIFT + data wheel** scrolls the rows.

The zoom and scroll stay with each clip: go to Steps or another clip and back, and the roll is as you left it.

### Snap

The snap is the grid moves and new notes land on, and the length of a drawn note. **Auto** (the snap to begin with) follows your zoom: it is the finest grid whose steps are still about 3 mm wide, and the button shows it (**Auto · 1/16**). It moves to a finer grid only once that grid's steps are a little wider still, so a small zoom doesn't make it flip back and forth, and it never changes while your fingers are down. **Auto T** does the same in triplets. Tap the snap button again for a fixed grid, from a bar to 1/64 with triplets, then off. Moves with Auto keep each note's feel; **Timing Correct** is the way to put notes on the grid.

### Scale

Each clip can have a scale of its own, so a melody stays in key while you move and draw notes. It is saved with the clip, like its step size, and changing it is a take: **UNDO** takes it back. A scale never moves a note by itself; it changes how the roll looks and how moves and new notes land.

{{shot: sequencer-notes-scale | The roll with C Dorian on: the scale's rows lit, the C rows marked, and a G#2 marked as off the scale.}}

- **Pick one.** Tap the **Scale** chip above the roll's top right ("Scale: Off" to begin with) for the Scale menu. Its top row is the 12 roots; under them come the scales: **Off**, then the scale the track's pads play in on the Synth tab ("Pads: D Minor", root and all) when they have one, then every scale you can pick for the pads, then the ones you made yourself. Tap a scale to give the clip it on the clip's root (or the pads' root, or C, for the first scale you pick), then tap a root to change the root. **Off** takes the scale away. Tap anywhere else to close the menu.
- **What it shows.** The scale's rows are lit across the roll, and each root row (every C in C Dorian) is lit more strongly, with a mark in the keys. The rows off the scale are dark. A note off the scale has a small dot at its start, in the warning colour, so you can find the notes that clash.
- **Moves stay in key.** Drag a note up or down and it goes from one scale note to the next, skipping the rows off the scale; **Q3** reads **Scale steps** and moves the picked notes a scale note each step. A chord keeps its shape in the scale: C, E and G in C major go up a step to D, F and A. A note off the scale counts the scale note beside it as its first step. Hold **SHIFT** as you start a drag, or while you turn **Q3**, to move by semitones instead. The **Transpose** row of the Edit options always moves by semitones.
- **New notes land in key.** With a scale on, a note you draw or tap in on a row off the scale lands on the scale note nearest it (the lower one when two are as near). Hold **SHIFT** to put it exactly where you tap.
- **Fit to scale.** The last row of the Edit options moves the picked notes, or every note when none are picked, onto the nearest scale note. Its value says how many notes are off the scale ("3 off"); turn the data wheel either way to fit them, and one **UNDO** takes it back.
- **Find them.** **Off the scale** in the Select menu picks every note off the clip's scale.
- **Pads.** A drum clip has no scale, so the **Scale** chip isn't there (see [Drum clips](#drum-clips)). That includes an older clip on a synth track that follows the pads; convert it to notes first ([Melody: Convert to notes](clips.md#melody-convert-to-notes)) to give it a scale.

{{shot: sequencer-notes-scales | The Scale menu open over the roll: the roots along the top, then the pads' scale and every other scale.}}

**Fold** sets which rows the roll shows. Tap the chip beside **Scale** to step through:

- **All rows**: every pitch, as to begin with.
- **Rows with notes**: only the rows that have a note on them, which is handy for a drum part played on keys, or a long part with a few pitches.
- **Scale rows** (while the clip has a scale): only the scale's rows, so every row you tap is in key.

Where rows are folded away, a bold line between the rows shows the gap. A row with a picked note always shows, so you never lose sight of what you are editing. When Fold hides notes (notes off the scale, with **Scale rows**), a chip says how many ("2 notes hidden"); tap it to show every row again. The roll keeps each clip's Fold.

### Chords

With the **Chord** chip on, a note you draw puts in a whole chord, built up from the row you touch.

{{shot: sequencer-notes-chords | The Chord menu open over the roll on Seventh, with Major lit; a chord is picked and the Q-Link column names it: "4 notes · G#".}}

- **Pick a shape.** Tap the **Chord** chip, left of **Scale** ("Chord: Off" to begin with), for the Chord menu: **Off**, **Triad**, **Seventh**, **Sus2**, **Sus4**, **Power** (the root and the fifth), **Octave**, and **Pads** while the pads have a scale. A shape closes the menu; tap anywhere else to close it without changing anything.
- **In key.** With a [scale](#scale) on the clip, a Triad or a Seventh takes the scale's own notes, as the Chord pads do: in C major, a D gives D F A (D minor) and a G gives G B D F (G7). With no scale, the menu also has **Major** and **Minor**, and the Triad and the Seventh follow them (the chip says "Chord: Minor triad"). Sus2, Sus4, Power and Octave are the same shape in any key.
- **Pads** puts in the triad the Chord pads play: in the scale the track's pads use on the Synth tab, starting from the scale note nearest the row you touch.
- **Draw.** With **Draw** on, tap an empty spot for a chord one snap long, or drag along a row for a chord as long as your drag; with Draw off, tap twice. While you drag, the whole chord shows where it will go. The chord's notes are picked afterwards, so the Q-Links, a drag or **Delete** act on the whole chord, and one **UNDO** takes it all back. Each note is its own note: a tap with Draw on takes just that one out.
- **Its name.** When the picked notes make a chord, the top of the Q-Link column names it after the count ("3 notes · Dm7", or "C/E" when the lowest note isn't the root).
- **It plays whole.** A chord you draw plays all together or not at all, like a chord pad's: with a chance or a condition, the whole chord rolls once. Drawn on a step that has a chord already, it is a chord of its own there.
- A clip of pad hits takes no chords: it has no Chord chip.

### Tools: Strum, Invert, Spread, Chop, Legato, Humanize

The last rows of the Edit options are tools. Pick some notes, then tap either arrow of a tool's row (or turn the data wheel on it): the tool opens on the picked notes. The roll shows the result straight away and the Q-Links change it, but nothing goes into the clip until you tap **Done**, so you can try settings freely.

{{shot: sequencer-notes-tool | Chop open on two long notes played with pressure: the roll shows each cut into four, every piece keeping its part of the swell. The tool's bar has Cancel, Reset pressure and Done; Q1 says Pieces: 4.}}

- **The tool's bar** takes the place of the buttons under the roll: **Cancel** on the left closes the tool and leaves the clip as it was; **Done** on the right puts the result in, and one **UNDO** takes it all back. Between them, the tool's name and what it shows, or why it changes nothing ("Nothing changes yet").
- **The Q-Link column** shows the tool's settings in place of Position, Length, Semitones and Velocity. Hold **SHIFT** while turning for the finest steps. Turning the data wheel on the open tool's row turns its Q1.
- While a tool is open, the notes stay as they are under your fingers: tapping or dragging them, the lane, and other edits wait until you tap Done or Cancel (the log says so). Two fingers still zoom and scroll, and the overview, Fit and Follow still work. Tap another tool's row to open that one instead.

| Tool | What it does | Q-Links |
|---|---|---|
| **Strum** | Starts each note of a chord a little after the one under it, as a strummed guitar does. Each note keeps its end. A strummed chord's notes start apart, so each then rolls its own chance. | **Q1 Apart**: the notes this many ticks further apart (about 5 ms at 120 BPM a step; turn left to bring them closer, down to together). **Q2 Way**: As it is, **Up** (the lowest note first) or **Down** (the highest first). |
| **Invert** | Takes a chord's lowest note up an octave, or its highest down one. | **Q1 Inversions**: Up 1, Up 2 ... to the right; Down 1 ... to the left. |
| **Spread** | Opens a chord: the note above the lowest goes up an octave (C E G becomes C G E). Needs chords of three notes or more. | **Q1 Spread**: Open 1 ... to the right; Close 1 ... to the left, bringing the highest note down while it stays above the lowest. |
| **Chop** | Cuts each note into equal pieces. Each piece keeps its part of the pressure the note was played with, starting from the pressure held where it was cut. **Reset pressure** (on the tool's bar, lit while on) leaves the pieces with none. A chord that plays whole or not at all still does at each piece. | **Q1 Pieces**: 2 to 16. |
| **Legato** | Holds each note on toward the next note that starts after it (notes that start together all go to the same next note; the last notes keep their length). A note that already runs past the next start comes back toward it. | **Q1 Fill**: how much of the gap, 0 to 100 %. 100 % meets the next note exactly. |
| **Humanize** | Nudges each note's time, velocity and length by a small random amount, so a drawn part sounds played. **Again** (on the tool's bar) tries another set of random amounts. The **Humanize** MIDI effect does this as the notes play, differently each time round; this tool changes the notes in the clip once. | **Q1 Timing**: up to this many ticks either way. **Q2 Velocity** and **Q3 Length**: up to this percent either way. **Q4 Ahead/behind**: leans the timing ahead of the beat or behind it ("Even" for neither). |

Strum, Invert and Spread work on chords: the picked notes that sound together, each chord on its own, so a whole run of chords changes at once. Their rows show the first chord's strum ("↑ 20t", "↓ 20t", "none"), its name ("C/E") and whether it is "open" or "close"; Chop, Legato and Humanize say "open"; the open tool's row says "on".

Every edit is a take, so **UNDO** takes it back and **SHIFT + UNDO** puts it back again.

If the clip changes while you edit, because you're recording into it, pressed **UNDO**, or tapped a step on Steps, your picked notes stay picked wherever they now sit. If a picked note was taken out, the picks are let go and the line over the roll says "the clip changed: picks cleared". A tap on a note that landed just as the clip changed does nothing, so it never hits the wrong note: tap again. If an edit can't be played (rare), it is taken back and the log says so. Tap **‹ Steps**, or press **STEP SEQ**, to close the editor and see the clip as steps again. The editor stays open while you visit other tabs, and the Sequencer tab shows it again when you come back.

## The tracker

The tracker shows the clip as a list of steps from top to bottom, empty steps included, the way a tracker does. It is handy for drum clips, where you want to read every step's settings at a glance.

{{shot: sequencer-notes-tracker | The tracker on a bar of drums: a nudged hat, a 1:2 hat, a FILL open hat, a ratchet, and the picked snare with a lock.}}

- **One row per step**, at the snap's size (with Auto, the grid your roll's zoom picked). A step with several notes, such as a kick and a hat together, gives each note a row of its own under the step. An empty step shows dots. Rows are 40 pixels tall, as are the list's, so a tap lands on the row you meant.
- **The columns**: the step (bar.beat.step), the pad or note, velocity, length, nudge (how many ticks early or late), chance, condition, ratchet (with its velocity ramp when it has one: "×4 -40" is four hits, the last 40 softer than the first), and how many parameter locks the step has. A setting left at its default shows as a faint dot, so what you changed stands out. A note whose step is off has a faint row and says "off" after its name; a tied or sliding note says "tie" or "slide".
- **Tap a note's row** to pick it, then change it with the Q-Links or the Edit options. With **Draw** on, tap an empty step to put a note there: the sound of the last note you tapped, a step long. Hold **ERASE** and tap a note's row to take it out.
- The step on each beat is shaded, a bar starts with a line, and a green mark shows the step playing.

With the snap off, or finer than 1/32, the tracker uses 1/16 steps, and its title line says so. The names of the parameters a step locks are coming; for now it shows how many there are.

## Timing Correct

Timing Correct pulls notes toward the beat, so a part played a little loose sits in time. Open the drawer (**Edit**) and tap **Correct picked** under the Edit options to correct the picked notes; with none picked it reads **Correct all** and corrects every note. The MPC's **TC** button gets you here from any page: it opens the note editor on the track you are on with the drawer open on Timing Correct's settings, and pressed again it corrects. The settings are the first rows of the Edit options. Tap a row and turn the data wheel to change it:

- **Correct**: what moves. **Start** moves each note's start onto the grid. **End** moves its end. **Length** makes its length a whole number of grid steps. **Legato** holds each note until the next one starts.
- **Division**: the grid, from a whole bar to 1/64, with triplets.
- **Swing**: 50 % is straight. Higher values push every second step later, for a shuffle.
- **Window**: only notes this close to a grid line are corrected. At 100 %, every note is.
- **Strength**: how far of the way each note moves. At 100 % it lands on the line. Lower values keep some of the feel.
- **Shift**: moves the corrected notes a little early or late afterwards.

The next rows act on the picked notes straight away: **Nudge** moves them a step, **Transpose** a semitone, **Velocity** up or down, **Length** longer or shorter, **Chance** 5 % at a time, **Condition** through the trig conditions (Always, 1:2 to 8:8, then FILL, !FILL, PRE, !PRE, NEI, !NEI, 1ST and !1ST), and **Ratchet** from 1 to 8 hits. A run of turns on the same row is one take, so one **UNDO** takes it all back. **Fit to scale**, after the Euclidean fill's rows, moves notes onto the clip's scale (see [Scale](#scale)), and the last rows open the tools: **Strum**, **Invert**, **Spread**, **Chop**, **Legato** and **Humanize** (see [Tools](#tools-strum-invert-spread-chop-legato-humanize)).

## Euclidean fill

The Euclidean fill spreads a number of hits as evenly as they go over a number of steps, a quick way to write a groove: 4 hits over 16 steps is four on the floor, 3 over 8 is the tresillo, 5 over 8 the cinquillo. It writes one sound's row, and its four rows are the last of the Edit options:

- **Euclid**: the sound it writes. It follows the last note you tapped; turn it to pick another pad or note, and that sound's notes light up.
- **Hits**: how many hits.
- **Steps**: how many steps the pattern spans (up to 64), at the snap's size, for example "16 × 1/16".
- **Rotate**: turns the pattern so it starts later.

Each turn of Hits, Steps or Rotate writes the pattern straight away, repeated through the whole clip. Steps that keep a hit keep their notes as they were; new hits are a step long and as loud as the sound's first note. Other sounds stay as they are. A pattern of 12 steps on a 16-step clip runs on into the next bar, for a part that drifts against the others. A run of turns is one take, so one **UNDO** puts the row back as it was.

> [!MPC]
> This is the MPC's Grid View, List Edit and Timing Correct in one place. TC here corrects notes already recorded, and the **TC** button opens it as on the MPC. To correct notes as you record them, set **Quantise** on the Record tab (see [Recording a clip](clips.md#recording-a-clip)), or press **SHIFT + TC**, the MPC's TC On/Off. The tracker and the Euclidean fill aren't on the MPC: the fill is like the Euclidean mode of Elektron's Digitakt.

