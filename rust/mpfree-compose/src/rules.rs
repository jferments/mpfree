//! Three kinds of rule (contract 6 of the design).
//!
//! - **Structural**: a real pitch and time, inside the instrument's range
//!   and the clip limits. Never gives way.
//! - **Yours**: locked bars and notes, chords you set, the Keeps, and the
//!   ranges you picked. Never gives way.
//! - **Preference**: leap size, how chromatic notes move, the parts of an
//!   ending, repeats. Weighed against each other, with weights per style
//!   ([`Weights`]), so Jazz-ish can use enclosures and a leap over an
//!   octave is rare rather than banned.
//!
//! When the structural rules and yours can't all be met, nothing is
//! quietly broken: the request says which setting is in the way and
//! suggests one change ([`Unmet::words`]).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleKind {
    Structural,
    Yours,
    Preference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    // Structural.
    ValidPitch,
    ValidTime,
    InstrumentRange,
    ClipLimits,
    // Yours.
    Locked,
    ChordsYouSet,
    KeepRhythm,
    KeepTuneShape,
    KeepChords,
    KeepAccents,
    RangeSetting,
    // Preferences.
    LeapSize,
    ChromaticMotion,
    EndingTune,
    EndingChord,
    EndingRhythm,
    LoopReturn,
    Repeats,
}

impl Rule {
    pub const fn kind(self) -> RuleKind {
        match self {
            Rule::ValidPitch | Rule::ValidTime | Rule::InstrumentRange | Rule::ClipLimits => {
                RuleKind::Structural
            }
            Rule::Locked
            | Rule::ChordsYouSet
            | Rule::KeepRhythm
            | Rule::KeepTuneShape
            | Rule::KeepChords
            | Rule::KeepAccents
            | Rule::RangeSetting => RuleKind::Yours,
            Rule::LeapSize
            | Rule::ChromaticMotion
            | Rule::EndingTune
            | Rule::EndingChord
            | Rule::EndingRhythm
            | Rule::LoopReturn
            | Rule::Repeats => RuleKind::Preference,
        }
    }

    pub const fn can_give_way(self) -> bool {
        matches!(self.kind(), RuleKind::Preference)
    }
}

/// How much each preference costs when broken, per style. Endings are
/// scored in parts: the tune's last note, the chord, the rhythm, and for
/// a loop the way back to bar 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    /// Per semitone of leap beyond a fifth; an octave and more costs a lot
    /// but is never banned.
    pub leap: f32,
    /// A chromatic note that doesn't move by a half step (enclosures and
    /// unresolved neighbours cost little in Jazz-ish).
    pub chromatic: f32,
    pub ending_tune: f32,
    pub ending_chord: f32,
    pub ending_rhythm: f32,
    pub loop_return: f32,
    /// Per note in a run of three or more of the same pitch.
    pub repeats: f32,
}

impl Weights {
    pub fn of(&self, rule: Rule) -> Option<f32> {
        Some(match rule {
            Rule::LeapSize => self.leap,
            Rule::ChromaticMotion => self.chromatic,
            Rule::EndingTune => self.ending_tune,
            Rule::EndingChord => self.ending_chord,
            Rule::EndingRhythm => self.ending_rhythm,
            Rule::LoopReturn => self.loop_return,
            Rule::Repeats => self.repeats,
            _ => return None,
        })
    }
}

/// The Range control's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RangeSetting {
    Narrow,
    Medium,
    Wide,
}

impl RangeSetting {
    pub const fn name(self) -> &'static str {
        match self {
            RangeSetting::Narrow => "Narrow",
            RangeSetting::Medium => "Medium",
            RangeSetting::Wide => "Wide",
        }
    }

    /// The next wider setting, if any.
    pub const fn wider(self) -> Option<RangeSetting> {
        match self {
            RangeSetting::Narrow => Some(RangeSetting::Medium),
            RangeSetting::Medium => Some(RangeSetting::Wide),
            RangeSetting::Wide => None,
        }
    }
}

/// The one change a refusal suggests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Suggestion {
    Range(RangeSetting),
    UnlockBar(u16),
    LetChordsChange,
    FewerBars(u8),
}

impl Suggestion {
    fn words(self) -> String {
        match self {
            Suggestion::Range(r) => format!("Try Range {}?", r.name()),
            Suggestion::UnlockBar(b) => format!("Unlock bar {b}?"),
            Suggestion::LetChordsChange => "Let it change your chords?".to_string(),
            Suggestion::FewerBars(n) => format!("Try {n} bars?"),
        }
    }
}

/// A request that couldn't be met without breaking a rule that never
/// gives way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unmet {
    pub rule: Rule,
    /// The bar being made, counted from 1.
    pub bar: Option<u16>,
    pub range: Option<RangeSetting>,
    pub suggestion: Option<Suggestion>,
}

impl Unmet {
    /// "Can't fit a new bar 6 between your bars with Range Narrow. Try
    /// Range Medium?"
    pub fn words(&self) -> String {
        let what = match self.bar {
            Some(b) => format!("a new bar {b} between your bars"),
            None => "new bars here".to_string(),
        };
        let why = match (self.rule, self.range) {
            (Rule::RangeSetting, Some(r)) => format!(" with Range {}", r.name()),
            (Rule::Locked, _) => " around the locked notes".to_string(),
            (Rule::KeepRhythm, _) => " keeping your rhythm".to_string(),
            (Rule::KeepTuneShape, _) => " keeping your tune shape".to_string(),
            (Rule::KeepChords | Rule::ChordsYouSet, _) => " on your chords".to_string(),
            (Rule::KeepAccents, _) => " keeping your accents".to_string(),
            (Rule::InstrumentRange, _) => " in this instrument's range".to_string(),
            (Rule::ClipLimits, _) => " inside the clip's limits".to_string(),
            _ => String::new(),
        };
        let mut s = format!("Can't fit {what}{why}.");
        if let Some(sg) = self.suggestion {
            s.push(' ');
            s.push_str(&sg.words());
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_preferences_give_way() {
        assert_eq!(Rule::InstrumentRange.kind(), RuleKind::Structural);
        assert_eq!(Rule::Locked.kind(), RuleKind::Yours);
        assert_eq!(Rule::LeapSize.kind(), RuleKind::Preference);
        assert!(!Rule::KeepRhythm.can_give_way());
        assert!(Rule::ChromaticMotion.can_give_way());
    }

    #[test]
    fn refusals_name_the_setting_and_suggest_one_change() {
        let u = Unmet {
            rule: Rule::RangeSetting,
            bar: Some(6),
            range: Some(RangeSetting::Narrow),
            suggestion: RangeSetting::Narrow.wider().map(Suggestion::Range),
        };
        assert_eq!(
            u.words(),
            "Can't fit a new bar 6 between your bars with Range Narrow. Try Range Medium?"
        );
        let l = Unmet {
            rule: Rule::Locked,
            bar: Some(3),
            range: None,
            suggestion: Some(Suggestion::UnlockBar(4)),
        };
        assert_eq!(
            l.words(),
            "Can't fit a new bar 3 between your bars around the locked notes. Unlock bar 4?"
        );
        assert_eq!(RangeSetting::Wide.wider(), None);
    }
}
