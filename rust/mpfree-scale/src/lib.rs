//! Musical scales for MPFree: the arp, the synth pads and anything else that
//! plays in key use this one crate.
//!
//! - A [`Scale`] is a root and which of the 12 semitones above it belong to
//!   it, so any 12-note scale fits (2048 of them have their root). A scale
//!   may have a different set of notes coming down ([`Scale::down`]), as the
//!   classical melodic minor does.
//! - [`SCALES`] is the built-in library, grouped by [`Family`]. Custom
//!   scales are written as semitones or degrees ([`parse_steps`]) and saved
//!   by name next to the owner's patterns.
//! - [`Snap`] says what happens to a note outside the scale.
//! - Every part has its own scale by default ([`PartScale`]). A project key
//!   exists only when the owner makes a project with one; parts then follow
//!   it unless switched back to their own.
//!
//! - [`pads`] lays the 16 pads out from a scale for a synth on the pads:
//!   Chromatic with the scale lit, In key, or Chord pads.
//!
//! Notes are MIDI note numbers (60 = middle C) as `i16`, so steps below 0
//! and above 127 can be computed and skipped by the caller. Nothing here
//! allocates except [`steps_text`] and the parse error text.
#![forbid(unsafe_code)]

pub mod pads;
pub use pads::{InKeyRows, Pad, PadLayout, PadRole, PadSetup};

/// A 12-bit set from semitones above the root (each 0..=11).
pub const fn mask(intervals: &[u8]) -> u16 {
    let mut m = 0u16;
    let mut i = 0;
    while i < intervals.len() {
        m |= 1 << (intervals[i] % 12);
        i += 1;
    }
    m
}

