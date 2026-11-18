//! The granular effect: grains from the last few seconds of a live signal,
//! mixed back in, with freeze, feedback and repeat.

use mpfree_grain::fx::REPEAT_FADE_S;
use mpfree_grain::{Budget, GrainFx, GrainView, Params, MAX_GRAINS};
use mpfree_node::{Held, HeldStep};

const RATE: u32 = 44_100;

fn sine(frames: usize, hz: f32, level: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| level * (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin())
        .collect()
}

fn cloud() -> Params {
    Params {
        position: 0.9,
        spray: 0.05,
        size_ms: 80.0,
        density: 30.0,
        spread: 0.5,
        ..Params::default()
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn crossings(x: &[f32]) -> usize {
    x.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

/// Runs `fx` over `input` (the same signal both sides) in blocks of
/// `block`, applying each change on its exact frame. Returns left, right.
fn run(
    fx: &mut GrainFx,
    budget: &mut Budget,
    input: &[f32],
    block: usize,
    changes: &[(usize, Change)],
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (
        input.to_vec(),
        input.iter().map(|x| -x).collect::<Vec<f32>>(),
    );
    let mut at = 0;
    let mut next = 0;
    while at < input.len() {
        while next < changes.len() && changes[next].0 <= at {
            match changes[next].1 {
                Change::Mix(m) => fx.set_mix(m),
                Change::Freeze(on) => fx.set_frozen(on),
                Change::Feedback(f) => fx.set_feedback(f),
                Change::Repeat(beats) => fx.start_repeat(beats),
                Change::StopRepeat => fx.stop_repeat(),
            }
            next += 1;
        }
        let stop = changes
            .get(next)
            .map_or(input.len(), |c| c.0)
            .min(at + block)
            .min(input.len());
        fx.process(budget, at as u64, &mut l[at..stop], &mut r[at..stop]);
        at = stop;
    }
    (l, r)
}

#[derive(Clone, Copy)]
enum Change {
    Mix(f32),
    Freeze(bool),
    Feedback(f32),
    Repeat(f64),
    StopRepeat,
}

/// White noise at `level`, the same every run.
fn noise(frames: usize, level: f32) -> Vec<f32> {
    let mut seed = 0x1234_5678u32;
    (0..frames)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            level * ((seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0)
        })
        .collect()
}

/// Output frames per beat at 120 BPM, the effect's tempo until set.
const FPB: f64 = 22_050.0;

/// A repeat of `beats`: its length and its slice's (frames).
fn slice_of(beats: f64) -> (f64, usize) {
    let len = beats * FPB;
    let fade = (REPEAT_FADE_S * RATE as f64).max(len / 99.0).ceil();
    (len, (len + fade).ceil() as usize)
}

/// The worst difference, over each repeat `k` in `ks` from frame `t0`
/// every `len` frames, between the output and the input from `from`, away
/// from the joins.
fn repeat_error(
    out: &[f32],
    input: &[f32],
    t0: usize,
    len: f64,
    from: usize,
    ks: std::ops::Range<usize>,
) -> f32 {
    let edge = (REPEAT_FADE_S * RATE as f64).max(len / 99.0).ceil() as usize + 20;
    let mut worst = 0.0f32;
    for k in ks {
        let at = t0 + (k as f64 * len).round() as usize;
        for j in edge..len as usize - edge {
            worst = worst.max((out[at + j] - input[from + j]).abs());
        }
    }
    worst
}

/// A second of 440 Hz then `secs` of silence through a full-wet cloud
/// reading a second back, with `feedback`, in blocks of `block`: left.
fn trails(feedback: f32, secs: usize, block: usize) -> Vec<f32> {
    let mut input = sine(RATE as usize, 440.0, 0.5);
    input.resize(RATE as usize * (1 + secs), 0.0);
    let mut fx = GrainFx::new(RATE, 2.0, 3).unwrap();
    fx.set_params(Params {
        position: 0.5,
        ..cloud()
    });
    let mut budget = Budget::new(64);
    let changes = [(0, Change::Mix(1.0)), (0, Change::Feedback(feedback))];
    run(&mut fx, &mut budget, &input, block, &changes).0
}

/// RMS of second `k` of `x`.
fn second(x: &[f32], k: usize) -> f32 {
    rms(&x[k * RATE as usize..(k + 1) * RATE as usize])
}

#[test]
fn new_checks_its_arguments() {
    assert!(GrainFx::new(0, 2.0, 1).is_none());
    assert!(GrainFx::new(RATE, 0.1, 1).is_none());
    assert!(GrainFx::new(RATE, 31.0, 1).is_none());
    assert!(GrainFx::new(RATE, f32::NAN, 1).is_none());
    let fx = GrainFx::new(RATE, 2.0, 1).unwrap();
    assert_eq!(fx.buffer().capacity(), 88_200);
    assert_eq!((fx.mix(), fx.frozen()), (0.0, false));
}

#[test]
fn fully_dry_passes_the_input_through_exactly_and_makes_no_grains() {
    let input: Vec<f32> = (0..20_000)
        .map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0)
        .collect();
    let mut fx = GrainFx::new(RATE, 2.0, 1).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    let (l, r) = run(&mut fx, &mut budget, &input, 128, &[]);
    assert_eq!(l, input);
    assert!(r.iter().zip(&input).all(|(a, b)| *a == -*b));
    assert_eq!(fx.stats().spawned, 0);
    assert_eq!(budget.live(), 0);
    // The buffer still fills, so turning the mix up has something to play.
    assert_eq!(
        fx.buffer().written(),
        20_000 - 20_000 % GrainFx::CHUNK as u64
    );
}

#[test]
fn freeze_keeps_the_sound_after_the_input_stops() {
    // A second of 440 Hz, then two seconds of silence.
    let mut input = sine(RATE as usize, 440.0, 0.5);
    input.resize(RATE as usize * 3, 0.0);
    let tail = |freeze: bool| {
        let mut fx = GrainFx::new(RATE, 2.0, 3).unwrap();
        fx.set_params(cloud());
        let mut budget = Budget::new(64);
        let mut changes = vec![(0, Change::Mix(1.0))];
        if freeze {
            changes.push((40_000, Change::Freeze(true)));
        }
        let (l, _) = run(&mut fx, &mut budget, &input, 128, &changes);
        l[RATE as usize * 2..].to_vec() // the last second: a second after the input stopped
    };
    let (held, gone) = (tail(true), tail(false));
    assert!(
        rms(&held) > 0.1,
        "frozen, the cloud keeps playing: rms {}",
        rms(&held)
    );
    assert!(
        rms(&gone) < 1e-4,
        "not frozen, it plays the silence that came after: rms {}",
        rms(&gone)
    );
    // And it is still the 440 Hz tone (880 crossings a second).
    let c = crossings(&held) as f32;
    assert!((c / 880.0 - 1.0).abs() < 0.05, "{c} crossings");
}

#[test]
fn feedback_plays_the_grains_again_and_dies_away_below_full() {
    let none = trails(0.0, 9, 128);
    let half = trails(0.5, 9, 128);
    // Without feedback the cloud has played the tone through by second 3.
    assert!(second(&none, 3) < 1e-4, "{}", second(&none, 3));
    // With it the tone comes round again, a little quieter each time.
    let levels: Vec<f32> = (2..9).map(|k| second(&half, k)).collect();
    assert!(levels[0] > 0.05, "{levels:?}");
    assert!(levels[6] < 0.5 * levels[0], "it dies away: {levels:?}");
    assert!(levels.iter().all(|x| x.is_finite() && *x > 0.0));
    // At 0 the effect is exactly as it was (the same as not setting it).
    let mut fx = GrainFx::new(RATE, 2.0, 3).unwrap();
    fx.set_params(Params {
        position: 0.5,
        ..cloud()
    });
    let mut input = sine(RATE as usize, 440.0, 0.5);
    input.resize(RATE as usize * 10, 0.0);
    let plain = run(
        &mut fx,
        &mut Budget::new(64),
        &input,
        128,
        &[(0, Change::Mix(1.0))],
    )
    .0;
    assert_eq!(plain, none);
}

#[test]
fn full_feedback_keeps_the_cloud_going_and_never_runs_away() {
    // Loud input and full feedback: the trail keeps going, bounded.
    let full = trails(1.0, 19, 128);
    let late: Vec<f32> = (10..20).map(|k| second(&full, k)).collect();
    assert!(late.iter().all(|x| *x > 0.02), "it keeps going: {late:?}");
    let peak = full.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak.is_finite() && peak < 2.0, "peak {peak}");
    // No growth over its last ten seconds.
    assert!(late[9] < 1.5 * late[0], "{late:?}");
}

