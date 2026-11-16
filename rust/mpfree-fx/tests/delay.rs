//! The delays' loops (FX quality plan, section 5): the first repeat lands
//! on its time (D1); the loop keeps its band at any fraction of a frame,
//! so the repeats' brightness never depends on the tempo (D2); each pass
//! round the tape is the tone the page draws (D3); and the tape's first
//! echo has been round it once already (D8).

use mpfree_fx::ids::{delay as dl, multitap as mt, tape};
use mpfree_fx::mpfree_node::{Ctx, Node};
use mpfree_fx::{registry, Svf, SvfMode};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const DIGITAL: &str = "mpfree.fx.delay";
const TAPE: &str = "mpfree.fx.tape_delay";
const MULTITAP: &str = "mpfree.fx.multitap";

fn make(id: &str, values: &[(u16, f32)]) -> Box<dyn Node> {
    let mut n = registry::make(id).unwrap();
    n.prepare(RATE, BLOCK);
    for &(k, v) in values {
        n.set_param(k, v);
    }
    n.reset();
    n
}

/// The engine set up to show its loop: one echo every `ms` (for the
/// multi-tap, tap 4 alone, a sixteenth at `bpm`), Feedback `fb`, the tone
/// open, no saturation, no wow or flutter, wet only.
fn open_loop(id: &str, ms: f32, fb: f32) -> Box<dyn Node> {
    match id {
        DIGITAL => make(
            id,
            &[
                (dl::SYNC, 0.0),
                (dl::TIME, ms),
                (dl::FEEDBACK, fb),
                (dl::TONE, 20_000.0),
                (dl::SATURATION, 0.0),
                (dl::MIX, 100.0),
            ],
        ),
        TAPE => make(
            id,
            &[
                (tape::SYNC, 0.0),
                (tape::TIME, ms),
                (tape::FEEDBACK, fb),
                (tape::TONE, 16_000.0),
                (tape::LOW_CUT, 20.0),
                (tape::SATURATION, 0.0),
                (tape::WOW, 0.0),
                (tape::FLUTTER, 0.0),
                (tape::SPREAD, 0.0),
                (tape::MIX, 100.0),
            ],
        ),
        _ => {
            let mut v = vec![
                (mt::FEEDBACK, fb),
                (mt::TONE, 20_000.0),
                (mt::LOW_CUT, 20.0),
                (mt::MIX, 100.0),
                (mt::time(3), 1.0),
                (mt::level(3), 100.0),
                (mt::pan(3), 0.0),
            ];
            for t in 0..3 {
                v.push((mt::level(t), 0.0));
            }
            make(id, &v)
        }
    }
}

/// The tempo that makes a sixteenth `ms` long.
fn bpm_for(ms: f32) -> f64 {
    15_000.0 / ms as f64
}

/// The impulse response, the two sides summed, `frames` long, after a
/// second at `bpm` for a synced time to settle.
fn impulse(n: &mut dyn Node, frames: usize, bpm: f64) -> Vec<f32> {
    let settle = RATE as usize;
    let mut l = vec![0.0f32; settle + frames];
    l[settle] = 0.5;
    let mut r = l.clone();
    let mut at = 0u64;
    for (a, b) in l.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)) {
        let ctx = Ctx {
            bpm,
            ..Ctx::new(RATE, at)
        };
        n.process(&ctx, &[], a, b);
        at += a.len() as u64;
    }
    l.iter().zip(&r).skip(settle).map(|(a, b)| a + b).collect()
}

/// The level of `x` at `hz`, dB, over a Hann window of `len` frames
/// centred on `at`.
fn level_db(x: &[f32], at: f64, len: usize, hz: f64) -> f64 {
    let start = (at - len as f64 / 2.0).round() as usize;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for k in 0..len {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * k as f64 / len as f64).cos();
        let v = f64::from(x[start + k]) * w;
        let a = std::f64::consts::TAU * hz * (start + k) as f64 / RATE as f64;
        re += v * a.cos();
        im -= v * a.sin();
    }
    10.0 * (re * re + im * im).max(1e-30).log10()
}

/// How much more each repeat loses at `hz` than at 1 kHz, dB a repeat,
/// over the first `count` repeats `period` frames apart.
fn loss_per_repeat(x: &[f32], period: f64, count: usize, hz: f64) -> f64 {
    let tilt = |n: usize| {
        let at = n as f64 * period;
        level_db(x, at, 512, hz) - level_db(x, at, 512, 1000.0)
    };
    (tilt(1) - tilt(count)) / (count - 1) as f64
}

#[test]
fn the_first_repeat_lands_on_its_time() {
    // 100 ms is 4800 frames.
    for id in [DIGITAL, TAPE, MULTITAP] {
        let mut n = open_loop(id, 100.0, 0.0);
        let x = impulse(n.as_mut(), 6000, bpm_for(100.0));
        let at = (0..x.len())
            .max_by(|&a, &b| x[a].abs().total_cmp(&x[b].abs()))
            .unwrap();
        assert!(at.abs_diff(4800) <= 1, "{id}: at {at}");
    }
}

#[test]
fn the_loop_keeps_its_band_at_any_fraction_of_a_frame() {
    // The loss a repeat adds at 10 and 15 kHz, at a whole number of
    // frames and half way between two: the tone takes the same at both,
    // so any difference is the read's (the cubic took 0.7 and 3.5 dB).
    let half = 100.0 + 0.5 * 1000.0 / RATE as f32;
    for id in [DIGITAL, TAPE, MULTITAP] {
        let loss = |ms: f32, hz: f64| {
            let mut n = open_loop(id, ms, 80.0);
            let period = ms as f64 * RATE as f64 / 1000.0;
            let x = impulse(n.as_mut(), (period * 5.5) as usize, bpm_for(ms));
            loss_per_repeat(&x, period, 5, hz)
        };
        for (hz, most) in [(10_000.0, 0.1), (15_000.0, 0.3)] {
            let hidden = loss(half, hz) - loss(100.0, hz);
            assert!(
                hidden.abs() <= most,
                "{id} at {hz} Hz: {hidden:.3} dB a repeat"
            );
        }
    }
}

