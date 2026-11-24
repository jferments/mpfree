//! `listen --ideas`: the blind listen for the composer's phrases (P2):
//! twelve pairs, an idea beside the simple way of making the same kind of
//! phrase (docs/dev/design/compose.md, "New ideas").
//!
//! The pairs are every kind of phrase in every style, at lengths and in
//! scales set here by rule ([`CASES`]), each as busy as the middle of the
//! setting. Every one takes the tool's `--seed` (1 unless given) as it
//! is: no idea is picked by ear or by eye, so what is heard is what the
//! first tap of New idea would give.
//!
//! Both of a pair are made from one request ([`new_idea`] and
//! [`simple_idea`]) and heard in one setting, as `--phrases` plays a
//! phrase ([`super::render`]): the arp's engine plays each from a held
//! note that moves, on the same sound, at the same tempo, over the same
//! beat, in the same key. Which is A and which B is shuffled by the seed,
//! the composer's first in half the pairs.
//!
//! The files' names say nothing. `answers.json` says which is which, with
//! each phrase's text and what its checks found, for the listening page
//! to show once the listener has chosen; `report.txt` lists it all. With
//! `EXPLAIN=1` the notes the engine played are printed, bar by bar.

use super::super::{json, kit_name, order, wav, FILE_LUFS, RATE};
use super::{explain, held_words, note_name, render, setting_as, shape, Beat, Heard, Setting};
use mpfree_arp::{Phrase, PhraseKind};
use mpfree_compose::idea::simple_idea;
use mpfree_compose::style::STYLES;
use mpfree_compose::{new_idea, Idea, NewIdea, Style};
use std::fmt::Write as _;
use std::path::Path;

/// What the listener is asked of every pair.
const QUESTION: &str = "Which would you keep?";

/// How busy every idea of the set is asked to be: the middle.
const ACTIVITY: f32 = 0.5;

/// One pair's request: a kind, a style (its place in [`STYLES`]), a
/// length in bars and a scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Case {
    kind: PhraseKind,
    style: usize,
    bars: u8,
    minor: bool,
}

const fn case(kind: PhraseKind, style: usize, bars: u8, minor: bool) -> Case {
    Case {
        kind,
        style,
        bars,
        minor,
    }
}

/// The twelve pairs: every kind in every style (Minimal, Groove,
/// Jazz-ish). A bass and a riff are heard at two and four bars, chords
/// and a line at one and two; each kind is heard twice in a minor key and
/// once in a major one.
const CASES: [Case; 12] = [
    case(PhraseKind::Bass, 0, 2, true),
    case(PhraseKind::Bass, 1, 4, true),
    case(PhraseKind::Bass, 2, 4, false),
    case(PhraseKind::Riff, 0, 2, true),
    case(PhraseKind::Riff, 1, 2, false),
    case(PhraseKind::Riff, 2, 4, true),
    case(PhraseKind::Chords, 0, 1, true),
    case(PhraseKind::Chords, 1, 2, true),
    case(PhraseKind::Chords, 2, 2, false),
    case(PhraseKind::Line, 0, 1, true),
    case(PhraseKind::Line, 1, 2, false),
    case(PhraseKind::Line, 2, 2, true),
];

/// How a style's ideas are heard: the tempo, the beat, and the sound of
/// each kind (a bass, a riff, chords, a line).
const HEARD: [(u16, Beat, [&str; 4]); 3] = [
    (
        124,
        Beat::House,
        [
            "bass: Round Mono",
            "poly: Pluck",
            "keys: Hard Tines",
            "poly: Harmonic Pluck",
        ],
    ),
    (
        96,
        Beat::BoomBap,
        [
            "bass: Funk Bass",
            "keys: Funk Clav",
            "keys: Mellow Tines",
            "keys: Bell Keys",
        ],
    ),
    (
        120,
        Beat::Ride,
        [
            "pluck: Upright Bass",
            "pluck: Vibes",
            "keys: Soft Tines",
            "pluck: Celesta",
        ],
    ),
];

impl Case {
    fn style(&self) -> Style {
        STYLES[self.style]
    }

    fn request(&self, seed: u64) -> NewIdea {
        NewIdea {
            kind: self.kind,
            style: self.style(),
            minor: self.minor,
            bars: self.bars,
            activity: ACTIVITY,
            seed,
            attempt: 0,
        }
    }

