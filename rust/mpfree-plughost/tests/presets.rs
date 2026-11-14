//! A plugin's own presets through the rack: listed by a process of their
//! own (once per file), loaded into a running plugin by its instance
//! number, with the plugin's home folder where the rack's program says.

use mpfree_clap::testplug::{HOME_PRESETS, PRESETS_ENV};
use mpfree_clap::{Place, Preset};
use mpfree_node::{Ctx, Node};
use mpfree_plughost::child::preset_line;
use mpfree_plughost::rack::Rack;
use mpfree_plughost::{parse_preset_line, parse_presets, Deadline, Program};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SYNTH: &str = "clap:org.mpfree.test-synth";
const GAIN: &str = "clap:org.mpfree.test-gain";

fn exe() -> PathBuf {
    std::env::var_os("MPFREE_PLUGHOST_TEST_EXE").map_or_else(
        || PathBuf::from(env!("CARGO_BIN_EXE_mpfree-plughost")),
        PathBuf::from,
    )
}

/// The test plugins' shared library (as in tests/rack.rs).
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

fn param(n: &dyn Node, name: &str) -> u16 {
    n.params().iter().find(|p| p.name == name).expect(name).id
}

#[test]
fn presets_are_listed_once_and_load_into_a_running_plugin() {
    assert!(std::env::var_os(PRESETS_ENV).is_none());
    let d = std::env::temp_dir().join(format!("mpfree-rack-presets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let (plugins, home) = (d.join("plugins"), d.join("home"));
    std::fs::create_dir_all(&plugins).unwrap();
    std::fs::create_dir_all(home.join(HOME_PRESETS)).unwrap();
    std::fs::copy(plugin_file(), plugins.join("a-test.clap")).unwrap();
    std::fs::write(
        home.join(HOME_PRESETS).join("Lead.mptp"),
        mpfree_clap::testplug::preset_file(-4.0, 700.0, 2.0),
    )
    .unwrap();

    // The plugin finds its user presets in the home the program gives it.
    let program = Program::new(&exe()).with_home(&home);
    let (mut rack, _) = Rack::scan(program, &plugins, "the folder");
    rack.set_deadline(Deadline::Fixed(Duration::from_secs(1)));
    rack.take_lines();
    let list = rack.presets(SYNTH).unwrap();
    let names: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Soft Sine", "Bright Saw", "Short Square", "Lead"]);
    assert_eq!(
        list[3].place,
        Place::File(home.join(HOME_PRESETS).join("Lead.mptp"))
    );
    assert_eq!(list[1].category, "Leads");
    let lines = rack.take_lines().unwrap_or_default();
    assert_eq!(lines, ["plugins: 4 preset(s) in a-test.clap"]);
    // Kept: no second listing, no second line.
    assert_eq!(rack.presets(SYNTH).unwrap(), list);
    assert!(rack.take_lines().is_none());
    assert!(rack.presets(GAIN).unwrap().is_empty(), "the synth's only");
    assert!(rack.presets("clap:nope").is_err());

    // Loading one into the running synth: its values come back with the
    // next blocks.
    let (instance, mut node) = rack.make_instance(SYNTH).unwrap();
    node.prepare(44_100, 128);
    let (wave, decay) = (param(node.as_ref(), "Wave"), param(node.as_ref(), "Decay"));
    let mut l = vec![0f32; 128];
    let mut r = vec![0f32; 128];
    node.process(&Ctx::new(44_100, 0), &[], &mut l, &mut r);
    assert_eq!(node.param(wave), 0.0);
    rack.load_preset(instance, &list[3]).unwrap();
    for k in 1..4 {
        node.process(&Ctx::new(44_100, k * 128), &[], &mut l, &mut r);
    }
    assert_eq!((node.param(wave), node.param(decay)), (2.0, 700.0));
    rack.load_preset(instance, &list[1]).unwrap();
    node.process(&Ctx::new(44_100, 512), &[], &mut l, &mut r);
    assert_eq!((node.param(wave), node.param(decay)), (1.0, 300.0));

    // What can't load says why.
    let bogus = Preset {
        load_key: Some("nope".into()),
        ..list[0].clone()
    };
    assert_eq!(
        rack.load_preset(instance, &bogus),
        Err("no such preset".to_string())
    );
    assert!(rack.load_preset(instance + 99, &list[0]).is_err());
    let (other, _n2) = rack.make_instance(SYNTH).unwrap();
    assert_ne!(other, instance);
    drop(node);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn preset_lines_read_back_and_junk_is_skipped() {
    let p = Preset {
        name: "Tab\there\\ and\nline".into(),
        place: Place::File(Path::new("/data/Surge XT/Leads/A b.fxp").into()),
        load_key: None,
        plugins: vec!["org.a".into(), "org.b".into()],
        collection: "Factory".into(),
        category: "Leads / Mono".into(),
        creator: "Someone".into(),
        flags: 1,
    };
    let line = preset_line(&p).unwrap();
    assert_eq!(line.matches('\n').count(), 0);
    assert_eq!(parse_preset_line(&line), Some(p.clone()));
    let inside = Preset {
        place: Place::Plugin,
        load_key: Some("k1".into()),
        plugins: Vec::new(),
        ..p.clone()
    };
    assert_eq!(
        parse_preset_line(&preset_line(&inside).unwrap()),
        Some(inside.clone())
    );
    let out = format!(
        "Dexed banner on stdout\n{}\n{}\nerror\tcan't read\\tthis\nmore-errors\t3\ncut-short\npreset\tbad\n",
        line,
        preset_line(&inside).unwrap()
    );
    let f = parse_presets(&out);
    assert_eq!(f.presets, [p, inside]);
    assert_eq!(f.errors, ["can't read\tthis"]);
    assert_eq!((f.more_errors, f.cut_short), (3, true));
    assert_eq!(
        parse_preset_line("preset\t\tplugin\t\t0\t\t\t"),
        None,
        "no name"
    );
    assert_eq!(
        parse_preset_line("preset\tA\tdisk\t\t0\t\t\t"),
        None,
        "bad place"
    );
}
