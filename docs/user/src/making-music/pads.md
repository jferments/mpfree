<!--
status: changing
covers: rust/mpfree-sampler-ui/src/pads_tab.rs rust/mpfree-app/src/sample_page.rs rust/mpfree-app/src/sampler.rs rust/mpfree-app/src/perform_tab.rs rust/mpfree-app/src/perform_pads.rs rust/mpfree-app/src/pad_synths.rs rust/mpfree-app/src/kit.rs rust/mpfree-browser-ui
checked: 2026-11-19 a6fb29b2
-->

# The Pads tab

The **Pads** tab is where you decide what each pad plays. It shows a whole bank of 16 pads beside the settings of the pad you pick, and the sounds in the kit beside that. Use it to put sounds on pads, set how each pad plays (one shot, looped, gated), turn a pad into a synth voice or a note for your outside gear, give each track a kit of its own, and set up the Perform set of action pads.

The [Sample tab](sampling.md) works on one pad's sound in detail (recording, trimming, chopping), and [Shaping how a pad plays](shaping-a-pad.md) covers a pad's layers, envelope, filter and effects.

{{shot: pads-pad | The Pads tab: the tracks that play kits along the top, bank A of the Breaks kit on the left with A5 picked, A5's settings in the middle with Sample, Loop, Instrument and MIDI out over them, and the sounds in the kit on the right.}}

## Put a sound on a pad

1. Open the **Pads** tab and **hit the pad** you want to fill (or tap it on the screen). Its name is at the top left, such as **Pad A5**.
2. Tap **Browse for A5** at the top right. The browser opens over the page.
3. Find a sound: pick **Samples** on the left, open a folder, and tap a sound to hear it (or press **PLAY**).
4. Tap **Load on A5**. The sound is on the pad. Hit another pad and the button names that one, so you can fill a kit pad by pad without closing the browser.
5. Press **BROWSE**, or tap the folder or a tab, to close the browser.

A sound already in the kit can go on another pad without the browser: pick the pad, tap the sound in **Sounds in the kit** on the right, and tap **Put on A5**. The hint line says when the pad plays all of it already.

## The page

**Along the top:** the pad picked and its sound, with how many of the bank's pads are in use and how many sounds the kit has. On the right, **Kit** opens the list of kits, to put another kit on the selected track, and **Browse for** opens the browser for the pad picked.

**The Pads of row:** the tracks that play kits, each with its kit's name, the selected track outlined. Tap a track to select it, and the pads play its kit. **+ Kit track** adds a track with a new empty kit of its own and selects it, ready for sounds (one **UNDO** takes back the track and its kit, and **SHIFT + UNDO** brings both back). **Perform pads** at the right end shows the project's [Perform set](#the-perform-set-on-the-pads-tab) in the kit's place.

**The pads, on the left:** the 16 pads of the bank, laid out as on the MPC (pad 1 at the bottom left), each with its name, its sound and how it plays. Over them, the kit's banks **A** to **H** each show a dot for every pad that has a sound; tap a bank to show it and put it on the pads, as **PAD BANK** does. Tap a pad to pick it, and tap it again to hear it; a pad you hit lights up and is picked.

**Under the pads:** **Copy** copies the pad picked, **Paste A5** puts the copy on the pad picked, **Clear** empties it, and **Map to keys** turns the bank into one keyboard instrument (see [Playing a kit on keys](shaping-a-pad.md#playing-a-kit-on-keys)). Each is one **UNDO** step.

**In the middle:** what the pad plays (four tabs: Sample, Loop, Instrument, MIDI out) and its settings.

**On the right:** **Sounds in the kit**, every sound the kit holds, with its length and how many pads play it, and **Put on** under them.

## A pad's settings

For a **Sample** pad (every pad starts as one), the settings are:

- its **sound**;
- how it plays (**Mode**): **One shot** plays to the end, **Gate** only while you hold the pad, **Toggle** starts and stops with each hit, **Loop** repeats its loop region, and **Granular** plays it as grains;
- how many notes can ring at once (**Polyphony**). Kit pads, and a sound you load onto an empty pad, start at 4: hit a pad again (or let Note Repeat do it) and its fifth hit stops its oldest, while other pads' sounds ring on. MPFree plays up to 64 sounds at once: plain drum hits (One shot, no filter, the envelope open, at the sound's own pitch) can use all 64, and sounds with a filter, an envelope, a loop, grains or a tune up to 24 of them. When too many sounds play at once, MPFree stops the oldest sound of the pad that has the most playing first;
- a **mute group**: pads in the same group cut each other off, like an open and a closed hi-hat;
- its **level**, **pan** and **tune**. Hold **SHIFT** and touch one of them to give it a knob on your MIDI controller (see [MIDI Learn](midi-and-cv.md#midi-learn));
- **Effects**: the effects on the pad's own mixer channel (**+ FX** when it has none). Tap it, or push the wheel on it, to open the pad's chain in the Mix tab, and **‹** brings you back (see [Effects](effects.md#adding-an-effect-the-chain-editor)). A pad without a channel of its own (any but kit 1's A1 to A16) gets one when you add an effect, and gives it back once it has none (see [Effects on one pad](shaping-a-pad.md#effects-on-one-pad-or-on-the-whole-kit)).

Change a row as in any list (the wheel picks it, **–** and **+** change it, or push the wheel and turn), or rest a finger on it and turn the wheel. **Q-Links 1 to 4** turn level, pan, tune and voices. **SHIFT** + the data wheel steps through the pads that have a sound.

## What a pad plays: Sample, Loop, Instrument, MIDI out

Over the pad's settings, four tabs set what a hit on the pad does. Each switch is one step: **UNDO** takes it back.

- **Sample** ("its sound") plays the pad's sound with the settings above. Every pad starts as one.
- **Loop** ("in time") plays the pad's sound as a loop at the project's tempo: a hit starts it and the next hit stops it. While the song plays, both wait for the next launch line, as a tapped clip does (the Record panel's **Launch**, one bar unless you change it), so a loop always starts and stops on the beat however early you hit it. While a hit waits, its pad blinks amber, on the MPC and on the screen (an amber outline), as a clip waiting to launch does, until it sounds. Turning **PAD BANK** before the line still starts the pad you hit. A hit made during a count-in waits for the first launch line once recording starts, not a line in the count-in. With the song stopped, or Launch at once, a hit acts at once. **STOP** forgets a hit that is still waiting: it never sounds. This works from the pads, the screen's pads and a keyboard's keys on the kit, and a clip you record takes the hit where the loop started. MPFree loops the whole sound and warps it to the tempo. A sound that has no tempo of its own yet is taken to be at the project's tempo, so set **Tempo** to the tempo it was played at: a step is a tenth of a BPM, and the row shows how many bars that makes ("92.0 BPM · 2 bars"). Its other settings are the sound, mute group, level, pan, tune and effects, and Q-Links 1 to 4 turn level, pan, tune and tempo. A pad needs a sound to loop: on an empty pad the hint line says to Browse for one first. Back on **Sample**, the pad keeps its loop and its **Mode**, for you to change there.
- **Instrument** ("a synth voice") plays one note of a synth when you hit the pad: pick the **Engine** (one of the [synths](synths.md#the-twelve-synths), Poly at first) and the **Note** (C4 at first). The pad plays that engine's sound as the Synth tab has it now, at the pad's velocity, and the note lasts as long as you hold the pad (or as the step's length on the Sequencer). Each Instrument pad gets a synth of its own with that sound, so it plays through the pad's own channel on the Mix tab, where a sample on the pad would (kit 1's A1 to A16 on Pad Mix, another kit's pads on its track's channel), with that channel's fader, effects and sends. Change the sound on the Synth tab and the pads that play it follow within a moment. Up to 16 pads can have a synth of their own; past that a pad plays through the Synth channel instead, the screen tells you once, and the line under that pad's settings says so in the warning colour. A pad just made an Instrument plays through the Synth channel for the moment its own synth takes to start. Q-Links 1 and 2 turn the note and the engine.
- **MIDI out** ("to your gear") sends a note to a synth or drum machine plugged into MPFree: pick the **Port**, the **Channel** and the **Note**. A new MIDI out pad starts on the note and channel a kit on that port would send for it: what the port's [device profile](midi-and-cv.md#device-profiles) gives the pad, else the pad's own note (A1 is 37, as the MPC sends it; see [A kit on a drum machine](midi-and-cv.md#a-kit-on-a-drum-machine)) on channel 10, the drum channel on most gear. A hit sends the note at the pad's velocity, and letting go ends it. Q-Links 1 to 3 turn the note, the channel and the port. MIDI out pads play from any track.

Under the settings a line says what a hit does: "A hit sends note 49 on channel 10 to Volca Kick", "A hit plays the Synth tab's Poly sound at C4, through this pad's channel on Mix". When it would play or send nothing, the line is in the warning colour and says why: the port isn't plugged in, MIDI out is off, or there is no engine of that name. An Instrument or MIDI out pad needs no sound, and an empty pad you make one takes the engine's or the port's name. Muting its track, or soloing another, ends its notes.

The pads show their type in the corner (**LOOP**, **SYNTH**, **MIDI**) and what they play under their name ("ch 10 · 49", "Poly C4", "2 bars"). On Play's rows the type shows under the pad's name too.

{{shot: pads-types | A pad of each type: A5 loops Break 92, A11 plays the Poly synth at C4, and A13, picked, sends note 49 on channel 10 to a Volca Kick.}}

On a track whose **Plays on** is a port (see [A kit on a drum machine](midi-and-cv.md#a-kit-on-a-drum-machine)), every pad goes to that port as a note, whatever its type.

## Finding sounds: Browse for

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

- **The pads play the selected track's kit.** Select another track (in the **Pads of** row here, with **‹ Track 1 ›** over Play's rows, or by tapping its name on Arrange) and the pads play its kit. A track's clips play its kit whichever track is selected.
- **To change a track's kit,** select the track and pick the kit in the list of kits (tap **Kit** at the top of the Pads tab, or **PADS PLAY** on Play's transport bar), or with **SHIFT** + the data wheel on the Play page. Only these change what a track plays: opening a page never does. The synth comes after the kits (see [Put a synth on the pads](synths.md#put-a-synth-on-the-pads)), and the list's last row, **New kit**, gives the selected track an empty kit of its own (Kit 8, say): fill its pads by loading sounds, recording, or sending pads to it. Up to eight new kits are free in a session; one shows in the list once a track plays it or it has sounds, and the project keeps it with its sounds.
- **A kit has 128 pads,** 16 in each of the banks A to H. **PAD BANK** puts another bank on the pads: **B** plays pads B1 to B16, and **SHIFT** (or a second press) reaches E to H. The bank stays when you select another track, and **PADS PLAY** on Play ("Drums · Boom Bap · B") and the Pads tab's banks say which one is on the pads. Whatever you play on bank B records onto those pads, and the Sample tab edits the pad you hit last on any bank. Load or record a sound onto an empty pad of any bank, or send a pad there with **Send to pad**.
- **Every new track starts on kit 1.**
- **In the mixer,** kit 1 plays through the 16 pad channels and the **Drums** strip. Another kit plays into the strip of the first track that plays it (**Track 2**, say), with its own fader, effects and sends; tracks that play the same kit share it. Put effects for the whole kit on the track's strip; a pad that needs effects of its own gets a channel when you add one (see [Effects on one pad](shaping-a-pad.md#effects-on-one-pad-or-on-the-whole-kit)). Pad mutes are kit 1's A1 to A16 alone for now, so mute the track rather than a pad: on another kit's track the Play page's pad speakers and **TRACK MUTE**'s pad mutes say so and mute nothing.
- **The project keeps** which kit each track plays.

## The Perform set on the Pads tab

The **Perform set** is eight pages of action pads, kept apart from the kit's sounds and saved with the project: pads that throw effects, play the loop keys, mute tracks, launch scenes and turn macros. It goes on the pads from the Looper tab (see [More pages: the Perform set](looping.md#more-pages-the-perform-set)); this is where you set up what each pad does.

On the **Pads** tab, tap **Perform pads** at the right end of the **Pads of** row: the project's Perform set takes the kit's place. Tap a track's chip to go back to its kit.

- On the left are the page's 16 pads, each with what it does in its corner (**THROW**, **MACRO**, **LOOP**, **LOOP MUTE**, **MUTE** or **SCENE**). The pages **A** to **H** are over them, with a dot for each pad that does something: tap one to show it. **PAD BANK** picks the page here too, and the set comes back on the pads at that page.
- **Hit a pad** on the MPC, or tap it on the screen, to pick it. A throw plays while you hold its pad (here always while held, even one that latches; one set to a bar plays its bar), so you can hear it, and a macro pad turns its macro while you hold it; any other pad is only picked. Nothing records, mutes or launches, so you can set up the set while a song plays, and the kit's pads don't play while the set shows.
- Over the pad's settings, six chips say **what it does**: **Throw**, **Macro**, **Loop key**, **Loop mute**, **Track mute** or **Scene**. Under them are the eight choices for that kind: Play's eight throws, the eight macros (with how many knobs each moves), Loops 1 to 8, the song's tracks, or the scene rows (eight at a time, around the one picked). Tap one to put it on the pad.
- The **Pad** list under them has the pad's own settings, for a tap, the wheel and **-** / **+**, and under the list a line that says what a hit does. (What the pad does and its choice are the chips above it; **Clear** empties the pad.)
  - A throw: its effect is the one picked of the eight above the list; then **On**, where it plays: **Master** (everything, as Play's throws), one of the tracks as the **Mix** tab names them (**Drums**, **Synth**, **Loop**, **Audio**, and a track that plays its own instrument or kit, by its name), or a submix (**Sub 1** to **Sub 8**), so an echo can catch only the drums; **Hold**: **While held**, **Latch** (a hit starts it and the next hit lets it go) or **1 bar** (a hit, however short, plays it for a bar, and a hit while it plays starts it again); **Starts**: **At once**, **Next 1/16** (as Play's tiles) or **Next beat**, while the song plays (stopped, it starts at once); **Let go**: **Ring out** (an echo's repeats and a reverb's tail carry on) or **Cut** (they stop at once).
  - A macro pad: its **Macro** and a **Value** (0 to 100 %). Held, it turns the macro to that value, and every knob on the macro moves with it; let go, they go back where they were. A macro that moves no knobs says so: put knobs on it from an effect's knobs in the chain editor.
  - A loop pad's **Loop**, a mute pad's **Track** or a scene pad's **Scene**.
- Under the list, for a throw: **Edit the effect ›** opens the Perform FX of the throw's strip in the **Mix** tab, on the page with the throw's own settings (**‹** comes back here). **Hear it** plays the throw for a bar on its strip, from its **Starts** while the song plays, without changing the mix.
- **Learn a key**, beside them, lets a key, pad or footswitch on your gear play the picked pad: tap it, then press the key (or hold **SHIFT** and tap a pad on the map). The key plays that pad as hitting it does, on every page, whatever the MPC's pads are doing: a throw while you hold it, a macro, a loop key (hold it a second to clear the loop), a mute or a scene. It plays whatever is on that pad, so change the pad and the key follows. Tap **Learn a key** again before pressing anything to cancel. See [Playing a Perform pad from your gear](midi-and-cv.md#playing-a-perform-pad-from-your-gear).
- The **Q-Links** turn the picked throw's own settings, named for its effect on the cards at the right: **Q1** its length and **Q2** its amount. A **Delay throw** has **Time** and **Feedback**; a **Reverb wash** has **Decay** only (on Q2); **Stutter**, **Beat repeat**, **Tape stop** and **Vinyl brake** have **Length** and **Mix**; a **Filter dip** has **Sweep** and **Depth**. **SHIFT** turns the amount finer. These settings belong to the Perform FX of the strip the throw plays on, so every pad that throws the same effect on the same strip shares them (the hint says so: "every Delay throw on Drums"). **Q3** and **Q4**, and all four on a pad that isn't a throw, are faint: turning one says why. If the strip's Perform FX is gone (taken off on the Mix tab), the cards are faint and say to pick the pad's On again. A turn records like any knob while you record; one turn, until you lift your finger, is one **UNDO** step. **Q-LINK** does nothing on this view.
- **Copy**, **Paste** and **Clear** work on the pad picked. **Name page** types the page's name on the keyboard; the name shows over the pads and in the log when PAD BANK turns to it. Each change is one step: **UNDO** takes it back and **SHIFT + UNDO** puts it back. The set is saved with the project.
- **On the pads** at the top right puts the set on the pads at the page shown, as **SHIFT + SAMPLER** pressed again on the Looper tab does: Play and the Looper tab play it. Tap it again to give the pads back to the kit.

A throw's On, Starts, Let go and 1 bar are the pad's own: Play's throw tiles keep playing on the master as they did, and so do the Perform FX pads on the master's chain. A throw on a track or a submix plays through a **Perform FX** at the end of that strip's effects. Picking the strip in **On** puts it there (the hint says so), never a hit on the pad, so playing the pads never changes the mix. The **Mix** tab shows it, it passes the sound through unchanged while nothing throws, and it is saved with the project. When no pad throws on that strip any more (you step On on, or clear or change the pad, or UNDO), a Perform FX that picking On put there comes off again, so stepping through the list leaves nothing behind; one that was there already stays. If the strip's effects are full, the hint says so. If a throw's strip has lost its Perform FX (you took it off on the Mix tab), a hit says so: pick the pad's On again. A track that has no channel of its own (its sound goes through Drums or Synth) isn't on the list: pick Drums or Synth instead. If a throw is on such a track's channel, the line under the list says, in the warning colour, that it would be heard on nothing.

{{shot: pads-perform | The Pads tab showing the Perform set: page B's throws, a macro pad, two track mutes, a loop key and a scene, with Reverb wash on B2 picked.}}

{{shot: pads-perform-knobs | The Perform view with B13, the Delay throw on Drums, picked: Q1 its Time and Q2 its Feedback on the Drums track's Perform FX.}}

