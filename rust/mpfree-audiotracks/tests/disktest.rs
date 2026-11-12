//! mpfree-disktest on a desktop folder: runs every step quickly, reports
//! each, and leaves nothing behind; refuses a folder holding files that
//! are not its own, and refuses to run anywhere but the stick without the
//! desktop flag.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mpfree-disktest"))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-disktest-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_quick_run_reports_every_step_and_leaves_nothing() {
    let d = scratch("run");
    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .arg("--quick")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for want in [
        "free before:",
        "1. Sequential write",
        "2. Paced write",
        "3a. Sequential read",
        "3b. 8 streams",
        "4. Mixed",
        "fdatasync:",
        "furthest behind schedule",
        "the folder is deleted",
        "done",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
    std::fs::remove_dir(&d).unwrap();
}

#[test]
fn a_folder_with_other_files_is_left_alone() {
    let d = scratch("other");
    let mine = d.join("mpfree-disktest");
    std::fs::create_dir(&mine).unwrap();
    std::fs::write(mine.join("seq.bin"), b"x").unwrap();
    std::fs::write(mine.join("notes.txt"), b"keep me").unwrap();
    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .arg("--quick")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not the tool's"));
    assert_eq!(std::fs::read(mine.join("notes.txt")).unwrap(), b"keep me");
    assert_eq!(std::fs::read(mine.join("seq.bin")).unwrap(), b"x");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_leftover_folder_of_its_own_is_cleared_first() {
    let d = scratch("left");
    let mine = d.join("mpfree-disktest");
    std::fs::create_dir(&mine).unwrap();
    std::fs::write(mine.join("mix2.bin"), b"old").unwrap();
    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .arg("--quick")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!mine.exists());
    std::fs::remove_dir(&d).unwrap();
}

#[test]
fn without_the_desktop_flag_it_runs_only_on_the_stick() {
    // On a build machine there is no stick partition 4 at /run/mpfree-data
    // (or an MMC device is present): refused, nothing written.
    let out = bin().output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("refused"));
    let out = bin().arg("--quick").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
