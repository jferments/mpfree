//! Phrases: hold one note and a whole riff plays from it.
//!
//! A phrase is a short piece of music written from a home note: a bass
//! line, a riff, chord stabs or a line that moves, each note with its own
//! start, length and loudness. Held on the Arp panel's Phrase tab
//! ([`crate::Mode::Phrase`]), the phrase plays from the note you hold, in
//! its own rhythm, and moves to each new note you hold, as the MPC's phrase
//! player does (MPC 3.10.1 user guide, pp. 82 and 83: "Hold down a single
//! note to trigger a melodic phrase", which "will be transposed").
//!
//! A phrase is written as text on a grid, one token per step, so a phrase
//! fits on one line of a library file, as a latch pattern does:
//!
//! ```text
//! bass minor : 1 . 1 1 . 1 . 5- 1 . b7- . 5- . b7- 1
//! chords either 1/8 : 1,3,5! . . 1,3,5' . 1,3,5 . .
//! riff minor swung : 5 . b7 1+ _ . b7 5 | vel 110 80 90 100
//! ```
//!
//! The words before `:` say what it is: its kind (`bass`, `riff`, `chords`
//! or `line`), the scale it was written in (`major`, `minor`, or `either`
//! when it works in both), the grid it is written on (`1/8`, `1/8T`,
//! `1/16`, the default, `1/16T`, `1/32`, `1/32T`), its meter (`4/4`, the
//! default, or `3/4`), `swung` when it was written to swing, and `N bars`
//! when it is longer than its steps (1 to 8).
//!
//! | Token | Step |
//! |---|---|
//! | `.` | rest |
//! | `_` | tie: the notes of the step before hold on through this one |
//! | `1`..`7` | a step of the scale from the home note (`1` is the home note) |
//! | `b3`, `#4` | the same, lowered or raised a half step (from the major scale's, as degrees are usually named) |
//! | `5-`, `1+` | an octave down or up (up to four) |
//! | `1,b3,5` | notes together, a chord |
//!
//! After a note or a chord: `!` accents it, `'` makes it short, `~` holds
//! it for the whole step. Lanes follow after a `|`, each looping at its own
//! length as a pattern's do: `vel` (1 to 127; 0 is the phrase's 100),
//! `gate` (1 to 100 percent of the last step; 0 is the token's) and `nudge`
//! (early or late by up to half a step, in percent). A phrase has no random
//! lanes: it plays the same notes every time, so a recording of it plays
//! back as heard.
//!
//! Parsing and writing text allocate, so they belong on the screen's or a
//! worker's thread; a [`Phrase`] itself is fixed size, so the engine can
//! hold and swap one on the audio thread.

use crate::engine::{Rate, PPQ};
use crate::pattern::{Pattern, Step};
use crate::scale::{Scale, Snap};

/// Most notes in a phrase.
pub const MAX_PHRASE_NOTES: usize = 256;
/// Longest phrase, in bars.
pub const MAX_BARS: u8 = 8;
/// Velocity of a note with no velocity of its own.
pub const PLAIN_VELOCITY: u8 = 100;
/// Velocity of an accented note (`!`).
pub const ACCENT_VELOCITY: u8 = 127;
/// Gate of a plain note, in percent of its last step.
pub const GATE_WRITTEN: u8 = 80;
/// Gate of a short note (`'`).
pub const GATE_SHORT: u8 = 25;
/// Gate of a held note (`~`).
pub const GATE_FULL: u8 = 100;
/// Most octaves a note may be moved from its scale step.
const MAX_OCTAVES: i8 = 4;

/// What a phrase is for, as the library shelves them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    Bass,
    /// A hook or a lead line.
    #[default]
    Riff,
    /// Stabs and comps, as chord shapes over the held note.
    Chords,
    /// A pulse that moves, like an arpeggio written out.
    Line,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Bass, Kind::Riff, Kind::Chords, Kind::Line];

    /// The name a screen shows.
    pub const fn label(self) -> &'static str {
        match self {
            Kind::Bass => "Bass",
            Kind::Riff => "Riff",
            Kind::Chords => "Chords",
            Kind::Line => "Arp lines",
        }
    }

    /// The word in the text form.
    pub const fn word(self) -> &'static str {
        match self {
            Kind::Bass => "bass",
            Kind::Riff => "riff",
            Kind::Chords => "chords",
            Kind::Line => "line",
        }
    }
}

/// The scale a phrase was written in. Its steps are counted in that scale,
/// and a note outside it (a `#4` in minor) keeps its distance from the
/// held note wherever the phrase is held ([`Phrase::pitch`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Written {
    Major,
    Minor,
    /// Works in either: In key it moves by steps of the scale alone.
    #[default]
    Either,
}

impl Written {
    pub const ALL: [Written; 3] = [Written::Major, Written::Minor, Written::Either];

    pub const fn word(self) -> &'static str {
        match self {
            Written::Major => "major",
            Written::Minor => "minor",
            Written::Either => "either",
        }
    }

    /// Semitones from the home note to each of its seven steps; Either
    /// counts from the major scale, as degrees are named.
    const fn steps(self) -> [i8; 7] {
        match self {
            Written::Minor => [0, 2, 3, 5, 7, 8, 10],
            Written::Major | Written::Either => [0, 2, 4, 5, 7, 9, 11],
        }
    }

    /// Whether a phrase written in this scale suits `key`, for a list
    /// that shows the ones that don't last: a major phrase suits a key
    /// with a major third, a minor one a key with a minor third, and one
    /// written for either suits any. A key with both thirds or neither
    /// (chromatic, a blues scale with both, a scale of fourths) is of no
    /// kind, so every phrase suits it, as every phrase suits no key. In
    /// key, a phrase still plays in any key: its steps follow the key's.
    pub const fn suits(self, key: Option<Scale>) -> bool {
        let Some(key) = key else {
            return true;
        };
        let minor = key.mask & (1 << 3) != 0;
        let major = key.mask & (1 << 4) != 0;
        match self {
            Written::Either => true,
            Written::Major => major || !minor,
            Written::Minor => minor || !major,
        }
    }
}

/// How a phrase's rhythm feels, as the list says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feel {
    Straight,
    /// Written to swing.
    Swung,
    /// Written on a triplet grid.
    Triplet,
}

impl Feel {
    pub const ALL: [Feel; 3] = [Feel::Straight, Feel::Swung, Feel::Triplet];

    pub const fn label(self) -> &'static str {
        match self {
            Feel::Straight => "Straight",
            Feel::Swung => "Swung",
            Feel::Triplet => "Triplet",
        }
    }
}

/// How a phrase follows the held note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    /// By steps of the arp's scale, so a riff stays in the key; with no
    /// scale set, as [`Follow::Exact`].
    #[default]
    InKey,
    /// By semitones, keeping every interval as written.
    Exact,
}

impl Follow {
    pub const ALL: [Follow; 2] = [Follow::InKey, Follow::Exact];

    pub const fn label(self) -> &'static str {
        match self {
            Follow::InKey => "In key",
            Follow::Exact => "Exact",
        }
    }
}

/// How fast a phrase plays against its written rhythm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Speed {
    Half,
    #[default]
    AsWritten,
    Double,
}

impl Speed {
    pub const ALL: [Speed; 3] = [Speed::Half, Speed::AsWritten, Speed::Double];

    pub const fn label(self) -> &'static str {
        match self {
            Speed::Half => "Half",
            Speed::AsWritten => "As written",
            Speed::Double => "Double",
        }
    }

    /// Written ticks played per tick of time.
    pub const fn factor(self) -> f64 {
        match self {
            Speed::Half => 0.5,
            Speed::AsWritten => 1.0,
            Speed::Double => 2.0,
        }
    }
}

