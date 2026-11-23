<!--
status: changing
covers: rust/mpfree-seq rust/mpfree-play rust/mpfree-capture rust/mpfree-time-ui/src/view.rs rust/mpfree-time-ui/src/widgets.rs rust/mpfree-transport
checked: 2026-11-19 f0b39161
-->

# Clips and patterns

A **clip** is a piece of music you recorded or programmed: a drum pattern, a bass line, a chord part. The pattern on the Play page is a clip. Clips live on **tracks**, and the **Arrange** tab shows them all as a grid so you can record new ones, keep the best takes and switch between them while the music plays.

{{shot: clips | The Arrange tab mid-jam: tracks across the top, clip slots down (playing, waiting, recording and empty), the record settings and the takes beside them, and the Q-Links down the right edge.}}

## Tracks and slots

The grid has a column per **track** and a row per **slot**. Each track plays one clip at a time, so a column might hold four different drum patterns, and you pick which one plays. In Make the grid shows 8 rows at a time, each clip with its name and length (and a small ring going round once a pass while it plays); in Perform it shows 5 bigger rows (see [Make and Perform](playing-live.md#make-and-perform)). SHIFT + the wheel scrolls the rest.

- **Tap a clip** to launch it. It starts on the next bar line (or the line **Launch** sets, below), so it always lands in time. Tap a playing clip to stop it at the next bar. A clip launches when you lift your finger, because you can also drag it.
- **Tap +** in an empty slot to make a new clip there, ready to record into. It happens when you lift your finger, so a drag that starts on an empty slot makes nothing. In Perform, hold the slot for half a second instead (see [Make and Perform](playing-live.md#make-and-perform)).
- **Drag a clip** to move it: once your finger moves, it lifts and follows you, and the slot it will land in is outlined. Lift your finger to drop it. In an empty slot it moves there; onto another clip, the two change places, so nothing is lost. In another track's column it plays that track's sound. A clip that is playing keeps playing where it lands, and the song's blocks that use it go with it. Hold **SHIFT** as you lift to drop a copy instead (into an empty slot). Lift it where it started to leave it be.
- **Drag a row's number** to move the whole row, every track's clip in it: a line shows where it will go, and the rows in between move up or down one, so nothing is written over. With **SHIFT** held as you lift, a copy of the row goes in at the line and the rows under it move down one (the last row has to be empty). A tap on the number still plays the row.
- Dragged to the grid's edge, a clip or a row turns the page: the tracks before or after, or the slots above or below.
- **UNDO** takes back any drop, and the Takes list names it ("Move 7 · Drums / 2, to Bass / 4").
- **Tap a track's name** to **arm** it: the pads record into that track. The track arms when you lift your finger.
- **Hold ERASE and tap a clip** to delete it. **UNDO** brings it back. A clip the song uses leaves the grid, and the song keeps it and plays on; **UNDO** puts it back on the grid.
- **Hold SHIFT and tap a clip**, a row's number or a track's name, then press a pad or key on a MIDI controller: from then on that control launches it, as a tap would. See [Launching clips and scenes from your controller](midi-and-cv.md#launching-clips-and-scenes-from-your-controller).
- **Hold COPY and tap a clip** to copy it. Then **hold COPY and tap an empty slot**, on any track and any page, and a copy goes there; tap more empty slots for more copies. **UNDO** takes a copy away. It's handy for a variation: copy the verse, then change the copy.
- **Under each name** is what the track plays: **Kit**, or its own instrument (FM, Poly, a plugin synth). Tap that line to change it: the track is armed, and the Synth tab opens its MENU on the track's instrument row (see [A synth for each track](synths.md#a-synth-for-each-track)).

{{shot: arrange-instruments | Each track's instrument under its name: Track 2 plays FM, Track 4 Poly and Track 6 a plugin synth, and the rest play the kit.}}

### More tracks

A new project has 8 tracks, and a project can have up to 128, as on Akai's MPC. **+ Track**, on the grid's title line, adds one: it is named "Track 9" and so on, armed, ready to record. **UNDO** takes it away again.

The grid shows 8 tracks at a time. With more than 8, the title line shows which ones, such as "9–16 of 24": tap **‹** or **›**, or swipe along the track names, to see the 8 before or after. When you arm a track that's out of view, from the Record panel's **Armed** row for example, the grid moves to it.

{{shot: clips-tracks | A project of 24 tracks: the grid shows tracks 9 to 16 because Vox chop is armed, and the pager says so.}}

Every track can have a kit of its own (see [A kit for each track](pads.md#a-kit-for-each-track)) and its own MIDI out to your gear (see [Playing a synth from a track](midi-and-cv.md#playing-a-synth-from-a-track)). Any track can also have an instrument of its own, up to 16 tracks at once (see [A synth for each track](synths.md#a-synth-for-each-track)).

### Loop tracks

The loops you record on the Play page or the Looper tab show on **Loop tracks**, columns named **Loops A**, **Loops B** and on, added at the end of the grid as they are needed, with a little wave and **Pads** under the name (they record the pads and the synth). Each cell names its loop ("Loop 1"), shows its wave and length, and launches like a clip: one loop plays per Loop track, and loops on different Loop tracks play together. The pads don't play a Loop track. **SHIFT + + Track** adds an empty one, and a tap on an empty Loop cell arms it, so REC records a loop there. See [Loops in the Clips grid](looping.md#loops-in-the-clips-grid).

A clip's border tells you what it's doing: a ring going round as it plays one pass, amber while it waits to start, red while recording. A loop's cell has no ring: its wave lights up to where the loop is.

## The Q-Links on Arrange

The four Q-Link cards run down the right edge of Arrange, in the same place as on the Play page, so the knobs never move between the two. They do the same as on Play: the knobs of the Play view you picked last, as the cards show (see [The Q-Links on the Play page](playing-live.md#the-q-links-on-the-play-page)). Turn a knob, or drag up or down on its card. **Q-LINK** steps the cards to Macros 1-4 and Macros 5-8, and Play and Arrange share the bank.

With an audio region picked in the Song view, the cards turn the audio track's level and pan and the region's gain and fades instead (see [Recording audio tracks](audio-tracks.md#editing-regions)).

Coming:
- A track with its own instrument has its own mixer channel (see [A synth for each track](synths.md#a-synth-for-each-track)), but its level and pan aren't on the knobs here yet. Tracks that play the kit share the kit's channels.

## Recording a clip

Set up the recording in the Record panel, beside the grid:

- **Armed:** the track that records.
- **Mode:** **Replace** records over what's there; **Overdub** adds to it. The **OVERDUB** button switches this too.
- **Count-in:** a count before recording starts, so you can come in on time. **SHIFT + OVERDUB** turns it off and back on from any page.
- **Loop:** plays the same bars round and round while you record, so you can keep trying a part. It counts in bars from bar 1.
- **Tempo:** the song's tempo, in BPM.
- **Pressure:** whether the armed track records how hard you press the pads while they're held (**Recorded**, the setting for a new track, or **Off**).
- **Quantise:** puts each note you record on the nearest line of a grid, from **1/4** to **1/32** with triplets, as the MPC's Timing Correct does while recording. **Off** (the setting for a new track) keeps the notes exactly as you played them. It belongs to the armed track, so drums can go in on 1/16 while the bass keeps its feel. The clip's own **Quantise** (see [Clip settings](#clip-settings)) still moves the notes as they play, and you can change that later; this one moves them for good, so they sit on the steps on the Sequencer and in the note editor. **SHIFT + TC** turns it off, and on again at the grid it had (1/16 the first time), for the track you are on.
- **Launch:** when a clip you tap, a stop or a scene starts while the music plays: on the next bar line (**1 bar**, the setting for a new project), the next **2**, **4** or **8** bars for a big change, the next **1/2** to **1/16** note for a quick one, or **at once**. The transport bar shows it as **Launch 1 bar**. A clip with a Launch of its own on the Clip tab (below) keeps it; a scene's clips all start together on this one. Change it while something waits and that still starts where it was going to.
- **Transpose:** shifts every note the armed track plays, live, without changing its clips: **as written**, or up to 24 semitones either way (**+5 st**). A track set to transpose in key shows **+3 in key** and steps through its scale. Its notes already sounding keep their pitch; the next ones move. 16 Levels' Transpose sets it from the pads (see [Transpose](playing-live.md#transpose)). Only a track that plays notes (the synth, or its own instrument) has it; a kit's track shows **–**.
- **Follows:** **On** and this track moves with whichever track you transpose from the pads, so a bass and its chords change key together. **Off** (the setting for a new track) keeps its own.

**Pressure**, **Quantise**, **Launch**, **Transpose** and **Follows** are the last rows: pick Loop or Tempo and the list moves up to show them.

Tap a row's arrows, or the left or right of its value, to change it. Or turn the data wheel to pick a row and press **–** or **+**. Push the wheel and turning it changes the row picked; push again to go back to picking rows.

Then press **REC** and **PLAY**, play, and press **STOP**. The new notes go into the armed track's clip. Picking a track arms its notes, and the REC chip at the right of the transport bar names the clip REC records into (see [What REC records](sequencer.md#what-rec-records)).

### Erasing while you record

Hit a wrong note? Keep recording, hold **ERASE** and hold that pad: its notes come out of the clip as the playhead passes them, on every pass, and the pad makes no sound while you hold it. On a track that plays a synth, those are the notes the pad plays (a chord pad's every note), so a note another pad plays too comes out as well. The other pads play on. Let go of the pad, or of **ERASE**, and it stops. It erases on the armed track only, and the notes it took out are part of the take, so **UNDO** brings them back with it. With **STEP SEQ** on, **ERASE** and a pad clears that step instead.

It works the same while the music only plays, without recording: hold **ERASE** and the pad for as long as you want its notes gone. Each time you hold **ERASE** is one **UNDO** step ("Erase 1"). With the music stopped, **ERASE** and a pad takes all of that pad's notes out of the clip at once, also one **UNDO** step.

### Knob moves while you record

Turn a knob while you record and the move goes into the clip as a **lock** on the step you're playing: a knob of the armed track's synth on the Synth tab (or a Q-Link turning it), its mixer strip's fader, pan or sends, or an effect's knob on its strip. Hold a pad and turn, and the lock goes on that pad's note; turn just after a hit and it goes on that hit; otherwise it lands on the nearest step with no note of its own. A turn is one lock, however far you turn it, and it plays back on every pass (see [Locks](sequencer.md#the-pads-as-steps-step-seq) for what a lock does). The take's **UNDO** takes the locks back with its notes. Only the armed track's knobs record. A knob on your own controller that you taught with [MIDI Learn](midi-and-cv.md#recording-a-learned-control) records the same way. Coming: the Play page's Q-Links and a kit pad's sound knobs on the screen.

### Pad pressure

The pads feel how hard you keep pressing after you hit them. While you record, that pressure goes into the clip with each note, and the clip plays it back the way you played it: a synth swells, opens up or wobbles just as it did live. It works on drum pads too, where a sound's pressure settings use it.

- The pressure stays with its note. If the clip's quantise moves a note into time, or you move the note in the note editor, its pressure moves with it.
- MPFree keeps only the movement that matters: a change of two steps or more right away, a small change once it has lasted a moment (10 ms). A finger's tiny wobble isn't kept, so clips stay light.
- The clip keeps the raw pressure, and each sound decides what pressure does. Change a sound's pressure curve and the clip plays back through the new one.
- Splitting a note in the note editor keeps its pressure on the first part only.
- The note editor shows it: a note you leaned into has a lighter shape along its bottom, as high as you pressed (see [Editing notes](editing-notes.md)).
- Capture keeps it too, and the song plays it wherever a clip is placed.
- Turn **Pressure** to **Off** for a track that shouldn't record it, for example drums you want to stay steady. Capture leaves it out on that track too.
- Coming: editing pressure in the note editor.

## Clip settings

The Record panel has two tabs at its top: **Record**, the record settings, and **Clip**, the settings of one clip. A scene's settings are on its scene card: hold the row's number (see [Setting up a scene](arranging.md#setting-up-a-scene)). The Clip tab names the clip it sets, such as "Clip · Verse B": the armed track's playing clip, or its first clip when none is playing, the same clip the Sequencer's Steps and the note editor show.

{{shot: clips-card | The Record panel's Clip tab: the settings of Bass's clip beside the grid.}}

- **Length:** how long the clip is, in steps of a sixteenth. Notes past a shorter end come back with **UNDO**.
- **Speed:** how fast it plays against the song, from ×1/8 to ×4. **×1/2** plays it at half speed, so it lasts twice as long; **×2** plays it twice as fast. **×2/3** and **×3/2** give triplet feels.
- **Swing:** **track** swings with its track and the song. Or pick 50 % (straight) to 75 % for this clip alone, on its sixteenths.
- **Quantise:** pulls the notes into time as the clip plays, to 1/4 notes down to 1/32 notes, triplets too. The notes keep where you played them, so **off** brings your own timing back.
- **Strength:** how far quantise pulls them, 10 to 100 %.
- **Follow:** what the track plays after this clip. **Next**, **Previous**, **First** and **Last** pick from the track's clips; **Any** picks one at random, this one included, and **Other** any but this one; **Again** plays it from its start; **Stop** stops the track.
- **After:** after how many bars, counted from where the clip started. Turning **Follow** on starts it at one pass of the clip.
- **Launch:** **project** starts this clip on the Record tab's **Launch** line. Or give it its own, from **at once** to **8 bars**: a fill on **1/4** comes in on the next beat while the rest of the set changes on the bar.
- **Mode:** **Restart** starts the clip from its top. **Legato** takes over from the clip the track is playing at the same place in its pass, so a variation slides in mid-phrase: with **Verse A** two bars long, a legato **Verse B** that starts on the bar line halfway through it plays on from its own second bar. With nothing playing, it starts from its top. **Once** makes it a one-shot: it plays one pass and then its track stops, right where the pass ends, whatever the Launch line (in song mode the song carries on). A fill of three beats stops after three beats. From its last bar its cell says where it stops ("stops 3.4"). A **Follow** set on it waits.
- **Program:** the program change the clip sends to outside gear as it starts, **1** to **128**, or **off** for none. It goes on the clip's first tick, after the clip before it ends its notes and before this clip's first note, so a synth on a MIDI Out track changes sound with the clip. Only a track that plays on MIDI Out sends it (its **Plays on** port and channel, with **Send MIDI and CV out** on in Prefs); on the pads it does nothing. It goes again each time the clip is launched, as each song placement of it starts, and when the music starts, but not on every loop.
- **Bank:** the bank select sent just before the program, shown as the two numbers sent (CC 0 / CC 32), **0/0** to **127/127**, or **off** to send none. Your synth's manual says which bank numbers it uses. Hold **SHIFT** as you tap its arrows to step the first number (a whole CC 0).
- **Direction:** the order the clip plays its steps of a sixteenth in. **Forward** plays it as written. **Reverse** plays the last step first. **Ping pong** plays forward to the end, then back to the start, and on, the end steps once each turn, so it turns wherever it reached when the clip starts its next pass. **Random** plays every step once a pass in an order of its own, a new order each pass; each pass plays the same every time you play the song, so an export sounds as you heard it. Each step's notes and locks go with it, and a note played a little early or late keeps that place around its step. The clip's knob moves still play forward. The playhead on the step pads and on Play's rows lights the step you hear, so it runs backwards, turns or jumps with the clip. A clip that isn't two whole steps or more plays forward and the row shows **—**: a tap puts it back to **Forward**.
- **Mutate:** a new variation of the clip's notes, and back. **›** mutates it; **‹** restores the original. See [Mutate and Restore](#mutate-and-restore) below.
- **Melody:** **pads** or **notes**: whether the clip's tune follows what its pads play now, or keeps its own notes. On a synth track, **›** converts a clip that follows the pads. See [Melody: Convert to notes](#melody-convert-to-notes) below.

{{shot: clips-card-direction | The Clip tab's Direction row: Clip 1 plays its steps in reverse.}}

When you record into a clip that plays in another direction, each note goes on the step you heard as you played it, so it plays back where you played it. A clip in **Random** order never plays the same way twice, so a mixer strip can't **Freeze** it (it says why); **Print** records one pass of it. A **Ping pong** clip's loop to freeze is every pass until it comes round the same again: 15 passes for a bar of 16 steps.

Tap a row's arrows, or turn the data wheel to pick a row and press **–** or **+**, as on the Record tab. Hold **SHIFT** as you tap **Length**'s arrows to **double** the clip (its notes play again in the new half) or **halve** it, and **Speed**'s to play it twice as fast or half as fast. Each change plays at once. A row you change is one take ("Settings 4 · Bass / 2"), however far you turn it, so **UNDO** puts it back in one go.

A Loop track's cell plays its loop as you recorded it, so only **Follow**, **After** and **Launch** apply to it.

A MIDI file you export from the song keeps each clip's program change at its start.

### Save state and Reload

Under the rows, **Save state** keeps the clip as it is now: its notes, locks, knob moves, length and settings. Change the clip as much as you like, then tap **Reload** (or **Reload clip** on the morph bar under the grid) to bring the saved state back. While the clip plays, the reload waits for the clip's Launch line, so the saved pattern starts on the beat, and the button is outlined in amber until then; tap it again to take the reload back. With the music stopped it happens at once. A reload is a take ("Reload 3 · saved state"), so **UNDO** brings back what you had before it.

A clip keeps one saved state, and a new Save state replaces it. Saved states last while the project is open; they are not saved with it. Deleting, moving or copying a clip, or moving a row, forgets the saved states of the slots it touches. A Loop track's cell has no Save state: the Looper's **UNDO** takes layers back.

### Mutate and Restore

When a pattern has gone stale, let the clip vary itself. On the Clip tab's **Mutate** row, tap **›** (or pick the row with the data wheel and press **+**): some of the clip's notes change, about one in four, at once, while it plays. Each changed note gets one of these:

- a softer or harder hit;
- a chance: a note that always played now plays half or three quarters of the time, and one with a chance plays every time again;
- a ratchet: a single hit becomes two or three quick ones, or a ratchet becomes a single hit;
- on a track that plays notes (a synth or outside gear, not a kit), the next pitch up or down among the notes the clip already plays, so the tune stays in its key. A kit's pads never change sound.

Hold **SHIFT** as you tap **›** for a bolder variation, about half the notes. Steps that are off stay off, and tied and slid notes keep their pitch so the tie still holds. A chord a pad played changes as one: its chance once for the whole chord, every note softer or harder by the same amount so the voicing stays, and its notes keep their pitches.

{{shot: clips-card-mutate | The Clip tab's Mutate row: Clip 1 plays a variant, its original kept.}}

The row says **original** until you mutate, then **variant**. The clip keeps its original beside it, so tap **›** again for another variation of the original (not of the variation), as many times as you like. Tap **‹** (or press **–**) to **restore** the original: its notes, locks, length and knob moves come back as they were before the first Mutate, whatever you changed since. The clip's settings (speed, direction and the rest) stay as they are now.

Each Mutate and Restore is a take ("Mutate 4 · variant 2", "Restore 5 · the original"), so **UNDO** steps back through them. The original is saved with the project, so Restore still works after you open it again. Mutate waits until a recording is done, and a clip with no notes, or a Loop track's cell, has nothing to mutate.

### Melody: Convert to notes

A clip on a synth track either **uses notes**, each note keeping its pitch whatever you do to the pads, or **follows the pads**: it remembers which pads were hit and plays whatever those pads play now, so a new scale, pad layout or octave changes its tune. Recording on a synth track makes clips that use notes. Clips made before that, and every clip on a kit, follow the pads. The Clip tab's **Melody** row says which: **pads** or **notes**.

To fix a clip's tune as it sounds now, tap **›** on its **Melody** row (or pick the row with the data wheel and press **+**). Nothing changes yet: the line under the screen says what it would do ("Convert to notes: 8 pad hits become 12 notes. Keeps what Verse plays now with these pads (C3 to D#4)"), and the buttons under the rows are **Cancel** and **Convert**. Tap **Convert**, and each hit becomes the note its pad plays now, or every note of a chord pad, each keeping the hit's timing, length, velocity, chance, condition, ratchet, tie or slide, and its pressure. A chord pad's notes stay one chord: with a chance or a condition, they play together or not at all, on the same passes as the pad's hit did. A lock on the synth's sound stays on its step. The clip plays exactly as it did, the row says **notes**, and the line says "Converted: 8 pad hits are 12 notes". It is one take ("Convert 5 · converted to notes"), so **UNDO** brings back the clip that follows the pads, and **SHIFT + UNDO** converts it again. If you mutated the clip, its original is converted too, so **Restore** brings back notes.

{{shot: clips-card-convert | Convert to notes' preview: what it would make on the line under the screen, Cancel and Convert under the rows.}}

Convert works with the music stopped: while it plays, the line asks you to press **STOP** first, and **Convert** appears when you do. If the clip or the pads change after the preview, **Convert** changes nothing and asks you to look again. Doing anything else on the Arrange page closes the preview, as **Cancel** does.

When converting couldn't keep exactly what the clip plays, nothing changes, only **Cancel** shows, and the line names everything to fix at once:

- **One pad hit twice on the same step with a different chance or condition,** such as "Pad A1 is hit twice on step 5 with a different chance or condition": as notes, a pad's notes on one step play together or not at all. Take one off, or set them alike.
- **A lock on a pad's own setting,** such as "Pad A3's Tune on step 5": it has no note to follow. The lock lane's **×** takes it off.
- **A pad that plays no note now,** because its notes fall off the top or bottom of the keyboard: take its steps off, or move the pads so it plays one.
- **Two pads that play the same note, sounding together,** such as "Pad A1 and Pad A4 both play C3 and sound together on step 5": as notes, one would cut the other short. Move one of them.
- **Two pads with the same root, with locks on the same step:** they would land on one row. Take one off.
- **More notes than a clip holds** (8,192): chord pads multiply them.

A kit's clips always follow the pads, so their row has no **›**. There is no way yet to turn a clip of notes back into one that follows the pads. **Reload** brings a saved state back as you saved it, so a state saved before converting follows the pads again: save it again after converting.

Coming: the clip's name and colour, and its step size.

## Takes

Every recording is a **take**, listed under Takes. **UNDO** takes back the newest take, and **SHIFT + UNDO** puts it back, so you can try a part again and again without worrying about losing the last one.

## Capture: keep what you just played

You weren't recording, but you just played something great. Tap the **Capture** button under Takes (it says how many bars it would keep), or press **SHIFT + REC**, and MPFree keeps it. (On the Looper tab, SHIFT + REC keeps the sound instead, as a loop: see [Recording loops](looping.md#capture-keep-what-you-just-played).)

MPFree is always listening to the pads, and to keyboards a route plays on other tracks (see [Playing several tracks from your keyboards](midi-and-cv.md#playing-several-tracks-from-your-keyboards)). Capture keeps your **last phrase**: everything since the last bar of silence, up to 32 bars, cut to whole bars and with your exact timing.

- If you played it in time over a clip that was playing, the notes go into that clip.
- It keeps what recording would: a new clip on a track that plays a synth gets the notes you played (a chord pad's every note), and on a kit the pads you hit. Over a clip that was playing, it adds in that clip's way, as recording into it would: the pads you hit into an older clip of pads on a synth track, notes into a clip of notes.
- Otherwise it becomes a new clip, named "Capture 1", "Capture 2" and so on, and starts playing in time.
- It keeps how hard you pressed the pads as you held them, as recording does (see [Pad pressure](#pad-pressure)), unless the track's **Pressure** setting is **Off**.
- The pads' notes go into the armed track. What a keyboard played on another track through a route goes into that track, so a jam of drums on the pads and bass on a keyboard comes back as two clips that play together. On a track that plays a synth, each key goes in as the notes of the pad that plays it there; on a kit, as that pad. A key whose note no single pad plays on that track isn't kept yet. The FX moves you made over the phrase (the XY pad, the DJ knob) go with the armed track's notes; when only a keyboard played, on other tracks, they're left out and the line under the screen says so.

UNDO takes a capture back like any other take, every track's part of it at once.

> [!MPC]
> Capture is like the MPC's Retro Rec, but it keeps whole bars, works whether the music was playing or stopped, and can add to the clip that's playing instead of always making a new one.

## Bringing in a MIDI file

A MIDI file (`.mid`) from another sequencer, a computer or your MPC loads as clips, so a beat you made somewhere else plays on your pads.

1. Put the file on the stick, in **Patterns** or any folder of the browser (see [Getting files on and off the stick](saving.md#getting-files-on-and-off-the-stick)).
2. Arm the track it should go on: tap its name on Arrange, or set the Record panel's **Armed** row.
3. Press **BROWSE**, find the file, tap it to pick it and tap it again to load it (or push the data wheel).

Each track of the file becomes a clip in the first empty slot of a track: the first on the armed track, the next on the track after it, and so on. A file with all its parts on one track, as many are, gets a clip for each channel. The clips are as long as the file, in whole bars, and wait in their slots for you to play them. The line under the screen says how many clips came in, where, and what was left out. **UNDO** takes the whole file away again.

**Its notes go in as a recording would keep them.** On a track that plays a synth, each note goes in as it is; on a kit, each note goes on the pad that plays it:

- **On a kit**, each note on the pad that sends it (see [A kit on a drum machine](midi-and-cv.md#a-kit-on-a-drum-machine)): C#2 is A1, C2 A2 and so on through bank A, as the MPC sends and receives them, and the other notes on banks B to H. So a drum track exported from an MPC comes back on bank A's pads, a drum part from a computer that uses those notes does too, and MPFree's own export of a kit comes back on the pads that played it, in every bank.
- **On a synth** or a track's own instrument, every note as it is, so a melody comes in whole, whatever the pad layout. The Sequencer shows the notes on the rows of the pads that play them; a note no pad plays still plays, but has no row until a pad plays it.
- **On the Drums synth**, which plays pads, a note goes on the pad that plays it. A note no pad plays is left out, and counted.

**Left out** for now: controllers, pitch bend, pressure and program changes (they come with lanes), and SysEx. The file's tempo is said but not set, so set the project's tempo to match if you want it. A file in clock time (made for video) is read at 120 BPM.

> [!MPC]
> To bring a sequence over from your MPC, export it there with *Track as MIDI/Pattern* (one track, with **Export MIDI As** set to MIDI files) or *All Tracks*, copy what it saves to MPFree's stick, arm the track the first part should go on, and load the file. Drum tracks land on the pads they were played on. Load the kit too ([Sampling](sampling.md)), and the beat plays as it did.

## Steps

The pattern on the **Sequencer** tab's **Steps** view is a clip shown as a grid of steps, 16 to a page. Each step can have its own **velocity**, **length**, **nudge** (early or late) and **chance** (how often it plays). The **+** and **–** at the top right of the pattern double or halve its steps (16, 32, 64 or 128), and **‹** and **›** turn its pages. Under a sound, a **lock lane** sets one of its settings (or one of the track's, such as a send) step by step: **+ Lane** adds one. See [Your first beat](sequencer.md#the-pattern). **STEP SEQ** puts one sound's steps on the pads, a page of 16 at a time, where you hold a step to change it or lock its sound on the Q-Links, hold one and hit a later one for a longer note, and copy steps with **COPY**: see [Program a pattern on the pads](sequencer.md#the-pads-as-steps-step-seq).

## Editing notes

To fix what you played (pull notes into time, move them, change how hard they hit, draw new ones), tap **Notes** on the Sequencer tab, or **Edit ›** above the clip grid, which takes you there. The note editor has a chapter of its own: [Editing notes](editing-notes.md).

> [!MPC]
> On the MPC, a sequence holds every track and you switch whole sequences. In MPFree each track has its own clips, launched on their own like Ableton Live's clips, and clips of different lengths play side by side.
