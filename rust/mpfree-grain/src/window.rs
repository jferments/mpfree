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
}

impl WindowTable {
    pub fn new(shape: Window) -> Self {
        let mut w = WindowTable {
            shape,
            t: [0.0; TABLE + 1],
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
