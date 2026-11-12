//! Live settings (`Event::PadLive`, `Event::ProgramLive`) and the hard
//! stop (`Engine::stop_all`): a change lands on its frame, a p-locked hit
//! plays as a pad set that way would, playing voices glide to the new
//! level, pan, cutoff and tune, NaN and a new view give the pad's own back,
//! and the hard stop leaves nothing playing, here or on the lane.

use std::time::{Duration, Instant};

use mpfree_lanes::Config;
use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadField, ProgramField};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

/// Stereo noise, `frames` long.
fn noise(frames: usize, seed: u32) -> Source {
    let mut x = seed;
    let data = (0..frames * 2).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new("noise", RATE, 2, data.collect()).unwrap()
}

/// Pad 0 plays `pad` made from `s`; pad 1 is a granular pad, pad 2 a
/// looped one.
fn kit(s: &Source, pad: Pad) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(0, Some(pad), None).unwrap();
    let mut grains = Region::whole(s);
    grains.mode = PlayMode::Granular;
    grains.grain = GrainParams {
        density: 150.0,
        size_ms: 40.0,
        ..GrainParams::default()
    };
    p.set_pad(1, Some(Pad::new("grains", Layer::new(grains))), None)
        .unwrap();
    let mut lp = Region::whole(s);
    lp.mode = PlayMode::Loop;
    lp.sustain_loop = Some(Loop {
        start: 1000,
        end: 9000,
        crossfade: 200,
        mode: LoopMode::Forward,
    });
    p.set_pad(2, Some(Pad::new("loop", Layer::new(lp))), None)
        .unwrap();
    p
}

fn plain(s: &Source) -> Pad {
    let mut r = Region::whole(s);
    r.mode = PlayMode::Gate;
    Pad::new("noise", Layer::new(r))
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(p, RATE));
    e
}

/// `frames` frames in blocks of `block`, left then right interleaved per
/// frame; `events` are (frame, event) from the start. Split, the lane gets
/// a deadline it always makes, so a busy test machine can't make it late (a
/// plain render waits no longer than the block lasts, and a late lane drops
/// its part by design); a late part would fail here rather than as a
/// difference in the sound.
fn play(e: &mut Engine, frames: usize, block: usize, events: &[(usize, Event)]) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    let (mut l, mut r) = (vec![0.0; block], vec![0.0; block]);
    let mut at = 0;
    while at < frames {
        let n = block.min(frames - at);
        for &(f, ev) in events {
            if (at..at + n).contains(&f) {
                assert!(e.schedule((f - at) as u32, ev));
            }
        }
        e.render_until(
            &mut l[..n],
            &mut r[..n],
            Instant::now() + Duration::from_secs(5),
        );
        for k in 0..n {
            out.push(l[k]);
            out.push(r[k]);
        }
        at += n;
    }
    assert_eq!(e.stats().lane_late, 0, "the lane was late");
    out
}

fn hit(pad: u8) -> Event {
    Event::NoteOn { pad, velocity: 100 }
}

fn live(pad: u8, field: PadField, value: f32) -> Event {
    Event::PadLive { pad, field, value }
}

/// Root mean square of frames `a..b` (both channels).
fn rms(x: &[f32], a: usize, b: usize) -> f32 {
    let s = &x[2 * a..2 * b];
    (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
}

/// The first frame where `a` and `b` differ.
fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y).map(|i| i / 2)
}

