//! Fuzz (FX quality round 2, decision 10.6): silence stays silent with
//! every model, the Gate and Low keep; Model and Low keep changes and
//! thrown knobs don't click; it sustains at full fuzz and, with the Gate
//! up, breaks up as a note dies away (A10); Low keep leaves the sub as it
//! came in; both sides are the same on a mono input; and any block size
//! sounds the same while the knobs glide.

use std::f32::consts::PI;

use mpfree_fx::ids::fuzz::*;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::Fuzz;

const RATE: u32 = 48_000;

fn fuzz(values: &[(u16, f32)]) -> Fuzz {
    let mut f = Fuzz::new();
    f.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        f.set_param(id, v);
    }
    f.reset();
    f
}

fn sine(hz: f32, level: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| level * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

/// `x` through `f` in blocks of `block` frames, with `events` (frame, id,
/// value): the left and right outputs.
fn run_in(
    f: &mut Fuzz,
    x: &[f32],
    events: &[(usize, u16, f32)],
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (x.to_vec(), x.to_vec());
    let mut at = 0;
    while at < l.len() {
        let len = block.min(l.len() - at);
        let ev: Vec<Timed> = events
            .iter()
            .filter(|(fr, _, _)| (at..at + len).contains(fr))
            .map(|&(fr, id, value)| Timed::new((fr - at) as u32, NodeEvent::Param { id, value }))
            .collect();
        f.process(
            &Ctx::new(RATE, at as u64),
            &ev,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn run(f: &mut Fuzz, x: &[f32], events: &[(usize, u16, f32)]) -> Vec<f32> {
    let (l, r) = run_in(f, x, events, 128);
    assert_eq!(l, r, "a mono input comes out the same on both sides");
    l
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// The largest second difference in `x`: a click's sharpest corner.
fn corner(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0f32, f32::max)
}

/// The amplitude of the `hz` component of `x` over whole periods.
fn amplitude_at(x: &[f32], hz: f32) -> f32 {
    let per = RATE as f32 / hz;
    let n = ((x.len() as f32 / per).floor() * per) as usize;
    let w = 2.0 * PI * hz / RATE as f32;
    let (mut c, mut s) = (0.0f64, 0.0f64);
    for (i, v) in x[x.len() - n..].iter().enumerate() {
        c += f64::from(v * (w * i as f32).cos());
        s += f64::from(v * (w * i as f32).sin());
    }
    (2.0 * (c * c + s * s).sqrt() / n as f64) as f32
}

#[test]
fn silence_stays_silent_with_every_model_the_gate_and_low_keep() {
    let x = vec![0.0f32; RATE as usize / 2];
    for model in 0..3 {
        for gate in [0.0, 100.0] {
            for keep in [0.0, 4.0] {
                let mut f = fuzz(&[
                    (MODEL, model as f32),
                    (FUZZ, 100.0),
                    (GATE, gate),
                    (LOW_KEEP, keep),
                ]);
                let y = run(&mut f, &x, &[]);
                let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(peak < 1e-6, "model {model} gate {gate} keep {keep}: {peak}");
            }
        }
    }
}

#[test]
fn changes_and_thrown_knobs_dont_click() {
    // A 220 Hz sine at -12 dBFS; halfway through, one control changes.
    // The sharpest corner around the change stays within 3 dB of the
    // steady sound's, before or after (A8).
    let n = RATE as usize;
    let x = sine(220.0, 0.25, n);
    let half = n / 2;
    for model in 0..3 {
        let to_model = ((model + 1) % 3) as f32;
        for (id, from, to) in [
            (MODEL, model as f32, to_model),
            (LOW_KEEP, 0.0, 3.0),
            (LOW_KEEP, 3.0, 0.0),
            (LOW_KEEP, 1.0, 4.0),
            (FUZZ, 0.0, 100.0),
            (FUZZ, 100.0, 0.0),
            (TONE, 0.0, 100.0),
            (GATE, 0.0, 100.0),
            (LEVEL, 0.0, -24.0),
        ] {
            let mut f = fuzz(&[(MODEL, model as f32), (id, from)]);
            let y = run(&mut f, &x, &[(half, id, to)]);
            let steady = corner(&y[n / 4..half - 10]).max(corner(&y[half + 9600..n]));
            let at = corner(&y[half - 10..half + 9600]);
            assert!(
                db(at) <= db(steady) + 3.0,
                "model {model}, control {id} {from} to {to}: {:.1} dB over",
                db(at) - db(steady)
            );
        }
    }
}

#[test]
fn it_sustains_and_with_the_gate_up_breaks_up_as_a_note_dies() {
    // A 220 Hz note from -12 dBFS falling 40 dB a second.
    let n = 2 * RATE as usize;
    let x: Vec<f32> = sine(220.0, 0.25, n)
        .iter()
        .enumerate()
        .map(|(i, v)| v * 10f32.powf(-2.0 * i as f32 / RATE as f32))
        .collect();
    // 10 ms windows: at 0.5 s (-32 dBFS in) and at 1 s (-52 dBFS in).
    let w = RATE as usize / 100;
    let at = |y: &[f32], s: f32| rms(&y[(s * RATE as f32) as usize..][..w]);
    for model in 0..3 {
        let mut open = fuzz(&[(MODEL, model as f32), (FUZZ, 100.0)]);
        let y = run(&mut open, &x, &[]);
        let start = at(&y, 0.1);
        // Without the Gate it holds on: 20 dB down the input, at most 6
        // down the output.
        assert!(
            db(at(&y, 0.6)) > db(start) - 6.0,
            "model {model} sustains: {:.1} dB",
            db(at(&y, 0.6)) - db(start)
        );
        let mut gated = fuzz(&[(MODEL, model as f32), (FUZZ, 100.0), (GATE, 70.0)]);
        let y = run(&mut gated, &x, &[]);
        // With it, the note is gone well before -40 dBFS in.
        assert!(
            db(at(&y, 0.7)) < db(start) - 40.0,
            "model {model} breaks up: {:.1} dB",
            db(at(&y, 0.7)) - db(start)
        );
    }
}

#[test]
fn low_keep_leaves_the_sub_as_it_came_in() {
    // A 50 Hz sine at -12 dBFS under a 150 Hz crossover, at full fuzz: the
    // sub comes out as it went in, and the fuzz it drives is above it.
    let x = sine(50.0, 0.25, RATE as usize);
    for model in 0..3 {
        let mut f = fuzz(&[(MODEL, model as f32), (FUZZ, 100.0), (LOW_KEEP, 3.0)]);
        let y = run(&mut f, &x, &[]);
        let half = RATE as usize / 2;
        let g = db(amplitude_at(&y[half..], 50.0) / 0.25);
        assert!(g.abs() < 1.0, "model {model}: {g:.2} dB at 50 Hz");
        // And the fuzz above it is still there (the sine's harmonics).
        let h = db(amplitude_at(&y[half..], 250.0) / 0.25);
        assert!(h > -40.0, "model {model}: harmonics at {h:.1} dB");
    }
}

#[test]
fn any_block_size_sounds_the_same_while_knobs_glide() {
    let n = RATE as usize / 2;
    let x = sine(330.0, 0.3, n);
    let events = [
        (1000, FUZZ, 100.0),
        (5000, TONE, 10.0),
        (9000, GATE, 60.0),
        (13000, MODEL, 1.0),
        (17000, LOW_KEEP, 2.0),
        (21000, MODEL, 2.0),
        (23000, LEVEL, -6.0),
    ];
    let mut a = fuzz(&[]);
    let mut b = fuzz(&[]);
    let (ya, _) = run_in(&mut a, &x, &events, 128);
    let (yb, _) = run_in(&mut b, &x, &events, 37);
    assert_eq!(ya, yb);
}
