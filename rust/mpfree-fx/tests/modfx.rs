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
        // Echoes every 2 ms (96 frames), each the last times the feedback,
        // all trimmed by sqrt(1 - g²) (the level trim at Mix 100 %). The
        // flat read spreads each echo over a few frames, summing to it.
        let g = fb / 100.0;
        let trim = (1.0 - g * g).sqrt();
        for k in 1..5 {
            let want = trim * g.powi(k as i32 - 1);
            let echo: f32 = l[96 * k - 30..96 * k + 30].iter().sum();
            assert!(
                (echo - want).abs() < 1e-3,
                "feedback {fb}: echo {k} is {echo}, want {want}"
            );
            let peak = (96 * k - 30..96 * k + 30)
                .max_by(|&a, &b| l[a].abs().total_cmp(&l[b].abs()))
                .unwrap();
            assert_eq!(peak, 96 * k, "feedback {fb}: echo {k}");
        }
    }
}

/// The response in dB at `hz` of an impulse response.
fn response_db(h: &[f32], hz: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / f64::from(RATE);
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, &v) in h.iter().enumerate() {
        re += f64::from(v) * (w * n as f64).cos();
        im -= f64::from(v) * (w * n as f64).sin();
    }
    10.0 * (re * re + im * im).max(1e-30).log10()
}

