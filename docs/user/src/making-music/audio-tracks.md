<!--
status: changing
covers: rust/mpfree-audiotracks rust/mpfree-time-ui/src/song.rs rust/mpfree-time-ui/src/widgets.rs rust/mpfree-app/src/audiotracks.rs
checked: 2026-11-24 dad871b0
-->

# Recording audio tracks

**Audio tracks** record what comes into the MPC's INPUTS (a vocal, a guitar, a synth) onto the song's timeline, next to your clips. Record a part a few times, then keep the best bits of each. Use them when a part is too long or too free to loop: a verse, a solo, a whole take of a song. An audio track is also where your **loops** live: each loop sits in one of its cells and plays through its channel (see [Loops on audio tracks](looping.md#loops-on-audio-tracks)).

An audio track is a track of your project like any other: it sits in the track list with your other tracks, and **Mute** and **Solo** work on it as on any track. On the Arrange tab's **Song** view its lane sits in its place among your tracks, and that is where you record and edit it; the **Clips** grid shows it as a column too, whose cells hold **audio clips** you launch like any clip. An audio track plays its launched clip, and otherwise, when the song plays (song mode on), its song; with neither it stays silent, though an armed track still lets you hear its input.

{{shot: arrange-song-audio | The Song view with two audio lanes: a vocal recorded twice, its newer take picked, and a guitar armed.}}

## Record your first take

1. Plug the instrument or microphone preamp into the **INPUTS** and set **REC VOL**.
2. Turn on **Prefs > Line input** (**SHIFT + MENU**). The input is off every time MPFree starts.
3. On Arrange, open the Song view (**Song ›**) and turn song mode on.
4. Tap **+ Track** after the last lane and pick **Audio track**. It joins the end of the track list as **Audio 1**, armed (its arm dot is red), and its lane comes into view. **Undo** takes it away again.
5. Play a little and watch the track's level bar: it should move well up but not turn red.
6. Press **REC**, then **PLAY**, and play. Press **STOP** when you're done.

The take shows as a **region**: a block in the track's colour with its waveform. While a track records, a red block grows from where it started.

**A sound from the Sample tab.** A sample can go on an audio track too: on the Sample tab's **Slice** tool, set **Put on** to **Audio track** and tap **To audio track**. It goes on the first empty track from the bar the playhead is in, each slice a region of its own (see [Audio track: the slices on the song](sampling.md#audio-track-the-slices-on-the-song)). **Undo** here takes it off.

## The lanes

On the left of each audio lane are:

- the **arm dot**, red while the track is armed: tap it to arm or disarm;
- the track's **name**, and its **input** (**L** to begin with);
- a thin **level bar** along the bottom, showing what the track plays and, while it is armed, the input it records. It turns red near full scale.

Every audio track has a lane, in its place in the track list. To add another, tap **+ Track** after the last lane and pick **Audio track** (a project can have 8), or hold **SHIFT** and tap **+ Track** to add one at once. When there are more lanes than fit, drag up or down on the names to scroll.

## In the Clips grid

On the Clips grid an audio track is a column like the others, with a wave mark and its input (**In L**) under its name. Tap its name to arm it, as for any track. **+ Track** on the grid opens the same choice: **Note track** (plays a kit or an instrument and records notes) or **Audio track** (records sound from an input: loops in its cells, takes on the song, with its own mixer channel). **SHIFT + + Track** adds an audio track at once, and **UNDO** takes a new track away.

An audio track's cells hold **audio clips** of your takes (below) and **loops** you record into them (see [Loops on audio tracks](looping.md#loops-on-audio-tracks)). A loop cell shows **⟳** and the loop's name, where a clip shows a small wave.

### The input list

Tap the input under an audio track's name on the Clips grid (or the input chip on its lane in the Song view) to open its **input list**. One input serves the track's loops and its takes alike:

- **Line**: **L**, **R**, **L+R (mono)** or **Stereo**, from the INPUTS.
- **Pads**: the instrument the pads play, after its effects and fader, with its clips ("Pads · Kit 1, with its clips").
- **Mix ›** a strip: a track or a submix, after its fader, as the Mix tab names it.
- **Mix › Master**: all you hear but this track, before the master's own effects.

An input the mixer can't give this track is dim, and the line under its group says why ("Vocal plays into Master through Bus A: pick another input, or take Vocal's send off Bus A"): a track can't record itself. Tap an input to pick it; the list closes when you tap elsewhere or change page. A loop takes the input its track has when its first pass starts, and each overdub the input it has then.

{{shot: arrange-clips-audio | The Clips grid with two audio tracks, Vocal and Guitar (armed), and + Track's choice of a kind of track open.}}

### Audio clips

An audio track's cells hold **audio clips**: a stretch of your recordings that you launch from the grid, loop or play once, as you would a clip of notes. Tap one and it starts on the next Launch line; scenes, a row's number, **Stop all** and follow actions work on audio clips too. The song itself doesn't change: launched, the clip plays in place of the track's song until you stop it or go **Back to song**.

There are two ways to make one:

- **To a cell**, on the Song view: pick a region and tap **To a cell**. The region goes in its track's first empty cell, top down, as a clip as long as the region, named after the track and its bars ("Vocal 5–8"). The hint line says where it went ("Vocal, bars 5 to 8: in row 3 · UNDO takes it out"). If the track has no empty cell, nothing happens and the hint line says so.
- **Put in a cell**, on the Sample page: see [Audio track: the slices on the song](sampling.md#audio-track-the-slices-on-the-song).

An empty audio cell is drawn dashed; a tap on it says how to fill it.

{{shot: arrange-clips-audio-cells | Audio clips at 92 BPM: Hook vox playing, Ad libs and Outro waiting to start, Riff made at 120, Solo missing its audio, Intro still ringing past its stop, and Amen not ready.}}

**What a cell says.** Each audio cell shows the clip's name and a small wave of one pass. Playing, the wave lights up to the playhead. Under the name, in the wave's place while the clip is stopped or waiting (and while it plays, for the words about its tempo), a word says what matters, and a tap on the cell says it in full on the hint line:

| Word | What it means |
|---|---|
| **from 17.1** (amber) | Launched: it starts at bar 17. |
| **waits 18.1** (amber) | Its audio is still loading from the drive, so it starts a little later, on bar 18, in time with the bar. |
| **made at 120** (amber) | It was made at 120 BPM, its **Tempo** is **Off**, and the project plays at another tempo, so each pass leaves a gap or cuts its end off. **Repitch** or **Stretch** on the Clip tab keeps it on the bars (below). |
| **not ready** (red) | Its audio couldn't be loaded in time, or on **Stretch** with **Until ready** set to **keep pitch** its copy wasn't made in time, so the launch was cancelled. Tap it to try again. |
| **missing** (red) | A recording it plays wasn't found when the project opened. It can't play. |
| **copy 40 %** (blue, a line filling along its foot) | On **Stretch**, its copy for the tempo now is being made (0 % while it waits its turn behind others). |
| **waits copy** (amber) | Launched on **Stretch**, it moved to a later line to wait for its copy. On **on time** it starts there anyway, on Repitch if the copy still isn't ready; on **keep pitch** it is cancelled rather than start out of pitch. |
| **repitch now** (amber) | Playing on **Stretch** before its copy was ready, so this pass plays faster and higher (or slower and lower), like tape. Its own pitch comes back at the start of its next pass once the copy is ready. |
| **rejoins 9.1** (amber) | On **keep pitch**, the tempo changed while it played: it plays on in its own pitch, off the bars, and comes back onto them at bar 9 once its new copy is ready (it says **copy 40 %** until then). |
| **too fast** (red) | It would play more than 4 times its speed at this tempo, so it's silent. Check the tempo it was made at on the Clip tab. |

{{shot: arrange-clips-audio-copies | Audio clips at 92 BPM following the tempo: Hook vox playing Repitch until its copy is made, Riff rejoining the bars at bar 17, Outro's copy 40 % made, Amen waiting for its copy, Intro not ready, Pad wash too fast.}}

A clip on **Stretch** that plays its copy, or any clip at its own tempo, shows its wave and no word. If a copy couldn't be made (there was no room for it), the clip plays Repitch; its cell has no word, and a tap on it says so. How much room the copies take, and **Clear copies** to free what isn't playing, are on System status' Memory ([When something goes wrong](../reference/when-something-goes-wrong.md#the-system-status-page)).

A clip set to play **once** shows its playhead dimmed while its sound rings on after its track stopped, so the grid never looks silent while you hear it. In Perform the cell's foot gives its length ("2 bars"), or a once clip's time ("3:17 once").

**Moving and copying.** Drag an audio clip to another row or another audio track; **SHIFT** as you lift makes a copy, which plays the same recordings. An audio clip can't go on a note track (nor a clip of notes on an audio track): the hint line says why. **COPY** + a clip and an empty audio cell works too.

### Audio clips in the song

With **Rec song** on, an audio clip you launch goes into the song as a block on its track's lane, as a clip of notes does (see [Recording a performance into the song](arranging.md#recording-a-performance-into-the-song)). In song mode it plays from the song just as you heard it from the grid.

{{shot: arrange-song-audio-blocks | Two blocks of audio clips on Guitar's lane after its recording: Riff, two bars played twice (picked), and Outro, a once clip.}}

- A block of an audio clip has a strip along its top in the track's colour with the clip's name, and its wave under it, drawn again each time the clip goes round, with a faint line where each pass starts. A recorded region has no strip, so you can always tell them apart. A once clip says so ("Outro · once").
- A block and a region never overlap on a lane. Where Rec song puts a block over recorded regions, the block replaces them, and **UNDO** brings them back.
- Dragging a block over a region, or making it longer over one, is refused, and the hint line says why ("Riff can't go over the region at bar 9: a block and a region never overlap on a lane"). While you drag, its outline turns red where it can't go.
- Recording over a block cuts the block where the take starts and ends. So does swiping a take lane, dragging a region or bringing a sound in over it. **UNDO** takes back the recording and the cut together.
- Drag a block to another audio track's lane to play it through that track. An audio clip's block can't go on a note track's lane, nor a clip of notes on an audio track's. A loop's block stays on its own track's lane (see [Loops in the song](looping.md#loops-in-the-song)).

### The Clip tab for an audio clip

Arm an audio track (its name on the grid, or its arm dot on the Song view) and the Record panel's **Clip** tab shows its playing clip, else its first, until you arm a note track again.

{{shot: arrange-clips-audio-card | The Clip tab for Riff, made at 120 BPM in a project at 92, on Off: its rows, the gap it leaves each pass, and From its length.}}

| Row | What it does |
|---|---|
| **Length** | A loop's length in bars and beats. **-** and **+** step a beat, **SHIFT** a bar. Shorter, the clip stops its sound there; longer again, what was cut comes back, and past its sound it is silent. A once clip shows its time (3:17) and doesn't step. |
| **Tempo** | What the clip does when the song plays at another tempo than its audio was made at. **Off**: as recorded, each pass at its own speed from its start (so it can leave a gap or cut its end). **Repitch**: on its bars, faster and higher (or slower and lower), like tape. **Stretch**: on its bars at its own pitch, from a copy of its audio made for that tempo. Where the song has a tempo point coming up, the copy for the tempo after it is made ahead, and the clip changes to it right on the point, in the middle of a pass if need be. Through a ramp it plays **Repitch**, and keeps its pitch again where the ramp ends. Every new clip starts on **Off**: it follows the tempo only when you set it here. If nothing says what tempo its audio was made at, it stays **Off** until you set that too (below). |
| **Until ready** | Only with **Stretch** (otherwise "—"): what the clip does while its copy for a new tempo is still being made. **on time** (the default) plays it **Repitch** meanwhile, on the bars, and it comes back to its own pitch at the start of its next pass. **keep pitch** never plays it off pitch: a launch waits for its copy, for 2 launch lines at most, then is cancelled; a clip already playing when the tempo changes plays on in pitch and rejoins the bars on the next bar line once its copy is ready. |
| **Mode** | **Restart**, **Legato** or **Once**: as for a clip of notes. A once clip plays its sound to the end. |
| **Launch**, **Follow**, **After** | As for a clip of notes. |
| **Gain** | The clip's own level, from off to +12 dB, in 0.5 dB steps (**SHIFT**: 0.1 dB), on top of each region's gain. |
| **Name** | Tap it to rename the clip on the keyboard. |

Each row you turn is one step **UNDO** takes back.

**The tempo it was made at.** Under the rows, two lines say the tempo the clip's audio was made at and what its **Tempo** does with it at the tempo now:

- "Made at 120 BPM" over "Stretch: the same pitch at 92" or "Repitch: 4.6 semitones down" (green).
- On **Off**, "Made at 120 BPM:" over "at 92 a 1.2 s gap each pass" (amber).
- While its copy is being made, "Making its copy: 40 %" over "Repitch for now: 4.6 down" (on time) or "Waits for its copy" (keep pitch), with a line filling under them. You can keep working; it is made in the background.
- "Made at 120 and 100 BPM" for a clip made of two takes recorded at different tempos: each part keeps its own, so both play on their beats.
- "Made at: not known" (amber) when nothing says, such as a take recorded while the tempo changed.
- Faster than 4 times its own speed, a clip on **Repitch** or **Stretch** is silent, and the lines say so in red.

{{shot: arrange-clips-audio-stretch | Riff on Stretch at 92 BPM while its copy is made: Repitch for now, 4.6 semitones down.}}

**Setting it.** Tap the two lines to pick them: the wheel then sets the tempo the audio was made at, 0.1 BPM a step (**SHIFT**: 1), and **-** and **+** do the same. **From its length**, the button under them, works it out from the clip's length in bars instead: a 2-bar loop whose sound lasts 4 seconds was made at 120 BPM. Either is one step **UNDO** takes back. From its length is grey for a once clip, for a clip made of takes at two tempos, and when the clip's length already fits its sound; a tap then says why.

{{shot: arrange-clips-audio-made-at | The two lines picked: the wheel sets the tempo Riff was made at.}}

## The track panel

Tap a track's name (or its input) to open its **track panel** over the lanes. Tap the name again, or **Close**, to shut it.

{{shot: arrange-song-panel | The guitar's track panel: armed, soloed and set to stereo, with its level and pan on the Q-Links.}}

| Setting | What it does |
|---|---|
| **Arm**, **Mute**, **Solo** | Mute and Solo are the track's own, the same as **TRACK MUTE** and the mute and solo anywhere else: soloing an audio track silences every other track, the drums and synths too, and soloing any other track silences the audio tracks that aren't soloed. |
| **Input** | What the track records. **L** (the left jack, the default), **R**, **L+R** (both jacks mixed into one), **ST** (stereo) or **Mix** (a sound from inside mpfree: see [Record what mpfree plays](#record-what-mpfree-plays)). It can't change while the track records. |
| **Monitor** | When you hear the input through the track. **Auto** (the default) plays it while the track is armed, except during playback. **In** always plays it, armed or not. **Off** never does: use it when you hear your instrument another way, such as through a mixer. With **Mix** picked the panel says **Not played through** here instead: you already hear that sound. |
| **Level**, **Pan** | The track's own mixer strip's fader and pan, with a step either way on screen. Q-Links 1 and 2 turn them too while the panel is open (their cells say **its strip**). Turned here or on the **Mix** tab, it is the same fader and pan. |
| **Its strip in Mix ›** | Opens the **Mix** tab's tracks with this track's strip picked, for its effects, sends and output. |
| **Takes** | Opens the track's take lanes (below). |

## Record what mpfree plays

An audio track can also record a sound from inside mpfree: a synth, a kit, a submix or the whole mix. Use it to turn a synth part into audio you can cut up and comp, or to print a performance with its effects.

{{shot: arrange-song-panel-mix | The vocal's track panel recording FM, the synth the pads play, after its effects.}}

1. Open the track panel and tap **Mix** in the **Input** row. A row more appears.
2. **From** names what the track records. Tap **‹** or **›** to step through the synths and kits that are playing (each named for what it is, such as **FM · Pads** or **Poly · Track 2**), then **Drums**, **Synth**, each submix, and **Master**.
3. **Effects**: **After** (the default) records the sound with its effects, **Before** without them. Either way the channel's fader doesn't change the recording.
4. Arm the track and record as usual: **REC**, then **PLAY**, and play.

The track records in stereo. It never plays the sound back while it records, so you don't hear it twice, and the take lines up with the song where you played it. Tap **L**, **R**, **L+R** or **ST** to record the INPUTS again; **Mix** brings back the sound you picked last. The project keeps what each track records from.

If the sound you picked goes away (its channel is taken out of the mix), the track records silence and the log says so.

## Editing regions

Tap a region to pick it. The buttons along the bottom become **Split**, **Erase**, **Fades**, **Gain**, **Tempo**, **Takes** and **To a cell** for that region; tap empty space to go back to the song's own buttons. (**To a cell** makes the region an audio clip: see [Audio clips](#audio-clips). **Tempo** is below: [A region's tempo](#a-regions-tempo).)

- **Split** cuts the region at the playhead, so put the playhead inside it first. Nothing you hear changes: a fade the cut goes through carries on across it, each part showing its share of the slope.
- **Erase** takes the region out.
- **Drag** a region sideways to move it. It snaps to sixteenth notes; hold **SHIFT** to move it freely. It stays picked where it lands.
- **Gain** and **Fades** are on the Q-Links. While a region is picked, the four knobs turn the track's **Level** (Q1) and **Pan** (Q2), which are its mixer strip's, and the region's **Gain** (Q3) and **Fades** (Q4, the fade in and the fade out together). Hold **SHIFT** for fine steps. A fade shows as a slope on the region. On a part cut from a longer region (by **Split**, a punch in, or moving the song's bars), the fades you set are its own, at its own ends.
- **UNDO** takes back the last region change, and **SHIFT + UNDO** redoes what UNDO took back, as long as region changes are what you did last. After anything else (To a cell, a clip changed) UNDO takes that back first.
- **Moving a section, + Bar and Delete bars** on the Song view move the regions with the song (see [Audio in the song](arranging.md#audio-in-the-song)). **UNDO** takes such an edit back with the song, after any region changes made since, and **SHIFT + UNDO** puts it back.

### A region's tempo

A region on the song plays as it was recorded: if you change the song's tempo, a region longer than a beat or two drifts off the bars. Like an audio clip (see [The Clip tab for an audio clip](#the-clip-tab-for-an-audio-clip)), a region can follow the tempo instead, each region with its own setting.

Pick the region and tap **Tempo**. A small card opens over the lanes with the region's name, take and bars, and three rows:

| Row | What it does |
|---|---|
| **Tempo** | **Off**: as recorded. **Repitch**: on its bars, faster and higher (or slower and lower), like tape. **Stretch**: on its bars at its own pitch, from a copy of its audio made for the song's tempo there; where the tempo changes under the region (a tempo point or a ramp), its copy follows the change, so it keeps its pitch right through it. Repitch and Stretch need the tempo it was made at (the third row). |
| **Until ready** | Only with **Stretch** (otherwise "—"): **on time** plays it Repitch while its copy is made; **keep pitch** never plays it off pitch: through a tempo change it plays on in its own pitch, off the bars, and rejoins them once its copy is ready. |
| **Made at** | The tempo its audio was made at: its take's, or "not known" (amber), for instance a take recorded while the tempo changed. |

Under the rows a line says what it does at the song's tempo there ("Stretch: the same pitch at 92", "Repitch: 4.6 semitones down", "Off: off the bars at 92", or "Making its copy: 40 %" while its copy is made). The wheel picks a row, and **-** and **+** change it (**SHIFT**: **Made at** a whole BPM); you can also tap a row's **‹** and **›**. **From its length** sets **Made at** from the region's length in whole bars: a region 4.1 bars long at 120 BPM was made at 117.1, so it fills exactly 4 bars. Each change is one step **UNDO** takes back. Tap **Tempo** again, or anywhere off the card, to close it.

{{shot: arrange-song-region-tempo | The vocal's take 2 at 92 BPM: made at 120, on Stretch, its card open. The guitar's recording on Repitch carries its tag.}}

A region on **Repitch** or **Stretch** carries a small tag after its name. **Split**, **Takes** and **To a cell** keep a region's setting: a part comped from another take keeps the setting of what it plays over, at its own take's tempo.

## Comping with take lanes

Every recording on a track is kept as a **take**. Comping means recording a part a few times, then choosing the best bits of each, and take lanes are where you do it.

{{shot: arrange-song-takes | The vocal's take lanes: take 2 plays over bars 5 to 8 and take 1 everywhere else.}}

Tap **Takes** (in the track panel, or in the bottom row with a region picked). A thin lane opens under the track for each take, newest first, with its waveform drawn brighter where the track plays it now.

- **Swipe** across a take lane to play that stretch of the take on the track, in place of what was there. The swipe snaps to sixteenth notes; hold **SHIFT** for a free swipe.
- **Tap** a take lane to play that take everywhere it has audio.
- **UNDO** takes each swipe back.

When an older take plays, its region on the track says which one ("take 1"). Tap **Takes** again to close the lanes.

## Timing, memory and the mixer

- **Latency.** Sound takes a few milliseconds to come in through the inputs, so a take would land slightly late. Audio tracks make up for it with **Line input latency** in [Prefs](../reference/prefs.md), so a take lines up with the beat you played it against. It's the same setting as the Looper's **Latency to make up for**: change either and both follow.
- **Memory.** The audio tracks share one minute of stereo memory to begin with. The Song view's legend says how much is free ("Audio 0:42 of 1:00 free"). **Audio track memory** in [Prefs](../reference/prefs.md) sets 30 seconds to 10 minutes, from the next start. A recording that runs out of memory stops there and keeps what it got.
- **The mixer.** Each audio track plays through a mixer strip of its own (**TRACK MIX**), named after the track, in its colour, with a wave mark and its input (**In L**, **Mix**) by the name. Add effects and sends there, or send it to a submix. The strip's **M** and **S** are the track's own **Mute** and **Solo**, so a solo there plays this track alone among all your tracks. A strip has no freeze: an audio track already plays recorded sound. To give several audio tracks the same effects, send their strips to one submix.
- **Projects from before.** A project saved with audio tracks before they became tracks in the track list doesn't open in this mpfree, and the log says why. If an older project's shared **Audio** strip had effects or settings, they show on a strip called **Audio (old)**, which nothing plays through any more.
- **Projects from another sample rate.** A project whose audio tracks were recorded at another sample rate (48 kHz on a computer, say, opened on the MPC at 44.1 kHz) still plays at its own speed and pitch. As it opens, mpfree converts each take to the rate it runs at, with the same high-quality converter the Sample tab uses, and the log says so. Opening takes a moment longer, and the next save keeps the converted takes.

{{shot: mixer-audio-tracks | The Mix tab's tracks with two audio tracks, Vocal and Guitar, each on its own strip with a wave mark and its input; the guitar is picked, 3 dB down and panned right.}}

## Saving

The audio tracks are saved with the project (**SHIFT + BROWSE**): every take, the regions you comped from them, and each track's input, monitor, mute and solo; its level, pan and effects are saved with its mixer strip. If a track is recording when you save, the save waits until it stops and its take is safely on the drive. Opening a project ends any recording and brings in the project's audio tracks; **New project** empties them. See [Saving your work](saving.md).

### Every take goes to the drive as you record

You don't have to save to keep a take safe. Each take is written to a file of its own on the drive while you record (the SD card in the MPC's slot when it is open for recording, otherwise the USB stick: see [Recording on the card](saving.md#recording-on-the-card)), a little at a time, named after its track: a take on **Vocal** becomes a file like `Vocal_1f3a…wav`. The file is made as soon as you arm the track, so if the drive can't take it (it's full, or not there), the track doesn't arm and the log says why. When you save the project, each take's file is copied into the project's `samples` folder as it is.

If mpfree stops while you record (a crash, or the power goes), the take is not lost. At the next start, mpfree finishes it up to the last moment that reached the drive (usually all but the last two seconds or so) and puts it in **Recordings** in the browser. Takes you recorded but never saved in a project go there too. The log says what was found, and the screen tells you when takes were recovered. To use one again, load it on the Sample tab and put it on an audio track, as in [A sound from the Sample tab](#record-your-first-take).

If a take's file runs into trouble while you record (the drive gets full or falls behind), the log says so in plain words. The take keeps recording into memory, and saving the project still keeps it.

> [!MPC]
> On the MPC, audio tracks sit in the same sequence as MIDI tracks and each has its own mixer channel. In MPFree they are in the same track list as your other tracks too, each with its own mixer strip, and you record them on the Song view's timeline, each in its own lane among your tracks.
