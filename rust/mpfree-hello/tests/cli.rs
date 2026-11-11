//! The mpfree-hello program, run whole on the host in its one host-only mode:
//! `--preview DIR` draws the console and every sample screen into new PNG
//! files and opens no device. (Its other modes need the MPC.)
//!
//! Each screen is also compared with a stored fingerprint, golden/screens.txt:
//! the mean brightness of every 20x20 cell, within 1 level. A screen that
//! changes on purpose gets new references in the same PR, with the reason in
//! the commit:
//!
//!     MPFREE_GOLDEN_BLESS=1 cargo test -p mpfree-hello --test cli

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Where the program appends its STOP line on the MPC; tests that end in a
/// STOP run only when it is absent, so they never write there.
const REPORT_DIR: &str = "/run/mpfree-hello";

/// Runs the program. In CI's ARMv7 step the tests run under qemu-arm (cargo's
/// target runner), so the ARM program is started through the same runner.
fn hello(args: &[&str]) -> Output {
    let exe = env!("CARGO_BIN_EXE_mpfree-hello");
    let runner = std::env::var("CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_RUNNER").ok();
    let mut cmd = match runner.filter(|_| cfg!(target_arch = "arm")) {
        Some(r) => {
            let mut c = Command::new(r);
            c.arg(exe);
            c
        }
        None => Command::new(exe),
    };
    cmd.args(args).output().expect("run mpfree-hello")
}

fn folder(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("mpfree-hello-cli")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Width and height from a PNG's IHDR, after checking the signature.
fn png_size(path: &Path) -> (u32, u32) {
    let b = std::fs::read(path).unwrap();
    assert_eq!(&b[..8], b"\x89PNG\r\n\x1a\n", "{}", path.display());
    assert_eq!(&b[12..16], b"IHDR", "{}", path.display());
    let be = |at: usize| u32::from_be_bytes(b[at..at + 4].try_into().unwrap());
    (be(16), be(20))
}

/// The RGB pixels of a PNG written by `mpfree_ui::png::encode_rgb`: 8-bit RGB,
/// no interlace, a zlib stream of stored (uncompressed) deflate blocks, and
/// filter type 0 on every row. Anything else is refused.
fn png_rgb(path: &Path) -> (usize, usize, Vec<u8>) {
    let b = std::fs::read(path).unwrap();
    let (w, h) = png_size(path);
    assert_eq!(
        &b[24..29],
        &[8, 2, 0, 0, 0],
        "{}: not 8-bit RGB",
        path.display()
    );
    let mut z = Vec::new();
    let mut at = 8;
    while at + 8 <= b.len() {
        let len = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        if &b[at + 4..at + 8] == b"IDAT" {
            z.extend_from_slice(&b[at + 8..at + 8 + len]);
        }
        at += 12 + len;
    }
    let mut raw = Vec::new();
    let mut i = 2; // the zlib header
    loop {
        let last = z[i] & 1 == 1;
        assert_eq!(z[i] & 6, 0, "{}: a compressed block", path.display());
        let len = u16::from_le_bytes([z[i + 1], z[i + 2]]) as usize;
        raw.extend_from_slice(&z[i + 5..i + 5 + len]);
        i += 5 + len;
        if last {
            break;
        }
    }
    let (w, h) = (w as usize, h as usize);
    assert_eq!(raw.len(), h * (1 + 3 * w), "{}", path.display());
    let mut rgb = Vec::with_capacity(w * h * 3);
    for row in raw.chunks_exact(1 + 3 * w) {
        assert_eq!(row[0], 0, "{}: filtered row", path.display());
        rgb.extend_from_slice(&row[1..]);
    }
    (w, h, rgb)
}

/// Cell size of a screen fingerprint, in pixels.
const CELL: usize = 20;
/// How far a cell's mean brightness may move (0 to 255) and still match.
const CELL_TOLERANCE: i32 = 1;

/// "WxH " and the mean brightness of every CELL x CELL cell, row by row, in hex.
fn screen_print(path: &Path) -> String {
    let (w, h, rgb) = png_rgb(path);
    let mut out = format!("{w}x{h} ");
    for cy in 0..h / CELL {
        for cx in 0..w / CELL {
            let mut sum = 0u64;
            for y in cy * CELL..(cy + 1) * CELL {
                for x in cx * CELL..(cx + 1) * CELL {
                    let p = &rgb[(y * w + x) * 3..(y * w + x) * 3 + 3];
                    sum += (299 * u64::from(p[0]) + 587 * u64::from(p[1]) + 114 * u64::from(p[2]))
                        / 1000;
                }
            }
            out.push_str(&format!("{:02x}", sum / (CELL * CELL) as u64));
        }
    }
    out
}

/// Why `got` differs from `want`, or None when every cell is within tolerance.
fn screen_differs(want: &str, got: &str) -> Option<String> {
    let (ws, wc) = want.split_once(' ')?;
    let (gs, gc) = got.split_once(' ')?;
    if ws != gs {
        return Some(format!("{gs}, not {ws}"));
    }
    let cells = |s: &str| -> Vec<i32> {
        s.as_bytes()
            .chunks(2)
            .map(|c| i32::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    };
    let (w, g) = (cells(wc), cells(gc));
    let off: Vec<usize> = (0..w.len().min(g.len()))
        .filter(|&i| (w[i] - g[i]).abs() > CELL_TOLERANCE)
        .collect();
    if w.len() != g.len() || !off.is_empty() {
        let cols = ws
            .split_once('x')
            .map(|(a, _)| a.parse::<usize>().unwrap() / CELL)
            .unwrap_or(1);
        let at: Vec<String> = off
            .iter()
            .take(5)
            .map(|&i| format!("({}, {})", (i % cols) * CELL, (i / cols) * CELL))
            .collect();
        return Some(format!(
            "{} of {} cells changed, first at pixels {}",
            off.len(),
            w.len(),
            at.join(" ")
        ));
    }
    None
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden/screens.txt")
}

/// Compares every PNG in `dir` with golden/screens.txt, or rewrites it when
/// MPFREE_GOLDEN_BLESS is set.
fn check_screens(dir: &Path, names: &[String]) {
    let got: Vec<(String, String)> = names
        .iter()
        .map(|n| {
            (
                n.trim_end_matches(".png").to_string(),
                screen_print(&dir.join(n)),
            )
        })
        .collect();
    if std::env::var_os("MPFREE_GOLDEN_BLESS").is_some() {
        let mut text = String::from(
            "# mpfree-hello --preview screens: WxH, then the mean brightness of each 20x20 cell,\n\
             # row by row, in hex. Written by MPFREE_GOLDEN_BLESS=1 cargo test -p mpfree-hello --test cli\n\
             # Compared within 1 level per cell (tests/cli.rs).\n",
        );
        for (n, p) in &got {
            text.push_str(&format!("{n} {p}\n"));
        }
        std::fs::create_dir_all(golden_path().parent().unwrap()).unwrap();
        std::fs::write(golden_path(), text).unwrap();
        eprintln!(
            "wrote {} screen fingerprints to {}",
            got.len(),
            golden_path().display()
        );
        return;
    }
    let text = std::fs::read_to_string(golden_path()).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (bless with MPFREE_GOLDEN_BLESS=1)",
            golden_path().display()
        )
    });
    let want: Vec<(String, String)> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            l.split_once(' ')
                .map(|(n, p)| (n.to_string(), p.to_string()))
        })
        .collect();
    let mut problems = Vec::new();
    for (n, p) in &got {
        match want.iter().find(|(w, _)| w == n) {
            None => problems.push(format!("{n}: new screen, no reference")),
            Some((_, w)) => {
                if let Some(why) = screen_differs(w, p) {
                    problems.push(format!("{n}: {why}"));
                }
            }
        }
    }
    for (n, _) in &want {
        if !got.iter().any(|(g, _)| g == n) {
            problems.push(format!("{n}: in the references but no longer drawn"));
        }
    }
    assert!(
        problems.is_empty(),
        "{} of {} screens look different from golden/screens.txt (pictures in {}):\n{}\n\
         If the change is meant, bless the new screens: MPFREE_GOLDEN_BLESS=1 cargo test -p mpfree-hello --test cli",
        problems.len(),
        got.len(),
        dir.display(),
        problems.join("\n")
    );
}

