//! Granular pads: a region played as a grain cloud through the engine.

use mpfree_grain::{Budget, Clip, GrainVoice};
use mpfree_sampler::engine::{Engine, EngineView, Event, GRAIN_BUDGET, KILL_FRAMES};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const C: f32 = std::f32::consts::FRAC_1_SQRT_2; // centre pan gain

fn ramp(frames: usize) -> Source {
    Source::new(
        "ramp",
        RATE,
        1,
        (1..=frames).map(|i| i as f32 * 1e-4).collect(),
    )
    .unwrap()
}

fn granular_pad(s: &Source, grain: GrainParams) -> Pad {
    let mut r = Region::whole(s);
    r.mode = PlayMode::Granular;
    r.grain = grain;
    Pad::new("cloud", Layer::new(r))
}

fn engine_for(pads: &[(usize, Pad)], s: &Source, voices: usize) -> Engine {
    let mut p = Project::new();
    p.add_source(s.clone());
    for (i, pad) in pads {
        p.set_pad(*i, Some(pad.clone()), None).unwrap();
    }
    let mut e = Engine::new(RATE, voices);
    e.set_view(EngineView::build(&p, RATE));
    e
}

fn run(e: &mut Engine, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    let mut at = 0;
    while at < frames {
        let n = block.min(frames - at);
        e.render(&mut l[at..at + n], &mut r[at..at + n]);
        at += n;
    }
    (l, r)
}

/// What a lone grain voice makes from the same source and settings.
fn reference(s: &Source, p: GrainParams, frames: usize) -> Vec<f32> {
    let clip = Clip::new(s.data(), 1, s.rate()).unwrap();
    let mut g = GrainVoice::new(RATE, 0);
    g.set_params(p);
    g.start();
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    g.render(&clip, &mut Budget::new(64), 0, &mut l, &mut r);
    l
}

#[test]
fn a_granular_pad_plays_the_grain_engine_with_tune_and_reverse_folded_in() {
    let s = ramp(44_100);
    // One grain, no randomness in play: the output is exactly a lone grain voice's, panned centre.
    let base = GrainParams {
        position: 0.5,
        size_ms: 10.0,
        density: 1.0,
        ..GrainParams::default()
    };
    for (tune, reverse, want) in [
        (0, false, base),
        (
            1200,
            false,
            GrainParams {
                pitch: 12.0,
                ..base
            },
        ),
        (
            -700,
            true,
            GrainParams {
                pitch: -7.0,
                reverse: 1.0,
                ..base
            },
        ),
    ] {
        let mut pad = granular_pad(&s, base);
        pad.layers[0].region.tune_cents = tune;
        pad.layers[0].region.reverse = reverse;
        let mut e = engine_for(&[(0, pad)], &s, 4);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        let (l, r) = run(&mut e, 600, 128);
        let reference = reference(&s, want, 600);
        for k in 0..600 {
            assert!(
                (l[k] - C * reference[k]).abs() < 1e-6,
                "tune {tune} rev {reverse} frame {k}: {} vs {}",
                l[k],
                C * reference[k]
            );
            assert_eq!(l[k], r[k]);
        }
        assert!(l[100] != 0.0);
    }
}

#[test]
fn lifting_the_pad_stops_grains_and_releases() {
    let s = ramp(44_100);
    let mut e = engine_for(
        &[(
            3,
            granular_pad(
                &s,
                GrainParams {
                    size_ms: 40.0,
                    density: 80.0,
                    spray: 0.3,
                    ..GrainParams::default()
                },
            ),
        )],
        &s,
        4,
    );
    e.apply(Event::NoteOn {
        pad: 3,
        velocity: 100,
    });
    let (held, _) = run(&mut e, 4410, 128);
    assert!(held[2000..].iter().any(|&x| x != 0.0));
    assert!(e.live_grains() > 0);
    e.apply(Event::NoteOff { pad: 3 });
    // A 5 ms release, then nothing.
    let (after, _) = run(&mut e, 4410, 128);
    assert!(after[..50].iter().any(|&x| x != 0.0));
    assert!(after[300..].iter().all(|&x| x == 0.0));
    assert_eq!(e.active_voices(), 0);
    assert_eq!(e.live_grains(), 0, "a finished voice hands its grains back");
}

