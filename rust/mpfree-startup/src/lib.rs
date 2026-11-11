//! MPFree's themed startup screens.
//!
//! Each theme in `mpfree-ui` names a startup screen ([`Startup`]). This crate
//! draws three of them (synthwave is the default theme's); the fourth, the
//! spiral galaxy, is `mpfree-splash`.
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

use mpfree_ui::{SoftCanvas, Startup};

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

/// The scene for a theme's startup screen, or None for the galaxy (which is
/// `mpfree-splash`).
pub fn scene(kind: Startup, w: i32, h: i32) -> Option<Box<dyn Scene>> {
    match kind {
        Startup::Galaxy => None,
        Startup::Console => Some(Box::new(console::Console::new(w, h))),
        Startup::Synthwave => Some(Box::new(synthwave::Synthwave::new(w, h))),
        Startup::Spaceship => Some(Box::new(spaceship::Spaceship::new(w, h))),
    }
}
