//! MPFree's GUI toolkit: a small retained-mode UI for the MPC One's screen.
//!
//! The pieces, from the bottom up:
//!
//! - [`Canvas`] is the only thing that touches pixels. [`SoftCanvas`] draws in
//!   software into a plain pixel buffer; [`fb::FbTarget`] copies the parts that
//!   changed to `/dev/fb0`. A GPU canvas (OpenGL ES, the project's GUI
//!   decision) can implement the same trait later without any widget changing.
//! - [`Theme`] holds every colour, size and spacing, so the look changes in one
//!   place. [`Fonts`] holds the three faces (sans, bold, mono), built in or
//!   loaded from files.
//! - [`Widget`]s draw themselves into the rectangle they are given and turn
//!   touches into [`EventKind`]s. They never talk to hardware or to the audio
//!   engine: the app sets what they show and decides what their events do.
//! - Pages are layouts written as short text specs ([`layout`]), such as
//!   `col(header:60, row(pads:*, side:480):*)`. A widget can sit on several
//!   pages; rearranging a page means editing its spec, not the widgets.
//! - [`wave`] has the sampler's waveform widgets: a zoomable waveform with
//!   marker tabs and playheads, and an overview strip, drawn from peaks
//!   (min, max and RMS per pixel column) so any zoom costs the same.
//! - [`Ui`] ties these together: the current page, which widgets changed
//!   (only those are redrawn), and which widget owns each finger (the one it
//!   first touched, until it lifts).

#![forbid(unsafe_code)]

pub mod canvas;
pub mod color;
pub mod draw;
pub mod fb;
pub mod geom;
pub mod gpu_screen;
pub mod layout;
pub mod png;
pub mod qlink;
pub mod record;
pub mod shape;
pub mod style;
pub mod text;
pub mod theme;
pub mod ui;
pub mod wave;
pub mod widgets;

pub use canvas::{Canvas, Image, SoftCanvas};
pub use color::Color;
pub use geom::Rect;
pub use shape::{Part, Shape};
pub use style::Styled;
pub use text::{Align, FontId, Fonts};
pub use theme::{Backdrop, Corners, Startup, Theme, Tone};
pub use ui::{EventKind, Gesture, PageTarget, PaintCx, Target, Touch, Ui, UiEvent, Widget};
