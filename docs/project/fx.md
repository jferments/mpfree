# FX: the first effects and the insert chain

**Status:** 2026-11-11, host-only and tested. The code is [`rust/mpfree-fx`](../../rust/mpfree-fx), built on the shared [`mpfree-node`](../../rust/mpfree-node) interface ([synths.md](synths.md) describes it). Nothing here is wired to the pad player, the mixer or the GUI yet, and nothing has run on the MPC. The full design (what makes FX sound great or cheap, what users ask for, other devices, libraries, the architecture, the roadmap and the decisions for Jay) is in the design doc "MPFree FX and filters design"; the skill tree and the mockups are in mpfree-claude (`skill-tree/fx-skill-tree.html`, `fx/mockups/`).

## Clipping

Deliberate clipping is a feature; clipping nobody asked for is not. No effect clamps a signal unless that is its job (Drive, Crush, the Delay's feedback saturation, Master safety), and nothing fails, stops or mutes because a level is high. Every output is meant to end in **Master safety**, which keeps the converter from clipping unless the user picks its Raw mode, and counts what it caught.

## The effects

Every effect is a `Node`, so it takes sample-timed parameter events (automation, p-locks, macros), describes its parameters for a generic GUI page, and keeps the node rules: no allocation, lock or panic in `process`, and the same output, sample for sample, for any block size.

- **Multi filter:** zero-delay-feedback state-variable filter after Andrew Simper (Cytomic): low-pass, high-pass, band-pass and notch at 12 or 24 dB/octave, a bell and two shelves, resonance up to a long ring, and optional drive into a soft saturator before it. Cutoff sweeps glide in the log domain and never zip or blow up.
- **DJ filter:** one knob, low-pass to the left (20 kHz down to 80 Hz), high-pass to the right (20 Hz up to 8 kHz), and bit-for-bit off in the middle.
- **Drive:** Soft (tanh), Hard, Tube (biased), Foldback, Fuzz and Rectify, run inside 2x (Eco) or 4x (HQ) oversampling with linear-phase half-band filters, then a DC blocker, a tone low-pass, mix and output. Auto-gain keeps the level where it was, so more drive means more grit, not just louder. 55 frames of latency in both qualities, so switching never moves the sound.
- **Crush:** bit depth (1 to 16) and sample-rate reduction (200 Hz to 48 kHz) with optional TPDF dither. Aliasing on purpose.
- **Delay:** free (1 ms to 4 s) or synced to the tempo (1/32 to 1 bar, dotted and triplet), ping-pong, and a feedback path through a high-pass, a tone low-pass and a soft saturator, so repeats darken and roughen like tape. Feedback goes to 110 %: the saturator keeps self-oscillation bounded. Time changes glide (tape-style pitch bend, no clicks). Freeze holds the buffer as a loop.
- **Compressor:** feed-forward, soft knee, gain computed and smoothed in decibels (Giannoulis, Massberg and Reiss, JAES 2012), stereo-linked, makeup, parallel mix, a sidechain input for ducking, and a gain-reduction meter.
- **Master safety:** trim, a 5 Hz DC blocker, then Safe limiter (default), Soft clip or Raw. The Safe limiter looks 1.5 ms ahead; its gain is the smallest any frame in the window needs, released and averaged over the window, so it is always down before the peak arrives and the output never goes over the ceiling (-0.3 dBFS by default). Every mode counts overs and frames that leave above 0 dBFS. The latency is the same in every mode.

## The insert chain

`Chain` runs effects in series, the way a track, bus or master will:

- **Bypass is click-free and keeps tails:** switching a slot off fades its input out over 20 ms and its dry signal in, so a delay keeps ringing (spillover) and a drive crossfades to dry. A bypassed slot keeps running on silence for exactly its tail, then rests and is reset.
- **Latency never moves:** each slot's dry path is delayed by the slot's latency, on or off.
- **Events:** each slot has a queue of sample-timed events (256 per block); a full queue drops and counts.
- **A NaN guard:** a node that puts out a non-finite sample is reset, its slot plays dry for that block, and the chain counts it.
- **Block-size independent,** including where a tail starts and ends. The one exception: a node with an endless tail (a frozen delay) that is bypassed and then unfrozen picks up its new tail at the end of that block.

## Tests

`cargo test -p mpfree-fx` (29 tests):

- every parameter description valid, ids equal to indices, defaults reported
- every effect gives bit-identical output at block sizes 1, 7, 128, 1000, 4096, 8192 and random sizes, with automation on every parameter; so does a seven-slot chain with bypass switching (1, 64, 333, 4096, 8192 and random)
- every parameter alone at its minimum and maximum, and all of them together, on hot noise, DC and impulses: finite and bounded
- the Delay at 110 % feedback self-oscillates but stays bounded; synced time lands exactly on the grid and follows a tempo change; ping-pong alternates
- filter slopes (24 dB/oct: -66 to -78 dB three octaves up; 12 dB/oct: -33 to -39 dB), the bell's gain, the shelves; the DJ filter is bit-for-bit off in the middle
- Drive latency is 55 frames in both qualities and they line up to well under a frame; Mix 0 is the input, exactly, 55 frames late
- **aliasing** of a 5 kHz sine through tanh at +18 dB, worst alias in the audible band against the fundamental: **-26 dB** plain (no oversampling), **-59 dB** Eco 2x, **-97 dB** HQ 4x
- the Compressor reaches its static curve (to 0.05 dB) and ducks from a sidechain
- Master safety never passes the ceiling (44.1, 48 and 96 kHz; ceilings 0, -0.3 and -6 dB; trim up to +12 dB; a step to +15.6 dBFS), counts overs, clips nothing; it is transparent below the ceiling; Soft clip stays under; Raw passes overs and counts them
- chain bypass makes no click, keeps the latency, and gives the input exactly once the tail has rested; a delay keeps echoing after bypass and then dies away; the NaN guard; the full queue
- `tests/no_alloc.rs`: a counting allocator watches 4000 blocks through a chain of every effect plus each effect on its own, with random parameter moves, bypass switching, freeze, sidechain, resets and NaN, and sees no allocation or free.

## Listening and timing

- `cargo run --release -p mpfree-fx --example render [out.wav]` writes `~/mpfree-work/fx-demo.wav`: 12 bars at 90 BPM where each effect gets two bars through chains in 128-frame blocks (dry; Fuzz drive into a resonant filter sweep on the bass; crushed drums and kick-ducked chords; ping-pong delay; the delay bypassed and ringing on under DJ filter sweeps; a last hit echoing out). The mix runs hot on purpose into Master safety; it prints per-bar levels and limiting, the overs caught, and how many frames a Raw master would have clipped, and checks that nothing clips and the end is quiet.
- `cargo run --release -p mpfree-fx --example bench` times each effect on 128-frame blocks. On the build host (an x86 desktop core) Drive costs 0.8 % of real time in Eco and 1.3 % in HQ, the Delay 0.3 %, the Compressor 0.2 %, Master safety 0.15 %, the filters 0.05 to 0.13 %. The MPC One's Cortex-A17 is slower; it needs its own run before slot limits are set.

## Next

- F2: a Cloud reverb (a port of the MIT Cloud Seed), a plate, EQ, chorus/flanger/phaser, lo-fi.
- F3: the mixer: track chains, 4 buses, sends, the master chain and Master safety on the output, sidechain taps.
- F4: FX pages in the GUI, built from each effect's parameter list.
- F5: Perform FX (punch-in, beat repeat, filter and tape stop on pads).
