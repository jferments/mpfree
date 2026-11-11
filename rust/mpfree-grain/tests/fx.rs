//! The granular effect: grains from the last few seconds of a live signal,
//! mixed back in, with freeze.

use mpfree_grain::{Budget, GrainFx, GrainView, Params, MAX_GRAINS};

const RATE: u32 = 44_100;

fn sine(frames: usize, hz: f32, level: f32) -> Vec<f32> {
    (0..frames).map(|i| level * (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin()).collect()
}

fn cloud() -> Params {
    Params { position: 0.9, spray: 0.05, size_ms: 80.0, density: 30.0, spread: 0.5, ..Params::default() }
}

fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt() }

fn crossings(x: &[f32]) -> usize { x.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() }

/// Runs `fx` over `input` (the same signal both sides) in blocks of
/// `block`, applying each change on its exact frame. Returns left, right.
fn run(fx: &mut GrainFx, budget: &mut Budget, input: &[f32], block: usize, changes: &[(usize, Change)]) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (input.to_vec(), input.iter().map(|x| -x).collect::<Vec<f32>>());
    let mut at = 0;
    let mut next = 0;
    while at < input.len() {
        while next < changes.len() && changes[next].0 <= at {
            match changes[next].1 {
                Change::Mix(m) => fx.set_mix(m),
                Change::Freeze(on) => fx.set_frozen(on),
            }
            next += 1;
        }
        let stop = changes.get(next).map_or(input.len(), |c| c.0).min(at + block).min(input.len());
        fx.process(budget, at as u64, &mut l[at..stop], &mut r[at..stop]);
        at = stop;
    }
    (l, r)
}

#[derive(Clone, Copy)]
enum Change {
    Mix(f32),
    Freeze(bool),
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
    let input: Vec<f32> = (0..20_000).map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0).collect();
    let mut fx = GrainFx::new(RATE, 2.0, 1).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    let (l, r) = run(&mut fx, &mut budget, &input, 128, &[]);
    assert_eq!(l, input);
    assert!(r.iter().zip(&input).all(|(a, b)| *a == -*b));
    assert_eq!(fx.stats().spawned, 0);
    assert_eq!(budget.live(), 0);
    // The buffer still fills, so turning the mix up has something to play.
    assert_eq!(fx.buffer().written(), 20_000 - 20_000 % GrainFx::CHUNK as u64);
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
    assert!(rms(&held) > 0.1, "frozen, the cloud keeps playing: rms {}", rms(&held));
    assert!(rms(&gone) < 1e-4, "not frozen, it plays the silence that came after: rms {}", rms(&gone));
    // And it is still the 440 Hz tone (880 crossings a second).
    let c = crossings(&held) as f32;
    assert!((c / 880.0 - 1.0).abs() < 0.05, "{c} crossings");
}

#[test]
fn unfreezing_takes_in_new_audio_again() {
    // 440 Hz, frozen, then the input changes to 1 kHz and freeze is let go.
    let mut input = sine(RATE as usize, 440.0, 0.5);
    input.extend(sine(RATE as usize * 2, 1000.0, 0.5));
    let mut fx = GrainFx::new(RATE, 2.0, 4).unwrap();
    fx.set_params(cloud());
    let mut budget = Budget::new(64);
    let changes = [(0, Change::Mix(1.0)), (40_000, Change::Freeze(true)), (RATE as usize * 2, Change::Freeze(false))];
    let (l, _) = run(&mut fx, &mut budget, &input, 128, &changes);
    let frozen = crossings(&l[RATE as usize * 3 / 2..RATE as usize * 2]) as f32 / 0.5;
    let after = crossings(&l[RATE as usize * 5 / 2..]) as f32 / 0.5;
    assert!((frozen / 880.0 - 1.0).abs() < 0.05, "still 440 Hz while frozen: {frozen} crossings a second");
    assert!((after / 2000.0 - 1.0).abs() < 0.05, "1 kHz after letting go: {after} crossings a second");
}

#[test]
fn the_output_is_the_same_at_any_period_size() {
    let input: Vec<f32> = sine(RATE as usize * 2, 330.0, 0.4).iter().enumerate().map(|(i, x)| x + ((i * 31) % 97) as f32 * 1e-3).collect();
    let changes = [
        (1000, Change::Mix(0.6)),
        (30_011, Change::Freeze(true)),
        (50_000, Change::Mix(1.0)),
        (61_234, Change::Freeze(false)),
        (80_003, Change::Mix(0.0)),
    ];
    let render = |block: usize| {
        let mut fx = GrainFx::new(RATE, 1.0, 5).unwrap();
        fx.set_params(Params { spray: 0.3, pitch_spray: 3.0, reverse: 0.3, ..cloud() });
        let mut budget = Budget::new(64);
        run(&mut fx, &mut budget, &input, block, &changes)
    };
    let first = render(128);
    assert!(first.0 != input, "the effect did something");
    // As for a lone grain voice: the same to within float rounding of the
    // grains' read positions.
    for block in [1, 37, 64, 500, 4096] {
        let other = render(block);
        let worst = first.0.iter().chain(&first.1).zip(other.0.iter().chain(&other.1)).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "period {block}: {worst}");
    }
}

#[test]
fn the_mix_glides_and_lands_exactly() {
    let input = vec![0.5f32; 20_000];
    let mut fx = GrainFx::new(RATE, 1.0, 6).unwrap();
    // One 80 ms grain when the mix comes up, then none for ten seconds.
    fx.set_params(Params { density: 0.1, ..cloud() });
    let mut budget = Budget::new(64);
    let changes = [(1000, Change::Mix(1.0)), (10_000, Change::Mix(0.0))];
    let (l, _) = run(&mut fx, &mut budget, &input, 128, &changes);
    let worst = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(worst < 0.01, "a 10 ms glide, not a jump of 0.5: step {worst}");
    assert!(l[..1000].iter().all(|&x| x == 0.5));
    assert!(l[5000..10_000].iter().all(|&x| x == 0.0), "fully wet (and the one grain over)");
    assert!(l[10_000 + 441..].iter().all(|&x| x == 0.5), "and back to exactly dry");
    assert_eq!(fx.stats().spawned, 1);
}

#[test]
fn grains_share_the_budget_and_hand_it_back() {
    let input = sine(RATE as usize, 200.0, 0.5);
    let mut a = GrainFx::new(RATE, 1.0, 7).unwrap();
    let mut b = GrainFx::new(RATE, 1.0, 8).unwrap();
    for fx in [&mut a, &mut b] {
        fx.set_params(Params { size_ms: 200.0, density: 200.0, ..cloud() });
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
    fx.set_params(Params { position: 1.0, spray: 0.0, size_ms: 50.0, density: 40.0, ..Params::default() });
    assert!(fx.params().position < 1.0, "held just below 1, so it doesn't wrap round to the oldest");
    fx.set_mix(1.0);
    let mut budget = Budget::new(64);
    let (mut l, mut r) = (input.clone(), input.clone());
    fx.process(&mut budget, 0, &mut l, &mut r);
    let mut views = [GrainView::default(); MAX_GRAINS];
    let n = fx.grains(&mut views);
    assert!(n > 0);
    assert!(views[..n].iter().all(|g| g.position > 0.9), "{:?}", &views[..n]);
    assert!(fx.playhead() > 0.99);
}
