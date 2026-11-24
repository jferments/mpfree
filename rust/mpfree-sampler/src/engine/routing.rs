//! Where the sound goes: one mixed pair ([`Engine::render`]) or a pair for
//! each pad ([`PadOut`]), and the voices split over two cores
//! ([`Engine::split`]).

use super::*;

/// Split ([`Engine::split`]), the lane's voices render at most this many
/// frames at a time, each into its own pair (longer blocks go in parts).
pub const LANE_BLOCK: usize = 1024;

/// Parts of a block shorter than this (between two events close together)
/// play the lane's voices on the audio thread: handing them over would
/// cost more than it saves.
pub const LANE_MIN_FRAMES: usize = 16;

/// Where [`Engine::render_routed`] puts each pad's sound: one stereo pair
/// per pad, such as a mixer's pad channels.
pub trait PadOut {
    /// Pad `pad` of kit `kit`: its pair for this block, both sides at
    /// least the block long. The engine adds the pad's voices into the
    /// start and never clears it. It may ask for the same pad more than
    /// once in a block. `None` leaves the pad unheard: its voices still
    /// play on (into a scratch buffer) and end on time. An engine of one
    /// kit asks for kit 0 only.
    fn pad(&mut self, kit: usize, pad: usize) -> Option<(&mut [f32], &mut [f32])>;
}

/// Every pad into one pair: what [`Engine::render`] renders into.
pub(super) struct Mixed<'a> {
    pub(super) left: &'a mut [f32],
    pub(super) right: &'a mut [f32],
}

impl PadOut for Mixed<'_> {
    fn pad(&mut self, _kit: usize, _pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        Some((&mut *self.left, &mut *self.right))
    }
}

/// What the lane runs: the second share of the voices, for one part of a
/// block.
pub(super) struct Carrier {
    pub(super) pool: Option<Box<Pool>>,
    pub(super) frames: usize,
    pub(super) frame: u64,
    pub(super) rate: f32,
}

impl mpfree_lanes::Job for Carrier {
    fn run(&mut self, _block: u64) {
        if let Some(p) = self.pool.as_mut() {
            p.render_own(self.frames, self.frame, self.rate);
        }
    }
}

impl Engine {
    /// Renders `left.len()` frames, replacing what is in the buffers.
    /// Panics if the two buffers differ in length (a programming error).
    /// Split (see [`Engine::split`]), it waits for the lane's voices no
    /// longer than the block lasts; [`Engine::render_until`] takes a
    /// deadline instead.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.render_mixed(left, right, None);
    }

    /// What [`Engine::render`] does, waiting for the lane's voices (once
    /// split) until `deadline` at most: past it they are left out of that
    /// part of the block and the audio thread carries on.
    pub fn render_until(&mut self, left: &mut [f32], right: &mut [f32], deadline: Instant) {
        self.render_mixed(left, right, Some(deadline));
    }

    pub(super) fn render_mixed(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        deadline: Option<Instant>,
    ) {
        assert_eq!(
            left.len(),
            right.len(),
            "left and right must be the same length"
        );
        let n = left.len();
        left.fill(0.0);
        right.fill(0.0);
        self.run(n, &mut Mixed { left, right }, true, deadline);
    }

    /// Renders the next `frames` frames with each pad apart: every voice
    /// adds its sound into its pad's pair from `out` (see [`PadOut`]).
    /// Events, voices and grains run exactly as in [`Engine::render`], and
    /// the pads' pairs add up to what it renders. The master effect, which
    /// works on the mixed pair, doesn't run. Panics if `out` hands back a
    /// pair shorter than `frames` (a programming error). Split, it waits
    /// for the lane's voices no longer than the block lasts.
    pub fn render_routed(&mut self, frames: usize, out: &mut impl PadOut) {
        self.run(frames, out, false, None);
    }

    /// What [`Engine::render_routed`] does, waiting for the lane's voices
    /// (once split) until `deadline` at most: past it they are left out of
    /// that part of the block and the audio thread carries on.
    pub fn render_routed_until(&mut self, frames: usize, out: &mut impl PadOut, deadline: Instant) {
        self.run(frames, out, false, Some(deadline));
    }

    /// Splits the voice pool in two: the second half renders on a lane
    /// (another core, see `mpfree-lanes`) while the audio thread renders
    /// the first, and its sound is added in after. Events, chokes, mute
    /// groups, polyphony and stealing still see every voice, on the audio
    /// thread; new voices go to the half playing fewer; the grain budget is
    /// shared between the halves before each part of a block. The output is
    /// the same whether the lane runs on its own thread or not
    /// (`config.threaded`), and the same as unsplit but for rounding (the
    /// voices' order of adding) and which grains a full budget skips.
    /// Allocates and starts a thread: call it before the sound starts.
    /// Does nothing if already split or with fewer than 2 voices.
    pub fn split(&mut self, config: mpfree_lanes::Config) -> Option<&Setup> {
        let n = self.pool.a.voices.len();
        if self.lane.is_none() && n >= 2 {
            let keep = n.div_ceil(2);
            let rate = self.rate as u32;
            let mut b = Box::new(Pool::new(rate, keep, n - keep, true));
            // The voices that move stop first (nothing should be playing).
            for k in keep..n {
                if self.pool.a.voices[k].granular {
                    let a = &mut self.pool.a;
                    a.grains[k].kill(&mut a.budget);
                }
            }
            self.pool.a.voices.truncate(keep);
            self.pool.a.grains.truncate(keep);
            b.budget.limit = 0;
            self.pool.b = Some(b);
            self.pool.split = true;
            let carrier = Carrier {
                pool: None,
                frames: 0,
                frame: 0,
                rate: self.rate,
            };
            self.lane = Some(Lane::new(carrier, config));
        }
        self.lane.as_ref().map(|l| l.setup())
    }

    /// The lane's counters, once split.
    pub fn lane_stats(&self) -> Option<mpfree_lanes::Stats> {
        self.lane.as_ref().map(|l| l.stats())
    }

    /// Takes the lane's share back if it came back from a late part: it
    /// follows the tempo and the tape it missed.
    pub(super) fn fetch_lane(&mut self) {
        if self.pool.b.is_some() {
            return;
        }
        let Some(b) = self
            .lane
            .as_mut()
            .and_then(|l| l.job())
            .and_then(|c| c.pool.take())
        else {
            return;
        };
        self.pool.b = Some(b);
        if std::mem::take(&mut self.stop_lane) {
            if let Some(b) = self.pool.b.as_deref_mut() {
                silence(b);
            }
        }
        if self.edit_lane.iter().any(|&e| e) {
            if let Some(b) = self.pool.b.as_deref_mut() {
                for (v, g) in b.voices.iter_mut().zip(b.grains.iter_mut()) {
                    let (k, p) = (usize::from(v.kit), usize::from(v.pad));
                    let ours = p < PADS || v.pad == TAKE_PAD;
                    if v.active && ours && self.edit_lane.get(k) == Some(&true) {
                        let live = &self.lives[k];
                        let program = live.program_of(v, &self.tunings[k]);
                        follow_edit(v, g, &self.views[k], live, &program, true, self.tempo);
                    }
                }
            }
            self.edit_lane.fill(false);
        }
        let tempo = self.tempo;
        for (v, _) in self.pool.iter_mut() {
            if v.active && v.warp_bpm > 0.0 {
                v.base_step = warped_step(v.plain_step, tempo, v.warp_bpm);
                retime(v);
            }
            if v.active && v.motion.beats > 0.0 {
                v.motion.retempo(tempo);
            }
        }
        self.tape_moved = true;
    }
}
