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
    let data = s.data().to_vec();
    let clip = Clip::new(&data, 1, s.rate()).unwrap();
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
fn a_tone_pads_tune_moves_its_note_not_its_grains_pitch() {
    let s = ramp(44_100);
    // A2 with grains four cycles long, regular, frozen.
    let base = GrainParams {
        position: 0.5,
        density: 110.0,
        size_ms: 4.0 * 1000.0 / 110.0,
        timing: 0.0,
        ..GrainParams::default()
    };
    for tune in [1200, -700] {
        let mut pad = granular_pad(&s, base);
        pad.layers[0].region.grain_mode = GrainMode::Tone;
        pad.layers[0].region.tune_cents = tune;
        let mut e = engine_for(&[(0, pad)], &s, 4);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        let (l, _) = run(&mut e, 4_410, 128);
        // The note (the density) moves, with the size, so each grain keeps
        // its four cycles; the grains' own pitch doesn't.
        let k = (tune as f32 / 1200.0).exp2();
        let want = GrainParams {
            density: base.density * k,
            size_ms: base.size_ms / k,
            ..base
        };
        let reference = reference(&s, want, 4_410);
        for f in 0..4_410 {
            assert!(
                (l[f] - C * reference[f]).abs() < 1e-6,
                "tune {tune} frame {f}: {} vs {}",
                l[f],
                C * reference[f]
            );
        }
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
    // Each cloud starts with a few grains under way and fills in over a
    // grain (thinned to fit its voice): within 0.35 s they want more than
    // the budget holds.
    for _ in 0..120 {
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
fn a_stretch_of_a_sound_with_a_tempo_plays_in_time() {
    let s = ramp(441_000);
    let stretch = |warp: Option<Warp>, bpm: f64| {
        let mut pad = granular_pad(
            &s,
            GrainParams {
                position: 0.0,
                scan: 1.0,
                ..GrainParams::default()
            },
        );
        pad.layers[0].region.grain_mode = GrainMode::Stretch;
        pad.layers[0].region.warp = warp;
        let mut e = engine_for(&[(0, pad)], &s, 4);
        e.set_tempo(bpm);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 100,
        });
        run(&mut e, 44_100, 128);
        let heads: Vec<_> = e.playheads().collect();
        heads[0].2
    };
    // Played at 100 BPM: at 120 it reads 1.2 s of sound a second.
    let at = stretch(Some(Warp::new(100.0)), 120.0);
    assert!(at.abs_diff(52_920) <= 128, "{at}");
    // Without a tempo, or as a cloud, the scan is as set.
    let at = stretch(None, 120.0);
    assert!(at.abs_diff(44_100) <= 128, "{at}");
}

/// Where each burst of sound starts (silence before it).
fn bursts(x: &[f32]) -> Vec<usize> {
    (0..x.len())
        .filter(|&i| x[i] != 0.0 && (i == 0 || x[i - 1] == 0.0))
        .collect()
}

#[test]
fn a_pads_lfo_moves_its_grains_with_the_tempo_and_starts_again_each_hit() {
    let s = ramp(44_100);
    let g = GrainParams {
        position: 0.5,
        lfo: GrainLfo {
            shape: LfoShape::Square,
            rate_hz: 1.0,
            beats: Some(1.0),
            target: MotionTarget::Position,
            depth: 0.25,
        },
        ..GrainParams::default()
    };
    let mut e = engine_for(&[(0, granular_pad(&s, g))], &s, 4);
    let head = |e: &Engine| e.playheads().next().map(|h| h.2);
    let hit = |e: &mut Engine| {
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 100,
        })
    };
    // 120 BPM: up a quarter of the sound for half a beat, then down.
    hit(&mut e);
    run(&mut e, 5_000, 128);
    assert_eq!(head(&e), Some(33_075));
    run(&mut e, 8_000, 128);
    assert_eq!(head(&e), Some(11_025));
    // At 60 BPM the beat is twice as long; a new hit starts the LFO again.
    e.apply(Event::NoteOff { pad: 0 });
    run(&mut e, 22_050, 128);
    assert_eq!(head(&e), None, "the first hit is over");
    e.set_tempo(60.0);
    hit(&mut e);
    run(&mut e, 20_000, 128);
    assert_eq!(head(&e), Some(33_075));
    run(&mut e, 5_000, 128);
    assert_eq!(head(&e), Some(11_025));
}

