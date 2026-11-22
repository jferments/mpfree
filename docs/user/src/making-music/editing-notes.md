<!--
status: changing
covers: rust/mpfree-time-ui/src/editor.rs rust/mpfree-time-ui/src/editor/zoom.rs rust/mpfree-time-ui/src/editor/gesture.rs rust/mpfree-seq/src/notes.rs rust/mpfree-app/src/time.rs
checked: 2026-11-22 829e1568
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

The notes are bars on a piano roll: higher notes higher up, time running left to right, longer notes longer. A drum clip shows its 16 pads instead of keys. Under the roll, the velocity lane shows how hard each note plays. A note played with [pad pressure](clips.md#pad-pressure) shows it as a lighter shape along its bottom: it rises where you pressed harder and falls where you let up.

Notes are drawn as they will play. A note whose step is off (tapped off on Steps) is only an outline: it keeps its place and settings and plays nothing. A note that repeats (a ratchet) shows each hit, lighter or darker as its velocity ramps, and each hit shows in the velocity lane too. A tied note has a short level mark just before it, and a sliding note a short slanting one.

- **Draw on: tap a beat in.** With **Draw** lit, the roll works like the Steps view. Tap an empty spot to put a note there, one snap long and as loud as the last note you used. Tap a short note (a step long or less) to take it out again. A longer note is picked instead, so a quick tap never loses a long note. Drag along an empty row to draw a note as long as your drag. Hold a note for a moment to pick it without taking it out.
- **Draw off: pick and change.** Tap a note to pick it, just that one. A tap just beside a note, in its row, picks it too, so you don't have to hit a short note exactly. Tap an empty spot to let the picks go, or tap it twice quickly to put a note there.
- **Pick a group.** With Draw off, drag across an empty spot to draw a box: when you lift, every note sounding inside it is picked instead of the ones before. Or hold one note (with Draw on or off) and, still holding it, tap other notes: each tap adds a note to the picks, or takes it out if it was picked, as holding steps does on the step pads.
- **The Select menu.** Tap **Select** for a menu over the roll. **All** picks every note, **None** lets them go, and **Invert** picks the notes that weren't picked instead of the ones that were. **Fit to picked** zooms the roll to the picked notes. The rest pick from the whole clip: **Same pitch** (every note at the pitch of the picked ones, or of the sound you last worked on), **Pitch range** (every note from the lowest picked note to the highest), **Time range** (every note starting from the first picked note to the end of the last; for both ranges, pick a few notes first), **Quiet notes** and **Loud notes** (softer, or harder, than halfway between the clip's softest and loudest), **With a condition**, **With repeats** (a ratchet) and **Overlapping** (notes that sound over another note of the same pitch). Tap anywhere else to close the menu.
- **Add to pick.** In the Select menu, **Add to pick** is a switch: while it is on, it and **Select** stay lit. Then a tap on a note adds it to the picks or takes it out (with Draw on, a short note still comes out), a box adds the notes in it, and the menu's choices add to the picks you have, so you can gather notes of several kinds: **Same pitch**, then a box somewhere else. Tap it again to turn it off. **All**, **None** and **Invert** always replace the picks.
- **Nothing happens until you lift.** A finger on a note is a tap when it lifts, a hold once it has stayed still for 0.4 seconds, or a drag once it moves about 3 mm. Nothing goes in or comes out before your finger lifts.
- **Erase.** Hold **ERASE** and tap a note to take it out, with Draw on or off.
- **Copy and paste.** Hold **COPY** and tap a note: the picked notes are copied, or just that note if it isn't picked. Then hold **COPY** again and tap an empty spot, in this clip or in another track's clip: the copy goes in starting where you tap, each note keeping its pitch and its place after the first. The notes bring their pad pressure and their locks. A lock on the instrument comes into another track's clip when that track plays the same kind of instrument the lock was made for, so it turns the same control there; a clip's instrument locks are all for one instrument, so it doesn't come into a clip whose instrument locks are for another. A lock on a kit pad, an insert or MIDI out comes only into another clip of the same track, since another track may play another kit or send to other gear. The hint line says how many locks were left out and what they were on, and for instrument locks which instrument they were made for ("made for Poly, Lead plays FM"). Pad hits paste only into a clip of pad hits, and notes into a clip of notes. A chord whose notes play together or not at all (a chord pad's) stays one when you copy all of it, and stays apart from a chord already where it lands; copy only some of its notes and each of them rolls its own chance. The pasted notes are picked, and one **UNDO** takes the paste back. The copy stays until you copy again, so you can paste it as many times as you like. In the tracker, tap an empty line to paste there.
- **Move them.** Drag a note to move it, along time and up or down in pitch. If it is picked, every picked note moves with it; if not, it moves on its own and is picked after. With **Auto** snap a move goes by whole grid steps and each note keeps its feel: a note played a little late stays a little late. With a fixed snap, the note you drag lands on a line. While you drag, the notes show where they will land, an outline shows where they were, and a box above your finger says where the note you hold lands ("2.3.000 · D3").
- **Change their length.** A picked note has a handle at each end, just outside it; on a note too short to grab, the two handles hang just under it. Drag the right handle to change where the picked notes end, or the left one to change where they start (their ends stay put). The box above your finger shows the new length or start, and it all goes in when you lift. Handles show while up to 64 picked notes are on the screen; for more, use the **Length** option.
- **Velocity.** Slide a finger along the velocity lane to set the picked notes' velocity. Their bars follow your finger, and the new velocity goes in when you lift it, so **UNDO** takes back the whole slide.
- **One finger edits, two move the view.** A drag belongs to the finger that started it. If a second finger touches the roll while you drag, the drag stops, the notes stay where they were, and the two fingers zoom and scroll instead (see below).
- **Duplicate** copies the picked notes to just after them. **Delete** takes them out (in Perform, hold it for half a second).
- **Grid, List, Tracker.** The second button says which view is showing (**Grid** to begin with) and switches to the next. **List** shows the notes as a list, one row per note, with its position, note, velocity, length and chance. An off note's row is faint and its note says "off"; a tied or sliding note's says "tie" or "slide". **Tracker** shows every step, described below.

