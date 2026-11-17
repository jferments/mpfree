//! Movement: a voice's LFO and envelope, checked through the settings they
//! move and the grains they make.

use mpfree_grain::*;

const RATE: u32 = 48_000;
/// The voice moves its settings at the start of each part of its clock.
const PART: u64 = 64;

fn run<S: GrainSource>(v: &mut GrainVoice, src: &S, frames: usize, block: usize) -> Vec<f32> {
    let mut budget = Budget::new(256);
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
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
    l
}

fn noise(frames: usize) -> Vec<f32> {
    let mut x = 5u32;
    (0..frames)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        })
        .collect()
}

fn lfo(shape: LfoShape, rate_hz: f32, target: MotionTarget, depth: f32) -> GrainLfo {
    GrainLfo {
        shape,
        rate_hz,
        beats: None,
        target,
        depth,
    }
}

/// The voice clock frame the settings were last moved at, after `frames`
/// rendered from a start at 0.
fn last_part(frames: u64) -> u64 {
    (frames - 1) / PART * PART
}

#[test]
fn an_lfo_moves_its_setting_at_its_rate() {
    let data = noise(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params {
        pitch: 2.0,
        lfo: lfo(LfoShape::Sine, 2.0, MotionTarget::Pitch, 0.5),
        ..Params::default()
    });
    v.start();
    assert_eq!(v.moved_params().pitch, 2.0);
    let mut done = 0u64;
    for frames in [3_000u64, 6_000, 12_000, 20_000, 30_000] {
        run(&mut v, &src, (frames - done) as usize, 128);
        done = frames;
        let t = last_part(frames) as f32 / RATE as f32;
        let want = 2.0 + 6.0 * (std::f32::consts::TAU * 2.0 * t).sin();
        let got = v.moved_params().pitch;
        assert!((got - want).abs() < 1e-3, "{frames}: {got} against {want}");
        // The settings as set stay as set.
        assert_eq!(v.params().pitch, 2.0);
    }
}

#[test]
fn an_lfo_on_position_moves_where_grains_read() {
    let data = noise(96_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params {
        position: 0.5,
        lfo: lfo(LfoShape::Triangle, 1.0, MotionTarget::Position, 0.25),
        ..Params::default()
    });
    v.start();
    // A quarter of a cycle: up a quarter of the span; three quarters: down.
    run(&mut v, &src, 12_000 + PART as usize, 128);
    assert!((v.playhead() - 0.75).abs() < 0.002, "{}", v.playhead());
    run(&mut v, &src, 24_000, 128);
    assert!((v.playhead() - 0.25).abs() < 0.002, "{}", v.playhead());
    // And the grains read there.
    let mut views = [GrainView::default(); MAX_GRAINS];
    let n = v.grains(&mut views);
    assert!(n > 0);
    assert!(
        views[..n].iter().any(|g| (g.position - 0.25).abs() < 0.05),
        "{:?}",
        &views[..n]
    );
}

#[test]
fn a_synced_lfo_follows_the_tempo() {
    let data = noise(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params {
        lfo: GrainLfo {
            beats: Some(1.0),
            ..lfo(LfoShape::Square, 0.01, MotionTarget::Pitch, 1.0)
        },
        ..Params::default()
    });
    // No tempo set: 120 BPM, a beat 24,000 frames, up for the first half.
    v.start();
    run(&mut v, &src, 11_968 + 1, 128);
    assert_eq!(v.moved_params().pitch, 12.0);
    run(&mut v, &src, 128, 128);
    assert_eq!(v.moved_params().pitch, -12.0);
    // At 60 BPM a cycle takes 48,000 frames.
    v.set_tempo(48_000.0);
    v.start();
    run(&mut v, &src, 20_000, 128);
    assert_eq!(v.moved_params().pitch, 12.0);
    run(&mut v, &src, 5_000, 128);
    assert_eq!(v.moved_params().pitch, -12.0);
}

#[test]
fn the_envelope_rises_falls_to_its_sustain_and_starts_again() {
    let data = noise(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params {
        env: GrainEnv {
            attack_ms: 100.0,
            decay_ms: 200.0,
            sustain: 0.25,
            target: MotionTarget::Spray,
            depth: 1.0,
        },
        ..Params::default()
    });
    v.start();
    assert_eq!(v.moved_params().spray, 0.0);
    // Half way up.
    run(&mut v, &src, 2_400 + 1, 128);
    let want = last_part(2_401) as f32 / 4_800.0;
    assert!((v.moved_params().spray - want).abs() < 1e-4);
    // Full at the end of the attack, then down.
    run(&mut v, &src, 4_800 - 2_401 + 1, 128);
    assert!(v.moved_params().spray > 0.99, "{}", v.moved_params().spray);
    run(&mut v, &src, 4_800, 128);
    let s = v.moved_params().spray;
    assert!(s < 0.5 && s > 0.25, "{s}");
    // At the sustain level by the decay time, and held there.
    run(&mut v, &src, 9_600, 128);
    assert!((v.moved_params().spray - 0.25).abs() < 1e-3);
    run(&mut v, &src, 24_000, 128);
    assert!((v.moved_params().spray - 0.25).abs() < 1e-4);
    // A new start starts it again.
    v.start();
    assert_eq!(v.moved_params().spray, 0.0);
}

