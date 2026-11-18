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
//!
//! Repeat ([`GrainFx::start_repeat`], R7 step C2) is a synced stutter: from
//! the frame it starts on, the last stretch of the input (a beat, an
//! eighth) plays again and again, once a stretch, in place of the input
//! and the cloud. It has a grain voice of its own reading that slice, so
//! the output crossfades from the input and the cloud to the repeat over
//! 10 ms and back when it stops ([`GrainFx::stop_repeat`]), and the cloud
//! starts afresh. Each repeat is one grain a little longer than the
//! stretch, so one repeat crossfades into the next over a few ms
//! ([`REPEAT_FADE_S`]) without a click; Pitch and Reverse play through it.
//! Starting again while it repeats, with another length, re-cuts the same
//! slice from its start: going from quarters to eighths to sixteenths
//! rolls the start of the beat, as a beat repeat does. The buffer keeps
//! taking the input meanwhile (feedback rests, as the cloud is silent),
//! unless that would overwrite the slice: then it holds, as when frozen,
//! until the repeat ends.

use crate::buffer::{GrainSource, LiveBuffer};
use crate::voice::{Budget, GrainView, GrainVoice, Params, Stats, TempoSync};
use crate::window::Window;

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
    /// The repeat's own voice and its output for a chunk.
    rvoice: GrainVoice,
    rep_l: Vec<f32>,
    rep_r: Vec<f32>,
    /// The repeat while it plays or fades out.
    rep: Option<Repeat>,
    /// How far the output has gone over to the repeat (0 the input and the
    /// cloud, 1 the repeat): now, where it glides to, its step and the
    /// frames left of the glide.
    rg: f32,
    rg_to: f32,
    rg_step: f32,
    rg_left: u32,
    /// The buffer holds still because new audio would overwrite the slice.
    held: bool,
    /// The cloud rests: only the repeat is heard.
    resting: bool,
    /// Output frames per beat, for the repeat's length.
    fpb: f64,
    rate: u32,
}

/// The repeat's slice and clock.
#[derive(Clone, Copy, Debug)]
struct Repeat {
    /// The slice of the buffer it plays, in absolute frames.
    start: i64,
    end: i64,
    /// Frames since it started: its grid counts from here.
    clock: u64,
}

/// Each repeat runs on into the next by this long (or a 99th of a repeat,
/// if that is longer), fading out as the next fades in, so the joins don't
/// click.
pub const REPEAT_FADE_S: f64 = 0.003;

/// The repeat's slice of the buffer: the buffer's audio, with only
/// `start..end` open to new grains.
struct Slice<'a> {
    buf: &'a LiveBuffer,
    start: i64,
    end: i64,
}

impl GrainSource for Slice<'_> {
    fn rate(&self) -> u32 {
        self.buf.rate()
    }
    fn channels(&self) -> usize {
        self.buf.channels()
    }
    fn span(&self) -> (i64, i64) {
        (self.start, self.end)
    }
    #[inline]
    fn sample(&self, ch: usize, frame: i64) -> f32 {
        self.buf.sample(ch, frame)
    }
    #[inline(always)]
    fn read(&self, frame: i64, t: f32) -> [f32; 2] {
        self.buf.read(frame, t)
    }
}

/// The repeat voice's settings: one grain a repeat, on the beat grid from
/// the repeat's start, each `beats` long plus the crossfade `fade` (output
/// frames) into the next, from the slice's start; the effect's pitch,
/// reverse and level, and nothing random.
fn repeat_params(p: &Params, beats: f64, fpb: f64, fade: f64) -> Params {
    let len = beats * fpb + fade;
    Params {
        position: 0.0,
        pitch: p.pitch,
        scale: p.scale,
        reverse: p.reverse,
        gain: p.gain,
        timing: 0.0,
        window: Window::Trapezoid {
            ramp: (fade / len) as f32,
        },
        sync: Some(TempoSync {
            frames_per_beat: fpb,
            grains_per_beat: Some((1.0 / beats) as f32),
            size_beats: Some((len / fpb) as f32),
        }),
        ..Params::default()
    }
}

