<!--
status: changing
covers: rust/mpfree-time-ui/src/editor.rs rust/mpfree-time-ui/src/editor/zoom.rs rust/mpfree-time-ui/src/editor/gesture.rs rust/mpfree-seq/src/notes.rs rust/mpfree-app/src/time.rs
checked: 2026-11-22 829e1568
-->

# Editing notes

The note editor shows a clip's notes so you can fix what you played: pull notes into time, move them, change how hard they hit, draw new ones and erase others. It is the **Sequencer** tab's **Notes** view: tap **Notes** at the right end of the Sequencer's transport bar, or **Edit ›** over the clip grid on Arrange, which takes you there. It opens the clip the Sequencer's Steps show, which is the armed track's playing clip, or its first clip.

{{shot: sequencer-notes | The note editor's piano roll across the screen, the overview strip over it. Two long melody notes show the pressure they were played with.}}

The Notes view has the Arrange tab's transport bar along the top (**Stop**, **Play** and **Rec**, the record mode, the position, the tempo, the time signature, the launch line, **Undo** and the REC chip), the roll across the screen, and a row of buttons under the roll. The **Edit** options and the clip's **Takes** are in a drawer at the right: **Edit**, **TC** or the data wheel opens it, and the roll makes room.

| Button | What it does |
|---|---|
| **‹ Steps** | Back to the Steps view (so does **STEP SEQ**). |
| **Grid** | Which view shows: Grid, List or Tracker. Tap it for the next. |
| **Select** | The tool: Select, Draw or Erase. Tap it for the next. |
| **Auto · 1/16** | The snap: the grid that moves and new notes land on. **Auto** follows your zoom (see [Snap](#snap)); tap for Auto triplets, then a fixed grid from a bar to 1/64, then off. |
| **All** or **None** | Picks every note, or lets them go. |
| **Duplicate** | Copies the picked notes to just after them. |
| **Delete** | Takes the picked notes out (in Perform, hold it). |
| **Edit** | Opens the drawer with the Edit options (Timing Correct and more) and the clip's Takes; lit while it shows. Tap it again to close it. |

## The piano roll

The notes are bars on a piano roll: higher notes higher up, time running left to right, longer notes longer. A drum clip shows its 16 pads instead of keys. Under the roll, the velocity lane shows how hard each note plays. A note played with [pad pressure](clips.md#pad-pressure) shows it as a lighter shape along its bottom: it rises where you pressed harder and falls where you let up.

Notes are drawn as they will play. A note whose step is off (tapped off on Steps) is only an outline: it keeps its place and settings and plays nothing. A note that repeats (a ratchet) shows each hit, lighter or darker as its velocity ramps, and each hit shows in the velocity lane too. A tied note has a short level mark just before it, and a sliding note a short slanting one.

- **Pick notes.** Tap a note to pick it. **All** picks every note, and **None** lets them go.
- **Move them.** Drag a picked note and every picked note moves with it, along time and up or down in pitch. With **Auto** snap a move goes by whole grid steps and each note keeps its feel: a note played a little late stays a little late. With a fixed snap, the note you drag lands on a line.
- **Velocity.** Slide a finger along the velocity lane to set the picked notes' velocity. Their bars follow your finger, and the new velocity goes in when you lift it, so **UNDO** takes back the whole slide.
- **One finger edits, two move the view.** A drag belongs to the finger that started it. If a second finger touches the roll while you drag, the drag stops, the notes stay where they were, and the two fingers zoom and scroll instead (see below).
- **Draw and erase.** The tool button switches between Select, Draw and Erase. With Draw, tap an empty spot to add a note there, one snap long: it goes in when you lift your finger, so a pinch that starts on an empty spot adds nothing. With Erase, tap a note to take it out.
- **Duplicate** copies the picked notes to just after them. **Delete** takes them out (in Perform, hold it for half a second).
- **Grid, List, Tracker.** The second button says which view is showing (**Grid** to begin with) and switches to the next. **List** shows the notes as a list, one row per note, with its position, note, velocity, length and chance. An off note's row is faint and its note says "off"; a tied or sliding note's says "tie" or "slide". **Tracker** shows every step, described below.

{{shot: sequencer-notes-list | The same notes as a list, the picked ones lit.}}

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
- **Tap a note's row** to pick it, then change it with the Edit options. With the Draw tool, tap an empty step to put a note there: the sound of the last note you tapped, a step long. With Erase, tap a note's row to take it out.
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