#[test]
fn a_live_level_lands_on_its_frame_for_any_block_size() {
    let s = noise(40_000, 3);
    let p = kit(&s, plain(&s));
    let at = 1_000;
    let events = [(0, hit(0)), (at, live(0, PadField::Level, -12.0))];
    let before = play(&mut engine(&p), 20_000, 128, &events[..1]);
    let after = play(&mut engine(&p), 20_000, 128, &events);
    assert_eq!(first_difference(&before, &after), Some(at));
    // The same for blocks of 1, 37 and 1000 frames.
    for block in [1, 37, 1000] {
        assert!(
            play(&mut engine(&p), 20_000, block, &events) == after,
            "blocks of {block}"
        );
    }
    // It glides there, with no jump: a tenth of the way in its first 16
    // frames' step at most, 1/e of the way left after 10 ms, 12 dB down
    // after 150 ms. (The sound is the same noise, so the ratio of the
    // frames is the gain.)
    let gain = |f: usize| after[2 * f] / before[2 * f];
    let want = |ms: f32| 0.2512 + (1.0 - 0.2512) * (-ms / 10.0).exp();
    assert!(gain(at) < 1.0 && gain(at) > 0.99, "{}", gain(at));
    assert!((at..at + 2_000).all(|f| gain(f + 1) <= gain(f) + 1e-6));
    assert!(
        (gain(at + 441) - want(10.0)).abs() < 0.02,
        "{}",
        gain(at + 441)
    );
    assert!(
        (gain(at + 6_615) - 0.2512).abs() < 0.001,
        "{}",
        gain(at + 6_615)
    );
}

#[test]
fn a_p_locked_hit_plays_as_a_pad_set_that_way() {
    let s = noise(20_000, 5);
    let p = kit(&s, plain(&s));
    let lock = [
        (PadField::Level, -6.0),
        (PadField::Pan, 0.5),
        (PadField::Tune, 700.0),
        (PadField::Cutoff, 800.0),
        (PadField::Resonance, 0.5),
        (PadField::Attack, 5.0),
        (PadField::Decay, 50.0),
        (PadField::Release, 30.0),
    ];
    let mut events: Vec<(usize, Event)> = lock.iter().map(|&(f, v)| (300, live(0, f, v))).collect();
    events.push((300, hit(0)));
    events.push((6_000, Event::NoteOff { pad: 0 }));
    let locked = play(&mut engine(&p), 12_000, 128, &events);
    // The same pad with those settings of its own.
    let mut pad = plain(&s);
    pad.level_db = -6.0;
    pad.pan = 0.5;
    pad.tune_cents = 700;
    pad.filter = Filter {
        kind: FilterKind::LowPass,
        poles: 2,
        cutoff_hz: 800.0,
        resonance: 0.5,
        env_octaves: 0.0,
    };
    pad.amp = Ahdsr {
        attack_ms: 5.0,
        decay_ms: 50.0,
        release_ms: 30.0,
        ..pad.amp
    };
    let own = play(
        &mut engine(&kit(&s, pad)),
        12_000,
        128,
        &[(300, hit(0)), (6_000, Event::NoteOff { pad: 0 })],
    );
    assert!(rms(&own, 300, 6_000) > 0.01, "it plays");
    let worst = locked
        .iter()
        .zip(&own)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(
        worst < 1e-4,
        "the lock differs from the pad's own by {worst}"
    );
    // Frames before the hit are silent in both.
    assert!(locked[..600].iter().all(|x| *x == 0.0));
}

#[test]
fn playing_voices_follow_pan_cutoff_and_the_programs_offsets() {
    let s = noise(80_000, 9);
    let p = kit(&s, plain(&s));
    // Pan hard left while it plays: the right side fades out.
    let out = play(
        &mut engine(&p),
        20_000,
        128,
        &[(0, hit(0)), (2_000, live(0, PadField::Pan, -1.0))],
    );
    let right = |a: usize| {
        let s: f32 = (a..a + 512).map(|k| out[2 * k + 1].powi(2)).sum();
        (s / 512.0).sqrt()
    };
    assert!(right(1_000) > 0.1);
    assert!(right(15_000) < 1e-3, "{}", right(15_000));

    // The program's cutoff 4 octaves down on a pad with no filter darkens
    // it (a low-pass comes in, from open), and back at 0 it opens again.
    let events = [
        (0, hit(0)),
        (
            4_000,
            Event::ProgramLive {
                field: ProgramField::Cutoff,
                value: -4.0,
            },
        ),
        (
            20_000,
            Event::ProgramLive {
                field: ProgramField::Cutoff,
                value: 0.0,
            },
        ),
    ];
    let dark = play(&mut engine(&p), 40_000, 128, &events);
    let dry = play(&mut engine(&p), 40_000, 128, &events[..1]);
    assert!(dark[..2 * 4_000] == dry[..2 * 4_000], "nothing before it");
    let ratio = |a: usize| rms(&dark, a, a + 2_000) / rms(&dry, a, a + 2_000);
    assert!(ratio(14_000) < 0.5, "{}", ratio(14_000));
    assert!(ratio(36_000) > 0.97, "{}", ratio(36_000));

    // The program's level, on every voice.
    let events = [
        (0, hit(0)),
        (0, hit(2)),
        (
            1_000,
            Event::ProgramLive {
                field: ProgramField::Level,
                value: -20.0,
            },
        ),
    ];
    let quiet = play(&mut engine(&p), 12_000, 128, &events);
    let loud = play(&mut engine(&p), 12_000, 128, &events[..2]);
    let r = rms(&quiet, 10_000, 12_000) / rms(&loud, 10_000, 12_000);
    assert!((r - 0.1).abs() < 0.002, "{r}");
}

