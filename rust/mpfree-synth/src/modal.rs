//! The modal bank: struck bars, bells, tines and drums as a sum of decaying
//! modes, each a two-pole resonator, struck by a short half-sine pulse.
//! Shared by Pluck's mallets and the drums that need it (toms, congas,
//! woodblocks, bells).
//!
//! A mode at `hz` that falls 60 dB in `t60` seconds is the resonator
//! `y = b0 x + c1 y1 - c2 y2` with pole radius `r = 10^(-3 / (t60 rate))`,
//! `c1 = 2 r cos w`, `c2 = r^2` and `b0 = sin w x gain`, so a unit impulse
//! rings as `gain x r^n sin(w (n + 1))`. A mode at or above 0.45 of the rate,
//! or under 10 Hz, is silenced rather than left to alias or rumble.
//!
//! The mallet is a half-sine pulse ([`Pulse`]): a harder mallet is a shorter
//! pulse, so its spectrum ([`pulse_spectrum`]) reaches higher modes.
//! [`pulse_len`] gives Pluck's law for it. The material tables ([`Modes`])
//! are measured or textbook ratios; Pluck picks one with its Material
//! setting.
//!
//! Nothing here allocates; a bank is a few hundred bytes and `Copy`, so a
//! voice can hold one.

use std::f32::consts::{PI, TAU};

/// A material's modes: (frequency ratio, level, decay ratio).
pub type Modes = &'static [(f32, f32, f32)];

/// Marimba: bars tuned 1 : 4 : 10.
pub const MARIMBA: Modes = &[(1.0, 1.0, 1.0), (3.99, 0.35, 0.45), (9.95, 0.15, 0.25)];
/// Vibes: like the marimba, longer and with less of the upper modes.
pub const VIBES: Modes = &[(1.0, 1.0, 1.0), (4.0, 0.25, 0.6), (10.0, 0.08, 0.3)];
/// A church bell's partials: hum, prime, minor third, fifth, nominal ...
pub const BELL: Modes = &[
    (0.5, 0.6, 1.3),
    (1.0, 1.0, 1.0),
    (1.183, 0.7, 0.9),
    (1.506, 0.5, 0.8),
    (2.0, 0.6, 0.7),
    (2.514, 0.35, 0.6),
    (2.662, 0.3, 0.5),
    (3.011, 0.25, 0.45),
];
/// A wine glass.
pub const GLASS: Modes = &[
    (1.0, 1.0, 1.0),
    (2.32, 0.6, 0.8),
    (4.25, 0.4, 0.6),
    (6.63, 0.25, 0.45),
    (9.38, 0.15, 0.35),
];
/// Kalimba tine.
pub const KALIMBA: Modes = &[
    (1.0, 1.0, 1.0),
    (2.0, 0.08, 0.6),
    (5.4, 0.25, 0.3),
    (13.2, 0.1, 0.15),
];
/// Steel drum: nearly harmonic.
pub const STEEL_DRUM: Modes = &[
    (1.0, 1.0, 1.0),
    (2.0, 0.7, 0.8),
    (3.0, 0.4, 0.6),
    (4.02, 0.25, 0.5),
    (5.03, 0.15, 0.4),
];

/// Materials with many modes would be louder: they share 1.5 among them.
pub fn mode_share(modes: Modes) -> f32 {
    1.5 / modes.iter().map(|m| m.1).sum::<f32>()
}

/// A mode's ratio with the stretch (as a share, -0.5 to 0.5) applied: the
/// distance from the fundamental grows or shrinks by that share.
pub fn stretched(ratio: f32, stretch: f32) -> f32 {
    1.0 + (ratio - 1.0) * (1.0 + stretch)
}

/// A mode's T60 from the note's decay `base` (seconds), its decay ratio
/// and a high-mode damping (as a share, 0 to 1): the more damping, the
/// shorter the modes whose decay ratio is under 1.
pub fn mode_t60(base: f32, dr: f32, damp: f32) -> f32 {
    (base * dr.powf(0.5 + 1.5 * damp)).max(0.005)
}

