# Scales (`rust/mpfree-scale`)

One crate for every part that plays in key: the arp now, the synth pads next. It allocates nothing except when reading or writing text.

## Each part has its own scale (owner, 2026-11-11)

There is no forced project-wide key. Each part (the arp, the synth pads, later a track) has its own scale, Off until one is picked. A project key exists only when the owner makes a project with one; parts then follow it, and any part can be switched back to its own. `PartScale { own, follow_project }` with `resolve(project_key)` is that rule. Projects don't exist yet, so nothing calls it with a key today.

## What a scale is

`Scale { root, mask, down }`: a root (0 = C … 11 = B) and which of the 12 semitones above it belong to the scale going up (`mask`) and coming down (`down`, the same for most scales). Any 12-note scale fits: 2048 of them contain their root. Notes outside 12-tone equal temperament (quarter tones, just intonation, Scala files) are not covered yet: our synth works in fractional pitch inside, but notes reach it as whole MIDI numbers, and MIDI out would need MPE or the MIDI Tuning Standard.

- `snapped(note, snap)`: a note outside the scale goes to the scale note below (`Snap::Down`, the default), above (`Up`), the nearer one (`Nearest`, below when as near), or is dropped (`Skip`).
- `walk(note, degrees)`: up the notes going up, down the notes coming down.
- `fit(note, degrees, snap)`: both, as the arp uses it.
- `nth(from, index)`: the `index`th scale note from the first one at or above `from`, for in-key pad layouts.

## The built-in library (`SCALES`, 49)

| Family | Scales |
|---|---|
| Chromatic | Chromatic |
| Major and its modes | Major, Dorian, Phrygian, Lydian, Mixolydian, Minor, Locrian |
| Melodic minor and its modes | Melodic minor, Dorian b2, Lydian augmented, Lydian dominant, Mixolydian b6, Locrian natural 2, Altered; Mel. minor classical (natural minor coming down) |
| Harmonic minor and its modes | Harmonic minor, Locrian natural 6, Ionian #5, Dorian #4, Phrygian dominant, Lydian #2, Ultralocrian |
| More seven-note scales | Harmonic major, Double harmonic, Hungarian minor, Hungarian major, Neapolitan major, Neapolitan minor, Persian, Enigmatic |
| Symmetric | Whole tone, Diminished W-H (whole step, then half), Diminished H-W, Augmented |
| Bebop | Bebop dominant, Bebop major, Bebop dorian |
| Pentatonic, blues and six-note | Major pentatonic, Minor pentatonic, Blues, Major blues, Egyptian, Prometheus |
| Japanese | Hirajoshi, In-sen, Iwato, Yo, Kumoi |

Names stay within about 18 letters so they fit the menu's value box. `by_name` also takes other names (`ALIASES`: Ionian, Aeolian, Natural minor, Jazz minor, Super locrian, Spanish, and the long forms such as Diminished whole-half), ignoring case, spaces and punctuation.

The tests check that every scale has its root, no two are the same, the families stay together, and each family's seven modes are rotations of their parent scale.

## The owner's own scales

They go in the patterns file after a `[scales]` line (`[patterns]` switches back), one per line, either as semitones from `0` or as degrees from `1` with `b` and `#`, and `/` before the notes coming down when they differ:

```text
[scales]
Bright minor   = 1 2 b3 4 5 6 7
Classical min  = 0 2 3 5 7 9 11 / 0 2 3 5 7 8 10
```

A set with a `b` or `#`, or with `1` and no `0`, is read as degrees. The root is always included. A custom scale may not take a built-in name. In the pad player they follow the built-in scales in the arp's Scale row, and Save writes them back with the patterns (only with `--save-patterns`).

## The pads (`mpfree_scale::pads`)

For a synth on the pads, as the owner decided on 2026-11-11 (09:55Z). `layout(&PadSetup)` gives the 16 pads, each with its notes (a chord's root last, so a mono synth that plays the last note plays the root), its role for colour (`Root`, `InScale`, `OutOfScale`, or `Plain` with no scale) and, on chord pads, its scale degree. `pad_name` gives a tile's text and `range_text` the chip's range. Nothing is decided per audio block: the layout is worked out when a setting changes.

| `PadLayout` | Pads |
|---|---|
| `Chromatic` (default) | Pad 1 = C3, a semitone each, as in round 6. With a scale, only the colours change. |
| `InKey` | Only scale notes, pad 1 = the root in octave 3. `InKeyRows::Sequence` (default) climbs 16 scale notes; `Fourths` starts each row three scale notes higher. |
| `Chords` | Triads on the bottom two rows (I to VII, then I an octave up), sevenths on the top two. Tiles: numeral and name (iv7, Dm7) in seven-note scales, else the degree and the notes. |

`PadLayout::toggled` is the one quick switch between Chromatic and In key; which button or touch does it is the Hello MPC thread's proposal. `DEFAULT_BANKS`: A Chromatic, B In key, C Chord pads, D In key an octave up, E to H copies of A to D (approved for now, to be discussed again). With no scale every layout is plain Chromatic.

## Next

- Arp: done (chord on each pad, scale run, chords taking turns; see [arpeggiator.md](arpeggiator.md)).
- Synth pads (owner approved 2026-11-11 09:55Z): chromatic by default with the scale lit, one quick action to switch to In key (the Hello MPC thread proposes which), In key rows in sequence (or a fourth apart), chord pads (triads on the bottom two rows, sevenths on the top two), and each pad bank holding its own layout while the synth is on the pads. This crate's `pads` module gives each layout's notes, colour roles and names (above); the synth thread plays them and the Hello MPC thread draws them. The pad player plays them (round 8): the Synth tab's options pick the scale, root, the bank's layout and the In key rows, chord pads send every note with pressure on each, and `App::synth_toggle_layout` and `App::synth_bank` wait for the Hello MPC thread's button choices ([synths.md](synths.md#in-the-pad-player-rustmpfree-hello)).
