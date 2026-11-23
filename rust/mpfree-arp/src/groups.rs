//! MPFree's built-in pattern library, in groups: the ten starters
//! ([`BUILT_IN`]) and 64 more, eight to a group, each written for MPFree.
//!
//! Every pattern plays the notes held or latched, so it follows the chord:
//! `x` is the next note in the arp's order, `1` the lowest held note, `l`
//! and `h` the lowest and highest, `*` the whole chord (the syntax is in
//! [`crate::pattern`]). Most are one bar of sixteenths at the default rate.
//! Lane values for `deg` are kept to steps that sound right both as
//! semitones (no scale picked) and as scale degrees (a scale picked).
//!
//! None of these is copied from Akai's factory patterns, which MPFree does
//! not ship (`docs/dev/project/factory-content-policy.md`); the rhythms
//! with a traditional name (tresillo, clave, Charleston) are centuries-old
//! figures that belong to everyone.

use crate::pattern::BUILT_IN;

/// One group of the built-in library: its name, as the Pattern list's tabs
/// show it, and its patterns as `(name, text)`.
#[derive(Clone, Copy, Debug)]
pub struct Group {
    pub name: &'static str,
    pub patterns: &'static [(&'static str, &'static str)],
}

/// The group the owner's own patterns go in (saved from the workspace, or
/// read from their patterns file): always after the built-in ones.
pub const MINE: &str = "My patterns";