/// The repeat voice's random sequence, apart from the cloud's.
const REPEAT_SEED: u64 = 0x7265_7065_6174;

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
            rvoice: GrainVoice::new(rate, seed ^ REPEAT_SEED),
            rep_l: vec![0.0; Self::CHUNK],
            rep_r: vec![0.0; Self::CHUNK],
            rep: None,
            rg: 0.0,
            rg_to: 0.0,
            rg_step: 0.0,
            rg_left: 0,
            held: false,
            resting: false,
            fpb: 0.5 * f64::from(rate), // 120 BPM
            rate,
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
        self.buffer.set_frozen(on || self.held);
    }

    /// The tempo the repeat follows, as output frames per beat (60 × rate
    /// / BPM). Ignored unless finite and at least one frame.
    pub fn set_tempo(&mut self, frames_per_beat: f64) {
        if frames_per_beat.is_finite() && frames_per_beat >= 1.0 {
            self.fpb = frames_per_beat;
            self.rvoice.set_tempo(frames_per_beat);
        }
    }

    /// Starts repeating from the next frame: the last `beats` beats of the
    /// input (at the tempo set), again every `beats` beats, in place of the
    /// input and the cloud (a 10 ms crossfade). While it already repeats,
    /// re-cuts its slice from the same start to the new length, so a
    /// shorter length rolls the start of the slice. Ignored unless `beats`
    /// is finite and the repeat is at least 1 ms.
    pub fn start_repeat(&mut self, beats: f64) {
        let len = beats * self.fpb;
        let rate = f64::from(self.rate);
        if !len.is_finite() || len < 0.001 * rate {
            return;
        }
        let fade = (REPEAT_FADE_S * rate).max(len / 99.0).ceil();
        let whole = (len + fade).ceil() as i64;
        // The slice reaches the frame it starts on.
        self.flush();
        let now = self.buffer.written() as i64;
        let (start, end) = match self.rep {
            Some(r) if self.rg_to > 0.0 => (r.start, (r.start + whole).min(now)),
            _ => (now - whole, now),
        };
        self.rep = Some(Repeat {
            start,
            end,
            clock: 0,
        });
        let p = repeat_params(self.voice.params(), beats, self.fpb, fade);
        self.rvoice.set_params(p);
        self.rvoice.set_tempo(self.fpb);
        self.rvoice.start();
        self.glide_repeat(1.0);
    }

    /// Stops repeating: the output crossfades back to the input and the
    /// cloud over 10 ms, and the cloud starts again.
    pub fn stop_repeat(&mut self) {
        if self.rep.is_some() {
            self.glide_repeat(0.0);
        }
    }

    /// True while the repeat plays (not while it fades out).
    pub fn repeating(&self) -> bool {
        self.rep.is_some() && self.rg_to > 0.0
    }

    fn glide_repeat(&mut self, to: f32) {
        self.rg_to = to;
        self.rg_step = (to - self.rg) / self.ramp_frames as f32;
        self.rg_left = if self.rg == to { 0 } else { self.ramp_frames };
    }

    /// Silences the grains at once and hands them back to the budget (call
    /// it before dropping or replacing the effect while it plays). A
    /// repeat ends.
    pub fn kill(&mut self, budget: &mut Budget) {
        self.voice.kill(budget);
        self.running = false;
        self.end_repeat(budget);
    }

    fn end_repeat(&mut self, budget: &mut Budget) {
        self.rvoice.kill(budget);
        self.rep = None;
        (self.rg, self.rg_to, self.rg_left) = (0.0, 0.0, 0);
        self.resting = false;
        if self.held {
            self.held = false;
            self.buffer.set_frozen(self.frozen);
        }
    }

    /// Silences the grains, forgets the audio it holds and finishes any mix
    /// glide at once; the settings, the mix and freeze stay, and a repeat
    /// ends. Doesn't allocate.
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
    /// Grains playing now, the cloud's and the repeat's.
    pub fn live(&self) -> usize {
        self.voice.stats().active + self.rvoice.stats().active
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
            // The cloud rests while only the repeat is heard, from the first
            // chunk boundary there (so at the same frame whatever the
            // period size).
            if self.rg_to < 1.0 {
                self.resting = false;
            } else if self.rg >= 1.0 && self.frame.is_multiple_of(chunk) {
                self.resting = true;
            }
            let wants = (self.wet_to > 0.0 || self.ramp_left > 0) && !self.resting;
            if wants && !self.running {
                self.voice.start();
                self.running = true;
            } else if !wants && self.running {
                self.voice.kill(budget);
                self.running = false;
            }
            let (wl, wr) = (&mut self.wet_l[..k], &mut self.wet_r[..k]);
            wl.fill(0.0);
            wr.fill(0.0);
            if self.running {
                self.voice
                    .render(&self.buffer, budget, transport_frame + at as u64, wl, wr);
            }
            if let Some(r) = &mut self.rep {
                // Hold the buffer before new audio reaches the slice.
                let reach = self.buffer.written() as i64 + 2 * chunk as i64;
                if !self.held && reach >= r.start + self.buffer.capacity() as i64 {
                    self.held = true;
                    self.buffer.set_frozen(true);
                }
                let (rl, rr) = (&mut self.rep_l[..k], &mut self.rep_r[..k]);
                rl.fill(0.0);
                rr.fill(0.0);
                let slice = Slice {
                    buf: &self.buffer,
                    start: r.start,
                    end: r.end,
                };
                self.rvoice.render(&slice, budget, r.clock, rl, rr);
                r.clock += k as u64;
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
                if self.rg_left > 0 {
                    self.rg_left -= 1;
                    self.rg = if self.rg_left == 0 {
                        self.rg_to
                    } else {
                        self.rg + self.rg_step
                    };
                }
                if !self.frozen && !self.held {
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
                if self.rep.is_some() {
                    let (g, a) = (self.rg, 1.0 - self.rg);
                    left[at + i] =
                        a * (x * self.dry + self.wet_l[i] * self.wet) + g * self.rep_l[i];
                    right[at + i] =
                        a * (y * self.dry + self.wet_r[i] * self.wet) + g * self.rep_r[i];
                } else {
                    left[at + i] = x * self.dry + self.wet_l[i] * self.wet;
                    right[at + i] = y * self.dry + self.wet_r[i] * self.wet;
                }
            }
            self.frame += k as u64;
            at += k;
            let edge = self.frame.is_multiple_of(chunk);
            if edge && self.rep.is_some() && self.rg_to <= 0.0 && self.rg <= 0.0 {
                // Faded back: the repeat is over.
                self.end_repeat(budget);
            }
            if self.frame.is_multiple_of(chunk) {
                self.flush();
            }
        }
    }
}
