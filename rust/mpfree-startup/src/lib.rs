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
//!   a racing grid, with the title in blade letters ([`blade`]) over the sun.
//! - [`spaceship::Spaceship`]: a purple spiral galaxy in open space under a
//!   sci-fi overlay analysing it, then the same title, white to violet.
//!
//! A [`Scene`] draws the whole screen for a time in seconds. Everything that
//! never moves is drawn once into layers when the scene is made; a frame
//! copies layers and draws only what moves, to stay inside the RK3288's
//! budget (see `cargo run -p mpfree-startup --release -- --bench`).
//! Scenes draw the same picture for the same time on any machine (no
//! clocks or random seeds from outside), so previews and tests are exact.

#![forbid(unsafe_code)]

pub mod blade;
pub mod console;
pub mod gfx;
pub mod spaceship;
pub mod synthwave;

use mpfree_ui::{SoftCanvas, Startup, Theme};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

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

/// The startup screen for the theme the user picked in Prefs > Appearance,
/// read from the data folder the app saves to (`--data`,
/// docs/dev/design/storage-spec.md): `settings/app.conf`, key `theme`. That is
/// a built-in name, or a theme file in the themes folder
/// (`files/themes/NAME.theme`), whose `startup` (or its base's) is used.
/// Gives the built-in theme whose startup screen that is ("console",
/// "synthwave", "spaceship"; "default" for synthwave).
///
/// Only reads, and only those two files; follows no symlinks, never makes
/// a folder, never writes. Anything missing, unreadable, too big, not a
/// theme, or the galaxy (which is mpfree-splash) gives None, so the caller
/// shows the default: a startup screen must always come up.
pub fn saved_theme(data: &Path) -> Option<String> {
    let app = read_plain(data, &["settings", "app.conf"])?;
    let name = mpfree_store::Settings::parse(&app)
        .get_str("theme")?
        .trim()
        .to_ascii_lowercase();
    if Theme::builtin(&name).is_some() {
        return Some(name);
    }
    if !name.ends_with(".theme") || !mpfree_store::valid_name(&name) {
        return None;
    }
    let text = read_plain(data, &["files", "themes", &name])?;
    match Theme::parse(&text).ok()?.startup {
        Startup::Galaxy => None,
        Startup::Console => Some("console".into()),
        Startup::Synthwave => Some("synthwave".into()),
        Startup::Spaceship => Some("spaceship".into()),
    }
}

/// A plain text file under `dir` (folders, then the file; no symlinks),
/// at most the store's largest file.
fn read_plain(dir: &Path, parts: &[&str]) -> Option<String> {
    let mut path = dir.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        path.push(part);
        let m = fs::symlink_metadata(&path).ok()?;
        let last = i + 1 == parts.len();
        if last && !m.is_file() || !last && !m.is_dir() {
            return None;
        }
    }
    let mut bytes = Vec::new();
    File::open(&path)
        .ok()?
        .take(mpfree_store::MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > mpfree_store::MAX_FILE {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mpfree-startup-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("settings")).unwrap();
        d
    }

    #[test]
    fn saved_theme_reads_the_apps_settings() {
        let d = folder("saved");
        fs::write(
            d.join("settings/app.conf"),
            "curve = soft\ntheme = console\n",
        )
        .unwrap();
        assert_eq!(saved_theme(&d).as_deref(), Some("console"));
        fs::write(d.join("settings/app.conf"), "theme = Spaceship\n").unwrap();
        assert_eq!(saved_theme(&d).as_deref(), Some("spaceship"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn saved_theme_follows_a_themes_folder_file() {
        let d = folder("file");
        fs::create_dir_all(d.join("files/themes")).unwrap();
        fs::write(d.join("settings/app.conf"), "theme = sunrise.theme\n").unwrap();
        assert_eq!(saved_theme(&d), None, "the file is not there");
        fs::write(
            d.join("files/themes/sunrise.theme"),
            "base = spaceship\nname = Sunrise\n",
        )
        .unwrap();
        assert_eq!(saved_theme(&d).as_deref(), Some("spaceship"));
        fs::write(
            d.join("files/themes/sunrise.theme"),
            "base = spaceship\nstartup = console\n",
        )
        .unwrap();
        assert_eq!(saved_theme(&d).as_deref(), Some("console"));
        fs::write(d.join("files/themes/sunrise.theme"), "startup = galaxy\n").unwrap();
        assert_eq!(saved_theme(&d), None, "the galaxy is mpfree-splash");
        fs::write(d.join("files/themes/sunrise.theme"), "accent = blue\n").unwrap();
        assert_eq!(saved_theme(&d), None, "not a theme");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn saved_theme_falls_back_on_anything_odd() {
        let d = folder("odd");
        assert_eq!(saved_theme(&d), None, "no app.conf");
        fs::write(d.join("settings/app.conf"), "curve = soft\n").unwrap();
        assert_eq!(saved_theme(&d), None, "no theme key");
        fs::write(d.join("settings/app.conf"), "theme = ../../etc\n").unwrap();
        assert_eq!(saved_theme(&d), None, "not a built-in theme");
        fs::write(d.join("settings/app.conf"), [0xFFu8, 0xFE, b'\n']).unwrap();
        assert_eq!(saved_theme(&d), None, "not text");
        assert_eq!(saved_theme(&d.join("nowhere")), None, "no folder");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn saved_theme_writes_nothing() {
        let d = std::env::temp_dir().join(format!("mpfree-startup-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        assert_eq!(saved_theme(&d), None);
        assert_eq!(
            fs::read_dir(&d).unwrap().count(),
            0,
            "the folder stays empty"
        );
        let _ = fs::remove_dir_all(&d);
    }
}
