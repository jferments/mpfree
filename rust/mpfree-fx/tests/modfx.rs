//! Chorus, flanger and phaser: Mix 0 is the input, each does what its
//! controls say (delay, voices and width; the feedback comb; the phaser's
//! notches), count changes never click, and the tail estimate covers the
//! ring-out.

use std::f32::consts::PI;

use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::*;

const RATE: u32 = 48_000;

fn set_up(mut node: Box<dyn Node>, values: &[(u16, f32)]) -> Box<dyn Node> {
    node.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        node.set_param(id, v);
    }
    node.reset();
    node
}

fn run(node: &mut dyn Node, l: &mut [f32], r: &mut [f32], moves: &[(usize, u16, f32)]) {
    let mut at = 0;
    for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
        let ev: Vec<Timed> = moves
            .iter()
            .filter(|m| m.0 >= at && m.0 < at + a.len())
            .map(|m| {
                Timed::new(
                    (m.0 - at) as u32,
                    NodeEvent::Param {
                        id: m.1,
                        value: m.2,
                    },
                )
            })
            .collect();
        node.process(&Ctx::new(RATE, at as u64), &ev, a, b);
        at += a.len();
    }
}

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len() as f64).sqrt() as f32
}

fn noise(frames: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..frames)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 9) as f32 / (1u32 << 22) as f32 - 1.0
        })
        .collect()
}

