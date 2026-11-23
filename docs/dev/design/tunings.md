<!--
covers: rust/mpfree-tuning rust/mpfree-sampler/src/engine/tuning.rs rust/mpfree-app/src/sampler.rs
checked: 2026-11-23 9d75788
-->
# Tunings for keygroups and every instrument

**Status:** 2026-11-23, plan approved as written by the owner; T1 to T3 (phase 1) built. T2 (the sampler engine) is described in [Sampler architecture](sampler-architecture.md#kits-in-a-tuning-tunings-t2-mpfree-sampler). The plan, its research (Scala and MTS-ESP, Kontakt, Ableton Live 12, Pianoteq, Surge XT, the MIDI Tuning Standard) and the four decisions are in the Claude Doc [Microtonal tunings for keygroups and every instrument](https://claude.ai/artifact/4x2unuJnGr322L9XHd4Xfy). How a `Tuning` works today, and the Synth tab's Tuning rows, are in [Synths](synths.md#tunings-and-per-note-expression).

Keygroups get the tunings the synths already have, and every instrument can have a tuning of its own that starts as the song's. Later phases bring plugin synths and outside gear into the tuning (MPE, MTS, CV), and teach the pads, the arp and the scales about tunings that don't have 12 notes.

## The rules

- **Notes follow the tuning, hits don't.** Keygroup notes (from the pads playing notes, a keyboard, a clip, the arp or a MIDI route), 16 Levels set to Tune, glide and legato, warped and grain keygroups follow the tuning. A pad hit, a sliced pad, slices put on keys (Chop to keys) and the Drums synth play as tuned on the pad, as drum hits do everywhere.
- **A keygroup's root stays at its 12-TET pitch.** Key *n* sounds at the tuning's pitch for *n*: the voice adds `Tuning::offset_cents(n)` to its Tune. A4 = 432 moves a keygroup by the same -31.8 cents it moves a synth.
- **Bends move along the tuning's keys**, as the synths' bends already do: a bend of *b* keys plays the tuning's pitch at *note + b*.
- **The song has one tuning, and each instrument has Song's or its own.** It belongs to the instrument (by its id in the project's [instrument list](instruments.md)), not the track. Duplicate copies the pick; a preset doesn't carry one; a patch never changes it ([Live shows](live-shows.md)).
- **Auto-sample renders our own synths in 12-TET**, so the samples' roots are clean and the kit's tuning applies once.
- **Nothing new allocates or runs per sample on the audio thread**: a key's offset is worked out when its note starts or the tuning changes.

## Roadmap

Ten steps in four phases, one draft pull request each through the merge desk, each with its manual and design pages.