#[test]
fn feedback_is_the_same_at_any_period_size() {
    let a = trails(0.7, 3, 128);
    for block in [1, 64, 333, 4096] {
        let b = trails(0.7, 3, block);
        let worst = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-4, "block {block}: {worst}");
    }
}

#[test]
fn unfreezing_takes_in_new_audio_again() {
    // 440 Hz, frozen, then the input changes to 1 kHz and freeze is let go.
    let mut input = sine(RATE as usize, 440.0, 0.5);
    input.extend(sine(RATE as usize * 2, 1000.0, 0.5));
    let mut fx = GrainFx::new(RATE, 2.0, 4).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    let changes = [
        (0, Change::Mix(1.0)),
        (40_000, Change::Freeze(true)),
        (RATE as usize * 2, Change::Freeze(false)),
    ];
    let (l, _) = run(&mut fx, &mut budget, &input, 128, &changes);
    let frozen = crossings(&l[RATE as usize * 3 / 2..RATE as usize * 2]) as f32 / 0.5;
    let after = crossings(&l[RATE as usize * 5 / 2..]) as f32 / 0.5;
    assert!(
        (frozen / 880.0 - 1.0).abs() < 0.05,
        "still 440 Hz while frozen: {frozen} crossings a second"
    );
    assert!(
        (after / 2000.0 - 1.0).abs() < 0.05,
        "1 kHz after letting go: {after} crossings a second"
    );
}

