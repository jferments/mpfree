//! mpfree-sampler-preview: WORKSTATION ONLY. Draws the sampler's sample
//! screens as PNG files, with a real project behind each one, so they can be
//! looked at before they go on the MPC. Opens no device.
//!
//! Usage: `mpfree-sampler-preview OUT_DIR`. OUT_DIR must be new or empty;
//! no file is ever overwritten.

#![forbid(unsafe_code)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use mpfree_sampler_ui::sample::samples;
use mpfree_ui::{Fonts, SoftCanvas};

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn run() -> Result<(), String> {
    let out = std::env::args().nth(1).map(PathBuf::from).ok_or("usage: mpfree-sampler-preview OUT_DIR")?;
    if out.exists() && fs::read_dir(&out).map_err(|e| format!("{}: {e}", out.display()))?.next().is_some() {
        return Err(format!("{} is not empty; give a new or empty folder", out.display()));
    }
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    for mut s in samples(Fonts::embedded)? {
        let (w, h) = s.ui.size();
        let mut c = SoftCanvas::new(w, h);
        s.ui.paint(&mut c);
        let path = out.join(format!("{}.png", s.name));
        write_new(&path, &mpfree_ui::png::encode_rgb(w as u32, h as u32, &c.to_rgb()))?;
        println!("wrote {} ({})", path.display(), s.about);
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("mpfree-sampler-preview: {e}");
        std::process::exit(1);
    }
}
