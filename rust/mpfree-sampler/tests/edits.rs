//! Edits heard on a voice already playing (`Engine::edit_view`): a held
//! loop takes its moved loop points (jumping into a loop moved behind it
//! without a step, playing on through one moved ahead), and level, pan,
//! cutoff, tune and grain settings reach the sound still sounding. A new
//! sound on the pad, or another kit (`Engine::set_view`), leaves the
//! playing voice as it began. Split, the voices on both shares follow.

use std::time::{Duration, Instant};

use mpfree_lanes::Config;
use mpfree_sampler::dsp::{Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
/// The ramp's frames, and what one frame adds to it.
const RAMP: usize = 1 << 16;
const STEP: f32 = 1.0 / RAMP as f32;

/// A mono ramp: frame `i` is `i / RAMP`, so the sound tells which frame it
/// plays.
fn ramp() -> Source {
    let data = (0..RAMP).map(|i| i as f32 * STEP).collect();
    Source::new("ramp", RATE, 1, data).unwrap()
}

/// Stereo noise, `frames` long.
fn noise(frames: usize, seed: u32) -> Source {
    let mut x = seed;
    let data = (0..frames * 2).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new("noise", RATE, 2, data.collect()).unwrap()
}

/// A project with pad 0 playing `s` in `mode`, looped over `lp` if given.
fn kit(s: &Source, mode: PlayMode, lp: Option<(u64, u64, LoopMode)>) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(s);
    r.mode = mode;
    r.sustain_loop = lp.map(|(start, end, mode)| Loop {
        start,
        end,
        crossfade: 0,
        mode,
    });
    p.set_pad(0, Some(Pad::new("pad", Layer::new(r))), None)
        .unwrap();
    p
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(p, RATE));
    e
}

/// `frames` frames in blocks of 128, left and right apart. Split, the lane
/// gets a deadline it always makes.
fn run(e: &mut Engine, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    let (mut ol, mut or) = (Vec::new(), Vec::new());
    let mut at = 0;
    while at < frames {
        let n = 128.min(frames - at);
        e.render_until(
            &mut l[..n],
            &mut r[..n],
            Instant::now() + Duration::from_secs(5),
        );
        ol.extend_from_slice(&l[..n]);
        or.extend_from_slice(&r[..n]);
        at += n;
    }
    assert_eq!(e.stats().lane_late, 0, "the lane was late");
    (ol, or)
}

fn hold(e: &mut Engine) {
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
}

/// Pad 0's settings changed in a copy of `p`, built.
fn edited(p: &Project, f: impl FnOnce(&mut Pad)) -> (Project, EngineView) {
    let mut q = p.clone();
    q.edit_pad(0, None, f).unwrap();
    let v = EngineView::build(&q, RATE);
    (q, v)
}

fn set_loop(pad: &mut Pad, start: u64, end: u64) {
    let lp = pad.layers[0].region.sustain_loop.as_mut().unwrap();
    (lp.start, lp.end) = (start, end);
}

/// The ramp frame each output sample plays, from the gain `g` it plays at.
fn frames_of(x: &[f32], g: f32) -> Vec<f32> {
    x.iter().map(|s| s / g / STEP).collect()
}

/// The ramp's gain on the left, from frame 500 of a voice started at 0.
fn ramp_gain(left: &[f32]) -> f32 {
    left[500] / (500.0 * STEP)
}

/// Root mean square of `x`.
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn a_held_loop_jumps_into_its_loop_moved_behind_it_without_a_step() {
    let s = ramp();
    for mode in [LoopMode::Forward, LoopMode::PingPong] {
        let p = kit(&s, PlayMode::Loop, Some((1000, 9000, mode)));
        let mut e = engine(&p);
        hold(&mut e);
        let (before, _) = run(&mut e, 4000);
        let g = ramp_gain(&before);
        // The loop moves to 2000..3000 while the voice plays frame 4000.
        let (_, view) = edited(&p, |pad| set_loop(pad, 2000, 3000));
        drop(e.edit_view(view));
        let (after, _) = run(&mut e, 6000);
        let at = frames_of(&after, g);
        for (k, &f) in at.iter().enumerate().skip(usize::from(JUMP)) {
            assert!(
                (1999.5..3000.5).contains(&f),
                "{mode:?}: frame {k} after the edit plays {f}, outside the loop"
            );
        }
        // The jump (back 2000 frames, or 1000 turning back at the loop's
        // end) is smoothed: spread over at least 32 frames.
        let back = match mode {
            LoopMode::Forward => 2000.0,
            LoopMode::PingPong => 1000.0,
        };
        let jump = g * back * STEP;
        let worst = std::iter::once(before[3999])
            .chain(after[..200].iter().copied())
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < jump / 32.0,
            "{mode:?}: a step of {worst} at the jump (the jump is {jump})"
        );
    }
}

