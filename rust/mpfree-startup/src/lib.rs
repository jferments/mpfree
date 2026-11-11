//! MPFree's themed startup screens.
//!
//! This crate draws three startup screens; synthwave is the default. (When
//! the theme system lands, each theme names its own; the spiral galaxy stays
//! in `mpfree-splash`.)
//!
//! - [`console::Console`]: a chaotic swirl of green characters that settles
//!   into "MPFree OS" drawn in characters (VT323 terminal font), shaded by
//!   character density and colour, then a prompt types out.
//! - [`synthwave::Synthwave`]: a striped sun rising over wavy neon hills and
//!   a racing grid, with a slashed brush-marker title over the sun.
//! - [`spaceship::Spaceship`]: the view from a ship's bridge, a purple spiral
//!   galaxy through the windshield while the dashboard and HUD boot.
//!
//! A [`Scene`] draws the whole screen for a time in seconds. Everything that
//! never moves is drawn once into layers when the scene is made; a frame
//! copies layers and draws only what moves, to stay inside the RK3288's
//! budget (see `cargo run -p mpfree-startup --release -- --bench`).
//! Scenes draw the same picture for the same time on any machine (no
//! clocks or random seeds from outside), so previews and tests are exact.

#![forbid(unsafe_code)]

pub mod console;
pub mod gfx;
pub mod spaceship;
pub mod synthwave;

use mpfree_ui::SoftCanvas;

/// The name every startup screen shows.
pub const TITLE: &str = "MPFree";
pub const VERSION: &str = "v0.1";

pub trait Scene {
    /// Draw the frame at `t` seconds into `c` (the whole screen).
    fn draw(&mut self, t: f32, c: &mut SoftCanvas);
    /// When the animation has settled; frames after this hold still (some
    /// keep a slow idle movement) and the last one can stay behind the
    /// loading card.
    fn length(&self) -> f32;
}

/// The startup screens this crate draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Console,
    Synthwave,
    Spaceship,
}

impl Kind {
    /// The startup screen MPFree shows (Jay, 2026-11-11, from round 8).
    pub const DEFAULT: Kind = Kind::Synthwave;
    pub const ALL: [Kind; 3] = [Kind::Console, Kind::Synthwave, Kind::Spaceship];

    /// By name: console, synthwave, spaceship, or default.
    pub fn from_name(name: &str) -> Option<Kind> {
        match name {
            "console" => Some(Kind::Console),
            "synthwave" => Some(Kind::Synthwave),
            "spaceship" => Some(Kind::Spaceship),
            "default" => Some(Kind::DEFAULT),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Console => "console",
            Kind::Synthwave => "synthwave",
            Kind::Spaceship => "spaceship",
        }
    }
}

/// The scene for a startup screen, sized for a w x h screen.
pub fn scene(kind: Kind, w: i32, h: i32) -> Box<dyn Scene> {
    match kind {
        Kind::Console => Box::new(console::Console::new(w, h)),
        Kind::Synthwave => Box::new(synthwave::Synthwave::new(w, h)),
        Kind::Spaceship => Box::new(spaceship::Spaceship::new(w, h)),
    }
}