#[test]
fn stealing_and_the_budget_bound_the_grains() {
    let s = ramp(44_100);
    let busy = GrainParams {
        size_ms: 500.0,
        density: 500.0,
        spray: 1.0,
        ..GrainParams::default()
    };
    let mut pad = granular_pad(&s, busy);
    pad.polyphony = 1;
    let mut e = engine_for(&[(0, pad)], &s, 8);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    run(&mut e, 4410, 128);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    run(&mut e, usize::from(KILL_FRAMES) + 128, 128);
    assert_eq!(e.active_voices(), 1, "the first hit was pushed out");
    assert!(e.live_grains() <= mpfree_grain::MAX_GRAINS);

    // Eight busy pads at once: the shared budget holds.
    let pads: Vec<(usize, Pad)> = (0..8).map(|i| (i, granular_pad(&s, busy))).collect();
    let mut e = engine_for(&pads, &s, 16);
    for i in 0..8 {
        e.apply(Event::NoteOn {
            pad: i,
            velocity: 100,
        });
    }
    for _ in 0..40 {
        run(&mut e, 128, 128);
        assert!(e.live_grains() <= GRAIN_BUDGET);
    }
    assert_eq!(e.live_grains(), GRAIN_BUDGET);
    assert!(e.stats().skipped_grains > 0);
    e.apply(Event::AllOff);
    run(&mut e, 4410, 128);
    assert_eq!((e.active_voices(), e.live_grains()), (0, 0));
}

#[test]
fn same_hits_same_sound() {
    let s = Source::new(
        "noise",
        RATE,
        2,
        (0..88_200)
            .map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0)
            .collect(),
    )
    .unwrap();
    let p = GrainParams {
        spray: 0.7,
        size_ms: 60.0,
        density: 90.0,
        pitch_spray: 7.0,
        reverse: 0.3,
        spread: 1.0,
        scan: 0.5,
        ..GrainParams::default()
    };
    let render = || {
        let mut e = engine_for(
            &[
                (0, granular_pad(&s, p)),
                (1, granular_pad(&s, GrainParams { position: 0.7, ..p })),
            ],
            &s,
            8,
        );
        e.schedule(
            10,
            Event::NoteOn {
                pad: 0,
                velocity: 100,
            },
        );
        e.schedule(
            3000,
            Event::NoteOn {
                pad: 1,
                velocity: 60,
            },
        );
        let a = run(&mut e, 10_000, 128);
        e.apply(Event::NoteOff { pad: 0 });
        let b = run(&mut e, 10_000, 333);
        (a, b)
    };
    let (x, y) = (render(), render());
    assert_eq!(x, y);
    assert_ne!(x.0 .0, x.0 .1, "spread pans grains apart");
}

#[test]
fn the_playhead_shows_where_grains_read_and_view_swaps_keep_playing() {
    let s = ramp(44_100);
    let mut e = engine_for(
        &[(
            5,
            granular_pad(
                &s,
                GrainParams {
                    position: 0.25,
                    scan: 0.0,
                    ..GrainParams::default()
                },
            ),
        )],
        &s,
        4,
    );
    e.apply(Event::NoteOn {
        pad: 5,
        velocity: 100,
    });
    run(&mut e, 1000, 128);
    let heads: Vec<_> = e.playheads().collect();
    assert_eq!(heads.len(), 1);
    assert_eq!((heads[0].0, heads[0].2), (5, 11_025));
    // A new view without the pad: the playing voice keeps its audio and its grains.
    e.set_view(EngineView::empty());
    let (l, _) = run(&mut e, 2000, 128);
    assert!(l.iter().any(|&x| x != 0.0));
    assert!(e.live_grains() > 0);
}

#[test]
fn grain_settings_are_checked_and_kept_across_modes() {
    let s = ramp(1000);
    let mut r = Region::whole(&s);
    r.grain.density = f32::NAN;
    assert_eq!(r.check(&s), Err(ModelError::BadRegion("grain settings")));
    r.grain.density = 40.0;
    r.grain.sync = Some(TempoSync {
        frames_per_beat: f64::INFINITY,
        grains_per_beat: None,
        size_beats: None,
    });
    assert_eq!(r.check(&s), Err(ModelError::BadRegion("grain settings")));
    r.grain.sync = None;
    r.mode = PlayMode::Granular;
    assert_eq!(r.check(&s), Ok(()));
    r.mode = PlayMode::OneShot;
    assert_eq!(r.grain.density, 40.0);
}
