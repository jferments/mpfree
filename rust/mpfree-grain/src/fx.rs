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
//!
//! Feedback ([`GrainFx::set_feedback`]) writes the grains back into the
//! buffer with the input, so they play again in later grains: trails that
//! build, and at full an endless, self-renewing cloud. In the loop the
//! grains pass a high-pass (40 Hz, so nothing builds up at the bottom) and
//! a soft saturation, straight up to a quarter of full scale and bending
//! to at most half, so the loop can't run away: what goes back is never
//! more than half of full scale on top of the input. Loud trails grit up,
//! on purpose (the granular plan's decision 4: grit in the engine).

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
    /// Feedback: the gain now, where it glides to, its step and the frames
    /// left of the glide.
    fb: f32,
    fb_to: f32,
    fb_step: f32,
    fb_left: u32,
    /// The loop's high-pass: its coefficient, and each channel's last input
    /// and output.
    hp_a: f32,
    hp_x: [f32; 2],
    hp_y: [f32; 2],
}

/// Feedback's high-pass corner, Hz.
const FEEDBACK_HP_HZ: f32 = 40.0;

/// Where the feedback loop's saturation starts to bend, and the most it
/// lets back.
const KNEE: f32 = 0.25;
const CEILING: f32 = 0.5;

/// The feedback loop's soft saturation: straight up to [`KNEE`], then
/// bending smoothly towards [`CEILING`], never past it.
#[inline]
fn saturate(x: f32) -> f32 {
    let a = x.abs();
    if a <= KNEE {
        return x;
    }
    // tanh's rational fit (exact at 0, within 2 % below 3, 1 past it).
    let y = ((a - KNEE) / (CEILING - KNEE)).min(3.0);
    let t = y * (27.0 + y * y) / (27.0 + 9.0 * y * y);
    (KNEE + (CEILING - KNEE) * t).copysign(x)
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
            fb: 0.0,
            fb_to: 0.0,
            fb_step: 0.0,
            fb_left: 0,
            hp_a: (-std::f32::consts::TAU * FEEDBACK_HP_HZ / rate as f32).exp(),
            hp_x: [0.0; 2],
            hp_y: [0.0; 2],
        })
    }

    /// The grain settings. `position` 0 is the oldest audio the buffer
    /// holds and 1 the newest.
    pub fn params(&self) -> &Params {
        self.voice.params()
    }

    /// Sets the grain settings. A `position` of 1 or more is held just
    /// below 1: a grain voice's position wraps round, and here 1 should
    /// mean the newest audio, not the oldest.
    pub fn set_params(&mut self, mut p: Params) {
        if p.position >= 1.0 {
            p.position = 0.999_99;
        }
        self.voice.set_params(p);
    }

    pub fn mix(&self) -> f32 {
        self.mix
    }

    /// From 0 (only the input) to 1 (only grains), equal power. Glides
    /// there over 10 ms. At 0 the output is the input exactly and no grains
    /// are made.
    pub fn set_mix(&mut self, mix: f32) {
        let mix = if mix.is_finite() {
            mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if mix == self.mix {
            return;
        }
        self.mix = mix;
        (self.dry_to, self.wet_to) = gains(mix);
        let n = self.ramp_frames as f32;
        self.dry_step = (self.dry_to - self.dry) / n;
        self.wet_step = (self.wet_to - self.wet) / n;
        // Already there (a change undone before any frame ran): no glide.
        let there = self.dry == self.dry_to && self.wet == self.wet_to;
        self.ramp_left = if there { 0 } else { self.ramp_frames };
    }

    pub fn frozen(&self) -> bool {
        self.frozen
    }

    pub fn feedback(&self) -> f32 {
        self.fb_to
    }

    /// How much of the grains goes back into the buffer, 0 (none: the
    /// buffer holds the input exactly) to 1 (an endless cloud). Glides
    /// there over 10 ms.
    pub fn set_feedback(&mut self, feedback: f32) {
        let f = if feedback.is_finite() {
            feedback.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if f == self.fb_to {
            return;
        }
        self.fb_to = f;
        self.fb_step = (f - self.fb) / self.ramp_frames as f32;
        self.fb_left = if self.fb == f { 0 } else { self.ramp_frames };
    }

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

    /// Silences the grains, forgets the audio it holds and finishes any mix
    /// glide at once; the settings, the mix and freeze stay. Doesn't allocate.
    pub fn reset(&mut self, budget: &mut Budget) {
        self.kill(budget);
        self.buffer.clear();
        self.staged = 0;
        (self.dry, self.wet) = (self.dry_to, self.wet_to);
        self.ramp_left = 0;
        (self.fb, self.fb_left) = (self.fb_to, 0);
        (self.hp_x, self.hp_y) = ([0.0; 2], [0.0; 2]);
    }

    /// The buffer the grains read, for drawing it.
    pub fn buffer(&self) -> &LiveBuffer {
        &self.buffer
    }
    pub fn stats(&self) -> Stats {
        self.voice.stats()
    }
    /// The grains playing now, for drawing them; returns how many.
    pub fn grains(&self, out: &mut [GrainView]) -> usize {
        self.voice.grains(out)
    }
    /// Where grains read (before spray), 0 (oldest) to 1 (newest).
    pub fn playhead(&self) -> f32 {
        self.voice.playhead()
    }

    fn flush(&mut self) {
        if self.staged > 0 {
            self.buffer.write(&self.stage[..self.staged * 2]);
            self.staged = 0;
        }
    }

    /// Runs the effect on `left` and `right` in place. `transport_frame`
    /// is the absolute frame of `left[0]` (for grains synced to the tempo);
    /// `budget` is the grain budget shared with every other grain voice.
    pub fn process(
        &mut self,
        budget: &mut Budget,
        transport_frame: u64,
        left: &mut [f32],
        right: &mut [f32],
    ) {
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
                self.voice
                    .render(&self.buffer, budget, transport_frame + at as u64, wl, wr);
            }
            for i in 0..k {
                let (x, y) = (left[at + i], right[at + i]);
                if self.fb_left > 0 {
                    self.fb_left -= 1;
                    self.fb = if self.fb_left == 0 {
                        self.fb_to
                    } else {
                        self.fb + self.fb_step
                    };
                }
                if !self.frozen {
                    let (mut bx, mut by) = (x, y);
                    if self.fb > 0.0 {
                        let w = [self.wet_l[i], self.wet_r[i]];
                        for c in 0..2 {
                            // y[n] = a (y[n-1] + x[n] - x[n-1])
                            self.hp_y[c] = self.hp_a * (self.hp_y[c] + w[c] - self.hp_x[c]);
                            self.hp_x[c] = w[c];
                        }
                        bx += self.fb * saturate(self.hp_y[0]);
                        by += self.fb * saturate(self.hp_y[1]);
                    }
                    self.stage[2 * self.staged] = bx;
                    self.stage[2 * self.staged + 1] = by;
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
            if self.frame.is_multiple_of(chunk) {
                self.flush();
            }
        }
    }
}
