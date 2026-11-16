//! Preview and time the themed startup screens on the workstation.
//!
//! Usage:
//!   mpfree-startup --preview DIR [--theme NAME | --data DIR] [--fps N] [--seconds S]
//!       Draws every frame and saves them as PNG files in DIR (new files
//!       only; DIR must be new or empty). Without --theme: console,
//!       synthwave and spaceship, each in its own folder.
//!   mpfree-startup --bench [--theme NAME | --data DIR] [--frames N]
//!       Draws frames as fast as it can and prints the time per frame.
//!       Touches no device and writes nothing.
//!   mpfree-startup --framebuffer [--theme NAME | --data DIR] [--fb DEVICE] [--sysfs DIR]
//!                  [--rotation 0|90|180|270] [--seconds N] [--fps N] [--setup-status FILE]
//!       On the MPC One: plays the startup screen on /dev/fb0 for --seconds
//!       (default: the scene's own length), then exits and leaves its last
//!       frame on the screen for the app's loading page (`app --keep-screen`).
//!       Without --theme, the theme saved in --data, else the default
//!       theme's startup screen (synthwave).
//!       It writes only to the framebuffer device and to the framebuffer's
//!       sysfs `pan` file (which asks the kernel to show the framebuffer;
//!       nothing is stored), like mpfree-splash. It never writes storage.
//!       With --setup-status FILE (or MPFREE_SETUP_STATUS=FILE, which the
//!       USB creator's sticks set in a drop-in for the splash unit), it then
//!       stays while the stick's first-start format step runs, showing the
//!       pad player's loading card over its last frame with what is
//!       happening and how far it is (mpfree_startup::setup); it only reads
//!       FILE, and ends when the step is done or had nothing to do.
//!
//! --data DIR (any mode): the app's data folder (`app --data`, on the stick
//!       /run/mpfree-data). When --theme is not given, the startup screen is
//!       the theme picked in Prefs > Appearance (`settings/app.conf`, key
//!       `theme`; for a theme file, its `startup` from `files/themes/`).
//!       Only those files are read; nothing in DIR is written or made. If
//!       anything is missing or odd, the default (synthwave) shows.
//!
//! --preview writes only PNG files in DIR.

use mpfree_screen_log::Rotation;
use mpfree_startup::setup::{self, Smooth};
use mpfree_startup::{scene, Scene};
use mpfree_ui::fb::FbTarget;
use mpfree_ui::widgets::Loading;
use mpfree_ui::{Fonts, Image, Rect, SoftCanvas, Theme, Ui};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const SCREEN: (i32, i32) = (1280, 800);
const THEMES: [&str; 3] = ["console", "synthwave", "spaceship"];
const USAGE: &str = "usage: mpfree-startup --preview DIR [--theme NAME | --data DIR] [--fps N] [--seconds S]\n       \
    mpfree-startup --bench [--theme NAME | --data DIR] [--frames N]\n       \
    mpfree-startup --framebuffer [--theme NAME | --data DIR] [--fb DEVICE] [--sysfs DIR] [--rotation 0|90|180|270] [--seconds N] [--fps N] [--setup-status FILE]";

fn make(name: &str) -> Result<Box<dyn Scene>, String> {
    make_sized(name, SCREEN)
}

fn make_sized(name: &str, (w, h): (i32, i32)) -> Result<Box<dyn Scene>, String> {
    let t = Theme::builtin(name).ok_or_else(|| format!("no theme {name:?}"))?;
    scene(t.startup, w, h).ok_or_else(|| format!("{name}: its startup screen is mpfree-splash"))
}

/// Play the scene on the framebuffer in real time, then leave its last
/// frame showing (and with `setup_status`, wait for the first-start format
/// step over it).
fn framebuffer(
    name: &str,
    fb: &Path,
    sysfs: &Path,
    rotation: Rotation,
    seconds: Option<f32>,
    fps: f32,
    setup_status: Option<&Path>,
) -> Result<(), String> {
    let mut target = FbTarget::open(fb, sysfs, rotation)?;
    let (w, h) = target.logical_size();
    let mut s = make_sized(name, (w, h))?;
    let seconds = seconds.unwrap_or(s.length()).max(0.0);
    let mut c = SoftCanvas::new(w, h);
    let all = [Rect::new(0, 0, w, h)];
    let frame = Duration::from_secs_f32(1.0 / fps);
    let start = Instant::now();
    loop {
        let t = start.elapsed().as_secs_f32().min(seconds);
        s.draw(t, &mut c);
        target.present(&c, &all)?;
        if t >= seconds {
            break;
        }
        // Wait for the next frame, but never past the end.
        let next = (t + frame.as_secs_f32()).min(seconds);
        let wait = next - start.elapsed().as_secs_f32();
        if wait > 0.0 {
            thread::sleep(Duration::from_secs_f32(wait));
        }
    }
    match setup_status {
        Some(status) => wait_for_setup(&mut target, &c, status),
        None => Ok(()),
    }
}

