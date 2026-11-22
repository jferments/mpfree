<!--
status: changing
covers: rust/mpfree-sampler rust/mpfree-sampler-ui/src/pads_tab.rs rust/mpfree-sampler-ui/src/rec_tab.rs rust/mpfree-sampler-ui/src/recall.rs rust/mpfree-sampler-ui/src/wave_tab.rs rust/mpfree-sampler-ui/src/grain_tool.rs rust/mpfree-sampler-ui/src/sound_panel.rs rust/mpfree-sampler-ui/src/warp_tool.rs rust/mpfree-sampler-ui/src/slice_tool.rs rust/mpfree-sampler-ui/src/auto_tab.rs rust/mpfree-app/src/sample_page.rs rust/mpfree-app/src/sampler.rs
checked: 2026-11-18 ec8cb82
-->

# Sampling

**Sampling** means taking a recording (a drum break, a chord from a record, your own voice) and playing it from the pads. This chapter covers the **Sample** tab: recording sounds, trimming them, fitting them to the tempo and chopping them into slices. Loading a sound onto a pad from the browser, and setting how each pad plays, is on [the Pads tab](pads.md); [Shaping how a pad plays](shaping-a-pad.md) covers a pad's layers, envelope, filter and grains.

The **Sample** tab works on one pad at a time. **Hit a pad** to pick it; its name and sound are at the top left of the page. It is one page: the recorder, the tools that trim, slice, warp and grain the sound, the wave, and the pad's settings with its **Sound** beside them. Beside the pad's name, **Send to pad** and **Chop to pads** (see [below](#send-to-pad-and-chop-to-pads)). The pad's layers, keygroups, scenes and each slice's own settings are on the Pads tab's **Edit pad** (see [Shaping how a pad plays](shaping-a-pad.md)). The four **Q-Link** cards are down the right-hand side, as on every other page: each says what its knob turns here, and you can drag a card up or down to turn it from the screen.

The Sample tab is yours whatever the pads play. While the pads play the synth, the page stays on the kit and pad it showed: record, keep takes, trim, slice, warp, make grains and put the result on pads or in your library as ever. Hitting a pad plays a note of the synth then, so it doesn't change the pad the page shows (pick another one with SHIFT + the data wheel), and a tap on the wave plays the page's pad on its own, not a synth note. **Load to pad** and **Send to pad**, which wait for a pad hit, wait for the pads to play a kit again.

{{shot: sample | The Sample tab on Wave: pad A5's name and source at the top, the recorder row, the Trim, Slice, Warp and Grain tools with the overview, the wave with its Start, Loop and End markers, and the pad's mode, gain and fades, options and sound underneath.}}

## Recording a sample

The row along the top of the Sample tab's **Wave** page is a recorder: it records from the INPUTS on the back of the MPC, or from MPFree's own sound (resampling), without leaving the page you edit on.

{{shot: sample-recording | The Wave page while REC is on: REC lit with the take's time under it, the take drawing into the wave from the left with the bar lines and its recording head, Keep last 4 bars waiting on the right, and the panel under the wave showing the Input section.}}

1. Pick what to record on the **Source** card. **Line in** is the INPUTS; turn the line input on first (**Prefs > Line input**, or the Looper tab's options), and set **REC VOL** so the Source card's bars stay out of the red. The other sources are MPFree's own sound: **Master** (everything you hear), a track (**Drums**, **Synth**, **Loop**), the delay's return, or one pad's channel.
2. Set how the take starts and ends on the **Starts** and **Ends** cards, or leave them: by default it starts when you press **REC** and ends when you press it again.
3. Tap **REC**, play, and tap **REC** again (or press **STOP**).

The take stays on the Sample page until you put it on a pad: the **Wave** page shows it as a new take ("Take 3 · new"), ready to hear and trim (see [A take on the page](#a-take-on-the-page)). To put takes straight onto a pad instead, set **Onto** to **Next empty** in the **Input** section while a take records (below): the take goes onto the next pad with no sound, starting from the pad picked, the Sample tab moves to that pad, and the line under **REC** says which pad ("next take onto A6"). **This pad** replaces the picked pad's sound. **Onto** stays as you left it for the next take. One **UNDO** takes a take off the page, or off its pad, again.

### The recorder row

- **REC** starts a take at once; the line under REC says where it goes ("next take stays here", or "next take onto A6" with **Onto** on **Next empty**), and while it records, how long it has run ("0:05.1 · stays here"). With **Starts** on **Next bar** it waits for the music's next bar line, so with the music stopped it waits for **PLAY**. It lights while the take records, and the **REC** chip at the top names the take ("● REC → Take 4 · Line in"), or the pad it goes onto ("● REC → Pad A6 · Line in"). **REC** again (on the screen or on the MPC) or **STOP** keeps the take, and the music plays on; **UNDO** throws it away. Then **REC** on the MPC records what it did before. When REC can't take, the line under it says why in orange: the line input is off ("Line in off: Looper tab, MENU"), there is no empty pad left in the bank for **Next empty**, or another take is counting in or recording ("Finish the take first: Drums records"; a long reason is cut short there, and the hint line at the bottom says it in full).
- **Source**, **Starts** and **Ends** are cards: tap a card's left half to step it back and its right half to step it on. **Source** is what you record, with its level on the two little bars: **Line in**, **Master** (the whole mix as you hear it), the pad on the page (**Flatten A5**), **Auto-sample**, a track, the delay's return or a pad's channel. **Starts** is **Now**, **On input** (when the sound crosses the threshold) or **Next bar**. **Ends** is **On STOP**, **On silence**, or **After 1 bar** on through 2, 4, 8 and 16 to **After 32 bars**.
- **Flatten A5** (the pad the page shows) records the pad as it sounds now (see [Flatten](#flatten-print-a-pad-to-a-new-sound)): **REC** plays it once, lets go after a while and records until it goes quiet, and the take lands where **Onto** says (with **Stay here**, on the page as a new take). The next two cards become **Velocity**, how hard REC hits the pad, and **Held for**, how long it holds it; the level bars show the pad. Recording starts on the pad's first sound, so **Gain**, **Threshold**, **Pre-roll** and **Monitor** are faint. The take is named after the pad ("Break 92 flat"). It is REC's take in every other way: the **REC** chip names it ("● REC → Break 92 flat · Flatten A5"); **REC** again (on the screen or on the MPC) or **STOP** stops it early and keeps what it recorded, or keeps nothing if the pad hasn't sounded yet; **UNDO** throws it away; and while another take counts in or records, REC says so. While the page shows a take, REC says "Flatten plays a pad, not a take": hit a pad first. Holding an empty pad and pressing **REC** records an input onto that pad, not a pad playing, so with **Flatten A5** as the Source it says so, from any page: "A held pad records an input: set the Sample page's Source to Line in or a point in the mix".
- **Auto-sample** makes REC open [Auto-sample](#auto-sample-a-synth-or-your-gear-onto-the-pads) over the page, to play the synth or your gear note by note onto the pads. Its own page sets how it plays, so the next two cards are faint, and its **Wave** button brings you back.
- **The take draws into the wave as it records.** Once its sound starts, the wave and the strip over it show the take so far, from the left, with a line where it has got to and the bar lines while the music plays. The wave spans the take's length when **Ends** is **After** some bars, or four bars (ten seconds with the music stopped), and zooms out to twice that whenever the take gets to the end. The bar over the page says "Take 4 · recording" and how long it has run. The wave can't be edited or zoomed while it records (a tap says so); when the take stops, the wave shows it (with **Stay here**) or the pad's sound again.
- While REC is on, the panel under the wave shows **Input** (**Source**, **Channels**, **Onto** and **Tail**, then **Gain**, **Threshold**, **Pre-roll**, **Silence for** and **Monitor**) in place of the region's settings, and the Q-Links turn **Gain**, **Threshold**, **Pre-roll** and how loud **Monitor** plays the input. **Onto** is where the take goes when it stops (**Stay here**, **Next empty** or **This pad**), so you can change your mind while it records. A take still waiting for its sound (**Starts** on **On input**) listens for the **Threshold** and **Pre-roll** you turn to, so you can turn it down if your source plays quieter than you thought.
- **Tail** is what a take with **Ends** on **After** a number of bars does after them: **Cut** ends it there, **Ring out** lets its echo or reverb ring on until it is quiet, and **Loop** makes it a loop of exactly its bars, with the ringing folded back onto its start so it goes round without a seam. **Silence for** is how long a quiet stretch has to last: for **Ends** on **On silence** (the take ends after the last sound before it), for a tail that rings out or loops (it stops ringing then), and for **Flatten A5** (it records until the pad has been quiet that long). Each is faint, saying why, when the take doesn't use it. Turn them while the take runs and it follows: a longer **Silence for** keeps it going through a pause, and **Ring out** turned on before its bars are up lets it ring. Once a take is past its bars, a change of **Tail** is for the next take, and the log says so. The region's settings and the four markers come back when the take stops, and the page shows the new take, or moves to its pad.
- **Gain** turns the line input up or down (-24 to +24 dB) on its way into the take. Watch the **Source** card's bars: the level they show, and the level the **Threshold** listens for, are after Gain. Turn it while a take records and the take follows, with no click. It changes what you record, not what you hear.
- **Monitor** lets you hear the line input through MPFree while you record. It is the same switch as the Looper's **Monitor the input**, so turning it on here turns it on there. **Q-Link 4** sets how loud you hear it: that is the **Input 1** fader on the Mix tab, and the card says **monitor off** under the level while you aren't hearing it. While Monitor is on, the input is part of the mix at that level, so [Sample Recall](#sample-recall-keep-what-just-played-from-any-page), **Keep last 4 bars** and resampling the master keep it as you hear it; the looper and the audio tracks record the input before the fader, as it comes in. Turn your source down before you switch it on, to avoid feedback.
- **Gain** and **Monitor** are for the line input. Recording a point in the mix (resampling) takes it as you hear it, so both are faint then and say why.
- **Keep last 4 bars**, the card just above the right end of the wave, keeps what just played in one tap: [Sample Recall](#sample-recall-keep-what-just-played-from-any-page)'s last 4 bars of the whole mix while the music plays (fewer, early in a song), or its last 10 seconds while it is stopped, and the card says which ("Keep last 10 s"). Its small wave on the right shows exactly what a tap keeps, as it plays, and its line names the bars ("bars 11 to 14, as a new take"). It becomes a new take where **Onto** says, as a take from **REC** would: on the page, where the wave below then shows it, or on the pad **Next empty** finds ("bars 11 to 14, onto A6"). Whole bars are cut on the bar lines and play in time at the project's tempo; a short fade goes in and out. One **UNDO** takes it back (the history calls it "Recall take", or "Recall to A6"). **Hold** the card instead to open Sample Recall over the page, to pick another length, hear it first or choose the pad. While a take is ready, counting in or recording (on the Wave page, or a clip's notes, or a loop), the card waits ("finish the take first"), as Sample Recall does, so nothing is kept under a take.

### A take on the page

A take that stays is kept with the kit, on no pad, until you put it somewhere. The **Wave** page shows it with a red swatch and its name ("Take 3 · new"), and works on it as it does on a pad: tap the wave to hear it, set its start, end and loop with the Q-Links or the markers, shape it with the region's settings, **Normalize to 0 dB** and the **Sound** controls. Each change is one **UNDO**, as on a pad.

- **Send to pad**, then hit a pad: the take goes on that pad, replacing any sound there, and leaves the page; the page shows the pad. One **UNDO** puts it back on the page.
- **SHIFT + the wheel** steps through the takes on the page, then the pads with a sound (**SHIFT + –** / **+** still zooms).
- The **Slice** tool cuts a take and plays its slices, but **Chop to pads**, the Pads tab's **Edit pad** and the Warp tool's **New sound** work on pads, so put the take on a pad first; they say so if you try.
- A take on the page is part of the kit, so you never lose one without being asked: saving the project keeps it, autosave keeps it too, and opening another project or **New project** asks first, as for any change you haven't saved. The page keeps up to 32 takes.

### Keep what just came in

The recorder is always listening while its Source is on, so you can keep what already came in without having pressed **REC**. **Keep last 4 bars** keeps what the whole mix just played. For what came in on the Source alone (the line input, say), open [Sample Recall](#sample-recall-keep-what-just-played-from-any-page) and set its **From** to the Source ("Line in"): it reads what the Source has kept (the last 1:15), with its bars, its bracket and **Audition**. It is the sampler's version of [Capture](clips.md#capture-keep-what-you-just-played): play something good, then keep it. (Sample Recall's **From** took over from the Rec page's **Skip back**.)

### Hold an empty pad and press REC: one take, from any page

You don't have to open the Sample tab to record onto a pad. **Hold a pad that has no sound and press REC**, on any page: the Sample page's recorder is armed onto that pad for one take, with its settings (its Source, Starts and Ends). The REC chip on the transport bar says so, **REC → Pad A5 · Line in**, and pages without a transport bar show it at the top. Let go of the pad; the take waits.

- It starts when the sound crosses the threshold, if the recorder's **Starts** is **On input**. Otherwise it starts with the music: press **PLAY**, or, if the music is already playing, it starts at once (**Next bar**: on the next bar line). While it records the chip reads **● REC → Pad A5 · Line in**.
- **REC** again, or **STOP**, ends it and the take lands on the pad, ready to play. STOP ends the take but leaves the music playing; press STOP again to stop.
- **UNDO** while it records throws the take away.
- The recorder runs while the line input is on (**Prefs > Line input**), whatever the Source. If it is off, the take ends at once, the log says why, and REC goes back to what it recorded before.
- Pressed before the take starts, REC or STOP calls it off. So does a tap on an Arm pill or a row of the REC chip's list, which then arms what you tapped.

It is one take only. When it ends, REC goes straight back to what it recorded before, the notes or a loop, so in the middle of a set REC never stays pointed at a pad. A pad that already has a sound is played as usual: holding it while you press REC is plain REC. It works while the pads play a kit, not while they are steps (STEP SEQ), mute keys (TRACK MUTE), 16 Levels or the loop keys, and not while another take counts in or records.

The rule to remember: **a pad + REC records from now**; SHIFT + REC keeps what you just played ([Capture](clips.md#capture-keep-what-you-just-played)).

### How a take starts and ends

The **Source**, **Starts** and **Ends** cards are on the recorder row; the rest of the recorder's settings are in the **Input** section, which the panel under the wave shows while **REC** is on. Each setting stays as you left it for the next take.

| Control | Where | What it does |
|---|---|---|
| **Source** | card, Input | What it records (above). |
| **Starts** | card | **Now**: when you tap REC. **On input**: when the sound gets louder than the **Threshold**, for catching a hit without missing its start. **Next bar**: on the next bar line, in time with the music. |
| **Ends** | card | **On STOP**: when you tap REC again or press STOP. **On silence**: when the sound has been under the threshold for the **Silence for** time. **After 1 bar** to **After 32 bars**: after that many bars. |
| **Channels** | Input | **Left + right**, **Left**, **Right** or **Mono mix** (both mixed into one). |
| **Onto** | Input | **Stay here** (the take stays on the Sample page until you put it on a pad), **Next empty** or **This pad** (above). |
| **Tail** | Input | What a take that ends after some bars does at its end. **Cut** ends on the bar line. **Ring out** keeps recording until the sound dies away (up to 20 seconds more), for a one-shot with its reverb. **Loop** makes a loop of exactly its bars, with the ringing folded onto its start, so it loops with no seam. |
| **Gain** | Input, Q-Link 1 | How loud the line input records, from -24 to +24 dB. The Source card's bars, the threshold and the take hear it; what you hear doesn't change. Faint while you resample a point in the mix. |
| **Threshold** | Input, Q-Link 2 | How loud the sound must be to start a take (and quiet, to end one on silence), from -60 to -6 dB. |
| **Pre-roll** | Input, Q-Link 3 | With Starts on On input, how much from just before the sound to keep as well (0 to 500 ms), so the attack isn't cut. |
| **Silence for** | Input | How long the quiet must last (250 ms to 4 s) to end a take on silence, a Ring out, or a Flatten. |
| **Monitor** | Input, Q-Link 4 | Hear the line input while you record (above). |
| **Velocity**, **Held for** | cards, with Flatten | How REC plays the pad (below). |

A take that starts on the bar line and lasts whole bars lands warped to the project's tempo, so it stays in time if you change the tempo later. A take can be up to three minutes long.

### Flatten: print a pad to a new sound

Set the recorder's **Source** to the pad on the page (**Flatten A5**) and tap **REC**: it records the pad as it sounds now, with its layers, envelope, filter, warp and pan in one new sound. It plays the pad once at the **Velocity** card's velocity, lets go after **Held for**, and records until the sound has been silent for the **Silence for** time. With **Onto** on **Stay here** the result stays on the page as a new take, and on **Next empty** it goes onto the next empty pad; either way the pad channel's effects are printed in. With **This pad** they are left out, because the channel will play them again. It records the **Channels** you set.

Use it to free up a busy pad, or to chop a sound you have already shaped.

> [!TIP]
> Resampling is the classic MPC trick: play the loop, the synth or the whole mix into a pad, then chop it and play it again. After a jam you liked, tap **Keep last 4 bars**: MPFree always keeps the last two minutes of what you hear, so it reaches back to before you thought of keeping it. To record what comes next instead, pick **Master** as the Source and tap **REC**. From any other page, [Sample Recall](#sample-recall-keep-what-just-played-from-any-page) does the same without leaving it.

## Sample Recall: keep what just played, from any page

MPFree always keeps the last two minutes of what you hear. When something good just happened (a fill, a lucky loop, a few bars of the whole mix), you can put it on a pad without stopping or leaving the page you're on.

1. **Hold the pad** you want it on and press **SHIFT + REC**. On the Sample tab's **Wave** page, holding **Keep last 4 bars** opens it too, and a tap on that card keeps the last 4 bars at once ([The recorder row](#the-recorder-row)).
2. Sample Recall opens over the page. While the music plays it brackets the **last 4 whole bars**; stopped, the **last 10 seconds**.
3. Press **PLAY** to hear the bracket over the music, **REC** to keep it, or **STOP** to close without keeping. The music keeps playing either way.

{{shot: sample-recall | Sample Recall opened from Play on pad A5: the last two minutes along the top with their bar numbers, bars 12 to 15 bracketed and drawn large, and Keep landing them on A6, the next free pad.}}

{{shot: sample-recall-from | Sample Recall with From on Line in: the line input's last 1:15 along the top, its bars 8 to 11 bracketed, and the REC chip saying "Pad A6 · Line in".}}

Kept, the bracket lands on the pad you held if it is empty, or else on the next empty pad in its bank (the panel says which: "Pad A6 · next free"). It is cut on the bar lines, with a short fade in and out, and with **Warp** on it plays in time at the project's tempo from then on. The pad is picked, so **SAMPLE EDIT** opens it on the Sample tab, ready to chop. It is one step on the Sample and Pads tabs: **UNDO** there takes it back (the history calls it "Recall to A6").

| Control | What it does |
|---|---|
| **From** | **Master**, the whole mix as you hear it (its last two minutes), or the recorder's Source ("Line in", or the track or channel the Sample page's **Source** card is on): what came in over its last 1:15. It opens on Master. The Source's choice is faint when there is nothing of it to read (the line input is off: "Line in off: Looper tab, MENU"), or when the Source is the master, which **Master** already is; a tap says so. If the line input goes off while it reads it, it goes back to **Master** and says so. |
| **Length** | 1, 2, 4, 8 or 16 whole bars, ending on the last bar line; or **Free**, a number of seconds ending where Sample Recall opened. |
| **Lands on** | The pad it keeps onto. Step it to another pad; on a pad that already has a sound it says "replaces its sound". |
| **Warp** | On: whole bars play in time at the project's tempo. Free always lands as played. |
| **Audition**, **Keep** | The same as PLAY and REC. |

The Q-Links turn **Length** (through the bars, then Free's seconds up to two minutes), **Ends at** (a bar further back each step, or a tenth of a second on Free), **Fade** (0 to 50 ms) and **Pad**.

> [!NOTE]
> **SHIFT + REC** without a pad held is still [Capture](clips.md#capture-keep-what-you-just-played). A held pad only counts while the pads play the kit: with **STEP SEQ**, **TRACK MUTE** or **16 LEVEL** on, or a synth on the pads, SHIFT + REC is Capture too. While a recording is ready, counting in or running, Sample Recall waits: finish the take first.

## Send to pad and Chop to pads

The two buttons at the top right of the Sample page (and of the Pads tab's Edit pad):

- **Send to pad** copies the pad you are editing, its sound and every edit, onto the next pad you hit. The button says **Hit a pad** while it waits; press it again to call it off. One UNDO takes the copy back.
- **Chop to pads** picks the **Slice** tool on the Sample page, so you can check the cuts first. Press it again (or the Put on section's own **Chop to** button) to put the slices on the pads. On Edit pad it takes you to the Sample page with the Slice tool, and chops nothing yet.

Both buttons are faint while the pad picked has no sound.

## Naming a pad and its sound

{{shot: sample-name | The text keyboard: the pad's name with the cursor at its end, Pad and Sound at the top right, and Cancel, Space and Enter along the bottom.}}

The pad's name at the top left of every Sample page has a dotted line under it: tap it to type a new one. The keyboard opens with the name as it is, so **Delete** takes off the last letter and **Clear** the whole name. **Shift** gives one capital (or the symbol shown on the key); an empty name starts with one. **#+=** turns the letter keys into symbols (# & + = ( ) [ ] ! ? @ ~ and more) until you press it again (it says **ABC** then). Turn the data wheel, or press **–** / **+**, to move the cursor and fix a letter in the middle.

{{shot: sample-name-symbols | The keyboard with #+= on: the letter keys type symbols, and the key says ABC to turn them back.}}

**Pad** and **Sound** at the top right pick which name you are typing: the pad's, or the name of the sound it plays (a recording's "Take 3", say). A sound's name is the same on every pad that plays it, and the keyboard says how many that is. Tap **Enter** (or push the wheel) to keep both names; **Cancel** leaves them as they were. One **UNDO** takes both back. A name can have up to 32 characters, and can't be empty.

## Wave: trim, loop and sound

{{shot: sample-zoomed | The Wave page zoomed in 9.3 times on the loop's start: the overview over the wave outlines the part showing, and Q-Link 3 (Loop start) is lit while it moves the marker.}}

The **Wave** page shows the sample across the screen. Set where it **starts** and **ends**, and where its **loop** starts and ends, with the four Q-Links, or rest a finger on a marker's tab and turn the data wheel: it moves a screen pixel a turn, or one frame at a time with **SHIFT** (see [Hold and turn](../getting-started/the-screen.md#hold-and-turn)). Hold **SHIFT** and press **+** to zoom in, **SHIFT + –** to zoom out (with every tool); the card beside the strip at the top says how far in you are. Tap the wave to hear it.

**The tools.** At the left of the strip over the wave are four tools: **Trim**, **Slice**, **Warp** and **Grain**. The tool you pick decides what a finger on the wave moves, what the section under it holds and what the Q-Links turn. Every tool's marks stay on the wave, but only the tool you pick has tabs along the top: the other tools' marks are grey lines with no tab, and you move them by picking their tool. **Trim** is Start, End, the loop and the fades. **Warp** is the pins that put the sound's hits on the beat, with the Warp section under the wave (see [Warp](#warp-put-the-hits-on-the-beat)). **Grain** is the grain window, where the grains start and how long they are, with the Grain section under the wave (see [Grain](shaping-a-pad.md#grain-stretch-and-smear)). **Slice** is the cuts that make slices, with the Slice and Put on sections under the wave (see [Slice](#slice-cut-across-the-pads)). While a take records the tools are grey: they wait for the take.

Under the wave, with Trim picked, the region's settings are laid out like an instrument's panel, in three sections, with the pad's **Sound** beside them:

- **Mode:** how a hit plays (**One shot**, **Gate**, **Toggle**, **Loop** or **Granular**), the **Loop** (**Off**, **Forward** or **Ping-pong**) and its **Crossfade**, which smooths a forward loop's seam. Tap a button to pick it.
- **Gain and fades:** the **Gain**, **Fade in** and **Fade out** knobs, and **Normalize to 0 dB**.
- **Options:** **Reverse**, how markers **Snap** (off, to a zero crossing, or to 16ths) and which **Channels** play.
- **Sound:** how the pad plays its sound, four controls at a time. Tap **Amp** for its envelope (**Attack**, **Decay**, **Sustain**, **Release**), **Filter** for the filter (its type, **Cutoff**, **Resonance** and **Env depth**), **Pitch** for **Tune**, **Wow & flutter**, **Vel → pitch** (harder hits play higher) and **Vintage**, or **Movement** for what the LFO moves (**LFO to**), its **LFO rate** and **LFO depth**, and the **Pitch env**. **Grain** shows the grain cloud's **Position**, **Size**, **Density** and **Pitch** (grey until the pad's Mode is Granular). The first four are Edit pad's own controls (see [Shaping how a pad plays](shaping-a-pad.md)): a change here shows there, and the other way round. Turning **Tune** here tunes the whole pad.

Drag a knob up to turn it up and down to turn it down; one drag is one **UNDO**. Tap ‹ › or − + to step a setting. Touching a control also gives it the wheel and **–** / **+**, and an outline shows which one has it; turn the wheel to pick the next control along. Or rest a finger on any control with a value and turn the wheel: it turns that control until you lift (see [Hold and turn](../getting-started/the-screen.md#hold-and-turn)). A grey control has nothing to do yet, such as Crossfade without a forward loop, or the filter's controls while the filter is off; tap it and the hint line says why.

**Normalize to 0 dB** (under the Gain and fades knobs) turns the gain so the loudest moment between Start and End plays at full level, 0 dB, and never over. Tap it, or push the wheel on it. It only sets the gain, so after moving Start or End you can tap it again for the new part; **UNDO** takes it back. A silent part is left as it is. It is quick on a recording of any length. Right after a long recording comes in, its waveform takes a moment to be worked out: tap Normalize then and the hint line says it will normalize when its waveform is ready, and the gain moves a moment later (its own **UNDO**). If you change the part before then, it is left as you set it.

MPFree finds each sound's hits, tempo and note (for Slice, Warp, the 16ths snap and Map to keys) while you work, never holding up the screen. A long sound shows as a flat line for a moment, then its wave, then its hits and tempo a little later: a minute-long sound takes a second or two on the MPC. The pages fill in as soon as each is found, and a sound already looked at is ready at once. A saved project keeps what was found beside each sound, so when you open it again its sounds are ready at once too.

**Edits you hear while it plays.** Hold a looping pad (or tap the wave in Loop mode, so it keeps sounding) and move its loop: the loop that is playing moves too. Moved behind the sound, it jumps into the new loop without a click; moved ahead, you hear it when the sound gets there. The pad's level, pan, tune, gain, filter cutoff and resonance, and a grain cloud's settings change on a sound still playing too, smoothly. The start, end, fades, reverse, mode, envelope and filter type, and a new sound on the pad, are heard from the next hit.

## Warp: put the hits on the beat

{{shot: sample-warp | The Warp tool: a break with its hits pinned to the 1/16 grid, pin 4 picked, and the Warp section under the wave.}}

**Warp** plays the sound in time with the project's tempo: **Stretch** keeps its pitch, **Repitch** speeds it up or slows it down like a record. **Warp tempo** is the tempo it was played at. Pick **Warp** in the tools over the wave to set both, and to fix a warped sound whose hits don't sit quite on the beat, such as a loop played by hand. The wave then shows the bars and beats of its warp tempo, and the hits MPFree finds are small ticks along its bottom.

- **Tap near a hit** to pin it to the nearest grid line, or tap a pinned hit to pick its pin. A pin is a numbered tab over the wave; the sound between two pins stretches to fit.
- **Drag a pin's tab** along the beats to move it. The wave is drawn as it plays, stretched between the pins, so a pinned hit sits right on its beat line and follows your finger.
- **Pin hits** pins every hit near a grid line onto it, which tightens a loose loop in one go. **Remove pin** takes the picked pin off, and **Clear pins** all of them.
- The Warp section under the wave has two parts. **Warp** holds **Warp** (**Off**, **Stretch** or **Repitch**), **Warp tempo** (tap the number to type one), the **Grid** (1/4 to 1/32), and **New sound's pitch** with **New sound**. **Pins** holds which **Hits** it finds (**Strongest**, **Most** or **All**), the picked **Pin** and its **Pin beat**, and **Pin hits**, **Remove pin** and **Clear pins**. A grey button says why when you tap it, such as "Tap a pin first".
- **New sound** makes a new sound from the region, with the warp built in. With Warp on, the new sound is at the project's tempo, its pinned hits right on their beats, so it plays without stretching. **New sound's pitch** moves it up or down as well, in semitones (–24 to +24), keeping its length and its hits sharp; with Warp off, the pitch alone is what changes. It changes nothing you hear until the new sound is made (the part's note says what a tap will make, such as "new sound: 2 st up"); to tune the pad as it plays, use **Tune** in the Sound section. Tap **Make it**: the new sound is made in the background while you work, the button shows how far it has got, and tapping it again stops it. The new sound takes the region's place; **UNDO** brings the old one back.
- **Q-Link 1** picks a pin, **2** nudges its beat, **3** fine-tunes the warp tempo and **4** scrolls along the wave. Hold **SHIFT** for finer steps. The data wheel zooms in and out round the pin picked.

Pins only work while the sound is stretched. With Warp on Repitch, the sound plays straight through. With another tool picked, the pins stay on the wave as grey lines.

**Typing a number.** A value with a dotted line under it, such as **Warp tempo**, can be typed: tap the value and a number pad opens. Tap the number and **Enter**; the pad shows the range it takes and the value now, and says so if the number is out of range. **Cancel** leaves the value as it was. The data wheel and **–** / **+** step the number too, and pushing the wheel is Enter. **UNDO** takes a typed value back.

## Slice: cut across the pads

{{shot: sample-chop | The Slice tool: the break cut into 15 slices at its hits, slice 3 picked, each slice's pad named under the wave (A6 to B4), with the Slice section (Method, its setting, the slice, Split, Merge and Clear) and the Put on section (the bank's pad map, where they go, Phrase and the Chop button) under it.}}

**Chopping** cuts a sample into slices, one per pad, so you can replay a drum break in a new order or pick out single hits. Tap **Slice** in the tools strip over the wave (or **Chop to pads** at the top right). The slices show on the wave, numbered and each in a colour of its own, and the strip under the wave names the pad each one will land on ("A6", "A7" and so on). The panel under it has two sections: **Slice**, how the sound is cut, and **Put on**, where the slices go.

1. Step **Method** with its ‹ ›: **Transient** cuts at each hit, **Beat** on the beats, **Grid** into equal pieces, **Silence** at the gaps, **Pitch** where the note changes (below), **Manual** leaves the cuts to you, and **Live** cuts where you play (below).
2. Drag the **Sensitivity** knob (or turn Q-Link 1) up for more slices, down for fewer (on Beat, Grid, Silence and Pitch the knob sets the slice length, the number of slices, the threshold or the change).
3. Tap a slice to hear it; **Slice** (or Q-Link 2) picks one too, and the wheel zooms round it. **Split** and **Merge** add or remove a cut by hand, and **Clear** takes every cut away. Drag a cut's tab, or turn Q-Link 3, to move the picked slice's start (with **SHIFT**, one frame at a time). The first marker and **END** are the pad's own start and end, so they stay put here: move them with **Trim**.
4. In the Put on section, tap the big **Chop to** button to put the slices on the pads. They go on the first empty pads after the one you chopped, running on past A16 into bank B (and so on, up to H16), so the pad itself and the sounds around it stay; the button names the pads ("Chop to A6–B4"), and **PAD BANK B** plays the ones on bank B. The section's **pad map** shows a bank's 16 pads as the MPC's pads sit: each slice's number on its pad in the slice's colour, the picked one outlined, and the pads that already have a sound raised. To start somewhere else, tap a pad on the map, or turn Q-Link 4. The map's ‹ › show the other banks' pads ("B1–B16") without changing the bank the MPC's pads play: that stays **PAD BANK**'s; the section's note says whether any pad with a sound would be replaced ("replaces A5"). Chop the same pad again and the new slices replace the ones you just made. **Put on** picks where they go: the **Pads**, **1 pad** (every slice on one pad, below), the pads and **Keys**, a **New kit** on a new track (below), an **Audio track** on the song (below), or your **Library** (below). Turn **Phrase** on to get a clip that plays the slices in time as well (for a sound with a beat, on Pads, 1 pad or New kit).

{{shot: sample-chop-on | The break's 15 slices on pads A6 to B4: their names lit under the wave, the pad map's pads raised, and the Sound section following slice 3 to pad A8.}}

**Each slice's own sound.** Once the slices are on their pads, the names under the wave light up, and the **Sound** section beside Put on follows the slice you pick to the pad that plays it ("Sound · slice 3 → A8"): its knobs set that pad, so you can give the snare a shorter release or the kick its own filter without leaving the page. Each change is one **UNDO** step. Move a cut and that slice's name goes dark: its pad no longer plays it until you chop again, and the Sound section goes back to the pad you are chopping. While every slice is on its pad, **Chop to** is grey and says so when you tap it: chopping again would change nothing but undo what you set on those pads. Change a cut, move the first pad or turn **Phrase** on, and it lights again. A slice's grains are set on its own pad: hit the pad.

**Your chop stays with the sound.** Once you change the cuts (a method, a split, a cut moved, a live take), MPFree keeps them for that sound: pick another pad or tool and come back, or open the project tomorrow, and the Slice tool shows your slices as you left them, not a fresh chop. Keeping them isn't an edit, so **UNDO** leaves them be; they go on pads only when you tap **Chop to**.

**A take on the page** can be sliced and put on pads straight away: tap its slices to hear them, then tap **Chop to** (or **Chop to pads** at the top right). A take has no pad of its own, so its slices start on the first empty pads from A1. The take leaves the page with them, since its sound now lives on those pads, and one **UNDO** brings the take back and empties the pads. **New kit** and **Library** work on a take too. Live chop needs the take on a pad first: tap **Send to pad** and hit one.

### One pad: a whole break on one pad

{{shot: sample-chop-one-pad | The Slice tool with Put on at 1 pad: the break's 15 slices are ready to go on pad A5 itself, and the button reads Slices on A5.}}

With **Put on** at **1 pad**, every slice goes on one pad instead of a pad each, so a whole break plays from one pad and one track. The button reads **Slices on A5**: the pad you chopped becomes the sliced pad (tap another pad on the map, or turn Q-Link 4, to put it there), and no other pad changes. Up to 128 slices fit on one pad.

- A **hit** on the pad plays slice 1, or the one **Slice** picks on the **Pad** section of the Pads tab's **Edit pad** ("3 of 15").
- A **step lock** picks the slice that step plays: lock **Slice** on a step and that hit plays that slice. With **Phrase** on, the clip already has a Slice lock on every hit, so it plays the break back slice by slice, and you can move, mute or re-lock any hit.
- On a **keyboard** (Edit pad's keys, or a kit playing notes), C2 plays slice 1, C#2 slice 2 and so on up, each at the sound's own pitch.

Each slice plays as it would on a pad of its own: from its cut to the next, with **Gate** or **One shot** as the pad is set (a **Loop** pad plays each slice as Gate). **UNDO** gives the pad back as it was.

### New kit: the slices on a track of their own

{{shot: sample-chop-new-kit | The Slice tool with Put on at New kit and Phrase on: the break's 15 slices are ready to go on an empty kit from A1, for a new track, and the button reads Phrase to new kit.}}

With **Put on** at **New kit**, the slices don't touch the kit you chopped. They go on an empty kit from pad A1 (up to 128 slices, running on through banks B to H), MPFree adds a new track at the end of the song, arms it and gives it that kit, named after the pad ("Break 92 chops"). The button reads **Chop to new kit**, or **Phrase to new kit** with **Phrase** on, and then the phrase goes in the new track's first slot, ready to launch. The pads play the new kit straight away, and the Sample tab shows its first slice.

- **UNDO** on the Sample tab takes the slices back off the new kit. The track stays, playing the now empty kit. **UNDO** on Arrange takes back the track and its kit together, and **REDO** brings both back.
- **Pads** and Q-Link 4 have nothing to move: a new kit's slices always start on A1.
- New kit is faint, and a tap says why, when no empty kit is left, when the song already has 128 tracks, or for the moment while the last new track is on its way.

A sliced pad shows its slices everywhere: grey lines on the **Wave** page with the other tools (pick **Slice** to move them), "15 slices" under its name on the **Pads** tab, and the **Slice** tool opens on its own slices, so you can move a cut and chop it again.

#### Each slice its own sound

{{shot: pads-edit-slices | Edit pad's Slices section on a break sliced in 8: slice 3 is picked and shaded, up 5 semitones with its own 2.4 kHz filter.}}

The **Slices** section of **Edit pad** (on the Pads tab: pick the pad, tap **Edit pad** or press **PROG EDIT**; see [Shaping how a pad plays](shaping-a-pad.md)) gives each slice of a sliced pad its own settings, on top of the pad's. Tap a slice on the picture (or turn **Slice**, Q-Link 1) to pick it, then set:

- **Pitch**: up or down, in semitones (±24), so one snare can sit a fifth higher.
- **Level**: louder or quieter, in dB.
- **Reverse**: that slice alone backwards.
- **Filter**: a low-pass of its own, in place of the pad's filter for that slice. **Off** (all the way up) uses the pad's.
- **Fade out**: a fade at its end, to cut off a tail that runs into the next hit. **Region's** keeps the pad's own fade.
- **Plays to**: **Its end** (the next cut), or **Sound end**, so the slice plays on through the rest of the break.

Q-Links 2 to 4 turn the picked slice's pitch, level and filter. The **Slices** chip says how many slices have settings of their own ("8 · 1 set"). Each change is one **UNDO** step. Chopping the pad again onto itself keeps the settings of the slices that are still there, by number.

### Audio track: the slices on the song

{{shot: sample-chop-audio | The Slice tool with Put on at Audio track: the break's 15 slices ready to go on an audio track one after another, the pad map grey, and the button reads To audio track.}}

With **Put on** at **Audio track**, the slices go on the song instead of on pads, as the MPC's **Audio Track from Samples** puts them. Tap **To audio track**: the sound goes on the first empty audio track, from the start of the bar the playhead is in, each slice a region of its own, one after the other, at the sound's own speed. The hint line says where it went ("Break 92's 15 slices on audio 1 from bar 5"). A long sound, or one recorded at another rate, takes a moment to copy out: the hint line says "Copying Break 92 out for an audio track" meanwhile, and you can keep working. Then, on Arrange's **Song** view, move, erase, split or turn up any slice as you would any region (see [Editing regions](audio-tracks.md#editing-regions)).

- The sound goes on as it was recorded: the pad's level, tuning and filter stay on the pad. The pads and the page don't change, so you can put it on again at another bar.
- The first empty track can be one whose lane isn't showing yet: its lane shows, as **+ Audio** would add it. When all 8 audio tracks have something on them, the slices go on the first track with nothing from that bar on. If there is none, nothing goes on and the hint line says why.
- The Song view's **Undo** takes all the slices off again in one go.
- The sound is copied for the track, which on a long recording takes memory. If the MPC is short of it, nothing goes on and the hint line says how much it needed (see [When memory runs short](pads.md)).
- Audio tracks play when the song plays (song mode on). With song mode off, the hint line reminds you.
- The audio tracks share one memory (see [Timing, memory and the mixer](audio-tracks.md#timing-memory-and-the-mixer)). A sound longer than what is free doesn't go on, and the hint line says so.
- The pad map and Q-Link 4 have no pad to move, and **Phrase** is grey: it is for pads, and on an audio track the slices already play one after another. They play at the sound's own speed, so they keep to the bars when the sound's tempo is the song's.
- A take on the page goes on an audio track the same way, and stays on the page.

### Pitch: a slice a note

{{shot: sample-chop-pitch | Pitch chop: a sung line cut into a slice a note, at each change of a semitone or more and after its rest.}}

**Pitch** cuts a melody into its notes: a sung line, a bass part or a lead, a slice for each note. It follows the sound's pitch and cuts where the note moves, and where a note starts after a rest.

- The knob is the **Change**: how far the note has to move to count, in semitones, from **1 semi** (every note) up to an **Octave** (only big leaps, and notes after a rest). Turn it up when a slide or a wobbly note makes too many slices.
- Vibrato and slides don't cut; notes shorter than 50 ms (a scoop into a note) don't count.
- Drums and noise have no pitch to follow, so Pitch makes few cuts on them: use Transient there.
- With **Snap** on, each cut moves to the nearest zero crossing, so a slice starts without a click.

The first time you pick Pitch on a sound, MPFree listens through it in the background; on a long sound the slices show a moment later.

### Live: chop by playing

{{shot: sample-chop-live | Live chop: the break playing, with a cut dropped at each pad hit so far.}}

**Live** chops the way you hear it: the sound plays, and every pad you hit drops a cut at that moment.

1. Pick **Live** as the **Method**. The **Split** button becomes **Start**.
2. Tap **Start**. The sound plays from its start, and any cuts already there are cleared.
3. Hit any pad in time with the sound wherever a slice should begin. While live chop runs, the pads drop cuts instead of playing their own sounds.
4. The take ends when the sound ends, or when you tap **Stop**; picking another tool, pad or page stops it too. Then tap **Chop to** as usual, or **Start** again for a new take.

**Pull to hits** (where **Sensitivity** is for the other methods) moves each cut onto the nearest drum hit within that many milliseconds, so a slightly early or late pad still cuts right on the hit; **Off** keeps each cut where you played it. With **Snap** on **Zero crossing**, a cut with no hit near it moves to the nearest point where the wave crosses zero, so its slice starts without a click. Nothing reaches the pads until you tap **Chop to**, so a take you don't like costs nothing: tap **Start** again (the page keeps your last take for the sound). Live plays a pad, so a take kept on the page goes on a pad first.

> [!TIP]
> Live chop is quickest with a sound you know well: hit the pads where you would clap along, then fix any stray cut with **Split** and **Merge**.

### Library: saving the pad and its slices

With **Put on** at **Library**, the button reads **Save** (**Save 15 slices**, or **Save the pad** when there are no cuts) and, instead of filling pads, it saves the pad in your library, as files in a folder of its own under **Samples › Exports** in the browser:

- **Break 92.wav**: the pad's whole sound. Its loop and slice points are inside it too, where most samplers and audio editors find them.
- **Break 92.mpfree**: the pad's settings, a small text file beside the sound: its trim, loop, warp, envelope, filter, layers and slices.
- **Break 92 01.wav**, **Break 92 02.wav** and so on: each slice as a file of its own, to use anywhere.

Load **Break 92.wav** back, on this MPC or another, and the pad comes back exactly as you exported it, slices and all, as long as **Break 92.mpfree** is beside it. Without it, the sound loads on its own. Export the same pad again and it goes into **Break 92 2**, so nothing is written over.

**Library** is offered when MPFree has its library on the stick; without one there is nowhere to put them.

## Auto-sample: a synth or your gear onto the pads

{{shot: sample-auto | Auto-sample on the Synth tab's Poly Pluck: a note every 5 semitones from C2 to A5, in three velocity layers, onto pads A6 to A15.}}

**Auto-sample** plays an instrument note by note, records every note and makes a playable instrument of it on the kit's pads: one pad a note, each covering the keys around it, with velocity layers and round robins if you ask for them. Open it with **Auto-sample** as the recorder's **Source** on the Wave page, then **REC**. In the list, set the **Lowest note** and **Highest note**, how far apart the sampled notes are (**Every**), the velocity **Layers**, **Round robins**, how long each note is held (**Hold**) and how long its tail may ring (**Tail**). Then tap **Start**; **Wave** goes back to the Wave page. The bar says how many takes it makes, how long it takes and how much memory, before anything starts. **Cancel** stops and keeps the notes done, and one UNDO takes the whole instrument off.

The **Source** row says what plays:

- **The Synth tab's patch.** MPFree plays a copy of it, much faster than real time, so a whole instrument takes seconds.
- **External gear**, on a MIDI channel from 1 to 16. MPFree plays your synth through **MIDI OUT** and records it from the line input, in real time. Connect the synth's audio out to the line input, set it to the channel you picked and turn the line input on. MPFree first plays the middle note twice to hear how loud the synth is and how late it answers, so no note starts late or cut off. If the line input stops or you switch the recorder's Source to another source while it runs, it stops, lets go of the note and keeps the notes done.

The notes go out of the 5-pin **MIDI OUT** jack. MIDI out is built but not yet checked on an MPC One, so it only sends while **Send MIDI and CV out** is on in [Prefs](../reference/prefs.md) (see [Playing other gear](midi-and-cv.md)). Until then the Source row offers the Synth tab's patch only, and with the gear picked the bar says there is no MIDI out for it. Every note it plays is let go when it ends or you cancel it, and the MIDI Monitor shows them as sent by the auto-sample.

> [!MPC]
> The MPC's Sampler and Sample Edit modes are one page in MPFree, the Sample tab, always on the pad you last hit; its Program Edit (Track Edit) is the Pads tab's Edit pad. **SAMPLER** and **SAMPLE EDIT** open the Sample tab, and **PROG EDIT** the Pads tab, where it opens Edit pad on the pad picked and, pressed again, goes back to the pads. **UNDO** takes back any edit on the Sample and Pads tabs.