/// How much mode `k` (0 first) is struck at strike position `pos` (0 to 1,
/// from the end of the bar towards its middle): a mode with a node there
/// barely sounds, but never less than 0.3.
pub fn strike_place(k: usize, pos: f32) -> f32 {
    0.3 + 0.7 * (PI * (k + 1) as f32 * pos.max(0.02)).sin().abs()
}

/// The mallet pulse's length in frames, from a hardness (0 to 100), the
/// velocity's brightness (0 to 1) and the note's frequency. A soft mallet
/// is a long pulse, but never longer than a third of the note's period, so
/// high notes still sound.
pub fn pulse_len(hardness: f32, bright: f32, hz: f32, rate: f32) -> f32 {
    let hard = (hardness / 100.0 * bright).clamp(0.0, 1.0);
    let period = rate / hz.max(1.0);
    (rate * (0.0002 + 0.006 * (1.0 - hard) * (1.0 - hard)))
        .min(period / 3.0)
        .max(2.0)
}

/// The magnitude spectrum of a half-sine pulse `secs` long at `hz`, 1 at
/// 0 Hz: how strongly a mallet that long drives a mode at `hz`.
pub fn pulse_spectrum(hz: f32, secs: f32) -> f32 {
    let d = 1.0 - (2.0 * hz * secs).powi(2);
    if d.abs() < 1e-4 {
        PI / 4.0
    } else {
        ((PI * hz * secs).cos() / d).abs()
    }
}

/// Up to `N` modes, each a two-pole resonator, summed. See the module docs.
#[derive(Clone, Copy, Debug)]
pub struct ModalBank<const N: usize> {
    n: usize,
    y1: [f32; N],
    y2: [f32; N],
    c1: [f32; N],
    c2: [f32; N],
    b0: [f32; N],
    /// Each mode's pole radius and gain, kept for [`ModalBank::retune`].
    r: [f32; N],
    gain: [f32; N],
}

impl<const N: usize> Default for ModalBank<N> {
    fn default() -> ModalBank<N> {
        ModalBank {
            n: 0,
            y1: [0.0; N],
            y2: [0.0; N],
            c1: [0.0; N],
            c2: [0.0; N],
            b0: [0.0; N],
            r: [0.0; N],
            gain: [0.0; N],
        }
    }
}

impl<const N: usize> ModalBank<N> {
    /// How many modes [`ModalBank::tick`] runs.
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Runs the first `n` modes (at most `N`); the others are left as they
    /// are and not heard.
    pub fn set_len(&mut self, n: usize) {
        self.n = n.min(N);
    }

    /// Sets mode `k` to ring at `hz` for `t60` seconds (to -60 dB) with
    /// `gain`, at `rate`. A mode at or above 0.45 of the rate, or at or
    /// under 10 Hz, is silenced; returns whether it sounds. Keeps the mode's
    /// state, so a ringing mode changes pitch or decay without a click.
    pub fn set(&mut self, k: usize, hz: f32, t60: f32, gain: f32, rate: f32) -> bool {
        let live = self.set_with(k, hz, t60, rate, |s| s * gain);
        self.gain[k] = if live { gain } else { 0.0 };
        live
    }

    /// As [`ModalBank::set`], with `b0` worked out from `sin w` by `b0`, so
    /// an engine can keep its own order of multiplication (and with it its
    /// output to the bit). [`ModalBank::retune`] then takes the gain as
    /// `b0 / sin w`.
    pub fn set_with(
        &mut self,
        k: usize,
        hz: f32,
        t60: f32,
        rate: f32,
        b0: impl FnOnce(f32) -> f32,
    ) -> bool {
        if hz >= 0.45 * rate || hz <= 10.0 {
            self.silence(k);
            return false;
        }
        let w = TAU * hz / rate;
        let rr = 10f32.powf(-3.0 / (t60 * rate));
        let s = w.sin();
        self.c1[k] = 2.0 * rr * w.cos();
        self.c2[k] = rr * rr;
        self.b0[k] = b0(s);
        self.r[k] = rr;
        self.gain[k] = self.b0[k] / s;
        true
    }

