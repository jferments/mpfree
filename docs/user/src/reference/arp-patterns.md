<!--
status: changing
covers: rust/mpfree-arp/src/groups.rs rust/mpfree-arp/src/pattern.rs rust/mpfree-arp-ui/src/widgets.rs
checked: 2026-11-23 d9540108
-->

# Arp patterns

These are the patterns MPFree comes with, as the Arp panel's Pattern mode lists them (see [Patterns](../making-music/note-repeat-and-arp.md#patterns)). Every one plays the notes you hold or latch, so it follows your chord: **1** is the lowest note held, **2** the next, **x** the next note in the arp's order, **l** and **h** the lowest and highest, and **\*** the whole chord. Most are one bar of sixteenths at the default rate of 1/16. Your own patterns go after these, under **My patterns**.

The steps column is the pattern written as text, as the workspace and your patterns file write it: **.** is a rest, **\_** a tie, **!** an accent, **+** and **-** an octave up or down, **:3** three hits in the step, **%50** a 50 % chance, **'** a short note and **~** a long one. After a **|** come lanes that loop on their own length: **oct** (octaves), **deg** (steps of the scale, or semitones with no scale picked), **vel** (velocity), **gate** (note length in percent) and **nudge**. A lane name with **~** and a number varies by up to that much each time.

> [!NOTE]
> MPFree wrote all of these for itself; none is copied from Akai's patterns. The rhythms with old names (tresillo, clave, Charleston) are centuries-old figures that belong to everyone.

## Starters

| Pattern | What it plays | Steps |
|---|---|---|
| Straight | Every step, the held notes in turn | `x` |
| Gallop | Two hits, a rest, a hit | `x x . x` |
| Bounce | First, third, second, third note | `1 3 2 3` |
| Climb | Three notes, then the same an octave up and two up | `1 2 3 1+ 2+ 3+ 1++ .` |
| Pulse | Accents and soft notes in a pumping pulse | `x! x x' x x! x' x x'` |
| Stabs | Chord stabs with rests between | `* . . * . . * .` |
| Rolls | Single hits, then quick rolls of two and three | `x x:2 x x:3` |
| Drift | Loose: random notes, chance steps and ties | `x x%60 ? x%30 . x _ ?` |
| Three over four | A three-note octave figure across a four-step bar, with its own accents | `x x x x \| oct 0 1 0 \| vel 120 70 90 70 80` |
| Climbing line | One held note becomes a scale line going up and back | `1 1 1 1 1 1 1 1 \| deg 0 2 4 5 7 4 2 \| gate 40 40 80` |

## Chords

The whole chord in rhythm, as the MPC's Rhythm action plays it.

| Pattern | What it plays | Steps |
|---|---|---|
| Offbeats | The chord on every offbeat, as a house stab | `. . * . . . * . . . * . . . * .` |
| Two and four | Accented chords on the backbeat | `. . . . *! . . . . . . . *! . . .` |
| Tresillo | Three hits over two beats (3-3-2), the first accented | `*! . . * . . * .` |
| Clave | The 3-2 clave across a bar | `* . . * . . * . . . * . * . . .` |
| Charleston | A hit on the beat and a short one just before the next | `*! . . *' . . . .` |
| Two-step | Short, skipping chords with a gap in the middle of the bar | `*' . . *' . . . . . . *' . *' . . .` |
| Long chords | Held chords: half a bar, then a quarter and an eighth | `*~ _ _ _ _ _ _ _ *~ _ _ _ *~ _ . .` |
| Funk comp | Short chords on the off-sixteenths | `. *' . *' *' . . *' . *' . . *' . *' .` |

## Gates

The chord chopped and pumped.

| Pattern | What it plays | Steps |
|---|---|---|
| Trance gate | The classic gated chord, two on and one off | `* * . * * . * . * * . * * . * * \| gate 60` |
| Pump | Quiet after the beat and swelling up to the next, like a sidechained pad | `. *' * *~ \| vel 50 70 95 120` |
| Stutter | Four straight, a gap, then doubled and quadrupled chords | `* * * * . . . . *:2 *:2 . . *:4 . . .` |
| Dotted chops | Chords three steps apart, against the beat | `* . . * . . * . . * . . * . * .` |
| Choppy | Short chords with a loud-soft swing | `*' *' . *' . *' *' . \| vel 120 80 100 70` |
| Swell | Eight chords growing louder and longer | `* * * * * * * * \| vel 50 60 70 80 90 100 110 120 \| gate 15 30 45 60 75 90 100 100` |
| Shimmer | A fast tremolo of the chord, loose in velocity | `*:3 *:2 *:3 *:2 \| vel~20 \| gate 40` |
| Morse | Short and long chords, like a code | `*' *' . *~ _ . *' . *~ _ _ . *' *' *' .` |

## Rolls

Rolls for hats and snares, or for any held note.

| Pattern | What it plays | Steps |
|---|---|---|
| Trap hats | Eighths with triplet and quick rolls | `x . x . x . x:3 . x . x . x:2 x:2 x:4 .` |
| Snare build | A bar that rolls faster and louder to the end | `x x x x x:2 x:2 x:2 x:2 x:3 x:3 x:3 x:3 x:4 x:4 x:6 x:8 \| vel 60 … 127` |
| Triplet roll | A triplet roll every other step | `x:3 x x:3 x` |
| Skip roll | An accent, then skipping rolls | `x! . x:2 . x . x:3 x` |
| Machine gun | Four hits a step, loud and soft | `x:4 x:4 x:4 x:4 \| vel 127 90 110 80` |
| Ghost rolls | Accents with quiet rolls that come and go | `x! x:2%40 x x:3%30 \| vel 120 40 90 40` |
| Four-bar fill | Three bars of straight steps, then a fill that rolls faster to the end | 64 steps |
| Ping rolls | Rolls that bounce between the held notes | `1 2:2 3 2:2 1 2:3 3 h:4` |

