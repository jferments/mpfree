//! The per-grain filter (the granular roadmap's step C4, the plan's
//! decision 4: grit in the engine). Each grain can go through a low, band
//! or high pass of its own, its cutoff set when it is born and, with
//! spray, a random distance from the cutoff in octaves: a cloud of grains
//! each a different colour, from glassy (a high pass, sprayed) to gnarly
//! (a resonant low pass swept by the LFO), as Grainferno does.
//!
//! The filter is a state-variable filter in its trapezoidal form
//! (Zavalishin, "The Art of VA Filter Design"; Simper's "Linear Trap
//! Integrated SVF"): stable at any cutoff and resonance, and with
//! coefficients that are fixed for the grain's life it costs a few
//! multiplies a sample. It filters what the grain reads, before its
//! window, so the filter's start from silence is under the window's
//! fade-in and every grain still ends at zero.

/// Which pass, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterKind {
    #[default]
    Off,
    /// 12 dB an octave above the cutoff; resonance peaks at it.
    Low,
    /// Around the cutoff, its peak at full level at any resonance (which
    /// narrows it).
    Band,
    /// 12 dB an octave below the cutoff; resonance peaks at it.
    High,
}

/// The lowest and highest cutoff, Hz (a grain's cutoff is held under 45 %
/// of the rate as well).
pub const CUTOFF_MIN_HZ: f32 = 20.0;
pub const CUTOFF_MAX_HZ: f32 = 20_000.0;
/// Octaves either way a grain's cutoff may land from the cutoff at full
/// spray (even in octaves).
pub const FILTER_SPRAY_OCTAVES: f32 = 3.0;
/// The filter's Q at no resonance (Butterworth: flat to the cutoff) and at
/// full.
pub const Q_MIN: f32 = std::f32::consts::FRAC_1_SQRT_2;
pub const Q_MAX: f32 = 12.0;

/// A grain filter's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrainFilter {
    pub kind: FilterKind,
    /// Hz, [`CUTOFF_MIN_HZ`] to [`CUTOFF_MAX_HZ`].
    pub cutoff_hz: f32,
    /// 0 to 1: Q from [`Q_MIN`] to [`Q_MAX`], evenly in octaves.
    pub resonance: f32,
    /// 0 to 1: how far each grain's cutoff lands from the cutoff, up to
    /// [`FILTER_SPRAY_OCTAVES`] either way.
    pub spray: f32,
}

impl Default for GrainFilter {
    fn default() -> Self {
        GrainFilter {
            kind: FilterKind::Off,
            cutoff_hz: 2_000.0,
            resonance: 0.0,
            spray: 0.0,
        }
    }
}

impl GrainFilter {
    /// The filter is in the path.
    pub fn on(&self) -> bool {
        self.kind != FilterKind::Off
    }

    /// The cutoff a grain gets for an even draw `r` in -1 to 1, clamped.
    pub fn cutoff_for(&self, r: f32) -> f32 {
        let spray = if self.spray.is_finite() {
            self.spray.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let c = if self.cutoff_hz.is_finite() {
            self.cutoff_hz
        } else {
            2_000.0
        };
        (c * (FILTER_SPRAY_OCTAVES * spray * r).exp2()).clamp(CUTOFF_MIN_HZ, CUTOFF_MAX_HZ)
    }

    /// The Q for its resonance.
    pub fn q(&self) -> f32 {
        let r = if self.resonance.is_finite() {
            self.resonance.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Q_MIN * (Q_MAX / Q_MIN).powf(r)
    }
}

/// One grain's filter: fixed coefficients and a stereo state.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
    /// The output's mix of the input, the band and the low pass.
    m0: f32,
    m1: f32,
    m2: f32,
    /// Each side's two integrators.
    s: [f32; 4],
}

impl Svf {
    /// A filter of `f`'s kind and resonance at `cutoff_hz`, at `rate`.
    /// `None` when the kind is Off.
    pub(crate) fn new(f: &GrainFilter, cutoff_hz: f32, rate: u32) -> Option<Svf> {
        let rate = rate.max(1) as f32;
        let fc = cutoff_hz.clamp(CUTOFF_MIN_HZ, 0.45 * rate);
        let g = (std::f32::consts::PI * fc / rate).tan();
        let k = 1.0 / f.q();
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let (m0, m1, m2) = match f.kind {
            FilterKind::Off => return None,
            FilterKind::Low => (0.0, 0.0, 1.0),
            // k times the band: its peak is at full level.
            FilterKind::Band => (0.0, k, 0.0),
            FilterKind::High => (1.0, -k, -1.0),
        };
        Some(Svf {
            a1,
            a2,
            a3,
            m0,
            m1,
            m2,
            s: [0.0; 4],
        })
    }

