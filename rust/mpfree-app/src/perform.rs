//! The Play page's performance effects: the throws (hold to play) and the
//! XY pad, as one [`Node`] on the master strip's first insert slot, before
//! Master safety (so whatever they add is still caught by the limiter).
//!
//! Each throw plays while its parameter is 1 (the screen sends 1 on the
//! finger down and 0 on the lift):
//!
//! - **Delay throw**: the music goes into a 1/8 dotted delay while held;
//!   the echoes ring on after.
//! - **Reverb wash**: the music goes into a long, dark reverb while held;
//!   the tail rings on after (a small comb and all-pass reverb, cheap
//!   enough to run all the time).
//! - **Stutter** and **Beat repeat**: the last 1/16 (or the last beat) as
//!   it was when the finger went down repeats until it lifts.
//! - **Tape stop** (1 beat) and **Vinyl brake** (2 beats): the music slows
//!   to a stop, read back from a 4-second ring of what played; when the
//!   finger lifts the music is back where it is now.
//! - **Filter dip**: a low-pass sweeps down over a beat, and back up when
//!   the finger lifts.
//!
//! The XY pad, while a finger is on it: a low-pass (left to right, 150 Hz
//! to fully open) on the music, and the delay fed at the FX mix with its
//! feedback following the finger up.
//!
//! "Drop drums" is not here: it mutes the drum pads' strips while held.
//!
//! Everything is allocated in [`Node::prepare`]; `process` never
//! allocates, and costs a few multiply-adds a frame while nothing is held.

use mpfree_fx::{Svf, SvfMode};
use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Unit};

pub mod ids {
    pub const DELAY_THROW: u16 = 0;
    pub const REVERB_WASH: u16 = 1;
    pub const STUTTER: u16 = 2;
    pub const TAPE_STOP: u16 = 3;
    pub const FILTER_DIP: u16 = 4;
    pub const BEAT_REPEAT: u16 = 5;
    pub const VINYL_BRAKE: u16 = 6;
    pub const XY_ON: u16 = 7;
    /// 0 to 1, left to right: the low-pass's cutoff.
    pub const XY_X: u16 = 8;
    /// 0 to 1, bottom to top: the delay's feedback.
    pub const XY_Y: u16 = 9;
    /// 0 to 1: how much of the music the XY pad sends into the delay.
    pub const FX_MIX: u16 = 10;
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.app.perform",
    name: "Throws and XY",
    kind: Kind::Effect,
};

const fn toggle(id: u16, name: &'static str) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group: "Throws",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    }
}

const fn level(id: u16, name: &'static str, default: f32) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group: "XY",
        min: 0.0,
        max: 1.0,
        default,
        unit: Unit::None,
        curve: Curve::Linear,
        readonly: false,
    }
}

static PARAMS: [ParamInfo; 11] = [
    toggle(ids::DELAY_THROW, "Delay throw"),
    toggle(ids::REVERB_WASH, "Reverb wash"),
    toggle(ids::STUTTER, "Stutter"),
    toggle(ids::TAPE_STOP, "Tape stop"),
    toggle(ids::FILTER_DIP, "Filter dip"),
    toggle(ids::BEAT_REPEAT, "Beat repeat"),
    toggle(ids::VINYL_BRAKE, "Vinyl brake"),
    toggle(ids::XY_ON, "XY held"),
    level(ids::XY_X, "XY cutoff", 0.7),
    level(ids::XY_Y, "XY feedback", 0.4),
    level(ids::FX_MIX, "FX mix", 0.35),
];

