# Sampler feature map

**Status:** proposal, 2026-11-10, for the owner's review. Companion to the [sampler design](sampler-design.md) (the model, engine and steps) and the [sampler research](../research/sampler-landscape.md) (sources and libraries).

The owner asked for three things: every feature the MPC sampler offers, rebuilt by MPFree with a better UI; the capabilities other samplers have that the MPC lacks, and how MPFree would build them; and what makes a sampler great for both production and live improvisation. This page answers in that order, starting with the last, because it is the test every feature below has to pass.

Feature names for the MPC come from Akai's documentation where we could read it (the MPC Sample guide's [Sample Mode](https://www.akaipro.com/guides/mpc-sample/sample_mode.htm) and [Pad Play](https://www.akaipro.com/guides/mpc-sample/pad_play.htm) pages, the older MPC manual's [Process list](https://www.manualslib.com/manual/724425/Akai-Mpc.html?page=74)) and otherwise from memory of the MPC 2.x and 3.x software; the full v3.10 guide could not be downloaded from this session. Items marked *(memory)* need checking against the guide before we claim parity.

## What makes a sampler great

### For production

1. **Nothing is final until you say so.** Every edit is reversible, and the original take is always there. Producers come back to a chop weeks later.
2. **Depth within reach.** Every parameter of every slice can be reached in two levels or fewer, and seen at once rather than page by page.
3. **Precision.** Sample-accurate markers, zero-crossing snap, zoom to single samples, numeric entry when you want it.
4. **Organisation.** Names, colours, tags, search, and knowing which pads, programs and patterns use a sample before you delete it.
5. **Sound quality with character on demand.** Clean interpolation and stretching by default; vintage colour (bit depth, rate, filters) as a deliberate choice.
6. **Get it out.** Export stems, slices, kits and projects in open formats.

### For live performance and improvisation

1. **Never stop the music.** No modal dialogs, no loading pauses, no operation that blocks the audio. Loading, analysing and saving happen in the background.
2. **Capture anything, at any time.** Skip-back on the input and on the master, resample in one press, record in time with the bar.
3. **Instant, eyes-free control.** Every performance action is on a pad, button or knob with a light that shows its state. The screen confirms; it is never required.
4. **Undo you can trust on stage.** Undo the last recording, the last chop, the last resample, without stopping.
5. **Gestures that are musical, not administrative.** Hold to latch, press harder to intensify, release to return. Momentary by default, sticky with a modifier.
6. **Safe to explore.** Snapshots and scenes so you can mangle a sound and snap back.

### The shared rule

A feature belongs in MPFree when a performer can use it without looking and a producer can reach its detail on the screen in two levels or fewer (the rule from the [project research](https://github.com/jferments/mpfree-claude/blob/main/notes/research/mpc-one-complaints-and-os-design.md)).

## How the UI beats the MPC's, everywhere

These apply to every feature in the tables below, so the tables don't repeat them.

- **One place for a sound.** The MPC splits a sound across Sample Edit, Program Edit, Pad Mixer and Track View, and owners say they don't know "which place" does what ([Elektronauts](https://www.elektronauts.com/t/the-all-things-mpc-thread-part-4/240914?page=132)). MPFree has one **Sound** page per pad with tabs (Wave, Chop, Play, Shape, Grain, FX) on one screen, and the same tabs whether you got there from the pad, the browser or the sequencer.
- **Hit a pad to focus it.** Hitting a pad (or touching it on screen) makes it the subject of every page. The MPC has this; MPFree makes it universal and shows the focused pad's name and colour in the header at all times.
- **Knobs follow the screen.** The four Q-Links always control the four parameters drawn in the Q-Link strip at the bottom of the current tab, labelled and coloured like the knobs. Turning a knob highlights its parameter. No hidden Q-Link modes.
- **Waveform first.** The waveform is always visible on Wave, Chop and Grain, with markers as handles at the edge, not icons on top of the audio. Zoom stays centred on what you last touched.
- **Every value visible and touchable.** Tap a value to type it; drag it to sweep it; double-tap to reset it.
- **Previews before commits.** Chop sensitivity, normalise, stretch and similar show their result live; nothing is written until you accept.
- **Names before destruction.** Deletes and replacements show what goes and what uses it, and are undoable anyway.
- **Light the hardware.** Pad colours show slices, layers, mute and choke states; button lights show modes. The pad player already drives both (see the [controller protocol](../hardware/controller-protocol.md)).
- **Layouts are strings.** Because `mpfree-ui` pages are one-line layout specs, a performer's layout and a producer's layout of the same tab are two strings, switchable with one button.

## MPC features, rebuilt

"Step" refers to the steps in the [design](sampler-design.md#steps).

### Recording

| MPC feature | What the MPC does | MPFree version and UI improvement | Build | Step |
|---|---|---|---|---|
| Sampler mode: input source, mono L/R or stereo, monitor | Choose input and channels, monitor on or off | Same, plus a large live meter with peak hold and clip lamp, and the last 10 s of input drawn as a scrolling waveform so you see what's coming | ACVA capture in `mpfree-audio`; meter and scrolling waveform widgets | S4 |
| Threshold | Recording starts when input crosses a level | Threshold drawn as a line across the live meter and waveform; drag it. Pre-roll is automatic from the rolling buffer, so no pre-record setting | Rolling buffer in the audio thread | S4 |
| Pre-record time *(memory)* | Keeps N ms before the threshold | Not needed: the rolling buffer always has it; the take's start marker is placed at the detected onset and can be dragged earlier | Same | S4 |
| Time limit / record length | Stops after a time | Also "N bars", synced to the sequencer, making a perfect loop | Transport-synced arming | S4 |
| Assign to pad after recording | Puts the take on a pad | Hit a pad to assign the take, or ignore it and keep recording; takes stack in a "Takes" list you can audition from the pads | Takes list in the project | S4 |
| Resample *(memory: via the input or "Resample" source)* | Records the MPC's own output | One press from anywhere: master, focused pad, program, or a track, before or after its effects; also skip-back on the master | Internal taps in the mixer | S4 |
| Looper | Records and overdubs a loop in time | MPFree looper is a pad mode: a pad becomes a looper slot (record, overdub with feedback amount, undo last layer, half/double speed, reverse); any looper layer can become a sample in one press | Looper voice type over a fixed RAM buffer | S7 |

### Editing (Trim and Process)

| MPC feature | What the MPC does | MPFree version and UI improvement | Build | Step |
|---|---|---|---|---|
| Start, End, Loop point | Three points; the loop runs from the loop point to the end | Start, end, **loop start and loop end separate**, so the tail after the loop plays on release (an owner complaint). Q-Links 1-4 move them; SHIFT for single samples | Region recipe | S1 |
| Loop Lock | Ties loop point to start | A "link" toggle per pair of markers, shown as a chain icon between them | Region recipe | S5 |
| Zoom on Start/End/Loop | Zoom to a point | Zoom stays centred on the last touched marker; pinch and the wheel both zoom; the overview strip shows where you are | `Waveform` and `WaveOverview` widgets | S2 |
| Snap to zero crossing | Option | On by default, with transient and grid snap too; snap target shown as a small tick before you let go | Analysis cache | S2 |
| Discard, Delete, Silence, Extract | Destructive edits of a range | All non-destructive: Extract makes a new region (no new audio), Discard is a trim, Delete and Silence are recipe ranges; "Render" bakes when wanted | Region recipe with edit list | S5 |
| Normalise | Raises peak to full scale | A gain value (shown in dB) rather than rewritten audio; option to normalise to RMS or LUFS for consistent kits | Analysis (peak, RMS) | S1 |
| Reverse | Reverses the sample | A per-region switch, also per pad and per step in the sequencer | Voice plays backwards | S1 |
| Fade In, Fade Out (Linear, Log, Exp) | Destructive fades | Fade handles at the region's corners, dragged on the waveform; curve chosen by dragging the handle up or down | Envelope in the voice | S5 |
| Pitch Shift, Time Stretch | Offline process | Non-destructive: a new source is rendered in the background with Signalsmith Stretch while the old one keeps playing; the result is swapped in when ready | Worker-thread render | S7 |
| Bit Reduce *(memory)*, vintage modes | Lo-fi colour | A "Colour" tab: bit depth, sample rate, a choice of vintage filter and saturation, all real time and per pad | Voice DSP | S7 |
| Stereo to Mono | Converts | A per-region channel choice (stereo, mono sum, L, R) that costs nothing | Voice read | S1 |
| Sample rate conversion *(memory)* | Converts | Imports convert to 44.1 kHz on load with `rubato`; the original file is kept | Loader | S8 |
| Save sample | From the pool or browser | From anywhere: the Sound page, the takes list, the browser; one press exports the focused region, slice set or pad as WAV | Writer thread | S5 |

### Chopping

| MPC feature | What the MPC does | MPFree version and UI improvement | Build | Step |
|---|---|---|---|---|
| Threshold chop | Slices at transients | Sensitivity on a Q-Link with slices appearing and disappearing live on the waveform and on the pads (each pad lights as its slice appears) | Spectral-flux onsets | S3 |
| Regions chop (equal parts) | N slices | N on a Q-Link (2-128) | Grid | S3 |
| BPM / beat chop | Slices on beat divisions | Tempo detected and shown; division on a Q-Link | Tempo estimate | S3 |
| Manual chop | Place by hand | Plus **live chop**: play the sample and hit pads to drop markers in time, quantised to transients if snap is on | Pad events to markers | S5 |
| Split, Merge, Erase slice | Edit slices | Same, as touch gestures (tap the gap between two slices to merge, long-press inside one to split at the finger) and as buttons | Chop set edits | S5 |
| Link slices | Moving one slice end moves the next start | On by default; hold SHIFT to move one edge alone | Chop set edits | S5 |
| Convert: new program, existing pads, non-destructive | Slices to pads | Plus to keys, to a phrase (a pattern that replays the slices in time), to files; preview each before committing | Convert functions | S3, S7 |
| Patched phrase *(memory)* | A program that plays the phrase in slices, tempo-following | The phrase convert above; the pattern is a normal pattern you can edit | Sequencer pattern | S7 |
| Up to 128 slices | Limit | Same limit at first; chop sets are named and kept, so re-chopping never loses the old chop | Chop sets | S3 |

### Programs and pads

| MPC feature | What the MPC does | MPFree version and UI improvement | Build | Step |
|---|---|---|---|---|
| Drum program, 128 pads in 8 banks | Kits | Same; the pad grid on screen shows bank, colour and a tiny waveform per pad | Model | S1 |
| Up to 8 layers per pad (4 in keygroups) | Velocity, cycle, random or simultaneous layers | 16 layers per pad and per keygroup zone; layers drawn as stacked bars over a velocity axis, dragged to set ranges and crossfades | Layer model | S1 |
| Keygroup program | Multisample across keys | Same, with zones drawn as boxes over a keyboard and velocity grid; drag edges | Layer model | S7 |
| One Shot / Note On | Trigger modes | Plus Toggle (press to start, press to stop) and Loop-while-held | Voice modes | S1 |
| Polyphony: Mono or Poly | Per pad | Mono, poly, or a voice limit (1-16), with legato option for mono | Voice allocation | S1 |
| Mute groups (choke) | Off, 1-16 | Same, plus one-way chokes (open hat stops when closed hat hits, not the reverse). Groups shown as coloured rings on the pads | Voice allocation | S1 |
| Pad Link / Simult | One pad also plays others | Same, drawn as lines between pads in a link view | Trigger fan-out | S5 |
| Offset | Start offset | Plus velocity-to-start and random start, each a knob | Voice | S1 |
| Semi Tune, Fine Tune | Per sound | Same, with the detected root note shown and a "tune to C" button | Analysis (pitch) | S1 |
| Amp envelope (Attack, Decay/Release, Decay From Start/End, Vel Sens) | Per sound | An AHDSR drawn over the waveform's own time scale, so you see where the decay lands on the sound; drag the curve | Voice envelope | S1 |
| Filter (Classic, LPF/HPF/BPF 2 and 4 pole) with envelope | Per sound | Same family from our own state-variable and ladder filters; cutoff and envelope drawn as a curve | Voice DSP | S1 |
| Warp (Off, 50-200 %, Seq; Time Stretch or Pitch; # Beats) | Tempo-synced playback | Same choices, plus beats detected; pre-stretched in the background in high quality, with real-time granular stretch while it renders | Worker + grain engine | S7 |
| 16 Levels (Velocity, Tune, Filter; more on MPC) | One sound across 16 pads | Velocity, tune, filter, decay, attack, slice start, grain position, layer. The pads light as a gradient of the chosen parameter | Trigger mapping | S7 |
| Full Level, Half Level | Fixed velocity | Same, with the level shown in the header; Half Level's value adjustable | Velocity mapping | S1 |
| Note Repeat with latch | Repeats while held, swing, rate | Rate on the pads' top row or a knob; pressure controls repeat velocity; latch per pad. Also ratchets and rolls that speed up with pressure | Sequencer | Later |
| Pad Mute, Track Mute | Mute pads or tracks | Same, with mute states in pad lights; mute groups can be captured into scenes | Mixer | Later |
| Pad Perform (scales, chords, progressions) | Melodic pads | Same idea; slices and keygroups can be played in a scale across the pads | Trigger mapping | Later |
| Insert effects per pad *(memory: up to 4)* | FX on a pad | Same, plus resample-through-FX in one press (see below) | FX chain | Later |
| Q-Link modes (Project, Program, Pad Scene, Pad Param, Screen) | Five modes | One rule: knobs follow the screen (above). A "pinned" set of four macros can stay put for performance | UI rule | S5 |

### Newer MPC features

| MPC feature | MPFree view |
|---|---|
| Stem separation (MPC 3) | Too heavy for the RK3288. Offered later as an optional job on a workstation over Ethernet ([auxiliary compute](../research/auxiliary-compute.md)), with the stems coming back as sources |
| Clip launching / Matrix (Pro Pack) | Belongs to the sequencer's clips-and-scenes design; slices and phrases become clips. Free in MPFree |
| Super Warp, better FFT stretch (Pro Pack / Live III) | Signalsmith Stretch (MIT) is our high-quality stretch; no paywall |
| Auto-sampler *(memory)* | See "Auto-sample gear" below |
| MPCe position-sensing pads (Live III) | Not on the MPC One's hardware. The One's pads do send pressure (aftertouch), which MPFree uses fully |

## Beyond the MPC

Capabilities other samplers have that the MPC lacks or does weakly, and how MPFree builds each.

| Capability | Seen on | How MPFree builds it | Step |
|---|---|---|---|
| **Skip-back sampling** of the input and the master | SP-404MKII (25 s) | Two rolling buffers (2 minutes each, about 85 MB), "last N seconds" or "last N bars" in one press, cut on bar lines when the sequencer runs | S4 |
| **Resample through effects in one press** | SP-404MKII | A "Resample" button on the focused pad, program or master: renders N bars (or while held) post-FX into a new take, assigned to the next free pad | S4 |
| **Granular instrument and effect** | Torso S-4, Blackbox, Clouds/Beads, Morphagene, Deluge community firmware | The grain engine in the [design](sampler-design.md#granular): a playback mode for any region and an effect on live input, a pad, a program or the master; pressure drives density or size | S6 |
| **Tape-style track / reel** | Torso S-4 Tape, OP-1, Morphagene | A long take with splices (slices) and varispeed; scrub with the wheel; play splices from pads. The same region model with a "tape" playback mode | S7 |
| **Sample locks and slice locks per step** | Elektron (Digitakt, Octatrack, Tonverk) | Every sequencer step can override a pad's sample, slice, start, pitch, filter or grain position. One pad can play a whole break, one slice per step | Sequencer |
| **Random and probabilistic slice playback** | Digitakt II, Octatrack | Slice choice per hit: in order, random, random-without-repeat, or by probability; shown as a slice "dice" setting on the pad | S7 |
| **Crossfader between scenes** | Octatrack | Two snapshots of any set of parameters (scene A, scene B) blended by a Q-Link or the wheel; capture a scene by holding a button and turning knobs | Later |
| **Recorder buffers that run with the pattern** | Octatrack | The "in time" recording mode, plus "record on trig": a sequencer step can start a recording of N steps, for live sampling during a performance | S4, Sequencer |
| **Pickup / looper with overdub** | Octatrack Pickup machines, MPC Looper | The looper pad mode above, synced or free | S7 |
| **Retrig, stutter, beat-repeat** | Octatrack, SP-404 DJ FX, Deluge | A performance effect on any pad or the master: hold to repeat the last N beats at a chosen division, pressure shortens it | Later |
| **Bus effects with one knob each** | SP-404MKII | Two or more FX buses with a "one knob" macro per effect for performance, full detail on the screen | Later |
| **Auto-sample external gear into a keygroup** | Elektron Tonverk, MPC auto-sampler | Send MIDI notes at chosen pitches and velocities out of the MIDI port, record each from the input, trim silence, map to zones | S7 |
| **Spectral and resonator processing** | Torso S-4 resonator, Ableton | A tuned resonator bank on a pad or bus, scale-aware | Later |
| **Wavetable from a sample** | Polyend Tracker, Deluge | Cut a sample into single cycles (pitch-detected) and scan through them with a knob | Later |
| **Tracker-style detail per step** | Polyend Tracker | A step list view showing slice, pitch, FX per step, for producers who want text-level detail | Sequencer |
| **Long takes to disk** | 1010music Blackbox | Writer thread records to a stick or SD as it goes; RAM pool caches the start | S4, S8 |
| **Fully touch-driven sample editing** | Blackbox, Koala Sampler *(memory)* | `mpfree-ui` widgets with gestures; every touch action also has a hardware path | S2, S5 |
| **Edits saved with the sample** | Ableton (warp markers), owner request | Analysis and recipe stored with the source; exporting a sample can include a small sidecar file | S5 |
| **Open formats and file sharing** | Ableton Move export, community wishlists | WAV and FLAC in a plain project folder; later a web file share over Ethernet | Later |
| **Community-extensible** | Deluge community firmware | MIT code, documented formats, plug-in points for new playback modes and effects | Ongoing |

## Performance mode

A sampler for improvising needs a few things to work together, so they are designed as one mode rather than scattered features.

- **Capture row.** The top row of pads (or a button layer) holds Skip Back (input), Skip Back (master), Resample, and Looper. Each takes the last N bars and drops a new sound on the next free pad, lit so you can play it at once.
- **Mangle without fear.** Every pad has a snapshot. Hold a pad and turn knobs to mangle it; press "Return" (proposed: a button to be chosen) to snap all mangled pads back. Scenes A and B and the crossfader sit on top of this.
- **Pressure as the expression axis.** Pad pressure drives the parameter chosen for that pad: filter, grain density, stutter rate, pitch bend. Shown as a ring that fills on the pad's screen tile.
- **Latch.** NOTE REPEAT/LATCH latches loops, grains, stutter and drones, so hands are free. Latched pads pulse their lights.
- **Nothing blocks.** Analysis, stretching and saving run in the background and appear when ready; the sound you have keeps playing.
- **Undo the last capture.** UNDO removes the last take, layer or resample without touching anything else.

## Hardware map (proposed)

Button numbers beyond SHIFT, FULL LEVEL, PLAY and STOP are not captured yet, so this is a proposal by label, to be confirmed by a read-only button capture (which also serves the owner's requests for Menu, Shift, Latch and Full Level in the pad player thread).

| Control | Sampler meaning |
|---|---|
| Pads | Play; focus a pad; assign (with a held button); live chop (in Chop); slice select |
| Pad pressure | The pad's chosen expression parameter |
| Q-Links 1-4 | The four parameters in the current tab's Q-Link strip |
| SHIFT + Q-Link | Fine adjustment |
| Wheel | Zoom (Wave, Chop), scan speed (Grain), select (lists); SHIFT + wheel scrolls |
| SHIFT | Second function of every control, shown on screen while held |
| MENU | The current page's options; SHIFT + MENU opens settings (the owner's proposal) |
| FULL LEVEL / HALF LEVEL | Fixed velocity |
| 16 LEVELS | Spread the focused sound across the pads |
| NOTE REPEAT / LATCH | Repeat; latch loops, grains and drones |
| ERASE + pad | Clear a pad (undoable) |
| UNDO / REDO | The command log |
| REC, PLAY, STOP | Record a take (SHIFT + REC: skip back), transport |

## What changes in the plan

The steps in the [design](sampler-design.md#steps) stay as they are, with three additions from this map:

- **S4 grows:** skip-back on the master as well as the input, Resample from any pad or program, and the capture row.
- **S7 grows:** looper pad mode, tape playback mode, random slice playback, auto-sample gear.
- **A new later step for performance effects:** scenes and crossfader, stutter and beat-repeat, bus effects with one-knob macros, snapshots and Return.