/// One note of a phrase, from its home note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhraseNote {
    /// Where it starts, in ticks ([`PPQ`] to a beat) from the phrase's
    /// start, as written.
    pub start: u32,
    /// How long it sounds, in ticks, at least 1.
    pub len: u32,
    /// Steps of the written scale from the home note, seven to an octave
    /// (7 is the home note an octave up).
    pub steps: i8,
    /// Half steps off that step of the written scale: a note outside it.
    /// Always 0 in a phrase written for either scale.
    pub alt: i8,
    /// Semitones from the home note, as written: what Exact plays.
    pub semis: i8,
    /// 1..=127.
    pub velocity: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based position of the token that is wrong (0: the phrase as a whole).
    pub token: usize,
    pub reason: &'static str,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.token == 0 {
            write!(f, "{}", self.reason)
        } else {
            write!(f, "token {}: {}", self.token, self.reason)
        }
    }
}

impl std::error::Error for ParseError {}

/// A phrase: its notes in time order, and what it is. Fixed size, so the
/// engine holds it without allocating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phrase {
    notes: [PhraseNote; MAX_PHRASE_NOTES],
    len: usize,
    pub kind: Kind,
    pub written: Written,
    /// Written to swing (a label for the list; the swing played is the
    /// transport's or the arp's, as on the other tabs).
    pub swung: bool,
    /// The grid it is written on.
    pub unit: Rate,
    /// Beats to a bar: 4 (4/4) or 3 (3/4).
    pub beats: u8,
    /// 1..=[`MAX_BARS`].
    pub bars: u8,
}

impl Default for Phrase {
    /// The home note on every beat of one bar.
    fn default() -> Self {
        let mut p = Self::empty();
        for b in 0..4 {
            let _ = p.push(PhraseNote {
                start: b * PPQ,
                len: PPQ / 2,
                steps: 0,
                alt: 0,
                semis: 0,
                velocity: PLAIN_VELOCITY,
            });
        }
        p
    }
}

impl Phrase {
    /// One bar of 4/4 on 1/16s with no notes.
    pub const fn empty() -> Self {
        Self {
            notes: [PhraseNote {
                start: 0,
                len: 1,
                steps: 0,
                alt: 0,
                semis: 0,
                velocity: PLAIN_VELOCITY,
            }; MAX_PHRASE_NOTES],
            len: 0,
            kind: Kind::Riff,
            written: Written::Either,
            swung: false,
            unit: Rate::Sixteenth,
            beats: 4,
            bars: 1,
        }
    }

