//! The Ducker, Transient shaper and Pitch shifter each do what they say:
//! the ducker dips on a key and on the beat and comes back, the shaper
//! lifts or softens the front of a hit and leaves a steady tone alone, and
//! the shifter moves a tone by the interval asked.

use std::f32::consts::PI;

use mpfree_fx::ids::{ducker, pitch, transient};
use mpfree_fx::mpfree_node::{Ctx, Node};
use mpfree_fx::*;

const RATE: u32 = 48_000;

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

fn set_up<N: Node>(mut n: N, values: &[(u16, f32)]) -> N {
    n.prepare(RATE, 512);
    for &(id, v) in values {
        n.set_param(id, v);
    }
    n.reset();
    n
}

/// Runs `n` in 256-frame blocks, with `key` as the sidechain and the
/// transport playing from beat 0 at `bpm` when `playing`.
fn run(n: &mut dyn Node, x: &[f32], key: Option<&[f32]>, bpm: f64, playing: bool) -> Vec<f32> {
    let (mut l, mut r) = (x.to_vec(), x.to_vec());
    let mut at = 0;
    while at < x.len() {
        let len = 256.min(x.len() - at);
        let side = key.map(|k| (&k[at..at + len], &k[at..at + len]));
        let ctx = Ctx {
            bpm,
            beats: at as f64 * bpm / 60.0 / RATE as f64,
            playing,
            sidechain: side,
            ..Ctx::new(RATE, at as u64)
        };
        n.process(&ctx, &[], &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    assert_eq!(l, r, "a mono input stays mono");
    l
}

/// The gain at each 1 ms of `y` against a steady `amp` sine input.
fn gain_db(y: &[f32], amp: f32, at_ms: usize) -> f32 {
    let ms = RATE as usize / 1000;
    let w = &y[at_ms * ms..(at_ms + 5) * ms];
    20.0 * (rms(w) / (amp / 2f32.sqrt())).log10()
}

#[test]
fn the_ducker_dips_on_the_key_and_comes_back() {
    let frames = RATE as usize;
    let x = sine(1000.0, 0.5, frames);
    // A 50 ms kick at 100 ms.
    let key: Vec<f32> = (0..frames)
        .map(|i| if (4800..7200).contains(&i) { 0.9 } else { 0.0 })
        .collect();
    let mut d = set_up(
        Ducker::new(),
        &[
            (ducker::DEPTH, 18.0),
            (ducker::ATTACK, 1.0),
            (ducker::HOLD, 0.0),
            (ducker::RELEASE, 200.0),
        ],
    );
    let y = run(&mut d, &x, Some(&key), 120.0, false);
    assert!(gain_db(&y, 0.5, 50).abs() < 0.1, "untouched before the key");
    // Held down while the key is over (and the follower's fall after).
    let held = gain_db(&y, 0.5, 120);
    assert!(
        (held + 18.0).abs() < 0.5,
        "down by Depth while held: {held}"
    );
    // Coming back during the release, then all the way.
    let mid = gain_db(&y, 0.5, 260);
    assert!(mid < -1.0 && mid > -17.0, "part way back: {mid}");
    assert!(gain_db(&y, 0.5, 600).abs() < 0.1, "back after the release");
    assert_eq!(d.param(ducker::GAIN), 0.0);

    // No key, no duck.
    let mut d = set_up(Ducker::new(), &[]);
    let y = run(&mut d, &x, None, 120.0, false);
    assert_eq!(y, x);
}

#[test]
fn the_ducker_pumps_on_the_beat() {
    // 120 BPM quarters: a trigger every 24 000 frames.
    let frames = 2 * RATE as usize;
    let x = sine(500.0, 0.5, frames);
    for playing in [true, false] {
        let mut d = set_up(
            Ducker::new(),
            &[
                (ducker::SOURCE, 1.0),
                (ducker::DEPTH, 24.0),
                (ducker::ATTACK, 1.0),
                (ducker::HOLD, 0.0),
                (ducker::RELEASE, 100.0),
            ],
        );
        let y = run(&mut d, &x, None, 120.0, playing);
        for beat in 0..4 {
            let at = beat * 500;
            let down = gain_db(&y, 0.5, at + 2);
            let up = gain_db(&y, 0.5, at + 300);
            assert!(down < -18.0, "beat {beat} ducks ({playing}): {down}");
            assert!(up.abs() < 0.1, "beat {beat} recovers ({playing}): {up}");
        }
    }
}

#[test]
fn the_ducker_curve_draws_attack_hold_and_release() {
    let d = set_up(
        Ducker::new(),
        &[
            (ducker::ATTACK, 10.0),
            (ducker::HOLD, 10.0),
            (ducker::RELEASE, 80.0),
        ],
    );
    let c = d.curve(100.0, 101);
    assert_eq!(c[0], 0.0);
    assert!((c[5] - 0.5).abs() < 1e-5);
    assert_eq!(c[15], 1.0);
    assert!(c[60] < 1.0 && c[60] > 0.0);
    assert!(c[100].abs() < 1e-6);
}

/// Bursts of a 200 Hz tone: 10 ms hits every 250 ms with an exponential
/// decay, like plucks.
fn hits(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let t = (i % 12_000) as f32 / RATE as f32;
            0.5 * (-t * 25.0).exp() * (2.0 * PI * 200.0 * t).sin()
        })
        .collect()
}

