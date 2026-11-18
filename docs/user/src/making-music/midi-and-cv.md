<!--
status: changing
covers: rust/mpfree-midi rust/mpfree-midi-ui/src/learn.rs rust/mpfree-midi-ui/src/port_page.rs rust/mpfree-app/src/midi_in.rs rust/mpfree-app/src/midi_out.rs rust/mpfree-app/src/cv_out.rs
checked: 2026-11-16 08282af
-->

# Playing other gear: MIDI, sync and CV

The **MIDI IN** and **MIDI OUT** jacks, USB keyboards and synths on the **USB** port and the **CV/Gate** outputs connect MPFree to other instruments. Playing MPFree from a keyboard and [MIDI Learn](#midi-learn) work now. Following another device's clock works too. Playing gear from **MIDI OUT**, from a USB device and from the CV outputs is built but not yet checked on an MPC One, so it stays off until you turn on **Send MIDI and CV out** in [Prefs](../reference/prefs.md). Its value reads **On (sends MIDI)** while it sends, and MPFree remembers it at the next start.

All of these settings are in one place: the mixer's [I/O view](mixing-and-effects.md#the-io-view), which you open by tapping **I/O** on the **Mix** tab, in its **MIDI**, **Routing**, **Sync** and **CV** sections. A row whose part isn't working yet says so, and keeps what you set. Each port's input, what it ignores, its delay, its clock's nudge and its profile describe your gear, so they stay the same for every project: MPFree keeps them when it quits and puts them back at the next start (with a data folder on the stick). Everything else (what each track plays on, which channels play which tracks and which go thru under Routing, where the tempo comes from, clock out, the CV outputs and learned controls) belongs to the song: it is saved with the project and comes back when you open it, and a new project starts from the defaults.

## Playing MPFree from a keyboard

Plug a keyboard into **MIDI IN**, or a USB keyboard into the **USB** port, and it plays the armed track, just as the pads do (to play other tracks too, see [Playing several tracks from your keyboards](#playing-several-tracks-from-your-keyboards)). Its knobs, faders and pedals can turn effect knobs with [MIDI Learn](#midi-learn). It listens on every MIDI channel, and you can plug a USB keyboard in or out while MPFree runs. A USB interface with several MIDI ports shows each one as a port of its own, named as the device names it (up to 15 USB ports at once). If a keyboard sends **All notes off** (or **All sound off**), the keys it holds on that channel let go.

- **With a synth on the pads,** each key plays its own note, through the arpeggiator, so **NOTE REPEAT** and Latch work from the keyboard too. A note held on a key and a pad ends when both let go.
- **With a drum kit,** the keys that match the MPC's own pad notes hit the pads: C#2 is pad 1, C2 is pad 2, and so on. Other keys play nothing.
- The pad velocity setting, and Full Level, shape the keys' velocity as they do the pads'.

### The pitch wheel, pressure and MPE

A keyboard's **pitch wheel** bends every note of the sound it plays, by that sound's own *Bend range* on the Synth tab. **Pressure** on a key (polyphonic aftertouch) or on the whole keyboard (channel aftertouch) goes to the sound's pressure destinations, as pressing a pad harder does: channel aftertouch presses every key held on its channel. A route's track takes its port's wheel and pressure too.

An **MPE** controller (an expressive keyboard that gives each finger its own channel, such as a Seaboard, a LinnStrument or an Osmose) plays each note with its own expression:

- sliding a finger bends only its note (48 semitones across the wheel's range, unless the controller says another),
- pressing harder presses only its note,
- moving a finger up the key (MPE's Y axis, CC 74) shades its note's **tone**, which each sound sends where it offers, as it does pressure,
- the wheel and pressure on the zone's main channel still move every note.

Most MPE controllers say they are MPE when they start, and MPFree hears it: its log says *MPE lower zone, channels 2 to 16 (it said so)*. If yours doesn't, give its port a [profile](#device-profiles) with an **mpe** line.

The wheel, pressure and MPE play MPFree's own sounds: the synth and a track's own instrument (a kit's pads take pressure only). They go into the arpeggiator's and the MIDI effects' notes only while those notes are the keys as they came: a note the MIDI effects moved, or one the arpeggiator plays, takes the wheel but not its key's slide or tone. A track that plays a synth through MIDI out sends them to that synth ([The wheel, pressure and MPE on a synth](#the-wheel-pressure-and-mpe-on-a-synth)), and its [CV outputs](#cv-and-gate) take them too. Recording them into a clip comes later.

## Playing several tracks from your keyboards

{{shot: mixer-io-routing | The I/O view's Routing section: channel 10 of MIDI IN plays Drums, whichever track is armed, and the rest of its notes play the armed track. The keyboard's channel 3 goes thru to MIDI out; with MIDI out off in Prefs, its square says so.}}

The I/O view's **Routing** section shows where each source's MIDI goes, on one grid. Down the side are your ports (the 5-pin **MIDI in** and each USB device), **The pads**, and the tracks. Across the top: **Armed track**, **Tracks by channel**, **Learn**, each MIDI out port and **CV out**. A square with a green bar plays something in MPFree; a blue bar sends something out. **+** means a tap sets it up, and **—** means nothing can go there (a tap says why). A small number in a square's corner counts its [blocks](#blocks-on-a-route).

- **Tracks by channel** sends one channel of a port to one track, whichever track is armed. Tap a port's square and pick the channel and the track on the card below the grid (with – and +, or Q-Links 1 and 2). **+ Route** adds another, **Remove route** takes the picked one away. Two routes from the same channel to two tracks play both, to layer sounds. For example: a drum machine on **MIDI IN**, channel 10, plays **Drums**, while a keyboard on channel 1 plays **Bass** and you work on another track with the pads.
- Notes a route takes go only to its track, never to the armed track as well, so nothing plays twice. The rest of the port's notes go to the armed track, as before.
- A track played by a route plays where its clips play: its own sound, its synth or drum machine through MIDI out, and its CV outputs. With a drum kit, the keys that match the MPC's pad notes hit the pads. Only the armed track's notes go through the arpeggiator, and only the armed track records.
- **Armed track** and **Learn** are the port's **Input** (Both, Plays tracks or Learn only), the same setting as on its row in the MIDI section. Turning both off isn't allowed: turn the other on first. A port set to **Learn only** still plays its routes.
- A track's square under a port is its **Plays on**, as in the MIDI section: tap it to play that track on that port, and pick the channel on the card.
- Routes belong to the song and are saved with it. A route from a device that isn't plugged in waits for it.
- Up to 32 routes in a project.

The pads always play the armed track. When that track plays out of a port (its **Plays on**), the pads play the gear there too, a kit's pads by the gear's [profile](#a-kit-on-a-drum-machine); their squares under the outputs say so.

### Blocks on a route

{{shot: mixer-io-routing-blocks | The keyboard's Armed track square picked: its notes reach the armed track through a Filter that drops program changes, a Transpose an octave down and a soft Velocity curve from 20 to 127, with + Block for a fourth.}}

Each route can change what it takes before it plays: up to four **blocks**, one after another, as tiles on the card under the grid.

- **Filter** drops kinds of message: notes, bend and pressure, controls (CCs) or program changes. Its Q-Links 1 to 4 each let one by (turn left) or drop it (turn right).
- **Channel** sends everything on one channel (Q-Link 1). Useful on thru, for a synth that listens on its own channel.
- **Split** takes only the keys from its lowest to its highest (Q-Links 1 and 2). The keys outside it aren't the route's: they go on to another route on the same channel, or to the armed track. Two routes from one keyboard to two tracks, each with a Split, make a split keyboard: bass on the left hand, chords on the right.
- **Transpose** moves notes by semitones (Q-Link 1) or octaves (Q-Link 2). A note moved off the keyboard is dropped.
- **Velocity** shapes how hard you play: a curve (Q-Link 1: **soft** makes gentle playing louder, **hard** takes a harder hit to sound loud), then the lowest and highest velocity that come out (Q-Links 2 and 3), or one velocity for every note (Q-Link 4, **Fixed**; turned fully left it is off).

Where the blocks are:

- A port's **Armed track** square shows its blocks on the card at once: they shape everything the port plays on the armed track (or, with tracks that listen to their own ports, on those).
- A route under **Tracks by channel**, and a thru under a MIDI out port, has a **Blocks ›** button on its line. Tap it for that route's blocks; **‹ Routes** goes back.

On the card, **+ Block** lists the five kinds: tap one to add it at the end. A tap on a tile picks it, and the Q-Links turn it. The **›** between two tiles swaps them, and the picked tile's **×** (or **–**) takes it away. Blocks run in order from left to right, so a Transpose before a Split moves the notes first and the Split then looks at the moved notes.

The card's last line shows what last went through and what it became: *Last through Note on C4 vel 96 → C3 vel 104 played the armed track · 2 s ago*, or that a block dropped it. [MIDI Learn](#midi-learn) and MIDI Follow hear a port's messages as they came, before any block, so a Filter never hides a knob from them. A key you hold while you change a block still ends on the note it started.

Blocks belong to the song and are saved with it, with their routes.

### Thru: a keyboard straight to a synth

**Thru** sends what comes in on a port straight out of another, as it came: play a synth on **MIDI OUT** from a USB keyboard, with MPFree in between. A port's square under a MIDI out port is its thru.

- Tap the square: every channel of that port goes thru. On the card, pick one channel instead with – and + (or Q-Link 1), and **+ Route** adds another channel. **Remove route** takes the picked one away. The square then reads, say, *thru · ch 10 only*.
- The channels that go thru play nothing inside MPFree: not the armed track. The rest of the port's notes play the armed track, as before. A route to a track on the same channel plays as well, so you can layer the synth with a sound in MPFree.
- Notes, aftertouch, pitch bend, controllers and program changes go thru. Clock never does (each port's **Clock out**, under Sync, sends MPFree's own), and neither does SysEx, nor anything the port is set to ignore on its page. A knob you taught with [MIDI Learn](#midi-learn) moves its value and doesn't go thru.
- Thru leaves when the armed track's notes would (a few milliseconds after the key), and sends only while **Send MIDI and CV out** is on in Prefs. Until then its square says *MIDI out is off*. Pressing **STOP** while stopped, the Monitor's **Send note-offs**, or the port page's **All notes off** ends the notes it left on.
- A USB device's own output is **—**: its messages never go back to it. **MIDI in** to **MIDI out** is two pieces of gear, so that one is fine. If the gear on **MIDI OUT** sends what it receives back to **MIDI IN** (its own thru or echo), thru from **MIDI in** makes a loop: turn that off on the gear.
- The **Thru** field on a port's row in the MIDI section is the same setting: **Off**, or the output its every channel goes to. A port whose thru is one channel, or goes to more than one output, says *thru: set on the Routing section*.
- Thru belongs to the song, like routes, and waits for a device that isn't plugged in. Up to 16 in a project.

> [!MPC]
> On the MPC, a track's MIDI input is set on each track. In MPFree the Routing section shows every port's routes on one grid, so you can see what goes where at a glance.

## Playing a synth from a track

A track can play a synth or drum machine through **MIDI OUT**, or a USB synth or interface plugged into the **USB** port, instead of a sound inside MPFree. It has one setting for where it plays, **Plays on** in the I/O view's MIDI section: the port and the MIDI channel together. Every port with an output is in the list, the USB ones under the names the devices give them. A track can also send a program change (**Program**), so the synth switches to the right sound when the song starts. A clip can send its own as it starts, with a bank select too, so a verse and a chorus can play different sounds on the same synth: see the clip's **Program** and **Bank** in [Clip settings](clips.md#clip-settings).

If a track would send nothing, it says why under the track's name: no port is picked, the device is unplugged or has no output, **Send MIDI and CV out** is off in Prefs, or the port's output didn't open (the log says why; turn **Send MIDI and CV out** off and on, or plug the device in again, to try again). A track that sends nothing plays its own sound inside MPFree instead. If you unplug a USB synth while it plays, MPFree can't send it the ends of its notes; switch it off and on, or send it **All notes off** once it is back.

> [!MPC]
> On the MPC, a MIDI track needs two settings with nearly the same name before a note leaves the jack: the track's MIDI output channel and its MIDI Out routing. In MPFree it's one setting, the port and the channel together.

### The wheel, pressure and MPE on a synth

A track that plays a synth through MIDI out passes your keyboard's [pitch wheel, pressure and MPE](#the-pitch-wheel-pressure-and-mpe) on to it, so the synth answers them as MPFree's own sounds do. Most synths have one bend, one pressure and one tone for all their notes, so MPFree makes one of each from the keys you hold:

- **Bend:** the wheel, plus the slide of the key you played last, over the synth's bend range. That is 2 semitones unless the synth's [profile](#device-profiles) has a **bend** line; set it to the range the synth is set to, so a finger slid a semitone bends the synth a semitone.
- **Pressure:** the hardest-pressed key's, as channel pressure (aftertouch). A synth that takes each note's own pressure (polyphonic aftertouch) gets each key's on its note instead when its profile says `pressure = poly`.
- **Tone:** the tone of the key you played last, as CC 74, which most synths have on their filter cutoff.

Each goes out only when it changes. When you let go of the last key, the bend goes back to the wheel's and the pressure to 0, and a stop, a panic or a new **Plays on** puts the synth back at rest. Keys that play a kit's pads send each pad's pressure on its notes, and no bend. To send a synth the keyboard's own messages as they came (an MPE synth's channels and all), use [Thru](#thru-a-keyboard-straight-to-a-synth) instead.

### A kit on a drum machine

A track that plays a kit sends each pad as a note, so the kit can play a drum machine: set the track's **Plays on** to the drum machine's port and channel. Each pad plays its own note:

- Bank A's pads play the MPC's own pad notes, as an MPC sends them: pad A1 is C#2 (37), A2 is C2 (36), A3 is F#2 (42), and so on.
- Banks B to H play the other 112 notes, one each, upwards from B1 (35, the General MIDI kit's lowest drum): B1 is 35, B2 is 39, B3 is 41, and so on up to 127, then from 0. So every one of a kit's 128 pads plays a note no other pad plays, and banks B and C hold the General MIDI drums bank A doesn't.

If your drum machine listens for other notes, or has a channel per drum (as many do), give it a [device profile](#device-profiles) with a **pad** line for each pad you use, and pick the profile on its port's page: `pad.A1 = 36 ch 1`, `pad.A2 = 38 ch 2`, and so on. Every kit on that port then plays those notes and channels. A note always ends where it started, even if you change the profile while it sounds. The port's page says how many pads its profile covers.

## Controller mode: the pads and Q-Links play your gear

Controller mode makes the MPC's pads and Q-Links a controller for a synth, a drum machine or a computer on **MIDI OUT** or the **USB** port. While it is on, the pads and the Q-Links play your gear and not MPFree. Everything else carries on as before: the transport, the screen and the tracks that are playing.

{{shot: mixer-io-controller | The I/O view's Controller section with controller mode on: the pads and Q-Links play a Juno-X on MIDI OUT, channel 3, with kit notes and poly pressure, and its device profile names the first four Q-Links' controls.}}

Its settings are in the mixer's I/O view, in the **Controller** section:

- **Controller mode**: **Off** or **On**. The line under it says what it does now, or why it sends nothing.
- **Sends to**: the port and the MIDI channel. Every port with an output is in the list. A port that isn't plugged in is kept until it is.
- **Pads send**: **Kit notes** sends the notes a kit sends to a drum machine (see [A kit on a drum machine](#a-kit-on-a-drum-machine)). Bank A's pads send the MPC's own pad notes. If the port's [device profile](#device-profiles) has **pad** lines, each pad sends the note and channel its line gives. **One by one** sends a run of notes upwards from **From**: A1 sends the note picked, A2 the next, and so on through the banks. A pad past note 127 sends nothing. The line under it says what A1 and A16 send.
- **Pad pressure**: **Off**, **Poly** (each pad held sends its own pressure on its note) or **Channel** (one pressure for the channel: the hardest pad held).
- **Q-Links 1-4** to **13-16**: the control change (CC) each Q-Link sends, four banks of four. Each field shows what the CC moves on your gear: the name in the port's device profile, else the control's common name (**Cutoff**), else its number. The line under each row gives the numbers. They start as CCs 16 to 31.

While it is on:

- The pads light blue, brighter while held. **PAD BANK** picks their bank: A to D, and SHIFT or a second press for E to H.
- **Q-LINK** steps the Q-Links to the next four: 1-4, 5-8, 9-12, 13-16, then 1-4 again.
- The Q-Links send their CCs on every page, and the Q-Link column shows them: the name of what each moves and the value it sent last. A Q-Link turned up or down moves its value from 0 to 127. Each starts at 0, because MPFree can't know where your gear's control is.
- **FULL LEVEL** and **HALF LEVEL** still set how hard the pads play.
- STEP SEQ, TRACK MUTE and 16 LEVEL wait: the pads are the controller's.
- In the top bar, a chip takes the kit chip's place: *Controller → Juno-X · ch 3 · A* (the port, the channel and the pads' bank), or why nothing goes (*Controller: MIDI out is off in Prefs*). A tap on the chip turns controller mode off.

{{shot: play-controller | Play with controller mode on: the Controller chip in the top bar, and the Q-Link column showing the four CCs the Q-Links send to the Juno-X, the first two turned.}}

Turning controller mode off, or changing its port or channel, ends any notes still held. Its settings are kept for every project. The switch itself isn't kept, so MPFree always starts with the pads its own. Like everything MPFree sends, controller mode sends nothing while **Send MIDI and CV out** is off in Prefs.

Controller mode plays gear on **MIDI OUT** or the **USB** port where USB synths plug in. Playing a computer through the MPC's own USB port, the one a computer plugs into, isn't built yet.

## A port's page

Each port's row in the MIDI section has **Open ›** at its right; it, or a tap on the port's name, opens the port's page. **‹ MIDI** or **BACK** closes it.

- **This port** says what it is (the MPC's 5-pin jacks, or a USB device), its **Name**, its **Input** (**Both**, **Plays tracks** or **Learn only**, the same setting as on its row) and its **Profile**.
- **Name** gives a USB device a name of your own, such as *Juno* for a box that calls itself *USB MIDI Interface*. A tap (or +) opens the on-screen keyboard; type the name and tap **Enter**. From then on MPFree uses it everywhere it names the port: its row, Routing, a track's **Plays on**, Sync, Learn, the Monitor and the tempo chip. The page still says the name the device calls itself. A name has at most 24 characters, and two ports can't share one (whatever the capitals). Type the device's own name to go back to it. The name belongs to your setup, not the song, so every project uses it. The 5-pin jacks keep the names on the MPC's panel, **MIDI In** and **MIDI Out**.
- **Profile** picks the [device profile](#device-profiles) that names this port's controls and programs (and, for a drum machine, gives a kit's pads their notes): the Monitor then shows *CC 74 Filter cutoff* where it would show *CC 74*, and the Learn section *CC 1 Mod wheel*. A tap steps through **Auto**, **None** and each profile by name (– and + step too). **Auto** takes a profile made for the name the device calls itself, if there is one, and the box says which (*Auto · Juno-X*). **None** names only the controls most gear agrees on, such as *CC 7 Volume* and *CC 64 Sustain*. The line under it says how many controls and programs the profile names. Like the name, the profile belongs to your setup, so every project uses it.
- **Ignore coming in** has four switches: **Clock**, **Program**, **Aftertouch** and **All notes off**. A lit one is ignored: those messages from this port play nothing, teach nothing and move no tempo. The Monitor still lists them, marked *ignored: this port's setting*, so you can see they arrive. **Clock** also covers the Start, Stop and Song Position that come with it. Ignore **Clock** on a drum machine that sends clock you don't want; ignore **Program** on a keyboard that sends one when you change its sound.
- **What it does** lists, in words, what this port's messages do today and what plays on it: its routes to tracks (*Ch 10 → Drums*) and its thru (*Ch 3 → MIDI out, thru*), where the rest of its notes go, the tracks that play out of it with their channels, thru from other ports, the controls learned from it, and whether the tempo follows it. Change its routes in the Routing section.
- **Now** shows the newest message in and out and how long ago.
- **Clock to this port** is **Off**, **While playing** or **Always**: the same setting as **Clock out** under Sync, saved with the project.
  - **Divide** (1 to 8, – and +, or Q-Link 4) slows the clock this port gets: at 2 the gear hears half the pulses, so it plays at half the tempo, and its song position counts its own (slower) sixteenths. Use it for a drum machine at half time, or gear whose arpeggiator should run slower than the song. Changed while playing, the port gets Stop, then joins again at the next sixteenth at the new rate. Divide is saved with the project.
  - **Nudge** (up to 20 ms either way, in steps of 0.5 ms) moves this port's clock, start and stop later (+) or earlier (–) than its notes, for gear whose own sequencer runs a little ahead of or behind the clock it follows. It comes on top of the gear's delay, so earlier is limited the same way (about 9 ms in all). Nudge belongs to the gear, so it is kept for every project, like the delay.
  - The line under them says what the clock did on the way out, measured over the last beat: *Pulses leave every 20.8 ms, ±0.3 ms* (at 120 BPM a pulse is due every 20.8 ms; the ± is how far the pulses strayed). It shows while the port gets clock; otherwise the line says why the clock is only kept (MIDI out is off, the port has no output, or it didn't open).
- **This gear's delay** is how late the gear on this port sounds after MIDI reaches it, in steps of 0.5 ms (– and +, or Q-Link 3). MPFree sends that port's notes and clock that much early, so they land on the beat. It can send up to about 9 ms early (the sound's buffer, the time between MPFree making a sound and you hearing it); the card says when the delay is more than that.
- **Send to it** (marked **SENDS MIDI**): **Test note** plays C4 for half a second on the first channel a track plays this port on (else channel 1), to check the cable and the synth's channel. **All notes off** ends MPFree's own notes on this port (other ports' notes go on), then sends All notes off (CC 123) on all 16 channels, for a synth that hangs. Both are greyed while **Send MIDI and CV out** is off, and on a port that isn't plugged in, has no output or whose output didn't open; the greyed line says which.

The wheel picks a setting on the page, – and + change it, and Q-Links 1 to 4 turn **Input**, **Clock**, the delay and **Divide**.

### Device profiles

A device profile is a list of what one synth or controller calls its controls (CCs), its NRPN parameters and its programs, and for a drum machine, the note (and channel) each of a kit's pads plays on it. MPFree has **General MIDI** built in: the 128 General MIDI sounds by number (*Program 34 Electric Bass (finger)*) and the controllers General MIDI gear answers to, such as *CC 91 Reverb* and *CC 93 Chorus*. Pick it on the page of a port with a General MIDI sound module.

You can write your own for any gear, as a text file, and put it on the MPC's data partition in `files/midi-profiles/`, with a name ending `.txt`. MPFree reads that folder when it starts; it only reads the files, never changes them. A profile looks like this:

```text
mpfree-profile 1
name = Juno-X
match = Juno-X
cc.74 = Filter cutoff
cc.71 = Filter resonance
nrpn.1.23 = Osc 1 shape
program.1 = Init
```

- **name** is what the port's page shows: at most 32 characters, and not *Auto* or *None*. Two profiles can't share a name, so one with the name of the built-in General MIDI is left out.
- **match** is a name a USB device calls itself (the page says it: *it calls itself …*). A port set to **Auto** takes the first profile with its name. Put up to 8 **match** lines in one profile, or none.
- **cc.N** names CC N (0 to 127); **nrpn.MSB.LSB** names the NRPN parameter sent as CC 99 = MSB and CC 98 = LSB; **program.N** names program N, counted from 1 as everywhere on screen. Each name has at most 32 characters.
- **mpe** says the controller plays [MPE](#the-pitch-wheel-pressure-and-mpe): `mpe = lower 15` (main channel 1, notes on channels 2 to 16; fewer channels with a smaller number), `mpe = upper 15` (main channel 16) or `mpe = off`.
- **bend** is a synth's pitch bend range in semitones, 1 to 96 (`bend = 12`), which your keyboard's [wheel and slides](#the-wheel-pressure-and-mpe-on-a-synth) bend it by. Without it MPFree takes 2, the range most synths start with.
- **pressure** is what a synth takes for your keys' pressure: `pressure = channel` (one value for all its notes, which most synths take, and what MPFree sends without the line) or `pressure = poly` (each note's own).
- **knobs** lists a controller's knobs by their CCs, in order, up to 16 (`knobs = 74 71 76 77`). [MIDI Follow](#midi-follow-your-controllers-knobs-on-the-q-links) turns Q-Links 1 to 4 with the first four, unless you taught it the controller's knobs yourself.
- **pad.A1** to **pad.H16** (or **pad.1** to **pad.128**) is the note a kit's pad plays on a drum machine on this port, as a number (0 to 127) or a name (*C2*, *F#1*, *Bb0*; C4 is 60), and after **ch**, a channel of its own (1 to 16): `pad.A1 = 36 ch 10`. A pad without a line plays its usual note on the track's channel (see [A kit on a drum machine](#a-kit-on-a-drum-machine)).
- A line starting with `#` is a note to yourself. A line MPFree can't read is skipped, and the rest of the profile still works; a file with no **name** isn't a profile. MPFree's log says what it skipped and which profiles it found.

A port's profile names what comes in from it and what goes out to it. Only its **pad**, **bend** and **pressure** lines change what MPFree sends: the notes and channels a kit's pads play there, and how your keyboard's expression bends and presses a synth. Everything else only names.

## Staying in time with other gear

MPFree can follow another device's MIDI clock. In the mixer's I/O view, under **Sync**, set **Tempo from** to the port the device is plugged into:

- Its tempo becomes MPFree's and moves when it moves. The top bar shows a tempo chip such as **120.0 from MIDI In**, with a green dot, until its clock stops.
- With **Start and stop** on, its Start and Stop start and stop MPFree. While the device is stopped, **PLAY** waits for it and says *Waiting for clock: PLAY starts when MIDI In starts*: press PLAY (or PLAY START, for bar 1) to get the pattern ready, then start the device.
- To record from the device's start, press **REC** (or OVERDUB) while both are stopped. The take records from the device's first beat, with no count-in, since the device counts. PLAY keeps the take ready and says so; STOP lets go of it.
- With **Position** on, MPFree plays from the same bar as the device, and goes back in step if it drifts away. The device owns the song's timeline, so MPFree's own loop is off while Position is on: the Record tab's **Loop** row reads *off while following*, and what you set there comes back when you turn Position off. Clips and scenes you launch still land on the bar line, which is the device's bar line too.
- If its clock stops arriving, MPFree keeps playing at the last tempo.

MPFree can also send its own clock from **MIDI OUT**, or to a USB device, so a drum machine or an arpeggiator plays in time with it. Each port's **Clock out** is **Off**, **While playing** or **Always**, and each port with a delay gets its clock that much early. On a port's page, **Divide** slows the clock one port gets and **Nudge** moves it a little later or earlier than its notes (see [A port's page](#a-ports-page)).

## MIDI Learn

MIDI Learn lets a knob, fader, button, key or pedal on your keyboard or controller turn a value in MPFree: a mixer strip's fader, pan or send, its mute or solo, an effect's knob, a macro, a knob on the Synth tab, the tempo, or the transport's buttons (see [Playing and stopping from your controller](#playing-and-stopping-from-your-controller)).

**The quick way: SHIFT + touch.** Hold **SHIFT**, touch the value on the screen, then move the control on your keyboard. Until a control takes it, the hint line reads *LEARNING Volume (Bass): move a knob, a key or a pedal on any controller*; then it says what the control moves (*Learned: CC 74 on Keystep 37 moves Poly cutoff (Track 2)*). While SHIFT is held the touch only picks the value: the fader doesn't move. Touch the same value with SHIFT again to cancel, or another one to learn that instead. It works on:

- the mixer's Faders look: a strip's fader, pan and sends, and its **M** (mute) and **S** (solo);
- the tempo on the Play page;
- the [chain editor](effects.md#adding-an-effect-the-chain-editor): an effect's knobs, faders, sliders and drawbars;
- the macros panel: a macro's knob;
- the Synth tab: the synth's knobs, faders, sliders and drawbars, for the track it shows (the track's own instrument, or the synth on the pads).

Anywhere else SHIFT + a touch keeps its own job, such as SHIFT + drag on an audio region.

**From the chain editor:**

1. Open the effect in the chain editor and touch the knob.
2. Tap **Learn** in the **Modulate** row under the page. It now reads **Turn a Q-Link**, and the line under the knob's name says it is learning.
3. Move the control on your keyboard. From now on it turns that knob, as far up and down as the knob goes. (Turn a Q-Link instead, and the Q-Link learns it, as before.)

The move that teaches a control doesn't play a note, and neither does a learned control. Teach a control again and it moves to the new value.

### The Learn section

{{shot: mixer-io-learn | The I/O view's Learn section: what each control moves, where it stands now and where it is kept, with the picked control's range, how it works and how it takes over.}}

Every learned control is listed in the mixer's I/O view, in the **Learn** section: the device and the control on it (**CC 74**, **note C2 ch 10**), what it moves, a bar for where that stands now, and where it is kept. A row in grey or amber says why its control does nothing: the device isn't plugged in (*unplugged, waits*), its port is set to **Plays tracks**, which keeps every message for playing (*its port only plays tracks*), or the song has its own mapping for the same control (*the song's own mapping wins*). A port set to **Learn only** turns knobs but plays no notes on the armed track (its [routes](#playing-several-tracks-from-your-keyboards) still play).

Tap a row, or turn the data wheel, to pick it. The card under the list sets it:

- **Range**: where the control's bottom and top land, in the knob's own units (– and +, or Q-Links 1 and 2; SHIFT for finer steps). Stop a fader's top at 0 dB, or keep a filter between 200 Hz and 8 kHz. A bottom above the top turns the control upside down.
- **Works as** (– and +, or Q-Link 3) says how it moves the knob:
  - **Knob or fader**: the knob follows it. Every control but a key starts as this.
  - **Endless**: for a knob that turns forever. Each click moves the knob one step on from where it is, even after you changed it on the screen. Controllers send these three ways, so if the knob jumps or runs backwards, try the next one: **1 up, 127 down**, **65 up, 63 down** or **1 up, 65 down**.
  - **Button: on, then off**: each press switches between the range's top and bottom.
  - **Button: while held**: top while held, bottom when let go. A key starts as this, except on a mute, a solo or **PLAY and STOP**, where it starts as **Button: on, then off**.
- **Takeover** (Q-Link 4), for a knob or fader, says what happens when it and the screen disagree, after you changed the value on the screen or opened another song:
  - **Jump**: the value goes straight to where the knob is.
  - **Pick up**: the value waits until the knob passes it, so nothing jumps.
  - **Scale**: the value moves at once, but in step with the knob, so the two meet at the end the knob turns to.
- **Kept in**: **Project** keeps it with this song; **Rig** keeps it for every project, like a controller's tempo fader or a drum machine's mute key. A control learned on the transport or the tempo starts in **Rig**, anything else in **Project**. While a song has its own mapping for the same control, the song's wins.
- **Learn again**: the next control you move takes this one's place, with the same range, takeover and **Kept in**. The banner at the top says what it waits for; **Cancel** stops it and keeps the old control.
- **Remove** takes the control off.

Learned controls kept in **Project** are saved with the song; those kept in **Rig** stay when you open another project and come back at the next start (with a data folder on the stick). Still to come: learning the values on the Play page, the Pads and Sample tabs, the arpeggiator and a plugin's own page; and recording a learned synth knob's moves into a clip.

### Playing and stopping from your controller

A button, pad, key or footswitch on your controller can press MPFree's transport buttons, and a knob or fader can set the tempo. In the Learn section, tap **Transport and tempo** by the heading (the card shows them straight away while nothing is learned yet), tap one, then press the button or move the knob:

- **PLAY and STOP**: one button for both. It plays when MPFree is stopped and stops it when it plays. A key or a pad flips it on each press; a footswitch that sends a CC plays while held, so set its **Works as** to **Button: on, then off**.
- **PLAY**, **STOP**, **PLAY START**, **REC**, **OVERDUB** and **TAP TEMPO**: each does just what the MPC's own button does on the page you're on: PLAY while playing is FILL, and tapping TAP TEMPO in time sets the tempo. A press counts when the control goes past halfway and lets go when it comes back, so leave these as **Button: while held** or **Knob or fader**.
- **Tempo**: from 30 to 300 BPM over the control's travel. Narrow it with **Range**, such as 110 to 130 for a fader that nudges, and use **Pick up** so the tempo doesn't jump when you first touch the fader. While the tempo follows another device's clock (Sync's **Tempo from**), the learned tempo control does nothing.

For the tempo the quick way works too: hold **SHIFT** and touch the tempo on the Play page. To mute a track from a controller, hold **SHIFT** and touch its **M** in the mixer, then press a key: each press mutes or unmutes it.

Transport controls are kept in **Rig**, so they work in every project. Nothing here sends anything to your controller.

### MIDI Follow: your controller's knobs on the Q-Links

With MIDI Follow on, four knobs on one controller turn whatever the MPC's four Q-Links turn on the screen you're looking at: the synth's knobs on the Synth tab, an effect's knobs in the chain editor, the picked strip in the mixer, and so on. Go to another page and the same knobs turn that page's Q-Links, in every song, with nothing to learn.

The **MIDI Follow** card is in the **Learn** section. Tap **MIDI Follow** by the section's heading to show it (the button is lit while it shows); tap the button again, a row or **Transport and tempo** to go back. On a screen wide enough it sits beside the picked control's card instead. On the card:

- **Controller**: the device whose knobs follow (– and + step through the ports with an input).
- **Follow**: **Off** or **On**. **On** picks the first device plugged in if none is picked.
- **Send values back**: **Off** or **On** (Off until you turn it on). On, each knob is sent its Q-Link's value whenever it changes: on another page, by a turn on the MPC, or by an LFO. A controller with lights round its knobs or motor faders then shows where each value is. Turn it on only for a controller like that: a plain knob can't move, so its next turn would jump. Nothing goes back to a knob while you turn it, or for a moment after.
- **Knobs**: the CCs of the knobs that turn Q-Links 1 to 4. Tap **Learn knobs**, then turn the knobs you want, one at a time, in Q-Link order: after the fourth, or when you tap **Done**, it keeps them. **Clear** forgets them, and the controller's [device profile](#device-profiles) gives its knobs instead (its **knobs** line), if it has one.

The line under them says what it does now, or why it waits (*Keystep 37 isn't plugged in: Follow waits for it.*).

- A knob turns its Q-Link by as much as it moves, so a value never jumps when you start turning. The first move after you plug the controller in or turn Follow on only tells MPFree where the knob is.
- The controller's keys still play the armed track, as any keyboard's do, and its other controls do what they did. A control you taught with MIDI Learn keeps its job: Learn wins over Follow.
- MIDI Follow is kept with the rig, so it stays when you open another project.
- Values go back only while **Send values back** is on and **Send MIDI and CV out** is on in Prefs, and only to a controller with an output. Each goes on the CC and channel its knob sends.

### Launching clips and scenes from your controller

A pad, key or button on your controller can do what a tap on the Clips grid does, so a grid controller can launch your song. On the **Arrange** tab's Clips grid, hold **SHIFT** and tap one of these, then press the pad, key or button:

- **A clip** (or an empty cell): the control does what a tap there does at that moment. It launches the clip, stops it while it plays, takes back a launch that is waiting, or in an empty cell makes a clip and starts recording into it. The control belongs to the cell, not to the clip: whatever clip you put in that track's row later, the same pad launches it.
- **A row's number**: the control launches that whole row as a scene.
- **Stop all** or **Back to song**.
- **A track's name**: the control arms the track, as a tap on its name does (pressed again, it lets go).

Launches land where a tap's would: on the next **Launch** line (the Record tab's, or the clip's own). While MIDI Learn waits, the line under the screen says what for, then what the control launches; SHIFT and the same tap again cancels. SHIFT and a drag still copies a clip or a row, and SHIFT + **+ Track** still adds a Loop track.

The Learn section lists them by where they are: **Launch Bass · row 2**, **Scene 3 · Verse**, **Arm Bass**, **Stop all**. They're kept with the song (**Project**); set **Kept in** to **Rig** to keep a grid controller's layout in every project. Still to come: a **Clips and scenes** card in the Learn section for a track's mute, solo and stop. A key on a track's mute flips it on each press, and in Perform it waits for the launch line, as TRACK MUTE's pads do.

## MIDI files

A MIDI file from another sequencer, a computer or an MPC loads from BROWSE as clips, with each note on the pad that plays it: see [Bringing in a MIDI file](clips.md#bringing-in-a-midi-file). **Export** makes a MIDI file of your song for a computer: see [Exporting your music](exporting.md).

## CV and gate

The **CV/Gate** jacks play modular synths. Each of the eight outputs has one job:

- a track's **pitch**, 1 V per octave (or Hz per volt, for older Korg and Yamaha synths), with the lowest, highest or last note winning when several are held;
- a track's **gate**, high while a note is held, or a short **trigger** on each note (of every note, or of one note, for a drum module);
- a track's **velocity**, its **pressure**, or one of its **CCs**, as a voltage;
- an **LFO** or an **envelope** that moves on its own (see [LFOs and envelopes](#lfos-and-envelopes));
- a **clock** at the rate you pick, **run** (high while the song plays) or **reset** (a pulse when the song starts from the top);
- or a **fixed** voltage.

The outputs are set in the mixer's I/O view, under CV. A track that drives CV still plays its own sound too, so turn it down in the mixer if you only want the module.

When one note ends just as the next starts, as a sequence's notes do, the gate drops for 2 ms between them, so the module's envelope starts again. Hold two notes at once for a legato line instead.

Your keyboard's [wheel, pressure and MPE](#the-pitch-wheel-pressure-and-mpe) reach the outputs of the track it plays, as they reach a synth through MIDI out: a **pitch** output bends by the wheel, plus the slide of the key you played last, over its own bend range; a **pressure** output follows the hardest-pressed key (or a pad's pressure in a clip); and a **CC** output on CC 74 follows the tone of the key you played last. A stop or a panic unbends the pitch and drops the pressure to 0 V.

### LFOs and envelopes

An output whose job is **LFO** or **Envelope** moves by itself, so a filter can sweep or a VCA can open without a module of its own for it. Its row gets two more rows under it for the rest of its settings; the wheel and the Q-Links work on them as on any row.

{{shot: mixer-io-cv-moving | The CV section with an envelope of the Drums track's keys on CV 2, a Drift LFO on CV 3 that follows the song's bar and plays A minor pentatonic, and a free sine LFO on CV 4.}}

An **LFO** sweeps between its **Low** and **High** voltages in one of seven shapes: **Sine**, **Triangle**, **Ramp up**, **Ramp down**, **Square**, **Random** (a new level each cycle, held) and **Drift** (gliding from one random level to the next).

- **Follows**: **Tempo** sets its speed in bars and notes (16 bars down to 1/32, with triplets), and while the song plays its cycle starts on the beat, so a one-bar LFO starts again with every bar and a Random one plays the same levels each time the song comes round. **Seconds** sets it in hertz, from 0.02 to 20, running free.
- **Restart** (the row's Track column): pick a track and the LFO starts its cycle again at each of that track's notes; **Runs free** for none.
- **Notes**: **Smooth** leaves the voltage as it is. Pick **Semitones**, **Major**, **Minor**, **Major pent.**, **Minor pent.**, **Fifths** or **Octaves** and the voltage snaps to those notes at 1 V per octave, from the **Root** you pick, so a Random or Drift LFO into an oscillator plays a melody in key. C is 0 V, so tune the oscillator to a C at 0 V.

An **Envelope** opens on the row's track's keys: up to its top in **Attack**, down to the **Sustain** level in **Decay**, held while any of the track's keys is down, and down to 0 V in **Release** once the last key lets go. A new note while keys are held starts the attack again from where it is. The row's setting is the top voltage. **Fixed top** goes all the way up on every note; with **By velocity** each note's velocity sets how high it goes.

An LFO or an envelope sends a new value every 4 ms while it moves (and right at a note), and nothing while it holds still. Changing a setting while it runs doesn't start it again.

CV 1 to 4 send while **Send MIDI and CV out** is on in Prefs. They haven't been checked on an MPC One with a meter yet, so treat their volts as a first guess; CV 5 to 8 come after that check. Until an output works, its row in the I/O view says it sends nothing. Modules differ, so each output will also be tunable against your module.

## Seeing what comes in and goes out

The mixer's I/O view has a **Monitor** section: tap **I/O** on the **Mix** tab and pick **Monitor** in the list on the left. It changes no setting; it shows every MIDI message going in and out of the 5-pin jacks and USB devices as it happens.

- **The lights.** Each port has a row with a light for each of the 16 channels: the upper half lights green when something comes in on that channel, the lower half blue when something goes out. The last column shows the tempo of any clock coming in or going out.
- **The list,** newest at the top: when it happened, the port and which way (*in* or *out*, the outgoing ones in blue), the channel, the message in words (*Note on · C3 · vel 100*, *CC 1 Mod wheel · 64*, *Program 5*), where it went, and its bytes. Clock, active sensing and time code arrive many times a second, so they are counted rather than listed: one line when each starts and one when it stops. A SysEx message is one line with its length and, for well-known makers, the maker's name.
- **Where it went.** Each line says what took its message, so nothing that arrives goes unexplained:
  - *Bass, the armed track*: a key played the track on screen. *Drums, by its route*: a route in the Routing section played that track. *thru to MIDI out*: thru sent it straight on (*thru: not sent, MIDI out is off* while Prefs keeps MIDI out off). With a drum kit it says the pad the note plays (*Drums, pad 1*), or *nothing: no pad on Drums* for a note outside the 16 pad notes.
  - *Learn: Cutoff (Filter, Bass)*, in purple: a learned control moved that value. *Learn took it* marks the move that taught it.
  - *clock: the tempo follows it*, in green: the clock, Start, Stop or Song Position of the port the tempo comes from (Sync in the I/O view).
  - In yellow, what nothing took: *nothing takes it* (a knob nobody learned, a pitch bend the tracks don't take from a keyboard yet), *not played: Learn only* (the port's input is Learn only), *not followed* (clock on a port the tempo isn't from), *ignored: this port's setting* (switched off under **Ignore coming in** on [the port's page](#a-ports-page)), and *ignored* for messages MPFree never reads, such as SysEx.
  - Going out: *sent by Bass* names the track that played it, *thru from Keystep 37 in* what thru passed on, *MPFree's clock* marks the clock MPFree sends, and *sent from its page* marks a port page's test note and All notes off.
- **Picking what is listed.** The chips along the top switch each kind of message on and off (Notes, CC, Program, Bend & pressure, Clock, SysEx) and each way (In, Out). Tap a channel's light to list only that port, way and channel, or a port's name to list only that port and way; tap it again to list everything.
- **Looking back.** Turn the data wheel (or use the page arrows) to scroll back; the list holds still while you look. Scroll back to the top and it follows again. **Hold** stops the list until you tap it again, and **Clear** empties it.
- **Save log** keeps the list as it shows now (only what the chips and lights let through) in a text file on the stick's data partition, in `files/midi-logs/`, named by the date and time (`midi-log-2026-11-16-1958.txt`). The line under the screen says which file it wrote; a second save in the same minute gets `-2`, so a save never replaces a file. It is greyed when MPFree has no data partition to write to.

### Now: what each port holds this moment

**List | Now** at the top left switches the Monitor between the list and **Now**, which shows the state a port and channel are in rather than the messages that put them there. It picks the port and channel that sent something last; tap a port's chip to pick another, and turn the data wheel or tap ‹ › to change the channel.

- **Keys down:** a keyboard with each key held lit and its velocity on it, the notes with velocity and any key pressure, how long they have been held, whether the sustain pedal is down, and where they went.
- **Controls that moved:** each controller's last value, newest first, by its common name (*CC 74 Cutoff*), with how long ago it moved. A learned control says what it moves, in purple. The bank and program last chosen are at the top.
- **Clock:** the port's clock tempo, the gap between pulses and how much it wanders, the worst gap in the last minute, and how many pulses came late or were lost. Each bar is one second's worst gap; a late one is yellow.
- **Bend and pressure:** the pitch bend and channel pressure where they are now.
- **Notes still on:** the notes MPFree sent out of the 5-pin jacks or to a USB device that have not ended yet, how long each has been on and which track sent it. One left on for 30 seconds or more is marked stuck. **Send note-offs to MIDI out** ends only those notes (never an "all notes off" to the whole synth). It is greyed while MIDI out is off or no note is on.

**Hold** freezes the Now view so you can read it; tap it again to follow.

The Monitor shows the external ports only. With **Show developer tools** on in [Prefs](../reference/prefs.md), the Dev tab's MIDI page is the same monitor with more sources: the MPC's own pads, buttons, Q-Links and data wheel as MPFree read them, the button lights, pad colours, CV values and keep-alive MPFree sends to the MPC, and MPFree's own events (a USB device plugged in or unplugged, MIDI out to a port stopping). Its chips pick sources instead of kinds of message: Ports, MPC controls, Lights & colours, CV out, System, Clock and Keep-alive. Pad pressure is listed there only while **Pad pressure in the MIDI log** is on (MENU on the Dev tab).
