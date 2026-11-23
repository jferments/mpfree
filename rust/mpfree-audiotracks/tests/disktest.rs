//! mpfree-disktest on a desktop folder: runs every step quickly, reports
//! each, and leaves nothing behind; refuses a folder holding files that
//! are not its own, and refuses to run anywhere but the stick without the
//! desktop flag. With `--read-only` it reads the audio under `library/`
//! and the files under `backup/` and changes nothing at all, never
//! following a link out of them. With `--under-load` it reads them the
//! same way while its load runs, writes only its own files, and leaves
//! nothing behind. With `--recorder` MPFree's own recorder writes its takes
//! beside the reads, every take saved whole, and the writeback limit (a
//! stand-in for `/proc/sys/vm` on a desktop) is raised for two steps and
//! put back as found.

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

/// The stand-in's settings, by name (what they say, not when they were written).
fn settings(vm: &Path) -> BTreeMap<String, String> {
    std::fs::read_dir(vm)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(e.path()).unwrap(),
            )
        })
        .collect()
}

fn writeback_stand_in(name: &str, bytes: &str) -> PathBuf {
    let vm = scratch(name);
    for (n, v) in [
        ("dirty_bytes", bytes),
        ("dirty_ratio", "0"),
        ("dirty_background_bytes", "51200"),
        ("dirty_background_ratio", "0"),
    ] {
        std::fs::write(vm.join(n), format!("{v}\n")).unwrap();
    }
    vm
}

fn library(d: &Path) {
    for i in 0..6 {
        noise(
            &d.join(format!("library/Samples/s{i}.wav")),
            (1 << 20) + i * 70_000,
            i as u32 + 1,
        );
    }
}

#[test]
fn a_recorder_run_saves_every_take_whole_and_puts_the_writeback_limit_back() {
    let d = scratch("rec");
    library(&d);
    std::fs::write(d.join("library/notes.txt"), b"keep me").unwrap();
    let vm = writeback_stand_in("rec-vm", "1048576");
    let before = snapshot(&d);
    let vm_before = settings(&vm);
    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .args(["--recorder", "--quick"])
        .env("MPFREE_DISKTEST_VM", &vm)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for want in [
        "recorder: MPFree's own recorder writes 8 takes in",
        "found: 6 audio files in library/",
        "writeback as found: at most 1.0 MiB waiting (dirty_bytes 1048576), written out from 50.0 KiB (dirty_background_bytes 51200)",
        "1. A round every 0.05 s, spread across the interval, writeback as found",
        "2. A round every 0.1 s, spread across the interval, writeback as found",
        "3. A round every 0.25 s, spread across the interval, writeback as found",
        "4. A round every 0.5 s, spread across the interval, writeback as found",
        "5. A round every 0.1 s, the takes together, writeback as found",
        "6. A round every 0.25 s, the takes together, writeback as found",
        "7. A round every 0.1 s, spread across the interval, writeback limit raised to 32 MiB",
        "8. A round every 0.25 s, spread across the interval, writeback limit raised to 32 MiB",
        "writeback limit raised to 32 MiB (dirty_bytes 33554432), until the last of these steps",
        "recording: 8 takes, stereo float at 48 kHz (3.07 MB/s in all)",
        "frames refused: none",
        "8 of 8 takes saved, 8 of them whole (every frame in the file)",
        "2 streams, 24-bit",
        "worst shortfall",
        "writeback limit put back as found: dirty_bytes 1048576",
        "Summary:",
        "the files read in library/ and backup/ are as they were",
        "the folder is deleted",
        "done",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    assert_eq!(
        text.matches("8 of 8 takes saved, 8 of them whole").count(),
        8,
        "{text}"
    );
    assert!(!text.contains("left out"), "{text}");
    assert!(!text.contains("NOT ALL FINISHED"), "{text}");
    for name in ["s0.wav", "notes.txt", "Samples"] {
        assert!(!text.contains(name), "{name} named in\n{text}");
    }
    // Its folder is gone, nothing else changed, and the limit is as found.
    assert_eq!(snapshot(&d), before);
    assert_eq!(settings(&vm), vm_before);
    std::fs::remove_dir_all(&d).unwrap();
    std::fs::remove_dir_all(&vm).unwrap();
}

#[test]
fn a_recorder_run_leaves_the_writeback_limit_alone_unless_it_can_raise_it() {
    let d = scratch("rec-alone");
    library(&d);
    let run = |vm: Option<&Path>| {
        let mut c = bin();
        c.arg("--desktop-test")
            .arg(&d)
            .args(["--recorder", "--quick"]);
        match vm {
            Some(vm) => c.env("MPFREE_DISKTEST_VM", vm),
            None => c.env_remove("MPFREE_DISKTEST_VM"),
        };
        let out = c.output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let left: Vec<String> = text
            .lines()
            .filter(|l| l.starts_with("  left out: "))
            .map(str::to_string)
            .collect();
        assert_eq!(left.len(), 2, "{text}");
        assert!(!text.contains("raised to 32 MiB (dirty_bytes"), "{text}");
        assert!(!d.join("mpfree-disktest").exists());
        (text, left)
    };
    // A desktop with no stand-in: the desktop's own settings are never read or written.
    let (text, left) = run(None);
    assert!(
        text.contains("writeback: left alone (a desktop test without a stand-in)"),
        "{text}"
    );
    assert!(
        left[0].contains("the writeback settings are left alone here"),
        "{left:?}"
    );
    // A limit counted as a share of memory, or one already 32 MiB or more.
    for (bytes, why) in [
        ("0", "a share of memory (dirty_ratio 0 %), not a size"),
        ("67108864", "already 64.0 MiB"),
    ] {
        let vm = writeback_stand_in("rec-alone-vm", bytes);
        let vm_before = snapshot(&vm);
        let (_, left) = run(Some(&vm));
        assert!(left[0].contains(why), "{left:?}");
        assert_eq!(snapshot(&vm), vm_before);
        std::fs::remove_dir_all(&vm).unwrap();
    }
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_cut_off_recorder_runs_takes_are_cleared_first_and_nothing_else() {
    let d = scratch("rec-left");
    library(&d);
    let mine = d.join("mpfree-disktest");
    std::fs::create_dir(&mine).unwrap();
    let id = "00112233445566778899aabbccddeeff";
    for ext in ["wav.recording", "wav.checkpoint", "wav"] {
        std::fs::write(mine.join(format!("rec3_{id}.{ext}")), b"x").unwrap();
    }
    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .args(["--recorder", "--quick"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!mine.exists());
    // A take of someone else's in its folder: left alone, nothing written.
    std::fs::create_dir(&mine).unwrap();
    std::fs::write(mine.join(format!("Vocal_{id}.wav")), b"keep me").unwrap();
    std::fs::write(mine.join(format!("rec1_{id}.wav")), b"x").unwrap();
    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .args(["--recorder", "--quick"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not the tool's"));
    assert_eq!(
        std::fs::read(mine.join(format!("Vocal_{id}.wav"))).unwrap(),
        b"keep me"
    );
    assert_eq!(
        std::fs::read(mine.join(format!("rec1_{id}.wav"))).unwrap(),
        b"x"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn only_one_kind_of_run_at_a_time() {
    for pair in [
        ["--recorder", "--under-load"],
        ["--recorder", "--read-only"],
        ["--read-only", "--under-load"],
    ] {
        let out = bin().args(pair).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{pair:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("are different runs"));
    }
}
