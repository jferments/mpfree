//! The plugin rack with MPFree's own test plugins in a real plugins folder:
//! what a scan finds and leaves out, a plugin made from it, a crash
//! restarted from the last snapshot, the give-up after too many crashes,
//! and a dropped node's process let go. Every step is said in a line.

use mpfree_clap::testplug::{GAIN_ID, SYNTH_ID};
use mpfree_node::{Ctx, Kind, Node};
use mpfree_plughost::rack::{Rack, Summary, MAX_RESTARTS, RESTART_WINDOW};
use mpfree_plughost::{Deadline, Program};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RATE: u32 = 44_100;

fn exe() -> PathBuf {
    std::env::var_os("MPFREE_PLUGHOST_TEST_EXE").map_or_else(
        || PathBuf::from(env!("CARGO_BIN_EXE_mpfree-plughost")),
        PathBuf::from,
    )
}

/// The test plugins' shared library (as in tests/sandbox.rs).
fn plugin_file() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let name = format!(
        "{}mpfree_clap_testplug{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let here = exe.parent().expect("test folder");
    let mut dirs = vec![here.to_path_buf(), here.join("..")];
    if let Some(build) = here.ancestors().nth(3).filter(|b| b.ends_with("build")) {
        if let Ok(rd) = std::fs::read_dir(build.join("mpfree-clap-testplug")) {
            dirs.extend(rd.flatten().map(|e| e.path().join("out")));
        }
    }
    dirs.iter()
        .map(|d| d.join(&name))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("{name} not built next to {}", exe.display()))
}

/// A plugins folder: the test plugins as `a-test.clap` (and a second copy
/// with the same ids), a `.clap` that isn't a library, and a text file.
fn folder(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-rack-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::copy(plugin_file(), d.join("a-test.clap")).unwrap();
    std::fs::copy(plugin_file(), d.join("b-copy.clap")).unwrap();
    std::fs::write(d.join("c-junk.clap"), b"not a library").unwrap();
    std::fs::write(d.join("notes.txt"), b"not a plugin").unwrap();
    d
}

fn scanned(tag: &str) -> (Rack, Vec<String>, PathBuf) {
    let d = folder(tag);
    let (mut r, notes) = Rack::scan(Program::new(&exe()), &d, "the folder");
    // Long enough for a loaded machine, short enough that the blocks a
    // crash eats don't hold the test up.
    r.set_deadline(Deadline::Fixed(Duration::from_secs(1)));
    (r, notes, d)
}

fn param(n: &dyn Node, name: &str) -> u16 {
    n.params().iter().find(|p| p.name == name).expect(name).id
}

/// One block of a steady input through `n`; returns the left channel.
fn block(n: &mut dyn Node, frame: u64) -> Vec<f32> {
    let mut l = vec![0.5f32; 256];
    let mut r = vec![0.5f32; 256];
    n.process(&Ctx::new(RATE, frame), &[], &mut l, &mut r);
    l
}

