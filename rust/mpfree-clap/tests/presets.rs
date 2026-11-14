//! A plugin's own presets: finding them (CLAP preset discovery) and loading
//! one (CLAP preset-load), with MPFree's test synth.
//!
//! One test only: it sets the environment variable that points the test
//! synth at its preset files, which every test in this process would see.

use mpfree_clap::presets::{MAX_DEPTH, MAX_PRESETS};
use mpfree_clap::sys::{
    CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT, CLAP_PRESET_DISCOVERY_IS_USER_CONTENT,
};
use mpfree_clap::testplug::{self, synth_params, GAIN_ID, PRESETS_ENV, SYNTH_ID};
use mpfree_clap::{request, Library, Place, Preset};
use mpfree_node::Node;
use std::path::PathBuf;

fn folder() -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-clap-presets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("Bass")).unwrap();
    std::fs::write(
        d.join("Bass/Deep.mptp"),
        testplug::preset_file(-3.0, 1500.0, 1.0),
    )
    .unwrap();
    std::fs::write(d.join("Top.mptp"), testplug::preset_file(-20.0, 90.0, 2.0)).unwrap();
    std::fs::write(d.join("broken.mptp"), b"not a preset").unwrap();
    std::fs::write(
        d.join(".hidden.mptp"),
        testplug::preset_file(0.0, 10.0, 0.0),
    )
    .unwrap();
    std::fs::write(d.join("notes.txt"), b"not a preset either").unwrap();
    // Deeper than the walk goes.
    let mut deep = d.clone();
    for k in 0..=MAX_DEPTH {
        deep = deep.join(format!("d{k}"));
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(
        deep.join("Lost.mptp"),
        testplug::preset_file(0.0, 10.0, 0.0),
    )
    .unwrap();
    d
}

#[test]
fn presets_are_found_and_loaded() {
    let dir = folder();
    std::env::set_var(PRESETS_ENV, &dir);
    let lib = Library::from_entry(&testplug::ENTRY, "linked").unwrap();
    let found = lib.presets();
    assert!(!found.cut_short);
    let names: Vec<(&str, &str, &str)> = found
        .presets
        .iter()
        .map(|p| (p.name.as_str(), p.category.as_str(), p.collection.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("Soft Sine", "Keys", "Built in"),
            ("Bright Saw", "Leads", "Built in"),
            ("Short Square", "Plucks", "Built in"),
            ("Deep", "Bass", "Your presets"),
            ("Top", "", "Your presets"),
        ],
        "built-ins first, then the files sorted; hidden, other types and too-deep files left out"
    );
    assert_eq!(found.errors, ["not a test synth preset"], "the broken file");
    let soft = &found.presets[0];
    assert_eq!(soft.place, Place::Plugin);
    assert_eq!(soft.load_key.as_deref(), Some("soft"));
    assert_eq!(soft.creator, "MPFree");
    assert_eq!(soft.flags, CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT);
    assert!(soft.is_for(SYNTH_ID) && !soft.is_for(GAIN_ID));
    let deep = &found.presets[3];
    assert_eq!(deep.place, Place::File(dir.join("Bass/Deep.mptp")));
    assert_eq!(deep.load_key, None);
    assert!(deep.is_user() && !deep.is_factory());
    assert_eq!(deep.flags, CLAP_PRESET_DISCOVERY_IS_USER_CONTENT);

    // Loading one into a running synth: its values follow at once.
    let mut node = lib.create(SYNTH_ID).unwrap();
    node.prepare(44_100, 128);
    let control = node.control();
    assert!(control.loads_presets());
    control.idle();
    let wave = node.param_index(synth_params::WAVE).unwrap();
    let decay = node.param_index(synth_params::DECAY).unwrap();
    assert_eq!(node.param(wave), 0.0);
    control.load_preset(&found.presets[1]).unwrap();
    assert_eq!((node.param(wave), node.param(decay)), (1.0, 300.0));
    let bits = control.idle();
    assert_eq!(
        bits & (request::PRESET_LOADED | request::PARAMS_VALUES | request::PARAMS_RESCAN),
        request::PRESET_LOADED | request::PARAMS_VALUES,
        "a values rescan doesn't ask for the node to be made again"
    );
    control.load_preset(deep).unwrap();
    assert_eq!((node.param(wave), node.param(decay)), (1.0, 1500.0));
    // The state after a preset is the preset's.
    let state = control.save_state().unwrap();
    assert_eq!(state, testplug::preset_file(-3.0, 1500.0, 1.0));

    // A preset the plugin doesn't know: its own message.
    let bogus = Preset {
        load_key: Some("nope".into()),
        ..found.presets[0].clone()
    };
    assert_eq!(
        control.load_preset(&bogus),
        Err("no such preset".to_string())
    );
    assert_eq!(node.param(decay), 1500.0, "nothing changed");
    let gone = Preset {
        place: Place::File(dir.join("missing.mptp")),
        load_key: None,
        ..found.presets[0].clone()
    };
    assert!(control.load_preset(&gone).is_err());

    // The gain effect loads no presets.
    let gain = lib.create(GAIN_ID).unwrap();
    assert!(!gain.control().loads_presets());
    assert!(gain.control().load_preset(soft).is_err());

    // Without the folder, only the built-ins.
    std::env::remove_var(PRESETS_ENV);
    assert_eq!(lib.presets().presets.len(), 3);
    assert!(MAX_PRESETS >= 1024);
    let _ = std::fs::remove_dir_all(&dir);
}
