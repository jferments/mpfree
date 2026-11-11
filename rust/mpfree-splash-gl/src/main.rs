//! MPFree startup screen, SDL3 + OpenGL ES 3.0 version: the same galaxy and
//! title as the character-cell `mpfree-splash`, at full resolution with
//! textured arms, dust lanes, star-forming knots, a nebula sky and a glowing
//! chrome title.
//!
//! Usage:
//!   mpfree-splash-gl [--fullscreen] [--size WxH] [--seconds N]
//!       A window (default 1280x800, the MPC One's screen in landscape).
//!       Esc, q or closing the window quits.
//!   mpfree-splash-gl --screenshot DIR [--size WxH] [--frames N] [--every S]
//!       Renders frames offscreen and saves them as PNG files in DIR (new
//!       files only). Works headless with SDL_VIDEO_DRIVER=offscreen.

mod render;
mod shaders;

use mpfree_splash::title::TitleFont;
use render::Renderer;
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use sdl3::video::GLProfile;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Options {
    fullscreen: bool,
    size: (u32, u32),
    seconds: f32,
    screenshot: Option<PathBuf>,
    frames: usize,
    every: f32,
    font: Option<PathBuf>,
    tracking: f32,
}

const USAGE: &str = "usage: mpfree-splash-gl [--fullscreen] [--size WxH] [--seconds N] [--font FILE [--tracking N]]\n       \
    mpfree-splash-gl --screenshot DIR [--size WxH] [--frames N] [--every S] [--font FILE [--tracking N]]\n\
    --font: the title's TrueType/OpenType font (default: built-in block letters)\n\
    --tracking N: extra letter spacing for --font, in cap heights (e.g. 0.2)";

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        fullscreen: false,
        size: (1280, 800),
        seconds: 0.0,
        screenshot: None,
        frames: 4,
        every: 3.0,
        font: None,
        tracking: 0.0,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fullscreen" => o.fullscreen = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))?;
                let number = |what: &str| -> Result<f32, String> {
                    value
                        .parse::<f32>()
                        .ok()
                        .filter(|v| v.is_finite() && *v >= 0.0)
                        .ok_or_else(|| format!("{what} needs a number, not {value:?}"))
                };
                match arg.as_str() {
                    "--size" => {
                        let (w, h) = value
                            .split_once('x')
                            .ok_or("--size needs WxH, like 1280x800")?;
                        let w = w.parse::<u32>().map_err(|_| "bad --size width")?;
                        let h = h.parse::<u32>().map_err(|_| "bad --size height")?;
                        if !(64..=8192).contains(&w) || !(64..=8192).contains(&h) {
                            return Err("--size must be between 64 and 8192 on each side".into());
                        }
                        o.size = (w, h);
                    }
                    "--seconds" => o.seconds = number("--seconds")?,
                    "--screenshot" => o.screenshot = Some(PathBuf::from(value)),
                    "--frames" => o.frames = number("--frames")? as usize,
                    "--every" => o.every = number("--every")?,
                    "--font" => o.font = Some(PathBuf::from(value)),
                    "--tracking" => {
                        o.tracking = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .ok_or_else(|| format!("--tracking needs a number, not {value:?}"))?
                    }
                    _ => return Err(format!("unknown option {arg}\n{USAGE}")),
                }
            }
        }
    }
    Ok(o)
}