{{shot: sequencer-notes-list | The same notes as a list, the picked ones lit.}}

### Change notes with the Q-Links

While notes are picked, the four Q-Links change them, and the Q-Link column says so: its top reads **Note edit** with how many notes are picked, and its four cards, in the same colour, show what each knob changes and the value now, or the range across the picked notes ("80–104").

- **Q1 Position** moves the picked notes a snap each step of the knob. With **Auto** snap each note keeps its feel; with a fixed snap they land on lines. Hold **SHIFT** to move them 4 ticks a step, off the grid.
- **Q2 Length** makes them a snap longer or shorter each step; with **SHIFT**, 4 ticks.
- **Q3 Semitones** moves them up or down a semitone each step. If one would go past the lowest or the highest note, nothing moves, so a chord keeps its shape.
- **Q4 Velocity** makes them louder or softer, each keeping its difference from the others: the loudest stops at 127 and the softest at 1, and the rest stop with them. With **SHIFT** it widens or narrows the gap between the softest and the loudest, the notes in between keeping their places.
- **One turn, one UNDO.** The Q-Links feel your fingers: from the first knob you touch to the last you let go is one turn, however many knobs you turn, and one **UNDO** takes it all back. Touching the screen, hitting a pad, pressing a button or 3 seconds without turning ends a turn too.
- **Q-LINK** still shows Play's macro banks, over the note knobs; press it until the banks come round again to get **Note edit** back.
- With no notes picked, the Q-Links do what they do on Play.

### – / + and the data wheel

While the drawer is closed:

- **–** and **+** move the picked notes a snap earlier or later, as **Q1** does (the MPC's Nudge). A run of presses is one take. With no notes picked they open the drawer.
- **SHIFT + – / +** zoom the roll in time, around the cursor when it is on screen.
- **The data wheel** moves the **cursor**, a line across the roll in the note edit colour with a mark on the ruler, a snap at a time and round the clip; the roll scrolls to keep it in view. In the tracker the cursor's step is outlined and stays near the top. The cursor marks where the pads will write notes once they can. **SHIFT + wheel** still scrolls the rows, and pushing the wheel opens the drawer. On the list the wheel opens the drawer, as before.

With the drawer open, the wheel, – and + work the Edit options as before.

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

The next rows act on the picked notes straight away: **Nudge** moves them a step, **Transpose** a semitone, **Velocity** up or down, **Length** longer or shorter, **Chance** 5 % at a time, **Condition** through the trig conditions (Always, 1:2 to 8:8, then FILL, !FILL, PRE, !PRE, NEI, !NEI, 1ST and !1ST), and **Ratchet** from 1 to 8 hits. A run of turns on the same row is one take, so one **UNDO** takes it all back.

## Euclidean fill

The Euclidean fill spreads a number of hits as evenly as they go over a number of steps, a quick way to write a groove: 4 hits over 16 steps is four on the floor, 3 over 8 is the tresillo, 5 over 8 the cinquillo. It writes one sound's row, and its four rows are the last of the Edit options:

- **Euclid**: the sound it writes. It follows the last note you tapped; turn it to pick another pad or note, and that sound's notes light up.
- **Hits**: how many hits.
- **Steps**: how many steps the pattern spans (up to 64), at the snap's size, for example "16 × 1/16".
- **Rotate**: turns the pattern so it starts later.

Each turn of Hits, Steps or Rotate writes the pattern straight away, repeated through the whole clip. Steps that keep a hit keep their notes as they were; new hits are a step long and as loud as the sound's first note. Other sounds stay as they are. A pattern of 12 steps on a 16-step clip runs on into the next bar, for a part that drifts against the others. A run of turns is one take, so one **UNDO** puts the row back as it was.

> [!MPC]
> This is the MPC's Grid View, List Edit and Timing Correct in one place. TC here corrects notes already recorded, and the **TC** button opens it as on the MPC. To correct notes as you record them, set **Quantise** on the Record tab (see [Recording a clip](clips.md#recording-a-clip)), or press **SHIFT + TC**, the MPC's TC On/Off. The tracker and the Euclidean fill aren't on the MPC: the fill is like the Euclidean mode of Elektron's Digitakt.