    /// Its notes, in time order (lowest first where they start together).
    pub fn notes(&self) -> &[PhraseNote] {
        &self.notes[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn bar_ticks(&self) -> u32 {
        u32::from(self.beats.max(1)) * PPQ
    }

    /// Its whole length, in written ticks: it loops from there.
    pub fn ticks(&self) -> u32 {
        self.bar_ticks() * u32::from(self.bars.clamp(1, MAX_BARS))
    }

    /// Its feel: triplet on a triplet grid, else swung or straight as it
    /// was written.
    pub fn feel(&self) -> Feel {
        match self.unit {
            Rate::EighthTriplet | Rate::SixteenthTriplet | Rate::ThirtySecondTriplet => {
                Feel::Triplet
            }
            _ if self.swung => Feel::Swung,
            _ => Feel::Straight,
        }
    }

    /// Steps of its grid to a bar.
    pub fn steps_per_bar(&self) -> u32 {
        (self.bar_ticks() / self.unit.ticks()).max(1)
    }

    /// Adds a note, keeping time order; false when it is full or the note
    /// starts past the end.
    pub fn push(&mut self, n: PhraseNote) -> bool {
        if self.len == MAX_PHRASE_NOTES || n.start >= self.ticks() {
            return false;
        }
        let n = PhraseNote {
            len: n.len.max(1),
            velocity: n.velocity.clamp(1, 127),
            ..n
        };
        let at = self.notes[..self.len]
            .iter()
            .position(|m| (m.start, m.semis) > (n.start, n.semis))
            .unwrap_or(self.len);
        self.notes.copy_within(at..self.len, at + 1);
        self.notes[at] = n;
        self.len += 1;
        true
    }

    /// The note `n` plays from `held`, or None when it falls outside
    /// 0..=127 or the scale's snap skips the held note. In key with a
    /// scale, the held note is brought into the scale by `snap` and the
    /// note moves from it by steps of the scale; otherwise by semitones.
    ///
    /// A note written outside its scale (a blue note, a leading tone)
    /// keeps the distance it was written at from the held note, In key
    /// too. Moving it by the key's step and then by its half step lands
    /// it on its neighbour wherever the key's own step is already there:
    /// the lowered 7th of a major walk held on the key's fifth, where the
    /// seventh step is lowered already, would play the 6th twice.
    pub fn pitch(
        &self,
        n: &PhraseNote,
        held: u8,
        follow: Follow,
        scale: Option<Scale>,
        snap: Snap,
    ) -> Option<u8> {
        let held = i16::from(held);
        let note = match (follow, scale) {
            (Follow::InKey, Some(scale)) => {
                let base = scale.snapped(held, snap)?;
                if n.alt != 0 {
                    base + i16::from(n.semis)
                } else {
                    let steps = i16::from(n.steps);
                    let (octave, step) = (steps.div_euclid(7), steps.rem_euclid(7));
                    let per_octave = scale.len() as i16;
                    let j = if per_octave == 7 {
                        step
                    } else {
                        nearest_step(&scale, base, self.written.steps()[step as usize])
                    };
                    scale.walk(base, octave * per_octave + j)
                }
            }
            _ => held + i16::from(n.semis),
        };
        (0..=127).contains(&note).then_some(note as u8)
    }

    /// How long note `i` sounds with the arp's gate (1 to 100 %), in
    /// written ticks: at 50 % as written, below it shorter in proportion,
    /// above it held towards the next note's start, which it reaches at
    /// 100 %.
    pub fn gated(&self, i: usize, gate: u8) -> f64 {
        let notes = self.notes();
        let n = notes[i];
        let len = f64::from(n.len);
        let gate = f64::from(gate.clamp(1, 100));
        if gate <= 50.0 {
            return (len * gate / 50.0).max(1.0);
        }
        let total = f64::from(self.ticks());
        // To the next note that starts later, round the phrase's end.
        let gap = notes
            .iter()
            .map(|m| {
                let d = f64::from(m.start) - f64::from(n.start);
                if d > 0.0 {
                    d
                } else {
                    d + total
                }
            })
            .fold(total, f64::min);
        len + (gap - len).max(0.0) * (gate - 50.0) / 50.0
    }

    /// A phrase from notes as they were played, from `home`: `notes` are
    /// (start, length, note, velocity) in ticks from the phrase's start,
    /// written in `written` (or either), on `unit`'s grid of `beats` to a
    /// bar. Notes past [`MAX_BARS`] or the first [`MAX_PHRASE_NOTES`], and
    /// notes more than four octaves from home, are left out. Starts and
    /// lengths are kept to the tick; [`Phrase::to_text`] writes them on the
    /// grid.
    pub fn from_notes(
        notes: &[(u32, u32, u8, u8)],
        home: u8,
        written: Written,
        unit: Rate,
        beats: u8,
    ) -> Self {
        let mut p = Self {
            written,
            unit,
            beats: beats.clamp(1, 12),
            bars: MAX_BARS,
            ..Self::empty()
        };
        let mut end = 0;
        for &(start, len, note, velocity) in notes {
            let semis = i16::from(note) - i16::from(home);
            if semis.unsigned_abs() > 12 * MAX_OCTAVES as u16 {
                continue;
            }
            let (steps, alt) = steps_of(written, semis as i8);
            if p.push(PhraseNote {
                start,
                len,
                steps,
                alt,
                semis: semis as i8,
                velocity,
            }) {
                end = end.max(start + 1);
            }
        }
        let bar = p.bar_ticks();
        p.bars = end.div_ceil(bar).clamp(1, u32::from(MAX_BARS)) as u8;
        p
    }

    /// A latch pattern played on one held note, as a phrase (the Yours
    /// shelf's "a pattern that plays one note"): every step that plays
    /// sounds `home`, moved by its octaves (the step's and the `oct`
    /// lane's), as loud as its accent or the `vel` lane says (else
    /// `velocity`), as long as its gate (the step's, the `gate` lane's,
    /// else `gate`, the arp's setting, in percent of the step), with its
    /// ratchets and its nudge; a tie holds the notes before it on. The
    /// steps, a `rate` apart, repeat until they fill whole bars of `beats`
    /// (at most [`MAX_BARS`]), so the phrase loops as the pattern did.
    /// Scale degrees, chances and random amounts are left out: a phrase
    /// plays the same notes every time. Written for either scale, on the
    /// pattern's rate (1/8 for a pattern at 1/4).
    pub fn from_pattern(
        pattern: &Pattern,
        rate: Rate,
        beats: u8,
        gate: u8,
        velocity: u8,
        home: u8,
    ) -> Self {
        let beats = beats.clamp(1, 12);
        let step = rate.ticks();
        let bar = u32::from(beats) * PPQ;
        let n = pattern.len().max(1) as u32;
        let most = u32::from(MAX_BARS) * bar;
        // Whole bars: the steps repeated until both line up, or as many
        // as eight bars hold.
        let round = n * step;
        let whole = round / gcd(round, bar) * bar;
        let total = if whole <= most {
            whole / step
        } else {
            most / step
        };
        let lanes = &pattern.lanes;
        let mut notes: Vec<(u32, u32, u8, u8)> = Vec::new();
        // The notes the last playing step started, to hold on over a tie.
        let mut last: core::ops::Range<usize> = 0..0;
        for i in 0..total {
            let at = i * step;
            let k = u64::from(i);
            match pattern.steps()[(i % n) as usize] {
                Step::Rest => last = notes.len()..notes.len(),
                Step::Tie => {
                    for t in &mut notes[last.clone()] {
                        t.1 = at + step - t.0;
                    }
                }
                Step::Play {
                    octave,
                    accent,
                    gate: g,
                    ratchet,
                    ..
                } => {
                    let octave = i16::from(octave) + i16::from(lanes.octave.at(k).unwrap_or(0));
                    let note = (i16::from(home) + 12 * octave.clamp(-3, 3)).clamp(0, 127) as u8;
                    let vel = match lanes.velocity.at(k) {
                        _ if accent => ACCENT_VELOCITY,
                        Some(v) if v > 0 => v,
                        _ => velocity,
                    };
                    let g = [g, lanes.gate.at(k).unwrap_or(0), gate]
                        .into_iter()
                        .find(|g| *g > 0)
                        .unwrap_or(GATE_WRITTEN)
                        .min(100);
                    let nudge = i64::from(lanes.nudge.at(k).unwrap_or(0).clamp(-50, 50));
                    let start = (i64::from(at) + i64::from(step) * nudge / 100).max(0) as u32;
                    let hits = u32::from(ratchet.clamp(1, 8));
                    let each = step / hits;
                    let first = notes.len();
                    for h in 0..hits {
                        let len = (each * u32::from(g) / 100).max(1);
                        notes.push((start + h * each, len, note, vel));
                    }
                    last = first..notes.len();
                }
            }
        }
        let unit = if rate == Rate::Quarter {
            Rate::Eighth
        } else {
            rate
        };
        let mut p = Self::from_notes(&notes, home, Written::Either, unit, beats);
        p.bars = (total * step).div_ceil(bar).clamp(1, u32::from(MAX_BARS)) as u8;
        p
    }

    /// Reads the text form described in the module docs.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let whole = |reason| ParseError { token: 0, reason };
        let (head, body) = text
            .split_once(':')
            .ok_or(whole("a phrase needs its steps after a ':'"))?;
        let mut p = Self::empty();
        let mut bars: Option<u8> = None;
        let mut token = 0;
        let mut words = head.split_whitespace();
        while let Some(w) = words.next() {
            token += 1;
            let t = token;
            let at = move |reason| ParseError { token: t, reason };
            if let Some(k) = Kind::ALL.iter().find(|k| k.word() == w) {
                p.kind = *k;
            } else if let Some(s) = Written::ALL.iter().find(|s| s.word() == w) {
                p.written = *s;
            } else if let Some(r) = Rate::ALL.iter().find(|r| r.label().eq_ignore_ascii_case(w)) {
                if *r == Rate::Quarter {
                    return Err(at("a phrase's grid is 1/8 to 1/32T"));
                }
                p.unit = *r;
            } else if w == "4/4" {
                p.beats = 4;
            } else if w == "3/4" {
                p.beats = 3;
            } else if w == "swung" {
                p.swung = true;
            } else if let Ok(n) = w.parse::<u8>() {
                token += 1;
                match words.next() {
                    Some("bar" | "bars") if (1..=MAX_BARS).contains(&n) => bars = Some(n),
                    Some("bar" | "bars") => return Err(at("a phrase is 1 to 8 bars")),
                    _ => return Err(at("a number in the header must be followed by bars")),
                }
            } else {
                return Err(at(
                    "unknown word (a kind, a scale, a grid, a meter, swung or N bars)",
                ));
            }
        }
        let mut parts = body.split('|');
        // Steps: each a rest, a tie, or notes with their marks.
        let mut steps: Vec<Option<StepNotes>> = Vec::new();
        for word in parts.next().unwrap_or("").split_whitespace() {
            token += 1;
            let t = token;
            let at = move |reason| ParseError { token: t, reason };
            steps.push(match word {
                "." => Some(StepNotes::Rest),
                "_" => Some(StepNotes::Tie),
                _ => Some(parse_notes(word).map_err(at)?),
            });
        }
        if steps.is_empty() {
            return Err(whole("no steps"));
        }
        let mut lanes = TextLanes::default();
        for part in parts {
            token += 1;
            let mut words = part.split_whitespace();
            let t = token;
            let at = move |reason| ParseError { token: t, reason };
            let name = words.next().ok_or(at("an empty lane after |"))?;
            let (lo, hi) = match name {
                "vel" => (0, 127),
                "gate" => (0, 100),
                "nudge" => (-50, 50),
                _ => return Err(at("unknown lane (use vel, gate or nudge)")),
            };
            let mut vals = Vec::new();
            for w in words {
                token += 1;
                let t = token;
                let at = move |reason| ParseError { token: t, reason };
                let v: i32 = w
                    .parse()
                    .map_err(|_| at("a lane value must be a whole number"))?;
                if !(lo..=hi).contains(&v) {
                    return Err(at(match name {
                        "vel" => "velocity must be 0 to 127",
                        "gate" => "gate must be 0 to 100",
                        _ => "nudge must be -50 to 50",
                    }));
                }
                vals.push(v);
            }
            if vals.is_empty() {
                return Err(at("a lane needs at least one value"));
            }
            match name {
                "vel" => lanes.vel = vals,
                "gate" => lanes.gate = vals,
                _ => lanes.nudge = vals,
            }
        }
        let spb = p.steps_per_bar() as usize;
        let need = steps.len().div_ceil(spb);
        p.bars = match bars {
            Some(b) if usize::from(b) < need => return Err(whole("more steps than its bars hold")),
            Some(b) => b,
            None if need > usize::from(MAX_BARS) => return Err(whole("a phrase is 1 to 8 bars")),
            None => need as u8,
        };
        let unit = p.unit.ticks();
        for (i, s) in steps.iter().enumerate() {
            let Some(StepNotes::Notes { notes, mark }) = s else {
                continue;
            };
            let ties = steps[i + 1..]
                .iter()
                .take_while(|t| matches!(t, Some(StepNotes::Tie)))
                .count() as u32;
            let gate = match (lanes.at(&lanes.gate, i), mark.gate) {
                (Some(g), _) if g > 0 => g as u32,
                (_, Some(g)) => u32::from(g),
                _ => u32::from(GATE_WRITTEN),
            };
            let velocity = match (mark.accent, lanes.at(&lanes.vel, i)) {
                (true, _) => ACCENT_VELOCITY,
                (false, Some(v)) if v > 0 => v as u8,
                _ => PLAIN_VELOCITY,
            };
            let nudge = lanes.at(&lanes.nudge, i).unwrap_or(0);
            let base = i as i64 * i64::from(unit) + i64::from(nudge) * i64::from(unit) / 100;
            let start = base.clamp(0, i64::from(p.ticks()) - 1) as u32;
            let len = (ties * unit + unit * gate / 100).max(1);
            for &(steps, semis) in notes {
                let alt = written_alt(p.written, steps, semis);
                if !p.push(PhraseNote {
                    start,
                    len,
                    steps,
                    alt,
                    semis,
                    velocity,
                }) {
                    return Err(whole("more than 256 notes"));
                }
            }
        }
        Ok(p)
    }

    /// The text form: every step of every bar, then the lanes it needs.
    /// Notes start on the grid's steps (a nudge lane holds what is off
    /// it), notes that start together share a step and its length, and a
    /// note is tied on until the next note starts.
    pub fn to_text(&self) -> String {
        let unit = self.unit.ticks();
        let total = (self.ticks() / unit) as usize;
        let mut head = vec![self.kind.word(), self.written.word()];
        if self.unit != Rate::Sixteenth {
            head.push(self.unit.label());
        }
        if self.beats == 3 {
            head.push("3/4");
        }
        if self.swung {
            head.push("swung");
        }
        // Each step's notes: the nearest step to where each note starts.
        let mut at: Vec<Vec<PhraseNote>> = vec![Vec::new(); total];
        for n in self.notes() {
            let i = ((n.start + unit / 2) / unit) as usize;
            at[i.min(total - 1)].push(*n);
        }
        let mut tokens: Vec<String> = Vec::with_capacity(total);
        let (mut vel, mut gate, mut nudge) =
            (vec![None; total], vec![None; total], vec![None; total]);
        let mut i = 0;
        while i < total {
            let Some(first) = at[i].first().copied() else {
                tokens.push(".".into());
                i += 1;
                continue;
            };
            let names: Vec<String> = at[i].iter().map(degree_name).collect();
            let mut t = names.join(",");
            // Length: whole steps tied on, then the gate of the last one.
            let next = (i + 1..total).find(|&k| !at[k].is_empty()).unwrap_or(total);
            let len = first.len;
            let whole = ((len.saturating_sub(1)) / unit) as usize;
            let ties = whole.min(next - i - 1);
            let rest = len.saturating_sub(ties as u32 * unit);
            let g = (rest * 100).div_ceil(unit).clamp(1, 100) as u8;
            gate[i] = Some(match g {
                GATE_WRITTEN => 0,
                GATE_SHORT => {
                    t.push('\'');
                    0
                }
                GATE_FULL => {
                    t.push('~');
                    0
                }
                g => i32::from(g),
            });
            vel[i] = Some(match first.velocity {
                PLAIN_VELOCITY => 0,
                ACCENT_VELOCITY => {
                    t.push('!');
                    0
                }
                v => i32::from(v),
            });
            let off = i64::from(first.start) - (i * unit as usize) as i64;
            nudge[i] = Some((off * 100 / i64::from(unit)) as i32);
            tokens.push(t);
            for _ in 0..ties {
                tokens.push("_".into());
            }
            i += 1 + ties;
        }
        let mut s = format!("{} : {}", head.join(" "), tokens.join(" "));
        for (name, lane) in [("vel", &vel), ("gate", &gate), ("nudge", &nudge)] {
            if let Some(values) = shortest_lane(lane) {
                s.push_str(" | ");
                s.push_str(name);
                for v in values {
                    s.push_str(&format!(" {v}"));
                }
            }
        }
        s
    }
}

/// One step of the text form.
enum StepNotes {
    Rest,
    Tie,
    Notes {
        /// Each note's step of the scale (0 for `1`) and octaves, and its
        /// semitones from home as named (from the major scale).
        notes: Vec<(i8, i8)>,
        mark: Mark,
    },
}

/// What a [`Playhead`] found next ([`Playhead::turn`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    /// Note `index` of the phrase is due, `at` written ticks from where
    /// the phrase started (counting on through every time round), on time
    /// through `pass` (0 the first).
    Note { index: usize, at: f64, pass: u64 },
    /// The line where a waiting phrase comes in ([`Join`]). The playhead
    /// is at its start already ([`Playhead::origin`] is the line): put
    /// the waiting phrase in and ask again.
    Swap,
}

