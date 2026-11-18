<!--
status: changing
covers: rust/mpfree-midi rust/mpfree-midi-ui/src/learn.rs rust/mpfree-midi-ui/src/port_page.rs rust/mpfree-app/src/midi_in.rs rust/mpfree-app/src/midi_out.rs rust/mpfree-app/src/cv_out.rs
checked: 2026-11-16 08282af
-->

# Playing other gear: MIDI, sync and CV

The **MIDI IN** and **MIDI OUT** jacks, USB keyboards and synths on the **USB** port and the **CV/Gate** outputs connect MPFree to other instruments. Playing MPFree from a keyboard and [MIDI Learn](#midi-learn) work now. Following another device's clock works too. Playing gear from **MIDI OUT**, from a USB device and from the CV outputs is built but not yet checked on an MPC One, so it stays off until you turn on **Send MIDI and CV out** in [Prefs](../reference/prefs.md). Its value reads **On (sends MIDI)** while it sends, and MPFree remembers it at the next start.

All of these settings are in one place: the mixer's [I/O view](mixing-and-effects.md#the-io-view), opened with the I/O icon at the right end of the top bar, in its **MIDI**, **Routing**, **Sync** and **CV** sections. A row whose part isn't working yet says so, and keeps what you set. Each port's input, what it ignores and its delay describe your gear, so they stay the same for every project: MPFree keeps them when it quits and puts them back at the next start (with a data folder on the stick). Everything else (what each track plays on, which channels play which tracks and which go thru under Routing, where the tempo comes from, clock out, the CV outputs and learned controls) belongs to the song: it is saved with the project and comes back when you open it, and a new project starts from the defaults.

## Playing MPFree from a keyboard

Plug a keyboard into **MIDI IN**, or a USB keyboard into the **USB** port, and it plays the armed track, just as the pads do (to play other tracks too, see [Playing several tracks from your keyboards](#playing-several-tracks-from-your-keyboards)). Its knobs, faders and pedals can turn effect knobs with [MIDI Learn](#midi-learn). It listens on every MIDI channel, and you can plug a USB keyboard in or out while MPFree runs. A USB interface with several MIDI ports shows each one as a port of its own, named as the device names it (up to 15 USB ports at once). If a keyboard sends **All notes off** (or **All sound off**), the keys it holds on that channel let go.

- **With a synth on the pads,** each key plays its own note, through the arpeggiator, so **NOTE REPEAT** and Latch work from the keyboard too. A note held on a key and a pad ends when both let go.
- **With a drum kit,** the keys that match the MPC's own pad notes hit the pads: C#2 is pad 1, C2 is pad 2, and so on. Other keys play nothing.
- The pad velocity setting, and Full Level, shape the keys' velocity as they do the pads'.

## Playing several tracks from your keyboards

{{shot: mixer-io-routing | The I/O view's Routing section: channel 10 of MIDI IN plays Drums, whichever track is armed, and the rest of its notes play the armed track. The keyboard's channel 3 goes thru to MIDI out; with MIDI out off in Prefs, its square says so.}}

The I/O view's **Routing** section shows where each source's MIDI goes, on one grid. Down the side are your ports (the 5-pin **MIDI in** and each USB device), **The pads**, and the tracks. Across the top: **Armed track**, **Tracks by channel**, **Learn**, each MIDI out port and **CV out**. A square with a green bar plays something in MPFree; a blue bar sends something out. **+** means a tap sets it up, **—** means nothing can go there, and **later** marks what isn't built yet.

- **Tracks by channel** sends one channel of a port to one track, whichever track is armed. Tap a port's square and pick the channel and the track on the card below the grid (with – and +, or Q-Links 1 and 2). **+ Route** adds another, **Remove route** takes the picked one away. Two routes from the same channel to two tracks play both, to layer sounds. For example: a drum machine on **MIDI IN**, channel 10, plays **Drums**, while a keyboard on channel 1 plays **Bass** and you work on another track with the pads.
- Notes a route takes go only to its track, never to the armed track as well, so nothing plays twice. The rest of the port's notes go to the armed track, as before.
- A track played by a route plays where its clips play: its own sound, its synth or drum machine through MIDI out, and its CV outputs. With a drum kit, the keys that match the MPC's pad notes hit the pads. Only the armed track's notes go through the arpeggiator, and only the armed track records.
- **Armed track** and **Learn** are the port's **Input** (Both, Plays tracks or Learn only), the same setting as on its row in the MIDI section. Turning both off isn't allowed: turn the other on first. A port set to **Learn only** still plays its routes.
- A track's square under a port is its **Plays on**, as in the MIDI section: tap it to play that track on that port, and pick the channel on the card.
- Routes belong to the song and are saved with it. A route from a device that isn't plugged in waits for it.
- Up to 32 routes in a project.

Not yet: the pads to an output, and the blocks between a source and where it goes (an arpeggiator or a transpose on a route). Their squares say **later**.

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

## A port's page

Each port's row in the MIDI section has **Open ›** at its right; it, or a tap on the port's name, opens the port's page. **‹ MIDI** or **BACK** closes it.

- **This port** says what it is (the MPC's 5-pin jacks, or a USB device), its **Name** and its **Input**: **Both**, **Plays tracks** or **Learn only**, the same setting as on its row.
- **Name** gives a USB device a name of your own, such as *Juno* for a box that calls itself *USB MIDI Interface*. A tap (or +) opens the on-screen keyboard; type the name and tap **Enter**. From then on MPFree uses it everywhere it names the port: its row, Routing, a track's **Plays on**, Sync, Learn, the Monitor and the tempo chip. The page still says the name the device calls itself. A name has at most 24 characters, and two ports can't share one (whatever the capitals). Type the device's own name to go back to it. The name belongs to your setup, not the song, so every project uses it. The 5-pin jacks keep the names on the MPC's panel, **MIDI In** and **MIDI Out**.
- **Ignore coming in** has four switches: **Clock**, **Program**, **Aftertouch** and **All notes off**. A lit one is ignored: those messages from this port play nothing, teach nothing and move no tempo. The Monitor still lists them, marked *ignored: this port's setting*, so you can see they arrive. **Clock** also covers the Start, Stop and Song Position that come with it. Ignore **Clock** on a drum machine that sends clock you don't want; ignore **Program** on a keyboard that sends one when you change its sound.
- **What it does** lists, in words, what this port's messages do today and what plays on it: its routes to tracks (*Ch 10 → Drums*) and its thru (*Ch 3 → MIDI out, thru*), where the rest of its notes go, the tracks that play out of it with their channels, thru from other ports, the controls learned from it, and whether the tempo follows it. Change its routes in the Routing section.
- **Now** shows the newest message in and out and how long ago.
- **Clock to this port** is **Off**, **While playing** or **Always**: the same setting as **Clock out** under Sync, saved with the project.
- **This gear's delay** is how late the gear on this port sounds after MIDI reaches it, in steps of 0.5 ms (– and +, or Q-Link 3). MPFree sends that port's notes and clock that much early, so they land on the beat. It can send up to about 9 ms early (the sound's buffer, the time between MPFree making a sound and you hearing it); the card says when the delay is more than that.
- **Send to it** (marked **SENDS MIDI**): **Test note** plays C4 for half a second on the first channel a track plays this port on (else channel 1), to check the cable and the synth's channel. **All notes off** ends MPFree's own notes on this port (other ports' notes go on), then sends All notes off (CC 123) on all 16 channels, for a synth that hangs. Both are greyed while **Send MIDI and CV out** is off, and on a port that isn't plugged in, has no output or whose output didn't open; the greyed line says which.

The wheel picks a setting on the page, – and + change it, and Q-Links 1 to 3 turn **Input**, **Clock** and the delay.

## Staying in time with other gear

MPFree can follow another device's MIDI clock. In the mixer's I/O view, under **Sync**, set **Tempo from** to the port the device is plugged into:

- Its tempo becomes MPFree's and moves when it moves. The top bar shows a tempo chip such as **120.0 from MIDI In**, with a green dot, until its clock stops.
- With **Start and stop** on, its Start and Stop start and stop MPFree.
- With **Position** on, MPFree plays from the same bar as the device, and goes back in step if it drifts away.
- If its clock stops arriving, MPFree keeps playing at the last tempo.

MPFree can also send its own clock from **MIDI OUT**, or to a USB device, so a drum machine or an arpeggiator plays in time with it. Each port's **Clock out** is **Off**, **While playing** or **Always**, and each port with a delay gets its clock that much early.

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

Learned controls kept in **Project** are saved with the song; those kept in **Rig** stay when you open another project and come back at the next start (with a data folder on the stick). Still to come: learning the values on the Play page, the Pads and Sample tabs, the arpeggiator and a plugin's own page; recording a learned synth knob's moves into a clip; and launching clips and scenes from a controller.

### Playing and stopping from your controller

A button, pad, key or footswitch on your controller can press MPFree's transport buttons, and a knob or fader can set the tempo. In the Learn section, tap **Transport and tempo** by the heading (the card shows them straight away while nothing is learned yet), tap one, then press the button or move the knob:

- **PLAY and STOP**: one button for both. It plays when MPFree is stopped and stops it when it plays. A key or a pad flips it on each press; a footswitch that sends a CC plays while held, so set its **Works as** to **Button: on, then off**.
- **PLAY**, **STOP**, **PLAY START**, **REC**, **OVERDUB** and **TAP TEMPO**: each does just what the MPC's own button does on the page you're on: PLAY while playing is FILL, and tapping TAP TEMPO in time sets the tempo. A press counts when the control goes past halfway and lets go when it comes back, so leave these as **Button: while held** or **Knob or fader**.
- **Tempo**: from 30 to 300 BPM over the control's travel. Narrow it with **Range**, such as 110 to 130 for a fader that nudges, and use **Pick up** so the tempo doesn't jump when you first touch the fader. While the tempo follows another device's clock (Sync's **Tempo from**), the learned tempo control does nothing.

For the tempo the quick way works too: hold **SHIFT** and touch the tempo on the Play page. To mute a track from a controller, hold **SHIFT** and touch its **M** in the mixer, then press a key: each press mutes or unmutes it.

Transport controls are kept in **Rig**, so they work in every project. Nothing here sends anything to your controller.

## MIDI files

A MIDI file from another sequencer, a computer or an MPC loads from BROWSE as clips, with each note on the pad that plays it: see [Bringing in a MIDI file](clips.md#bringing-in-a-midi-file). **Export** makes a MIDI file of your song for a computer: see [Exporting your music](exporting.md).

## CV and gate

The **CV/Gate** jacks play modular synths. Each of the eight outputs has one job:

- a track's **pitch**, 1 V per octave (or Hz per volt, for older Korg and Yamaha synths), with the lowest, highest or last note winning when several are held;
- a track's **gate**, high while a note is held, or a short **trigger** on each note (of every note, or of one note, for a drum module);
- a track's **velocity**, or one of its **CCs**, as a voltage;
- a **clock** at the rate you pick, **run** (high while the song plays) or **reset** (a pulse when the song starts from the top);
- or a **fixed** voltage.

The outputs are set in the mixer's I/O view, under CV. A track that drives CV still plays its own sound too, so turn it down in the mixer if you only want the module.

When one note ends just as the next starts, as a sequence's notes do, the gate drops for 2 ms between them, so the module's envelope starts again. Hold two notes at once for a legato line instead.

CV 1 to 4 send while **Send MIDI and CV out** is on in Prefs. They haven't been checked on an MPC One with a meter yet, so treat their volts as a first guess; CV 5 to 8 come after that check. Until an output works, its row in the I/O view says it sends nothing. Modules differ, so each output will also be tunable against your module.

## Seeing what comes in and goes out

The mixer's I/O view has a **Monitor** section: open the mixer with the I/O icon and pick **Monitor** in the list on the left. It changes no setting; it shows every MIDI message going in and out of the 5-pin jacks and USB devices as it happens.

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