    /// One stereo frame through it.
    #[inline(always)]
    pub(crate) fn tick(&mut self, [l, r]: [f32; 2]) -> [f32; 2] {
        let mut side = |v0: f32, c: usize| {
            let (ic1, ic2) = (self.s[c], self.s[c + 1]);
            let v3 = v0 - ic2;
            let v1 = self.a1 * ic1 + self.a2 * v3;
            let v2 = ic2 + self.a2 * ic1 + self.a3 * v3;
            self.s[c] = 2.0 * v1 - ic1;
            self.s[c + 1] = 2.0 * v2 - ic2;
            self.m0 * v0 + self.m1 * v1 + self.m2 * v2
        };
        let a = side(l, 0);
        let b = side(r, 2);
        [a, b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(kind: FilterKind, cutoff: f32, res: f32, hz: f32) -> f32 {
        let f = GrainFilter {
            kind,
            cutoff_hz: cutoff,
            resonance: res,
            spray: 0.0,
        };
        let mut s = Svf::new(&f, cutoff, 48_000).unwrap();
        let w = std::f32::consts::TAU * hz / 48_000.0;
        let mut peak = 0f32;
        for i in 0..48_000 {
            let x = (w * i as f32).sin();
            let [y, _] = s.tick([x, x]);
            if i > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        20.0 * peak.log10()
    }

    #[test]
    fn each_pass_passes_and_stops_where_it_should() {
        // Low: flat below, -3 dB at the cutoff, 12 dB an octave above.
        assert!(gain(FilterKind::Low, 1_000.0, 0.0, 100.0).abs() < 0.1);
        assert!((gain(FilterKind::Low, 1_000.0, 0.0, 1_000.0) + 3.0).abs() < 0.2);
        assert!(gain(FilterKind::Low, 1_000.0, 0.0, 8_000.0) < -34.0);
        // High: the mirror.
        assert!(gain(FilterKind::High, 1_000.0, 0.0, 10_000.0).abs() < 0.2);
        assert!(gain(FilterKind::High, 1_000.0, 0.0, 125.0) < -34.0);
        // Band: full level at the cutoff, at any resonance.
        for res in [0.0, 0.5, 1.0] {
            assert!(
                gain(FilterKind::Band, 1_000.0, res, 1_000.0).abs() < 0.1,
                "{res}"
            );
        }
        assert!(gain(FilterKind::Band, 1_000.0, 1.0, 2_000.0) < -20.0);
    }

    #[test]
    fn resonance_peaks_at_its_q_and_never_runs_away() {
        let q = GrainFilter {
            resonance: 1.0,
            ..GrainFilter::default()
        }
        .q();
        assert!((q - Q_MAX).abs() < 1e-3);
        let peak = gain(FilterKind::Low, 1_000.0, 1.0, 1_000.0);
        assert!((peak - 20.0 * Q_MAX.log10()).abs() < 0.3, "{peak}");
        // Near the top of the range the cutoff is held under Nyquist.
        assert!(gain(FilterKind::Low, 20_000.0, 1.0, 15_000.0).is_finite());
    }

    #[test]
    fn spray_spreads_cutoffs_evenly_in_octaves() {
        let f = GrainFilter {
            kind: FilterKind::Low,
            cutoff_hz: 1_000.0,
            resonance: 0.0,
            spray: 1.0,
        };
        assert!((f.cutoff_for(1.0) - 8_000.0).abs() < 1.0);
        assert!((f.cutoff_for(-1.0) - 125.0).abs() < 0.1);
        assert_eq!(f.cutoff_for(0.0), 1_000.0);
        let none = GrainFilter { spray: 0.0, ..f };
        assert_eq!(none.cutoff_for(1.0), 1_000.0);
        assert_eq!(
            GrainFilter {
                cutoff_hz: f32::NAN,
                ..none
            }
            .cutoff_for(0.0),
            2_000.0
        );
    }
}