fn run(o: &Options) -> Result<(), String> {
    let sdl = sdl3::init().map_err(|e| e.to_string())?;
    let video = sdl.video().map_err(|e| e.to_string())?;
    let attr = video.gl_attr();
    attr.set_context_profile(GLProfile::GLES);
    attr.set_context_version(3, 0);
    attr.set_double_buffer(true);
    let mut builder = video.window("MPFree", o.size.0, o.size.1);
    builder.opengl().resizable();
    if o.fullscreen {
        builder.fullscreen();
    }
    if o.screenshot.is_some() {
        builder.hidden();
    }
    let window = builder.build().map_err(|e| e.to_string())?;
    let context = window.gl_create_context().map_err(|e| e.to_string())?;
    window
        .gl_make_current(&context)
        .map_err(|e| e.to_string())?;
    let gl = unsafe {
        glow::Context::from_loader_function(|name| {
            video
                .gl_get_proc_address(name)
                .map_or(std::ptr::null(), |f| f as *const std::os::raw::c_void)
        })
    };
    {
        use glow::HasContext;
        let (version, renderer) = unsafe {
            (
                gl.get_parameter_string(glow::VERSION),
                gl.get_parameter_string(glow::RENDERER),
            )
        };
        eprintln!(
            "mpfree-splash-gl: {version} on {renderer} (driver {})",
            video.current_video_driver()
        );
    }
    let started = Instant::now();
    let (pw, ph) = window.size_in_pixels();
    let font = match &o.font {
        None => TitleFont::Block,
        Some(path) => TitleFont::load(path)
            .map(|f| f.with_tracking(o.tracking))
            .unwrap_or_else(|e| {
                eprintln!("mpfree-splash-gl: {e}; using the built-in block letters");
                TitleFont::Block
            }),
    };
    eprintln!("mpfree-splash-gl: title font: {}", font.name());
    let mut renderer = Renderer::new(gl, pw as i32, ph as i32, font)?;
    renderer.finish();
    eprintln!(
        "mpfree-splash-gl: textures ready in {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );

    if let Some(dir) = &o.screenshot {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for i in 0..o.frames.max(1) {
            let t = i as f32 * o.every;
            let px = renderer.snapshot(o.size.0 as i32, o.size.1 as i32, t)?;
            let path = dir.join(format!(
                "splash-gl-{}x{}-{i:02}-t{t:05.1}s.png",
                o.size.0, o.size.1
            ));
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path);
            use std::io::Write;
            file.and_then(|mut f| {
                f.write_all(&mpfree_splash::png::encode(
                    o.size.0 as usize,
                    o.size.1 as usize,
                    &px,
                ))
            })
            .map_err(|e| format!("{}: {e}", path.display()))?;
            println!("wrote {}", path.display());
        }
        return Ok(());
    }

    // Vsync if the driver offers it; otherwise the frame pacing below.
    let _ = video.gl_set_swap_interval(sdl3::video::SwapInterval::VSync);
    let mut events = sdl.event_pump().map_err(|e| e.to_string())?;
    let begin = Instant::now();
    let (mut frames, mut report) = (0u64, Instant::now());
    let mut worst = Duration::ZERO;
    loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. } => return Ok(()),
                Event::KeyDown {
                    keycode: Some(Keycode::Escape | Keycode::Q),
                    ..
                } => return Ok(()),
                Event::Window {
                    win_event: WindowEvent::PixelSizeChanged(w, h),
                    ..
                } => renderer.resize(w, h)?,
                _ => {}
            }
        }
        let t = begin.elapsed().as_secs_f32();
        if o.seconds > 0.0 && t >= o.seconds {
            break;
        }
        let tick = Instant::now();
        renderer.draw(t);
        window.gl_swap_window();
        worst = worst.max(tick.elapsed());
        frames += 1;
        if report.elapsed() >= Duration::from_secs(5) {
            eprintln!(
                "mpfree-splash-gl: {:.1} fps, worst frame {:.1} ms",
                frames as f64 / begin.elapsed().as_secs_f64(),
                worst.as_secs_f64() * 1000.0
            );
            report = Instant::now();
        }
    }
    eprintln!(
        "mpfree-splash-gl: done: {frames} frames, {:.1} fps",
        frames as f64 / begin.elapsed().as_secs_f64()
    );
    Ok(())
}

fn main() {
    if let Err(e) = parse_args().and_then(|o| run(&o)) {
        eprintln!("mpfree-splash-gl: {e}");
        std::process::exit(1);
    }
}
