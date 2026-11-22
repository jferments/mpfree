//! The layout's sizes: the text sizes, the space between panels and
//! inside them, and the faces text is drawn in. They are the same in
//! every theme (owner, 22 Nov 2026: no theme may break a layout), so a
//! theme file can't set them; the [layout spec] owns them. A theme still
//! carries them ([`crate::Theme::size_body`] and the rest) so drawing code
//! reads every measure from one place, always these.
//!
//! [layout spec]: ../../../docs/dev/design/layout-spec.md

use crate::text::FontId;

/// Space between panels and around the page.
pub const GAP: i32 = 12;
/// Space inside a panel, from its edge to its content.
pub const PAD: i32 = 14;
/// The wordmark's size.
pub const SIZE_BRAND: f32 = 28.0;
/// A panel's title.
pub const SIZE_TITLE: f32 = 19.0;
/// Body text and button labels.
pub const SIZE_BODY: f32 = 17.0;
/// Small text: values, notes, hints.
pub const SIZE_SMALL: f32 = 14.0;
/// Numbers and code in the mono face.
pub const SIZE_MONO: f32 = 14.0;
/// The face drawn wherever a widget asks for the regular face.
pub const FACE_BODY: FontId = FontId::Sans;
/// The face drawn wherever a widget asks for the bold face.
pub const FACE_TITLE: FontId = FontId::Bold;

/// The theme file keys these replaced: a line setting one is ignored,
/// with a note ([`crate::theme::ProblemKind::Ignored`]).
pub const RETIRED_KEYS: [&str; 9] = [
    "gap",
    "pad",
    "size_brand",
    "size_title",
    "size_body",
    "size_small",
    "size_mono",
    "face_body",
    "face_title",
];
