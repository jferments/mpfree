//! Continue (composing tools, phase 1) as the time engine's screens see
//! it: what a touch asks for ([`ContinueAction`], [`ContinueAsk`]) and
//! what the screens show while a clip is being carried on ([`Continuing`]).
//!
//! The time engine runs it (mpfree-app's `time/compose.rs`): a result is
//! heard in place of its clip from the next bar line, Keep writes it as
//! one take, Back leaves the clip as it was. The design is
//! docs/dev/design/compose.md.

use std::sync::Arc;

use mpfree_compose::{Continued, InstrumentKind, Lanes, Settings, State};
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
}

/// Continue, or Try again on everything: a new result for clip `slot` of
/// `track` with these settings. The time engine picks the seed, so each
/// ask sounds new.
#[derive(Clone, Debug, PartialEq)]
pub struct ContinueAsk {
    pub track: usize,
    pub slot: usize,
    pub settings: Settings,
    /// The kind of instrument the track plays.
    pub kind: InstrumentKind,
    /// The drum each pad of a drum track plays, as the kit names them.
    pub lanes: Lanes,
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
}
