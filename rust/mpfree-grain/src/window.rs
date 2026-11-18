//! Grain windows as 1024-entry tables read with linear interpolation.

use std::f32::consts::PI;

/// Table resolution. The table has one extra entry so phase 1.0 reads the
/// last point without a branch on the hot path.
pub const TABLE: usize = 1024;

/// The shape of a grain's amplitude.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Window {
    /// Smooth bell. The default.
    #[default]
    Hann,
    /// Flat top with cosine tapers. `taper` is the fraction of the grain
    /// spent fading (0.01 to 1; 1 is the same as Hann).
    Tukey { taper: f32 },
    /// Linear fades, flat top. `ramp` is each fade's fraction (0.01 to 0.5).
    Trapezoid { ramp: f32 },
    /// Percussive: a 2 % attack, then an exponential decay to zero.
    /// `decay` is the steepness (1 to 20).
    Exponential { decay: f32 },
    /// One control from square to plucked, `shape` 0 to 1 (the Shape
    /// knob): square at 0 (1 % linear fades), the fades lengthening to a
    /// triangle at 0.4, its corners rounding to Hann at 0.6, the peak
    /// moving late to a swell at 0.8 (slow attack, fast decay: peak at
    /// 92 %), then sweeping back to a pluck at 1 (fast attack, slow decay:
    /// peak at 4 %). See [`shaped`].
    Shape { shape: f32 },
}

impl Window {
    /// Where this window sits on the Shape control (0 to 1): exactly for a
    /// [`Window::Shape`], Hann and a trapezoid; the nearest for a Tukey
    /// window (the trapezoid with its fades' length) and the exponential
    /// (a pluck).
    pub fn shape_value(self) -> f32 {
        let trapezoid = |ramp: f32| ((ramp.clamp(0.01, 0.5) - 0.01) / 0.49 * 0.4).clamp(0.0, 0.4);
        match self {
            Window::Shape { shape } => clean_shape(shape),
            Window::Hann => 0.6,
            Window::Trapezoid { ramp } => trapezoid(ramp),
            Window::Tukey { taper } if taper >= 1.0 => 0.6,
            Window::Tukey { taper } => trapezoid(taper * 0.5),
            Window::Exponential { .. } => 1.0,
        }
    }
}

/// A Shape value in 0 to 1 (Hann's for one that isn't a number).
fn clean_shape(shape: f32) -> f32 {
    if shape.is_finite() {
        shape.clamp(0.0, 1.0)
    } else {
        0.6
    }
}

/// [`Window::Shape`]'s window at `x` in 0..1.
pub fn shaped(shape: f32, x: f32) -> f32 {
    let s = clean_shape(shape);
    let x = x.clamp(0.0, 1.0);
    if s <= 0.4 {
        // Square to triangle: linear fades from 1 % to half the grain.
        let r = 0.01 + 0.49 * (s / 0.4);
        return (x / r).min((1.0 - x) / r).clamp(0.0, 1.0);
    }
    if s <= 0.6 {
        // Triangle to Hann.
        let u = (s - 0.4) / 0.2;
        let tri = 1.0 - (2.0 * x - 1.0).abs();
        let hann = 0.5 - 0.5 * (2.0 * PI * x).cos();
        return (1.0 - u) * tri + u * hann;
    }
    // Hann to swell (peak 0.5 to 0.92), then the peak sweeps back to a
    // pluck (0.92 to 0.04): a raised-cosine rise to the peak and fall
    // after it, which is Hann when the peak is in the middle.
    let p = if s <= 0.8 {
        0.5 + 0.42 * (s - 0.6) / 0.2
    } else {
        0.92 - 0.88 * (s - 0.8) / 0.2
    };
    if x < p {
        0.5 - 0.5 * (PI * x / p).cos()
    } else {
        0.5 + 0.5 * (PI * (x - p) / (1.0 - p)).cos()
    }
}

/// The window's value at `x` in 0..1. Every shape is 0 at both ends, so
/// grains never click.
pub fn value(shape: Window, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    match shape {
        Window::Hann => 0.5 - 0.5 * (2.0 * PI * x).cos(),
        Window::Tukey { taper } => {
            let h = taper.clamp(0.01, 1.0) * 0.5;
            if x < h {
                0.5 - 0.5 * (PI * x / h).cos()
            } else if x > 1.0 - h {
                0.5 - 0.5 * (PI * (1.0 - x) / h).cos()
            } else {
                1.0
            }
        }
        Window::Trapezoid { ramp } => {
            let r = ramp.clamp(0.01, 0.5);
            (x / r).min((1.0 - x) / r).clamp(0.0, 1.0)
        }
        Window::Shape { shape } => shaped(shape, x),
        Window::Exponential { decay } => {
            let a = 0.02;
            let k = decay.clamp(1.0, 20.0);
            if x < a {
                x / a
            } else {
                let u = (x - a) / (1.0 - a);
                let floor = (-k).exp();
                (((-k * u).exp() - floor) / (1.0 - floor)).max(0.0)
            }
        }
    }
}

/// A window shape sampled into a fixed table. Building one does not
/// allocate (the table is inline), so a voice can change shape on the audio
/// thread; it costs 1025 evaluations.
#[derive(Clone, Debug)]
pub struct WindowTable {
    shape: Window,
    t: [f32; TABLE + 1],
    /// Where the window first reaches its top, 0 to 1.
    peak: f32,
}

impl WindowTable {
    pub fn new(shape: Window) -> Self {
        let mut w = WindowTable {
            shape,
            t: [0.0; TABLE + 1],
            peak: 0.5,
        };
        w.fill();
        w
    }

    /// Changes the shape. Does nothing if it is the same.
    pub fn set(&mut self, shape: Window) {
        if shape != self.shape {
            self.shape = shape;
            self.fill();
        }
    }

    fn fill(&mut self) {
        for (i, v) in self.t.iter_mut().enumerate() {
            *v = value(self.shape, i as f32 / TABLE as f32);
        }
        let top = self.t.iter().fold(0.0f32, |a, &b| a.max(b));
        let i = self
            .t
            .iter()
            .position(|&v| v >= top * 0.999)
            .unwrap_or(TABLE / 2);
        self.peak = i as f32 / TABLE as f32;
    }

    /// Where the window first reaches its top (0 to 1): the middle of a
    /// Hann window, the end of a square's fade-in, early in a pluck.
    pub fn peak(&self) -> f32 {
        self.peak
    }

    pub fn shape(&self) -> Window {
        self.shape
    }

    /// The window at `phase` in 0..1, linearly interpolated.
    #[inline]
    pub fn at(&self, phase: f32) -> f32 {
        let p = phase.clamp(0.0, 1.0) * TABLE as f32;
        let i = (p as usize).min(TABLE - 1);
        let f = p - i as f32;
        self.t[i] + (self.t[i + 1] - self.t[i]) * f
    }
}
