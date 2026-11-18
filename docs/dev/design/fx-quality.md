<!--
covers: rust/mpfree-fxlab
checked: 2026-11-18 dd1645f
-->
# FX quality: targets, the report and the ratchet

**Status:** measuring built 2026-11-16 (delay, EQ and reverb). The sound changes so far are below: PR B (the delays' and reverbs' Mix curve, the Room's level and the Tape and Multi-tap tails), PR C (the delays: flat reads, saturation at twice the rate, the Tape's colour on every echo and a Feedback that is its loop gain) and PR D (the EQ's bells and shelves matched to their analog curves). The fixes come one class at a time, each in its own pull request, judged by these numbers and by blind, level-matched listening. Research and the full plan: jferments/mpfree-claude `fx/quality/` (plan.md, research-delay.md, research-eq.md, research-reverb.md, baseline-2026-11-16.md).

An effect is good when it meets its measured targets, wins or ties in blind, level-matched listening, has the controls musicians know, and fits the MPC's budget. This page holds the first of those: each target as a number with its reason, how fxlab measures it, and the ratchet on CI that stops any number getting worse.

**Second round** (measuring built 2026-11-17): drive (Drive and Character), crush, compressor, filters (Multi and DJ) and lo-fi, by the same method, in [Round 2](#round-2-drive-crush-compressor-filters-and-lo-fi) below; the amp joined it on 17 Nov (class `amp`, ids A; plan-2 section 12, research-amp.md). Research and plan: jferments/mpfree-claude `fx/quality/` (plan-2.md, research-drive.md, research-crush.md, research-compressor.md, research-filter.md, research-lofi.md, baseline-2026-11-17.md).

**Fourth round** (measuring built 2026-11-17): the rest of the dynamics (Limiter and Master safety, Gate, De-esser, Multiband compressor, Ducker, Transient shaper, Channel strip), in [Round 4](#round-4-the-rest-of-the-dynamics) below; the fixes come one class at a time. Research and plan: jferments/mpfree-claude `fx/quality/` (research-dynamics.md, plan-4.md, baseline-dynamics-2026-11-17.md).

**Fifth round** (measuring built 2026-11-17): the movement, pitch and voice effects (the shared LFO, Tremolo, Auto-pan, Rotary, Auto wah, Ring mod, Frequency shifter, Pitch shifter, Pitch correct, Harmonizer, Tuner and Talk box), in [Round 5](#round-5-movement-pitch-and-voice) below; the fixes come one class at a time, and a Vocoder joins with its own. Movement and pitch are done (PRs W and 2 to 6, 18 Nov: every target met); the talk box and the vocoder (PRs 7 and 8, plan-5 sections 10 and 11) are held for a later session. Research and plan: jferments/mpfree-claude `fx/quality/` (research-movement.md, research-pitch.md, research-voice.md, plan-5.md, baseline-r5-2026-11-17.md).

**The sampler** (measuring built 2026-11-17; the sampler review's engine item 7): a pad of `mpfree-sampler` measured by the same kind of numbers, with its own ratchet: [The sampler](#the-sampler) below.

**The synths** (measuring built 2026-11-18; synth library stage 2): every synth engine and factory preset by the same method, with its own ratchet (`tests/synth.rs`, `quality/synth.tsv`): [Synth quality](synth-quality.md).

## The report

`fxlab quality [--class CLASS]... [--out DIR]` (CLASS: `delay`, `eq`, `reverb`, `drive`, `crush`, `compressor`, `filter`, `lofi`, `amp`, `modulation`, `limiter`, `gate`, `deesser`, `multiband`, `ducker`, `transient`, `strip`, `movement`, `pitch` or `voice`) measures every target of a class (all of them when none is named) and prints one row per number: the target id, what was measured, the value, the target and whether it is met; DIR/quality.json holds the same. Subjects are the modes of the effect's family at their defaults (`delay/digital`, `reverb/hall`), and for the cheaper measures each family preset in its mode (`reverb/plate[Vocal plate]`). The EQ's subjects are `eq`, `eq[preset]` and `eq[HQ 2x]` (its Quality at HQ 2x); an engine that isn't in a family is `class/engine` (`crush/crush[Chip]`). All twenty classes take about 75 s in a release build (the round-2 classes 39 s: Lo-fi about 24 s of it, for its minute-long pitch reads, and most of the rest the Drive's 4x quality; the modulation class about 3 s; the seven dynamics classes of round 4 about 2.5 s together; the three of round 5 about 8 s).

New analyses in `rust/mpfree-fxlab` for it, each tested on signals with known answers:

| Module | What it measures |
|---|---|
| `echo` | A delay's repeats from its impulse response: where each lands (its group delay from 1 to 4 kHz, the slope of its phase, so fractions of a frame show and an all-pass read reads at its true delay), its level at 1, 5, 10 and 15 kHz, left against right; loss per repeat from a line through the repeats' tilts |
| `prototype` | The analog curves the EQ follows (RBJ cookbook bell and shelves, Butterworth cuts), the worst difference of a digital curve from them, the half-gain bandwidth and the octaves an RBJ Q stands for |
| `mixlaw` | Loudness change (BS.1770) against the dry across Mix 0 to 100 % on pink noise and on drum-like hits; `Law` gives the gains of the linear, equal-power and dry-first curves |
| `knob` | Decay against T60 at mid frequencies (the mean of the 500 Hz and 1 kHz octaves' T30); Pre-delay against when the reverb starts |
| `clarity` | C50 and C80, EDT over T30, and which side a reverb starts on when fed from the left |
| `flutter` | Wow and flutter of a 3.15 kHz tone (IEC 386, AES6): instantaneous frequency of the isolated tone, peak and RMS deviation in percent, unweighted |
| `wobble` | The spread, in cents RMS, of a held 440 Hz tone's spectrum after the effect: how much modulation smears a note; 0 for an unmodulated path |
| `quality` | The report above, its text and JSON, and the ratchet |

## The ratchet

`tests/quality.rs` runs the whole report and compares every row that has a better direction with `rust/mpfree-fxlab/quality/ratchet.tsv` (`key<TAB>value`). It fails when a number is worse than recorded by more than its tolerance, when a row is new, or when a recorded row is no longer measured. Targets not met yet are printed, not failed, so quality can only go up while the work to meet them goes on.

- A change that improves numbers records them: `MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test quality`. The pull request shows the file's diff.
- A number made worse on purpose (a deliberate change of character) is blessed the same way and explained in the pull request.
- Levels far below hearing read as -140 dB, and zipper steps below 0 dB as 0, so numerical noise is never held to account.
- CI runs it in the `rust` job (release, about a minute). In a debug build the test is ignored: the renders take minutes there.

## Delay

Engines: Digital (`delay.rs`), Tape and Multi-tap (`echoes.rs`), the Delay family's modes. The loop is measured "open": tone at its brightest, no saturation, wow or flutter, one head, Feedback 80 %, Mix 100 %; the multi-tap with only tap 4 (the one that feeds back).

| Id | Row | Target | Why |
|---|---|---|---|
| D1 | `time_error_frames` | First repeat within 0.5 frame of Time (its group delay from 1 to 4 kHz) | Grooves |
| D2 | `hidden_loss_10k_db`, `hidden_loss_15k_db` | Loss per repeat at a time half a frame off a whole frame, minus the loss at a whole frame: ≤ 0.1 dB at 10 kHz, ≤ 0.3 dB at 15 kHz | Brightness must not depend on the tempo; loss compounds per repeat |
| D4 | `runaway_peak_dbfs`, `runaway_dc_dbfs` | At the highest Feedback, 2 s of noise then 8 s ringing: peak ≤ +6 dBFS, DC ≤ -60 dBFS | Throws that howl but never blow up |
| D5 | `alias_5k_0_db` | Worst alias of a 5 kHz full-scale sine ≤ -70 dB, every preset | Aliases recirculate |
| D6 | `mix_dip_lu`, `mix_bump_lu` | Loudness within -1.0 to +1.5 LU of the dry at every Mix | No dip; going in shouldn't make it quieter |
| D7 | `zipper_worst_db` | ≤ 3 dB on every control, every preset | No zips; the Time modes (Repitch, Fade, Jump) add their own rows |
| D8 | `first_echo_colour_share` | The Tape's first echo carries at least 40 % of the second's darkening at 10 kHz | Every echo was on the tape |
| D9 | `flutter_off_peak_pct` | Wow and Flutter at 0: ≤ 0.01 %; at the defaults and at 100 % reported | From a clean deck to a worn machine |
| D10 | `pingpong_side_db` | Ping-pong repeats alternate, each at least 20 dB louder on its side | Familiar |
| D11 | `mono_sum_lu` | The wet signal's mono sum within 3.5 LU of stereo | Phones and club systems |
| D12 | `tail_cut_db` | Level when the slot stops processing (its tail estimate after 1 s of noise) ≤ -60 dB under the input, every preset | No cut-off echoes |

D3 (loop filters equal their drawing: `mpfree-fx/tests/delay.rs`), D13 (exact silence, the gates' `idle_db`) and D14 (cost, the instruction gate, which counts the Delay, Tape delay and Multi-tap delay cases) are checked elsewhere.

## EQ

Engine: `eq.rs` on `svf.rs`. Curves are the EQ's own `response_db`, which E2 holds to what it plays.

| Id | Row | Target | Why |
|---|---|---|---|
| E2 | `drawn_vs_heard_db` | Measured response within 0.05 dB of the drawn curve | The screen tells the truth |
| E3 | `bell_<f>_q<Q>_worst_db`, `..._width_ratio` | +12 dB bells at 1 to 16 kHz, Q 0.71 to 4: within 0.5 dB of the analog prototype from 20 Hz to 20 kHz, half-gain width (both measured inside that band) within 5 %; at Normal and at HQ 2x | Users learned on analog-style EQs; about 1 dB is audible on broad curves |
| E3 | `high_shelf_<f>_worst_db`, `low_shelf_<f>_worst_db` | +6 dB shelves within 0.5 dB of the prototype | The same |
| E3 | `low_cut_...`, `high_cut_..._worst_db` | Against Butterworth down to -12 dB: reported, no target yet | A digital low-pass near Nyquist falls faster than the analog one |
| E4 | `q_convention_oct`, `shelf_midpoint_db` | Q 1.41 is one octave at half gain (±0.05); a shelf is at half its gain at its frequency (±0.1 dB) | Settings transfer between EQs |
| E5 | `mirror_db` | A +9 and a -9 dB bell in series flat within 0.01 dB | Expected |
| E6 | `thd_100_db` | A +12 dB, Q 4 bell at 100 Hz under -140 dB THD | Clean in 32-bit float |
| E7 | `zipper_worst_db` | ≤ 3 dB on every control, every preset | No zips |

E1 (flat when flat) is a gate in `tests/gates.rs`; E8 (switches without clicks) is in the EQ's own tests; E9 (a drag on the screen measured at the speaker) goes in the app's smoke session; E10 (Solo band) and E11 (cost) are checked elsewhere.

## Reverb

Engines: Room, Plate, Spring (`rooms.rs`) and the Cloud engine's Hall, Cloud and Echoes (`cloud.rs`), the Reverb family's modes, wet only (Mix 100 %) unless a row says otherwise.

| Id | Row | Target | Why |
|---|---|---|---|
| R1 | `decay_error_pct@<s>s` | T60 at mid frequencies within ±10 % of Decay at three or four settings | JND about 5 % |
| R3 | `edt_t30_log2` | EDT/T30 from 0.8 to 1.25 (Hall and Cloud up to 1.6; Echoes reported) | No slap and then a faint tail |
| R4 | `diffuse_ms` | Echo density 0.9 within Plate 30 ms, Room 50, Hall and Cloud 100 | No flutter or grain |
| R5 | `tail_excess_over_ideal_db` | Tail excess no more than 1.5 dB over an ideal noise tail of the same T60 (Spring and Echoes reported) | No metallic ring |
| R6 | `wet_lu` | Wet loudness within ±1 LU of the dry on pink noise, every mode and preset | Switching modes doesn't jump |
| R7 | `mix_dip_lu`, `mix_bump_lu` | As D6 | Going in shouldn't make it quieter |
| R8 | `iacc_tail`, `mono_sum_lu` | Tail IACC ≤ 0.2; mono sum within 3.5 LU | Wide but mono-safe |
| R9 | `wobble_cents` | A held 440 Hz tone smeared ≤ 3 cents RMS at the default modulation | No chorusing on piano |
| R10 | `early_side_db` | Fed from the left, the first 50 ms at least 3 dB louder on the left | The reverb keeps the source's place |
| R11 | `zipper_worst_db` | ≤ 3 dB on every control, every preset | No zips |
| R12 | `predelay_error_frames` | The start moves by Pre-delay within one frame | Familiar |

`c80_db` is reported for each mode. R2 (Damping does what it says), R13 (Freeze and spillover) and R14 (cost) are still to measure or checked elsewhere.

## Baseline (2026-11-16, before any fix)

55 of 81 delay targets, 21 of 50 EQ targets and 68 of 114 reverb targets met. The large gaps: delay repeats lose 0.74 dB per repeat at 10 kHz and 3.5 dB at 15 kHz when the time falls between frames (the default 1/8 dotted at 120 BPM does); the Tape's first echo has none of the tape's colour; every delay and reverb dips 2 to 6 LU as Mix turns (a linear crossfade); EQ bells above 5 kHz are cramped (a 12 kHz, Q 0.71 bell is 6.6 dB short at 20 kHz and 42 % narrower than its prototype); Room is 4.9 LU quieter than the dry, starts with a slap (EDT/T30 0.40), takes 314 ms to diffuse and rings 4.9 dB over an ideal tail; Plate takes 167 ms to diffuse and smears a held note by 4.9 cents; no reverb but the Cloud engine's keeps a source's side.

## PR B: the Mix curve, the Room's level and the echo tails

What changed (design: [fx.md](fx.md#the-delays-and-reverbs-mix-curve)): every delay and reverb blends on an equal-power curve by default, with **Dry first** and **Linear** as choices (D6, R7); the Room's wet signal is 4 dB louder (R6); and the Tape and Multi-tap work their tails out from Feedback instead of a fixed 8 s (D12). After it: 62 of 79 delay targets, 21 of 50 EQ targets and 78 of 114 reverb targets met.

- **Met now:** every delay and reverb mode dips less than 1 LU as Mix turns (it was 2 to 6); the Room and its four presets are within 1 LU of the dry (they were 3 to 4.9 LU under); every Multi-tap preset's tail is cut under -60 dB (Build up was cut at -28.5 dB, the defaults at -52.5); the Tape's Space heads went from -100 to under -140 dB.
- **Rings on:** the Tape's Dub echo and Runaway gain on each pass (their saturation lifts quiet echoes up to 1.43 times at Saturation 55 %, and the head bump adds 2 dB), so their tails are endless and a bypassed slot rings on until Feedback comes down. Their D12 rows (they were cut at -8.6 and +0.5 dB) are notes now ("rings on"); making those loops lose again is a sound change for PR C.
- **Blessed worse, and why:** at the same Mix the equal-power curve plays more of the effect (at 35 %, the wet is 3.5 dB louder and the dry 2.4 dB), and the Room's trim adds 4 dB more, so every number measured on the blend against the input moved by about that much: the delays' runaway peaks and DC (D4; the peaks still 10 dB inside the target), their aliases (D5), the Digital delay's tail cuts (D12) and the Slapback's Time sweep (D7), and the zipper of the Room's and the Snare plate's Pre-delay and Size (their wet path's own Doppler on a time sweep, now louder against the dry; R11) and of the Digital delay's Mix sweep on two presets (D7: an equal-power blend of two signals in phase adds up to 3 dB half way; 1.5 dB at most, inside the 3 dB target). With the curve set to Linear and the Room's trim taken out, every one of those rows comes back to its old value; only the Tape's default and Worn tape tails stay shorter (cut at -115 and -120 dB where the fixed 8 s left -140), still 55 dB inside the target.
- **Cost:** no bench case rises: the Delay's 128-frame block counts 72.5k instructions against the baseline's 77.7k (its gains rest once they arrive), and the Cloud engine's modes 0.2 to 0.6 % more.

## PR C: the delays

What changed (design: [fx.md](fx.md), the Delay, Tape delay and Multi-tap entries): all three delays read a fraction of a frame through a flat all-pass while the time is still (`kit::FlatRead`; the cubic while it glides); the Digital delay's and the Tape's loop saturation runs at twice the rate (`kit::Os2`); the Tape's tone, head bump and saturation sit at the record head, so the first echo has been round the tape once; and the Tape's saturation never adds gain, so Feedback is the loop's gain (its range goes to 150 % so Runaway still runs away). After it: 74 of 80 delay targets met; EQ and reverb unchanged.

- **Met now:** the repeats keep their top end at any time (D2: a repeat half a frame off lost 0.74 to 0.76 dB more at 10 kHz and 2.3 to 3.6 dB more at 15 kHz; now within 0.01 dB, the Tape 0.17 dB at 15 kHz); every delay preset's 5 kHz alias is under -87 dB (D5: the Digital's Tape runaway preset was -57, the Tape's Slapback -44); the Tape's first echo carries half the second's darkening (D8: 0.50, it had none); the Tape's Dub echo dies away again (D12: its tail is cut at -76 dB; it rang on) and the Digital's Dub echo and Quarter repeats tails are cut 10 to 30 dB lower (-74 and -73 dB, the tail counts one pass more).
- **Still unmet:** the Time sweeps of the Digital's Slapback and of the Tape at its defaults, Slapback, Space heads and Worn tape (D7, as before: a time change slides in pitch, and the P2 time modes are the fix), and the Multi-tap's Mix bump (below).
- **Measured differently:** fxlab's D1 now reads where a repeat lands from its group delay from 1 to 4 kHz, not its energy centroid, which puts an all-pass read half a frame late when it is on time. The delays read 0.013 frame (the all-pass is a hundredth of a frame late by 4 kHz) and the Tape 0.41: its first echo now goes through the tone, and a 5 kHz low-pass delays 1 to 4 kHz by about that much. Both inside 0.5.
- **Blessed worse, and why:** the Tape's runaway (D4, at Feedback 150 %, the new top) peaks at -2.8 dBFS and its DC at -63 dB (were -4.2 and -70), inside +6 and -60; the Tape's default and Worn tape tails are cut at -73 and -76 dB (were -112 and -116): the tail is now the passes at Feedback times the tone's peak, where the saturation's lift made it much longer than it needed to be, still 13 dB inside the target. The Digital's and the Multi-tap's full wet keep their top end, so they measure louder against the dry: the Digital's Mix bump 0.38 LU (was 0.01, target 1.5) and the Multi-tap's 1.69 LU (was 1.44), now 0.19 over the target; a trim on the Multi-tap's taps is left for its own change rather than moving its level here. Some zipper rows rose inside the target: the Digital's Mix sweep 0.53 dB, the Multi-tap's Triplet scatter Tap 1 level 2.7 dB, the Tape's Dub echo Saturation 2.8 dB and Runaway Tone 1.0 dB (both measured 0 while the loop ran away and hid every step). The Tape's Slapback Time sweep, unmet before (the tape's time slides in pitch; the P2 time modes are the fix), rose from 5.1 to 6.9 dB, because its echo plays 2.5 dB up against the dry (`TAPE_TRIM`). The Digital's default and Dotted ping-pong tails moved by 0.03 dB.
- **Cost** (the instruction gate's cases, rustc 1.97.0, a 128-frame block at 44.1 kHz, against PR B): the Delay 72.5k to 70.9k (the flat read costs less than the cubic, which pays for the oversampled saturator), the Tape delay 70.1k to 87.9k (1.25 times, inside the plan's 1.3), the Multi-tap delay 89.2k to 95.2k. The Tape and Multi-tap cases are new on the gate with this PR.

## PR D: the EQ

What changed (design: [fx.md](fx.md), the EQ entry): the bells and shelves are designed to follow their analog curves instead of the bilinear transform's cramped ones. A bell's poles are its analog poles mapped exactly (impulse invariance) and its zeros are chosen so the gain is right at DC, at the peak and flat at the peak (Vicanek's matched design); a shelf's poles and zeros are both mapped, at the frequencies the analog shelf puts them, and scaled to its gain at DC. The design runs in double precision every 16 frames while a band moves and glides the filter's coefficients in between; a still band costs what it did. The cuts are unchanged. A new **Quality** choice, HQ 2x, runs the bands at twice the rate between IIR half-bands for when the top octave must be exact. After it: 39 of 50 EQ targets met at Normal (21 before) and all 40 of HQ 2x's own rows, 79 of 90 in all; delay and reverb unchanged.

- **Met now:** every bell at Q 4 and every bell up to 8 kHz at Q 1.41 is within 0.5 dB of its prototype from 20 Hz to 20 kHz, and every bell's half-gain width is within 5 % of the prototype's but one (below). Before, a 16 kHz bell fell up to 7.9 dB short of its prototype, a 12 kHz bell 3.1 to 6.6 dB, and both were far narrower (a third and three fifths of the width, measured the old way, below). The 12 kHz high shelf is within 0.31 dB (it was 1.03 dB over); every other shelf within 0.07. Q follows the RBJ convention to 0.0004 octave (it was 0.005).
- **Still unmet:** the broad bells high up: Q 0.71 at 5, 8, 12 and 16 kHz (0.53, 0.86, 0.93 and 0.51 dB) and Q 1.41 at 12 and 16 kHz (0.80 and 0.67 dB). A matched filter at the normal rate meets the curve at DC, the peak and Nyquist and is up to about 1 dB over it between the peak and Nyquist on the broadest boosts (about 2 dB on deep cuts). Meeting 0.5 dB to 20 kHz takes bands at twice the rate: Quality HQ 2x (below). Below 20 kHz the 16 kHz, Q 0.71 bell is 5.4 % wider than its prototype.
- **HQ 2x** (`eq[HQ 2x]`, every E3 row again, plus E2 and E7): every row met. The bells are within 0.035 dB of their prototypes to 20 kHz and their widths within 0.3 %, the shelves within 0.003 dB; what it draws is what it plays within 0.004 dB with a 12 kHz, Q 0.71 bell measured to 20 kHz (E2, which counts the half-bands); the default's sweep reads 0.013 dB (E7). The cuts stay at the base rate, so their rows are Normal's.
- **Measured differently:** a bell's half-gain width is now measured inside 20 Hz to 20 kHz for both curves, the band the target is set on. Before, a digital width cut off at Nyquist was set against an analog one that ran on, so a bell whose upper edge lay above Nyquist could never meet it however exact it was (at HQ 2x the 16 kHz bells at Q 0.71 and 1.41 read 0.75 and 0.94). Every width row moved a little with the finer search; the 12 kHz, Q 0.71 bell now reads 1.009 (was 0.973) and the 16 kHz, Q 1.41 1.030 (was 0.986, blessed).
- **Not a zipper:** the presets' Freq sweeps read 3.1 to 6.2 dB (E7, as before and 0.01 to 0.04 dB lower): fxlab sweeps a band from end to end, and a narrow, deep band sweeping past a tone changes its level that much whatever the filter. The default's sweep, a broad band, reads 0.013 dB. A gliding band now lands exactly on the curve it draws (`tests/eq.rs`).
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): a still EQ is within 1 % of before (109k); the new case with all four bands' Freq gliding counts 256k instructions a block, against 332k for the same glide before (the old design ran in single precision every frame; this one runs in double precision every 16 frames). The new HQ 2x case, the same EQ at HQ 2x, counts 188k, 1.7 times Normal: the bands twice, the cuts once, and 16 first-order sections a frame for the half-bands.

## PR E: the Room

What changed (design: [fx.md](fx.md), the Room entry): the Room is a new reverb under the same node id and parameters. Each side's input goes through four short all-passes (a new **Diffusion** control), then into twelve early reflections a side, heard mostly on their own side, and into a sixteen-line feedback delay network mixed by a Hadamard matrix, each line damped by Jot's one-pole set from its length, the eight longest drifting slowly. The reflections stay out of the network. It works through each block in chunks, a stage at a time, on a fixed grid, so its output is the same, bit for bit, at any block size. After it: 87 of 114 reverb targets met (78 before); delay and EQ unchanged.

- **Met now, every Room row at its defaults and presets:** Decay within 4 % from 0.3 to 5 s (R1; it was 16 % short at 0.3 s); one slope, EDT/T30 0.91 (R3; it was 0.40, a slap then a faint tail); dense in 18 ms (R4; it took 314); 1.26 dB over the ideal tail (R5; it was 4.93, a metallic ring); a source on the left keeps 4.3 dB more early energy on the left (R10; it summed to mono); the start moves by exactly Pre-delay (R12; it was a frame off); no control zips over 2.4 dB on the defaults or any preset (R11; up to 6.8 before, Size and Pre-delay sliding, and Damping now glides); every preset within 1 LU of the dry (R6).
- **Blessed worse, and why:** a held note smears by 1.87 cents (R9; 0.17 before, the old Room barely moved), inside the 3-cent target: fewer wobbling lines leave the tail ringing (four: 3.15 dB over the ideal tail). The mono sum is 2.31 LU down (R8; 1.92 before), inside 3.5.
- **The colour target's margin:** R5 compares against one ideal noise tail. Against the mean of four, the Room reads about 1.65 dB over at its defaults and 2.0 over nine Size and Decay settings near them. More wobble or longer lines buy little (all sixteen lines wobbling: 1.6, with a 5-cent smear; lines 1.6 times longer: 1.6, with fewer echoes early on), so this is about where a sixteen-line network sits at 0.8 s.
- **Not yet:** R2 (Damping's ratio) is still unmeasured; Plate, Spring, Hall and Cloud come next (the Plate in PR E2 and the Spring in PR E3, below).
- **Tests** (`mpfree-fx/tests/room.rs`): the ids saved projects use, Decay at 44.1, 48 and 96 kHz and four sizes, Pre-delay moving the whole wet signal exactly, a source keeping its side, the tail ringing out, Size, Damping, Diffusion and Decay swept under noise, and the same output, bit for bit, at any block size; in `rooms.rs`, the outputs read off the transform equal the output vectors, and no line is shorter than a chunk at any rate. The Mix curve test (`tests/mix_curve.rs`) now lets an engine add controls after the Mix curve when the test names them, here the Room's Diffusion: the curve still comes after every control the engine had before it.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, Mix 100 %): 145.9k instructions, against 171.9k for the old Room on the same case (15 % fewer). The Room, Plate and Spring cases are new on the gate with this PR (Plate 186.7k and Spring 185.6k, unchanged). The old Room cost 155k on ARMv7 under qemu (the fx.md table); the device round measures the new one.

## PR E2: the Plate

What changed (design: [fx.md](fx.md), the Plate entry): the Plate is a new reverb under the same node id and parameters, built on the Room's sixteen-line network (`Net`, now shared, each reverb a `Voice`). Each side's input goes through Tone and four short all-passes, then into a dense onset (twenty taps a side from 1 to 29 ms, the same at every Size, stronger on the source's side) and into the network at a plate's lengths (32 to 153 ms at size 1, Size scaling them 0.55 to 1.05 times). There are no early reflections: a plate has no walls. After it: 98 of 114 reverb targets met (87 after PR E); delay, EQ and the Room unchanged (the Room's code moved into `Net` with no change to its sound: its rows, golden case and instruction count are as PR E left them).

- **Met now, every Plate row at its defaults and presets:** Decay within 1.1 % at 0.5, 2 and 6 s (R1; it was 14.6 % long at 0.5 s); dense in 10 ms (R4; it took 167 ms); 0.38 dB over the ideal tail (R5; it was 2.75, a ring at 3.7 kHz); a source on the left keeps 6.0 dB more early energy on the left (R10; it summed to mono, -0.18); a held note smears by 1.47 cents (R9; it was 4.95, over the 3-cent target); no control zips over 1 dB on the defaults or any preset (R11; Size and Pre-delay zipped 5 to 8 dB); every preset within 0.9 LU of the dry (R6; the Endless plate was 6 LU loud).
- **Beyond the defaults** (fxlab's measures at Size 50 to 150 % and Decay 1.2, 2 and 3.5 s, 21 settings): dense in 10 ms at every one; the tail about 1.2 dB over the mean of four ideal tails on average (2.5 at worst); a held note under 2.8 cents; EDT/T30 0.87 to 1.17; a source's side 6 dB or more.
- **Why these lengths:** with the Room's lengths the tail rang about 1 dB more (2.1 over the mean of four ideal tails, against 1.2), and the wobble barely helped; longer lines, more modes, did. With lines that long the first ones come back only after 18 to 34 ms, so the onset is a fixed 29 ms of dense taps that bridges the gap at every Size (`tests/plate.rs` checks there is no dip), and Size spans 0.55 to 1.05 times the lengths rather than 0.5 to 1.5.
- **Blessed worse, and why:** the mono sum is 2.40 LU down (R8; 3.09 before), inside 3.5. C80 (information only) is 1.0 dB, against -5.1: the onset is now heard at once rather than building up.
- **Tests** (`mpfree-fx/tests/plate.rs`): the ids saved projects use, Decay at 44.1, 48 and 96 kHz from 0.5 to 6 s and at Size 50 and 150 %, Pre-delay (40 and 200 ms) moving the whole wet signal exactly, a source keeping its side, the onset running into the tail with no gap at Size 50, 100 and 150 %, the tail ringing out, every control swept under noise at an 18 s Decay and full Mod, and the same output, bit for bit, at any block size; in `rooms.rs`, no Plate line is shorter than a chunk at any rate.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, Mix 100 %): 155.8k instructions, against 186.7k for Dattorro's tank on the same case (17 % fewer). The old Plate cost 139k on ARMv7 under qemu (the fx.md table); the device round measures the new one.

## PR E3: the Spring

What changed (design: [fx.md](fx.md), the Spring entry): the Spring keeps its chirp chains (40 stretched all-passes a spring) and every parameter, but now each side has its own spring, hearing 0.35 of the other; Drip is an all-pass round the chain, so it never touches Decay; Tone filters the output instead of darkening every trip (a fixed one-pole at 5 kHz damps the loop); the stretch follows the rate; the delay is shortened by the chain's delay at 707 Hz, so an echo comes back every Length; and the loss is solved so the 500 Hz and 1 kHz octaves average Decay. After it: 102 of 114 reverb targets met (98 after PR E2); delay, EQ, the Room and the Plate unchanged.

- **Met now, every Spring row at its defaults and presets:** Decay -2.5, -4.1 and -4.7 % at 1, 2.5 and 6 s (R1; it was +4.0, -4.5 and -14.5 %, over the 10 % target at 6 s); a source on the left keeps 9.0 dB more early energy on the left (R10; 0.76, it summed to mono); every preset within 0.71 LU of the dry (R6; Dub splash was 1.29 loud); no control zips over 1.42 dB (R11; Dub splash zipped 3.20). EDT/T30 0.95 (R3; 0.85), and 4.68 dB over the ideal tail (R5, the Spring's own target 8 dB; 5.63).
- **Beyond the defaults** (fxlab's decay measure at 0.5 to 8 s, Length 20 to 80 ms, Tension and Drip 0 to 100 %, Tone 1 and 8 kHz, at 44.1, 48 and 96 kHz): Decay within -6.8 to +8.9 %, most within 5 %; the widest is Length 20 ms at full Tension, where the chain's delay is most of a trip.
- **Why 410 and 820 Hz:** each octave's T30 is set by its slow end (the chain delays lows more, the damping takes less there); solving at the octaves' centres left Decay 3 to 8 % long, more on short Lengths (up to 28 % at 20 ms and full Tension), and at a single 560 Hz point 8 % short. With Tone in the loop, a 1 kHz Tone made the loss swing too far (Decay up to 140 % long), which is why Tone moved to the output.
- **Blessed worse, and why:** the mono sum is 3.00 LU down (R8; 2.95 before), inside 3.5: two springs, one a side, still sum like two different tanks.
- **Tests** (`mpfree-fx/tests/spring.rs`): the ids saved projects use, Decay at 44.1, 48 and 96 kHz at 1, 2.5 and 6 s and with Length 20 and 80 ms, Tension and Drip at both ends and Tone at 1 and 8 kHz (within 12 % on an octave band-pass at 500 Hz and 1 kHz), a longer Decay ringing longer rather than louder (0.5 and 8 s within 2 dB on noise), a source keeping its side, the tail ringing out within its rest, every control swept under noise at an 8 s Decay and full Drip, and the same output, bit for bit, at any block size.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, Mix 100 %): 138.0k instructions, against 185.6k before (26 % fewer). The old Spring cost 206k on ARMv7 under qemu (the fx.md table); the device round measures the new one.

## PR E4: Hall, Cloud and Echoes

What changed (design: [fx.md](fx.md), the Cloud reverb entry): the Cloud engine (`cloud.rs`), whose characters are the Hall, Cloud and Echoes modes (and the Cloud reverb's own Plate and Room), keeps its structure, seeds and every parameter. Its loss is now a rate per frame spread over each line and each of its diffuser stages by delay, solved so the 500 Hz and 1 kHz octaves fall 60 dB in Decay; the lines' input follows the loudness each loop keeps, so Decay changes how long the tail rings and not how loud; Size and Pre-delay crossfade between lengths instead of sliding; the taps make the early sound after the diffuser, the Hall's lines take a share of them and Echoes' lines all of them; and each character's line modulation and trim are set anew. After it: 113 of 113 reverb targets met (102 of 114 after PR E3; Echoes' R3 is now reported only, below); delay, EQ, the Room, the Plate and the Spring unchanged.

- **Met now, every Hall, Cloud and Echoes row at its defaults and presets:**

  | Target | Hall before → now | Cloud before → now | Echoes before → now |
  |---|---|---|---|
  | Decay error at 1, 2.5 and 6 s (R1) | +8.2, +0.9, -1.7 → +4.7, -0.1, +0.4 % | +19.4, +21.7, +8.2 → -2.4, -2.1, -0.2 % | -1.5, -14.2, -26.8 → +3.9, +1.0, +1.1 % |
  | Over the ideal tail (R5) | 2.23 → 0.99 dB | 0.74 → 0.29 dB | 0.65 → 0.28 dB (reported) |
  | A held note's smear (R9) | 3.15 → 2.69 cents | 3.55 → 1.86 cents | 3.04 → 2.63 cents |
  | Worst control zip (R11) | 2.68 → 0.47 dB (Big hall 1.95 → 1.52) | 2.99 → 2.23 dB (Endless cloud 2.88 → 2.86) | 18.06 → 0.46 dB (Echo chamber 21.16 → 0.95) |
  | Wet level (R6) | +0.79 → -0.21 LU (Big hall +0.65 → +0.24) | -0.65 → +0.62 LU (Endless cloud +1.21 → -0.62) | -0.73 → -0.55 LU (Echo chamber -0.83 → +0.54) |
  | EDT/T30 (R3) | 1.20 → 1.10 | 1.12 → 1.39 | 2.47 → 2.13 (reported) |

- **Why the loss is spread by delay:** a loop's modes decay at its loss over the time a trip takes them, and an all-pass diffuser in the loop delays some pitches far more than others (its group delay swings with its feedback). With a fixed loss each pass, as in Cloud Seed, those pitches ring longest, and T30 follows the slowest: the Cloud character, with six stages in each line, ran 22 % long at 2.5 s, and Echoes' tail fell in steps. Lost per frame in each line and inside each stage's feedback, every path through the loop loses exactly in proportion to the time it takes, so the impulse response is the lossless one times one exponential. The rate is a quadratic in it, because each octave also loses the loop's shelves and damping once a pass: `a = (K - T (m1 + m2) + sqrt(T² (m1 - m2)² + K²)) / 2T`, with `K = 3 ln 10`, `T` Decay in frames and `m` each octave's EQ loss over the trip (`loss_rate`, `mid_loss`).
- **Why the level model:** scaling the lines' input by `1 / Σ 1 / (1 - pass)` holds the level at long decays but not at short ones, where a pass through a lossy all-pass keeps far more than its delay says (the part that goes straight through keeps its `g²` whatever the loss): wide open, with that scaling, the Cloud read 9 dB down at 0.3 s. Each stage's energy (`stage_energy`), the first pass out (after the diffuser and EQ, or straight from the delay for Echoes) and six octaves weighted like pink noise through the loudness meter make it flat from 0.3 to 8 s within 0.1 LU at the defaults.
- **Why the Hall's lines take the taps:** at a big Size each pass round a line is a good part of a short Decay (at Size 600, 300 to 900 ms against 1.5 s), and with the lines fed one compact burst the tail came in clusters with gaps between: 46 % short at Size 600 and 1.5 s on the middle octaves' T30, against 15 % before. A share of the taps spreads each line's input through the pattern's 450 ms. All of them (as before) tinted the tail with the pattern's colour (R5 over 2 dB); 0.2 of the taps to 0.9 of the diffused input leaves R5 at 0.99 and reads Decay within 1 % at Size 600 (3 % short at 1000 ms and 2 s, the widest).
- **The crossfades:** a Size or Pre-delay move slid every length, which bends the pitch as the read point moves (a held 1 kHz tone had nearly all its energy more than 60 Hz off while Size moved). Now each line, diffuser stage and tap reads both lengths for 25 ms (Pre-delay 20 ms) and crossfades; a move during a fade starts the next fade when it ends. Modulation glides over 300 ms, as its read points otherwise stepped (the Cloud's Modulation zipped 3.6 dB).
- **R3 for Echoes, reported only:** Echoes is a few lines heard before their diffusers, so its echoes come in distinct and build as they blur; the early decay is slow by design (EDT/T30 2.1 to 2.5), not a slap and a faint tail. Hall and Cloud keep the 1.6 ceiling.
- **Blessed worse, inside its target:** the Hall's tail IACC is 0.046 (R8, target 0.2; 0.022 before); the Cloud's EDT/T30 1.39 (1.12) and its mono sum 3.03 LU down (3.02); Echoes' Decay at 1 s +3.9 % (-1.5 %).
- **Not measured by fxlab:** the characters at Full quality (fxlab runs the default, Eco). At Full a held note smears 3.7 cents on the Hall and 4.1 on the Cloud, over the 3-cent target; Full is for when the top end matters, and its modulation is as before.
- **Tests** (`mpfree-fx/tests/cloud.rs`): Decay as the 500 Hz and 1 kHz octaves' T60 (a line fitted to each Schroeder curve from -5 to -60 dB, so a big Size's clustered passes read true) for every character at 44.1, 48 and 96 kHz and at Size 40 to 1000 ms, within 8 %; the octaves from 250 Hz to 2 kHz falling within 25 % of each other in every character but Echoes; a jump in Size (both ways) or Pre-delay leaving a held tone with under a sixteenth of its energy more than 60 Hz off; a longer Decay ringing longer, not louder (0.5 and 8 s within 1.5 dB from 250 Hz to 4 kHz); and a source keeping its side in every character. The existing Cloud tests pass as they were (the bit-exact block-size checks among them). Re-blessed: the Cloud reverb's golden cases and the mixes with a reverb, and the ratchet's Hall, Cloud and Echoes rows.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, Mix 100 %): 7 to 11 % more than before in every character and quality, the Cloud character 991.8k at Full (908.7k before) and 275.0k at Eco (254.4k). Each diffuser stage's loss is one multiply a frame (164 stages a frame in the Cloud character at Full), and the line gains, the level and the fades glide per frame. The mixer cases with a song's inserts, whose reverb return is a Hall, rise with it: 1,475k to 1,542k (+4.5 %), with the Light reverb 1,087k to 1,123k (+3.4 %), with the Eco reverb 999k to 1,018k (+1.8 %), and with six tracks frozen 1,301k to 1,367k (+5.1 %), each by what the Hall's character costs more. The ten Cloud rows and these four are updated by hand.

## Running it

```sh
cd rust
cargo build --release --locked -p mpfree-fxlab
./target/release/fxlab quality --out /tmp/quality                 # every class
./target/release/fxlab quality --class reverb --out /tmp/quality  # one
cargo test --release --locked -p mpfree-fxlab --test quality      # the ratchet
```

## Round 2: drive, crush, compressor, filters and lo-fi

The targets of plan-2 (ids G, B, K, F and L, and A for the amp), measured by `quality/round2.rs`. The measures it adds, beside the first round's:

| Measure | What it reads |
|---|---|
| Aliasing grid | The worst alias of 2.5, 5 and 8 kHz sines at 0 dBFS and of 5 kHz at -12 dBFS (the `harmonics` analysis), against the fundamental; in dBFS where the fundamental goes (Rectify) |
| Switch click | A choice changed halfway through a second of a sine (a mode, a quality, a filter type or slope): the largest second difference around the change over the steady signal's on either side, dB, at least 0 |
| Slope zipper | A sweep's largest second difference against a clean sine's at the local level (its RMS over two periods), so a filter whose ends attenuate the tone isn't judged against them. fxlab's usual zipper (against the resting ends) reads the DJ filter at 45 dB; this reads 2.3 |
| Off grid | For crush: a sine through the sample and hold, its spectrum through a 4-term Blackman-Harris window (a rate that doesn't divide the session's puts lines between bins, and a plain spectrum's leakage would read as stray energy); everything not at k R ± f below the session's Nyquist, against the tone |
| Heard against drawn | The gain of a sine at chosen frequencies (a lock-in over whole periods) against the engine's own `response_db` or `cuts_db` |
| IMD | SMPTE (60 Hz at -8 dBFS with 7 kHz at -20 dBFS: the sidebands at ±60, ±120, ±180 Hz against 7 kHz) and the CCIF difference tone (19 and 20 kHz at -12 dBFS each) |
| Null | The largest difference between the output and the input as the node saw it (32-bit), late by the reported latency, dBFS; -140 when they are the same |
| Power chord mud | For the amp: E2, B2 and E3 together (a metal rhythm part's power chord) through it, Blackman-Harris windowed; the share of the output from 20 to 70 Hz against all of it from 20 Hz up, dB. Clipping a chord makes its 41 Hz difference tone, the mud a tight boost takes away |

### Drive (G)

Subjects: the Drive family's ten modes (`drive/soft` to `drive/soften`) at their defaults, in both qualities where the engine has them, and its presets.

| Id | Row | Target | Why |
|---|---|---|---|
| G1 | `alias_5k_0dbfs_eco`, `_hq`; Character's `alias_5k_0dbfs` | Smooth curves (Soft, Tube, Console, Tape): ≤ -65 dB in Eco and Character, ≤ -90 dB in HQ; 2.5 kHz, 8 kHz and -12 dBFS held | Inharmonic fizz; 2x oversampling with first-order ADAA reaches -69.5 dB on tanh |
| G2 | the same for Hard, Fuzz, Foldback; Rectify in dBFS | ≤ -50 dB in Eco, ≤ -60 dB in HQ | A hard edge has harmonics to every order; 4x with ADAA reaches -65 dB |
| G3 | presets' `alias_5k_0dbfs` | Held | Presets drive hard on purpose |
| G4 | `lu_pink18_drive0` to `36`, `lu_pink18_amount0` to `100` | Within ±1 LU of the dry on pink at -18 dBFS RMS (Rectify and Foldback ±3) | More drive is more grit, not louder |
| G5 | `lu_pink6_*`, `lu_hits_*` | Reported | Rounding loud peaks loses loudness by design |
| G6 | `bias_change_db` | The output changes by at least -20 dB of itself between Bias 0 and 100 (1 kHz, -12 dBFS) in every mode but Soft, where the knob is dimmed (Jay, 17 Nov) | A control that does nothing is a bug |
| G7 | `drive/switch/click_*` | ≤ 3 dB, Mode and Quality changes | No clicks when trying modes |
| G8 | `flat_m40dbfs*`, `flat_mix50` | Within ±0.5 dB from 20 Hz to 20 kHz at -40 dBFS and at Mix 50 % (Tape's colour and Rectify reported) | Clean when quiet |
| G9 | `subsonic_hits_bias100`, `dc_hits_bias100_dbfs` | Under 15 Hz ≤ -20 dB of the total on hits (Tube and Fuzz, Bias 100, Drive 24); DC ≤ -100 dBFS | Inaudible energy eats headroom |
| G10 | `h2_minus_h3`, `high_order_share`, `thd_1k_*dbfs`, `imd_smpte`, `imd_ccif` | Reported | A mode's profile becomes a target once it is designed to one |
| G11 | `zipper_db`; Rectify's `zipper_bias_db` | ≤ 3 dB, every mode and preset; Rectify's Bias sweep reported (moving its fold point is a change of sound) | Smooth automation |

### Crush (B)

| Id | Row | Target | Why |
|---|---|---|---|
| B1 | `sinad_error_bits4` to `16` | Noise and distortion of a full-scale sine within 1 dB of 6.02 N + 1.76 dB | Bits mean bits |
| B2 | `dither_noise_rise_bits8`, `thd_m40dbfs_bits8_dither` | TPDF raises the noise 4 to 6 dB; a -40 dBFS sine at 8 bits ≤ -30 dB THD with dither | Dither turns distortion into steady noise |
| B3 | `off_grid_<R>hz`, `_at48k`; presets' `off_grid_at_16bits` | ≤ -45 dB at 8, 10, 11.025, 22.05, 26.04 and 32 kHz, in this session and at 48 kHz | Stray tones from the session's frame grid and folded images are in no sampler being imitated; a prototype with exact sampling times and band-limited steps reaches about -50 dB |
| B4 | `image_error_quarter_rate` | Within 1 dB of the zero-order hold's sin(x)/x | The images are the effect |
| B6 | `m50dbfs_bits8_out_dbfs`, `m50dbfs_bits8_buzz_out_dbfs` | Cut reported (mid-tread gates under half a step); with Tails on Buzz (a mid-riser) a -50 dBFS sine at 8 bits comes out at -60 dBFS or more | Tails cut off sound broken, so there is a choice that keeps them |
| B7 | `filters_drawn_vs_heard_db` | The Filters card's curve within 0.5 dB of a sine through them (Both against Off, 11,025 Hz, 200 to 4800 Hz) | What is drawn is what plays |
| B8 | `step_spike_db` | A Bits or Rate change's step ≤ 3 dB over the crushed signal's own (first differences) | Stepping is the sound, a click isn't; fxlab's usual zipper is reported, not judged |
| B9 | `lu_pink18` | Within ±1 LU, defaults and presets | Fair A/B |
| B10 | `silence_out_dbfs` | ≤ -120 dBFS | Clean rests |

### Compressor (K)

| Id | Row | Target | Why |
|---|---|---|---|
| K1 | `curve_error_peak`, `_knee0`, `_rms` | A steady 1 kHz sine from -39 to 0 dBFS within 0.25 dB of the drawn curve | The curve on screen is a promise |
| K2 | `attack_ratio_<ms>` | The gain's 63 % time on a -40 to 0 dBFS step within ±15 % of Attack, 1 to 100 ms (0.1 ms reported) | Settings learned elsewhere must work |
| K3 | `release_ratio_<ms>` | Within ±10 % of Release, 50 ms to 2 s | The same |
| K4 | `thd_<hz>hz_gr10_*` | About 10 dB of reduction at the default times, peak: 50 Hz ≤ -50 dB, 100 Hz ≤ -56 dB; RMS and faster times held | Gain riding inside the wave distorts bass |
| K5 | `meter_error` | ≤ 0.1 dB | Honest meter |
| K6 | `link_error` | ≤ 0.1 dB | Steady image |
| K7 | `lu_pink18_auto`; the presets' `lu_pink18` | With Auto makeup on (Makeup 0), the defaults and every preset within ±1.5 LU of the dry on pink at -18 dBFS; each preset with its own makeup the same (Sidechain pump, keyed, and the defaults, which have none, reported); `lu_pink6*` reported | Fair A/B; louder wins |
| K8 | `null_ratio1_dbfs`, `null_below_knee_dbfs`, `null_mix0_dbfs` | Bit for bit | Inserting it costs nothing |
| K9 | `overshoot_db_attack*` | Reported | No look-ahead (the Limiter has it) |
| K10 | `zipper_db` | ≤ 3 dB, defaults and presets | Smooth automation |
| K12 | `release_slowdown`, `release90_after_50ms_ms`, `release90_after_3s_ms` | The time to come 90 % back after 3 s of 0 dBFS against after 50 ms: Clean within ±10 % of 1; Opto, Bus and FET at least 2 | A program-dependent release is quick after a hit and slow after long reduction; a plain one is the same either way |
| K13 | `curve_error_range6` | Range 6: within 0.25 dB of the drawn curve | Range caps where it says |
| K14 | `unlinked_quiet_side_db`, `half_link_error_db` | A -30 dBFS right side under a 0 dBFS left: at Link 0 within 0.1 dB of untouched; at Link 50 within 0.25 dB of half the left's reduction | Link means what it says |
| K15 | `compressor/<style>/*` | Opto, Bus and FET each meet K1, K2 and K3 (the release read after a burst, before the slow stage charges; ±15 %), K4 at 50 and 100 Hz (FET's colour reported) and K12, and a Style change mid-note clicks ≤ 3 dB; FET's 5 kHz alias at 0 dBFS ≤ -60 dB (8 kHz and its THD at 10 dB of reduction reported) | Each style is as honest as Clean |

The attack and release times are read from the gain at each millisecond's exact frame (the tone demodulated through a 1 ms Hann window, the crossing found between milliseconds by halving). The first version read 2 ms windows every 44 frames and counted each as a millisecond, so at 44.1 kHz its clock ran 0.2 % fast, its level steps landed a few milliseconds off where it looked for them, and its times came in whole steps: on main's engine it read the attacks as 1.6 to 1.86 times their labels (now 1.48 to 1.58) and a 1 ms attack as exactly 1.

### Filters (F)

| Id | Row | Target | Why |
|---|---|---|---|
| F1 | `cutoff_error_pct_12db`, `_24db`; `heard_vs_drawn_*` | The drawn -3 dB point within 1 % of Cutoff from 100 Hz to 16 kHz; heard within 0.1 dB of drawn | The knob means the frequency |
| F2 | `passband_res<n>_*` | 100 Hz under a 1 kHz low-pass within ±0.5 dB at any Resonance | No hidden level change |
| F3 | `swept_pink_peak_res100_*` | Pink at -18 dBFS RMS through a low-pass swept 100 Hz to 10 kHz at Resonance 100: peak ≤ -1 dBFS (75 held) | A mix-level source shouldn't clip from Resonance alone |
| F4 | `peak_gain_m40dbfs_res100_*` | ≥ +28 dB at the cutoff for a -40 dBFS sine | Bounding loud signals mustn't flatten quiet ones |
| F5 | `click_lp_to_hp_24db`, `click_12_to_24db` | ≤ 3 dB | No clicks when switching |
| F6 | `sweep50ms_<type><slope>_res0` | ≤ 3 dB (slope zipper, end to end in 50 ms); Resonance 50 reported, as it rings at its own moving pitch | Smooth sweeps |
| F7 | `drive12_alias_5k_0dbfs` | ≤ -65 dB | As G1 |
| F8 | DJ: `null_middle_dbfs`, `position_sweep50ms_res0`, the corners, `swept_pink_peak_res100` | Bit for bit off in the middle; ≤ 3 dB; low-pass to 100 Hz or lower, high-pass from 5 kHz or higher; ≤ -1 dBFS | DJ mixers' colour filters |
| F9 | Ladder: `selfosc_cents_<hz>`, `selfosc_level_spread_db`, and its `swept_pink_peak_res100`, `heard_vs_drawn_res50`, `click_multi_to_ladder` | Pitch within 5 cents of the cutoff from 100 Hz to 5 kHz, level within 1 dB; ≤ -1 dBFS; ≤ 0.2 dB; ≤ 3 dB | The classic test of an analog-style filter, and the clean filter's tests where they apply |
| F10 | `zipper_db` | Reported (fxlab's usual zipper reads high where a filter's ends attenuate the tone) | |
| F12 | Res mode: on Hard, `swept_pink_peak_res100_*`, `peak_gain_m40dbfs_res100_*`, `heard_vs_drawn_lp_res50_*`; `click_soft_to_hard` and `click_soft_to_clean` on both models; the Hard ladder's `selfosc_cents_1000` and `swept_pink_peak_res100`; the Clean ladder's `selfosc_level_dbfs_1k` | ≤ -1 dBFS; ≥ +28 dB; ≤ 0.1 dB (drawn at twice the rate); ≤ 3 dB; within 5 cents; ≤ -1 dBFS; ≤ -100 dBFS | Each mode does what it says: Hard bounds the ring and leaves quiet sounds alone, Clean is the linear filter, and its ladder never sings |

### Lo-fi (L)

| Id | Row | Target | Why |
|---|---|---|---|
| L1 | `wf_peak_pct`, `wow_peak_pct_top`, `wf_peak_pct_amount0` | The defaults' wow and flutter within ±20 % of 0.116 % peak; Amount and Wow at 100 at least 2 %; Amount 0 under 0.001 % | From a clean deck to a broken machine |
| L2 | `flutter_peak_pct_top` | At least 0.5 % | A worn transport |
| L3 | `mean_pitch_error_pct_*` | ≤ 0.01 % | The track stays in tune |
| L4 | `wow_line_share` | The pitch track's strongest line holds at most half its power | A plain sine reads as vibrato |
| L5 | `sat40_alias_5k_0dbfs` | ≤ -65 dB (Saturation 100 held) | As G1 |
| L6, L7 | `sat*_h3_minus_h2`, `noise_*`, `follow0_noise_dbfs`; `follow0_after_vs_alone_db` | Reported; with Follow at 0 the noise within 1 dB from playing to silence | Tape's distortion is mostly third harmonic; noise levels; a noise bed keeps going |
| L8 | `cuts_heard_vs_drawn_*` | ≤ 0.5 dB | The screen tells the truth |
| L9 | `lu_pink18` (read over 9 s: dropouts are random) | Within ±1 LU, defaults and presets (Old radio reported) | Fair A/B |
| L10 | `null_amount0_dbfs` | Bit for bit | Inserting it changes nothing |
| L11 | `zipper_db` | ≤ 3 dB | Smooth automation |
| L12 | `dropout_depth_error_db`, `dropout_fall_ms`, `dropouts_per_s` | Within 1 dB of the set depth; a fall of at least 5 ms (10 to 90 %); the rate reported | Worn tape, without clicks |
| L13 | `latency_frames` | Reported (the instruction gate holds the cost) | MPC budget |
| L14, L15 | `stereo100_pitch_apart_pct`, `hum100_dbfs` | Reported | |

The pitch rows (L1 to L4) listen for 60 s: a random wander needs that long to show its size and to be told from a line. The presets' L1 and L3 rows are read with Dropouts, Hum and Stereo off, since a dip or the sides drifting apart beat in the measure's mid.

### Amp (A)

The Amp at each model (`amp/clean`, `amp/crunch`, `amp/lead`, `amp/bass`) and its presets (`amp/amp[Modern lead]`). The targets reach rich, high-gain distortion as one use among many; presets and defaults stay broad across styles.

| Id | Row | Target | Why |
|---|---|---|---|
| A1 | `alias_1k_m6dbfs_gain10`, `alias_2k_m6dbfs_gain10`, `alias_5k_m12dbfs_gain10` | ≤ -60, -60 and -50 dB at Gain 10, every model (Gain 5 and the presets' 5 kHz kept by the ratchet) | Inharmonic fizz on high notes; about -60 dB is where it stops mattering |
| A2 | `cleanup_db_gain5` | A 1 kHz sine's THD falls at least 10 dB from -6 to -30 dBFS at Gain 5 (Clean, Crunch, Bass; Lead reported) | Playing softer cleans up |
| A3 | `lu_pink18_gain*` | Pink at -18 dBFS within ±2 LU of the dry from Gain 2.5 to 10 (Gain 0 reported) | Gain changes grit, not loudness |
| A4 | `lu_pink18` (presets) | Within ±1 LU | Fair A/B |
| A5 | `power_chord_sub70_db_gain10_nocab`, `..._tight` | A power chord's energy from 20 to 70 Hz against all of it, at Gain 10 with the cab off: reported plain; with the Tight boost ≤ -25 dB on Lead (the other models' kept) | Palm mutes and chords stay clear, when asked |
| A6 | `bass_jump_db`, `mid_jump_db`, `treble_jump_db` | A knob thrown from 0 to 10 under a 220 Hz sine spikes ≤ 3 dB over the steady sound (switch click) | Smooth knobs |
| A7 | `gain_zipper_db` | fxlab's zipper on Gain ≤ 3 dB | Smooth knob |
| A8 | `model_change_db`, `cab_change_db`, `boost_change_db` | ≤ 3 dB (switch click) | No clicks |
| A9 | `silence_out_dbfs_gain10`; on Lead with Gate 8 (-45 dBFS): `gate_hiss_out_dbfs_gain10`, `gate_open_ms`, `gate_closes_on_decay` | Silence ≤ -120 dBFS. A note then hiss at -60 dBFS RMS: the last half second of the rest ≤ -90 dBFS at Gain 10. A note after silence is within 1 dB of the ungated amp's in ≤ 2 ms, and a note decaying 40 dB a second shuts the gate once (gated over ungated, 1 ms windows, under a half after over 0.9); both at Gain 0, where the amp is nearly straight, so a half-open gate can't hide in the clipping | Clean rests, notes not chopped |

The Fuzz (decision 10.6) is measured in the same class, at each model (`fuzz/face`, `fuzz/muff`, `fuzz/octave`) and its presets (`fuzz/fuzz[Sputter]`):

| Id | Row | Target | Why |
|---|---|---|---|
| A3 | `lu_pink18_fuzz*` | Pink at -18 dBFS within ±2 LU of the dry at Fuzz 0, 25, 50, 75 and 100 | Fuzz changes grit, not loudness |
| A4 | `lu_pink18` (presets) | Within ±1 LU | Fair A/B |
| A8 | `model_change_db`, `low_keep_change_db` | ≤ 3 dB (switch click) | No clicks |
| A9 | `silence_out_dbfs_fuzz100_gate0`, `..._gate100` | ≤ -120 dBFS | Silence stays silent, starved or not |
| A10 | `sustain_drop_db_fuzz100`; `breakup_dbfs_fuzz100_gate70` (`breakup_dbfs_fuzz100` reported) | At full fuzz a 220 Hz sine 20 dB quieter (-12 to -32 dBFS) comes out ≤ 6 dB quieter (RMS, as the octave's is an octave up). With Gate 70, a note decaying 40 dB a second from -12 dBFS breaks up (its output, in 10 ms windows, 20 dB under its level 100 ms in) while the note is still above -40 dBFS | The two fuzz signatures: sustain, and the starved sputter |
| A11 | `alias_5k_m12dbfs_fuzz100` (the 1 kHz row, Fuzz 60 and the presets kept by the ratchet) | ≤ -50 dB, every model | As G1 |
| A13 | `low_keep_50hz_gain_db` | A 50 Hz sine at -12 dBFS with Low keep at 150 Hz and full fuzz comes out within ±1 dB of how it went in | The sub stays the sub |

A12 is the instruction gate's Amp cases and its Fuzz case.

### Baseline (2026-11-17, before any fix)

76 of 109 drive targets, 19 of 32 crush, 16 of 24 compressor, 25 of 30 filter and 17 of 26 lo-fi targets met. The gaps:

- **Drive:** Eco aliases (-53 dB Soft, -38 Hard, -39 Fuzz at 5 kHz) and Character has no oversampling (Console -41, Tape -34); HQ's hard edges miss too (Hard -50, Fuzz -55). Auto gain keeps a -12 dBFS sine level, so mix-level pink gets louder with Drive (+3.4 LU at 36 dB, Foldback +5.7 at 24). Bias does nothing in Hard, Foldback and Rectify. A mode change clicks (Soft to Foldback 31 dB, to Fuzz 17 dB) and so does a Quality change (68 dB). Tube and Fuzz at Bias 100 put 13 to 14 dB of their hits' energy under 15 Hz.
- **Crush:** at rates that don't divide the session's, 11 to 16 dB under the tone lies off the sample and hold's grid (8 kHz: -13.3 dB; the 12-bit classic preset's 26.04 kHz: -13.4); rates that divide it are clean (under -89 dB), so the same knob sounds different at 44.1 and 48 kHz.
- **Compressor:** steady signals get up to 0.95 dB less reduction than drawn and the attack takes 1.6 to 1.9 times its label (peak detection; RMS detection meets the curve within 0.02 dB); 50 Hz distorts at -44 dB and 100 Hz at -50 at the defaults; Ratio 1 is not quite bit for bit (-132 dBFS).
- **Filters:** a mix-level source through a low-pass swept at full Resonance reaches +2.8 dBFS (12 dB) and +1.0 (24 dB); a type change clicks (64 dB) and a slope change (29 dB); the drive aliases at -20 dB.
- **Lo-fi:** the wow tops out at 0.39 % and the flutter at 0.13 %; one line holds 81 % of the wow; saturation aliases at -28 dB; the defaults and tape and vinyl presets are 1.2 to 1.6 LU quieter than the dry; at Amount 0 the cuts stay at 20 Hz and 20 kHz (-3 dB there, flat within 0.3 dB from 40 Hz to 16 kHz), so the output isn't the input.

### Drive PR

What changed (design: [fx.md](fx.md), the Drive and Character entries): Drive's curves are averaged by first-order antiderivative antialiasing (ADAA) inside its oversampling (Eco 2x, HQ 4x), and Character runs at twice the rate with ADAA on its curves; auto gain holds Gaussian noise at -18 dBFS RMS at its level; a change of Mode or Quality crossfades; Bias acts in Hard, Foldback and Rectify (decision 10.1) and is dimmed in Soft; the DC blocker is a fourth-order 17 Hz high-pass on the output, dry and wet together. After it: 106 of 109 drive targets met (76 before); the other classes are unchanged.

- **Met now:** a 5 kHz sine at 0 dBFS aliases at -68.6 dB (Soft), -55.1 (Hard), -65.2 (Tube) and -59.6 (Foldback) in Eco, and -103.7, -68.9, -104.0 and -79.3 in HQ (G1, G2; Eco was -52.6, -38.1, -53.8 and -53.4, Hard's HQ -50.1); Fuzz's HQ -74.7 (was -54.7) and Rectify's -75.3 dBFS (was -48.0); Character's Console -98.9 and Tape -94.7 (were -41.2 and -33.6). Pink at -18 dBFS stays within 1 LU of the dry from Drive 0 to 36 in Soft, Hard, Tube and Fuzz (G4; up to +3.5 before), and within 3 in Foldback and Rectify (+5.7 and +6.6 before). Bias changes the sound in every mode but Soft (G6). A Mode change clicks at most 0.9 dB over the sound and a Quality change not at all (G7; 31 and 68 dB before). Quiet signals are flat within 0.2 dB (G8; 0.35). The energy under 15 Hz on hits at Bias 100 is -23.5 dB (Tube) and -22.6 dB (Fuzz) of the hit (G9; -13.8 and -12.8).
- **Still unmet:** Fuzz's and Rectify's Eco alias at 5 kHz, -49.99 dB and -49.3 dBFS against -50 (G2; were -39.2 and -32.8): at 2x a hard edge's harmonics fold before first-order ADAA can take them out, and more half-band sections change nothing; HQ meets both. Tape at Amount 100 plays pink 1.77 LU under the dry (G4; 1.86 before), its own head bump and top-end loss; left as designed.
- **Measured differently:** fxlab's response measure now starts each window, the device's and the wire's, the same time (up to 4096 frames) before its own peak. A 512-frame start cut the sweep's band-edge ringing at a different point for a device with latency, and read a plain delay as a few tenths of a dB of ripple under 200 Hz. It moved the EQ's drawn-against-heard row from 0.027 to 0.004 dB and no other row of the first round. Foldback's G4 tolerance is ±3 LU, as Rectify's: its folds spread power past 20 kHz at high drive, so no fixed gain holds its loudness the way it holds a clipper's (the exact Gaussian gain reads the same). Rectify's Bias sweep is reported apart, at 6.6 dB with no target: moving the fold point is a change of sound, and a sweep 20 times slower reads the same.
- **Blessed worse, and why:** Soft's and Tube's 2.5 kHz Eco aliases (-67.3 and -68.1 dB, were -69.3 and -70.2), Foldback's (-52.9, was -54.6) and Foldback's 8 kHz HQ alias (-62.3, was -64.8). The worst line in each is still the harmonic just above 22.05 kHz that the down-sampler's transition band lets through (the 9th of 2.5 kHz, at 22.5 kHz; the 3rd of 8 kHz, at 24 kHz). Under ADAA a curve's harmonics come out as the continuous curve's average over a step, which droops less near the top than the two-point mean the flat filter undoes, so that one line is about 2 dB louder; every other alias line fell (Soft's next, the 11th harmonic, from -91 to -100 dB). Hard's -12 dBFS aliases, -67.6 dB in Eco and -91.7 in HQ (were -140, no clipping at all): Bias now acts in Hard, and at the default Bias 20 a -12 dBFS sine at Drive 12 clips on one side; at Bias 0 it reads -110. The Drive sweep's zipper on the Foldback grit and Fuzz box presets, 2.8 and 1.1 dB (were 2.3 and 0.4), inside the 3 dB target: the sweep's sharpest corner fell (0.237 to 0.189, and 0.050 to 0.041), but the steady sound's corners, the reference, fell further, because they no longer carry aliases. Inside their tolerance, and within the 3 dB target too: Hard's zipper 0.34 dB (was 0.05), Hard clip's 0.43 (0), Tube glow's 0.57 (0.16), Fuzz's 2.65 (2.32).
- **Cost** (the instruction gate's cases, rustc 1.97.0, a 128-frame block at 44.1 kHz): Drive's Eco 154.6k to 202.6k instructions (1.31 times), HQ 272.3k to 430.3k (1.58 times), the drive, filter, delay and compressor chain 255.1k to 327.1k. Character costs 4.2 to 5.2 times its old base-rate cost: Console 20.1k to 104.2k, Soften 23.2k to 97.1k, Iron 27.0k to 120.1k, Tape 32.9k to 149.8k. Three trims kept the sound to the bit or within 2e-15: unrolled half-bands (`oversample::IirPair`), their states flushed every 32 frames (16 in Character) instead of every section and sample, and MPFree's own `exp` and `ln` for the antiderivatives (the C library's were a quarter of Drive's cost). Three half-band sections each way in Character would save another 15k but let Tape alias at -61 dB, past its target. Iron and Soften aliased little at the base rate (-140 and -72 dB at 5 kHz), so running those two modes there would bring them near their old cost, at the price of more code in the mode crossfade: for the device round to decide.

### Crush PR

What changed (design: [fx.md](fx.md), the Crush entry; decision 10.2): each hold falls at its exact time, reads the input then with a cubic, and steps there by a minimum-phase band-limited step (no latency); new **Tails** (Cut, Buzz), **Filters** (Off, Before, After, Both: fourth-order Butterworth low-passes at 0.45 times the rate, on in the vintage presets), **Jitter** and **Output**; three presets (12-bit 40k, 8-bit sampler, Wobbly clock). After it: 40 of 40 crush targets met (19 of 32 before; B6's Buzz row, B7 and the new presets' rows are new); the other classes are unchanged.

- **Met now:** what lies off the hold's grid, against the tone, at 8, 10, 11.025, 22.05, 26.04 and 32 kHz: -74.3, -54.6, -91.4, -92.3, -64.0 and -68.3 dB in a 44.1 kHz session, and -90.0, -72.8, -60.7, -64.7, -69.6 and -67.6 at 48 kHz (B3; -11 to -16 dB before at the rates that don't divide the session's). The presets at 16 bits: -59.0 (Wobbly clock, measured with Jitter at 0) to -93.2 dB. The images at a quarter of the rate follow sin(x)/x within 0.0015 dB (B4). Buzz keeps a -50 dBFS sine at 8 bits at -48.2 dBFS (B6). The Filters' drawing is within 0.0012 dB of what plays (B7). Every preset and the default within 0.74 LU of the dry (B9).
- **Measured differently:** the presets' off-grid row runs with Jitter at 0: jitter moves the holds off any grid on purpose (Wobbly clock reads -6.8 dB with it).
- **Blessed worse, and why:** Soft grit's loudness, -0.38 LU (was -0.05), inside ±1 LU: its 32 kHz rate doesn't divide a 44.1 kHz session, and the images that folded back under 22 kHz and the stray tones of the uneven holds no longer add to its level.
- **Cost** (the instruction gate's case, rustc 1.97.0, a 128-frame block at 44.1 kHz, the default 8 bits at 11,025 Hz): 10.3k to 36.2k instructions (3.5 times), about 1 % of a period on the MPC. Each hold that changes the value adds 32 corrections for each side (one at the same value adds none): the step is read from a table of 17 phases (one per sixteenth of a frame, interpolated between two rows) in two straight runs round the ring, which halved a first version's 71.3k. A rate at or above the session's holds every frame and adds none. Perform FX's own crush is a separate engine and is unchanged.

### Amp baseline (2026-11-17, before any fix)

38 of 64 amp targets met (rows added by the amp's measuring PR; nothing in the amp changed). The gaps:

- **Aliasing at high gain:** Lead at Gain 10 aliases at -38, -38 and -32 dB (1 and 2 kHz at -6 dBFS, 5 kHz at -12) and Crunch's 2 and 5 kHz at -48 and -36; Clean and Bass meet A1. The Modern lead preset's 5 kHz alias is -35 dB.
- **Level:** Lead plays pink 12.5 LU over the dry at Gain 2.5 and 8.5 at Gain 10; Crunch reaches +2.05 at Gain 10. The presets are -1.5 to +9 LU (Classic crunch +2.2, Modern lead +9.0, Lo-fi radio +4.1, Bass grit -1.5; Sparkle clean +0.5 meets).
- **Knobs and switches:** the tone stack's knobs jump: Treble 0 to 10 spikes 38.8 dB on Clean, 22.0 on Bass and 6.1 on Crunch; Mid 28.5 (Clean) and 13.5 (Bass); Bass 16.3 (Clean). Gain's zipper is 5.1 to 8.2 dB on Crunch, Lead and Bass (Clean 0.9). Taking the cab out clicks 38.0 dB on Clean, 21.8 on Bass and 12.8 on Crunch (the research's first-difference probe missed it); Lead's distortion hides the same steps. Model changes are clean.
- **Reported:** Lead cleans up 1.9 dB from -6 to -30 dBFS (the others 10.3 to 26.2 dB); the power chord's mud at Gain 10 with the cab off is -13.6 dB on Lead and -22 to -26 on the others.
- **Cost** (the instruction gate's new cases): Amp (Crunch) 206.6k instructions a block, Amp (Lead, Gain 10) 232.8k.


### Amp PR

What changed (design: [fx.md](fx.md), the Amp entry): the first clipping stage's curve is averaged by first-order ADAA inside the 4x path, and the tone stack and the power stage moved inside it; the output trims are fitted on pink noise at -18 dBFS at every step of Gain; Gain, the tone knobs and Level glide (the stack's values every 4 frames on a frame grid); Model, Boost and Cab changes crossfade over 10 ms; new **Boost** (Off, Tight, Treble), **Gate** and **Release**, **Sag** and **Depth** (ids 8 to 12; 0 to 7 unchanged); four presets (Edge of breakup, Tight rhythm, Treble lead, Synth grit) and the old ones' Level set so each plays as loud as the dry. After it: all 76 amp targets met (38 of 64 before; the Tight boost's A5 rows, the Boost change's A8 rows, the gate's three A9 rows and the new presets' rows are new); the other classes are unchanged.

- **Met now:** at Gain 10 Lead aliases at -64.1, -64.3 and -57.6 dB (1 and 2 kHz at -6 dBFS, 5 kHz at -12; were -38, -38 and -32) and Crunch at -83.0, -73.8 and -61.5 (were -68, -48 and -36) (A1). Pink at -18 dBFS within 0.4 LU of the dry from Gain 0 to 10 on every model (A3; Lead was up to +12.5; the trims meet it exactly at whole steps of Gain, and Lead at 2.5 reads 0.38 between them) and every preset within 0.1 LU (A4; Modern lead was +9.0). The Tight boost takes a power chord's mud at Lead 10 from -13.6 to -27.9 dB (A5). A tone knob thrown from 0 to 10 spikes at most 0.05 dB over the sound (A6; Clean's Treble was 38.8), Gain's zipper is at most 0.08 dB (A7; up to 8.2), a Cab change reads 0 (A8; Clean's was 38.0) and a Boost change at most 0.8. With Gate 8 the rest after a note is digital silence even at Lead 10 with -60 dBFS hiss in, a note is through within 1 ms and a decaying note shuts the gate once (A9).
- **Reported:** on the other models the Tight boost adds mud instead (Clean -25.6 to -17.7 dB, Crunch -22.2 to -18.7, Bass -23.4 to -18.6): its own clipping makes the chord's difference tones, which only Lead's 300 Hz tightening high-pass takes out, as a real overdrive into a clean amp does. Cutting the boost's lows harder (two poles at 150 Hz) made Lead worse (-15.4 dB) and its switch click.
- **Blessed worse, and why:** a Model change reads 0.16 to 0.45 dB (was 0; target 3): the old model and the new now play together for 10 ms instead of the sound jumping, and the jump read 0 because the level after it was higher. Bass's clean-up, 16.45 dB (was 16.48), inside its tolerance.
- **Cost** (the instruction gate's cases, rustc 1.97.0, a 128-frame block at 44.1 kHz): Amp (Crunch) 206.6k to 319.6k instructions (1.55 times), Amp (Lead, Gain 10) 232.8k to 341.6k (1.47 times). ADAA on every stage and the power stage cost 424k and 506k and changed the worst alias by under 0.5 dB: the first stage takes the most gain and squares the wave off, while the later ones clip what its Miller pole has already smoothed, and the power stage clips gently at 4x. The gate, the boost and Depth cost nothing while off; Sag's follower runs once a frame.

### Fuzz PR

What changed (design: [fx.md](fx.md), the Fuzz entry): a new effect, **Fuzz** (`mpfree.fx.fuzz`, decision 10.6), with Face, Muff and Octave models and Fuzz, Tone, Gate, Low keep and Level, built on the Drive PR's ADAA inside the Amp's 4x half-bands, and its rows in the amp class (60 new ratchet rows; the other rows are main's). All 45 of its targets are met.

- **Met:** at full fuzz a 5 kHz sine at -12 dBFS aliases at -65.9 dB (Face), -72.5 (Muff) and -54.0 (Octave) (A11). A 20 dB drop in at full fuzz drops the output 1.0, 0.1 and 2.5 dB (A10); with Gate 70 a decaying note breaks up at -31.2, -31.2 and -27.2 dBFS (without it at -57.6, -70 and -55.2). Pink at -18 dBFS within 0.1 LU of the dry at every Fuzz (A3) and every preset within 0.05 LU (A4). A Model change reads at most 0.5 dB and Low keep turned on at most 1.6 (A8). With Low keep at 150 Hz a 50 Hz sine comes out within 0.13 dB (A13).
- **Chosen while building:** the Octave's top gain is 44 dB, not the 36 first drafted: at 36 its output fell 9 dB for a 20 dB drop in (the rectified octave is about 7.5 dB under the note), and at 48 its 5 kHz alias was -50.5 dB, at the target's edge. Low keep lets the fuzz hear everything and high-passes what it makes at the crossover, rather than fuzzing only what is above it: a bare 808 has almost nothing above 150 Hz, and fuzzing only that left it clean.
- **Cost** (the instruction gate's `Fuzz` case, the default Face at Fuzz 60, rustc 1.97.0, a 128-frame block at 44.1 kHz): 386.5k instructions. It runs in stereo (the Amp is mono), each side at 4x with ADAA on its first curve; Drive HQ is 430.3k. The device round decides whether it needs a cheaper Eco path (2x, as Drive's Eco).

### Compressor PR

What changed (design: [fx-compressor.md](fx-compressor.md); decision 10.3): the detector is Giannoulis, Massberg and Reiss's smooth decoupled peak detector on the reduction in dB, its release solved through the attack (`release_tau`); new **Style** (Clean, Opto, Bus, FET), **Auto makeup** (off by default), **Range** and **Link** (ids 11 to 14; 0 to 10 unchanged); the presets set to their styles and to the dry's level, with Opto bass and FET vocal added; fxlab's timing measure fixed (above). After it: all 76 compressor targets met (16 of 24 before; K12 to K15, auto makeup's K7 rows and the new presets' rows are new); the other classes are unchanged.

- **Met now:** a steady sine gets the drawn curve within 0.013 dB (Peak; was 0.95 short) (K1). Attack takes 1.08, 1.00, 1.00 and 1.00 times its label at 1, 10, 30 and 100 ms (K2; 1.58, 1.48, 1.49 and 1.51 on main by the fixed measure), release within 1 % (K3). 10 dB of reduction at the defaults distorts 50 Hz at -57.7 dB and 100 Hz at -69.3 (K4; -43.8 and -49.8). Ratio 1 is bit for bit (K8; -132 dBFS). With auto makeup the defaults play pink at -18 dBFS 0.55 LU under the dry, and every preset with its own makeup within 0.3 LU (K7; the presets were +1.9 to -8.2). After 3 s of 0 dBFS Opto comes 90 % back 7.4 times slower than after 50 ms, Bus 2.3 and FET 2.7 times, Clean the same (K12). Range 6 caps where it says (K13); Link 0 leaves a quiet side untouched and Link 50 gives it half the loud side's reduction (K14). Each style meets the curve, the times and the bass targets, and a style change reads 0 dB; FET's 5 kHz alias at 0 dBFS is -84.6 dB (K15).
- **Reported:** FET's colour at 10 dB of reduction, -42.4 dB THD at 50 Hz, -43.0 at 100 Hz and -43.2 at 1 kHz: the grit it is for. An 8 kHz tone through FET at 0 dBFS: its third harmonic (24 kHz) folds to 20.1 kHz at -42.4 dB. Overshoot on a -40 to 0 dBFS step, 11.4 dB at 1 ms and 13.2 at 10 ms: no look-ahead (the Limiter has it). The defaults without makeup play pink 6.5 LU under the dry (was 3.0): they now reach the reduction drawn.
- **Blessed worse, and why:** the 1 ms attack reads 1.08 (was 1): the old measure read whole milliseconds, and main's engine reads 1.58 by the fixed one. RMS curve error 0.026 dB (was 0.019), inside 0.25: the hold keeps the top of the RMS level's small ripple on a 1 kHz sine, where the old detector averaged it (the likely cause, not isolated). The zipper on Drum punch, 0.49 dB (was 0.12), Vocal level, 0.64 (0.20), and Sidechain pump, 0.639 (0.635), inside 3 dB: the presets now reach their full reduction and two carry their styles' slow stage, so a sweep moves a larger gain.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): Compressor 21.0k to 27.7k instructions (1.32 times). The cases that carry it rose too: the drive, filter, delay and compressor chain 2.1 %, the mixer with a song's inserts (seven compressors) 3.8 to 5.7 %, the frozen-tracks mixer 2.1 % and the song 5.0 %.

### Lo-fi PR

What changed (design: [fx-lofi.md](fx-lofi.md); decision 10.5): the wow follows a smooth random path at about the new **Rate**, and **Stereo** lets the sides drift apart; wow and flutter grow along a curve that keeps the bottom where it was and reaches 2.2 % and 0.55 %; the saturation runs at twice the rate with first-order ADAA (Drive's helpers); new **Dropouts**, **Hum** with **Mains**, and **Follow** (ids 10 to 15; 0 to 9 unchanged); a fixed makeup of 2.5 dB at Amount 100; Amount's first 2 % fades the effect in, so Amount 0 is bit for bit; the tape centred at 10 ms (latency 445 frames at 44.1 kHz, from 88); Broken machine (the one preset that sets Amount, to 100), VHS and Vinyl bed added. After it: all 38 lo-fi targets met (17 of 26 before; the Follow and dropout targets and the new presets' level and zipper are new); the other classes are unchanged.

- **Met now:** Amount and Wow at 100 bend the pitch 2.15 % (was 0.39), Flutter 0.50 % (0.13), the defaults 0.126 % (0.116) (L1, L2); the wow's strongest line holds 0.19 of its power (0.81) (L4); Saturation 40's 5 kHz alias at 0 dBFS -89.2 dB (-27.6), 100's -49.6 (-16.3) (L5); with Follow at 0 the noise is the same playing and in silence (L7); pink at -18 dBFS: the defaults +0.17 LU (-1.30), Worn tape +0.07, Dusty vinyl +0.26, Cassette -0.50, Broken machine -0.58, VHS -0.41, Vinyl bed +0.67 (L9; the old presets were -1.2 to -1.6); Amount 0 bit for bit (L10); a dropout within 0.005 dB of its depth and falling over 6 ms (L12).
- **Reported:** Old radio -1.55 LU (from -3.01); the third harmonic 4.4 dB over the second at Saturation 40 and 9.5 at 100; the noise 40.1 dB under a -18 dBFS tone at the defaults, -52.4 dBFS at Follow 0; dropouts about 2.8 a second at full; Stereo 100's sides 3.2 % apart; Hum 100 at -37.5 dBFS; latency 445 frames.
- **Changed measures:** the pitch rows listen 60 s and the level 9 s (above); a short window can't tell a random wander from a line (two sines read 0.68 over a few seconds, the random path 0.1 to 0.25) and one dropout more or less moves a 1 s level by a dB.
- **Cost** (the instruction gate's `Lo-fi` case, rustc 1.97.0, a 128-frame block at 44.1 kHz, Amount 100): 117.0k to 222.0k instructions (1.9 times), next to Drive Eco's 202.6k: the saturation at 2x with ADAA is about half of it.

### Filters PR

What changed (design: [fx-filter.md](fx-filter.md); fx.md, the Multi filter entry; decision 10.4): the Multi filter's resonance bends instead of clipping (its band-pass state bent past a knee that falls with Resonance, at Resonance 0 and in the notch, bell and shelves nothing bends); a type or slope change crossfades over 10 ms; the drive runs at twice the rate with ADAA against a phase-matched dry side; the latency is a fixed 4 frames, drive on or off. The DJ filter is unchanged. After it: all 30 filter targets met (25 of 30 before); the other classes are unchanged.

- **Met now:** pink at -18 dBFS RMS through a low-pass swept 100 Hz to 10 kHz at Resonance 100 peaks at -2.01 dBFS (12 dB) and -3.09 (24 dB) (F3; were +2.77 and +1.03; at Resonance 75, -2.77 and -3.24, were -0.83 and -2.53). A type change from low-pass to high-pass at 24 dB reads 0 dB and 12 to 24 dB 1.19 (F5; were 63.5 and 29.3). At Drive 12 a 5 kHz sine at 0 dBFS aliases at -71.3 dB (F7; was -19.7; Drive 24, reported, -41.1, was -15.4).
- **Kept:** a -40 dBFS sine at the cutoff at Resonance 100 rings up 31.15 dB (12 dB) and 31.10 (24 dB) (F4; were 32.04 and 32.04: below the knee the filter is linear, and the bend takes the top of the ring). The cutoffs, the passband and the Resonance-0 sweeps read as before (F1, F2, F6; the sweeps within 0.02 dB).
- **Blessed worse, and why:** the 12 and 24 dB low-pass sweeps at Resonance 0 read 0.516 and 1.448 dB (were 0.515 and 1.435), and the 24 dB band-pass 1.290 (was 1.287): the sound is now 4 frames late against the same sweep of the cutoff, all well inside 3 dB.
- **Cost** (the instruction gate's cases, rustc 1.97.0, a 128-frame block at 44.1 kHz): Multi filter (LP 12) 11.3k to 8.5k instructions (0.75 times; at rest the filter runs a block at a time), Multi filter (LP 24, drive) 33.7k to 139.8k (4.1 times: the drive at twice the rate with ADAA, as Drive's Eco), the drive, filter, delay and compressor chain 333.9k to 339.8k.

### Filters: Ladder and Res mode PR

What changed (design: [fx-filter.md](fx-filter.md); fx.md, the Multi filter entry; decision 10.4, Jay's pick "Ladder plus Resonance switch"): **Model** (id 6: Multi, Ladder) adds the **Ladder**, a Moog-style TPT ladder at twice the rate with its input through a curve; **Res mode** (id 7: Soft, Hard, Clean) picks what holds the resonance back, on both models. Soft is the Filters PR's bend (`tanh` on the ladder) and the default, so saved projects sound as they did; Hard clips the band-pass state at a ceiling with a rounded edge, the filter running at twice the rate (on the ladder, its input clipped instead of `tanh`); Clean is the linear filter with no ceiling (on the ladder, the feedback stops at 3.9, short of singing). A Model or Res mode change crossfades over 10 ms. After it: all 52 filter targets met (the ladder's 9 and the Res mode's 13 are new); every other row, the Multi filter's at its defaults included, reads as before.

- **The ladder (F9, and F1, F3, F5 for it):** it self-oscillates within 0.06 cents of the cutoff at 100 Hz to 5 kHz at -10.8 dBFS RMS, the level the same within 0.01 dB; swept pink at full Resonance peaks at -2.2 dBFS; heard within 0.002 dB of its drawing; Multi to Ladder reads 1.2 dB. Reported: Resonance 75's swept pink -6.3 dBFS; a quiet sine at Resonance 90 rings up 24.2 dB; the passband at full Resonance -4.7 dB; a 5 kHz sine at 0 dBFS aliases at -114.9 dB with no drive and -52.4 dB at Drive 12; Ladder bass's zipper 12.8.
- **The Res mode (F12):** on Hard, swept pink at full Resonance peaks at -1.6 dBFS (12 dB) and -1.9 (24 dB), a -40 dBFS sine at the cutoff rings up 32.0 dB (straight below the clip's edge), and at Resonance 50 it is heard exactly as drawn; Soft to Hard reads 0.5 dB and Soft to Clean 0. The Hard ladder sings within 0.24 cents of the cutoff at -10.6 dBFS RMS and swept pink peaks at -3.6 dBFS; the Clean ladder's click has rung out to silence (-140 dBFS) by three seconds; on the ladder Soft to Hard and Soft to Clean read 0.0 dB. Reported: Clean's swept pink peaks at +2.76 and +1.02 dBFS (the baseline's +2.77 and +1.03: no ceiling, as before the Filters PR); a -12 dBFS sine at a full-Resonance 24 dB high-pass's cutoff (2 kHz), where the ceiling works hardest and nothing after it filters what it adds, aliases at -57.5 dB on Soft (at the session rate) and -82.9 dB on Hard (at twice it); the Hard ladder's 5 kHz sine at 0 dBFS aliases at -54.5 dB (its clip is not antialiased; the Amp's and Fuzz's heavy-distortion targets are -50); the Clean ladder's swept pink peaks at +0.7 dBFS; Screaming low-pass's zipper 11.3.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): the new Multi filter (ladder) case 92.0k instructions (Resonance 70, no drive) and Multi filter (LP 12, Hard) 80.4k (the stages at twice the rate between the half-bands); Multi filter (LP 24, drive) 139.8k to 143.7k and (LP 12) 8.5k to 8.6k (the engine and Res mode checks).

## Round 3: chorus, flanger and phaser

Measuring built 2026-11-17 with the chorus's PR (M1), class `modulation`, target ids M; research and plan in jferments/mpfree-claude `fx/quality/` (research-modulation.md, plan-3.md); the design is [fx-modulation.md](fx-modulation.md). Subjects are `modulation/chorus`, `modulation/flanger` and `modulation/phaser` at their defaults and each preset (`modulation/chorus[Lush ensemble]`), at 44.1 kHz.

| Id | Row | Target | Why |
|---|---|---|---|
| M1 | `insert_lu_pink`, `insert_lu_hits` | Loudness change on insert within ±1 LU, every effect and preset | "Goes in quieter" sounds weak |
| M2 | `feedback_lu_m95` to `feedback_lu_p95` | At the default Mix, within ±3 LU of Feedback 0 from -95 to +95 % (flanger, phaser) | More feedback sharpens the resonance, not the level |
| M3 | `wet_corr_300hz_<n>v` | Wet left/right correlation above 300 Hz on a mono source at full Width ≤ 0.3, every voice count | A Width that does nothing at some counts is a bug |
| M4 | `mono_fold_lu` | Folding to mono loses ≤ 1.5 LU beyond the stereo change | A wide effect must not vanish in mono |
| M5 | `bend_cents_*` | A 1 kHz tone's peak bend through the chorus ≤ ±100 cents anywhere on the knobs (the flanger's reported) | Past a semitone a chorus is seasick |
| M6 | `bend_ratio_delay5_to_40` | The bend at one Rate and Depth changes ≤ 10 % from Delay 5 to 40 ms | Delay moves where the copy sits, not how much it wobbles |
| M7 | `half_frame_db_to_16k`, `half_frame_notch_db_12k_16k` | Held, the chorus's wet at half a frame within 0.5 dB of a whole frame to 16 kHz; the flanger's high notches ≤ -30 dB | The highs mustn't flutter as a sweep crosses fractions |
| M8 | `read_sidebands_db_12k` | A 12 kHz tone through the default chorus (one voice, wet): energy beyond ±5 % of it ≤ -70 dB | Interpolation sidebands are a grainy shimmer |
| M9 | `drawn_err_db_*` | The flanger's and phaser's held response within 0.5 dB of the drawn one (outside notches deeper than -20 dB) | The page's graph is how they are set |
| M10 | `longest_delay_ms` | The flanger's swept delay ≤ 20 ms | Past that it is a doubler |
| M11 | `sync_error_cycles_after_wrap` | A synced LFO within 1 % of a cycle of the song a bar after a 3.5-bar loop wraps | A bar-synced sweep stays on the bar |
| M12 | `rest_dbfs_*` | 20 s of noise at ±95 % Feedback with extreme settings: finite, and under -60 dBFS after the reported tail | No runaway, no stuck tail |
| M13 | `click_db_over_6k`, `click_db_over_6k_shape` | Each control swept end to end in 50 ms under a 997 Hz tone: the loudest millisecond above 6 kHz ≤ -60 dB against the tone (the Delay's row reported; from M4, a change of Shape at the defaults too) | A bend or a resonance passing the tone is not a click; energy two octaves up is |
| M14 | the instruction gate | Rises stated in the PR | The MPC's budget |

### Baseline (2026-11-17, before any fix)

61 of 113 modulation targets met. The chorus is 3.0 to 3.7 LU quieter on insert (a linear 50 % blend of a copy that, above the bass, is a different moment of the music); with three voices its sides correlate 0.67 above 300 Hz and with four 1.00 (the right side's voices a quarter cycle on land on the left's); its sweep is a share of the Delay, so the bend is 7.9 times more at 40 ms than at 5 and reaches ±866 cents at Rate 10, Depth 100, Delay 40 (the read runs backwards); its cubic read is 4.6 dB apart between half and whole frames to 16 kHz, and a 12 kHz tone grows sidebands at -49.8 dB. A synced LFO is half a cycle off after a 3.5-bar loop wraps. The flanger is 0.3 to 2.9 LU quieter, 6.0 to 6.5 LU louder at ±95 % Feedback, its high notches only 21 dB deep at half a frame, its drawn comb up to 46 dB wrong at half a frame and -90 % Feedback, and its sweep reaches 40 ms; "Jet sweep" loses 1.75 LU in mono and clicks at -54 dB. The phaser is 0.9 to 1.9 LU quieter and 5.7 to 7.4 LU louder at ±95 %; its notches are exactly where drawn. Feedback is stable everywhere.

### PR M1: the chorus

What changed (design: [fx-modulation.md](fx-modulation.md), [fx.md](fx.md)): the chorus reads through an 8-tap windowed sinc, Depth is a sweep in milliseconds under a semitone ceiling, the right side's voices sit between the left's at every count, it has the Mix curve (Equal by default), and a synced LFO follows the song on all three effects. After it: 82 of 113 met (61 before); every chorus row met.

- **Met now, the chorus at its defaults and every preset:** on insert -0.66 to +0.06 LU (M1; -3.0 to -3.7); correlation -0.08 to 0.03 above 300 Hz at every voice count (M3; 0.67 and 1.00 at three and four); bend at most ±96 cents anywhere (M5; ±866) and the same at every Delay, ratio 1.00 (M6; 7.9); half and whole frames within 0.16 dB to 16 kHz (M7; 4.6) and sidebands at -87.3 dB (M8; -49.8); a synced LFO back on the bar after the wrap, 0.000 of a cycle (M11; 0.5), on the flanger and phaser too.
- **Blessed worse, inside the targets:** the mono fold of "Lush ensemble" and "Bar swirl" is -0.99 and -1.01 LU (M4; 0.00 and -0.33): they are wide now, and a wide chorus loses a little in mono. The click measure of the presets reads -66.4 to -87.2 dB (M13; -82.3 to -95.7 before; Lush ensemble, the deepest sweep, the highest), still 6.4 dB or more inside the target.
- **Not in this PR:** the flanger (M2, M7, M9, M10 and its level) and the phaser (M2 and its level), PRs M2 and M3.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): Chorus (2 voices) 95.0k to 103.8k (+9.2 %), Chorus (4 voices) 154.9k to 173.9k (+12.3 %): eight taps a read where the cubic took four. Gathering the taps with a wrap mask each cost 53 to 61 % more; writing each sample twice so a read's taps sit side by side, summing in four lanes and skipping the glides that have settled brought it here. The flanger and phaser are unchanged.

### PR M2: the flanger

What changed (design: [fx-modulation.md](fx-modulation.md), [fx.md](fx.md)): the flanger's loop reads through the chorus's flat sinc (fading to the cubic within 4 frames of the write), its drawing uses the same weights, its sweep stops at 20 ms, and a level trim holds it at the dry's level at any Feedback and Mix; and through-zero flanging, off by default, is added. After it: 99 of 113 met (82 with M1), and with through zero's six new rows 105 of 119.

- **Met now, the flanger at its defaults and every preset:** on insert -0.46 to +0.09 LU (M1; -2.94 to -0.26); across Feedback -1.20 to -0.14 LU from Feedback 0 at ±70 and ±95 % (M2; +1.41 to +6.45); the high notches at half a frame -54.8 dB (M7; -21.4); the drawn comb within 0.0003 dB of the heard one at whole and half frames and ±90 % (M9; up to 46.3 dB); the longest swept delay 20 ms (M10; 40).
- **Why a trim and not a feedback gain on the wet:** scaling the wet by √(1 - g²) holds the level too, but at +90 % it leaves the wet a quarter of the dry at the notches, so they fill in (the comb's peak-to-notch contrast falls from 27 dB to 17). The trim, `1 / √((1 - m)² + m² / (1 - g²))`, scales dry and wet together, so the comb's shape and the notches' depth stay as they were. The model predicts the old flanger's insert levels within about 0.5 dB at every preset (-2.34 dB at the defaults, where fxlab measured -2.37 LU).
- **Blessed worse, inside the targets:** the click measure, now -81.6 to -66.6 dB (M13; -83.8 to -77.9; the loudest is Rate swept to 10 Hz, its bends round the loop; the trim lifts the wet by up to 3 dB at low Feedback and the sinc keeps the highs the cubic dulled); the level 20 s into ±95 % noise after the reported tail, -78.4 and -79.2 dBFS (M12, target -60; -89.4 and -89.1): the tail estimate follows the 20 ms ceiling, so it is half as long at Delay 10 ms, Depth 100 %, and still covers the ring-out; the mono fold 0.01 to 0.04 LU more.
- **Still unmet:** "Jet sweep" and "Hollow flange" lose 1.77 and 1.56 LU in mono (M4, target 1.5): with high Feedback the two sides' resonances fall at different pitches, so they partly cancel when summed; and "Jet sweep" reads -55.0 dB on the click measure (M13; -54.4 before) with Rate at 10 Hz, the sweep's sidebands round a 75 % loop rather than a step.
- **Through zero** (Jay, 17 Nov: "Add it, off by default"): the dry waits 5 ms (the latency reported) and the copy, inverted, sweeps Delay × Depth either side of it, so it passes the dry and cancels there; the dry is read through the copy's own flat read, so the crossing is a complete null. Six new rows, all met: the Tape flange preset at +0.23 LU on pink noise and -0.34 on hits (M1), -1.30 LU in mono (M4) and -63.4 dB on the click measure (M13), and the comb drawn against heard at the crossing, wet, within 0.0000 and 0.0007 dB at Feedback 0 and +90 % (M9). Its level needed one more term: the inverted copy cancels what it shares with the dry, mostly the bass, so without it the preset went in at -0.42 LU on pink and -1.00 on hits; the trim now takes `2 · 0.14 · m (1 - m)` off the power it models. After it: 105 of 119.
- **Not in this PR:** the phaser (M3).
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): Flanger 55.1k to 64.3k (+16.6 %): eight taps a read a side where the cubic took four. Reading through the sinc's own fused path where it can was dearer (66.4k): the shared weights are one table blend and one sum. Through zero off is 0.8 % of that (one loop, as before, while it is off and settled; writing the dry's lines all the time cost 26 %); on, a new case, 92.6k.

### PR M3: the phaser

What changed (design: [fx-modulation.md](fx-modulation.md), [fx.md](fx.md)): a level trim on dry and wet together, a 10 Hz high-pass in the feedback path, a Stereo choice (Quarter or Opposite) and a Main page. After it: 115 of 119 met (105 with M2 and its through zero).

- **Met now, the phaser at its defaults and every preset:** on insert -0.51 to +0.28 LU (M1; -1.93 to -0.92); across Feedback -1.00 to +1.17 LU from Feedback 0 at ±70 and ±95 % (M2; +0.80 to +7.35).
- **The trim**, `1 / √((1 - m)² + m² / (1 - g²) + 2 m (1 - m) · 0.3)`: the loop is all-pass, so white noise round it gains exactly 1 / (1 - g²) whatever the stages and the sweep (wet only, the trim holds the level exactly); the third term is the dry and the wet adding below the first notch, 0.3 of their cross power on K-weighted pink over the sweep at the defaults. A model of fxlab's measure (the response held at 200 points of the sweep, pink through a K-weighting) gave the old levels within 0.05 LU at every preset and within 1.2 LU across Feedback; with the trim, every Centre from 100 Hz to 8 kHz at every stage count sits within -0.9 to +1.8 LU. A trim that followed the stages and Centre could flatten that, but a phaser should change as its notches move, and a constant never moves during the sweep.
- **The feedback's high-pass:** the stages pass DC unturned, so with positive Feedback the loop rang at DC: at +95 % a sound's lowest few hertz came out 26 dB up (an unweighted pink noise 5.8 dB up at Mix 50 %, which the K-weighting doesn't hear) and a DC offset six times over. A 10 Hz one-pole on what goes round ends that and leaves the notches above about 30 Hz where they were; the drawing includes it (M9 0.0001 dB), and the tail estimate covers its ring (M12 stays at -140 dBFS; without that, -77).
- **Blessed worse, inside the targets:** the click measure on four presets, -79.5 to -89.4 dB (M13, target -60: "Funk wobble" -90.9 to -79.5, "Deep phase" -81.0 to -80.0, "Quarter pulse" -84.8 to -83.4, "Liquid" -89.8 to -89.4; a Feedback sweep moves the trim with it); at the defaults and on "Slow swirl" it is 4.5 dB better. The mono fold, up to 0.01 LU more.
- **Still unmet:** "Liquid" loses 1.68 LU in mono (M4, target 1.5): with twelve stages at 70 % Feedback the sides' resonances fall at different pitches, and fxlab hears 0.4 of its 10 s cycle (over a whole cycle the model gives 1.53). At 60 % Feedback it would about meet the target, but that changes the preset's character, so it stays.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): Phaser (4 stages) 61.6k to 66.0k (+7.1 %), (12 stages) 100.5k to 107.3k (+6.8 %): the high-pass, the trim's two gains with Mix folded in, and a check each frame whether Feedback or Mix moved. With the high-pass before the stages it was 15.7 to 17.6 %: the extra state there pushed the stage loop's values out of registers.

### PR M4: the LFO's shape and syncs

What changed (design: [fx-modulation.md](fx-modulation.md), [fx.md](fx.md)): a Shape for the shared LFO (Sine, or a Triangle with rounded corners), appended to all three, and the dotted and triplet syncs (1/8T, 1/8D, 1/4T, 1/4D) appended to the Sync list; plan-3 P2 items 8 and 9. Saved projects open on Sine and keep their sync. With Shape the flanger has nine controls, so it gets a Main page (Rate, Depth, Feedback, Mix, Sync, Delay, Width, Through zero). Four rows added, all met: 119 of 123 (115 of 119 with M3, and no earlier row moved).

- **New rows:** M5 `bend_cents_depth100_r10_d40_triangle`, the chorus's Triangle at Rate 10 Hz, Depth 100 % and Delay 40 ms, 68.2 cents (the Sine's 95.7: a triangle's steepest slope is 4/0.9 a cycle against a sine's 2π, so the ceiling worked out for the sine holds); M13 `click_db_over_6k_shape` at each effect's defaults, Shape from Sine to Triangle and back under the 997 Hz tone: chorus -85.9 dB, flanger -89.6, phaser -102.1 (target -60).
- **The corners:** a parabola a twentieth of a cycle either side of each, so the slope turns round over a tenth of a cycle instead of flipping; the peaks are scaled back to ±1, so a Triangle sweeps exactly as far as the Sine. A change of Shape crossfades the two over 50 ms.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): the shape is picked once a frame and the frame's sweeps worked out together (`Lfo::waves`), which lets the compiler do the sines side by side. Chorus (2 voices) 103.8k to 99.2k (-4.4 %), (4 voices) 173.9k to 147.1k (-15.5 %), where it works out all eight sweeps (four voices a side) at once; Flanger 64.3k to 64.9k (+1.0 %), with Through zero on 92.6k (+0.0 %); Phaser (4 stages) 66.0k to 66.5k (+0.7 %), (12 stages) 107.3k to 107.8k (+0.4 %), the shape's check each frame. Picking the shape for each read instead cost the flanger 8.5 % and the chorus 3.3 %.

### The Ensemble (synth library wave 1, 18 Nov)

The string machine's chorus ([fx-modulation.md](fx-modulation.md#ensemble)) joins the class as `modulation/ensemble` and its six presets: M1 on insert, M4 mono fold and M13 clicks at each; the mix law (`mix_dip_lu`, `mix_bump_lu`, M1's numbers across the Mix knob, as the delays' D6); and the synth library's S3 width (`wet_corr_300hz_3lines`, `_4lines`, target 0.2 to 0.6, [synth-quality.md](synth-quality.md#wave-1-targets-bass-keys-and-strings-stage-4)). All met when added: on insert -0.27 to +0.36 LU, mono fold -1.27 to 0 LU, clicks -60.7 to -81.0 dB (Slow shimmer's Slow rate the highest), correlation 0.51 and 0.58, mix dip -0.33 LU. Only these 32 rows were added to the ratchet; the rest kept their recorded values.

## The sampler

`mpfree_fxlab::sampler` plays test sounds on a pad of `mpfree-sampler` (the engine itself, in 128-frame blocks, with the sound's octave copies made first) and measures what the voice does to them. `tests/sampler.rs` holds every number to `rust/mpfree-fxlab/quality/sampler.tsv` the same way `tests/quality.rs` holds the effects', and prints the report. It takes a few seconds in a debug build, so it runs with the workspace's tests on CI, with no release step. Record a change with `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test sampler`.

| Id | Row | Target | How it is measured |
|---|---|---|---|
| S1 | `pitch[...]/spur_worst_db`, `spur_hz` | -50 dB | A -6 dBFS sine tuned so its output lands on one FFT bin, at 3 and 9 kHz out, a fifth down and a semitone up (the Hermite read), a fifth up (the band-limited read) and an octave and a fifth up (an octave copy): the loudest other bin from 20 Hz to 20 kHz against the tone, and where it is (shown, not kept) |
| S1 | `pitch[...]/level_db` | within 1 dB | The tone's level against the same tone played at pitch, which is a plain copy |
| S2 | `fold[...]/heard_db` | -50 dB | What is heard of a tone that must fold away, against the tone: 15 kHz an octave up (30 kHz, above what 44.1 kHz holds), and a 96 kHz file's 30 kHz tone at pitch |
| S3 | `loop[...]/step_db` | none yet | A held 440 Hz sine looped over 300.48 periods, so the ends don't meet: forward with no crossfade, with a 1,000-frame one, and ping-pong. The sharpest second difference in the loop against the sine's own (0 when no sharper) |
| S4 | `stop[...]/step_db` | none yet | The same step when a held voice is stolen (its pad hit again at polyphony 1), choked (a pad of its mute group hit), and let go with the Play tab's shortest release (1 ms) and with none (0 ms, which a kit or project file can hold) |
| S5 | `read[at pitch, no filter]/flatness_db` | 0.1 dB | The pad's response from a sweep, 30 Hz to 16 kHz, against its level at 1 kHz |
| S5 | `filter[...]/error_db` | 1 dB | The response with each filter at 1 kHz (low-pass 2 and 4 poles, low-pass with resonance 0.9, high-pass, band-pass with resonance 0.5) against its analog prototype, `1 / (s² + ks + 1)` with `k = 2 - 1.95 × resonance` per stage (the second of a 4-pole at half the resonance, as the engine sets it), from 30 Hz to 8 kHz wherever the prototype is above -30 dB |

A mutation check: with Vintage on, S2 reads -3.0 and -4.1 dB and the ratchet fails.

### First numbers (2026-11-17)

- **Met:** the band-limited read and the octave copies keep every spur 73 dB down or more, and fold the 30 kHz tones 86 and 77 dB down. Up to 3 kHz out every read is clean (-50 to -73 dB), and every level is within 0.02 dB. The read at pitch is flat to under 0.0001 dB. A crossfaded loop steps no more sharply than the sine itself. Four of the five filters are within 0.03 to 0.74 dB of their prototypes.
- **Not yet, the Hermite read with high content:** a 9 kHz tone a semitone up has a spur at 6.4 kHz, 31 dB down, and a 13.5 kHz tone a fifth down has one at 5.8 kHz, 44 dB down and plays 1.1 dB quiet. These are the Hermite kernel's images folding back, not content above Nyquist, so `CLEAN_ABOVE`'s argument (nothing folds below 19.35 kHz up to two semitones) doesn't cover them. Closing them takes the band-limited read, or a longer interpolator, for every pitched voice, which is a cost the bench would count.
- **Not yet, the band-pass:** 1.02 dB from its prototype at 8 kHz. This is the state-variable filter's bilinear mapping, which puts 8 kHz where the prototype has 8.98 kHz, and the band-pass's 6 dB an octave turns that into 1 dB.
- **No target yet, the steps:** a forward loop with no crossfade steps 42 dB over the sine, and ping-pong 30 dB, as any reversal of a sine mid-wave does. A steal or a choke (the 32-frame linear fade, `STEAL_FRAMES`) steps 13 dB. A Gate pad let go steps 33 dB with the Play tab's shortest release (1 ms) and 47 dB with none: the release is exponential, so its first frames fall fastest (a 1 ms release loses about a fifth of the level in its first frame), and at 0 ms it is one frame, not the 64-frame fade of a hard stop (`KILL_FRAMES`).

## Round 4: the rest of the dynamics

Measuring built 2026-11-17, by `quality/round4.rs`: the Limiter and Master safety (class `limiter`, ids P), the Gate (`gate`, N), the De-esser (`deesser`, S), the Multiband compressor (`multiband`, X), the Ducker (`ducker`, U), the Transient shaper (`transient`, T) and the Channel strip (`strip`, C). Research and plan: jferments/mpfree-claude `fx/quality/` (research-dynamics.md, plan-4.md, baseline-dynamics-2026-11-17.md). The seven classes take about 2.5 s together. Subjects are `class/engine` at its defaults and presets, and named settings: `limiter/limiter-true-peak`, `limiter/limiter-auto-off`, `limiter/master-safety` (Safe limiter), `limiter/master-soft-clip`, `deesser/split`, `deesser/wide`, `deesser/relative`, `deesser/absolute`, `ducker/tempo`, and (from the Ducker PR) `ducker/tempo-dotted8` and `ducker/tempo-triplet4`.

The measures it adds, beside the earlier rounds':

| Measure | What it reads |
|---|---|
| Peaks over the ceiling | Pink at -14 dBFS RMS and a loud programme (that pink, drum hits peaking about +2.5 dBFS and a 55 Hz bass at -12 dBFS), 5 s each, pushed 0, 6 and 12 dB: the worst sample peak and 4x true peak against the ceiling, the first second left out |
| A tone falling through a gate | A sine falling evenly from 20 dB over the threshold to 20 dB under in 3 s: the gain's spread inside each half cycle while the input is 2 dB or more over (chatter), and the input level where the gain has fallen under -6 dB (where it shut) |
| The gate's fade | A 1 kHz tone dropping from 20 dB over the threshold to 20 dB under, Hold 0: the time from -0.5 dB to 1 dB short of Range, against that share of Release (the fade is even in dB) |
| Response at full reduction | Pink through the de-esser with Absolute detection at its lowest threshold, a Welch transfer function (4096-point Hann) smoothed to third octaves |
| A vocal-like line | A 150 Hz buzz for 400 ms, then an s (white noise high-passed twice at 4 kHz) over the buzz 12 dB down, repeated; the de-esser's meter read each block during the s and between them (the 100 ms after each s left out) |
| Gain against a reference | The multiband's mid band soloed, its output against the same band at ratio 1, so neither the crossovers' phase nor a level step's spread into the other bands counts |
| Against the drawing | The ducker in tempo mode on a 200 Hz tone in cosine phase (the beats land on its peaks): the gain at each frame where the tone is over half its peak against the page's curve at that song position, a quarter of a millisecond either way allowed |
| Song position | Nodes run with the transport playing at 120 BPM, a song position given per block, and a side-chain when one is fed; a jump moves the position at a block's start |
| Fronts | Drum hits from silence: the energy of each hit's first millisecond out against in (gate), or the peak of its first 5 ms (transient shaper) |

### Limiter and Master safety (P)

| Id | Row | Target | Why |
|---|---|---|---|
| P1 | `sample_peak_over_ceiling_db` | ≤ 0.01 dB, every subject | The promise |
| P2 | `true_peak_over_ceiling_db` | ≤ 0.1 dB with True peak on and in Master safety's Safe limiter; off reported | The converter rebuilds the wave between the samples |
| P3 | `thd_<hz>hz_<over>db_over` | 6 dB of limiting at the defaults: 50 Hz ≤ -40 dB, 100 Hz ≤ -50 dB (Limiter and Master safety); 12 dB, the other settings and the presets reported | Gain riding inside bass cycles |
| P4 | `release_slowdown` | With Auto release, the release (63 %) after 3 s of 6 dB limiting at least twice that after 0.1 s; the times reported | Fast after a short peak, slow under a push |
| P5 | `release_ratio_<ms>` | Auto off, and Master safety: 63 % within ±15 % of Release, 20 to 300 ms | The compressor's convention |
| P6 | the soft clip's `alias_5khz_6db_over`, `true_peak_over_ceiling_db` | ≤ -60 dB; ≤ 0.1 dB (12 dB and the limiters' reported) | Colour without fold-back, and never over |
| P7 | `latency_spread_frames` | 0 across every mode and setting | Exports and the mixer's compensation |
| P8 | `zipper_<control>` | ≤ 3 dB (round 2's slope zipper) | Smooth automation |

### Gate (N)

| Id | Row | Target | Why |
|---|---|---|---|
| N1 | `onset_1ms_db_attack<ms>` | The first millisecond of a hit from silence within 0.5 dB at Attack up to the look-ahead (5 ms and the presets reported) | The front is the drum |
| N2 | `ripple_db_<hz>hz_hold<ms>` | ≤ 0.5 dB, 40 Hz to 1 kHz, Hold 0 and the default | Gates mustn't trigger on bass cycles |
| N3 | `shut_error_db_<hz>hz_hold<ms>` | Shuts within 1 dB of the threshold less its 4 dB of hysteresis | The stated hysteresis at every pitch |
| N4 | `release_ratio_<ms>` | Within ±10 %, 50 to 500 ms | The label |
| N5 | `zipper_<control>`, `range_change_while_shut_error_db` | ≤ 3 dB; Range moved from -80 to -40 dB while shut lands within 1 dB of -40 | Smooth automation, and a control that works while it is shut |
| N6 | `expander_curve_error_db` | In Expander mode, a steady tone 3, 6 and 12 dB under the threshold (40 Hz and 1 kHz, Ratio 2 and 4) comes out within 0.5 dB of (Ratio - 1) dB down per dB under | The ratio means what it says (added with the mode, by the Gate PR) |

### De-esser (S)

| Id | Row | Target | Why |
|---|---|---|---|
| S1 | `lift_db_range<db>_<hz>hz` | Split at full reduction: no third octave under 0.8 times Freq rises more than 0.2 dB (Wide reported) | Turning the s down mustn't boost the voice under it |
| S2 | `range_error_db_range<db>_<hz>hz` | Split: within 1.5 dB of Range from 1.5 times Freq to 16 kHz | The knob means what it says |
| S3 | `s_reduction_spread_db` | Relative: the reduction during the s within 1 dB at -6, -18 and -30 dBFS (Absolute and the levels reported) | A threshold that works at any level |
| S4 | `voice_reduction_db_<level>dbfs` | ≤ 0.5 dB between the s | The voice left alone |
| S5 | `one_side_error_db` | An s on one side gets within 1 dB of the reduction it gets in the centre, both detections | Doubled and panned vocals |
| S6 | `zipper_<control>` | ≤ 3 dB (Freq and Range at full reduction) | Smooth automation |

### Multiband compressor (X)

| Id | Row | Target | Why |
|---|---|---|---|
| X1 | `idle_error_db` | Ratio 1, Mix 100: flat within ±0.1 dB, 20 Hz to 20 kHz | Inserting it costs nothing |
| X2 | `ratio1_error_db_mix<n>` | Ratio 1 at Mix 25, 50 and 75: flat within ±0.5 dB | A Mix that notches the crossovers is broken |
| X3 | `curve_error_mid_1khz` | The mid band's gain within 0.25 dB of its curve, -40 to 0 dBFS (the whole output's reported) | As K1 |
| X4 | `attack_ratio_<ms>` | Within ±15 %, 1 to 30 ms | As K2 |
| X5 | `release_ratio_<ms>` | Within ±10 %, 60 to 500 ms (90 % reported) | As K3 |
| X6 | `thd_<hz>hz_12db_over` | 50 Hz 12 dB over the low band's threshold ≤ -50 dB; 100 Hz and the presets reported | As K4 |
| X7 | `lu_pink_auto`, `lu_pink` | With Auto makeup on, pink at -18 dBFS within ±1.5 LU of the dry, at the defaults and each preset; with it off, reported | As K7 |
| X8 | `zipper_<control>` | ≤ 3 dB, every control | Smooth automation |

### Ducker (U)

| Id | Row | Target | Why |
|---|---|---|---|
| U1 | `beat_error_db` | Tempo mode: within 1 dB of full depth from each beat until the hold ends, the defaults and every tempo preset | The kick lands where the bass is already down |
| U2 | `drawing_error_db` | Within 1 dB of the drawing over the whole step: of the gains it passes through within a quarter of a millisecond either way (the Ducker PR; before, of its points a quarter of a millisecond apart) | The page is how it's set |
| U3 | `response_ms_attack<ms>` | Side-chain: the duck reaches -1 dB within 1 ms of the key crossing the threshold | |
| U3 | `hats_duck_db_key_lp120`, `kick_depth_error_db_key_lp120` | Key LP at 120 Hz on a drum key: hats at -6 dBFS duck the tone by at most 1 dB, and each 55 Hz kick takes it within 1 dB of Depth (the Ducker PR) | A drum bus keys on its kick |
| U4 | `click_db_attack<ms>` | Reported: a 60 Hz tone ducked by the beat, the worst of eight starting phases (the shortest attacks click by choice) | |
| U5 | `drawing_error_db_after_jump`, `_after_wrap` | Within 1 dB of the drawing from 50 ms after the song position jumps into a step or a loop wraps to the start | As M11 |
| U6 | `zipper_<control>` | ≤ 3 dB, while a long duck holds and releases | Smooth automation |

### Transient shaper (T) and Channel strip (C)

| Id | Row | Target | Why |
|---|---|---|---|
| T1 | `front_spread_db_attack<n>` | Attack 50 and 100: the fronts' gain within 1 dB across hits at 0, -12, -24 and -36 dB (Attack -100 and the gains reported) | A shaper with no threshold works at any level |
| T2 | `ripple_db_<setting>` | ≤ 0.5 dB on steady tones, 30 Hz to 1 kHz | Steady tones untouched |
| T3 | `hits_peak_change_db`, `hits_lu` | Reported, defaults and presets (decision 11.4) | An insert shouldn't jump unexpectedly |
| T4 | `zipper_<control>` | ≤ 3 dB | Smooth automation |
| C1 | `gate_*`, `comp_*` | The strip's gate alone as N1 to N3 (40 Hz and 1 kHz), its compressor alone as K1 to K4 against the compressor engine set as the strip sets it | Each stage as good as on its own |
| C2 | `lu_pink_auto`, `lu_pink` | With Auto makeup on, the defaults and presets within ±1.5 LU of the dry on pink at -18 dBFS; off, reported | Fair A/B |
| C3 | `switch_click_db_<stage>` | ≤ 3 dB: the gate switched on, the compressor off, a 6 dB EQ off and the order changed, under a 220 Hz tone | Switching a stage mid-note |
| C4 | `latency_spread_frames` | 0 whatever is on | The mixer's compensation |

The cost of each is in the instruction gate: new cases Limiter (12 dB of push) 39.5k instructions a block, with True peak 72.6k, Gate (threshold over the program's tone, so it opens on each burst) 17.4k, De-esser 42.4k, Multiband compressor 141.5k, Ducker (tempo) 14.8k and Transient shaper 26.0k; Master safety's was there (29.0k).

### Baseline (2026-11-17, before any fix)

87 of 152 dynamics targets met: limiter 17 of 33, gate 20 of 30, de-esser 11 of 20, multiband 15 of 27, ducker 6 of 18, transient 8 of 10, strip 10 of 14.

- **Limiter and Master safety:** sample peaks exact; true peaks over by up to 0.48 dB with True peak on (0.61 for Loud master), 1.38 off, and 1.11 in Master safety; 50 Hz under 6 dB of limiting -29.8 dB THD (Master safety -38.7); Auto release comes back in 21.8 ms after 0.1 s and after 3 s alike; with it off 63 % comes at 0.80 of Release; the Soft clip aliases at -27.6 dB with true peaks 4.4 dB over.
- **Gate:** fronts kept exactly; with Hold 0 it rides 40 and 60 Hz (3.7 and 2.3 dB); under 1 kHz it shuts 0.2 to 1.1 dB under the threshold rather than 4; the fade means its label (within 0.3 %); Range moved while it is shut does nothing until it opens again.
- **De-esser:** Relative detection reads the s the same at every level and leaves the voice alone; Split lifts the band under Freq by 1.3 to 1.8 dB and falls 6.9 to 22.1 dB short of Range above it; an s on one side gets 4.4 dB less with Absolute detection.
- **Multiband:** flat at Mix 100, but Mix 50 notches 19.7 dB at a crossover (25 and 75: 5.9); the mid band 0.57 dB off its curve; attacks 1.7 to 2.1 times their labels, releases 0.81 to 0.85; the mid band's Threshold and Ratio zipper (10.6 and 6.8 dB).
- **Ducker:** the tempo duck starts at the beat, so each beat is up to its full depth away from the drawing's (17.8 dB at the defaults); within 1.1 dB of the drawing at the defaults, 8.2 and 8.5 dB for Sixteenth gate and Slow swell; after the song position jumps into a step it stays 17.7 dB off its drawing (a wrap to the start is followed); the side-chain duck starts 0.43 ms after the key.
- **Transient shaper:** steady tones untouched; the fronts' gain depends on level (8.7 dB spread at Attack 50, 3.1 at 100); the defaults add 12.8 dB to hits' peaks and 13.6 LU.
- **Channel strip:** its compressor meets K1 to K4; its gate shuts 2.75 dB short at 40 Hz; switching the gate on clicks 12.9 dB (it restarts with an empty look-ahead), a 6 dB EQ off 9.3 dB (the crossfade starts at full speed) and the order 54.4 dB (no crossfade); 5.8 LU quieter on insert.

### Limiter and Master safety PR

What changed (design: [fx-dynamics.md](fx-dynamics.md); decision 11.1): the Limiter and Master safety's Safe limiter share a new core (`limit.rs`): a 4x true-peak detector matched to the meter (32-tap Kaiser-windowed sinc phases, folded), a hold of a quarter of Release up to 5 ms, a release in decibels that means its label, an Auto release whose slow stage acts, then the ramp over the look-ahead. Master safety always catches true peaks, its Soft clip runs at 2x with ADAA and then the core, its modes crossfade over 10 ms and its Ceiling glides. fxlab gains a reported P3 row, the loudness a programme pushed 6 dB keeps (`programme_lu_change_6db_push`). After it: all 33 limiter targets met (17 before); the other classes are unchanged.

- **Met now:** true peaks over the ceiling, 0.003 dB with True peak on (0.48; Loud master 0.61), 0.0005 in Master safety (1.11) and 0.002 through Soft clip (4.37) (P2, P6). 50 Hz under 6 dB of limiting distorts at -53.9 dB at the defaults (-29.8) and -63.1 in Master safety (-38.7); 100 Hz at -124.6 and -140 (P3). Auto release is 63 % back 26.4 ms after 0.1 s and 242.8 ms after 3 s of limiting, 9.2 times slower (21.8 ms after both) (P4). With Auto off, and in Master safety, 63 % comes at 0.99 to 1.00 of Release (0.80 to 0.83) (P5). The Soft clip's worst alias for 5 kHz pushed 6 dB is -81.8 dB (-27.6) (P6). The latency spread stays 0 (P7).
- **Reported:** with True peak off, 0.89 dB over (1.38). Under 12 dB of limiting 50 Hz distorts at -53.9 dB (Limiter) and -63.1 (Master safety); the Soft clip's alias at 12 dB is -62.3 (-17.0). The latency at 44.1 kHz is 81 frames for the Limiter (71) and 85 for Master safety (66): the detector's 15-frame lag and the soft clip's 4-frame pad, the same in every mode and setting. A programme pushed 6 dB keeps 2.61 LU of the push at the Limiter's defaults, 2.40 with True peak, 1.80 with Auto off and 1.86 in Master safety (3.21, 3.08, 2.47 and 2.54 measured on the old engines with this row): they were louder because they rode the gain inside the bass's cycles and let go early. A blind level-matched listening page goes with the pull request.
- **Blessed worse, and why:** none.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): Limiter 39.5k to 44.9k instructions (1.14 times), Limiter (true peak) 72.6k to 98.3k (1.35), Master safety 29.0k to 30.3k (1.05). The idle mixer fell from 52.8k to 44.8k (0.85): a quiet block passes the core at rest with only its delay, bit for bit. The routing-only mixers rose 1.46 and 1.25 times: a dozen loud tracks keep the master limiting nearly all the time, so it works out every point between samples; the mixers with a song's inserts rose 1.05 to 1.07.

### Multiband PR

What changed (design: [fx-dynamics.md](fx-dynamics.md#multiband-comp)): the Multiband's dry is the bands' sum at unity, so any Mix adds in phase; each band runs the Compressor's detector (the smooth decoupled peak detector in dB, its release solved through the attack); thresholds and ratios glide over 20 ms; Auto makeup is new (id 19, off by default, on in every preset; Drum crush drops its +6 dB a band for it). X7 now measures with Auto makeup on (`lu_pink_auto`, within ±1.5 LU) and reports it off (`lu_pink`). After it: 31 of 32 multiband targets met (15 of 27; the five Auto makeup rows are new targets); the other classes are unchanged.

- **Met now:** ratio 1 at any Mix flat within 0.06 dB (19.7 dB notch at Mix 50, 5.9 at 25 and 75) (X2). The mid band within 0.12 dB of its curve (0.57) (X3). Attack 0.97 and 0.94 of 10 and 30 ms (1.69) (X4); release 1.00 of 60, 150 and 500 ms (0.81 to 0.85) (X5). 50 Hz 12 dB over at -61.7 dB THD (-50.0) (X6). Pink at -18 dBFS with Auto makeup: -0.31 LU at the defaults, -0.31 to +0.79 at the presets (X7). The mid Threshold and Ratio zipper 0.05 and 0.03 dB (10.6 and 6.8) (X8).
- **Not met:** Attack at 1 ms reads 1.44 of its label (2.08): the mid band builds up through the crossovers for a few tenths of a millisecond before the detector can see it, and only a look-ahead would hide that.
- **Reported:** the whole output against the curve, 0.51 dB (0.92): the other bands' share of the 1 kHz tone passes uncompressed. With Auto makeup off, pink comes out -1.4 LU at the defaults, Drum crush -11.6 (its gains are gone).
- **Blessed worse, and why:** the zipper rows' floor rose from 0.018 to 0.023 dB and the crossovers' from 0.48 and 0.55 to 0.54 and 0.58, Release's from 0.02 to 0.07: the detector in dB follows the test tone a hair more closely; all far under 3 dB. Drum crush's 50 Hz THD 12 dB over went from -35.9 to -34.5 dB (reported only): at 1 ms and 8:1 the new detector rides closer to its curve inside the bass's cycles.
- **Cost:** Multiband compressor 141.5k to 149.9k instructions a block (1.06): the detector in dB and the new glides. Taking the dry from the bands' sum made the Mix fix free (a second pair of all-passes on the dry cost 1.22).

### De-esser PR

What changed (design: [fx-dynamics.md](fx-dynamics.md#de-esser); decision 11.2): Split is an eighth-order Linkwitz-Riley split a sixth of an octave under Freq, its low band plus its high band at the gain, which never lifts; detection takes the louder side; the sound is held back 2 ms so the gain is down when each s arrives (the latency, fixed, is 88 frames at 44.1 kHz); Wide passes the split's all-pass too, so switching modes at rest doesn't click. After it: all 20 de-esser targets met (11 before); the other classes are unchanged.

- **Met now:** Split's lift under 0.8 times Freq at full reduction is 0.002 dB (1.3 to 1.8) (S1), and from 1.5 times Freq to 16 kHz it is within 0.10 dB of Range 10 and 0.67 of Range 24 (7.7 and 22.1 short) (S2). An s on one side gets the reduction it gets in the centre within 0.0003 dB in Relative (0.70) and exactly in Absolute (4.40) (S5).
- **Reported:** Wide's rows are as before within 0.003 dB.
- **Blessed worse, and why:** Freq's zipper rose from 0.09 to 0.25 dB (the eighth-order split is re-tuned every 16 frames as Freq glides; far under 3 dB). Wide's lift and range rows moved by 0.002 to 0.003 dB and Absolute's voice reduction at -6 dBFS by 0.0001 dB: the all-pass in the path.
- **Cost:** De-esser 42.4k to 59.0k instructions a block (1.39): the split's five stages a side (one before) and the look-ahead's delay; run a side at a time it was 87.7k.

### Gate and Channel strip PR

What changed (design: [fx-dynamics.md](fx-dynamics.md#gate) and [Channel strip](fx-dynamics.md#channel-strip)): the gate shuts once the key hasn't reached its shut level for Hold, at least 20 ms, so a bass note's cycles never count as gaps; a Range raised while it is shut is reached over about 10 ms; an Expander mode (Mode, Ratio) and Key listen are new. The Channel strip's switches fade on an S-curve, its gate keeps its look-ahead filled while off and comes back open, an Order change dips to silence and swaps there, and its compressor gains the Compressor's Style and Auto makeup (on in every preset). After it: all 31 gate targets met (20 of 30 before; N6 is new) and all 20 strip targets (10 of 14; C2's six Auto makeup rows are targets now it is on the page); the other classes are unchanged.

- **Met now:** no chatter at 40, 60 or 100 Hz with Hold 0 (3.7, 2.3 and 0.41 dB) (N2); it shuts within 0.37 to 0.61 dB of the threshold less 4 dB from 40 Hz to 1 kHz at Hold 0 and 30 (up to 3.8) (N3); Range moved from -80 to -40 dB while shut lands within 0.001 dB (88: it didn't move) (N5); the expander's curve within 0.02 dB (N6). The strip's gate shuts within 0.75 dB at 40 Hz (2.75) (C1); with Auto makeup the strip is within -0.77 to +0.65 LU of the dry at its defaults and presets (C2); switching its gate on reads 0 dB over the tone (12.9), a 6 dB EQ off 0.43 (9.3) and an Order change 2.2 (54.4) (C3).
- **Blessed worse, and why:** the gate's 1 kHz shut at Hold 30 went from 0.10 to 0.51 dB and the strip gate's from 0.10 to 0.72 (both under the 1 dB target): the old 0.1 was two errors cancelling, the detector falling 1.1 dB early between peaks and the 30 ms Hold making it up; now Hold counts from the last peak, so it shuts Hold's worth of the falling tone late, as it does at every pitch. Range's zipper went from 0 to 0.05 dB (with the gate shut it used to do nothing at all). The strip's compressor switch went from 0.30 to 0.35 dB (the S-curve fades in 20 ms where the one-pole was 95 % there). The gate's release ratios at 120 and 500 ms moved by 0.004 and 0.0005 from 1.
- **Cost:** Gate 17.4k to 17.3k instructions a block (-0.8 %; the bench's tone sits just over the shut level, so it now stays open, as it should). The strip has no bench case; its idle gate now writes two samples a frame into its look-ahead.

### Ducker PR

What changed (design: [fx-dynamics.md](fx-dynamics.md#ducker); decision 11.3): on Tempo the envelope is the step itself, from where the song is in it: down from the beat through the hold and release, with the next beat's attack running before the beat so the duck is down when it lands; the drawing is the same function; after a jump, a wrap or a turn it catches up at the attack's speed (5 ms at most), and its timing controls glide. Nudge (±20 ms), the 1/8T, 1/8D, 1/4T and 1/4D rates, Key HP and Key LP, and a Kick from drums preset are new. fxlab: U2 compares with the drawing's span within a quarter of a millisecond (it compared with points that far apart, between which a steep attack moves several dB), U4 takes the worst of eight phases of its tone, and rows for the new rates and the key filter are added. After it: all 24 ducker targets met (6 of 18 before); the other classes are unchanged.

- **Met now:** each beat within 0.003 dB of full depth (17.8 at the defaults, 15.4 to 34.4 in the presets) (U1); every tempo setting on its drawing, Slow swell's release longer than its step included (8.4 measured the new way; 8.5 the old) (U2); 50 ms after a jump into a step, on its drawing (17.7) (U5); hats through Key LP 120 Hz don't duck (U3).
- **Reported:** clicks on a 60 Hz tone, the worst of eight phases, at Attack 0.1, 0.5, 2 and 10 ms: 70.5, 64.1, 50.9 and 38.8 dB (77.1, 64.7, 52.9 and 38.9 measured the same way on the old engine).
- **Blessed worse, and why:** the Release zipper rose from 0.023 to 0.024 dB: turning Release on Tempo moves where in the step's release the duck is (it glides over 20 ms). Shape's zipper fell from 2.90 to 0.026.
- **Cost:** Ducker (tempo) 14.8k to 16.1k instructions a block (+8.9 %): the step's value on every frame, where the stage machine sat idle between ducks.

### Transient shaper PR

What changed (design: [fx-dynamics.md](fx-dynamics.md#transient-shaper); decision 11.4): each slow follower is kept within 25 dB of its fast one, so a hit out of silence opens the same gap at any level; the followers' floor moved from -60 to -80 dBFS. Attack 50 stays the default. After it: all 10 transient targets met (8 before); the other classes are unchanged.

- **Met now:** the fronts' gain across hits at 0, -12, -24 and -36 dB spreads 0.50 dB at Attack 50 (8.73) and 0.67 at Attack 100 (3.11) (T1): at Attack 50 the loudest hit's front comes up 7.5 dB (14.2) and the quietest's 7.0 (5.5).
- **Reported:** Attack -100 takes 11.0 dB off the loudest front and 7.8 off the quietest (9.5 and 5.2), a 3.1 dB spread (4.3): the first millisecond passes before the detector hears it. An insert at the defaults raises hits' peaks 7.5 dB and their loudness 6.9 LU (12.8 and 13.6); the presets' peaks Punchy kick +11.2 (+15.0), Snappy snare +10.7 (+14.6), Tight room -1.3 (+0.6), Soft hits -5.9 (-5.5), Long tails +0.2 (-1.2). Steady tones (T2) and the zippers (T4) as before.
- **Blessed worse, and why:** none.
- **Cost:** Transient shaper 26.0k to 27.1k instructions a block (+3.9 %): the two clamps a frame.

## The grain engine

`mpfree_fxlab::grain` measures `mpfree-grain`'s `GrainVoice` on its own (no amp envelope, filter or effects after it), at 44.1 kHz in 128-frame blocks with fixed seeds, on pink noise and a pad (an A minor chord of detuned band-limited saws, both -20 dBFS RMS), a 1 kHz sine and a steady level (DC, whose output is the sum of the grains' windows). Grains read from 30 % into the source, with the voice's defaults (80 ms, 25 a second, Hann, timing 30 % random, no spray, no scan; before step A3, 20 a second and regular) unless a row says otherwise. `tests/grain.rs` holds every number to `rust/mpfree-fxlab/quality/grain.tsv` as the sampler's ratchet does, and prints the report; it takes under two seconds, so it runs with the workspace's tests. Record a change with `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test grain`. The targets are the granular engine plan's (sampler roadmap R7, mpfree-claude `sampler/granular/plan.md`).

| Id | Row | Target | How it is measured |
|---|---|---|---|
| G1 | `level[pink]/range_lu`, `level[pad]/range_lu` | 3 LU | Loudness against the source's at 40, 80 and 400 ms grains overlapping 2, 4 and 16 times, sprayed (0.3), scanning (1) and both: largest less smallest. The quietest and loudest are shown, not kept |
| G1 | `level[..., defaults]/offset_lu` | within 1 LU | The defaults against the source |
| G11 | `tone[A2, A4, B5]/pitch_cents` | within 5 cents | Short regular grains of one spot of the pad, four cycles long, at the note's density: the spectrum's peak near the note (65536-point transform, parabolic) against the note (step B5) |
| G12 | `hits[drums, half speed or double speed]/pre_db`, `attack_ms`, `flam_db` | ≤ -24 dB; ≤ 1 ms; ≤ 3 dB | White-noise bursts from silence every 0.25 s, each decaying 40 ms, stretched as Stretch sets it (90 ms, 45 a second, regular, from the start) to half and double speed with the bursts' starts as hits; the medians over the hits of: the loudest millisecond in the 30 ms before each hit's time against the hit's; from its time until the level first reaches half the hit's, in half milliseconds; the largest rise of the level (4 ms, a millisecond at a time) over its lowest since the hit's peak, in the next 60 ms. The loudness against the drums' and the same without hits are shown (step B5) |
| G13 | `move[...]/level_change_lu`, `extra_swing_db` | within 1 LU; 3 dB | A sprayed cloud (80 ms, 80 a second, spray 0.3) on pink and the pad moved by its LFO (a 0.5 Hz sine) on position (full depth), size, density, pitch (half depth) and spray (from 0.5, full), or by its envelope on size (0.5 s up, 1 s down to half): the loudness against the cloud unmoved, and how much further its level (400 ms windows every 100 ms) swings over two cycles (step B3) |
| G1 | `vary[...]/level_change_lu` | within 1 LU | Full size spray, full level spray and peaked spray, each against the same cloud without it (40 a second, spray 0.3), on pink and the pad (step B1) |
| G14 | `preset[...]/level_change_lu` | within 1.5 LU (the stutter: of -4) | Each grain preset (`mpfree_sampler::grain_presets`) on pink and the pad at the lab's position: the loudness against the default cloud's there. The stutter's grains fill half of each sixteenth with their fades, so it sits about 4 LU down (step B6) |
| G16 | `repeat[pink, 1/4 and 1/16]/level_change_db`; `repeat[tone 110 Hz, 1/4 and 1/16]/join_step_ratio` | within 0.5 dB; 1.5 | The Grains effect fully dry at 120 BPM, repeating a quarter note and a sixteenth from 2 s: on pink, the level from half a second on against the stretch of pink it repeats; on a 110 Hz tone at -20 dBFS, the biggest step from one sample to the next within 10 ms of each join against the tone's own (a cut with no fade jumps up to 140 times as far) (step C2) |
| G17 | `filter[pink, low pass and high pass 1 kHz]/stop_db`, `pass_db`; `filter[pink, band 1 kHz, Q 12, full spray]/octave_spread_db`; `filter[pink, low pass 1 kHz, Q 12]/peak_gain_db` | ≤ -20 dB, within 0.5 dB; 3 dB; 12 dB | The default cloud on pink with each grain's low pass and high pass at 1 kHz (no resonance): the level two octaves past the cutoff (4 kHz and 250 Hz, ±1/6 octave) and two octaves inside it against the same cloud unfiltered. A band pass at 1 kHz, full resonance and full spray, on a cloud reading all over the pink (spray 0.5, 8 s): how far apart its levels at 250 Hz, 1 kHz and 4 kHz (an octave each) are against the cloud unfiltered. A low pass at 1 kHz at full resonance: its sample peak against the unfiltered cloud's. The sprayed band's loudness is shown (step C4) |
| G18 | `quality[pink and pad, tape and lofi]/level_lu`; `quality[feedback pink -14 dBFS, full, tape]/growth_db`, `peak_dbfs` | within 1 LU; 1 dB, 3 dBFS | Tape and Lo-fi on the default cloud on pink and the pad, against the same cloud Clean; the Grains effect at full feedback in Tape, as G15. Shown: on the 1 kHz tone held in a time stretch (scan 1, no randomness, so the tone itself comes out), how far Tape and Clean move its pitch, peak to peak in cents over 20 ms windows (step C5) |
| G15 | `feedback[pink -14 dBFS, full]/peak_dbfs`, `growth_db` | 3 dBFS; 1 dB | The Grains effect (`GrainFx`, 2 s of memory, fully wet, reading half way back, spray 0.1) on 4 s of pink at -14 dBFS RMS (peaks near full scale) then 16 s of silence, at full feedback: the sample peak, and how far the level rises from seconds 8-12 to 16-20. The level held (seconds 16-20) and, at half feedback, how fast the trail falls (seconds 5-7 to 14-16) are shown (step C1) |
| G2 | `coherent[pink, overlap N]/stretch_over_sprayed_lu` | within 1.5 LU | A granular time stretch (scan 1, no spray) against the same cloud sprayed, at overlaps 16 (200 ms) and 64 (400 ms) |
| G3 | `smooth[...]/ripple_db`, `modulation_db` | 0.5 dB, -40 dB | The DC level through the grains: largest over smallest, and the RMS of the change against the mean; at the defaults (overlap 1.6) and with each window at overlap 2, including the Shape knob's square, triangle, swell and pluck (step B2). The exponential window is percussive on purpose: shown, not kept |
| G3 | `smooth[defaults, 1 kHz]/sideband_db` | -40 dB | The first grain-rate sideband (1 kHz ± the density, ±3 Hz) of a held sine against the tone, from a 65536-point transform (0.7 Hz bins). Until step A3 it used 8192 points, whose window's main lobe (±21.5 Hz) let the tone itself into the band: the -16.4 and -20.9 dB there were mostly the tone. `flutter_db`, the held sine's 10 ms level against its mean, is shown, not kept |
| G4 | `frozen[...]/swing_db` | 3 dB (pad), 6 dB (sine) | A frozen cloud (no scan, no spray) at every density from 15 to 60 a second: how far its RMS swings |
| G5 | `pitch[...]/spur_db`, `heard_db` | -60 dB | A sine pitched by a sprayed cloud (200 ms, 20 a second): the power within 60 Hz of the loudest bin from 20 Hz to 20 kHz at least 200 Hz from the wanted tone (and never within 140 Hz of it), against the tone through the same cloud at pitch; `heard_db` where the wanted tone is above 20 kHz and must fold away. Where the spur is is shown, not kept |
| G6 | `cap[2 s grains, ...]/level_change_lu` | within 1 LU | A sprayed cloud of 2 s grains at 50 and 80 a second (100 and 160 wanted, 64 a voice) against 400 ms grains at the same density. The share of grains skipped is shown |
| G7 | `spread[full]/level_change_lu`, `correlation`, `mono_fold_lu` | 0.5 LU, 0.45, 1.5 LU | Full spread against none on pink (80 ms, 40 a second, spray 0.3): the loudness change, the left/right correlation, and what folding to mono changes. The plan asked for a correlation of 0.3, but folding equal channels to mono keeps (1 + ρ)/2 of their power, so 1.5 dB needs ρ of 0.41 or more: the two targets can't both be met, and since step A5 the correlation's is 0.45 |
| G8 | `onset[pad, ...]/ms` | 15 ms | From the start to the end of the first 10 ms (in 2.5 ms steps) within 3 dB of the steady level, with 200 ms, 500 ms and 1 s grains |

Cost isn't in the report: `mpfree-bench`'s "Sampler grains (per grain)" scaling case counts it on CI (the instruction gate) and on the MPC in the device round, and that sets the grain budget (`GRAIN_BUDGET`). On the MPC, `mpfree-bench --afford` ends with a "grain budget: N grains" line (step A6): the grains that fit in the sampler's share of a period (25 %, worst block) beside 16 looping sample voices, rounded down to 16, from the fitted costs of "Sampler grains (per grain)" and "Sampler, looping pads held"; the line before it says what a sample voice costs in grains there (the engine's `VOICE_COST`, 8 from the workstation).

### Baseline (2026-11-17, before any R7 change)

- **Met:** Hann at overlap 2 is flat (ripple 0.004 dB); two octaves and three octaves up stay clean (-66 and -95 dB); mono fold at full spread 0.6 LU.
- **G1, G2:** the level moves 10.7 LU on pink and 11.0 LU on the pad across the grid; the loudest is always the time stretch, because grains read in step add in amplitude while the voice divides by √overlap as if they were unrelated (+10.5 LU over the sprayed cloud at overlap 16, +16.5 at 64). The defaults play 3.7 LU (pink) and 4.2 LU (pad) under the source.
- **G3:** the defaults overlap 1.6 times, so the window sum ripples 4.2 dB at 20 Hz (modulation -15 dB, the sine's sidebands -16 dB). Tukey and trapezoid ripple 6 dB at overlap 2.
- **G4:** a frozen cloud swings 10.3 dB on the pad and 32 dB on the sine as density moves by one grain a second at a time: grains reading one spot at regular times comb.
- **G5:** the Hermite read: a 15 kHz tone an octave up folds back whole (0 dB at 14.1 kHz); 9 kHz a semitone or a fifth up leaves spurs 30 dB down; 13.5 kHz a fifth down, 43 dB.
- **G6:** 22 % and 51 % of grains are skipped, and the clouds lose 4.5 and 5.7 LU.
- **G7:** full spread loses 1.8 LU and leaves a correlation of 0.76.
- **G8:** 97, 192 and 484 ms to speak with 200 ms, 500 ms and 1 s grains.

### Step A2: level and smoothness (2026-11-17)

The voice no longer scales each grain by 1/√overlap. Every grain sounds at full level and each output frame is divided by how much window is sounding, measured as the cloud plays ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain), Level).

- **G1, G2 met:** the level moves 0.84 LU on pink and 0.99 LU on the pad across the grid (from 10.7 and 11.0); the defaults are +0.15 LU and -0.25 LU from the source (from -3.7 and -4.2); a time stretch plays 0.42 LU (overlap 16) and 0.39 LU (overlap 64) over the sprayed cloud (from +10.5 and +16.5).
- **G3 ripple and modulation met for every window:** the defaults ripple 0.006 dB (from 4.2), modulation -83 dB (from -15); Tukey and trapezoid at overlap 2 ripple 0.008 dB (from 6), modulation -78 dB. The sine's sideband at the defaults is -20.9 dB (from -16.4): what is left comes from the grains starting at regular times, which step A3 (a little random timing) is for.
- **G6 level met:** with grains skipped at the cap the cloud keeps its level (0.01 and 0.16 LU, from -4.5 and -5.7); as many grains are skipped as before.
- **G8:** 50, 125 and 215 ms to speak (from 97, 192 and 484): a cloud that has only just started is no longer scaled for the full overlap it hasn't reached. The floor keeps the first grains' windows, so it isn't instant (A5 is the step for that).
- **Unchanged or nearly:** G4 swings 8.1 dB on the pad and 27.6 dB on the sine (from 10.3 and 32: the comb is A3's); G5 within 0.5 dB, but for three octaves up at -101 dB (from -95); G7 the same.
- **Heard:** a blind, loudness-matched A/B of five cases (a frozen pad at the defaults, a stretch with a Tukey window, drums with Density rising from 20 to 400 a second, long-grain piano presses and the Grains FX on drums).
- **Cost:** the level costs four running sums a grain frame, and the coherence measure one more sum a grain frame in one 64-frame part of every four. The instruction gate's granular cases rose (blessed): the 4 granular pads alone +12.0 %, Grains FX (dense) +9.0 %, 20 pads + master grains +4.6 %, 20 pads +2.5 %, the song +3.8 % (about 2 % of it this change). The MPC's figure comes with step A6.

### Step A3: timing (2026-11-17)

Grains no longer arrive in lockstep: each gap between free-running grains is anywhere from `1 - timing` to `1 + timing` density periods (uniform; the average density is unchanged), drawn from a random sequence of its own so the other settings' draws don't move and timing 0 plays exactly as before. The default is 30 % random (decision D3) and the default density is 25 a second, so the default 80 ms grains overlap twice. Synced density stays on the beat grid. The Grain page and the Grains node have a Timing control.

Why a bounded spread rather than a Poisson process: with exponential gaps at 30 %, overlap 2 leaves real holes (the window sum is under the level's floor 3.7 % of the time and the DC level modulates at -12 dB), and the A2 level law can't fill a hole. Uniform gaps up to 1.3 periods keep the window sum over 0.54 at overlap 2, so the level stays flat, and they take the first grain-rate line about as far down (the gaps' characteristic function at the grain rate is 0.50 against 0.47). A position jitter for frozen clouds, which the plan suggested, turned out not to be needed: timing alone takes the comb away.

- **G4 met:** a frozen cloud swings 2.8 dB on the pad and 2.7 dB on the sine as density moves from 15 to 60 (from 8.1 and 27.6): its level no longer rises and falls with the density.
- **G3 ripple and modulation still met** at the defaults and for every window (0.01 dB, -76 to -77 dB).
- **G3 sideband, measured properly (see the table):** -19.3 dB at the defaults. A frozen sine is where random timing costs most: grains of one spot at random times add with random phases, so it flutters (-13.8 dB) where regular grains at a density that divides its frequency (20 or 25 a second against 1 kHz) held it still (about -70 dB); at any other density regular grains comb it (G4). Set Timing to Regular for that stillness; step R7b's Tone mode is the planned way to keep a frozen tone still at any density.
- **G1, G2, G6 still met** (the spread of levels 0.83 LU on pink and 1.6 LU on the pad); G7 and G8 unchanged; G5 within about 1 dB but for two octaves and three octaves up, where the spurs move with the grains' times (-68 and -85 dB, still met). G5 is step A4's.
- **Cost:** one random draw a grain.

### Step A5: the cap, width and onsets (2026-11-17)

Three changes to how a voice places its grains ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain), Cap, running start and pan).

- **A cloud that wants more grains than fit thins evenly.** A voice holds 64 grains. A cloud whose size and density want more (2 s grains at 80 a second want 160) used to make grains on time and skip each one that found the array full, so it played in bursts with random holes. Now the gap between free-running grains is never shorter than the grains' length over 54, so the cloud keeps about 54 grains, evenly spaced, and random timing's shorter gaps rarely reach 64. The length is what the grains really get: a grain whose read at the cloud's pitch wouldn't fit in the sound is shortened, and the fit goes by that. `Stats::thinned` says when it happens, for the cloud view (step B7). Synced grains keep the beat grid and still skip at the cap.
- **A cloud speaks at once.** `start` makes, with the first grain, a few of the ones a cloud that had been running for a grain's length would have: walking back one gap at a time (drawn as the timing draws them), at least an eighth of a grain apart, each grain made that far through its window and reading from where the scan was then; the voice fades in over 4 ms so the first frame doesn't click. The level law (step A2) sets the level from the window that is sounding, so up to 8 grains are enough to play at the cloud's level from the start; the density fills in over the next grain. A first try started with the whole cloud (up to 54 grains): the same onset, but short hits then cost the whole cloud for their whole life (the song's instruction count rose 21 %, the 4 granular pads' 284 %). A slow swell is what the pad's amp Attack is for.
- **Width.** Pans are equal-power (each side `√2·cos` and `√2·sin` of the pan's angle, so 1 and 1 in the centre), so a spread cloud of unrelated grains keeps its level; before, a grain panned to one side lost the other side's half of its power. At full spread the random pans lean toward the sides (`|r|^0.5`), which makes the cloud wider than an even spread would.

Measured:

- **G6 met, and nothing skipped:** 0 % of the 2 s grains are skipped at 50 and 80 a second (from 22 % and 51 %), and the level is within 0.17 and 0.22 LU of grains that fit.
- **G7 met:** full spread keeps the level (+0.05 LU, from -1.77), the correlation is 0.44 (from 0.75) and folding to mono loses 1.38 LU (from 0.58), inside its 1.5. Equal-power pans spread evenly leave a correlation of 0.63 (mono fold 0.88 LU); the lean takes it to 0.44, and leaning harder (`|r|^0.3`: 0.31) takes the mono fold past its target (1.78 LU).
- **G8 met:** 10 ms to speak with 200 ms, 500 ms and 1 s grains (from 50, 122 and 222): the 4 ms fade and the first 10 ms window. `mpfree-grain`'s `a_cloud_sounds_at_once_without_a_bump` holds every 10 ms of a white-noise cloud's first second within 3 dB of its steady level.
- **Still met, within the measures' spread:** G1 on pink 0.63 LU across the grid and on the pad 0.64 (from 0.83 and 1.60); the defaults +0.35 LU on pink and -0.51 on the pad (from +0.38 and -0.33: the running start draws its grains' times first, so every later grain moves, and a frozen pad's level depends on those draws, as G4's swing shows). G2 0.38 and 0.42 LU. G3 ripple 0.004 to 0.006 dB and modulation -83 to -85 dB, sideband -19.5 dB. G4 2.7 dB on the pad and 2.4 on the sine. G5 within 2 dB.
- **Cost:** a test a frame for the fade, and up to 8 grains made at once on a press. The instruction gate's granular cases (blessed): the 4 granular pads alone +30.8 %, 20 pads +7.8 %, 20 pads + master grains +5.9 %, the song +2.6 %, Grains FX (dense) -0.9 %. The case hits a pad every 4 blocks, so its clouds are young: they now sound, and cost, from their first frame. The most a block can cost is unchanged, since the shared budget (256 grains) bounds it.

### Step A4: clean pitching (2026-11-17)

A pad's grains pitched more than two semitones up read band-limited, as a sample voice pitched up does: a Kaiser-windowed sinc stretched to the grain's speed, and past an octave the sound's octave copies ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain), Clean pitching). Each grain is aimed when it is born, so with pitch spray only the grains that go past two semitones pay for it. Vintage pads keep the Hermite read. The lab reads its sources as a pad does (`mpfree_sampler::grain_clip::GrainClip`, with octave copies for the pitched-up rows).

- **G5 met where the grains go up past two semitones:** 15 kHz an octave up is heard at -82.5 dB (from -0.3: it folded back whole), 9 kHz a fifth up leaves its worst spur at -71.0 dB (from -30.5), 6 kHz two octaves up -86.1 dB (from -68.9) and 2 kHz three octaves up -85.7 dB (the same).
- **Not yet, as for a sample voice:** 9 kHz a semitone up (-29.0 dB) and 13.5 kHz a fifth down (-44.2 dB). Those spurs are the Hermite read's own images of a tone near the top of the band, not folding: the sampler's pads keep the Hermite read there too (`sampler.tsv`: -31.5 and -43.6 dB), because the band-limited read costs 3 to 5 times as much a frame. A clean read at every pitch is a candidate for step C5's quality modes.
- **Unchanged:** every other row (no grain there goes past two semitones up, so it reads as before, bit for bit).
- **The Grains FX** (a live buffer) has no octave copies and keeps the Hermite read for now.
- **Cost:** the band-limited read is 3 to 5 times Hermite's a frame. The bench's 4 granular pads (Pitch spray ±5 st, about 3 grains in 10 past two semitones) count 82 % more instructions, 20 pads 26 %, with the master grains 17 %, the song 8 %. A Vintage pad keeps the old cost. The MPC's numbers come from the device round (step A6).

### Step B2: one Shape knob (2026-11-17)

The window is one control from square to pluck (the plan's decision 2): square at 0 %, triangle at 40, Hann at 60 (the default), swell at 80, pluck at 100, each joined smoothly to the next ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain), Shape). The older windows (Tukey, trapezoid, percussive) stay as they were.

- **G3 met at every named point, at overlap 2:** square ripples 0.005 dB (modulation -83.3 dB), triangle 0.007 dB (-81.2), swell 0.004 dB (-86.0), pluck 0.16 dB (-71.4). The pluck ripples most because its fast attack leaves the least overlap to even out; the level law keeps it well inside the target.
- **Unchanged:** every other row (the default stays `Window::Hann`, so every cloud at 60 % plays as before, bit for bit).

### Step B1: variation on every dimension (2026-11-17)

Each way grains differ has its own amount: Spray (with Spray shape, Even or Peaked), Size spray, Pitch spray, Level spray, Spread, Reverse and Timing ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain), Variation). New rows (G1, `vary[...]`): each new variation at full against the same cloud without it (40 a second, spray 0.3), on pink and the pad.

- **G1 met:** full size spray -0.05 LU on pink and +0.14 on the pad, full level spray -0.08 and -0.01, peaked spray +0.11 and -0.25. Level spray first measured -1.1 LU on pink: the level law's floor (0.3 from overlap 2) held a cloud of quieter grains under its level, so the floor's dense end now comes down with the grains' RMS level.
- **Unchanged:** every other row, bit for bit (with no variation nothing moves: size and level draw from a sequence of their own).

### Step B5: grain modes, a tone in tune and a stretch that keeps its hits (2026-11-17)

Grain modes (Cloud, Stretch, Repeat, Tone) set which controls the Grain page shows and where they start ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain), Modes). Cloud, Stretch and Repeat are today's engine with their own settings. Tone needs the grain rate to be a note in tune, so regular grains now keep their exact times, between frames when the period isn't a whole number of frames (Timing). New rows: G11, a tone's pitch at A2, A4 and B5.

- **G11 met:** A2 +0.01 cents, A4 -0.01, B5 -0.00. Started on whole frames, as before this step, the same tones were -0.4, -13.3 and -13.7 cents flat (each period rounded up to the next frame).
- **Unchanged:** every other row (only regular, free-running grains start between frames, and the lab's regular rows fall on whole frames).

Part 4, Stretch keeps hits (`GrainSource::next_hit`, `GrainVoice::render` "Hits", the sampler's `hits` module): a pad in Stretch gives its grains the sound's hits as `mpfree-stretch` finds them; a forward grain stops at the next hit (its window fitted to end there, dropped under a quarter of its size), the scan reaching a hit starts one grain on it at its window's peak (to the window's end over at least half the size, at the cloud's pitch, centred, full level), the grains playing fade out under it over 3 ms outside the level's sums, and the regular grains start again a period after it. A scanning cloud's running start no longer wraps round the region: a grain born before the start began where the scan was then, before the region's start if it must, as only what it still has to play is read, so a cloud scanning from the start opens in step with what follows instead of with the region's far end.

- **G12 met:** half speed, the 30 ms before a hit -70.8 dB under it (without hits +2.4: every grain reading ahead plays it early, louder than the hit itself); double speed, half the hit's level 0.5 ms after its time (without hits 20.0 ms: the grains reach it one by one); no second attack (1.9 and 0.4 dB). A stretch keeping its hits is 1.4 LU quieter at half speed (its hits are half as many a second, and what a slowed stretch has too little of just before each thins out) and 1.5 LU louder at double speed (twice as many whole hits).
- **Unchanged:** every other row.

### Step B3: movement per pad (2026-11-17)

Each grain voice has an LFO and an envelope (`mpfree_grain::motion`, `Params::lfo` and `env`), each moving one setting of the grains as they are made: position, size, density, pitch or spray ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain), Movement). They are worked out at the start of each 64-frame part of the voice's clock, so a moving cloud renders the same at any period size (`tests/motion.rs`), and the level law plays the moved cloud as any other. New rows: G13.

- **G13 met:** the loudness moves by at most 0.23 LU (the pad with its position swept across the whole sound), and no movement makes the level swing more than 1.3 dB further than the cloud unmoved does (pink with its spray swept from 0 to 1; every other case within 0.4 dB, most a little less than unmoved).
- **Unchanged:** every other row, bit for bit (with nothing to move, the settings are as set; the LFO's Hold and Drift values come from a random sequence of their own).

### Step B6: presets (2026-11-17)

Eight grain presets (`mpfree_sampler::grain_presets`), each a mode with its settings, key tracking, glide and movement, put on a pad by the Grain list's Preset row; they keep the sound, the position and the level ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain), Presets). New rows: G14.

- **G14 met:** every preset within 1.3 LU of the default cloud on pink and the pad (Tape stretch and Sub drone on pink, -1.28 and -1.27, the furthest; Shimmer fifths on the pad +1.07); the stutter 4.43 and 3.66 LU down, its gaps. Grit cloud was 2.2 LU down on pink at first (15 ms grains, 90 a second, half level spray: a sparse cloud keeps its grains' own levels), so it plays 20 ms grains at 100 a second with level spray 0.4 (-0.96 LU).
- **Unchanged:** every other row, bit for bit (presets are settings; the engine is the same).

### Step C1: Grains effect feedback (2026-11-18)

The Grains effect (`GrainFx`, and the `mpfree.grains` node's Feedback) writes its grains back into its buffer with the input, through a 40 Hz high-pass and a soft saturation that lets back at most half of full scale ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain)). New rows: G15.

- **G15 met:** at full feedback a loud input (pink, -14 dBFS RMS) peaks at +2.2 dBFS, and the trail doesn't grow: it falls 2.2 dB from seconds 8-12 to 16-20 and holds near -16.5 dBFS. At half feedback it falls 8.3 dB a second.
- First try: a ceiling of full scale (knee at half) let a -6 dBFS input peak at +9.6 dBFS (the input's own peaks plus a full-scale loop), so the ceiling is half of full scale.
- **Unchanged:** every other row, bit for bit (at 0 the buffer holds the input exactly; `tests/fx.rs` checks a feedback of 0 against none).

### Step C2: Grains effect repeat (2026-11-18)

The Grains effect's Repeat (`GrainFx::start_repeat`, the `mpfree.grains` node's Repeat, a choice from a half note to a 32nd) plays the last stretch of the input again and again on the beat grid, in place of the input and the cloud, from a grain voice of its own; one repeat crossfades into the next over 3 ms ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain)). New rows: G16.

- **G16 met:** the repeats sit within 0.06 dB of the stretch they repeat (quarter notes -0.02, sixteenths -0.05), and a join steps no further than the tone does (quarter notes 1.0 times, sixteenths 1.4: the fade is 3 ms, a third of the tone's period).
- First try measured the level against the pink's long-term level, which a 128 ms stretch of pink misses by up to a dB either way, and the joins by their level over the fade, which measures the pink at the slice's two ends rather than the fade; the rows now measure against the stretch itself, and the joins by their steps on a tone.
- `tests/fx.rs` checks each repeat against the input sample for sample away from the joins, the roll, the 10 ms crossfade back (the input exactly after it at Mix 0), the held memory under a repeat longer than it, and the same output at any period size; `tests/node.rs` checks the start on the grid line and Off on its frame.
- **Unchanged:** every other row, bit for bit (with Repeat off the effect runs as before).

### Step C4: a filter on each grain (2026-11-18)

Each grain can go through a low, band or high pass of its own (`mpfree_grain::filter`, a trapezoidal state-variable filter with coefficients fixed at the grain's birth), its cutoff sprayed up to three octaves either way, on granular pads and the Grains effect ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain)). New rows: G17.

- **G17 met:** two octaves past the cutoff the low pass is 24.5 dB down and the high pass 22.2 dB (the filter's own response is 24.1 and 23.6 dB down there), and two octaves inside both pass within 0.07 dB. Fully sprayed, a band pass at full resonance colours 250 Hz, 1 kHz and 4 kHz within 1.5 dB of each other (even in octaves), 16.6 LU under the open cloud (a Q of 12 passes a twelfth of each octave). A low pass at full resonance peaks 4.0 dB over the open cloud.
- First try measured the spray's spread on the default cloud, which reads one 80 ms stretch of the pink over and over; that stretch's own spectrum put 250 Hz 6 dB under the rest. The row now reads all over the pink.
- `tests/grain.rs` checks that a filtered grain still starts and ends at zero, each pass's colour, that a filter set Off (whatever its other settings) plays bit for bit as none, that full resonance on a loud source stays bounded, the spray's spread from grain to grain and that the grains read, pitch and pan as they would unfiltered; `src/filter.rs` checks each pass's response and the resonance peak; `tests/motion.rs` the LFO's sweep in octaves; `tests/node.rs` the effect's four parameters.
- **Unchanged:** every other row, bit for bit (with the filter off, a grain's reads take the loops they always did, and its cutoff draws come from a sequence of their own).
- **Cost** (instruction gate, rustc 1.97.0): a new bench case, Grains FX (dense, filtered), the dense case with a sprayed band pass on every grain, counts 629,770 instructions against the unfiltered case's 481,492 (31 % more); every other case stays within the gate's 5 %.

### Step C5: quality modes (2026-11-18)

Each voice's cloud can be coloured on its way out, after its level (`mpfree_grain::quality`, `Params::quality`): **Clean** (the default) as it is, **Tape** through a short delay moved by wow (0.6 Hz, ±0.3 % of the speed, with 0.37 Hz at ±0.1 % under it) and flutter (7 Hz, ±0.05 %), read with four-point Hermite, then a `tanh`-shaped saturation and a one-pole roll-off at 12 kHz, and **Lo-fi** held a sample at a time for a quarter of 44.1 kHz (4 frames, 11 kHz, no filter first, so the highs alias) through an 8-bit µ-law curve. On granular pads and the Grains effect (whose feedback loop takes the coloured cloud, so a Tape trail wobbles more each time round) ([sampler architecture, Granular engine](sampler-architecture.md#granular-engine-mpfree-grain)). New rows: G18.

- **G18 met:** Tape plays 0.82 LU under Clean on pink and 0.16 on the pad; Lo-fi 0.22 under on pink and 0.01 over on the pad. A held 1 kHz tone wavers 14.6 cents peak to peak in Tape (the three wobbles' ±7.8 cents at most), 0.0 in Clean. At full feedback in Tape the trail falls 3.4 dB from seconds 8-12 to 16-20 and peaks at -1.5 dBFS (Clean's: 2.2 dB, +2.2 dBFS).
- First try read the tape with a straight line between two frames: as the delay moves, that dulls the top by up to 3 dB at 10 kHz, and Tape sat 1.05 LU under Clean on pink (pink's top counts for more in loudness than the pad's). Four-point Hermite keeps it at 0.82.
- Clean is bit for bit as before: the voice skips the stage altogether (`GrainVoice::level` adds as it always did), and every older row is unchanged. A switch while the voice plays crossfades over 10 ms, running both modes (one asked for during a crossfade follows it), and a silent voice switches at once; the tape fills from its first frame when it starts again, so the delayed sound comes in without a step. The wow's place and the lo-fi's holds go by the voice's clock, so the output is the same at any period size.
- `tests/quality.rs` checks Clean as the default and a round trip through Tape coming back bit for bit 10 ms after, Tape and Lo-fi at any period size, both within 1 dB of Clean, Tape's wobble (5 to 16 cents peak to peak, Clean under 1), Lo-fi's holds of four frames and few levels near zero, no click through every switch (two inside one crossfade), a killed voice holding nothing, and the effect's Quality parameter (and a capture keeping it); `src/quality.rs` the saturation, the µ-law steps, the tape's room at every rate from 8 to 192 kHz, and the crossfade; `tests/no_alloc.rs` all three modes and the switches.
- **Unchanged:** every other row, bit for bit.
- **Cost** (instruction gate, rustc 1.97.0): two new bench cases, Grains FX (dense, tape) and (dense, lo-fi), the dense case in each mode: 501,065 and 491,230 instructions a block against Clean's 481,599 (4.0 % and 2.0 % more; Clean was 481,492 before C5). Tape adds a read, a write, a Hermite and a saturation per frame per voice, not per grain; Clean adds one check a part.
- **On the MPC (round 2):** ten device checks, one for each step from B5 part 4 to C5, are in `tools/mpc-one/app_checks.py` (the rows from `grain-stretch-hits` to `grains-capture` in [device checks](../hardware/device-checks.md)): seven on the Grains kit's pad 2 on the Grain page, three on the Grains effect on the Drums row. Each ends where it began. Two app tests make the same steps and write the same log lines: `the_grain_pages_device_checks_log_what_they_set_and_end_where_they_began` and `the_grains_effect_repeats_and_holds_its_trail_at_full_feedback_on_the_drums` (with Feedback at 100 % and the input stopped, the trail fades about 8 dB over 12 s and never rises above its first second; at Feedback 0 it is 34 dB down within 2 s and gone in 3). The same round's benchmark gives step A6: the "grain budget" line of `mpfree-bench --afford`.

### Speed: grain reads in runs (2026-11-18)

Jay picked "make grains cheaper on the MPC" after C5. A grain now asks its source for a whole part's frames at once (`GrainSource::read_run`, and `read_aimed_run` for a grain read clean), at most 64 a call, instead of one frame a call. The source checks once whether every frame the run reads is inside the sound (or, for a live buffer, inside what it holds and not across its wrap), and then reads them with no edge test; a clean run works out the octave copy, its frames' range and the sinc's reach once, and its fixed-point position from an `i32` and a `u32` (one instruction each on ARMv7) instead of from an `i64` (two library calls there). `bandlimit::read2` is now always inlined into the run that calls it. A run near a sound's edges, past 2^31 frames, or from a source without its own takes the old way, frame by frame.

- **Unchanged, bit for bit:** every render. Hashes of 37 renders were the same before and after (granular pads with 1 and 2 channels, floats and 16 bits, forward, reversed, pitched up past two semitones and filtered, in 128- and 37-frame blocks; the Grains effect at Feedback 0 and 60; a clip with 1 and 2 channels), and so is every lab row.
- **On the MPC's processor** (ARMv7 instructions under qemu, the grain engine's own code, over 10 blocks of each bench case): the 4 granular pads 112,518 to 91,868 instructions a block (18 % fewer), the Grains effect (dense) 337,051 to 279,994 (17 % fewer). Before, a clean read was 56k of the pads' 113k, the voice's loop 38k, the call per frame 10k and the 64-bit conversions' library calls 2k. Moving the runs without always inlining `read2` gave the pads 12 %.
- **Instruction gate** (x86, rustc 1.97.0): Sampler, the 4 granular pads alone 632,178 to 601,018 (-4.9 %), Sampler, 20 pads 1,452,847 to 1,415,256 (-2.6 %), 20 pads + master grains 2,045,946 to 2,001,054 (-2.2 %), Grains FX (dense) 481,599 to 475,712 (-1.2 %), its tape and lo-fi cases -1.2 % each, filtered 629,813 to 635,281 (+0.9 %). x86 converts 64-bit integers in one instruction, so it gains less.
- **What it doesn't change:** the clean read's own arithmetic (16 to 32 taps a frame) is now most of a clean grain's cost and near its floor without NEON. Cheaper still means reading differently (fewer taps, clean only further up than two semitones, octave copies sooner), which changes the sound: Jay's call (A4 kept clean grains at their cost). Device round 2 measures the real time on the MPC.

## Round 5: movement, pitch and voice

Measuring built 2026-11-17, by `quality/round5.rs`: the movement effects (class `movement`: the shared LFO heard through the tremolo, the Tremolo, Auto-pan, Rotary and Auto wah, ids W; the Ring mod and Frequency shifter, Z), the pitch effects (`pitch`: the Pitch shifter, Pitch correct, Harmonizer and Tuner, H) and the Talk box (`voice`, V). Research and plan: jferments/mpfree-claude `fx/quality/` (research-movement.md, research-pitch.md, research-voice.md, plan-5.md, baseline-r5-2026-11-17.md). The three classes take about 8 s together. Subjects are `class/engine` at its defaults and presets (`movement/rotary[Chorale]`, `pitch/pitch-correct`), and `movement/lfo` for the LFO (the tremolo at Depth 100). Clicks are round 3's M13 (each control swept end to end in 50 ms and back under a 997 Hz tone at -12 dBFS, the loudest millisecond above 6 kHz against the tone) for every control of every effect, under the id named below; the controls that move the sound past 6 kHz by design (the ring mod's Freq and Sweep, the frequency shifter's Shift and Feedback, the pitch shifter's Feedback, the rotary's Drive) are left out.

The measures it adds, beside the earlier rounds':

| Measure | What it reads |
|---|---|
| The LFO as heard | The tremolo's gain frame by frame from a steady input (Volume mode, Width 0, Depth 100), every shape at 0.5, 4 and 20 Hz, at the report's rate and again at 48 kHz (where a phase lands near a wrap depends on the rate): the largest change in one frame, and a Square's 10 to 90 % rise |
| Steady clicks | A 997 Hz tone at -12 dBFS through the effect for 3 s with nothing moving: the loudest millisecond above 6 kHz against the tone (the LFO's own resets and edges) |
| Sync after a wrap | The song position looping from 3.5 bars back to the start at 120 BPM, the LFO at 1 bar: the phase of the gain against the song position over the last bar before the wrap, against the same over the bar from 50 ms after it, in cycles |
| A rotor | The analytic signal of one test tone through the rotary (the drum's 200 Hz and the horn's 1.5, 3 and 6 kHz at -18 dBFS, together): the tone's level swings once a rotation, so the swing's own analytic phase gives the rotor's speed at every moment, and which microphone hears it first; the tone's instantaneous frequency gives the Doppler |
| Off the true products | A sine through the ring mod or the frequency shifter (Mix 100), Hann-windowed: the power more than 30 Hz (the shifter: 6 Hz) from every product it should make, against the products' |
| A shifted tone | 1.5 s after half a second to settle: the power more than 2 % from the target against the target's; its pitch from the upward zero crossings, each placed between frames; its level per whole period of the target (at least 5 ms) |
| A voice's pitch | The instantaneous frequency of the corrected or harmonized fundamental (fxlab's `flutter`), in cents against the note; averaged over 10 ms where a settling time or a vibrato is read, so a single-frame grain glitch doesn't count as unsettled |
| Off the harmonics | A saw's spectrum: the power more than 1 % from every harmonic of the wanted pitch, against the whole |
| Meters | Pitch correct's note and the tuner's cents and frequency, read after every 128-frame block |
| A vowel | The talk box's impulse response (65,536 points): its peaks near each formant of Peterson and Barney's table, and the deepest dip between F1 and F2 under F2 against an all-pole cascade with the same formants (bandwidths 80, 100 and 140 Hz); on a 110 Hz saw, the harmonics nearest F2 and F3 under the one nearest F1, against the table's levels |

### The LFO, tremolo and auto-pan (W1 to W8)

| Id | Row | Target | Why |
|---|---|---|---|
| W1 | `jump_<shape>_<hz>hz`, `click_db_<shape>_<hz>hz` (`_48k` at 48 kHz) | The largest one-frame change ≤ 0.05 of the range; ≤ -60 dB | A reset in one frame clicks every cycle |
| W2 | `edge_ms_<hz>hz` | Square: 1 to 4 ms at 0.5, 4 and 20 Hz | A chop at any rate, and never a click |
| W3 | `sync_error_cycles_tremolo`, `_autopan` | Within 0.01 of a cycle, 50 ms after a loop wrap | As M11 |
| W4 | `click_db_<control>` (tremolo and auto-pan) | ≤ -60 dB, Shape included | Switching is a performance move |
| W5 | `depth_error_db_<depth>`, `trough_db_depth100` | Depth d swings 20·log10(1/(1 - d)) dB within 0.2 dB; ≤ -60 dB at 100 (under -80 reads -80) | The knob says what it does |
| W6 | `harmonic_whole_swing_db`, `harmonic_3khz_swing_db` | Harmonic mode on a 200 Hz and a 3 kHz tone: the whole ≤ 4.5 dB while the 3 kHz tone swings ≥ 20 dB | The swirl without a pump |
| W7 | `click_db_mode` | ≤ -60 dB | As W4 |
| W8 | `power_swing_db` | Auto-pan at 1 Hz on mono pink: left plus right power within 0.25 dB | A pan that pumps is a tremolo |

The tremolo goes in quieter by what its depth takes, and the auto-pan is a balance on a stereo source; both are by design (plan-5 section 5).

### Rotary (W9 to W14)

| Id | Row | Target | Why |
|---|---|---|---|
| W9 | `horn_hz_<speed>`, `drum_hz_<speed>`, `horn_doppler_cents`, `drum_doppler_cents` | Horn 0.8 and 6.7 Hz, drum 0.67 and 5.7 Hz, within 2 %; at Fast the horn ±30 to 45 cents, the drum ±5 to 10 | These numbers are the sound |
| W10 | `ramp_up_s_<rotor>`, `ramp_down_over_up_<rotor>`, `click_db_<control>` | Slow to Fast at the default Ramp, until within 10 % of Fast for good: the horn in 0.8 to 1.6 s, the drum in 3.5 to 6 s; slowing down (to within 10 % of the span of Slow) no quicker; every control but Drive ≤ -60 dB | The ramp is played |
| W11 | `lu_<pink or hits>_<speed>`, the presets' `lu_pink` | Within ±1.5 LU | Louder wins every A/B |
| W12 | `beam_db` | At Fast, the horn's level swing on 6 kHz at least 4 dB more than on 1.5 kHz | A horn's beam narrows with frequency |
| W13 | `alias_db_5khz_drive<n>` | Stopped, 5 kHz at -6 dBFS: ≤ -60 dB at Drive 50, ≤ -50 dB at 100 | Inharmonic fizz (round 2's G1 and G2) |
| W14 | `opposite_rotation` | 1: the microphone that hears the horn first hears the drum last | The Leslie's rotors turn opposite ways |

### Auto wah (W15 to W18)

| Id | Row | Target | Why |
|---|---|---|---|
| W15 | `peak_error_pct_res<n>` | Source Pedal: the peak within 3 % of Low·(High/Low)^Center at Center 0, 50 and 100, Resonance 0, 60 and 100 | The page is how it's set |
| W16 | `lu_pink` | Within ±2 LU, the defaults and every preset | As W11 |
| W17 | `lu_vs_res60_<wah or band-pass>_res<n>` | Within ±3 LU of the level at Resonance 60, from 0 to 100 | A sharper peak shouldn't be a quieter effect |
| W18 | `click_db_<control>` | ≤ -60 dB, the Filter type included | As W4 |

### Ring mod and frequency shifter (Z1 to Z6)

| Id | Row | Target | Why |
|---|---|---|---|
| Z1 | `image_db_<44k1 or 48k>_shift<hz>` | The wrong-way shift ≤ -60 dB on tones from 40 Hz to 18 kHz, shifts of 100 Hz and 1 kHz, at 44.1 and 48 kHz | The image is a second, opposite shift |
| Z2 | `spurs_db_sine` (ring mod, a 7 kHz tone on a 3 kHz sine), `spurs_db_<rate>_shift<hz>` | ≤ -80 dB | An approximate sine adds products |
| Z3 | `alias_db_triangle`, `alias_db_square`, `alias_db_sine_top_octave` | 440 Hz carriers on a 1 kHz tone ≤ -60 dB; a 5 kHz sine on a tone at 0.95 of Nyquist reported | Inharmonic fizz that moves the wrong way in a sweep |
| Z4 | `wave_spread_lu` | The three waves within 1.5 LU of each other on pink | A choice of tone shouldn't be a jump in level |
| Z5 | `wave_click_db_<from>-to-<to>`, `click_db_<control>` | The ring mod's Wave: round 2's switch click ≤ 3 dB (a square carrier's products pass 6 kHz, so M13 can't read it); the shifter's Mode and the rest ≤ -60 dB | As W4 |
| Z6 | `feedback95_ringout_db` | 95 % feedback, 4 s of white noise at shifts 0.01, 5, 100 and 2000 Hz: finite, and the third second after it 60 dB under the level while it played (the peak reported) | No runaway |

### Pitch shifter, pitch correct, harmonizer and tuner (H1 to H14)

| Id | Row | Target | Why |
|---|---|---|---|
| H1 | `off_target_db_<shift>`, `pitch_error_cents_<shift>`, `level_swing_db_<shift>` | 13 sines from 110 Hz to 1.8 kHz at +12, +7, -5 and -12 semitones, the default window: every tone ≤ -30 dB off target, within 5 cents, its level swing ≤ 3 dB (the median off target reported) | One note clean and the next gone is the classic delay-line flaw |
| H2 | `chord_off_target_db` | A3, C#4 and E4 saws up an octave ≤ -15 dB off the target partials | Polyphonic material is the hard case |
| H3 | `impulse_copies_up12`, `impulse_spread_ms_up12` | Reported: an impulse's copies over -12 dB at +12, and their spread | A hit becomes a flam |
| H4 | `delay_ms`, `reported_latency_frames` | At 0 semitones and the default window: ≤ 15 ms, reported 0 (plan-5 decision 12.2: immediate, shortened); Mix 50's comb reported | Mixed with the dry, a 25 ms lag is a slapback |
| H5 | `hf_tilt_error_db_up1` | A semitone up on pink: 12 to 16 kHz against 1 to 2 kHz within 0.5 dB of the input's | Interpolation dulls a moving read |
| H6 | `accuracy_cents_<range>`, `wobble_cents_<range>`, the harmonizer's `interval_error_cents_<voice>` | Key C major, Speed 0: saws in each range's span (Low 60 to 600 Hz, Mid 90 to 900, High 150 to 1500) within 3 cents of the nearest scale note on average, wobble ≤ 2 cents; voices within 3 cents | Heard against an in-tune keyboard |
| H7 | `vibrato_left_cents_speed0`, `vibrato_kept_speed100` | A ±40-cent vibrato at 5.5 Hz on A4: ≤ 5 cents left at Speed 0; ≥ 90 % kept at 100 | Speed 0 steps flat between notes |
| H8 | `settle_ms_speed<n>` | A saw 30 cents sharp from silence: within 5 cents for good in at most 1.8 × Speed + 25 ms after the latency | Speed means its label |
| H9 | `in_tune_off_harmonic_over_input_db`, `correcting_off_harmonic_db`, the harmonizer's `voice_off_harmonic_db_<voice>` | In tune (220 Hz): within 3 dB of the input's own; 45 cents sharp: ≤ -50 dB; each voice ≤ -40 dB | Rough grains are heard as buzz |
| H10 | `formant_<f1 or f2>_error_harmonics_<voice>` | An "AH" sung at 130 Hz: F1 and F2 within one harmonic of the input's after an octave up or down or a 5th up | Without it a 5th up is a chipmunk |
| H11 | `noise_change_db`, `wrong_notes` | White noise within 0.5 dB; the note meter right on a sine, saw, square and missing fundamental at A2, A3 and A4 | Breaths and "s" survive; no octave errors |
| H12 | `latency_error_frames_<range>`, the harmonizer's `latency_error_frames` | The delay of pink (which passes uncorrected) within 1 frame of the reported latency | The mixer lines slots up by it |
| H13 | `click_db_<control>` | ≤ -60 dB, every control of the four | As W4 |
| H14 | `error_cents_<signal>`, `settle_ms_<signal>`, `jitter_cents_<signal>` | Twelve notes from A0 to G6, 13 cents sharp, as a sine, a saw and a saw with white noise 30 dB down: within 0.5 cent after 1 s, within 2 cents in ≤ 100 ms, jitter ≤ 1 cent | Pedal tuners quote ±1 cent |

### Talk box (V1 to V5) and the Vocoder (V6)

| Id | Row | Target | Why |
|---|---|---|---|
| V1 | `formant_error_pct_<vowel>` | Every formant within 3 % of the table | The vowel is recognised from F1 and F2 |
| V2 | `dip_excess_db_<vowel>` | The dip between F1 and F2 within 3 dB of the cascade's | A mouth has no zeros between formants |
| V3 | `f2_level_error_db_<vowel>`, `f3_level_error_db_<vowel>` | Within 4 dB of the table's levels under F1 (Peterson and Barney 1952, table II, the averages over all speakers; checked against the paper for this PR) | The levels make it a mouth |
| V4 | `vowel_spread_lu`, `lu_pink` | ≤ 4 LU across the nine vowels on the saw; within ±2 LU on pink, the defaults and every preset | A vowel sweep shouldn't pump |
| V5 | `click_db_<control>` | ≤ -60 dB | It is made to be moved |
| V6 | `latency_frames`, `lu_carrier_<vowel>`, `f1_error_bands_<vowel>`, `f2_error_bands_<vowel>`, `lu_pink`, `s_level_db`, `s_level_db_keyed`, `click_db_<control>` | Latency 0; within ±2 LU of the carrier at the defaults (keyed: a 110 Hz saw through the input and each sung vowel at 220 Hz on the key, both at -18 dBFS RMS; and the defaults and presets on pink at the track); a sung vowel's F1 and F2 on a saw carrier within one band of the voice's (where the output's gain over the saw peaks against where the voice is loudest, on the saw's harmonics); an "s" (noise above 4 kHz) no more than 6 dB under the voice's level at the default Unvoiced, on the track and on the key; clicks ≤ -60 dB (the 997 Hz tone as voice and carrier both, every control but Carrier) | Intelligible, level, playable live. Its rows came with the Vocoder (PR 8, plan-5 decision 12.1) |

The cost of each is in the instruction gate, at its defaults (new cases, instructions a 128-frame block): Tremolo 16.7k, Auto-pan 22.0k, Ring mod 24.6k, Auto wah 28.5k, Tuner 32.2k, Talk box 37.3k, Frequency shifter 55.3k, Pitch shifter 59.0k, Pitch correct 60.1k, Harmonizer 99.3k and Rotary 115.7k.

### Baseline (2026-11-17, before any fix)

183 of 307 targets met: movement 101 of 171, pitch 53 of 82, voice 29 of 54. These are at 44.1 kHz; plan-5's numbers were read at 48 kHz, and where the two differ it is said.

- **LFO:** Sine, Triangle and Square meet W1 at every rate. The Ramps jump the whole range in one frame (-18 dB at 4 and 20 Hz, -31 at 0.5 Hz). Random is clean at 44.1 kHz, but at 48 kHz it jumps 0.60 and 0.91 of the range at 4 and 20 Hz (-20 and -17 dB): its phase rounds to 1.0 at the wrap on some rates and not others. A Square's edge takes 85.8 ms at 0.5 Hz and 10.7 ms at 4 Hz (2.2 ms at 20). Synced at a bar, the tremolo and auto-pan are half a cycle off after a 3.5-bar loop wraps.
- **Tremolo and auto-pan:** Depth exact and a full cut at 100; Harmonic mode met (the whole 4.0 dB, the 3 kHz tone 33.1 dB); the auto-pan's power within 0.01 dB. Shape clicks -28.8 dB (auto-pan -21.4) and Mode -22.8 dB.
- **Rotary:** the speeds within 0.1 % and the Doppler ±36.8 and ±8.1 cents; the horn ramps up in 0.82 s but the drum in 3.28 s (plan-5's 4.5 s was read one rotation at a time; the speed at every moment shows it gets there sooner), and both slow down 1.04 to 1.05 times as long; 3.2 LU quieter at Slow and Fast, on pink and hits (Gritty organ +0.2); the 6 kHz swing is 1.7 dB less than the 1.5 kHz one, so no beam; the rotors turn the same way; the drive folds back at -30.8 dB (Drive 50) and -13.3 dB (100).
- **Auto wah:** the peak within 0.6 % at Resonance 60 and 100, 3.02 % at 0; 5.7 LU quieter at the defaults (the presets -7.0 to +0.1); Band-pass 3.6 LU louder at Resonance 0 than at 60; Filter clicks -8.4 dB, Resonance -42.3, Source -54.1.
- **Ring mod and frequency shifter:** the ring mod's sine leaves spurs at -62.2 dB; a triangle carrier aliases at -62.5 dB (met), a square at -36.8; the sine's top-octave fold is as loud as the products; the waves are 5.0 LU apart; Sine to Triangle switches with a 5.8 dB spike. The shifter's image is -44.4 dB at 44.1 kHz and -44.3 at 48, at both shifts, and its spurs -59.8 to -60.8; 95 % feedback rings out (its peak -13.2 dBFS); Mode clicks -13.4 dB.
- **Pitch shifter:** the worst of the 13 tones is 68.7 dB off target at +12 and 64.8 at -12 (+7: -2.5, -5: -3.5; the medians -43.4, -38.7, -39.0 and -31.1); the pitch 53.9, 36.2, 37.5 and 114.4 cents off at worst; levels swing 11.7 to 12.9 dB; the chord -5.8 dB; an impulse makes 2 copies 12.5 ms apart; the delay 25.0 ms, reported 0 (Mix 50 combs 57.7 dB deep); the highs 2.3 dB dull; Window clicks -57.5 dB.
- **Pitch correct:** at worst 4.1 cents off the note on Low, 3.3 on Mid and 11.4 on High (plan-5's 7.9 was at 48 kHz), wobbling 2.2, 1.0 and 2.7 cents; Speed 0 leaves 16.7 cents of the vibrato (Speed 100 keeps all of it); it settles in 7, 25, 69, 145 and 289 ms at Speeds 0, 20, 50, 100 and 200, all within their labels (plan-5's 487 ms at 200 was a single-frame glitch read as unsettled); in tune it puts 20.0 dB more energy off the harmonics than the input has, and correcting 45 cents -33.9 dB; noise passes, the notes are right and the latency is exact; no control clicks.
- **Harmonizer:** intervals within 0.06 cent and formants kept within a harmonic; each voice -26.0 to -27.6 dB off its harmonics; latency exact; Range clicks -19.5 dB and Voice 1 -57.1.
- **Tuner:** within 0.34 cent, within 2 cents in 81 ms, jitter 0.49 cent with the noise. Met.
- **Talk box:** formants within 1.7 %; the dips between F1 and F2 7.1 to 16.3 dB deeper than a cascade's ("AE" the deepest); levels off the table by 10.1 dB ("EE" F2), 7.3 ("OU" F2), 5.8 ("EE" F3), 5.1 ("AE" F2) and 4.7 ("OO" F3); 6.8 LU between the vowels on the saw; 6.4 LU quieter on pink (the presets 4.9 to 8.2); Vowel clicks -38.1 dB, LFO depth -40.8, Formant -42.8, Env depth -43.4 and Resonance -51.0.

### PR W: the LFO, tremolo and auto-pan

What changed (design: [fx-movement.md](fx-movement.md), [fx.md](fx.md)): the shared LFO (`kit::Lfo`) is rebuilt (plan-5 sections 4 and 5). A Square's edges and a Ramp's return take 4 ms (10 to 90 % in 2.36 ms), at most an eighth of a cycle; Random glides through values worked out a cycle ahead, so a Width offset passes through the same ones, and its phase can no longer round into the next cycle; synced, the LFO follows the song position while it plays; and any jump (a Shape or Sync change, a loop wrap, a locate) puts the LFO where it should be at once and glides its output there over 20 ms. The tremolo's Mode crossfades between Volume and Harmonic over 20 ms. After it: movement 136 of 171 met (101 before); every LFO, tremolo and auto-pan row met.

- **Met now:** Ramp up and Ramp down move at most 0.0076 to 0.0088 of the range in a frame at 0.5, 4 and 20 Hz, at 44.1 and 48 kHz (W1; 1.00), and click at -86.1 to -102.4 dB (-18.0 to -31.4); Random at most 0.0006 (0.60 and 0.91 at 4 and 20 Hz at 48 kHz) and -102.2 dB or under (-20 and -17); the Square's edge 2.36 ms at every rate (W2; 85.8, 10.7 and 2.2); synced, the tremolo and the auto-pan are 0.000 of a cycle off the song after a 3.5-bar loop wraps (W3; 0.5); Shape changes -86.5 dB on the tremolo and -78.7 on the auto-pan (W4; -28.8 and -21.4); Mode -83.7 dB (W7; -22.8).
- **Blessed worse, inside the targets:** a Square's largest step is now 0.0081 to 0.0089 of the range at 0.5 and 4 Hz (W1; 0.0002 and 0.0017), and its click at 0.5 Hz -98.4 dB (-102.5): its edge is 2.36 ms at any rate instead of a share of the cycle, which is the point. A Sync change clicks at -84.3 dB on the tremolo and -75.9 on the auto-pan (W4; -98.0 and -80.3): it now puts the LFO on the song at once and glides there, where before it only changed speed.
- **Not in this PR:** the auto wah, ring mod and talk box use the same LFO and get its fixes, but their own rows move with their PRs.
- **Cost** (the instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz): Tremolo 16.7k to 11.3k (-32.1 %), Auto-pan 22.0k to 21.3k (-2.9 %), Ring mod 24.6k to 20.3k (-17.6 %), Auto wah 28.5k to 25.5k (-10.7 %), Talk box 37.3k to 30.2k (-18.8 %). The glide after a jump and the wrap into a new cycle are out of line, so a frame with neither costs one compare for both; the Sine is tested first; the tremolo works out one value a frame while its sides move together in Volume mode (Width 0, the default); the auto wah and the talk box step the LFO every frame but read it only when they retune, every 16 frames; the ring mod reads it only while Sweep is up. The first version, with the glide in line and the shape matched for every value, cost the tremolo 45 % more.

### PR 2: the rotary

What changed (design: [fx-movement.md](fx-movement.md), [fx.md](fx.md)): the rotary (plan-5 section 6). The drum turns the other way from the horn; the horn's treble beams (a one-pole low-pass at 2 kHz a microphone, crossfaded to the whole horn as it faces the microphone); Ramp is now the drum's time from Slow to within 10 % of Fast, the horn takes about a quarter of it, and both coast down a quarter more slowly than they speed up; the horn and the drum are set 1.07 and 1.28 times up, so it goes in at the dry's level; Drive runs 2x oversampled with first-order ADAA (round 2's way), its level held, while the plain path waits the same 4 frames so switching it never moves the sound in time. Saved rotaries keep their controls and change sound this way. After it: movement 148 of 171 met (136 after PR W); every rotary row met.

- **Met now:** the horn ramps up in 1.14 s and the drum in 4.66 s (W10; 0.82 and 3.28), and they take 1.42 and 1.25 times as long to slow down (1.05 and 1.04); within 0.3 LU of the dry on pink and hits at Slow and Fast, and every preset within 0.2 (W11; -3.2, and Gritty organ +0.2); the 6 kHz swing 5.3 dB more than the 1.5 kHz one (W12; 1.7 dB less); Drive folds back at -96.3 dB (Drive 50) and -55.6 dB (100) (W13; -30.8 and -13.3); the rotors turn opposite ways (W14; the same way). The horn's Doppler is ±37.6 cents (±36.8).
- **Blessed worse, inside the targets:** the speeds read through the level swing, which the beam reshapes: Slow 0.796 and 0.668 Hz (W9; 0.797 and 0.669), Fast 6.699 Hz (6.700), the rotors themselves unchanged; the drum's Doppler ±8.3 cents (±8.1). The clicks of Speed, Balance, Width and Mix at -91.1, -93.0, -91.2 and -104.3 dB (W10; -99.3, -98.1, -100.9 and -105.1), all far under -60: a louder cabinet with more movement in it.
- **Cost** (the instruction gate): 115.7k to 108.2k (-6.5 %). Each rotor's angle at each microphone comes from the rotor's and the spread's sines by the angle sums, six sines a frame instead of eight, more than paying for the beam's two one-poles and the drive's delay.

### PR 3: the auto wah

What changed (design: [fx-movement.md](fx-movement.md), [fx.md](fx.md)): the auto wah (plan-5 section 7). The band's gain is 1.95·√Q, since a band-pass passes about 1/Q of a broad sound's power, so the effect keeps its level as Resonance sharpens the peak and the defaults go in at the dry's level; the filter still retunes every 16 frames, but its coefficients and the gains they scale glide in a straight line between retunes; Resonance, Low and High glide over 20 ms; a Filter change crossfades the shapes (each shape's share through two 2.5 ms one-poles, so a change mid-way carries on from where it is); a Source change glides the sweep from where it was over 30 ms. Saved auto wahs keep their controls; in the Wah and Band-pass shapes they are about 5 LU louder, at the dry's level. After it: movement 158 of 171 met (148 after PR 2); every auto wah row met.

- **Met now:** the defaults within 0.2 LU of the dry on pink, the presets within 1.4 (W16; -5.7, and -7.0 to -4.4 in four presets); Band-pass within 0.7 LU of its level at Resonance 60 from 0 to 100 (W17; +3.6 at 0); the peak within 1.2 % of where it is set (W15; 3.0 % at Resonance 0); the Filter switch at -87.2 dB, Resonance -97.1, Source -89.0, and every other control -85 dB or under (W18; -8.4, -42.3 and -54.1).
- **Blessed worse, inside the targets:** the Wah shape at Resonance 0 and 30 is 1.1 and 0.5 LU under its level at 60 (W17; +0.5 and +0.1), since its leak doesn't grow with the band. Rate and Sync, which the default Envelope source doesn't use, read -89.0 dB (W18; -101.2), Release -86.6 (-86.8) and Mix -105.3 (-105.8): what they measure is the steady sound's own content above 6 kHz under the 997 Hz tone, which follows the band's gain (half the gain reads -97.2).
- **Cost** (the instruction gate): 25.5k after PR W to 26.0k (+2.1 %; 28.5k before round 5): the ramps of the coefficients and gains, five adds a frame. The shares' crossfade runs only while a Filter change does.

### PR 4: the ring mod and frequency shifter

What changed (design: [fx-movement.md](fx-movement.md), [fx.md](fx.md)): the ring mod and the frequency shifter (plan-5 section 8). The ring mod's Sine is a fitted polynomial of a double-precision phase; Triangle and Square are worked out at twice the rate with their corners and steps smoothed by a cubic B-spline across four frames, and the product runs at twice the rate between half-bands; the three waves are at the sine's RMS; a Wave change crossfades (shares through two 2.5 ms one-poles). The shifter's 90° pair is Valenzuela and Constantinides' elliptic design with twelve coefficients for the rate (from 35 Hz), its phasor the same fine sine and cosine; a Mode change crossfades; the feedback loop is kept between 20 Hz and 18 kHz. Saved ring mods with Triangle come out about 2 LU louder and with Square about 3 LU quieter, at the Sine's level. After it: movement 171 of 171 met (158 after PR 3).

- **Met now:** the ring mod's sine spurs at -100.6 dB (Z2; -62.2), a triangle's aliasing at -100.0 and a square's at -68.2 (Z3; -62.5 and -36.8), the waves within 0.25 LU (Z4; 5.0), a Wave change 0.0 to 0.1 dB over the steady level (Z5; up to 5.8); the shifter's image at -65.8 to -67.1 dB at 44.1 and 48 kHz (Z1; -44.3 and -44.4), its spurs at -90.2 to -99.2 (Z2; -59.8 to -60.8), a Mode change at -92.6 dB (Z5; -13.4). Every other ring mod and shifter control -103 dB or under; 95 % Feedback still rings out (Z6; -140 dB).
- **The spur measure moved:** the shifter's spurs are now the power beyond ±30 Hz of the shifted tone and its image (as the ring mod's), not ±6 Hz: within ±6 Hz is still the Hann window's own skirt, -67 to -80 dB, which read as spurs even in double precision. Measured the new way, the old network and phasor left spurs at -62.1 to -62.2 dB (the old phasor's own).
- **Blessed worse, inside the targets:** the Triangle to Square change reads 0.10 dB over the steady level (Z5; 0.01): the two now cross at one level, where the Triangle was about 5 dB quieter before.
- **Cost** (the instruction gate): the ring mod 20.3k after PR W to 12.9k (-36.3 %; 24.6k before round 5): with the Sine alone nothing runs at twice the rate, and the phase step is worked out once while the pitch is still. The shifter 55.3k to 53.4k (-3.4 %): twelve coefficients and the fine phasor cost more, but the loop is skipped with no Feedback and the phasor's step is worked out once while the Shift is still. Triangle and Square cost more while heard (three nine-section half-bands at twice the rate); the gate measures the defaults.

### PR 5: the pitch shifter

What changed (design: [fx-pitch.md](fx-pitch.md), [fx.md](fx.md)): the pitch shifter (plan-5 section 9, with decision 12.2). It is now a WSOLA shifter: one read a side moves through the line at `1 - ratio` a frame, and each splice jumps to where the sound lines up with what the read just played (normalised correlation on a copy at an eighth of the rate over the last 8 ms, then frame by frame over 4 ms, then between frames by a parabola), the old read fading out in a smoothstep of at most 6 ms whose gain comes from how well they matched. Reads are a twelve-tap Kaiser-windowed sinc (images 76 dB down); going down, a low-pass at the shifted band's top fades in. The Window's default is 30 ms (was 50), so the shifted sound lags about 11 ms; saved shifters keep their Window. After it: pitch 69 of 82 met (53 before); every pitch shifter row met.

- **Met now:** the worst of the 13 tones -61.0, -56.6, -43.1 and -39.7 dB off target at +12, +7, -5 and -12 (H1; +68.7, -2.5, -3.5 and +64.8), within 0.011 cents (53.9 to 114.4), level swings 0.02 to 0.03 dB (11.7 to 12.9); the chord -21.6 dB (H2; -5.8); the delay 11.4 ms (H4; 25.0); the highs 0.05 dB off the input's tilt (H5; -2.3); Window's click -89.4 dB (H13; -57.5), Pitch's -68.9 (-60.2), Fine's -74.6 (-64.4).
- **Reported:** an impulse at +12 makes 3 copies over 15.0 ms (H3; 2 over 12.5): a hit near a splice is heard where the old read fades out as well as where the new one starts. At Mix 50 and 0 semitones the comb with the dry is 74.2 dB deep (57.7), its teeth twice as far apart with the shorter delay.
- **Cost** (the instruction gate): 59.0k to 72.0k (+22.0 %), blessed: about half is the twelve-tap read, the rest the search (about 45 times a second a voice an octave up). Its sums are four lanes wide, summed in order: summed in pairs, the compiler built the lanes two wide, and with both of a search's sums in one loop it paired each sample's two products (the first version cost 46 % more).

### PR 6: pitch correct and the harmonizer

What changed (design: [fx-pitch.md](fx-pitch.md), [fx.md](fx.md)): pitch correct and the harmonizer (plan-5 section 9, with decision 12.3). The period is found again at the full rate on the low-passed middle, to a hundredth of a frame; each look is stamped at the middle of what it heard, and the note is read where the grains read; marks sit between frames, and every grain reads the input at its own fraction through the pitch shifter's twelve-tap sinc (a whole frame read as itself); the grain windows are smootherstep; near ratio 1 a grain eases back to its mark by at most 0.1 % of a period, fading in between 0.1 % and 0.03 %; Speed is a one-pole of Speed ms; a Range change dips the output 5 ms either side; each harmony voice passes a DC blocker at 5 Hz. New controls: **Flex-Tune** (corrects only notes sung near the note) and **Humanize** (slows the correction on held notes), both 0 by default, ids 8 and 9; the Natural preset uses them (Speed 30, Flex-Tune 10, Humanize 60; was Speed 60, Amount 80), and A minor snap's Speed is 2 (was 5), so it snaps as close as it did. After it: pitch 84 of 84 met (two rows new: Flex-Tune's and Humanize's clicks).

- **Met now:** pitch correct within 0.010, 0.005 and 0.044 cents on Low, Mid and High (H6; 4.07, 3.35 and 11.43), wobbling 0.039, 0.049 and 0.075 (2.18, 1.00 and 2.68); Speed 0 leaves 2.96 cents of the vibrato (H7; 16.73); in tune exactly as rough as the input (H9; 19.95 dB rougher), correcting 45 cents -62.9 dB (-33.9); Speed 0 settles in 4.2 ms (H8; 7.3). The harmonizer's voices off their harmonics at -61.4, -56.0 and -59.3 dB for a 3rd up, an octave up and a 3rd down (H9; -26.9, -27.6 and -26.0), its Range click -68.5 dB (H13; -19.5), its Voice 1 click -62.5 (-57.1). Every pitch correct control -85 dB or under.
- **The pitch meter moved:** H6 to H8 read the pitch from a soft band around the note it should be (`tone_cents`: full within 15 %, a cosine to nothing at 30 %), not a brick-wall band, whose ringing read 2 to 3 cents of wobble on a plain saw at 70 to 100 Hz; the new meter reads 0.02 to 0.4 cents there. The rotary's Doppler keeps the old meter.
- **Blessed worse, inside the targets:** Speed 100 keeps 0.975 of the vibrato (H7; 1.150: the old correction overshot and widened it). Settling at Speed 20, 50, 100 and 200 takes 40.2, 94.0, 183.6 and 362.4 ms (H8; 24.5, 68.7, 145.0 and 289.3; targets 61, 115, 205 and 385): Speed now means a one-pole of Speed ms, where it ran faster than its label. The harmonizer's Level 1, Level 2 and Dry clicks read -63.3, -65.0 and -65.0 dB (-67.3, -65.4 and -65.5), all under -60; its 3rd down is 0.0024 cents off (0.0023).
- **Not as the plan said:** a Range change dips the output for 5 ms either side instead of crossfading two trackers: it costs nothing while Range is still, and its click is -85.5 dB on pitch correct and -68.5 on the harmonizer.
- **Cost** (the instruction gate): pitch correct 60.1k to 74.4k (+23.8 %), blessed: the twelve-tap reads (stereo, two grains a frame where they overlap), the period found again every look, and the looks lined up with the grains. The harmonizer 99.3k to 99.1k. The first version cost 60 % and 45 % more, until grains were laid 128 frames at a time, four frames at a time through the taps, the low-passed copy was written twice instead of copied for each refinement, each ratio was worked out once a change, and the read hint went through 32 bits (a float to a 64-bit integer is a library call on ARM).


### PR 7: the talk box

What changed (design: [fx-voice.md](fx-voice.md), [fx.md](fx.md)): the talk box (plan-5 section 11). Its three band-passes are summed with alternating signs (F1 +, F2 -, F3 +, Klatt's parallel bank), each normalised to unity at its peak and scaled by a level fitted so a 110 Hz saw comes out at Peterson and Barney's sung levels (the prototype, mpfree-claude `fx/quality/tools/talkbox_proto.py`, also tried an all-pole cascade: it meets V2 but leaves F3 5 to 20 dB weak on the saw, with or without a fixed F4 and F5); each vowel has its own make-up gain (11.3 to 16.2 dB, from its loudness halfway between pink and the saw), glided with the vowel, in place of one fixed 9.5 dB; the gains rise with the square root of Resonance's sharpness; the bandwidths scale with Formant, so each formant's Q stays as Resonance sets it; the coefficients and the three output gains glide between retunes, as the auto wah's do since PR 3; Vowel, Formant and Resonance glide over 20 ms. Saved talk boxes keep their controls; they go in at the dry's level, about 6 LU louder than they did (5 to 8 LU in the presets). After it: voice 54 of 54 met (29 before).

- **Met now:** the dips between F1 and F2 at most 1.7 dB deeper than the cascade's (V2; 7.1 to 16.3), "IH" 6.4 dB shallower; F2 and F3 within 0.005 dB of the sung levels on the saw (V3; up to 10.1); 2.0 LU between the vowels on the saw (V4; 6.8), the defaults +0.6 LU on pink and the presets -0.8 to +0.3 (V4; -6.4, and -4.9 to -8.2); Vowel's click -69.4 dB, LFO depth -67.0, Env depth -72.2, Resonance -79.0 and Formant -105.4 (V5; -38.1, -40.8, -43.4, -51.0 and -42.8).
- **Blessed worse, inside the targets:** the controls that move nothing at the defaults (Attack, Release, Sensitivity, LFO rate, LFO sync and LFO shape, with both depths at 0) read -79.1 dB (V5; -103.5), and Mix -100.8 (-105.8): what they measure is the floor under the 997 Hz tone, which follows the tone's level through the effect. The tone sits between AH's F1 and F2, where the old bank's notch took it about 6 dB down; it now passes about 10 dB up. A steady tone through the new bank has no sidebands at the retune rate (under -150 dB).
- **Cost** (the instruction gate): 30.2k after PR W to 31.3k (+3.6 %; 37.3k before round 5): the ramps of the three filters' coefficients and gains, twelve adds a frame.

### PR 8: the vocoder

What changed (design: [fx-voice.md](fx-voice.md), [fx.md](fx.md)): a new effect, the Vocoder (plan-5 decision 12.1, section 11), with its V6 rows (48), a gate case "Vocoder" and a golden line. 4 to 40 bands (16 by default) of 4th-order band-passes spaced evenly in pitch from Low to High; the carrier's bands summed with alternating signs, crossing 3 dB down, so the bank is flat within 1.5 dB (same-sign, it would notch 10 to 40 dB at every crossing: mpfree-claude `fx/quality/tools/vocoder_bank.py`); each band's power read from the band-pass and a partner in quadrature with it, so it doesn't ripple; the bands' gain from their Q, the range and the bank's summed power at that Bandwidth, so the level holds as the layout changes; Carrier Saw (following the voice's pitch with pitch correct's tracker, the default), Noise or Input (keyed by a voice; silent without a key, which only a project can set today); Unvoiced (the voice above 4 kHz), a gate, Spread and Mix; Low, High, Formant, Bandwidth and Spread glided, a 12 ms dip on a Carrier or Bands change. fxlab's `NodeDut` can now key a node by its own input (`keyed_by_input`), and round 4's keyed runner is shared with round 5.

- **Met:** latency 0; the nine sung vowels keyed onto the saw within -1.5 and +1.4 LU of it; F1 and F2 within 0.8 bands of the voice's (most within 0.5); the defaults -0.5 LU on pink and the presets -1.1 to +1.0; an "s" 3.0 dB down on the track and 2.9 on the key; every control's click -64.8 dB or under (Bands, through its dip; Bandwidth -71.9; the rest -76 or under). Voice: 102 of 102 met.
- **Tuned on the way:** the level constant (0.607; the theory gives 0.66) on the sung vowels; Bandwidth stops at 150 %, as wider alternating bands cancel (the summed power falls to 0.46 at 150 % and lower beyond); the presets keep the saw's fundamental inside the bands (Deep robot's Low 50 Hz) and Wide choir at Spread 80. The formant rows first compared the output with the table's formants: a sung vowel at 220 Hz has no harmonic at OO's F1 (300 Hz), so they now compare with where the voice itself is loudest.
- **Cost** (the instruction gate): Vocoder 111.2k at its defaults. As ARMv7 code, 275.1k at 16 bands and 590.6k at 40 (about 13k a band): about 6 % of a period on one MPC core at 16 bands, by estimate (fx-voice.md section 8).