/// The line of the playing phrase a waiting one comes in on
/// ([`Playhead::turn`]). Whoever asks picks it: the playhead only finds
/// the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// Its next bar line: the bars the phrase keeps with everything else
    /// that plays go on unbroken.
    Bar,
    /// The next step of the grid it is written on: as soon as can be
    /// without the pulse stumbling.
    Step,
}

/// Where a phrase is as it plays: which note is next, which time round,
/// and where its start falls on the clock. It keeps place only: it works
/// in ticks of a clock the caller reads (the transport's or its own), never
/// in frames, and plays nothing itself. The arp's engine asks it what is
/// due before the end of each period ([`Playhead::turn`]).
///
/// A phrase's own time is in *written* ticks from its first step; `factor`
/// written ticks pass per clock tick (2 at double speed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playhead {
    /// The clock tick it started on ([`Playhead::start`]), whatever its
    /// speed did since.
    started: f64,
    /// The clock tick where written tick 0 of the first time round falls
    /// at the speed it plays at now.
    origin: f64,
    factor: f64,
    /// The next note, and which time round it is.
    next: usize,
    pass: u64,
    /// Written ticks done: every note before it has been given out.
    played: f64,
    /// Where the last note given out was (-1 before any).
    last: f64,
}

impl Default for Playhead {
    fn default() -> Self {
        Self::new()
    }
}

impl Playhead {
    pub const fn new() -> Self {
        Self {
            started: 0.0,
            origin: 0.0,
            factor: 1.0,
            next: 0,
            pass: 0,
            played: 0.0,
            last: -1.0,
        }
    }

    /// From the phrase's first note, its first step on clock tick
    /// `origin`, at `factor` written ticks per clock tick.
    pub fn start(&mut self, origin: f64, factor: f64) {
        *self = Self {
            started: origin,
            origin,
            factor: sound_factor(factor),
            ..Self::new()
        };
    }

    /// A new speed from clock tick `now` on. The phrase keeps its place:
    /// what it has played stays played, and it carries on from there.
    pub fn respeed(&mut self, now: f64, factor: f64) {
        let factor = sound_factor(factor);
        let w = self.written(now);
        if w > 0.0 {
            self.origin = now - w / factor;
        }
        self.factor = factor;
    }

    pub fn factor(&self) -> f64 {
        self.factor
    }