#[test]
fn the_transient_shaper_lifts_and_softens_the_front() {
    let frames = RATE as usize;
    let x = hits(frames);
    let front = |y: &[f32]| rms(&y[12_000..12_000 + 240]);
    let tail = |y: &[f32]| rms(&y[12_000 + 4800..12_000 + 9600]);
    let mut flat = set_up(TransientShaper::new(), &[(transient::ATTACK, 0.0)]);
    let y0 = run(&mut flat, &x, None, 120.0, false);
    assert_eq!(y0, x, "no attack and no sustain changes nothing");
    let mut up = set_up(TransientShaper::new(), &[(transient::ATTACK, 100.0)]);
    let yu = run(&mut up, &x, None, 120.0, false);
    let mut down = set_up(TransientShaper::new(), &[(transient::ATTACK, -100.0)]);
    let yd = run(&mut down, &x, None, 120.0, false);
    assert!(front(&yu) > 1.5 * front(&x), "more attack");
    assert!(front(&yd) < 0.7 * front(&x), "less attack");
    let mut long = set_up(
        TransientShaper::new(),
        &[(transient::ATTACK, 0.0), (transient::SUSTAIN, 100.0)],
    );
    let yl = run(&mut long, &x, None, 120.0, false);
    assert!(tail(&yl) > 1.5 * tail(&x), "more sustain");

    // A steady tone has no front and no tail: after it settles the gain is
    // back at 0 dB.
    let s = sine(300.0, 0.3, frames);
    let mut sh = set_up(
        TransientShaper::new(),
        &[(transient::ATTACK, 100.0), (transient::SUSTAIN, 100.0)],
    );
    let y = run(&mut sh, &s, None, 120.0, false);
    let g = gain_db(&y, 0.3, 800);
    assert!(g.abs() < 0.3, "steady tone untouched: {g} dB");
}

/// The tone's frequency from its zero crossings over `y`.
fn freq(y: &[f32]) -> f32 {
    let crossings = y.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    crossings as f32 * RATE as f32 / y.len() as f32
}

#[test]
fn the_pitch_shifter_moves_a_tone_by_the_interval() {
    let frames = RATE as usize;
    let x = sine(440.0, 0.5, frames);
    for (semis, want) in [(12.0, 880.0), (-12.0, 220.0), (7.0, 659.26), (0.0, 440.0)] {
        let mut p = set_up(PitchShifter::new(), &[(pitch::PITCH, semis)]);
        let y = run(&mut p, &x, None, 120.0, false);
        let f = freq(&y[RATE as usize / 4..]);
        assert!(
            (f / want - 1.0).abs() < 0.02,
            "{semis} semitones: {f} Hz, want {want}"
        );
        let level = rms(&y[RATE as usize / 4..]) / rms(&x);
        assert!(level > 0.6 && level < 1.3, "{semis}: level {level}");
    }
    // Spread detunes the sides apart.
    let mut p = set_up(
        PitchShifter::new(),
        &[(pitch::PITCH, 0.0), (pitch::SPREAD, 50.0)],
    );
    let (mut l, mut r) = (x.clone(), x.clone());
    for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
        p.process(&Ctx::new(RATE, 0), &[], a, b);
    }
    let (fl, fr) = (freq(&l[12_000..]), freq(&r[12_000..]));
    assert!(fl < 440.0 && fr > 440.0, "left {fl}, right {fr}");
}
