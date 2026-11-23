//! Add a part and the row's chords (composing tools S2.6) as the time
//! engine's screens see them: what a touch asks for ([`AddAction`],
//! [`PartAsk`], [`ChordsAction`]), what the screens show while a part is
//! being added ([`Adding`]), and the kept clips that no longer fit their
//! row's chords ([`Unfit`]).
//!
//! The time engine runs them (mpfree-app's `time/adding.rs`): a part is
//! heard in its empty cell with the row from the next bar line, Keep
//! writes it (with any chords picked on the panel) as one take, Back
//! leaves the row as it was. A chord set outside the panel is one take;
//! the clips it leaves off the chords are marked, never rewritten, and Fit
//! moves the ones asked as one take. The design is
//! docs/dev/design/compose.md (S2.5, S2.6).

use std::sync::Arc;

use mpfree_compose::{InstrumentKind, Lanes, PartMade, Relation, Settings};
use mpfree_context::chord::ChordTimeline;
use mpfree_context::PartRole;
use mpfree_transport::time::Tick;

/// A touch on the Add a part panel.
#[derive(Clone, Debug, PartialEq)]
pub enum AddAction {
    /// Add a part, a choice changed, or Try again: a new part for the
    /// row with these choices. The time engine picks the seed, so each ask
    /// sounds new.
    Ask(Box<PartAsk>),
    /// ‹ (`back`) or ›: the try before or after this one, of the last 8,
    /// heard from the next bar line.
    Step { back: bool },
    /// Chords picked on the panel: the row's whole line as the panel has
    /// it. The part being tried plays them at once (its notes moved to
    /// fit, as Fit moves them), the next try is made over them, and Keep
    /// puts them on the row with the part. Nothing is written before Keep.
    Line(ChordTimeline),
    /// Keep: the shown try in its cell, with the panel's chords, as one
    /// take that UNDO takes back.
    Keep,
    /// Back: nothing written; the row as it was from the next bar line.
    Back,
}

/// Add a part, or Try again: a part for the empty cell of `track` in row
/// `row`, played by `kind`, made by `part` and `relation` with `settings`.
#[derive(Clone, Debug, PartialEq)]
pub struct PartAsk {
    pub row: usize,
    /// The track whose empty cell in the row takes the part.
    pub track: usize,
    pub part: PartRole,
    pub relation: Relation,
    pub settings: Settings,
    /// What plays it: the track's instrument, as the screen knows it.
    pub kind: InstrumentKind,
    /// The drum each pad of every kit track plays, as the kits name them.
    pub lanes: Lanes,
    /// For drums, the pad for kick, snare, hats and the rest.
    pub pads: [Option<u8>; 4],
}

/// The row's chords outside the Add a part panel (the Chords tab).
#[derive(Clone, Debug, PartialEq)]
pub enum ChordsAction {
    /// Use: row `row`'s chords as picked, one take ("row N's chords").
    /// Kept clips the change leaves off the chords are marked. `shown` is
    /// the line as the screen showed it before the change (the chords
    /// heard): what the clips are measured against when the row had no
    /// chords of its own.
    Set {
        row: usize,
        line: ChordTimeline,
        shown: ChordTimeline,
    },
    /// Fit to the new chords: the marked clips of row `row` on `tracks`
    /// (the ticked ones) moved to the row's chords, one take ("the new
    /// chords").
    Fit { row: usize, tracks: Vec<usize> },
    /// Not a touch: the row whose chords the screens show (the Chords
    /// tab's), heard again whenever its clips or chords change, into
    /// [`crate::view::TimeState::row_chords`]; None stops.
    Hear { row: Option<usize> },
}

/// Where Add a part is, for the screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddState {
    /// A part is being made.
    Making,
    /// A try is ready: it plays in its cell from the next bar line, or
    /// once the row plays.
    Ready,
    /// The shown try plays in its cell with the row.
    Playing,
    /// Nothing to hear: why is in [`Adding::words`].
    Nothing,
}

/// Add a part as the screens show it, from the ask until Keep or Back
/// are done.
#[derive(Clone, Debug, PartialEq)]
pub struct Adding {
    pub row: usize,
    pub track: usize,
    pub state: AddState,
    /// The shown try: the part, the chords it plays over and the clips it
    /// heard. None until the first arrives.
    pub made: Option<Arc<PartMade>>,
    /// Which try is shown, from 1, and how many there are: ‹ ›.
    pub position: Option<(usize, usize)>,
    /// The song tick on which the music changes next: the try coming in,
    /// or the cell going quiet again.
    pub line: Option<Tick>,
    /// The chords picked on the panel and not kept yet.
    pub picked: Option<ChordTimeline>,
    /// Why nothing can be heard or kept, in words, when that is so.
    pub words: Option<String>,
    pub part: PartRole,
    pub relation: Relation,
}

/// A row whose chords changed since its clips fitted them, and the kept
/// clips that no longer do. The screens mark those cells and the Chords
/// tab offers Fit.
#[derive(Clone, Debug, PartialEq)]
pub struct Unfit {
    pub row: usize,
    /// The row's chords as its clips fitted them.
    pub before: ChordTimeline,
    /// Each clip off the new chords: its track, and how many of its notes
    /// Fit would move.
    pub clips: Vec<(usize, usize)>,
}
