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
            seed: 1,
        }
    }
}
