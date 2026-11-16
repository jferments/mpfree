//! MPC plugins in the sandbox: a plugin folder holding MPFree's Delay or
//! Squelch built as an MPC plugin (`mpfree-mpchost-testplug`), found by
//! the rack, listed and started in a real `mpfree-plughost` process. The
//! sound must match the same plugin loaded in this process; state, value
//! text, programs as presets and a restart cross the process.

use mpfree_mpchost::Library;
use mpfree_node::{Ctx, Kind, Node, NodeEvent, Timed};
use mpfree_plughost::rack::Rack;
use mpfree_plughost::{list, spawn, Deadline, Listed, Program};
use std::path::{Path, PathBuf};
use std::time::Duration;

const RATE: u32 = 44_100;
const PATIENT: Deadline = Deadline::Fixed(Duration::from_secs(5));

/// The process binary (or `MPFREE_PLUGHOST_TEST_EXE` under an emulator).
fn exe() -> PathBuf {
    std::env::var_os("MPFREE_PLUGHOST_TEST_EXE").map_or_else(
        || PathBuf::from(env!("CARGO_BIN_EXE_mpfree-plughost")),
        PathBuf::from,
    )
}

/// The test plugin library Cargo builds with this test.
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

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("plughost-mpc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A catalog-style ("portable") plugin folder: the library and a
/// plugin-meta.xml line, as the Open MPC releases ship them.
fn portable(root: &Path, name: &str, instrument: bool) -> PathBuf {
    let d = root.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::copy(library(), d.join("test.so")).unwrap();
    std::fs::write(
        d.join("plugin-meta.xml"),
        format!(
            r#"<PLUGIN name="x" format="VST" manufacturer="MPFree Tests" file="%payload-path%/{name}/test.so" uid="0" isInstrument="{}" numInputs="2" numOutputs="2"/>"#,
            u8::from(instrument)
        ),
    )
    .unwrap();
    d
}

fn render(n: &mut dyn Node, first: &[NodeEvent], blocks: usize) -> Vec<f32> {
    let mut out = Vec::new();
    for b in 0..blocks {
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        if n.info().kind == Kind::Effect {
            for (i, (a, c)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
                *a = ((b * 128 + i) as f32 * 0.03).sin() * 0.5;
                *c = ((b * 128 + i) as f32 * 0.02).cos() * 0.4;
            }
        }
        let ev: Vec<Timed> = if b == 0 {
            first.iter().map(|&ev| Timed { offset: 3, ev }).collect()
        } else {
            Vec::new()
        };
        let c = Ctx {
            playing: true,
            ..Ctx::new(RATE, (b * 128) as u64)
        };
        n.process(&c, &ev, &mut l, &mut r);
        out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
    }
    out
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn an_mpc_effect_and_instrument_sound_the_same_in_their_own_process() {
    let r = root("sound");
    for (name, inst, first) in [
        ("MPFree Tests - VST - Test Delay", false, vec![]),
        (
            "MPFree Tests - VST - Test Squelch",
            true,
            vec![NodeEvent::NoteOn {
                note: 48,
                velocity: 110,
            }],
        ),
    ] {
        let d = portable(&r, name, inst);
        let listed = list(&Program::new(&exe()), &d).expect("list");
        let id = if inst {
            "MTsq.test-squelch"
        } else {
            "MTdl.test-delay"
        };
        assert_eq!(
            listed,
            vec![Listed {
                id: id.into(),
                name: if inst { "Test Squelch" } else { "Test Delay" }.into(),
                vendor: "MPFree Tests".into(),
                kind: if inst { Kind::Instrument } else { Kind::Effect },
            }]
        );
        let (mut sb, ctl) = spawn(&exe(), &d, id).expect("spawn");
        assert_eq!(sb.info().id, format!("mpc:{id}"));
        sb.set_deadline(PATIENT);
        sb.prepare(RATE, 128);
        let (mut local, _c) = Library::open(&d).unwrap().create().unwrap();
        local.prepare(RATE, 128);
        let got = render(&mut sb, &first, 40);
        let want = render(&mut local, &first, 40);
        assert!(got.iter().any(|v| v.abs() > 1e-3), "{name} sounds");
        assert!(
            max_diff(&got, &want) < 1e-5,
            "{name}: {}",
            max_diff(&got, &want)
        );
        assert_eq!(sb.stats().misses, 0);
        // State, text and the plugin's program across the process.
        sb.set_param(1, 0.7);
        let _ = render(&mut sb, &[], 1);
        let v = sb.param(1);
        assert!(ctl.param_text(1, v).is_some_and(|t| !t.is_empty()));
        // Every value and text at once, as the plugin holds them (a skin's).
        let (values, texts) = ctl.values().expect("values");
        assert_eq!(values.len(), sb.params().len());
        assert_eq!(texts.len(), values.len());
        assert!((values[1] - v).abs() < 1e-6);
        assert_eq!(Some(texts[1].clone()), ctl.param_text(1, v));
        let state = ctl.save_state().expect("state");
        let (mut other, ctl2) = spawn(&exe(), &d, id).expect("spawn");
        other.set_deadline(PATIENT);
        other.prepare(RATE, 128);
        assert!(ctl2.load_state(&state));
        let _ = render(&mut other, &[], 1);
        assert!((other.param(1) - v).abs() < 1e-6);
        let presets = mpfree_plughost::presets(&Program::new(&exe()), &d).expect("presets");
        assert_eq!(presets.presets.len(), 1, "the plugin's one program");
        let p = &presets.presets[0];
        assert!(p.is_for(id) && p.is_factory());
        assert!(ctl.load_preset(p).is_ok());
    }
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn a_folder_holding_another_plugin_is_refused() {
    let r = root("wrong");
    let d = portable(&r, "MPFree Tests - VST - Test Delay", false);
    let e = spawn(&exe(), &d, "MTsq.test-squelch")
        .err()
        .expect("refused");
    assert!(e.to_string().contains("not MTsq.test-squelch"), "{e}");
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn the_rack_finds_mpc_plugins_where_people_unpack_them() {
    let r = root("rack");
    // A copied "VST2 Plugins" folder and a catalog release folder.
    portable(
        &r.join("VST2 Plugins"),
        "MPFree Tests - VST - Test Delay",
        false,
    );
    std::fs::create_dir_all(r.join("Test-Squelch-1.0.0/portable")).unwrap();
    portable(
        &r.join("Test-Squelch-1.0.0/portable"),
        "MPFree Tests - VST - Test Squelch",
        true,
    );
    // Something that isn't a plugin: left out with a line.
    std::fs::create_dir_all(r.join("Broken - VST - Nope")).unwrap();
    std::fs::write(r.join("Broken - VST - Nope/nope.so"), b"not ELF").unwrap();
    let (mut rack, notes) = Rack::scan(Program::new(&exe()), &r, "plugins/");
    rack.set_deadline(PATIENT);
    let ids: Vec<&str> = rack.entries().iter().map(|e| e.node_id).collect();
    assert_eq!(
        ids,
        ["mpc:MTsq.test-squelch", "mpc:MTdl.test-delay"],
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.contains("Broken - VST - Nope left out")),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.contains("1 effect(s) and 1 instrument(s)")),
        "{notes:?}"
    );
    assert!(Rack::is_plugin("mpc:MTdl.test-delay"));
    let mut fx = rack.make("mpc:MTdl.test-delay").expect("make");
    fx.prepare(RATE, 128);
    let out = render(fx.as_mut(), &[], 4);
    assert!(out.iter().any(|v| v.abs() > 1e-3));
    let (inst, _node) = rack.make_instance("mpc:MTsq.test-squelch").expect("make");
    let presets = rack.presets("mpc:MTsq.test-squelch").expect("presets");
    assert_eq!(presets.len(), 1);
    rack.load_preset(inst, &presets[0]).expect("program");
    let (values, texts) = rack.values(inst).expect("values");
    assert!(!values.is_empty() && texts.len() == values.len());
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn a_plugin_installed_while_running_is_offered_at_once() {
    let r = root("add");
    // No plugins at start: the rack still keeps the plugin process.
    let (mut rack, notes) = Rack::scan(Program::new(&exe()), &r, "plugins/");
    rack.set_deadline(PATIENT);
    assert!(rack.entries().is_empty() && notes.is_empty(), "{notes:?}");
    let d = portable(&r, "MPFree Tests - VST - Test Delay", false);
    let (found, lines) = rack.add(&d).expect("add");
    assert_eq!(found.len(), 1, "{lines:?}");
    assert_eq!(found[0].node_id, "mpc:MTdl.test-delay");
    assert!(lines.iter().any(|l| l.contains("added from")), "{lines:?}");
    assert_eq!(rack.effects().count(), 1);
    assert!(rack.find("mpc:MTdl.test-delay").is_some());
    let mut fx = rack.make("mpc:MTdl.test-delay").expect("make");
    fx.prepare(RATE, 128);
    assert!(render(fx.as_mut(), &[], 4).iter().any(|v| v.abs() > 1e-3));
    // Installed again (an update in place): the same entry, not a second.
    let (again, _) = rack.add(&d).expect("add again");
    assert_eq!(again[0].node_id, "mpc:MTdl.test-delay");
    assert_eq!(rack.entries().len(), 1);
    // Not a plugin: refused with the reason.
    let bad = r.join("Broken - VST - Nope");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("nope.so"), b"not ELF").unwrap();
    assert!(rack.add(&bad).unwrap_err().contains("left out"));
    assert_eq!(rack.entries().len(), 1);
    let _ = std::fs::remove_dir_all(&r);
}
