//! `mpfree-disktest --sd-card` on a desktop stand-in: a thread here plays
//! the stick's two SD card windows (the read-only one, S1, and the write
//! one, S2a) through their request, alive, close and status files, as
//! `setup-sd-card.sh` and `setup-sd-card-write.sh` answer them. A quick run
//! reads the card's own files without changing one, runs every write step,
//! keeps the windows' counters moving, closes both windows and leaves its
//! folders deleted; a write window that refuses, a card that isn't there,
//! a close that isn't clean and a stranger's file in the test's folder each
//! stop it the way the stick's run would.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mpfree-disktest"))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-sdtest-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for sub in [
        "run/mpfree-app",
        "run/mpfree-setup",
        "run/mpfree-sd",
        "run/mpfree-data",
    ] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    d
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

/// Every entry under `dir` but `skip`: its kind, length, change time and
/// bytes.
fn snapshot(dir: &Path, skip: &Path) -> BTreeMap<PathBuf, (bool, u64, i64, i64, Vec<u8>)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p == skip {
                continue;
            }
            let m = std::fs::symlink_metadata(&p).unwrap();
            let bytes = if m.is_dir() {
                stack.push(p.clone());
                Vec::new()
            } else {
                std::fs::read(&p).unwrap()
            };
            out.insert(p, (m.is_dir(), m.len(), m.mtime(), m.mtime_nsec(), bytes));
        }
    }
    out
}

/// How the stand-in answers the write window's request.
#[derive(Clone, Copy, PartialEq)]
enum Write {
    /// Open, and close saying the card checks clean.
    Open,
    /// Refused, as for a marked card.
    Refuse,
    /// Open, but closing without a clean check.
    Unclean,
    /// A marked card: refused, unless the request's second line asks for a
    /// repair; then repaired and opened.
    Marked,
}

/// How the stand-in answers the read-only window's request.
#[derive(Clone, Copy, PartialEq)]
enum ReadOnly {
    Open,
    NoCard,
    /// No card for this many requests, then the card.
    LateCard(u32),
}

struct Window {
    request: &'static str,
    close: &'static str,
    alive: &'static str,
    status: &'static str,
    name: &'static str,
}

const READ: Window = Window {
    request: "sd-card.request",
    close: "sd-card.close",
    alive: "sd-card.alive",
    status: "sd-card-status",
    name: "read",
};

const WRITE: Window = Window {
    request: "sd-card-write.request",
    close: "sd-card-write.close",
    alive: "sd-card-write.alive",
    status: "sd-card-write-status",
    name: "write",
};

/// The stick's services, played by a thread until dropped: what each did,
/// in order.
struct Services {
    on: Arc<AtomicBool>,
    t: Option<std::thread::JoinHandle<Vec<String>>>,
}

