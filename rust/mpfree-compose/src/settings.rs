//! The controls, in the same words everywhere (the design's table): what a
//! request carries besides what was played.

use crate::rules::RangeSetting;
use crate::style::{Style, MINIMAL};

/// How the last bar closes, in tune, chords and rhythm together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ending {
    /// "Lands home": the home chord and a settled rhythm; the tune ends on
    /// the home note, its 3rd or 5th.
    Home,
    /// "Leaves it open": a chord that wants to go on.
    Open,
    /// "Loops back": leading back into the first bar.
    LoopsBack,
}

impl Ending {
    pub const fn words(self) -> &'static str {
        match self {
            Ending::Home => "Lands home",
            Ending::Open => "Leaves it open",
            Ending::LoopsBack => "Loops back",
        }
    }
}

/// What stays exactly as yours (Keep: Rhythm · Tune shape · Chords ·
/// Accents).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Keeps {
    pub rhythm: bool,
    pub tune_shape: bool,
    pub chords: bool,
    pub accents: bool,
}

/// What the new bars are for, in a word (S5): the brief, shown in words
/// and never as a number, and never guessed from your notes. How close
/// stays Similarity; the energy's shape and the ending stay Bars and
/// Ending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Character {
    /// The style's own plan, as before Character.
    #[default]
    AsStyle,
    /// Fewer notes, a small high point, lands on time.
    Restrained,
    /// Reaches up, leans on the high notes, lands a little late.
    Yearning,
    /// Jumps that turn back, short notes, answers.
    Playful,
    /// Notes that pull, the landing held off.
    Tense,
    /// Busier, pushes ahead, climbs fast.
    Urgent,
}

impl Character {
    /// In the Character row's order.
    pub const ALL: [Character; 6] = [
        Character::AsStyle,
        Character::Restrained,
        Character::Yearning,
        Character::Playful,
        Character::Tense,
        Character::Urgent,
    ];

    pub const fn words(self) -> &'static str {
        match self {
            Character::AsStyle => "As the style",
            Character::Restrained => "Restrained",
            Character::Yearning => "Yearning",
            Character::Playful => "Playful",
            Character::Tense => "Tense",
            Character::Urgent => "Urgent",
        }
    }

    /// What it does, for the line under the roll.
    pub const fn says(self) -> &'static str {
        match self {
            Character::AsStyle => "the style's own plan",
            Character::Restrained => "fewer notes, a small high point, lands on time",
            Character::Yearning => "reaches up, leans on the high notes, lands a little late",
            Character::Playful => "jumps that turn back, short notes, answers",
            Character::Tense => "notes that pull, the landing held off",
            Character::Urgent => "busier, pushes ahead, climbs fast",
        }
    }

    /// What it asks of the plan and the playing.
    pub(crate) const fn brief(self) -> Brief {
        let plain = Brief {
            activity: 1.0,
            lift: 0,
            early: None,
            push_peak: false,
            length: 1.0,
            turns: false,
            answers: false,
            pull: false,
            close_on_beat: false,
            borrowed: true,
            last: 1.0,
            push_ms: 0.0,
            lean: 0.0,
        };
        match self {
            Character::AsStyle => plain,
            Character::Restrained => Brief {
                activity: 0.6,
                lift: -1,
                close_on_beat: true,
                borrowed: false,
                last: 0.0,
                ..plain
            },
            Character::Yearning => Brief {
                activity: 0.9,
                lift: 2,
                early: Some(false),
                last: 1.8,
                lean: 1.0,
                ..plain
            },
            Character::Playful => Brief {
                length: 0.5,
                turns: true,
                answers: true,
                last: 0.5,
                ..plain
            },
            Character::Tense => Brief {
                lift: 1,
                pull: true,
                lean: 0.3,
                ..plain
            },
            Character::Urgent => Brief {
                activity: 1.4,
                lift: 1,
                early: Some(true),
                push_peak: true,
                length: 0.85,
                last: 0.3,
                push_ms: -6.0,
                ..plain
            },
        }
    }
}

