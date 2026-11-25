//! Continue (composing tools, phase 1) as the time engine's screens see
//! it: what a touch asks for ([`ContinueAction`], [`ContinueAsk`]), what
//! each track plays as the screen knows it ([`Parts`]), and what the
//! screens show while a clip is being carried on ([`Continuing`]). The
//! note editor's Continue tool (`editor::compose`) asks through its own
//! `Done`, which the time engine completes with the [`Parts`].
//!
//! The time engine runs it (mpfree-app's `time/compose.rs`): a result is
//! heard in place of its clip from the next bar line, Keep writes it as
//! one take, Back leaves the clip as it was. The design is
//! docs/dev/design/compose.md.

use std::sync::Arc;

use mpfree_compose::{Aim, Continued, InstrumentKind, Lanes, Settings, State};
use mpfree_seq::Clip;
use mpfree_transport::time::Tick;

/// A touch on Continue.
#[derive(Clone, Debug, PartialEq)]
pub enum ContinueAction {
    /// Continue, or Try again on everything.
    Ask(Box<ContinueAsk>),
    /// ‹ (`back`) or ›: the result before or after this one, of the last
    /// 8, heard from the next bar line.
    Step { back: bool },
    /// Keep: the result as one take, which UNDO takes back.
    Keep,
    /// Back: nothing written; the clip as it was from the next bar line.
    Back,
    /// From the clip card's Continue row (phase 1 step 4): the note
    /// editor opens on clip `slot` of `track` with Continue open, which
    /// asks at once.
    Open { track: usize, slot: usize },
    /// Not a touch: what each track plays, as the screen knows it, sent
    /// when that changes, for the note editor's asks.
    Parts(Arc<Parts>),
}

/// What each track plays, as the screen knows it (the time engine
/// doesn't): the kind of instrument, where it can tell, and the drum
/// each pad of a kit plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parts {
    /// Track by track: None where the screen can't tell (a plug-in, MIDI
    /// out), and the time engine reads it from the clip.
    pub kinds: Vec<Option<InstrumentKind>>,
    pub lanes: Lanes,
}

/// Continue, or Try again: a new result for clip `slot` of `track` with
/// these settings, or part of the shown one again. The time engine picks
/// the seed, so each ask sounds new.
#[derive(Clone, Debug, PartialEq)]
pub struct ContinueAsk {
    pub track: usize,
    pub slot: usize,
    pub settings: Settings,
    /// The kind of instrument the track plays.
    pub kind: InstrumentKind,
    /// The drum each pad of a drum track plays, as the kit names them.
    pub lanes: Lanes,
    /// Everything, or only the picked bars or notes of the shown result.
    pub scope: Scope,
    /// New bars (from 0, the clip's first bar) that stay exactly as the
    /// shown result has them through every new try: locked.
    pub locked: Vec<u16>,
}

/// What Try again makes again (phase 1 step 3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// A new result: Continue, a setting changed, or Try again with
    /// nothing picked. Locked bars stay.
    #[default]
    All,
    /// The picked new bars of the shown result (from 0, the clip's first
    /// bar), every other bar as it is.
    Bars(Vec<u16>),
    /// The picked new notes of the clip as heard with the shown result
    /// (where each starts, in the clip's ticks), each in its place, every
    /// other note as it is.
    Notes(Vec<Tick>),
    /// An aimed try (S5, Try again's ▾): the picked new bars of the shown
    /// result (every new bar when none are picked) again with one thing
    /// changed, the rest kept. Locked bars stay.
    Aimed { aim: Aim, bars: Vec<u16> },
}

impl Scope {
    /// What a try of it is, for the tries list: None for a whole new one.
    pub fn again(&self) -> Option<Again> {
        let n = |l: usize| u16::try_from(l).unwrap_or(u16::MAX);
        match self {
            Scope::All => None,
            Scope::Bars(b) if b.len() == 1 => Some(Again::Bar(b[0])),
            Scope::Bars(b) => Some(Again::Bars(n(b.len()))),
            Scope::Notes(v) => Some(Again::Notes(n(v.len()))),
            Scope::Aimed { aim, bars } => Some(Again::Aimed {
                aim: *aim,
                bars: n(bars.len()),
            }),
        }
    }
}

/// A try made of part of the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Again {
    /// One bar (from 0).
    Bar(u16),
    /// How many bars.
    Bars(u16),
    /// How many notes.
    Notes(u16),
    /// An aimed try: what it was aimed at, and how many bars it made
    /// again (0: every new bar).
    Aimed { aim: Aim, bars: u16 },
}

impl Again {
    /// "bar 6 again", "2 bars again", "3 notes again", "every new bar
    /// again, calmer".
    pub fn words(self) -> String {
        match self {
            Again::Bar(b) => format!("bar {} again", b + 1),
            Again::Bars(n) => format!("{n} bars again"),
            Again::Notes(1) => "1 note again".to_string(),
            Again::Notes(n) => format!("{n} notes again"),
            Again::Aimed { aim, bars: 0 } => format!("every new bar again, {}", aim.try_words()),
            Again::Aimed { aim, bars: 1 } => format!("1 bar again, {}", aim.try_words()),
            Again::Aimed { aim, bars } => format!("{bars} bars again, {}", aim.try_words()),
        }
    }

    /// The tries list's short form, where "+4 bars" stands for a whole
    /// try (an aimed try of every new bar too): "bar 6", "2 bars", "3
    /// notes".
    pub fn short(self) -> String {
        match self {
            Again::Bar(b) => format!("bar {}", b + 1),
            Again::Bars(n) => format!("{n} bars"),
            Again::Notes(1) => "1 note".to_string(),
            Again::Notes(n) => format!("{n} notes"),
            Again::Aimed { bars: 1, .. } => "1 bar".to_string(),
            Again::Aimed { bars, .. } => format!("{bars} bars"),
        }
    }

    /// What an aimed try was aimed at: None for the others.
    pub fn aim(self) -> Option<Aim> {
        match self {
            Again::Aimed { aim, .. } => Some(aim),
            _ => None,
        }
    }
}

/// Continue as the screens show it, while one is under way (from the ask
/// until Keep or Back are done).
#[derive(Clone, Debug, PartialEq)]
pub struct Continuing {
    pub track: usize,
    pub slot: usize,
    pub state: State,
    /// The shown result: what was made (each bar's job, the chords, the
    /// new notes) and the clip as it is heard with it (yours, then the new
    /// bars). None until the first result arrives.
    pub made: Option<Arc<Continued>>,
    pub clip: Option<Arc<Clip>>,
    /// Where the new bars start, in the clip's ticks.
    pub new_from: Tick,
    /// Which result is shown, from 1, and how many there are: ‹ ›.
    pub position: Option<(usize, usize)>,
    /// The result is playing in place of the clip.
    pub heard: bool,
    /// The song tick on which the music changes next: the result coming
    /// in, or the clip coming back.
    pub line: Option<Tick>,
    /// Why nothing can be heard or kept, in words, when that is so: the
    /// clip changed, a setting is in the way, Keep was refused.
    pub words: Option<String>,
    pub settings: Settings,
    /// The last 8 results, oldest first, for the tries list.
    pub tries: Vec<Try>,
}

/// One result in the tries list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Try {
    /// The style's name.
    pub style: &'static str,
    pub bars: u8,
    /// Made of part of the try before it: which.
    pub again: Option<Again>,
}
