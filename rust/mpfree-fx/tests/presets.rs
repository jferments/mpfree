//! Every effect's factory presets: each value names a parameter the effect
//! has, once, in its range; each preset changes something; no preset sets Mix,
//! Quality or a meter (the effect page keeps those); names are unique; and
//! each preset plays, finite and audible, without running away.

use mpfree_fx::mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_fx::{
    presets, Character, Chorus, Cloud, Comb, Compressor, Crush, Delay, DjFilter, Drive, Ducker, Eq,
    Flanger, Harmonizer, LoFi, MasterSafety, MultiFilter, Perform, Phaser, PitchCorrect,
    PitchShifter, TransientShaper, Tuner,
};

const RATE: u32 = 48_000;

fn nodes() -> Vec<Box<dyn Node>> {
    vec![
        Box::new(Eq::new()),
        Box::new(MultiFilter::new()),
        Box::new(DjFilter::new()),
        Box::new(Drive::new()),
        Box::new(Comb::new()),
        Box::new(Crush::new()),
        Box::new(Delay::new()),
        Box::new(Chorus::new()),
        Box::new(Flanger::new()),
        Box::new(Phaser::new()),
        Box::new(LoFi::new()),
        Box::new(Compressor::new()),
        Box::new(Cloud::new()),
        Box::new(MasterSafety::new()),
        Box::new(Perform::new()),
        Box::new(Character::new()),
        Box::new(Ducker::new()),
        Box::new(TransientShaper::new()),
        Box::new(PitchShifter::new()),
        Box::new(PitchCorrect::new()),
        Box::new(Harmonizer::new()),
        Box::new(Tuner::new()),
    ]
}

/// Noise in bursts (a beat's worth of hits), the same every run.
fn input(frames: usize) -> Vec<f32> {
    let mut seed = 7u32;
    (0..frames)
        .map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            if i % (RATE as usize / 2) < RATE as usize / 8 {
                0.4 * x
            } else {
                0.0
            }
        })
        .collect()
}

#[test]
fn every_effect_with_a_sound_to_pick_has_presets() {
    for n in nodes() {
        let id = n.info().id;
        let none = matches!(
            id,
            "mpfree.fx.dj_filter"
                | "mpfree.fx.master_safety"
                | "mpfree.fx.perform"
                | "mpfree.fx.tuner"
        );
        assert_eq!(presets(id).is_empty(), none, "{id}");
        if !none {
            assert!(presets(id).len() >= 4, "{id} has {}", presets(id).len());
        }
    }
    assert!(presets("mpfree.fx.nothing").is_empty());
}

#[test]
fn presets_are_in_range_and_change_something() {
    for n in nodes() {
        let id = n.info().id;
        let params = n.params();
        let list = presets(id);
        for (i, p) in list.iter().enumerate() {
            assert!(
                list[..i].iter().all(|q| q.name != p.name),
                "{id}: {} twice",
                p.name
            );
            // A preset may name a default (the filter type it is built on),
            // but it must change something.
            assert!(
                p.values
                    .iter()
                    .any(|&(pid, v)| params.iter().any(|q| q.id == pid && q.default != v)),
                "{id}: {} is the defaults",
                p.name
            );
            for (k, &(pid, v)) in p.values.iter().enumerate() {
                let info = params
                    .iter()
                    .find(|q| q.id == pid)
                    .unwrap_or_else(|| panic!("{id}: {} names id {pid}", p.name));
                assert!(
                    p.values[..k].iter().all(|&(q, _)| q != pid),
                    "{id}: {} sets {} twice",
                    p.name,
                    info.name
                );
                assert!(!info.readonly, "{id}: {} sets a meter", p.name);
                assert!(
                    !matches!(info.name, "Mix" | "Quality"),
                    "{id}: {} sets {}, which the page keeps",
                    p.name,
                    info.name
                );
                assert_eq!(
                    info.clamp(v),
                    v,
                    "{id}: {}: {} = {v} is in range",
                    p.name,
                    info.name
                );
            }
        }
    }
}

#[test]
fn every_preset_plays() {
    let frames = 2 * RATE as usize;
    let x = input(frames);
    for n in nodes() {
        let id = n.info().id;
        for p in presets(id) {
            let mut node = mpfree_fx_node(id);
            node.prepare(RATE, MAX_BLOCK);
            for &(pid, v) in p.values {
                node.set_param(pid, v);
            }
            // Effects that mix keep their default Mix: a preset only sets
            // the sound.
            node.reset();
            let (mut l, mut r) = (x.clone(), x.iter().map(|v| -0.8 * v).collect::<Vec<_>>());
            let mut at = 0;
            while at < frames {
                let len = 128.min(frames - at);
                let ctx = Ctx {
                    bpm: 120.0,
                    beats: at as f64 * 2.0 / RATE as f64,
                    playing: true,
                    ..Ctx::new(RATE, at as u64)
                };
                node.process(&ctx, &[], &mut l[at..at + len], &mut r[at..at + len]);
                at += len;
            }
            assert!(
                l.iter().chain(&r).all(|v| v.is_finite()),
                "{id}: {} is finite",
                p.name
            );
            let peak = l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()));
            let rms = (l.iter().map(|v| v * v).sum::<f32>() / frames as f32).sqrt();
            println!("{id}: {:<18} peak {peak:.2}, rms {rms:.3}", p.name);
            assert!(peak < 4.0, "{id}: {} peaks at {peak}", p.name);
            assert!(rms > 0.01, "{id}: {} is audible ({rms})", p.name);
        }
    }
}

/// A fresh node by id.
fn mpfree_fx_node(id: &str) -> Box<dyn Node> {
    nodes()
        .into_iter()
        .find(|n| n.info().id == id)
        .expect("a node with that id")
}
