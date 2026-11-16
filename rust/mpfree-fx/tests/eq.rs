//! The EQ: flat by default to the bit, every curve as drawn (the response
//! it reports is what a sine measures through it), Butterworth cuts, and
//! type and slope changes that never click.

use std::f32::consts::PI;

use mpfree_fx::ids::eq::*;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::{gain_to_db, Eq};

fn eq(rate: u32, values: &[(u16, f32)]) -> Eq {
    let mut e = Eq::new();
    e.prepare(rate, MAX_BLOCK);
    for &(id, v) in values {
        e.set_param(id, v);
    }
    e.reset();
    e
}

fn run(e: &mut Eq, rate: u32, l: &mut [f32], r: &mut [f32], events: &[(usize, u16, f32)]) {
    let mut at = 0;
    for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
        let ev: Vec<Timed> = events
            .iter()
            .filter(|m| m.0 >= at && m.0 < at + a.len())
            .map(|m| {
                Timed::new(
                    (m.0 - at) as u32,
                    NodeEvent::Param {
                        id: m.1,
                        value: m.2,
                    },
                )
            })
            .collect();
        e.process(&Ctx::new(rate, at as u64), &ev, a, b);
        at += a.len();
    }
}

fn sine(hz: f32, amp: f32, rate: u32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / rate as f32).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len() as f64).sqrt() as f32
}

/// The gain a steady sine measures through the EQ, in dB.
fn measure(e: &mut Eq, rate: u32, hz: f32) -> f32 {
    e.reset();
    let n = rate as usize;
    let x = sine(hz, 0.1, rate, n);
    let (mut l, mut r) = (x.clone(), x.clone());
    run(e, rate, &mut l, &mut r, &[]);
    gain_to_db(rms(&l[n / 2..]) / rms(&x[n / 2..]))
}

/// The largest second difference: a click shows up as a spike here, far
/// above what a low sine can make.
fn roughness(x: &[f32]) -> f32 {
    x.windows(3)
        .fold(0.0f32, |m, w| m.max((w[2] - 2.0 * w[1] + w[0]).abs()))
}

#[test]
fn defaults_are_bit_for_bit_flat() {
    let mut e = eq(48_000, &[]);
    let x = sine(997.0, 0.7, 48_000, 20_000);
    let (mut l, mut r) = (x.clone(), x.iter().map(|v| -v * 0.5).collect::<Vec<_>>());
    let r0 = r.clone();
    run(&mut e, 48_000, &mut l, &mut r, &[]);
    assert!(l == x && r == r0);
    assert_eq!(e.tail_frames(), 0);
}

#[test]
fn the_reported_curve_is_what_a_sine_measures() {
    let settings: [&[(u16, f32)]; 4] = [
        &[
            (gain(0), 6.0),
            (gain(1), -9.0),
            (q(1), 3.0),
            (gain(2), 12.0),
            (q(2), 0.4),
            (gain(3), -6.0),
            (OUTPUT, -3.0),
        ],
        &[
            (LOW_SLOPE, 2.0),
            (LOW_CUT, 300.0),
            (HIGH_SLOPE, 1.0),
            (HIGH_CUT, 6000.0),
            (band_type(1), 1.0),
            (gain(1), 4.0),
        ],
        &[
            (band_type(0), 2.0),
            (freq(0), 3000.0),
            (gain(0), -12.0),
            (q(0), 1.5),
            (band_type(3), 0.0),
            (freq(3), 150.0),
            (gain(3), 18.0),
            (q(3), 18.0),
        ],
        &[
            (band_type(0), 1.0),
            (q(0), 9.0),
            (gain(0), 15.0),
            (freq(0), 800.0),
        ],
    ];
    let probes = [
        40.0f32, 100.0, 150.0, 300.0, 800.0, 1000.0, 2500.0, 3000.0, 6000.0, 9000.0, 15_000.0,
    ];
    for rate in [44_100, 48_000, 96_000] {
        for set in settings {
            let mut e = eq(rate, set);
            let mut want = vec![0.0; probes.len()];
            e.response_db(&probes, &mut want);
            for (&hz, &w) in probes.iter().zip(&want) {
                let got = measure(&mut e, rate, hz);
                // Deep in a cut the measurement meets the float floor.
                if w < -60.0 {
                    assert!(got < -55.0, "{rate} Hz {set:?} at {hz}: {got} vs {w}");
                    continue;
                }
                assert!(
                    (got - w).abs() < 0.05,
                    "{rate} Hz {set:?} at {hz} Hz: measured {got:.3} dB, curve {w:.3} dB"
                );
            }
        }
    }
}