    /// Moves mode `k` to `hz`, keeping its decay (pole radius) and gain:
    /// cheaper than [`ModalBank::set`] for a pitch that moves every control
    /// step (a drum's pitch drop). A mode moved out of range is silenced
    /// and stays so until it is set again.
    pub fn retune(&mut self, k: usize, hz: f32, rate: f32) -> bool {
        if hz >= 0.45 * rate || hz <= 10.0 || self.r[k] == 0.0 {
            self.silence(k);
            return false;
        }
        let w = TAU * hz / rate;
        let (s, c) = w.sin_cos();
        self.c1[k] = 2.0 * self.r[k] * c;
        self.b0[k] = s * self.gain[k];
        true
    }

    /// Silences mode `k` from now on: what it was ringing stops at once.
    pub fn silence(&mut self, k: usize) {
        self.b0[k] = 0.0;
        self.c1[k] = 0.0;
        self.c2[k] = 0.0;
        self.r[k] = 0.0;
        self.gain[k] = 0.0;
    }

    /// Stops every mode ringing (the settings stay).
    pub fn clear(&mut self) {
        self.y1 = [0.0; N];
        self.y2 = [0.0; N];
    }

    /// One frame: drives every mode with `x` and returns their sum.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let mut s = 0.0;
        for k in 0..self.n {
            let y = self.b0[k] * x + self.c1[k] * self.y1[k] - self.c2[k] * self.y2[k];
            self.y2[k] = self.y1[k];
            self.y1[k] = y;
            s += y;
        }
        s
    }
}

/// A half-sine mallet pulse, `len` frames long, peaking at `amp`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pulse {
    /// Frames of it played so far.
    pub at: u32,
    /// Its length in frames.
    pub len: u32,
    /// Its peak.
    pub amp: f32,
}

impl Pulse {
    /// Starts a pulse `len` frames long peaking at `amp`.
    pub fn start(&mut self, len: u32, amp: f32) {
        self.at = 0;
        self.len = len;
        self.amp = amp;
    }

    /// Whether the pulse is still playing.
    #[inline]
    pub fn active(&self) -> bool {
        self.at < self.len
    }

    /// Its next frame (0 once it has ended).
    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.at < self.len {
            let t = (self.at as f32 + 0.5) / self.len as f32;
            self.at += 1;
            self.amp * (PI * t).sin()
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// The frequency of a ringing signal from its zero crossings.
    fn crossings_hz(x: &[f32]) -> f32 {
        let ups: Vec<f32> = (1..x.len())
            .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
            .map(|i| (i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]))
            .collect();
        let n = ups.len() - 1;
        RATE * n as f32 / (ups[n] - ups[0])
    }

    #[test]
    fn a_mode_rings_at_its_pitch_and_falls_60_db_in_its_t60() {
        let mut b = ModalBank::<4>::default();
        b.set_len(1);
        assert!(b.set(0, 440.0, 0.5, 1.0, RATE));
        let mut x: Vec<f32> = Vec::new();
        x.push(b.tick(1.0));
        for _ in 0..(RATE as usize) {
            x.push(b.tick(0.0));
        }
        let hz = crossings_hz(&x[..12_000]);
        assert!((hz - 440.0).abs() < 0.05, "{hz}");
        // The envelope after t60 is 60 dB under the start.
        let peak = |from: usize| x[from..from + 200].iter().fold(0f32, |m, v| m.max(v.abs()));
        let drop = 20.0 * (peak(24_000) / peak(0)).log10();
        assert!((drop + 60.0).abs() < 0.5, "{drop}");
    }

