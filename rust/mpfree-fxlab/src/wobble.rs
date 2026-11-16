//! How much a reverb's or a delay's modulation smears a held note: a
//! steady tone goes in, and the spread of the output's spectrum around
//! the tone is read in cents (its RMS distance from the tone, power
//! weighted, within 100 cents either side). For a tone whose pitch
//! wavers, that equals the RMS of the wavering; an unmodulated (linear,
//! time-invariant) path leaves a pure tone and reads near 0. The
//! analysis window's own spread is taken out (the FX quality plan's R9).

use crate::fft::{hann_f64, power_spectrum};
use crate::signals::{db_to_amp, sine};
use crate::wav::Stereo;

/// Frames analysed: about 3 s at 44.1 kHz, a resolution of 1.3 cents at
/// 440 Hz.
pub const FRAMES: usize = 1 << 17;

/// The spread of `x`'s last [`FRAMES`] around `tone_hz`, cents RMS.
pub fn spread_cents(x: &[f64], rate: u32, tone_hz: f64) -> Option<f64> {
    if x.len() < FRAMES {
        return None;
    }
    let w = hann_f64(FRAMES);
    let piece: Vec<f64> = x[x.len() - FRAMES..]
        .iter()
        .zip(&w)
        .map(|(v, k)| v * k)
        .collect();
    let p = power_spectrum(&piece);
    let hz = |k: usize| k as f64 * rate as f64 / FRAMES as f64;
    let span = 2f64.powf(100.0 / 1200.0);
    let (mut sw, mut sf) = (0.0, 0.0);
    let bins: Vec<usize> = (1..p.len())
        .filter(|k| hz(*k) >= tone_hz / span && hz(*k) <= tone_hz * span)
        .collect();
    for &k in &bins {
        sw += p[k];
        sf += p[k] * hz(k);
    }
    if sw <= 0.0 {
        return None;
    }
    let mean = sf / sw;
    let var = bins
        .iter()
        .map(|&k| p[k] * (hz(k) - mean).powi(2))
        .sum::<f64>()
        / sw;
    // A Hann window of T seconds spreads a pure tone by 1 / (sqrt 3 T) Hz.
    let window = rate as f64 / (3f64.sqrt() * FRAMES as f64);
    let sigma = (var - window * window).max(0.0).sqrt();
    Some(1200.0 * (1.0 + sigma / mean).log2())
}

/// Plays a 440 Hz tone at -18 dBFS through a device for `secs` seconds
/// (at least [`FRAMES`] plus a second for the effect to settle) and reads
/// [`spread_cents`] of the output's middle while the tone still plays.
pub fn measure(dut: &mut dyn crate::dut::Dut, secs: f64) -> Option<f64> {
    let rate = dut.rate();
    let n = ((secs * rate as f64) as usize).max(FRAMES + rate as usize);
    let out = dut.render(
        &Stereo::mono(sine(440.0, db_to_amp(-18.0), n, rate), rate),
        0,
    );
    spread_cents(&out.mid(), rate, 440.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vibrato(cents: f64, rate_hz: f64) -> Vec<f64> {
        let fs = 44_100.0;
        let mut ph = 0.0f64;
        (0..FRAMES + 4410)
            .map(|i| {
                let t = i as f64 / fs;
                let c = cents * (std::f64::consts::TAU * rate_hz * t).sin();
                ph += std::f64::consts::TAU * 440.0 * 2f64.powf(c / 1200.0) / fs;
                ph.sin()
            })
            .collect()
    }

    #[test]
    fn a_pure_tone_reads_near_zero_and_vibrato_reads_its_rms() {
        let pure = spread_cents(&vibrato(0.0, 5.0), 44_100, 440.0).unwrap();
        assert!(pure < 0.5, "{pure}");
        for (depth, rate) in [(10.0, 5.0), (25.0, 3.0), (4.0, 0.7)] {
            let got = spread_cents(&vibrato(depth, rate), 44_100, 440.0).unwrap();
            let want = depth / std::f64::consts::SQRT_2;
            assert!(
                (got - want).abs() < 0.12 * want + 0.3,
                "{depth} cents at {rate} Hz: {got} against {want}"
            );
        }
    }
}
