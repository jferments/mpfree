<!--
covers: rust/mpfree-fxlab/src/readers.rs rust/mpfree-fxlab/tests/readers.rs rust/mpfree-fxlab/quality/readers.tsv rust/mpfree-sampler/src/engine/render.rs rust/mpfree-sampler/src/bandlimit.rs rust/mpfree-sampler/src/grain_clip.rs rust/mpfree-grain/src/buffer.rs rust/mpfree-stream/src/cursor.rs rust/mpfree-stretch/src/resample.rs
checked: 2026-11-23 50f7ac73
-->
# Pitched reads: one standard for every read at another speed

**Status:** standard written and every reader measured, 2026-11-23 (DSP quality step 1.6). The gaps at the end are open; each has an owner.

Five parts of mpfree play recorded audio at another speed: the sampler's pads, the sampler's granular pads, the grain engine (the Grains effect), the streaming cursor (stems and long sounds read from the card) and the stretch (pitch shift, and sounds loaded at another rate). Each keeps its own code, tuned to how it reaches its audio. This page is the one standard they are all measured against, so that any difference between them is chosen, not an accident. The measures are in `rust/mpfree-fxlab/src/readers.rs`; the plan this comes from is [DSP quality](dsp-quality.md).

## The readers

| Reader | Code | How it reads between samples | Cost a voice, from the bench |
|---|---|---|---|
| Sampler pad | `mpfree-sampler` `engine/render.rs`, `bandlimit.rs` | the sound's own samples at pitch; an 8-point windowed sinc up to two semitones up and for any pitch down; a band-limited read above two semitones up, from octave copies past an octave | 16 held stereo pads, filtered, pitched −6 to +5.5 semitones: 274 instructions a voice a frame; 3 to 14.5 semitones up: 740 |
| Vintage pad | the same | 4-point Hermite at every speed, no low-pass: aliasing is its sound | |
| Sampler grains | `mpfree-sampler` `grain_clip.rs` | the band-limited read above two semitones up; Hermite at and under | Grains worst, 5 pads: 4.0 M instructions a block; clean read 14.5 M |
| Grain engine | `mpfree-grain` `buffer.rs` (`Clip`, the live buffer) | Hermite at every speed | |
| Streaming cursor | `mpfree-stream` `cursor.rs` | band-limited, 8 zero crossings a side (the default) or 16; Hermite as the cheap choice | 64 voices at 1.5x: 782 instructions a voice a frame (8), 1,442 (16), 135 (Hermite) |
| Stretch | `mpfree-stretch` `resample.rs` (`convert_rate`) | a long windowed sinc, off the sound thread | a minute of 48 kHz stereo is about 300 million multiplies |

The pad and grain costs are a whole voice's (envelope, filter and mix included), from `rust/mpfree-bench/instructions.txt`; the streaming cursor's are its read alone.

## The standard

Every read is measured the same way: a sine whose output lands on one FFT bin, at 3, 9 and 13.5 kHz out, played from an octave down to an octave and a fifth up. A row whose sine would be above 21 kHz in the sound is left out, as is a speed a reader can't play (the streaming cursor's fastest is four times).