#[test]
fn the_output_is_the_same_at_any_period_size() {
    let input: Vec<f32> = sine(RATE as usize * 2, 330.0, 0.4)
        .iter()
        .enumerate()
        .map(|(i, x)| x + ((i * 31) % 97) as f32 * 1e-3)
        .collect();
    let changes = [
        (1000, Change::Mix(0.6)),
        (30_011, Change::Freeze(true)),
        (50_000, Change::Mix(1.0)),
        (61_234, Change::Freeze(false)),
        (80_003, Change::Mix(0.0)),
    ];
    let render = |block: usize| {
        let mut fx = GrainFx::new(RATE, 1.0, 5).unwrap();
        fx.set_params(Params {
            spray: 0.3,
            pitch_spray: 3.0,
            reverse: 0.3,
            ..cloud()
        });
        let mut budget = Budget::new(64);
        run(&mut fx, &mut budget, &input, block, &changes)
    };
    let first = render(128);
    assert!(first.0 != input, "the effect did something");
    // As for a lone grain voice: the same to within float rounding of the
    // grains' read positions.
    for block in [1, 37, 64, 500, 4096] {
        let other = render(block);
        let worst = first
            .0
            .iter()
            .chain(&first.1)
            .zip(other.0.iter().chain(&other.1))
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "period {block}: {worst}");
    }
}

#[test]
fn the_mix_glides_and_lands_exactly() {
    let input = vec![0.5f32; 20_000];
    let mut fx = GrainFx::new(RATE, 1.0, 6).unwrap();
    // One 80 ms grain when the mix comes up, then none for ten seconds.
    fx.set_params(Params {
        density: 0.1,
        ..cloud()
    });
    let mut budget = Budget::new(64);
    let changes = [(1000, Change::Mix(1.0)), (10_000, Change::Mix(0.0))];
    let (l, _) = run(&mut fx, &mut budget, &input, 128, &changes);
    let worst = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        worst < 0.01,
        "a 10 ms glide, not a jump of 0.5: step {worst}"
    );
    assert!(l[..1000].iter().all(|&x| x == 0.5));
    assert!(
        l[5000..10_000].iter().all(|&x| x == 0.0),
        "fully wet (and the one grain over)"
    );
    assert!(
        l[10_000 + 441..].iter().all(|&x| x == 0.5),
        "and back to exactly dry"
    );
    assert_eq!(fx.stats().spawned, 1);
}