const JUMP: u16 = mpfree_sampler::engine::JUMP_FRAMES;

#[test]
fn a_loop_moved_on_is_heard_on_this_pass() {
    let s = ramp();
    let p = kit(&s, PlayMode::Loop, Some((1000, 9000, LoopMode::Forward)));
    let mut e = engine(&p);
    hold(&mut e);
    let (before, _) = run(&mut e, 4000);
    let g = ramp_gain(&before);
    let (_, view) = edited(&p, |pad| set_loop(pad, 6000, 12_000));
    drop(e.edit_view(view));
    let (after, _) = run(&mut e, 14_000);
    let at = frames_of(&after, g);
    // On through the old end to the new one, then round the new loop.
    assert!((at[7000] - 11_000.0).abs() < 0.5, "frame {}", at[7000]);
    assert!((at[8000] - 6000.0).abs() < 0.5, "frame {}", at[8000]);
    assert!(at[8000..].iter().all(|f| (5999.5..12_000.5).contains(f)));
}

#[test]
fn another_kit_or_another_sound_leaves_the_voice_as_it_began() {
    let s = ramp();
    let p = kit(&s, PlayMode::Loop, Some((1000, 9000, LoopMode::Forward)));
    let (_, moved) = edited(&p, |pad| set_loop(pad, 2000, 3000));
    // The same edit as another kit: the voice plays its old loop.
    let mut e = engine(&p);
    hold(&mut e);
    let (before, _) = run(&mut e, 4000);
    let g = ramp_gain(&before);
    drop(e.set_view(moved.clone()));
    let (kit, _) = run(&mut e, 6000);
    assert!((frames_of(&kit, g)[3000] - 7000.0).abs() < 0.5);
    // Another sound on the pad, with its loop and level moved: nothing
    // reaches the voice, so it plays as with another kit, sample for
    // sample.
    let other = noise(RAMP, 7);
    let mut q = p.clone();
    q.add_source(other.clone());
    q.edit_pad(0, None, |pad| {
        pad.layers[0].region = Region::whole(&other);
        pad.layers[0].region.mode = PlayMode::Loop;
        pad.layers[0].region.sustain_loop = Some(Loop {
            start: 2000,
            end: 3000,
            crossfade: 0,
            mode: LoopMode::Forward,
        });
        pad.level_db = -12.0;
    })
    .unwrap();
    let mut e = engine(&p);
    hold(&mut e);
    run(&mut e, 4000);
    drop(e.edit_view(EngineView::build(&q, RATE)));
    let (swapped, _) = run(&mut e, 6000);
    assert_eq!(swapped, kit);
}

#[test]
fn level_and_pan_glide_to_the_edit_on_a_held_sound() {
    let s = noise(RATE as usize * 2, 3);
    let p = kit(&s, PlayMode::Gate, None);
    let mut e = engine(&p);
    hold(&mut e);
    let (l0, r0) = run(&mut e, 4410);
    let (_, view) = edited(&p, |pad| pad.level_db = -12.0);
    drop(e.edit_view(view));
    let (l1, r1) = run(&mut e, 8820);
    let db = |a: &[f32], b: &[f32]| 20.0 * (rms(b) / rms(a)).log10();
    // 12 dB down once it has glided (10 ms), on both sides.
    assert!((db(&l0[2000..], &l1[4410..]) + 12.0).abs() < 0.3);
    assert!((db(&r0[2000..], &r1[4410..]) + 12.0).abs() < 0.3);
    // A glide, not a step: the gain moves over more than 5 ms.
    assert!(
        db(&l0[2000..], &l1[..64]) > -3.0,
        "the first frames still loud"
    );
    // Panned hard right, the left side goes quiet.
    let (_, view) = edited(&p, |pad| pad.pan = 1.0);
    drop(e.edit_view(view));
    let (l2, r2) = run(&mut e, 8820);
    assert!(rms(&l2[4410..]) < 1e-4, "left {}", rms(&l2[4410..]));
    assert!(rms(&r2[4410..]) > rms(&r0[2000..]));
}