    #[test]
    fn modes_out_of_range_are_silent() {
        let mut b = ModalBank::<3>::default();
        b.set_len(3);
        assert!(!b.set(0, 0.45 * RATE, 1.0, 1.0, RATE));
        assert!(!b.set(1, 10.0, 1.0, 1.0, RATE));
        assert!(b.set(2, 1000.0, 1.0, 1.0, RATE));
        b.silence(2);
        let sum: f32 = (0..1000)
            .map(|i| b.tick(if i == 0 { 1.0 } else { 0.0 }).abs())
            .sum();
        assert_eq!(sum, 0.0);
    }

    #[test]
    fn retune_moves_the_pitch_and_keeps_the_decay_and_gain() {
        let mut a = ModalBank::<1>::default();
        let mut b = ModalBank::<1>::default();
        a.set_len(1);
        b.set_len(1);
        a.set(0, 300.0, 0.4, 0.7, RATE);
        b.set(0, 600.0, 0.4, 0.7, RATE);
        b.retune(0, 300.0, RATE);
        for i in 0..4000 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (ya, yb) = (a.tick(x), b.tick(x));
            assert!((ya - yb).abs() < 1e-6, "{i}: {ya} {yb}");
        }
        // Out of range silences it, and it stays silent.
        assert!(!b.retune(0, 30_000.0, RATE));
        assert!(!b.retune(0, 300.0, RATE));
    }

    #[test]
    fn set_with_keeps_the_engines_own_order_of_multiplication() {
        let (l, p, h) = (0.35f32, 0.913, 0.6818);
        let mut a = ModalBank::<1>::default();
        a.set_with(0, 523.25, 1.0, RATE, |s| s * l * p * h);
        let w = TAU * 523.25 / RATE;
        assert_eq!(a.b0[0].to_bits(), (w.sin() * l * p * h).to_bits());
    }

    #[test]
    fn the_pulse_is_a_half_sine_of_its_length() {
        let mut p = Pulse::default();
        assert!(!p.active());
        assert_eq!(p.next(), 0.0);
        p.start(10, 2.0);
        let x: Vec<f32> = (0..12).map(|_| p.next()).collect();
        assert!((x[4] - 2.0 * (PI * 0.45).sin()).abs() < 1e-6);
        assert!((x[4] - x[5]).abs() < 1e-6);
        assert_eq!(x[10], 0.0);
        assert!(!p.active());
        // Its spectrum: 1 at 0 Hz, the first zero at 1.5 / length.
        assert!((pulse_spectrum(0.0, 0.01) - 1.0).abs() < 1e-6);
        assert!(pulse_spectrum(150.0, 0.01) < 1e-5);
        assert!((pulse_spectrum(50.0, 0.01) - PI / 4.0).abs() < 1e-6);
    }

    #[test]
    fn the_helpers_keep_their_laws() {
        assert_eq!(stretched(1.0, 0.5), 1.0);
        assert!((stretched(4.0, 0.5) - 5.5).abs() < 1e-6);
        assert!((mode_t60(2.0, 1.0, 0.7) - 2.0).abs() < 1e-6);
        assert!(mode_t60(2.0, 0.25, 1.0) < mode_t60(2.0, 0.25, 0.0));
        assert!((strike_place(0, 0.5) - 1.0).abs() < 1e-6);
        assert!((strike_place(1, 0.5) - 0.3).abs() < 1e-6);
        assert!((mode_share(MARIMBA) - 1.5 / 1.5).abs() < 1e-6);
        // A soft mallet is long, but never past a third of the period.
        assert!(pulse_len(0.0, 1.0, 100.0, RATE) > pulse_len(100.0, 1.0, 100.0, RATE));
        assert!(pulse_len(0.0, 1.0, 2000.0, RATE) <= RATE / 2000.0 / 3.0 + 1e-3);
    }
}