/// A creator stick's first start: hold the last frame, with the pad
/// player's loading card over it while the format step runs (see
/// mpfree_startup::setup). Ends when the step has finished or had nothing
/// to do, when it does not answer within setup::ANSWER, or at setup::LIMIT.
fn wait_for_setup(target: &mut FbTarget, last: &SoftCanvas, status: &Path) -> Result<(), String> {
    let (w, h) = (last.width(), last.height());
    let mut img = Image::new(w, h);
    img.px.copy_from_slice(&last.px);
    let mut loading = Loading::new(mpfree_startup::TITLE, "");
    loading.set_backdrop(Some(Arc::new(img)));
    let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), w, h);
    ui.add("loading", loading);
    ui.add_full_page("Loading", "loading")?;
    let mut canvas = SoftCanvas::new(w, h);
    let mut smooth = Smooth::new();
    let start = Instant::now();
    loop {
        let t = start.elapsed();
        let report = fs::read_to_string(status)
            .ok()
            .and_then(|text| setup::parse(&text));
        if t >= setup::LIMIT || report.is_none() && t >= setup::ANSWER {
            eprintln!(
                "mpfree-startup: the first-start format step {} after {:.0} s; the pad player follows",
                if report.is_none() { "did not answer" } else { "was still running" },
                t.as_secs_f32()
            );
            return Ok(());
        }
        let state = report.as_ref().map(|r| r.state.clone()).unwrap_or_default();
        if report.as_ref().is_some_and(|r| r.finished()) && state != "done" {
            return Ok(());
        }
        if let Some((message, detail, value)) =
            setup::card(report.as_ref(), &mut smooth, t.as_secs_f32())
        {
            if let Some(l) = ui.get_mut::<Loading>("loading") {
                l.message = message;
                l.detail = detail;
                l.progress = value;
            }
            let damage = ui.paint(&mut canvas);
            if !damage.is_empty() {
                target.present(&canvas, &damage)?;
            }
            if state == "done" {
                thread::sleep(Duration::from_millis(400));
            }
        }
        if state == "done" {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(125));
    }
}

fn write_png(path: &Path, c: &SoftCanvas) -> Result<(), String> {
    let png = mpfree_ui::png::encode_rgb(c.width() as u32, c.height() as u32, &c.to_rgb());
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(&png))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn preview(dir: &Path, names: &[&str], fps: f32, seconds: Option<f32>) -> Result<(), String> {
    for name in names {
        let mut s = make(name)?;
        let d = if names.len() > 1 {
            dir.join(name)
        } else {
            dir.to_path_buf()
        };
        fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        if fs::read_dir(&d)
            .map(|mut r| r.next().is_some())
            .unwrap_or(false)
        {
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
    let mut data: Option<PathBuf> = None;
    let (mut theme, mut fps, mut seconds, mut frames, mut do_bench) =
        (None, 30.0f32, None, 120usize, false);
    let mut on_fb = false;
    let mut setup_status = std::env::var_os(setup::STATUS_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let (mut fb, mut sysfs) = (
        PathBuf::from("/dev/fb0"),
        PathBuf::from("/sys/class/graphics/fb0"),
    );
    // The MPC One's panel is mounted left side up (device tree rotation = 270).
    let mut rotation = Rotation::Deg270;
    let mut i = 0;
    let result = (|| {
        while i < args.len() {
            let a = args[i].as_str();
            let mut val = || {
                i += 1;
                args.get(i)
                    .cloned()
                    .ok_or_else(|| format!("{a} needs a value"))
            };
            match a {
                "--preview" => dir = Some(PathBuf::from(val()?)),
                "--theme" => theme = Some(val()?),
                "--data" => data = Some(PathBuf::from(val()?)),
                "--fps" => fps = val()?.parse().map_err(|_| "--fps: a number".to_string())?,
                "--seconds" => {
                    seconds = Some(
                        val()?
                            .parse()
                            .map_err(|_| "--seconds: a number".to_string())?,
                    )
                }
                "--frames" => {
                    frames = val()?
                        .parse()
                        .map_err(|_| "--frames: a number".to_string())?
                }
                "--bench" => do_bench = true,
                "--framebuffer" => on_fb = true,
                "--fb" => fb = PathBuf::from(val()?),
                "--setup-status" => setup_status = Some(PathBuf::from(val()?)),
                "--sysfs" => sysfs = PathBuf::from(val()?),
                "--rotation" => {
                    let v = val()?;
                    rotation = v
                        .parse::<u32>()
                        .ok()
                        .and_then(Rotation::from_degrees)
                        .ok_or_else(|| format!("--rotation must be 0, 90, 180 or 270, not {v}"))?;
                }
                _ => return Err(format!("unknown option {a}\n{USAGE}")),
            }
            i += 1;
        }
        if let (None, Some(d)) = (&theme, &data) {
            let saved = mpfree_startup::saved_theme(d);
            eprintln!(
                "mpfree-startup: theme {} ({})",
                saved.as_deref().unwrap_or("default"),
                if saved.is_some() {
                    "saved in --data"
                } else {
                    "nothing saved in --data"
                }
            );
            theme = Some(saved.unwrap_or_else(|| "default".into()));
        }
        if on_fb {
            if dir.is_some() || do_bench {
                return Err("--framebuffer cannot go with --preview or --bench".into());
            }
            let name = theme.as_deref().unwrap_or("default");
            return framebuffer(
                name,
                &fb,
                &sysfs,
                rotation,
                seconds,
                fps.clamp(1.0, 60.0),
                setup_status.as_deref(),
            );
        }
        let names: Vec<&str> = match &theme {
            Some(t) => vec![t.as_str()],
            None => THEMES.to_vec(),
        };
        match (&dir, do_bench) {
            (Some(d), _) => preview(d, &names, fps.clamp(1.0, 120.0), seconds),
            (None, true) => bench(&names, frames.max(1)),
            _ => Err(USAGE.into()),
        }
    })();
    if let Err(e) = result {
        eprintln!("mpfree-startup: {e}");
        std::process::exit(2);
    }
}
