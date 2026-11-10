//! MPFree startup screen: a spinning spiral galaxy with "MPCFree OS v0.1" in
//! block letters at its centre, drawn with ratatui.
//!
//! Usage:
//!   mpfree-splash [--seconds N] [--fps N]
//!       Desktop preview in this terminal. Press q, Esc or Ctrl-C to quit.
//!   mpfree-splash --preview DIR [--frames N] [--every SECONDS] [--rotation 0|90|180|270]
//!       Draws frames exactly as the MPC One's framebuffer would hold them and
//!       saves what the viewer would see as PNG files in DIR. Touches no device.
//!   mpfree-splash --framebuffer [--fb DEVICE] [--sysfs DIR] [--rotation N] [--seconds N] [--fps N]
//!       On a screen with no text console: draws on /dev/fb0. It writes only
//!       to the framebuffer device and to the framebuffer's sysfs `pan` file
//!       (which asks the kernel to show the framebuffer; nothing is stored).
//!       It never writes storage. Stops after --seconds (0 = until killed).
#![forbid(unsafe_code)]

use mpfree_splash::fb::{self, FramebufferBackend, Geometry, Rotation};
use mpfree_splash::galaxy::{Galaxy, GalaxyWidget};
use mpfree_splash::png;
use ratatui::Terminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const WAIT_FOR_FB: Duration = Duration::from_secs(60);

enum Mode {
    Terminal,
    Preview(PathBuf),
    Framebuffer,
}

struct Options {
    mode: Mode,
    seconds: f32,
    fps: f32,
    frames: usize,
    every: f32,
    rotation: Rotation,
    fb: PathBuf,
    sysfs: PathBuf,
}

const USAGE: &str = "usage: mpfree-splash [--seconds N] [--fps N]\n       \
    mpfree-splash --preview DIR [--frames N] [--every SECONDS] [--rotation 0|90|180|270]\n       \
    mpfree-splash --framebuffer [--fb DEVICE] [--sysfs DIR] [--rotation N] [--seconds N] [--fps N]";

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        mode: Mode::Terminal,
        seconds: 0.0,
        fps: 30.0,
        frames: 6,
        every: 1.5,
        // The MPC One's panel is mounted left side up (device tree rotation = 270).
        rotation: Rotation::Deg270,
        fb: PathBuf::from("/dev/fb0"),
        sysfs: PathBuf::from("/sys/class/graphics/fb0"),
    };
    let mut fps_given = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--framebuffer" {
            o.mode = Mode::Framebuffer;
            continue;
        }
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        let value = args.next().ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))?;
        let number = |what: &str| -> Result<f32, String> {
            value.parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| format!("{what} needs a number, not {value:?}"))
        };
        match arg.as_str() {
            "--seconds" => o.seconds = number("--seconds")?,
            "--fps" => {
                o.fps = number("--fps")?.clamp(1.0, 120.0);
                fps_given = true;
            }
            "--frames" => o.frames = number("--frames")? as usize,
            "--every" => o.every = number("--every")?,
            "--rotation" => {
                o.rotation = value.parse::<u32>().ok().and_then(Rotation::from_degrees)
                    .ok_or_else(|| format!("--rotation must be 0, 90, 180 or 270, not {value}"))?
            }
            "--fb" => o.fb = PathBuf::from(value),
            "--sysfs" => o.sysfs = PathBuf::from(value),
            "--preview" => o.mode = Mode::Preview(PathBuf::from(value)),
            _ => return Err(format!("unknown option {arg}\n{USAGE}")),
        }
    }
    if matches!(o.mode, Mode::Framebuffer) && !fps_given {
        o.fps = 20.0;
    }
    Ok(o)
}

fn run_terminal(o: &Options) -> Result<(), String> {
    let galaxy = Galaxy::new();
    let mut terminal = ratatui::init();
    let start = Instant::now();
    let frame = Duration::from_secs_f32(1.0 / o.fps);
    let result = (|| -> Result<(), String> {
        loop {
            let t = start.elapsed().as_secs_f32();
            if o.seconds > 0.0 && t >= o.seconds {
                return Ok(());
            }
            terminal.draw(|f| f.render_widget(GalaxyWidget { galaxy: &galaxy, seconds: t }, f.area()))
                .map_err(|e| e.to_string())?;
            if event::poll(frame).map_err(|e| e.to_string())?
                && let Event::Key(k) = event::read().map_err(|e| e.to_string())?
            {
                let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
                if k.kind == KeyEventKind::Press && (ctrl_c || matches!(k.code, KeyCode::Char('q') | KeyCode::Esc)) {
                    return Ok(());
                }
            }
        }
    })();
    ratatui::restore();
    result
}

fn run_preview(o: &Options, dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let galaxy = Galaxy::new();
    let g = fb::mpc_one();
    let backend = FramebufferBackend::new(g, o.rotation)?;
    let mut term = Terminal::new(backend).map_err(|e| e.to_string())?;
    let (lw, lh) = g.logical_size(o.rotation);
    let size = term.size().map_err(|e| e.to_string())?;
    println!("MPC One framebuffer {}x{} (stride {}), rotation {:?}: the viewer sees {lw}x{lh}, {}x{} cells",
             g.width, g.height, g.stride, o.rotation, size.width, size.height);
    for i in 0..o.frames.max(1) {
        let t = i as f32 * o.every;
        let started = Instant::now();
        term.draw(|f| f.render_widget(GalaxyWidget { galaxy: &galaxy, seconds: t }, f.area()))
            .map_err(|e| e.to_string())?;
        let took = started.elapsed();
        let b = term.backend();
        let mut pixels = Vec::with_capacity(lw * lh);
        for y in 0..lh {
            for x in 0..lw {
                pixels.push(b.logical_pixel(x, y));
            }
        }
        let path = dir.join(format!("mpc-one-splash-{i:02}-t{t:05.1}s.png"));
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        file.write_all(&png::encode(lw, lh, &pixels)).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {} (frame drawn in {:.1} ms on this machine)", path.display(), took.as_secs_f64() * 1000.0);
    }
    Ok(())
}

