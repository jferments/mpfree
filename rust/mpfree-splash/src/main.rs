//! MPFree startup screen: a spinning spiral galaxy with "MPCFree OS v0.1" in
//! its heart, drawn with ratatui, and optionally the boot menu under it.
//!
//! Usage:
//!   mpfree-splash [--seconds N] [--fps N] [MENU]
//!       Desktop preview in this terminal. Press q, Esc or Ctrl-C to quit.
//!       With --menu: Left/Right (or Up/Down) stand in for the data wheel,
//!       Enter or Space for PLAY.
//!   mpfree-splash --preview DIR [--frames N] [--every SECONDS] [--rotation 0|90|180|270] [MENU]
//!       Draws frames exactly as the MPC One's framebuffer would hold them and
//!       saves what the viewer would see as PNG files in DIR. Touches no device.
//!   mpfree-splash --framebuffer [--fb DEVICE] [--sysfs DIR] [--rotation N] [--seconds N] [--fps N] [MENU]
//!       On a screen with no text console: draws on /dev/fb0. It writes only
//!       to the framebuffer device and to the framebuffer's sysfs `pan` file
//!       (which asks the kernel to show the framebuffer; nothing is stored).
//!       It never writes storage. Stops after --seconds (0 = until killed).
//!
//! MENU: --menu [--no-mpfree] [--menu-seconds N] [--midi CARD,DEVICE,SUBDEVICE | --midi-file PATH]
//!       Shows the boot menu. It prints "choice=mpfree" and exits 0, or
//!       prints "choice=factory" and exits 3. Any other exit means no choice
//!       was made; the boot script must then start the factory OS.
//!       --no-mpfree: the MPFree stick is missing or failed its check.
//!       --midi: reads the internal controller from ALSA RawMIDI, read only.
//!       --midi-file: reads raw MIDI bytes from a file or pipe (for tests).
//!
//! Every mode takes --font PATH: draw the title with this TrueType/OpenType
//! file instead of the built-in block letters. If the file can't be used,
//! the program says why and carries on with the block letters. --tracking N
//! adds N cap heights between letters (for fonts that crowd them).
#![deny(unsafe_code)]

mod midi_in;

use mpfree_controller::midi::Parser;
use mpfree_controller::mpc_one::{self, ControlEvent};
use mpfree_splash::fb::{self, FramebufferBackend, Geometry, Rotation};
use mpfree_splash::galaxy::{Galaxy, GalaxyWidget};
use mpfree_splash::menu::{BootMenu, Choice};
use mpfree_splash::png;
use mpfree_splash::title::TitleFont;
use ratatui::Terminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const WAIT_FOR_FB: Duration = Duration::from_secs(60);
/// Exit status for "start the factory OS".
const EXIT_FACTORY: i32 = 3;

enum Mode {
    Terminal,
    Preview(PathBuf),
    Framebuffer,
}

enum MidiSource {
    Alsa {
        card: u32,
        device: u32,
        subdevice: u32,
    },
    File(PathBuf),
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
    menu: bool,
    mpfree_available: bool,
    menu_seconds: f32,
    midi: Option<MidiSource>,
    font: Option<PathBuf>,
    tracking: f32,
}

const USAGE: &str = "usage: mpfree-splash [--seconds N] [--fps N] [MENU]\n       \
    mpfree-splash --preview DIR [--frames N] [--every SECONDS] [--rotation 0|90|180|270] [MENU]\n       \
    mpfree-splash --framebuffer [--fb DEVICE] [--sysfs DIR] [--rotation N] [--seconds N] [--fps N] [MENU]\n\
    MENU: --menu [--no-mpfree] [--menu-seconds N] [--midi CARD,DEVICE,SUBDEVICE | --midi-file PATH]\n\
    Any mode: --font FILE.ttf|FILE.otf [--tracking N] (title typeface; default: built-in block letters;\n\
    N: extra letter spacing in cap heights, e.g. 0.2)";

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
        menu: false,
        mpfree_available: true,
        menu_seconds: 5.0,
        midi: None,
        font: None,
        tracking: 0.0,
    };
    let mut fps_given = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--framebuffer" => {
                o.mode = Mode::Framebuffer;
                continue;
            }
            "--menu" => {
                o.menu = true;
                continue;
            }
            "--no-mpfree" => {
                o.mpfree_available = false;
                continue;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => {}
        }
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
            "--seconds" => o.seconds = number("--seconds")?,
            "--fps" => {
                o.fps = number("--fps")?.clamp(1.0, 120.0);
                fps_given = true;
            }
            "--frames" => o.frames = number("--frames")? as usize,
            "--every" => o.every = number("--every")?,
            "--menu-seconds" => o.menu_seconds = number("--menu-seconds")?.clamp(1.0, 120.0),
            "--rotation" => {
                o.rotation = value
                    .parse::<u32>()
                    .ok()
                    .and_then(Rotation::from_degrees)
                    .ok_or_else(|| format!("--rotation must be 0, 90, 180 or 270, not {value}"))?
            }
            "--fb" => o.fb = PathBuf::from(value),
            "--sysfs" => o.sysfs = PathBuf::from(value),
            "--preview" => o.mode = Mode::Preview(PathBuf::from(value)),
            "--midi" => {
                let parts: Vec<u32> = value
                    .split(',')
                    .map(|p| p.trim().parse::<u32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| {
                        format!("--midi needs CARD,DEVICE,SUBDEVICE numbers, not {value:?}")
                    })?;
                let [card, device, subdevice] = parts[..] else {
                    return Err(format!("--midi needs CARD,DEVICE,SUBDEVICE, not {value:?}"));
                };
                o.midi = Some(MidiSource::Alsa {
                    card,
                    device,
                    subdevice,
                });
            }
            "--midi-file" => o.midi = Some(MidiSource::File(PathBuf::from(value))),
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
    if matches!(o.mode, Mode::Framebuffer) && !fps_given {
        o.fps = 20.0;
    }
    Ok(o)
}