#[test]
fn cuts_are_butterworth() {
    let mut out = [0.0f32; 3];
    let hz = [1000.0, 125.0, 8000.0];
    let lo12 = eq(48_000, &[(LOW_SLOPE, 1.0), (LOW_CUT, 1000.0)]);
    lo12.response_db(&hz, &mut out);
    assert!(
        (out[0] + 3.01).abs() < 0.02,
        "-3 dB at the corner: {}",
        out[0]
    );
    assert!(
        out[1] < -35.0 && out[1] > -37.0,
        "12 dB/oct, 3 octaves: {}",
        out[1]
    );
    assert!(out[2].abs() < 0.01);
    let mut lo24 = eq(48_000, &[(LOW_SLOPE, 2.0), (LOW_CUT, 1000.0)]);
    lo24.response_db(&hz, &mut out);
    assert!(
        (out[0] + 3.01).abs() < 0.02,
        "-3 dB at the corner: {}",
        out[0]
    );
    assert!(
        out[1] < -71.0 && out[1] > -73.0,
        "24 dB/oct, 3 octaves: {}",
        out[1]
    );
    assert!((measure(&mut lo24, 48_000, 1000.0) + 3.01).abs() < 0.05);
    let mut hi24 = eq(48_000, &[(HIGH_SLOPE, 2.0), (HIGH_CUT, 2000.0)]);
    let g = measure(&mut hi24, 48_000, 8000.0);
    // A little over 48 dB: the bilinear transform steepens a low-pass as
    // it nears Nyquist.
    assert!(g < -48.0 && g > -54.0, "24 dB/oct, 2 octaves: {g}");
}

#[test]
fn type_and_slope_changes_do_not_click() {
    let rate = 48_000;
    let n = rate as usize;
    // A low sine through a boosted band: its own second difference is tiny,
    // so any step stands out.
    let x = sine(110.0, 0.5, rate, n);
    let calm = roughness(&x) * 8.0 + 1e-4;
    let mut e = eq(rate, &[(freq(1), 110.0), (gain(1), 12.0), (q(1), 1.0)]);
    let (mut l, mut r) = (x.clone(), x.clone());
    let moves = [
        (10_000, band_type(1), 1.0),
        (20_000, band_type(1), 2.0),
        (30_000, band_type(1), 0.0),
        (35_000, LOW_SLOPE, 2.0),
        (35_000, LOW_CUT, 90.0),
        (36_000, LOW_SLOPE, 1.0),
        (38_000, LOW_SLOPE, 0.0),
        (38_100, HIGH_SLOPE, 2.0),
        (38_100, HIGH_CUT, 1000.0),
    ];
    run(&mut e, rate, &mut l, &mut r, &moves);
    let rough = roughness(&l[100..]);
    assert!(rough < calm, "roughness {rough} vs a calm limit of {calm}");
    // Each change landed: the band ends a bell at +12 dB again, the low
    // cut off and the high cut far above.
    assert!((gain_to_db(rms(&l[n - 4800..]) / rms(&x[n - 4800..])) - 12.0).abs() < 0.3);
}

#[test]
fn a_band_at_zero_db_is_exactly_flat_whatever_its_type_and_q() {
    for kind in [0.0, 1.0, 2.0] {
        let mut e = eq(
            48_000,
            &[(band_type(2), kind), (q(2), 18.0), (freq(2), 1000.0)],
        );
        let x = sine(1000.0, 0.5, 48_000, 4800);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut e, 48_000, &mut l, &mut r, &[]);
        assert!(l == x, "type {kind}");
    }
}

/// Projects saved with the four-band EQ keep their ids and their sound:
/// bands 1 to 4 sit where they were and start On, bands 5 to 8 and the
/// Use params come after Output and start Unused, and Unused is flat.
#[test]
fn the_four_band_ids_and_sound_are_kept() {
    assert_eq!(BANDS, 8);
    assert_eq!(
        (band_type(0), freq(0), gain(3), q(3), OUTPUT),
        (4, 5, 18, 19, 20)
    );
    assert_eq!(
        (band_type(4), q(7), band_use(0), band_use(7)),
        (21, 36, 37, 44)
    );
    let e = Eq::new();
    let p = e.params();
    assert_eq!(p.len(), 46);
    for (i, info) in p.iter().enumerate() {
        assert_eq!(info.id as usize, i);
        if let Some((b, what)) = band_of(info.id) {
            assert_eq!(info.group, format!("Band {}", b + 1));
            let name = ["Type", "Freq", "Gain", "Q", "Use"][what as usize];
            assert_eq!(info.name, name, "id {i}");
        }
    }
    for b in 0..BANDS {
        let want = if b < 4 { USE_ON } else { USE_UNUSED };
        assert_eq!(e.param(band_use(b)), want, "band {}", b + 1);
    }
    // A band 5 to 8 set up but Unused is not heard, and the default EQ is
    // still exactly the input.
    let mut quiet = eq(48_000, &[(gain(5), 12.0), (freq(5), 1000.0)]);
    assert!(measure(&mut quiet, 48_000, 1000.0).abs() < 0.001);
    let x = sine(997.0, 0.3, 48_000, 4096);
    let (mut l, mut r) = (x.clone(), x.clone());
    let mut flat = eq(48_000, &[]);
    run(&mut flat, 48_000, &mut l, &mut r, &[]);
    assert_eq!(l, x);
}

