# Wavetable synth

`mpfree_synth::Wavetable` (node id `mpfree.wavetable`) is MPFree's third
native synth, after Poly and FM. It plays stacks of single-cycle frames and
scans through them, so one knob (Position) moves a sound from one timbre to
another: sine to square, a vowel A to U, a sung tone's attack to its tail.

It is node N5 in the synth skill tree. The node rules are Poly's: it adds
into the buffers, never allocates, locks or frees in `process`, and gives
the same output for any block size (tests check both).

## The voice

| Part | What it does |
| --- | --- |
| Osc A, Osc B | Table (10 factory tables or the user table), Position through it, Warp mode and amount, octave, semi, fine, level |
| Warp | Bend (phase distortion, the cycle squeezed to the front), Sync (the cycle repeats up to 8 times a period), Mirror (forward then back), FM (the other oscillator moves this one's phase) |
| Unison | 1 to 8 stacks on both oscillators, detune and stereo spread |
| Mix | A sub sine an octave under Osc A, and noise |
| Filter | Poly's: ladder LP 24, state-variable LP 12, BP 12, HP 12, with drive, key track, velocity |
| Mod env | ADSR to cutoff, to Position A and B, and to warp; velocity to position |
| Amp env | ADSR, velocity to level |
| LFO | Sine, triangle, saw, square, S&H, free or synced to the transport; to pitch, cutoff, Position A and B, level |
| Voice | Poly, mono or legato, 1 to 16 voices, glide, bend range |
| Pressure | Four slots through the shared shaper: Position, Warp, Cutoff, Resonance, Level (a swell), Vibrato, LFO rate, Pitch |

Presets: Init, Morph Pad, Vowel Lead, Sync Sweep, Digital Bell, Growl Bass,
Organ, Harmonic Pluck, FM Keys.

## Tables

A table is up to 256 frames. Each frame is analysed (FFT) and kept at 17
bandwidths, half an octave apart, from 512 harmonics down to 1. A note reads
the widest level whose top harmonic stays below Nyquist, taking unison
detune and the warp's speed-up into account, so high notes don't alias:
the worst alias of a saw at C8 is about 57 dB under the fundamental (the
test `high_notes_do_not_alias`). Reading is linear in phase and linear
between neighbouring frames.

Memory: about 49 KB per frame. The ten factory tables (176 frames) take
8.8 MB once per process, shared by every Wavetable instance. A 16-frame
user table is 0.8 MB; a full 256-frame import about 12.6 MB.

Factory tables are made from formulas, with no samples and no third-party
content: Basic (sine, triangle, saw, square), Harmonics (a saw built up a
harmonic band at a time), Pulse (50 % to 3 %), Sync (hard-synced saw, ratio
1 to 6), Vowels (A, E, I, O, U formants), Organ (drawbar settings), Bits (a
sine at 64 down to 2 steps), Resonance (CZ-style windowed resonance), FM
(two-operator index sweep) and Drift (seeded random spectra).

## Importing tables

All of this allocates and runs on the control thread
(`mpfree_synth::wavetable::import`):

- `table_from_wav(name, bytes)`: PCM 8/16/24/32-bit or float WAVs, any
  channel count (mixed to mono). A `clm ` chunk (Serum writes `<!>2048 ...`)
  gives the frame size; without one, whole 2048-sample frames are a
  multi-frame table and a file of up to 4096 samples is one single cycle
  (AKWF-style files are 600 samples).
- `table_from_sample(name, mono, rate, frames)`: a pad's sample becomes a
  table. It finds the root pitch (YIN, from `mpfree-analysis`), cuts
  `frames` cycles spread across the sample, each from a rising zero
  crossing and as long as the pitch measured there, and closes each loop
  without a step. Unpitched audio (drums, noise) is refused with
  `TableError::NoPitch`.

A built table reaches the audio thread as an `Arc<Table>` through
`Wavetable::set_user_table(osc, table)`, which returns the table it
replaces for the host to drop on the control thread. Choosing "User" in an
oscillator's Table plays it; "User" with nothing loaded plays Basic.

## Cost

Desktop (`mpfree-bench --filter Wavetable`, p99 of a 2.9 ms period): one
oscillator at 8 voices costs about what Poly's Init does (about 50 us);
both oscillators with unison 4 at 8 voices about 100 us; the heaviest case,
16 voices, both oscillators, unison 8, 200 to 450 us on a shared machine.

On the MPC One the synth budget is 20 % of a period (580 us). Round 6
measured one Poly at up to 18 voices at 577 us on the device, about
32 us a voice. Wavetable's one-oscillator voice costs about the same on
the desktop, so the estimate is 8 voices of a one-oscillator patch at
about 9 % of a period, and 8 voices of a two-oscillator, unison-3 patch
near the budget. Morph Pad is set to 6 voices for that reason. These are
estimates until the device bench runs (`docs/project/profiling.md`).

## Synth tab

`mpfree-synth-ui` names the pages: Osc A, Osc B, Mix, Filter, Mod, LFO, Env,
Voice, Pressure. The Q-Links on an oscillator page turn Position, Warp
amount, Fine and Level. The visual strip draws each oscillator's table as
a 3D stack of frames with the frame its Position reads drawn bright and
bent by its warp (`wt_visual`), next to Poly's filter and envelope views.
Preview screens: `mpfree-synth-preview` (synth-wt-*.png).

## Demo

`cargo run --release -p mpfree-synth --example wt_render` renders a 90 BPM
beat: Growl Bass, Morph Pad, Vowel Lead scanned by pad pressure, Sync Sweep
stabs, and a sung tone turned into a 16-frame table and scanned by the LFO.
It prints each stem's level per bar and the lead's table position per beat,
and checks the ending is silent and nothing clips.

## Next

- Wire it into the pad player's Engine choice, after Poly and FM (the synth
  thread owns `mpfree-hello/src/synth.rs`).
- The Sample tab: "Make a wavetable" from a pad's sample, then pick it in
  Osc A or B.
- Load WAV tables from the USB stick's library (storage spec).
- Pass the user table and live positions into the Synth tab so the visual
  draws them.
- Spectral warps (formant shift, harmonic stretch) if the device bench
  leaves room.