fn read_trimmed(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map(|s| s.trim().to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

/// The visible size from `modes` ("U:800x1280p-0"), and the stride and depth.
fn geometry_from_sysfs(dir: &Path) -> Result<Geometry, String> {
    let modes = read_trimmed(&dir.join("modes"))?;
    let first = modes.lines().next().unwrap_or("");
    let size = first.split(':').nth(1).unwrap_or("");
    let digits: String = size.chars().take_while(|c| c.is_ascii_digit() || *c == 'x').collect();
    let (w, h) = digits.split_once('x').ok_or_else(|| format!("cannot read a size from modes {first:?}"))?;
    let num = |s: &str, what: &str| s.parse::<usize>().map_err(|_| format!("bad {what} {s:?}"));
    let stride = num(&read_trimmed(&dir.join("stride"))?, "stride")?;
    let bpp = read_trimmed(&dir.join("bits_per_pixel"))?.parse::<u32>().map_err(|e| format!("bits_per_pixel: {e}"))?;
    Geometry::new(num(w, "width")?, num(h, "height")?, stride, bpp)
}

fn run_framebuffer(o: &Options) -> Result<(), String> {
    let start = Instant::now();
    while !(o.fb.exists() && o.sysfs.join("modes").exists()) {
        if start.elapsed() > WAIT_FOR_FB {
            return Err(format!("{} did not appear within {} s", o.fb.display(), WAIT_FOR_FB.as_secs()));
        }
        thread::sleep(Duration::from_millis(500));
    }
    let g = geometry_from_sysfs(&o.sysfs)?;
    let backend = FramebufferBackend::new(g, o.rotation)?;
    let mut term = Terminal::new(backend).map_err(|e| e.to_string())?;
    let size = term.size().map_err(|e| e.to_string())?;
    eprintln!("mpfree-splash: {}x{} pixels, stride {}, {:?}, rotation {:?}: {}x{} cells, {} fps",
              g.width, g.height, g.stride, g.format, o.rotation, size.width, size.height, o.fps);
    // Write only: no create, no truncate. Each frame goes to offset 0, the shown page.
    let mut fb = OpenOptions::new().write(true).open(&o.fb).map_err(|e| format!("{}: {e}", o.fb.display()))?;
    let galaxy = Galaxy::new();
    let frame = Duration::from_secs_f32(1.0 / o.fps);
    let begin = Instant::now();
    let mut shown = false;
    let (mut frames, mut busy, mut worst) = (0u64, Duration::ZERO, Duration::ZERO);
    let mut report = Instant::now();
    loop {
        let tick = Instant::now();
        let t = begin.elapsed().as_secs_f32();
        if o.seconds > 0.0 && t >= o.seconds {
            break;
        }
        term.draw(|f| f.render_widget(GalaxyWidget { galaxy: &galaxy, seconds: t }, f.area()))
            .map_err(|e| e.to_string())?;
        fb.seek(SeekFrom::Start(0)).and_then(|_| fb.write_all(term.backend().frame()))
            .map_err(|e| format!("writing {}: {e}", o.fb.display()))?;
        if !shown {
            show_framebuffer(&o.sysfs);
            shown = true;
        }
        let took = tick.elapsed();
        frames += 1;
        busy += took;
        worst = worst.max(took);
        if report.elapsed() >= Duration::from_secs(5) {
            eprintln!("mpfree-splash: {frames} frames, average {:.1} ms, worst {:.1} ms per frame",
                      busy.as_secs_f64() * 1000.0 / frames as f64, worst.as_secs_f64() * 1000.0);
            report = Instant::now();
        }
        if let Some(rest) = frame.checked_sub(took) {
            thread::sleep(rest);
        }
    }
    eprintln!("mpfree-splash: done: {frames} frames in {:.1} s, average {:.1} ms, worst {:.1} ms per frame",
              begin.elapsed().as_secs_f64(), busy.as_secs_f64() * 1000.0 / frames.max(1) as f64,
              worst.as_secs_f64() * 1000.0);
    Ok(())
}

fn show_framebuffer(sysfs: &Path) {
    // Writing the pan offset makes the kernel's framebuffer emulation commit its
    // mode, which lights the panel. Without a text console nothing else does.
    match OpenOptions::new().write(true).open(sysfs.join("pan")).and_then(|mut f| f.write_all(b"0,0\n")) {
        Ok(()) => eprintln!("mpfree-splash: asked the kernel to show the framebuffer (pan 0,0)"),
        Err(e) => eprintln!("mpfree-splash: pan 0,0 failed: {e}"),
    }
}

fn main() {
    let result = parse_args().and_then(|o| match &o.mode {
        Mode::Terminal => run_terminal(&o),
        Mode::Preview(dir) => run_preview(&o, dir),
        Mode::Framebuffer => run_framebuffer(&o),
    });
    if let Err(e) = result {
        eprintln!("mpfree-splash: {e}");
        std::process::exit(1);
    }
}
