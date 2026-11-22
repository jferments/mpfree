//! mpfree-disktest on a desktop folder: runs every step quickly, reports
//! each, and leaves nothing behind; refuses a folder holding files that
//! are not its own, and refuses to run anywhere but the stick without the
//! desktop flag. With `--read-only` it reads the audio under `library/`
//! and the files under `backup/` and changes nothing at all, never
//! following a link out of them. With `--under-load` it reads them the
//! same way while its load runs, writes only its own files, and leaves
//! nothing behind.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
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
    let out = bin().arg("--read-only").output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Nothing was written"));
    let out = bin().args(["--read-only", "--quick"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = bin().arg("--under-load").output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Nothing was written"));
    let out = bin()
        .args(["--read-only", "--under-load"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// Every entry under `dir`: its kind, length, change time and bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (String, u64, i64, i64, Vec<u8>)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let m = std::fs::symlink_metadata(&p).unwrap();
            let (kind, bytes) = if m.file_type().is_symlink() {
                (
                    "link",
                    std::fs::read_link(&p)
                        .unwrap()
                        .into_os_string()
                        .into_encoded_bytes(),
                )
            } else if m.is_dir() {
                stack.push(p.clone());
                ("dir", Vec::new())
            } else {
                ("file", std::fs::read(&p).unwrap())
            };
            out.insert(
                p,
                (kind.to_string(), m.len(), m.mtime(), m.mtime_nsec(), bytes),
            );
        }
    }
    out
}

fn noise(path: &Path, len: usize, seed: u32) {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    let bytes: Vec<u8> = (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_read_only_run_reads_the_library_and_changes_nothing() {
    let d = scratch("read");
    let lib = d.join("library");
    for i in 0..6 {
        noise(
            &lib.join(format!("Samples/kit/s{i}.wav")),
            (1 << 20) + i * 70_000,
            i as u32 + 1,
        );
    }
    noise(&lib.join("Projects/song/take.WAV"), 3 << 20, 9);
    // A backup image: read too, in pieces (1 MiB in a quick run).
    noise(
        &d.join("backup/mpc-one-emmc-x/mmcblk0.img"),
        (5 << 20) + 4096,
        11,
    );
    std::fs::write(d.join("backup/mpc-one-emmc-x/SHA256SUMS"), b"short").unwrap();
    // Left alone: not in library/ or backup/.
    noise(&d.join("loose.wav"), 2 << 20, 12);
    noise(&lib.join("Samples/tiny.wav"), 1000, 10);
    std::fs::write(lib.join("notes.txt"), b"keep me").unwrap();
    // Audio outside the library, reached only through links: never read.
    let outside = scratch("read-outside");
    for i in 0..4 {
        noise(&outside.join(format!("big{i}.wav")), 2 << 20, 20 + i);
    }
    std::os::unix::fs::symlink(&outside, lib.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(outside.join("big0.wav"), lib.join("link.wav")).unwrap();
    let before = snapshot(&d);
    let outside_before = snapshot(&outside);

    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .args(["--read-only", "--quick"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for want in [
        "read-only: reads the audio under",
        "found: 7 audio files in library/ (10.0 MiB), 1 file in backup/ (5.0 MiB); the largest 5.0 MiB; 20 pieces to read",
        "left out: 2 under 64 KiB, 2 links, 0 on another file system",
        "1. Sequential read of the largest pieces",
        "2. Streams, 0.4 s each",
        "2 streams, 24-bit",
        "4 streams, 16-bit",
        "4 streams, float",
        "p99.9 ",
        "p99.99 ",
        "worst shortfall",
        "longest behind",
        "the plan's window (2 x shortfall + 1 s, at least 4 s): 4.0 s a stream",
        "Summary",
        "the files read are as they were: nothing was written",
        "done",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    // No file names in the report.
    for name in [
        "s0.wav",
        "take.WAV",
        "notes.txt",
        "big0.wav",
        "kit",
        "mmcblk0",
        "loose",
    ] {
        assert!(!text.contains(name), "{name} named in\n{text}");
    }
    assert_eq!(snapshot(&d), before);
    assert_eq!(snapshot(&outside), outside_before);
    assert!(!d.join("mpfree-disktest").exists());
    std::fs::remove_dir_all(&d).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn a_read_only_run_with_too_little_audio_is_refused() {
    let d = scratch("read-little");
    let run = |d: &Path| {
        bin()
            .arg("--desktop-test")
            .arg(d)
            .args(["--read-only", "--quick"])
            .output()
            .unwrap()
    };
    // No library folder.
    let out = run(&d);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Nothing was written"));
    // One file: too few for two streams.
    noise(&d.join("library/one.wav"), 1 << 20, 1);
    let out = run(&d);
    assert_eq!(out.status.code(), Some(3));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("too little to read"), "{err}");
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn an_under_load_run_reports_every_step_and_leaves_nothing() {
    let d = scratch("load");
    let lib = d.join("library");
    for i in 0..6 {
        noise(
            &lib.join(format!("Samples/kit/s{i}.wav")),
            (1 << 20) + i * 70_000,
            i as u32 + 1,
        );
    }
    noise(
        &d.join("backup/mpc-one-emmc-x/mmcblk0.img"),
        (5 << 20) + 4096,
        11,
    );
    std::fs::write(lib.join("notes.txt"), b"keep me").unwrap();
    let before = snapshot(&d);

    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .args(["--under-load", "--quick"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for want in [
        "under load: reads the audio under",
        "found: 6 audio files in library/",
        "files: the recording up to 3 MiB, the heavy write up to 4 MiB",
        "1. Nothing else, with the recording",
        "2. A heavy write, with the recording",
        "3. A heavy read, with the recording",
        "4. A heavy write and a heavy read, with the recording",
        "5. A busy processor, with the recording",
        "6. Heavy task switching, with the recording",
        "7. A busy processor and heavy task switching, with the recording",
        "8. Everything at once, with the recording",
        "load: CPUs busy ",
        "task switches a second",
        "a 1 ms sleep woke late by:",
        "2 streams, 24-bit",
        "4 streams, 24-bit",
        "worst shortfall",
        "recording (3.10 MB/s, fdatasync every second)",
        "heavy write: ",
        "heavy read: ",
        "busy threads: ",
        "switching threads: 8 pairs",
        "Summary:",
        "the files read in library/ and backup/ are as they were",
        "the folder is deleted",
        "done",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    assert!(!text.contains("left out"), "{text}");
    for name in ["s0.wav", "notes.txt", "kit", "mmcblk0"] {
        assert!(!text.contains(name), "{name} named in\n{text}");
    }
    // Its folder is gone and nothing else changed.
    assert_eq!(snapshot(&d), before);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn under_load_the_load_steps_are_watched_for_heat() {
    let d = scratch("load-hot");
    for i in 0..6 {
        noise(
            &d.join(format!("library/s{i}.wav")),
            (1 << 20) + i * 70_000,
            i as u32 + 1,
        );
    }
    let zones = scratch("load-zones");
    let z = zones.join("thermal_zone0");
    std::fs::create_dir_all(&z).unwrap();
    std::fs::write(z.join("type"), "cpu-thermal\n").unwrap();
    std::fs::write(z.join("trip_point_0_temp"), "70000\n").unwrap();
    std::fs::write(z.join("trip_point_0_type"), "passive\n").unwrap();
    let run = |temp: &str| {
        std::fs::write(z.join("temp"), temp).unwrap();
        let out = bin()
            .arg("--desktop-test")
            .arg(&d)
            .args(["--under-load", "--quick"])
            .env("MPFREE_DISKTEST_THERMAL", &zones)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            text.contains("temperature: cpu-thermal now ")
                && text.contains("trip points: passive 70 °C"),
            "{text}"
        );
        assert!(
            text.contains("the busy and switching threads stop at 70 °C"),
            "{text}"
        );
        text
    };
    // Cool: every step runs, the temperature is reported.
    let text = run("45000\n");
    assert!(!text.contains("left out"), "{text}");
    assert!(text.contains("45.0 °C at the most"), "{text}");
    // Hot: the steps that heat the processor are left out, the rest run.
    let text = run("80000\n");
    assert!(
        text.contains("left out: the MPC is at 80.0 °C, at or over the stop at 70 °C"),
        "{text}"
    );
    let left: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("  left out: "))
        .collect();
    assert_eq!(left.len(), 4, "{text}");
    assert!(text.contains("heavy write: "), "{text}");
    assert!(!text.contains("busy threads: "), "{text}");
    assert!(!d.join("mpfree-disktest").exists());
    std::fs::remove_dir_all(&d).unwrap();
    std::fs::remove_dir_all(&zones).unwrap();
}