    /// The clock tick it started on.
    pub fn started(&self) -> f64 {
        self.started
    }

    /// The clock tick of its first step (after a [`Turn::Swap`], the line
    /// the new phrase came in on).
    pub fn origin(&self) -> f64 {
        self.origin
    }

    /// The clock tick of written tick `w`.
    pub fn tick_of(&self, w: f64) -> f64 {
        self.origin + w / self.factor
    }

    /// Written ticks from its first step at clock tick `t` (negative
    /// before it starts).
    pub fn written(&self, t: f64) -> f64 {
        (t - self.origin) * self.factor
    }

    /// Where `phrase` is at clock tick `t`, from 0 up to its length, for
    /// a cursor; None before it starts.
    pub fn position(&self, phrase: &Phrase, t: f64) -> Option<f64> {
        let w = self.written(t);
        (w >= 0.0).then(|| w.rem_euclid(f64::from(phrase.ticks())))
    }

    /// The next thing due before clock tick `until`: a note of `phrase`
    /// or, with a phrase `waiting` to come in, the line where it does
    /// (its next bar line or its next step, as `waiting` says). None once
    /// everything before `until` has been given out. Notes come in time
    /// order, round and round the phrase; a waiting phrase comes in on
    /// the first such line no note of this one has played on yet, before
    /// any note on that line.
    pub fn turn(&mut self, phrase: &Phrase, waiting: Option<Join>, until: f64) -> Option<Turn> {
        let end = self.written(until);
        if self.played >= end {
            return None;
        }
        let total = f64::from(phrase.ticks());
        let next = phrase
            .notes()
            .get(self.next)
            .map(|n| self.pass as f64 * total + f64::from(n.start));
        if let Some(join) = waiting {
            let unit = f64::from(match join {
                Join::Bar => phrase.bar_ticks(),
                Join::Step => phrase.unit.ticks(),
            });
            let mut line = (self.played / unit).ceil() * unit;
            if line <= self.last {
                line += unit;
            }
            if line < end && next.is_none_or(|w| line <= w) {
                *self = Self {
                    started: self.started,
                    origin: self.tick_of(line),
                    factor: self.factor,
                    ..Self::new()
                };
                return Some(Turn::Swap);
            }
        }
        match next {
            Some(at) if at < end => {
                let turn = Turn::Note {
                    index: self.next,
                    at,
                    pass: self.pass,
                };
                self.played = at;
                self.last = at;
                self.next += 1;
                if self.next >= phrase.len() {
                    self.next = 0;
                    self.pass += 1;
                }
                Some(turn)
            }
            _ => {
                self.played = end;
                None
            }
        }
    }
}

/// A speed a playhead can play at: finite and above zero, else as written.
fn sound_factor(factor: f64) -> f64 {
    if factor.is_finite() && factor > 0.0 {
        factor
    } else {
        1.0
    }
}

/// Clock tick `t` moved by swing: time is counted from `base` in pairs of
/// `grid` ticks, and the second cell of each pair starts late by
/// (`amount` - 50) / 50 of a cell (`amount` is 50, straight, to 75), the
/// points inside the pair stretched and squeezed to match. Starts and
/// ends moved through it keep their order, so a note never ends after
/// the next one starts.
pub fn swing(t: f64, grid: f64, amount: u8, base: f64) -> f64 {
    let s = f64::from(amount.clamp(50, 75) - 50) / 50.0;
    if s == 0.0 || grid <= 0.0 {
        return t;
    }
    let pair = 2.0 * grid;
    let r = t - base;
    let start = (r / pair).floor() * pair;
    let p = r - start;
    let moved = if p < grid {
        p * (1.0 + s)
    } else {
        grid * (1.0 + s) + (p - grid) * (1.0 - s)
    };
    base + start + moved
}

#[derive(Default)]
struct Mark {
    accent: bool,
    gate: Option<u8>,
}

#[derive(Default)]
struct TextLanes {
    vel: Vec<i32>,
    gate: Vec<i32>,
    nudge: Vec<i32>,
}

impl TextLanes {
    fn at(&self, lane: &[i32], step: usize) -> Option<i32> {
        (!lane.is_empty()).then(|| lane[step % lane.len()])
    }
}

/// `1,b3,5!` and the like.
fn parse_notes(word: &str) -> Result<StepNotes, &'static str> {
    let body = word.trim_end_matches(['!', '\'', '~']);
    let mut mark = Mark::default();
    for c in word[body.len()..].chars() {
        match c {
            '!' => mark.accent = true,
            '\'' => mark.gate = Some(GATE_SHORT),
            _ => mark.gate = Some(GATE_FULL),
        }
    }
    let mut notes = Vec::new();
    for n in body.split(',') {
        notes.push(parse_degree(n)?);
    }
    Ok(StepNotes::Notes { notes, mark })
}

/// `b3`, `5-`, `#4+`: the step of the scale with its octaves (seven to an
/// octave), and the semitones from home it names.
fn parse_degree(s: &str) -> Result<(i8, i8), &'static str> {
    let mut chars = s.chars().peekable();
    let mut accidental: i8 = 0;
    while let Some(&c) = chars.peek() {
        match c {
            'b' => accidental -= 1,
            '#' => accidental += 1,
            _ => break,
        }
        chars.next();
    }
    let degree = match chars.next() {
        Some(c @ '1'..='7') => c as u8 - b'1',
        _ => return Err("a note is a scale step 1 to 7, maybe with b or #, then + or -"),
    };
    let mut octave: i8 = 0;
    for c in chars {
        match c {
            '+' => octave += 1,
            '-' => octave -= 1,
            _ => return Err("after a note's step only + or - (then ! ' ~)"),
        }
    }
    if !(-2..=2).contains(&accidental) {
        return Err("at most two b or # on a note");
    }
    if octave.abs() > MAX_OCTAVES {
        return Err("a note moves at most four octaves");
    }
    let major = Written::Major.steps()[usize::from(degree)];
    Ok((7 * octave + degree as i8, major + accidental + 12 * octave))
}

/// Half steps a note `semis` from home lies off step `steps` of `written`
/// (0 for a phrase written for either scale).
fn written_alt(written: Written, steps: i8, semis: i8) -> i8 {
    if written == Written::Either {
        return 0;
    }
    let s = i16::from(steps);
    let at = i16::from(written.steps()[s.rem_euclid(7) as usize]) + 12 * s.div_euclid(7);
    (i16::from(semis) - at) as i8
}

/// Steps of `written` from home, and the half steps off it, for a note
/// `semis` from home, spelled as degrees usually are: b2, b3, b6 and b7, and
/// the tritone as b5 in minor and #4 otherwise.
fn steps_of(written: Written, semis: i8) -> (i8, i8) {
    let s = i16::from(semis);
    let (octave, within) = (s.div_euclid(12), s.rem_euclid(12));
    // The degree (0 for 1) each semitone is named as.
    let degree: i16 = match within {
        0 => 0,
        1 | 2 => 1,
        3 | 4 => 2,
        5 => 3,
        6 if written == Written::Minor => 4,
        6 => 3,
        7 => 4,
        8 | 9 => 5,
        _ => 6,
    };
    let steps = (octave * 7 + degree) as i8;
    (steps, written_alt(written, steps, semis))
}

/// The step `0..scale.len()` of `scale` above `base` whose interval is
/// nearest `semis` (the lower of two): how a step of the seven-note scale
/// a phrase was written in lands in a scale of five, six or eight.
fn nearest_step(scale: &Scale, base: i16, semis: i8) -> i16 {
    let mut best = (i16::MAX, 0);
    for j in 0..scale.len() as i16 {
        let d = (scale.walk(base, j) - base - i16::from(semis)).abs();
        if d < best.0 {
            best = (d, j);
        }
    }
    best.1
}

