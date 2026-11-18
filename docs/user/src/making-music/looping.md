<!--
status: changing
covers: rust/mpfree-looper rust/mpfree-app/src/looper_page.rs rust/mpfree-app/src/looper_deck.rs rust/mpfree-app/src/looper_tab.rs rust/mpfree-app/src/looping.rs rust/mpfree-seq/src/loops.rs
checked: 2026-11-17 3a1575d
-->

# Recording loops

[Your first loop](../getting-started/first-loop.md) showed the basics: record a pass, overdub layers on top, undo one. This chapter covers how the looper works in more depth, the **Looper** tab, recording from an instrument, and getting the timing right.

You play the loops on the **Looper** tab, or on the Play page next to your patterns (the looper strip in the All view, or the bigger Loop view).

## How the looper keeps time

The looper is part of the song, not a separate machine running beside it:

- **It starts and ends on bar lines.** Press Record and the loop waits for the next bar line, then records until you press Record again, when it ends on the next bar line. Press **STOP** instead and it stops at once and keeps what you played: the loop runs on to the next bar line, silent after the stop (if you press STOP just after a bar line, within a sixteenth note, it ends on that bar line instead). With a set length (**First pass**, below) it ends by itself after that many bars. So the loop is always a whole number of bars, and always in time with your patterns.
- **It follows the song's position.** Stop and start the transport and the loop picks up exactly where the song is. It is silent while the transport is stopped, and its playhead stops too, at the place the song will start from again (press STOP twice and it goes back to the start of the loop, with the song).
- **The tempo is the song's tempo.** Change the tempo and a playing loop follows it and keeps its pitch. MPFree re-makes the loop at the new tempo in the background, which takes a fraction of a second; until it is ready, the loop plays faster or slower, like tape. Overdub over a loop at its new tempo and MPFree first fits the loop to that tempo, so the new layer lines up. The fit is its own step: UNDO takes back the overdub, then the fit.
- **There are no clicks.** The end of the loop is blended into its start, and every start, stop, punch in and out, mute and undo is faded in a few milliseconds.

## Layers

The first pass is the loop's first **layer**. Each overdub adds another, up to 16. **Undo** fades out the newest layer and **Redo** brings it back, until you record a new overdub. The loop shows the layers in different colours, and on the Play page each layer has a speaker icon to mute it on its own. An overdub in which you played nothing (only silence came in) leaves no layer, so it takes no memory and no step of the history.