/// Seconds of what played, for the tape stops.
const RING_SECONDS: f32 = 4.0;
/// The longest repeat, seconds (a beat at 30 BPM).
const REPEAT_SECONDS: f32 = 2.0;
/// The delay's longest time, seconds.
const DELAY_SECONDS: f32 = 2.0;
/// Fades between the music and an effect's own sound, seconds.
const FADE_SECONDS: f32 = 0.005;
/// The delay's feedback for a throw.
const THROW_FEEDBACK: f32 = 0.45;
/// The delay and the reverb's level back into the music.
const DELAY_RETURN: f32 = 0.7;
const REVERB_RETURN: f32 = 0.5;
/// The reverb's comb lengths at 44.1 kHz (Freeverb's), the right side
/// a little longer.
const COMBS: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASSES: [usize; 2] = [556, 441];
const SPREAD: usize = 23;
/// About 4 s to fall 60 dB at these lengths.
const COMB_FEEDBACK: f32 = 0.88;
const COMB_DAMP: f32 = 0.35;
/// The filter's range: fully open, and where the dip and the XY pad's
/// left edge go.
const OPEN_HZ: f32 = 18_000.0;
const LOW_HZ: f32 = 150.0;
/// Under this (about -100 dBFS) the delay and the reverb have rung out.
const QUIET: f32 = 1e-5;

/// The XY pad's low-pass at `x` (0 to 1, left to right), Hz.
pub fn xy_cutoff(x: f32) -> f32 {
    LOW_HZ * (OPEN_HZ / LOW_HZ).powf(x.clamp(0.0, 1.0))
}

/// The delay's feedback with the XY pad's finger at `y` (0 to 1, bottom to
/// top).
pub fn xy_feedback(y: f32) -> f32 {
    0.15 + 0.75 * y.clamp(0.0, 1.0)
}

/// A repeat or a stop: which, and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hold {
    None,
    /// Frames in the repeat buffer, and the place in it.
    Repeat {
        len: usize,
        at: usize,
    },
    /// The ring's read place, frames to slow down over, frames gone,
    /// and how sharply (1 for tape, 2 for vinyl).
    Stop {
        read: f64,
        over: f64,
        gone: f64,
        curve: f32,
    },
}

#[derive(Clone, Copy, Debug, Default)]
struct Comb {
    at: usize,
    store: f32,
}

pub struct PerformFx {
    params: [f32; 11],
    rate: u32,
    ring: Vec<f32>,
    ring_at: usize,
    repeat: Vec<f32>,
    hold: Hold,
    /// 0: the music, 1: the hold's own sound.
    fade: f32,
    delay: Vec<f32>,
    delay_at: usize,
    combs: Vec<Vec<f32>>,
    comb_state: [Comb; 8],
    allpasses: Vec<Vec<f32>>,
    ap_at: [usize; 4],
    filter: Svf,
    /// The filter's cutoff now, Hz, moving towards its target each block.
    cutoff: f32,
    /// How far the dip has gone, 0 to 1.
    dip: f32,
    /// Frames the delay and the reverb have been quiet with nothing
    /// feeding them; past the delay's length they have rung out, and both
    /// rest (cost nothing) until a throw or the XY pad feeds them again.
    quiet_frames: usize,
    tails_quiet: bool,
}

impl Default for PerformFx {
    fn default() -> Self {
        PerformFx::new()
    }
}

impl PerformFx {
    pub fn new() -> PerformFx {
        let mut params = [0.0; 11];
        for p in &PARAMS {
            params[p.id as usize] = p.default;
        }
        PerformFx {
            params,
            rate: 44_100,
            ring: Vec::new(),
            ring_at: 0,
            repeat: Vec::new(),
            hold: Hold::None,
            fade: 0.0,
            delay: Vec::new(),
            delay_at: 0,
            combs: Vec::new(),
            comb_state: [Comb::default(); 8],
            allpasses: Vec::new(),
            ap_at: [0; 4],
            filter: Svf::default(),
            cutoff: OPEN_HZ,
            dip: 0.0,
            quiet_frames: 0,
            tails_quiet: true,
        }
    }

    fn on(&self, id: u16) -> bool {
        self.params[id as usize] >= 0.5
    }

    fn ring_frames(&self) -> usize {
        self.ring.len() / 2
    }

