<!--
status: changing
covers: rust/mpfree-audiotracks rust/mpfree-time-ui/src/song.rs rust/mpfree-time-ui/src/widgets.rs rust/mpfree-app/src/audiotracks.rs
checked: 2026-11-22 9d8bc53c
-->

# Recording audio tracks

**Audio tracks** record what comes into the MPC's INPUTS (a vocal, a guitar, a synth) onto the song's timeline, next to your clips. Record a part a few times, then keep the best bits of each. Use them when a part is too long or too free to loop: a verse, a solo, a whole take of a song.

An audio track is a track of your project like any other: it sits in the track list with your other tracks, and **Mute** and **Solo** work on it as on any track. On the Arrange tab's **Song** view its lane sits in its place among your tracks, and that is where you record and edit it; the **Clips** grid shows it as a column too. Audio tracks play when the song plays (song mode on); with song mode off they stay silent, though an armed track still lets you hear its input.

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

Every audio track has a lane, in its place in the track list. To add another, tap **+ Track** after the last lane and pick **Audio track** (a project can have 8). **SHIFT + + Track** still adds a Loop track at once. When there are more lanes than fit, drag up or down on the names to scroll.

## In the Clips grid

On the Clips grid an audio track is a column like the others, with a wave mark and its input (**In L**) under its name. Tap its name to arm it, as for any track. Its cells are drawn dashed: audio clips aren't in the grid yet, so a tap on one tells you to record the track on the Song view. **+ Track** on the grid opens the same choice of Note track, Audio track or Loop track.

{{shot: arrange-clips-audio | The Clips grid with two audio tracks, Vocal and Guitar (armed), and + Track's choice of a kind of track open.}}

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
2. **From** names what the track records. Tap **‹** or **›** to step through the synths and kits that are playing (each named for what it is, such as **FM · Pads** or **Poly · Track 2**), then **Drums**, **Synth** and the **Loop** track, each submix, and **Master**.
3. **Effects**: **After** (the default) records the sound with its effects, **Before** without them. Either way the channel's fader doesn't change the recording.
4. Arm the track and record as usual: **REC**, then **PLAY**, and play.

The track records in stereo. It never plays the sound back while it records, so you don't hear it twice, and the take lines up with the song where you played it. Tap **L**, **R**, **L+R** or **ST** to record the INPUTS again; **Mix** brings back the sound you picked last. The project keeps what each track records from.

If the sound you picked goes away (its channel is taken out of the mix), the track records silence and the log says so.

## Editing regions

Tap a region to pick it. The buttons along the bottom become **Split**, **Erase**, **Fades**, **Gain**, **Takes** and **Undo** for that region; tap empty space to go back to the song's own buttons.

- **Split** cuts the region at the playhead, so put the playhead inside it first.
- **Erase** takes the region out.
- **Drag** a region sideways to move it. It snaps to sixteenth notes; hold **SHIFT** to move it freely. It stays picked where it lands.
- **Gain** and **Fades** are on the Q-Links. While a region is picked, the four knobs turn the track's **Level** (Q1) and **Pan** (Q2), which are its mixer strip's, and the region's **Gain** (Q3) and **Fades** (Q4, the fade in and the fade out together). Hold **SHIFT** for fine steps. A fade shows as a slope on the region.
- **Undo** takes back the last change, and **SHIFT + Undo** redoes what Undo took back.
- **Moving a section, + Bar and Delete bars** on the Song view move the regions with the song (see [Audio in the song](arranging.md#audio-in-the-song)). Undo here stops at such an edit and says so: **UNDO** takes it back, with the song, and **SHIFT + UNDO** puts it back.

## Comping with take lanes

Every recording on a track is kept as a **take**. Comping means recording a part a few times, then choosing the best bits of each, and take lanes are where you do it.

{{shot: arrange-song-takes | The vocal's take lanes: take 2 plays over bars 5 to 8 and take 1 everywhere else.}}

Tap **Takes** (in the track panel, or in the bottom row with a region picked). A thin lane opens under the track for each take, newest first, with its waveform drawn brighter where the track plays it now.

- **Swipe** across a take lane to play that stretch of the take on the track, in place of what was there. The swipe snaps to sixteenth notes; hold **SHIFT** for a free swipe.
- **Tap** a take lane to play that take everywhere it has audio.
- **Undo** takes each swipe back.

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

You don't have to save to keep a take safe. Each take is written to a file of its own on the drive while you record, a little at a time, named after its track: a take on **Vocal** becomes a file like `Vocal_1f3a…wav`. The file is made as soon as you arm the track, so if the drive can't take it (it's full, or not there), the track doesn't arm and the log says why. When you save the project, each take's file is copied into the project's `samples` folder as it is.

If mpfree stops while you record (a crash, or the power goes), the take is not lost. At the next start, mpfree finishes it up to the last moment that reached the drive (usually all but the last two seconds or so) and puts it in **Recordings** in the browser. Takes you recorded but never saved in a project go there too. The log says what was found, and the screen tells you when takes were recovered. To use one again, load it on the Sample tab and put it on an audio track, as in [A sound from the Sample tab](#record-your-first-take).

If a take's file runs into trouble while you record (the drive gets full or falls behind), the log says so in plain words. The take keeps recording into memory, and saving the project still keeps it.

> [!MPC]
> On the MPC, audio tracks sit in the same sequence as MIDI tracks and each has its own mixer channel. In MPFree they are in the same track list as your other tracks too, each with its own mixer strip, and you record them on the Song view's timeline, each in its own lane among your tracks.
