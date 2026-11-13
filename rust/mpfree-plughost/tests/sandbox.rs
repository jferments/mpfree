//! The sandbox with MPFree's own test plugins, loaded from the real `.clap`
//! file in a real `mpfree-plughost` process: the same sound as in-process,
//! a plugin that stalls, a plugin that crashes, state and text across the
//! process, and a sandboxed synth on a track.

use mpfree_clap::testplug::{gain_params, synth_params, GAIN_ID, SYNTH_ID};
use mpfree_clap::{ClapNode, Library};
use mpfree_node::{Ctx, Kind, Node, NodeEvent, Timed};
use mpfree_plughost::{
    list, parse_list_line, spawn, Deadline, Error, Health, Listed, Program, Sandbox, SandboxNode,
};
use mpfree_track::{Instrument, Sum, TrackHost};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RATE: u32 = 44_100;
/// Generous: these tests are about what arrives, not how fast.
const PATIENT: Deadline = Deadline::Fixed(Duration::from_secs(5));

/// The process binary. `MPFREE_PLUGHOST_TEST_EXE` stands in for it when
/// the tests run under an emulator (a wrapper that starts the ARM build
/// under qemu, since a process started from inside qemu-user runs on the
/// host CPU).
fn exe() -> PathBuf {
    std::env::var_os("MPFREE_PLUGHOST_TEST_EXE").map_or_else(
        || PathBuf::from(env!("CARGO_BIN_EXE_mpfree-plughost")),
        PathBuf::from,
    )
}