    /// A throw's finger went down: a repeat or a stop starts from now.
    fn start(&mut self, id: u16, beat: f64) {
        let rf = self.ring_frames();
        if rf == 0 {
            return;
        }
        match id {
            ids::STUTTER | ids::BEAT_REPEAT => {
                let len = if id == ids::STUTTER { beat / 4.0 } else { beat };
                let len = (len as usize).clamp(16, self.repeat.len() / 2);
                for k in 0..len {
                    let from = (self.ring_at + rf - len + k) % rf;
                    self.repeat[2 * k] = self.ring[2 * from];
                    self.repeat[2 * k + 1] = self.ring[2 * from + 1];
                }
                self.hold = Hold::Repeat { len, at: 0 };
            }
            ids::TAPE_STOP | ids::VINYL_BRAKE => {
                let (beats, curve) = if id == ids::TAPE_STOP {
                    (1.0, 1.0)
                } else {
                    (2.0, 2.0)
                };
                self.hold = Hold::Stop {
                    read: self.ring_at as f64,
                    over: (beat * beats).max(1.0),
                    gone: 0.0,
                    curve,
                };
            }
            _ => {}
        }
    }

    /// Whether any repeat or stop is still held.
    fn holding(&self) -> bool {
        [
            ids::STUTTER,
            ids::BEAT_REPEAT,
            ids::TAPE_STOP,
            ids::VINYL_BRAKE,
        ]
        .iter()
        .any(|&id| self.on(id))
    }

    fn set(&mut self, id: u16, value: f32, beat: f64) {
        let Some(p) = PARAMS.iter().find(|p| p.id == id) else {
            return;
        };
        let v = value.clamp(p.min, p.max);
        let was = self.on(id);
        self.params[id as usize] = v;
        if p.curve == Curve::Toggle && v >= 0.5 && !was {
            self.start(id, beat);
        }
    }

    /// The hold's own sound for one frame.
    fn held_frame(&mut self) -> (f32, f32) {
        let rf = self.ring_frames();
        match &mut self.hold {
            Hold::None => (0.0, 0.0),
            Hold::Repeat { len, at } => {
                let k = *at;
                *at = (*at + 1) % (*len).max(1);
                // A short fade at the seam.
                let edge = (k.min(*len - k) as f32 / 32.0).min(1.0);
                (self.repeat[2 * k] * edge, self.repeat[2 * k + 1] * edge)
            }
            Hold::Stop {
                read,
                over,
                gone,
                curve,
            } => {
                let left = (1.0 - *gone / *over).max(0.0);
                let speed = left.powf(f64::from(*curve));
                *gone += 1.0;
                if speed <= 0.0 {
                    return (0.0, 0.0);
                }
                let i = read.floor();
                let f = (*read - i) as f32;
                let a = (i as usize) % rf;
                let b = (a + 1) % rf;
                *read = (*read + speed) % rf as f64;
                (
                    self.ring[2 * a] * (1.0 - f) + self.ring[2 * b] * f,
                    self.ring[2 * a + 1] * (1.0 - f) + self.ring[2 * b + 1] * f,
                )
            }
        }
    }

    fn reverb(&mut self, input: f32) -> (f32, f32) {
        let mut out = [0f32; 2];
        for (ch, o) in out.iter_mut().enumerate() {
            let mut sum = 0.0;
            for c in 0..4 {
                let k = ch * 4 + c;
                let line = &mut self.combs[k];
                let st = &mut self.comb_state[k];
                let y = line[st.at];
                st.store = y * (1.0 - COMB_DAMP) + st.store * COMB_DAMP;
                line[st.at] = input + st.store * COMB_FEEDBACK;
                st.at = (st.at + 1) % line.len();
                sum += y;
            }
            let mut v = sum * 0.25;
            for a in 0..2 {
                let k = ch * 2 + a;
                let line = &mut self.allpasses[k];
                let at = self.ap_at[k];
                let b = line[at];
                line[at] = v + b * 0.5;
                v = b - v;
                self.ap_at[k] = (at + 1) % line.len();
            }
            *o = v;
        }
        (out[0], out[1])
    }
}

