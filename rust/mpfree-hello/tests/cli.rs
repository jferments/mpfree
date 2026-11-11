//! The mpfree-hello program, run whole on the host in its one host-only mode:
//! `--preview DIR` draws the console and every sample screen into new PNG
//! files and opens no device. (Its other modes need the MPC.)

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Where the program appends its STOP line on the MPC; tests that end in a
/// STOP run only when it is absent, so they never write there.
const REPORT_DIR: &str = "/run/mpfree-hello";

fn hello(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpfree-hello"))
        .args(args)
        .output()
        .expect("run mpfree-hello")
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
