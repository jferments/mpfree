<!--
covers: rust/mpfree-fxlab/src/readers.rs rust/mpfree-fxlab/tests/readers.rs rust/mpfree-fxlab/quality/readers.tsv rust/mpfree-sampler/src/engine/render.rs rust/mpfree-sampler/src/engine/voices.rs rust/mpfree-sampler/src/engine/follow.rs rust/mpfree-sampler/src/bandlimit.rs rust/mpfree-stream/src/resample.rs rust/mpfree-sampler/src/grain_clip.rs rust/mpfree-grain/src/buffer.rs rust/mpfree-stream/src/cursor.rs rust/mpfree-stretch/src/resample.rs
checked: 2026-11-23 4cc96132
-->
# Pitched reads: one standard for every read at another speed

**Status:** standard written and every reader measured, 2026-11-23 (DSP quality step 1.6). Step 1.7 closed two of the gaps at the end (a pad's loop wraps, the streaming cursor's Hermite); the others are open, each with an owner.

Five parts of mpfree play recorded audio at another speed: the sampler's pads, the sampler's granular pads, the grain engine (the Grains effect), the streaming cursor (stems and long sounds read from the card) and the stretch (pitch shift, and sounds loaded at another rate). Since the audio tracks' phase 5a a sixth, an audio track's region following the tempo on Repitch, reads through the streaming cursor's code (`read_gathered`, which `Cursor::play` calls bit for bit as before). Each keeps its own code, tuned to how it reaches its audio. This page is the one standard they are all measured against, so that any difference between them is chosen, not an accident. The measures are in `rust/mpfree-fxlab/src/readers.rs`; the plan this comes from is [DSP quality](dsp-quality.md).

## The readers

| Reader | Code | How it reads between samples | Cost a voice, from the bench |
|---|---|---|---|
| Sampler pad | `mpfree-sampler` `engine/render.rs`, `bandlimit.rs` | the sound's own samples at pitch; an 8-point windowed sinc up to two semitones up and for any pitch down; a band-limited read above two semitones up, from octave copies past an octave | 16 held stereo pads, filtered, pitched −6 to +5.5 semitones: 274 instructions a voice a frame; 3 to 14.5 semitones up: 740 |
| Vintage pad | the same | 4-point Hermite at every speed, no low-pass: aliasing is its sound | |
| Sampler grains | `mpfree-sampler` `grain_clip.rs` | the pad's 8-point read at and under two semitones up and for any pitch down; the band-limited read above (DSP quality step 1.8; Hermite at and under before it) | Grains worst, 5 pads: 4.8 M instructions a block (4.0 M with Hermite); clean read 14.2 M |
| Grain engine | `mpfree-grain` `buffer.rs` (`Clip`, the live buffer) | the pad's 8-point read at and under two semitones up; faster, a Kaiser-windowed sinc stretched by the speed, up to an octave's stretch (step 1.8; Hermite at every speed before it) | Grains FX (dense): 529 k instructions a block (473 k with Hermite) |
| Streaming cursor | `mpfree-stream` `cursor.rs` | band-limited, 8 zero crossings a side (the default) or 16; the pads' 8-point read as the cheap choice (`Read::Near`, its cut stretched with the speed past 1) | 64 voices at 1.5x: 782 instructions a voice a frame (8), 1,442 (16), 453 (8-point); at 0.75x, 225 (8-point) |
| Audio tracks, Repitch | `mpfree-audiotracks` `lib.rs` (`Reader::add_pitched`), through `mpfree-stream` `cursor.rs` (`read_gathered`) | the streaming cursor's band-limited read, 8 zero crossings a side, on frames the track gathers itself, at every speed but 1 (at 1, the recording's own samples), up to four times ([tempo-follow.md](tempo-follow.md), phase 5a) | as the streaming cursor's 8; its own rows and bench rows come with phase 5a.2 |
| Stretch | `mpfree-stretch` `resample.rs` (`convert_rate`) | a long windowed sinc, off the sound thread | a minute of 48 kHz stereo is about 300 million multiplies |

The pad and grain costs are a whole voice's (envelope, filter and mix included), from `rust/mpfree-bench/instructions.txt`; the streaming cursor's are its read alone.

## The standard

