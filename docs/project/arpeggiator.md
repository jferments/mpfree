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
- **Scales** from the shared `mpfree-scale` crate ([scales.md](scales.md)): 49 named scales in nine families plus the owner's own, with any root, applied last so octave and degree offsets always land in key. A held note outside the scale goes to the scale note below (the default), above, the nearest, or is not played (`Settings::snap`). Degree offsets going down use the scale's notes coming down, for scales such as the classical melodic minor. Off by default and for drum pads: the arp has its own scale, and follows a project key only when a project has one (owner, 2026-11-11). **Chords from the scale:** each held pad can grow into a chord built from the scale (`Settings::chord`: power, triad, sus2, sus4, seventh, ninth), so one pad is enough for an arpeggio; a scale run (`Settings::run`) fills the steps with every scale note between the lowest and highest held note. **Chords take turns** (`Settings::turns`: every beat, 2 beats, bar or 2 bars): with two or more pads held, the steps play from one pad's chord at a time, in the order they were pressed, each turn starting the order again from its root. Holding A, F, C and G with triads in A minor arpeggiates Am F C G (owner asked 2026-11-11 09:51Z).
- **Timing** rates 1/4 to 1/32 with triplets, at 960 ticks per quarter; gate 1–100 %; swing 50–75 % on every second step. Free-running, the first step plays on the press. Following a transport (`Engine::sync` with a `Clock`, called before each `process`), the steps sit on the transport's tick grid: notes start on the next step, beat or bar (`Settings::start`), tempo changes and loop jumps are followed, and a transport swing (grid and amount) bends the grid; when the transport stops, the engine free-runs again.
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
| `nudge` | moves the step early (negative) or late, −50 to 50 percent of a step, on top of swing; a step never moves ahead of the one before it |

**Random amounts per lane.** A lane name can carry `~N`: each time a step plays, a random number from −N to N is added to that lane's value, so `vel~20` varies velocity by up to 20, `gate~30` the length, `oct~1` the octave, `deg~2` the degree and `nudge~10` the timing. A lane with a random amount needs no values (`x x x x | vel~25` varies the played velocity). Accented steps keep their accent. The engine's seed picks the numbers, so a seed always plays the same way, and a pattern without `~` draws no random numbers at all.

