//! MPFree kexec test boot: show the live log on the MPC's screen.
//!
//! Reads log lines on stdin (the test boot pipes `journalctl -f` into it) and
//! draws the newest of them on /dev/fb0, the kernel's framebuffer, about four
//! times a second. It writes only to the framebuffer device and to the
//! framebuffer's sysfs `pan` file, which asks the kernel to show the
//! framebuffer (a mode set; nothing is stored). It never writes storage.
//!
//! Usage: mpfree-screen-log [--rotation 0|90|180|270] [--fb DEVICE] [--sysfs DIR] [--title TEXT]
//!        mpfree-screen-log --preview FILE.ppm [--rotation N]   (host only: draw a sample, no device)
#![forbid(unsafe_code)]

use mpfree_screen_log::{Geometry, LogView, Rotation, grid, render};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_TITLE: &str = "MPFree test boot from the USB stick. Switch off to return to the stock firmware.";
const WAIT_FOR_FB: Duration = Duration::from_secs(60);
const REDRAW: Duration = Duration::from_millis(250);
const STATUS_EVERY: Duration = Duration::from_secs(1);

struct Options {
    rotation: Rotation,
    fb: PathBuf,
    sysfs: PathBuf,
    title: String,
    preview: Option<PathBuf>,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        rotation: Rotation::Deg0,
        fb: PathBuf::from("/dev/fb0"),
        sysfs: PathBuf::from("/sys/class/graphics/fb0"),
        title: DEFAULT_TITLE.to_string(),
        preview: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            "--rotation" => {
                o.rotation = value.parse::<u32>().ok().and_then(Rotation::from_degrees)
                    .ok_or_else(|| format!("--rotation must be 0, 90, 180 or 270, not {value}"))?
            }
            "--fb" => o.fb = PathBuf::from(value),
            "--sysfs" => o.sysfs = PathBuf::from(value),
            "--title" => o.title = value,
            "--preview" => o.preview = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    Ok(o)
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

fn status_line() -> String {
    let uptime = fs::read_to_string("/proc/uptime").ok()
        .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()))
        .map(|v| format!("up {v:.0} s"))
        .unwrap_or_else(|| "up ? s".to_string());
    let mut links = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        let mut names: Vec<String> = entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "lo").collect();
        names.sort();
        for name in names {
            let state = read_trimmed(&Path::new("/sys/class/net").join(&name).join("operstate"))
                .unwrap_or_else(|_| "?".to_string());
            links.push(format!("{name} {state}"));
        }
    }
    format!("{uptime} | network: {}", if links.is_empty() { "none".to_string() } else { links.join(", ") })
}

fn preview(o: &Options) -> Result<(), String> {
    // The MPC One's framebuffer as the kernel reports it: 800x1280, 32 bits, stride 3200.
    let g = Geometry::new(800, 1280, 3200, 32)?;
    let (cols, rows) = grid(&g, o.rotation);
    let mut view = LogView::new(cols, rows - 2);
    for i in 0..60 {
        view.push(&format!("[{:>5}.{:06}] sample line {i}: the quick brown fox jumps over the lazy dog, \
                            and keeps going past the edge of the screen", i, i * 1234));
    }
    let mut buf = vec![0u8; g.frame_len()];
    render(&g, o.rotation, &o.title, "up 42 s | network: eth0 up", &view, &mut buf)?;
    // A PPM of the native panel, as the panel itself is addressed (portrait).
    let mut out = format!("P6\n{} {}\n255\n", g.width, g.height).into_bytes();
    for y in 0..g.height {
        for x in 0..g.width {
            let p = &buf[y * g.stride + x * 4..y * g.stride + x * 4 + 3];
            out.extend_from_slice(&[p[2], p[1], p[0]]);
        }
    }
    let path = o.preview.as_ref().unwrap();
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(&out).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("preview written: {} ({cols} columns, {rows} rows)", path.display());
    Ok(())
}

fn show_framebuffer(sysfs: &Path) {
    // Writing the pan offset makes the kernel's framebuffer emulation commit its
    // mode, which lights the panel. Without a text console nothing else does.
    match OpenOptions::new().write(true).open(sysfs.join("pan")).and_then(|mut f| f.write_all(b"0,0\n")) {
        Ok(()) => eprintln!("mpfree-screen-log: asked the kernel to show the framebuffer (pan 0,0)"),
        Err(e) => eprintln!("mpfree-screen-log: pan 0,0 failed: {e}"),
    }
    if let Ok(entries) = fs::read_dir("/sys/class/backlight") {
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            let get = |n: &str| read_trimmed(&p.join(n)).unwrap_or_else(|_| "?".to_string());
            eprintln!("mpfree-screen-log: backlight {}: brightness {} actual {} bl_power {}",
                      entry.file_name().to_string_lossy(), get("brightness"), get("actual_brightness"),
                      get("bl_power"));
        }
    }
}

fn run(o: &Options) -> Result<(), String> {
    let start = Instant::now();
    while !(o.fb.exists() && o.sysfs.join("modes").exists()) {
        if start.elapsed() > WAIT_FOR_FB {
            return Err(format!("{} did not appear within {} s", o.fb.display(), WAIT_FOR_FB.as_secs()));
        }
        thread::sleep(Duration::from_millis(500));
    }
    let g = geometry_from_sysfs(&o.sysfs)?;
    let (cols, rows) = grid(&g, o.rotation);
    eprintln!("mpfree-screen-log: {}x{} pixels, stride {}, {:?}, rotation {:?}: {cols} columns, {rows} rows",
              g.width, g.height, g.stride, g.format, o.rotation);
    // Write only: no create, no truncate. Each frame goes to offset 0, the shown page.
    let mut fb = OpenOptions::new().write(true).open(&o.fb).map_err(|e| format!("{}: {e}", o.fb.display()))?;
    let mut buf = vec![0u8; g.frame_len()];
    let mut view = LogView::new(cols, rows.saturating_sub(2));

    let (tx, rx) = mpsc::channel::<Option<String>>();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => if tx.send(Some(l)).is_err() { return },
                Err(_) => break,
            }
        }
        let _ = tx.send(None);
    });

    let mut shown = false;
    let mut ended = false;
    let mut dirty = true;
    let mut last = Instant::now() - STATUS_EVERY;
    loop {
        match rx.recv_timeout(REDRAW) {
            Ok(Some(line)) => {
                view.push(&line);
                dirty = true;
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !ended {
                    view.push("(the log input ended; the service restarts it)");
                    ended = true;
                    dirty = true;
                } else {
                    thread::sleep(Duration::from_millis(20));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if (dirty && last.elapsed() >= REDRAW) || last.elapsed() >= STATUS_EVERY {
            render(&g, o.rotation, &o.title, &status_line(), &view, &mut buf)?;
            fb.seek(SeekFrom::Start(0)).and_then(|_| fb.write_all(&buf))
                .map_err(|e| format!("writing {}: {e}", o.fb.display()))?;
            if !shown {
                show_framebuffer(&o.sysfs);
                shown = true;
            }
            dirty = false;
            last = Instant::now();
            if ended {
                return Ok(());
            }
        }
    }
}

fn main() {
    let result = parse_args().and_then(|o| if o.preview.is_some() { preview(&o) } else { run(&o) });
    if let Err(e) = result {
        eprintln!("mpfree-screen-log: {e}");
        std::process::exit(1);
    }
}