#[test]
fn grains_share_the_budget_and_hand_it_back() {
    let input = sine(RATE as usize, 200.0, 0.5);
    let mut a = GrainFx::new(RATE, 1.0, 7).unwrap();
    let mut b = GrainFx::new(RATE, 1.0, 8).unwrap();
    for fx in [&mut a, &mut b] {
        // Regular timing, so a freed slot is taken at once and the budget
        // is full at the end.
        fx.set_params(Params {
            size_ms: 200.0,
            density: 200.0,
            timing: 0.0,
            ..cloud()
        });
        fx.set_mix(1.0);
    }
    let mut budget = Budget::new(10);
    let (mut l, mut r) = (input.clone(), input.clone());
    for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        a.process(&mut budget, 0, cl, cr);
        b.process(&mut budget, 0, cl, cr);
        assert!(budget.live() <= 10);
    }
    assert!(budget.skipped > 0 && budget.live() == 10);
    // Fully dry stops the grains and hands them back once the glide ends.
    a.set_mix(0.0);
    a.process(&mut budget, 0, &mut l[..500], &mut r[..500]);
    assert_eq!(a.stats().active, 0);
    b.kill(&mut budget);
    assert_eq!(budget.live(), 0);
}

#[test]
fn position_one_reads_the_newest_audio() {
    let input = sine(RATE as usize, 300.0, 0.5);
    let mut fx = GrainFx::new(RATE, 2.0, 9).unwrap();
    fx.set_params(Params {
        position: 1.0,
        spray: 0.0,
        size_ms: 50.0,
        density: 40.0,
        ..Params::default()
    });
    assert!(
        fx.params().position < 1.0,
        "held just below 1, so it doesn't wrap round to the oldest"
    );
    fx.set_mix(1.0);
    let mut budget = Budget::new(64);
    let (mut l, mut r) = (input.clone(), input.clone());
    fx.process(&mut budget, 0, &mut l, &mut r);
    let mut views = [GrainView::default(); MAX_GRAINS];
    let n = fx.grains(&mut views);
    assert!(n > 0);
    assert!(
        views[..n].iter().all(|g| g.position > 0.9),
        "{:?}",
        &views[..n]
    );
    assert!(fx.playhead() > 0.99);
}

#[test]
fn repeat_plays_the_last_stretch_again_and_again_in_place_of_the_input() {
    let input = noise(RATE as usize * 3, 0.3);
    let mut fx = GrainFx::new(RATE, 2.0, 3).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    // Fully dry, so all that differs from the input is the repeat: a
    // sixteenth (a quarter beat) from frame t0.
    let t0 = 44_100;
    let (len, whole) = slice_of(0.25);
    let (l, r) = run(
        &mut fx,
        &mut budget,
        &input,
        128,
        &[(t0, Change::Repeat(0.25))],
    );
    assert_eq!(l[..t0], input[..t0], "the input until it starts");
    assert!(fx.repeating());
    // Each repeat is the slice that ended on t0, from its start.
    let from = t0 - whole + 1;
    let worst = repeat_error(&l, &input, t0, len, from, 1..12);
    assert!(worst < 1e-3, "left: {worst}");
    let minus: Vec<f32> = input.iter().map(|x| -x).collect();
    let worst = repeat_error(&r, &minus, t0, len, from, 1..12);
    assert!(worst < 1e-3, "right: {worst}");
    // At the level of the input, joins included.
    let after = &l[t0 + 2 * len as usize..t0 + 10 * len as usize];
    let db = 20.0 * (rms(after) / rms(&input[..t0])).log10();
    assert!(db.abs() < 0.5, "{db:.2} dB");
}

