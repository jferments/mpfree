<!--
covers: rust/mpfree-fx/src/lofi.rs
checked: 2026-11-17 0becf78
-->
# Lo-fi: design

**Status:** 2026-11-17, FX quality round 2's Lo-fi PR. **Lo-fi** (`mpfree.fx.lofi`; the MPC's AIR Lo-Fi opens as it) is worn tape and dusty vinyl under one **Amount** knob: wow that wanders, flutter, saturation at twice the rate, dropouts, crackle, hiss and hum that follow the music or run on, a low and a high cut and width. Its short entry is in [fx.md](fx.md); its measured targets are in [fx-quality.md](fx-quality.md) (ids L1 to L15). The research notes and the plan this page draws on are in jferments/mpfree-claude `fx/quality/` (research-lofi.md, plan-2.md section 8, decision 10.5). The digital side of lo-fi, bits and sample rate, is [Crush](fx-crush.md).

## 1. What it is for

| Job | Typical move | What matters most |
|---|---|---|
| Age and warmth on keys, guitars or a mix ("lo-fi hip hop") | A slow, slight pitch drift, a little hiss and crackle, the top rolled off at 6 to 10 kHz, gentle saturation | Believable, not seasick; the noise under the music, not on top of it |
| A medium you can name: cassette, VHS, worn reel, vinyl, AM radio | Presets: cassette (flutter, hiss, a 6 to 12 kHz top), vinyl (crackle, rumble, a narrow image), radio (300 Hz to 3 kHz, hum) | Recognisable at once |
| A broken machine | Wow of 1 % or more, dropouts, heavy saturation | Dramatic but musical and controllable |
| A noise bed under a beat | Crackle and hiss on their own, running whether the music plays or not | Noise that can run without the music, or follow it |

The tools most producers know are XLN RC-20 Retro Color (Noise with Follow, Wobble, Distort, Digital, Space and Magnetic modules under one Magnitude), iZotope Vinyl (dust, crackle, warp, mechanical and electrical noise), Wavesfactory Cassette, Aberrant SketchCassette, Baby Audio TAIP and Ableton's Vinyl Distortion.

## 2. Principles

**Wow and flutter** are speed errors of a transport, heard as pitch: wow is slow (under about 4 Hz), flutter faster (4 to 100 Hz). They are measured on a 3.15 kHz tone (IEC 60386, AES6) as the peak or RMS deviation of its frequency in percent. For scale: professional tape about 0.02 % (inaudible), a good cassette deck 0.08 %, a portable player 0.2 to 0.5 %, and 1 % is heard easily. A lo-fi effect has to span about 0.05 % to a few percent. Pitch comes from a delay line whose length moves: a delay that changes by `d'(t)` frames a frame plays at `1 - d'(t)` of the speed, so the pitch deviation is the delay's slope, and the delay must hold the integral of the deviation.

**Real wow is irregular.** A capstan's eccentricity gives a line, but the tape's tension, the reels and the pinch roller add slow random motion, so worn wow wanders. A plain sine at one rate reads as vibrato or chorus (the research's judgement, and why RC-20 adds its Flux randomness). The mean pitch must stay exact, or the track goes out of tune.

**Tape saturation** is soft and mostly odd: a tape's maximum output level is defined at 3 % third harmonic, with a little second from asymmetry. Like any curve it makes harmonics above the Nyquist frequency that fold back as inharmonic aliases, so it is oversampled and antialiased as Drive is ([fx-drive.md](fx-drive.md)).

**Noise has to be at a believable level and behave as chosen.** Cassette without noise reduction has an S/N of about 50 to 60 dB; vinyl's surface noise is of the same order with clicks on top. On a song with lo-fi on several tracks, noise that never stops piles up and fills the gaps; for a noise bed it has to keep going. Hence Follow (as RC-20's Noise).

**A fair A/B needs the same loudness.** Cuts and saturation take level away, and an effect that is quieter loses every comparison. A fixed makeup at the defaults is enough: the cuts' loss doesn't depend much on the material at mix levels.

## 3. Research

The full notes are research-lofi.md. What shaped the engine:

