//! The granular effect: grains from the last few seconds of a live signal,
//! mixed back over it, with freeze (the Clouds and S-4 Mosaic idea).
//!
//! [`GrainFx`] is an insert on a stereo signal. Each period it keeps the
//! incoming audio in a [`LiveBuffer`], plays a [`GrainVoice`] from that
//! buffer and mixes the grains back in. Freeze stops the buffer taking new
//! audio, so the cloud keeps playing what was there while the dry signal
//! carries on.
//!
//! Real time: everything is allocated in [`GrainFx::new`]; `process`,
//! `set_params`, `set_mix`, `set_frozen` and `kill` never allocate, free,
//! lock or block.
//!
//! The output doesn't depend on how the audio is cut into periods. Incoming
//! audio reaches the buffer in fixed chunks of [`GrainFx::CHUNK`] frames,
//! counted from the effect's first frame, so a grain sees the same buffer
//! whatever the period size. That delays new audio by at most `CHUNK`
//! frames (1.5 ms at 44.1 kHz) before it can be granulated. Freeze is the
//! exception to the chunking: it takes in whatever is waiting first, so the
//! cloud holds everything up to the frame it was pressed on.

use crate::buffer::LiveBuffer;
use crate::voice::{Budget, GrainView, GrainVoice, Params, Stats};

/// A granular insert on a stereo signal.
#[derive(Clone, Debug)]
pub struct GrainFx {
    buffer: LiveBuffer,
    voice: GrainVoice,
    /// True while the voice is making grains (the mix is above zero).
    running: bool,
    /// Audio waiting for the next chunk boundary, interleaved.
    stage: Vec<f32>,
    staged: usize,
    wet_l: Vec<f32>,
    wet_r: Vec<f32>,
    /// Frames processed since the effect was made.
    frame: u64,
    mix: f32,
    dry: f32,
    wet: f32,
    dry_to: f32,
    wet_to: f32,
    dry_step: f32,
    wet_step: f32,
    ramp_left: u32,
    ramp_frames: u32,
    frozen: bool,
}

/// Dry and wet gains for a mix from 0 (dry) to 1 (wet), equal power, exact
/// at both ends.
fn gains(mix: f32) -> (f32, f32) {
    if mix <= 0.0 {
        (1.0, 0.0)
    } else if mix >= 1.0 {
        (0.0, 1.0)
    } else {
        let a = mix * std::f32::consts::FRAC_PI_2;
        (a.cos(), a.sin())
    }
}

impl GrainFx {
    /// Incoming audio reaches the buffer in chunks of this many frames.
    pub const CHUNK: usize = 64;

    /// A stereo effect at `rate` with `seconds` of memory (0.5 to 30),
    /// fully dry, with the grain voice's default settings. Allocates
    /// everything now (call it off the audio thread). `None` if `rate` is
    /// zero or `seconds` is out of range.
    pub fn new(rate: u32, seconds: f32, seed: u64) -> Option<GrainFx> {
        if rate == 0 || !(0.5..=30.0).contains(&seconds) {
            return None;
        }
        let buffer = LiveBuffer::new(2, rate, (seconds * rate as f32) as usize)?;
        Some(GrainFx {
            buffer,
            voice: GrainVoice::new(rate, seed),
            running: false,
            stage: vec![0.0; Self::CHUNK * 2],
            staged: 0,
            wet_l: vec![0.0; Self::CHUNK],
            wet_r: vec![0.0; Self::CHUNK],
            frame: 0,
            mix: 0.0,
            dry: 1.0,
            wet: 0.0,
            dry_to: 1.0,
            wet_to: 0.0,
            dry_step: 0.0,
            wet_step: 0.0,
            ramp_left: 0,
            ramp_frames: (rate / 100).max(1), // 10 ms
            frozen: false,
        })
    }

    /// The grain settings. `position` 0 is the oldest audio the buffer
    /// holds and 1 the newest.
    pub fn params(&self) -> &Params { self.voice.params() }