type Make<'a> = &'a dyn Fn() -> Box<dyn Node>;
type Case = (&'static str, Box<dyn Node>, Vec<(u16, f32)>);

fn roughness(x: &[f32]) -> f32 {
    x.windows(3)
        .fold(0.0f32, |m, w| m.max((w[2] - 2.0 * w[1] + w[0]).abs()))
}

fn all() -> Vec<(&'static str, Box<dyn Node>, u16)> {
    vec![
        ("chorus", Box::new(Chorus::new()), ids::chorus::MIX),
        ("flanger", Box::new(Flanger::new()), ids::flanger::MIX),
        ("phaser", Box::new(Phaser::new()), ids::phaser::MIX),
    ]
}

#[test]
fn mix_zero_is_the_input_exactly() {
    for (name, node, mix) in all() {
        let mut n = set_up(node, &[(mix, 0.0)]);
        let x = noise(20_000, 3);
        let y = noise(20_000, 4);
        let (mut l, mut r) = (x.clone(), y.clone());
        run(n.as_mut(), &mut l, &mut r, &[]);
        assert!(l == x && r == y, "{name}");
    }
}

#[test]
fn the_chorus_delays_by_its_delay_and_width_zero_is_mono() {
    use ids::chorus::*;
    let mut c = set_up(
        Box::new(Chorus::new()),
        &[
            (DEPTH, 0.0),
            (VOICES, 1.0),
            (DELAY, 10.0),
            (TONE, 20_000.0),
            (MIX, 100.0),
        ],
    );
    let mut l = vec![0.0f32; 2000];
    l[100] = 1.0;
    let mut r = l.clone();
    run(c.as_mut(), &mut l, &mut r, &[]);
    let at = (0..l.len())
        .max_by(|&a, &b| l[a].abs().total_cmp(&l[b].abs()))
        .unwrap();
    assert_eq!(at, 100 + 480, "10 ms at 48 kHz");

    let x = noise(48_000, 9);
    for (width, mono) in [(0.0, true), (100.0, false)] {
        let mut c = set_up(
            Box::new(Chorus::new()),
            &[(WIDTH, width), (VOICES, 3.0), (DEPTH, 80.0), (MIX, 100.0)],
        );
        let (mut l, mut r) = (x.clone(), x.clone());
        run(c.as_mut(), &mut l, &mut r, &[]);
        if mono {
            assert!(l == r, "width 0 on a mono input stays mono");
        } else {
            let (a, b) = (&l[24_000..], &r[24_000..]);
            let corr = a.iter().zip(b).map(|(p, q)| p * q).sum::<f32>()
                / (rms(a) * rms(b) * a.len() as f32);
            assert!(
                corr < 0.9,
                "full width spreads the sides: correlation {corr}"
            );
        }
    }
}

#[test]
fn the_flanger_feedback_is_a_comb() {
    use ids::flanger::*;
    for fb in [80.0f32, -80.0] {
        let mut f = set_up(
            Box::new(Flanger::new()),
            &[(DEPTH, 0.0), (DELAY, 2.0), (FEEDBACK, fb), (MIX, 100.0)],
        );
        let mut l = vec![0.0f32; 1000];
        l[0] = 1.0;
        let mut r = l.clone();
        run(f.as_mut(), &mut l, &mut r, &[]);
        // Echoes every 2 ms (96 frames), each the last times the feedback.
        for k in 1..5 {
            let want = (fb / 100.0).powi(k as i32 - 1);
            assert!(
                (l[96 * k] - want).abs() < 1e-3,
                "feedback {fb}: echo {k} is {}, want {want}",
                l[96 * k]
            );
        }
    }
}

#[test]
fn the_phaser_notches_where_the_stages_turn_half_a_cycle() {
    use ids::phaser::*;
    // Four stages at 1 kHz: each turns 45 degrees at 414 Hz and 135 at
    // 2414 Hz, 180 in all, so the half-wet mix cancels there; at 1 kHz
    // they turn a whole cycle and add.
    let gain_at = |hz: f32| {
        let mut p = set_up(
            Box::new(Phaser::new()),
            &[
                (DEPTH, 0.0),
                (CENTER, 1000.0),
                (STAGES, 1.0),
                (FEEDBACK, 0.0),
                (MIX, 50.0),
            ],
        );
        let x = sine(hz, 0.5, 24_000);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(p.as_mut(), &mut l, &mut r, &[]);
        20.0 * (rms(&l[12_000..]) / rms(&x[12_000..])).log10()
    };
    assert!(
        gain_at(414.2) < -40.0,
        "notch at 414 Hz: {}",
        gain_at(414.2)
    );
    assert!(
        gain_at(2414.2) < -40.0,
        "notch at 2414 Hz: {}",
        gain_at(2414.2)
    );
    assert!(
        gain_at(1000.0).abs() < 0.05,
        "in phase at the centre: {}",
        gain_at(1000.0)
    );
}

#[test]
fn voice_and_stage_changes_do_not_click() {
    // The largest second difference of a low sine through the effect: a
    // hard switch between settings shows as a spike far above what the
    // effect makes holding still, while a glide's corner stays near it.
    let x = sine(110.0, 0.5, 48_000);
    let render = |make: Make, values: &[(u16, f32)], moves: &[(usize, u16, f32)]| {
        let mut n = set_up(make(), values);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(n.as_mut(), &mut l, &mut r, moves);
        l
    };
    use ids::{chorus, phaser};
    let chorus = || -> Box<dyn Node> { Box::new(Chorus::new()) };
    let phaser = || -> Box<dyn Node> { Box::new(Phaser::new()) };
    let cases: [(&str, Make, u16, Vec<f32>); 2] = [
        ("chorus", &chorus, chorus::VOICES, vec![2.0, 4.0, 1.0, 3.0]),
        (
            "phaser",
            &phaser,
            phaser::STAGES,
            vec![1.0, 4.0, 0.0, 2.0, 3.0],
        ),
    ];
    for (name, make, id, values) in cases {
        let stills: Vec<Vec<f32>> = values
            .iter()
            .map(|&v| render(make, &[(id, v)], &[]))
            .collect();
        // From after the delay line has filled (the wet copy starting is
        // not a change).
        let still = stills
            .iter()
            .map(|s| roughness(&s[4000..]))
            .fold(0.0f32, f32::max);
        let moves: Vec<(usize, u16, f32)> = values[1..]
            .iter()
            .enumerate()
            .map(|(k, &v)| (8_000 * (k + 1), id, v))
            .collect();
        let moving = roughness(&render(make, &[(id, values[0])], &moves)[4000..]);
        // The same changes as hard cuts between the still renders.
        let spliced: Vec<f32> = (0..x.len())
            .map(|i| stills[(i / 8_000).min(values.len() - 1)][i])
            .collect();
        let hard = roughness(&spliced[4000..]);
        assert!(hard > 0.01, "{name}: a hard switch is visible ({hard})");
        assert!(
            moving < still + 0.1 * (hard - still),
            "{name}: roughness {moving} changing, {still} holding still, {hard} switching hard"
        );
    }
}

#[test]
fn the_tail_estimate_covers_the_ring_out() {
    use ids::{chorus, flanger, phaser};
    let cases: [Case; 4] = [
        (
            "chorus",
            Box::new(Chorus::new()),
            vec![(chorus::DELAY, 40.0), (chorus::DEPTH, 100.0)],
        ),
        (
            "flanger",
            Box::new(Flanger::new()),
            vec![
                (flanger::FEEDBACK, 95.0),
                (flanger::DELAY, 10.0),
                (flanger::DEPTH, 100.0),
            ],
        ),
        (
            "flanger",
            Box::new(Flanger::new()),
            vec![(flanger::FEEDBACK, -95.0), (flanger::DELAY, 0.1)],
        ),
        (
            "phaser",
            Box::new(Phaser::new()),
            vec![
                (phaser::FEEDBACK, 95.0),
                (phaser::STAGES, 4.0),
                (phaser::CENTER, 100.0),
                (phaser::DEPTH, 100.0),
            ],
        ),
    ];
    for (name, node, values) in cases {
        let mut n = set_up(node, &values);
        let tail = n.tail_frames() as usize;
        let frames = tail + 48_000;
        let mut l = vec![0.0f32; frames];
        l[..480].copy_from_slice(&noise(480, 5));
        let mut r = l.clone();
        run(n.as_mut(), &mut l, &mut r, &[]);
        let peak = l.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let after = l[480 + tail..]
            .iter()
            .chain(&r[480 + tail..])
            .fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            after < peak * 1e-3,
            "{name} {values:?}: {} dB after the {tail}-frame tail",
            20.0 * (after / peak).log10()
        );
    }
}
