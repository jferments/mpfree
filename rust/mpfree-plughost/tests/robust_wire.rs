//! Robustness: the host reads whatever a plugin process writes, and a
//! plugin process is exactly what the sandbox does not trust. Every damaged
//! message is answered with an error or a message the host can use: never a
//! panic, and never a parameter description that panics when a value is set
//! (`f32::clamp` panics on a range whose ends are NaN or the wrong way round).

#[path = "../../test-support/mutate.rs"]
mod mutate;

use mpfree_clap::{intern_params, CurveSpec, ParamSpec};
use mpfree_node::Kind;
use mpfree_plughost::wire::{
    read_to_child, read_to_host, send_to_child, send_to_host, Hello, ToChild, ToHost,
};
use mutate::Mutator;

fn spec(name: &str, min: f32, max: f32, default: f32, curve: CurveSpec) -> ParamSpec {
    ParamSpec {
        name: name.into(),
        group: "G".into(),
        min,
        max,
        default,
        curve,
        readonly: false,
    }
}

fn to_host() -> Vec<Vec<u8>> {
    let hello = Hello {
        id: "org.example.fuzz".into(),
        name: "Fuzz".into(),
        kind: Kind::Effect,
        params: vec![
            spec("Cutoff", 20.0, 20_000.0, 1_000.0, CurveSpec::Linear),
            spec(
                "Mode",
                0.0,
                2.0,
                0.0,
                CurveSpec::Choice(vec!["a".into(), "b".into(), "c".into()]),
            ),
            spec("On", 0.0, 1.0, 1.0, CurveSpec::Toggle),
            spec("Steps", -4.0, 4.0, 0.0, CurveSpec::Stepped),
        ],
    };
    [
        ToHost::Hello(hello),
        ToHost::Prepared {
            ok: true,
            latency: 3,
            tail: 900,
        },
        ToHost::State(Some(vec![9; 40])),
        ToHost::State(None),
        ToHost::Loaded(true),
        ToHost::Text(Some("Saw".into())),
        ToHost::Failed("no".into()),
    ]
    .iter()
    .map(|m| {
        let mut buf = Vec::new();
        send_to_host(&mut buf, m).unwrap();
        buf
    })
    .collect()
}

fn to_child() -> Vec<Vec<u8>> {
    [
        ToChild::Prepare {
            rate: 44_100,
            max_block: 256,
        },
        ToChild::SaveState,
        ToChild::LoadState(vec![1, 2, 3, 4]),
        ToChild::ParamText {
            index: 7,
            value: -3.5,
        },
        ToChild::Quit,
    ]
    .iter()
    .map(|m| {
        let mut buf = Vec::new();
        send_to_child(&mut buf, m).unwrap();
        buf
    })
    .collect()
}

/// What the host does with a plugin's parameters: intern them and set
/// values, edge cases included.
fn use_params(case: usize, params: Vec<ParamSpec>) {
    let infos = intern_params(&format!("fuzz{case}"), params);
    for p in infos {
        for v in [
            f32::NAN,
            f32::NEG_INFINITY,
            -1e30,
            -1.0,
            0.0,
            0.5,
            1.0,
            1e30,
            f32::INFINITY,
        ] {
            let x = p.clamp(v);
            assert!(x.is_finite(), "case {case}: {} clamps {v} to {x}", p.name);
        }
    }
}

#[test]
fn damaged_messages_from_a_plugin_are_refused_or_usable() {
    let seeds = to_host();
    let mut m = Mutator::new(5);
    let (mut ok, mut refused) = (0, 0);
    for case in 0..20_000 {
        let input = m.mutate(&seeds[case % seeds.len()]);
        match read_to_host(&mut input.as_slice()) {
            Ok(msg) => {
                ok += 1;
                if let ToHost::Hello(h) = msg {
                    use_params(case, h.params);
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(ok > 1_000 && refused > 1_000, "ok {ok}, refused {refused}");
}

#[test]
fn damaged_messages_to_a_plugin_are_refused_or_read() {
    let seeds = to_child();
    let mut m = Mutator::new(6);
    for case in 0..20_000 {
        let input = m.mutate(&seeds[case % seeds.len()]);
        let _ = read_to_child(&mut input.as_slice());
    }
}

#[test]
fn any_parameter_range_a_plugin_reports_is_safe_to_set() {
    let edges = [
        f32::NAN,
        f32::NEG_INFINITY,
        -1.0,
        0.0,
        1.0,
        5.0,
        f32::INFINITY,
    ];
    let mut case = 0;
    for &min in &edges {
        for &max in &edges {
            for &default in &edges {
                for curve in [CurveSpec::Linear, CurveSpec::Stepped, CurveSpec::Toggle] {
                    use_params(case, vec![spec("P", min, max, default, curve)]);
                    case += 1;
                }
            }
        }
    }
}
