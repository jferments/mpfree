//! Preview and time the themed startup screens on the workstation.
//!
//! Usage:
//!   mpfree-startup --preview DIR [--theme NAME] [--fps N] [--seconds S]
//!       Draws every frame and saves them as PNG files in DIR (new files
//!       only; DIR must be new or empty). Without --theme: console,
//!       synthwave and spaceship, each in its own folder.
//!   mpfree-startup --bench [--theme NAME] [--frames N]
//!       Draws frames as fast as it can and prints the time per frame.
//!
//! It touches no device: it only writes PNG files in DIR.

use mpfree_startup::{scene, Scene};
use mpfree_ui::{SoftCanvas, Theme};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

const SCREEN: (i32, i32) = (1280, 800);
const THEMES: [&str; 3] = ["console", "synthwave", "spaceship"];

fn make(name: &str) -> Result<Box<dyn Scene>, String> {
    let t = Theme::builtin(name).ok_or_else(|| format!("no theme {name:?}"))?;
    scene(t.startup, SCREEN.0, SCREEN.1).ok_or_else(|| format!("{name}: its startup screen is mpfree-splash"))
}

fn write_png(path: &Path, c: &SoftCanvas) -> Result<(), String> {
    let png = mpfree_ui::png::encode_rgb(c.width() as u32, c.height() as u32, &c.to_rgb());
    OpenOptions::new().write(true).create_new(true).open(path).and_then(|mut f| f.write_all(&png))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn preview(dir: &Path, names: &[&str], fps: f32, seconds: Option<f32>) -> Result<(), String> {
    for name in names {
        let mut s = make(name)?;
        let d = if names.len() > 1 { dir.join(name) } else { dir.to_path_buf() };
        fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        if fs::read_dir(&d).map(|mut r| r.next().is_some()).unwrap_or(false) {
            return Err(format!("{} is not empty", d.display()));
        }
        let len = seconds.unwrap_or(s.length() + 1.0);
        let n = (len * fps).ceil() as usize;
        let mut c = SoftCanvas::new(SCREEN.0, SCREEN.1);
        for i in 0..=n {
            s.draw(i as f32 / fps, &mut c);
            write_png(&d.join(format!("{name}-{i:04}.png")), &c)?;
        }
        println!("{name}: {} frames in {}", n + 1, d.display());
    }
    Ok(())
}

fn bench(names: &[&str], frames: usize) -> Result<(), String> {
    for name in names {
        let t0 = Instant::now();
        let mut s = make(name)?;
        let setup = t0.elapsed().as_secs_f64() * 1000.0;
        let mut c = SoftCanvas::new(SCREEN.0, SCREEN.1);
        let mut worst = 0.0f64;
        let t1 = Instant::now();
        for i in 0..frames {
            let f = Instant::now();
            s.draw(i as f32 * s.length() / frames as f32, &mut c);
            worst = worst.max(f.elapsed().as_secs_f64() * 1000.0);
        }
        let avg = t1.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!("{name}: setup {setup:.0} ms, frame {avg:.1} ms average, {worst:.1} ms slowest ({frames} frames)");
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir: Option<PathBuf> = None;
    let (mut theme, mut fps, mut seconds, mut frames, mut do_bench) = (None, 30.0f32, None, 120usize, false);
    let mut i = 0;
    let result = (|| {
        while i < args.len() {
            let a = args[i].as_str();
            let mut val = || {
                i += 1;
                args.get(i).cloned().ok_or_else(|| format!("{a} needs a value"))
            };
            match a {
                "--preview" => dir = Some(PathBuf::from(val()?)),
                "--theme" => theme = Some(val()?),
                "--fps" => fps = val()?.parse().map_err(|_| "--fps: a number".to_string())?,
                "--seconds" => seconds = Some(val()?.parse().map_err(|_| "--seconds: a number".to_string())?),
                "--frames" => frames = val()?.parse().map_err(|_| "--frames: a number".to_string())?,
                "--bench" => do_bench = true,
                _ => return Err(format!("unknown option {a}")),
            }
            i += 1;
        }
        let names: Vec<&str> = match &theme {
            Some(t) => vec![t.as_str()],
            None => THEMES.to_vec(),
        };
        match (&dir, do_bench) {
            (Some(d), _) => preview(d, &names, fps.clamp(1.0, 120.0), seconds),
            (None, true) => bench(&names, frames.max(1)),
            _ => Err("give --preview DIR or --bench".into()),
        }
    })();
    if let Err(e) = result {
        eprintln!("mpfree-startup: {e}");
        std::process::exit(2);
    }
}