/// Bands 5 to 8 are full bands: what they draw is what a sine measures.
#[test]
fn the_new_bands_measure_as_drawn() {
    let set = [
        (band_use(4), USE_ON),
        (freq(4), 180.0),
        (gain(4), -9.0),
        (q(4), 2.5),
        (band_use(6), USE_ON),
        (band_type(6), 2.0),
        (freq(6), 6000.0),
        (gain(6), 6.0),
        (band_use(7), USE_ON),
        (band_type(7), 3.0),
        (freq(7), 2500.0),
        (q(7), 4.0),
    ];
    let probes = [
        60.0f32, 180.0, 400.0, 1000.0, 2000.0, 3000.0, 6000.0, 12_000.0,
    ];
    let mut e = eq(48_000, &set);
    let mut want = vec![0.0; probes.len()];
    e.response_db(&probes, &mut want);
    assert!(want[1] < -8.5, "the bell cuts at 180 Hz: {}", want[1]);
    for (&hz, &w) in probes.iter().zip(&want) {
        let got = measure(&mut e, 48_000, hz);
        assert!(
            (got - w).abs() < 0.05,
            "{hz} Hz: measured {got:.3}, curve {w:.3}"
        );
    }
}

/// A notch takes out its frequency whatever its Gain says, and leaves an
/// octave away nearly untouched.
#[test]
fn a_notch_cuts_deep_and_narrow() {
    let mut e = eq(
        48_000,
        &[
            (band_type(2), 3.0),
            (freq(2), 1000.0),
            (q(2), 8.0),
            (gain(2), 0.0),
        ],
    );
    let mut out = [0.0f32; 3];
    e.response_db(&[1000.0, 500.0, 2000.0], &mut out);
    assert!(out[0] < -50.0, "deep at 1 kHz: {}", out[0]);
    assert!(out[1].abs() < 0.3 && out[2].abs() < 0.3, "{out:?}");
    assert!(measure(&mut e, 48_000, 1000.0) < -45.0);
}

/// Bypassing a band glides it flat without a click, and On brings it back
/// the same way.
#[test]
fn bypass_glides_and_is_flat() {
    let rate = 48_000;
    let set = [(freq(1), 1000.0), (gain(1), 18.0), (q(1), 4.0)];
    let mut e = eq(rate, &set);
    assert!(measure(&mut e, rate, 1000.0) > 17.0);
    let n = rate as usize / 2;
    let x = sine(1000.0, 0.05, rate, n);
    let jumps = |e: &mut Eq, events: &[(usize, u16, f32)]| {
        e.reset();
        let (mut l, mut r) = (x.clone(), x.clone());
        run(e, rate, &mut l, &mut r, events);
        // Past the start-up ring, the biggest step from one sample to the next.
        l[n / 8..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    };
    let steady = jumps(&mut e, &[]);
    let switched = jumps(
        &mut e,
        &[
            (n / 4, band_use(1), USE_BYPASSED),
            (3 * n / 4, band_use(1), USE_ON),
        ],
    );
    assert!(
        switched <= steady * 1.01,
        "switching steps {switched} a sample, the boosted sine only {steady}"
    );
    let mut off = eq(rate, &[set[0], set[1], set[2], (band_use(1), USE_BYPASSED)]);
    assert!(measure(&mut off, rate, 1000.0).abs() < 0.001);
    let mut want = [0.0f32];
    off.response_db(&[1000.0], &mut want);
    assert!(want[0].abs() < 1e-6);
}

/// Listen plays the band alone: its frequency comes through, frequencies
/// far from it fade away, and Off brings back the full EQ.
#[test]
fn listen_plays_the_band_alone() {
    let rate = 48_000;
    let mut e = eq(
        rate,
        &[
            (freq(1), 1000.0),
            (gain(1), 6.0),
            (q(1), 4.0),
            (LISTEN, 2.0),
        ],
    );
    assert!(
        measure(&mut e, rate, 1000.0).abs() < 0.5,
        "the band's own frequency"
    );
    assert!(measure(&mut e, rate, 100.0) < -20.0, "far below the band");
    assert!(
        measure(&mut e, rate, 10_000.0) < -20.0,
        "far above the band"
    );
    e.set_param(LISTEN, 0.0);
    e.reset();
    assert!(
        measure(&mut e, rate, 100.0).abs() < 0.5,
        "Off: the EQ again"
    );
    assert!((measure(&mut e, rate, 1000.0) - 6.0).abs() < 0.5);
}
