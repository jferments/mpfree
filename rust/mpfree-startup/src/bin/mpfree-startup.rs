//! Preview and time the themed startup screens on the workstation.
//!
//! --theme NAME is console, synthwave, spaceship or default.
//!
//! Usage:
//!   mpfree-startup --preview DIR [--theme NAME] [--fps N] [--seconds S]
//!       Draws every frame and saves them as PNG files in DIR (new files
//!       only; DIR must be new or empty). Without --theme: console,
//!       synthwave and spaceship, each in its own folder.
//!   mpfree-startup --bench [--theme NAME] [--frames N]
//!       Draws frames as fast as it can and prints the time per frame.
//!       Touches no device and writes nothing.
//!   mpfree-startup --framebuffer [--theme NAME] [--fb DEVICE] [--sysfs DIR]
//!                  [--rotation 0|90|180|270] [--seconds N] [--fps N]
//!       On the MPC One: plays the startup screen on /dev/fb0 for --seconds
//!       (default: the scene's own length), then exits and leaves its last
//!       frame on the screen for the app's loading page (`app --keep-screen`).
//!       Without --theme, the default startup screen (synthwave).
//!       It writes only to the framebuffer device and to the framebuffer's
//!       sysfs `pan` file (which asks the kernel to show the framebuffer;
//!       nothing is stored), like mpfree-splash. It never writes storage.
//!
//! --preview writes only PNG files in DIR.

use mpfree_screen_log::Rotation;
use mpfree_startup::{scene, Kind, Scene};
use mpfree_ui::fb::FbTarget;
use mpfree_ui::{Rect, SoftCanvas};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const SCREEN: (i32, i32) = (1280, 800);
const THEMES: [&str; 3] = ["console", "synthwave", "spaceship"];
const USAGE: &str = "usage: mpfree-startup --preview DIR [--theme NAME] [--fps N] [--seconds S]\n       \
    mpfree-startup --bench [--theme NAME] [--frames N]\n       \
    mpfree-startup --framebuffer [--theme NAME] [--fb DEVICE] [--sysfs DIR] [--rotation 0|90|180|270] [--seconds N] [--fps N]";

fn make(name: &str) -> Result<Box<dyn Scene>, String> {
    make_sized(name, SCREEN)
}

fn make_sized(name: &str, (w, h): (i32, i32)) -> Result<Box<dyn Scene>, String> {
    let k = Kind::from_name(name).ok_or_else(|| {
        format!("no startup screen {name:?} (console, synthwave, spaceship or default)")
    })?;
    Ok(scene(k, w, h))
}

/// Play the scene on the framebuffer in real time, then leave its last
/// frame showing.
fn framebuffer(
    name: &str,
    fb: &Path,
    sysfs: &Path,
    rotation: Rotation,
    seconds: Option<f32>,
    fps: f32,
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
    Ok(())
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
    let (mut theme, mut fps, mut seconds, mut frames, mut do_bench) =
        (None, 30.0f32, None, 120usize, false);
    let mut on_fb = false;
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
        if on_fb {
            if dir.is_some() || do_bench {
                return Err("--framebuffer cannot go with --preview or --bench".into());
            }
            let name = theme.as_deref().unwrap_or("default");
            return framebuffer(name, &fb, &sysfs, rotation, seconds, fps.clamp(1.0, 60.0));
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