/// How a note is written: its step named from the major scale with its b
/// or #, then its octaves.
fn degree_name(n: &PhraseNote) -> String {
    let s = i16::from(n.steps);
    let (octave, step) = (s.div_euclid(7), s.rem_euclid(7) as usize);
    let named = i16::from(Written::Major.steps()[step]) + 12 * octave;
    let acc = i16::from(n.semis) - named;
    let mut t = String::new();
    for _ in 0..acc.unsigned_abs() {
        t.push(if acc < 0 { 'b' } else { '#' });
    }
    t.push((b'1' + step as u8) as char);
    for _ in 0..octave.unsigned_abs() {
        t.push(if octave < 0 { '-' } else { '+' });
    }
    t
}

/// The shortest lane that, looping, gives each step with notes its value
/// (`Some`; steps without notes take any): None when every value is 0.
fn shortest_lane(values: &[Option<i32>]) -> Option<Vec<i32>> {
    if values.iter().all(|v| v.unwrap_or(0) == 0) {
        return None;
    }
    (1..=values.len()).find_map(|n| {
        let mut lane: Vec<Option<i32>> = vec![None; n];
        for (i, v) in values.iter().enumerate() {
            let Some(v) = *v else { continue };
            match lane[i % n] {
                None => lane[i % n] = Some(v),
                Some(w) if w == v => {}
                Some(_) => return None,
            }
        }
        Some(lane.into_iter().map(|v| v.unwrap_or(0)).collect())
    })
}

