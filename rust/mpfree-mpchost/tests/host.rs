//! The MPC plugin host with a real plugin library: MPFree's own Delay and
//! Squelch made into MPC plugins by `mpfree-mpcplug`
//! (`mpfree-mpchost-testplug`), loaded from plugin folders the way MPFree
//! finds them. The sound must match the same engine run in-process, sample
//! for sample; state, programs, value text and the audio thread's
//! no-allocation rule are checked too.

use mpfree_mpchost::{Folder, Library, MpcNode};
use mpfree_node::{Ctx, Kind, Node, NodeEvent, Timed};
use mpfree_prof::alloc::{count_in, installed, CountingAlloc};
use std::path::{Path, PathBuf};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const RATE: u32 = 44_100;

/// The test plugin library, which Cargo builds with this test (beside it,
/// or from Cargo 1.100 in its own build folder).
fn library() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let name = format!(
        "{}mpfree_mpchost_testplug{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let here = exe.parent().expect("test folder");
    let mut dirs = vec![here.to_path_buf(), here.join("..")];
    if let Some(build) = here.ancestors().nth(3).filter(|b| b.ends_with("build")) {
        if let Ok(rd) = std::fs::read_dir(build.join("mpfree-mpchost-testplug")) {
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

/// A plugin folder `<root>/<name>/` holding a copy of the library as
/// `VST/test.so` and a `plugin.json` naming it.
fn folder(root: &Path, name: &str) -> PathBuf {
    let d = root.join(name);
    std::fs::create_dir_all(d.join("VST")).unwrap();
    std::fs::copy(library(), d.join("VST/test.so")).unwrap();
    std::fs::write(
        d.join("plugin.json"),
        r#"{"name": "x", "manufacturer": "MPFree Tests", "format": "vst2", "file": "VST/test.so"}"#,
    )
    .unwrap();
    d
}

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpchost-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn load(dir: &Path) -> (MpcNode, mpfree_mpchost::MpcControl) {
    Library::open(dir).unwrap().create().unwrap()
}

fn ctx(frame: u64) -> Ctx<'static> {
    Ctx {
        rate: RATE,
        frame,
        bpm: 100.0,
        beats: 0.0,
        playing: true,
        sidechain: None,
    }
}

/// Runs `n` through `blocks` blocks of `block` frames, starting with
/// `first` (offset 0 of block 0) and an impulse on both sides for an
/// effect.
fn render(n: &mut dyn Node, block: usize, blocks: usize, first: &[NodeEvent]) -> Vec<f32> {
    let mut out = Vec::new();
    for b in 0..blocks {
        let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
        if b == 0 && n.info().kind == Kind::Effect {
            l[0] = 1.0;
            r[0] = 0.5;
        }
        let ev: Vec<Timed> = if b == 0 {
            first.iter().map(|&ev| Timed { offset: 0, ev }).collect()
        } else {
            Vec::new()
        };
        n.process(&ctx((b * block) as u64), &ev, &mut l, &mut r);
        out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
    }
    out
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn an_effect_sounds_as_the_engine_does_in_process() {
    let r = root("fx");
    let d = folder(&r, "MPFree Tests - VST - Test Delay");
    let f = Folder::open(&d).unwrap();
    assert_eq!(f.library, d.join("VST/test.so"));
    let (mut node, control) = load(&d);
    assert_eq!(node.kind(), Kind::Effect);
    assert_eq!((node.inputs(), node.outputs()), (2, 2));
    assert_eq!(node.info().id, "mpc:MTdl.test-delay");
    assert_eq!(node.info().name, "Test Delay");
    assert_eq!(control.vendor(), "MPFree Tests");
    assert_eq!(control.plugin_id(), "MTdl.test-delay");
    let local = mpfree_mpchost_testplug::make_delay();
    assert_eq!(
        node.params().len(),
        mpfree_mpcplug::host_params(local.as_ref()).len(),
        "every host parameter"
    );
    node.prepare(RATE, 256);
    assert!(node.is_active());
    let mut local = mpfree_mpchost_testplug::make_delay();
    local.prepare(RATE, 256);
    // Blocks shorter than the one announced, as the MPC sends them.
    let got = render(&mut node, 96, 40, &[]);
    let want = render(local.as_mut(), 96, 40, &[]);
    assert!(got.iter().any(|v| v.abs() > 1e-3), "the echo comes back");
    assert!(max_diff(&got, &want) < 1e-6, "{}", max_diff(&got, &want));
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn an_instrument_plays_its_notes_as_the_engine_does() {
    let r = root("synth");
    let d = folder(&r, "MPFree Tests - VST - Test Squelch");
    let (mut node, _control) = load(&d);
    assert_eq!(node.kind(), Kind::Instrument);
    assert_eq!(node.info().id, "mpc:MTsq.test-squelch");
    assert_eq!(node.outputs(), 2);
    node.prepare(RATE, 128);
    let mut local = mpfree_mpchost_testplug::make_squelch();
    local.prepare(RATE, 128);
    let on = [NodeEvent::NoteOn {
        note: 45,
        velocity: 100,
    }];
    let got = render(&mut node, 128, 30, &on);
    let want = render(local.as_mut(), 128, 30, &on);
    assert!(got.iter().any(|v| v.abs() > 1e-2), "it sounds");
    assert!(max_diff(&got, &want) < 1e-5, "{}", max_diff(&got, &want));
    // An instrument adds to what is already there.
    let (mut l, mut rr) = (vec![1.0f32; 64], vec![1.0f32; 64]);
    node.reset();
    node.process(&ctx(0), &[], &mut l, &mut rr);
    assert!(
        l.iter().all(|v| (v - 1.0).abs() < 0.5),
        "kept the 1.0 under it"
    );
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn state_programs_and_text_cross_instances() {
    let r = root("state");
    let d = folder(&r, "MPFree Tests - VST - Test Delay");
    let (mut a, ca) = load(&d);
    a.prepare(RATE, 128);
    a.set_param(1, 0.8);
    let v = a.param(1);
    assert!((v - 0.8).abs() < 0.02, "{v}");
    let text = ca.param_text(1, v).expect("text for the current value");
    assert!(!text.is_empty());
    assert_eq!(
        ca.param_text(1, 0.123),
        None,
        "only the current value has text"
    );
    let state = ca.save_state().expect("a state");
    let (b, cb) = load(&d);
    assert!((b.param(1) - 0.8).abs() > 0.01, "starts at its default");
    assert!(cb.load_state(&state));
    assert!((b.param(1) - v).abs() < 1e-6, "{} vs {v}", b.param(1));
    assert!(!cb.load_state(b"nonsense"));
    assert!(!cb.load_state(&state[..state.len() - 1]));
    let programs = ca.programs();
    assert_eq!(programs.len(), 1);
    assert!(ca.load_program(0).is_ok());
    assert!(ca.load_program(1).is_err());
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn the_audio_threads_part_never_allocates() {
    assert!(installed());
    let r = root("alloc");
    let d = folder(&r, "MPFree Tests - VST - Test Squelch");
    let (mut node, _c) = load(&d);
    node.prepare(RATE, 256);
    let ev = [
        Timed {
            offset: 0,
            ev: NodeEvent::NoteOn {
                note: 40,
                velocity: 90,
            },
        },
        Timed {
            offset: 10,
            ev: NodeEvent::Bend { value: 0.3 },
        },
        Timed {
            offset: 20,
            ev: NodeEvent::Param { id: 2, value: 0.4 },
        },
        Timed {
            offset: 30,
            ev: NodeEvent::AllOff,
        },
    ];
    let (mut l, mut rr) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    // Warm up: the plugin's own first block may set things up.
    node.process(&ctx(0), &ev, &mut l, &mut rr);
    let ((), n) = count_in(|| {
        for b in 0..50u64 {
            node.process(&ctx(b * 200), &ev, &mut l[..200], &mut rr[..200]);
            node.set_param(3, 0.5);
            let _ = node.param(3);
        }
    });
    assert_eq!(n, 0, "allocations on the audio thread");
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn a_library_that_isnt_a_plugin_is_refused() {
    let r = root("bad");
    let d = r.join("Nope - VST - Nope");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("nope.so"), b"not an ELF file").unwrap();
    let e = Library::open(&d).err().expect("refused");
    assert!(e.contains("nope.so"), "{e}");
    // A real shared library without VSTPluginMain.
    let libc = ["/lib/x86_64-linux-gnu/libm.so.6", "/usr/lib/libm.so.6"]
        .iter()
        .map(Path::new)
        .find(|p| p.is_file());
    if let Some(m) = libc {
        std::fs::copy(m, d.join("nope.so")).unwrap();
        let e = Library::open(&d).err().expect("refused");
        assert!(e.contains("VSTPluginMain"), "{e}");
    }
    let _ = std::fs::remove_dir_all(&r);
}
