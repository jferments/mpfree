//! The mixer's I/O view: where a project's audio, MIDI, sync and CV go.
//!
//! Jay's calls (2026-11-12, the MIDI, sync and CV thread's cards): a
//! project's routing lives in an I/O view inside the mixer, and its own
//! icon beside the mixer icon opens it. Here it is [`MixView::Io`], the
//! mixer's fifth view: the master strip stays on the right as in every
//! view, a slim list of sections runs down the left, and the selected
//! section fills the rest.
//!
//! - **Audio** is the mixer's own section (this crate): every strip that
//!   has an output, one row each, with where it goes; an input's row adds
//!   its Source and Monitor. A tap on a row's name selects that strip (the
//!   Q-Links follow it), a tap on its output opens the output picker. The
//!   data wheel moves the selection, and - and + step the selected row's
//!   output.
//! - **Other sections** (MIDI ports and track outputs, clock, CV) come from
//!   other crates through [`IoSection`], so this crate depends on none of
//!   them. Each keeps its own state; the app reads it after a section says
//!   it changed something. The app hands the sections to the widget with
//!   [`crate::add_io_section`].
//!
//! [`MixView::Io`]: crate::MixView::Io

use std::time::Duration;

use mpfree_ui::{PaintCx, Rect, Touch};

/// A press or turn the app passes to the I/O view
/// ([`crate::io_input`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoInput {
    /// The data wheel, by this many detents (positive clockwise).
    Wheel(i32),
    /// The - (-1) or + (1) button.
    Step(i32),
    /// The view's page arrows: back (-1) or on (1) a page.
    Page(i32),
}

/// What a section did with a touch, a press or a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoReply {
    /// Not for it: nothing changed.
    Ignored,
    /// It took it; nothing on the screen changed.
    Handled,
    /// It took it and the screen needs drawing again. When a setting
    /// changed, the app reads the section's state now.
    Redraw,
}

impl IoReply {
    /// The stronger of two replies.
    pub fn max(self, o: IoReply) -> IoReply {
        match (self, o) {
            (IoReply::Redraw, _) | (_, IoReply::Redraw) => IoReply::Redraw,
            (IoReply::Handled, _) | (_, IoReply::Handled) => IoReply::Handled,
            _ => IoReply::Ignored,
        }
    }
}

/// One Q-Link's card while a section is showing.
#[derive(Clone, Debug, PartialEq)]
pub struct IoKnob {
    pub label: String,
    /// The value as the section writes it ("120.0 BPM", "Ch 10").
    pub text: String,
    /// How far round the knob's arc is lit, 0 to 1.
    pub pos: f32,
    /// False when the knob has nothing to move.
    pub enabled: bool,
}

/// A section of the I/O view from another crate (MIDI, sync, CV). The view
/// gives it the area right of the section list; it draws everything there,
/// a detail of one port included (open it over its own area, and close it
/// on [`IoSection::back`]). Colours and fonts come from `cx.theme`, as
/// everywhere in the app.
pub trait IoSection: Send {
    /// Its name in the section list ("MIDI", "Sync", "CV").
    fn label(&self) -> &str;

    /// Draws into `r`, which it fills (background included).
    fn paint(&self, cx: &mut PaintCx, r: Rect);

    /// A finger went down, moved or lifted at (x, y). A finger that went
    /// down inside `r` comes back here until it lifts, wherever it moves.
    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> IoReply;

    /// The data wheel, - and +, or the page arrows, while it shows.
    fn input(&mut self, _ev: IoInput) -> IoReply {
        IoReply::Ignored
    }

    /// Called each frame while it shows, with the time since the app
    /// started, for live values (activity lights, the clock's tempo,
    /// meters). [`IoReply::Redraw`] draws it again; anything else lets the
    /// screen skip the frame.
    fn tick(&mut self, _now: Duration) -> IoReply {
        IoReply::Ignored
    }

    /// It started (`true`) or stopped (`false`) showing: the mixer's I/O
    /// view opened or closed, or another section was picked. A section can
    /// stop polling what only its screen needs while it is hidden ([`tick`]
    /// only comes while it shows).
    ///
    /// [`tick`]: IoSection::tick
    fn shown(&mut self, _on: bool) {}

