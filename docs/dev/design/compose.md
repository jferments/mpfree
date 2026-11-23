<!--
covers: rust/mpfree-context rust/mpfree-compose rust/mpfree-app/src/time/compose.rs rust/mpfree-time-ui/src/continuing.rs
checked: 2026-11-22 d9a2b851
-->

# The composer and the Musical Context

**Status:** phases 0A (the musical model) and 0B (the check through real playback), and phase 0C's engine: the listener, plan, new chords, phrases, playing, Continue end to end, the simpler ways it is tested against, the listening set and the timing bench, 2026-11-22. Phase 0C's gate: the owner's first listen (2026-11-22, 3 of 20 blind cases and the 32 bars) found the output mechanical, "chiptune" in sound, short of rhythm and variety, and asked for work to go on with a sound and rhythm pass later. Phase 1's first step, Continue in the time engine with no screen yet, is built: a result heard in place of its clip from the next bar line, kept as one take, or left with Back ([below](#continue-in-the-time-engine-phase-1)). The code is [`rust/mpfree-context`](../../../rust/mpfree-context), [`rust/mpfree-compose`](../../../rust/mpfree-compose) and, in the app, [`time/compose.rs`](../../../rust/mpfree-app/src/time/compose.rs) with its part of `TimeControl`. Nothing is on screen and nothing has run on the MPC. The owner approved the design on 2026-11-22 with Continue as the first tool.

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
| 0C | The full engine: listening, plan, harmony, phrase, playing, groove; three styles; two-sided Try again; the reading budget; listening tests against simpler baselines; MPC timings | Built: listening, the plan, new chords, phrases, two-sided Try again, playing and groove, Continue end to end, the simpler ways, the listening set (`examples/listen.rs`) and `mpfree-bench --compose`. Gate: the owner's blind tests and 32-bar listen; MPC timings estimated, measured at the next bench round |
| 1 | Continue, with the preview states and an all-or-nothing Keep | Step 1 built: the composer on its own thread, results heard in place of the clip on bar lines, Keep as one take, Back, and every change or REC stopping a result ([below](#continue-in-the-time-engine-phase-1)). Next: step 2, the Continue panel in the note editor; step 3, Try again on picked bars or notes and locked bars; step 4, Continue on Steps and the clip card. Gate: a device round (preview, Keep, UNDO, scene changes, the clip limits) |
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

`read` (`read.rs`, phase 0C) reads clips into a Musical Context as they play: notes after quantise and speed, a shorter clip looping over the longest as a row plays them; clips of notes give the key and chords, drum clips the groove (heard with playback's swing, which the template then takes out); every clip goes in with its revision, and a pitched part made of pad hits is left out and said so, since a pad's pitch depends on the kit.

## The model

A part keeps its **decisions** apart from its **notes** (`event.rs`). A `NoteDecision` is everything about a note except its pitch: when, how long, how loud, its `Role` over the chord (a chord tone, colour, scale note, approach, passing, neighbour, ghost or drum hit), its `Motion`, the motif it comes from, a pitch that must not change (a locked note, a note held into a selection, a drum pad), the drum lane whose groove it follows, its timing offsets and its `Source` (you, mpfree, or an arp pattern). Only `Source::You` notes are ever learned from.

`realize` turns decisions into notes under a harmony, a key (or none) and an instrument, with no dice: the same input always makes the same notes. Anchors (chord tones, colour and scale notes) are placed first, each following its motion from the anchor before it; approach notes then take a half step into the next note, and passing, neighbour and ghost notes lean on the notes either side. With no clear chord, chord-tone roles keep to the pitch classes played, so nothing is invented. Lengths follow the instrument.

`decisions_of` reads decisions back out of plain notes: each note's role over the chord it was heard on, and its motion. Realising them under the same harmony gives the same notes for diatonic material, so "Fit to the new chords" works on any clip, including one written by hand.

## The plan and the chords (phase 0C)

`Settings` (`settings.rs`) carries the controls in the design's words: Style, Similarity, Variation, Activity, Range, Keep, Bars, Ending and the seed.

`plan` (`plan.rs`) gives every bar a job, a tension aim, an activity and a similarity aim before any note. Jobs are yours, repeat, loop, vary, sequence, answer, develop, contrast, return and land (home, open, or leading back). A style's plan shapes are of two kinds: its pattern sets the jobs (Minimal loops, Groove calls and answers, Jazz-ish builds sentences), and its tension curve, picked by the seed from the style's own (one climax, small waves, a slow build), sets the rise and fall. Sixteen bars or more get a contrasting phrase and a return. With the second prototype's numbers, the test's 16 Jazz-ish bars contrast in bars 9 and 10 (similarity 0.31), peak in bar 12 (tension 0.975), return in bar 13 (aiming at 0.91) and land home in bar 16 (0.125), as the prototype measured.

`progression` (`progression.rs`) makes the chord timeline over every bar. Your bars keep the chords heard in them; a repeat or a return plays your bar's chords; every other new bar takes chords from the style's palette (diatonic triads or sevenths on its degrees, plus a borrowed V in minor for Jazz-ish), cut into the style's harmonic rhythm (two bars, one bar, half a bar). A landing bar's last beat takes the ending's chord. The chain is sampled by forward filtering and backward sampling from your last chord, weighted by the style's stay and root-motion weights, so it never dead-ends and Try again (the `attempt` in the dice name) gives another good progression; Variation flattens or sharpens the weights. New chords are marked suggested, shown dashed. A loop bar plays your riff over the style's chords rather than copying yours, so Minimal mostly stays on Cm and sometimes moves to Bb or Fm and back. With no clear key, the new bars play your bars' chords again in order, and nothing is invented.

## Phrases (phase 0C)

A phrase is written as a tune of places in the scale on the 1/16 grid (`motif.rs`): `Key::position` turns a pitch into a scale place and how far it is raised, so moving a motif two places is a sequence that stays in the key. `motifs` cuts a line at its phrase breaks (Cambouropoulos's boundary model: changes in leap, time between starts and rests); `similarity` says how alike two stretches are, whatever key they're in (0.35 leaps and lengths as an edit distance, 0.25 shape, 0.3 rhythm, 0.1 accents); `transform` has the classic changes (move, invert, backwards, augment, diminish, head, tail, shift, widen, end on).

`rhythm.rs` keeps, varies, reshapes and makes a bar's rhythm on any meter's grid (`Grid`: the 4/4 table of strong and weak steps generalised to any bar and beat). `vary` splits, joins and nudges notes; `feel` is the style's: Minimal keeps the strongest notes, Groove pushes beats a 1/16 early and chops long notes, Jazz-ish moves notes to 1/8s and anticipates beats; `fresh` makes a new rhythm for a contrast near the style's syncopation (Longuet-Higgins and Lee) and away from yours.

`phrase.rs` writes the new bars. `Phrasing::new` reads your notes (only notes a person played; anything the composer or an arp made is left out), learns how you move from note to note (a variable-order model of scale steps), and finds the middle of your tune and the notes you use most. For each new bar, `bar_material` takes the rhythm and a target from the bar's job: your bar again (a loop varies half the time), varied, moved up or down the scale, the first half of the call with the second half of your other bar ending elsewhere, the opening motif twice a step apart, or a new rhythm under your tune upside down. `fit` then picks the notes with a beam search (12 lines kept at each note) over every pitch in range, scoring your steps blended with the style's, the chord under it, leaps and a turn after one, repeats, register following the bar's tension, the target's place and steps, and, for the last note, the way into the next bar and the ending; Gumbel noise scaled by Variation makes each try different. The finished line nearest the bar's similarity aim wins. With no clear key, new notes keep to the notes you played.

Contract 6's kinds hold: the Range you picked (`RangeSetting::steps`, scale places either side of your tune's middle), the instrument's range, locked bars and the Keeps never give way, and when they can't all be met the request is refused with the setting and one change ("Can't fit a new bar 3 between your bars with Range Narrow. Try Range Medium?", or the instrument's range when no wider setting helps). Keep Rhythm holds every new bar's rhythm as your bar's; Keep Tune shape makes every step go the way yours did. A leap over an octave, a chromatic note that doesn't move on by a half step and the ending's last note (home: the home note, or less well its 3rd or 5th; open: the 2nd, 5th or 7th; leading back: near your first note or the 5th) were hard rules in the prototype and are preferences here, weighed by the style; when one gives way the result says so (`Line::gave_way`), so Jazz-ish enclosures show up there and nothing is hidden.

`continue_line` writes every new bar in order, each with its own dice (`Job::Rhythm` and `Job::Pitch`, by bar and try); `try_again` remakes one bar with everything else locked; `decisions` reads the new bars back as decisions under the chords, so a later chord change re-fits them, and realising them gives the same notes for all three styles. A 16-bar line takes 1 to 2 ms on one desktop core (9,400 to 21,000 candidates scored); MPC timings come with the rest of 0C.

## Playing, and Continue end to end (phase 0C)

`playing.rs` plays the new notes once their pitches are set, after the KTH performance rules as the second prototype tuned them, and writes the result into each note's decision, so a chord change re-fits the notes and keeps how they were played. Your touch (`Touch`) comes first: the new notes start from how you played your bars, their mean loudness moved three quarters of the way from the style's to yours, and the style's gate three quarters of the way to your notes' length against the gap to the next note (less when you played fewer than eight notes to go by). The first renders for the listening tests showed why: Groove carried a soft legato riff on louder and shorter, like another player, and repeating the riff would have won on that alone. Loudness: the style's level and accents per 1/16, a rise and fall over every four bars, the bar's tension, melodic charge over the chord (Friberg), contour peaks and notes after a leap, higher notes a little louder; how far these move a note is the instrument's velocity response, and Keep Accents leaves your loudness. Length: the gate, repeated notes shorter, short notes a little shorter and softer, and a landing note held, then a breath. Timing (`timing`): with drums read, each note on the grid takes the drummer's residual at its step for its lane (a bass the kick's, the rest the hats'), moved onto the swing the clip plays with; with none, the style's slow drift where it has one (Groove and Jazz-ish, 6 ms; never Minimal): one smooth 1/f curve (`drift`) drawn from the request's seed alone, so every part of a request drifts together.

`request.rs` is Continue end to end. `continue_notes` takes your notes in the clip's ticks, a reading and the settings: the plan for your bars plus the new ones, the chords (Keep Chords plays your chords again), the phrase, the decisions, the playing, and the notes Keep writes, with the clip's new length. `try_bar` makes one new bar afresh and keeps every other note exactly as it was, played as it was. `continue_clip` reads a clip and the rest of its row from a project (a pitched clip of pad hits is refused, since a pad's note depends on the kit), maps the chords onto the clip's own ticks at its speed, takes its notes after quantise, and returns the `Proposal` a preview plays and Keep writes, bound to every clip it read. A refusal says why in words (`Refused::words`). Carrying 16 bars on takes 1 to 2.2 ms on one desktop core after the reading.

## Simpler ways and the listening tests (phase 0C)

Each stage has to beat a simpler way of doing its job by ear, or it is reworked or dropped for the simpler way. `baseline.rs` makes the simpler ways, each returning the new notes as Keep would write them:

| Job | Played beside |
|---|---|
| Continue a melody | your phrase repeated exactly as played (`Simpler::Repeat`), or moved a scale step up, then down (`Simpler::Moved`) |
| Styles | random notes from the key's scale around your tune, in rhythms from the style's density and syncopation, played with the style's accents on your touch (`Simpler::Random`) |
| Groove | the same notes on the grid (a fixed swing when the clip swings), or with ordinary humanize (`regroove`) |
| 16 bars | the first four bars repeated with small random changes (`motif_repeated`) |
| Try again on a bar | the same bar made without looking at the bar after it (`try_bar_one_sided`) |

A bass beside root notes on the kicks comes with Add a part (phase 2). Their tests check the shape of each, and that mpfree's new bars keep closer to the chords and to your tune than random notes do.

`rust/mpfree-compose/examples/listen.rs` renders the listening set as files (workstation only; it reads only the MIDI file it is given and writes into one new folder):

```text
cargo run --release -p mpfree-compose --example listen -- --demo
cargo run --release -p mpfree-compose --example listen -- --midi my-clip.mid [--from 1] [--bars 2] [--style groove]
```

`--demo` uses four built-in phrases (the prototype's C minor riff, an F major tune, a swung A minor line, a D Dorian waltz in 3/4); `--midi` reads your own clip from a MIDI file (its first track with notes, the top note where notes start together). For each phrase it writes every case as a WAV and a MIDI file of what is heard: Continue beside each simpler way, unlabelled and shuffled into A and B; the three styles as X, Y and Z; mpfree's feel beside the plain ones; 16 bars beside the repeated motif; two Try again pairs; a 16-bar continuation for "do you hear your motif?"; and about 32 bars (Continue by 16, then by 16 again) with the chords as a quiet pad. Every file plays on the same sounds at the same level (Keys "Mellow Tines", a plain beat on the drum synth, Strings "Ballad Pad" for the pad), and the pairs have no chords under them, so neither side is helped by the composer's own. `answers.json` says which file is which and `report.txt` lists every case. The owner's listening page plays a selection of 20 of them blind, asks "which would you keep?" and the eight ratings (intentional, keeps the idea, suits the style, harmonic sense, groove, develops, avoids monotony, would you keep it), and reveals the answers only at the end.

What a request costs is `mpfree-bench --compose` ([Profiling](profiling.md#the-composer---compose)): about 1.8 ms on a desktop core for an 18-bar Continue, so about 18 ms on the MPC by the calibration (23 ms at worst), and a background job never on the audio thread.

## Continue in the time engine (phase 1)

Phase 1's first step puts Continue in the app with no screen yet: `TimeAction::Continue` carries a `ContinueAction` (`rust/mpfree-time-ui/src/continuing.rs`): Ask (Continue, or Try again on everything, with the settings, the kind of instrument and the kit's drums), Step (‹ ›), Keep and Back. The time engine's control half runs it (`TimeControl`'s Continue section in `time/control.rs`, with `Composing` in `time/compose.rs`) and publishes `TimeState::continuing`: the state, the shown result (what was made, with each bar's job and the chords, and the clip as heard with it), where the new bars start, which of the last 8 is shown, whether it plays and the bar line the music changes on next, and why nothing can be heard or kept when that is so.

**The composer off the music's threads.** An ask copies just what the reading needs into a `ContinueJob` (`job.rs`): the tracks it reads, each with only its read clip, at its place, with the track's swing; nothing else of the project. That goes to a thread of its own, started on the first Continue, at idle priority on the helper cores (`mpfree_lanes::os::idle_job`), which runs only the newest job waiting. The control half takes the answer on a later period and checks it against the project as it is then (`Preview::arrived`); one overtaken by a newer ask, or made from a clip that changed meanwhile, is dropped and counted. The rest of the row is read from the clip's row on every other heard track of notes, a clip of pad hits as drums; the kit's drums (`Lanes`) come with the ask, from the screen that knows the pad names.

**Heard in place of the clip.** While a result is in the view, `TimeControl::compile` swaps the result's clip into the clip's slot for the view's compile only, and back (no copy of the project). The music changes on the next bar line while the clip plays and the transport rolls (a line within the next period is as good as now, as Reload clip does), and at once otherwise:

| What happens | When the music changes |
|---|---|
| A result arrives (or Try again's next one, or ‹ ›) | On the next bar line; the last result plays on until then |
| Back | On the next bar line: the clip as it was |
| Keep | At once: the view is compiled from the kept clip, which plays the same notes |
| The clip, or a clip the reading listened to, changes (an edit, a recording, UNDO) | At once: the clip as it now is, with "The clip changed: try again?" |
| REC | At once, before the take starts; the result stays ready, and none comes in while recording |
| Stopped, or the clip doesn't play | At once |

A result's tick 0 falls where its clip started (the launch's `start`): the launch's origin is anchored for the longer clip, so a riff carried on from 2 bars to 4 lines up with the 4-bar part that started with it, as the reading heard them. Without it the new length would play from wherever the old length's origin happened to fall (a 1-bar clip's origin is any whole bar back), which could put your first bar over the other part's third. Back gives the launch its origin back; Keep leaves it, in time with the kept clip.

While Continue is under way, edits compile rather than go in place, so each compile checks the result against its clip (`Preview::clip_changed`) and no result in the view is edited in place. A recorded change checks it each period it lands.

**Keep.** The shown result's clip goes in as one take, `Takes::replace_clip` with `Replaced::Continue { bars }` beside Mutate and Restore, as the design asked; the Takes list says "continue" and "2 more bars". Phase 0B kept through `Takes::edit` with the same `Edit`; the whole clip replaced is the same notes and length, puts the clip back whole on UNDO, and says what it was. A refused Keep (the clip changed, or its own limits) writes nothing and leaves no result playing. Keep is refused while recording.

**What gave way is counted** and goes on the status line once anything was asked: "continue: 3 asked, 3 made, 1 stopped by a change, 1 stopped by REC" (asked, made, refused, overtaken, made from a clip that changed, stopped by a change, clips left out of readings, keeps refused, stopped by REC). They move to the system status monitor with the rest when it takes the time engine's counts.

**Agreed with the piano roll** (22 Nov): Continue becomes a tool in the note editor's Edit options drawer (`ToolKind::Continue`), working like Strum or Humanize: the Q-Links while you hear it, Done for Keep, Cancel for Back. Its preview is the clip `TimeState::continuing` publishes, not a tool edit; the roll takes its length from it and draws the new bars from `new_from` in the faint mix the other clips use, with each bar's job on the ruler and the tries under the options where Takes sit. No new bottom button. That is step 2, with its layout spec change for the owner.

## Implementation contracts

A design review set four must-haves before the code that needs them, and six more to settle early. The table says where each stands.

| Contract | Due | In 0A |
|---|---|---|
| 1. Try again respects both neighbours | Before phase 1 | Built in 0C: `phrase.rs` scores the way in and the way out, and a loop's way back; measured below |
| 2. What a change re-fits, and stable dice | Phase 0 | Built: `refit.rs`, `realize.rs`, `dice.rs` |
| 3. One timing contract | Phase 0 | Built: `timing.rs`, `groove.rs`, `keep.rs`; new notes take the groove or the drift in 0C (`playing.rs`) |
| 4. One harmony question for every place | Before phase 2 | Built: `harmony.rs` |
| 5. Saying how sure it is | Phase 0 | Built: `key.rs` (words), `chord.rs` (no clear chord, long chords) |
| 6. Three kinds of rule | Phase 0 | Built as data: `rules.rs`; the phrase keeps them (0C) |
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

The last 8 results for the clip are kept, and ‹ › steps through them (`step`); Keep writes the one shown. Try again while a result plays keeps it playing until the new one arrives, so the music doesn't drop out (the design's table had the clip come back while preparing; this is the one change, made because a gap on every Try again would be heard). The binding is checked when a result arrives, after every edit (`clip_changed`) and once more at Keep, against the bound clip and every clip the reading listened to. Continue's edit (`keep::continuation`) makes the clip longer and adds the new notes as one `Edit`, so one UNDO takes back the length and the notes together. Phase 0B's test auditions by copying the whole project; the app doesn't (phase 1, below): its compile puts the result's clip in the clip's place just for the compile. The piano roll's tool previews turned out to be drawn only, so there was no shared audition layer to play through, and phase 1 built the substitute; the piano roll will use it for its tool previews.

### Rules, styles and instruments (contracts 6, 10, 11)

`Rule::kind` sorts every rule into structural (a real pitch and time, the instrument's range, the clip limits), yours (locks, chords you set, the Keeps, the Range you picked) and preferences (leap size, chromatic motion, the four parts of an ending, repeats). Only preferences give way, weighed by the style's `Weights` (a leap costs `leap` per scale step beyond a fourth, more again over an octave), so a leap over an octave is rare rather than banned and Jazz-ish can use enclosures. When the others can't all be met, `Unmet::words` says which setting is in the way and suggests one change: "Can't fit a new bar 6 between your bars with Range Narrow. Try Range Medium?"

A `Style` is components (harmony, rhythm, bass, melody, playing, development) plus weights and drift; `Style::with` swaps one component, so jazz harmony can go without the walking bass. The three presets (`MINIMAL`, `GROOVE`, `JAZZISH`) carry the second prototype's numbers, tuned by ear, with each style's swing as a suggestion for the clip's playback swing and a few plan shapes each.

Five `InstrumentKind`s give a `Profile`: polyphony, range, articulation, overlap, velocity response, slides and whether length matters. `realize` keeps notes in the profile's range and takes lengths from it: a bass and a one-note lead end before the next note (the lead legato), pads may overlap, hits get a fixed short length.

### Try again on one bar (contract 1)

Trying again on a bar is filling a gap: each try is scored on its inside, its way in and its way out, everything outside the selection stays (including a note held into it), and a loop's last bar leads into its first. The second prototype measured it: over 40 seeds, four bars each and three tries, big steps (more than a major third) into the next bar fell from 28 % with the way out ignored to 17 % with it, the same as music written in order (16 %); at the end of a four-bar loop, big steps back into bar 1 fell from 53 % to 17 %. Phase 0C built the scoring (`try_again`), and its test repeats the measurement on the same riff and plan: over 40 seeds, trying bars 3, 6, 10 and 13 three times each, big steps into the next bar are 14 % with the way out scored and 28 % without (8 % in the line as first written), and at the end of a four-bar loop big steps back into bar 1 are 16 % with the wrap and 39 % without. The other bars stay exactly as they were. The listening set counts how often scoring the way out changes the bar at all: over every new bar of a 16-bar continuation and 20 tries each, 15 tries in 100 on the riff and 1 to 7 on the other three phrases; the rest pick the same bar either way. Its blind pairs are tries where the two differ.

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
- a step tapped on a clip the reading listened to invalidates it too;
- Continue end to end (phase 0C): a swung riff and a hat part read from a project, carried on two bars in Groove, heard where `heard` says, every new note on an off-beat 1/8 landing with the drummer's late hats (not where the clip's swing alone would put it), kept as one take that plays what was heard, and undone.

Phase 1's are in the app (`cargo test -p mpfree-app --bin mpfree -- continue try_again an_edit_or_rec stopped_a_result`), through the real time engine with the composer on its own thread, plus `job.rs`'s:

- Continue on a playing riff comes in on the next bar line, in place of the clip and in time with where it started; nothing is written until Keep, which is one take (`Replaced::Continue`) that plays on unchanged, and UNDO takes back length and all;
- Try again plays the last result on until the next has come and its line; ‹ › step on bar lines; Back brings the clip back on the next line as it was, and writes nothing;
- an edit to the clip while its result plays stops it at once, the edited clip plays, and Keep is refused; REC brings the clip back before the take, the result stays ready, and none comes in while recording;
- stopped, a result comes in at once and Back is at once; a clip of pad hits and an empty slot are refused with why;
- with the halves apart (Arrange 2.6b), asked, heard and kept the same way;
- a job's copy holds only what is read and answers as the whole project would; a missing other clip is counted as left out.

Phase 0C's modules carry their own tests, among them: the listener hears the prototype's examples as it did; the plan's 16 bars match the prototype's numbers; a progression keeps your chords and lands; a 16-bar phrase stays in range, lands home and sits within 0.03 of each bar's similarity aim on average; only your notes are learned from; Try again changes one bar and leads into the next (the measurement above); the Keeps hold; a range that can't fit is refused with one change; bars of three beats stay in their bars; the playing rules ring out peaks, shorten repeats and hold the landing; new notes start from your touch (three quarters of the way to your loudness, a longer gate after legato playing); the simpler ways fill the same bars, the repeat exactly as played, and mpfree keeps closer to the chords and your tune than random notes; the drift is smooth, shared and only where the style has one; the three styles carry the riff on each in its own way (Groove busiest and most syncopated, Minimal thinner, Jazz-ish on 1/8s); trying a bar again keeps every other note.

## What phase 1 decides

Decided in step 1: the `Preview` lives in the control half beside the takes and Keep compiles after its take and takes it back if the view can't be built, as step edits do; results play through a substitute in `TimeControl::compile` on bar lines, in time with where the clip started. Still open:

- What Keep says when the clip plays in more than one place, or when a longer clip makes its row play longer.
- Continue carries on a clip's top line; a clip of chords waits for phase 2's comping, and until then Continue on one is a melody over its chords or is refused (phase 1 picks).
- Whether a new clip takes the style's suggested swing (phase 2, Add a part); Continue leaves the clip's swing as it is.