/// The greatest common divisor of `a` and `b`.
fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale::{MAJOR, MINOR, PENTATONIC_MINOR};

    /// A pattern played on one note becomes a phrase that plays the same:
    /// its steps repeated to a whole bar, each note on its step for its
    /// gate, ties holding on, accents and octaves kept, ratchets split.
    #[test]
    fn a_pattern_on_one_note_becomes_a_phrase_that_loops_as_it_did() {
        let pat = |t: &str| Pattern::parse(t).unwrap();
        let notes = |p: &Phrase| -> Vec<(u32, u32, i8, u8)> {
            p.notes()
                .iter()
                .map(|n| (n.start, n.len, n.semis, n.velocity))
                .collect()
        };
        let s = Rate::Sixteenth.ticks();
        // Four steps of 1/16 fill a beat: four times to the bar.
        let p = Phrase::from_pattern(&pat("x . x x"), Rate::Sixteenth, 4, 50, 100, 60);
        assert_eq!((p.bars, p.len()), (1, 12));
        assert_eq!(
            &notes(&p)[..3],
            [
                (0, s / 2, 0, 100),
                (2 * s, s / 2, 0, 100),
                (3 * s, s / 2, 0, 100)
            ]
        );
        // A tie holds on, an accent and an octave stay, a ratchet splits.
        let p = Phrase::from_pattern(&pat("x _ . x+! x:2 . . ."), Rate::Sixteenth, 4, 50, 90, 60);
        assert_eq!(
            &notes(&p)[..4],
            [
                (0, 2 * s, 0, 90),
                (3 * s, s / 2, 12, ACCENT_VELOCITY),
                (4 * s, s / 4, 0, 90),
                (4 * s + s / 2, s / 4, 0, 90),
            ]
        );
        // Three steps don't fill a bar: three bars do.
        let p = Phrase::from_pattern(&pat("x x ."), Rate::Sixteenth, 4, 50, 100, 60);
        assert_eq!((p.bars, p.len()), (3, 32));
        // A pattern at 1/4 is written on 1/8s.
        let p = Phrase::from_pattern(&pat("x"), Rate::Quarter, 4, 50, 100, 60);
        assert_eq!((p.unit, p.len()), (Rate::Eighth, 4));
    }

    fn semis(p: &Phrase) -> Vec<i8> {
        p.notes().iter().map(|n| n.semis).collect()
    }

    #[test]
    fn reads_steps_ties_chords_and_marks() {
        let p = Phrase::parse("bass minor : 1 . b3 _ 5-! . 1,b3,5' 4~").unwrap();
        assert_eq!(p.kind, Kind::Bass);
        assert_eq!(p.written, Written::Minor);
        assert_eq!(p.bars, 1);
        let n = p.notes();
        assert_eq!(n.len(), 7);
        assert_eq!((n[0].start, n[0].len, n[0].semis), (0, 192, 0));
        // b3 tied over a step: one step and 80 % of the next.
        assert_eq!(
            (n[1].start, n[1].len, n[1].semis, n[1].steps),
            (480, 432, 3, 2)
        );
        assert_eq!(
            (n[2].start, n[2].semis, n[2].steps, n[2].velocity),
            (960, -5, -3, 127)
        );
        // A chord, short.
        assert_eq!(
            n[3..6].iter().map(|n| (n.start, n.len)).collect::<Vec<_>>(),
            [(1440, 60), (1440, 60), (1440, 60)]
        );
        assert_eq!(semis(&p)[3..6], [0, 3, 7]);
        let p = Phrase::parse("riff : 1 . . . . . . . . . . . . . . . 4~").unwrap();
        assert_eq!(p.bars, 2);
        assert_eq!(p.notes()[1].len, 240);
    }

    /// In key, a note written outside its scale keeps its distance from
    /// the held note wherever the phrase is held, where a step of the key
    /// and then a half step would land it on its neighbour.
    #[test]
    fn a_note_outside_the_written_scale_keeps_its_distance_from_the_held_note() {
        let play = |p: &Phrase, held: u8, scale: Scale| -> Vec<u8> {
            p.notes()
                .iter()
                .filter_map(|n| p.pitch(n, held, Follow::InKey, Some(scale), Snap::Down))
                .collect()
        };
        // A blues walk in C major: the 6th, then the lowered 7th.
        let walk = Phrase::parse("bass major : 1 6 b7").unwrap();
        let c_major = Scale::new(0, MAJOR);
        assert_eq!(play(&walk, 48, c_major), [48, 57, 58], "C, A, B flat");
        // Held on F, the key's seventh step is E: lowered, E flat.
        assert_eq!(play(&walk, 53, c_major), [53, 62, 63], "F, D, E flat");
        // Held on G it is F already, and F it stays: not the E below it,
        // which the 6th plays.
        assert_eq!(play(&walk, 55, c_major), [55, 64, 65], "G, E, F");
        // A leading tone in A minor. Held on F, the step below is E, and
        // the leading tone is E: raised, it would be F itself.
        let turn = Phrase::parse("riff minor : 1 7- 1 b3").unwrap();
        let a_minor = Scale::new(9, MINOR);
        assert_eq!(
            play(&turn, 57, a_minor),
            [57, 56, 57, 60],
            "A, G sharp, A, C"
        );
        assert_eq!(play(&turn, 53, a_minor), [53, 52, 53, 57], "F, E, F, A");
        assert_eq!(
            play(&turn, 55, a_minor),
            [55, 54, 55, 59],
            "G, F sharp, G, B"
        );
        // In a scale of five notes too.
        let penta = Scale::new(9, PENTATONIC_MINOR);
        assert_eq!(play(&turn, 57, penta), [57, 56, 57, 60]);
    }

    /// A phrase suits a key of its own kind, by the key's third; a key of
    /// no kind, or no key, suits all.
    #[test]
    fn a_phrase_suits_a_key_of_the_kind_it_was_written_in() {
        let key = |mask| Some(Scale::new(9, mask));
        let dorian = crate::scale::by_name("Dorian").unwrap().mask;
        let mixolydian = crate::scale::by_name("Mixolydian").unwrap().mask;
        for (k, major, minor) in [
            (key(MAJOR), true, false),
            (key(mixolydian), true, false),
            (key(MINOR), false, true),
            (key(dorian), false, true),
            (key(PENTATONIC_MINOR), false, true),
            // Both thirds, neither, and no key: of no kind.
            (key(crate::scale::CHROMATIC), true, true),
            (key(crate::scale::MAJOR_BLUES), true, true),
            (key(crate::scale::mask(&[0, 2, 5, 7, 10])), true, true),
            (None, true, true),
        ] {
            assert_eq!(Written::Major.suits(k), major, "{k:?}");
            assert_eq!(Written::Minor.suits(k), minor, "{k:?}");
            assert!(Written::Either.suits(k), "{k:?}");
        }
    }

    #[test]
    fn notes_outside_the_written_scale_are_told_from_its_steps() {
        let p = Phrase::parse("riff minor : 1 #4 3 7").unwrap();
        let n = p.notes();
        // #4 in minor: the 4th raised; a 3 is minor's b3 raised; 7 is b7 raised.
        assert_eq!((n[1].steps, n[1].alt), (3, 1));
        assert_eq!((n[2].steps, n[2].alt), (2, 1));
        assert_eq!((n[3].steps, n[3].alt), (6, 1));
        // Written for either scale, a b3 is just the third step.
        let p = Phrase::parse("riff either : b3 3").unwrap();
        assert_eq!((p.notes()[0].steps, p.notes()[0].alt), (2, 0));
        assert_eq!((p.notes()[1].steps, p.notes()[1].alt), (2, 0));
    }

    #[test]
    fn in_key_moves_by_steps_of_the_scale_and_exact_by_semitones() {
        let p = Phrase::parse("riff minor : 1 b3 5 b7 1+").unwrap();
        let c_minor = Scale::new(0, MINOR);
        let c_major = Scale::new(0, MAJOR);
        let play = |held: u8, f: Follow, s: Option<Scale>| -> Vec<u8> {
            p.notes()
                .iter()
                .filter_map(|n| p.pitch(n, held, f, s, Snap::Down))
                .collect()
        };
        // From C in C minor, as written.
        assert_eq!(play(60, Follow::InKey, Some(c_minor)), [60, 63, 67, 70, 72]);
        // In C major the same steps: the riff takes the key's colour.
        assert_eq!(play(60, Follow::InKey, Some(c_major)), [60, 64, 67, 71, 72]);
        // From D in C major it moves by steps: D F A C D.
        assert_eq!(play(62, Follow::InKey, Some(c_major)), [62, 65, 69, 72, 74]);
        // Exact keeps every interval from D.
        assert_eq!(play(62, Follow::Exact, Some(c_major)), [62, 65, 69, 72, 74]);
        assert_eq!(play(61, Follow::Exact, None), [61, 64, 68, 71, 73]);
        // In key with no scale is Exact.
        assert_eq!(play(61, Follow::InKey, None), [61, 64, 68, 71, 73]);
        // A held note outside the scale snaps first; Skip plays nothing.
        assert_eq!(play(61, Follow::InKey, Some(c_major)), [60, 64, 67, 71, 72]);
        let skipped: Vec<u8> = p
            .notes()
            .iter()
            .filter_map(|n| p.pitch(n, 61, Follow::InKey, Some(c_major), Snap::Skip))
            .collect();
        assert!(skipped.is_empty());
    }

    #[test]
    fn a_seven_step_phrase_lands_on_the_nearest_notes_of_a_pentatonic_scale() {
        let p = Phrase::parse("riff minor : 1 2 b3 4 5 b6 b7 1+").unwrap();
        let pent = Scale::new(9, PENTATONIC_MINOR); // A C D E G
        let notes: Vec<u8> = p
            .notes()
            .iter()
            .filter_map(|n| p.pitch(n, 57, Follow::InKey, Some(pent), Snap::Down))
            .collect();
        // From A: the 2nd and b3 land on C, the b6 on E (the nearer).
        assert_eq!(notes, [57, 60, 60, 62, 64, 64, 67, 69]);
    }

    #[test]
    fn the_text_form_reads_back_as_itself() {
        for text in [
            "bass minor : 1 . 1 1 . 1 . 5- 1 . b7- . 5- . b7- 1",
            "chords either 1/8 : 1,3,5! . . 1,3,5' . 1,3,5 . .",
            "riff minor swung : 5 . b7 1+ _ . b7 5 . . . . . . . . | vel 110 0 90 0",
            "line major 1/16T 3/4 : 1 3 5 1+ 5 3 1 3 5 1+ 5 3 1 3 5 1+ 5 3",
            "riff major : #4 b2- 7+ 1~ 5 . . . . . . . . . . . | gate 0 0 0 0 60 | nudge 0 10",
        ] {
            let p = Phrase::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(p.to_text(), text);
        }
    }

    #[test]
    fn notes_as_played_become_a_phrase_from_their_home_note() {
        // A two-bar riff in A minor, played a little late on its third note.
        let notes = [
            (0, 200, 57, 100),
            (480, 200, 60, 90),
            (972, 400, 64, 110),
            (1920, 900, 55, 100),
            (3840, 400, 57, 100),
        ];
        let p = Phrase::from_notes(&notes, 57, Written::Minor, Rate::Sixteenth, 4);
        assert_eq!(p.bars, 2);
        assert_eq!(semis(&p), [0, 3, 7, -2, 0]);
        assert_eq!(p.notes()[2].start, 972);
        let text = p.to_text();
        assert!(text.starts_with("riff minor : 1 . b3 . 5 _ "), "{text}");
        let back = Phrase::parse(&text).unwrap();
        assert_eq!(semis(&back), semis(&p));
        // Within a tick or two of where they were played, on the same grid.
        for (a, b) in back.notes().iter().zip(p.notes()) {
            assert!(a.start.abs_diff(b.start) <= 3, "{a:?} {b:?}");
        }
    }

    #[test]
    fn says_what_is_wrong() {
        let bad = |t: &str| Phrase::parse(t).unwrap_err();
        assert_eq!(bad("1 2 3").reason, "a phrase needs its steps after a ':'");
        assert_eq!(bad("riff :").reason, "no steps");
        assert_eq!(bad("riff : 1 8").token, 3);
        assert_eq!(
            bad("riff 1/4 : 1").reason,
            "a phrase's grid is 1/8 to 1/32T"
        );
        assert_eq!(
            bad("riff 1 bars : 1 . . . . . . . . . . . . . . . . 1").reason,
            "more steps than its bars hold"
        );
        assert_eq!(
            bad("riff : 1 | vel 200").reason,
            "velocity must be 0 to 127"
        );
        assert_eq!(
            bad("riff : 1 | wobble 2").reason,
            "unknown lane (use vel, gate or nudge)"
        );
        assert_eq!(
            bad("blues : 1").reason,
            "unknown word (a kind, a scale, a grid, a meter, swung or N bars)"
        );
    }
    fn note(index: usize, at: f64, pass: u64) -> Turn {
        Turn::Note { index, at, pass }
    }

    /// Every turn before clock tick `until`.
    fn turns(h: &mut Playhead, p: &Phrase, waiting: Option<Join>, until: f64) -> Vec<Turn> {
        let mut got = Vec::new();
        while let Some(t) = h.turn(p, waiting, until) {
            got.push(t);
            if t == Turn::Swap {
                break;
            }
        }
        got
    }

    #[test]
    fn a_playhead_gives_the_notes_in_order_round_and_round() {
        // Notes on 1/16s 0, 2 and 6 of one bar (3840 ticks).
        let p = Phrase::parse("bass either : 1 . 1 . . . 5 .").unwrap();
        let mut h = Playhead::new();
        h.start(1000.0, 1.0);
        assert_eq!(h.turn(&p, None, 1000.0), None, "nothing before it starts");
        assert_eq!(h.position(&p, 999.0), None);
        assert_eq!(
            turns(&mut h, &p, None, 1000.0 + 2.0 * 3840.0),
            [
                note(0, 0.0, 0),
                note(1, 480.0, 0),
                note(2, 1440.0, 0),
                note(0, 3840.0, 1),
                note(1, 4320.0, 1),
                note(2, 5280.0, 1),
            ]
        );
        // A note right on the end of what was asked for waits for the next
        // ask, so no note is ever given twice or left out.
        assert_eq!(
            turns(&mut h, &p, None, 1000.0 + 2.0 * 3840.0 + 1.0),
            [note(0, 7680.0, 2)]
        );
        assert_eq!(h.tick_of(7680.0), 8680.0);
        assert_eq!(h.position(&p, 1000.0 + 3840.0 + 100.0), Some(100.0));
    }

    #[test]
    fn a_waiting_phrase_comes_in_on_the_next_bar_line_before_its_notes() {
        let p = Phrase::parse("bass either : 1 . . . 1 . . . 1 . . . 1 . . .").unwrap();
        let q = Phrase::parse("riff either : 5 5").unwrap();
        let mut h = Playhead::new();
        h.start(0.0, 1.0);
        assert_eq!(
            turns(&mut h, &p, None, 1000.0),
            [note(0, 0.0, 0), note(1, 960.0, 0)]
        );
        // A phrase waits from here: the bar's other notes play, then the
        // bar line comes before the note on it.
        assert_eq!(
            turns(&mut h, &p, Some(Join::Bar), 3841.0),
            [note(2, 1920.0, 0), note(3, 2880.0, 0), Turn::Swap]
        );
        assert_eq!(h.origin(), 3840.0, "the new phrase starts on the line");
        assert_eq!(h.started(), 0.0, "where it all started stays");
        assert_eq!(turns(&mut h, &q, None, 3841.0), [note(0, 0.0, 0)]);

        // Picked before anything has played, it comes in at once.
        let mut h = Playhead::new();
        h.start(500.0, 1.0);
        assert_eq!(turns(&mut h, &p, Some(Join::Bar), 501.0), [Turn::Swap]);
        assert_eq!(h.origin(), 500.0);

        // Picked once the note on a bar line has played, it waits for the
        // next line.
        let mut h = Playhead::new();
        h.start(0.0, 1.0);
        assert_eq!(turns(&mut h, &p, None, 1.0), [note(0, 0.0, 0)]);
        assert_eq!(turns(&mut h, &p, Some(Join::Bar), 900.0), []);
        assert_eq!(
            turns(&mut h, &p, Some(Join::Bar), 3841.0),
            [
                note(1, 960.0, 0),
                note(2, 1920.0, 0),
                note(3, 2880.0, 0),
                Turn::Swap
            ]
        );
    }

    /// On the next step instead, a waiting phrase comes in on the first
    /// line of the playing phrase's grid that no note has played on.
    #[test]
    fn a_waiting_phrase_can_come_in_on_the_next_step() {
        // On 1/8s (480 ticks a step): notes on steps 0 and 3.
        let p = Phrase::parse("bass either 1/8 : 1 . . 1 . . . .").unwrap();
        let mut h = Playhead::new();
        h.start(0.0, 1.0);
        assert_eq!(turns(&mut h, &p, None, 100.0), [note(0, 0.0, 0)]);
        // Picked 100 ticks in: the next 1/8 line is at 480.
        assert_eq!(turns(&mut h, &p, Some(Join::Step), 481.0), [Turn::Swap]);
        assert_eq!(h.origin(), 480.0);
        assert_eq!(h.started(), 0.0);

        // Picked once the note on a step line has played, it waits for
        // the next line.
        let mut h = Playhead::new();
        h.start(0.0, 1.0);
        assert_eq!(
            turns(&mut h, &p, None, 1441.0),
            [note(0, 0.0, 0), note(1, 1440.0, 0)]
        );
        assert_eq!(turns(&mut h, &p, Some(Join::Step), 1900.0), []);
        assert_eq!(turns(&mut h, &p, Some(Join::Step), 1921.0), [Turn::Swap]);
        assert_eq!(h.origin(), 1920.0);

        // At double speed its step is half as long on the clock.
        let mut h = Playhead::new();
        h.start(0.0, 2.0);
        assert_eq!(turns(&mut h, &p, None, 50.0), [note(0, 0.0, 0)]);
        assert_eq!(turns(&mut h, &p, Some(Join::Step), 241.0), [Turn::Swap]);
        assert_eq!(h.origin(), 240.0);
    }

    #[test]
    fn a_new_speed_keeps_the_place() {
        let p = Phrase::parse("bass either : 1 1 1 1 1 1 1 1").unwrap();
        let mut h = Playhead::new();
        h.start(0.0, 1.0);
        assert_eq!(
            turns(&mut h, &p, None, 480.0),
            [note(0, 0.0, 0), note(1, 240.0, 0)]
        );
        // Twice as fast from clock tick 480, where it is two 1/16s in.
        h.respeed(480.0, 2.0);
        assert_eq!(h.written(480.0), 480.0);
        assert_eq!(
            turns(&mut h, &p, None, 721.0),
            [note(2, 480.0, 0), note(3, 720.0, 0), note(4, 960.0, 0)]
        );
        assert_eq!(
            [h.tick_of(480.0), h.tick_of(720.0), h.tick_of(960.0)],
            [480.0, 600.0, 720.0]
        );
        assert_eq!(h.started(), 0.0);
        // A speed that makes no sense plays as written.
        h.respeed(720.0, f64::NAN);
        assert_eq!(h.factor(), 1.0);
    }

    #[test]
    fn the_gate_shortens_notes_or_holds_them_to_the_next() {
        // Two notes two 1/16s apart, each 80 % of a 1/16 (192 ticks).
        let p = Phrase::parse("bass either : 1 . 1 .").unwrap();
        assert_eq!(p.gated(0, 50), 192.0, "as written");
        assert_eq!(p.gated(0, 25), 96.0);
        assert_eq!(p.gated(0, 1), 192.0 / 50.0);
        assert_eq!(p.gated(0, 75), 336.0, "half way to the next note");
        assert_eq!(p.gated(0, 100), 480.0, "to the next note");
        assert_eq!(p.gated(1, 100), 3360.0, "round the end to the first note");
        // Notes that start together are not each other's next note.
        let c = Phrase::parse("chords either : 1,3,5 . 1 .").unwrap();
        assert_eq!(c.gated(0, 100), 480.0);
        assert_eq!(c.gated(2, 100), 480.0);
    }

    #[test]
    fn swing_moves_the_second_sixteenth_and_keeps_the_order() {
        let swung = |t: f64| swing(t, 240.0, 66, 0.0);
        assert_eq!(swung(0.0), 0.0);
        assert!((swung(240.0) - 316.8).abs() < 1e-9, "32 % of a 1/16 late");
        assert_eq!(swung(480.0), 480.0, "every pair starts on time");
        let mut last = -1.0;
        for t in 0..960 {
            let s = swung(f64::from(t));
            assert!(s > last, "{t}");
            last = s;
        }
        // Counted from a base, for a phrase that started off the grid.
        assert!((swing(1240.0, 240.0, 66, 1000.0) - 1316.8).abs() < 1e-9);
        assert_eq!(swing(1240.0, 240.0, 50, 1000.0), 1240.0, "straight");
    }
}