#[test]
fn the_programs_tune_moves_playing_voices_and_new_ones() {
    let s = noise(200_000, 11);
    let p = kit(&s, plain(&s));
    let mut e = engine(&p);
    let at = |e: &Engine| e.playheads().find(|h| h.0 == 0).map(|h| h.2).unwrap();
    play(&mut e, 128, 128, &[(0, hit(0))]);
    let a = at(&e);
    play(&mut e, 4_410, 128, &[]);
    let speed = (at(&e) - a) as f64 / 4_410.0;
    assert!((speed - 1.0).abs() < 1e-3, "{speed}");
    // An octave up: after the glide, it reads twice as fast.
    play(
        &mut e,
        8_820,
        128,
        &[(
            0,
            Event::ProgramLive {
                field: ProgramField::Tune,
                value: 1200.0,
            },
        )],
    );
    let a = at(&e);
    play(&mut e, 4_410, 128, &[]);
    let speed = (at(&e) - a) as f64 / 4_410.0;
    assert!((speed - 2.0).abs() < 1e-3, "{speed}");
    assert_eq!(e.program_live_value(ProgramField::Tune), 1200.0);
    // A new hit starts there; a pad's live tune adds a fifth to it.
    play(
        &mut e,
        128,
        128,
        &[
            (0, Event::AllOff),
            (0, live(0, PadField::Tune, 700.0)),
            (1, hit(0)),
        ],
    );
    play(&mut e, 4_410, 128, &[]);
    let heads: Vec<u64> = e.playheads().filter(|h| h.0 == 0).map(|h| h.2).collect();
    let a = *heads.iter().min().unwrap();
    play(&mut e, 4_410, 128, &[]);
    let b = e
        .playheads()
        .filter(|h| h.0 == 0)
        .map(|h| h.2)
        .min()
        .unwrap();
    let speed = (b - a) as f64 / 4_410.0;
    let want = 2.0 * (7.0f64 / 12.0).exp2();
    assert!((speed - want).abs() < 2e-3, "{speed} vs {want}");
}

#[test]
fn nan_and_a_new_view_give_the_pads_own_back() {
    let s = noise(20_000, 13);
    let p = kit(&s, plain(&s));
    let mut e = engine(&p);
    e.apply(live(0, PadField::Level, -6.0));
    e.apply(live(0, PadField::Cutoff, 1e9));
    e.apply(live(200, PadField::Level, -6.0)); // no such pad: nothing
    e.apply(Event::ProgramLive {
        field: ProgramField::Level,
        value: -3.0,
    });
    assert_eq!(e.pad_live_value(0, PadField::Level), Some(-6.0));
    assert_eq!(
        e.pad_live_value(0, PadField::Cutoff),
        Some(22_000.0),
        "clamped"
    );
    assert_eq!(e.pad_live_value(1, PadField::Level), None);
    e.apply(live(0, PadField::Level, f32::NAN));
    assert_eq!(e.pad_live_value(0, PadField::Level), None);
    e.set_view(EngineView::build(&p, RATE));
    assert_eq!(e.pad_live_value(0, PadField::Cutoff), None);
    assert_eq!(e.program_live_value(ProgramField::Level), -3.0, "kept");
    e.apply(Event::ProgramLive {
        field: ProgramField::Level,
        value: f32::NAN,
    });
    assert_eq!(e.program_live_value(ProgramField::Level), 0.0);
    // Cleared, the pad plays exactly as before.
    let clean = play(&mut engine(&p), 4_000, 128, &[(0, hit(0))]);
    assert!(play(&mut e, 4_000, 128, &[(0, hit(0))]) == clean);
}