pub const CHROMATIC: u16 = 0x0FFF;
// Major modes.
pub const MAJOR: u16 = mask(&[0, 2, 4, 5, 7, 9, 11]);
pub const DORIAN: u16 = mask(&[0, 2, 3, 5, 7, 9, 10]);
pub const PHRYGIAN: u16 = mask(&[0, 1, 3, 5, 7, 8, 10]);
pub const LYDIAN: u16 = mask(&[0, 2, 4, 6, 7, 9, 11]);
pub const MIXOLYDIAN: u16 = mask(&[0, 2, 4, 5, 7, 9, 10]);
pub const MINOR: u16 = mask(&[0, 2, 3, 5, 7, 8, 10]);
pub const LOCRIAN: u16 = mask(&[0, 1, 3, 5, 6, 8, 10]);
// Melodic minor modes.
pub const MELODIC_MINOR: u16 = mask(&[0, 2, 3, 5, 7, 9, 11]);
pub const DORIAN_FLAT_2: u16 = mask(&[0, 1, 3, 5, 7, 9, 10]);
pub const LYDIAN_AUGMENTED: u16 = mask(&[0, 2, 4, 6, 8, 9, 11]);
pub const LYDIAN_DOMINANT: u16 = mask(&[0, 2, 4, 6, 7, 9, 10]);
pub const MIXOLYDIAN_FLAT_6: u16 = mask(&[0, 2, 4, 5, 7, 8, 10]);
pub const LOCRIAN_NATURAL_2: u16 = mask(&[0, 2, 3, 5, 6, 8, 10]);
pub const ALTERED: u16 = mask(&[0, 1, 3, 4, 6, 8, 10]);
// Harmonic minor modes.
pub const HARMONIC_MINOR: u16 = mask(&[0, 2, 3, 5, 7, 8, 11]);
pub const LOCRIAN_NATURAL_6: u16 = mask(&[0, 1, 3, 5, 6, 9, 10]);
pub const IONIAN_SHARP_5: u16 = mask(&[0, 2, 4, 5, 8, 9, 11]);
pub const DORIAN_SHARP_4: u16 = mask(&[0, 2, 3, 6, 7, 9, 10]);
pub const PHRYGIAN_DOMINANT: u16 = mask(&[0, 1, 4, 5, 7, 8, 10]);
pub const LYDIAN_SHARP_2: u16 = mask(&[0, 3, 4, 6, 7, 9, 11]);
pub const ULTRALOCRIAN: u16 = mask(&[0, 1, 3, 4, 6, 8, 9]);
// More seven-note scales.
pub const HARMONIC_MAJOR: u16 = mask(&[0, 2, 4, 5, 7, 8, 11]);
pub const DOUBLE_HARMONIC: u16 = mask(&[0, 1, 4, 5, 7, 8, 11]);
pub const HUNGARIAN_MINOR: u16 = mask(&[0, 2, 3, 6, 7, 8, 11]);
pub const HUNGARIAN_MAJOR: u16 = mask(&[0, 3, 4, 6, 7, 9, 10]);
pub const NEAPOLITAN_MAJOR: u16 = mask(&[0, 1, 3, 5, 7, 9, 11]);
pub const NEAPOLITAN_MINOR: u16 = mask(&[0, 1, 3, 5, 7, 8, 11]);
pub const PERSIAN: u16 = mask(&[0, 1, 4, 5, 6, 8, 11]);
pub const ENIGMATIC: u16 = mask(&[0, 1, 4, 6, 8, 10, 11]);
// Symmetric scales.
pub const WHOLE_TONE: u16 = mask(&[0, 2, 4, 6, 8, 10]);
pub const DIMINISHED_WHOLE_HALF: u16 = mask(&[0, 2, 3, 5, 6, 8, 9, 11]);
pub const DIMINISHED_HALF_WHOLE: u16 = mask(&[0, 1, 3, 4, 6, 7, 9, 10]);
pub const AUGMENTED: u16 = mask(&[0, 3, 4, 7, 8, 11]);
// Bebop scales (eight notes).
pub const BEBOP_DOMINANT: u16 = mask(&[0, 2, 4, 5, 7, 9, 10, 11]);
pub const BEBOP_MAJOR: u16 = mask(&[0, 2, 4, 5, 7, 8, 9, 11]);
pub const BEBOP_DORIAN: u16 = mask(&[0, 2, 3, 4, 5, 7, 9, 10]);
// Pentatonic, blues and six-note scales.
pub const PENTATONIC_MAJOR: u16 = mask(&[0, 2, 4, 7, 9]);
pub const PENTATONIC_MINOR: u16 = mask(&[0, 3, 5, 7, 10]);
pub const BLUES: u16 = mask(&[0, 3, 5, 6, 7, 10]);
pub const MAJOR_BLUES: u16 = mask(&[0, 2, 3, 4, 7, 9]);
pub const EGYPTIAN: u16 = mask(&[0, 2, 5, 7, 10]);
pub const PROMETHEUS: u16 = mask(&[0, 2, 4, 6, 9, 10]);
// Japanese scales.
pub const HIRAJOSHI: u16 = mask(&[0, 2, 3, 7, 8]);
pub const IN_SEN: u16 = mask(&[0, 1, 5, 7, 10]);
pub const IWATO: u16 = mask(&[0, 1, 5, 6, 10]);
pub const YO: u16 = mask(&[0, 2, 5, 7, 9]);
pub const KUMOI: u16 = mask(&[0, 2, 3, 7, 9]);

/// Groups of scales, for lists on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Chromatic,
    MajorModes,
    MelodicMinorModes,
    HarmonicMinorModes,
    MoreSevenNote,
    Symmetric,
    Bebop,
    PentatonicAndBlues,
    Japanese,
}

impl Family {
    pub const ALL: [Family; 9] = [
        Family::Chromatic,
        Family::MajorModes,
        Family::MelodicMinorModes,
        Family::HarmonicMinorModes,
        Family::MoreSevenNote,
        Family::Symmetric,
        Family::Bebop,
        Family::PentatonicAndBlues,
        Family::Japanese,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Family::Chromatic => "Chromatic",
            Family::MajorModes => "Major and its modes",
            Family::MelodicMinorModes => "Melodic minor and its modes",
            Family::HarmonicMinorModes => "Harmonic minor and its modes",
            Family::MoreSevenNote => "More seven-note scales",
            Family::Symmetric => "Symmetric",
            Family::Bebop => "Bebop",
            Family::PentatonicAndBlues => "Pentatonic, blues and six-note",
            Family::Japanese => "Japanese",
        }
    }
}