#[test]
fn a_synced_cloud_follows_the_project_tempo_not_its_saved_one() {
    let s = Source::new("dc", RATE, 1, vec![0.5; RATE as usize * 2]).unwrap();
    // Saved at some other tempo: the engine's is the one that counts.
    let g = GrainParams {
        sync: Some(TempoSync {
            frames_per_beat: 31_000.0,
            grains_per_beat: Some(4.0),
            size_beats: Some(1.0 / 16.0),
        }),
        ..GrainParams::default()
    };
    let mut e = engine_for(&[(0, granular_pad(&s, g))], &s, 8);
    assert_eq!(e.frames_per_beat(), 22_050.0, "120 BPM at 44.1 kHz");
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    let gaps = |l: &[f32]| -> Vec<usize> { bursts(l).windows(2).map(|w| w[1] - w[0]).collect() };
    let (l, _) = run(&mut e, 44_100, 128);
    let at_120 = gaps(&l);
    assert_eq!(at_120.len(), 7, "eight grains in a second: {at_120:?}");
    assert!(
        at_120.iter().all(|&d| d == 5512 || d == 5513),
        "a sixteenth at 120 BPM: {at_120:?}"
    );
    // Halving the tempo spaces the grains of the voice playing twice as far.
    e.set_tempo(60.0);
    let (l, _) = run(&mut e, 66_150, 128);
    let at_60 = gaps(&l);
    assert!(at_60.len() >= 4, "{at_60:?}");
    assert!(at_60.iter().all(|&d| d == 11_025), "{at_60:?}");
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

#[test]
fn the_screen_sees_the_grains_of_the_pad_and_layer_it_shows() {
    let s = ramp(RATE as usize);
    let g = GrainParams {
        size_ms: 40.0,
        density: 200.0,
        spray: 0.3,
        ..GrainParams::default()
    };
    let mut e = engine_for(&[(2, granular_pad(&s, g)), (5, granular_pad(&s, g))], &s, 8);
    let mut views = [mpfree_sampler::engine::GrainView::default(); 64];
    assert_eq!(e.grains_of(2, 0, &mut views), 0, "nothing plays yet");
    e.apply(Event::NoteOn {
        pad: 2,
        velocity: 100,
    });
    run(&mut e, 4410, 128);
    let n = e.grains_of(2, 0, &mut views);
    assert!((5..=10).contains(&n), "about 200/s of 40 ms grains: {n}");
    assert!(views[..n]
        .iter()
        .all(|v| (0.0..=1.0).contains(&v.position) && (0.0..=1.0).contains(&v.level)));
    assert_eq!(e.grains_of(5, 0, &mut views), 0, "another pad");
    assert_eq!(e.grains_of(2, 1, &mut views), 0, "another layer");
    assert_eq!(
        e.grains_of(2, 0, &mut views[..3]),
        3,
        "a short list holds what fits"
    );
    // A second voice on another pad adds its own, not to pad 2's.
    e.apply(Event::NoteOn {
        pad: 5,
        velocity: 100,
    });
    run(&mut e, 2205, 128);
    let (a, b) = (
        e.grains_of(2, 0, &mut views),
        e.grains_of(5, 0, &mut [Default::default(); 64]),
    );
    assert!(a > 0 && b > 0, "{a} and {b}");
    assert!(a + b <= e.live_grains());
}

#[test]
fn a_stretch_keeps_its_sounds_hits_sharp() {
    // Noise bursts from silence every quarter second, each decaying.
    let every = RATE as usize / 4;
    let mut seed = 0x2468_ACE1u32;
    let data: Vec<f32> = (0..RATE as usize * 2)
        .map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            let k = i % every;
            0.5 * noise * (-(k as f32) / (0.04 * RATE as f32)).exp()
        })
        .collect();
    let s = Source::new("drums", RATE, 1, data).unwrap();
    // Found here, not on their thread, so the test doesn't hang on it.
    s.find_hits();
    let hits = s.hits().get().expect("found").to_vec();
    for k in 1..8 {
        let want = (k * every) as u64;
        assert!(
            hits.iter().any(|&h| h.abs_diff(want) <= 64),
            "burst {k}: {hits:?}"
        );
    }
    let peak = |x: &[f32]| x.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    // Half speed, from the start: each burst due at twice its time.
    let play = |mode: GrainMode| {
        let mut pad = granular_pad(
            &s,
            GrainParams {
                position: 0.0,
                scan: 0.5,
                size_ms: 90.0,
                density: 45.0,
                timing: 0.0,
                ..GrainParams::default()
            },
        );
        pad.layers[0].region.grain_mode = mode;
        let mut e = engine_for(&[(0, pad)], &s, 4);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 100,
        });
        run(&mut e, 3 * every * 2, 128).0
    };
    let stretch = play(GrainMode::Stretch);
    let cloud = play(GrainMode::Cloud);
    for k in 1..3 {
        let h = hits
            .iter()
            .copied()
            .find(|h| h.abs_diff((k * every) as u64) <= 64)
            .unwrap() as usize;
        let at = 2 * h;
        let (before, after) = (peak(&stretch[at - 2_000..at]), peak(&stretch[at..at + 200]));
        assert!(before < 0.05 * after, "burst {k}: {before} then {after}");
        // As a cloud, its grains play it early.
        let (before, after) = (peak(&cloud[at - 2_000..at]), peak(&cloud[at..at + 200]));
        assert!(
            before > 0.3 * after,
            "cloud, burst {k}: {before} then {after}"
        );
    }
}
