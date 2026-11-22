<!--
covers: rust/mpfree-context rust/mpfree-compose
checked: 2026-11-22 d9a2b851
-->

# The composer and the Musical Context

**Status:** phases 0A (the musical model) and 0B (the check through real playback), and phase 0C's listener, 2026-11-22. The code is [`rust/mpfree-context`](../../../rust/mpfree-context) and [`rust/mpfree-compose`](../../../rust/mpfree-compose). Nothing is on screen, nothing runs on the MPC, and nothing else in mpfree uses these crates yet. The owner approved the design on 2026-11-22 with Continue as the first tool.

## What it is for

Four composing tools share one engine and one set of controls:

- **Continue** carries a clip on for 1 to 16 bars, in the note editor, on Steps and from the clip card.
- **Add a part** listens to a row of the clip grid and plays a bass, chords, a melody or drums with it.
- **Improvise**, a fifth tab on the Arp panel, plays a line over a held chord.
- **Answer** replies to a phrase when you stop playing.

There is no trained model: the engine is procedural. It reads what was played (key, chords, groove, motifs), plans what each bar is for, then picks chords, then each phrase's rhythm and notes, then how they are played. Every result is heard before anything is written, and Keep writes it as one take that UNDO takes back.

## Roadmap

| Phase | What | State |
|---|---|---|
| 0A | The musical model: the Musical Context, phrases of events with roles and sources, decisions kept apart from notes, stable dice, the re-fit table, the timing contract, rule kinds, style components, instrument profiles | Built (this page) |
| 0B | An integration check, developer only: one simple phrase played in place of a clip through the real play view, kept and undone; clip revisions; the preview states tried against the real play view, player and take undo | Built (this page) |
| 0C | The full engine: listening, plan, harmony, phrase, playing, groove; three styles; two-sided Try again; the reading budget; listening tests against simpler baselines; MPC timings | Listening built; the rest next |
| 1 | Continue, with the preview states and an all-or-nothing Keep | |
| 2 | Add a part, with one harmony answer for rows and the song | |
| 3 | Improvise and Answer, with record sources, chord settle and note owners | |
| 4 | Later: Bridge between two clips or scenes, sections, styles from your clips | |

## Two crates

- **`mpfree-context`** holds the Musical Context's types: one reading of what was played, made once and read by every stage. It is its own crate so that other features can use the key, chords or groove later. Nothing else in mpfree changes for it until one does.
- **`mpfree-compose`** is the composer: a pure library with no screen and no sound. A result depends only on what it was given, the settings and a seed.

Both are plain data and functions. Nothing here runs on the audio thread; from phase 0C the composer runs as an idle-priority background job with a fixed budget per request.

## The Musical Context

- **Key** (`key.rs`): 72 keys (12 tonics, six modes). A `KeyReading` keeps up to three candidates with their odds, used inside for ranking only. The screen speaks in words: "Likely D minor, also possible A minor, not sure yet", "C minor, fairly sure", "C minor", or "No clear key", which is an answer in its own right. Fewer than four different notes never read better than "not sure yet". The lines between the words (`FAIRLY_SURE`, `SURE`, `NO_CLEAR_KEY`, `TIE`) are provisional until phase 0C checks them against labelled melodies; until then no number is shown.
- **Chords** (`chord.rs`): 12 qualities on 12 roots. A `ChordTimeline` is spans with their own start and length from tick 0, with no gaps; a span may last a beat or the whole clip (drones), and may say "no clear chord". Each span says whether it was heard, suggested (shown dashed) or set by the musician.
- **The harmony question** (`harmony.rs`): every part asks "what is the harmony here?" through the `Harmony` trait. A `Resolver` answers from the first source that applies: the chord held (live tools), then in song mode the song section's chords, on the clip grid the row's Harmony line, then suggested chords, else no clear chord. A row's chords run for the row's length (as `mpfree_seq::scene_length` defines it) and a line shorter than the row repeats; `row_tick` places a shorter clip's notes on each pass, so a two-bar bass in an eight-bar row meets four stretches of chords.
- **Groove** (`groove.rs`): played drums as residual timing. See the timing contract below.
- **The listener** (`listen.rs`, phase 0C): notes read as a key and a chord timeline, with the second prototype's numbers. Every note is weighted by how long it sounds and where it starts (half as much again on a bar line, a fifth more on a beat) and compared with the six modes in all 12 keys; the home note is picked by the cues a player uses (the last note, the first strong note, the lowest and longest, each bar's first note, a note held at a bar's end), the evidence is capped at 24 weighted notes, and a mode prior favours major and minor. Chords are a semi-Markov search over beats: every span from one beat to the whole of what was read, against 144 chord shapes and a 145th, no chord, scored by the notes that fit, the notes that don't, the chord notes missing and the lowest note on the root, plus how long chords usually last. A beat's rest stays inside its chord; a bar of silence, or notes that fit no chord (`NO_CLEAR_CHORD`), is no clear chord. `chord_of` names the one chord a handful of notes make. The tests hear the prototype's examples as it did: D F A is "Dm" and "Likely D minor, also possible A minor, not sure yet", the riff "C minor, fairly sure", the Dorian tune "D Dorian, fairly sure", and Cm7 F7 | Bbmaj7 Ebmaj7 | Am7b5 D7 | Gm6 exactly. A full reading (16 bars, 2,048 notes) takes about 6.5 ms on one desktop core; MPC timings come with the rest of 0C.
- **The context** (`context.rs`): the key, chords, groove, the pitch classes played, motifs (their place only, until 0C), and every clip read with its `Revision`, so a stale reading is never used (`is_stale`). `ReadBudget` caps what a reading reads. The context only compares revisions; the composer makes them (see "Previews and Keep" below).