/// A named scale in the built-in library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaleDef {
    pub name: &'static str,
    pub family: Family,
    /// The notes going up (and for everything that has no direction).
    pub mask: u16,
    /// The notes coming down; the same as `mask` for most scales.
    pub down: u16,
}

const fn def(name: &'static str, family: Family, mask: u16) -> ScaleDef {
    ScaleDef {
        name,
        family,
        mask,
        down: mask,
    }
}

/// The built-in scales, Chromatic first, then by family.
pub const SCALES: [ScaleDef; 49] = [
    def("Chromatic", Family::Chromatic, CHROMATIC),
    def("Major", Family::MajorModes, MAJOR),
    def("Dorian", Family::MajorModes, DORIAN),
    def("Phrygian", Family::MajorModes, PHRYGIAN),
    def("Lydian", Family::MajorModes, LYDIAN),
    def("Mixolydian", Family::MajorModes, MIXOLYDIAN),
    def("Minor", Family::MajorModes, MINOR),
    def("Locrian", Family::MajorModes, LOCRIAN),
    def("Melodic minor", Family::MelodicMinorModes, MELODIC_MINOR),
    def("Dorian b2", Family::MelodicMinorModes, DORIAN_FLAT_2),
    def(
        "Lydian augmented",
        Family::MelodicMinorModes,
        LYDIAN_AUGMENTED,
    ),
    def(
        "Lydian dominant",
        Family::MelodicMinorModes,
        LYDIAN_DOMINANT,
    ),
    def(
        "Mixolydian b6",
        Family::MelodicMinorModes,
        MIXOLYDIAN_FLAT_6,
    ),
    def(
        "Locrian natural 2",
        Family::MelodicMinorModes,
        LOCRIAN_NATURAL_2,
    ),
    def("Altered", Family::MelodicMinorModes, ALTERED),
    // Up as melodic minor, down as natural minor, as taught in classical
    // theory. Names stay short enough for a menu value box (about 18 letters).
    ScaleDef {
        name: "Mel. minor classical",
        family: Family::MelodicMinorModes,
        mask: MELODIC_MINOR,
        down: MINOR,
    },
    def("Harmonic minor", Family::HarmonicMinorModes, HARMONIC_MINOR),
    def(
        "Locrian natural 6",
        Family::HarmonicMinorModes,
        LOCRIAN_NATURAL_6,
    ),
    def("Ionian #5", Family::HarmonicMinorModes, IONIAN_SHARP_5),
    def("Dorian #4", Family::HarmonicMinorModes, DORIAN_SHARP_4),
    def(
        "Phrygian dominant",
        Family::HarmonicMinorModes,
        PHRYGIAN_DOMINANT,
    ),
    def("Lydian #2", Family::HarmonicMinorModes, LYDIAN_SHARP_2),
    def("Ultralocrian", Family::HarmonicMinorModes, ULTRALOCRIAN),
    def("Harmonic major", Family::MoreSevenNote, HARMONIC_MAJOR),
    def("Double harmonic", Family::MoreSevenNote, DOUBLE_HARMONIC),
    def("Hungarian minor", Family::MoreSevenNote, HUNGARIAN_MINOR),
    def("Hungarian major", Family::MoreSevenNote, HUNGARIAN_MAJOR),
    def("Neapolitan major", Family::MoreSevenNote, NEAPOLITAN_MAJOR),
    def("Neapolitan minor", Family::MoreSevenNote, NEAPOLITAN_MINOR),
    def("Persian", Family::MoreSevenNote, PERSIAN),
    def("Enigmatic", Family::MoreSevenNote, ENIGMATIC),
    def("Whole tone", Family::Symmetric, WHOLE_TONE),
    // W-H: a whole step, then a half step, and so on; H-W the other way.
    def("Diminished W-H", Family::Symmetric, DIMINISHED_WHOLE_HALF),
    def("Diminished H-W", Family::Symmetric, DIMINISHED_HALF_WHOLE),
    def("Augmented", Family::Symmetric, AUGMENTED),
    def("Bebop dominant", Family::Bebop, BEBOP_DOMINANT),
    def("Bebop major", Family::Bebop, BEBOP_MAJOR),
    def("Bebop dorian", Family::Bebop, BEBOP_DORIAN),
    def(
        "Major pentatonic",
        Family::PentatonicAndBlues,
        PENTATONIC_MAJOR,
    ),
    def(
        "Minor pentatonic",
        Family::PentatonicAndBlues,
        PENTATONIC_MINOR,
    ),
    def("Blues", Family::PentatonicAndBlues, BLUES),
    def("Major blues", Family::PentatonicAndBlues, MAJOR_BLUES),
    def("Egyptian", Family::PentatonicAndBlues, EGYPTIAN),
    def("Prometheus", Family::PentatonicAndBlues, PROMETHEUS),
    def("Hirajoshi", Family::Japanese, HIRAJOSHI),
    def("In-sen", Family::Japanese, IN_SEN),
    def("Iwato", Family::Japanese, IWATO),
    def("Yo", Family::Japanese, YO),
    def("Kumoi", Family::Japanese, KUMOI),
];

