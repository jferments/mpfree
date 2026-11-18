<!--
status: changing
covers: rust/mpfree-sampler rust/mpfree-sampler-ui/src/pads_tab.rs rust/mpfree-sampler-ui/src/rec_tab.rs rust/mpfree-sampler-ui/src/recall.rs rust/mpfree-sampler-ui/src/wave_tab.rs rust/mpfree-sampler-ui/src/sound_panel.rs rust/mpfree-sampler-ui/src/warp_tab.rs rust/mpfree-sampler-ui/src/chop_tab.rs rust/mpfree-sampler-ui/src/auto_tab.rs rust/mpfree-app/src/sample_page.rs rust/mpfree-app/src/sampler.rs
checked: 2026-11-18 ec8cb82
-->

# Sampling

**Sampling** means taking a recording (a drum break, a chord from a record, your own voice) and playing it from the pads. This chapter covers getting sounds onto pads: loading them, recording them, trimming them, fitting them to the tempo and chopping them into slices. [Shaping how a pad plays](shaping-a-pad.md) then covers what each pad does when you hit it.

Two tabs share the work:

- The **Pads** tab, next to Play, shows the whole bank of 16 pads beside the settings of the pad picked: which sound is on which pad, and how it plays. **Browse for** at its top right opens the browser to find a sound for that pad, and **Kit** beside it opens the list of kits, to put another kit on the selected track.
- The **Sample** tab works on one pad at a time. **Hit a pad** to pick it; its name and sound are at the top left of the page. Along the top are a page for each job: **Wave**, **Warp**, **Chop**, **Play**, **Grain** and **Rec**. Beside them, **Send to pad** and **Chop to pads** work from any page (see [below](#send-to-pad-and-chop-to-pads)). The four **Q-Link** cards are down the right-hand side, as on every other page: each says what its knob turns here, and you can drag a card up or down to turn it from the screen.

## The Pads tab: sounds onto pads

On the left of the Pads tab are the 16 pads of the bank, laid out as on the MPC (pad 1 at the bottom left), each with its name and how it plays. Over them, the kit's banks **A** to **H** each show a dot for every pad that has a sound; tap a bank to put it on the pads, as **PAD BANK** does. The **Pads of** row across the top names the tracks that play kits, each with its kit, the selected track outlined: tap a track to select it, and the pads play its kit. **+ Kit track** after them adds a track with a new empty kit of its own and selects it, ready for sounds (one **UNDO** takes back the track and its kit, and **REDO** brings both back). **Perform pads** at the right end shows the project's [Perform set](looping.md#setting-up-the-perform-set-on-the-pads-tab) in the kit's place: the pads that throw effects, play the loop keys, mute tracks and launch scenes. Tap a pad to pick it, and tap it again to hear it; a pad you hit lights up. Under the pads, **Copy**, **Paste** and **Clear** move pads around, and **Map to keys** turns the bank into a keyboard instrument (see [Playing a kit on keys](shaping-a-pad.md#playing-a-kit-on-keys)).

{{shot: pads-pad | The Pads tab: the bank on the left, the picked pad's settings in the middle, the sounds in the kit on the right, and Kit and Browse for at the top right.}}

Beside the pads are the settings of the pad picked:

- its **sound**;
- how it plays (**Mode**): **One shot** plays to the end, **Gate** only while you hold the pad, **Toggle** starts and stops with each hit, **Loop** repeats its loop region, and **Granular** plays it as grains;
- how many notes can ring at once (**Polyphony**). Kit pads, and a sound you load onto an empty pad, start at 4: hit a pad again (or let Note Repeat do it) and its fifth hit stops its oldest, while other pads' sounds ring on. MPFree plays up to 64 sounds at once: plain drum hits (One shot, no filter, the envelope open, at the sound's own pitch) can use all 64, and sounds with a filter, an envelope, a loop, grains or a tune up to 24 of them. When too many sounds play at once, MPFree stops the oldest sound of the pad that has the most playing first;
- a **mute group**: pads in the same group cut each other off, like an open and a closed hi-hat;
- its **level**, **pan** and **tune**. Hold **SHIFT** and touch one of them to give it a knob on your MIDI controller (see [MIDI Learn](midi-and-cv.md#midi-learn));
- **Effects**: the effects on the pad's own mixer channel (**+ FX** when it has none). Tap it, or push the wheel on it, to open the pad's chain in the Mix tab, and **‹** brings you back (see [Effects](effects.md#adding-an-effect-the-chain-editor)). A pad without a channel of its own (any but kit 1's A1 to A16) gets one when you add an effect, and gives it back once it has none (see [Effects on one pad](shaping-a-pad.md#effects-on-one-pad-or-on-the-whole-kit)).

Q-Links 1 to 4 turn level, pan, tune and voices. **Put on** places the sound picked on the right onto the pad, the whole of it; the hint line says when the pad plays all of it already.

### What a pad plays: Sample, Loop, Instrument, MIDI out

Over the pad's settings, four tabs set what a hit on the pad does. Each switch is one step: **UNDO** takes it back.

- **Sample** ("its sound") plays the pad's sound with the settings above. Every pad starts as one.
- **Loop** ("in time") plays the pad's sound as a loop at the project's tempo: a hit starts it and the next hit stops it. While the song plays, both wait for the next launch line, as a tapped clip does (the Record panel's **Launch**, one bar unless you change it), so a loop always starts and stops on the beat however early you hit it. While a hit waits, its pad blinks amber, on the MPC and on the screen (an amber outline), as a clip waiting to launch does, until it sounds. Turning **PAD BANK** before the line still starts the pad you hit. A hit made during a count-in waits for the first launch line once recording starts, not a line in the count-in. With the song stopped, or Launch at once, a hit acts at once. **STOP** forgets a hit that is still waiting: it never sounds. This works from the pads, the screen's pads and a keyboard's keys on the kit, and a clip you record takes the hit where the loop started. MPFree loops the whole sound and warps it to the tempo. A sound that has no tempo of its own yet is taken to be at the project's tempo, so set **Tempo** to the tempo it was played at: a step is a tenth of a BPM, and the row shows how many bars that makes ("92.0 BPM · 2 bars"). Its other settings are the sound, mute group, level, pan, tune and effects, and Q-Links 1 to 4 turn level, pan, tune and tempo. A pad needs a sound to loop: on an empty pad the hint line says to Browse for one first. Back on **Sample**, the pad keeps its loop and its **Mode**, for you to change there.
- **Instrument** ("a synth voice") plays one note of a synth when you hit the pad: pick the **Engine** (one of the [nine synths](synths.md#the-nine-synths), Poly at first) and the **Note** (C4 at first). The pad plays that engine's sound as the Synth tab has it now, at the pad's velocity, and the note lasts as long as you hold the pad (or as the step's length on the Sequencer). Each Instrument pad gets a synth of its own with that sound, so it plays through the pad's own channel on the Mix tab, where a sample on the pad would (kit 1's A1 to A16 on Pad Mix, another kit's pads on its track's channel), with that channel's fader, effects and sends. Change the sound on the Synth tab and the pads that play it follow within a moment. Up to 16 pads can have a synth of their own; past that a pad plays through the Synth channel instead, the screen tells you once, and the line under that pad's settings says so in the warning colour. A pad just made an Instrument plays through the Synth channel for the moment its own synth takes to start. Q-Links 1 and 2 turn the note and the engine.
- **MIDI out** ("to your gear") sends a note to a synth or drum machine plugged into MPFree: pick the **Port**, the **Channel** and the **Note**. A new MIDI out pad starts on the note and channel a kit on that port would send for it: what the port's [device profile](midi-and-cv.md#device-profiles) gives the pad, else the pad's own note (A1 is 37, as the MPC sends it; see [A kit on a drum machine](midi-and-cv.md#a-kit-on-a-drum-machine)) on channel 10, the drum channel on most gear. A hit sends the note at the pad's velocity, and letting go ends it. Q-Links 1 to 3 turn the note, the channel and the port. MIDI out pads play from tracks 1 to 16.

Under the settings a line says what a hit does: "A hit sends note 49 on channel 10 to Volca Kick", "A hit plays the Synth tab's Poly sound at C4, through this pad's channel on Mix". When it would play or send nothing, the line is in the warning colour and says why: the port isn't plugged in, MIDI out is off, or there is no engine of that name. An Instrument or MIDI out pad needs no sound, and an empty pad you make one takes the engine's or the port's name. Muting its track, or soloing another, ends its notes.

The pads show their type in the corner (**LOOP**, **SYNTH**, **MIDI**) and what they play under their name ("ch 10 · 49", "Poly C4", "2 bars"). On Play's rows the type shows under the pad's name too.

{{shot: pads-types | A pad of each type: A5 loops Break 92, A11 plays the Poly synth at C4, and A13, picked, sends note 49 on channel 10 to a Volca Kick.}}

On a track whose **Plays on** is a port (see [A kit on a drum machine](midi-and-cv.md#a-kit-on-a-drum-machine)), every pad goes to that port as a note, whatever its type.

### Finding sounds: Browse for

**Browse for A5** at the top right of the Pads tab (it names the pad picked) opens the browser over the page. **BROWSE**, and the folder at the right end of the top bar, open the same browser from any page. **BROWSE**, the folder or a tab closes it again.

{{shot: browser-samples | The browser opened from Browse for A5: Samples / Drums, with Kick 01 picked, its waveform, the kit's pads with A5 picked, and Load on A5 and Next pad hit under them.}}

Pick a place on the left (**Samples**, **Kits** or **Recordings**, say, or **Recent** and **Favourites**). Tap a folder to open it, and tap a sound to pick it and see its length, format and waveform. Then:

- **Load on A5** puts it on the pad picked (so does a second tap on it). Hit another pad and the button names that pad instead: browse, hit a pad, load, hit the next pad, load again.
- **Next pad hit** puts it on the next pad you hit.
- **The kit's pads**, under the sound, show what each pad of the kit holds, a bank at a time, with the pad picked outlined. Put a finger on one to pick it, as a hit on the MPC's pad does, and hear what is on it until you lift your finger; **Load on** then names it. The map opens on the bank of the pad picked, and **‹** and **›** beside its heading show the kit's other banks.
- **Audition**, or **PLAY**, lets you hear it first, at the level Q-Link 3 sets.
- The **star** at the right end of each row adds it to Favourites; tap it again to take it out.

**16-bit sounds take half the memory.** A 16-bit WAV or AIFF, and anything you trim, slice or cut from one, is kept as 16 bits: about 10 MB a minute of stereo, half what a 24-bit or 32-bit float sound takes. It sounds exactly the same.

**Sounds at other rates.** MPFree plays at 44.1 kHz. A sound recorded at another rate (48 kHz from a video or a phone, 96 kHz from a studio, 22 kHz from an old sampler) is converted to 44.1 kHz as it loads, and the Console says so ("converted from 48 kHz to 44.1 kHz"). Nothing you can hear changes: it plays at its own pitch, and as lightly as a sound recorded at 44.1 kHz. Whatever its settings file places on it (trim, loop, fades, warp and slices) moves with it to the same moments, and the file itself stays as it was. Kits and projects convert their sounds as they open, too. When memory is short, a sound stays at its own rate; it still plays at its pitch, and the Console lists it.

**Whole kits.** A kit (green icon) loads all at once: pick it and tap it again, push the data wheel, or tap **Load on**, which names the selected track (**Load on Drums**). It takes the place of the kit that track and the pads play now, with every pad's sound and settings. When another track with a clip plays that kit too, the loaded kit goes into a new kit of its own for the selected track instead, so the other tracks sound as they did; if all eight new kits are in use, MPFree says so and loads nothing. **New track with this kit**, beside Load, keeps the kit the pads play as it is: it adds a track, as **+ Track** on Arrange does, and the kit loads into a new kit for that track alone. The new track is the one selected, so the pads play the kit at once. One **UNDO** on Arrange takes back the new track and its kit together, and **REDO** brings both back. Once you do something else, so REDO can't bring them back, the kit shows again among the project's kits, played by no track, so nothing you loaded is lost. To see the sounds inside a kit one by one, press **+**. **Save kit**, at the top of the **Kits** place, keeps the kit the pads play as a kit of its own, so you can load it in another project; save it before you load another kit over it if you want to keep your changes. MPFree puts its own drum kits in **Kits > MPFree** the first time it starts with your library.

{{shot: browser-kits | The browser in Kits, with Boom bap picked: Load on Drums (in place of the kit the selected track, Drums, plays) and New track with this kit.}}

**MPFree's free kits.** Next to MPFree's own kits, made by its drum synth, are three kits of real recorded drums: **Big Rusty Drums** (a big acoustic kit) and **Swirly Drums** (a second acoustic kit, with cowbell, splash and china), both recorded by Karoryfer Samples, and **VCSL Percussion** (claves, cajon, tambourine, shaker, claps, bongos, congas, cowbell, agogo, triangle and more), from Versilian Studios' community sample library. Their makers gave the recordings to the public domain (CC0), so anything you make with them is yours, with no credit or fee owed. MPFree took one hit per pad from each library and brought every hit to about the same loudness, so set each pad's level to taste. Each hit's source is listed in the file `free-kits-credits.txt` that comes with the USB creator's download.

The data wheel, its push, **–** and **+** move through the browser too; see [Buttons and controls](../reference/controls.md#the-browser). The browser needs the data partition on MPFree's stick; without it, it opens empty, and the Pads tab still works.

**When memory runs short.** The MPC has 2 GB of memory, and running out would stop the sound, so MPFree always keeps 256 MB free. A sound that would leave less isn't loaded, and the hint line says so with how much it needed. The same goes for the sounds of a project you open (it opens without the ones that don't fit, and the Console lists them), for Auto-sample (it doesn't start), and for a warped pad's stretched copy (the pad speeds up or slows down with the tempo, like a record, instead of stretching). To make room, open a smaller project or a new one, or lower **Loop memory** or **Audio track memory** in [Prefs](../reference/prefs.md) for the next start.

## A kit for each track

Each track plays a kit of its own, as each track plays a program on the MPC, and every track's kit sounds at the same time: a drum kit on track 1, chopped vocals on track 2 and one-shot stabs on track 3 all play together in the song.

- **The pads play the selected track's kit.** Select another track (on Arrange, or with the track chip) and the pads play its kit. A track's clips play its kit whichever track is selected.
- **To change a track's kit,** select the track and pick the kit in the list of kits (tap **Kit** at the top of the Pads tab, or **PADS PLAY** on Play's transport bar), or with **SHIFT** + the data wheel on the Play page. Only these change what a track plays: opening a page never does. The synth comes after the kits (see [Put a synth on the pads](synths.md#put-a-synth-on-the-pads)), and the list's last row, **New kit**, gives the selected track an empty kit of its own (Kit 8, say): fill its pads by loading sounds, recording, or sending pads to it. Up to eight new kits are free in a session; one shows in the list once a track plays it or it has sounds, and the project keeps it with its sounds.
- **A kit has 128 pads,** 16 in each of the banks A to H. **PAD BANK** puts another bank on the pads: **B** plays pads B1 to B16, and **SHIFT** (or a second press) reaches E to H. The bank stays when you select another track, and **PADS PLAY** on Play ("Drums · Boom Bap · B") and the Pads tab's banks say which one is on the pads. Whatever you play on bank B records onto those pads, and the Sample tab edits the pad you hit last on any bank. Load or record a sound onto an empty pad of any bank, or send a pad there with **Send to pad**.
- **Every track starts on kit 1,** so a project saved before tracks had a kit each plays as it did.
- **In the mixer,** kit 1 plays through the 16 pad channels and the **Drums** strip, as before. Another kit plays into the strip of the first track that plays it (**Track 2**, say), with its own fader, effects and sends; tracks that play the same kit share it. Put effects for the whole kit on the track's strip; a pad that needs effects of its own gets a channel when you add one (see [Effects on one pad](shaping-a-pad.md#effects-on-one-pad-or-on-the-whole-kit)). Pad mutes are kit 1's A1 to A16 alone for now, so mute the track rather than a pad: on another kit's track the Play page's pad speakers and **TRACK MUTE**'s pad mutes say so and mute nothing.
- **The project keeps** which kit each track plays.

## Recording a sample

The **Rec** page records onto a pad: from the INPUTS on the back of the MPC, or from MPFree's own sound (resampling).

{{shot: sample-rec | The Rec page recording the line input onto pad A6: a take that started on the bar line is two bars in, and the blue span is what Skip back would keep.}}

1. Pick what to record with **Source**, in the panel's **Input** section. **Line in** is the INPUTS; turn the line input on first (**Prefs > Line input**, or the Looper tab's options), and set **REC VOL** so the meters stay out of the red. The other sources are MPFree's own sound: **Master** (everything you hear), a track (**Drums**, **Synth**, **Loop**), the delay's return, or one pad's channel.
2. Set how the take starts and ends (below), or leave them: by default it starts when you press **Record** and ends when you press **Stop**.
3. Tap **Record**, play, and tap **Stop**.

The take stays on the Sample page until you put it on a pad: the **Wave** page shows it as a new take ("Take 3 · new"), ready to hear and trim (see [A take on the page](#a-take-on-the-page)). The title says so ("Record: the take stays here"). To put each take straight onto a pad instead, set **Onto** to **Next empty**: the take goes onto the next pad with no sound, starting from the pad picked, the Sample tab moves to that pad, and the title says which pad it will land on ("Record onto pad A6"). **This pad** replaces the picked pad's sound. One **UNDO** takes a take off the page, or off its pad, again.

The picture shows the source's level on two meters, and the last 30 seconds of it scrolling past with the bar lines, so you can see what came in.

### Recording from the Wave page

You can also record without leaving the **Wave** page: the row over the wave is a recorder, with the Rec page's own settings (a change on one shows on the other).

{{shot: sample-recording | The Wave page while REC is on: REC lit with the take's time under it, Keep last 4 bars waiting on the right, and the panel under the wave showing the Input section.}}

- **REC** starts a take at once; the line under REC says where it goes ("next take stays here", or "next take onto A6" with **Onto** on **Next empty**), and while it records, how long it has run ("0:05.1 · stays here"). With **Starts** on **Next bar** it waits for the music's next bar line, so with the music stopped it waits for **PLAY**. It lights while the take records, and the **REC** chip at the top names the take ("● REC → Take 4 · Line in"), or the pad it goes onto ("● REC → Pad A6 · Line in"). **REC** again (on the screen or on the MPC) or **STOP** keeps the take, and the music plays on; **UNDO** throws it away. Then **REC** on the MPC records what it did before. When REC can't take, the line under it says why in orange: the line input is off ("Line in off: Looper tab, MENU"), there is no empty pad left in the bank for **Next empty**, or another take is counting in or recording ("Finish the take first: Drums records"; a long reason is cut short there, and the hint line at the bottom says it in full).
- **Source**, **Starts** and **Ends** are cards: tap a card's left half to step it back and its right half to step it on. **Source** is what you record, with its level on the two little bars: **Line in**, **Master** (the whole mix as you hear it), the pad on the page (**Flatten A5**), **Auto-sample**, a track, the delay's return or a pad's channel. **Starts** is **Now**, **On input** (when the sound crosses the threshold) or **Next bar**. **Ends** is **On STOP**, **On silence** or **After** a number of bars.
- **Flatten A5** (the pad the page shows) records the pad as it sounds now, as [Flatten](#flatten-print-a-pad-to-a-new-sound) does: **REC** plays it once, lets go after a while and records until it goes quiet, and the take lands where **Onto** says (with **Stay here**, on the page as a new take). The next two cards become **Velocity**, how hard REC hits the pad, and **Held for**, how long it holds it; the level bars show the pad. Recording starts on the pad's first sound, so **Gain**, **Threshold**, **Pre-roll** and **Monitor** are faint. The take is named after the pad ("Break 92 flat"). It is REC's take in every other way: the **REC** chip names it ("● REC → Break 92 flat · Flatten A5"); **REC** again (on the screen or on the MPC) or **STOP** stops it early and keeps what it recorded, or keeps nothing if the pad hasn't sounded yet; **UNDO** throws it away; and while another take counts in or records, REC says so. While the page shows a take, REC says "Flatten plays a pad, not a take": hit a pad first. Holding an empty pad and pressing **REC** records an input onto that pad, not a pad playing, so with **Flatten A5** as the Source it says so, from any page: "A held pad records an input: set the Sample page's Source to Line in or a point in the mix".
- **Auto-sample** makes REC open [Auto-sample](#auto-sample-a-synth-or-your-gear-onto-the-pads) over the page, to play the synth or your gear note by note onto the pads. Its own page sets how it plays, so the next two cards are faint, and its **Wave** button brings you back.
- While REC is on, the panel under the wave shows **Input** (**Source**, **Channels** and **Onto**, then **Gain**, **Threshold**, **Pre-roll** and **Monitor**) in place of the region's settings, and the Q-Links turn **Gain**, **Threshold**, **Pre-roll** and how loud **Monitor** plays the input. **Onto** is where the take goes when it stops (**Stay here**, **Next empty** or **This pad**), so you can change your mind while it records. A take still waiting for its sound (**Starts** on **On input**) listens for the **Threshold** and **Pre-roll** you turn to, so you can turn it down if your source plays quieter than you thought. The region's settings and the four markers come back when the take stops, and the page shows the new take, or moves to its pad.
- **Gain** turns the line input up or down (-24 to +24 dB) on its way into the take. Watch the **Source** card's bars: the level they show, and the level the **Threshold** listens for, are after Gain. Turn it while a take records and the take follows, with no click. It changes what you record, not what you hear.
- **Monitor** lets you hear the line input through MPFree while you record. It is the same switch as the Looper's **Monitor the input**, so turning it on here turns it on there. **Q-Link 4** sets how loud you hear it: that is the **Input 1** fader on the Mix tab, and the card says **monitor off** under the level while you aren't hearing it. While Monitor is on, the input is part of the mix at that level, so [Sample Recall](#sample-recall-keep-what-just-played-from-any-page), **Keep last 4 bars** and resampling the master keep it as you hear it; the looper and the audio tracks record the input before the fader, as it comes in. Turn your source down before you switch it on, to avoid feedback.
- **Gain** and **Monitor** are for the line input. Recording a point in the mix (resampling) takes it as you hear it, so both are faint then and say why.
- **Keep last 4 bars**, at the end of the row, keeps what just played in one tap: [Sample Recall](#sample-recall-keep-what-just-played-from-any-page)'s last 4 bars of the whole mix while the music plays (fewer, early in a song), or its last 10 seconds while it is stopped, and the card says which ("Keep last 10 s"). It becomes a new take where **Onto** says, as a take from **REC** would: on the page ("what just played, as a new take"), or on the pad **Next empty** finds ("what just played, onto A6"). Whole bars are cut on the bar lines and play in time at the project's tempo; a short fade goes in and out. One **UNDO** takes it back (the history calls it "Recall take", or "Recall to A6"). **Hold** the card instead to open Sample Recall over the page, to pick another length, hear it first or choose the pad. While a take is ready, counting in or recording (on the Wave page, or a clip's notes, or a loop), the card waits ("finish the take first"), as Sample Recall does, so nothing is kept under a take.

### A take on the page

A take that stays is kept with the kit, on no pad, until you put it somewhere. The **Wave** page shows it with a red swatch and its name ("Take 3 · new"), and works on it as it does on a pad: tap the wave to hear it, set its start, end and loop with the Q-Links or the markers, shape it with the region's settings, **Normalize to 0 dB** and the **Sound** controls. Each change is one **UNDO**, as on a pad.

- **Send to pad**, then hit a pad: the take goes on that pad, replacing any sound there, and leaves the page; the page shows the pad. One **UNDO** puts it back on the page.
- **SHIFT + the wheel** steps through the takes on the page, then the pads with a sound (**SHIFT + –** / **+** still zooms).
- **Chop to pads**, the other pages and **Process** work on pads, so put the take on a pad first; they say so if you try.
- A take on the page is part of the kit, so you never lose one without being asked: saving the project keeps it, autosave keeps it too, and opening another project or **New project** asks first, as for any change you haven't saved. The page keeps up to 32 takes.

### Skip back: keep what you just played

The Rec page is always listening while its source is on. **Skip back** keeps what has already come in, without your having pressed Record: the last 10 seconds to begin with, or whatever the selector beside the **Skip back** button says (5 seconds to a minute, or 1 to 8 whole bars cut on the bar lines). The blue span on the picture shows what it would keep. It is the sampler's version of [Capture](clips.md#capture-keep-what-you-just-played): play something good, then keep it.

[Sample Recall](#sample-recall-keep-what-just-played-from-any-page) does the same from any page: set its **From** to the recorder's Source ("Line in") and it reads what the Source has kept, with its bars, its bracket and **Audition**. Skip back's place is there from now on: the Rec page is going.

### Hold an empty pad and press REC: one take, from any page

You don't have to open the Rec page to record onto a pad. **Hold a pad that has no sound and press REC**, on any page: the Rec page's recorder is armed onto that pad for one take, with the Rec page's settings (its Source, Start and End). The REC chip on the transport bar says so, **REC → Pad A5 · Line in**, and pages without a transport bar show it at the top. Let go of the pad; the take waits.

- It starts when the sound crosses the threshold, if the Rec page's **Start** is **Threshold**. Otherwise it starts with the music: press **PLAY**, or, if the music is already playing, it starts at once (**Next bar**: on the next bar line). While it records the chip reads **● REC → Pad A5 · Line in**.
- **REC** again, or **STOP**, ends it and the take lands on the pad, ready to play. STOP ends the take but leaves the music playing; press STOP again to stop.
- **UNDO** while it records throws the take away.
- The recorder runs while the line input is on (**Prefs > Line input**), whatever the Source. If it is off, the take ends at once, the log says why, and REC goes back to what it recorded before.
- Pressed before the take starts, REC or STOP calls it off. So does a tap on an Arm pill or a row of the REC chip's list, which then arms what you tapped.

It is one take only. When it ends, REC goes straight back to what it recorded before, the notes or a loop, so in the middle of a set REC never stays pointed at a pad. A pad that already has a sound is played as usual: holding it while you press REC is plain REC. It works while the pads play a kit, not while they are steps (STEP SEQ), mute keys (TRACK MUTE), 16 Levels or the loop keys, and not while another take counts in or records.

The rule to remember: **a pad + REC records from now**; SHIFT + REC keeps what you just played ([Capture](clips.md#capture-keep-what-you-just-played)).

### How a take starts and ends

The panel under the picture sets up the take, in five sections: **Input** (Source and Channels), **Start** (with Gain, Threshold and Pre-roll), **End**, **Take** (Record, Skip back and Onto) and **Resample** (Flatten and Auto-sample). Tap a choice's button to pick it, drag a knob up or down, or tap a selector's arrows; the data wheel and **–** / **+** change the control you touched last, and a finger resting on a control lets the wheel turn it.

| Control | What it does |
|---|---|
| **Source** | What it records (above). |
| **Channels** | **Left + right**, **Left**, **Right** or **Mono mix** (both mixed into one). |
| **Gain** | How loud the line input records, from -24 to +24 dB. The meters, the threshold and the take hear it; what you hear doesn't change. Faint while you resample a point in the mix. |
| **Start** | **Now**: when you tap Record. **Threshold**: when the sound gets louder than the Threshold knob, for catching a hit without missing its start. **Next bar**: on the next bar line, in time with the music. |
| **Threshold** | How loud the sound must be to start a take (and quiet, to end one on silence), from -60 to -6 dB. |
| **Pre-roll** | With Start on Threshold, how much from just before the sound to keep as well (0 to 500 ms), so the attack isn't cut. |
| **End** | **Stop**: when you tap Stop. **Silence**: when the sound has been under the threshold for the Silence time. **Length**: after a number of bars. |
| **Length** | With End on Length: 1 to 32 bars. |
| **Tail** | What a take of a Length does at its end. **Cut** ends on the bar line. **Ring out** keeps recording until the sound dies away (up to 20 seconds more), for a one-shot with its reverb. **Loop** makes a loop of exactly its bars, with the ringing folded onto its start, so it loops with no seam. |
| **Silence for** | How long the quiet must last (250 ms to 4 s) to end a take on silence, or a Ring out. |
| **Skip back** | How far Skip back reaches. |
| **Onto** | **Stay here** (the take stays on the Sample page until you put it on a pad), **Next empty** or **This pad** (above). Skip back keeps what it reaches the same way. |
| **Flatten velocity**, **Flatten hold** | How Flatten plays the pad (below). |

A take that starts on the bar line and lasts whole bars lands warped to the project's tempo, so it stays in time if you change the tempo later. A take can be up to three minutes long.

### Flatten: print a pad to a new sound

**Flatten** records the picked pad as it sounds now, with its layers, envelope, filter, warp and pan in one new sound. It plays the pad once at the **Flatten velocity**, lets go after the **Flatten hold**, and records until the sound has been silent for the Silence time. Onto the next empty pad (where it goes with **Onto** on **Stay here** too), the pad channel's effects are printed in too; onto the pad itself they are left out, because the channel will play them again. It records the **Channels** you set.

On the Wave page, the recorder does the same with **Flatten A5** as its **Source** (above), and there **Stay here** keeps the result on the page as a new take, with the effects printed in.

Use it to free up a busy pad, or to chop a sound you have already shaped.

> [!TIP]
> Resampling is the classic MPC trick: play the loop, the synth or the whole mix into a pad, then chop it and play it again. Pick **Master** as the Source and use **Skip back** after a jam you liked. MPFree always keeps the last two minutes of what you hear, so Skip back on **Master** reaches back to before you picked it. From any other page, [Sample Recall](#sample-recall-keep-what-just-played-from-any-page) does the same without leaving it.

## Sample Recall: keep what just played, from any page

MPFree always keeps the last two minutes of what you hear. When something good just happened (a fill, a lucky loop, a few bars of the whole mix), you can put it on a pad without stopping or leaving the page you're on.

1. **Hold the pad** you want it on and press **SHIFT + REC**. On the Sample tab's **Wave** page, holding **Keep last 4 bars** opens it too, and a tap on that card keeps the last 4 bars at once ([Recording from the Wave page](#recording-from-the-wave-page)).
2. Sample Recall opens over the page. While the music plays it brackets the **last 4 whole bars**; stopped, the **last 10 seconds**.
3. Press **PLAY** to hear the bracket over the music, **REC** to keep it, or **STOP** to close without keeping. The music keeps playing either way.

{{shot: sample-recall | Sample Recall opened from Play on pad A5: the last two minutes along the top with their bar numbers, bars 12 to 15 bracketed and drawn large, and Keep landing them on A6, the next free pad.}}

{{shot: sample-recall-from | Sample Recall with From on Line in: the line input's last 1:15 along the top, its bars 8 to 11 bracketed, and the REC chip saying "Pad A6 · Line in".}}

Kept, the bracket lands on the pad you held if it is empty, or else on the next empty pad in its bank (the panel says which: "Pad A6 · next free"). It is cut on the bar lines, with a short fade in and out, and with **Warp** on it plays in time at the project's tempo from then on. The pad is picked, so **SAMPLE EDIT** opens it on the Sample tab, ready to chop. It is one step on the Sample and Pads tabs: **UNDO** there takes it back (the history calls it "Recall to A6").

| Control | What it does |
|---|---|
| **From** | **Master**, the whole mix as you hear it (its last two minutes), or the recorder's Source ("Line in", or the track or channel the Sample page's **Source** card is on): what came in over its last 1:15, as **Skip back** keeps it. It opens on Master. The Source's choice is faint when there is nothing of it to read (the line input is off: "Line in off: Looper tab, MENU"), or when the Source is the master, which **Master** already is; a tap says so. If the line input goes off while it reads it, it goes back to **Master** and says so. |
| **Length** | 1, 2, 4, 8 or 16 whole bars, ending on the last bar line; or **Free**, a number of seconds ending where Sample Recall opened. |
| **Lands on** | The pad it keeps onto. Step it to another pad; on a pad that already has a sound it says "replaces its sound". |
| **Warp** | On: whole bars play in time at the project's tempo. Free always lands as played. |
| **Audition**, **Keep** | The same as PLAY and REC. |

The Q-Links turn **Length** (through the bars, then Free's seconds up to two minutes), **Ends at** (a bar further back each step, or a tenth of a second on Free), **Fade** (0 to 50 ms) and **Pad**.

> [!NOTE]
> **SHIFT + REC** without a pad held is still [Capture](clips.md#capture-keep-what-you-just-played). A held pad only counts while the pads play the kit: with **STEP SEQ**, **TRACK MUTE** or **16 LEVEL** on, or a synth on the pads, SHIFT + REC is Capture too. While a recording is ready, counting in or running, Sample Recall waits: finish the take first.

## Send to pad and Chop to pads

The two buttons at the top right of every Sample page:

- **Send to pad** copies the pad you are editing, its sound and every edit, onto the next pad you hit. The button says **Hit a pad** while it waits; press it again to call it off. One UNDO takes the copy back.
- **Chop to pads** opens the Chop page, so you can check the cuts first. Press it again there (or the Chop page's own button) to put the slices on the pads.

Both buttons are faint while the pad picked has no sound.

## Naming a pad and its sound

{{shot: sample-name | The text keyboard: the pad's name with the cursor at its end, Pad and Sound at the top right, and Cancel, Space and Enter along the bottom.}}

The pad's name at the top left of every Sample page has a dotted line under it: tap it to type a new one. The keyboard opens with the name as it is, so **Delete** takes off the last letter and **Clear** the whole name. **Shift** gives one capital (or the symbol shown on the key); an empty name starts with one. **#+=** turns the letter keys into symbols (# & + = ( ) [ ] ! ? @ ~ and more) until you press it again (it says **ABC** then). Turn the data wheel, or press **–** / **+**, to move the cursor and fix a letter in the middle.

{{shot: sample-name-symbols | The keyboard with #+= on: the letter keys type symbols, and the key says ABC to turn them back.}}

**Pad** and **Sound** at the top right pick which name you are typing: the pad's, or the name of the sound it plays (a recording's "Take 3", say). A sound's name is the same on every pad that plays it, and the keyboard says how many that is. Tap **Enter** (or push the wheel) to keep both names; **Cancel** leaves them as they were. One **UNDO** takes both back. A name can have up to 32 characters, and can't be empty.

## Wave: trim, loop and sound

{{shot: sample | The Wave page: the recorder row at the top (REC, Source, Starts and Ends), the tools (Trim, Slice, Warp and Grain) beside the whole sample under it, the zoomed region across the page, the region's knobs, buttons and switches under it in three sections with the pad's Sound beside them, and the four Q-Links (Start, End, Loop start, Loop end) down the right. Send to pad and Chop to pads are at the top right.}}

The **Wave** page shows the sample across the screen. Set where it **starts** and **ends**, and where its **loop** starts and ends, with the four Q-Links, or rest a finger on a marker's tab and turn the data wheel: it moves a screen pixel a turn, or one frame at a time with **SHIFT** (next build; see [Hold and turn](../getting-started/the-screen.md#hold-and-turn)). Hold **SHIFT** and press **+** to zoom in, **SHIFT + –** to zoom out (the Warp and Chop pages too); the card beside the strip at the top says how far in you are. Tap the wave to hear it.

**The tools.** At the left of the strip over the wave are four tools: **Trim**, **Slice**, **Warp** and **Grain**. The tool you pick decides what a finger on the wave moves and what the Q-Links turn. **Trim** is what this page does now: Start, End, the loop and the fades. The other three are moving onto this page one at a time; until each one has, its button opens its own page (**Slice** opens [Chop](#chop-slice-across-the-pads), **Warp** opens [Warp](#warp-put-the-hits-on-the-beat), **Grain** opens [Grain](shaping-a-pad.md#grain-stretch-and-smear)), and **Wave** at the top right brings you back. While a take records the tools are grey: they wait for the take.

Under the wave, the region's settings are laid out like an instrument's panel, in three sections, with the pad's **Sound** beside them:

- **Mode:** how a hit plays (**One shot**, **Gate**, **Toggle**, **Loop** or **Granular**), the **Loop** (**Off**, **Forward** or **Ping-pong**) and its **Crossfade**, which smooths a forward loop's seam. Tap a button to pick it.
- **Gain and fades:** the **Gain**, **Fade in** and **Fade out** knobs, and **Normalize to 0 dB**.
- **Options:** **Reverse**, how markers **Snap** (off, to a zero crossing, or to 16ths) and which **Channels** play.
- **Sound:** how the pad plays its sound, four controls at a time. Tap **Amp** for its envelope (**Attack**, **Decay**, **Sustain**, **Release**), **Filter** for the filter (its type, **Cutoff**, **Resonance** and **Env depth**), **Pitch** for **Tune**, **Wow & flutter**, **Vel → pitch** (harder hits play higher) and **Vintage**, or **Movement** for what the LFO moves (**LFO to**), its **LFO rate** and **LFO depth**, and the **Pitch env**. These are the Play page's own controls (see [Shaping how a pad plays](shaping-a-pad.md)): a change here shows there, and the other way round. Turning **Tune** here tunes the whole pad.

Drag a knob up to turn it up and down to turn it down; one drag is one **UNDO**. Tap ‹ › or − + to step a setting. Touching a control also gives it the wheel and **–** / **+**, and an outline shows which one has it; turn the wheel to pick the next control along. Or rest a finger on any control with a value and turn the wheel: it turns that control until you lift (see [Hold and turn](../getting-started/the-screen.md#hold-and-turn)). A grey control has nothing to do yet, such as Crossfade without a forward loop, or the filter's controls while the filter is off; tap it and the hint line says why.

**Normalize to 0 dB** (under the Gain and fades knobs) turns the gain so the loudest moment between Start and End plays at full level, 0 dB, and never over. Tap it, or push the wheel on it. It only sets the gain, so after moving Start or End you can tap it again for the new part; **UNDO** takes it back. A silent part is left as it is.

MPFree finds each sound's hits, tempo and note (for Chop, Warp, the 16ths snap and Map to keys) while you work, never holding up the screen. A long sound shows as a flat line for a moment, then its wave, then its hits and tempo a little later: a minute-long sound takes a second or two on the MPC. The pages fill in as soon as each is found, and a sound already looked at is ready at once. A saved project keeps what was found beside each sound, so when you open it again its sounds are ready at once too.

**Edits you hear while it plays.** Hold a looping pad (or tap the wave in Loop mode, so it keeps sounding) and move its loop: the loop that is playing moves too. Moved behind the sound, it jumps into the new loop without a click; moved ahead, you hear it when the sound gets there. The pad's level, pan, tune, gain, filter cutoff and resonance, and a grain cloud's settings change on a sound still playing too, smoothly. The start, end, fades, reverse, mode, envelope and filter type, and a new sound on the pad, are heard from the next hit.

## Warp: put the hits on the beat

{{shot: sample-warp | The Warp page: a break at 92 BPM with 14 hits pinned to the 1/16 grid, pin 4 picked, and its settings in the panel under it.}}

**Warp** plays the sound in time with the project's tempo: **Stretch** keeps its pitch, **Repitch** speeds it up or slows it down like a record. **Warp tempo** is the tempo it was played at. Both are on the **Warp** page, which is also for a warped sound whose hits don't sit quite on the beat, such as a loop played by hand. It draws the sound against the bars and beats of its warp tempo, with the hits MPFree finds marked along the bottom.

- **Tap near a hit** to pin it to the nearest grid line. A pin is a numbered flag; the sound between two pins stretches to fit.
- **Drag a pin's handle** to move it along the grid.
- **Pin hits** pins every hit near a grid line onto it, which tightens a loose loop in one go. **Remove pin** takes the picked pin off, and **Clear pins** all of them.
- The panel under the sound holds **Warp** (**Off**, **Stretch** or **Repitch**) and **Warp tempo** (tap the number to type one) with the **Grid** (1/4 to 1/32); which **Hits** it finds (**Strongest**, **Most** or **All**) with **Pin hits**; the picked **Pin** and its **Pin beat** with **Remove pin** and **Clear pins**; and **Shift** and **Process**. A grey button says why when you tap it, such as "Tap a pin first".
- **Process** makes a new sound from the region. With Warp on, the new sound is at the project's tempo, its pinned hits right on their beats, so it plays without stretching. **Shift** first moves it up or down, in semitones (–24 to +24), keeping its length and its hits sharp; with Warp off, Shift alone is what changes. Tap **New sound**: it is made in the background while you work, the button shows how far it has got, and tapping it again stops it. The new sound takes the region's place; **UNDO** brings the old one back.
- The Q-Links pick a pin, nudge its beat, fine-tune the warp tempo and scroll. Hold **SHIFT** for finer steps.

Pins only work while the sound is stretched. With Warp on Repitch, the sound plays straight through.

**Typing a number.** A value with a dotted line under it, such as **Warp tempo**, can be typed: tap the value and a number pad opens. Tap the number and **Enter**; the pad shows the range it takes and the value now, and says so if the number is out of range. **Cancel** leaves the value as it was. The data wheel and **–** / **+** step the number too, and pushing the wheel is Enter. **UNDO** takes a typed value back.

## Chop: slice across the pads

{{shot: sample-chop | The Chop page: the break cut into 15 slices at its hits, with the method, the slice, the pads and the Chop button in the panel under it, ready to put on pads A6 to B4.}}

**Chopping** cuts a sample into slices, one per pad, so you can replay a drum break in a new order or pick out single hits.

1. Under the wave, tap a **Method**: **Transient** cuts at each hit, **Beat** on the beats, **Grid** into equal pieces, **Silence** at the gaps, **Pitch** where the note changes (below), **Manual** leaves the cuts to you, and **Live** cuts where you play (below).
2. Drag the **Sensitivity** knob up for more slices, down for fewer (on Beat, Grid, Silence and Pitch the knob sets the slice length, the number of slices, the threshold or the change).
3. Tap a slice to hear it. **Split** and **Merge** add or remove a cut by hand; drag a cut's tab, or turn the Q-Links, to move it. The first marker and **END** are the pad's own start and end, so they stay put here: move them on the Wave page.
4. Tap the big **Chop to** button to put the slices on the pads. They go on the first empty pads after the one you chopped, running on past A16 into bank B (and so on, up to H16), so the pad itself and the sounds around it stay; the button names the pads ("Chop to A6–B4"), and **PAD BANK B** plays the ones on bank B. To start somewhere else, turn **Pads** (or Q-Link 4); the list says which pads already have a sound. Chop the same pad again and the new slices replace the ones you just made. **Put on** picks where they go: the **Pads**, **1 pad** (every slice on one pad, below), the pads and **Keys**, a **New kit** on a new track (below), or **Files**. Turn **Phrase** on to get a clip that plays the slices in time as well (for a sound with a beat, on Pads, 1 pad or New kit).

**Your chop stays with the sound.** Once you change the cuts (a method, a split, a cut moved, a live take), MPFree keeps them for that sound: pick another pad and come back, or open the project tomorrow, and the Chop page shows your slices as you left them, not a fresh chop. Keeping them isn't an edit, so **UNDO** leaves them be; they go on pads only when you tap **Chop to**.

### One pad: a whole break on one pad

{{shot: sample-chop-one-pad | The Chop page with Put on at 1 pad: the break's 15 slices are ready to go on pad A5 itself, and the button reads Slices on A5.}}

With **Put on** at **1 pad**, every slice goes on one pad instead of a pad each, so a whole break plays from one pad and one track. The button reads **Slices on A5**: the pad you chopped becomes the sliced pad (turn **Pads** or Q-Link 4 to put it on another pad), and no other pad changes. Up to 128 slices fit on one pad.

- A **hit** on the pad plays slice 1, or the one **Slice** picks on the Play page's **Pad** section ("3 of 15").
- A **step lock** picks the slice that step plays: lock **Slice** on a step and that hit plays that slice. With **Phrase** on, the clip already has a Slice lock on every hit, so it plays the break back slice by slice, and you can move, mute or re-lock any hit.
- On a **keyboard** (the Play page's keys, or a kit playing notes), C2 plays slice 1, C#2 slice 2 and so on up, each at the sound's own pitch.

Each slice plays as it would on a pad of its own: from its cut to the next, with **Gate** or **One shot** as the pad is set (a **Loop** pad plays each slice as Gate). **UNDO** gives the pad back as it was.

### New kit: the slices on a track of their own

{{shot: sample-chop-new-kit | The Chop page with Put on at New kit and Phrase on: the break's 15 slices are ready to go on an empty kit from A1, for a new track, and the button reads Phrase to new kit.}}

With **Put on** at **New kit**, the slices don't touch the kit you chopped. They go on an empty kit from pad A1 (up to 128 slices, running on through banks B to H), MPFree adds a new track at the end of the song, arms it and gives it that kit, named after the pad ("Break 92 chops"). The button reads **Chop to new kit**, or **Phrase to new kit** with **Phrase** on, and then the phrase goes in the new track's first slot, ready to launch. The pads play the new kit straight away, and the Sample tab shows its first slice.

- **UNDO** on the Sample tab takes the slices back off the new kit. The track stays, playing the now empty kit. **UNDO** on Arrange takes back the track and its kit together, and **REDO** brings both back.
- **Pads** and Q-Link 4 have nothing to move: a new kit's slices always start on A1.
- New kit is faint, and a tap says why, when no empty kit is left, when the song already has 128 tracks, or for the moment while the last new track is on its way.

A sliced pad shows its slices everywhere: numbered lines on the **Wave** page (move them on the Chop page), "15 slices" under its name on the **Pads** tab, and the **Chop** page opens on its own slices, so you can move a cut and chop it again.

#### Each slice its own sound

{{shot: sample-play-slices | The Play page's Slices section on a break sliced in 8: slice 3 is picked and shaded, up 5 semitones with its own 2.4 kHz filter.}}

The Play page's **Slices** section gives each slice of a sliced pad its own settings, on top of the pad's. Tap a slice on the picture (or turn **Slice**, Q-Link 1) to pick it, then set:

- **Pitch**: up or down, in semitones (±24), so one snare can sit a fifth higher.
- **Level**: louder or quieter, in dB.
- **Reverse**: that slice alone backwards.
- **Filter**: a low-pass of its own, in place of the pad's filter for that slice. **Off** (all the way up) uses the pad's.
- **Fade out**: a fade at its end, to cut off a tail that runs into the next hit. **Region's** keeps the pad's own fade.
- **Plays to**: **Its end** (the next cut), or **Sound end**, so the slice plays on through the rest of the break.

Q-Links 2 to 4 turn the picked slice's pitch, level and filter. The **Slices** chip says how many slices have settings of their own ("8 · 1 set"). Each change is one **UNDO** step. Chopping the pad again onto itself keeps the settings of the slices that are still there, by number.

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
4. The take ends when the sound ends, or when you tap **Stop**. Then tap **Chop to** as usual, or **Start** again for a new take.

**Pull to hits** (where **Sensitivity** is for the other methods) moves each cut onto the nearest drum hit within that many milliseconds, so a slightly early or late pad still cuts right on the hit; **Off** keeps each cut where you played it. With **Snap** on **Zero crossing**, a cut with no hit near it moves to the nearest point where the wave crosses zero, so its slice starts without a click. Nothing reaches the pads until you tap **Chop to**, so a take you don't like costs nothing: tap **Start** again (the page keeps your last take for the sound).

> [!TIP]
> Live chop is quickest with a sound you know well: hit the pads where you would clap along, then fix any stray cut with **Split** and **Merge**.

### Exporting a pad as files

With **Put on** at **Files**, the button reads **Export** (**Export 15 slices**, or **Export the pad** when there are no cuts) and, instead of filling pads, it saves the pad as files in a folder of its own under **Samples › Exports** in the browser:

- **Break 92.wav**: the pad's whole sound. Its loop and slice points are inside it too, where most samplers and audio editors find them.
- **Break 92.mpfree**: the pad's settings, a small text file beside the sound: its trim, loop, warp, envelope, filter, layers and slices.
- **Break 92 01.wav**, **Break 92 02.wav** and so on: each slice as a file of its own, to use anywhere.

Load **Break 92.wav** back, on this MPC or another, and the pad comes back exactly as you exported it, slices and all, as long as **Break 92.mpfree** is beside it. Without it, the sound loads on its own. Export the same pad again and it goes into **Break 92 2**, so nothing is written over.

**Files** is offered when MPFree has its library on the stick; without one there is nowhere to put them.

## Auto-sample: a synth or your gear onto the pads

{{shot: sample-auto | Auto-sample on the Synth tab's Poly Pluck: a note every 5 semitones from C2 to A5, in three velocity layers, onto pads A6 to A15.}}

**Auto-sample** plays an instrument note by note, records every note and makes a playable instrument of it on the kit's pads: one pad a note, each covering the keys around it, with velocity layers and round robins if you ask for them. Open it with **Auto-sample** as the recorder's **Source** on the Wave page, then **REC** (its **Wave** button brings you back), or with the **Auto-sample** button on the **Rec** page. In the list, set the **Lowest note** and **Highest note**, how far apart the sampled notes are (**Every**), the velocity **Layers**, **Round robins**, how long each note is held (**Hold**) and how long its tail may ring (**Tail**). Then tap **Start**; **Rec** (or **Wave**) goes back to the page you came from. The bar says how many takes it makes, how long it takes and how much memory, before anything starts. **Cancel** stops and keeps the notes done, and one UNDO takes the whole instrument off.

The **Source** row says what plays:

- **The Synth tab's patch.** MPFree plays a copy of it, much faster than real time, so a whole instrument takes seconds.
- **External gear**, on a MIDI channel from 1 to 16. MPFree plays your synth through **MIDI OUT** and records it from the line input, in real time. Connect the synth's audio out to the line input, set it to the channel you picked and turn the line input on. MPFree first plays the middle note twice to hear how loud the synth is and how late it answers, so no note starts late or cut off. If the line input stops or you switch the Rec page to another source while it runs, it stops, lets go of the note and keeps the notes done.

The notes go out of the 5-pin **MIDI OUT** jack. MIDI out is built but not yet checked on an MPC One, so it only sends while **Send MIDI and CV out** is on in [Prefs](../reference/prefs.md) (see [Playing other gear](midi-and-cv.md)). Until then the Source row offers the Synth tab's patch only, and with the gear picked the bar says there is no MIDI out for it. Every note it plays is let go when it ends or you cancel it, and the MIDI Monitor shows them as sent by the auto-sample.

> [!MPC]
> The MPC's Sampler, Sample Edit and Program Edit modes are all on one tab in MPFree, always on the pad you last hit. **SAMPLER** opens the Sample tab, **SAMPLE EDIT** its Wave page, and **PROG EDIT** the Pads tab's Pad view. **UNDO** takes back any edit on the Sample and Pads tabs.
