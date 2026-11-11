//! The mpfree-display program, run as a whole on the host: the self-test
//! report, the three preview pictures, and that it never overwrites a file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Output};

fn display(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpfree-display"))
        .args(args)
        .output()
        .expect("run mpfree-display")
}

fn report(out: &Output) -> HashMap<String, String> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().next(), Some("MPFREE_OFFSCREEN_TEST_CARD_V1"));
    stdout
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn folder(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("mpfree-display-cli")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn self_test_is_the_default_and_names_both_sizes() {
    let named = report(&display(&["--self-test"]));
    assert_eq!(report(&display(&[])), named);
    assert_eq!(named["hardware_access"], "none");
    assert_eq!(
        (named["width"].as_str(), named["height"].as_str()),
        ("1280", "800")
    );
    assert_eq!(
        (
            named["native_width"].as_str(),
            named["native_height"].as_str()
        ),
        ("800", "1280")
    );
    assert_eq!(named["status"], "PASS");
}

#[test]
fn previews_are_ppms_of_the_logical_and_both_panel_turns() {
    let dir = folder("previews");
    let logical_sum = report(&display(&["--self-test"]))["checksum_fnv1a64"].clone();
    let mut pictures = Vec::new();
    for (flag, w, h) in [
        ("--preview-logical", 1280, 800),
        ("--preview-cw", 800, 1280),
        ("--preview-ccw", 800, 1280),
    ] {
        let path = dir.join(format!("{}.ppm", &flag[2..]));
        let r = report(&display(&[flag, path.to_str().unwrap()]));
        assert_eq!(
            (r["width"].clone(), r["height"].clone()),
            (w.to_string(), h.to_string()),
            "{flag}"
        );
        if flag == "--preview-logical" {
            assert_eq!(r["checksum_fnv1a64"], logical_sum);
        }
        let bytes = std::fs::read(&path).unwrap();
        let header = format!("P6\n{w} {h}\n255\n");
        assert!(bytes.starts_with(header.as_bytes()), "{flag}");
        assert_eq!(bytes.len(), header.len() + w * h * 3, "{flag}");
        pictures.push(bytes);
    }
    // The two turns are different pictures of the same card.
    assert_ne!(pictures[1], pictures[2]);
}

#[test]
fn a_preview_never_overwrites_a_file() {
    let dir = folder("taken");
    let path = dir.join("taken.ppm");
    std::fs::write(&path, b"keep me").unwrap();
    let out = display(&["--preview-cw", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot create new preview"));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}

#[test]
fn unknown_arguments_print_the_usage() {
    for args in [
        vec!["--preview"],
        vec!["--preview-cw"],
        vec!["--self-test", "x"],
    ] {
        let out = display(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("usage: mpfree-display"));
    }
}