To get rid of a whole loop, first pass included, use **Clear** (on the Play page or the Looper tab), or hold the loop's ring on the Looper tab. The **×** on a loop's last remaining layer does the same. **UNDO brings a cleared loop back** as it was, layers, history and all, until you record into that loop again or the memory is needed for another loop's layers (the oldest cleared loop goes first). In Perform, **Clear** and **×** take a half-second hold, so a stray touch can't clear a loop mid-set (see [Make and Perform](playing-live.md#make-and-perform)); the ring's hold is a second long already.

The looper holds five minutes of stereo sound across all its loops and layers (the Looper tab shows how much is left). At 120 BPM a bar lasts two seconds, so that is room for about 37 four-bar layers, spread over up to eight loops of up to 16 layers each. If you need more or less, **Loop memory** in [Prefs](../reference/prefs.md) (also in the Looper tab's MENU) sets it from 1 to 20 minutes. The memory is set aside when MPFree starts, so a new size takes effect from the next start; the row says "(next start)" until then, and shows how many megabytes it uses (with the 30 seconds the looper keeps for [Capture](#capture-keep-what-you-just-played): 120 MB for five minutes).

## The buttons

These are on the Play page's Loop view. The MPC's own **REC** and **OVERDUB** do the same on the loop picked while the loop is armed: the Loop view and the Looper tab arm it, and the REC chip on the transport bar says "→ Loop 1" (see [What REC records](../getting-started/first-beat.md#what-rec-records)). On the home view, the **Arm** pill at the left of the loop strip arms the loop too: red when REC records it, grey when it doesn't. **SHIFT** + Arm adds the loop to the pattern, so one REC and PLAY record the pads into a clip and a loop's first pass together, from the same bar line. **UNDO** while the first pass or an overdub records throws that take away and the loop plays on. On the Looper tab, **SHIFT + REC** captures the last bars you played into a loop (see [Capture](#capture-keep-what-you-just-played)). **UNDO** (and **SHIFT + UNDO** for Redo) works on the loop on the Looper tab and the Loop view, and on the All view when the loop is what you changed last. To record a pattern from the pads with REC, open the Sequencer tab (it arms the notes) or pick a track on the Arrange tab:

| Button | Empty loop | Recording the first pass | Playing |
|---|---|---|---|
| **Record** | Waits for the next bar line (or, stopped, for PLAY), then records. Press again before it starts to cancel. | Ends the loop on the next bar line. | Starts an overdub. |
| **Overdub** | Nothing. | Goes straight on into an overdub when the first pass ends. | Starts an overdub; press again to stop adding. |
| **Undo** | Brings back the loop you cleared, if there is one. | Throws the recording away. | Removes the newest layer (or the overdub in progress). |
| **Redo** | Nothing. | Nothing. | Brings back the last layer undone. |
| **Clear** | | | Deletes the whole loop, so you can start again. UNDO brings it back. |
| **Mute** | | | Silences the loop without losing it. Press again to hear it. |

**STOP** while the first pass records stops it at once and keeps what you played, up to the next bar line, so a loop is never lost. Stopping the transport while you overdub ends the overdub. With the transport stopped, Record or Overdub on a loop that has something in it waits for **PLAY**, with OVERDUB and PLAY blinking; **STOP** then lets go of it (and of a loop armed with Record), so nothing is left blinking.

## The Looper tab

The **Looper** tab is the looper as an instrument: every loop in sight, one touch to record, overdub or mute any of them, and the loop you picked laid out underneath with a lane per layer. Press **SHIFT + SAMPLER** to open it.

{{shot: looper | The Looper tab: the eight loops across the top as rings, the picked loop under them with a lane per layer and the controls a loop that plays needs, and the loops' levels on the Q-Links.}}

### The loops

The eight loops sit side by side across the top, each the same: its name and length, a **ring**, a **level bar** beside the ring, and a **speaker** and an **S** under it. The ring is the loop:

- **Its wave runs round the ring**, and an arc fills as the loop plays, with a tick for each bar and a hand at the place it has got to. The middle says which bar is playing ("2 of 4").
- **Its colour is what it is doing:** the loop's own colour while it plays, red while it records its first pass, amber while it overdubs, grey when muted.
- **An empty loop is a dashed circle** with **+** in the middle and what it will record under it (**Pads**, **Line in** or **Loops**). Once you tap it, it waits for the next bar line: the circle turns red and the middle says the bar it starts on ("13, REC on bar"), or **REC on PLAY** while the music is stopped. Set to start [when you play](#start-when-you-play), it says **Play to start** until your first note.

Tap a ring and the loop does its next step, on the moment your finger lands, so a punch is where you played it:

| The loop is | A tap on its ring |
|---|---|
| Empty | Records from the next bar line (or from PLAY), or from your first note when it [starts when you play](#start-when-you-play). |
| Waiting for its bar | Cancels. |
| Recording its first pass | Ends it on the next bar line, and it plays. |
| Playing | Starts an overdub. |
| Overdubbing | Ends the overdub; it plays on. |
| Muted | Lets it play again. |

The ring you tap also picks that loop. To pick a loop without starting anything, tap its **name**.

**Hold a ring** for a second to clear the loop, every layer and the first pass: the arc drains in red while you hold, and letting go sooner leaves the loop as it was. Hold **ERASE** and tap a ring (or a loop's name) to clear it at once. Either way **UNDO** brings it back: the empty ring says **Cleared · UNDO** until you record into it.

The **speaker** under a ring mutes the loop without picking it: it is kept and keeps its place in the song, but stays silent, through STOP and PLAY too, until you tap it again. The **S** beside it **solos** the loop: while any loop is soloed only the soloed loops play, and the others stay silent in their places (their rings go dim and say **by solo**). Tap S again to let go; clearing a soloed loop lets go of its solo too. The **level bar** beside the ring sets how loud the loop plays (-24 to +6 dB, with a meter of what it plays now): drag it, or rest a finger on it and turn the data wheel (half a decibel a click, SHIFT for a tenth).

Every loop plays at once, in time with the song, but only one records at a time: starting to record in one loop ends the recording in the others. The pads can do all of this too: see [The pads as loop keys](#the-pads-as-loop-keys).

Over the loops, the song's clock (a dot a beat, the bar and beat, the tempo), **Capture** ("Capture last 2 bars", below) and the loop memory every loop shares ("1:42 of 5 min").

### The picked loop

Under the loops, the picked loop has the whole width. Its header has its colour and name, where it lives on the Clips grid with its length and layers ("Loops A · Verse · 4 bars · 6 layers"), and:

- **The input** it records from, with a meter of how loud it is coming in. Tap it to change: **Pads**, then **Line in**, then **Loops**.
- **Send**: what to do with the loop (below).
- **Clear** (outlined in red): deletes the whole loop, like holding its ring. UNDO brings it back.

Under the header, a ruler of the loop's bars, then a **lane for each layer**, a fingertip tall, the newest on top. Each lane has the layer's name ("Drums", "Bass", named after what you played into it) and a tag ("dub 4 · -2.0 dB", "dub 3 · muted", "first · at 49 %"), its own wave, and:

- **a tap on its name** to mute that layer (tap again to hear it),
- **S** to solo it (hear only this layer of the loop),
- **×** on the picked layer, to delete it. A deleted layer comes back with UNDO, like an overdub. On the loop's only layer, **×** clears the whole loop.

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
| **Records from** | **Pads**: MPFree's own sound as you hear it, with the effects on its tracks and the echoes and reverbs of the sends. It leaves out the loops, the audio tracks, the line input and the metronome, and the master's own effects and Level, which the loop plays through anyway. **Line in**: the INPUTS on the back of the MPC (it opens the line input). **Loops**: all the other loops as you hear them, so you can bounce several loops into one and keep layering. |
| **Monitor** | With Line in: hear the input through the outputs, about 9 ms late. Leave it off if you already hear your instrument another way, such as through a mixer. |
| **Length** | **Free** (the default): from the next bar line until you tap the ring again (it ends on the next bar line) or press STOP (it stops at once). Or 1, 2, 4 or 8 bars, and it ends by itself. |
| **Starts** | **On the bar** (the default): the first pass starts on the next bar line, or when PLAY starts the music. **When I play**: the loop waits for you, and your first note starts it ([below](#start-when-you-play)). With **Loops** it is always on the bar. |
| **Then** | What the loop does when its first pass ends: **Play**, or go straight into an **Overdub**. |

{{shot: looper-empty | An empty loop picked: What you played over the last eight bars, the two Capture would take marked, and how the loop will record under it.}}

#### Start when you play

With **Starts: When I play**, tap the ring (or press REC) and the loop waits for you for as long as you like: the ring says **Play to start**. Your first note starts it, so you can arm the loop and then play with both hands, or a guitar, without reaching back for a button.

- **What counts as your first note:** with **Records from: Pads**, the first pad you play (a pad, not the sound: the music playing doesn't start it). With **Line in**, the first sound on the input louder than -40 dB.
- **With the music stopped**, your note starts the music too, from the start of the bar it stopped in (bar 1 after STOP twice), with no count-in. Bar 1 begins on your note, so the loop, the sequencer and the metronome all start where you started. If notes are armed and ready to record (SHIFT + the track's Arm), they record from that note too.
- **With the music playing**, the loop starts on the bar line nearest your note. Come in a little late (in the first half of a bar) and it starts on that bar's line, with what you played since kept: the looper was listening all along. Come in early (in the second half) and it starts on the next bar line, and the notes you played before it lead into the loop each time it comes round, as a pickup does.
- **Counting in,** a note in the last half bar before bar 1 is a pickup into bar 1. A note earlier than that is forgotten, and the loop waits for the next one.
- If the music can't start (it follows another device's clock, or an export is running), the screen says why and the loop keeps waiting.

{{shot: looper-listening | Loop 5 set to start when you play, waiting: Play to start.}}

Two things to know about recording the pads with their effects. The loop's own echo is recorded too if you turn up the **Looper**'s Delay send on the Play page, since that echo is part of the mix. And if you send the Loop or Audio track (or the line input) to a submix in the mixer, the looper can't leave it out of the mix, so it records the pads and the synth without their effects until that track goes back to the master.

Press **MENU** on the Looper tab for the rarer settings as a list, with an explanation beside each: the line input itself, the latency to make up for, **Loop level** (-24 to +6 dB) and **Loop memory**. The Loop level, and the latency once you set it by hand, are kept for you whatever project is open; the line input is off at every start, so the Looper records the pads until you turn it on.

### Capture: keep what you just played

The looper is always listening. While the song plays it keeps the last 30 seconds of what comes in from **Records from** (the pads, the line input or the loops), and where each bar line fell, whether you pressed Record or not. So when you play something good without recording it, it isn't lost: **Capture** turns the last bars you played into a loop.

- Press **SHIFT + REC** on the Looper tab, or tap **Capture** over the loops; it says how many bars it will take ("Capture last 2 bars"). On an empty loop's page, the button over **What you played** names them ("Capture bars 10 and 11").
- The bars go into the picked loop if it is empty, or else into the first empty loop. Its **Length** says how many: 1, 2, 4 or 8 bars, and 4 for **Free**.
- They are the last whole bars, up to the bar line you just passed. Press it in the last beat of a bar and it waits for the bar line coming, so the bar you are finishing goes in too.
- A moment later the loop plays, in time with the song, as if you had recorded those bars: each bar plays where it belongs against the beat. Until then its ring waits in red, as a loop armed to record does, and says **Capturing** (or the bar it waits for, "13, Capture on bar"). What you played on a bar line lands on the loop's first beat, with the [latency](#latency) made up for as when you record.
- If fewer bars were played than the Length asks, it takes fewer, halving (8, then 4, 2 and 1): only bars since you pressed PLAY count, and since **Records from** changed. Before a whole bar has gone by there is nothing to take, and Capture says so.
- **Capture again**, **UNDO**, or a tap on the loop's ring (or its loop key) while it waits for its bar line or copies the bars in calls it off. Once it plays it is a loop like any other: overdub on it, change it, clear it.

Capture needs room in the loop memory for the bars, as a recording does; when there isn't any it says so.

### The Q-Links

| Q-LINK bank | Q1 to Q4 |
|---|---|
| **Loops 1-4** (the first) | Loops 1 to 4's levels, half a decibel a click (SHIFT: a tenth). |
| **Loops 5-8** | Loops 5 to 8's levels. |
| **The picked loop** | **Feedback**, the picked layer's **level** and **pan**, and the **seam fade**: how smoothly the loop's end joins its start, from a hard cut to the looper's longest fade, a millisecond a click. |

Press **Q-LINK** to step from one bank to the next; the line at the bottom names the bank.

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

The pads stay loop keys on the Looper tab and the Play page, so you can play the loops beside your patterns; on Play a **Pads: loop keys** chip in the top bar says so, and a tap on it gives the pads back. On the other tabs the pads play the kit as usual, and they are loop keys again when you come back. **SHIFT + SAMPLER** on the Looper tab, or **Pads as loop keys** again, gives the pads back for good. So do **TRACK MUTE**, **STEP SEQ** and **16 Levels**, since the pads do one job at a time.

### More pages: the Perform set

The loop keys are page A of the project's **Perform set**: eight pages of pad actions, kept apart from the kit's sounds and saved with the project. While the pads are the set (SHIFT + SAMPLER on the Looper tab), the **PAD BANK** buttons pick its page, as they pick a kit's banks: **A** to **D**, and **SHIFT** or a second press for **E** to **H**. The lit button is the page on the pads.

| Page | What its pads do |
|---|---|
| **A · Loop keys** | The loop keys and mutes above. |
| **B · Throws** | Pads 1 to 8 are Play's eight [throws](playing-live.md#throws), in the order of their tiles: Delay throw, Reverb wash, Stutter, Tape stop, Filter dip, Beat repeat, Drop drums and Vinyl brake. Hold a pad and its throw plays; let go and it stops, as a tile on Play. Drop drums drops the kicks, snares and hats of the kit the selected track plays. |
| **C to H** | Empty to start with: their pads are dark and do nothing until you [put actions on them](#setting-up-the-perform-set-on-the-pads-tab). |

A throw pad is dim in its tile's colour and lights up fully while its throw plays. A **track mute** pad is red while its track is muted (in Perform it waits for the launch line, as **TRACK MUTE**'s pads do), and a **scene** pad is dim green (a hit launches that row of the grid). You set up what each pad does on the Pads tab (below).

On a page other than A, the **Pads as loop keys** switch is dark, since the pads aren't the loop keys; tap it to go back to page A. The hint line names the page, and on Play the chip in the top bar says **Pads: Perform B** (or the page's letter): a tap on it gives the pads back. **SHIFT + SAMPLER** brings the set back on the page you left it on. When the pads go back to the kit, any throw a pad was playing stops.

{{shot: looper-perform-throws | The Looper tab with the Perform set's page B on the pads: the hint line names the page.}}

### Setting up the Perform set on the Pads tab

On the **Pads** tab, tap **Perform pads** at the right end of the **Pads of** row: the project's Perform set takes the kit's place. Tap a track's chip to go back to its kit.

- On the left are the page's 16 pads, each with what it does in its corner (**THROW**, **LOOP**, **LOOP MUTE**, **MUTE** or **SCENE**). The pages **A** to **H** are over them, with a dot for each pad that does something: tap one to show it. **PAD BANK** picks the page here too, and the set comes back on the pads at that page.
- **Hit a pad** on the MPC, or tap it on the screen, to pick it. A throw plays while you hold its pad (here always while held, even one that latches), so you can hear it; any other pad is only picked. Nothing records, mutes or launches, so you can set up the set while a song plays, and the kit's pads don't play while the set shows.
- Over the pad's settings, five chips say **what it does**: **Throw**, **Loop key**, **Loop mute**, **Track mute** or **Scene**. Under them are the eight choices for that kind: Play's eight throws, Loops 1 to 8, the song's tracks, or the scene rows (eight at a time, around the one picked). Tap one to put it on the pad.
- The **Pad** list under them has the same settings for the wheel and **-** / **+**: **Pad does** (or Nothing), then a throw's **Effect** and **Hold** (**While held**, or **Latch**: a hit starts it and the next hit lets it go), a loop pad's **Loop**, a mute pad's **Track** or a scene pad's **Scene**. Under the list a line says what a hit does.
- **Copy**, **Paste** and **Clear** work on the pad picked. Each change is one step: **UNDO** takes it back and **SHIFT + UNDO** puts it back. The set is saved with the project.
- **On the pads** at the top right puts the set on the pads at the page shown, as **SHIFT + SAMPLER** pressed again on the Looper tab does: Play and the Looper tab play it. Tap it again to give the pads back to the kit.

Throws play on the master and ring out when you let go, as Play's tiles do. Choosing where a throw plays, when it starts and how it stops, and pads that move a macro, come later.

{{shot: pads-perform | The Pads tab showing the Perform set: page B's throws, two track mutes, a loop key and a scene, with Reverb wash on B2 picked.}}

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

## Loops in the Clips grid

Every loop you record also shows on the **Arrange** tab's Clips grid, on a **Loop track**: a column named **Loops A**, **Loops B** and so on, with a little wave and what it records under the name (**Pads**, for the pads and the synth). A loop's cell is named after it ("Loop 2") and shows the loop's wave and its length; while it plays, the wave lights up to where the loop is, and it turns red with **overdub** while you add a layer. The grid's title, **Loops 4 of 8 · memory 38 %**, says how many of the eight loops hold something and how much of the loop memory they use. A loop's cell launches, stops and plays in scenes like any clip, on the bar line:

- **A loop you record lands on a Loop track with nothing playing**, in the row of the scene playing (row 1 if none), and keeps playing. When every Loop track already plays a loop, MPFree adds one, so loops you record one after another still play together, as before.
- **One loop plays per Loop track**, as one clip plays per track; loops on different Loop tracks play together. Tap another loop's cell on the same Loop track and the one playing goes quiet on the next bar line as the new one comes in, both on the bar line exactly.
- **Scenes, Stop all and TRACK MUTE work on loops too.** A scene plays the loops in its row, and a Loop track with nothing in that row stops. Stop all stops every loop on the next bar line. A muted Loop track's loop goes quiet at once, or on the launch line when the mute waits (in Perform; see [Muting tracks and pads](playing-live.md#muting-tracks-and-pads)).
- **A loop that isn't playing keeps its place** in the song: launch it again and it comes back in step.
- **Clear** a loop, or undo its first pass, and its cell empties (Redo of the first pass puts it back). **½ length**, **× 2** and **Trim** change the cell's length with the loop's.
- **A Loop track past the first 8 tracks** is on the grid's next page: tap **›** over the grid, or **Send** > **Show in Clips** on the Looper tab.
- **The pads never play a Loop track.** Picking one keeps the pads on the track they were on; a tap on an empty Loop cell records a loop there instead (below). Drag a loop's cell to another row or another Loop track. Loops stay on Loop tracks, and clips on the other tracks.
- **Copy a loop's cell** by holding **SHIFT** as you lift it:
  - **Into another row of its own Loop track**, both rows play the same loop. A scene change from one of those rows to the other keeps the loop playing without a break.
  - **Onto an empty cell of another Loop track**, it becomes a new loop with a copy of every layer and its mix. The cell says **copying in** for a moment, then shows the wave. The copy stays quiet until you tap its cell, so you can change it on the Looper tab while the first one plays. It takes one of the eight loops and room in the loop memory, and says so when there isn't any. **UNDO** on the Looper tab, with the copy picked, takes its layers back; once they are all gone, its cell empties.

A project saved before Loop tracks opens with each of its loops on a Loop track of its own, in row 1, all playing as they did. A project saved with Loop tracks opens with nothing launched, as its clips do: tap a loop's cell, or its scene, to play it.

### Recording a loop into a cell

1. To add a Loop track, hold **SHIFT** and tap **+ Track** over the grid: it is named Loops A, Loops B and so on. **UNDO** takes it away again.
2. Tap the line under a Loop track's name to choose what its new loops record: **Pads** (the pads and the synth), **Line in** (the MPC's inputs; it turns **Line input** on) or **Resample** (the other loops, as you hear them).
3. Tap an empty cell of the Loop track. It turns red and says **Armed**, and the REC chip names it ("REC → Loops C · Drop", the row's scene name or "row 3").
4. Press **REC**: the cell says **Ready**. Press **PLAY** if the music is stopped; the loop starts on the next bar line. The cell says **REC** while it records.
5. Press **REC** again: the loop closes on the next bar line and plays in that cell. If another loop was playing on that Loop track, it stops, since one loop plays per Loop track.

With a cell armed, **OVERDUB** adds a layer to the loop that Loop track is playing. Tap the armed cell again, or arm the notes (a track's Arm pill, the Sequencer tab), to let go of it. When all eight loops hold something, or the loop memory is nearly full, the tap and REC say so: clear a loop on the Looper tab first. A new project starts with no Loop track; the first loop you record makes Loops A, as before.

### Loops in the song

Loop tracks have lanes in the **Song view** too (tap **Song ›** above the grid), with what they record under the name. A loop's block plays like a clip's:

- **Song on** brings each loop in where its block starts and takes it out where it ends, on the tick, on any Loop track where you haven't launched a cell and that isn't muted. Launch a cell and it takes that Loop track over, as a clip does; stop it and the track goes back to the song.
- **Rec song** keeps the loops you launch on the grid, each as a block from where it came in to where it went quiet, linked to its cell.
- **Scenes → song** puts the loops in each row into the song with the clips.
- A block plays the loop where it would be if it had played all along, as a launch does: a block of a two-bar loop that starts a bar into the loop's pass starts in its second bar. A loop's block stays on its own lane: drag it along the lane, but not onto another track, and no clip goes onto a Loop track's lane.

{{shot: arrange-song-loops | The Song view with Loop tracks: Loops A and Loops B play their loops' blocks under the drums, hats and bass.}}

### A loop's name

The Play page's loop strip, the Looper tab and the REC chip name a loop by its place on the grid: **Loops B · Loop 2 · Verse** is Loop 2, on Loops B, in the row of the Verse scene ("row 2" for a row with no scene name). On the Looper tab, the picked loop's header shows its track and row ("Loops B · Verse · 2 bars · 3 layers"). **Show in Clips**, under **Send**, opens the Arrange tab with the loop's cell in view.

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
- **Save loop**: keep it on the stick with its layers. Your loops are also saved with your project, every layer with its mix (see [Saving your work](saving.md)).

> [!MPC]
> On the MPC the Looper is a separate screen with one loop, which can run free of the transport and start when the input passes a threshold. MPFree has up to eight loops with a mixable lane per layer, and Capture keeps what you played even when you didn't press Record. Its loops always follow the song's bars and tempo, can record MPFree's own pads and synth as well as the inputs, and make up for the input's latency so overdubs land in time.
