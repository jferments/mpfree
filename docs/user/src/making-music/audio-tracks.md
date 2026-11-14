<!--
status: changing
covers: rust/mpfree-audiotracks rust/mpfree-time-ui/src/song.rs rust/mpfree-time-ui/src/widgets.rs rust/mpfree-app/src/audiotracks.rs
checked: 2026-11-14 b76c3c7
-->

# Recording audio tracks

**Audio tracks** record what comes into the MPC's INPUTS (a vocal, a guitar, a synth) onto the song's timeline, next to your clips. Record a part a few times, then keep the best bits of each. Use them when a part is too long or too free to loop: a verse, a solo, a whole take of a song.

They live on the Arrange tab's **Song** view, under the clip lanes. Audio tracks play when the song plays (song mode on); with song mode off they stay silent, though an armed track still lets you hear its input.

{{shot: arrange-song-audio | The Song view with two audio lanes: a vocal recorded twice, its newer take picked, and a guitar armed.}}

## Record your first take

1. Plug the instrument or microphone preamp into the **INPUTS** and set **REC VOL**.
2. Turn on **Prefs > Line input** (**SHIFT + MENU**). The input is off every time MPFree starts.
3. On Arrange, open the Song view (**Song ›**) and turn song mode on.
4. Tap **+ Audio** at the bottom of the lanes to add a track, then tap its **arm dot** so it turns red.
5. Play a little and watch the track's level bar: it should move well up but not turn red.
6. Press **REC**, then **PLAY**, and play. Press **STOP** when you're done.

The take shows as a **region**: a block in the track's colour with its waveform. While a track records, a red block grows from where it started.

## The lanes

On the left of each audio lane are:

- the **arm dot**, red while the track is armed: tap it to arm or disarm;
- the track's **name**, and its **input** (**L** to begin with);
- a thin **level bar** along the bottom, showing what the track plays and, while it is armed, the input it records. It turns red near full scale.

A lane shows once its track is armed or has audio on it. **+ Audio** adds the next track, up to 8. When there are more lanes than fit, drag up or down on the names to scroll.

## The track panel

Tap a track's name (or its input) to open its **track panel** over the lanes. Tap the name again, or **Close**, to shut it.

{{shot: arrange-song-panel | The guitar's track panel: armed, soloed and set to stereo, with its level and pan on the Q-Links.}}

| Setting | What it does |
|---|---|
| **Arm**, **Mute**, **Solo** | Solo works among the audio tracks: a soloed track silences the other audio tracks, not the drums or the synth. |
| **Input** | What the track records. **L** (the left jack, the default), **R**, **L+R** (both jacks mixed into one) or **ST** (stereo). It can't change while the track records. |
| **Monitor** | When you hear the input through the track. **Auto** (the default) plays it while the track is armed, except during playback. **In** always plays it, armed or not. **Off** never does: use it when you hear your instrument another way, such as through a mixer. |
| **Level**, **Pan** | With a step either way on screen. Q-Links 1 and 2 turn them too while the panel is open. |
| **Takes** | Opens the track's take lanes (below). |

## Editing regions

Tap a region to pick it. The buttons along the bottom become **Split**, **Erase**, **Fades**, **Gain**, **Takes** and **Undo** for that region; tap empty space to go back to the song's own buttons.

- **Split** cuts the region at the playhead, so put the playhead inside it first.
- **Erase** takes the region out.
- **Drag** a region sideways to move it. It snaps to sixteenth notes; hold **SHIFT** to move it freely. It stays picked where it lands.
- **Gain** and **Fades** are on the Q-Links. While a region is picked, the four knobs turn the track's **Level** (Q1) and **Pan** (Q2), and the region's **Gain** (Q3) and **Fades** (Q4, the fade in and the fade out together). Hold **SHIFT** for fine steps. A fade shows as a slope on the region.
- **Undo** takes back the last change, and **SHIFT + Undo** redoes what Undo took back.

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
- **The mixer.** All the audio tracks play through one **Audio** strip in the mixer (**TRACK MIX**), where you can add effects to them together.

## Saving

The audio tracks are saved with the project (**SHIFT + BROWSE**): every take, the regions you comped from them, and each track's input, monitor, level, pan, mute and solo. If a track is recording when you save, the save waits until it stops. Opening a project ends any recording and brings in the project's audio tracks; **New project** empties them. See [Saving your work](saving.md).

> [!MPC]
> On the MPC, audio tracks sit in the same sequence as MIDI tracks and each has its own mixer channel. In MPFree they play on the Song view's timeline, under the clip lanes, and share one Audio strip in the mixer for now.