fn until_gone(r: &Rack, crashes: u64) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        r.watch(Instant::now());
        if r.summary().is_some_and(|s| s.crashes >= crashes) {
            return;
        }
        assert!(Instant::now() < until, "the crash wasn't seen");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_scan_lists_each_file_in_its_own_process_and_says_what_it_left_out() {
    let (r, notes, d) = scanned("scan");
    let got: Vec<(&str, &str, Kind)> = r
        .entries()
        .iter()
        .map(|e| (e.node_id, e.plugin_id.as_str(), e.kind))
        .collect();
    assert_eq!(
        got,
        vec![
            ("clap:org.mpfree.test-synth", SYNTH_ID, Kind::Instrument),
            ("clap:org.mpfree.test-gain", GAIN_ID, Kind::Effect)
        ]
    );
    assert!(r.entries().iter().all(|e| e.file == d.join("a-test.clap")));
    assert!(r.entries().iter().all(|e| e.about.starts_with("Plugin")));
    let fx: Vec<&str> = r.effects().map(|e| e.node_id).collect();
    assert_eq!(fx, ["clap:org.mpfree.test-gain"]);
    let said = notes.join("\n");
    assert!(
        said.contains("b-copy.clap left out: another file has the same id"),
        "{said}"
    );
    assert!(said.contains("c-junk.clap left out"), "{said}");
    assert!(!said.contains("notes.txt"), "{said}");
    assert!(
        said.contains("1 effect(s) and 1 instrument(s) in the folder"),
        "{said}"
    );
    assert!(Rack::is_plugin("clap:x") && !Rack::is_plugin("mpfree.fx.eq"));
    assert!(r.make("clap:org.nobody.nothing").is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_crashed_plugin_is_restarted_from_its_snapshot_then_left_off() {
    let (r, _, d) = scanned("crash");
    let mut n = r.make("clap:org.mpfree.test-gain").expect("make");
    n.prepare(RATE, 256);
    assert_eq!(n.info().id, "clap:org.mpfree.test-gain");
    let started = r.take_lines().expect("a line");
    assert!(
        started[0].contains("started in its own process"),
        "{started:?}"
    );
    let (gain, crash) = (param(&*n, "Gain"), param(&*n, "Crash"));
    n.set_param(gain, -20.0 * 2f32.log10());
    let half = block(&mut *n, 0);
    assert!((half[0] - 0.25).abs() < 1e-4, "{}", half[0]);
    r.refresh_states();
    assert_eq!(
        r.summary(),
        Some(Summary {
            found: 2,
            running: 1,
            ..Summary::default()
        })
    );
    // Crash it MAX_RESTARTS times: each is restarted with the half gain.
    for i in 0..MAX_RESTARTS as u64 {
        n.set_param(crash, 1.0);
        let _ = block(&mut *n, 256);
        until_gone(&r, i + 1);
        let lines = r.take_lines().expect("said").join("\n");
        assert!(
            lines.contains("crashed (signal 6); restarted from its last state"),
            "{lines}"
        );
        let back = block(&mut *n, 512);
        assert!(
            (back[0] - 0.25).abs() < 1e-4,
            "restart {i}: the snapshot's gain, {}",
            back[0]
        );
        assert_eq!(n.param(crash), 0.0);
    }
    // One more within the window: left off, its input passed through.
    n.set_param(crash, 1.0);
    let _ = block(&mut *n, 768);
    until_gone(&r, MAX_RESTARTS as u64 + 1);
    let lines = r.take_lines().expect("said").join("\n");
    assert!(lines.contains("left off"), "{lines}");
    assert!(
        lines.contains(&format!("in {} s", RESTART_WINDOW.as_secs())),
        "{lines}"
    );
    assert_eq!(block(&mut *n, 1024), vec![0.5; 256], "dry");
    let s = r.summary().unwrap();
    assert_eq!(
        (s.running, s.down, s.crashes, s.restarts, s.failed),
        (0, 1, MAX_RESTARTS as u64 + 1, MAX_RESTARTS as u64, 0)
    );
    assert!(s.line().unwrap().contains("1 down"));
    // Watching again doesn't count it again.
    r.watch(Instant::now());
    assert_eq!(r.summary().unwrap().crashes, MAX_RESTARTS as u64 + 1);
    assert!(r.take_lines().is_none(), "nothing new to say");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_dropped_node_lets_its_process_go() {
    let (r, _, d) = scanned("drop");
    let n = r.make("clap:org.mpfree.test-synth").expect("make");
    assert_eq!(n.info().kind, Kind::Instrument);
    assert_eq!(r.summary().unwrap().running, 1);
    drop(n);
    r.watch(Instant::now());
    assert_eq!(r.summary().unwrap().running, 0);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_empty_or_missing_folder_finds_nothing_and_says_nothing() {
    let (r, notes) = Rack::scan(Program::new(&exe()), Path::new("/nonexistent/plugins"), "x");
    assert!(r.entries().is_empty() && notes.is_empty());
    assert_eq!(r.summary().unwrap().line(), None);
}
