//! The Ensemble against the wave 1 targets it carries
//! (`docs/dev/design/synth-quality.md`, S3 and S4) and the FX rules: the
//! stereo width a string machine has, a top end like the bucket-brigade
//! lines', its level against the dry, and no click when Lines changes.
//! Its pitch movement (S1, S2) is checked frame by frame in the module's
//! own tests; block sizes, extremes and presets in `coverage.rs`.

use mpfree_fx::ids::ensemble as ids;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_fx::Ensemble;

const RATE: u32 = 48_000;

fn noise(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        })
        .collect()
}

fn render(
    e: &mut Ensemble,
    l: &[f32],
    r: &[f32],
    events: &[(usize, u16, f32)],
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (l.to_vec(), r.to_vec());
    let ctx = Ctx::new(RATE, 0);
    let block = 256;
    let mut at = 0;
    while at < l.len() {
        let end = (at + block).min(l.len());
        let ev: Vec<Timed> = events
            .iter()
            .filter(|(f, _, _)| *f >= at && *f < end)
            .map(|&(f, id, value)| Timed::new((f - at) as u32, NodeEvent::Param { id, value }))
            .collect();
        e.process(&ctx, &ev, &mut l[at..end], &mut r[at..end]);
        at = end;
    }
    (l, r)
}

fn ensemble(values: &[(u16, f32)]) -> Ensemble {
    let mut e = Ensemble::new();
    e.prepare(RATE, 256);
    for &(id, v) in values {
        e.set_param(id, v);
    }
    e.reset();
    e
}

fn power(x: &[f32]) -> f64 {
    x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64
}

fn corr(a: &[f32], b: &[f32]) -> f64 {
    let ab: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    ab / (power(a) * power(b)).sqrt() / a.len() as f64
}

/// The two sides' correlation and the mono fold-down's loss in dB, on mono
/// noise after the first second.
fn width(values: &[(u16, f32)]) -> (f64, f64) {
    let x = noise(4 * RATE as usize, 1);
    let mut e = ensemble(values);
    let (l, r) = render(&mut e, &x, &x, &[]);
    let s = RATE as usize;
    let (l, r) = (&l[s..], &r[s..]);
    let mono: Vec<f32> = l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect();
    let loss = 10.0 * (power(&mono) / (0.5 * (power(l) + power(r)))).log10();
    (corr(l, r), loss)
}

#[test]
fn it_is_as_wide_as_a_string_machine_and_survives_mono() {
    // S3: correlation 0.2 to 0.6 with the ensemble on; mono loss under
    // 3 dB.
    for lines in [0.0, 1.0] {
        let (c, loss) = width(&[(ids::LINES, lines)]);
        println!("lines {lines}: correlation {c:.3}, mono loss {loss:.2} dB");
        assert!((0.2..=0.6).contains(&c), "lines {lines}: {c:.3}");
        assert!(loss > -3.0, "lines {lines}: {loss:.2} dB");
    }
    // Width 0 is mono.
    let (c, _) = width(&[(ids::WIDTH, 0.0)]);
    assert!(c > 0.999, "{c}");
}

/// The wet path's level at `hz` against 500 Hz, dB, with the lines held
/// still (so they add as one).
fn top_end(hz: f32, tone: f32) -> f64 {
    let level = |f: f32| {
        let n = RATE as usize;
        let x: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * f * i as f32 / RATE as f32).sin() * 0.25)
            .collect();
        let mut e = ensemble(&[
            (ids::SLOW_DEPTH, 0.0),
            (ids::FAST_DEPTH, 0.0),
            (ids::TONE, tone),
        ]);
        let (l, _) = render(&mut e, &x, &x, &[]);
        power(&l[n / 2..])
    };
    10.0 * (level(hz) / level(500.0)).log10()
}

#[test]
fn the_top_end_rolls_off_like_the_lines_it_copies() {
    // S4: the wet path 3 dB down between 6 and 9 kHz at the default Tone.
    let at6 = top_end(6_000.0, 7_500.0);
    let at9 = top_end(9_000.0, 7_500.0);
    println!("6 kHz {at6:.2} dB, 9 kHz {at9:.2} dB");
    assert!(at6 > -3.0 && at9 < -3.0, "{at6:.2} {at9:.2}");
    // Tone moves it.
    assert!(top_end(9_000.0, 15_000.0) > -1.5);
}

#[test]
fn the_wet_sound_is_about_as_loud_as_the_dry() {
    let x = noise(4 * RATE as usize, 2);
    for lines in [0.0, 1.0] {
        let mut e = ensemble(&[(ids::LINES, lines), (ids::TONE, 20_000.0)]);
        let (l, r) = render(&mut e, &x, &x, &[]);
        let s = RATE as usize;
        let wet = 0.5 * (power(&l[s..]) + power(&r[s..]));
        let db = 10.0 * (wet / power(&x[s..])).log10();
        println!("lines {lines}: {db:+.2} dB");
        assert!(db.abs() < 1.5, "lines {lines}: {db:+.2} dB");
    }
}

#[test]
fn changing_lines_or_depth_never_clicks() {
    // A low tone, so any step stands out against its smooth slope.
    let n = 2 * RATE as usize;
    let x: Vec<f32> = (0..n)
        .map(|i| (std::f32::consts::TAU * 110.0 * i as f32 / RATE as f32).sin() * 0.5)
        .collect();
    let steady = {
        let mut e = ensemble(&[]);
        let (l, _) = render(&mut e, &x, &x, &[]);
        l.windows(2)
            .skip(RATE as usize / 2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    };
    for events in [
        vec![
            (RATE as usize / 2, ids::LINES, 1.0),
            (RATE as usize, ids::LINES, 0.0),
        ],
        vec![
            (RATE as usize / 2, ids::SLOW_DEPTH, 30.0),
            (RATE as usize, ids::FAST_DEPTH, 15.0),
        ],
        vec![
            (RATE as usize / 2, ids::WIDTH, 0.0),
            (RATE as usize, ids::TONE, 2_000.0),
        ],
    ] {
        let mut e = ensemble(&[]);
        let (l, _) = render(&mut e, &x, &x, &events);
        let worst = l
            .windows(2)
            .skip(RATE as usize / 2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1.5 * steady, "{events:?}: {worst} against {steady}");
    }
}