/// Other names the built-in scales go by, and their long forms, as (other
/// name, name in [`SCALES`]), for [`by_name`].
pub const ALIASES: [(&str, &str); 11] = [
    ("Ionian", "Major"),
    ("Aeolian", "Minor"),
    ("Natural minor", "Minor"),
    ("Jazz minor", "Melodic minor"),
    ("Super locrian", "Altered"),
    ("Melodic minor classical", "Mel. minor classical"),
    ("Diminished whole-half", "Diminished W-H"),
    ("Whole-half diminished", "Diminished W-H"),
    ("Diminished half-whole", "Diminished H-W"),
    ("Half-whole diminished", "Diminished H-W"),
    ("Spanish", "Phrygian dominant"),
];

/// A built-in scale by name or one of its [`ALIASES`], ignoring case,
/// spaces, `-`, `_`, `,` and `.`.
pub fn by_name(name: &str) -> Option<&'static ScaleDef> {
    let key = |s: &str| {
        s.to_ascii_lowercase()
            .replace([' ', '-', '_', ',', '.'], "")
    };
    let k = key(name);
    let k = ALIASES
        .iter()
        .find(|(other, _)| key(other) == k)
        .map_or(k, |(_, name)| key(name));
    SCALES.iter().find(|d| key(d.name) == k)
}

/// Note names, C = 0, with sharps.
pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A note's name with its octave (middle C, 60, is C4).
pub fn note_name(note: i16) -> String {
    format!(
        "{}{}",
        NOTE_NAMES[note.rem_euclid(12) as usize],
        note.div_euclid(12) - 1
    )
}

/// What happens to a note outside the scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Snap {
    /// The scale note below it.
    #[default]
    Down,
    /// The scale note above it.
    Up,
    /// The nearer of the two; the one below when they are as near.
    Nearest,
    /// It is not played.
    Skip,
}

impl Snap {
    pub const ALL: [Snap; 4] = [Snap::Down, Snap::Up, Snap::Nearest, Snap::Skip];

    pub fn label(self) -> &'static str {
        match self {
            Snap::Down => "Down",
            Snap::Up => "Up",
            Snap::Nearest => "Nearest",
            Snap::Skip => "Skip",
        }
    }
}

/// A scale: a root (0 = C … 11 = B) and which of the 12 semitones above it
/// belong to it (bit N = N semitones above the root), going up and coming
/// down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    pub root: u8,
    pub mask: u16,
    pub down: u16,
}

const fn clean(mask: u16) -> u16 {
    let mask = mask & CHROMATIC;
    if mask == 0 {
        CHROMATIC
    } else {
        mask | 1
    }
}

impl Scale {
    /// A scale with the same notes both ways. An empty mask is treated as
    /// chromatic, and the root is always in it.
    pub const fn new(root: u8, mask: u16) -> Self {
        let mask = clean(mask);
        Self {
            root: root % 12,
            mask,
            down: mask,
        }
    }

    /// A scale with other notes coming down.
    pub const fn with_down(root: u8, up: u16, down: u16) -> Self {
        Self {
            root: root % 12,
            mask: clean(up),
            down: clean(down),
        }
    }