/// One thing an aimed try changes, keeping the rest (S5): from Try
/// again's ▾. The landing notes and the high point stay where they were
/// unless the aim is about them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aim {
    /// More notes, the same plan.
    Busier,
    /// Fewer notes, longer ones.
    Calmer,
    /// The last bar lands home, on the beat.
    LandIt,
    /// Lower, softer and, with Sound on, darker.
    LessBright,
    /// Higher, harder and, with Sound on, opener.
    Brighter,
}

impl Aim {
    /// In the menu's order.
    pub const ALL: [Aim; 5] = [
        Aim::Busier,
        Aim::Calmer,
        Aim::LandIt,
        Aim::LessBright,
        Aim::Brighter,
    ];

    pub const fn words(self) -> &'static str {
        match self {
            Aim::Busier => "Busier",
            Aim::Calmer => "Calmer",
            Aim::LandIt => "Land it",
            Aim::LessBright => "Less bright",
            Aim::Brighter => "Brighter",
        }
    }

    /// What it does, beside it in the menu.
    pub const fn says(self) -> &'static str {
        match self {
            Aim::Busier => "more notes, the same plan",
            Aim::Calmer => "fewer notes, longer ones",
            Aim::LandIt => "the last bar lands home",
            Aim::LessBright => "lower, softer, darker",
            Aim::Brighter => "higher, harder, opener",
        }
    }

    /// The Tries list's name for a try it made ("Try 3 · calmer").
    pub const fn try_words(self) -> &'static str {
        match self {
            Aim::Busier => "busier",
            Aim::Calmer => "calmer",
            Aim::LandIt => "landed",
            Aim::LessBright => "less bright",
            Aim::Brighter => "brighter",
        }
    }
}

/// What a [`Character`] asks of the phrase plan ([`crate::sketch`]), the
/// notes ([`crate::draft`]) and the playing ([`crate::perform`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Brief {
    /// Times every new bar's activity.
    pub activity: f32,
    /// Scale steps added to the high point's aim; below 0 a smaller one.
    pub lift: i32,
    /// The high point a bar sooner (`Some(true)`), never sooner
    /// (`Some(false)`), or either, by draft (`None`).
    pub early: Option<bool>,
    /// The high point pushes in an 1/8 early.
    pub push_peak: bool,
    /// Times the length of every note but the plan's anchors.
    pub length: f32,
    /// An answer's tune jumps up and turns back once.
    pub turns: bool,
    /// Developing bars answer instead.
    pub answers: bool,
    /// Landings on the five chord's leading note, the last bar starting
    /// on the five chord and its landing held to the bar's middle.
    pub pull: bool,
    /// The last note on the last bar's downbeat.
    pub close_on_beat: bool,
    /// A borrowed chord may be the surprise.
    pub borrowed: bool,
    /// Times how late the instrument plays a phrase's last note (0: on
    /// time).
    pub last: f32,
    /// Every new note this many ms ahead (below 0) or behind.
    pub push_ms: f32,
    /// The high point louder and held longer (0: as the style plays it).
    pub lean: f32,
}

/// One request's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub style: Style,
    /// Like mine (1) to New (0).
    pub similarity: f32,
    /// Steady (0) to Restless (1).
    pub variation: f32,
    /// Sparse (0) to Busy (1); 0.5 is as busy as what was played.
    pub activity: f32,
    pub range: RangeSetting,
    pub keeps: Keeps,
    /// Bars to add: 1, 2, 4, 8 or 16.
    pub bars: u8,
    pub ending: Ending,
    /// What the new bars are for (S5).
    pub character: Character,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            style: MINIMAL,
            similarity: 0.6,
            variation: 0.4,
            activity: 0.5,
            range: RangeSetting::Medium,
            keeps: Keeps::default(),
            bars: 4,
            ending: Ending::Home,
            character: Character::AsStyle,
            seed: 1,
        }
    }
}
