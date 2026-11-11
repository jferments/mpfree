//! Pad pressure: pressing harder on a held pad moves the setting its
//! pressure route names, smoothed so it never clicks.

use mpfree_sampler::dsp::{Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const C: f32 = std::f32::consts::FRAC_1_SQRT_2; // centre pan gain

fn source(name: &str, frames: usize, f: impl Fn(usize) -> f32) -> Source {
    Source::new(name, RATE, 1, (0..frames).map(f).collect()).unwrap()
}
fn dc() -> Source {
    source("dc", RATE as usize * 2, |_| 0.25)
}
fn ramp() -> Source {
    source("ramp", RATE as usize * 2, |i| (i + 1) as f32 * 1e-5)
}
fn sine(hz: f32) -> Source {
    source("sine", RATE as usize * 2, |i| {
        0.5 * (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin()
    })
}

fn routed(pad: &mut Pad, target: PressureTarget, depth: f32) -> &mut Pad {
    pad.pressure = PressureRoute { target, depth };
    pad
}

fn sample_pad(s: &Source) -> Pad {
    Pad::new("p", Layer::new(Region::whole(s)))
}

fn granular_pad(s: &Source, grain: GrainParams) -> Pad {
    let mut r = Region::whole(s);
    r.mode = PlayMode::Granular;
    r.grain = grain;
    Pad::new("g", Layer::new(r))
}

fn engine_for(pads: &[(usize, Pad)], s: &Source) -> Engine {
    let mut p = Project::new();
    p.add_source(s.clone());
    for (i, pad) in pads {
        p.set_pad(*i, Some(pad.clone()), None).unwrap();
    }
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    e
}

fn run(e: &mut Engine, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    (l, r)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn zero_crossings(x: &[f32]) -> usize {
    x.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

#[test]
fn level_pressure_glides_up_twelve_db_and_back_without_a_click() {
    let s = dc();
    let mut pad = sample_pad(&s);
    routed(&mut pad, PressureTarget::Level, 1.0);
    let mut e = engine_for(&[(0, pad)], &s);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let base = 0.25 * C;

    let (l, _) = run(&mut e, 4410);
    assert!(
        (l[4000] - base).abs() < 1e-6,
        "no pressure, no change: {}",
        l[4000]
    );

    e.apply(Event::Pressure { pad: 0, value: 127 });
    let (up, _) = run(&mut e, 4410);
    let loud = base * 10f32.powf(12.0 / 20.0);
    assert!(
        (up[4000] - loud).abs() < loud * 0.005,
        "full pressure is +12 dB: {} vs {loud}",
        up[4000]
    );
    assert!(
        up[100] > base && up[100] < loud,
        "it glides, it does not jump"
    );

    e.apply(Event::Pressure { pad: 0, value: 0 });
    let (down, _) = run(&mut e, 4410);
    assert!(
        (down[4000] - base).abs() < base * 0.005,
        "letting go returns to the base level: {}",
        down[4000]
    );

    // No step between neighbouring frames is anywhere near the 12 dB jump.
    let all: Vec<f32> = l.iter().chain(&up).chain(&down).copied().collect();
    let worst = all
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(worst < 0.01, "a step of {worst} would click");
}

#[test]
fn pitch_pressure_changes_how_fast_a_sample_plays() {
    let s = ramp();
    for (depth, speed) in [(0.0, 1.0), (1.0, 2.0), (-1.0, 0.5), (0.5, 2f32.sqrt())] {
        let mut pad = sample_pad(&s);
        routed(&mut pad, PressureTarget::Pitch, depth);
        let mut e = engine_for(&[(0, pad)], &s);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        e.apply(Event::Pressure { pad: 0, value: 127 });
        let (l, _) = run(&mut e, 8000);
        // The ramp rises 1e-5 per source frame: its slope is the read speed.
        let slope = (l[7000] - l[6000]) / 1000.0 / (1e-5 * C);
        assert!(
            (slope - speed).abs() < speed * 0.01,
            "depth {depth}: speed {slope}, want {speed}"
        );
    }
}

#[test]
fn filter_pressure_opens_and_closes_the_cutoff() {
    let s = sine(2000.0);
    let level = |depth: f32| {
        let mut pad = sample_pad(&s);
        pad.filter = Filter {
            kind: FilterKind::LowPass,
            poles: 2,
            cutoff_hz: 500.0,
            resonance: 0.0,
            env_octaves: 0.0,
        };
        routed(&mut pad, PressureTarget::Filter, depth);
        let mut e = engine_for(&[(0, pad)], &s);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        e.apply(Event::Pressure { pad: 0, value: 127 });
        let (l, _) = run(&mut e, 8820);
        rms(&l[4410..])
    };
    let (shut, base, open) = (level(-1.0), level(0.0), level(1.0));
    assert!(
        open > base * 3.0,
        "4 octaves up lets 2 kHz through: {open} vs {base}"
    );
    assert!(
        shut < base * 0.2,
        "4 octaves down shuts it: {shut} vs {base}"
    );
}

#[test]
fn density_and_size_pressure_thicken_the_grain_cloud() {
    let s = sine(441.0);
    let grains = |target: PressureTarget, depth: f32| {
        let mut pad = granular_pad(
            &s,
            GrainParams {
                size_ms: 50.0,
                density: 20.0,
                ..GrainParams::default()
            },
        );
        routed(&mut pad, target, depth);
        let mut e = engine_for(&[(0, pad)], &s);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        e.apply(Event::Pressure { pad: 0, value: 127 });
        let mut most = 0;
        for _ in 0..300 {
            run(&mut e, 128);
            most = most.max(e.live_grains());
        } // 0.87 s
        most
    };
    let base = grains(PressureTarget::Off, 0.0);
    assert!(base <= 2, "one grain at a time without pressure: {base}");
    let dense = grains(PressureTarget::GrainDensity, 1.0); // 20/s becomes 320/s
    assert!(dense >= 12, "16 times the density: {dense} grains at once");
    let long = grains(PressureTarget::GrainSize, 1.0); // 50 ms becomes 400 ms
    assert!(long >= 6, "8 times longer grains overlap: {long} at once");
    assert!(
        grains(PressureTarget::GrainDensity, -1.0) <= 1,
        "less pressure thins it out"
    );
}

#[test]
fn position_pressure_moves_where_grains_read() {
    let s = sine(441.0);
    let mut pad = granular_pad(
        &s,
        GrainParams {
            position: 0.1,
            size_ms: 40.0,
            density: 30.0,
            ..GrainParams::default()
        },
    );
    routed(&mut pad, PressureTarget::GrainPosition, 0.5);
    let mut e = engine_for(&[(0, pad)], &s);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    run(&mut e, 2048);
    let at = |e: &Engine| e.playheads().next().unwrap().2 as f64 / s.frames() as f64;
    assert!((at(&e) - 0.1).abs() < 0.01, "no pressure: {}", at(&e));
    e.apply(Event::Pressure { pad: 0, value: 127 });
    run(&mut e, 4096);
    assert!(
        (at(&e) - 0.6).abs() < 0.01,
        "full pressure moves half the region: {}",
        at(&e)
    );
}

#[test]
fn pitch_pressure_raises_the_grains_too() {
    let s = sine(441.0);
    let crossings = |value: u8| {
        let mut pad = granular_pad(
            &s,
            GrainParams {
                position: 0.25,
                size_ms: 50.0,
                density: 40.0,
                ..GrainParams::default()
            },
        );
        routed(&mut pad, PressureTarget::Pitch, 1.0);
        let mut e = engine_for(&[(0, pad)], &s);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        e.apply(Event::Pressure { pad: 0, value });
        let (l, _) = run(&mut e, 26_460);
        zero_crossings(&l[4410..])
    };
    let (low, high) = (crossings(0), crossings(127));
    let ratio = high as f32 / low as f32;
    assert!(
        (1.9..2.1).contains(&ratio),
        "an octave up doubles the crossings: {low} then {high}"
    );
}

#[test]
fn pressure_moves_only_its_own_pad_and_only_playing_voices() {
    let s = sine(441.0);
    let mut a = sample_pad(&s);
    routed(&mut a, PressureTarget::Level, 1.0);
    let mut b = sample_pad(&s);
    routed(&mut b, PressureTarget::Level, 1.0);
    let play = |pressure: bool| {
        let mut e = engine_for(&[(0, a.clone()), (1, b.clone())], &s);
        if pressure {
            e.apply(Event::Pressure { pad: 1, value: 127 });
        } // before pad 1 plays: ignored
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 100,
        });
        e.apply(Event::NoteOn {
            pad: 1,
            velocity: 100,
        });
        if pressure {
            e.apply(Event::Pressure { pad: 0, value: 127 });
        }
        run(&mut e, 4410)
    };
    let (quiet, _) = play(false);
    let (pressed, _) = play(true);
    // Solo pad 1 to compare with: the same with pad 0 never pressed.
    let mut e = engine_for(&[(1, b.clone())], &s);
    e.apply(Event::NoteOn {
        pad: 1,
        velocity: 100,
    });
    let (solo_b, _) = run(&mut e, 4410);
    let mut e = engine_for(&[(0, a.clone())], &s);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    e.apply(Event::Pressure { pad: 0, value: 127 });
    let (solo_a, _) = run(&mut e, 4410);
    for k in 0..4410 {
        assert!(
            (pressed[k] - (solo_a[k] + solo_b[k])).abs() < 1e-5,
            "frame {k}"
        );
    }
    assert!(rms(&pressed[2205..]) > rms(&quiet[2205..]) * 1.5);
}

#[test]
fn pressure_is_deterministic_and_every_target_stays_finite() {
    let s = sine(330.0);
    let targets = [
        PressureTarget::Level,
        PressureTarget::Pitch,
        PressureTarget::Filter,
        PressureTarget::GrainDensity,
        PressureTarget::GrainSize,
        PressureTarget::GrainPosition,
        PressureTarget::GrainSpray,
    ];
    let session = || {
        let mut pads = Vec::new();
        for (i, &t) in targets.iter().enumerate() {
            let mut g = granular_pad(
                &s,
                GrainParams {
                    spray: 0.2,
                    size_ms: 60.0,
                    density: 25.0,
                    ..GrainParams::default()
                },
            );
            g.filter = Filter {
                kind: FilterKind::LowPass,
                poles: 4,
                cutoff_hz: 1500.0,
                resonance: 0.3,
                env_octaves: 0.0,
            };
            routed(&mut g, t, if i % 2 == 0 { 1.0 } else { -1.0 });
            let mut p = sample_pad(&s);
            p.filter = g.filter;
            routed(&mut p, t, if i % 2 == 0 { -0.7 } else { 0.7 });
            pads.push((i, g));
            pads.push((i + 8, p));
        }
        let mut e = engine_for(&pads, &s);
        let mut out = Vec::new();
        for block in 0..200u32 {
            if block % 20 == 0 {
                for &(i, _) in &pads {
                    e.apply(Event::NoteOn {
                        pad: i as u8,
                        velocity: 100,
                    });
                }
            }
            for &(i, _) in &pads {
                e.schedule(
                    block % 128,
                    Event::Pressure {
                        pad: i as u8,
                        value: ((block * 13 + i as u32 * 29) % 128) as u8,
                    },
                );
            }
            let (l, r) = run(&mut e, 128);
            out.extend(l.into_iter().zip(r));
        }
        out
    };
    let first = session();
    assert!(first.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
    assert!(first.iter().any(|&(l, _)| l != 0.0));
    assert_eq!(first, session());
}

#[test]
fn depth_must_be_a_number_from_minus_one_to_one() {
    let s = dc();
    let mut p = Project::new();
    p.add_source(s.clone());
    for (depth, ok) in [
        (1.0, true),
        (-1.0, true),
        (0.3, true),
        (1.01, false),
        (-2.0, false),
        (f32::NAN, false),
        (f32::INFINITY, false),
    ] {
        let mut pad = sample_pad(&s);
        routed(&mut pad, PressureTarget::GrainDensity, depth);
        let got = p.set_pad(0, Some(pad), None);
        assert_eq!(got.is_ok(), ok, "depth {depth}: {got:?}");
        if !ok {
            assert_eq!(got, Err(ModelError::BadPad));
        }
    }
}