    /// A built-in scale on `root`.
    pub const fn of(root: u8, def: &ScaleDef) -> Self {
        Self::with_down(root, def.mask, def.down)
    }

    fn has(mask: u16, root: u8, note: i16) -> bool {
        let pc = (note - i16::from(root)).rem_euclid(12);
        mask & (1 << pc) != 0
    }

    /// In the scale (going up).
    pub fn contains(&self, note: i16) -> bool {
        Self::has(self.mask, self.root, note)
    }

    /// In the scale coming down.
    pub fn contains_down(&self, note: i16) -> bool {
        Self::has(self.down, self.root, note)
    }

    pub fn is_root(&self, note: i16) -> bool {
        (note - i16::from(self.root)).rem_euclid(12) == 0
    }

    /// Notes per octave, going up.
    pub fn len(&self) -> u32 {
        self.mask.count_ones()
    }

    /// Never true: a scale always has its root.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The nearest scale note at or below `note`.
    pub fn snap(&self, note: i16) -> i16 {
        let mut n = note;
        while !self.contains(n) {
            n -= 1;
        }
        n
    }

    /// The nearest scale note at or above `note`.
    pub fn snap_up(&self, note: i16) -> i16 {
        let mut n = note;
        while !self.contains(n) {
            n += 1;
        }
        n
    }

    /// `note` brought into the scale by `snap`; None for [`Snap::Skip`]
    /// when it is outside.
    pub fn snapped(&self, note: i16, snap: Snap) -> Option<i16> {
        if self.contains(note) {
            return Some(note);
        }
        match snap {
            Snap::Down => Some(self.snap(note)),
            Snap::Up => Some(self.snap_up(note)),
            Snap::Nearest => {
                let (below, above) = (self.snap(note), self.snap_up(note));
                Some(if above - note < note - below {
                    above
                } else {
                    below
                })
            }
            Snap::Skip => None,
        }
    }

    /// Moves a scale note `degrees` scale notes up (with the notes going
    /// up) or down (with the notes coming down).
    pub fn walk(&self, note: i16, degrees: i16) -> i16 {
        let mut n = note;
        let mut left = degrees;
        while left > 0 {
            n += 1;
            while !self.contains(n) {
                n += 1;
            }
            left -= 1;
        }
        while left < 0 {
            n -= 1;
            while !self.contains_down(n) {
                n -= 1;
            }
            left += 1;
        }
        n
    }

    /// Snaps `note` into the scale (down), then moves it `degrees` scale
    /// notes up or down.
    pub fn step(&self, note: i16, degrees: i16) -> i16 {
        self.walk(self.snap(note), degrees)
    }

    /// Snaps `note` by `snap`, then moves it `degrees` scale notes; None
    /// when `snap` skips it.
    pub fn fit(&self, note: i16, degrees: i16, snap: Snap) -> Option<i16> {
        self.snapped(note, snap).map(|n| self.walk(n, degrees))
    }

    /// The `index`th scale note from the first one at or above `from`
    /// (index 0 is that note): the notes of an in-key pad layout.
    pub fn nth(&self, from: i16, index: i16) -> i16 {
        self.walk(self.snap_up(from), index)
    }
}

/// Where a part (the arp, the synth pads, a track) takes its scale from.
/// By default a part has its own scale, Off until one is picked. A project
/// key exists only when the owner makes a project with one; parts then
/// follow it, and each can be switched back to its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartScale {
    /// The part's own scale; None is Off.
    pub own: Option<Scale>,
    /// Follow the project's key when the project has one.
    pub follow_project: bool,
}

impl Default for PartScale {
    fn default() -> Self {
        Self {
            own: None,
            follow_project: true,
        }
    }
}

impl PartScale {
    /// The scale the part plays in, given the project's key (None when the
    /// project has none).
    pub fn resolve(&self, project_key: Option<Scale>) -> Option<Scale> {
        match project_key {
            Some(k) if self.follow_project => Some(k),
            _ => self.own,
        }
    }
}