#[test]
fn the_flanger_draws_what_it_plays() {
    // Held (Depth 0), the comb heard is the comb drawn, through the read
    // it is made with: at whole and half frames, near the write (where
    // the read fades to a cubic), at either Feedback and at two Mixes.
    use ids::flanger::{DELAY, DEPTH, FEEDBACK, MIX, WIDTH};
    let frame = 1000.0 / RATE as f32;
    let delays = [
        0.1,
        3.5 * frame,
        2.5 * frame,
        1.5,
        1.5 + 0.5 * frame,
        7.0 + 0.25 * frame,
    ];
    for delay in delays {
        for fb in [-90.0f32, 0.0, 90.0] {
            for mix in [50.0f32, 100.0] {
                let values = [
                    (DEPTH, 0.0),
                    (DELAY, delay),
                    (FEEDBACK, fb),
                    (MIX, mix),
                    (WIDTH, 0.0),
                ];
                let mut f = set_up(Box::new(Flanger::new()), &values);
                let mut l = vec![0.0f32; 1 << 15];
                l[0] = 1.0;
                let mut r = l.clone();
                run(f.as_mut(), &mut l, &mut r, &[]);
                let mut drawn = Flanger::new();
                drawn.prepare(RATE, MAX_BLOCK);
                for (id, v) in values {
                    drawn.set_param(id, v);
                }
                for k in 1..=90 {
                    let hz = 200.0 * k as f64;
                    let want = f64::from(drawn.response_db(hz as f32, 0.0));
                    if want < -20.0 {
                        continue;
                    }
                    let got = response_db(&l, hz);
                    assert!(
                        (got - want).abs() < 0.05,
                        "Delay {delay} ms, Feedback {fb}, Mix {mix}: {hz} Hz heard {got:.2} dB, drawn {want:.2}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_flanger_goes_in_at_the_dry_level_at_any_feedback() {
    // The trim holds a noise's level within 1 dB at any Feedback and Mix,
    // sweeping as it does by default. The noise is dulled a little, as
    // music is (6 dB down at 12 kHz, nothing at 24): the read rolls off
    // the last few kHz, so round a loop at 95 % white noise's top octave
    // loses more.
    use ids::flanger::{FEEDBACK, MIX};
    let white = noise(3 * RATE as usize + 2, 17);
    let x: Vec<f32> = white
        .windows(3)
        .map(|w| 0.25 * (w[0] + 2.0 * w[1] + w[2]))
        .collect();
    let start = RATE as usize / 2;
    let dry = rms(&x[start..]);
    for fb in [-95.0f32, -70.0, -30.0, 0.0, 30.0, 70.0, 95.0] {
        for mix in [25.0f32, 50.0, 100.0] {
            let mut f = set_up(Box::new(Flanger::new()), &[(FEEDBACK, fb), (MIX, mix)]);
            let (mut l, mut r) = (x.clone(), x.clone());
            run(f.as_mut(), &mut l, &mut r, &[]);
            for (side, y) in [("left", &l), ("right", &r)] {
                let db = 20.0 * (rms(&y[start..]) / dry).log10();
                assert!(
                    db.abs() < 1.0,
                    "Feedback {fb}, Mix {mix}, {side}: {db:.2} dB against the dry"
                );
            }
        }
    }
}

#[test]
fn the_flanger_never_sweeps_past_20_ms() {
    use ids::flanger::{DELAY, DEPTH};
    let longest = |delay: f32, depth: f32| {
        let mut f = Flanger::new();
        f.prepare(RATE, MAX_BLOCK);
        f.set_param(DELAY, delay);
        f.set_param(DEPTH, depth);
        let ms: Vec<f32> = (0..=400).map(|k| f.delay_ms(k as f32 / 400.0)).collect();
        (
            ms.iter().copied().fold(f32::MAX, f32::min),
            ms.iter().copied().fold(0.0f32, f32::max),
        )
    };
    for delay in [0.1f32, 1.0, 2.5, 5.0, 7.5, 10.0] {
        for depth in [0.0f32, 50.0, 100.0] {
            let (lo, hi) = longest(delay, depth);
            assert!(hi <= 20.0 + 1e-3, "Delay {delay}, Depth {depth}: {hi} ms");
            // The sweep stays centred on the Delay, in octaves.
            assert!(
                ((lo * hi).sqrt() - delay).abs() < 1e-3 * delay,
                "Delay {delay}, Depth {depth}: {lo} to {hi} ms"
            );
        }
    }
    // Below 20 ms, Depth keeps its meaning: two octaves each way at 100 %.
    let (lo, hi) = longest(2.5, 100.0);
    assert!(
        (lo - 0.625).abs() < 1e-3 && (hi - 10.0).abs() < 1e-2,
        "{lo} to {hi}"
    );
    // At Delay 10 ms the sweep narrows to an octave each way.
    let (lo, hi) = longest(10.0, 100.0);
    assert!(
        (lo - 5.0).abs() < 1e-2 && (hi - 20.0).abs() < 1e-2,
        "{lo} to {hi}"
    );
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
    let cases: [Case; 5] = [
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
            "flanger, through zero",
            Box::new(Flanger::new()),
            vec![
                (flanger::THROUGH_ZERO, 1.0),
                (flanger::FEEDBACK, 95.0),
                (flanger::DELAY, 10.0),
                (flanger::DEPTH, 100.0),
            ],
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

/// The correlation of two signals.
fn corr(a: &[f32], b: &[f32]) -> f32 {
    let dot = a.iter().zip(b).map(|(p, q)| f64::from(p * q)).sum::<f64>();
    (dot / (f64::from(rms(a)) * f64::from(rms(b)) * a.len() as f64)) as f32
}

#[test]
fn the_chorus_is_wide_at_every_voice_count() {
    // White noise, mostly highs, on both sides: at full Width the two
    // sides' copies must differ at every count. With the right side a
    // quarter cycle on, three voices nearly matched and four exactly.
    use ids::chorus::{MIX, TONE, VOICES};
    let x = noise(96_000, 11);
    for voices in 1..=4 {
        let mut c = set_up(
            Box::new(Chorus::new()),
            &[(VOICES, voices as f32), (TONE, 20_000.0), (MIX, 100.0)],
        );
        let (mut l, mut r) = (x.clone(), x.clone());
        run(c.as_mut(), &mut l, &mut r, &[]);
        let k = corr(&l[48_000..], &r[48_000..]);
        assert!(k.abs() < 0.3, "{voices} voices: correlation {k}");
    }
}

#[test]
fn chorus_depth_is_milliseconds_and_the_bend_stays_inside_a_semitone() {
    use ids::chorus::{DELAY, DEPTH, SYNC};
    // The sweep either side of the Delay, and the steepest the delay
    // changes (seconds a second), from the drawing's own function.
    let measure = |values: &[(u16, f32)]| {
        let mut c = Chorus::new();
        c.prepare(RATE, MAX_BLOCK);
        for &(id, v) in values {
            c.set_param(id, v);
        }
        c.reset();
        let (base, hz) = (c.param(DELAY), c.param(ids::chorus::RATE));
        let n = 4096;
        let d: Vec<f32> = (0..=n)
            .map(|i| c.delay_ms(0, i as f32 / n as f32).unwrap())
            .collect();
        let swing = d.iter().fold(0.0f32, |m, v| m.max((v - base).abs()));
        let slope =
            d.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs())) * n as f32 * hz / 1000.0;
        (swing, slope)
    };
    // Depth is a sweep in ms whatever the Delay (5 ms at 100 %).
    for delay in [5.0f32, 12.0, 40.0] {
        let (swing, _) = measure(&[(DEPTH, 40.0), (DELAY, delay), (ids::chorus::RATE, 0.6)]);
        assert!((swing - 2.0).abs() < 0.01, "Delay {delay}: {swing} ms");
    }
    let (swing, _) = measure(&[(DEPTH, 100.0), (DELAY, 12.0), (ids::chorus::RATE, 0.6)]);
    assert!((swing - 5.0).abs() < 0.01, "{swing}");
    // Never past 90 % of the Delay.
    let (swing, _) = measure(&[(DEPTH, 100.0), (DELAY, 3.0), (ids::chorus::RATE, 0.6)]);
    assert!((swing - 2.7).abs() < 0.01, "{swing}");
    // However fast and deep, the pitch moves less than a semitone: the
    // delay never changes faster than 1 - 2^(-1/12) seconds a second.
    let semitone = 1.0 - 2f32.powf(-1.0 / 12.0);
    for rate in [0.05f32, 0.6, 3.0, 10.0] {
        for delay in [2.0f32, 12.0, 40.0] {
            let (_, slope) = measure(&[(DEPTH, 100.0), (DELAY, delay), (ids::chorus::RATE, rate)]);
            assert!(slope < semitone, "Rate {rate}, Delay {delay}: {slope}");
        }
    }
    // Synced to a sixteenth at 120 BPM (8 cycles a second) too.
    let mut c = Chorus::new();
    c.prepare(RATE, MAX_BLOCK);
    c.set_param(SYNC, 1.0);
    c.set_param(DEPTH, 100.0);
    c.set_param(DELAY, 40.0);
    c.reset();
    let swing = (0..256)
        .map(|i| c.delay_ms(0, i as f32 / 256.0).unwrap() - 40.0)
        .fold(0.0f32, |m, v| m.max(v.abs()));
    let slope = 2.0 * PI * 8.0 * swing / 1000.0;
    assert!(slope < semitone * 1.001, "synced: {slope}");
}

#[test]
fn a_synced_chorus_is_back_on_the_bar_after_a_loop_wraps() {
    // A chorus synced to the bar plays 3.5 bars at 120 BPM, then the loop
    // wraps to the start, half a cycle off. Within half a second (a deep
    // sweep catches up no faster than its pitch ceiling allows) it must
    // sound like a chorus that started at the loop's start with the same
    // input.
    use ids::chorus::{DEPTH, MIX, SYNC, TONE};
    let values = [
        (SYNC, 5.0f32),
        (DEPTH, 100.0),
        (TONE, 20_000.0),
        (MIX, 100.0),
    ];
    let bar = 2 * RATE as usize;
    let wrap = bar * 7 / 2;
    let x = noise(wrap + RATE as usize, 21);
    let play = |n: &mut dyn Node, l: &mut [f32], r: &mut [f32], from: usize, song0: usize| {
        let mut at = from;
        for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            // The song position: frames since `song0`, wrapping at the loop.
            let song = (at - song0) % wrap;
            let ctx = Ctx {
                bpm: 120.0,
                beats: song as f64 / (RATE as f64 / 2.0),
                playing: true,
                ..Ctx::new(RATE, at as u64)
            };
            n.process(&ctx, &[], a, b);
            at += a.len();
        }
    };
    let mut looped = set_up(Box::new(Chorus::new()), &values);
    let (mut l, mut r) = (x.clone(), x.clone());
    play(looped.as_mut(), &mut l, &mut r, 0, 0);
    let mut fresh = set_up(Box::new(Chorus::new()), &values);
    let (mut fl, mut fr) = (x[wrap..].to_vec(), x[wrap..].to_vec());
    play(fresh.as_mut(), &mut fl, &mut fr, wrap, wrap);
    let after = RATE as usize / 2;
    let a = &l[wrap + after..];
    let b = &fl[after..];
    let worst = a
        .iter()
        .zip(b)
        .fold(0.0f32, |m, (p, q)| m.max((p - q).abs()));
    assert!(
        worst < 1e-3 * rms(b) * 4.0,
        "{worst} apart against {} RMS",
        rms(b)
    );
}

#[test]
fn through_zero_waits_5_ms_and_says_so() {
    // The dry waits 5 ms (to the frame), the latency reported, at every
    // rate; off, nothing waits. Mix 0 is the dry, waited, through the
    // flat read (so the crossing cancels exactly): within 0.15 dB to
    // 12 kHz and 1 dB to 16 kHz.
    use ids::flanger::{MIX, THROUGH_ZERO};
    for rate in [44_100u32, 48_000, 96_000] {
        let mut f = Flanger::new();
        f.prepare(rate, MAX_BLOCK);
        assert_eq!(f.latency(), 0, "{rate} Hz, off");
        f.set_param(THROUGH_ZERO, 1.0);
        f.set_param(MIX, 0.0);
        f.reset();
        let want = (0.005 * rate as f32).round() as usize;
        assert_eq!(f.latency() as usize, want, "{rate} Hz, on");
        let mut l = vec![0.0f32; 4096];
        l[0] = 1.0;
        let mut r = l.clone();
        f.process(&Ctx::new(rate, 0), &[], &mut l, &mut r);
        let peak = (0..l.len())
            .max_by(|&a, &b| l[a].abs().total_cmp(&l[b].abs()))
            .unwrap();
        assert_eq!(peak, want, "{rate} Hz: the dry comes out at the latency");
        assert_eq!(l, r, "{rate} Hz: both sides wait the same");
        let w = 2.0 * std::f64::consts::PI / f64::from(rate);
        for k in 1..=80 {
            let hz = 200.0 * f64::from(k);
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (n, &v) in l.iter().enumerate() {
                re += f64::from(v) * (w * hz * n as f64).cos();
                im -= f64::from(v) * (w * hz * n as f64).sin();
            }
            let db = 10.0 * (re * re + im * im).log10();
            let within = if hz <= 12_000.0 { 0.15 } else { 1.0 };
            assert!(db.abs() < within, "{rate} Hz: {hz} Hz at {db:.3} dB");
        }
    }
}

#[test]
fn through_zero_cancels_at_the_crossing() {
    // Held at the crossing (Depth 0) with no Feedback, the copy is the
    // dry inverted: Mix 50 is silence, exactly.
    use ids::flanger::{DEPTH, FEEDBACK, MIX, THROUGH_ZERO};
    let mut f = set_up(
        Box::new(Flanger::new()),
        &[
            (THROUGH_ZERO, 1.0),
            (DEPTH, 0.0),
            (FEEDBACK, 0.0),
            (MIX, 50.0),
        ],
    );
    let (mut l, mut r) = (noise(20_000, 7), noise(20_000, 8));
    run(f.as_mut(), &mut l, &mut r, &[]);
    assert!(
        l.iter().chain(&r).all(|&v| v == 0.0),
        "{} left, {} right",
        rms(&l),
        rms(&r)
    );
}

#[test]
fn through_zero_draws_what_it_plays() {
    // The right side starts a quarter cycle on (Width 100), where the
    // copy is furthest behind; at the slowest Rate it holds still for
    // the impulse's ring. Heard there, the comb is the comb drawn at
    // phase 0.25, at either Feedback and at two Mixes.
    use ids::flanger::{DELAY, DEPTH, FEEDBACK, MIX, RATE as LFO_RATE, THROUGH_ZERO, WIDTH};
    // At the widest reach the ring is long enough for the slow sweep to
    // move it, so that case is heard without Feedback.
    let cases = [
        (3.0f32, 80.0f32, &[-70.0f32, 0.0, 70.0][..]),
        (1.5, 60.0, &[-70.0, 0.0, 70.0]),
        (10.0, 100.0, &[0.0]),
    ];
    for (delay, depth, fbs) in cases {
        for &fb in fbs {
            for mix in [50.0f32, 100.0] {
                let values = [
                    (THROUGH_ZERO, 1.0),
                    (LFO_RATE, 0.02),
                    (DEPTH, depth),
                    (DELAY, delay),
                    (FEEDBACK, fb),
                    (MIX, mix),
                    (WIDTH, 100.0),
                ];
                let mut f = set_up(Box::new(Flanger::new()), &values);
                let mut l = vec![0.0f32; 1 << 14];
                l[0] = 1.0;
                let mut r = l.clone();
                run(f.as_mut(), &mut l, &mut r, &[]);
                let mut drawn = Flanger::new();
                drawn.prepare(RATE, MAX_BLOCK);
                for (id, v) in values {
                    drawn.set_param(id, v);
                }
                for k in 1..=90 {
                    let hz = 200.0 * k as f64;
                    let want = f64::from(drawn.response_db(hz as f32, 0.25));
                    if want < -20.0 {
                        continue;
                    }
                    let got = response_db(&r, hz);
                    assert!(
                        (got - want).abs() < 0.05,
                        "Delay {delay} ms, Depth {depth}, Feedback {fb}, Mix {mix}: {hz} Hz heard {got:.2} dB, drawn {want:.2}"
                    );
                }
            }
        }
    }
}

#[test]
fn through_zero_sweeps_evenly_either_side_of_the_dry() {
    // The gap drawn: Delay times Depth either way, never past the wait.
    use ids::flanger::{DELAY, DEPTH, THROUGH_ZERO};
    let mut f = Flanger::new();
    f.prepare(RATE, MAX_BLOCK);
    f.set_param(THROUGH_ZERO, 1.0);
    assert!((f.wait_ms() - 5.0).abs() < 1e-4);
    for (delay, depth, reach) in [
        (2.0f32, 50.0f32, 1.0f32),
        (3.0, 80.0, 2.4),
        (10.0, 100.0, 5.0 - 2.0 / 48.0),
    ] {
        f.set_param(DELAY, delay);
        f.set_param(DEPTH, depth);
        let gaps: Vec<f32> = (0..=400).map(|k| f.gap_ms(k as f32 / 400.0)).collect();
        let lo = gaps.iter().copied().fold(f32::MAX, f32::min);
        let hi = gaps.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            (hi - reach).abs() < 1e-3 && (lo + reach).abs() < 1e-3,
            "Delay {delay}, Depth {depth}: {lo} to {hi} ms"
        );
        assert!(f.gap_ms(0.0).abs() < 1e-4, "it starts at the crossing");
    }
}

#[test]
fn switching_through_zero_does_not_click() {
    // On, off and on again mid-note: the dry and the copy glide across,
    // far smoother than a cut between the two (the second time on, the
    // dry's lines have gone stale while off, and fill again first).
    use ids::flanger::THROUGH_ZERO;
    let x = sine(110.0, 0.5, 48_000);
    let render = |on: f32, moves: &[(usize, u16, f32)]| {
        let mut n = set_up(Box::new(Flanger::new()), &[(THROUGH_ZERO, on)]);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(n.as_mut(), &mut l, &mut r, moves);
        l
    };
    let stills = [render(0.0, &[]), render(1.0, &[])];
    let still = stills
        .iter()
        .map(|s| roughness(&s[4000..]))
        .fold(0.0f32, f32::max);
    let moving = roughness(
        &render(
            0.0,
            &[
                (12_000, THROUGH_ZERO, 1.0),
                (24_000, THROUGH_ZERO, 0.0),
                (40_000, THROUGH_ZERO, 1.0),
            ],
        )[4000..],
    );
    let spliced: Vec<f32> = (0..x.len())
        .map(|i| stills[usize::from((12_000..24_000).contains(&i) || i >= 40_000)][i])
        .collect();
    let hard = roughness(&spliced[4000..]);
    assert!(hard > 0.01, "a hard switch is visible ({hard})");
    assert!(
        moving < still + 0.1 * (hard - still),
        "roughness {moving} switching, {still} holding still, {hard} cutting"
    );
}

#[test]
fn through_zero_goes_in_at_the_dry_level() {
    // As off: a dulled noise's level within 1 dB at any Feedback and Mix,
    // sweeping as it does by default.
    use ids::flanger::{FEEDBACK, MIX, THROUGH_ZERO};
    let white = noise(3 * RATE as usize + 2, 17);
    let x: Vec<f32> = white
        .windows(3)
        .map(|w| 0.25 * (w[0] + 2.0 * w[1] + w[2]))
        .collect();
    let start = RATE as usize / 2;
    let dry = rms(&x[start..]);
    let mut worst = (0.0f32, String::new());
    for fb in [-95.0f32, -70.0, -30.0, 0.0, 30.0, 70.0, 95.0] {
        for mix in [25.0f32, 50.0, 100.0] {
            let mut f = set_up(
                Box::new(Flanger::new()),
                &[(THROUGH_ZERO, 1.0), (FEEDBACK, fb), (MIX, mix)],
            );
            let (mut l, mut r) = (x.clone(), x.clone());
            run(f.as_mut(), &mut l, &mut r, &[]);
            for (side, y) in [("left", &l), ("right", &r)] {
                let db = 20.0 * (rms(&y[start..]) / dry).log10();
                if db.abs() > worst.0.abs() {
                    worst = (db, format!("Feedback {fb}, Mix {mix}, {side}"));
                }
            }
        }
    }
    assert!(
        worst.0.abs() < 1.0,
        "{}: {:.2} dB against the dry",
        worst.1,
        worst.0
    );
}
