//! The curves the effect pages draw are what the effects do: each drawing
//! helper against a sine (or a steady level) measured through the node.

use mpfree_fx::ids::{
    chorus, cloud, compressor as comp, delay, dj_filter as dj, drive, filter, flanger, lofi,
    master, phaser,
};
use mpfree_fx::mpfree_node::{Ctx, Node, Timed};
use mpfree_fx::{
    Chorus, Cloud, Compressor, Delay, DjFilter, Drive, Flanger, LoFi, MasterSafety, MultiFilter,
    Phaser,
};

const RATE: u32 = 48_000;

fn set<N: Node>(mut n: N, values: &[(u16, f32)]) -> N {
    for &(id, v) in values {
        n.set_param(id, v);
    }
    n.prepare(RATE, 1024);
    n.reset();
    n
}

/// The gain in dB a steady sine at `hz` gets through `n`, from the RMS of
/// the last half second of two seconds (left channel).
fn measure(n: &mut dyn Node, hz: f32) -> f32 {
    n.reset();
    let frames = 2 * RATE as usize;
    let x: Vec<f32> = (0..frames)
        .map(|i| 0.25 * (2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin())
        .collect();
    let (mut l, mut r) = (x.clone(), x.clone());
    let ctx = Ctx::new(RATE, 0);
    for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
        n.process(&ctx, &[] as &[Timed], a, b);
    }
    let tail = frames / 4 * 3;
    let rms = |v: &[f32]| (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt();
    20.0 * (rms(&l[tail..]) / rms(&x[tail..])).log10()
}

fn close(what: &str, drawn: f32, measured: f32, tol: f32) {
    assert!(
        (drawn - measured).abs() <= tol,
        "{what}: drawn {drawn:.2} dB, measured {measured:.2} dB"
    );
}

#[test]
fn the_multi_filter_draws_what_it_does() {
    for (ty, slope, res, gain) in [
        (0.0, 0.0, 0.0, 0.0),
        (0.0, 1.0, 60.0, 0.0),
        (1.0, 1.0, 30.0, 0.0),
        (2.0, 0.0, 50.0, 0.0),
        (4.0, 0.0, 20.0, 9.0),
        (6.0, 0.0, 0.0, -6.0),
    ] {
        let mut f = set(
            MultiFilter::new(),
            &[
                (filter::TYPE, ty),
                (filter::SLOPE, slope),
                (filter::CUTOFF, 1000.0),
                (filter::RESONANCE, res),
                (filter::GAIN, gain),
            ],
        );
        for hz in [100.0, 700.0, 1000.0, 1500.0, 6000.0] {
            let drawn = f.response_db(hz);
            if drawn < -50.0 {
                continue;
            }
            let m = measure(&mut f, hz);
            close(
                &format!("type {ty} slope {slope} at {hz} Hz"),
                drawn,
                m,
                0.2,
            );
        }
    }
}

#[test]
fn the_dj_filter_draws_what_it_does() {
    for pos in [-70.0, -5.0, 0.0, 40.0] {
        let mut f = set(
            DjFilter::new(),
            &[(dj::POSITION, pos), (dj::RESONANCE, 50.0)],
        );
        for hz in [60.0, 300.0, 2000.0, 9000.0] {
            let drawn = f.response_db(hz);
            if drawn < -50.0 {
                continue;
            }
            let m = measure(&mut f, hz);
            close(&format!("position {pos} at {hz} Hz"), drawn, m, 0.2);
        }
    }
    let f = set(DjFilter::new(), &[(dj::POSITION, 0.0)]);
    assert_eq!(f.corner(), None);
    let f = set(DjFilter::new(), &[(dj::POSITION, -100.0)]);
    let (low, hz) = f.corner().unwrap();
    assert!(low && (hz - 80.0).abs() < 0.5, "{hz}");
}

#[test]
fn the_flanger_and_phaser_draw_their_comb_and_notches() {
    // Depth 0 holds the sweep still, so a sine measures the drawn curve.
    let mut fl = set(
        Flanger::new(),
        &[
            (flanger::DEPTH, 0.0),
            (flanger::DELAY, 2.0),
            (flanger::FEEDBACK, 60.0),
            (flanger::MIX, 50.0),
        ],
    );
    for hz in [125.0, 250.0, 375.0, 1000.0, 3300.0] {
        let drawn = fl.response_db(hz, 0.3);
        let m = measure(&mut fl, hz);
        close(&format!("flanger at {hz} Hz"), drawn, m, 0.3);
    }
    assert!((fl.delay_ms(0.25) - 2.0).abs() < 1e-4);
    let mut ph = set(
        Phaser::new(),
        &[
            (phaser::DEPTH, 0.0),
            (phaser::CENTER, 800.0),
            (phaser::STAGES, 1.0),
            (phaser::FEEDBACK, 40.0),
            (phaser::MIX, 50.0),
        ],
    );
    for hz in [100.0, 330.0, 800.0, 2000.0, 7000.0] {
        let drawn = ph.response_db(hz, 0.0);
        if drawn < -40.0 {
            continue;
        }
        let m = measure(&mut ph, hz);
        close(&format!("phaser at {hz} Hz"), drawn, m, 0.3);
    }
    // A sweep at full depth spans two and a half octaves each way.
    let ph = set(
        Phaser::new(),
        &[(phaser::DEPTH, 100.0), (phaser::CENTER, 800.0)],
    );
    let top = ph.center_hz(0.25);
    assert!((top / 800.0 - 2f32.powf(2.5)).abs() < 0.02, "{top}");
}

#[test]
fn the_chorus_voices_sweep_round_the_delay() {
    let c = set(
        Chorus::new(),
        &[
            (chorus::DELAY, 10.0),
            (chorus::DEPTH, 50.0),
            (chorus::VOICES, 2.0),
        ],
    );
    assert!((c.delay_ms(0, 0.25).unwrap() - 12.5).abs() < 0.01);
    assert!(
        (c.delay_ms(1, 0.25).unwrap() - 7.5).abs() < 0.01,
        "half a cycle on"
    );
    assert_eq!(c.delay_ms(2, 0.0), None);
}

#[test]
fn the_lofi_cuts_draw_what_they_do() {
    let mut f = set(
        LoFi::new(),
        &[
            (lofi::AMOUNT, 100.0),
            (lofi::WOW, 0.0),
            (lofi::FLUTTER, 0.0),
            (lofi::SATURATION, 0.0),
            (lofi::CRACKLE, 0.0),
            (lofi::NOISE, 0.0),
            (lofi::LOW_CUT, 200.0),
            (lofi::HIGH_CUT, 4000.0),
        ],
    );
    for hz in [100.0, 1000.0, 6000.0] {
        let drawn = f.cuts_db(hz);
        let m = measure(&mut f, hz);
        // The tape's own path (wow at 0 still reads through a delay line)
        // costs a little level; the shape is what is drawn.
        close(&format!("lo-fi at {hz} Hz"), drawn, m, 0.6);
    }
    let open = set(LoFi::new(), &[(lofi::AMOUNT, 0.0)]);
    assert!(open.cuts_db(1000.0).abs() < 0.05);
}

#[test]
fn the_static_curves_are_the_effects_own() {
    let c = set(
        Compressor::new(),
        &[
            (comp::THRESHOLD, -20.0),
            (comp::RATIO, 4.0),
            (comp::KNEE, 0.0),
            (comp::MAKEUP, 3.0),
        ],
    );
    assert!(
        (c.curve_db(-30.0) - -27.0).abs() < 1e-4,
        "below: makeup only"
    );
    assert!(
        (c.curve_db(-8.0) - (-20.0 + 3.0 + 3.0)).abs() < 1e-4,
        "4:1 above"
    );

    let m = set(
        MasterSafety::new(),
        &[(master::MODE, 0.0), (master::CEILING, -1.0)],
    );
    let ceil = 10f32.powf(-0.05);
    assert!((m.curve(2.0) - ceil).abs() < 1e-5);
    assert_eq!(m.curve(0.3), 0.3);
    let soft = set(
        MasterSafety::new(),
        &[(master::MODE, 1.0), (master::CEILING, 0.0)],
    );
    assert!(soft.curve(0.9) < 0.9 && soft.curve(0.9) > 0.5 && soft.curve(10.0) <= 1.0);
    let raw = set(MasterSafety::new(), &[(master::MODE, 2.0)]);
    assert_eq!(raw.curve(1.5), 1.0, "the converter clips at full scale");

    let d = set(Drive::new(), &[(drive::MODE, 1.0), (drive::DRIVE, 6.0)]);
    assert!((d.curve(0.25) - 0.25 * 10f32.powf(0.3)).abs() < 1e-4);
    assert_eq!(d.curve(0.9), 1.0, "hard clip");

    let e = set(Delay::new(), &[(delay::SYNC, 7.0)]);
    assert!((e.time_ms(120.0) - 375.0).abs() < 0.05, "1/8D at 120 BPM");
    let e = set(Delay::new(), &[(delay::SYNC, 0.0), (delay::TIME, 220.0)]);
    assert!((e.time_ms(120.0) - 220.0).abs() < 0.05);
}

#[test]
fn the_reverb_tone_card_has_its_cuts() {
    let c = set(
        Cloud::new(),
        &[
            (cloud::LOW_CUT, 200.0),
            (cloud::HIGH_CUT, 5000.0),
            (cloud::DAMPING, 3000.0),
            (cloud::QUALITY, 0.0),
        ],
    );
    let (mid, _) = c.cuts_db(1000.0);
    let (low, _) = c.cuts_db(40.0);
    let (high, damp) = c.cuts_db(15_000.0);
    assert!(
        mid > -3.0 && low < mid - 6.0 && high < mid - 6.0,
        "{low} {mid} {high}"
    );
    assert!(damp < -6.0, "{damp}");
}