#[test]
fn stopping_a_repeat_crossfades_back_to_the_input_in_10_ms() {
    let input = noise(RATE as usize * 2, 0.3);
    let mut fx = GrainFx::new(RATE, 2.0, 3).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    let (t0, t1) = (20_000, 50_000);
    let changes = [(t0, Change::Repeat(0.5)), (t1, Change::StopRepeat)];
    let (l, r) = run(&mut fx, &mut budget, &input, 128, &changes);
    let diff: Vec<f32> = l[t0 + 441..t1]
        .iter()
        .zip(&input[t0 + 441..t1])
        .map(|(a, b)| a - b)
        .collect();
    assert!(rms(&diff) > 0.1, "repeating: {}", rms(&diff));
    // 10 ms (441 frames) after the stop, and the rest of the chunk, it is
    // the input exactly again.
    let back = t1 + 441 + GrainFx::CHUNK;
    assert_eq!(l[back..], input[back..]);
    assert!(r[back..].iter().zip(&input[back..]).all(|(a, b)| *a == -b));
    assert!(!fx.repeating());
    assert_eq!(fx.live(), 0);
}

#[test]
fn a_shorter_repeat_while_repeating_rolls_the_start_of_the_slice() {
    let input = noise(RATE as usize * 5, 0.3);
    let mut fx = GrainFx::new(RATE, 2.0, 3).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    // A beat from t0, then eighths from three beats on.
    let t0 = 30_000;
    let (len, whole) = slice_of(1.0);
    let t1 = t0 + 3 * len as usize;
    let (half, _) = slice_of(0.5);
    let changes = [(t0, Change::Repeat(1.0)), (t1, Change::Repeat(0.5))];
    let (l, _) = run(&mut fx, &mut budget, &input, 128, &changes);
    let from = t0 - whole + 1;
    assert!(repeat_error(&l, &input, t0, len, from, 1..3) < 1e-3);
    // The eighths repeat the first half of the same slice.
    let worst = repeat_error(&l, &input, t1, half, from, 1..8);
    assert!(worst < 1e-3, "{worst}");
}

#[test]
fn a_long_repeat_holds_the_memory_so_its_slice_is_never_overwritten() {
    // Two seconds of memory, one-second repeats for four seconds.
    let input = noise(RATE as usize * 7, 0.3);
    let mut fx = GrainFx::new(RATE, 2.0, 3).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    let t0 = 60_000;
    let (len, whole) = slice_of(2.0);
    let t1 = t0 + 4 * len as usize + 1000;
    let (l, _) = run(
        &mut fx,
        &mut budget,
        &input[..t1],
        128,
        &[(t0, Change::Repeat(2.0))],
    );
    let worst = repeat_error(&l, &input, t0, len, t0 - whole + 1, 1..4);
    assert!(worst < 1e-3, "{worst}");
    let held = fx.buffer().written();
    assert!(held < t1 as u64 - 44_100, "held at {held}");
    // Once it has faded back, the input goes in again.
    fx.stop_repeat();
    let mut rest = input[t1..].to_vec();
    let mut minus: Vec<f32> = rest.iter().map(|x| -x).collect();
    fx.process(&mut budget, t1 as u64, &mut rest, &mut minus);
    assert!(fx.buffer().written() >= held + rest.len() as u64 - 1000);
}

#[test]
fn a_repeat_is_the_same_at_any_period_size() {
    let input = noise(RATE as usize * 2, 0.3);
    let changes = [
        (0, Change::Mix(0.5)),
        (10_007, Change::Repeat(0.25)),
        (21_000, Change::Repeat(0.125)),
        (33_333, Change::StopRepeat),
        (40_001, Change::Repeat(0.5)),
        (40_300, Change::StopRepeat),
        (40_500, Change::Repeat(0.25)),
    ];
    let render = |block: usize| {
        let mut fx = GrainFx::new(RATE, 1.0, 5).unwrap();
        fx.set_params(Params {
            pitch: 5.0,
            reverse: 0.4,
            ..cloud()
        });
        let mut budget = Budget::new(64);
        run(&mut fx, &mut budget, &input, block, &changes)
    };
    let first = render(128);
    for block in [1, 37, 64, 500, 4096] {
        let other = render(block);
        let worst = first
            .0
            .iter()
            .chain(&first.1)
            .zip(other.0.iter().chain(&other.1))
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "period {block}: {worst}");
    }
}

/// The input as the memory keeps it with no feedback: left `x`, right `-x`
/// (as [`run`] feeds it), interleaved.
fn kept(input: &[f32]) -> Vec<f32> {
    input.iter().flat_map(|&x| [x, -x]).collect()
}

