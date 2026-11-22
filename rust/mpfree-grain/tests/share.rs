//! A voice's share of a crowded grain budget (the sampler plan's step 9):
//! no share plays exactly as before, a share thins the cloud evenly at the
//! same level, and grains ended to make room fade along their own windows.

use mpfree_grain::*;

const RATE: u32 = 48_000;

/// Renders `frames` in periods of `block`, calling `each` before every
/// period.
fn run<S: GrainSource>(
    v: &mut GrainVoice,
    src: &S,
    budget: &mut Budget,
    frames: usize,
    block: usize,
    mut each: impl FnMut(&mut GrainVoice),
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    let mut at = 0;
    while at < frames {
        let n = block.min(frames - at);
        each(v);
        let t = v.played();
        v.render(src, budget, t, &mut l[at..at + n], &mut r[at..at + n]);
        at += n;
    }
    (l, r)
}

fn tone(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| 0.5 * (i as f32 * std::f32::consts::TAU * 110.0 / RATE as f32).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn largest_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// Long grains, far more wanted at once than a voice keeps.
fn dense() -> Params {
    Params {
        position: 0.5,
        spray: 0.4,
        size_ms: 400.0,
        density: 300.0,
        spread: 1.0,
        ..Params::default()
    }
}

#[test]
fn no_share_plays_exactly_as_before() {
    let data = tone(240_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let make = || {
        let mut v = GrainVoice::new(RATE, 5);
        v.set_params(dense());
        v.start();
        v
    };
    let (mut a, mut b) = (make(), make());
    let plain = run(&mut a, &src, &mut Budget::new(256), 96_000, 128, |_| {});
    // Asked about and given no share before every period, as the engine
    // does while every cloud fits.
    let asked = run(&mut b, &src, &mut Budget::new(256), 96_000, 128, |v| {
        let _ = (v.wants(), v.alive(), v.played(), v.ending(240));
        v.set_share(MAX_GRAINS);
    });
    assert!(plain.0 == asked.0 && plain.1 == asked.1, "bit for bit");
    assert_eq!(a.stats(), b.stats());
    assert_eq!(b.stats().held_back, 0);
}

#[test]
fn wants_is_about_what_the_cloud_keeps_alive() {
    let data = tone(480_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    for p in [
        dense(),
        Params {
            size_ms: 100.0,
            density: 20.0,
            ..dense()
        },
        Params {
            size_ms: 50.0,
            density: 200.0,
            ..dense()
        },
    ] {
        let mut v = GrainVoice::new(RATE, 3);
        v.set_params(p);
        v.start();
        run(&mut v, &src, &mut Budget::new(256), 48_000, 128, |_| {});
        let wants = v.wants();
        let mut most = 0;
        run(&mut v, &src, &mut Budget::new(256), 96_000, 128, |v| {
            most = most.max(v.alive());
        });
        assert!(most <= wants, "{p:?}: {most} alive, wants {wants}");
        assert!(most * 2 >= wants, "{p:?}: {most} alive, wants {wants}");
    }
    // Stopped, it wants nothing more.
    let mut v = GrainVoice::new(RATE, 3);
    v.set_params(dense());
    v.start();
    run(&mut v, &src, &mut Budget::new(256), 4_800, 128, |_| {});
    v.stop();
    assert_eq!(v.wants(), 0);
    assert!(v.alive() > 0, "its grains finish");
}

#[test]
fn a_share_thins_the_cloud_evenly_at_the_same_level() {
    let data = tone(480_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 7);
    v.set_params(dense());
    v.start();
    let mut budget = Budget::new(256);
    let (full, _) = run(&mut v, &src, &mut budget, 72_000, 128, |_| {});
    let before = rms(&full[36_000..]);
    v.set_share(16);
    assert_eq!(v.share(), 16);
    // Over one grain's length the grains it had finish.
    run(&mut v, &src, &mut budget, 24_000, 128, |_| {});
    let mut most = 0;
    let (thin, _) = run(&mut v, &src, &mut budget, 96_000, 128, |v| {
        most = most.max(v.alive());
    });
    assert!(most <= 16, "{most} alive");
    assert!(most >= 10, "thinned to fit, not starved: {most}");
    let after = rms(&thin);
    let db = 20.0 * (after / before).log10();
    assert!(db.abs() < 3.0, "the level moved {db:.1} dB");
    // Evenly: its gaps widen to fit, so few grains wait on the cap.
    let s = v.stats();
    assert!(s.held_back * 4 < s.spawned, "{s:?}");
}

#[test]
fn no_share_at_all_finishes_the_grains_playing_and_makes_none() {
    let data = tone(480_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 2);
    v.set_params(dense());
    v.start();
    let mut budget = Budget::new(256);
    let (calm, _) = run(&mut v, &src, &mut budget, 48_000, 128, |_| {});
    v.set_share(0);
    let mut last = v.alive();
    let spawned = v.stats().spawned;
    let (out, _) = run(&mut v, &src, &mut budget, 28_800, 128, |v| {
        assert!(v.alive() <= last, "a new grain");
        last = v.alive();
    });
    assert_eq!(v.alive(), 0, "every grain finished within one grain");
    assert_eq!(v.stats().spawned, spawned);
    assert!(v.stats().held_back > 0);
    assert!(largest_step(&out) <= largest_step(&calm[24_000..]) * 1.5);
}

#[test]
fn grains_ended_to_make_room_fade_along_their_windows_never_cut() {
    // A low tone, so neighbouring frames are close and a cut grain stands
    // out as a step (a steady level would not show it: the level law
    // keeps a cloud of it steady whatever its grains do).
    let data = tone(480_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let p = Params {
        spray: 0.2,
        size_ms: 400.0,
        density: 100.0,
        spread: 0.0,
        ..dense()
    };
    let settle = || {
        let mut v = GrainVoice::new(RATE, 4);
        v.set_params(p);
        v.start();
        let mut budget = Budget::new(256);
        let (l, _) = run(&mut v, &src, &mut budget, 48_000, 128, |_| {});
        // No new grains from here, so only the fades change the count.
        v.set_share(0);
        (v, budget, largest_step(&l[24_000..]))
    };
    let fade = 240u32; // 5 ms
    let (mut v, mut budget, calm) = settle();
    let alive = v.alive();
    assert!(alive > 20, "{alive}");
    let want = alive / 2;
    assert_eq!(v.fade_oldest(want, fade), want);
    assert!(v.ending(fade) >= want);
    assert_eq!(v.stats().faded, want as u64);
    let (out, _) = run(&mut v, &src, &mut budget, fade as usize + 2, 1, |_| {});
    assert!(v.alive() <= alive - want, "{} alive", v.alive());
    let step = largest_step(&out);
    assert!(step < calm * 1.5, "{step} against {calm}");

    // The same grains ended within two frames do step: the test can see a
    // cut.
    let (mut v, mut budget, _) = settle();
    v.fade_oldest(want, 2);
    let (cut, _) = run(&mut v, &src, &mut budget, 8, 1, |_| {});
    let step = largest_step(&cut);
    assert!(step > calm * 3.0, "{step} against {calm}");

    // A grain already ending within the fade is left as it is.
    let (mut v, _, _) = settle();
    let soon = v.ending(fade);
    assert_eq!(v.fade_oldest(alive, fade), alive - soon);
}