impl Services {
    fn start(root: &Path, read: ReadOnly, write: Write) -> Services {
        let on = Arc::new(AtomicBool::new(true));
        let flag = on.clone();
        let root = root.to_path_buf();
        let t = std::thread::spawn(move || {
            let app = root.join("run/mpfree-app");
            let setup = root.join("run/mpfree-setup");
            let mut log = Vec::new();
            // Each window: the word it's open for, and the counter then.
            let mut open: [Option<(String, String)>; 2] = [None, None];
            let mut reads = 0;
            let status = |w: &Window, lines: &str| {
                // Like the windows: they make their status folder as they start.
                std::fs::create_dir_all(&setup).unwrap();
                let tmp = setup.join(format!("{}.new", w.status));
                std::fs::write(&tmp, lines).unwrap();
                std::fs::rename(&tmp, setup.join(w.status)).unwrap();
            };
            while flag.load(Ordering::Relaxed) {
                for (i, w) in [&READ, &WRITE].into_iter().enumerate() {
                    let req = app.join(w.request);
                    if let Ok(text) = std::fs::read_to_string(&req) {
                        let ask: String = text
                            .chars()
                            .take(32)
                            .filter(|c| c.is_ascii_hexdigit())
                            .collect();
                        let repair = text.lines().nth(1) == Some("repair");
                        if i == 1 {
                            log.push(format!(
                                "write asked{}",
                                if repair { " with repair" } else { "" }
                            ));
                        }
                        let _ = std::fs::remove_file(&req);
                        let _ = std::fs::remove_file(app.join(w.close));
                        if i == 0 {
                            reads += 1;
                        }
                        let refused = (i == 0
                            && match read {
                                ReadOnly::NoCard => true,
                                ReadOnly::LateCard(n) => reads <= n,
                                ReadOnly::Open => false,
                            })
                            || (i == 1 && write == Write::Refuse)
                            || (i == 1 && write == Write::Marked && !repair);
                        if refused {
                            let (state, msg) = if i == 0 {
                                ("none", "There is no card in the SD slot.")
                            } else {
                                ("refused", "the card wasn't safely removed last time (its exFAT is marked in use): check it on a computer first, or ask for a repair in the MPC")
                            };
                            status(w, &format!("state={state}\nmessage={msg}\nask={ask}\n"));
                            log.push(format!("{} {state}", w.name));
                            continue;
                        }
                        if i == 1 {
                            std::fs::create_dir_all(root.join("run/mpfree-sd/MPFree")).unwrap();
                        }
                        let alive = std::fs::read_to_string(app.join(w.alive)).unwrap_or_default();
                        let repaired = if i == 1 && write == Write::Marked {
                            "repaired=the card wasn't safely removed last time (its exFAT is marked in use)\n"
                        } else {
                            ""
                        };
                        status(
                            w,
                            &format!(
                                "state=open\nmessage=The SD card is open.\nask={ask}\ncard=mmcblk1\nfs=exfat\nsectors=62521344\nfsck=exfatprogs 1.2.2\n{repaired}"
                            ),
                        );
                        log.push(format!("{} open", w.name));
                        open[i] = Some((ask, alive));
                    }
                    if open[i].is_some() && app.join(w.close).exists() {
                        let (ask, alive) = open[i].take().unwrap();
                        let _ = std::fs::remove_file(app.join(w.close));
                        let now = std::fs::read_to_string(app.join(w.alive)).unwrap_or_default();
                        if now != alive {
                            log.push(format!("{} alive moved", w.name));
                        }
                        let msg = match (i, write) {
                            (0, _) => "Closed. Nothing was written to the card.",
                            (_, Write::Unclean) => "The card was taken out while open for writing: check it on a computer before the next save.",
                            _ => "Closed cleanly: 3 MiB written; the card checks clean.",
                        };
                        status(w, &format!("state=closed\nmessage={msg}\nask={ask}\n"));
                        log.push(format!("{} closed", w.name));
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            log
        });
        Services { on, t: Some(t) }
    }

    fn log(mut self) -> Vec<String> {
        self.on.store(false, Ordering::Relaxed);
        self.t.take().unwrap().join().unwrap()
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        self.on.store(false, Ordering::Relaxed);
        if let Some(t) = self.t.take() {
            let _ = t.join();
        }
    }
}

/// The card's own files: a few big enough to read, a small one, a folder.
fn card_files(card: &Path) {
    for i in 0..3 {
        noise(
            &card.join(format!("Samples/kit{i}.wav")),
            (1 << 20) + i * 50_000,
            i as u32 + 7,
        );
    }
    noise(&card.join("notes.txt"), 300, 3);
    std::fs::create_dir_all(card.join("Empty")).unwrap();
}

fn run(root: &Path) -> (Option<i32>, String, String) {
    run_with(root, &[])
}

fn run_with(root: &Path, more: &[&str]) -> (Option<i32>, String, String) {
    let out = bin()
        .arg("--desktop-test")
        .arg(root)
        .args(["--sd-card", "--quick"])
        .args(more)
        .env_remove("MPFREE_DISKTEST_THERMAL")
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_quick_sd_card_run_reports_every_step_and_leaves_nothing() {
    let d = scratch("run");
    let card = d.join("run/mpfree-sd");
    card_files(&card);
    let before = snapshot(&card, &card.join("MPFree"));
    let svc = Services::start(&d, ReadOnly::Open, Write::Open);
    let (code, text, err) = run(&d);
    let log = svc.log();
    assert_eq!(code, Some(0), "{text}\n{err}");
    for want in [
        "== Part 1: the SD card read-only",
        "asked for the read-only window",
        "the card holds 2 folders and 4 files",
        "3 files of 64 KiB or more",
        "S1. Sequential read of the card's own files: 3 files",
        "S2. 8 streams read the card's own files",
        "closed: Closed. Nothing was written to the card.",
        "== Part 2: the SD card written in its MPFree folder",
        "asked for the write window",
        "free before: SD ",
        "-- Idle --",
        "1. SD: sequential write",
        "2. SD: sequential read",
        "3. USB: sequential write",
        "4. USB: sequential read",
        "5. SD: 8 streams playing",
        "6. SD: recording",
        "7. SD: 4 streams recording and 4 playing",
        "8. Copy from USB to SD",
        "9. Copy from SD to USB",
        "10. SD and USB written at once",
        "11. SD and USB read at once",
        "12. 8 streams playing from USB while recording to SD",
        "13. 8 streams playing from SD while recording to USB",
        "14. Saving 4 files of 64 KiB to SD",
        "fdatasync:",
        "furthest behind schedule",
        "no temperature to read: the busy-core steps are left out",
        "the test folders are deleted",
        "closed: Closed cleanly: 3 MiB written; the card checks clean.",
        "SUMMARY",
        "14. saving files to SD",
        "(left out)",
        "done",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    // Never a name of the card's own.
    for name in ["kit0", "Samples", "notes.txt"] {
        assert!(!text.contains(name), "{name} printed in\n{text}");
    }
    assert_eq!(
        log,
        [
            "read open",
            "read alive moved",
            "read closed",
            "write asked",
            "write open",
            "write alive moved",
            "write closed"
        ],
        "{text}"
    );
    // The card's own files as they were; its MPFree folder (the window's)
    // and the stick's partition empty again.
    assert_eq!(snapshot(&card, &card.join("MPFree")), before);
    assert_eq!(std::fs::read_dir(card.join("MPFree")).unwrap().count(), 0);
    assert_eq!(
        std::fs::read_dir(d.join("run/mpfree-data"))
            .unwrap()
            .count(),
        0
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn with_temperatures_the_steps_run_again_with_busy_cores() {
    let d = scratch("busy");
    card_files(&d.join("run/mpfree-sd"));
    let z = d.join("thermal/thermal_zone0");
    std::fs::create_dir_all(&z).unwrap();
    std::fs::write(z.join("type"), "cpu-thermal\n").unwrap();
    std::fs::write(z.join("temp"), "45000\n").unwrap();
    std::fs::write(z.join("trip_point_0_temp"), "70000\n").unwrap();
    std::fs::write(z.join("trip_point_0_type"), "passive\n").unwrap();
    let svc = Services::start(&d, ReadOnly::Open, Write::Open);
    let out = bin()
        .arg("--desktop-test")
        .arg(&d)
        .args(["--sd-card", "--quick"])
        .env("MPFREE_DISKTEST_THERMAL", d.join("thermal"))
        .output()
        .unwrap();
    drop(svc);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("-- The same steps with a busy thread on each of the"),
        "{text}"
    );
    assert!(text.contains("(they stop at 70 °C)"), "{text}");
    assert!(
        text.contains("hottest during these steps: 45.0 °C"),
        "{text}"
    );
    assert_eq!(text.matches("\n14. Saving").count(), 2, "{text}");
    assert!(!text.contains("(left out)"), "{text}");
    assert_eq!(
        std::fs::read_dir(d.join("run/mpfree-data"))
            .unwrap()
            .count(),
        0
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_refused_write_window_writes_nothing() {
    let d = scratch("refused");
    let card = d.join("run/mpfree-sd");
    card_files(&card);
    let before = snapshot(&card, &card.join("MPFree"));
    let svc = Services::start(&d, ReadOnly::Open, Write::Refuse);
    let (code, text, err) = run(&d);
    let log = svc.log();
    assert_eq!(code, Some(3), "{text}\n{err}");
    assert!(
        err.contains(
            "refused: the card can't be written: the card wasn't safely removed last time"
        ),
        "{err}"
    );
    assert!(err.contains("(--repair-card asks for it)"), "{err}");
    assert!(
        text.contains("S2. 8 streams read the card's own files"),
        "{text}"
    );
    assert!(!text.contains("1. SD: sequential write"), "{text}");
    assert_eq!(
        log,
        [
            "read open",
            "read alive moved",
            "read closed",
            "write asked",
            "write refused"
        ]
    );
    assert_eq!(snapshot(&card, &card.join("MPFree")), before);
    assert!(!card.join("MPFree").exists());
    assert_eq!(
        std::fs::read_dir(d.join("run/mpfree-data"))
            .unwrap()
            .count(),
        0
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn without_a_card_nothing_is_asked_of_the_write_window() {
    let d = scratch("nocard");
    // The windows make their status folder as they start.
    std::fs::remove_dir(d.join("run/mpfree-setup")).unwrap();
    let svc = Services::start(&d, ReadOnly::NoCard, Write::Open);
    let (code, text, err) = run(&d);
    let log = svc.log();
    assert_eq!(code, Some(3), "{text}\n{err}");
    assert!(
        err.contains("refused: there is no card in the SD slot"),
        "{err}"
    );
    assert_eq!(log, ["read none"]);
    assert!(!d.join("run/mpfree-app/sd-card-write.request").exists());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn with_card_wait_a_card_put_in_late_is_waited_for() {
    let d = scratch("late");
    card_files(&d.join("run/mpfree-sd"));
    let svc = Services::start(&d, ReadOnly::LateCard(1), Write::Open);
    let (code, text, err) = run_with(&d, &["--card-wait", "60"]);
    let log = svc.log();
    assert_eq!(code, Some(0), "{text}\n{err}");
    assert_eq!(
        text.matches("no card in the SD slot yet: put it in now (asking again in 10 s, for up to ")
            .count(),
        1,
        "{text}"
    );
    assert_eq!(
        log[..3],
        ["read none", "read open", "read alive moved"],
        "{log:?}"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn with_repair_card_a_marked_card_is_repaired_in_the_mpc_then_written() {
    let d = scratch("repair");
    let card = d.join("run/mpfree-sd");
    card_files(&card);
    let before = snapshot(&card, &card.join("MPFree"));
    let svc = Services::start(&d, ReadOnly::Open, Write::Marked);
    let (code, text, err) = run_with(&d, &["--repair-card"]);
    let log = svc.log();
    assert_eq!(code, Some(0), "{text}\n{err}");
    assert_eq!(
        log,
        [
            "read open",
            "read alive moved",
            "read closed",
            "write asked with repair",
            "write open",
            "write alive moved",
            "write closed"
        ],
        "{text}"
    );
    assert!(
        text.contains("asked for the write window (")
            && text.contains(", and a repair if the card needs one)"),
        "{text}"
    );
    assert!(
        text.contains("repaired in the MPC first (fsck.exfat -y, then checked again: clean, the mark clear); it had: the card wasn't safely removed last time"),
        "{text}"
    );
    assert!(
        text.contains("14. Saving 4 files of 64 KiB to SD"),
        "{text}"
    );
    assert_eq!(snapshot(&card, &card.join("MPFree")), before);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn without_repair_card_no_repair_is_asked() {
    let d = scratch("norepair");
    card_files(&d.join("run/mpfree-sd"));
    let svc = Services::start(&d, ReadOnly::Open, Write::Marked);
    let (code, text, err) = run(&d);
    let log = svc.log();
    assert_eq!(code, Some(3), "{text}\n{err}");
    assert_eq!(log[3..], ["write asked", "write refused"], "{log:?}");
    assert!(!text.contains("and a repair"), "{text}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn repair_card_goes_only_with_the_sd_card_run() {
    let out = bin().args(["--repair-card"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--repair-card goes with --sd-card"));
    let out = bin().args(["--heat", "--repair-card"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn card_wait_goes_only_with_the_sd_card_run_and_has_a_limit() {
    for args in [
        &["--card-wait", "10"][..],
        &["--sd-card", "--card-wait", "3601"][..],
        &["--sd-card", "--card-wait", "soon"][..],
        &["--sd-card", "--card-wait"][..],
    ] {
        let out = bin().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn a_write_window_that_does_not_close_clean_fails_the_run() {
    let d = scratch("unclean");
    card_files(&d.join("run/mpfree-sd"));
    let svc = Services::start(&d, ReadOnly::Open, Write::Unclean);
    let (code, text, err) = run(&d);
    drop(svc);
    assert_eq!(code, Some(1), "{text}\n{err}");
    assert!(text.contains("SUMMARY"), "{text}");
    assert!(
        err.contains("failed: the write window: The card was taken out while open for writing"),
        "{err}"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_strangers_file_in_the_test_folder_is_left_alone() {
    let d = scratch("stranger");
    card_files(&d.join("run/mpfree-sd"));
    let theirs = d.join("run/mpfree-sd/MPFree/mpfree-disktest/mine.wav");
    noise(&theirs, 1000, 9);
    let svc = Services::start(&d, ReadOnly::Open, Write::Open);
    let (code, text, err) = run(&d);
    let log = svc.log();
    assert_eq!(code, Some(3), "{text}\n{err}");
    assert!(err.contains("which is not the test's: left alone"), "{err}");
    assert!(theirs.exists());
    // The write window was still closed.
    assert_eq!(
        log.last().map(String::as_str),
        Some("write closed"),
        "{log:?}"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn without_the_stand_ins_files_it_is_refused() {
    let d = std::env::temp_dir().join(format!("mpfree-sdtest-bare-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (code, _, err) = run(&d);
    assert_eq!(code, Some(3), "{err}");
    assert!(err.contains("run/mpfree-app is missing"), "{err}");
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
    std::fs::remove_dir(&d).unwrap();
}

#[test]
fn the_sd_card_run_is_a_run_of_its_own() {
    for other in ["--read-only", "--under-load", "--recorder", "--heat"] {
        let out = bin().args(["--sd-card", other]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{other}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("are different runs"));
    }
}