#[test]
fn hold_draws_a_value_each_cycle_and_drift_glides_between_them() {
    let data = noise(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let values = |shape| {
        let mut v = GrainVoice::new(RATE, 3);
        v.set_params(Params {
            // 30 parts a cycle.
            lfo: lfo(shape, RATE as f32 / 1_920.0, MotionTarget::Pitch, 1.0),
            ..Params::default()
        });
        v.start();
        (0..300)
            .map(|_| {
                run(&mut v, &src, PART as usize, PART as usize);
                v.moved_params().pitch
            })
            .collect::<Vec<f32>>()
    };
    let hold = values(LfoShape::Hold);
    for cycle in hold.chunks(30).skip(1) {
        // Each value lasts one cycle (a part late: the first part of each
        // chunk is still the cycle before).
        assert!(cycle[1..].iter().all(|&x| x == cycle[1]), "{cycle:?}");
    }
    let held: Vec<f32> = hold.chunks(30).map(|c| c[1]).collect();
    assert!(held.windows(2).all(|w| w[0] != w[1]), "{held:?}");
    assert!(held.iter().all(|x| x.abs() <= 12.0));
    let drift = values(LfoShape::Drift);
    // No step bigger than a smooth glide's over one part (half a cosine
    // over 30 parts between values at most 24 st apart).
    let worst = drift
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(worst < 12.0 * std::f32::consts::PI / 30.0 + 0.01, "{worst}");
    assert!(drift.iter().any(|x| x.abs() > 1.0), "{drift:?}");
}

#[test]
fn movement_does_not_depend_on_the_period_size() {
    let data = noise(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let p = Params {
        spray: 0.2,
        size_ms: 30.0,
        density: 60.0,
        scan: 0.5,
        lfo: lfo(LfoShape::Sine, 7.0, MotionTarget::Position, 0.3),
        env: GrainEnv {
            attack_ms: 50.0,
            decay_ms: 100.0,
            sustain: 0.5,
            target: MotionTarget::Size,
            depth: 1.0,
        },
        ..Params::default()
    };
    let render = |block: usize, every: bool| {
        let mut v = GrainVoice::new(RATE, 9);
        v.set_params(p);
        v.start();
        let mut budget = Budget::new(256);
        let frames = 24_000;
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        let mut at = 0;
        while at < frames {
            let n = block.min(frames - at);
            if every {
                // A player setting the same settings each period.
                v.set_params(p);
            }
            v.render(
                &src,
                &mut budget,
                at as u64,
                &mut l[at..at + n],
                &mut r[at..at + n],
            );
            at += n;
        }
        l
    };
    let a = render(128, false);
    for (block, every) in [
        (1, false),
        (64, false),
        (333, false),
        (4096, false),
        (333, true),
    ] {
        let b = render(block, every);
        let worst = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-4, "block {block}: {worst}");
    }
}

#[test]
fn movement_off_or_at_no_depth_plays_as_without_it() {
    let data = noise(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let render = |p: Params| {
        let mut v = GrainVoice::new(RATE, 4);
        v.set_params(p);
        v.start();
        run(&mut v, &src, 12_000, 128)
    };
    let base = Params {
        spray: 0.3,
        scan: 1.0,
        ..Params::default()
    };
    let a = render(base);
    let off = render(Params {
        lfo: lfo(LfoShape::Hold, 9.0, MotionTarget::Off, 1.0),
        ..base
    });
    let none = render(Params {
        lfo: lfo(LfoShape::Sine, 9.0, MotionTarget::Pitch, 0.0),
        env: GrainEnv {
            target: MotionTarget::Density,
            depth: 0.0,
            ..GrainEnv::default()
        },
        ..base
    });
    assert_eq!(a, off);
    assert_eq!(a, none);
}

#[test]
fn bad_movement_settings_are_safe() {
    let data = noise(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 4);
    v.set_params(Params {
        lfo: GrainLfo {
            shape: LfoShape::Drift,
            rate_hz: f32::NAN,
            beats: Some(f32::INFINITY),
            target: MotionTarget::Size,
            depth: f32::NAN,
        },
        env: GrainEnv {
            attack_ms: f32::NAN,
            decay_ms: -5.0,
            sustain: f32::INFINITY,
            target: MotionTarget::Position,
            depth: 9.0,
        },
        ..Params::default()
    });
    v.start();
    let l = run(&mut v, &src, 12_000, 128);
    assert!(l.iter().all(|x| x.is_finite()));
    assert!(l.iter().any(|&x| x != 0.0));
}