/// One pass of the tape's tone as the page draws it, dB: Tone and Low cut
/// (12 dB an octave, Q 0.6) and the heads' bump (+2 dB at 110 Hz).
fn drawn_pass_db(tone: f32, low_cut: f32, hz: f32) -> f64 {
    [
        (SvfMode::LowPass, tone, 0.6, 0.0),
        (SvfMode::HighPass, low_cut, 0.6, 0.0),
        (SvfMode::Bell, 110.0, 0.8, 2.0),
    ]
    .iter()
    .map(|&(mode, corner, q, gain)| {
        let mut f = Svf::default();
        f.set(mode, corner, q, gain, RATE);
        let (re, im) = f.response(hz, RATE);
        10.0 * (re * re + im * im).log10()
    })
    .sum()
}

#[test]
fn each_pass_round_the_tape_is_the_tone_the_page_draws() {
    let (tone, low_cut, fb) = (4000.0, 150.0, 50.0);
    let mut n = make(
        TAPE,
        &[
            (tape::TIME, 200.0),
            (tape::FEEDBACK, fb),
            (tape::TONE, tone),
            (tape::LOW_CUT, low_cut),
            (tape::SATURATION, 0.0),
            (tape::WOW, 0.0),
            (tape::FLUTTER, 0.0),
            (tape::MIX, 100.0),
        ],
    );
    let period = 0.2 * RATE as f64;
    let x = impulse(n.as_mut(), (period * 3.5) as usize, 120.0);
    for hz in [150.0f32, 400.0, 1000.0, 2500.0, 6000.0] {
        let want = drawn_pass_db(tone, low_cut, hz) + 20.0 * (fb as f64 / 100.0).log10();
        let got =
            level_db(&x, 2.0 * period, 4096, hz as f64) - level_db(&x, period, 4096, hz as f64);
        assert!(
            (got - want).abs() < 0.5,
            "{hz} Hz: a pass is {got:.2} dB, the page draws {want:.2}"
        );
    }
}

#[test]
fn the_tapes_first_echo_has_been_round_it_once() {
    // At the defaults (Tone 5 kHz, Low cut 120 Hz): the first echo's tilt
    // against 1 kHz is one pass's, the second's two.
    let mut n = make(
        TAPE,
        &[
            (tape::TIME, 200.0),
            (tape::SATURATION, 0.0),
            (tape::WOW, 0.0),
            (tape::FLUTTER, 0.0),
            (tape::MIX, 100.0),
        ],
    );
    let period = 0.2 * RATE as f64;
    let x = impulse(n.as_mut(), (period * 2.5) as usize, 120.0);
    for hz in [200.0f32, 6000.0] {
        let pass = drawn_pass_db(5000.0, 120.0, hz) - drawn_pass_db(5000.0, 120.0, 1000.0);
        for k in 1..=2 {
            let at = k as f64 * period;
            let tilt = level_db(&x, at, 4096, hz as f64) - level_db(&x, at, 4096, 1000.0);
            let want = k as f64 * pass;
            assert!(
                (tilt - want).abs() < 1.0,
                "echo {k} at {hz} Hz: {tilt:.2} dB against 1 kHz, {want:.2} for {k} pass(es)"
            );
        }
    }
}

#[test]
fn the_tapes_feedback_is_its_loop_gain() {
    // Saturation never adds gain: at 80 % with the saturation full up,
    // quiet repeats fall at least as fast as Feedback and the tone say.
    let mut n = make(
        TAPE,
        &[
            (tape::TIME, 100.0),
            (tape::FEEDBACK, 80.0),
            (tape::SATURATION, 100.0),
            (tape::WOW, 0.0),
            (tape::FLUTTER, 0.0),
            (tape::MIX, 100.0),
        ],
    );
    let period = RATE as usize / 10;
    let x = impulse(n.as_mut(), period * 13, 120.0);
    let energy = |k: usize| -> f64 {
        x[k * period - 1024..k * period + 1024]
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum()
    };
    for k in 2..12 {
        let step = 10.0 * (energy(k + 1) / energy(k)).log10();
        assert!(
            step < 20.0 * 0.8f64.log10() + 0.1,
            "repeat {k}: {step:.2} dB"
        );
    }
}

#[test]
fn a_digital_tail_follows_the_tempo_and_rings_on_at_full_feedback() {
    let mut d = make(DIGITAL, &[(dl::SYNC, 9.0), (dl::FEEDBACK, 50.0)]);
    // 1/4 at 120 BPM is half a second a pass.
    let at_120 = d.tail_frames();
    let mut l = vec![0.0f32; BLOCK];
    let mut r = l.clone();
    let ctx = Ctx {
        bpm: 60.0,
        ..Ctx::new(RATE, 0)
    };
    for _ in 0..200 {
        d.process(&ctx, &[], &mut l, &mut r);
    }
    // At 60 BPM a pass takes twice as long, and so does the tail.
    let at_60 = d.tail_frames();
    assert!(
        (at_60 as f64 / at_120 as f64 - 2.0).abs() < 0.05,
        "{at_120} {at_60}"
    );
    d.set_param(dl::FEEDBACK, 100.0);
    assert_eq!(d.tail_frames(), u32::MAX);
}