    /// Sets the grain settings. A `position` of 1 or more is held just
    /// below 1: a grain voice's position wraps round, and here 1 should
    /// mean the newest audio, not the oldest.
    pub fn set_params(&mut self, mut p: Params) {
        if p.position >= 1.0 {
            p.position = 0.999_99;
        }
        self.voice.set_params(p);
    }

    pub fn mix(&self) -> f32 { self.mix }

    /// From 0 (only the input) to 1 (only grains), equal power. Glides
    /// there over 10 ms. At 0 the output is the input exactly and no grains
    /// are made.
    pub fn set_mix(&mut self, mix: f32) {
        let mix = if mix.is_finite() { mix.clamp(0.0, 1.0) } else { 0.0 };
        if mix == self.mix {
            return;
        }
        self.mix = mix;
        (self.dry_to, self.wet_to) = gains(mix);
        let n = self.ramp_frames as f32;
        self.dry_step = (self.dry_to - self.dry) / n;
        self.wet_step = (self.wet_to - self.wet) / n;
        self.ramp_left = self.ramp_frames;
    }

    pub fn frozen(&self) -> bool { self.frozen }

    /// Freeze on: the buffer keeps what it holds now (including audio
    /// waiting for the next chunk), and grains keep playing from it.
    /// Freeze off: new audio comes in again.
    pub fn set_frozen(&mut self, on: bool) {
        if on == self.frozen {
            return;
        }
        if on {
            self.flush();
        }
        self.frozen = on;
        self.buffer.set_frozen(on);
    }

    /// Silences the grains at once and hands them back to the budget (call
    /// it before dropping or replacing the effect while it plays).
    pub fn kill(&mut self, budget: &mut Budget) {
        self.voice.kill(budget);
        self.running = false;
    }

    /// The buffer the grains read, for drawing it.
    pub fn buffer(&self) -> &LiveBuffer { &self.buffer }
    pub fn stats(&self) -> Stats { self.voice.stats() }
    /// The grains playing now, for drawing them; returns how many.
    pub fn grains(&self, out: &mut [GrainView]) -> usize { self.voice.grains(out) }
    /// Where grains read (before spray), 0 (oldest) to 1 (newest).
    pub fn playhead(&self) -> f32 { self.voice.playhead() }

    fn flush(&mut self) {
        if self.staged > 0 {
            self.buffer.write(&self.stage[..self.staged * 2]);
            self.staged = 0;
        }
    }

    /// Runs the effect on `left` and `right` in place. `transport_frame`
    /// is the absolute frame of `left[0]` (for grains synced to the tempo);
    /// `budget` is the grain budget shared with every other grain voice.
    pub fn process(&mut self, budget: &mut Budget, transport_frame: u64, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let chunk = Self::CHUNK as u64;
        let mut at = 0;
        while at < n {
            let k = (Self::CHUNK - (self.frame % chunk) as usize).min(n - at);
            let wants = self.wet_to > 0.0 || self.ramp_left > 0;
            if wants && !self.running {
                self.voice.start();
                self.running = true;
            } else if !wants && self.running {
                self.kill(budget);
            }
            let (wl, wr) = (&mut self.wet_l[..k], &mut self.wet_r[..k]);
            wl.fill(0.0);
            wr.fill(0.0);
            if self.running {
                self.voice.render(&self.buffer, budget, transport_frame + at as u64, wl, wr);
            }
            for i in 0..k {
                let (x, y) = (left[at + i], right[at + i]);
                if !self.frozen {
                    self.stage[2 * self.staged] = x;
                    self.stage[2 * self.staged + 1] = y;
                    self.staged += 1;
                }
                if self.ramp_left > 0 {
                    self.ramp_left -= 1;
                    if self.ramp_left == 0 {
                        (self.dry, self.wet) = (self.dry_to, self.wet_to);
                    } else {
                        self.dry += self.dry_step;
                        self.wet += self.wet_step;
                    }
                }
                left[at + i] = x * self.dry + self.wet_l[i] * self.wet;
                right[at + i] = y * self.dry + self.wet_r[i] * self.wet;
            }
            self.frame += k as u64;
            at += k;
            if self.frame % chunk == 0 {
                self.flush();
            }
        }
    }
}
