<!--
covers: rust/mpfree-fxlab
checked: 2026-11-16 da8e97d3
-->
# FX quality: targets, the report and the ratchet

**Status:** measuring built 2026-11-16 (delay, EQ and reverb); no effect's sound has changed yet. The fixes come one class at a time, each in its own pull request, judged by these numbers and by blind, level-matched listening. Research and the full plan: jferments/mpfree-claude `fx/quality/` (plan.md, research-delay.md, research-eq.md, research-reverb.md, baseline-2026-11-16.md).

An effect is good when it meets its measured targets, wins or ties in blind, level-matched listening, has the controls musicians know, and fits the MPC's budget. This page holds the first of those: each target as a number with its reason, how fxlab measures it, and the ratchet on CI that stops any number getting worse.

## The report

`fxlab quality [--class delay|eq|reverb]... [--out DIR]` measures every target of a class (all three when none is named) and prints one row per number: the target id, what was measured, the value, the target and whether it is met; DIR/quality.json holds the same. Subjects are the modes of the effect's family at their defaults (`delay/digital`, `reverb/hall`), and for the cheaper measures each family preset in its mode (`reverb/plate[Vocal plate]`). The EQ's subjects are `eq` and `eq[preset]`. All three classes take about 20 s in a release build.

New analyses in `rust/mpfree-fxlab` for it, each tested on signals with known answers:

| Module | What it measures |
|---|---|
| `echo` | A delay's repeats from its impulse response: where each lands (energy centroid, so fractions of a frame show), its level at 1, 5, 10 and 15 kHz, left against right; loss per repeat from a line through the repeats' tilts |
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
| D1 | `time_error_frames` | First repeat within 0.5 frame of Time | Grooves |
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

D3 (loop filters equal their drawing), D13 (exact silence, the gates' `idle_db`) and D14 (cost, the instruction gate) are checked elsewhere.

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

## Today (2026-11-16, before any fix)

55 of 81 delay targets, 21 of 50 EQ targets and 68 of 114 reverb targets met. The large gaps: delay repeats lose 0.74 dB per repeat at 10 kHz and 3.5 dB at 15 kHz when the time falls between frames (the default 1/8 dotted at 120 BPM does); the Tape's first echo has none of the tape's colour; every delay and reverb dips 2 to 6 LU as Mix turns (a linear crossfade); EQ bells above 5 kHz are cramped (a 12 kHz, Q 0.71 bell is 6.6 dB short at 20 kHz and 42 % narrower than its prototype); Room is 4.9 LU quieter than the dry, starts with a slap (EDT/T30 0.40), takes 314 ms to diffuse and rings 4.9 dB over an ideal tail; Plate takes 167 ms to diffuse and smears a held note by 4.9 cents; no reverb but the Cloud engine's keeps a source's side.

## Running it

```sh
cd rust
cargo build --release --locked -p mpfree-fxlab
./target/release/fxlab quality --out /tmp/quality                 # all three classes
./target/release/fxlab quality --class reverb --out /tmp/quality  # one
cargo test --release --locked -p mpfree-fxlab --test quality      # the ratchet
```