**Velocity ramp.** `Settings.ramp = Ramp { target, steps }` moves velocity from where it starts towards `target` over `steps` steps after the notes start, then holds it (Ableton's Decay and Target). With `retrigger` on, every new chord starts the ramp again. Accents are left alone.

**Variation** that never loses the original: `Pattern::mutated(seed, amount)` returns a new pattern (the same seed gives the same variation; ties stay; at least one step plays) and `Pattern::rotated(n)` turns the steps. The app keeps the original and swaps back on "Original".

Ten original starter patterns are built in (Straight, Gallop, Bounce, Climb, Pulse, Stabs, Rolls, Drift, Three over four, Climbing line). The text form round-trips, so the screen editor and files use the same model.

## The screen and the buttons

All of this uses `mpfree-ui` widgets and one-line layouts (`docs/project/ui-architecture.md`, in PR #31), so the page can be rearranged later.

**Where the arp lives (owner decision, 2026-11-11).** The arp is a way of playing the pads, and it belongs to one track, so it is not a top-level tab. It has three surfaces over one engine per track: the **quick band** while NOTE REPEAT is held (big rate buttons, Latch, the next steps; the Q-Links become rate, gate, swing and octaves), the **Arp panel** with every setting of one track (on the Sound tab as the first stage of the track's chain, and beside the pads from the arp chip in the header), and a full-screen **pattern workspace** that returns to the page you came from. The full reasoning, workflows, controls map and architecture are in the arpeggiator design doc (a Claude Doc shared with the owner, 2026-11-11).

**Built (on the workstation): `rust/mpfree-arp-ui`.** The panel and the band as `mpfree-ui` widgets: mode tabs (Direct, Repeat, Arp, Pattern), the rate, order or pattern list (wheel), the Q-Link sliders (rate, gate, swing, octaves), the pattern buttons (Edit pattern, Record on pads, Mutate, Original), the band's rate buttons (at least 76 px) and Latch, the arp chip for the header, and the step strip, which shows the order's notes (with their octaves) or the pattern's steps (picks, octaves, ratchets, chance, accents) and marks the step playing. The strip shows only while the arp is generating. The panel reads an `ArpState` taken from the engine and turns touches into `ArpAction`s; the pad player keeps the pads, the header and the hint line, so the panel drops into its Play page's right-hand column. The **pattern workspace** (`workspace` module) is drawn too: a grid with a column per step and rows for the step (rest, note, tie, accent), the note pick, octave, ratchet, chance and gate; a tap cycles rest, note and accent, and a drag or the wheel changes a value; Rotate, Mutate, Original (back to the pattern it opened with), length, Save and Back; and recording on the pads, with the pads named for what each writes and the grid at 8 steps beside them. Its `Editor` is plain logic with tests. The pattern's lanes are kept as they are; editing them on screen comes later. `mpfree-arp-preview` draws sample screens, each with a real engine behind it, as PNG files.

**In the pad player (`rust/mpfree-hello`, for round 6).** The pad player plays the pads through one engine. Its buttons were measured in round 5: MENU 123, NOTE REPEAT 11, TAP TEMPO 53, - 55 and + 54, besides SHIFT 49, FULL LEVEL 39, STOP 81 and PLAY 82 (`mpfree_controller::mpc_one::button`). Latch as SHIFT + NOTE REPEAT worked in round 5. Everything works from the buttons, so touch is optional:

| Control | Play page | NOTE REPEAT held | Arp panel | Pattern workspace |
|---|---|---|---|---|
| NOTE REPEAT | Hold: the arp in the panel's mode (Repeat until the panel picks another; Repeat if it picks Direct). Press while latched: let go | Let go: back to playing as pressed, unless latched | Same | Same |
| NOTE REPEAT twice (within 0.4 s), or SHIFT + NOTE REPEAT | Latch on or off | Same | Same | Same |
| MENU | Play options | Opens the Arp panel | Arp options; MENU again comes back | Nothing yet |
| NOTE REPEAT + MENU | | Opens or closes the panel | Closes it | Back to the panel |
| - / + | Kit | Mode: Repeat, Arp, Pattern | Mode tabs, Direct included | Kit |
| Data wheel | Kit | Rate, order (Arp) or pattern (Pattern) | Same | Turns the selected cell |
| Q-Links 1-4 | Q-Link 1: level | Rate, gate, swing, octaves | Same | Q-Link 1: level |
| TAP TEMPO | Tempo | Same | Same | Same |
| STOP | Every sound stops, latched pads are let go | Same | Same | Same |
| Pads | Play as pressed | Play through the arp | Play through the arp in the panel's mode | Sound as pressed; while recording, each press writes the next step |
| Arp chip (header, touch) | Opens the panel | Same | Closes it | Back to the panel |

- The **quick band** is one slim strip (56 px) at the top of the Play page's right-hand column while NOTE REPEAT is held or pads are latched: what the arp plays (tap it to open the Arp panel), the rate with − and +, and Latch. The kit list gives up the strip's height; the pads, level and timing don't move (owner request, 2026-11-11: holding NOTE REPEAT or latching at a rate should be a tiny change).
- The **Arp panel** (the arp editor) takes the whole column, step strip included, so the pads never move or resize between Play, the band and the panel. With it open the pads play through its mode, so the NOTE REPEAT light is on; closing it, or picking Direct, plays them as pressed again.
- The **pattern workspace** opens from the panel's Edit pattern and Record on pads (Pattern mode). **Lanes** switches the grid to the pattern's lanes (octave, degree, velocity, gate, nudge), each looping at its own length: drag or turn a value, drag the `+` cell to add one, and Length −/+, Random −/+ and Clear lane work on the selected lane. **Recording** has Step time (one step per press, no clock, at least 8 steps, stops when full; the pads sound as pressed) and Free time (the pattern plays over the latched notes and each press lands on the nearest step; a pad held past the next step writes ties). Undo takes back a step in Step time; Clear starts again. **Save** keeps the pattern in the list and logs an `arp-pattern-saved: Name = pattern` line for the report. Save also keeps the owner's patterns (and custom scales) in the store as `files/arp/patterns.txt` under the `--data` folder (mpfree-store, see [storage-spec.md](storage-spec.md); on the test boot that is the stick's MPFREE-DATA partition), written whole or not at all by the store's own thread, and the next start reads them back. Without `--data` nothing is written and saved patterns last until the power goes off. `app --patterns FILE` reads a library at start in place of the saved one, and never writes it.
- **Arp options** (MENU on the Arp panel) are rows in the existing Options menu, so the layout does not change: Tempo; Accent level (1-127, steps of 8; what accented steps play, FULL LEVEL still wins); Scale (Off, the 48 named scales but Chromatic, then the owner's from the patterns file; Off by default, since on drum pads a scale would move hits to other pads), Scale root (pad 1 counts as C, as on the Synth's pads) and Out-of-key notes (note below, note above, nearest note, not played); Chord on each pad; Scale run; Chords take turns (Off, every beat, 2 beats, bar, 2 bars); Strum (0-100 % of a step, steps of 10); New notes restart (retrigger); Swing from (Transport, the default: while the sequencer plays with swing on, the arp swings with it on the sequencer's grid; Arp: always the arp's own swing). Each change logs an `arp ...:` line. The panel's own settings (rate, gate, swing, octaves) leave them alone.
- **Following the transport.** `App::set_clock(Option<Clock>)` gives the arp the transport's clock for the next period (the first span of it); `App::process` calls `Engine::sync` with it, so the arp's steps land on the sequencer's grid and follow its tempo, count-in included. Without a clock the arp free-runs at the app's tempo. The time engine calls it from round 7 (`TimeHost::arp_clock`, the transport thread's side).
- The button guide no longer opens at power-on for these buttons. It asks only for buttons nobody has pressed yet: REC, OVERDUB, UNDO and PLAY START, for the time engine. Prefs > Find the buttons again asks for all nine.
- On drum pads an octave is 12 pads: with Octaves at 2, pad 1's octave up plays pad 13, and anything past pad 16 is skipped. Octave steps in patterns (`1+`) work the same way.
- Q-Link turns arrive as steps of 1 or 2. Rate and octaves change every 3 and 4 units, so one detent does not skip values. Without Akai's session the Q-Links send positions (0-127), and those set the value directly.

**MIDI out of the arp (`mpfree_arp::midi`).** `MidiOut` turns the engine's events into note-on and note-off messages on one channel, through a `NoteMap`: `Same` (the notes are MIDI notes already), `Chromatic { base }` (pad slots: pad 1 sends `base`; `NoteMap::PADS` starts at C1, 36) or a 16-note table for a drum map. It counts the note-ons it sent per note, so each gets exactly one note-off even when a note plays again before it ends, and changing the channel or the map, or `all_off` (STOP), ends every sounding note first. It never allocates. `midi_file` writes the messages as a Standard MIDI File (format 0, 960 ticks per quarter note). No port is opened anywhere yet: which MIDI port the MPC sends on, and whether it may, goes through the Hello MPC thread and needs the owner's go.

## Making custom patterns

Three ways, all producing the same `Pattern`:

1. **Tap on the step grid** (above).
2. **Record on the pads** (`mpfree_arp::Recorder`, engine side done): pads 1-9 write the 1st to 9th held note, several of them pressed together a chord (`*`), 13 the next note (`x`), 14 random (`?`), 15 a tie, 16 a rest; pads 10-12 do nothing yet. A hit at or above the accent velocity (110 by default) writes an accent. Two modes, as in the record mockup:
   - **Step**: each press writes the next step, with no clock; Undo removes the last one; it stops when the pattern is full.
   - **Free time**: the pattern keeps playing and each press lands on the nearest step (`Engine::position()` gives where the pattern is), overwriting what was there; holding a pad past the middle of the next step writes ties.
3. **Text**, typed in Prefs or kept in a **pattern library file** (`mpfree_arp::Library`): one `Name = pattern` line each, `#` comments, written in any editor. Every bad line is reported with its line number, token and reason. [`rust/mpfree-arp/patterns/example.txt`](../../rust/mpfree-arp/patterns/example.txt) has eight original patterns and a summary of the syntax. Hear a whole file on the workstation with `mpfree-arp-render --patterns FILE`.

Later: import a MIDI file the owner made (notes become note picks by their rank in the file's chord); export what the arp played as a MIDI file or to a sequencer track.

Patterns save to `/data/mpfree/patterns/` as text files once MPFree has persistent storage; on the RAM-only test build they live in memory.

## Hearing it on the workstation (`mpfree-arp-render`)

`mpfree-arp-render` is a workstation-only program in the same crate. It runs the real engine in 128-frame periods at 44.1 kHz, the same way the pad player will, and plays its notes through a small built-in synth (two detuned saws, a low-pass filter and an envelope) or, for drum demos, 16 simple drum sounds made in code (pad 1 kick, 2 snare, 3 closed hat and so on). It needs no samples and no MPC, and its output is the same on every run.

```sh
# Every order, every starter pattern, drums, lanes, scale, strum, mutate, nudge, ramp and random lanes: 32 WAVs
cargo run --manifest-path rust/Cargo.toml --release --locked -p mpfree-arp --bin mpfree-arp-render -- --demo

# Every pattern in a library file of your own (copy rust/mpfree-arp/patterns/example.txt to start)
cargo run --manifest-path rust/Cargo.toml --release --locked -p mpfree-arp --bin mpfree-arp-render -- --patterns rust/mpfree-arp/patterns/example.txt

# One of your own: a chord per bar, looped
cargo run --manifest-path rust/Cargo.toml --release --locked -p mpfree-arp --bin mpfree-arp-render -- \
    --chords "57,60,64,67;53,57,60,64" --mode pattern --pattern "1 3 2 3 x:2 . 4 x | oct 0 0 1"
```

It writes to `~/mpfree-work/arp-render-<UTC>/` (or `--out DIR`): the WAVs, a `.mid` file of each (the same notes, at the job's tempo: drum pads on channel 10 from C1 up, other notes on channel 1 as played) plus a `report.txt` that lists each file with its length, note count, peak level, dropped events and two hashes: `events` (the notes played; the same on every computer, so it can be compared with Claude's run) and `audio` (the WAV bytes; repeats on one computer, but the synth's maths comes from the system library, so it can differ between computers). Run it through `tools/mpc-one/results.py run`, and the report goes into the results zip; the WAVs stay in the work folder for listening. Options: `--mode`, `--order`, `--pattern` or `--builtin NAME`, `--chords`, `--notes` or `--pads 1,3`, `--bpm`, `--rate 1/16T`, `--octaves`, `--gate`, `--swing`, `--strum`, `--scale d_dorian`, `--ramp 30:16`, `--retrigger`, `--bars`, `--seed`, `--name`.

## Implementation plan

| Step | What | Depends on | Device? |
|---|---|---|---|
| A (done, PR #33) | `mpfree-arp` engine: latch, repeat, arp orders, patterns, swing, ratchets, chance, ties; text format | nothing | no |
| A2 (done, PR #37) | `mpfree-arp-render`: hear every order and pattern as WAV files on the workstation | A | no |
| B (wired into the pad player, for round 6) | Done: NOTE REPEAT, Latch, TAP TEMPO, hits on their frame (`Mixer::hit_at`), the measured buttons, the quick band, the Arp panel, the arp chip and the pattern workspace in the pad player. Still to do: try it on the MPC | #48 (`mpfree-arp-ui`) | yes, paced round 6 |
| C (engine side done; screens drawn) | Done: record on pads (Step and Free time), pattern library files, `Engine::position`. Done too: the step strip and the pattern workspace with its record screen (`mpfree-arp-ui`). Wired into the pad player with B (recording in Step mode). Still to do: Free time recording on the device, pattern library files on the device, edit lanes on screen | B | yes |
| D (engine done, PR #35 and this PR) | Lanes with their own lengths (polymeter): octave, degree, velocity, gate, nudge; scales applied last. Still to do: the UI | C for the UI | no (engine), then yes |
| E (engine done except transport items, PR #35 and this PR) | Done: mutate with keep-original, rotate, Random no-repeat and Random once, strum, retrigger on a new note, repeat count, per-lane random amounts, velocity ramp. Still to do: retrigger on beat or bar (needs a transport) | D | no, then yes |
| F | Transport and MIDI: sync to the sequencer's clock (engine side done: `Engine::sync`; the transport thread drives it from round 7), MIDI clock in and out (done as bytes: `mpfree-midi-clock`, see [midi-clock.md](midi-clock.md); the time engine connects it), MIDI out of the arp (done as bytes: `mpfree_arp::MidiOut` and MIDI files, no port yet), capture to a track, MIDI file import | sequencer and MIDI I/O work | yes |

Each device step follows the project's device safety rule and the paced-test pattern: built and rehearsed on the workstation first, RAM-only on the test build, storage hashes before and after, one Enter per phase.

## Decisions (owner, 2026-11-10)

The owner accepted all four proposals, and they apply to every live test from round 5 on:

1. **Latch defaults to Reset:** a press with nothing held starts a new chord; Toggle is an option.
2. **Latched pads keep repeating** after NOTE REPEAT is released, until Latch is turned off.
3. **The generator starts on the press**, not on the next beat, until a transport exists; then it becomes a setting (`Settings::start`: Step, Beat or Bar, default Step).
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