/// Feeds `input` to `fx` as [`run`] does, in one go from `frame` on.
fn feed(fx: &mut GrainFx, budget: &mut Budget, input: &[f32], frame: usize) {
    let mut l = input.to_vec();
    let mut r: Vec<f32> = input.iter().map(|x| -x).collect();
    fx.process(budget, frame as u64, &mut l, &mut r);
}

#[test]
fn a_capture_copies_the_frozen_memory_exactly_a_piece_at_a_time() {
    let input = noise(66_150, 0.5);
    let mut fx = GrainFx::new(RATE, 1.0, 9).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    run(&mut fx, &mut budget, &input, 128, &[(0, Change::Mix(1.0))]);
    fx.set_frozen(true);
    let mut held = Held::new(2, RATE as usize * 2);
    let (mut calls, mut at) = (0, input.len());
    loop {
        calls += 1;
        match fx.copy_held(&mut held, 4096) {
            HeldStep::More => {
                // The cloud plays on between pieces; frozen, its memory
                // stays as it was.
                feed(&mut fx, &mut budget, &noise(256, 0.9), at);
                at += 256;
            }
            HeldStep::Done => break,
            s => panic!("{s:?}"),
        }
    }
    assert_eq!(calls, 11, "44,100 frames, 4,096 a call");
    let from = input.len() - RATE as usize;
    assert_eq!(held.span(), Some((from as u64, input.len() as u64)));
    assert_eq!((held.channels(), held.rate()), (2, RATE));
    assert!(
        held.data() == &kept(&input)[2 * from..],
        "the memory, exactly"
    );
}

#[test]
fn a_capture_while_audio_comes_in_keeps_the_span_it_began_with() {
    let input = noise(RATE as usize * 3, 0.5);
    let mut fx = GrainFx::new(RATE, 1.0, 9).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    let first = RATE as usize * 2;
    run(
        &mut fx,
        &mut budget,
        &input[..first],
        128,
        &[(0, Change::Mix(1.0))],
    );
    let written = fx.buffer().written() as usize;
    let mut held = Held::new(2, RATE as usize);
    let mut at = first;
    loop {
        match fx.copy_held(&mut held, 2048) {
            HeldStep::More => {
                feed(&mut fx, &mut budget, &input[at..at + 512], at);
                at += 512;
            }
            HeldStep::Done => break,
            s => panic!("{s:?}"),
        }
    }
    assert!(
        fx.buffer().written() as usize > written + 8_000,
        "audio came in"
    );
    // All but the oldest quarter, which new audio was about to overwrite.
    let from = written - RATE as usize * 3 / 4;
    assert_eq!(held.span(), Some((from as u64, written as u64)));
    assert!(held.data() == &kept(&input)[2 * from..2 * written]);
}

#[test]
fn a_capture_says_when_there_is_nothing_or_the_memory_went() {
    let mut fx = GrainFx::new(RATE, 1.0, 9).unwrap();
    let mut budget = Budget::new(64);
    let mut held = Held::new(2, RATE as usize);
    assert_eq!(fx.copy_held(&mut held, 4096), HeldStep::Nothing, "empty");
    feed(&mut fx, &mut budget, &noise(20_000, 0.5), 0);
    assert_eq!(fx.copy_held(&mut held, 4096), HeldStep::More);
    // A reset forgets the memory being copied.
    fx.reset(&mut budget);
    feed(&mut fx, &mut budget, &noise(20_000, 0.5), 0);
    assert_eq!(fx.copy_held(&mut held, 4096), HeldStep::Lost);
    // A copy far slower than the audio coming in is overtaken.
    let mut held = Held::new(2, RATE as usize);
    feed(&mut fx, &mut budget, &noise(RATE as usize, 0.5), 20_000);
    let mut at = 20_000 + RATE as usize;
    let lost = (0..100).any(|_| {
        let step = fx.copy_held(&mut held, 16);
        feed(&mut fx, &mut budget, &noise(4096, 0.5), at);
        at += 4096;
        step == HeldStep::Lost
    });
    assert!(lost, "overwritten, never a mix of old and new");
}