| Id | What | Sound thread | Off the sound thread |
|---|---|---|---|
| Q1 | Images: the loudest other bin from 20 Hz to 20 kHz, against the tone | −50 dB, on every row but the octave drops and 13.5 kHz a fifth down (step 3.3's) | −70 dB on every row |
| Q2 | Passband: the tone's level against the tone at pitch | within 1 dB, same rows, to 9 kHz out | within 0.5 dB wherever the sound's tone is at or under 20 kHz |
| Q3 | Folds: an 18 kHz sine a fifth up and a 15 kHz sine an octave up (27 and 30 kHz, which would fold back to 17.1 and 14.1 kHz): what is heard, RMS, against the tone | −50 dB | −70 dB |
| Q4 | Loop wraps: a loop of 100 periods of 1 kHz, the sound ending where the loop does, read a semitone up, two semitones down and a fifth up, against the same sound unrolled: the largest difference, against the tone's peak | −100 dB | (no loops) |

And, held by each reader's own tests rather than the lab:

- **At pitch, the sound's own samples**, bit for bit, from a whole frame.
- **Edges:** a read never looks outside its region or span, and a region's end is silence, not a read of whatever follows.
- **A change of read fades.** A read that changes mid-sound (the sampler picks one per voice per block) crossfades over 256 frames, about 6 ms; a read chosen per grain stays the same for the grain's life.
- **Cost:** each reader stays within its own limit on the instruction gate: the sampler's within the DSP quality limits (15 % on a voice), the others within their own plans.

Two reads are exceptions by choice, measured with no target: the **Vintage pad** (its aliasing is the sound it is for) and the **streaming cursor's Hermite** (the cheap read the sampler may pick when the sound thread is short; nothing picks it today).

## Where each read stands (23 Nov)

| Reader | Q1 images | Q2 passband | Q3 folds | Q4 loop wraps |
|---|---|---|---|---|
| Sampler pad | 20 of 20 (worst −55.1 dB) | 14 of 14 | 2 of 2 (−57.9) | **0 of 3 (−52.0 dB)** |
| Sampler grains | **11 of 20 (−26.3 at 9 kHz, −13.5 at 13.5 kHz)** | **13 of 14 (−1.1 dB)** | 2 of 2 (−54.9) | no loops |
| Grain engine | **9 of 20** | **13 of 14** | **0 of 2 (−2.4, 0.0 dB)** | no loops |
| Streaming cursor, 8 | 20 of 20 (−59.1) | 14 of 14 | 2 of 2 (−54.9) | 3 of 3 (exact) |
| Streaming cursor, 16 | 20 of 20 (−73.3) | 14 of 14 | 2 of 2 (−67.4) | 3 of 3 (exact) |
| Stretch | 23 of 23 (−82.9) | 22 of 22 | 2 of 2 (−94.1) | no loops |
| Vintage pad, stream Hermite | no target: −26.3 dB at 9 kHz two semitones down | no target: −1.1 dB at 9 kHz a fifth down | no target: −5.4 to 0.0 dB, folds come through almost whole | Vintage −50.8 (gap 1); Hermite exact |

The step 3.3 rows, tracked by every reader: an octave down at 9 kHz out is −22.5 dB on a pad, −41.8 on the streaming cursor's 8 and −71.6 on its 16; 13.5 kHz a fifth down is −9.3, −13.3 and −28.2.

The streaming cursor's default read is longer than the pad's 8-point read (16 taps a frame at its own speed or slower, against 8) and has more margin at 3 kHz; both meet the standard, so the difference is a choice of cost against margin, not a gap.

## Gaps and owners

1. **A pad's loop wrap reads past the loop's end** (Q4, −52 dB on a 1 kHz tone, worse on brighter sounds). Near the end of a loop, the 8-point, band-limited and Vintage reads take their neighbours from the sound after the loop end, not from the loop's start. When the loop ends where the sound does (common), the read clamps at the end, and every wrap ticks a little; when sound follows the loop, the wrap mixes in audio the loop never plays. With the sound continuing as the loop would, the difference vanishes, so it is the neighbours and nothing else. The streaming cursor gathers its frames round the loop and is exact. **Owner: DSP quality**, as a new step (the reads in `engine/render.rs` that step 1.4 built), with the sampler refactor thread.
2. **Sampler grains at and under two semitones up read Hermite** (Q1 −26 to −44 dB at 9 kHz, −13.5 to −22.4 at 13.5 kHz; Q2 −1.1 dB at 9 kHz a fifth down), where a pad playing the same sound uses the 8-point read. **Owner: DSP quality**, with the sampler refactor thread, once its cost is known: grains are many (256 at once), and the clean read already makes a burst 3.6 times dearer.
3. **The grain engine's own read is Hermite at every speed**, so a grain pitched up on the Grains effect folds back (Q3: an octave up, the fold is as loud as the tone). `fx-quality.md` has listed a cleaner grain read as a candidate since round 7. **Owner: DSP quality**, after gap 2, so both grain reads are chosen together.
4. **The streaming cursor's Hermite** fails every row it would be held to. It is an exception by choice, but the 8-point read the pads use meets the standard, and on the pads it cost 2.5 % more a held voice than Hermite did (step 1.4); its cost on the cursor is to be counted on the bench when it is offered. **Owner: the disk streaming thread**, offered the 8-point kernel in its place.
5. **Octave drops of bright sounds and 13.5 kHz a fifth down** fail on every read on the sound thread. **Owner: DSP quality step 3.3.**

## Tests that hold the rest

| | Sampler pad | Sampler grains | Grain engine | Streaming cursor | Stretch |
|---|---|---|---|---|---|
| At pitch, bit for bit | `a_plain_voice_renders_as_the_full_loop_does` | Hermite at a whole frame is the frame | the same | `a_file_at_the_engine_rate_plays_its_own_samples` | `convert_rate` returns the sound itself at the same rate |
| Never outside | `a_read_never_looks_outside_its_region`, `a_one_frame_region_reads_as_its_frame` | `grains_stay_inside_the_region` (grain engine) | `grains_stay_inside_the_region`; reads outside the span are 0 | `a_cursor_stops_at_its_end_and_reads_nothing_past_it` | before the first frame and after the last is silence |
| Loops | `a_seamless_loop_reads_as_the_unrolled_sound` (its sound continues past the loop, so it can't see gap 1); Q4 | no loops | no loops | `loops_seeks_and_speed_land_on_the_right_frames`; Q4 | no loops |
| Read changes fade | `a_voice_back_at_its_own_speed_fades_before_it_turns_plain` | fixed per grain | fixed per grain | nothing changes it today | |

## The ratchet

`tests/readers.rs` holds every number of the report to `rust/mpfree-fxlab/quality/readers.tsv`: a number worse than recorded by more than its tolerance, a new row, or a recorded row no longer measured fails it; targets not met are printed, not failed. It takes about 10 s in a debug build, so it runs with the workspace's tests. A change that improves a read records it with `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test readers`; one made worse on purpose is blessed the same way and explained in its pull request.