fn split(mut e: Engine, threaded: bool) -> Engine {
    e.split(Config {
        name: "test-lane".into(),
        threaded,
        ..Config::default()
    })
    .unwrap();
    e
}

/// Hits, lifts and live changes on both halves of a split pool.
fn busy() -> Vec<(usize, Event)> {
    let mut v = Vec::new();
    for k in 0..40usize {
        let f = k * 997;
        v.push((f, hit((k % 3) as u8)));
        v.push((
            f + 300,
            live(
                (k % 3) as u8,
                PadField::ALL[k % 8],
                (k as f32 * 37.0) % 900.0 - 300.0,
            ),
        ));
        if k % 4 == 1 {
            v.push((
                f + 500,
                Event::ProgramLive {
                    field: ProgramField::ALL[k % 3],
                    value: (k as f32 % 5.0) - 2.0,
                },
            ));
        }
        if k % 5 == 2 {
            v.push((f + 700, Event::NoteOff { pad: 0 }));
            v.push((f + 700, live(0, PadField::Pan, f32::NAN)));
        }
    }
    v.sort_by_key(|e| e.0);
    v
}

#[test]
fn split_or_not_the_live_settings_play_the_same() {
    let s = noise(30_000, 17);
    let p = kit(&s, plain(&s));
    let events = busy();
    let one = play(&mut engine(&p), 42_000, 128, &events);
    let here = play(&mut split(engine(&p), false), 42_000, 128, &events);
    let mut lane = split(engine(&p), true);
    let there = play(&mut lane, 42_000, 128, &events);
    assert!(here == there, "the lane's own core changes nothing");
    let worst = one
        .iter()
        .zip(&here)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst < 1e-5, "split differs from unsplit by {worst}");
    assert!(lane.stats().lane_parts > 50, "{:?}", lane.stats());
    // A change lands on its frame on the lane too.
    let at = 1_000;
    let events = [
        (0, hit(0)),
        (0, hit(2)),
        (0, hit(0)),
        (at, live(2, PadField::Level, -12.0)),
    ];
    let before = play(&mut split(engine(&p), true), 5_000, 128, &events[..3]);
    let after = play(&mut split(engine(&p), true), 5_000, 128, &events);
    assert_eq!(first_difference(&before, &after), Some(at));
}

#[test]
fn stop_all_leaves_nothing_playing_or_waiting() {
    let s = noise(30_000, 19);
    let p = kit(&s, plain(&s));
    let mut e = engine(&p);
    e.set_master_fx(Engine::make_master_fx(RATE));
    play(&mut e, 2_000, 128, &[(0, hit(0)), (0, hit(1)), (0, hit(2))]);
    assert_eq!(e.active_voices(), 3);
    assert!(e.live_grains() > 0);
    e.schedule(50, hit(0));
    e.stop_all();
    assert_eq!((e.active_voices(), e.live_grains()), (0, 0));
    let out = play(&mut e, 2_000, 128, &[]);
    assert!(
        out.iter().all(|x| *x == 0.0),
        "silent at once, and stays so"
    );
    assert_eq!(e.active_voices(), 0, "the waiting hit was dropped");
    // It plays again on the next hit.
    let out = play(&mut e, 1_000, 128, &[(0, hit(0))]);
    assert!(rms(&out, 0, 1_000) > 0.05);

    // Split, with the lane's share away (it missed its deadline): it stops
    // as it comes back.
    let mut e = split(engine(&p), true);
    for pad in 0..3 {
        e.apply(hit(pad));
        e.apply(hit(pad));
    }
    let (mut l, mut r) = (vec![0.0; 1024], vec![0.0; 1024]);
    e.render_until(&mut l, &mut r, Instant::now() + Duration::from_secs(5));
    assert_eq!(e.active_voices(), 6);
    e.render_until(&mut l, &mut r, Instant::now() - Duration::from_millis(1));
    e.stop_all();
    for _ in 0..4 {
        e.render_until(&mut l, &mut r, Instant::now() + Duration::from_secs(5));
    }
    assert_eq!((e.active_voices(), e.live_grains()), (0, 0));
    assert!(l.iter().chain(&r).all(|x| *x == 0.0));
}

