//! The host's part of the sandbox never allocates or frees on the audio
//! thread: notes, pressure, bend, parameters (as events and between
//! blocks), resets, the values the process sends back, blocks cut to the
//! process's size, misses and busy blocks. (The plugin process has its own
//! audio thread; what it does there is `mpfree-clap`'s, which has its own
//! test.)

use mpfree_clap::testplug::{gain_params, synth_params, GAIN_ID, SYNTH_ID};
use mpfree_clap::Library;
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_plughost::{spawn, Deadline};
use mpfree_prof::alloc::{installed, thread_count, CountingAlloc};
use std::path::PathBuf;
use std::time::Duration;

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const RATE: u32 = 44_100;

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

#[test]
fn the_audio_threads_part_never_allocates() {
    assert!(installed(), "the counting allocator is on for this test");
    let exe: PathBuf = std::env::var_os("MPFREE_PLUGHOST_TEST_EXE").map_or_else(
        || env!("CARGO_BIN_EXE_mpfree-plughost").into(),
        PathBuf::from,
    );
    let exe = exe.as_path();
    let file = plugin_file();
    // Indexes from the in-process node (the same order).
    let lib = Library::open(&file).expect("open");
    let (decay, wave) = {
        let n = lib.create(SYNTH_ID).expect("synth");
        (
            n.param_index(synth_params::DECAY).unwrap(),
            n.param_index(synth_params::WAVE).unwrap(),
        )
    };
    let (level, stall) = {
        let n = lib.create(GAIN_ID).expect("gain");
        (
            n.param_index(gain_params::GAIN).unwrap(),
            n.param_index(gain_params::STALL).unwrap(),
        )
    };
    let (mut synth, _s) = spawn(exe, &file, SYNTH_ID).expect("synth");
    let (mut gain, _g) = spawn(exe, &file, GAIN_ID).expect("gain");
    synth.set_deadline(Deadline::Fixed(Duration::from_secs(5)));
    gain.set_deadline(Deadline::Fixed(Duration::from_secs(5)));
    // Prepared for 64 frames, run in blocks of 128: every block is cut.
    synth.prepare(RATE, 64);
    gain.prepare(RATE, 256);
    let mut events: Vec<Timed> = Vec::with_capacity(16);
    let (mut l, mut r) = (vec![0f32; 128], vec![0f32; 128]);
    let before = thread_count();
    for b in 0..600u64 {
        events.clear();
        if b % 3 == 0 {
            events.push(Timed::new(
                5,
                NodeEvent::NoteOn {
                    note: 40 + (b % 40) as u8,
                    velocity: 100,
                },
            ));
        }
        if b % 4 == 0 {
            events.push(Timed::new(
                20,
                NodeEvent::Pressure {
                    note: 40 + (b % 40) as u8,
                    value: (b % 128) as u8,
                },
            ));
        }
        if b % 5 == 0 {
            events.push(Timed::new(
                70,
                NodeEvent::NoteOff {
                    note: 40 + ((b + 7) % 40) as u8,
                },
            ));
            events.push(Timed::new(
                90,
                NodeEvent::Param {
                    id: wave,
                    value: (b % 3) as f32,
                },
            ));
        }
        if b % 7 == 0 {
            events.push(Timed::new(
                100,
                NodeEvent::Bend {
                    value: ((b % 20) as f32 - 10.0) / 10.0,
                },
            ));
        }
        if b % 200 == 199 {
            events.push(Timed::new(127, NodeEvent::AllOff));
            synth.reset();
        }
        if b % 11 == 0 {
            synth.set_param(decay, 50.0 + (b % 1000) as f32);
            gain.set_param(level, -((b % 12) as f32));
        }
        // A stall, a miss and a busy block on the way.
        if b == 300 {
            gain.set_param(stall, 50.0);
            gain.set_deadline(Deadline::Fixed(Duration::from_millis(1)));
        }
        if b == 302 {
            gain.set_param(stall, 0.0);
        }
        if b == 303 {
            std::thread::sleep(Duration::from_millis(100));
            gain.set_deadline(Deadline::Fixed(Duration::from_secs(5)));
        }
        l.fill(0.0);
        r.fill(0.0);
        let c = Ctx {
            bpm: 90.0 + (b % 30) as f64,
            playing: true,
            ..Ctx::new(RATE, b * 128)
        };
        synth.process(&c, &events, &mut l, &mut r);
        gain.process(&c, &[], &mut l, &mut r);
        let _ = (synth.param(0), synth.stats(), gain.stats());
    }
    let calls = thread_count() - before;
    assert_eq!(
        calls, 0,
        "the audio thread's part allocated or freed {calls} times"
    );
    let (s, g) = (synth.stats(), gain.stats());
    assert_eq!((s.blocks, s.misses, s.busy), (1200, 0, 0));
    assert!(g.misses >= 1 && g.busy >= 1, "{g:?}");
}