| Step | What | State |
|---|---|---|
| T1 | `Tuning::offset_cents`; `TuningPick` and `TuningFile` move from the app into `mpfree-tuning` | built ([below](#t1-as-built)) |
| T2 | The sampler engine: a tuning per kit place; keygroup notes, pads playing notes, 16 Levels Tune, glide and legato, the wheel, grains and warped keygroups follow it | built ([Sampler architecture](sampler-architecture.md#kits-in-a-tuning-tunings-t2-mpfree-sampler)) |
| T3 | The app: kits follow the song's tuning; auto-sample in 12-TET; the manual's Tuning section and keygroup pages | built ([below](#t3-as-built)) |
| T4 | Mockups: where the song's tuning lives, an instrument's Tuning row, a kit's, a plugin that can't follow | planned, needs the owner's approval |
| T5 | The `tunings` project part by instrument id (Song's or own), project format 5, the resolver on the audio loop | planned |
| T6 | The approved screens; CLAP synths get each note's offset; the plugin sandbox gets the tuning; MIDI-only plugins say they play 12-TET | planned |
| T7 | Tunings with other than 12 notes across the pads, the arp, scales, note names and the piano roll | planned, its own design first |
| T8 | MPE gear plays each note's tuned pitch | planned |
| T9 | MIDI Tuning Standard messages for gear whose device profile takes them | planned |
| T10 | CV pitch follows the tuning | planned |

Phase 1 is T1 to T3 (keygroups follow the song's tuning, no new screen); phase 2 is T4 to T6 (one tuning per instrument); phases 3 (T7) and 4 (T8 to T10) come in either order after it.

## T1 as built

- **`Tuning::offset_cents(note)`** (`rust/mpfree-tuning/src/lib.rs`): how far a note, whole or fractional, sounds from where 12-TET at A4 = 440 Hz puts it, in cents, so that a sound playing note *n* at `offset_cents(n)` cents above its 12-TET pitch sounds at `hz(n)`. It is exactly 0.0 for the standard tuning at every note (the equal branch checks 12 steps on key 69 and returns the A4 shift, `1200 * log2(440 / 440)`, which is 0.0), so a keygroup in 12-TET will play to the bit as it does now. An equal tuning works it out from its formula; a table tuning from its own log2 pitch, interpolated as `hz` interpolates it. A note that isn't finite gives 0.0. It never allocates.
- **`TuningPick` and `TuningFile`** (`rust/mpfree-tuning/src/pick.rs`), with `A4_MIN` and `A4_MAX` (380 and 480 Hz): moved from `mpfree-app/src/synth.rs` unchanged, so the sampler, MIDI out and the app share one type. The app re-exports them from `synth`, so its code and tests didn't change. Note names come from `mpfree-scale`, now a dependency of `mpfree-tuning` (neither has other dependencies). `MAX_TUNING_FILES` stays in the app, which reads the tunings folder.
- **Tests:** a note moved by its offset sounds at the tuning's pitch (within 0.02 cents) for every built-in tuning on four roots at four A4s and 19-TET, from key -64 to 191 in steps of 0.07, and agrees with `cents_off` at every key; the standard tuning's offset is 0.0 to the bit at every note, out of range and not a number; A4 = 432 is -31.77 cents, a quarter tone above A4 is 50 under A#4, and just on C puts the third at -13.69 and the fifth at +1.96; a pick makes its tuning and its label, clamps A4, plays a file instead of its built-in, ignores root and A4 for a mapped file and falls back when the file isn't offered. The `no_alloc` test adds picks (built-in, file, missing file) and offsets.

## T3 as built

- **Every kit plays the song's tuning.** `Player::set_tuning` (`rust/mpfree-app/src/sampler.rs`) hands one `Tuning` to every kit place of the engine (`Engine::set_tuning`, cheap for a kit already in it). The pad player's loop calls `kits_follow_tuning` (`rust/mpfree-app/src/main.rs`) every period, after the tempo: it compares the Synth tab's `TuningPick` with the one handed over last and, only when they differ, makes the tuning (`Synth::tuning_now`) and hands it over. The Console line ("sampler: the kits' keygroups play Just (5-limit) on D (pad hits and slices as tuned on the pad)") comes with each change but not for 12-TET at the start. The pick is the song's (saved with the project, as before), so a project opened in a tuning plays its kits in it from the first period. Until T5 every kit plays the song's tuning; T5 resolves each instrument's own.
- **Auto-sample records in 12-TET.** It plays `synth::fresh_node`, a copy made from the patch's values and design. A tuning is handed to the engines by `Synth::retune`, outside their parameters, so the copy starts in 12-TET whatever the song plays in, and a keygroup made from it plays the tuning once. Gear on MIDI plays as it is tuned (the manual asks for standard tuning).
- **Tests:** `the_kits_follow_the_songs_tuning_and_say_so_on_a_change` (every kit gets the pick, once, with its line; the same pick again does nothing; 12-TET at the start says nothing; back to 12-TET is said) `an_auto_sample_copy_plays_in_12_tet_whatever_the_tuning` (for each of the 12 engines with A4 = 432: the patch's values and design don't change, the pads' engine plays the tuning but Drums, and the copy plays to the bit as the engine did in 12-TET), and the plan's whole-session proof, `smoke::song_workflows` `a_song_in_just_on_d_plays_its_keygroups_tuned_and_keeps_it`: Tuning and root picked on the Synth tab's options reach the kit with the Console line, a keygroup's F#4 sounds 13.7 cents under 12-TET (within 1 cent of the tuning's pitch, measured from the session's output), the project saves `tuning = just` and `tuning-root = D`, New project puts 12-TET back, and opening the song again tunes the kit again. The smoke session hands the kits the tuning as `main.rs` does.
- **The screens and the manual:** the Synth tab's Tuning row's help says kits' keygroups play the tuning too, and drums, pad hits and slices keep their own pitch (`app/synth_pads.rs`). In the manual, the Synth tab's Tuning section says which parts of a kit follow, Playing a kit on keys says where a keygroup's root sits, and Auto-sample and 16 Levels say what they do in a tuning.
