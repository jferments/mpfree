//! Quality modes (R7 step C5): Clean stays bit for bit, Tape wobbles,
//! Lo-fi holds and steps, both keep the level, a switch never clicks, and
//! none depends on the period size.

use mpfree_grain::node::{param, QUALITY_NAMES};
use mpfree_grain::*;
use mpfree_node::{Curve, Node};

const RATE: u32 = 44_100;

fn render(v: &mut GrainVoice, src: &Clip, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let mut budget = Budget::new(256);
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    let mut at = 0;
    while at < frames {
        let n = block.min(frames - at);
        v.render(
            src,
            &mut budget,
            at as u64,
            &mut l[at..at + n],
            &mut r[at..at + n],
        );
        at += n;
    }
    (l, r)
}

/// A noisy, pitched test sound: a few partials and some hash.
fn sound(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let hash = ((i * 7919) % 1013) as f32 / 1013.0 - 0.5;
            0.3 * (std::f32::consts::TAU * 220.0 * t).sin()
                + 0.15 * (std::f32::consts::TAU * 1370.0 * t).sin()
                + 0.1 * hash
        })
        .collect()
}

fn sine(frames: usize, hz: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| 0.5 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
        .collect()
}

fn cloud(quality: Quality) -> Params {
    Params {
        position: 0.3,
        spray: 0.3,
        size_ms: 60.0,
        density: 40.0,
        spread: 0.5,
        quality,
        ..Params::default()
    }
}

/// A time stretch at the sound's own speed: grains in step, so the
/// output is the sound itself (a sine stays a sine).
fn stretch(quality: Quality) -> Params {
    Params {
        size_ms: 80.0,
        density: 25.0,
        scan: 1.0,
        timing: 0.0,
        quality,
        ..Params::default()
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.log10()
}

#[test]
fn clean_is_the_default_and_a_round_trip_comes_back_bit_for_bit() {
    assert_eq!(Params::default().quality, Quality::Clean);
    let data = sound(RATE as usize * 3);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut plain = GrainVoice::new(RATE, 4);
    plain.set_params(cloud(Quality::Clean));
    plain.start();
    let a = render(&mut plain, &src, 30_000, 128).0;

    // Tape from frame 6,400 to 12,800, then Clean again.
    let mut v = GrainVoice::new(RATE, 4);
    v.set_params(cloud(Quality::Clean));
    v.start();
    let mut b = render(&mut v, &src, 6_400, 128).0;
    v.set_params(cloud(Quality::Tape));
    b.extend(render(&mut v, &src, 6_400, 128).0);
    v.set_params(cloud(Quality::Clean));
    b.extend(render(&mut v, &src, 30_000 - 12_800, 128).0);

    assert_eq!(a[..6_400], b[..6_400]);
    assert_ne!(a[6_400..12_800], b[6_400..12_800]);
    // 10 ms after the switch back, exactly as if it had never left.
    let back = 12_800 + RATE as usize / 100;
    assert_eq!(a[back..], b[back..]);
}

#[test]
fn tape_and_lofi_are_the_same_at_any_period_size() {
    let data = sound(RATE as usize * 3);
    let src = Clip::new(&data, 1, RATE).unwrap();
    for q in [Quality::Tape, Quality::LoFi] {
        let go = |block| {
            let mut v = GrainVoice::new(RATE, 9);
            v.set_params(cloud(q));
            v.start();
            render(&mut v, &src, 20_000, block)
        };
        let (al, ar) = go(128);
        for block in [1, 37, 64, 333, 4096] {
            let (bl, br) = go(block);
            let worst = al
                .iter()
                .zip(&bl)
                .chain(ar.iter().zip(&br))
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f32::max);
            assert!(worst < 1e-4, "{q:?} block {block}: {worst}");
        }
    }
}

#[test]
fn tape_and_lofi_keep_the_clouds_level() {
    let data = sound(RATE as usize * 3);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let level = |q| {
        let mut v = GrainVoice::new(RATE, 3);
        v.set_params(cloud(q));
        v.start();
        let (l, r) = render(&mut v, &src, RATE as usize * 2, 128);
        let skip = RATE as usize / 4;
        db((rms(&l[skip..]) + rms(&r[skip..])) / 2.0)
    };
    let clean = level(Quality::Clean);
    for q in [Quality::Tape, Quality::LoFi] {
        let d = level(q) - clean;
        assert!(d.abs() < 1.0, "{q:?}: {d:+.2} dB");
    }
}

/// The frequency of `x` over each window of `win` frames, from its
/// rising zero crossings (between frames).
fn frequencies(x: &[f32], win: usize) -> Vec<f64> {
    let crossings: Vec<f64> = (1..x.len())
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f64 + f64::from(-x[i - 1] / (x[i] - x[i - 1])))
        .collect();
    let mut out = Vec::new();
    let mut at = 0.0;
    while at + win as f64 <= x.len() as f64 {
        let inside: Vec<f64> = crossings
            .iter()
            .copied()
            .filter(|&c| c >= at && c < at + win as f64)
            .collect();
        if inside.len() > 2 {
            let span = inside[inside.len() - 1] - inside[0];
            out.push((inside.len() - 1) as f64 / span * f64::from(RATE));
        }
        at += win as f64;
    }
    out
}