## The model

A part keeps its **decisions** apart from its **notes** (`event.rs`). A `NoteDecision` is everything about a note except its pitch: when, how long, how loud, its `Role` over the chord (a chord tone, colour, scale note, approach, passing, neighbour, ghost or drum hit), its `Motion`, the motif it comes from, a pitch that must not change (a locked note, a note held into a selection, a drum pad), the drum lane whose groove it follows, its timing offsets and its `Source` (you, mpfree, or an arp pattern). Only `Source::You` notes are ever learned from.

`realize` turns decisions into notes under a harmony, a key (or none) and an instrument, with no dice: the same input always makes the same notes. Anchors (chord tones, colour and scale notes) are placed first, each following its motion from the anchor before it; approach notes then take a half step into the next note, and passing, neighbour and ghost notes lean on the notes either side. With no clear chord, chord-tone roles keep to the pitch classes played, so nothing is invented. Lengths follow the instrument.

`read` (`read.rs`, phase 0C) reads clips into a Musical Context as they play: notes after quantise and speed, a shorter clip looping over the longest as a row plays them; clips of notes give the key and chords, drum clips the groove (heard with playback's swing, which the template then takes out); every clip goes in with its revision, and a pitched part made of pad hits is left out and said so, since a pad's pitch depends on the kit.

`decisions_of` reads decisions back out of plain notes: each note's role over the chord it was heard on, and its motion. Realising them under the same harmony gives the same notes for diatonic material, so "Fit to the new chords" works on any clip, including one written by hand.

## Implementation contracts

A design review set four must-haves before the code that needs them, and six more to settle early. The table says where each stands.

| Contract | Due | In 0A |
|---|---|---|
| 1. Try again respects both neighbours | Before phase 1 | The prototype's result is recorded below; the scoring is phase 0C's |
| 2. What a change re-fits, and stable dice | Phase 0 | Built: `refit.rs`, `realize.rs`, `dice.rs` |
| 3. One timing contract | Phase 0 | Built: `timing.rs`, `groove.rs`, `keep.rs` |
| 4. One harmony question for every place | Before phase 2 | Built: `harmony.rs` |
| 5. Saying how sure it is | Phase 0 | Built: `key.rs` (words), `chord.rs` (no clear chord, long chords) |
| 6. Three kinds of rule | Phase 0 | Built as data: `rules.rs` |
| 7. Preview states and Keep | Before phase 1 | Built in 0B: `bind.rs`, `preview.rs`, `keep.rs`; checked through the real engine (`tests/playback.rs`) |
| 8. Recording and where notes came from | Before phase 3 | `Source` on every event; recording is phase 3's |
| 9. Live: chord settle, note owners, release | Before phase 3 | Phase 3 |
| 10. Instrument profiles | Data in phase 0 | Built: `profile.rs` |
| 11. Styles made of parts | Phase 0 data model | Built: `style.rs` |
| 12. Reading costs and memory | Phase 0 | `ReadBudget`, revisions and stale checks; the listener reads inside the budget and says when it was cut short (0C) |

### What a change re-fits (contract 2)

Two kinds of change: making new decisions (regenerating) and playing the same decisions under new conditions (re-fitting). `effect` is the table: a chord, groove, instrument or playing change re-fits; a motif, style or activity change regenerates what it touches. No entry redoes what it keeps, and every entry keeps locked notes and the Keeps. `action` says what happens to one part: in a preview it is re-fitted or regenerated at once; locked notes are left; a kept clip is never rewritten. When a chord change leaves a kept clip's notes off the new chords (`misfits`: chord tones on a beat that no longer are), it is marked and offered "Fit to the new chords", as one step UNDO takes back.

Chance comes from stable names (`DiceName`): the seed, the part, the phrase, the job, the bar and the try, mixed with SplitMix64. Rolling more dice for one bar never changes another, so trying bar 6 again leaves bars 7 to 16 as they were. New jobs are added at the end of `Job` so existing rolls stay the same.

### The timing contract (contract 3)

Every event's time is built in one order:

1. its musical position, in ticks on the clip's grid;
2. its groove offset: the drummer's residual timing at that step, as a share of a 1/16;
3. its playing offset, in ticks; 2 and 3 together stay within ±30 ms (`LIMIT_MS`);
4. playback's own quantise, speed and swing, as the play view applies them (`ClipView::note_view`, then the player's run collection in `mpfree-play`).

