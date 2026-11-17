<!--
covers: rust/mpfree-fxlab
checked: 2026-11-16 ececc7e
-->
# FX quality: targets, the report and the ratchet

**Status:** measuring built 2026-11-16 (delay, EQ and reverb). The first sound changes are below: PR B (the delays' and reverbs' Mix curve, the Room's level and the Tape and Multi-tap tails) and PR C (the delays: flat reads, saturation at twice the rate, the Tape's colour on every echo and a Feedback that is its loop gain). The fixes come one class at a time, each in its own pull request, judged by these numbers and by blind, level-matched listening. Research and the full plan: jferments/mpfree-claude `fx/quality/` (plan.md, research-delay.md, research-eq.md, research-reverb.md, baseline-2026-11-16.md).

An effect is good when it meets its measured targets, wins or ties in blind, level-matched listening, has the controls musicians know, and fits the MPC's budget. This page holds the first of those: each target as a number with its reason, how fxlab measures it, and the ratchet on CI that stops any number getting worse.

**Second round** (measuring built 2026-11-17): drive (Drive and Character), crush, compressor, filters (Multi and DJ) and lo-fi, by the same method, in [Round 2](#round-2-drive-crush-compressor-filters-and-lo-fi) below; the amp joined it on 17 Nov (class `amp`, ids A; plan-2 section 12, research-amp.md). Research and plan: jferments/mpfree-claude `fx/quality/` (plan-2.md, research-drive.md, research-crush.md, research-compressor.md, research-filter.md, research-lofi.md, baseline-2026-11-17.md).

## The report

`fxlab quality [--class CLASS]... [--out DIR]` (CLASS: `delay`, `eq`, `reverb`, `drive`, `crush`, `compressor`, `filter`, `lofi` or `amp`) measures every target of a class (all of them when none is named) and prints one row per number: the target id, what was measured, the value, the target and whether it is met; DIR/quality.json holds the same. Subjects are the modes of the effect's family at their defaults (`delay/digital`, `reverb/hall`), and for the cheaper measures each family preset in its mode (`reverb/plate[Vocal plate]`). The EQ's subjects are `eq` and `eq[preset]`; an engine that isn't in a family is `class/engine` (`crush/crush[Chip]`). All nine classes take about 38 s in a release build (the round-2 classes 15 s, most of it the Drive's 4x quality).

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
- CI runs it in the `rust` job (release, about 20 s). In a debug build the test is ignored: the renders take minutes there.

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
| E3 | `bell_<f>_q<Q>_worst_db`, `..._width_ratio` | +12 dB bells at 1 to 16 kHz, Q 0.71 to 4: within 0.5 dB of the analog prototype from 20 Hz to 20 kHz, half-gain width within 5 % | Users learned on analog-style EQs; about 1 dB is audible on broad curves |
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
| R3 | `edt_t30_log2` | EDT/T30 from 0.8 to 1.25 (Hall and Cloud up to 1.6) | No slap and then a faint tail |
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
| K7 | `lu_pink18`, `lu_pink6` | Reported until auto makeup exists | Fair A/B |
| K8 | `null_ratio1_dbfs`, `null_below_knee_dbfs`, `null_mix0_dbfs` | Bit for bit | Inserting it costs nothing |
| K9 | `overshoot_db_attack*` | Reported | No look-ahead yet |
| K10 | `zipper_db` | ≤ 3 dB, defaults and presets | Smooth automation |

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
| F10 | `zipper_db` | Reported (fxlab's usual zipper reads high where a filter's ends attenuate the tone) | |

### Lo-fi (L)

| Id | Row | Target | Why |
|---|---|---|---|
| L1 | `wf_peak_pct`, `wow_peak_pct_top`, `wf_peak_pct_amount0` | The defaults' wow and flutter within ±20 % of 0.116 % peak; Amount and Wow at 100 at least 2 %; Amount 0 under 0.001 % | From a clean deck to a broken machine |
| L2 | `flutter_peak_pct_top` | At least 0.5 % | A worn transport |
| L3 | `mean_pitch_error_pct_*` | ≤ 0.01 % | The track stays in tune |
| L4 | `wow_line_share` | The pitch track's strongest line holds at most half its power | A plain sine reads as vibrato |
| L5 | `sat40_alias_5k_0dbfs` | ≤ -65 dB (Saturation 100 held) | As G1 |
| L6, L7 | `sat*_h3_minus_h2`, `noise_*` | Reported | Tape's distortion is mostly third harmonic; noise levels |
| L8 | `cuts_heard_vs_drawn_*` | ≤ 0.5 dB | The screen tells the truth |
| L9 | `lu_pink18` | Within ±1 LU, defaults and presets (Old radio reported) | Fair A/B |
| L10 | `null_amount0_dbfs` | Bit for bit | Inserting it changes nothing |
| L11 | `zipper_db` | ≤ 3 dB | Smooth automation |

### Amp (A)

The Amp at each model (`amp/clean`, `amp/crunch`, `amp/lead`, `amp/bass`) and its presets (`amp/amp[Modern lead]`). The targets reach rich, high-gain distortion as one use among many; presets and defaults stay broad across styles.

| Id | Row | Target | Why |
|---|---|---|---|
| A1 | `alias_1k_m6dbfs_gain10`, `alias_2k_m6dbfs_gain10`, `alias_5k_m12dbfs_gain10` | ≤ -60, -60 and -50 dB at Gain 10, every model (Gain 5 and the presets' 5 kHz kept by the ratchet) | Inharmonic fizz on high notes; about -60 dB is where it stops mattering |
| A2 | `cleanup_db_gain5` | A 1 kHz sine's THD falls at least 10 dB from -6 to -30 dBFS at Gain 5 (Clean, Crunch, Bass; Lead reported) | Playing softer cleans up |
| A3 | `lu_pink18_gain*` | Pink at -18 dBFS within ±2 LU of the dry from Gain 2.5 to 10 (Gain 0 reported) | Gain changes grit, not loudness |
| A4 | `lu_pink18` (presets) | Within ±1 LU | Fair A/B |
| A5 | `power_chord_sub70_db_gain10_nocab` | Reported for today's sound; a Tight boost, once there is one, under -25 dB | Palm mutes and chords stay clear, when asked |
| A6 | `bass_jump_db`, `mid_jump_db`, `treble_jump_db` | A knob thrown from 0 to 10 under a 220 Hz sine spikes ≤ 3 dB over the steady sound (switch click) | Smooth knobs |
| A7 | `gain_zipper_db` | fxlab's zipper on Gain ≤ 3 dB | Smooth knob |
| A8 | `model_change_db`, `cab_change_db` | ≤ 3 dB (switch click) | No clicks |
| A9 | `silence_out_dbfs_gain10` | ≤ -120 dBFS; the gate's rows come with the gate | Clean rests |

The fuzz's rows (A10, A11) come with the fuzz (decision 10.6), and A12 is the instruction gate's two Amp cases.

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