/// The built-in library, in the order the Pattern list shows it. The first
/// group is the ten starters, so the list starts where it always has.
pub const GROUPS: [Group; 9] = [
    Group {
        name: "Starters",
        patterns: &BUILT_IN,
    },
    Group {
        // The whole chord in rhythms: the MPC's Rhythm action.
        name: "Chords",
        patterns: &[
            ("Offbeats", ". . * . . . * . . . * . . . * ."),
            ("Two and four", ". . . . *! . . . . . . . *! . . ."),
            ("Tresillo", "*! . . * . . * ."),
            ("Clave", "* . . * . . * . . . * . * . . ."),
            ("Charleston", "*! . . *' . . . ."),
            ("Two-step", "*' . . *' . . . . . . *' . *' . . ."),
            ("Long chords", "*~ _ _ _ _ _ _ _ *~ _ _ _ *~ _ . ."),
            ("Funk comp", ". *' . *' *' . . *' . *' . . *' . *' ."),
        ],
    },
    Group {
        // The chord chopped and pumped.
        name: "Gates",
        patterns: &[
            ("Trance gate", "* * . * * . * . * * . * * . * * | gate 60"),
            ("Pump", ". *' * *~ | vel 50 70 95 120"),
            ("Stutter", "* * * * . . . . *:2 *:2 . . *:4 . . ."),
            ("Dotted chops", "* . . * . . * . . * . . * . * ."),
            ("Choppy", "*' *' . *' . *' *' . | vel 120 80 100 70"),
            (
                "Swell",
                "* * * * * * * * | vel 50 60 70 80 90 100 110 120 | gate 15 30 45 60 75 90 100 100",
            ),
            ("Shimmer", "*:3 *:2 *:3 *:2 | vel~20 | gate 40"),
            ("Morse", "*' *' . *~ _ . *' . *~ _ _ . *' *' *' ."),
        ],
    },
    Group {
        // Ratchets: hats, snare builds and fills, on a drum pad or a note.
        name: "Rolls",
        patterns: &[
            ("Trap hats", "x . x . x . x:3 . x . x . x:2 x:2 x:4 ."),
            (
                "Snare build",
                "x x x x x:2 x:2 x:2 x:2 x:3 x:3 x:3 x:3 x:4 x:4 x:6 x:8 \
                 | vel 60 64 68 72 76 80 84 88 92 96 100 104 108 114 120 127",
            ),
            ("Triplet roll", "x:3 x x:3 x"),
            ("Skip roll", "x! . x:2 . x . x:3 x"),
            ("Machine gun", "x:4 x:4 x:4 x:4 | vel 127 90 110 80"),
            ("Ghost rolls", "x! x:2%40 x x:3%30 | vel 120 40 90 40"),
            (
                "Four-bar fill",
                "x x x x x x x x x x x x x x x x x x x x x x x x \
                 x x x x x x x x x x x x x x x x x x x x x x x x \
                 x:2 x:2 x x:2 x:3 x:3 x:4 x:4 x:3 x:4 x:4 x:6 x:6 x:8 x:8 x:8",
            ),
            ("Ping rolls", "1 2:2 3 2:2 1 2:3 3 h:4"),
        ],
    },
    Group {
        // Gallops, chugs and tremolo: the low note does the work.
        name: "Riffs",
        patterns: &[
            ("Gallop riff", "l! . l l l! . l l l! . l l 2! . 2 2"),
            ("Reverse gallop", "l l l! . l l l! . l l l! . l l 3! ."),
            ("Chug threes", "l! l' l' l! l' l' l! l'"),
            ("Palm mute", "l' l' l' l' l' l' l' l' | vel 110 70 80 70"),
            ("Breakdown", "l-! . . l-! . . l-! . . . l-! . l-! . . ."),
            ("Tremolo", "1:2 1:2 1:2 1:2 2:2 2:2 3:2 3:2"),
            ("Pedal tone", "l l h l l 3 l 2"),
            ("Seven chug", "l! l . l! l . l"),
        ],
    },
    Group {
        // Melodic shapes over the chord.
        name: "Lines",
        patterns: &[
            ("Up and over", "1 2 3 h 1+ h 3 2"),
            ("Spiral", "1 3 2 4 3 5 4 6"),
            ("Octave leaps", "1 1+ 2 2+ 3 3+ 2 2+"),
            ("Waterfall", "h+ h 3+ 3 2+ 2 1+ 1"),
            ("Call and answer", "1 2 3 . h . . . 3 2 1 . l- . . ."),
            ("Neighbour notes", "1 1 1 1 2 2 2 2 | deg 0 1 0 -1"),
            (
                "Sequence",
                "1 1 1 1 1 1 1 1 1 1 1 1 | deg 0 2 4 2 4 5 4 5 7 5 7 9",
            ),
            ("Pentatonic", "1 1 1 1 1 1 1 1 | deg 0 2 4 7 9 12 9 7"),
        ],
    },
    Group {
        // Bass lines from the lowest note held.
        name: "Bass",
        patterns: &[
            ("Disco octaves", "l' l+ l' l+"),
            ("Rolling", ". l' l' l'"),
            (
                "Acid",
                "l l+! l l _ l+ l! l | gate 50 50 50 100 100 30 50 50",
            ),
            ("Sub", "l~ _ _ . . . l' . l~ _ _ . . . . ."),
            ("Funk bass", "l! . l+ l . l . l+ . l . . l! . l+ ."),
            ("Garage bass", "l . . . . . l . . l . . l+ . . ."),
            (
                "Walking",
                "1~ _ _ _ 1~ _ _ _ 1~ _ _ _ 1~ _ _ _ | deg 0 0 0 0 2 2 2 2 4 4 4 4 5 5 5 5",
            ),
            ("Wobble steps", "l:2 l:2 l:3 l:3 l:4 l:4 l:6 l:2"),
        ],
    },
    Group {
        // Lanes and lengths that loop against the bar.
        name: "Polymeter",
        patterns: &[
            ("Five over four", "x x x x | oct 0 0 0 0 1"),
            ("Euclid 5 of 8", "x . x x . x x ."),
            ("Euclid 7 of 16", "x . . x . x . x . . x . x . x ."),
            ("Shifting accents", "x x x x | vel 127 70 70"),
            ("Phasing", "1 2 3 4 5 | oct 0 1 0 1"),
            ("Odd gates", "x x x x x x x x | gate 20 90 50"),
            ("Dotted eighths", "1 . . 2 . . 3 . . 2 . ."),
            ("Wheels", "x x x x x x x x | oct 0 1 2 | deg 0 2 4 2 0"),
        ],
    },
    Group {
        // Chance and random lanes: never quite the same bar twice.
        name: "Chance",
        patterns: &[
            ("Sparse", "x%40 x%40 x%40 x%40"),
            ("Ghost notes", "x! x%30 x%50 x%30 | vel 127 40 60 40"),
            ("Wander", "? ? ? ? | oct~1"),
            ("Humanize", "x x x x | vel~20 | gate~20 50 | nudge~8"),
            ("Rain", "?' ?'%50 ?' ?'%30 | oct~1 | vel~30"),
            ("Broken rolls", "x x:2%30 x x:3%20"),
            ("Scatter", "x%70 . ?%50 x%70 . ?%60 . x%50"),
            ("Random chords", "*%50 . ?%50 . *%30 . ? ."),
        ],
    },
];

/// How many built-in patterns there are.
pub const fn count() -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < GROUPS.len() {
        n += GROUPS[i].patterns.len();
        i += 1;
    }
    n
}

/// Each group's name and the index of its first pattern in the built-in
/// list (`Library::built_in`'s order).
pub fn starts() -> Vec<(&'static str, usize)> {
    let mut at = 0;
    GROUPS
        .iter()
        .map(|g| {
            let start = at;
            at += g.patterns.len();
            (g.name, start)
        })
        .collect()
}

