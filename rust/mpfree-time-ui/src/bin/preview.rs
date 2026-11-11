//! mpfree-time-preview: WORKSTATION ONLY. Draws the time engine's sample
//! screens (the transport bar, the Play page with the bar, and the Clips
//! page while playing, recording, after an undo, counting in and stopped)
//! as PNG files, with a real transport, player, recorder and take undo
//! behind each one, so the screens can be looked at before they go on the
//! MPC. Opens no device.
//!
//! Options: `--out DIR` (default ~/mpfree-work/time-preview-<UTC>; must be a
//! new or empty folder), `--font FILE`, `--bold-font FILE`, `--mono-font FILE`.

#![forbid(unsafe_code)]

use mpfree_time_ui::sample::samples;
use mpfree_ui::{Fonts, SoftCanvas};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

struct Options {
    out: Option<PathBuf>,
    font: Option<PathBuf>,
    bold: Option<PathBuf>,
    mono: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options { out: None, font: None, bold: None, mono: None };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().map(PathBuf::from).ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--out" => o.out = Some(value()?),
            "--font" => o.font = Some(value()?),
            "--bold-font" => o.bold = Some(value()?),
            "--mono-font" => o.mono = Some(value()?),
            "-h" | "--help" => return Err(String::new()),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    Ok(o)
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

/// New files only: never overwrite.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn run(o: &Options) -> Result<PathBuf, String> {
    let dir = o.out.clone().unwrap_or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let work = std::env::var_os("MPFREE_WORK_DIR").map(PathBuf::from).unwrap_or_else(|| home.join("mpfree-work"));
        work.join(format!("time-preview-{}", utc_stamp()))
    });
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let fonts = || Fonts::with_files(o.font.as_deref(), o.bold.as_deref(), o.mono.as_deref());
    fonts()?;
    let mut list = String::new();
    for mut s in samples(|| fonts().unwrap_or_else(|_| Fonts::embedded()))? {
        let (w, h) = s.ui.size();
        let mut canvas = SoftCanvas::new(w, h);
        s.ui.paint(&mut canvas);
        let png = mpfree_ui::png::encode_rgb(w as u32, h as u32, &canvas.to_rgb());
        write_new(&dir.join(format!("{}.png", s.name)), &png)?;
        let line = format!("{}.png  {}", s.name, s.about);
        println!("{line}");
        list.push_str(&line);
        list.push('\n');
    }
    write_new(&dir.join("screens.txt"), list.as_bytes())?;
    Ok(dir)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args).and_then(|o| run(&o)) {
        Ok(dir) => println!("\nPNG files and screens.txt are in {}", dir.display()),
        Err(e) => {
            if !e.is_empty() {
                eprintln!("mpfree-time-preview: {e}");
            }
            eprintln!("usage: mpfree-time-preview [--out NEW_DIR] [--font FILE] [--bold-font FILE] [--mono-font FILE]");
            std::process::exit(if e.is_empty() { 0 } else { 2 });
        }
    }
}