#[test]
fn a_cutoff_edit_reaches_a_held_sound_and_a_filter_comes_in_from_open() {
    let s = noise(RATE as usize * 2, 5);
    // A difference of neighbouring samples: how bright it is.
    let bright = |x: &[f32]| rms(&x.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>());
    let lowpass = |hz: f32| Filter {
        kind: FilterKind::LowPass,
        poles: 2,
        cutoff_hz: hz,
        resonance: 0.0,
        env_octaves: 0.0,
        drive_db: 0.0,
    };
    // A low-pass at 8 kHz, edited down to 300 Hz while held.
    let mut p = kit(&s, PlayMode::Gate, None);
    p.edit_pad(0, None, |pad| pad.filter = lowpass(8000.0))
        .unwrap();
    let mut e = engine(&p);
    hold(&mut e);
    let (open, _) = run(&mut e, 4410);
    let (_, view) = edited(&p, |pad| pad.filter = lowpass(300.0));
    drop(e.edit_view(view));
    let (closed, _) = run(&mut e, 8820);
    assert!(bright(&closed[4410..]) < bright(&open[1000..]) * 0.1);
    // A pad with no filter: a low-pass turned on comes in from open.
    let p = kit(&s, PlayMode::Gate, None);
    let mut e = engine(&p);
    hold(&mut e);
    let (open, _) = run(&mut e, 4410);
    let (_, view) = edited(&p, |pad| pad.filter = lowpass(300.0));
    drop(e.edit_view(view));
    let (closed, _) = run(&mut e, 8820);
    assert!(bright(&closed[4410..]) < bright(&open[1000..]) * 0.1);
    assert!(
        bright(&closed[..32]) > bright(&open[1000..]) * 0.5,
        "it starts open"
    );
}

#[test]
fn a_tune_edit_reaches_a_held_sound() {
    let s = ramp();
    let p = kit(&s, PlayMode::Gate, None);
    let mut e = engine(&p);
    hold(&mut e);
    let (before, _) = run(&mut e, 2000);
    let g = ramp_gain(&before);
    let (_, view) = edited(&p, |pad| pad.tune_cents = 100);
    drop(e.edit_view(view));
    let (after, _) = run(&mut e, 4000);
    let at = frames_of(&after, g);
    // A semitone up: 2^(1/12) frames a frame, from where it was.
    let speed = (at[3000] - at[1000]) / 2000.0;
    assert!(
        (speed - 2f32.powf(1.0 / 12.0)).abs() < 1e-3,
        "speed {speed}"
    );
    assert!(
        (at[0] - 2000.0).abs() < 1.5,
        "it carries on from frame {}",
        at[0]
    );
}

#[test]
fn a_granular_voice_takes_the_new_grain_settings() {
    let s = noise(RATE as usize * 2, 9);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Granular;
    r.grain = GrainParams {
        density: 20.0,
        size_ms: 40.0,
        ..GrainParams::default()
    };
    p.set_pad(0, Some(Pad::new("grains", Layer::new(r))), None)
        .unwrap();
    let mut e = engine(&p);
    hold(&mut e);
    run(&mut e, 22_050);
    let sparse = e.live_grains();
    let (_, view) = edited(&p, |pad| {
        pad.layers[0].region.grain.density = 400.0;
        pad.layers[0].region.grain.size_ms = 100.0;
    });
    drop(e.edit_view(view));
    run(&mut e, 22_050);
    let dense = e.live_grains();
    assert!(
        dense >= 20 && dense > sparse * 5,
        "{sparse} grains, then {dense}"
    );
}

#[test]
fn split_the_voices_on_both_cores_follow_an_edit() {
    let s = ramp();
    let mut p = kit(&s, PlayMode::Loop, Some((1000, 9000, LoopMode::Forward)));
    // Eight voices of the one pad, so both shares hold some.
    p.edit_pad(0, None, |pad| pad.polyphony = 8).unwrap();
    let mut e = Engine::new(RATE, 8);
    e.split(Config {
        name: "test-lane".into(),
        threaded: true,
        ..Config::default()
    })
    .unwrap();
    e.set_view(EngineView::build(&p, RATE));
    for _ in 0..8 {
        hold(&mut e);
    }
    assert_eq!(e.active_voices(), 8);
    let (before, _) = run(&mut e, 4000);
    let g = ramp_gain(&before);
    let (_, view) = edited(&p, |pad| set_loop(pad, 2000, 3000));
    drop(e.edit_view(view));
    let (after, _) = run(&mut e, 6000);
    // All eight play the same frame, inside the new loop.
    for (k, &f) in frames_of(&after, g)
        .iter()
        .enumerate()
        .skip(usize::from(JUMP))
    {
        assert!((1999.5..3000.5).contains(&f), "frame {k} plays {f}");
    }
}