/// The scene, with the title in the font asked for. A font that can't be
/// used never stops the startup screen: it falls back to the block letters.
fn make_galaxy(o: &Options) -> Galaxy {
    let font = match &o.font {
        None => TitleFont::Block,
        Some(path) => TitleFont::load(path)
            .map(|f| f.with_tracking(o.tracking))
            .unwrap_or_else(|e| {
                eprintln!("mpfree-splash: {e}; using the built-in block letters");
                TitleFont::Block
            }),
    };
    eprintln!("mpfree-splash: title font: {}", font.name());
    Galaxy::with_font(font)
}

/// Menu input, from any source.
enum Input {
    Wheel(i32),
    Confirm,
}

/// Reads the controller in a thread and sends wheel turns and PLAY/pad presses.
fn start_midi(source: &MidiSource) -> Result<mpsc::Receiver<Input>, String> {
    let mut reader: Box<dyn Read + Send> = match source {
        MidiSource::Alsa {
            card,
            device,
            subdevice,
        } => Box::new(midi_in::open_rawmidi(*card, *device, *subdevice)?),
        MidiSource::File(path) => {
            Box::new(fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?)
        }
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut parser = Parser::new();
        let mut buf = [0u8; 256];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            for &b in &buf[..n] {
                let Some(msg) = parser.feed(b) else { continue };
                let input = match mpc_one::decode(&msg) {
                    Some(ControlEvent::DataWheel { delta }) => Input::Wheel(delta as i32),
                    Some(ControlEvent::PadDown { .. }) => Input::Confirm,
                    Some(ControlEvent::Button {
                        note: mpc_one::button::PLAY,
                        pressed: true,
                    }) => Input::Confirm,
                    _ => continue,
                };
                if tx.send(input).is_err() {
                    return;
                }
            }
        }
    });
    Ok(rx)
}

/// Prints the choice for the boot script and exits with its status.
fn finish(choice: Choice) -> ! {
    match choice {
        Choice::MpFree => {
            println!("choice=mpfree");
            std::process::exit(0)
        }
        Choice::Factory => {
            println!("choice=factory");
            std::process::exit(EXIT_FACTORY)
        }
    }
}