- **Before this PR** (measured on main): wow and flutter topped out at 0.39 % and 0.13 % peak (0.52 % together), a portable player, not a broken machine; the wow was two fixed sines (0.53 and 0.89 Hz) the same on both sides, and its strongest spectral line held 81 % of its power; the saturation had no oversampling (a 5 kHz full-scale sine aliased at -28 dB at the default Saturation and -16 dB at 100); the defaults were 1.3 LU quieter than the dry; at Amount 0 the cuts stayed at 20 Hz and 20 kHz, so inserting it changed the sound; noise always followed the music; there were no dropouts, no stereo wow, no hum.
- **Features musicians expect** (research-lofi.md section 3): a rate and irregularity for the wow, stereo wow, dropouts and wear, noise that follows or not, hum as a noise type (iZotope Vinyl's electrical noise), media presets. Decision 10.5 (Jay, 17 Nov): the fixes plus Follow, dropouts, stereo wow and hum. Tape stop is left for later (Perform FX has one).

## 4. Our engine

Per frame, in this order:

1. **The glides.** Every control glides over 30 ms. Amount scales every part; the cuts move between 20 Hz and 20 kHz and their set corners along Amount on a log scale.
2. **Wow.** A delay line per channel (`Tape`, Hermite interpolation) centred 10 ms back. Its length follows a smooth random path (`Wander`): from one random point to the next along half a cosine, each leg 0.15 to 0.85 of a period of **Rate**, its steepest slope 0.35 to 1 of the whole wow, mostly back across the centre (one leg in five, while there is room, goes on away from it), and never further than the tape allows. The pitch keeps about the Rate and size set but never repeats, and the mean pitch is exact because the path always comes back. Right follows its own path, and **Stereo** blends from the left's path (0) to its own (100).
3. **Flutter.** Two sines at 7.1 and 11.3 Hz, weighted 0.7 and 0.3, added to both sides' delay.
4. **The size curve.** For the knob's share `k` (Amount times Wow or Flutter) the peak deviation is `dev(k) = BASE k (1 + LIFT k³)`: the bottom is the first version's straight line (0.4 % and 0.15 % at full), so saved settings and the presets barely move, and the top reaches a broken machine (2.2 % of wow, 0.55 % of flutter). The deepest wow fills 9 ms of the tape's 10, which is why the latency rose from 2 ms.
5. **Saturation** at twice the rate: Drive's Eco half-bands (`IirPair<5>`, flat to 0.19 of the doubled rate, 71 dB down from 0.31), the Soft curve with a bias of 0.15 times Saturation for even harmonics (its offset taken out), up to 18 dB of drive and back, averaged by first-order ADAA with `Flat2x`'s compensation after it. Below 5 % of saturation (Amount times Saturation) the curve fades into the straight line, so a little saturation doesn't switch in with a step. The drive and bias are worked out again only while the saturation glides.
6. **Dropouts.** Now and then the level dips by 4 to 24 dB (with **Dropouts**) for 20 to 150 ms, down and back along 8 ms raised cosines, at random gaps averaging `1 / (4 (0.06 + 0.94 w²))` seconds for wear `w`: from one every 4 s at a touch to four a second at full.
7. **The cuts**: a 12 dB per octave high-pass and low-pass (the shared `Svf`), after the saturation and dropouts.
8. **Noise**: hiss (white noise through a 6 kHz one-pole, about -45 dBFS RMS at full), crackle (clicks at random, mostly small and now and then a duller pop, up to 40 a second) and **Hum** (the **Mains** fundamental, 50 or 60 Hz, and four harmonics, weighted 1, 0.45, 0.3, 0.12 and 0.08, as a transformer's buzz has them). All three ride a follower of the input (10 ms up, 350 ms down, in full from -18 dBFS peak) blended by **Follow**: `ride = 1 - follow + follow × envelope`. At 100 % they rise with the music and fade after it; at 0 they run on.
9. **Makeup**, 2.5 dB at Amount 100, on everything above.
10. **Width** narrows the side signal (1 - Amount × (1 - Width)).
11. **Mix** against the dry, delayed by the same 445 frames. Amount's first 2 % also fades the Mix in (`mix × min(1, Amount / 0.02)`), so at Amount 0 the output is the delayed input to the bit.

The oversampler's filters settle their state every 64 frames on a frame count, so any block size gives the same output.

## 5. Controls

| Control | Range, default | What it does |
|---|---|---|
| Amount | 0 to 100 %, 60 | Scales everything; 0 is the input |
| Wow, Rate | 0 to 100 %, 40; 0.2 to 4 Hz, 0.7 | Slow drift and about how fast it wanders |
| Flutter | 0 to 100 %, 30 | Fast waver |
| Stereo | 0 to 100 %, 0 | How far the right side's wow follows its own path |
| Dropouts | 0 to 100 %, 0 | How often and how deep the dips are |
| Saturation | 0 to 100 %, 40 | Up to 18 dB of drive into the soft curve |
| Crackle, Noise, Hum | 0 to 100 %; 30, 20, 0 | The three noises |
| Mains | 50 Hz or 60 Hz, 60 | Hum's fundamental |
| Follow | 0 to 100 %, 100 | How far the noises follow the music |
| Low cut, High cut | 20 to 500 Hz, 80; 2 to 20 kHz, 8 kHz | The band |
| Width | 0 to 100 %, 70 | Age narrows the image |
| Mix | 0 to 100 %, 100 | Against the dry |

Ids 0 to 9 are the first version's; Rate, Stereo, Dropouts, Hum, Mains and Follow are 10 to 15 and start where the first version sounded (Stereo, Dropouts and Hum off, Follow at 100), so a saved Lo-fi keeps its character, with the fixes. The page: Lo-fi (Amount, Saturation), Tape (Wow, Rate, Flutter, Stereo, Dropouts), Noise (Crackle, Noise, Hum, Follow and Mains as a row), Tone (the cuts with their drawing, Width) and Output. The Wear card shows a bar for each part at its current share, Dropouts and Hum among them, and the wow's peak in percent; the Tone card draws `LoFi::cuts_db`, makeup included.

## 6. Defaults and presets

The defaults are a gentle tape: wow and flutter together 0.126 % peak (a worn cassette deck), the default Saturation, a little crackle and hiss that follow the music, 80 Hz to 8 kHz at Amount 100. The presets set the sound and leave Mix as it is (Amount stays at its default but in Broken machine, which reads 1.39 % peak), and each plays pink noise within 1 LU of the dry but Old radio (its 420 Hz to 3.2 kHz band, -1.55 LU, reported):

| Preset | For | Notable settings |
|---|---|---|
| Worn tape | A tape that has been played a lot | Wow 60, Saturation 60, Dropouts 15 |
| Dusty vinyl | An old record | Crackle 75, 70 Hz to 7.5 kHz, Width 60 |
| Cassette | A portable player | Flutter 65, Noise 45, 6 kHz top, Stereo 15 |
| Old radio | AM radio | 420 Hz to 3.2 kHz, mono, Saturation 85, Hum 30 |
| Broken machine | Seasick on purpose | Amount 100 (the only preset that sets it: its wow needs the top of the curve), Wow 85 at 0.9 Hz, Flutter 70, Saturation 50, Stereo 35, Dropouts 35 |
| VHS | A video tape's sound | Wow 55 at 1.6 Hz, Noise 55, 6.5 kHz top, Stereo 35, Hum 15 |
| Vinyl bed | Noise under a beat | Crackle 60, Follow 0, an open 40 Hz to 12 kHz |

Worn tape, Cassette and Old radio gained their dropouts, stereo wow and hum in this PR; Broken machine, VHS and Vinyl bed are new.

## 7. Measuring quality

`fxlab quality --class lofi` ([fx-quality.md](fx-quality.md), Lo-fi (L)) measures the defaults and each preset. The pitch rows listen for 60 s: a random wander needs that long to read its size and to tell it from a line (over a few seconds a random path can look like one). The quiet presets' pitch is read with Dropouts, Hum and Stereo off, since a dip or the two sides drifting apart beat in the measure's mid. The level is read over 9 s rather than the usual 1 s, because one dropout more or less in a short window moves it by a dB.

| Id | What | Target |
|---|---|---|
| L1 | Wow and flutter, peak: the defaults; Amount and Wow at 100; Amount 0 | Within ±20 % of 0.116 %; at least 2 %; under 0.001 % |
| L2 | Amount and Flutter at 100 | At least 0.5 % |
| L3 | Mean pitch | Within 0.01 % |
| L4 | The pitch track's strongest line | At most half its power |
| L5 | A 5 kHz sine at 0 dBFS, Saturation 40 | Alias ≤ -65 dB (100 reported) |
| L6, L7 | Third over second harmonic; noise levels; with Follow 0, the noise from playing to silence | Reported; within 1 dB |
| L8 | The heard cuts against the drawn | Within 0.5 dB |
| L9 | Pink at -18 dBFS, defaults and presets | Within ±1 LU (Old radio reported) |
| L10 | Amount 0 | Bit for bit |
| L11 | Zipper on every control | ≤ 3 dB |
| L12 | A dropout's depth and its fall | Within 1 dB of the set depth; at least 5 ms |
| L13 | Latency | Fixed and reported |
| L14, L15 | Stereo at 100, the sides' pitch apart; Hum at 100 | Reported |

All 38 targets are met. Wow and flutter at the defaults 0.126 % peak; the top 2.15 % (wow) and 0.50 % (flutter), from 0.39 and 0.13; mean pitch within 0.0074 %; the strongest line 0.19 of the wow's power, from 0.81; Saturation 40's alias -89.2 dB and 100's -49.6, from -27.6 and -16.3; the third harmonic 4.4 dB over the second at Saturation 40 and 9.5 at 100; the noise 40.1 dB under a -18 dBFS tone and -79.9 dBFS in silence with Follow at 100, -52.4 dBFS running on at Follow 0 and the same while the music plays (0 dB apart); the cuts within 0.006 dB of the drawing; Amount 0 bit for bit; zipper at most 0.41 dB; a dropout within 0.005 dB of its depth, falling over 6 ms (10 to 90 %), about 2.8 a second at full; Stereo 100 puts the sides 3.2 % apart in pitch; Hum 100 at -37.5 dBFS; latency 445 frames. Pink at -18 dBFS: the defaults +0.17 LU (from -1.30), Worn tape +0.07, Dusty vinyl +0.26, Cassette -0.50, Broken machine -0.58, VHS -0.41, Vinyl bed +0.67, Old radio -1.55 (from -3.01).

Listening: a blind, loudness-matched A/B page of main against this PR goes with the PR.

## 8. Cost

The instruction gate's `Lo-fi` case (rustc 1.97.0, a 128-frame stereo block at 44.1 kHz, the defaults at Amount 100): 222.0k instructions, from 117.0k (1.9 times), next to Drive's Eco at 202.6k: the saturation at twice the rate with ADAA is most of it (its two channels cost about as much as the rest of the effect together). Wow's path, dropouts and hum cost a few operations a frame and nothing while off. About 6 % of a period on the MPC by round 7's fit (the Compressor's 27.7k is about 0.8 %); the device round measures the real share. If it matters, Saturation 0 could skip the oversampling once its glide has settled.

## 9. Pitfalls and tests

| Pitfall | Guard |
|---|---|
| Too subtle at the top, seasick at the bottom | The size curve; L1, L2; `wow_bends_the_pitch_by_about_what_it_says` |
| Wobble that reads as vibrato | The random path; L4 |
| The track out of tune | The path comes back across the centre; L3 |
| Aliasing from the saturation | 2x with ADAA; L5 |
| Noise piling up, or never stopping | Follow at 100 by default; L7; `crackle_and_hiss_ride_the_music_and_rest_in_silence`, `follow_zero_keeps_the_noise_running_in_silence` |
| A level drop that loses every A/B | The makeup; L9 |
| Inserting it changes the sound | Amount's first 2 % fades it in; L10; `amount_zero_is_the_input_to_the_bit_and_on_time`, `mix_zero_is_the_input_delayed_by_the_latency` |
| Clicks from dropouts | Raised-cosine ramps; L12; `dropouts_dip_by_their_depth_along_ramps` |
| Stereo or hum not doing what they say | L14, L15; `stereo_moves_the_sides_apart_and_zero_keeps_them_together`, `hum_sits_on_the_mains` |
| A block size changing the sound | The frame-count settle grid; `any_block_size_sounds_the_same` (with a control change mid-run) |
| The drawing lying | `the_lofi_cuts_draw_what_they_do` (tests/curves.rs); L8 |
| A preset out of range or silent | `presets_are_in_range_and_play`, `every_preset_plays` |

## 10. History and next

- **First version:** wow and flutter from two pairs of sines (up to 0.4 % and 0.15 %), saturation at the base rate, crackle and hiss that followed the music, the cuts and width, a 2 ms tape; Worn tape, Dusty vinyl, Cassette, Old radio.
- **Round 2, Lo-fi PR:** the random wow path with Rate and Stereo, the size curves, saturation at 2x with ADAA, Dropouts, Hum and Mains, Follow, the makeup, Amount 0 bit for bit, a 10 ms tape; Broken machine, VHS and Vinyl bed. Saved projects' Lo-fi keeps its settings and gets the fixes: no fizz on high notes, a wow that wanders at about the same size, 1 to 1.5 dB louder, 8 ms more latency (compensated).
- **Next:** tape stop and spin-up; noise types (tape, vinyl, VHS colours of hiss); a Duck on the noise as RC-20 has; a cheaper path with Saturation at 0; the device round's cost.

## 11. Sources

- IEC 60386, DIN 45507 and AES6 on wow and flutter measurement; the summary and audibility notes in Wikipedia's "Wow and flutter measurement"; Lindos, "Wow and flutter measurement".
- XLN Audio, RC-20 Retro Color manual (Follow, Flux, Magnetic's wear and dropouts).
- iZotope Vinyl documentation (mechanical and electrical noise, warp).
- "Basics of tape performance", Audio, September 1982 (maximum output level at 3 % third harmonic).
- J. D. Parker, V. Zavalishin and E. Le Bivic, "Reducing the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016 (first-order ADAA), as in [fx-drive.md](fx-drive.md).