/// The group a pattern of a list starting with the built-in library is in:
/// the built-in group of index `i`, or [`MINE`] past them.
pub fn group_of(i: usize) -> &'static str {
    let mut at = 0;
    for g in &GROUPS {
        at += g.patterns.len();
        if i < at {
            return g.name;
        }
    }
    MINE
}

/// The groups of a pattern list that starts with the built-in library (the
/// app's: the owner's patterns file may replace a built-in pattern by its
/// name, and adds its own after them): each group's name and the index of
/// its first pattern. Everything from the first name that is not the
/// built-in one at its place is [`MINE`].
pub fn of_list<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<(&'static str, usize)> {
    let mut built = GROUPS
        .iter()
        .flat_map(|g| g.patterns.iter().map(move |(n, _)| (g.name, *n)));
    let mut out: Vec<(&'static str, usize)> = Vec::new();
    let mut mine = false;
    for (i, name) in names.into_iter().enumerate() {
        let group = match built.next() {
            Some((g, n)) if !mine && n.eq_ignore_ascii_case(name) => g,
            _ => {
                mine = true;
                MINE
            }
        };
        if out.last().is_none_or(|(g, _)| *g != group) {
            out.push((group, i));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, EventKind, Events, Mode, Settings};
    use crate::pattern::Pattern;

    #[test]
    fn every_pattern_reads_and_writes_back_as_written() {
        for g in &GROUPS {
            for (name, text) in g.patterns {
                let p = Pattern::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(p.to_text(), *text, "{name}");
            }
        }
    }

    #[test]
    fn names_are_unique_and_fit_and_the_groups_are_eight_after_the_starters() {
        let mut seen: Vec<String> = Vec::new();
        for g in &GROUPS {
            assert!(!g.name.eq_ignore_ascii_case(MINE));
            assert!(
                g.name.chars().count() <= 12,
                "{} is too long for a tab",
                g.name
            );
            for (name, _) in g.patterns {
                let low = name.to_lowercase();
                assert!(!seen.contains(&low), "{name} twice");
                assert!(name.chars().count() <= crate::library::MAX_NAME);
                seen.push(low);
            }
        }
        assert_eq!(GROUPS[0].patterns, &BUILT_IN[..]);
        assert!(GROUPS[1..].iter().all(|g| g.patterns.len() == 8));
        assert_eq!(count(), 10 + 64);
    }

    #[test]
    fn starts_and_group_of_agree() {
        let s = starts();
        assert_eq!(s[0], ("Starters", 0));
        assert_eq!(s[1], ("Chords", 10));
        for (k, (name, start)) in s.iter().enumerate() {
            assert_eq!(group_of(*start), *name);
            let end = s.get(k + 1).map_or(count(), |n| n.1);
            assert_eq!(group_of(end - 1), *name);
        }
        assert_eq!(group_of(count()), MINE);
    }

    #[test]
    fn a_list_is_grouped_as_built_in_then_mine() {
        let lib = crate::Library::built_in();
        let names: Vec<&str> = lib.iter().map(|(n, _)| n).collect();
        let s = starts();
        assert_eq!(of_list(names.iter().copied()), s);
        let mut more = names.clone();
        more.push("Skip");
        more.push("Lift");
        let g = of_list(more.iter().copied());
        assert_eq!(g.len(), GROUPS.len() + 1);
        assert_eq!(*g.last().unwrap(), (MINE, count()));
        // Only some of the built-in list (a test's, a preview's): the
        // groups it reaches.
        assert_eq!(
            of_list(names[..12].iter().copied()),
            vec![("Starters", 0), ("Chords", 10)]
        );
        // A list that is not the built-in one is all the owner's.
        assert_eq!(of_list(["Mine", "Straight"]), vec![(MINE, 0)]);
        assert!(of_list(std::iter::empty()).is_empty());
    }

    /// Every pattern makes notes over a three-note chord (chance ones over
    /// eight bars), and none plays forever-silent.
    #[test]
    fn every_pattern_plays_over_a_held_chord() {
        for g in &GROUPS {
            for (name, text) in g.patterns {
                let mut e = Engine::new(48_000, 120.0);
                e.set_settings(Settings {
                    mode: Mode::Pattern,
                    ..Settings::default()
                });
                e.set_pattern(Pattern::parse(text).unwrap());
                for n in [48, 52, 55] {
                    e.press(n, 100);
                }
                let mut ons = 0;
                let mut out = Events::new();
                // Eight bars at 120 BPM: 16 s.
                for _ in 0..(16 * 48_000 / 128) {
                    e.process(128, &mut out);
                    ons += out
                        .as_slice()
                        .iter()
                        .filter(|ev| matches!(ev.kind, EventKind::On { .. }))
                        .count();
                }
                assert!(ons >= 8, "{} / {name}: {ons} notes in eight bars", g.name);
            }
        }
    }
}