/// The test plugins' shared library, which Cargo builds with this test:
/// next to it (target/debug/deps, or target/debug when it is a workspace
/// member) or, from Cargo 1.100, in its own build folder.
fn plugin_file() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let name = format!(
        "{}mpfree_clap_testplug{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let here = exe.parent().expect("test folder");
    let mut dirs = vec![here.to_path_buf(), here.join("..")];
    // Cargo 1.100 builds each unit in build/PACKAGE/HASH/out, so the
    // library is in its own package's folders there; take the newest.
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

/// The same plugin, in this process.
fn local(id: &str, max_block: usize) -> ClapNode {
    let mut n = Library::open(&plugin_file())
        .expect("open")
        .create(id)
        .expect("create");
    n.prepare(RATE, max_block);
    n
}

fn sandboxed(id: &str, max_block: usize) -> (SandboxNode, Sandbox) {
    let (mut n, sb) = spawn(&exe(), &plugin_file(), id).expect("spawn");
    n.set_deadline(PATIENT);
    n.prepare(RATE, max_block);
    (n, sb)
}

fn input(frames: usize) -> (Vec<f32>, Vec<f32>) {
    let l = (0..frames)
        .map(|i| (i as f32 * 0.031).sin() * 0.8)
        .collect();
    let r = (0..frames)
        .map(|i| (i as f32 * 0.017).cos() * 0.6)
        .collect();
    (l, r)
}

/// Renders `frames` frames in blocks of `block` from `start`, with `events`
/// at absolute frames. Effects get `input`; instruments start from silence.
fn render(
    n: &mut dyn Node,
    events: &[(u64, NodeEvent)],
    start: u64,
    frames: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = if n.info().kind == Kind::Effect {
        input(frames)
    } else {
        (vec![0f32; frames], vec![0f32; frames])
    };
    let mut at = 0usize;
    let mut buf = Vec::new();
    while at < frames {
        let len = block.min(frames - at);
        let from = start + at as u64;
        buf.clear();
        for &(f, e) in events {
            if f >= from && f < from + len as u64 {
                buf.push(Timed::new((f - from) as u32, e));
            }
        }
        let c = Ctx {
            bpm: 97.0,
            playing: true,
            beats: from as f64 / 1000.0,
            ..Ctx::new(RATE, from)
        };
        n.process(&c, &buf, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    (l, r)
}

/// One block of 256 frames of the test input through `n`.
fn block(n: &mut dyn Node, frame: u64) -> (Vec<f32>, Vec<f32>) {
    render(n, &[], frame, 256, 256)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn scaled(x: &[f32], g: f32) -> Vec<f32> {
    x.iter().map(|s| s * g).collect()
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

/// -6.02 dB: half.
fn half_db() -> f32 {
    -20.0 * 2f32.log10()
}

#[test]
fn the_node_looks_the_same_as_in_process() {
    for id in [SYNTH_ID, GAIN_ID] {
        let here = local(id, 256);
        let (there, _sb) = sandboxed(id, 256);
        assert_eq!(there.info().id, here.info().id);
        assert_eq!(there.info().name, here.info().name);
        assert_eq!(there.info().kind, here.info().kind);
        assert_eq!(there.kind(), here.kind());
        let (a, b) = (here.params(), there.params());
        assert_eq!(a.len(), b.len());
        for (p, q) in a.iter().zip(b) {
            assert_eq!(
                (p.id, p.name, p.group, p.min, p.max, p.default, p.readonly),
                (q.id, q.name, q.group, q.min, q.max, q.default, q.readonly)
            );
            assert_eq!(p.curve, q.curve);
        }
        for i in 0..a.len() as u16 {
            assert_eq!(
                here.param(i).to_bits(),
                there.param(i).to_bits(),
                "{id} param {i}"
            );
        }
    }
}

#[test]
fn a_sandboxed_synth_sounds_exactly_like_in_process() {
    let mut here = local(SYNTH_ID, 256);
    let (mut there, _sb) = sandboxed(SYNTH_ID, 256);
    let decay = here.param_index(synth_params::DECAY).unwrap();
    let wave = here.param_index(synth_params::WAVE).unwrap();
    let events = [
        (
            10,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 100,
            },
        ),
        (
            300,
            NodeEvent::NoteOn {
                note: 64,
                velocity: 70,
            },
        ),
        (
            500,
            NodeEvent::Pressure {
                note: 64,
                value: 90,
            },
        ),
        (700, NodeEvent::Bend { value: 0.4 }),
        (
            1000,
            NodeEvent::Param {
                id: wave,
                value: 1.0,
            },
        ),
        (2000, NodeEvent::NoteOff { note: 60 }),
        (
            2600,
            NodeEvent::NoteOn {
                note: 67,
                velocity: 127,
            },
        ),
        (3500, NodeEvent::AllOff),
    ];
    for n in [&mut here as &mut dyn Node, &mut there] {
        n.set_param(decay, 150.0);
    }
    let a = render(&mut here, &events, 0, 4096, 256);
    let b = render(&mut there, &events, 0, 4096, 256);
    assert!(peak(&a.0) > 0.01, "the synth sounds");
    assert_eq!(a, b, "sample for sample the same");
    // The values the plugin sends back (voices, the tempo it saw) arrive.
    for i in 0..here.params().len() as u16 {
        assert_eq!(
            here.param(i).to_bits(),
            there.param(i).to_bits(),
            "param {i}"
        );
    }
    assert_eq!(
        there.param(here.param_index(synth_params::TEMPO).unwrap()),
        97.0
    );
    // A reset reaches the plugin before its next block: both go quiet.
    let more = [(
        4100,
        NodeEvent::NoteOn {
            note: 50,
            velocity: 110,
        },
    )];
    let _ = render(&mut here, &more, 4096, 512, 256);
    let _ = render(&mut there, &more, 4096, 512, 256);
    here.reset();
    there.reset();
    there.reset();
    let a = render(&mut here, &[], 4608, 512, 256);
    let b = render(&mut there, &[], 4608, 512, 256);
    assert_eq!(a, b);
    assert_eq!(peak(&b.0), 0.0, "reset silenced the voices");
    let s = there.stats();
    assert_eq!((s.misses, s.busy, s.down, s.plugin_errors), (0, 0, 0, 0));
    assert_eq!(s.blocks, 16 + 4);
}

#[test]
fn a_sandboxed_effect_matches_in_process() {
    let mut here = local(GAIN_ID, 128);
    let (mut there, _sb) = sandboxed(GAIN_ID, 128);
    let gain = here.param_index(gain_params::GAIN).unwrap();
    for n in [&mut here as &mut dyn Node, &mut there] {
        n.set_param(gain, -4.5);
    }
    let events = [(
        900,
        NodeEvent::Param {
            id: gain,
            value: 3.0,
        },
    )];
    // Blocks of 300 on a node prepared for 128: the host's own limit cuts
    // them, in both.
    let a = render(&mut here, &events, 0, 2048, 300);
    let b = render(&mut there, &events, 0, 2048, 300);
    assert_eq!(a, b);
    assert_eq!(there.param(gain), 3.0);
    // Each piece is a round trip: six blocks of 300 in three pieces, and
    // the last 248 frames in two.
    assert_eq!(there.stats().blocks, 6 * 3 + 2);
}

#[test]
fn a_stalled_plugin_misses_its_deadline_and_catches_up() {
    let here = local(GAIN_ID, 256);
    let (gain, stall) = (
        here.param_index(gain_params::GAIN).unwrap(),
        here.param_index(gain_params::STALL).unwrap(),
    );
    let (mut n, _sb) = sandboxed(GAIN_ID, 256);
    let (dry_l, dry_r) = input(256);
    n.set_param(gain, half_db());
    let (l, r) = block(&mut n, 0);
    assert!(close(&l, &scaled(&dry_l, 0.5)) && close(&r, &scaled(&dry_r, 0.5)));
    // A second asleep in process, against a 30 ms deadline.
    n.set_param(stall, 1000.0);
    n.set_deadline(Deadline::Fixed(Duration::from_millis(30)));
    let t = Instant::now();
    let (l, r) = block(&mut n, 256);
    assert!(
        t.elapsed() < Duration::from_millis(600),
        "the node didn't wait for the plugin: {:?}",
        t.elapsed()
    );
    assert_eq!(
        (l.as_slice(), r.as_slice()),
        (dry_l.as_slice(), dry_r.as_slice()),
        "a missed effect passes its input"
    );
    assert_eq!(n.stats().misses, 1);
    // Still asleep: the next block isn't even posted.
    n.set_param(stall, 0.0);
    let (l, _) = block(&mut n, 512);
    assert_eq!(l, dry_l);
    assert_eq!(n.stats().busy, 1);
    assert_eq!(n.stats().misses, 1, "a busy block isn't another wait");
    // Awake again: the queued change goes with the next block.
    std::thread::sleep(Duration::from_millis(1200));
    n.set_deadline(PATIENT);
    let (l, _) = block(&mut n, 768);
    assert!(close(&l, &scaled(&dry_l, 0.5)));
    assert_eq!(n.param(stall), 0.0);
    let s = n.stats();
    assert_eq!(
        (s.blocks, s.misses, s.busy, s.down, s.crashes),
        (2, 1, 1, 0, 0)
    );
}

/// Whether every thread of process `pid` is stopped (state T).
fn all_stopped(pid: libc::pid_t) -> bool {
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return false;
    };
    tasks.flatten().all(|t| {
        let stat = std::fs::read_to_string(t.path().join("stat")).unwrap_or_default();
        // "tid (name) S ...": the state follows the last ')'.
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            == Some("T")
    })
}

#[test]
fn a_frozen_instrument_is_a_silent_miss() {
    let (mut n, sb) = sandboxed(SYNTH_ID, 256);
    let pid = sb.pid().unwrap() as libc::pid_t;
    // SAFETY: signals to our own child.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
    // Every thread of it stopped (a stop reaches the threads one by one).
    let until = Instant::now() + Duration::from_secs(5);
    while !all_stopped(pid) {
        assert!(Instant::now() < until, "the process didn't stop");
        std::thread::sleep(Duration::from_millis(2));
    }
    n.set_deadline(Deadline::Fixed(Duration::from_millis(20)));
    let ev = [(
        0,
        NodeEvent::NoteOn {
            note: 60,
            velocity: 100,
        },
    )];
    let (l, _) = render(&mut n, &ev, 0, 256, 256);
    assert_eq!(peak(&l), 0.0);
    assert_eq!(n.stats().misses, 1);
    assert_eq!(sb.poll(), Health::Running, "stopped isn't gone");
    assert_eq!(unsafe { libc::kill(pid, libc::SIGCONT) }, 0);
    // Thawed, it answers the late block, then the next one on time.
    n.set_deadline(PATIENT);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let (l, _) = render(&mut n, &[], 256, 256, 256);
        if n.stats().blocks == 1 {
            assert!(peak(&l) > 0.01, "the note from the missed block sounds now");
            break;
        }
        assert!(Instant::now() < until, "{:?}", n.stats());
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_crash_is_noticed_and_restarted_from_saved_state() {
    let here = local(GAIN_ID, 256);
    let (gain, crash) = (
        here.param_index(gain_params::GAIN).unwrap(),
        here.param_index(gain_params::CRASH).unwrap(),
    );
    let (mut n, sb) = sandboxed(GAIN_ID, 256);
    let (dry_l, _) = input(256);
    n.set_param(gain, half_db());
    let (l, _) = block(&mut n, 0);
    assert!(close(&l, &scaled(&dry_l, 0.5)));
    let state = sb.save_state().expect("the gain has state");
    let first = sb.pid().expect("running");
    assert_eq!(sb.poll(), Health::Running);
    // The plugin aborts its process in the middle of a block.
    n.set_param(crash, 1.0);
    n.set_deadline(Deadline::Fixed(Duration::from_millis(300)));
    let (l, _) = block(&mut n, 256);
    assert_eq!(l, dry_l);
    assert_eq!(n.stats().misses, 1);
    let until = Instant::now() + Duration::from_secs(5);
    let gone = loop {
        match sb.poll() {
            Health::Gone(code) => break code,
            Health::Running if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Health::Running => panic!("the process didn't go"),
        }
    };
    assert_eq!(gone, -libc::SIGABRT);
    // Gone: the node doesn't wait at all.
    let (l, _) = block(&mut n, 512);
    assert_eq!(l, dry_l);
    assert_eq!((n.stats().down, n.stats().misses), (1, 1));
    // A new process, prepared as before, with the state from before.
    sb.restart(Some(&state)).expect("restart");
    assert_ne!(sb.pid(), Some(first));
    assert_eq!(sb.poll(), Health::Running);
    n.set_deadline(PATIENT);
    let (l, _) = block(&mut n, 768);
    assert!(close(&l, &scaled(&dry_l, 0.5)), "the saved gain is back");
    assert_eq!(n.param(crash), 0.0);
    let s = n.stats();
    assert_eq!(
        (s.blocks, s.misses, s.down, s.crashes, s.restarts),
        (2, 1, 1, 1, 1)
    );
}

#[test]
fn a_killed_process_is_noticed_too() {
    let (mut n, sb) = sandboxed(SYNTH_ID, 256);
    let pid = sb.pid().unwrap() as libc::pid_t;
    // SAFETY: a signal to our own child.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    let until = Instant::now() + Duration::from_secs(5);
    while sb.poll() == Health::Running {
        assert!(Instant::now() < until, "the process didn't go");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(sb.poll(), Health::Gone(-libc::SIGKILL));
    let _ = block(&mut n, 0);
    assert_eq!(
        n.stats().crashes,
        1,
        "counted once, however often it is polled"
    );
    sb.restart(None).expect("restart");
    let ev = [(
        256,
        NodeEvent::NoteOn {
            note: 60,
            velocity: 100,
        },
    )];
    let (l, _) = render(&mut n, &ev, 256, 1024, 256);
    assert!(peak(&l) > 0.01, "it plays again");
}

#[test]
fn state_and_value_text_cross_the_process() {
    let here = local(SYNTH_ID, 256);
    let (decay, wave) = (
        here.param_index(synth_params::DECAY).unwrap(),
        here.param_index(synth_params::WAVE).unwrap(),
    );
    let (mut n, sb) = sandboxed(SYNTH_ID, 256);
    for v in [0.0, 1.0, 2.0] {
        assert_eq!(sb.param_text(wave, v), here.control().param_text(wave, v));
    }
    assert_eq!(sb.param_text(wave, 1.0).as_deref(), Some("Saw"));
    assert_eq!(sb.param_text(999, 1.0), None);
    n.set_param(decay, 777.0);
    n.set_param(wave, 2.0);
    let _ = block(&mut n, 0);
    let state = sb.save_state().expect("state");
    // A fresh process for the same plugin takes the state; the node's
    // values follow from its next block.
    let (mut m, sb2) = sandboxed(SYNTH_ID, 256);
    assert_eq!(m.param(decay), 300.0);
    assert!(sb2.load_state(&state));
    let _ = block(&mut m, 0);
    assert_eq!((m.param(decay), m.param(wave)), (777.0, 2.0));
    assert!(!sb2.load_state(b"not a state"));
}

#[test]
fn a_missing_plugin_or_file_fails_cleanly() {
    match spawn(&exe(), &plugin_file(), "org.nobody.nothing") {
        Err(Error::Plugin(msg)) => assert!(msg.contains("org.nobody.nothing"), "{msg}"),
        other => panic!("expected a plugin error, got {:?}", other.err()),
    }
    match spawn(&exe(), Path::new("/nonexistent/none.clap"), SYNTH_ID) {
        Err(Error::Plugin(msg)) => assert!(msg.contains("none.clap"), "{msg}"),
        other => panic!("expected a plugin error, got {:?}", other.err()),
    }
    assert!(matches!(
        spawn(
            Path::new("/nonexistent/mpfree-plughost"),
            &plugin_file(),
            SYNTH_ID
        ),
        Err(Error::Io(_))
    ));
}

#[test]
fn dropping_the_node_and_handle_ends_the_process() {
    let (n, sb) = sandboxed(GAIN_ID, 256);
    let pid = sb.pid().unwrap() as libc::pid_t;
    drop(n);
    // SAFETY: signal 0 only asks whether the process exists.
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        0,
        "alive while the handle is"
    );
    drop(sb);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "gone and reaped");
}

#[test]
fn a_sandboxed_synth_plays_on_a_track() {
    let (mut synth, sb) = spawn(&exe(), &plugin_file(), SYNTH_ID).expect("spawn");
    synth.set_deadline(PATIENT);
    let mut host = TrackHost::new(2, RATE, 256);
    host.swap(0, Some(Instrument::new(Box::new(synth), RATE, 256)))
        .expect("slot 0");
    assert_eq!(
        host.instrument(0).map(|i| i.node().info().id),
        Some("clap:org.mpfree.test-synth")
    );
    host.note_on(0, 3, 69, 120);
    let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
    let mut loud = 0f32;
    for b in 0..8u64 {
        l.fill(0.0);
        r.fill(0.0);
        host.render(
            &Ctx::new(RATE, b * 256),
            256,
            &mut Sum {
                left: &mut l,
                right: &mut r,
            },
        );
        loud = loud.max(peak(&l));
    }
    assert!(loud > 0.01, "the sandboxed plugin sounds on its track");
    assert_eq!(sb.poll(), Health::Running);
}

#[test]
fn a_plugin_file_is_listed_from_its_own_process() {
    let got = list(&Program::new(&exe()), &plugin_file()).expect("list");
    let kinds: Vec<(&str, Kind)> = got.iter().map(|l| (l.id.as_str(), l.kind)).collect();
    assert_eq!(
        kinds,
        vec![(SYNTH_ID, Kind::Instrument), (GAIN_ID, Kind::Effect)]
    );
    let here = Library::open(&plugin_file()).unwrap().plugins();
    for (l, d) in got.iter().zip(&here) {
        assert_eq!((&l.name, &l.vendor), (&d.name, &d.vendor));
    }
    match list(&Program::new(&exe()), Path::new("/nonexistent/none.clap")) {
        Err(Error::Plugin(msg)) => assert!(msg.contains("none.clap"), "{msg}"),
        other => panic!("expected a plugin error, got {other:?}"),
    }
    assert!(matches!(
        list(
            &Program::new(Path::new("/nonexistent/mpfree-plughost")),
            &plugin_file()
        ),
        Err(Error::Io(_))
    ));
}

#[test]
fn list_lines_read_back_and_junk_is_refused() {
    let line = mpfree_plughost::child::list_line("a.b", "Na\tme", "V\nendor", Kind::Effect);
    assert_eq!(
        parse_list_line(&line),
        Some(Listed {
            id: "a.b".into(),
            name: "Na me".into(),
            vendor: "V endor".into(),
            kind: Kind::Effect
        })
    );
    for junk in [
        "",
        "plugin",
        "plugin\ta\tb\tc\tsynth",
        "plugin\t\tb\tc\teffect",
        "x\ta\tb\tc\teffect",
        "plugin\ta\tb\tc\teffect\textra",
    ] {
        assert_eq!(parse_list_line(junk), None, "{junk:?}");
    }
}

#[test]
fn a_crash_restarts_from_the_snapshot_and_the_node_reads_it() {
    let here = local(GAIN_ID, 256);
    let (gain, crash) = (
        here.param_index(gain_params::GAIN).unwrap(),
        here.param_index(gain_params::CRASH).unwrap(),
    );
    let (mut n, sb) = sandboxed(GAIN_ID, 256);
    assert_eq!(n.saved_state(), None, "nothing read yet");
    assert_eq!(n.plugin_file(), plugin_file().as_path());
    assert_eq!(sb.plugin_id(), GAIN_ID);
    let (dry_l, _) = input(256);
    n.set_param(gain, half_db());
    let _ = block(&mut n, 0);
    assert!(sb.refresh_state());
    let snap = n.saved_state().expect("the snapshot");
    assert_eq!(Some(snap.clone()), sb.save_state());
    n.set_param(crash, 1.0);
    n.set_deadline(Deadline::Fixed(Duration::from_millis(300)));
    let _ = block(&mut n, 256);
    let until = Instant::now() + Duration::from_secs(5);
    while sb.poll() == Health::Running {
        assert!(Instant::now() < until, "the process didn't go");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!sb.refresh_state(), "a gone process gives no state");
    assert_eq!(n.saved_state(), Some(snap), "the old snapshot stays");
    sb.restart_from_snapshot().expect("restart");
    n.set_deadline(PATIENT);
    let (l, _) = block(&mut n, 512);
    assert!(
        close(&l, &scaled(&dry_l, 0.5)),
        "the snapshot's gain is back"
    );
    assert!(!sb.orphaned());
    drop(n);
    assert!(sb.orphaned(), "only the handle is left");
}
