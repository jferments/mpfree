<!--
covers: rust/mpfree-fx/src/eq.rs rust/mpfree-fx/src/svf.rs rust/mpfree-filter/src/svf.rs
checked: 2026-11-17 a336df3
-->
# EQ

**Status:** built; FX quality PR D (17 Nov, matched bells and shelves, Quality HQ 2x) is in. 79 of 90 measured targets met: every one at HQ 2x, and at Normal all but the broadest bells above 5 kHz. What is left is under [History and next](#10-history-and-next).

This page is the design of MPFree's parametric EQ (`mpfree.fx.eq`, [`eq.rs`](../../../rust/mpfree-fx/src/eq.rs) on the state-variable stage in [`mpfree-filter`'s `svf.rs`](../../../rust/mpfree-filter/src/svf.rs), shared with the pads): what an EQ is for, the ideas underneath it, the research behind ours, the algorithms and why, how quality and cost are measured, and how the defaults and presets were chosen. The parameter summary is the EQ entry in [fx.md](fx.md); the measured targets and their history are in [fx-quality.md](fx-quality.md#eq). The research notes, plan and baselines are in jferments/mpfree-claude `fx/quality/` (research-eq.md, plan.md section 6, baseline-2026-11-16.md). The pages that draw and drag it are the chain editor's EQ page and the Mix tab's EQ look (`mpfree-chain-ui`).

## 1. What an EQ is for

An EQ turns frequency ranges up or down. In a beat it does four jobs (research-eq.md section 1):

| Job | Typical move | What matters most |
|---|---|---|
| **Clean up** (rumble, mud, boxiness, a ring) | A low cut at 30 to 120 Hz; narrow cuts of 2 to 8 dB at Q 2 to 10 around 200 to 500 Hz or on a resonance | Steep, clean cuts; narrow cuts that don't ring; a way to find the frequency (Solo band) |
| **Shape tone** (weight, presence, air) | Broad bells and shelves of 1 to 6 dB at Q 0.5 to 1.5: 60 to 120 Hz, 2 to 5 kHz, 10 to 16 kHz | Smooth broad curves; boosts high up that sound like the analog EQs people learned on |
| **Make room** for another sound | Complementary cuts and boosts between two tracks | A display that tells the truth; the same curve on every track |
| **Effect** (telephone, sweeps, resonant peaks) | Steep cuts, big boosts, moving bands | No zips, stable at the extremes |

## 2. Principles

**What is heard.** Broad changes of about 1 dB are audible on music; peaks are much easier to hear than dips of the same size and width; narrow dips are hard to hear at all (Bücklein 1981; Toole and Olive 1988). So a broad boost must be accurate to a fraction of a dB, and boosts deserve tighter tolerances than cuts.

**The analog prototype.** Every parametric EQ people know descends from analog second-order sections: a bell `(s² + (A/Q)s + 1) / (s² + s/(AQ) + 1)` and shelves of the same order. Users carry their settings between EQs, so the digital curve should be the analog one: same peak, same width, same shape up to the top of the audio band.

**Cramping.** The usual way to make a digital filter from an analog one, the bilinear transform, maps the whole analog frequency axis onto 0 to Nyquist. Near Nyquist it squeezes: a bell at 12 kHz comes out narrower and falls back to 0 dB at Nyquist, where the analog bell is still boosting. At 44.1 kHz a +12 dB, Q 0.71 bell at 12 kHz lost 6.6 dB at 20 kHz this way. An "air" boost, one of the most common EQ moves, sounded thin. Remedies are filters designed to match the analog magnitude (Orfanidis 1997; Vicanek 2016) or running the bands at a higher rate.

**Conventions.** EQs set to the same numbers draw different curves because "it is a definition problem" (Rane): Q may be measured at -3 dB or at half the gain, a shelf's frequency may be its corner or its midpoint, some EQs narrow their cuts. A good EQ picks the conventions people know and says which.

**Structure.** A direct-form biquad in 32-bit float gets noisy at low frequencies and high Q, and a moving direct-form filter can blow up. The trapezoidal state-variable filter (Simper 2013) stays clean and stable at any setting and under any modulation, so a band can be swept by hand or by an LFO without zips.

**Phase.** A normal EQ is minimum phase like an analog one: its phase follows from its magnitude, with no latency. Linear phase is a different tool (latency, pre-ringing) and is not what this EQ is.

## 3. Research

The research note (`fx/quality/research-eq.md`) set FabFilter Pro-Q (the most-used plugin EQ, and the model for the EQ page), Ableton's EQ Eight and typical hardware channel EQs as the reference, and read the filter-design literature under [Sources](#11-sources). The findings that shaped the EQ:

- **What a sampler user reaches for first:** a low cut, two or three broad bands, a shelf for air, a narrow cut for a ring. MPFree had those. What would be noticed was the shape of the high bands, an analyzer and the 6 and 48 dB cut slopes.
- **The design method:** Vicanek's matched second-order filters (2016) take the poles by impulse invariance (`z = e^(sT)`, so a resonance sits where the analog one does and keeps its width) and choose the zeros so the magnitude matches the analog curve at chosen points. For a bell: the gain at DC, the gain at the centre, and a flat top there. His paper's claim, that this "avoids the notorious cramping near the Nyquist frequency", held in our measurements for narrow and mid-width bells. For the broadest bells high up, any second-order design at 44.1 kHz is a compromise (section 4).
- **Conventions in use:** the RBJ cookbook's Q (half-gain bandwidth: Q 0.71 is 1.9 octaves, Q 1.41 one octave) and a shelf whose frequency is its half-gain midpoint are both common. MPFree already used them; the work was to test and state them (E4).
- **Measured on 16 Nov before any change:** flat bit for bit, drawn equals heard within 0.05 dB, boost and cut exact mirrors, THD under -150 dB, clean switches; but bells above about 5 kHz cramped (a 16 kHz, Q 0.71 bell 7.9 dB short at 20 kHz, its width a half of the analog one), and fast Freq sweeps on narrow presets read 3 to 6 dB of "zipper".
- **The MPC's own EQ** (AIR) has not been measured: the FXProbe capture has not run on the device, so this page makes no comparison with it.

## 4. The engine

```
in ─► Low cut ─► High cut ─► band 1 ─► … ─► band 8 ─► Output gain ─► out
      (Off, 12 or 24 dB/oct Butterworth)   (each: bell, shelf or notch, or flat)
                                     └─ Solo band: one band's band-pass, crossfaded in
```

- **The filter:** every section is Simper's trapezoidal SVF (`svf::Svf`), stereo, one 12 dB per octave stage; a 24 dB cut is two. A band at 0 dB, a band that is not On, and a cut that is Off rest and cost nothing, and at its defaults the EQ is the input bit for bit.
- **The design (PR D):** the SVF is kept for its stability, and only how its coefficients are made changes. Any stable digital biquad is the trapezoidal transform of some analog prototype `m0 + (m2 + m1 s) / (s² + k s + 1)`, so a biquad designed another way maps exactly onto the SVF's `g`, `k`, `m0`, `m1`, `m2` (`SvfCoefs::from_biquad`) and runs, sweeps and draws as before.
  - **Bells** (`SvfCoefs::matched`): poles by impulse invariance; zeros matched to the analog magnitude at DC (1), at the centre (the gain) and a zero slope there (Vicanek's peaking design), written in `p = sin²(ω/2)` and solved for the minimum-phase numerator.
  - **Shelves:** RBJ's analog shelf (half its gain in dB at the frequency), poles and zeros each mapped by impulse invariance at the frequencies the analog shelf puts them, then scaled to the analog gain at DC.
  - **Cuts and the notch:** the bilinear designs as before (Butterworth cuts measure as designed; a notch is a notch).
  - **Precision:** the design runs in `f64` (the mapping needs it at low frequencies) and costs a few `exp`, `cos` and `sqrt`. While a band moves it designs every 16 frames and glides the coefficients between designs (`SvfCoefs::toward`), every step a stable filter; a still band costs what it did. The old design ran in `f32` every frame.
- **Why not a simpler fix:** prewarping the centre (what the RBJ cookbook does) puts the peak in the right place but cannot widen a bell that Nyquist squeezes. Raising the Q at high frequencies by a rule of thumb is wrong at every gain but one. Oversampling everything doubles the cost for every user, every time.
- **What a matched filter at 44.1 kHz cannot do:** a second-order digital filter has one fewer degree of freedom near Nyquist than its analog prototype. The matched bell meets the analog curve at DC, at its peak and at Nyquist and is up to about 1 dB over it between the peak and Nyquist on the broadest boosts above 5 kHz (about 2 dB on deep cuts). That is the residue the Quality choice exists for.
- **Quality HQ 2x:** the bands, Solo band and the output gain run at twice the rate between polyphase IIR half-bands (four first-order all-pass sections each way, flat to 20 kHz within 0.002 dB, 36 dB down at the image of 20 kHz; up then down is one all-pass, about 1.6 frames late at low frequencies). The cuts stay at the base rate, ahead of the half-bands: they have no curve to keep near Nyquist, and it saves a third of the cost. A change of Quality wakes the other path, lets it settle unheard and crossfades over 20 ms. At HQ 2x every bell is within 0.035 dB of its prototype to 20 kHz.
- **Switching without clicks:** a slope change crossfades over about 20 ms; a band type change glides the band's gain to 0 dB (where every type passes the input exactly), swaps and glides back; leaving On glides the band flat the same way; a notch crossfades from the input into a true notch, so it switches as smoothly as a bell easing to 0 dB.
- **Solo band** (id 45; called Listen until 16 Nov): plays one band alone through a band-pass at its frequency and Q, scaled to unity at its peak, crossfaded in and out over 20 ms, so you can hear what the band touches. Pages turn it off when they close; it is not saved with a sound.
- **Drawing:** `Eq::response_db` gives the curve the settings are heading to, read from the same coefficients the filters play, so the screen is exact (E2: within 0.05 dB of a measured sine, 0.004 dB at HQ 2x counting the half-bands).

## 5. Controls

| Control | Range (default) | Notes |
|---|---|---|
| Low cut, Low slope | 20 Hz to 2 kHz (80 Hz); Off, 12 dB, 24 dB (Off) | Butterworth |
| High cut, High slope | 1 to 20 kHz (12 kHz); Off, 12 dB, 24 dB (Off) | Butterworth |
| Band n Type | Bell, Low shelf, High shelf, Notch (band 1 low shelf, band 4 high shelf, the rest bells) | A change glides through 0 dB |
| Band n Freq | 20 Hz to 20 kHz (bands 1 to 8: 100, 400, 2500, 8000, 60, 250, 1000, 5000 Hz) | Log |
| Band n Gain | ±18 dB (0) | A notch ignores it |
| Band n Q | 0.1 to 18 (0.71); a shelf's stops at 2 | RBJ half-gain Q; a shelf at Q 2 rings a 5 dB bump past its corner at full gain, at 18 it would be 24 dB |
| Band n Use | Unused, On, Bypassed (bands 1 to 4 On, 5 to 8 Unused) | Only On is heard; Unused bands are off the page |
| Output | ±18 dB (0) | |
| Solo band | Off, Band 1 to 8 | The page sets it while a band is touched |
| Quality | Normal, HQ 2x (Normal) | HQ 2x costs about 1.7 times as much |

Ids only grow. Bands 1 to 4 keep the four-band EQ's ids (4 to 19); bands 5 to 8 (21 to 36), the Use params (37 to 44), Solo band (45) and Quality (46) came after Output, so a project saved before any of them sounds and draws as it did. Below about 3 kHz the matched design changes curves by under 0.1 dB, so saved EQs sound the same except for bands high up, which now have their analog shape.

## 6. Defaults and presets

The defaults predate the quality work and stayed: a flat EQ that is the input bit for bit, with its four starting bands where the four jobs happen (a low shelf at 100 Hz for weight, bells at 400 Hz for mud and 2.5 kHz for presence, a high shelf at 8 kHz for air), Q 0.71 (about two octaves, the broad tone-shaping width), and the cuts placed (80 Hz, 12 kHz) but off, so turning one on gives a sensible starting point. The four extra bands sit between them (60, 250, 1000, 5000 Hz) so a band added on the page starts somewhere useful. Normal Quality is the default because it meets every target below 5 kHz at the base cost.

Presets are starting points for the jobs a beat needs most, each a few dB, not a finished mix:

| Preset | What it does | Job |
|---|---|---|
| Kick weight | Low cut 30 Hz (24 dB); +4 dB at 60 Hz; -4 dB at 350 Hz (Q 1.2); +3 dB at 3.5 kHz (Q 1.4) | Shape: weight and beater click, less box |
| Snare crack | Low cut 120 Hz; +3 dB at 200 Hz; -3 dB at 800 Hz; +4 dB at 5 kHz; +2 dB shelf at 10 kHz | Shape |
| Vocal clarity | Low cut 100 Hz (24 dB); -2 dB shelf at 200 Hz; -3 dB at 300 Hz; +3 dB at 3 kHz (Q 0.8); +3 dB air shelf at 12 kHz | Clean up and shape |
| Thin for layering | Low cut 250 Hz (24 dB); -2 dB at 500 Hz; -3 dB high shelf | Make room |
| Warm and dark | High cut on (12 dB); +2 dB at 120 Hz; -6 dB shelf at 6 kHz | Tone |
| Telephone | Cuts at 400 Hz and 3.5 kHz (24 dB); +6 dB at 1.5 kHz; Output -2 dB | Effect |

A preset never sets Quality (`page_keeps`), so a user's HQ choice survives a preset change.

## 7. Measuring quality

`fxlab quality --class eq` measures 90 numbers on the EQ at its defaults, every preset, and again at HQ 2x (`eq[HQ 2x]`), at 44.1 kHz. The ratchet (`rust/mpfree-fxlab/quality/ratchet.tsv`, on CI) fails a pull request that makes any worse. fxlab's `prototype` module holds the analog curves (RBJ bell and shelves, Butterworth cuts) the measures compare with.

| Id | What | Target | Today (Normal; HQ 2x) |
|---|---|---|---|
| E1 | Flat when flat | Bit for bit | Met (a gate) |
| E2 | Drawn equals heard | Within 0.05 dB | Met; 0.004 dB |
| E3 | Bells (+12 dB, 1 to 16 kHz, Q 0.71 to 4) and shelves (+6 dB) against the prototype, 20 Hz to 20 kHz; half-gain width | Within 0.5 dB; width within 5 % | Every Q 4 bell and every bell to 8 kHz at Q 1.41; the 0.71 bells at 5 to 16 kHz 0.51 to 0.93 dB and the 1.41 bells at 12 and 16 kHz 0.67 and 0.80 dB over; HQ 2x within 0.035 dB, widths within 0.3 % |
| E4 | Q 1.41 is one octave at half gain; a shelf is at half its gain at its frequency | ±0.05 oct; ±0.1 dB | 0.0004 oct |
| E5 | A +9 and a -9 dB bell in series | Flat within 0.01 dB | 0.0000 dB |
| E6 | THD of a +12 dB, Q 4 bell at 100 Hz | Under -140 dB | Met |
| E7 | Zipper while a control sweeps | ≤ 3 dB | The default 0.013 dB; the narrow presets' Freq sweeps 3.1 to 6.2 dB (below) |
| E8 | Type, Use, slope and Quality changes | No spike past the signal | Unit tests |
| E9 | A drag on the page measured at the speaker | Within 0.5 dB and 1/12 octave of the dot | Not built yet (the app's smoke session) |
| E10 | Solo band at unity, the band's width | | Met (unit test) |
| E11 | Cost | The instruction gate | Section 8 |

**Not a zipper:** fxlab sweeps a band from end to end in 50 ms; a narrow, deep band sweeping past a test tone changes the tone's level by that much whatever the filter does. The rows are kept (the ratchet stops them getting worse) and explained rather than chased.

Unit tests in `tests/eq.rs`: defaults bit for bit flat; the reported curve is what a sine measures; Butterworth cuts; type and slope changes don't click; a band at 0 dB is flat for any type and Q; the four-band ids and sound are kept; the new bands measure as drawn; a notch cuts deep and narrow; bypass glides; Solo band plays the band alone; bells follow the analog curve up to Nyquist; shelves follow the analog curve; Q and the shelf frequency follow the conventions; a gliding band lands on the curve it draws; HQ plays what it draws, its bells follow the analog curve to 20 kHz, Quality switches without a click, and HQ rings out within its tail.

Listening: PR D had a blind, loudness-matched A/B page for Jay (air boosts, broad presence, narrow cuts, before against after and Normal against HQ 2x).

## 8. Cost

Instructions per 128-frame block at 44.1 kHz, counted by callgrind on CI (`rust/mpfree-bench/instructions.txt`, `tools/ci/instr_gate.py`):

| Case | Before PR D | Now |
|---|---|---|
| EQ, four bands and both cuts, still | 108k | 108k |
| The same with all four bands' Freq gliding | 332k | 256k |
| The same at HQ 2x | | 188k |

The design runs every 16 frames in `f64` instead of every frame in `f32`, which is why a glide got cheaper. HQ 2x is 1.7 times Normal: the bands twice, the cuts once, and 16 first-order sections a frame for the half-bands.

## 9. Pitfalls and how the design avoids them

| Pitfall | Avoided by |
|---|---|
| Cramped, dull air boosts | The matched design; HQ 2x for the last dB (E3) |
| Settings that don't transfer from other EQs | RBJ Q and midpoint shelves, tested (E4) and stated in the manual |
| Zips on sweeps; blow-ups under modulation | The trapezoidal SVF; designs glided every 16 frames, each step stable |
| Clicks on type, Use, slope or Quality changes | Glides through 0 dB, crossfades (E8) |
| Low-frequency noise at high Q | The SVF in place of a direct-form biquad (E6) |
| A display that lies | `response_db` reads the coefficients the filters play (E2) |
| Loudness bias when comparing settings | Listening tests are loudness matched; auto gain is planned for users |
| Steep cuts ringing | Cuts stop at 24 dB per octave |

## 10. History and next

- **Before 16 Nov:** a four-band EQ grew to eight bands with Use, the notch, Listen and the EQ page (the EQ thread, #388 and earlier), on bilinear SVF designs.
- **16 Nov:** Listen renamed **Solo band** (the app thread, #428).
- **PR D (17 Nov, #446):** matched bells and shelves in the same SVF, designs every 16 frames in `f64`, and Quality HQ 2x. 21 of 50 targets met before, 79 of 90 after (HQ 2x's 40 rows all met).

**Still to do** (plan.md section 6.2):

1. **Finger-to-sound tests (E9)** in the app's smoke session, with the app thread: drag a band's dot, play a sweep, measure the gain at the dot.
2. **Cut slopes of 6 and 48 dB per octave.**
3. **Auto gain** (Output follows the curve's loudness change, so comparing is fair), off by default.
4. **An analyzer** behind the curve (Off, Pre, Post), once a spectrum tap from the audio thread fits the budget.
5. Later: typed values, mid/side per band, dynamic bands, proportional Q.

## 11. Sources

- M. Vicanek, "Matched second order digital filters", 2016, and "Matched two-pole digital shelving filters" (vicanek.de).
- S. J. Orfanidis, "Digital parametric equalizer design with prescribed Nyquist-frequency gain", JAES 45(6), 1997.
- R. Bristow-Johnson, "Cookbook formulae for audio EQ biquad filter coefficients"; "The equivalence of various methods of computing biquad coefficients for audio parametric equalizers", AES 97th Convention, 1994.
- A. Simper, "Linear trapezoidal integrated SVF", Cytomic, 2013.
- Rane, "Why DSP boxes set the same way differ" and "Bandwidth in octaves versus Q in bandpass filters" (RaneNotes).
- V. Välimäki and J. Reiss, "All about audio equalization: solutions and frontiers", Applied Sciences 6(5), 2016.
- R. Bücklein, "The audibility of frequency response irregularities", JAES 29(3), 1981; F. Toole and S. Olive, "The modification of timbre by resonances", JAES 36(3), 1988.
- FabFilter Pro-Q and Ableton EQ Eight feature sets (product pages and manuals).
