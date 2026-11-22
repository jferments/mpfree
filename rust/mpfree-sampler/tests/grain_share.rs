//! Grains shared by musical priority through the engine (the sampler
//! plan's step 9): over 64 dense clouds and the master Grains FX a new hit
//! sounds at once, the total never passes the budget, grains that make
//! room fade, and a session that fits shares nothing.

use mpfree_grain::GrainView;
use mpfree_sampler::engine::{Engine, EngineView, Event, GRAIN_BUDGET};
use mpfree_sampler::grain_share::{NEW_HIT_SECS, RESERVE};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const CLOUDS: usize = 64;
const TONE_HZ: f32 = 20.0;

/// Two seconds of a very low tone (20 Hz), so neighbouring frames are
/// close and a cut grain shows as a step.
fn tone() -> Source {
    let n = RATE as usize * 2;
    let data = (0..n)
        .map(|i| 0.5 * (i as f32 * std::f32::consts::TAU * TONE_HZ / RATE as f32).sin())
        .collect();
    Source::new("tone", RATE, 1, data).unwrap()
}

/// A dense cloud: long grains, far more of them wanted than fit.
fn dense(position: f32) -> GrainParams {
    GrainParams {
        position,
        spray: 0.3,
        size_ms: 400.0,
        density: 150.0,
        spread: 1.0,
        ..GrainParams::default()
    }
}

/// A short hit: 50 ms grains, 100 a second, so one period is 441 frames.
const HIT_DENSITY: f32 = 100.0;
fn hit() -> GrainParams {
    GrainParams {
        position: 0.2,
        spray: 0.05,
        size_ms: 50.0,
        density: HIT_DENSITY,
        ..GrainParams::default()
    }
}

fn granular(s: &Source, grain: GrainParams) -> Pad {
    let mut r = Region::whole(s);
    r.mode = PlayMode::Granular;
    r.grain = grain;
    Pad::new("cloud", Layer::new(r))
}

/// `clouds` dense pads (0 on), then the hit's pad, and the master Grains
/// FX half wet with `master`.
fn engine(clouds: usize, master: bool) -> Engine {
    engine_of(clouds, dense, master)
}

/// [`engine`] with clouds of `cloud`'s settings, given their positions.
fn engine_of(clouds: usize, cloud: fn(f32) -> GrainParams, master: bool) -> Engine {
    let s = tone();
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..clouds {
        let at = i as f32 / clouds.max(1) as f32;
        p.set_pad(i, Some(granular(&s, cloud(at))), None).unwrap();
    }
    p.set_pad(clouds, Some(granular(&s, hit())), None).unwrap();
    if master {
        p.set_master(
            MasterFx {
                mix: 0.5,
                grain: dense(0.5),
            },
            None,
        )
        .unwrap();
    }
    let mut e = Engine::new(RATE, clouds + 8);
    e.set_view(EngineView::build(&p, RATE));
    if master {
        assert!(e.set_master_fx(Engine::make_master_fx(RATE)).is_none());
    }
    e
}

fn on(pad: usize) -> Event {
    Event::NoteOn {
        pad: pad as u8,
        velocity: 100,
    }
}

/// Renders `frames` in blocks of `block`, checking the budget after each.
fn run(e: &mut Engine, frames: usize, block: usize) -> Vec<f32> {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    let mut at = 0;
    while at < frames {
        let n = block.min(frames - at);
        e.render(&mut l[at..at + n], &mut r[at..at + n]);
        assert!(e.live_grains() <= GRAIN_BUDGET, "{}", e.live_grains());
        at += n;
    }
    l
}

/// The 64 clouds hit one a block, so their ages differ, then held for a
/// second and a half: far past what the budget holds.
fn crowded() -> Engine {
    let mut e = engine(CLOUDS, true);
    for i in 0..CLOUDS {
        e.apply(on(i));
        run(&mut e, 128, 128);
    }
    run(&mut e, RATE as usize * 3 / 2, 128);
    e
}

fn largest_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

#[test]
fn a_new_hit_sounds_within_one_period_over_64_clouds_and_the_grains_fx() {
    let mut e = crowded();
    let st = e.stats();
    assert!(st.held_back_clouds > 0, "crowded past the protected shares");
    // Room is kept for a new hit.
    assert!(GRAIN_BUDGET - e.live_grains() >= RESERVE / 2);
    e.apply(on(CLOUDS));
    let period = (RATE as f32 / HIT_DENSITY) as usize;
    let mut views = [GrainView::default(); 64];
    let mut first = None;
    let mut at = 0;
    while at < period {
        run(&mut e, 32, 32);
        at += 32;
        if e.grains_of(CLOUDS as u8, 0, &mut views) > 0 {
            first = Some(at);
            break;
        }
    }
    let first = first.expect("the new hit made no grain within one period");
    assert!(first <= period, "{first}");
    // And it keeps sounding through its first moments.
    let new_hit = (NEW_HIT_SECS * f64::from(RATE)) as usize;
    let mut quiet = 0;
    for _ in 0..new_hit / 32 {
        run(&mut e, 32, 32);
        if e.grains_of(CLOUDS as u8, 0, &mut views) == 0 {
            quiet += 1;
        }
    }
    assert_eq!(quiet, 0, "the hit went quiet while new");
}

#[test]
fn grains_that_make_room_fade_and_the_total_never_passes_the_budget() {
    let mut e = engine(CLOUDS, true);
    let mut out = Vec::new();
    for i in 0..CLOUDS {
        e.apply(on(i));
        out.extend(run(&mut e, 128, 128));
    }
    out.extend(run(&mut e, RATE as usize, 128));
    // Three new hits over the crowd, each taking room from it.
    for _ in 0..3 {
        e.apply(on(CLOUDS));
        out.extend(run(&mut e, RATE as usize / 4, 64));
    }
    let st = e.stats();
    assert!(st.faded_grains > 0, "{st:?}");
    assert!(out.iter().all(|x| x.is_finite()));
    // No grain cut: every cloud reads the same very low tone, so however
    // many play, the sound is that tone getting louder and softer, and its
    // largest step against its peak stays near the tone's own (0.3 %).
    // Cutting the grains instead of fading them steps by about 11 %.
    let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let own = std::f32::consts::TAU * TONE_HZ / RATE as f32;
    let step = largest_step(&out) / peak;
    assert!(step < own * 3.0, "{step} of the peak, the tone's own {own}");
    e.apply(Event::AllOff);
    run(&mut e, RATE as usize / 2, 128);
    assert_eq!(e.active_voices(), 0);
    assert!(e.live_grains() <= 2 * mpfree_grain::MAX_GRAINS);
}

#[test]
fn a_session_that_fits_shares_nothing() {
    // Four light clouds, the hit and the Grains FX: well inside the
    // budget.
    let light = |position| GrainParams {
        size_ms: 100.0,
        density: 40.0,
        ..dense(position)
    };
    let mut e = engine_of(4, light, true);
    for i in 0..=4 {
        e.apply(on(i));
    }
    let mut most = 0;
    for _ in 0..400 {
        run(&mut e, 128, 128);
        most = most.max(e.live_grains());
    }
    assert!(most > 0 && most + RESERVE <= GRAIN_BUDGET, "{most}");
    let st = e.stats();
    assert_eq!(
        (st.faded_grains, st.held_back_clouds, st.skipped_grains),
        (0, 0, 0),
        "{st:?}"
    );
}