#[test]
fn tape_wobbles_the_pitch_by_a_few_cents_and_clean_holds_it() {
    let data = sine(RATE as usize * 6, 1000.0);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let spread = |q| {
        let mut v = GrainVoice::new(RATE, 1);
        v.set_params(stretch(q));
        v.start();
        let (l, _) = render(&mut v, &src, RATE as usize * 5, 128);
        let f = frequencies(&l[RATE as usize / 2..], RATE as usize / 50);
        let lo = f.iter().copied().fold(f64::MAX, f64::min);
        let hi = f.iter().copied().fold(0.0, f64::max);
        1200.0 * (hi / lo).log2()
    };
    let clean = spread(Quality::Clean);
    let tape = spread(Quality::Tape);
    assert!(clean < 1.0, "clean: {clean:.2} cents");
    // About ±5 cents of wow, a little flutter on top.
    assert!((5.0..16.0).contains(&tape), "tape: {tape:.2} cents");
}

#[test]
fn lofi_holds_each_sample_for_four_frames_on_the_voices_clock() {
    let data = sound(RATE as usize * 2);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 5);
    v.set_params(cloud(Quality::LoFi));
    v.start();
    let (l, _) = render(&mut v, &src, 8_192, 100);
    // Past the 10 ms fade in from Clean.
    for i in (1_024..8_192).step_by(4) {
        assert!(l[i..i + 4].iter().all(|x| *x == l[i]), "frame {i}");
    }
    // ... and stepped to 8 bits: few distinct levels near zero.
    let quiet: Vec<u32> = l[1_024..]
        .iter()
        .filter(|x| x.abs() < 0.01)
        .map(|x| x.to_bits())
        .collect();
    let mut kinds = quiet.clone();
    kinds.sort_unstable();
    kinds.dedup();
    assert!(kinds.len() < 40, "{} levels under 0.01", kinds.len());
}

#[test]
fn a_switch_while_it_plays_never_clicks() {
    let data = sine(RATE as usize * 4, 220.0);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 2);
    v.set_params(stretch(Quality::Clean));
    v.start();
    let mut out = render(&mut v, &src, 8_000, 128).0;
    // Every switch, one after another, and two inside one crossfade.
    for q in [Quality::Tape, Quality::LoFi, Quality::Clean, Quality::LoFi] {
        v.set_params(stretch(q));
        out.extend(render(&mut v, &src, 3_000, 128).0);
    }
    v.set_params(stretch(Quality::Tape));
    out.extend(render(&mut v, &src, 100, 128).0);
    v.set_params(stretch(Quality::Clean));
    out.extend(render(&mut v, &src, 4_000, 128).0);
    // The sine moves at most 2 pi 220 / 44100 of its peak a frame; the
    // lo-fi's steps of four frames move up to four times that, and its 8
    // bits a step more.
    let peak = out[RATE as usize / 10..]
        .iter()
        .fold(0.0, |m: f32, x| m.max(x.abs()));
    let step = 4.0 * std::f32::consts::TAU * 220.0 / RATE as f32 * peak * 1.2;
    let worst = out[RATE as usize / 10..]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(worst < step, "{worst} (at most {step}, peak {peak})");
}

#[test]
fn a_killed_voice_starts_again_with_nothing_held() {
    let data = sound(RATE as usize * 2);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut budget = Budget::new(256);
    let mut v = GrainVoice::new(RATE, 6);
    v.set_params(cloud(Quality::Tape));
    v.start();
    let _ = render(&mut v, &src, 4_000, 128);
    v.kill(&mut budget);
    let (l, r) = render(&mut v, &src, 512, 128);
    assert!(l.iter().chain(&r).all(|x| *x == 0.0));
}

#[test]
fn the_grains_effect_has_a_quality_choice() {
    let mut node = GrainNode::new(1);
    let info = node
        .params()
        .iter()
        .find(|p| p.id == param::QUALITY)
        .unwrap();
    assert_eq!(info.name, "Quality");
    assert_eq!(info.default, 0.0);
    assert!(matches!(info.curve, Curve::Choice(n) if n == QUALITY_NAMES));
    assert_eq!(
        Quality::ALL.map(Quality::name).as_slice(),
        QUALITY_NAMES,
        "the page's names are the engine's"
    );
    node.prepare(RATE, 128);
    for (i, q) in Quality::ALL.iter().enumerate() {
        node.set_param(param::QUALITY, i as f32);
        assert_eq!(node.fx().unwrap().params().quality, *q);
    }
    // A capture onto a pad keeps it.
    let p = node::voice_params(&[(param::QUALITY, 2.0)]);
    assert_eq!(p.quality, Quality::LoFi);
    assert_eq!(node::voice_params(&[]).quality, Quality::Clean);
}