impl Node for PerformFx {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        self.set(id, value, f64::from(self.rate) / 2.0);
    }

    fn param(&self, id: u16) -> f32 {
        self.params.get(id as usize).copied().unwrap_or(f32::NAN)
    }

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate;
        let r = rate as f32;
        self.ring = vec![0.0; 2 * (r * RING_SECONDS) as usize];
        self.ring_at = 0;
        self.repeat = vec![0.0; 2 * (r * REPEAT_SECONDS) as usize];
        self.delay = vec![0.0; 2 * (r * DELAY_SECONDS) as usize];
        self.delay_at = 0;
        let scale = r / 44_100.0;
        self.combs = (0..8)
            .map(|k| {
                let n = COMBS[k % 4] + if k >= 4 { SPREAD } else { 0 };
                vec![0.0; ((n as f32 * scale) as usize).max(1)]
            })
            .collect();
        self.allpasses = (0..4)
            .map(|k| {
                let n = ALLPASSES[k % 2] + if k >= 2 { SPREAD } else { 0 };
                vec![0.0; ((n as f32 * scale) as usize).max(1)]
            })
            .collect();
        self.reset();
    }

    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.repeat.fill(0.0);
        self.delay.fill(0.0);
        for c in self.combs.iter_mut().chain(self.allpasses.iter_mut()) {
            c.fill(0.0);
        }
        self.comb_state = [Comb::default(); 8];
        self.ap_at = [0; 4];
        self.hold = Hold::None;
        self.fade = 0.0;
        self.filter.reset();
        self.cutoff = OPEN_HZ;
        self.dip = 0.0;
        self.quiet_frames = 0;
        self.tails_quiet = true;
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        if self.ring.is_empty() || n == 0 {
            return;
        }
        let beat = f64::from(self.rate) * 60.0 / ctx.bpm.max(20.0);
        for e in events {
            if let NodeEvent::Param { id, value } = e.ev {
                self.set(id, value, beat);
            }
        }
        if !self.holding() && self.fade <= 0.0 {
            self.hold = Hold::None;
        }
        // The filter: the XY pad's place, else the dip, else open.
        let block = n as f32 / self.rate as f32;
        let dip_target = if self.on(ids::FILTER_DIP) { 1.0 } else { 0.0 };
        let dip_rate =
            block / (beat as f32 / self.rate as f32) * if dip_target > 0.0 { 1.0 } else { 4.0 };
        self.dip += (dip_target - self.dip).clamp(-dip_rate, dip_rate);
        let xy = self.on(ids::XY_ON);
        let target = if xy {
            xy_cutoff(self.params[ids::XY_X as usize])
        } else {
            OPEN_HZ * (LOW_HZ / OPEN_HZ).powf(self.dip.powf(0.7))
        };
        // Glide a fifth of the way each block (about 15 ms).
        self.cutoff *= (target / self.cutoff).powf(0.2);
        let filtering = self.cutoff < OPEN_HZ * 0.95;
        if filtering {
            self.filter
                .set(SvfMode::LowPass, self.cutoff, 0.9, 0.0, self.rate);
        }
        let fb = if xy {
            xy_feedback(self.params[ids::XY_Y as usize])
        } else {
            THROW_FEEDBACK
        };
        let send = if xy {
            self.params[ids::FX_MIX as usize]
        } else {
            0.0
        } + if self.on(ids::DELAY_THROW) { 1.0 } else { 0.0 };
        let wash = if self.on(ids::REVERB_WASH) { 0.35 } else { 0.0 };
        let delay_frames = ((beat * 0.75) as usize).clamp(1, self.delay.len() / 2 - 1);
        let df = self.delay.len() / 2;
        let rf = self.ring_frames();
        let fade_step = 1.0 / (FADE_SECONDS * self.rate as f32);
        let held = self.holding();
        let feeding = send > 0.0 || wash > 0.0;
        if feeding {
            self.tails_quiet = false;
            self.quiet_frames = 0;
        }
        let tails = !self.tails_quiet;
        let mut loudest = 0f32;
        for i in 0..n {
            let (l, r) = (left[i], right[i]);
            self.ring[2 * self.ring_at] = l;
            self.ring[2 * self.ring_at + 1] = r;
            self.ring_at = (self.ring_at + 1) % rf;
            // A repeat or a stop in place of the music, faded in and out.
            self.fade = if held {
                (self.fade + fade_step).min(1.0)
            } else {
                (self.fade - fade_step).max(0.0)
            };
            let (mut ml, mut mr) = (l, r);
            if self.fade > 0.0 {
                let (hl, hr) = self.held_frame();
                ml = l * (1.0 - self.fade) + hl * self.fade;
                mr = r * (1.0 - self.fade) + hr * self.fade;
            }
            if filtering {
                ml = self.filter.run(0, ml);
                mr = self.filter.run(1, mr);
            }
            if !tails {
                left[i] = ml;
                right[i] = mr;
                continue;
            }
            // The delay: fed by a throw or the XY pad, ringing on after.
            let rd = (self.delay_at + df - delay_frames) % df;
            let (dl, dr) = (self.delay[2 * rd], self.delay[2 * rd + 1]);
            self.delay[2 * self.delay_at] = ml * send + dr * fb;
            self.delay[2 * self.delay_at + 1] = mr * send + dl * fb;
            self.delay_at = (self.delay_at + 1) % df;
            // The reverb, fed while the wash is held.
            let (wl, wr) = self.reverb((ml + mr) * wash);
            loudest = loudest.max(dl.abs().max(dr.abs()).max(wl.abs()).max(wr.abs()));
            left[i] = ml + DELAY_RETURN * dl + REVERB_RETURN * wl;
            right[i] = mr + DELAY_RETURN * dr + REVERB_RETURN * wr;
        }
        if tails && !feeding {
            // Quiet for the delay line's whole length: everything in it,
            // and in the reverb's shorter lines, is quiet too.
            self.quiet_frames = if loudest < QUIET {
                self.quiet_frames + n
            } else {
                0
            };
            if self.quiet_frames >= df {
                self.tails_quiet = true;
            }
        }
    }

    fn tail_frames(&self) -> u32 {
        self.rate * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;
    const BLOCK: usize = 128;

    fn ctx() -> Ctx<'static> {
        let mut c = Ctx::new(RATE, 0);
        c.bpm = 120.0;
        c.playing = true;
        c
    }

    /// A click every half second (each beat at 120 BPM).
    fn clicks(at: usize) -> f32 {
        if at.is_multiple_of(22_050) {
            1.0
        } else {
            0.0
        }
    }

    fn run(fx: &mut PerformFx, from: usize, blocks: usize, ev: &[Timed]) -> Vec<f32> {
        let mut out = Vec::new();
        for b in 0..blocks {
            let mut l: Vec<f32> = (0..BLOCK).map(|i| clicks(from + b * BLOCK + i)).collect();
            let mut r = l.clone();
            fx.process(&ctx(), if b == 0 { ev } else { &[] }, &mut l, &mut r);
            out.extend(l);
        }
        out
    }

    fn param(id: u16, value: f32) -> Timed {
        Timed::new(0, NodeEvent::Param { id, value })
    }

    #[test]
    fn nothing_held_passes_the_music_as_it_is() {
        let mut fx = PerformFx::new();
        fx.prepare(RATE, BLOCK);
        let out = run(&mut fx, 0, 400, &[]);
        for (i, v) in out.iter().enumerate() {
            assert!((v - clicks(i)).abs() < 1e-6, "frame {i}: {v}");
        }
    }

    #[test]
    fn a_delay_throw_rings_on_after_the_lift() {
        let mut fx = PerformFx::new();
        fx.prepare(RATE, BLOCK);
        let n = 22_050 / BLOCK + 1;
        run(&mut fx, 0, n, &[param(ids::DELAY_THROW, 1.0)]);
        // Lifted: the next click's echo 1/8 dotted (0.375 s) later.
        let from = n * BLOCK;
        let out = run(&mut fx, from, 400, &[param(ids::DELAY_THROW, 0.0)]);
        let echo = 22_050 + 16_537 - from;
        let around = &out[echo.saturating_sub(4)..echo + 4];
        assert!(
            around.iter().any(|v| v.abs() > 0.2),
            "an echo of the click at frame 0: {around:?}"
        );
    }

    #[test]
    fn the_delay_and_reverb_rest_once_they_ring_out_and_wake_for_a_throw() {
        let mut fx = PerformFx::new();
        fx.prepare(RATE, BLOCK);
        assert!(fx.tails_quiet, "nothing has fed them yet");
        run(&mut fx, 0, 4, &[param(ids::DELAY_THROW, 1.0)]);
        assert!(!fx.tails_quiet);
        // Let go: the echoes fall away (feedback 0.45 a repeat), and once
        // the line has been quiet for its whole length the tails rest.
        let mut at = 4 * BLOCK;
        run(&mut fx, at, 1, &[param(ids::DELAY_THROW, 0.0)]);
        at += BLOCK;
        let mut blocks = 0;
        while !fx.tails_quiet && blocks < 20_000 {
            let mut l = [0f32; BLOCK];
            let mut r = [0f32; BLOCK];
            fx.process(&ctx(), &[], &mut l, &mut r);
            blocks += 1;
        }
        assert!(fx.tails_quiet, "rang out within {blocks} blocks");
        at += blocks * BLOCK;
        // Resting, the music passes as it is.
        let out = run(&mut fx, at, 50, &[]);
        for (i, v) in out.iter().enumerate() {
            assert!((v - clicks(at + i)).abs() < 1e-6, "frame {i}: {v}");
        }
        // The next throw wakes them.
        run(&mut fx, at + 50 * BLOCK, 1, &[param(ids::REVERB_WASH, 1.0)]);
        assert!(!fx.tails_quiet);
    }

    #[test]
    fn a_stutter_repeats_the_last_sixteenth() {
        let mut fx = PerformFx::new();
        fx.prepare(RATE, BLOCK);
        // Just after a click, the last 1/16 (5512 frames) holds it.
        let start = 22_050 + 1_000;
        let before = start / BLOCK;
        run(&mut fx, 0, before, &[]);
        let out = run(&mut fx, before * BLOCK, 200, &[param(ids::STUTTER, 1.0)]);
        let hits: Vec<usize> = out
            .iter()
            .enumerate()
            .filter(|(_, v)| v.abs() > 0.5)
            .map(|(i, _)| i)
            .collect();
        assert!(hits.len() >= 3, "{hits:?}");
        let gaps: Vec<usize> = hits.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|&g| g == 5_512), "{gaps:?}");
    }

    #[test]
    fn a_tape_stop_goes_silent_and_the_music_comes_back() {
        let mut fx = PerformFx::new();
        fx.prepare(RATE, BLOCK);
        run(&mut fx, 0, 100, &[]);
        let out = run(&mut fx, 100 * BLOCK, 400, &[param(ids::TAPE_STOP, 1.0)]);
        // After a beat (22050 frames) the tape has stopped.
        assert!(out[23_000..].iter().all(|v| v.abs() < 1e-6));
        let back = run(&mut fx, 500 * BLOCK, 400, &[param(ids::TAPE_STOP, 0.0)]);
        let first = (500 * BLOCK).div_ceil(22_050) * 22_050 - 500 * BLOCK;
        assert!(back[first] > 0.9, "the clicks again: {}", back[first]);
    }
}