/// Reads a custom scale: the notes going up, then optionally `/` and the
/// notes coming down. Each set is written either as semitones above the
/// root, starting from `0` (`0 2 3 5 7 9 11`), or as degrees, starting
/// from `1`, with `b` or `#` before a degree (`1 2 b3 4 5 6 7`). A set with
/// a `b` or `#`, or with `1` and no `0`, is read as degrees. The root is
/// always included. Returns (up, down) masks.
pub fn parse_steps(text: &str) -> Result<(u16, u16), String> {
    let (up, down) = match text.split_once('/') {
        Some((u, d)) => (u, Some(d)),
        None => (text, None),
    };
    let up = parse_set(up)?;
    let down = match down {
        Some(d) => parse_set(d)?,
        None => up,
    };
    Ok((up, down))
}

fn parse_set(text: &str) -> Result<u16, String> {
    let toks: Vec<&str> = text.split_whitespace().collect();
    if toks.is_empty() {
        return Err("no notes".to_string());
    }
    let degrees = toks.iter().any(|t| t.starts_with(['b', '#']))
        || (toks.contains(&"1") && !toks.contains(&"0"));
    let mut m = 1u16;
    for tok in toks {
        let n = if degrees {
            degree_of(tok)
        } else {
            tok.parse::<u8>().ok().filter(|&n| n <= 11)
        };
        let what = if degrees {
            "a degree (1 to 7, with b or # before it)"
        } else {
            "a semitone (0 to 11)"
        };
        m |= 1 << n.ok_or_else(|| format!("\"{tok}\" is not {what}"))?;
    }
    Ok(m)
}

fn degree_of(tok: &str) -> Option<u8> {
    let degree = tok.trim_start_matches(['b', '#']);
    let accidentals = &tok[..tok.len() - degree.len()];
    let shift: i16 = accidentals
        .chars()
        .map(|c| if c == '#' { 1 } else { -1 })
        .sum();
    let n: u8 = degree.parse().ok()?;
    let base: i16 = match n {
        1 => 0,
        2 => 2,
        3 => 4,
        4 => 5,
        5 => 7,
        6 => 9,
        7 => 11,
        _ => return None,
    };
    Some((base + shift).rem_euclid(12) as u8)
}