fn run_terminal(o: &Options) -> Result<Option<Choice>, String> {
    let galaxy = make_galaxy(o);
    let mut terminal = ratatui::init();
    let start = Instant::now();
    let frame = Duration::from_secs_f32(1.0 / o.fps);
    let mut menu = o
        .menu
        .then(|| BootMenu::new(o.mpfree_available, o.menu_seconds, 0.0));
    let midi = match (&o.midi, o.menu) {
        (Some(source), true) => Some(start_midi(source)),
        _ => None,
    };
    let midi = match midi.transpose() {
        Ok(m) => m,
        Err(e) => {
            ratatui::restore();
            return Err(e);
        }
    };
    let result = (|| -> Result<Option<Choice>, String> {
        loop {
            let t = start.elapsed().as_secs_f32();
            if let Some(m) = menu.as_mut() {
                if let Some(rx) = &midi {
                    while let Ok(input) = rx.try_recv() {
                        match input {
                            Input::Wheel(d) => m.wheel(d, t),
                            Input::Confirm => m.confirm(),
                        }
                    }
                }
                if let Some(choice) = m.tick(t) {
                    let text = m.lines(t);
                    terminal
                        .draw(|f| {
                            f.render_widget(
                                GalaxyWidget {
                                    galaxy: &galaxy,
                                    seconds: t,
                                    menu: Some(&text),
                                },
                                f.area(),
                            )
                        })
                        .map_err(|e| e.to_string())?;
                    thread::sleep(Duration::from_millis(600));
                    return Ok(Some(choice));
                }
            } else if o.seconds > 0.0 && t >= o.seconds {
                return Ok(None);
            }
            let text = menu.as_ref().map(|m| m.lines(t));
            terminal
                .draw(|f| {
                    f.render_widget(
                        GalaxyWidget {
                            galaxy: &galaxy,
                            seconds: t,
                            menu: text.as_ref(),
                        },
                        f.area(),
                    )
                })
                .map_err(|e| e.to_string())?;
            if event::poll(frame).map_err(|e| e.to_string())?
                && let Event::Key(k) = event::read().map_err(|e| e.to_string())?
                && k.kind == KeyEventKind::Press
            {
                let ctrl_c =
                    k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
                if ctrl_c || matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                    return Ok(None);
                }
                if let Some(m) = menu.as_mut() {
                    let t = start.elapsed().as_secs_f32();
                    match k.code {
                        KeyCode::Left | KeyCode::Up => m.wheel(-1, t),
                        KeyCode::Right | KeyCode::Down => m.wheel(1, t),
                        KeyCode::Enter | KeyCode::Char(' ') => m.confirm(),
                        _ => {}
                    }
                }
            }
        }
    })();
    ratatui::restore();
    result
}

fn run_preview(o: &Options, dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let galaxy = make_galaxy(o);
    let g = fb::mpc_one();
    let backend = FramebufferBackend::new(g, o.rotation)?;
    let mut term = Terminal::new(backend).map_err(|e| e.to_string())?;
    let (lw, lh) = g.logical_size(o.rotation);
    let size = term.size().map_err(|e| e.to_string())?;
    println!(
        "MPC One framebuffer {}x{} (stride {}), rotation {:?}: the viewer sees {lw}x{lh}, {}x{} cells",
        g.width, g.height, g.stride, o.rotation, size.width, size.height
    );
    // With --menu, the frames walk through the menu: countdown, wheel turned to Factory OS.
    let mut menu = o
        .menu
        .then(|| BootMenu::new(o.mpfree_available, o.menu_seconds.max(60.0), 0.0));
    for i in 0..o.frames.max(1) {
        let t = i as f32 * o.every;
        if let Some(m) = menu.as_mut()
            && i == o.frames.max(1) / 2
        {
            m.wheel(1, t);
        }
        let text = menu.as_ref().map(|m| m.lines(t));
        let started = Instant::now();
        term.draw(|f| {
            f.render_widget(
                GalaxyWidget {
                    galaxy: &galaxy,
                    seconds: t,
                    menu: text.as_ref(),
                },
                f.area(),
            )
        })
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
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        file.write_all(&png::encode(lw, lh, &pixels))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        println!(
            "wrote {} (frame drawn in {:.1} ms on this machine)",
            path.display(),
            took.as_secs_f64() * 1000.0
        );
    }
    Ok(())
}

fn read_trimmed(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The visible size from `modes` ("U:800x1280p-0"), and the stride and depth.
fn geometry_from_sysfs(dir: &Path) -> Result<Geometry, String> {
    let modes = read_trimmed(&dir.join("modes"))?;
    let first = modes.lines().next().unwrap_or("");
    let size = first.split(':').nth(1).unwrap_or("");
    let digits: String = size
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == 'x')
        .collect();
    let (w, h) = digits
        .split_once('x')
        .ok_or_else(|| format!("cannot read a size from modes {first:?}"))?;
    let num = |s: &str, what: &str| s.parse::<usize>().map_err(|_| format!("bad {what} {s:?}"));
    let stride = num(&read_trimmed(&dir.join("stride"))?, "stride")?;
    let bpp = read_trimmed(&dir.join("bits_per_pixel"))?
        .parse::<u32>()
        .map_err(|e| format!("bits_per_pixel: {e}"))?;
    Geometry::new(num(w, "width")?, num(h, "height")?, stride, bpp)
}

