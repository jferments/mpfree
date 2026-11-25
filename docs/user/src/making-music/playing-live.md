<!--
status: changing
covers: rust/mpfree-sampler-ui/src/levels_panel.rs rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs rust/mpfree-app/src/mixing.rs rust/mpfree-fx/src/perform.rs
checked: 2026-11-22 068a8998
-->

# Playing live

The **Play** page is MPFree's home: the place for finger drumming, making beats and live looping. Everything for a jam is on one page, so you never have to leave it while you play: press **MAIN** from anywhere to come back.

{{shot: play-all-loop | The Play page in a jam: six loops along the top (three playing, Loops 4 and 5 turned off, Loop 6 recording its first pass) and two empty, the armed track's pads as rows of steps in the middle (a hat switched off shows "off"), Stutter held among the throws, the XY pad playing filter and echo, and swing, tune, filter and delay on the Q-Links.}}

From top to bottom:

- **The transport bar**: stopped or playing, the position, the tempo, what **REC** records, and **PADS PLAY**, what the pads play (see [The transport bar](../getting-started/the-screen.md#the-transport-bar)).
- **The loop row**: all eight loops of the looper, a card each, with **Arm**, **Capture** and **Pads: loop keys** (see [Your first loop](../getting-started/first-loop.md#the-loops-on-the-play-page)).
- **The rows**: the armed track's pads as rows of steps, a simple step sequencer (below).
- **The throws** and **the XY pad**: effects you play by holding them (below).

There are no pads on the Play page: you play the real ones, and the room goes to the pattern, the loops and the effects. The pads' own settings and sounds are on the [Pads tab](pads.md), and the whole pattern, with your timing and each step's settings, is on the [Sequencer](sequencer.md) tab.

## The pads as rows of steps

The rows are a simple step sequencer, as on a Digitakt: one row for each pad of the track the pads play, in the bank on the pads (**A1** to **A16** on bank A). Each row has the sound's name, the pad's name on the right ("A1") and a speaker; an empty pad keeps its row, dashed, so a pad is always in the same place. A pad that loops, plays a synth voice or sends MIDI out says so under its name (**LOOP**, **SYNTH**, **MIDI**; see [What a pad plays](pads.md#what-a-pad-plays-sample-loop-instrument-midi-out)). A tap on an empty step puts a note there. A tap on a lit step takes it out; **UNDO** brings it back as it was, with its velocity, length, condition and locks (set them on the [Sequencer tab](sequencer.md#a-steps-settings)). A step switched off on the Sequencer tab shows as an outline with **off** in it, and a tap brings it back on. Six rows fit at a time: swipe up or down on the rows to see the others (the rail on the right shows where you are).

The heading over the rows:

- **‹ Track 1 ›**: the track the pads play. Tap its right half for the next track and its left half for the one before; that track gets armed and the pads play its kit. Audio tracks are skipped, since they have no pads.
- **A** to **H**: the kit's banks. Tap one, or press **PAD BANK**, and the pads and the rows go to that bank.
- **‹ 1–16 of 32 ›**: the steps shown, for a pattern longer than a bar. **Follow** turns the page as the pattern plays; turning a page yourself turns it off.

The pattern's length, the step inspector and your timing are on the [Sequencer](sequencer.md) tab, next to Play in the top bar.

## Throws

**Throws** are effects you play like an instrument: hold one and the effect comes in, let go and it stops. There are eight: **Delay throw** (a dotted echo), **Reverb wash**, **Stutter**, **Tape stop**, **Filter dip** (a low-pass sweep), **Beat repeat**, **Drop drums** and **Vinyl brake**. The Play page has room for the first six; the Sequencer's Steps view shows all eight while no step is picked, and page B of the [Perform set](looping.md#more-pages-the-perform-set) has the eight on pads 1 to 8. Each tile shows its timing or length. While the music plays, a throw starts on the next sixteenth note, so it lands in time even if you press a little early. Hold one at the end of a phrase and let go on the next downbeat. The data wheel's push holds the outlined throw on until you push again (see [The wheel's outline](../getting-started/the-screen.md#the-wheels-outline)).

**Drop drums** mutes the kick, snare and hi-hats while you hold it: the classic breakdown move.

**Several at once.** Hold throws with different fingers and they play together: Delay throw and Reverb wash stack, and Filter dip and Drop drums go on top of anything. Stutter, Tape stop, Beat repeat and Vinyl brake each take over the sound, so of those only the one you pressed last plays; let it go and the one you held before comes back.

## The XY pad

The **XY pad** plays an effect on the whole mix under one finger. The chip at its top left says what it plays, for example **XY · Filter + echo**.

**Filter + echo** (what it plays at first) is a filter and an echo together. Left of the middle is a low-pass filter (darker), right of it a high-pass filter (thinner), and the middle is no filter at all. Higher up adds more echo. When you lift your finger the pad glides back to the middle and the bottom, so the sound comes back clean. One finger plays it: a second finger on the pad does nothing until the first lifts. Its label says where it is, for example "HP 173 Hz · echo 62 %".

**To play something else,** tap the chip. A list takes the place of the pattern: Filter + echo, then the nine effects of [XY FX](effects.md#xy-fx) (Filter, Echo, Dub echo, Wash, Crush, Flanger, Phaser, Ring mod and Gate), each saying what across and up do. Tap one and the pad plays it from then on; **Close** or the chip again leaves it as it was. With one of the nine, the effect opens when your finger lands and lets go when it lifts (an echo or the wash rings out), and the dot stays where you lifted it. The pad's label names both settings, for example "Decay 2.4 s · Amount 40 %". MPFree remembers your pick, like the other settings in Prefs.

{{shot: play-xy-list | The list the XY chip opens: Filter + echo and the nine XY FX, each saying what across and up do. Wash is the pick here.}}

**For a bigger pad, press SHIFT + NEXT SEQ** from any page. The XY pad takes the whole of Play under the transport bar, with the master bar and the Q-Links still beside it, so you have room for big, slow moves. It plays just as the small one does, and its chip still opens the list. Under it are **Hold**, **Let go** and **Setup**. Press **SHIFT + NEXT SEQ** again, or go to another page, to get the small pad back.

{{shot: play-xy-full | The XY pad full screen (SHIFT + NEXT SEQ), playing Wash: decay across, brightness up. Under it, Hold (for layouts of yours), Let go and Setup.}}

**To put your own settings on the pad,** make a layout of your own: tap **+ New layout** in the chip's list, or **Setup** under the full-screen pad to start from the ready-made layout playing. See [Your own XY layouts](xy-layouts.md).

## Fills

While the music plays, hold **PLAY** for a fill: notes set to play only in fills come in, and notes set to drop out in fills go quiet, until you let go. Press **PLAY** twice to keep the fill on; press it again (or STOP) to end it. When the music is stopped, PLAY just starts it. The exact buttons are in [Buttons and controls](../reference/controls.md).

## Muting

There is no separate mute page. Each pattern row's name is its **mute button**, with a speaker icon in a box: tap anywhere on the name to mute that pad (the box turns red and the speaker shows a ×), and again to bring it back. On Play's loop row, each loop's speaker mutes the loop, and each layer of the picked loop is a chip at the top of the row that mutes that layer.

Hold **ERASE** and tap a row's name to delete that sound's steps from the pattern instead. **UNDO** brings them back.

### Muting tracks and pads

**TRACK MUTE** turns the pads into mute buttons over whatever page you are on, so you can drop parts in and out without leaving the screen you are using. Pads 1-16 are tracks 1-16: green while the track plays a clip, dim green when it has nothing playing, red while it is muted, and dark where there is no track. Tap a pad to mute or unmute its track; the track's clip keeps its place, so it comes back in time. With more than 16 tracks, the **PAD BANK** buttons pick which 16 the pads are, as on Akai's MPC: A is tracks 1-16, B is 17-32, and so on up to H, 113-128 (SHIFT or a second press gives E-H, as when you pick kits). The kit playing stays the same, and TRACK MUTE remembers the bank for next time. **SHIFT + TRACK MUTE** mutes pads instead, on the selected track: tap a pad and that track's clips leave it out (the kick, say), while another track playing the same kit keeps it. It works on any kit and any bank (**PAD BANK** picks which 16), and a pad you hit yourself still sounds. A pad lit in the kit's colour plays, a red one is muted. A track that plays notes (a synth or another instrument) has no pads to mute, and the hint says to mute the track instead. Press the same button again to play the pads as usual. The hint line at the bottom of the screen says which mode the pads are in. **TRACK MUTE**, **STEP SEQ** and **16 LEVEL** each take the pads over, one at a time: pressing one gives the pads up from the others.

**When a mute lands.** In Make a tap mutes or unmutes at once. In Perform it waits for the next launch line (the Record panel's **Launch**, one bar unless you change it), so parts drop out on the beat: the pad blinks amber while it waits, the track's name on the Clips grid is striped amber and says how soon ("mutes in 2 beats"), and the transport bar starts with **WAITING** and the tracks and bar it lands on ("WAITING Hats, Vox at 18.1"). Tap more pads and they all land together on the same line; tap a waiting pad again to take its change back. Hold **SHIFT** while you tap for the other timing: at once in Perform, on the line in Make. With the transport stopped, or Launch at once, a tap acts at once, and **STOP** lands whatever is waiting.

{{shot: clips-mutes | In Perform: Track 2's mute and Track 3's solo wait for bar 4, their names striped amber, and the transport bar says WAITING.}}

**Solo.** Hold **TRACK MUTE** and tap a pad: that track plays alone and the others go quiet. The soloed pad is lit blue, and the tracks it silences are red, like muted ones. Tap more pads while you hold it to solo more tracks, and tap a soloed pad to let go of its solo. A quick press of TRACK MUTE still turns the mute keys on and off; after a hold with a pad tapped, the pads are left as they were before you pressed it. A solo lands at once or on the line, just like a mute, and the project saves it. A soloed track's name on the Clips grid is outlined.

Mutes and solos aren't undo steps, and with **Rec song** on they go into the song: a track muted for a stretch leaves a gap there. Pad mutes (**SHIFT + TRACK MUTE**) work the same way: in Perform a pad's mute waits for the line with the tracks' (WAITING names the track and pad, "Drums B5 at 9.1"), and holding **TRACK MUTE** over the pads solos a pad on the selected track: its other pads go quiet there. The soloed pad lights blue and the pads it silences go red; letting go of TRACK MUTE leaves the pads as pad mutes. The project saves each track's pad mutes and solos. A project saved before pad mutes were a track's had kit 1's pads A1 to A16 muted for every track: it opens with each track on kit 1 muting them, and the log says so.

## 16 Levels

**16 LEVEL** spreads one sound over all 16 pads: the pad you hit last plays on every pad, each one a step of something else. Press **16 LEVEL** and the 16 Levels panel opens on the Play page, under the transport bar:

- The cells are the pads, pad 1 at the bottom left. Each one shows what that pad plays, and they are lit in the same colour as the pads, with the pad you are holding bright.
- The column on the right picks what the steps change: **Velocity** (8 to 127), **Tune** (a semitone a pad, pad 9 as recorded; in another [tuning](synths.md#tuning), a note of the tuning a pad), **Cutoff** (100 Hz to 20 kHz), **Decay** (25 ms to 4.5 s), **Start** (where in the sound it starts) or **Transpose** (the armed track's notes, below). Tap one, or turn the wheel.
- **Pick pad**, then hit a pad: the levels play that pad's sound instead.
- **Close** hides the panel and keeps the pads on 16 Levels. **Off**, or **16 LEVEL** again, gives the pads their own sounds back. The 16 LEVEL button is lit while 16 Levels is on, and a **16 Levels** chip in the top bar says what the steps change.

16 Levels belongs to the kit, so it is saved with it and one UNDO on the Sample tab takes it back.

{{shot: play-16-levels | The 16 Levels panel by Tune: pad 9 plays the sound as recorded, the others a semitone apart, and pad 12 is held.}}

### Transpose

The last kind in the column, **Transpose**, moves a playing line instead of a sound. The pads shift the armed track, and every track you've set to follow it, while its clips play: pad 9 plays as written, pad 14 is up 5 semitones (a fourth), pad 16 up 7 (a fifth), pad 1 down 8. It latches, so the shift stays until you hit another pad, and it lands on the track's next note: notes already sounding finish at their old pitch, so you can tap transposes in time with the music. Nothing in the clip changes, and pad 9 always brings it back.

- When the armed track plays the synth (Synth, the last row of the list of kits), **16 LEVEL** opens Transpose at once. On a kit it's the last kind in the column.
- When the synth's pads play **In key** (or chords), Transpose steps through the key instead of semitones, so the line stays in key: pad 16 is an octave up in a seven-note scale and pad 2 an octave down. The heading says the key ("Bass · -3 in C Minor").
- The heading names the armed track and the tracks that follow it, and where they are ("Bass, with Keys · +5 st"). The pad it's on is lit, on the screen and on the MPC's pads. Pick pad does nothing here.
- Only tracks that play notes move: a track that plays the synth, or a track's own instrument. A drum kit's notes are its pads, so a kit track, the Drums engine and audio tracks stay as they are, and the heading says so.
- The second Q-Link (Tune's place) turns Transpose too, two octaves either way, and so does the Record panel's **Transpose** row (see [Recording a clip](clips.md#recording-a-clip)). The Record panel's **Follows** row sets which tracks move with the armed one, so a bass and its chords can change key together.
- What you record while transposed goes in as written and plays back transposed, so it sounds as you heard it.

## When it gets busy

However much is playing, a pad, a key or a recording on the Sample page that you let go always stops on time, so no note is left hanging. If one moment holds more than MPFree can line up (hundreds of hits and pressure changes at once), it first merges pressure and knob moves into the newest one, then skips new hits, and never skips a let-go. With **Show developer tools** on in Prefs, the Dev tab's Debug page counts how often that happened.

## Your timing

The **Sequencer** tab's Steps view has a **Your timing** strip under the pattern that reads how early or late, how even and how hard you play the pads. Play along with a pattern there to practise: see [Your timing](sequencer.md#your-timing).

## Make and Perform

MPFree has two ways of answering your hands, for the same project: **Make**, for building, where everything acts at once and **UNDO** is the safety net, and **Perform**, for a stage. The chip at the left of the top bar's chips says which one you're in: **MAKE**, or **PERFORM** in the second accent colour. Tap it, or press **SHIFT + MAIN**, to switch. The hint line says what changed.

Switching never stops or changes anything playing. The project keeps it, so a set opens as you left it; a new project starts in Make.

What Perform changes today:

- **MAIN** goes to the Arrange tab's Clips grid, not the Play page. Pressed again, it shows the Song view, then the song list, then the grid again. While a [set](set-lists.md#playing-a-set) is open it goes to the Show view instead, and on Arrange goes Show, Clips, Song, List.
- **The arm is pinned**: opening a page doesn't move what REC records; only a tap on an Arm pill does. Picking a track selects it to edit and play without moving REC. The REC chip says PINNED. Back in Make, the arm follows the page again.
- **TRACK MUTE's pads wait for the launch line** before a mute or solo lands, and SHIFT + a pad acts at once (see [Muting tracks and pads](#muting-tracks-and-pads)).
- **REC brings Rec song with it**: each take also records what the grid plays into the song, for as long as it lasts, and the REC chip says "+ Rec song". Tap the chip and turn off **Rec song with REC** to leave one take out (see [Recording a performance into the song](arranging.md#recording-a-performance-into-the-song)).
- **Buttons that delete need a hold**: the Song view's **Delete** and **Erase**, the note editor's **Delete**, the Looper's **Clear**, a layer's **×** and **½ length**, the step's **Clear** and the song list's **Remove** take half a second in Perform. A red fill crosses the button while you hold it, and it acts once the fill is across; let go early, or slide off, and nothing happens, and the hint line says "Hold to delete". In Make one tap does it, and UNDO brings it back. **ERASE** + a tap works as before, since it already takes two hands.
- **Clips and rows don't drag**: a clip or a row's number launches when you lift your finger on it, and a finger that slides off first does nothing, so a slip can't move a clip. Several fingers each launch their own clip, in Make too.
- **The Clips grid shows 5 big rows**, and a clip waiting to start counts down: an amber bar along its foot fills as its line comes near. A clip about to stop, or to give its track back to the song, counts down the same way.
- **Capture scene** sits on the grid's title line, left of Back to song (or of **+ Track**). Found a combination you like while jamming? Tap it, and the clips playing now go as copies into the first empty row under the scene playing, as a new scene you can launch later. It is the scene card's **Capture** (see [Setting up a scene](arranging.md#setting-up-a-scene)) without opening the card, and **UNDO** takes it back. It is faint while nothing plays. With more than 8 tracks, Back to song or a set's **Show** button can leave it no room: then hold a row's number and use the card's Capture. **SHIFT** + a tap on it learns it for a controller's button (see [Launching clips and scenes from your controller](midi-and-cv.md#launching-clips-and-scenes-from-your-controller)).
- **An empty slot needs a hold** to make a clip: half a second, with the red fill crossing it. A tap makes nothing, and the hint line says to hold, so a stray finger can't stop what a track plays. In Make one tap still makes a clip. An empty slot of the armed audio track still arms for a loop with one tap.
- **NEXT** on the transport bar says what comes next and how soon, while the music plays: a scene you launched ("NEXT Drop in 2 beats"), the clips waiting ("Drop bass in 1 bar", "Bass stops in 3 beats"), the song list's next step ("Verse in 2 bars", "Stop all in 1 bar"), the next scene a follow action plays ("Verse in 3 bars", or "Drop or stay" when it is down to chance), or in song mode the next section ("Chorus in 4 bars"). When mutes are waiting too, WAITING comes first.

{{shot: clips-perform | In Perform: five big rows, the chords waiting for bar 6 with their countdown bar, the glitch stopping there, NEXT on the transport bar, and Capture scene and Back to song on the grid's title line.}}

{{shot: clips-perform-capture | In Perform with a set open and nothing playing: Show, then Capture scene, faint until a clip plays.}}

Clips and scenes start on the line the Record panel's **Launch** sets, in Make and Perform alike (see [Recording a clip](clips.md#recording-a-clip)), and the transport bar shows it.

More things help on a stage, in Make and Perform alike:

- **Back to song.** With **Song on**, a clip you launch takes its track over from the song. While any clip plays over the song, **Back to song** shows on the grid's title line: tap it and every clip stops on the next Launch line, and the song plays every track from there, on the beat (see [The song](arranging.md#the-song)).
- **Reload a clip.** Before you take a pattern apart live, tap **Save state** on the Clip tab (see [Clip settings](clips.md#clip-settings)). Add steps, change its speed, erase half of it; then tap **Reload clip** on the morph bar, and the pattern comes back as you saved it on the next Launch line, so it lands on the beat. In Perform, NEXT says when ("Verse B reloads in 2 beats"). Tap Reload clip again before then to take it back, and **UNDO** puts the wrecked version back if you liked it after all.
- **Live transpose.** Move a playing bass line or arp to another key from the pads, with 16 Levels' **Transpose** (see [Transpose](#transpose)): a progression from one loop, a lift for the last chorus, an octave jump for a build.
- **The song list.** Line your scenes up in the order they play, each so many times ("Intro × 2, Verse × 4, Drop × 4"), and let the list play them on the bar while your hands are free. In a new project the list is the song's parts, so **Play from here** plays the song from that part, each part bringing its scene's mutes (see [The song list](arranging.md#the-song-list)).

{{shot: arrange-list | The song list in Perform: seven steps, the Drop playing its third pass of four, the Break picked.}}

## The Q-Links on the Play page

| | Q-Link 1 | Q-Link 2 | Q-Link 3 | Q-Link 4 |
|---|---|---|---|---|
| The Play page | Swing | Tune (the pad played last) | The kit's filter | The kit's delay send |
| With 16 Levels on Transpose | Swing | Transpose (the armed track) | The kit's filter | The kit's delay send |

The Sequencer's Steps view has the same four while no step is picked, and the picked step's settings while one is (see [Throws and the Q-Links](sequencer.md#throws-and-the-q-links)). Press **Q-LINK** on Play to step through your [macros](effects.md#macros) instead.

Swing reads **off** at 50 %, which is straight time, as it does in the transport bar. Turn it up for a shuffle.

While you hold **NOTE REPEAT**, or the Arp panel is open, the knobs control the repeat instead: rate, gate, swing and octaves. See [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md).

> [!MPC]
> On the MPC, XYFX turns the whole screen into the XY pad, Track Mute is a mode with a page of its own, and Pad Perform opens in a window. In MPFree, the XY pad and the throws sit on the Play page next to your pattern and loops, and TRACK MUTE only changes what the pads do, so you can perform without leaving the page you are on. When you want the XY pad big, as on the MPC, SHIFT + NEXT SEQ puts it full screen.