## Riffs

The low note does the work: for heavy guitars, basses and synths.

| Pattern | What it plays | Steps |
|---|---|---|
| Gallop riff | An eighth and two sixteenths on the low note, moving to the second | `l! . l l l! . l l l! . l l 2! . 2 2` |
| Reverse gallop | Two sixteenths and an eighth | `l l l! . l l l! . l l l! . l l 3! .` |
| Chug threes | Palm-muted chugs accented 3-3-2 | `l! l' l' l! l' l' l! l'` |
| Palm mute | Short low notes, loud and soft | `l' l' l' l' l' l' l' l' \| vel 110 70 80 70` |
| Breakdown | Accented hits an octave down, with space between | `l-! . . l-! . . l-! . . . l-! . l-! . . .` |
| Tremolo | Two hits a step, climbing the chord | `1:2 1:2 1:2 1:2 2:2 2:2 3:2 3:2` |
| Pedal tone | The low note again and again, with the others between | `l l h l l 3 l 2` |
| Seven chug | A seven-step riff that shifts against the bar | `l! l . l! l . l` |

## Lines

Melodic shapes over the chord.

| Pattern | What it plays | Steps |
|---|---|---|
| Up and over | Up the chord and over the top, then back | `1 2 3 h 1+ h 3 2` |
| Spiral | Two up, one back, round and round | `1 3 2 4 3 5 4 6` |
| Octave leaps | Each note, then the same an octave up | `1 1+ 2 2+ 3 3+ 2 2+` |
| Waterfall | From the top an octave up, falling | `h+ h 3+ 3 2+ 2 1+ 1` |
| Call and answer | A phrase up, a phrase down | `1 2 3 . h . . . 3 2 1 . l- . . .` |
| Neighbour notes | Each note with the notes either side of it | `1 1 1 1 2 2 2 2 \| deg 0 1 0 -1` |
| Sequence | A three-note figure moving up step by step | `1 1 1 1 1 1 1 1 1 1 1 1 \| deg 0 2 4 2 4 5 4 5 7 5 7 9` |
| Pentatonic | A pentatonic run up and back from the lowest note | `1 1 1 1 1 1 1 1 \| deg 0 2 4 7 9 12 9 7` |

## Bass

Bass lines from the lowest note held.

| Pattern | What it plays | Steps |
|---|---|---|
| Disco octaves | Short low notes with the octave between | `l' l+ l' l+` |
| Rolling | Three short notes after every beat | `. l' l' l'` |
| Acid | Octave jumps, accents and a tie, for a resonant synth | `l l+! l l _ l+ l! l \| gate 50 50 50 100 100 30 50 50` |
| Sub | Long, low notes with space | `l~ _ _ . . . l' . l~ _ _ . . . . .` |
| Funk bass | Syncopated low notes and octaves | `l! . l+ l . l . l+ . l . . l! . l+ .` |
| Garage bass | A sparse, skipping line | `l . . . . . l . . l . . l+ . . .` |
| Walking | Four long notes walking up from the lowest | `1~ _ _ _ 1~ _ _ _ 1~ _ _ _ 1~ _ _ _ \| deg 0 0 0 0 2 2 2 2 4 4 4 4 5 5 5 5` |
| Wobble steps | Rolls on the low note that speed up and slow down | `l:2 l:2 l:3 l:3 l:4 l:4 l:6 l:2` |

## Polymeter

Lanes and lengths that loop against the bar, so the pattern turns over slowly.

| Pattern | What it plays | Steps |
|---|---|---|
| Five over four | An octave up every fifth note | `x x x x \| oct 0 0 0 0 1` |
| Euclid 5 of 8 | Five hits spread as evenly as they go over eight steps | `x . x x . x x .` |
| Euclid 7 of 16 | Seven hits spread over sixteen steps | `x . . x . x . x . . x . x . x .` |
| Shifting accents | An accent every third note | `x x x x \| vel 127 70 70` |
| Phasing | Five notes against an octave lane of four | `1 2 3 4 5 \| oct 0 1 0 1` |
| Odd gates | Note lengths in a cycle of three | `x x x x x x x x \| gate 20 90 50` |
| Dotted eighths | Notes three steps apart, a three-beat loop | `1 . . 2 . . 3 . . 2 . .` |
| Wheels | Octaves in threes and scale steps in fives | `x x x x x x x x \| oct 0 1 2 \| deg 0 2 4 2 0` |

## Chance

Never quite the same bar twice.

| Pattern | What it plays | Steps |
|---|---|---|
| Sparse | Each step plays two times in five | `x%40 x%40 x%40 x%40` |
| Ghost notes | An accent and quiet notes that come and go | `x! x%30 x%50 x%30 \| vel 127 40 60 40` |
| Wander | Random notes, sometimes an octave off | `? ? ? ? \| oct~1` |
| Humanize | Straight notes with loose velocity, length and timing | `x x x x \| vel~20 \| gate~20 50 \| nudge~8` |
| Rain | Short random notes, some missing, in random octaves | `?' ?'%50 ?' ?'%30 \| oct~1 \| vel~30` |
| Broken rolls | Rolls that only sometimes play | `x x:2%30 x x:3%20` |
| Scatter | Notes and random notes that come and go | `x%70 . ?%50 x%70 . ?%60 . x%50` |
| Random chords | The chord or a random note, each only sometimes | `*%50 . ?%50 . *%30 . ? .` |

