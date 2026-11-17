//! A whole sound put at another rate: the same tones at the same level,
//! the right length, and nothing above the new rate's Nyquist folded in.

use mpfree_stretch::convert_rate;

fn sine(rate: u32, seconds: f64, hz: f64, channels: usize) -> Vec<f32> {
    let n = (seconds * f64::from(rate)) as usize;
    (0..n)
        .flat_map(|i| {
            let v = (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin()) as f32;
            std::iter::repeat_n(v, channels)
        })
        .collect()
}

/// The level of `hz` in channel `c` of `x` (`ch` interleaved) at `rate`,
/// over its middle half, Hann-windowed.
fn level(x: &[f32], ch: usize, c: usize, rate: u32, hz: f64) -> f64 {
    let frames = x.len() / ch;
    let (a, b) = (frames / 4, 3 * frames / 4);
    let n = (b - a) as f64;
    let (mut re, mut im, mut ws) = (0.0, 0.0, 0.0);
    for i in a..b {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * (i - a) as f64 / n).cos();
        let ph = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
        let v = f64::from(x[i * ch + c]);
        re += w * v * ph.cos();
        im += w * v * ph.sin();
        ws += w;
    }
    2.0 * (re * re + im * im).sqrt() / ws
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

#[test]
fn a_tone_keeps_its_pitch_and_level_at_the_new_rate() {
    for (from, to, hz) in [
        (48_000, 44_100, 1_000.0),
        (48_000, 44_100, 15_000.0),
        (96_000, 44_100, 5_000.0),
        (22_050, 44_100, 3_000.0),
        (44_100, 48_000, 18_000.0),
    ] {
        for ch in [1usize, 2] {
            let x = sine(from, 0.5, hz, ch);
            let y = convert_rate(&x, ch, from, to);
            let want = (0.5 * f64::from(to)).round() as usize;
            assert_eq!(y.len(), want * ch, "{from} -> {to}: length");
            for c in 0..ch {
                let l = db(level(&y, ch, c, to, hz));
                assert!(
                    (l - db(0.5)).abs() < 0.05,
                    "{from} -> {to}, {hz} Hz, channel {c}: {l:.2} dB"
                );
            }
        }
    }
}

#[test]
fn what_the_new_rate_cannot_hold_is_gone_not_folded() {
    // 30 kHz in a 96 kHz file would land at 14.1 kHz at 44.1 kHz.
    let x = sine(96_000, 0.5, 30_000.0, 1);
    let y = convert_rate(&x, 1, 96_000, 44_100);
    let alias = db(level(&y, 1, 0, 44_100, 44_100.0 - 30_000.0));
    assert!(alias < -70.0, "alias {alias:.1} dB");
    // From 48 kHz the cut is gentler, centred just under the new
    // Nyquist: what folds lands above 20.6 kHz, out of hearing, and is at
    // least 60 dB down below that.
    for (hz, most) in [(23_500.0, -60.0), (23_000.0, -35.0)] {
        let x = sine(48_000, 0.5, hz, 1);
        let y = convert_rate(&x, 1, 48_000, 44_100);
        let alias = db(level(&y, 1, 0, 44_100, 44_100.0 - hz));
        assert!(alias < most, "{hz} Hz: alias {alias:.1} dB");
    }
    // The band below 20 kHz is kept.
    let x = sine(48_000, 0.5, 20_000.0, 1);
    let y = convert_rate(&x, 1, 48_000, 44_100);
    let kept = db(level(&y, 1, 0, 44_100, 20_000.0)) - db(0.5);
    assert!(kept > -0.5, "20 kHz: {kept:.2} dB");
}

#[test]
fn the_same_rate_is_the_same_sound() {
    let x = sine(44_100, 0.1, 440.0, 2);
    assert_eq!(convert_rate(&x, 2, 44_100, 44_100), x);
}

#[test]
fn an_odd_rate_with_few_common_factors_converts_too() {
    // 44,099 Hz and 44,100 Hz share no factor: the slow way.
    let x = sine(44_099, 0.5, 2_000.0, 2);
    let y = convert_rate(&x, 2, 44_099, 44_100);
    let l = db(level(&y, 2, 1, 44_100, 2_000.0));
    assert!((l - db(0.5)).abs() < 0.05, "{l:.2} dB");
}
