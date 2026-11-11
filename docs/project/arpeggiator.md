# Arpeggiator, Latch and latch patterns

**Status:** design and first engine, 2026-11-10; owner decisions recorded below. The engine is [`rust/mpfree-arp`](../../rust/mpfree-arp) (host-only, unit-tested, not yet wired to the pads). Nothing here has run on the MPC.

## What the owner asked for

From the round 4 notes (2026-11-10): get a **Latch** button working, and start separate work on **arpeggiator and latch pattern tools**. Then: research what people value in high-quality arpeggiators, what makes one great, how to make it intuitive yet flexible for many playing styles, and how to let people **easily design custom arp patterns**.

The Latch button itself (and Note Repeat, Full/Half Level, velocity) is being wired into the pad player in the Hello MPC work (`docs/guides/hello-mpc.md` in PR #31). This document designs everything that sits behind those buttons, and `mpfree-arp` is the engine both use.

## What the research says

Sources are listed at the end. Where a point comes from general knowledge of a product rather than a page read for this document, it says so.

### What makes an arpeggiator great

1. **It is immediate.** Press one button, hold notes, and something musical happens with good defaults. Users on KVR asked for arps that "work quickly and are easy to pick up, not one that takes study"; reviewers of the KeyStep mk2 and Hydrasynth praised hands-on control and criticised menu diving and Shift combinations. Cthulhu and Digitone's arp are called powerful but "not very immediate" with "a steep learning curve".
2. **The basics are complete.** Every good arp has: order (up, down, up-down with and without repeated ends, as played, random, chord), octave range, rate including triplets, gate (note length), swing, latch/hold, and played or fixed velocity. Novation's SL MkIII, Logic, Ableton, Akai Force and the KeyStep all share this core.
3. **Note order goes beyond up and down.** Logic offers outside-in and inside-out orders, four variations per order, inversions in place of octaves, and random without repeats. Ableton has Converge, Diverge, Play Order, Chord Trigger, Random Other (no repeat until all notes played) and Random Once (one random order, repeated until the notes change), plus a Pattern Offset that rotates the start.
4. **Latch has more than one meaning, and players want the right one.** Reset (a fresh press replaces the chord), Add (new notes join), toggle (Ableton's Hold: re-pressing a note removes it), and Logic's Transpose and Gated Transpose (a new key moves the whole latched pattern). Akai Force lets a sustain pedal latch, in Reset or Add mode. Sound On Sound notes the Pro One-style latch that adds held keys "found little musical use", so Reset is the right default.
5. **Rhythm is separate from note order.** Novation's rhythm pattern toggles steps on and off without changing the order; step probability (0–100 %) per step; Akai Force has Note Repeat, Rhythm and Pattern actions, and a Spread control that strums held notes. Ratchets (several hits in one step) with a chance setting are a highlight of the Hydrasynth; the KeyStep mk2 review lists missing ratchets and probability as faults.
6. **Variation without losing the original.** KeyStep mk2's Mutate makes new lines from the current one (a long press changes more), and Shift+Mutate restores the original; reviewers called it "invaluable, especially in live jams" but noted it can be "unpredictable" with chords. Its Spice/Dice changes rhythm and velocity without touching pitch. Novation users ask for Deviate and Mutate. Ultra's arp gives each lane its own random amount under a master random knob, and re-randomises on every pass in Cubase's pattern editor.
7. **Pitch tools keep it in key.** Scale snapping is one of the most requested features (Elektronauts, Digitone threads). Per-step semitone offsets turn one held note into a phrase (Elektron offsets, Ultra's sequencer mode, Sylenth1's step mode, Akai Force's Pattern action where the held note transposes a phrase). Digitone users complained that the scale was not applied to the offsets, so offsets landed out of key: whatever we build must apply the scale after every offset. Ableton's Distance and Steps transpose the whole pattern each repeat (by semitones or scale degrees).
8. **Velocity and feel.** Played, fixed or per-step velocity (added to or replacing the played one, in Ultra), accents, a velocity ramp towards a target (Ableton's Decay and Target), and swing per step, not just per pattern (Ultra's swing lane).
9. **Restart behaviour matters.** Ableton's Retrigger: never, on each new note, or on a beat or bar; Ultra's "Anchored" (locked to the timeline) versus starting on the key press; Repeats (play the pattern N times, for strums). The Hydrasynth review complained that turning the arp on silences sounding notes.
10. **What it plays can be kept.** Logic's Capture Live Performance drags the arp's output to a track as editable MIDI; Cubase converts patterns to MIDI parts; SOS suggests recording the arp and deleting notes to break up its regularity. KVR users ask for MIDI out and MIDI file import.
11. **Arps on drums.** SOS suggests letting each arpeggiated note trigger a different drum sound. On a pad sampler this is natural: latch a few pads and the arp becomes a drum pattern generator.

### What MPC users say about Akai's arpeggiator

Threads on MPC-Forums (few posts, so a thin picture): the built-in patterns "get bland after a bit"; users could not make their own patterns on the device; custom patterns **play one note at a time and don't follow the held chord** ("arpeggios are intelligent, patterns aren't"); and users want rhythms that play across a held chord. Akai's own help article says custom patterns are made by exporting a MIDI track into the Arp Patterns folder, and only appear after making a new project. The factory content on the owner's MPC has an `/MPC Documents/Arp Patterns/` folder; per the [factory content policy](factory-content-policy.md) we neither copy nor ship those files.

### What this means for MPFree

- **One button and good defaults first.** Note Repeat behaves as on any MPC; Latch is one button; the arp is one more. Everything else is on one screen page and four Q-Links, never more than one menu deep.
- **Patterns refer to held notes, not fixed pitches.** A step says "the 2nd held note", "the next note in the order", "every held note" or "the lowest", plus an octave and, later, a scale-degree offset. So a pattern follows the chord the player holds, which is exactly what MPC users said Akai's patterns cannot do.
- **Lanes, each with its own length.** Rhythm (on, rest, tie), note pick, octave, velocity, gate, ratchet and chance are separate lanes. Different lane lengths make polymeter, which suits the project's sequencer goals ([architecture](architecture.md)).
- **Make patterns three ways:** tap steps on the screen grid, play them in on the pads (record mode), or type them as text. Import MIDI files later.
- **Variation is safe.** Mutate and randomise always keep the original, one press away.

## Concepts

| Term | Meaning in MPFree |
|---|---|
| **Held notes** | Pads or keys physically down. |
| **Active notes** | What the generators play: held notes plus latched ones. |
| **Latch** | Keeps notes active after release. Modes: Reset (default), Toggle, and later Add and Transpose. Works in every mode, including plain playing, where it acts as a sustain. |
| **Direct** | No generator: notes play as pressed. |
| **Note Repeat** | Every active note retriggers on every step, following pad pressure, as on an MPC. |
| **Arp** | One note per step, walking the active notes in an order, over 1–4 octaves. |
| **Latch pattern** | A row of steps that says when to play and which active note(s) to play. Patterns and the arp share rate, gate, swing and octave settings. |

On the pad player, the "notes" fed to the engine are **pad slots** (pad 1 = 0 … pad 16 = 15), not Akai's scrambled pad note numbers, so "up" means up the pads. With a melodic program (later) they are MIDI notes.

## The engine (`rust/mpfree-arp`)

Notes in, sample-timed notes out. It lives in the audio thread: the app calls `press`, `release`, `pressure` and the setters between periods, then `process(frames)` once per period, which returns the note-ons and note-offs inside that period with their frame offsets. It never allocates, locks or does I/O after `Engine::new`, and its output does not depend on the period size (a test runs the same input in blocks of 1, 64, 127, 1000 and 100 000 frames and gets identical events).

Built now:

- **Latch** Off, Reset (called `Replace`), Toggle; clear latched notes; panic.
- **Modes** Direct, Repeat, Arp, Pattern. Switching mode never leaves a gap: notes sounding in Direct mode are stopped and the generator starts on the same frame.
- **Orders** Up, Down, Up-down, Up-down inclusive, Converge, Diverge, As played, Random (seeded, so it is repeatable), Random with no repeats until every note has played, Random once (one shuffled order kept until the notes change), Chord; 1–4 octaves; notes past 127 are skipped.
- **Feel** strum (spreads chord notes across up to the whole step, lowest first); retrigger (a new note restarts the order and pattern); repeat count (play N steps, then stop, for one-shot runs).
- **Scales** ten scales (major, minor, harmonic minor, dorian, phrygian, mixolydian, both pentatonics, blues, chromatic) with any root, applied last so octave and degree offsets always land in key. Off by default and for drum pads.
- **Timing** rates 1/4 to 1/32 with triplets, at 960 ticks per quarter; gate 1–100 %; swing 50–75 % on every second step. The first step plays on the press (there is no transport yet).
- **Velocity** played (following pad pressure) or fixed; accent velocity.
- **Patterns** up to 64 steps, written as text:

```text
x . x x        gallop: the next note, rest, next, next
1 3 2 3        a broken chord over whatever is held
1 2 3 1+ 2+ 3+ 1++ .   climb two octaves
* . . * . . * .        chord stabs on a 3-3-2 rhythm
x x:2 x x:3    ratchets
x x%60 ? x%30 . x _ ?  chance, random notes and a tie
```

| Token | Step |
|---|---|
| `.` | rest |
| `_` | tie: the step before holds on through this one |
| `x` | the next note in the arp order (with octaves) |
| `1`..`9` | the Nth active note, lowest first (wraps if fewer are held) |
| `*` | every active note (a chord) |
| `?` | a random active note |
| `l` / `h` | the lowest / highest active note |
| after a note: `+` `-` | an octave up / down (up to three) |
| `!` | accent |
| `~` / `'` | full-length / short note |
| `:N` | N hits in the step (2–8) |
| `%N` | N % chance |

**Lanes** follow the steps after a `|`, each with its own length, so they loop against the steps (polymeter):

```text
x x x x | oct 0 1 0 | vel 120 70 90 70 80     octave over 3, accents over 5, steps over 4
1 1 1 1 1 1 1 1 | deg 0 2 4 5 7 4 2 | gate 40 40 80   one held note plays a line
```

| Lane | Values |
|---|---|
| `oct` | octaves added to the step's own, −3 to 3 |
| `deg` | scale degrees (with a scale) or semitones (without), −24 to 24 |
| `vel` | velocity 1–127; 0 = as played; an accent still wins |
| `gate` | note length in percent; 0 = the step's or the engine's |

**Variation** that never loses the original: `Pattern::mutated(seed, amount)` returns a new pattern (the same seed gives the same variation; ties stay; at least one step plays) and `Pattern::rotated(n)` turns the steps. The app keeps the original and swaps back on "Original".

Ten original starter patterns are built in (Straight, Gallop, Bounce, Climb, Pulse, Stabs, Rolls, Drift, Three over four, Climbing line). The text form round-trips, so the screen editor and files use the same model.

## The screen and the buttons

All of this uses `mpfree-ui` widgets and one-line layouts (`docs/project/ui-architecture.md`, in PR #31), so the page can be rearranged later.

**Buttons** (only SHIFT, FULL LEVEL, PLAY, STOP and TAP TEMPO are measured; the Hello MPC round 5 guide will capture the rest. That Latch is Shift+NOTE REPEAT and Half Level is Shift+FULL LEVEL is a guess from the panel print, not verified):

| Control | Action |
|---|---|
| NOTE REPEAT (hold) | Repeat while held, as on an MPC. |
| NOTE REPEAT (double press, proposed) | Repeat stays on. |
| SHIFT + NOTE REPEAT (Latch, unverified) | Latch on/off; its light shows the state. Pressing it while on with notes latched clears them first (proposed). |
| TAP TEMPO | Sets the tempo the arp follows. |
| Q-Links 1–4 on the Arp page | Rate, gate, swing, octaves. |
| Data wheel | Order (Arp mode) or pattern (Pattern mode). |
| Pads | Hold notes; in pattern record mode, play steps in. |
| MENU on the Arp page | The page's options (latch mode, velocity, restart). |

**Arp page** (proposed layout, same style as the Play page):

```text
col(header:60, row(pads:*, col(mode:60, order:*, knobs:180):420):*, steps:150, hint:26)
```

- **mode**: Direct · Repeat · Arp · Pattern tabs.
- **order** or **patterns**: a list, chosen by the wheel or a tap.
- **knobs**: rate, gate, swing, octaves, each a fader that mirrors its Q-Link.
- **pads**: the pad grid, with latched pads outlined and the pad the arp is playing lit.
- **steps** (new widget, `StepGrid`): the pattern's steps in a row, the playing step marked. Tap a step to cycle rest → note → accent; drag up and down for the note pick and octave; long-press for ratchet, chance and gate. Each lane can be shown on its own row when the pattern editor opens full-screen.
- The **Debug** page already shows MIDI; it will also list the engine's generated events with their frame offsets.

## Making custom patterns

Three ways, all producing the same `Pattern`:

1. **Tap on the step grid** (above).
2. **Record on the pads** ("live mode", after Logic's): with a pattern armed, each pad press writes the next step. Pad 1 = "1st held note", pad 2 = "2nd" and so on; hitting several pads at once writes a chord step; a press of the rest pad (proposed: pad 16) writes a rest, and hold means tie. The step's velocity is the pad's velocity.
3. **Text**, in a file or typed in Prefs, as above.

Later: import a MIDI file the owner made (notes become note picks by their rank in the file's chord); export what the arp played as a MIDI file or to a sequencer track.

Patterns save to `/data/mpfree/patterns/` as text files once MPFree has persistent storage; on the RAM-only test build they live in memory.

## Hearing it on the workstation (`mpfree-arp-render`)

`mpfree-arp-render` is a workstation-only program in the same crate. It runs the real engine in 128-frame periods at 44.1 kHz, the same way the pad player will, and plays its notes through a small built-in synth (two detuned saws, a low-pass filter and an envelope) or, for drum demos, 16 simple drum sounds made in code (pad 1 kick, 2 snare, 3 closed hat and so on). It needs no samples and no MPC, and its output is the same on every run.

```sh
# Every order, every starter pattern, drums, lanes, scale, strum and mutate: 29 WAVs
cargo run --manifest-path rust/Cargo.toml --release --locked -p mpfree-arp --bin mpfree-arp-render -- --demo

# One of your own: a chord per bar, looped
cargo run --manifest-path rust/Cargo.toml --release --locked -p mpfree-arp --bin mpfree-arp-render -- \
    --chords "57,60,64,67;53,57,60,64" --mode pattern --pattern "1 3 2 3 x:2 . 4 x | oct 0 0 1"
```

It writes to `~/mpfree-work/arp-render-<UTC>/` (or `--out DIR`): the WAVs plus a `report.txt` that lists each file with its length, note count, peak level, dropped events and a hash. Run it through `tools/mpc-one/results.py run`, and the report goes into the results zip; the WAVs stay in the work folder for listening. Options: `--mode`, `--order`, `--pattern` or `--builtin NAME`, `--chords`, `--notes` or `--pads 1,3`, `--bpm`, `--rate 1/16T`, `--octaves`, `--gate`, `--swing`, `--strum`, `--scale d_dorian`, `--bars`, `--seed`, `--name`.

## Implementation plan

| Step | What | Depends on | Device? |
|---|---|---|---|
| A (done, PR #33) | `mpfree-arp` engine: latch, repeat, arp orders, patterns, swing, ratchets, chance, ties; text format | nothing | no |
| A2 (this PR) | `mpfree-arp-render`: hear every order and pattern as WAV files on the workstation | A | no |
| B | Wire into the pad player: NOTE REPEAT, Latch, TAP TEMPO; `Mixer::hit_at(offset)` so hits land on their frame instead of the period start; Arp page with mode tabs, order list, faders | PR #31 merged; round 5 button capture | yes, paced round |
| C | `StepGrid` widget and the pattern editor (tap, record on pads); pattern files | B | yes |
| D (engine done, PR #35) | Lanes with their own lengths (polymeter): octave, degree, velocity, gate; scales applied last. Still to do: per-step swing/nudge lane, and the UI | C for the UI | no (engine), then yes |
| E (engine mostly done, PR #35) | Done: mutate with keep-original, rotate, Random no-repeat and Random once, strum, retrigger on a new note, repeat count. Still to do: per-lane random amounts, velocity ramp, retrigger on beat or bar (needs a transport) | D | no, then yes |
| F | Transport and MIDI: sync to the sequencer's clock and MIDI clock in and out, MIDI out of the arp, capture to a track, MIDI file import | sequencer and MIDI I/O work | yes |

Each device step follows the project's device safety rule and the paced-test pattern: built and rehearsed on the workstation first, RAM-only on the test build, storage hashes before and after, one Enter per phase.

## Decisions (owner, 2026-11-10)

The owner accepted all four proposals, and they apply to every live test from round 5 on:

1. **Latch defaults to Reset:** a press with nothing held starts a new chord; Toggle is an option.
2. **Latched pads keep repeating** after NOTE REPEAT is released, until Latch is turned off.
3. **The generator starts on the press**, not on the next beat, until a transport exists; then it becomes a setting.
4. **Record-on-pads layout:** pads 1 to 9 are the 1st to 9th held note, several at once a chord, pad 13 the next note in the order, 14 random, 15 tie, 16 rest.

## Sources

- [Sound On Sound: Making the most of arpeggiators](https://www.soundonsound.com/techniques/making-most-arpeggiators)
- [Sound On Sound: Arp nouveau (Logic's arpeggiator)](https://www.soundonsound.com/techniques/arp-nouveau)
- [Logic Pro: note order variations](https://support.apple.com/en-ca/guide/logicpro/lgce2a5ad7b7/mac)
- [Ableton Live 12 manual: Arpeggiator](https://www.ableton.com/en/live-manual/12/live-midi-effect-reference/)
- [Novation SL MkIII: using the arpeggiator](https://userguides.novationmusic.com/hc/en-gb/articles/25626813121298-Using-the-SL-MkIII-s-Arpeggiators-Arp)
- [Akai Force manual, arpeggiator (p. 73)](https://www.manualslib.com/manual/1573562/Akai-Force.html?page=73)
- [Akai: MPC Series, creating custom arp patterns](https://support.akaipro.com/en/support/solutions/articles/69000856004-mpc-series-creating-custom-arp-patterns)
- MPC-Forums threads on the arpeggiator: [1](https://www.mpc-forums.com/viewtopic.php?p=1792503), [2](https://www.mpc-forums.com/viewtopic.php?p=1895822)
- [Synth Anatomy: Arturia KeyStep mk2 review](https://synthanatomy.com/2025/11/arturia-keystep-mk2-review-all-time-favorite-midi-sequencer.html)
- [Sound On Sound: ASM Hydrasynth review](https://www.soundonsound.com/node/4921184?page=2)
- [Ultra synth: Arp / Sequencer help](https://ultra.audio/help/synth/arp)
- [Sound On Sound: Cubase 14 pattern editor arps](https://www.soundonsound.com/techniques/cubase-14-pattern-editor-arps)
- [Attack Magazine: complex arps](https://www.attackmagazine.com/technique/tutorials/complex-arps-arpeggiator/3/)
- Forums: [Elektronauts, best hardware MIDI arpeggiator](https://www.elektronauts.com/t/best-hardware-midi-arpeggiator/48654?page=2), [Elektronauts, Digitone arp questions](https://www.elektronauts.com/t/digitone-arp-questions/130327), [KVR, most handy arpeggiator](https://www.kvraudio.com/forum/viewtopic.php?p=9056354), [KVR, should I develop an arpeggiator](https://www.kvraudio.com/forum/viewtopic.php?p=7639059)