#[test]
fn a_glide_that_starts_again_after_landing_is_the_same_for_any_block_size() {
    // A sine with no filter of its own: the program's cutoff darkens it and
    // opens it again (and the pad's level dips and comes back), long after
    // the first glide has landed and also before it lands.
    let data: Vec<f32> = (0..22_050)
        .flat_map(|i| {
            let x = (i as f32 * 330.0 * std::f32::consts::TAU / RATE as f32).sin() * 0.5;
            [x, x]
        })
        .collect();
    let s = Source::new("sine", RATE, 2, data).unwrap();
    let mut r = Region::whole(&s);
    r.mode = PlayMode::OneShot;
    let p = kit(&s, Pad::new("sine", Layer::new(r)));
    let cutoff = |value| Event::ProgramLive {
        field: ProgramField::Cutoff,
        value,
    };
    for back in [1_100, 3_000, 6_000, 9_000] {
        // The program's cutoff, then the pad's own level.
        for (down, up) in [
            (cutoff(-1.0), cutoff(0.0)),
            (
                live(0, PadField::Level, -12.0),
                live(0, PadField::Level, f32::NAN),
            ),
        ] {
            let events = [(0, hit(0)), (1_000, down), (back, up)];
            let want = play(&mut engine(&p), 12_000, 128, &events);
            for block in [1, 7, 64, 100, 1000] {
                let got = play(&mut engine(&p), 12_000, block, &events);
                assert_eq!(
                    first_difference(&got, &want),
                    None,
                    "{up:?} at {back}, blocks of {block}"
                );
            }
        }
    }
}

#[test]
fn the_view_tells_each_pads_own_values() {
    let s = noise(10_000, 9);
    let mut pad = plain(&s);
    pad.level_db = -6.0;
    pad.pan = 0.25;
    pad.tune_cents = -120;
    pad.amp = Ahdsr {
        attack_ms: 4.0,
        hold_ms: 0.0,
        decay_ms: 80.0,
        sustain: 0.5,
        release_ms: 300.0,
    };
    let mut p = kit(&s, pad);
    let v = EngineView::build(&p, RATE);
    let own = |v: &EngineView, f| v.pad_own(0, f);
    assert_eq!(own(&v, PadField::Level), Some(-6.0));
    assert_eq!(own(&v, PadField::Pan), Some(0.25));
    assert_eq!(own(&v, PadField::Tune), Some(-120.0));
    assert_eq!(own(&v, PadField::Attack), Some(4.0));
    assert_eq!(own(&v, PadField::Decay), Some(80.0));
    assert_eq!(own(&v, PadField::Release), Some(300.0));
    // No filter: nothing of its own for Cutoff and Resonance.
    assert_eq!(own(&v, PadField::Cutoff), None);
    assert_eq!(own(&v, PadField::Resonance), None);
    p.edit_pad(0, None, |pd| {
        pd.filter = Filter {
            kind: FilterKind::HighPass,
            poles: 2,
            cutoff_hz: 900.0,
            resonance: 0.3,
            env_octaves: 0.0,
        }
    })
    .unwrap();
    let v = EngineView::build(&p, RATE);
    assert_eq!(own(&v, PadField::Cutoff), Some(900.0));
    assert_eq!(own(&v, PadField::Resonance), Some(0.3));
    // A pad without a sound has none.
    assert!(PadField::ALL.iter().all(|&f| v.pad_own(9, f).is_none()));
    assert!(v.pad_own(99, PadField::Level).is_none());
}