Every read is measured the same way: a sine whose output lands on one FFT bin, at 3, 9 and 13.5 kHz out, played from an octave down to an octave and a fifth up. A row whose sine would be above 21 kHz in the sound is left out, as is a speed a reader can't play (the streaming cursor's fastest is four times).

| Id | What | Sound thread | Off the sound thread |
|---|---|---|---|
| Q1 | Images: the loudest other bin from 20 Hz to 20 kHz, against the tone | −50 dB, on every row but the octave drops and 13.5 kHz a fifth down (step 3.3's) | −70 dB on every row |
| Q2 | Passband: the tone's level against the tone at pitch | within 1 dB, same rows, to 9 kHz out | within 0.5 dB wherever the sound's tone is at or under 20 kHz |
| Q3 | Folds: an 18 kHz sine a fifth up and a 15 kHz sine an octave up (27 and 30 kHz, which would fold back to 17.1 and 14.1 kHz): what is heard, RMS, against the tone | −50 dB | −70 dB |
| Q4 | Loop wraps: a loop of 100.6 periods of 1 kHz (its seam a step), the sound ending where the loop does, read a semitone up, two semitones down and a fifth up, against the loop unrolled (the sound to the loop's end, then the loop over and over) read straight through: the largest difference, against the tone's peak | −100 dB | (no loops) |

And, held by each reader's own tests rather than the lab:

- **At pitch, the sound's own samples**, bit for bit, from a whole frame.
- **Edges:** a read never looks outside its region or span, and a region's end is silence, not a read of whatever follows.
- **A change of read fades.** A read that changes mid-sound (the sampler picks one per voice per block) crossfades over 256 frames, about 6 ms; a read chosen per grain stays the same for the grain's life.
- **Cost:** each reader stays within its own limit on the instruction gate: the sampler's within the DSP quality limits (15 % on a voice), the others within their own plans.

One read is an exception by choice, measured with no target: the **Vintage pad** (its aliasing is the sound it is for). The streaming cursor's cheap read, the one the sampler may pick when the sound thread is short (nothing picks it today), was Hermite, an exception too; since step 1.7 it is the pads' 8-point read, held to the standard.

## Where each read stands (23 Nov, after steps 1.7 and 1.8)

| Reader | Q1 images | Q2 passband | Q3 folds | Q4 loop wraps |
|---|---|---|---|---|
| Sampler pad | 20 of 20 (worst −55.1 dB) | 14 of 14 | 2 of 2 (−57.9) | 3 of 3 (exact; −52.0 dB before step 1.7) |
| Sampler grains | 20 of 20 (−55.1; 11 of 20 before step 1.8) | 14 of 14 (−0.35 dB; 13 of 14 before) | 2 of 2 (−54.9) | no loops |
| Grain engine | 20 of 20 (−55.1; 9 of 20 before step 1.8) | 14 of 14 (13 of 14 before) | 2 of 2 (−54.9 and −68.3; −2.4 and 0.0 dB before) | no loops |
| Streaming cursor, 8 | 20 of 20 (−59.1) | 14 of 14 | 2 of 2 (−54.9) | 3 of 3 (exact) |
| Streaming cursor, 16 | 20 of 20 (−73.3) | 14 of 14 | 2 of 2 (−67.4) | 3 of 3 (exact) |
| Streaming cursor, 8-point | 20 of 20 (−55.2) | 14 of 14 | **1 of 2 (−30.8 dB at 18 kHz a fifth up)** | 3 of 3 (exact) |
| Stretch | 23 of 23 (−82.9) | 22 of 22 | 2 of 2 (−94.1) | no loops |
| Vintage pad | no target: −26.3 dB at 9 kHz two semitones down | no target: −1.1 dB at 9 kHz a fifth down | no target: −5.4 and −3.0 dB, folds come through almost whole | exact (−50.8 dB before step 1.7) |

The step 3.3 rows, tracked by every reader: an octave down at 9 kHz out is −22.5 dB on a pad, −41.8 on the streaming cursor's 8 and −71.6 on its 16; 13.5 kHz a fifth down is −9.3, −13.3 and −28.2.

At its own speed or slower the streaming cursor's 8-point read is the pad's, from the same table and summed the same way, so their rows match within 0.005 dB; faster, it stretches where the pad hands over to its band-limited read, and that is where its one fold row misses. The streaming cursor's default read is longer than the pad's 8-point read (16 taps a frame at its own speed or slower, against 8) and has more margin at 3 kHz; both meet the standard, so the difference is a choice of cost against margin, not a gap.

## Gaps and owners

1. **Closed in step 1.7: a pad's loop wrap read past the loop's end** (Q4 was −52 dB on a 1 kHz tone). Near the end of a loop, the 8-point, band-limited and Vintage reads took their neighbours from the sound after the loop end, not from the loop's start, so a loop ending where its sound does ticked at every wrap. Each read now folds its neighbours round the loop as the voice plays it (`Voice::fold`), and every pad read is exact on Q4, as the streaming cursor, which gathers its frames round the loop, always was.
2. **Closed in step 1.8: sampler grains at and under two semitones up read Hermite** (Q1 was −26 to −44 dB at 9 kHz, −13.5 to −22.4 at 13.5 kHz; Q2 −1.1 dB at 9 kHz a fifth down), where a pad playing the same sound used the 8-point read. They now read with the pad's 8-point read there (`GrainClip`'s `NEAR` aim), so a pad's grains read as the pad does at every pitch. The owner chose it on its cost (23 Nov): the bench's 5 granular pads count 17.6 to 18.8 % more instructions.
3. **Closed in step 1.8: the grain engine's own read was Hermite at every speed**, so a grain pitched up on the Grains effect folded back (Q3: an octave up, the fold was as loud as the tone). It now reads with the 8-point read at and under two semitones up and, faster, a windowed sinc stretched by the speed (`mpfree_grain::buffer`'s clean reads), which keeps out what would fold back up to an octave up; past an octave the stretch stays an octave's, as the live buffer has no octave copies. The Grains effect counts 9.4 to 11.9 % more instructions.
4. **Closed in step 1.7: the streaming cursor's Hermite** failed every row it would be held to. At the disk streaming thread's request it is replaced by the pads' 8-point read, which meets every row but one fold: an 18 kHz tone a fifth up, −30.8 dB against −50 (`Read::Near`'s doc says so). Passing it would take a longer kernel faster than speed 1, which is what the default read is. Its cost, 64 voices at 1.5x: 3.7 M instructions a block, against Hermite's 1.1 M and the default's 6.4 M.
5. **Octave drops of bright sounds and 13.5 kHz a fifth down** fail on every read on the sound thread. **Owner: DSP quality step 3.3.**

