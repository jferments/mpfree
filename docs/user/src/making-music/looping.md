<!--
status: changing
covers: rust/mpfree-looper rust/mpfree-app/src/looper_page.rs rust/mpfree-app/src/loop_row.rs rust/mpfree-app/src/looper_deck.rs rust/mpfree-app/src/looper_tab.rs rust/mpfree-app/src/looping.rs rust/mpfree-seq/src/loops.rs
checked: 2026-11-23 4cc9c0ec
-->

# Recording loops

[Your first loop](../getting-started/first-loop.md) showed the basics: record a pass, overdub layers on top, undo one. This chapter covers how the looper works in more depth, the **Looper** tab, recording from an instrument, and getting the timing right.

You play the loops on the Play page, where the loop row under the transport bar shows all eight next to your pattern, or on the **Looper** tab, where you also shape them.

## How the looper keeps time

The looper is part of the song, not a separate machine running beside it:

- **It starts and ends on bar lines.** Press Record and the loop waits for the next bar line, then records. You always get at least a beat to get ready: press it in the last beat before a bar line and it waits for the bar after, and the loop's ring counts down the beats in the last bar ("3 beats to REC"). It records until you press Record again, when it ends on the next bar line. Press **STOP** instead and it stops at once and keeps what you played: the loop runs on to the next bar line, silent after the stop (if you press STOP just after a bar line, within a sixteenth note, it ends on that bar line instead). With a set length (**First pass**, below) it ends by itself after that many bars. So the loop is always a whole number of bars, and always in time with your patterns.
- **It follows the song's position.** Stop and start the transport and the loop picks up exactly where the song is. It is silent while the transport is stopped, and its playhead stops too, at the place the song will start from again (press STOP twice and it goes back to the start of the loop, with the song).
- **The tempo is the song's tempo.** Change the tempo and a playing loop follows it and keeps its pitch. MPFree re-makes the loop at the new tempo in the background, which takes a fraction of a second; until it is ready, the loop plays faster or slower, like tape. Overdub over a loop at its new tempo and MPFree first fits the loop to that tempo, so the new layer lines up. The fit is its own step: UNDO takes back the overdub, then the fit.
- **There are no clicks.** A new loop keeps a moment of what you played just before it started and just after it closed (about half a second in all). At the loop point its end fades into what you played just before its start, so it joins as you played it, and the **seam fade** Q-Link ([below](#the-q-links)) sets how long that fade is, from a hard cut to the longest fade; nothing is written into your recording, so you can change it any time. Every start, stop, punch in and out, mute and undo is faded in a few milliseconds.
- **Or it keeps its own time.** Set an empty loop's **Sync** to **Free** and it plays on when the song stops, records with the song stopped, and keeps its place and tempo whatever the song does: see [Free loops](#free-loops).

## Layers

The first pass is the loop's first **layer**. Each overdub adds another, up to 16. **Undo** fades out the newest layer and **Redo** brings it back, until you record a new overdub. The loop shows the layers in different colours, and on the Play page the picked loop's layers are chips at the top of the loop row: tap one to mute that layer on its own. An overdub in which you played nothing (only silence came in) leaves no layer, so it takes no memory and no step of the history.

To get rid of a whole loop, first pass included, use **Clear** on the Looper tab, or hold the loop's ring for a second (on the Looper tab or Play's loop row). The **×** on a loop's last remaining layer does the same. **UNDO brings a cleared loop back** as it was, layers, history and all, until you record into that loop again or the memory is needed for another loop's layers (the oldest cleared loop goes first). In Perform, **Clear** and **×** take a half-second hold, so a stray touch can't clear a loop mid-set (see [Make and Perform](playing-live.md#make-and-perform)); the ring's hold is a second long already.

The looper holds five minutes of stereo sound across all its loops and layers (the Looper tab shows how much is left). At 120 BPM a bar lasts two seconds, so that is room for about 37 four-bar layers, spread over up to eight loops of up to 16 layers each (a little less, as each loop's first pass also keeps the moment around it). If you need more or less, **Loop memory** in [Prefs](../reference/prefs.md) (also in the Looper tab's MENU) sets it from 1 to 20 minutes. The memory is set aside when MPFree starts, so a new size takes effect from the next start; the row says "(next start)" until then, and shows how many megabytes it uses (with the 60 seconds the looper keeps for [Capture](#capture-keep-what-you-just-played): 131 MB for five minutes).

## The buttons

The MPC's own **REC**, **OVERDUB** and **UNDO** work the picked loop while the loop is armed: the **Arm** pill at the left of Play's loop row arms it (red when REC records it, grey when it doesn't), so does opening the Looper tab, and the REC chip on the transport bar then says "→ Loop 1" (see [What REC records](sequencer.md#what-rec-records)). A tap on a loop's ring does the loop's next step without arming anything (see [The Looper tab](#the-looper-tab)). **SHIFT** + Arm adds the loop to the pattern, so one REC and PLAY record the pads into a clip and a loop's first pass together, from the same bar line. **UNDO** while the first pass or an overdub records throws that take away and the loop plays on. On the Looper tab, **SHIFT + REC** captures the last bars you played into a loop (see [Capture](#capture-keep-what-you-just-played)). **UNDO** (and **SHIFT + UNDO** for Redo) works on the loop on the Looper tab, and on the Play page when the loop is what you changed last. To record a pattern from the pads with REC, open the Sequencer tab (it arms the notes) or pick a track on the Arrange tab:

| Button | Empty loop | Recording the first pass | Playing |
|---|---|---|---|
| **REC** | Waits for the next bar line, at least a beat away (or, stopped, for PLAY), then records. Press again before it starts to cancel. | Ends the loop on the next bar line. | Starts an overdub. |
| **OVERDUB** | Nothing. | Goes straight on into an overdub when the first pass ends. | Starts an overdub; press again to stop adding. |
| **UNDO** | Brings back the loop you cleared, if there is one. | Throws the recording away. | Removes the newest layer (or the overdub in progress). |
| **SHIFT + UNDO** | Nothing. | Nothing. | Brings back the last layer undone. |
| **Clear** (Looper tab), or a ring held a second | | | Deletes the whole loop, so you can start again. UNDO brings it back. |
| **The loop's speaker** | | | Silences the loop without losing it. Tap again to hear it. |

**STOP** while the first pass records stops it at once and keeps what you played, up to the next bar line, so a loop is never lost. Stopping the transport while you overdub ends the overdub. If the song jumps while you overdub (you move the playhead, or the song's loop goes round where the loop doesn't fit it), the overdub carries on from the new place: over bars it has already recorded, it adds to them; past them, before it has gone round the loop once, it carries on as a new layer, so UNDO takes back what you played after the jump first. With the transport stopped, Record or Overdub on a loop that has something in it waits for **PLAY**, with OVERDUB and PLAY blinking; **STOP** then lets go of it (and of a loop armed with Record), so nothing is left blinking.

## The Looper tab

The **Looper** tab is the looper as an instrument: every loop in sight, one touch to record, overdub or mute any of them, and the loop you picked laid out underneath with a lane per layer. Press **SHIFT + SAMPLER** to open it.

{{shot: looper | The Looper tab mid-jam: the eight loops across the top as rings (three playing, two turned off, Loop 6 recording, two empty), the picked loop under them with a lane per layer and the controls a loop that plays needs, and the loops' levels on the Q-Links.}}

### The loops

The eight loops sit side by side across the top, each the same: its name and length, a **ring**, a **level bar** beside the ring, and a **speaker** and an **S** under it. The ring is the loop:

- **Its wave runs round the ring**, and an arc fills as the loop plays, with a tick for each bar and a hand at the place it has got to. The middle says which bar is playing ("2 of 4").
- **Its colour is what it is doing:** the loop's own colour while it plays, red while it records its first pass, amber while it overdubs, grey when muted.
- **An empty loop is a dashed circle** with **+** in the middle and what it will record under it (**Pads**, **Line in** or **Loops**; **Pads · free** when it will record a [free loop](#free-loops)). Once you tap it, it waits for the next bar line, at least a beat away: the circle turns red and the middle says the bar it starts on ("13, REC on bar"), then in the last bar counts down the beats ("3 beats to REC"), or says **REC on PLAY** while the music is stopped. Set to start [when you play](#start-when-you-play), it says **Play to start** until your first note.

Tap a ring and the loop does its next step, on the moment your finger lands, so a punch is where you played it:

| The loop is | A tap on its ring |
|---|---|
| Empty | Records from the next bar line at least a beat away (or from PLAY), or from your first note when it [starts when you play](#start-when-you-play). |
| Waiting for its bar | Cancels. |
| Recording its first pass | Ends it on the next bar line, and it plays. |
| Playing | Starts an overdub. |
| Overdubbing | Ends the overdub; it plays on. |
| Muted | Lets it play again. |
| A [free loop](#free-loops), at rest while the free loops are stopped (▶ in the middle) | Plays the free loops: at once with the song stopped, else on the song's next bar line ("on the bar" until then; tap again to call it off). |

{{shot: looper-countdown | Loop 7 armed with the music playing, in the last bar before it starts: its ring counts the beats left.}}

The ring you tap also picks that loop. To pick a loop without starting anything, tap its **name**.

**Hold a ring** for a second to clear the loop, every layer and the first pass: the arc drains in red while you hold, and letting go sooner leaves the loop as it was. Hold **ERASE** and tap a ring (or a loop's name) to clear it at once. Either way **UNDO** brings it back: the empty ring says **Cleared · UNDO** until you record into it.

The **speaker** under a ring mutes the loop without picking it: it is kept and keeps its place in the song, but stays silent, through STOP and PLAY too, until you tap it again. The **S** beside it **solos** the loop: while any loop is soloed only the soloed loops play, and the others stay silent in their places (their rings go dim and say **by solo**). Tap S again to let go; clearing a soloed loop lets go of its solo too. The **level bar** beside the ring sets how loud the loop plays (-24 to +6 dB, with a meter of what it plays now): drag it, or rest a finger on it and turn the data wheel (half a decibel a click, SHIFT for a tenth); SHIFT + the wheel's push puts it back to 0 dB.

Every loop plays at once, in time with the song, but only one records at a time: starting to record in one loop ends the recording in the others. The pads can do all of this too: see [The pads as loop keys](#the-pads-as-loop-keys).

Over the loops: whether the song is stopped or playing and its tempo (and, while a [free loop](#free-loops) has something in it, the free loops as a group of their own: **Free**, ▶ while they play or ■ while they are stopped, and their tempo, so "Song stopped 120 BPM · Free ▶ 93.4 BPM" says the song is stopped and the free loops play on; with the song's bar numbers there it leaves out the free loops' "BPM"), **Pads as loop keys** (see [The pads as loop keys](#the-pads-as-loop-keys)), **Records from** (what every loop records, with a meter of how loud it is coming in; tap it and its choices open in the lanes' place: **Pads**, **Line in**, **Loops**, or a mix; see [Recording a new loop](#recording-a-new-loop)) and the loop memory every loop shares ("37 s of 5 min").

### The picked loop

Under the loops, the picked loop has the whole width. Its header has its colour and name, where it lives on the Clips grid with its length and layers ("Loops A · Verse · 4 bars · 6 layers"), and:

- **Listen**: with the music stopped, plays the loop on its own, round and round from its start, so you can hear it without starting the song. It lights up and says **Stop** while it plays; tap it again, or press PLAY, to stop. While the music plays it is faint, since you hear the loop with everything else then. A [free loop](#free-loops) plays with the song stopped anyway, so it has **Stop free** and **Play free** for the free loops here instead.
- **Send**: what to do with the loop (below).
- **Clear** (outlined in red): deletes the whole loop, like holding its ring. UNDO brings it back.

Under the header, a ruler of the loop's bars, then a **lane for each layer**, a fingertip tall, the newest on top. Each lane has the layer's name ("Drums", "Bass", named after what you played into it) and a tag ("dub 4 · -2.0 dB", "dub 3 · muted", "first · at 49 %"), its own wave, and:

- **a tap on its name** to mute that layer (tap again to hear it),
- **▶** to listen to that layer on its own while the music is stopped, even a muted one (it lights up while it plays; tap it again, or press PLAY, to stop),
- **S** to solo it (hear only this layer of the loop),
- **×** on the picked layer, to delete it. A deleted layer comes back with UNDO, like an overdub. On the loop's only layer, **×** clears the whole loop.

{{shot: looper-listen | The music stopped, the Bass layer playing on its own: its ▶ is lit, and the playhead shows where Listen has got to.}}

Tap a lane's wave to pick that layer. Rest a finger on a lane and turn the data wheel to set its level (SHIFT: its pan).

Four lanes show at a time. When a loop has more layers, a thin map down the right has a mark for each, in its colour, with the four in view outlined, and a label says how many older layers are below ("↓ 2 older layers: swipe up"). **Swipe up or down on the lanes** to see the others, or tap the map to jump there. They stay where you leave them until you pick another layer.

You can also hold **ERASE** and tap a layer's lane to delete it, with the same UNDO.

### Changing a loop that plays

Under the lanes, a loop that plays shows how it can change:

- **Feedback**: turns the older layers down a little each time the loop goes round, from 100 % (kept as they are) down to 50 % kept per pass, so a loop can evolve as you keep overdubbing. Drag it up or down, or rest a finger on it and turn the wheel. A layer it has faded says how far ("first · at 49 %").
- **Overdubs**: how an overdub records. **Add** (the default) keeps adding on top, pass after pass, until you end it. **Replace** puts what you play in place of what was there, for as long as you record, and the rest of the loop plays as before: for fixing one bar, or swapping a part live. Nothing is erased: UNDO takes the new part away and the old one is back, and muting the new layer brings the old part back under it too. Its lane says **replace**. Held for more than a pass, a Replace takes the place of its own first pass as well, and a Replace in which you play nothing leaves those bars silent. **Once** records one pass of the loop and then the loop plays on by itself, so a tap is one take; the end of the pass blends into its start, so it has no seam.
- **Reverse** plays the loop backwards (it says **Reverse on** while it does).
- **Speed** plays the loop like tape: **½×** is half speed, an octave down and twice as many bars, **2×** double speed, an octave up and half the bars, and **1×** as you recorded it. The bars and the beat stay locked; a drum loop at half speed becomes a new, slower groove. UNDO takes a speed change back. A moment after you change it, the loop's sound is remade at that speed, which cleans up the octave. If you OVERDUB at another speed, the overdub waits for that, then the loop is fitted to it (its layers become one layer, as for a tempo change) and the overdub starts: UNDO takes the fit back. A loop fitted at half speed takes twice the memory, so in a full memory the oldest undo steps can go to make room.
- **½** keeps the first half; **× 2** doubles it, so you can overdub a longer phrase on top. The loop's length sits between them.
- **Undo** and **Redo**, with the loop's history between them as a row of coloured chips, one per step: an overdub in its layer's colour, an edit in grey, the overdub recording now outlined, and the steps you undid hollow. Tap the chips to open the undo history (below).
- **Merge** turns all the layers into one, which frees memory for more layers. It shows its progress while it works, and UNDO takes it back like any other step.

**Nudge:** drag the ruler sideways to move the loop earlier or later against the beat, a sixteenth note at a time.

**Trim:** a grip sits at each end of the ruler. Drag one in to cut the loop shorter; it moves in sixteenth notes and the part cut off is dimmed. Let go to trim.

These work while the loop plays, not while it records; tapped then, they say why on the line at the bottom.

### Recording a new loop

Pick an empty loop and the space under the lanes shows how it will record, and the lanes' space shows **What you played**: the input over the last eight bars, so you can see it is coming in before you start, with the song's bar numbers and the bars [Capture](#capture-keep-what-you-just-played) would take marked:

| Setting | What it does |
|---|---|
| **Sync** | **Song** (the default): the loop keeps time with the song, as above. **Free**: it keeps its own time with the other free loops, plays on when the song stops and records with the song stopped ([below](#free-loops)). Your choice sticks, as Starts' does. Sync shows the picked loop's own clock: once a loop waits to record, records or has sound, its Sync is faint and stays as it was; tap it and the hint line says why. |
| **Records from** (over the loops) | **Pads** (the default): what the pads play, so a drum pattern on another instrument stays out. It records the mixer channel of the instrument the pads play: the selected track's own synth, its kit (kit 1 is on **Drums**) or the **Synth** channel, with that channel's effects and fader; and the synth on the **Synth** channel while the synth is on the pads by **Play Poly on the pads** (the Synth tab; **Pads: Poly** at the top of the screen). That instrument's own clips play on the same channel, so they go in with what you play: the choice says so ("what the pads play, and clips on that sound"). It leaves out the echoes and reverbs of the sends (turn up the Loop track's own sends to hear the loop through them, or record **Master**) and the master's own effects and Level, which the loop plays through anyway. While the pads play MIDI out it records silence: record that synth's sound with **Line in**. **Line in**: the INPUTS on the back of the MPC (it opens the line input). **Loops**: all the other loops as you hear them, so you can bounce several loops into one and keep layering. **A mix**, from the row under them: **Master** is everything you hear but the loops (the tracks, the audio tracks and the line input), before the master's own effects and Level; a track or a submix is what it sends on, after its effects and fader, named as the Mix tab names it. The Loop track isn't offered, and a submix the Loop track plays into says it can't be recorded (the loop would record itself). |
| **Monitor** | With Line in: hear the input through the outputs, about 9 ms late. Leave it off if you already hear your instrument another way, such as through a mixer. |
| **Length** | **Pattern** (the default): as long as the pattern playing (the longest clip playing, in whole bars), so the loop always fits the beat; with nothing playing it records as on Until tap. **Until tap**: from the next bar line until you tap the ring again (it ends on the next bar line) or press STOP (it stops at once). Or 1, 2, 4 or 8 bars, and it ends by itself. With [Tempo from the first loop](#tempo-from-the-first-loop) on and the music stopped, a first loop with no set length records in your own time and sets the tempo; so does a first [free loop](#free-loops), which sets the free loops' tempo instead. |
| **Starts** | **On the bar** (the default): the first pass starts on the next bar line, or when PLAY starts the music. **When I play**: the loop waits for you, and your first note starts it ([below](#start-when-you-play)). Your choice sticks: every empty loop takes it until you change it. With **Loops** it is always on the bar. |
| **Then** | What the loop does when its first pass ends: **Play**, or go straight into an **Overdub**. |

{{shot: looper-sources | Records from tapped: Pads, Line in and Loops, and the mixes under them, the master first.}}

{{shot: looper-empty | An empty loop picked: What you played over the last eight bars, the two Capture would take marked, and how the loop will record under it.}}

#### Start when you play

With **Starts: When I play**, tap the ring (or press REC) and the loop waits for you for as long as you like: the ring says **Play to start**. Your first note starts it, so you can arm the loop and then play with both hands, or a guitar, without reaching back for a button.

- **What counts as your first note:** with **Records from: Pads** (or a mix), the first pad you play (a pad, not the sound: the music playing doesn't start it). With **Line in**, the first sound on the input louder than -40 dB.
- **With the music stopped**, your note starts the music too, from the start of the bar it stopped in (bar 1 after STOP twice), with no count-in. Bar 1 begins on your note, so the loop, the sequencer and the metronome all start where you started. If notes are armed and ready to record (SHIFT + the track's Arm), they record from that note too.
- **With the music playing**, the loop starts on the bar line nearest your note. Come in a little late (in the first half of a bar) and it starts on that bar's line, with what you played since kept: the looper was listening all along. Come in early (in the second half) and it starts on the next bar line, and the notes you played before it lead into the loop each time it comes round, as a pickup does.
- **Counting in,** a note in the last half bar before bar 1 is a pickup into bar 1. A note earlier than that is forgotten, and the loop waits for the next one.
- If the music can't start (it follows another device's clock, or an export is running), the screen says why and the loop keeps waiting.

{{shot: looper-listening | Loop 5 set to start when you play, waiting: Play to start.}}

#### Tempo from the first loop

To play in your own time and have everything follow you, turn on **Tempo from the first loop** in the Looper tab's MENU. Then, with the music stopped and every loop empty, the first loop doesn't wait for a bar line: tap its ring (or press REC) and it records from that moment, or, with **Starts: When I play**, from your first note. The ring says **Free** and counts the seconds.

When you tap the ring (or press REC) again, the loop closes there and its length sets the tempo. The music starts at that tempo just as the loop comes round, from the start of the bar it stopped in, so the loop, the sequencer and the metronome all follow it. If notes are armed and ready to record, they record from there too.

How many bars the loop is depends on its **Length**:

- **1, 2, 4 or 8 bars** means that many bars at the tempo you play. Set Length to 2 bars, play two bars at whatever speed feels right, close the loop, and the tempo is whatever makes those two bars. Nothing is guessed.
- **Until tap** (or **Pattern** with no pattern playing) lets MPFree work out the bars. A loop's length alone can't tell 1 bar at 80 BPM from 2 bars at 160, so it picks the likeliest: a 2.5-second riff becomes one bar at 96 BPM, and a 7-second one four bars at about 137 BPM. When two answers are close, such as 1 bar at 79 or 2 bars at 158, it still picks one so the music can start, and the picked loop's header says **Close call** with a button for the other ("Read as 2 bars at 158 BPM"). One tap switches: the loop sounds just the same, and the tempo changes on its next downbeat. UNDO reads it as before. [The loop point view](#the-loop-point-view) has **Read as** too, for half or double the bars. A tempo you set yourself before recording (tapped, typed or turned) counts towards the answer, so if you set 80 first, that loop is 1 bar at 79. When you play the loop on the pads or a keyboard, the beat you played counts too: hits that land on the beats of 3 bars, and not of 4, make it 3 bars, so a loop can also be 3, 5, 6 or 7 bars when what you played clearly says so. The log says when the beat decided it ("from the beat you played"). To be sure of the bars, set Length.

- **The tempo is exact,** to many decimal places, so the loop never drifts against the song; the project keeps it exactly when you save. The tempo chip shows it rounded.
- **Closed too soon** (shorter than a bar at 300 BPM, 0.8 s in 4/4, or than your Length's bars at 300 BPM), the loop keeps recording and closes as soon as it is that long. The ring says **closing**.
- **Too long for its Length** (2 bars set, but so long that 2 bars would be slower than 30 BPM), MPFree works out the bars instead and the screen says so.
- **PLAY while it records** throws the free loop away (the screen counts it), since the music would start at the old tempo. Close the loop with its ring or REC instead.
- It is off while another device's clock leads, the song has more than one tempo, or an export is running. A second loop, or a loop recorded with the music playing, keeps to the bars as usual.
- If the music still can't start at the new tempo, the screen says why, and the loop plays at the song's tempo when the music starts.

{{shot: looper-free | Loop 1 recording free with the music stopped: closing it sets the tempo.}}

A few things to know about where a loop records from:

- **Pads follows what the pads play.** When the pads play another instrument (another track picked while they follow the track, or **Play Poly on the pads** on the Synth tab), the looper moves to its channel (the log says which: "looper: records the pads (the synth)"), and **What you played** starts again, so a Capture never mixes two sounds.
- **A take keeps its source to the end.** While a loop records, overdubs or waits to capture, it keeps what it started with: pick another **Records from**, or move the pads to another instrument, and the take carries on as it was. The chip over the loops then says **THIS TAKE · NEXT: MASTER** (or what you picked) over what this take records, the hint line says so, and the next take records your pick.
- **A gap, said.** If a take loses its sound partway through (the Loop track's send turned up while it records Master, say), it records silence from there: the screen says so, and again when the sound comes back.
- **Nothing else in its place.** If the looper can't get the sound you picked (for example, the Loop or Audio track goes to a submix while you record Master), it records silence and says why, and a new take or overdub won't start until it can.
- **Master and the loop's own sound.** While the Loop track sends to a return (louder than the -60 dB "off" the Play page leaves it at), or goes to a submix, the looper won't record **Master**: the loop would record its own echoes, and every overdub would add them again. Turn that send down (or send the Loop track to the master), or record a track or submix instead. After the send comes down, Master waits until that return's echoes of the loop have died away (a delay's repeats, a reverb's tail), and the screen says so.

Press **MENU** on the Looper tab for the rarer settings as a list, with an explanation beside each: the line input itself, the latency to make up for, **Loop level** (-24 to +6 dB), **Tempo from the first loop** ([above](#tempo-from-the-first-loop)), **Smart loop end** ([below](#smart-loop-end)) and **Loop memory**. The Loop level, Tempo from the first loop, Smart loop end, and the latency once you set it by hand, are kept for you whatever project is open; the line input is off at every start, so the Looper records the pads until you turn it on.

#### Smart loop end

Closing a loop exactly on the beat is hard. Tap a little late and the loop is a little long, so the tempo comes out slow and the loop stumbles each time round. MPFree can listen to what you played around your tap and find the downbeat you meant: the notes, when you play that first loop on the pads or a keyboard, or the sound itself, when the Looper records the line input or a mix. Most players hit the loop's first beat again as they close it, and that is what it looks for. Choose what it does with **Smart loop end** in the Looper tab's MENU:

- **Suggest** (the default): the loop closes on your tap, as before. If you clearly played the downbeat a little off your tap, MPFree finds it, and the picked loop's header says so in a chip after its length ("48 ms late") with **Snap** in it. Tap Snap and the end moves there on the loop's next downbeat, and the tempo with it. The chip then says what it did ("Snapped 48 ms earlier") and offers **Use my tap**, to put it back the same way. UNDO takes a snap back too.
- **Auto**: after your tap the loop listens on for a moment (a tenth of a second at most, a little more from the line input or a mix; the ring says **closing**), then closes on the downbeat you played. The music starts in step with where the loop has got to, a little way into bar 1, so you hear no gap. With **Then: Overdub**, what you played while it listened on is the start of the overdub, so nothing you played is lost.
- **Off**: the loop closes on your tap.

The start gets the same care. If you started the loop a moment before your first hit (pressed REC, then played), that gap would otherwise sit at the start of every pass. With **Auto** the loop starts on your first hit instead, and the chip says so ("Started on your first hit, 80 ms after REC") with **Use my press** to put the start back; with **Suggest** the chip says the loop could ("first hit 80 ms after REC"), and Snap does it. Your first hit counts as the start only when it comes within a tenth of a second of your start; later than that, the loop starts where you started it.

It only looks near your tap: within a tenth of a second, or a 1/16 note at very fast tempos. A hit wins when it sounds like the loop's first hit (the same pad or note, about as hard), fits the beat of what you played, and what follows it sounds like the start of the loop again. A hit near the edge of that reach must also fit the beat: at fast tempos a pickup an 8th before the downbeat can sound just like your first hit. If no hit is clearly the one, your tap stands and the screen says why: "Kept your tap: no clear match near it."

When the Looper records the line input or a mix (**Records from**), Smart loop end listens to the sound itself. It hears each hit, a drum, a pluck or a strummed chord, and looks for the one that sounds like the loop's first: about as loud, with the same tone. It starts the loop on your first hit's first sound over -40 dB, and ends it where the sound coming round lines up with that start, to the sample, so both ends are measured the same way. Hearing takes a moment longer than notes: Auto waits up to 30 ms more for the answer, and if it doesn't come in time it keeps your tap and says so ("Kept your tap: it couldn't listen in time"). A sound without clear hits, such as a held pad or a drone, keeps your tap too: "Kept your tap: no hit near it."

The beat you played still helps count the bars when Smart loop end is Off.

The chip lasts until you keep a side (below), or the loop changes in another way: an overdub, an edit, a trim.

With your hands on an instrument, a footswitch can do the chip's job: learn one on **Snap** and one on **Use my tap** in the mixer's Learn section (see [A footswitch](#a-footswitch)).

##### The loop point view

Tap the chip's words (not its button) and the loop point view opens in the lanes' place; tap them again, or **×**, to close it. Nothing else on the tab moves.

- **Start** and **End**: the sound around your start and your tap, 300 ms of each, from the first pass as it was recorded. Your press (REC) and your tap are grey dashed lines; what smart loop end found is a line in the accent colour, with how far it is from you ("−48 ms"). What is outside the loop now is shaded.
- The line under them says what the other side makes of the loop: "Snap makes it 1 bar at 97.87 BPM, from the loop's next downbeat. Your tap stays one tap away."
- **Snap** and **Keep my tap**; once snapped, **Use my tap** (or **Use my press**) and **Keep the snap**. Each goes to that side on the loop's next downbeat, the tempo with it. A **Keep** button also puts the chip and the view away: you have decided.
- **A/B** swaps between your tap and the snap on every downbeat, so you can hear which sits better. While it swaps, the buttons are **Keep the snap** and **Keep my tap**; tap one to stop there.
- **Read as**: the loop's length read as other bars, such as half or double ("2 bars · 195.74 BPM"). The reading it is now is lit.

When nothing was found and the only question is a close call, the view shows your press and your tap, **Keep my tap** and **Read as**.

##### When a snap would move recorded audio

A snap changes the song's tempo. If the audio tracks hold recordings, they would play out of step at the new tempo, so the hint line asks first ("Loop 1: that changes the tempo under the audio tracks' recordings. Snap anyway?"), with **Snap anyway** and **Not now** at its right end. **Snap anyway** goes ahead on the next downbeat (Read as, Undo and Redo say **Do it anyway**); **Not now** leaves everything as it is.

### Capture: keep what you just played

The looper is always listening. While the song plays it keeps the last 60 seconds of what comes in from **Records from** (the pads, the line input, the loops or a mix), and where each bar line fell, whether you pressed Record or not. So when you play something good without recording it, it isn't lost: **Capture** turns the last bars you played into a loop.

- Press **SHIFT + REC** on the Looper tab, or pick an empty loop and tap the button over **What you played**, which sits on the bars it will take and names them ("Capture bars 10 and 11"). On the Play page's loop row, **Capture** says how many it will take ("Capture last 2 bars").
- The bars go into the picked loop if it is empty, or else into the first empty loop. Its **Length** says how many: 1, 2, 4 or 8 bars, the pattern's length for **Pattern**, and 4 for **Until tap** (or Pattern with nothing playing). Into a [free loop](#free-loops) (Sync: Free) it takes the free loops' last bars.
- They are the last whole bars, up to the bar line you just passed. Press it in the last beat of a bar and it waits for the bar line coming, so the bar you are finishing goes in too.
- **Pick other bars:** the bars Capture takes are shaded on **What you played**, with a grip at each end. Drag the shading to take earlier bars: it moves a bar at a time, and stays on those bars as the song plays on (drag it back to the newest bar line and it keeps up again). Drag an end to take more or fewer: it grows and shrinks a whole pattern at a time (with **Length: Pattern** and a pattern playing; otherwise by the Length's bars, or a bar), with a dashed line where each repeat of the pattern starts, and the button says how many ("Capture bars 5 to 16 · 3 × pattern"). What you played shows at least four patterns' worth of bars, as far back as the 60 seconds go. A Capture, or a new Length, puts it back to the Length's bars.
- A moment later the loop plays, in time with the song, as if you had recorded those bars: each bar plays where it belongs against the beat. Until then its ring waits in red, as a loop armed to record does, and says **Capturing** (or the bar it waits for, "13, Capture on bar"). What you played on a bar line lands on the loop's first beat, with the [latency](#latency) made up for as when you record.
- If fewer bars were played than the Length asks, it takes fewer, halving (8, then 4, 2 and 1): only bars since you pressed PLAY count, and since **Records from** (or, for Pads, the instrument the pads play) changed. Before a whole bar has gone by there is nothing to take, and Capture says so.
- **Capture again**, **UNDO**, or a tap on the loop's ring (or its loop key) while it waits for its bar line or copies the bars in calls it off. Once it plays it is a loop like any other: overdub on it, change it, clear it.

Capture needs room in the loop memory for the bars, as a recording does; when there isn't any it says so.

{{shot: looper-window | What you played's window over three times a 2-bar pattern, a bar back from the newest bar line: a grip at each end and a dashed line where each repeat starts.}}

### The Q-Links

| Q-LINK bank | Q1 to Q4 |
|---|---|
| **Loops 1-4** (the first) | Loops 1 to 4's levels, half a decibel a click (SHIFT: a tenth). |
| **Loops 5-8** | Loops 5 to 8's levels. |
| **The picked loop** | **Feedback**, the picked layer's **level** and **pan**, and the **seam fade**: how smoothly the loop's end joins its start, from a hard cut to the looper's longest fade, a millisecond a click. |

Press **Q-LINK** to step from one bank to the next, and **SHIFT + Q-LINK** to step back; the line at the bottom names the bank.

### The pads as loop keys

The pads can play the loops instead of the kit, so you can record, overdub and mute any loop without looking at the screen. Press **SHIFT + SAMPLER** again while the Looper tab is open (or tap **Pads as loop keys** over the loops) and the pads become the **loop keys**:

| Pads | Column 1 | Column 2 | Column 3 | Column 4 |
|---|---|---|---|---|
| Top row (13-16) | Loop 1's key | Loop 2's key | Loop 3's key | Loop 4's key |
| Second row (9-12) | Loop 5's key | Loop 6's key | Loop 7's key | Loop 8's key |
| Third row (5-8) | Loop 1's mute | Loop 2's mute | Loop 3's mute | Loop 4's mute |
| Bottom row (1-4) | Loop 5's mute | Loop 6's mute | Loop 7's mute | Loop 8's mute |

- **A loop's key** does what a tap on its ring does, on the moment you hit it: record from the next bar line, cancel, end the first pass, overdub, end the overdub, or let a muted loop play. It picks the loop too. **Hold a key for a second** to clear the loop (UNDO brings it back, as it was before your hit).
- **A loop's mute** mutes the loop at once, and unmutes it. On an empty loop it does nothing. **SHIFT + a mute** solos the loop, or lets go of its solo.

The pads light as the rings do: a key is dark while its loop is empty, blinks red while it waits for its bar, is red while it records the first pass and amber while it overdubs, and has the loop's colour while it plays, flashing brighter on the loop's first beat. A muted loop's key is faint and its mute is red. A soloed loop's mute is blue, and the keys of the loops it leaves out are faint.

The pads stay loop keys on the Looper tab and the Play page, so you can play the loops beside your patterns; on Play the loop row's **Pads: loop keys** lights, and a tap on it gives the pads back. On the other tabs the pads play the kit as usual, and they are loop keys again when you come back. **SHIFT + SAMPLER** on the Looper tab, or **Pads as loop keys** again, gives the pads back for good. So do **TRACK MUTE**, **STEP SEQ** and **16 Levels**, since the pads do one job at a time.

### More pages: the Perform set

The loop keys are page A of the project's **Perform set**: eight pages of pad actions, kept apart from the kit's sounds and saved with the project. While the pads are the set (SHIFT + SAMPLER on the Looper tab), the **PAD BANK** buttons pick its page, as they pick a kit's banks: **A** to **D**, and **SHIFT** or a second press for **E** to **H**. The lit button is the page on the pads.

| Page | What its pads do |
|---|---|
| **A · Loop keys** | The loop keys and mutes above. |
| **B · Throws** | Pads 1 to 8 are Play's eight [throws](playing-live.md#throws), in the order of their tiles: Delay throw, Reverb wash, Stutter, Tape stop, Filter dip, Beat repeat, Drop drums and Vinyl brake. Hold a pad and its throw plays; let go and it stops, as a tile on Play. Drop drums drops the kicks, snares and hats of the kit the selected track plays. |
| **C to H** | Empty to start with: their pads are dark and do nothing until you [put actions on them](pads.md#the-perform-set-on-the-pads-tab). |

A throw pad is dim in its tile's colour and lights up fully while its throw plays. A **track mute** pad is red while its track is muted (in Perform it waits for the launch line, as **TRACK MUTE**'s pads do), and a **scene** pad is dim green (a hit launches that row of the grid). You set up what each pad does on the Pads tab (see [The Perform set on the Pads tab](pads.md#the-perform-set-on-the-pads-tab)).

On a page other than A, the **Pads as loop keys** switch is dark, since the pads aren't the loop keys; tap it to go back to page A. The hint line names the page, and on Play the chip in the top bar says **Pads: Perform B** (or the page's letter): a tap on it gives the pads back. **SHIFT + SAMPLER** brings the set back on the page you left it on. When the pads go back to the kit, any throw a pad was playing stops.

{{shot: looper-perform-throws | The Looper tab with the Perform set's page B on the pads: the hint line names the page.}}

You set up what each pad of the set does on the Pads tab: see [The Perform set on the Pads tab](pads.md#the-perform-set-on-the-pads-tab).

### A footswitch

A MIDI footswitch or pedalboard can play the looper while your hands stay on your instrument: a loop's key and mute, the picked loop's key, **Next loop**, **Undo**, **Redo**, **Capture**, **Clear**, and smart loop end's **Snap** and **Use my tap**, on every page. A key held for a second clears its loop, as a pad does. The MPC One has no footswitch jack, so the footswitch goes into a MIDI controller or pedal. Teach it in the mixer's Learn section: see [Playing the looper from a footswitch](midi-and-cv.md#playing-the-looper-from-a-footswitch).

### Moving a layer to another loop

Drag a layer out of the lanes to give it to another loop. **Press and hold** the layer's lane for a moment, until it gets a blue outline, then move your finger: a card with the layer's name follows it, and the loops on the deck that can take it light up. The card says what will happen if you let go there.

- **Onto a loop that plays:** the layer moves there. It keeps its level, pan and mute, and it plays in time with that loop's bars.
- **Onto an empty loop:** the layer becomes a loop of its own, the same length as the loop it came from.
- **Hold SHIFT as you let go** to copy it instead: the layer stays in this loop too.
- **Let go anywhere that isn't lit** and nothing changes.

The layer comes in a moment later (about a sixteenth of a second for a 2-bar loop at 120 BPM), fading in where the loop has got to, and fades out of its own loop at the same moment, so you hear no gap and no doubling. Afterwards the loop that took it is picked, so the lanes show it there.

**UNDO** takes a move back in both loops at once, from either loop, as long as it was the last thing you did in each. Undoing a layer that made a loop of its own empties that loop again.

A layer only goes onto a loop of the same length in bars. A loop of another length isn't lit, and dropping on it tells you why ("Loop 2 is 2 bars and this layer 4 bars"). A loop that is recording or busy (merging, for instance) says so too, and so does a loop that already has as many layers as it can hold. A loop's last layer can't leave it, so dragging it copies it.

With the **data wheel**: pick a layer, push the wheel to pick it up, turn it to point at a loop, and push again to drop it there (hold SHIFT for a copy). Change page and it is put back.

### The undo history

Tap the history chips between Undo and Redo to open a list of everything you did to the picked loop, oldest first, in the lanes' place: Start (the first pass), then each overdub, deleted layer, edit and merge. The step you are on is outlined, and steps you undid are faint below it. Tap any step to go back (or forward) to just after it. Drag to scroll the list. Tap the chips again to close it; picking another loop closes it too.

{{shot: looper-history | The undo history open in the lanes' place on a loop with two overdubs.}}

### Send

**Send** shows what you can do with the whole loop, in the lanes' place. **Show in Clips** opens the Arrange tab with the loop's cell in view. **To a pad**, **To an audio track** and **Save WAV** are coming next; they show faint for now, and say so when tapped.

## Free loops

A loop keeps time with the song ([above](#how-the-looper-keeps-time)), unless you make it a **free loop**. A free loop keeps its own time: it plays on when the song stops, keeps its place when you move the song's playhead or the song's loop goes round, keeps its tempo when the song's changes, and records with the song stopped, at once or in your own time. It is the MPC's Looper with Sync off, with up to eight loops that stay together.

To make one, pick an empty loop on the Looper tab and tap **Sync: Free**, the first setting under What you played. The choice sticks, as Starts' does: every empty loop records free until you tap **Song** again (on an empty loop), and each says **Pads · free** (or what it records from) under its ring. A loop keeps the Sync it was recorded with: once it waits to record, records or has sound, its Sync shows its own clock, faint, and doesn't change (tap it and the hint line says "This loop is free and stays so: pick an empty loop to set Sync"). Pick an empty loop to set Sync for the loops you record next.

{{shot: looper-free-empty | Sync on Free for the empty Loop 5, Loop 1 a free loop playing on at its own tempo while the song is stopped: every empty loop says Pads · free, and the line over the loops adds the free loops' tempo.}}

### One free clock

All the free loops keep time together, on the looper's **free clock**, so free loops recorded over each other stay in step, as on a looper pedal. The first free loop sets it:

- **With the song playing**, the free clock is a copy of the song's: the loop starts on the song's next bar line and sounds exactly like a song loop, until the song stops, jumps or changes tempo. Then it plays on as if nothing had happened.
- **With the song stopped**, the loop records in your own time, at once (or on your first note, with **Starts: When I play**); its ring says **Free** and counts the seconds. With **Length: Until tap**, tap the ring or press REC again to close it: its length sets the free loops' tempo, read as [Tempo from the first loop](#tempo-from-the-first-loop) reads it, close calls and Smart loop end included, and it plays on at once. The song's tempo isn't touched and the song stays stopped. With a Length of 1, 2, 4 or 8 bars, it is that many bars at the tempo you played, and its header says so while it records ("recording 4 bars free · tap when they are done"): the pass still ends on your tap, since there is no tempo yet to count the bars.

A free loop says **free** on its card where a song loop says its bars, and its header says so too ("free · 4 bars · 3 layers"); its ring counts its bars on the free clock. When you clear the last free loop the free clock goes with it, and the next free loop sets a new one.

{{shot: looper-free-own-time | Loop 1 recording free in your own time with the song stopped: closing it sets the free loops' tempo, and the song stays stopped.}}

| You press Record (or tap an empty free loop's ring) when | It starts | It ends |
|---|---|---|
| **Free loops play** (the song playing or not) | On the free loops' next bar line, at least a beat away. | After Length's bars, or on the free loops' bar line after your closing tap. |
| **The song plays, and no free loop** | On the song's next bar line. Stopped free loops start again from their tops there, with it. | The same, in the song's bars. |
| **Nothing plays, the free loops are stopped** | At once (or on your first note). The free loops start from their tops with it. | The same, in the free loops' bars. |
| **Nothing plays, no free loop yet** | At once (or on your first note), in your own time. | Exactly on your closing tap: its length sets the free loops' tempo. |

While a free loop records, **PLAY** starts the song and the loop records on, and **STOP** with the song playing stops the song and the loop records on. **STOP** with the song stopped ends the pass at once and keeps what you played, as it does for any loop; in your own time it is your closing tap.

### Stopping and starting free loops

| You press | The song is playing | The song is stopped |
|---|---|---|
| **STOP** | Stops the song. The free loops play on. | Stops the free loops, at their tops, with every other sound. |
| **PLAY** | Nothing new. | Starts the song and the stopped free loops together. |
| **SHIFT + STOP** | Stops everything, the free loops too, and the song goes back to bar 1. | Stops the free loops, at their tops, with every other sound. |
| **Stop free** (on a free loop's header, in Listen's place) | Stops the free loops at their tops. | The same. |
| **Play free** (on a free loop's header, or a tap on a stopped free loop's ring or loop key) | Starts the free loops from their tops on the song's next bar line. Play free lights while it waits: tap it again to call it off. | Starts them at once. The song stays stopped. |

So STOP stops the song first, a second STOP stops the free loops too, SHIFT + STOP stops everything at once, and PLAY starts everything together. The line over the loops names both: the song ("Song stopped" or its bar and beat) and **Free** with ▶ or ■. The hint line at the bottom says what STOP will do now: **STOP: the song**, **STOP: free loops**, or, with the free loops stopped, **PLAY: song and free loops**.

The free loops stop at their **tops**: each waits at its place on the free loops' bar 1, so they come back together exactly as they played. While they are stopped, a free loop's ring rests faint where it will start again, with ▶ in the middle, and its header ends in **stopped**. REC or OVERDUB on a stopped free loop starts the free loops and records at once. To take one free loop out and back in, mute it, as any loop.

{{shot: looper-free-picked | A free loop picked while the song plays: Loops 1 and 3 say free on their cards, Free ▶ over the loops, Stop free in Listen's place, and the hint line says STOP stops the song.}}

{{shot: looper-free-stopped | After STOP, STOP: Song stopped and Free ■ over the loops, the free loops wait at their tops with ▶ on their rings, Play free in Listen's place, and the hint line says PLAY starts the song and the free loops.}}

### What else to know

- **Everything else works as on any loop**: overdubs (Add, Replace, Once), mute and solo, Feedback, Reverse, Speed, ½ and × 2, Nudge and Trim, Merge, Undo and the history, the layers, the Q-Links, the loop keys and a footswitch. Their bars and sixteenths are the free loops'.
- **The song doesn't reach them**: a tempo change, a jump of the playhead, the song's loop and a count-in leave free loops as they are, so they are never remade for a new tempo. **Speed** still plays one at half or double speed.
- **Capture** into a free loop takes the free loops' last bars, and What you played shows and numbers their bar lines. **Smart loop end** works as it does for the first loop: Snap, Use my tap and Read as change the free loops' tempo, never the song's.
- **Play's loop row** shows them as the Looper tab does: **free** after the loop's name, a stopped free loop's ring at rest with ▶ (a tap plays the free loops), and **Pads · free** on empty loops.
- **They are saved with the project**, free clock and all, and open stopped at their tops: PLAY, or Play on a free loop's header, starts them. An older MPFree won't open a project with a free loop.
- **They aren't on the Clips grid or in the song yet**: scenes, launches and Stop all don't reach them, and the looper alone says when they play. So a loop you record into a Loop cell keeps time with the song whatever Sync says: tapping an empty Loop cell picks an empty loop on **Song** (Sync stays Free for the loops you pick after it), and tapping **Sync: Free** with a cell armed lets the cell go ("Loops A · row 3 let go: a free loop has no cell").

{{shot: play-free-loops | Play with the song stopped and Loop 1 playing on free: free after its name, Pads · free on every empty loop, and the hint line says STOP stops the free loops.}}

## Loops in the Clips grid

Every loop you record also shows on the **Arrange** tab's Clips grid, on a **Loop track**: a column named **Loops A**, **Loops B** and so on, with a little wave and what it records under the name (**Pads**, for the pads and the synth). A loop's cell is named after it ("Loop 2") and shows the loop's wave and its length; while it plays, the wave lights up to where the loop is, and it turns red with **overdub** while you add a layer. The grid's title, **Loops 4 of 8 · memory 38 %**, says how many of the eight loops hold something and how much of the loop memory they use. A loop's cell launches, stops and plays in scenes like any clip, on the bar line:

- **A loop you record lands on a Loop track with nothing playing**, in the row of the scene playing (row 1 if none), and keeps playing. When every Loop track already plays a loop, MPFree adds one, so loops you record one after another still play together.
- **One loop plays per Loop track**, as one clip plays per track; loops on different Loop tracks play together. Tap another loop's cell on the same Loop track and the one playing goes quiet on the next bar line as the new one comes in, both on the bar line exactly.
- **Scenes, Stop all and TRACK MUTE work on loops too.** A scene plays the loops in its row, and a Loop track with nothing in that row stops. Stop all stops every loop on the next bar line. A muted Loop track's loop goes quiet at once, or on the launch line when the mute waits (in Perform; see [Muting tracks and pads](playing-live.md#muting-tracks-and-pads)).
- **A loop that isn't playing keeps its place** in the song: launch it again and it comes back in step.
- **Clear** a loop, or undo its first pass, and its cell empties (Redo of the first pass puts it back). **½ length**, **× 2** and **Trim** change the cell's length with the loop's.
- **A Loop track past the first 8 tracks** is on the grid's next page: tap **›** over the grid, or **Send** > **Show in Clips** on the Looper tab.
- **The pads never play a Loop track.** Picking one keeps the pads on the track they were on; a tap on an empty Loop cell records a loop there instead (below). Drag a loop's cell to another row or another Loop track. Loops stay on Loop tracks, and clips on the other tracks.
- **Copy a loop's cell** by holding **SHIFT** as you lift it:
  - **Into another row of its own Loop track**, both rows play the same loop. A scene change from one of those rows to the other keeps the loop playing without a break.
  - **Onto an empty cell of another Loop track**, it becomes a new loop with a copy of every layer and its mix. The cell says **copying in** for a moment, then shows the wave. The copy stays quiet until you tap its cell, so you can change it on the Looper tab while the first one plays. It takes one of the eight loops and room in the loop memory, and says so when there isn't any. **UNDO** on the Looper tab, with the copy picked, takes its layers back; once they are all gone, its cell empties.

A project opens with nothing launched, as its clips do: tap a loop's cell, or its scene, to play it.

### Recording a loop into a cell

1. To add a Loop track, hold **SHIFT** and tap **+ Track** over the grid: it is named Loops A, Loops B and so on. **UNDO** takes it away again.
2. Tap the line under a Loop track's name to choose what its new loops record: **Pads** (the pads and the synth), **Line in** (the MPC's inputs; it turns **Line input** on) or **Resample** (the other loops, as you hear them).
3. Tap an empty cell of the Loop track. It turns red and says **Armed**, and the REC chip names it ("REC → Loops C · Drop", the row's scene name or "row 3"). The loop it records keeps time with the song, even with Sync set to Free ([free loops](#free-loops) aren't on the grid yet).
4. Press **REC**: the cell says **Ready**. Press **PLAY** if the music is stopped; the loop starts on the next bar line. The cell says **REC** while it records.
5. Press **REC** again: the loop closes on the next bar line and plays in that cell. If another loop was playing on that Loop track, it stops, since one loop plays per Loop track.

With a cell armed, **OVERDUB** adds a layer to the loop that Loop track is playing. Tap the armed cell again, or arm the notes (a track's Arm pill, the Sequencer tab), to let go of it. When all eight loops hold something, or the loop memory is nearly full, the tap and REC say so: clear a loop on the Looper tab first. A new project starts with no Loop track; the first loop you record makes Loops A.

### Loops in the song

Loop tracks have lanes in the **Song view** too (tap **Song ›** above the grid), with what they record under the name. A loop's block plays like a clip's:

- **Song on** brings each loop in where its block starts and takes it out where it ends, on the tick, on any Loop track where you haven't launched a cell and that isn't muted. Launch a cell and it takes that Loop track over, as a clip does; stop it and the track goes back to the song.
- **Rec song** keeps the loops you launch on the grid, each as a block from where it came in to where it went quiet, linked to its cell.
- **Scenes → song** puts the loops in each row into the song with the clips.
- A block plays the loop where it would be if it had played all along, as a launch does: a block of a two-bar loop that starts a bar into the loop's pass starts in its second bar. A loop's block stays on its own lane: drag it along the lane, but not onto another track, and no clip goes onto a Loop track's lane.

{{shot: arrange-song-loops | The Song view with Loop tracks: Loops A and Loops B play their loops' blocks under the drums, hats and bass.}}

### A loop's name

The Looper tab and the REC chip name a loop by its place on the grid: **Loops B · Loop 2 · Verse** is Loop 2, on Loops B, in the row of the Verse scene ("row 2" for a row with no scene name). On the Looper tab, the picked loop's header shows its track and row ("Loops B · Verse · 2 bars · 3 layers"). **Show in Clips**, under **Send**, opens the Arrange tab with the loop's cell in view.

## Recording an instrument

1. Plug the instrument into the MPC's **INPUTS** and turn **REC VOL** up until the loudest parts are near the top of the meter but never at the end of it.
2. On the Looper tab, pick an empty loop and tap **Line in** under the lanes. It opens the line input, and the input chip's meter and the lanes' space show what comes in.
3. If you can't hear the instrument any other way, turn **Monitor** on.
4. Press **PLAY**, then tap the loop's ring (or press REC).

> [!NOTE]
> The current test build reads the INPUTS as left and right, but which jacks it hears has not been confirmed on every MPC One. If you hear nothing, check the input chip's meter on the Looper tab first.

### Latency

Sound takes a few milliseconds to reach the speakers, and your playing takes a few more to come back in through the inputs. So what you play along to the loop arrives slightly late. The looper makes up for it by writing the input that much earlier in the loop, so your overdubs land where you played them.

Recording MPFree's own pads needs nothing made up: they are already in time. For the line input, MPFree estimates the latency from the audio buffer setting. To measure it exactly, connect a cable from an output to an input and set **Latency to make up for** until a recorded click lines up with the bar line.

## Coming next

- **A loop's block from its own start**: a block that starts the loop from its top wherever it sits, and **Print to audio track**, turning a loop's blocks into regions on an audio track.
- **To a pad**: send a loop to a pad as a sample on the Sample tab. You will also be able to drag a single layer onto it.
- **Save loop**: keep it on the stick with its layers. Your loops are also saved with your project, every layer with its mix (see [Saving your work](saving.md)). A loop is saved as it sounds, its loop point and seam fade included; it opens again sounding the same, without the moment kept around it.

> [!MPC]
> On the MPC the Looper is a separate screen with one loop, its length set in bars before you record. With Sync off it records and plays while the sequencer is stopped, and only then can it start when the input passes a threshold. MPFree has up to eight loops with a mixable lane per layer, and Capture keeps what you played even when you didn't press Record. Its loops follow the song's bars and tempo (or, with [Tempo from the first loop](looping.md#tempo-from-the-first-loop), set it, or keep their own time as [free loops](looping.md#free-loops), the MPC's Sync off for up to eight loops on one free clock), can [wait for your first note](looping.md#start-when-you-play) or the line input, can record MPFree's own pads and synth as well as the inputs, and make up for the input's latency so overdubs land in time.