Steps 1 to 3 are written into the clip (`written`); step 4 is playback's (`heard`). A note pushed before the clip's start is written at its end, where playback plays it just before the next pass, as quantise already does with a note at the end.

Swing is a playback setting (the clip's, the track's or the transport's), so a `GrooveTemplate` never holds swing: each cell is the drummer's timing once the swing playback applies has been taken out (`unswing`, the exact inverse of `Swing::apply`). `Timing::from_groove` moves a template read against one swing onto the clip's swing first (`GrooveTemplate::against`), keeping where every hit lands. The tests check both ways round: grooved notes land within a millisecond of the drummer whether the groove was read against straight or swung playback, and offsets measured from the straight grid would have swung the hats twice (more than 15 ms off).

A clip whose quantise has a strength above 0 pulls grooved notes back towards its grid; `check` says so before Keep, in words. A style's slow drift is used only where the style asks for it (0 in Minimal) and never over a played groove (`drift_on`).

Keep writes `clip_notes`: the written ticks, all or nothing inside `MAX_NOTES` and `MAX_CLIP_TICKS`. The preview plays the same written notes through playback, so the preview and the kept clip play every note at the same time; a test compares them note by note, and phase 0B's tests repeat it through the real play view and player: `heard` lands on the player's frame for every note, with quantise, half, double and 3:4 speed, the clip's swing or the transport's, and a clip started off the bar line.

### Previews and Keep (contract 7)

A preview is bound to one clip at one revision (`Binding` in `bind.rs`): its track and slot, its lasting id and its `Revision`. A revision is a hash of what the clip plays and what the composer reads (every field of every note, the length, speed, swing, quantise, direction, what the notes stand for), worked out from the clip when asked; its name, locks, moves and launch settings are left out, and every field of `Clip` and `Note` is named so a new one must say whether it is in. It is a sum of one hash per note, so notes stored in another order, as UNDO puts them back, are the same revision. A counter kept with the clip was the other way, but every path that edits a clip (step edits, recording in place, undo, captures, Mutate, loading) would have to move it on, and one that forgot would let a stale result be kept; a hash can't forget.

`Preview` (`preview.rs`) is the state machine the app's control thread will drive:

| State | What the host does |
|---|---|
| Idle | Nothing |
| Preparing | A request is out, numbered; a result for an older request is dropped. A result that was playing (Try again while listening) plays on until the new one arrives |
| Ready | A result is in, not playing yet: auditions it at the next bar line |
| Auditioning | Compiles the play view from `audition`'s project: the result's edit applied to a copy of the clip with the same `Clip::apply` Keep's take uses |
| Committing | Writes the edit as one take (`mpfree_play::Takes::edit`, the transaction step edits use) and compiles; a refused Keep writes nothing and leaves no preview running (Ready again, with the refusal in words) |
| Cancelling | After Back while it played: compiles the clip's own view, then says it is in |
| Invalidated | The clip, or a clip the reading listened to, changed or went: nothing can be kept, and Try again asks afresh |

The last 8 results for the clip are kept, and ‹ › steps through them (`step`); Keep writes the one shown. Try again while a result plays keeps it playing until the new one arrives, so the music doesn't drop out (the design's table had the clip come back while preparing; this is the one change, made because a gap on every Try again would be heard). The binding is checked when a result arrives, after every edit (`clip_changed`) and once more at Keep, against the bound clip and every clip the reading listened to. Continue's edit (`keep::continuation`) makes the clip longer and adds the new notes as one `Edit`, so one UNDO takes back the length and the notes together. The audition copies the whole project, a stand-in for phase 1, where results play through the shared audition layer the piano roll's previews build (its step 2B.3).