#[test]
fn screen_fingerprints_compare_within_tolerance() {
    let a = "40x20 1020";
    assert_eq!(screen_differs(a, a), None);
    assert_eq!(screen_differs(a, "40x20 1121"), None);
    assert!(screen_differs(a, "40x20 1220")
        .unwrap()
        .contains("1 of 2 cells changed, first at pixels (0, 0)"));
    assert!(screen_differs(a, "20x40 1020")
        .unwrap()
        .contains("20x40, not 40x20"));
}

#[test]
fn preview_draws_every_screen_as_a_new_png() {
    let dir = folder("all");
    let out = hello(&["--preview", dir.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    // The screens CI checks under qemu, at least.
    for want in [
        "console",
        "loading",
        "loading-failed",
        "play",
        "debug",
        "syslog",
        "prefs",
        "options",
        "guide",
        "touchlab",
        "calibrate",
        "prefs-touch",
    ] {
        assert!(
            names.contains(&format!("{want}.png")),
            "no {want}.png in {names:?}"
        );
    }
    for name in &names {
        assert!(name.ends_with(".png"), "{name}");
        assert!(
            stdout.contains(&format!("preview written: {}", dir.join(name).display())),
            "{name} not reported"
        );
        let (w, h) = png_size(&dir.join(name));
        // The MPC One's panel is 1280x800 the way the app draws it (landscape).
        assert!(
            (w, h) == (1280, 800) || (w, h) == (800, 1280),
            "{name}: {w}x{h}"
        );
    }
    // The references are drawn on x86-64, where CI and the workstations run
    // `cargo test`. The ARMv7 run under qemu checks the pictures are valid
    // PNGs of the right size (above); its floating point may round a few
    // edge pixels differently, so it is not held to x86-64's fingerprints.
    if cfg!(target_arch = "x86_64") {
        check_screens(&dir, &names);
    }
}

#[test]
fn preview_never_overwrites_a_picture() {
    if Path::new(REPORT_DIR).exists() {
        eprintln!("skipped: {REPORT_DIR} exists, and a STOP line would be appended there");
        return;
    }
    let dir = folder("again");
    assert!(hello(&["--preview", dir.to_str().unwrap()])
        .status
        .success());
    let before = std::fs::read(dir.join("console.png")).unwrap();
    let out = hello(&["--preview", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("STOP:") && stdout.contains("console.png"),
        "{stdout}"
    );
    assert_eq!(std::fs::read(dir.join("console.png")).unwrap(), before);
}