## Tests that hold the rest

| | Sampler pad | Sampler grains | Grain engine | Streaming cursor | Stretch |
|---|---|---|---|---|---|
| At pitch, bit for bit | `a_plain_voice_renders_as_the_full_loop_does` | not since step 1.8: a grain at pitch reads through the 8-point read, whose cut (20.1 kHz) takes 0.35 dB off 13.5 kHz at any fraction; Hermite gave a whole frame exactly but took up to 2.3 dB off between frames | the same | `a_file_at_the_engine_rate_plays_its_own_samples` | `convert_rate` returns the sound itself at the same rate |
| Never outside | `a_read_never_looks_outside_its_region`, `a_one_frame_region_reads_as_its_frame` | `grains_stay_inside_the_region` (grain engine) | `grains_stay_inside_the_region`; reads outside the span are 0 | `a_cursor_stops_at_its_end_and_reads_nothing_past_it` | before the first frame and after the last is silence |
| Loops | `a_loop_ending_with_its_sound_reads_as_the_unrolled_loop`, `a_ping_pong_loop_reads_as_the_mirrored_loop`, `a_loop_start_moved_past_a_voice_that_went_round_plays_on_smoothly`; Q4 | no loops | no loops | `loops_seeks_and_speed_land_on_the_right_frames`; Q4 | no loops |
| Read changes fade | `a_voice_back_at_its_own_speed_fades_before_it_turns_plain` | fixed per grain | fixed per grain | nothing changes it today | |

## The ratchet

`tests/readers.rs` holds every number of the report to `rust/mpfree-fxlab/quality/readers.tsv`: a number worse than recorded by more than its tolerance, a new row, or a recorded row no longer measured fails it; targets not met are printed, not failed. It takes about 10 s in a debug build, so it runs with the workspace's tests. A change that improves a read records it with `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test readers`; one made worse on purpose is blessed the same way and explained in its pull request.