/// A mask as semitones above the root ("0 2 3 5 7 9 11"), for saving.
pub fn steps_text(mask: u16) -> String {
    (0..12)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A custom scale written back: up, and `/ down` when it differs.
pub fn scale_text(up: u16, down: u16) -> String {
    if up == down {
        steps_text(up)
    } else {
        format!("{} / {}", steps_text(up), steps_text(down))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_and_steps_through_scales() {
        let c_major = Scale::new(0, MAJOR);
        assert!(c_major.contains(60) && !c_major.contains(61));
        assert_eq!(c_major.snap(61), 60);
        assert_eq!(c_major.step(60, 2), 64);
        assert_eq!(c_major.step(60, 7), 72);
        assert_eq!(c_major.step(61, 1), 62, "C# snaps to C, then one up");
        assert_eq!(c_major.step(60, -1), 59);
        let a_minor_pent = Scale::new(9, PENTATONIC_MINOR);
        assert_eq!(a_minor_pent.step(57, 1), 60);
        assert_eq!(a_minor_pent.snap(59), 57);
        assert_eq!(
            Scale::new(14, 0),
            Scale {
                root: 2,
                mask: CHROMATIC,
                down: CHROMATIC
            }
        );
    }

    #[test]
    fn the_library_is_whole_and_well_formed() {
        assert_eq!(SCALES[0].name, "Chromatic");
        for (i, d) in SCALES.iter().enumerate() {
            assert!(
                d.mask & 1 == 1 && d.down & 1 == 1,
                "{} has its root",
                d.name
            );
            assert!(d.mask <= CHROMATIC && d.down <= CHROMATIC, "{}", d.name);
            assert!(
                SCALES[..i]
                    .iter()
                    .all(|e| !e.name.eq_ignore_ascii_case(d.name)),
                "{} twice",
                d.name
            );
            // The built-ins are all distinct sets of notes.
            assert!(
                SCALES[..i]
                    .iter()
                    .all(|e| (e.mask, e.down) != (d.mask, d.down)),
                "{} repeats another",
                d.name
            );
        }
        // Families are in order, each one together.
        let fams: Vec<Family> = SCALES.iter().map(|d| d.family).collect();
        let mut seen = Vec::new();
        for f in fams {
            if seen.last() != Some(&f) {
                assert!(!seen.contains(&f), "{f:?} is split");
                seen.push(f);
            }
        }
        assert_eq!(seen, Family::ALL);
        // Every mode of major is a rotation of major (the same notes from
        // another root), and likewise for melodic and harmonic minor.
        let rotations = |m: u16| {
            (0..12)
                .filter(|r| m & (1 << r) != 0)
                .map(|r| ((m >> r) | (m << (12 - r))) & CHROMATIC)
                .collect::<Vec<_>>()
        };
        for (family, parent) in [
            (Family::MajorModes, MAJOR),
            (Family::MelodicMinorModes, MELODIC_MINOR),
            (Family::HarmonicMinorModes, HARMONIC_MINOR),
        ] {
            let r = rotations(parent);
            for d in SCALES
                .iter()
                .filter(|d| d.family == family && d.mask == d.down)
            {
                assert!(r.contains(&d.mask), "{} is a mode of its family", d.name);
            }
            assert_eq!(
                SCALES
                    .iter()
                    .filter(|d| d.family == family && d.mask == d.down)
                    .count(),
                7,
                "{family:?}"
            );
        }
        assert_eq!(by_name("dorian b2").map(|d| d.mask), Some(DORIAN_FLAT_2));
        assert_eq!(
            by_name("Diminished half-whole").map(|d| d.mask),
            Some(DIMINISHED_HALF_WHOLE)
        );
        assert_eq!(
            by_name("harmonic_minor").map(|d| d.mask),
            Some(HARMONIC_MINOR)
        );
        assert_eq!(by_name("Aeolian").map(|d| d.mask), Some(MINOR));
        assert_eq!(by_name("jazz minor").map(|d| d.mask), Some(MELODIC_MINOR));
        for (other, name) in ALIASES {
            assert!(
                SCALES.iter().any(|d| d.name == name),
                "{other} names a scale that is there"
            );
            assert!(
                !SCALES.iter().any(|d| d.name.eq_ignore_ascii_case(other)),
                "{other} is not a scale's own name"
            );
        }
        for d in SCALES {
            assert!(d.name.len() <= 20, "{} fits a menu value box", d.name);
        }
        assert!(by_name("klingon").is_none());
    }

    #[test]
    fn snap_modes() {
        let c = Scale::new(0, MAJOR);
        assert_eq!(c.snapped(61, Snap::Down), Some(60));
        assert_eq!(c.snapped(61, Snap::Up), Some(62));
        assert_eq!(
            c.snapped(61, Snap::Nearest),
            Some(60),
            "as near both ways: the one below"
        );
        assert_eq!(c.snapped(61, Snap::Skip), None);
        assert_eq!(c.snapped(62, Snap::Skip), Some(62), "in the scale: kept");
        let blues = Scale::new(0, BLUES);
        // E (64) lies between Eb (63) and F (65): as near both ways.
        assert_eq!(blues.snapped(64, Snap::Nearest), Some(63));
        // D (62): C is 2 below, Eb 1 above.
        assert_eq!(blues.snapped(62, Snap::Nearest), Some(63));
        assert_eq!(c.fit(61, 2, Snap::Up), Some(65));
        assert_eq!(c.fit(61, 2, Snap::Skip), None);
    }

    #[test]
    fn going_down_uses_the_notes_coming_down() {
        let a = Scale::of(9, by_name("Melodic minor, classical").unwrap());
        assert_eq!(
            by_name("mel minor classical").map(|d| d.name),
            Some("Mel. minor classical")
        );
        // A melodic minor up: A B C D E F# G# A.
        let up: Vec<i16> = (0..8).map(|i| a.walk(57, i)).collect();
        assert_eq!(up, [57, 59, 60, 62, 64, 66, 68, 69]);
        // Down from A: G F E D C B A (natural minor).
        let down: Vec<i16> = (0..8).map(|i| a.walk(69, -i)).collect();
        assert_eq!(down, [69, 67, 65, 64, 62, 60, 59, 57]);
        assert!(a.contains(66) && !a.contains_down(66));
    }

    #[test]
    fn in_key_pad_notes_from_any_start() {
        let d_dorian = Scale::new(2, DORIAN);
        // From C3 (48): the first D dorian note at or above it is C (48).
        let pads: Vec<i16> = (0..9).map(|i| d_dorian.nth(48, i)).collect();
        assert_eq!(pads, [48, 50, 52, 53, 55, 57, 59, 60, 62]);
        let e_hira = Scale::new(4, HIRAJOSHI);
        // E hirajoshi is E F# G B C: from C3 the pads run C E F# G B C.
        let pads: Vec<i16> = (0..6).map(|i| e_hira.nth(48, i)).collect();
        assert_eq!(pads, [48, 52, 54, 55, 59, 60]);
        assert_eq!(e_hira.nth(49, 0), 52, "from C#: the next scale note, E");
    }

    #[test]
    fn a_part_has_its_own_scale_unless_the_project_has_a_key() {
        let own = Scale::new(2, DORIAN);
        let key = Scale::new(7, MAJOR);
        let off = PartScale::default();
        assert_eq!(off.resolve(None), None, "off by default, no forced key");
        assert_eq!(
            off.resolve(Some(key)),
            Some(key),
            "a project with a key: parts follow it"
        );
        let mine = PartScale {
            own: Some(own),
            follow_project: true,
        };
        assert_eq!(mine.resolve(None), Some(own));
        assert_eq!(mine.resolve(Some(key)), Some(key));
        let apart = PartScale {
            own: Some(own),
            follow_project: false,
        };
        assert_eq!(
            apart.resolve(Some(key)),
            Some(own),
            "switched back to its own"
        );
    }

    #[test]
    fn custom_scales_from_semitones_or_degrees() {
        assert_eq!(
            parse_steps("0 2 3 5 7 9 11"),
            Ok((MELODIC_MINOR, MELODIC_MINOR))
        );
        assert_eq!(
            parse_steps("1 2 b3 4 5 6 7 / 1 2 b3 4 5 b6 b7"),
            Ok((MELODIC_MINOR, MINOR))
        );
        assert_eq!(
            parse_steps("2 b3 5"),
            Ok((mask(&[0, 2, 3, 7]), mask(&[0, 2, 3, 7]))),
            "the root is added"
        );
        assert_eq!(
            parse_steps("0 1 4 5 7 8 10 11"),
            Ok((
                mask(&[0, 1, 4, 5, 7, 8, 10, 11]),
                mask(&[0, 1, 4, 5, 7, 8, 10, 11])
            ))
        );
        assert_eq!(
            parse_steps("#4 bb7"),
            Ok((mask(&[0, 6, 9]), mask(&[0, 6, 9])))
        );
        assert!(parse_steps("0 2 12")
            .unwrap_err()
            .contains("\"12\" is not a semitone"));
        assert!(parse_steps("1 2 8")
            .unwrap_err()
            .contains("\"8\" is not a degree"));
        assert_eq!(
            parse_steps("2 4 7").map(|p| p.0),
            Ok(mask(&[0, 2, 4, 7])),
            "no 1 and no accidentals: semitones"
        );
        assert_eq!(
            parse_steps("1 2 3 5 6").map(|p| p.0),
            Ok(PENTATONIC_MAJOR),
            "degrees"
        );
        assert!(parse_steps("1 2 x").is_err());
        assert!(parse_steps("").is_err());
        assert!(parse_steps("0 2 /").is_err());
        assert_eq!(steps_text(MAJOR), "0 2 4 5 7 9 11");
        assert_eq!(
            scale_text(MELODIC_MINOR, MINOR),
            "0 2 3 5 7 9 11 / 0 2 3 5 7 8 10"
        );
        for d in SCALES {
            assert_eq!(
                parse_steps(&scale_text(d.mask, d.down)),
                Ok((d.mask, d.down)),
                "{}",
                d.name
            );
        }
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(49), "C#3");
    }
}
