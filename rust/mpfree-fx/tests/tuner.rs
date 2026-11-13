//! The Tuner reads the note and the cents of a sound, to well under a cent,
//! from a five-string bass's low B to the top of a voice; follows the
//! reference pitch; holds a fading string's reading and lets go of it;
//! reads nothing in noise or silence; and passes the sound through
//! untouched unless muted.

use std::f32::consts::TAU;

use mpfree_fx::ids::tuner;
use mpfree_fx::mpfree_node::{Ctx, Node};
use mpfree_fx::Tuner;

const RATE: u32 = 48_000;

fn set_up(values: &[(u16, f32)]) -> Tuner {
    let mut n = Tuner::new();
    n.prepare(RATE, 512);
    for &(id, v) in values {
        n.set_param(id, v);
    }
    n.reset();
    n
}

fn run(n: &mut Tuner, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (x.to_vec(), x.to_vec());
    let mut at = 0;
    while at < x.len() {
        let len = 256.min(x.len() - at);
        n.process(
            &Ctx::new(RATE, at as u64),
            &[],
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn midi_hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

/// A string-like tone: a few harmonics, falling off, at `hz`, for
/// `seconds`, at -12 dBFS or so.
fn tone(hz: f32, seconds: f32) -> Vec<f32> {
    let n = (seconds * RATE as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / RATE as f64;
            (1..=6)
                .map(|k| {
                    let a = (hz as f64 * k as f64 * t).fract() as f32;
                    0.25 / k as f32 * (TAU * a).sin()
                })
                .sum::<f32>()
        })
        .collect()
}

fn reading(n: &Tuner) -> (f32, f32, f32) {
    (
        n.param(tuner::NOTE),
        n.param(tuner::CENTS),
        n.param(tuner::HZ),
    )
}

#[test]
fn it_names_the_note_and_the_cents_across_the_range() {
    // (MIDI note, cents off): a five-string bass's low B, bass E, guitar
    // E and G, a voice, a high A, the top of a voice.
    let cases = [
        (23, 0.0),
        (28, 12.0),
        (40, -7.0),
        (55, 3.0),
        (64, -21.0),
        (81, 9.0),
        (88, -4.0),
        (90, 0.0),
    ];
    for (note, off) in cases {
        let hz = midi_hz(note as f32 + off / 100.0);
        let mut t = set_up(&[]);
        run(&mut t, &tone(hz, 0.5));
        let (got_note, cents, got_hz) = reading(&t);
        assert_eq!(got_note, note as f32, "{hz} Hz");
        assert!(
            (cents - off).abs() < 0.5,
            "{hz} Hz: {cents} cents, want {off}"
        );
        assert!((got_hz / hz - 1.0).abs() < 3e-4, "{hz} Hz read {got_hz}");
    }
}

#[test]
fn a_pure_sine_reads_to_a_tenth_of_a_cent() {
    for hz in [110.0, 441.0, 1000.0] {
        let x: Vec<f32> = (0..RATE as usize / 2)
            .map(|i| 0.3 * (TAU * (hz * i as f64 / RATE as f64).fract() as f32).sin())
            .collect();
        let mut t = set_up(&[]);
        run(&mut t, &x);
        let midi = 69.0 + 12.0 * (hz as f32 / 440.0).log2();
        let want = 100.0 * (midi - midi.round());
        let got = t.param(tuner::CENTS);
        assert!(
            (got - want).abs() < 0.1,
            "{hz} Hz: {got} cents, want {want}"
        );
    }
}

#[test]
fn the_reference_moves_the_cents() {
    // A4 at 440 Hz against A = 432 Hz is 31.8 cents sharp.
    let mut t = set_up(&[(tuner::REFERENCE, 432.0)]);
    run(&mut t, &tone(440.0, 0.5));
    let (note, cents, _) = reading(&t);
    assert_eq!(note, 69.0);
    let want = 1200.0 * (440.0f32 / 432.0).log2();
    assert!((cents - want).abs() < 0.5, "{cents}, want {want}");
    // And changing it moves the meter at once.
    t.set_param(tuner::REFERENCE, 440.0);
    assert!(t.param(tuner::CENTS).abs() < 0.5);
}

#[test]
fn a_fading_string_holds_its_reading_then_lets_go() {
    // E2 decaying by 20 dB a second for 1.5 s, then silence.
    let hz = midi_hz(40.0);
    let mut x = tone(hz, 1.5);
    for (i, v) in x.iter_mut().enumerate() {
        *v *= 10f32.powf(-(i as f32) / RATE as f32);
    }
    let mut t = set_up(&[]);
    run(&mut t, &x);
    let (note, cents, _) = reading(&t);
    assert_eq!(note, 40.0);
    assert!(cents.abs() < 1.0, "{cents}");
    // 200 ms of silence: still held. A second: gone.
    run(&mut t, &vec![0.0; RATE as usize / 5]);
    assert_eq!(t.param(tuner::NOTE), 40.0, "held");
    run(&mut t, &vec![0.0; RATE as usize]);
    assert_eq!(reading(&t), (0.0, 0.0, 0.0), "let go");
}

#[test]
fn noise_and_silence_read_nothing() {
    let mut seed = 3u32;
    let noise: Vec<f32> = (0..RATE as usize / 2)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            0.3 * ((seed >> 8) as f32 / (1 << 23) as f32 - 1.0)
        })
        .collect();
    let mut t = set_up(&[]);
    run(&mut t, &noise);
    assert_eq!(t.param(tuner::NOTE), 0.0);
    let mut t = set_up(&[]);
    run(&mut t, &vec![0.0; RATE as usize / 2]);
    assert_eq!(reading(&t), (0.0, 0.0, 0.0));
}

#[test]
fn the_sound_passes_through_untouched_unless_muted() {
    let x = tone(196.0, 0.25);
    let mut t = set_up(&[]);
    let (l, r) = run(&mut t, &x);
    assert_eq!(l, x);
    assert_eq!(r, x);
    assert_eq!(t.latency(), 0);
    let mut t = set_up(&[(tuner::MUTE, 1.0)]);
    let (l, _) = run(&mut t, &x);
    assert!(l.iter().all(|&v| v == 0.0), "muted from the start");
    assert_eq!(t.param(tuner::NOTE), 55.0, "and still reading");
}
