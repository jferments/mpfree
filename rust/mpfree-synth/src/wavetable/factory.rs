//! The factory tables, generated from formulas when first needed (no
//! samples, no third-party content) and shared by every Wavetable node in
//! the process. About 8.6 MB in all; building them takes a few tens of
//! milliseconds on a desktop, so the first [`factory`] call belongs on the
//! control thread (Wavetable::new makes it).

use std::f64::consts::{PI, TAU};
use std::sync::{Arc, OnceLock};

use super::table::{Spectrum, Table, MAX_HARMONICS};

/// Factory table names, in the order of the Table choice.
pub const NAMES: &[&str] = &[
    "Basic",
    "Harmonics",
    "Pulse",
    "Sync",
    "Vowels",
    "Organ",
    "Bits",
    "Resonance",
    "FM",
    "Drift",
];

/// Samples per cycle for drawn (time-domain) frames.
const DRAW: usize = 16_384;

static TABLES: OnceLock<Vec<Arc<Table>>> = OnceLock::new();

/// Every factory table, built on the first call.
pub fn factory() -> &'static [Arc<Table>] {
    TABLES.get_or_init(|| {
        NAMES
            .iter()
            .enumerate()
            .map(|(i, n)| Arc::new(build(i, n)))
            .collect()
    })
}

/// Bytes the factory tables hold.
pub fn factory_bytes() -> usize {
    factory().iter().map(|t| t.bytes()).sum()
}

fn build(i: usize, name: &str) -> Table {
    let spectra = match i {
        0 => basic(),
        1 => harmonics(),
        2 => pulse(),
        3 => drawn(16, |k, t| saw((t * (1.0 + 5.0 * k)).fract())),
        4 => vowels(),
        5 => organ(),
        6 => drawn(16, |k, t| {
            let steps = (2.0f64.powf(1.0 + 5.0 * (1.0 - k))).round();
            ((TAU * t).sin() * steps).round() / steps
        }),
        7 => drawn(16, |k, t| (1.0 - t) * (TAU * t * (1.0 + 11.0 * k)).sin()),
        8 => drawn(16, |k, t| (TAU * t + 6.0 * k * (TAU * 3.0 * t).sin()).sin()),
        _ => drift(),
    };
    Table::from_spectra(name, &spectra).expect("factory tables are never silent")
}

/// A falling saw at phase `t`, 1 to -1.
fn saw(t: f64) -> f64 {
    1.0 - 2.0 * t
}

/// `n` frames drawn by `f(k, t)`: `k` is 0 to 1 through the table, `t` the
/// phase in the cycle.
fn drawn(n: usize, f: impl Fn(f64, f64) -> f64) -> Vec<Spectrum> {
    (0..n)
        .map(|j| {
            let k = j as f64 / (n - 1) as f64;
            let cycle: Vec<f32> = (0..DRAW)
                .map(|i| f(k, i as f64 / DRAW as f64) as f32)
                .collect();
            Spectrum::of_cycle(&cycle)
        })
        .collect()
}

/// Sine harmonics `amp(n)` for n = 1 to 512.
fn sines(amp: impl Fn(usize) -> f64) -> Spectrum {
    let a: Vec<f64> = (1..=MAX_HARMONICS).map(amp).collect();
    Spectrum::from_sines(&a)
}

/// Morphs through key spectra over `n` frames.
fn morph(keys: &[Spectrum], n: usize) -> Vec<Spectrum> {
    (0..n)
        .map(|j| {
            let x = j as f64 / (n - 1) as f64 * (keys.len() - 1) as f64;
            let i = (x.floor() as usize).min(keys.len() - 2);
            keys[i].lerp(&keys[i + 1], x - i as f64)
        })
        .collect()
}

/// Sine, triangle, saw, square.
fn basic() -> Vec<Spectrum> {
    let sine = sines(|n| if n == 1 { 1.0 } else { 0.0 });
    let tri = sines(|n| {
        if n % 2 == 1 {
            let s = if (n / 2) % 2 == 0 { 1.0 } else { -1.0 };
            s * 8.0 / (PI * PI * (n * n) as f64)
        } else {
            0.0
        }
    });
    let saw = sines(|n| 2.0 / (PI * n as f64));
    let square = sines(|n| {
        if n % 2 == 1 {
            4.0 / (PI * n as f64)
        } else {
            0.0
        }
    });
    morph(&[sine, tri, saw, square], 16)
}

/// A saw built up a harmonic band at a time: 1 harmonic to all 512.
fn harmonics() -> Vec<Spectrum> {
    let n = 32;
    (0..n)
        .map(|j| {
            let top = 2.0f64.powf(9.0 * j as f64 / (n - 1) as f64).round() as usize;
            sines(|h| if h <= top { 1.0 / h as f64 } else { 0.0 })
        })
        .collect()
}