fn run_framebuffer(o: &Options) -> Result<Option<Choice>, String> {
    let start = Instant::now();
    while !(o.fb.exists() && o.sysfs.join("modes").exists()) {
        if start.elapsed() > WAIT_FOR_FB {
            return Err(format!(
                "{} did not appear within {} s",
                o.fb.display(),
                WAIT_FOR_FB.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(500));
    }
    let g = geometry_from_sysfs(&o.sysfs)?;
    let backend = FramebufferBackend::new(g, o.rotation)?;
    let mut term = Terminal::new(backend).map_err(|e| e.to_string())?;
    let size = term.size().map_err(|e| e.to_string())?;
    eprintln!(
        "mpfree-splash: {}x{} pixels, stride {}, {:?}, rotation {:?}: {}x{} cells, {} fps",
        g.width, g.height, g.stride, g.format, o.rotation, size.width, size.height, o.fps
    );
    // Write only: no create, no truncate. Each frame goes to offset 0, the shown page.
    let mut fbdev = OpenOptions::new()
        .write(true)
        .open(&o.fb)
        .map_err(|e| format!("{}: {e}", o.fb.display()))?;
    let galaxy = make_galaxy(o);
    let frame = Duration::from_secs_f32(1.0 / o.fps);
    let mut menu = o
        .menu
        .then(|| BootMenu::new(o.mpfree_available, o.menu_seconds, 0.0));
    let midi = match (&o.midi, o.menu) {
        (Some(source), true) => Some(start_midi(source)?),
        (None, true) if o.mpfree_available => {
            eprintln!("mpfree-splash: no --midi input: the menu can only run its countdown");
            None
        }
        _ => None,
    };
    let begin = Instant::now();
    let mut shown = false;
    let (mut frames, mut busy, mut worst) = (0u64, Duration::ZERO, Duration::ZERO);
    let mut report = Instant::now();
    let mut chosen: Option<(Choice, Instant)> = None;
    loop {
        let tick = Instant::now();
        let t = begin.elapsed().as_secs_f32();
        if let Some(m) = menu.as_mut() {
            if let Some(rx) = &midi {
                while let Ok(input) = rx.try_recv() {
                    match input {
                        Input::Wheel(d) => m.wheel(d, t),
                        Input::Confirm => m.confirm(),
                    }
                }
            }
            if let Some(choice) = m.tick(t) {
                match chosen {
                    // Keep "Starting ..." on screen briefly before handing over.
                    Some((_, at)) if at.elapsed() >= Duration::from_millis(600) => {
                        return Ok(Some(choice));
                    }
                    None => chosen = Some((choice, Instant::now())),
                    _ => {}
                }
            }
        } else if o.seconds > 0.0 && t >= o.seconds {
            break;
        }
        let text = menu.as_ref().map(|m| m.lines(t));
        term.draw(|f| {
            f.render_widget(
                GalaxyWidget {
                    galaxy: &galaxy,
                    seconds: t,
                    menu: text.as_ref(),
                },
                f.area(),
            )
        })
        .map_err(|e| e.to_string())?;
        fbdev
            .seek(SeekFrom::Start(0))
            .and_then(|_| fbdev.write_all(term.backend().frame()))
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
            eprintln!(
                "mpfree-splash: {frames} frames, average {:.1} ms, worst {:.1} ms per frame",
                busy.as_secs_f64() * 1000.0 / frames as f64,
                worst.as_secs_f64() * 1000.0
            );
            report = Instant::now();
        }
        if let Some(rest) = frame.checked_sub(took) {
            thread::sleep(rest);
        }
    }
    eprintln!(
        "mpfree-splash: done: {frames} frames in {:.1} s, average {:.1} ms, worst {:.1} ms per frame",
        begin.elapsed().as_secs_f64(),
        busy.as_secs_f64() * 1000.0 / frames.max(1) as f64,
        worst.as_secs_f64() * 1000.0
    );
    Ok(None)
}

fn show_framebuffer(sysfs: &Path) {
    // Writing the pan offset makes the kernel's framebuffer emulation commit its
    // mode, which lights the panel. Without a text console nothing else does.
    match OpenOptions::new()
        .write(true)
        .open(sysfs.join("pan"))
        .and_then(|mut f| f.write_all(b"0,0\n"))
    {
        Ok(()) => eprintln!("mpfree-splash: asked the kernel to show the framebuffer (pan 0,0)"),
        Err(e) => eprintln!("mpfree-splash: pan 0,0 failed: {e}"),
    }
}

fn main() {
    let result = parse_args().and_then(|o| match &o.mode {
        Mode::Terminal => run_terminal(&o),
        Mode::Preview(dir) => run_preview(&o, dir).map(|_| None),
        Mode::Framebuffer => run_framebuffer(&o),
    });
    match result {
        Ok(Some(choice)) => finish(choice),
        Ok(None) => {}
        Err(e) => {
            eprintln!("mpfree-splash: {e}");
            std::process::exit(1);
        }
    }
}