    /// How both of the pair are heard. A phrase written for either scale
    /// is heard in the scale that was asked for, so the two share a key.
    fn heard(&self) -> Heard {
        let (bpm, beat, sounds) = HEARD[self.style];
        let kind = PhraseKind::ALL
            .iter()
            .position(|k| *k == self.kind)
            .unwrap_or(0);
        // An idea's chords are written around its home note, where
        // MPFree's own stand on theirs, so they are held from middle C:
        // in a major key the tool holds a chord phrase an octave under.
        let octave = i8::from(self.kind == PhraseKind::Chords && !self.minor);
        Heard {
            name: "",
            bpm,
            beat,
            sound: sounds[kind],
            major: !self.minor,
            octave,
        }
    }

    /// `03-bass-jazz-ish`: the pair's place in the set, its kind and its
    /// style.
    fn id(&self, number: usize) -> String {
        format!(
            "{number:02}-{}-{}",
            self.kind.word(),
            self.style().name.to_lowercase().replace(' ', "-")
        )
    }
}

/// One of a pair.
struct Take {
    /// The composer's idea (else the simple way).
    composer: bool,
    idea: Idea,
    phrase: Phrase,
}

impl Take {
    fn what(&self) -> &'static str {
        if self.composer {
            "the composer's"
        } else {
            "the simple way"
        }
    }
}

/// A pair in the order it is heard: A, then B.
struct Pair {
    case: Case,
    id: String,
    takes: [Take; 2],
}

/// Which pairs play the composer's idea as A: six of the twelve, two in
/// every style and one or two of every kind, so that neither a kind nor
/// a style gives the order away (shuffled freely, all three riffs could
/// play the composer's first). The seed turns the pattern: which kinds
/// have two, which styles, and whether the pattern or its opposite.
fn composer_first(seed: u64) -> [bool; 12] {
    // Kinds down, styles across: rows of two and one, columns of two.
    const PATTERN: [[bool; 3]; 4] = [
        [true, true, false],
        [false, false, true],
        [true, false, true],
        [false, true, false],
    ];
    let kinds = order(4, seed, "ideas: kinds");
    let styles = order(3, seed, "ideas: styles");
    let opposite = order(2, seed, "ideas: which first")[0] == 1;
    let mut first = [false; 12];
    for (i, case) in CASES.iter().enumerate() {
        let kind = PhraseKind::ALL
            .iter()
            .position(|k| *k == case.kind)
            .unwrap_or(0);
        first[i] = PATTERN[kinds[kind]][styles[case.style]] != opposite;
    }
    first
}

/// The set: every case's idea and its simple way, in the order heard.
fn pairs(seed: u64) -> Vec<Pair> {
    let first = composer_first(seed);
    CASES
        .iter()
        .enumerate()
        .map(|(i, case)| {
            let req = case.request(seed);
            let take = |composer: bool| {
                let idea = if composer {
                    new_idea(&req)
                } else {
                    simple_idea(&req)
                };
                let phrase = idea.phrase();
                Take {
                    composer,
                    idea,
                    phrase,
                }
            };
            Pair {
                case: *case,
                id: case.id(i + 1),
                takes: [take(first[i]), take(!first[i])],
            }
        })
        .collect()
}

/// The one setting both of a pair are heard in.
fn setting_of(pair: &Pair) -> Result<Setting, String> {
    let heard = pair.case.heard();
    let [a, b] = [&pair.takes[0], &pair.takes[1]].map(|t| setting_as(&pair.id, &t.phrase, heard));
    let same = a.held == b.held
        && a.major == b.major
        && (a.swing.grid, a.swing.amount) == (b.swing.grid, b.swing.amount)
        && a.session.meter == b.session.meter;
    if !same {
        return Err(format!(
            "{}: the two would not be heard alike (held {} and {}{}{})",
            pair.id,
            held_words(&a.held),
            held_words(&b.held),
            a.swing_words(),
            b.swing_words()
        ));
    }
    Ok(a)
}