    /// The app's Back (or the section list tapped): close a detail if one
    /// is open. True if it closed something.
    fn back(&mut self) -> bool {
        false
    }

    /// The four Q-Links' cards while it shows; `None` leaves the knobs on
    /// the selected mixer strip.
    fn qlinks(&self) -> Option<[IoKnob; 4]> {
        None
    }

    /// Q-Link `knob` (0 to 3) turned by `detents`; `fine` while SHIFT is
    /// held.
    fn qlink_turn(&mut self, _knob: usize, _detents: i32, _fine: bool) -> IoReply {
        IoReply::Ignored
    }
}

/// The Audio section's name in the list.
pub const AUDIO: &str = "Audio";
/// The section list's width.
pub const LIST_W: i32 = 170;
/// A row of the Audio section.
pub const ROW_H: i32 = 44;
/// The Audio section's column heads.
pub const HEAD_ROW_H: i32 = 30;

/// The I/O view's own state inside the mixer widget.
pub(crate) struct IoFrame {
    pub sections: Vec<Box<dyn IoSection>>,
    /// The section showing: 0 is Audio, then `sections` in order.
    pub sel: usize,
    /// The Audio section's first row shown.
    pub top: usize,
    /// Fingers that went down in an outside section, with that section.
    pub fingers: Vec<(u64, usize)>,
}

impl IoFrame {
    pub fn new() -> IoFrame {
        IoFrame {
            sections: Vec::new(),
            sel: 0,
            top: 0,
            fingers: Vec::new(),
        }
    }

    /// The names in the section list.
    pub fn labels(&self) -> Vec<String> {
        std::iter::once(AUDIO.to_string())
            .chain(self.sections.iter().map(|s| s.label().to_string()))
            .collect()
    }

    /// The outside section showing, if one is.
    pub fn section(&self) -> Option<&dyn IoSection> {
        self.sel
            .checked_sub(1)
            .and_then(|i| self.sections.get(i))
            .map(|b| b.as_ref())
    }

    pub fn section_mut(&mut self) -> Option<&mut Box<dyn IoSection>> {
        let i = self.sel.checked_sub(1)?;
        self.sections.get_mut(i)
    }
}

/// The I/O view's areas inside the strips' area (left of the master).
pub(crate) struct IoLayout {
    pub list: Vec<Rect>,
    pub panel: Rect,
}

pub(crate) fn io_layout(area: Rect, n: usize) -> IoLayout {
    let (list_area, rest) = area.take_left(LIST_W);
    let panel = rest.take_left(12).1;
    let list = (0..n)
        .map(|i| Rect::new(list_area.x, list_area.y + i as i32 * 52, list_area.w, 44))
        .collect();
    IoLayout { list, panel }
}

/// The parts of one Audio row: name, output, and an input's Source and
/// Monitor.
pub(crate) struct RowParts {
    pub name: Rect,
    pub output: Rect,
    pub source: Rect,
    pub monitor: Rect,
}

pub(crate) fn row_parts(r: Rect) -> RowParts {
    let inner = r.inset_xy(8, 4);
    let (name, rest) = inner.take_left(220);
    let rest = rest.take_left(10).1;
    let (output, rest) = rest.take_left(190);
    let rest = rest.take_left(10).1;
    let (source, rest) = rest.take_left(150);
    let rest = rest.take_left(10).1;
    let monitor = rest.take_left(150.min(rest.w)).0;
    RowParts {
        name,
        output,
        source,
        monitor,
    }
}

/// Rows of the Audio section that fit in `panel`.
pub(crate) fn rows_per_page(panel: Rect) -> usize {
    (((panel.h - HEAD_ROW_H) / ROW_H).max(1)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_take_the_stronger() {
        use IoReply::*;
        assert_eq!(Ignored.max(Handled), Handled);
        assert_eq!(Handled.max(Redraw), Redraw);
        assert_eq!(Ignored.max(Ignored), Ignored);
    }
}