### Rules, styles and instruments (contracts 6, 10, 11)

`Rule::kind` sorts every rule into structural (a real pitch and time, the instrument's range, the clip limits), yours (locks, chords you set, the Keeps, the Range you picked) and preferences (leap size, chromatic motion, the four parts of an ending, repeats). Only preferences give way, weighed by the style's `Weights`, so a leap over an octave is rare rather than banned and Jazz-ish can use enclosures. When the others can't all be met, `Unmet::words` says which setting is in the way and suggests one change: "Can't fit a new bar 6 between your bars with Range Narrow. Try Range Medium?"

A `Style` is components (harmony, rhythm, bass, melody, playing, development) plus weights and drift; `Style::with` swaps one component, so jazz harmony can go without the walking bass. The three presets (`MINIMAL`, `GROOVE`, `JAZZISH`) carry the second prototype's numbers, tuned by ear, with each style's swing as a suggestion for the clip's playback swing and a few plan shapes each.

Five `InstrumentKind`s give a `Profile`: polyphony, range, articulation, overlap, velocity response, slides and whether length matters. `realize` keeps notes in the profile's range and takes lengths from it: a bass and a one-note lead end before the next note (the lead legato), pads may overlap, hits get a fixed short length.

### Try again on one bar (contract 1)

Trying again on a bar is filling a gap: each try is scored on its inside, its way in and its way out, everything outside the selection stays (including a note held into it), and a loop's last bar leads into its first. The second prototype measured it: over 40 seeds, four bars each and three tries, big steps (more than a major third) into the next bar fell from 28 % with the way out ignored to 17 % with it, the same as music written in order (16 %); at the end of a four-bar loop, big steps back into bar 1 fell from 53 % to 17 %. Phase 0C builds the scoring; the dice contract above already keeps the other bars as they were.

## Tests

`cargo test -p mpfree-context -p mpfree-compose`. Each module has its own tests; `rust/mpfree-compose/tests/contracts.rs` is phase 0A's gate:

- a chord change re-fits the bass, keeping its rhythm, lengths, loudness, timing and shape, with the new chord's tones where the roles ask for them;
- a kept clip is marked and offered a fit, never rewritten, and the fit read back from its notes moves only what no longer fits;
- an instrument change moves range and lengths only;
- with no clear chord nothing is invented;
- new notes land on the drummer with swing applied once, and total offsets would have swung twice;
- the preview and the kept clip play every note at the same time;
- trying one bar again changes only that bar, in any order;
- the same request makes the same notes.

`rust/mpfree-compose/tests/playback.rs` is phase 0B's, through the real play view, player and take undo (`mpfree-play` is a test-only dependency):

- `heard` lands on the player's frame for every note, in eight playback set-ups;
- a one-bar phrase, realised from decisions under the chord, is heard in place of a one-bar riff, kept as one take that plays the same notes on the same frames, undone back to the riff (length, notes and revision), and redone;
- a step tapped on the clip while its result plays invalidates the result, and the tap's UNDO doesn't bring it back;
- a step tapped on a clip the reading listened to invalidates it too.

## What phase 1 decides

- Where the app keeps the `Preview` and drives it: the control thread's step edit path (`TimeControl::step_action`) already compiles after a take and takes the edit back if the view can't be built, and Keep should go the same way.
- Playing results through the shared audition layer (piano roll step 2B.3) in place of the project copy, from the next bar line.
- What Keep says when the clip plays in more than one place, or when a longer clip makes its row play longer.
