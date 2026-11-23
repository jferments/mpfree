//! The master effect (Grains FX over everything the voices play) and Tape.

use super::*;

/// Seconds of output the master effect remembers for its grains.
pub const MASTER_MEMORY_S: f32 = 8.0;

/// Wow and flutter at full depth: cents and cycles a second of each
/// wobble (two slow ones for the wow, one fast for the flutter).
pub(super) const WOW: [(f32, f32); 3] = [(15.0, 0.53), (10.0, 0.29), (5.0, 7.3)];

/// Below this speed the tape fades the sound out, reaching silence at a halt.
pub(super) const TAPE_FADE: f32 = 0.25;

impl Engine {
    /// A master effect for an engine at `rate`, holding [`MASTER_MEMORY_S`]
    /// seconds of stereo audio (2.8 MB at 44.1 kHz). Allocates: call it off
    /// the audio thread. `None` if `rate` is zero.
    pub fn make_master_fx(rate: u32) -> Option<GrainFx> {
        GrainFx::new(rate, MASTER_MEMORY_S, 0x6d61_7374_6572)
    }

    /// Puts in a master effect (or takes it out, with `None`) and hands back
    /// the old one, its grains silenced and returned to the budget, so the
    /// caller drops it off the audio thread. The new one takes the view's
    /// settings and glides up to its mix from dry, so it comes in without a
    /// click; the old one's grains stop at once, so take it out while the
    /// mix is 0 to avoid a click.
    pub fn set_master_fx(&mut self, fx: Option<GrainFx>) -> Option<GrainFx> {
        let mut old = std::mem::replace(&mut self.master, fx);
        if let Some(o) = &mut old {
            o.kill(&mut self.pool.a.budget);
        }
        self.apply_master();
        old
    }

    /// The master effect, for drawing its memory, grains and playhead.
    pub fn master_fx(&self) -> Option<&GrainFx> {
        self.master.as_ref()
    }

    pub(super) fn apply_master(&mut self) {
        if let (Some(fx), Some(view)) = (&mut self.master, self.views.get(self.master_kit)) {
            fx.set_params(view.master.grain);
            fx.set_mix(view.master.mix);
        }
    }

    /// The master effect takes its settings from kit `kit`'s view from now
    /// on (kit 0 until this is called), as when another kit plays on the
    /// pads. Out of range does nothing.
    pub fn set_master_kit(&mut self, kit: usize) {
        if kit < self.views.len() && kit != self.master_kit {
            self.master_kit = kit;
            self.apply_master();
        }
    }

    /// The tape's speed now: 1 running, 0 stopped.
    pub fn tape_speed(&self) -> f32 {
        self.tape
    }

    /// Moves the tape on by `frames` and has every voice follow it and its
    /// own wow and flutter. Once a period (or between events), never per
    /// frame; cheap while the tape runs and no pad wobbles.
    pub(super) fn follow_tape(&mut self, frames: usize) {
        if self.tape != self.tape_target {
            let d = self.tape_rate * frames as f32;
            self.tape = if self.tape < self.tape_target {
                (self.tape + d).min(self.tape_target)
            } else {
                (self.tape - d).max(self.tape_target)
            };
            self.tape_moved = true;
        }
        let moved = self.tape_moved;
        self.tape_moved = self.tape != 1.0 || self.tape != self.tape_target;
        let (tape, rate) = (self.tape, self.rate);
        let gain = (tape / TAPE_FADE).min(1.0);
        for (v, _) in self.pool.iter_mut() {
            if !v.active || !(moved || v.wow > 0.0) {
                continue;
            }
            let mut cents = 0.0;
            for (k, &(depth, hz)) in WOW.iter().enumerate() {
                if v.wow > 0.0 {
                    cents += v.wow * depth * (std::f32::consts::TAU * v.wow_phase[k]).sin();
                    v.wow_phase[k] = (v.wow_phase[k] + hz * frames as f32 / rate).fract();
                }
            }
            v.speed = tape
                * if cents == 0.0 {
                    1.0
                } else {
                    (cents / 1200.0).exp2()
                };
            v.tape_gain = gain;
            v.gain_l = v.base_gain_l * gain;
            v.gain_r = v.base_gain_r * gain;
            retime(v);
        }
    }
}
