//! The mpfree-screen-log program, run as a whole on the host with no device:
//! the --preview picture, its argument checks, and the live loop drawing
//! into a framebuffer that is a plain file with a fake sysfs folder beside it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn screen_log(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpfree-screen-log"))
        .args(args)
        .output()
        .expect("run mpfree-screen-log")
}

/// A fresh, empty folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("mpfree-screen-log-cli")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn preview_draws_the_panel_as_a_portrait_ppm() {
    for rotation in ["0", "90", "180", "270"] {
        let dir = folder(&format!("preview-{rotation}"));
        let ppm = dir.join("preview.ppm");
        let out = screen_log(&["--preview", ppm.to_str().unwrap(), "--rotation", rotation]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("preview written"));
        let bytes = std::fs::read(&ppm).unwrap();
        let header = b"P6\n800 1280\n255\n";
        assert_eq!(&bytes[..header.len()], header);
        let pixels = &bytes[header.len()..];
        assert_eq!(pixels.len(), 800 * 1280 * 3);
        // Text was drawn: the picture is not one colour.
        assert!(
            pixels.chunks_exact(3).any(|p| p != &pixels[..3]),
            "rotation {rotation}: blank picture"
        );
    }
}

#[test]
fn preview_never_overwrites_a_file() {
    let dir = folder("no-overwrite");
    let ppm = dir.join("taken.ppm");
    std::fs::write(&ppm, b"keep me").unwrap();
    let out = screen_log(&["--preview", ppm.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(std::fs::read(&ppm).unwrap(), b"keep me");
}

#[test]
fn bad_arguments_are_refused() {
    for (args, message) in [
        (
            vec!["--rotation", "45"],
            "--rotation must be 0, 90, 180 or 270, not 45",
        ),
        (vec!["--rotation"], "--rotation needs a value"),
        (vec!["--colour", "red"], "unknown option --colour"),
    ] {
        let out = screen_log(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }
}

/// A fake fb0: the framebuffer is a file of the right size, and the sysfs
/// folder holds the files the program reads, plus `pan`, which it writes.
fn fake_fb(name: &str, modes: &str) -> (PathBuf, PathBuf) {
    let dir = folder(name);
    let fb = dir.join("fb0");
    std::fs::write(&fb, vec![0u8; 800 * 1280 * 4]).unwrap();
    let sysfs = dir.join("sysfs");
    std::fs::create_dir(&sysfs).unwrap();
    std::fs::write(sysfs.join("modes"), modes).unwrap();
    std::fs::write(sysfs.join("stride"), "3200\n").unwrap();
    std::fs::write(sysfs.join("bits_per_pixel"), "32\n").unwrap();
    std::fs::write(sysfs.join("pan"), "").unwrap();
    (fb, sysfs)
}

fn run_with_log(fb: &Path, sysfs: &Path, log: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mpfree-screen-log"))
        .args([
            "--fb",
            fb.to_str().unwrap(),
            "--sysfs",
            sysfs.to_str().unwrap(),
            "--rotation",
            "90",
            "--title",
            "test boot",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start mpfree-screen-log");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(log.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn live_loop_draws_the_log_then_stops_when_the_input_ends() {
    let (fb, sysfs) = fake_fb("live", "U:800x1280p-0\n");
    let out = run_with_log(&fb, &sysfs, "first line\nsecond line\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("800x1280 pixels, stride 3200"), "{stderr}");
    assert!(
        stderr.contains("asked the kernel to show the framebuffer (pan 0,0)"),
        "{stderr}"
    );
    assert_eq!(std::fs::read_to_string(sysfs.join("pan")).unwrap(), "0,0\n");
    let frame = std::fs::read(&fb).unwrap();
    assert_eq!(
        frame.len(),
        800 * 1280 * 4,
        "the framebuffer file changed size"
    );
    assert!(frame.iter().any(|&b| b != 0), "nothing was drawn");
}

#[test]
fn live_loop_refuses_a_mode_it_cannot_read() {
    let (fb, sysfs) = fake_fb("bad-modes", "garbage\n");
    let out = run_with_log(&fb, &sysfs, "");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read a size from modes"));
    assert!(
        std::fs::read(&fb).unwrap().iter().all(|&b| b == 0),
        "drew despite a bad mode"
    );
}
