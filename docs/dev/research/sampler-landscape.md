# Samplers: what people like, what they don't, and what to borrow

**Status:** research, 2026-11-10. Feeds the [sampler design](../design/sampler-design.md). Nothing here has run on the MPC.

The owner asked for a sampler that gets "a TON of thought", because sampling is what the MPC is for. This page collects what MPC owners praise and complain about, how other samplers solve the same problems, what granular tools do, and which libraries MPFree can use under its MIT licence. Where a point comes from a source it is linked; where it is our own reading or from memory it says so.

## The MPC sampler today

How Akai's MPC software (2.x and 3.x, on the MPC One, Live and X) handles a sample, as documented:

- **Record:** a Sampler screen with input choice, a threshold that starts the recording, and a pre-record time so the attack is kept. The take lands in the project's sample pool.
- **Sample Edit:** Trim mode (start, end, loop point) and Chop mode. Chop has Threshold (transients), Regions (equal parts), BPM/beat divisions and Manual, up to 128 slices per sample, with an option to snap to zero crossings ([Audeobox chop guide](https://www.audeobox.com/learn/mpc-software/chop-shop-sampling/)).
- **Convert:** slices become a new drum program (A01, A02, ...), go to existing pads, or are kept non-destructively as slice references; older MPCs (MPC5000 "Chop Shop") could also make a new sequence that replays the phrase from the slices ([MPC5000 manual, p.126-127](https://www.manualslib.com/manual/1028640/Akai-Mpc5000.html?page=126)).
- **Programs:** drum programs (pads with up to 8 layers each, velocity or cycle switching, mute and choke groups, filter and envelopes) and keygroup programs (multisamples across the keyboard).
- **Playback:** one-shot or note-on, warp (tempo-synced stretch), 16 Levels (one sound spread over the 16 pads by velocity, tune, filter or decay), pad perform modes.
- **MPC 3 and Live III** added better FFT time stretching, a "pro" stem separator and combo inputs with mic preamps on the Live III ([MusicRadar Live III review](https://www.musicradar.com/music-tech/akai-mpc-live-3-review)). The MPC One has none of the new hardware: one stereo line input, no preamps ([earlier MPFree research](https://github.com/jferments/mpfree-claude/blob/main/notes/research/mpc-one-complaints-and-os-design.md)).

### What owners like

- **The pads and the feel.** Hitting a pad and hearing the chop at once is the whole instrument. 16 Levels and Full Level are old favourites (from memory of community use, not one source).
- **Threshold recording with pre-roll.** Arm, play the record, and the take starts on the first hit without losing the attack.
- **Chop to pads in a few presses.** Threshold or region chop, then convert to a new program, is the core beatmaking loop and is fast when it works.
- **Non-destructive slices.** The original stays untouched and can be re-chopped later ([Audeobox](https://www.audeobox.com/learn/mpc-software/chop-shop-sampling/)).
- **Standalone.** No computer needed for the whole path from record to beat.

### What owners complain about

From the Elektronauts MPC threads and MPFree's earlier complaint survey:

| Complaint | Detail | Source |
|---|---|---|
| End point is the loop point | You can't keep the release tail after a sustain loop; loops are tuned separately from the sample | [Elektronauts MPC thread](https://www.elektronauts.com/t/mpc-thread-mpc-live-mpc-x-mpc-one-part-2/173139/6163) |
| Audition limited to 30 s | Browsing long samples cuts off | same |
| Warp quality | Owners ask for a cleaner stretch, keeping the old one for "nasty FX" | same |
| Zoom loses your place | Zooming in Sample Edit doesn't stay centred on what you were looking at | same |
| Icons cover the waveform | The S and R markers sometimes block the waveform | same |
| Too easy to load or delete the wrong thing | Names aren't shown during loads and deletes | same |
| Layers per keygroup | Four layers is too few | same |
| Effects can't be printed non-destructively | Owners want to process a sample through any plugin, with its own undo | same |
| Can't save a sample from where you are | Not from Sample Edit or from the pool | [Elektronauts all-things-MPC thread](https://www.elektronauts.com/t/the-all-things-mpc-thread-part-4/240914?page=132) |
| Warp info isn't saved with the sample | Unlike Ableton, the stretch settings don't travel with the file | same |
| Hard to know where to go | "you just have to be in a really specific place to do the thing you want and it isn't immediately clear which place that is" | same |
| Menus and touchscreen dependence | Many steps need the screen; separate audio and MIDI tracks; per-sequence setup | [MPFree complaint survey](https://github.com/jferments/mpfree-claude/blob/main/notes/research/mpc-one-complaints-and-os-design.md) |
| Paid features | Super Warp and clip launching sold as a Pro Pack | same |
| Small storage, slow transfers | About 2 GB free; no file transfer in standalone mode | same |

## Other samplers and what to borrow

| Sampler | What it does well | Borrow |
|---|---|---|
| Roland SP-404MKII | Skip Back keeps the last 25 s of what you played, so you can grab a moment after it happened; resampling through the effects; the waveform and envelope editor are easy; real-time chop ([MusicTech review](https://musictech.com/reviews/hardware-instruments/roland-sp-404mkii-review/)) | An always-on input buffer; resample anything through effects in one press; chop while the sample plays |
| Elektron Octatrack | Recorder buffers that run with the sequencer, slice grids, timestretch locked to tempo, scenes on a crossfader ([Elektronauts](https://www.elektronauts.com/t/octatrack-or-digitakt-no-computer-setup/109164/10)) | Record in time with the transport for N bars; slices as a lockable parameter per step |
| Elektron Digitakt II | Slice Machine (grid or transient, up to 64 slices, join and split, start/end/loop per slice), random or linear slice playback, per-step parameter locks ([Gearnews review](https://www.gearnews.com/elektron-digitakt-ii-review/)) | Join/split slices; slice number as a sequencer parameter |
| Elektron Tonverk | An auto-sampler that builds velocity-layered multisamples from a synth ([MPFree survey](https://github.com/jferments/mpfree-claude/blob/main/notes/research/mpc-one-complaints-and-os-design.md)) | Auto-sample external MIDI gear into a keygroup (later) |
| Torso S-4 | Each track is a chain: Material (a tape recorder up to 6 min, or a poly sampler), Mosaic granular (up to 128 grains, ±36 semitones, tempo-locked spawn, scale-quantised pitch, randomised size/rate/detune, 4 s buffer), a resonator, colour and space. Bypassing Material makes it a live-input processor ([Sound On Sound](https://soundonsound.com/reviews/torso-electronics-s-4)) | Granular as one stage in a chain, usable on a sample or on live input; scale-quantised grain pitch |
| 1010music Blackbox | A fully touch-driven sampler: clip, slicer and granular modes, threshold or MIDI-synced recording straight to SD for long takes ([CDM review](https://cdm.link/review-1010-blackbox-sampling-workstation/)). The granular screen is "sparse" and live resampling has no cueing | Record long takes straight to disk; give granular a proper screen |
| Make Noise Morphagene (from memory) | Reels, splices and "gene size": one granular model that also does tape-style varispeed and splice playback | Granular, slice and tape playback as one engine with different settings |
| Mutable Instruments Clouds and Beads | The classic granular texture processor: freeze, position, size, density, texture, pitch, and a live buffer. Firmware for the STM32F modules is MIT licensed ([pichenettes/eurorack](https://github.com/pichenettes/eurorack), [CDM on Beads](https://cdm.link/beads-is-now-open-source/)) | A reference design we may study and port with attribution |
| Ableton Simpler and Push (from memory) | Classic, One-Shot and Slicing modes on one device; slice by transient, beat, region or manual; "slice to new MIDI track"; warp settings saved with the clip | One sampler with modes, not separate devices; save warp settings with the sample |
| Polyend Tracker+ | Every step shows note, instrument and FX; slicing and granular instrument modes | Show slice and grain settings on the step, not hidden in menus |
| Synthstrom Deluge community firmware | A small device that gained granular and manual slicing from its community ([Synth Anatomy](https://synthanatomy.com/2023/12/deluge-groovebox-just-got-the-first-open-source-community-firmware-update.html)). Its code is GPL-3, so MPFree can read about it but not copy it | Proof that an opened device keeps getting better |

### Lessons

1. **Never lose the moment.** SP-404 Skip Back and the Octatrack's recorders both keep audio you didn't plan to record. MPFree has the RAM for a long rolling buffer of the input and of its own output.
2. **One sampler, many modes.** Simpler, Morphagene and the S-4 treat one-shot, slice, loop, tape and granular as settings on one object. That avoids the MPC's "which screen do I need" problem.
3. **Edits are recipes.** Slices, loops, fades, warp and gain are settings stored with the sample and saved with it. Baking is an explicit, undoable step.
4. **Chop while it plays.** Real-time chop (tap a pad to drop a marker) is fast and musical, and only hardware with pads can do it well.
5. **Granular needs its own screen and real controls.** Blackbox's granular was criticised for a sparse screen; the S-4 and Clouds work because every parameter has a knob.
6. **Long takes go to disk.** The Blackbox records straight to SD; the S-4's 4 GB without a card slot is a complaint.

## Granular synthesis in brief

A granular engine plays many short overlapping pieces ("grains", typically 5 to 500 ms) of a sound, each with its own window (fade in and out), start position, pitch and pan. The common controls:

| Control | What it does |
|---|---|
| Position | Where in the sample (or live buffer) grains start |
| Spray (jitter) | Random spread around the position |
| Size | Grain length |
| Density or overlap | Grains per second, or how many overlap |
| Pitch, pitch spray | Transposition per grain, with optional random detune and scale quantising |
| Window | Grain envelope shape (Hann, Tukey, trapezoid, exponential); short windows click, long ones smear |
| Scan speed | How fast the position moves on its own. 1 = original speed (a granular time stretch), 0 = frozen |
| Reverse | Chance that a grain plays backwards |
| Spread | Stereo placement of grains |
| Freeze | Stop writing a live buffer and keep granulating what is in it |

Two uses matter for MPFree: a **granular instrument** (a sample played from pads or keys with these controls) and a **granular effect** on live input or a track (a circular buffer, as in Clouds or the S-4's Mosaic). The cost per grain is small (an interpolated read and a window multiply per output sample), so the limit is the grain count; MPFree must measure it on the RK3288.

## Libraries

Licences below were read from crates.io with `cargo info` on 2026-11-10. MPFree is MIT, so permissive licences (MIT, Apache-2.0, BSD, Zlib) are fine; MPL-2.0 is usable but keeps its own files under MPL; GPL and LGPL are excluded from anything MPFree ships.

| Job | Choice | Licence | Notes |
|---|---|---|---|
| Waveform display | Our own peak pyramid and `Waveform` widget in `mpfree-ui` | MIT (ours) | No library needed: min/max/RMS summaries at power-of-two block sizes, drawn one column per pixel. BBC's audiowaveform does the same but is GPL, so don't copy it |
| WAV read and write | [`hound`](https://crates.io/crates/hound) 3.5.1 | Apache-2.0 | `mpfree-sampler` already has its own small WAV parser; either keep it or switch |
| FLAC read | [`claxon`](https://crates.io/crates/claxon) 0.4.3 | Apache-2.0 | |
| FLAC write | [`flacenc`](https://crates.io/crates/flacenc) 0.5.1 | Apache-2.0 | Halves the size of saved takes; optional |
| MP3, AAC, Ogg import | [`symphonia`](https://crates.io/crates/symphonia) 0.6.1 | MPL-2.0 | Optional feature; import only |
| Sample-rate conversion | [`rubato`](https://crates.io/crates/rubato) 5.0.1 | MIT or Apache-2.0 | For imports that aren't 44.1 kHz |
| FFT | [`realfft`](https://crates.io/crates/realfft) 3.5.1 / [`rustfft`](https://crates.io/crates/rustfft) 6.4.1 | MIT / MIT or Apache-2.0 | Onset detection and analysis |
| Pitch detection | [`pitch-detection`](https://crates.io/crates/pitch-detection) 0.3.0, or our own YIN | MIT or Apache-2.0 | Root-note detection for keygroups |
| Time stretch and pitch shift (high quality, offline) | [`signalsmith-stretch`](https://crates.io/crates/signalsmith-stretch) 0.1.3 (Rust wrapper of Signalsmith Stretch, C++) | MIT | Needs a C++ cross-compiler for the ARM build. Bungee (`bungee-rs`, MPL-2.0) is the alternative |
| Real-time stretch | Our own granular / WSOLA voice | MIT (ours) | Same engine as granular |
| Real-time plumbing | [`rtrb`](https://crates.io/crates/rtrb) 0.4.1 (lock-free ring), [`basedrop`](https://crates.io/crates/basedrop) 0.1.3 (free memory off the audio thread) | MIT or Apache-2.0 | |
| No-allocation check in tests | [`assert_no_alloc`](https://crates.io/crates/assert_no_alloc) 1.1.2 | BSD-1-Clause | Debug and test builds only |
| Filters, envelopes | Our own (state-variable filter, ADSR) | MIT (ours) | Small and well documented; [`biquad`](https://crates.io/crates/biquad) and [`dasp`](https://crates.io/crates/dasp) (MIT or Apache-2.0) exist if needed |

**Excluded for licence reasons:** Rubber Band (GPL or commercial), SoundTouch (LGPL), aubio onset/tempo detection (GPL), BBC audiowaveform (GPL), Deluge firmware (GPL-3), Ableton Link's open version (GPL, already noted in the project).

**Too heavy for the device:** stem separation (Demucs and similar models). If MPFree ever offers it, it runs on a workstation over Ethernet, as in [auxiliary compute](auxiliary-compute.md).

## Open questions to measure

- How many sample voices and how many grains the RK3288 can run in a 128-frame period, with NEON, beside the UI.
- What the ACVA capture actually delivers: the stream is S32_LE, but how many of the 32 bits are real, the input's noise floor and its clip level are unmeasured.
- Whether Signalsmith Stretch is fast enough on the device to re-stretch a loop in the background in a second or two after a tempo change.
- The front-panel buttons that should open the sampler (most button numbers are not captured yet, see the [controller protocol](../hardware/controller-protocol.md)).