/// Writes the set into `dir`: two WAVs a pair, answers.json for the
/// listening page and report.txt.
pub(in super::super) fn run(seed: u64, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    let mut report = String::new();
    let _ = writeln!(
        report,
        "mpfree composer's phrases, the blind listen (P2): {} pairs, seed {seed}\n",
        CASES.len()
    );
    let _ = writeln!(
        report,
        "Each pair is one request made two ways, the composer's idea and the simple way of its kind\n(a bass: the chord's root on every kick; chords: a block chord on every bar line and change;\na riff: notes of the key by chance, as many as the composer's; a line: the chord going up on\nevery step), heard in one setting. The question: {QUESTION}\n"
    );
    let mut rows = Vec::new();
    let mut peaks = 0.0f32;
    // Checks passed and asked, for the composer's and the simple way.
    let mut passed = [(0usize, 0usize); 2];
    for (i, pair) in pairs(seed).iter().enumerate() {
        if pair.takes[0].phrase == pair.takes[1].phrase {
            return Err(format!("{}: the two are one phrase", pair.id));
        }
        let s = setting_of(pair)?;
        let case = &pair.case;
        let style = case.style();
        let _ = writeln!(
            report,
            "{}: {}, {}, {} bar{}, {}; {} bpm{}, {}, {} ({}); held {}; written in C over [{}]",
            pair.id,
            case.kind.label(),
            style.name,
            case.bars,
            if case.bars == 1 { "" } else { "s" },
            s.key(),
            s.heard.bpm,
            s.swing_words(),
            s.heard.beat.words(),
            s.heard.sound,
            kit_name(&s.session),
            held_words(&s.held),
            pair.takes[0].idea.chords.names()
        );
        let mut files = Vec::new();
        let mut seconds = 0.0;
        for (take, label) in pair.takes.iter().zip(["A", "B"]) {
            let (left, right, played) =
                render(&take.phrase, &s).map_err(|e| format!("{} {label}: {e}", pair.id))?;
            if played.notes == 0 {
                return Err(format!("{} {label}: nothing played", pair.id));
            }
            if std::env::var_os("EXPLAIN").is_some() {
                explain(
                    &format!("{} {label}, {}", pair.id, take.what()),
                    &s,
                    &played,
                );
            }
            let (bytes, peak) = wav(&left, &right);
            peaks = peaks.max(peak);
            let stem = format!("{}-{}", pair.id, label.to_lowercase());
            let path = dir.join(format!("{stem}.wav"));
            std::fs::write(&path, bytes)
                .map_err(|e| format!("can't write {}: {e}", path.display()))?;
            seconds = left.len() as f64 / f64::from(RATE);
            let verdict = &take.idea.verdict;
            let tally = &mut passed[usize::from(!take.composer)];
            tally.0 += verdict.passed();
            tally.1 += verdict.checks.len();
            let _ = writeln!(
                report,
                "  {label} {:<26} {:<15} {}/{} checks, {} notes, {seconds:.1} s, peak {:.1} dBFS\n      {}",
                format!("{stem}.wav"),
                take.what(),
                verdict.passed(),
                verdict.checks.len(),
                played.notes,
                20.0 * peak.max(1e-9).log10(),
                take.phrase.to_text()
            );
            let misses: Vec<String> = verdict
                .checks
                .iter()
                .filter(|(_, c)| !c.pass)
                .map(|(name, c)| format!("{name}: {}", c.words))
                .collect();
            for miss in &misses {
                let _ = writeln!(report, "      - {miss}");
            }
            files.push(format!(
                "{{\"label\": {}, \"file\": {}, \"composer\": {}, \"what\": {}, \"text\": {}, \"passed\": {}, \"checks\": {}, \"misses\": [{}], \"notes\": {}, \"ticks\": {}, \"shape\": [{}]}}",
                json(label),
                json(&stem),
                take.composer,
                json(take.what()),
                json(&take.phrase.to_text()),
                verdict.passed(),
                verdict.checks.len(),
                misses.iter().map(|m| json(m)).collect::<Vec<_>>().join(", "),
                played.notes,
                take.phrase.ticks(),
                shape(&take.phrase)
            ));
        }
        rows.push(format!(
            "  {{\"number\": {}, \"id\": {}, \"kind\": {}, \"style\": {}, \"bars\": {}, \"key\": {}, \"bpm\": {}, \"beat\": {}, \"sound\": {}, \"held\": [{}], \"seconds\": {seconds:.1}, \"files\": [\n    {}\n  ]}}",
            i + 1,
            json(&pair.id),
            json(case.kind.label()),
            json(style.name),
            case.bars,
            json(s.key()),
            s.heard.bpm,
            json(s.heard.beat.words()),
            json(s.heard.sound),
            s.held
                .iter()
                .map(|&n| json(&note_name(n)))
                .collect::<Vec<_>>()
                .join(", "),
            files.join(",\n    ")
        ));
    }
    let _ = writeln!(
        report,
        "\nThe checks, which were written beside the composer: its ideas pass {} of {}, the simple way {} of {}.",
        passed[0].0, passed[0].1, passed[1].0, passed[1].1
    );
    let _ = writeln!(
        report,
        "Every file is at the same loudness ({FILE_LUFS} LUFS, with a soft knee from -1.9 dBFS instead of clipping); the loudest peak was {:.1} dBFS.",
        20.0 * peaks.max(1e-9).log10()
    );
    let answers = format!(
        "{{\"seed\": {seed}, \"question\": {}, \"pairs\": [\n{}\n]}}\n",
        json(QUESTION),
        rows.join(",\n")
    );
    std::fs::write(dir.join("answers.json"), answers).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("report.txt"), &report).map_err(|e| e.to_string())?;
    print!("{report}");
    println!("Wrote {}", dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{play, sound};
    use super::*;

    #[test]
    fn the_set_is_every_kind_in_every_style() {
        for kind in PhraseKind::ALL {
            let of: Vec<&Case> = CASES.iter().filter(|c| c.kind == kind).collect();
            // Once in each style, twice in minor and once in major, at
            // two lengths.
            let mut styles: Vec<usize> = of.iter().map(|c| c.style).collect();
            styles.sort_unstable();
            assert_eq!(styles, [0, 1, 2], "{kind:?}");
            assert_eq!(of.iter().filter(|c| c.minor).count(), 2, "{kind:?}");
            let mut lengths: Vec<u8> = of.iter().map(|c| c.bars).collect();
            lengths.sort_unstable();
            lengths.dedup();
            assert_eq!(lengths.len(), 2, "{kind:?}");
        }
        for bars in [1, 2, 4] {
            assert!(CASES.iter().any(|c| c.bars == bars));
        }
        for case in &CASES {
            let heard = case.heard();
            assert!(sound(heard.sound).is_ok(), "{case:?}: {}", heard.sound);
            // The beat is the style's own drummer.
            assert_eq!(heard.beat.style().name, case.style().name);
        }
        // Each pair has a name of its own.
        let mut ids: Vec<String> = CASES.iter().enumerate().map(|(i, c)| c.id(i + 1)).collect();
        assert_eq!(ids[2], "03-bass-jazz-ish");
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CASES.len());
    }

    #[test]
    fn the_composer_s_idea_plays_first_in_half_the_pairs() {
        let mut patterns = Vec::new();
        for seed in 1..=24 {
            let first = composer_first(seed);
            assert_eq!(first, composer_first(seed));
            assert_eq!(first.iter().filter(|f| **f).count(), 6, "seed {seed}");
            // Two in every style, one or two of every kind.
            for style in 0..3 {
                let n = CASES
                    .iter()
                    .zip(first)
                    .filter(|(c, f)| c.style == style && *f)
                    .count();
                assert_eq!(n, 2, "seed {seed}, style {style}");
            }
            for kind in PhraseKind::ALL {
                let n = CASES
                    .iter()
                    .zip(first)
                    .filter(|(c, f)| c.kind == kind && *f)
                    .count();
                assert!((1..=2).contains(&n), "seed {seed}, {kind:?}");
            }
            if !patterns.contains(&first) {
                patterns.push(first);
            }
        }
        // The seeds do not all turn it alike.
        assert!(patterns.len() >= 8, "{} patterns", patterns.len());
        for (pair, composer_is_a) in pairs(1).iter().zip(composer_first(1)) {
            assert_eq!(pair.takes[0].composer, composer_is_a);
            assert_eq!(pair.takes[1].composer, !composer_is_a);
        }
    }

    #[test]
    fn both_of_a_pair_are_one_request_heard_in_one_setting() {
        for seed in [1, 2] {
            for pair in pairs(seed) {
                let [a, b] = &pair.takes;
                // One request: the same kind and length, over the same
                // chords, and two phrases.
                assert_eq!(a.idea.chords, b.idea.chords, "{}", pair.id);
                assert_eq!(
                    (a.phrase.kind, a.phrase.bars),
                    (pair.case.kind, pair.case.bars)
                );
                assert_eq!(
                    (b.phrase.kind, b.phrase.bars),
                    (a.phrase.kind, a.phrase.bars)
                );
                assert_ne!(a.phrase, b.phrase, "{}", pair.id);
                // One setting, in the scale that was asked for.
                let s = setting_of(&pair).unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(s.major, !pair.case.minor, "{}", pair.id);
                assert_eq!(s.swing.amount > 50, pair.case.style == 2, "{}", pair.id);
                // And the engine plays both.
                for take in &pair.takes {
                    let played = play(&take.phrase, &s).unwrap();
                    assert!(played.notes > 0, "{} {}", pair.id, take.what());
                }
            }
        }
    }
}