/// A pulse from square (50 %) to a narrow 3 % spike: a saw minus a copy
/// shifted by the width.
fn pulse() -> Vec<Spectrum> {
    let n = 16;
    (0..n)
        .map(|j| {
            let w = 0.5 - 0.47 * j as f64 / (n - 1) as f64;
            let mut s = Spectrum::zero();
            for h in 1..=MAX_HARMONICS {
                let a = 1.0 / h as f64;
                // Saw bin: -i a/2. Shifted copy: times e^{-i 2 pi h w}.
                let (c, sn) = ((TAU * h as f64 * w).cos(), (TAU * h as f64 * w).sin());
                let (re0, im0) = (0.0, -a / 2.0);
                let (re1, im1) = (re0 * c + im0 * sn, im0 * c - re0 * sn);
                s.re[h] = re0 - re1;
                s.im[h] = im0 - im1;
            }
            s
        })
        .collect()
}

/// A voice through A, E, I, O, U: harmonics of a 110 Hz voice shaped by
/// three formants.
fn vowels() -> Vec<Spectrum> {
    // (F1, F2, F3) in Hz.
    const V: [[f64; 3]; 5] = [
        [800.0, 1150.0, 2900.0],
        [400.0, 1600.0, 2700.0],
        [270.0, 2300.0, 3000.0],
        [450.0, 800.0, 2830.0],
        [325.0, 700.0, 2530.0],
    ];
    let f0 = 110.0;
    let n = 16;
    (0..n)
        .map(|j| {
            let x = j as f64 / (n - 1) as f64 * 4.0;
            let i = (x.floor() as usize).min(3);
            let t = x - i as f64;
            let f: Vec<f64> = (0..3)
                .map(|q| V[i][q] + (V[i + 1][q] - V[i][q]) * t)
                .collect();
            sines(|h| {
                let hz = h as f64 * f0;
                if hz > 8_000.0 {
                    return 0.0;
                }
                let res = |c: f64, bw: f64, g: f64| g / (1.0 + ((hz - c) / bw).powi(2));
                (res(f[0], 80.0, 1.0) + res(f[1], 100.0, 0.5) + res(f[2], 150.0, 0.25))
                    / (h as f64).sqrt()
            })
        })
        .collect()
}

/// Drawbar settings, 16' as the fundamental.
fn organ() -> Vec<Spectrum> {
    // Harmonic of each drawbar: 16', 5 1/3', 8', 4', 2 2/3', 2', 1 3/5', 1 1/3', 1'.
    const H: [usize; 9] = [1, 3, 2, 4, 6, 8, 10, 12, 16];
    const REG: [[f64; 9]; 6] = [
        [8.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [8.0, 8.0, 8.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [8.0, 6.0, 8.0, 8.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [8.0, 8.0, 8.0, 6.0, 0.0, 0.0, 0.0, 0.0, 4.0],
        [8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 0.0, 0.0, 0.0],
        [8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0],
    ];
    let keys: Vec<Spectrum> = REG
        .iter()
        .map(|r| {
            let mut a = [0.0; 16];
            for (d, &h) in H.iter().enumerate() {
                // A drawbar step is 3 dB.
                if r[d] > 0.0 {
                    a[h - 1] += 10f64.powf(-3.0 * (8.0 - r[d]) / 20.0);
                }
            }
            sines(|n| if n <= 16 { a[n - 1] } else { 0.0 })
        })
        .collect();
    morph(&keys, 16)
}

/// Seeded random spectra with a gentle tilt, morphing through five keys.
fn drift() -> Vec<Spectrum> {
    let mut rng = crate::dsp::Rng::new(0x00D1_2F7A_B1E5);
    let keys: Vec<Spectrum> = (0..5)
        .map(|_| {
            let r: Vec<f64> = (0..64).map(|_| f64::from(rng.unit())).collect();
            sines(|n| {
                if n <= 64 {
                    r[n - 1] * r[n - 1] / (n as f64).powf(0.7)
                } else {
                    0.0
                }
            })
        })
        .collect();
    morph(&keys, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_factory_table_builds_and_stays_in_budget() {
        let t = factory();
        assert_eq!(t.len(), NAMES.len());
        for (tab, name) in t.iter().zip(NAMES) {
            assert_eq!(&tab.name, name);
            assert!(tab.frames() >= 16);
            let p = tab
                .preview(0)
                .iter()
                .chain(tab.preview(tab.frames() - 1))
                .fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(p > 0.05 && p <= 1.0001, "{name}: {p}");
        }
        let mb = factory_bytes() as f64 / 1e6;
        assert!(mb < 10.0, "factory tables take {mb:.1} MB");
    }

    #[test]
    fn basic_starts_as_a_sine() {
        let t = &factory()[0];
        let lv = &t.levels()[0];
        let f = lv.frame(0);
        // A sine: zero at the start, its peak a quarter in, and the same
        // the other way at three quarters. (The square at the far end
        // peaks louder, and the table is normalised as a whole.)
        let peak = f.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(f[0].abs() < 1e-4);
        assert!((f[lv.len / 4] - peak).abs() < 1e-4 && (f[3 * lv.len / 4] + peak).abs() < 1e-4);
        assert!((f[lv.len / 8] - peak * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }
}
