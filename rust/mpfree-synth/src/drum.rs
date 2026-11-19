//! Drums: a drum synth with a model per pad. Sixteen pads, each one of
//! sixteen models (kick, 808 kick, snare, clap, closed and open hat,
//! cymbal, tom, conga, rim, cowbell, woodblock, bell, shaker, zap, FM
//! perc), each with its own knobs ([`crate::drum_params::knobs`]) and the
//! shared ones: level, pan, velocity to level and tone, accent, variation,
//! choke group, and whether it plays as a one-shot or while held.
//!
//! **Voices.** A kit plays its pads with the Classic voices (`classic.rs`,
//! the engine as first built, kept to the bit so old kits and projects
//! sound as they did) or the Current ones (`current.rs`, every factory
//! kit), by its Voices setting ([`crate::drum_params::VOICES`]). A model
//! the first engine didn't have always plays its Current voice.
//!
//! Notes play pads: pad 1 is the Pad 1 note parameter (36 by default) and
//! each pad after it a semitone higher. Other notes are ignored. A hit
//! rings for its decay; a Current pad set to play While held fades out
//! when its note is let go. A new hit on a pad fades its last hit out, and
//! a hit in a choke group fades out the group's other pads (an open hat
//! closed by the closed hat): over 3 ms in a straight line for Classic, as
//! it was, and over [`CHOKE_MS`] on a smooth step for Current, which
//! doesn't click.
//!
//! A hit takes its pad's settings when it starts, as drum machines do, so a
//! knob turn changes the next hit. The kit Level moves at once (smoothed).
//!
//! Every part of a voice runs every frame, and anything that moves at a
//! control rate moves on the hit's own frames, so the output does not
//! depend on how the host cuts blocks. Nothing here allocates after
//! `prepare`.

mod classic;
mod current;

use mpfree_node::{Ctx, Cull, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed};

pub use self::classic::sin_turns;
use self::classic::State;
pub use self::classic::METAL;
pub use self::current::{
    bessel_j, cymbal_bands, metal_bank, metal_hz, BP_HIGH, BP_LOW, CTRL, METAL_WIDTH,
};
use crate::drum_params::*;
use crate::dsp::Rng;
use crate::params::Preset;

/// Hit slots: one per pad plus room for fading hits.
pub const POOL: usize = 32;
/// A replaced or choked Classic hit fades out over this long.
const KILL_MS: f32 = 3.0;
/// A replaced or choked Current hit fades out over this long, on a smooth
/// step.
pub const CHOKE_MS: f32 = 5.0;
/// A Current hit that plays While held fades out over this long once let
/// go.
pub const RELEASE_MS: f32 = 50.0;
/// AllOff fades every hit out over this long.
const ALL_OFF_MS: f32 = 30.0;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.drums",
    name: "Drums",
    kind: Kind::Instrument,
};

/// One hit slot: a Classic or a Current voice. The pool is fixed, so the
/// slots are as big as the bigger voice rather than boxed (no allocation).
#[derive(Clone, Copy, Debug)]
#[allow(clippy::large_enum_variant)]
enum Slot {
    Classic(classic::Voice),
    Current(current::Voice),
}

impl Default for Slot {
    fn default() -> Slot {
        Slot::Classic(classic::Voice::default())
    }
}

impl Slot {
    fn state(&self) -> State {
        match self {
            Slot::Classic(v) => v.state,
            Slot::Current(v) => v.state,
        }
    }
    fn pad(&self) -> u8 {
        match self {
            Slot::Classic(v) => v.pad,
            Slot::Current(v) => v.pad,
        }
    }
    fn choke(&self) -> u8 {
        match self {
            Slot::Classic(v) => v.choke,
            Slot::Current(v) => v.choke,
        }
    }
    fn age(&self) -> u64 {
        match self {
            Slot::Classic(v) => v.age,
            Slot::Current(v) => v.age,
        }
    }
    fn level(&self) -> f32 {
        match self {
            Slot::Classic(v) => v.level(),
            Slot::Current(v) => v.level(),
        }
    }
    fn free(&mut self) {
        match self {
            Slot::Classic(v) => v.state = State::Free,
            Slot::Current(v) => v.state = State::Free,
        }
    }
    /// Fades the hit out: a Classic one in a straight line over
    /// `classic_ms`, a Current one on a smooth step over `current_ms`.
    fn fade_out(&mut self, classic_ms: f32, current_ms: f32, rate: f32) {
        match self {
            Slot::Classic(v) => {
                v.state = State::Kill;
                v.kill = 1.0;
                v.kill_step = 1.0 / (classic_ms * 0.001 * rate).max(1.0);
            }
            Slot::Current(v) => v.fade_out(current_ms * 0.001 * rate),
        }
    }
}

/// A clap's handclaps, for drawing them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bursts {
    /// When each starts after the hit, in seconds: the first `count`.
    pub starts: [f32; 8],
    pub count: usize,
    /// The share of a burst left after each frame.
    pub mul: f32,
    /// The tail's level against a burst's.
    pub tail: f32,
}

impl Bursts {
    /// The bursts' starts, in seconds.
    pub fn starts(&self) -> &[f32] {
        &self.starts[..self.count.min(8)]
    }
}

/// What a full-velocity hit of one pad does over time, for drawing it
/// (`mpfree-synth-ui`). Each part is (its level in the mix before the
/// filters, the share of it left after each frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitShape {
    /// The pad's model ("Kick").
    pub model: &'static str,
    /// The body's pitch once the sweep has settled, in Hz; 0 for a model
    /// with no body (hats, clap, cowbell, cymbal, shaker).
    pub hz: f32,
    /// The body: the tuned sines (or FM, or a drum's modes).
    pub body: (f32, f32),
    /// The noise and metal, through the pad's filters.
    pub noise: (f32, f32),
    /// The click.
    pub click: (f32, f32),
    /// The pitch sweep (or a drum head's bend): semitones above `hz` at
    /// the start, and the share of it left after each frame.
    pub sweep: (f32, f32),
    /// A clap's bursts. The noise envelope carries the tail.
    pub bursts: Option<Bursts>,
    /// Drive, 0 to 1.
    pub drive: f32,
}

/// The shape of a full-velocity hit of `pad` (0 to 15) at `rate`, with the
/// values `value` gives by parameter id (the kit's Voices among them).
/// Built by the same code as the hits [`Drums`] plays, with no variation.
pub fn hit_shape(value: impl Fn(u16) -> f32, pad: usize, rate: u32) -> HitShape {
    let pad = pad.min(PADS - 1);
    let get = |f: u16| value(pad_id(pad, f));
    let current = value(VOICES) >= 0.5;
    let m = model_of(get(field::MODEL));
    if current || m >= model::FIRST {
        let v = current::Voice::build(&current::Hit {
            get: &get,
            rate: rate as f32,
            pad,
            velocity: 127,
            seed: 0,
            clock: 0,
            current,
            signs: (1.0, 1.0),
            vary: false,
        });
        return v.shape(rate as f32);
    }
    let model = MODELS[m as usize];
    let v = classic::build(get, rate as f32, pad, 127, 0, 0);
    let body = v.lvl_a > 0.0;
    let mut starts = [0.0; 8];
    starts[..classic::CLAP_BURSTS.len()].copy_from_slice(&classic::CLAP_BURSTS);
    HitShape {
        model,
        hz: if body { v.f0 } else { 0.0 },
        body: (if body { v.lvl_a + v.lvl_b } else { 0.0 }, v.body_mul),
        noise: (
            v.noise_lvl + v.metal_lvl * (v.n_metal as f32).sqrt(),
            v.noise_mul,
        ),
        click: (v.click_lvl, v.click_mul),
        sweep: (if body { v.sweep } else { 0.0 }, v.pitch_mul),
        bursts: v.clap.then_some(Bursts {
            starts,
            count: classic::CLAP_BURSTS.len(),
            mul: v.burst_mul,
            tail: 0.35,
        }),
        drive: v.drive,
    }
}

/// The drum synth. See the module docs.
pub struct Drums {
    rate: u32,
    values: [f32; ID_SPACE],
    voices: [Slot; POOL],
    clock: u64,
    rng: Rng,
    level: Smooth,
    gains: Vec<f32>,
    hits: u64,
    /// Each pad's Current hits so far: the variation's signs alternate.
    pad_hits: [u32; PADS],
}

impl Drums {
    pub fn new(rate: u32) -> Drums {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS.iter() {
            values[p.id as usize] = p.default;
        }
        let mut d = Drums {
            rate,
            values,
            voices: [Slot::default(); POOL],
            clock: 0,
            rng: Rng::new(0x5EED_D2B1),
            level: Smooth::new(1.0),
            gains: Vec::new(),
            hits: 0,
            pad_hits: [0; PADS],
        };
        d.prepare(rate, mpfree_node::MAX_BLOCK);
        d
    }

    /// Loads a factory kit: the Init kit, then the kit's values. Hits ring on.
    pub fn load_preset(&mut self, kit: &Preset) {
        for p in PARAMS.iter() {
            if !p.readonly {
                self.set_param(p.id, p.default);
            }
        }
        for &(id, v) in kit.values {
            self.set_param(id, v);
        }
    }

    pub fn presets() -> &'static [Preset] {
        crate::factory::presets!("drums.presets", &INFO, &PARAMS)
    }

    /// Hits sounding or fading.
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.state() != State::Free)
            .count()
    }

    /// The model name on pad `pad` (0 to 15), for pad labels.
    pub fn pad_name(&self, pad: usize) -> &'static str {
        MODELS[model_of(self.v(pad_id(pad.min(PADS - 1), field::MODEL))) as usize]
    }

    /// The pad a note plays, if any.
    pub fn pad_of(&self, note: u8) -> Option<usize> {
        let p = i32::from(note) - self.v(BASE_NOTE) as i32;
        (0..PADS as i32).contains(&p).then_some(p as usize)
    }

    /// Whether the kit plays the Current voices.
    pub fn current(&self) -> bool {
        self.v(VOICES) >= 0.5
    }

    #[inline]
    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }
    fn pv(&self, pad: usize, f: u16) -> f32 {
        self.v(pad_id(pad, f))
    }

    fn level_target(&self) -> f32 {
        let db = self.v(LEVEL);
        if db <= -60.0 {
            0.0
        } else {
            10f32.powf(db / 20.0)
        }
    }

    /// Fades out every sounding hit `which` picks: Classic ones over
    /// `classic_ms`, Current ones over `current_ms`.
    fn kill_where(
        &mut self,
        classic_ms: f32,
        current_ms: f32,
        mut which: impl FnMut(&Slot) -> bool,
    ) {
        let rate = self.rate as f32;
        for v in self.voices.iter_mut() {
            if v.state() == State::On && which(v) {
                v.fade_out(classic_ms, current_ms, rate);
            }
        }
    }

    /// The hit load shedding gives up first: the quietest one sounding. A
    /// hit let go is already fading, so none counts as releasing; its own
    /// decay makes an old hit quiet. Hits already fading are left be.
    fn cull_pick(&self) -> Option<(usize, Cull)> {
        let mut best = None;
        for (i, v) in self.voices.iter().enumerate() {
            if v.state() != State::On {
                continue;
            }
            let c = Cull {
                level: v.level(),
                releasing: false,
            };
            best = Cull::first(best, Some((i, c)));
        }
        best
    }

    fn hit(&mut self, pad: usize, velocity: u8) {
        if velocity == 0 {
            // A note-on at velocity 0 is a note-off.
            self.release(pad);
            return;
        }
        let choke = self.pv(pad, field::CHOKE) as u8;
        self.kill_where(KILL_MS, CHOKE_MS, |v| {
            v.pad() as usize == pad || (choke > 0 && v.choke() == choke)
        });
        self.clock += 1;
        self.hits += 1;
        let slot = self
            .voices
            .iter()
            .position(|v| v.state() == State::Free)
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.state() == State::Kill)
                    .min_by_key(|(_, v)| v.age())
                    .map(|(i, _)| i)
            })
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age())
                    .map_or(0, |(i, _)| i)
            });
        let seed = self.rng.next_u64();
        let v = self.setup(pad, velocity, seed);
        self.voices[slot] = v;
    }

    /// Lets go of `pad`: its Current hits that play While held fade out.
    fn release(&mut self, pad: usize) {
        let frames = RELEASE_MS * 0.001 * self.rate as f32;
        for s in self.voices.iter_mut() {
            if let Slot::Current(v) = s {
                if v.gated && v.state == State::On && usize::from(v.pad) == pad {
                    v.fade_out(frames);
                }
            }
        }
    }

    /// Builds a hit of `pad` from its settings: a Classic voice for one of
    /// the first models in a Classic kit, else a Current one.
    fn setup(&mut self, pad: usize, velocity: u8, seed: u64) -> Slot {
        let rate = self.rate as f32;
        let current = self.current();
        let m = model_of(self.pv(pad, field::MODEL));
        if !current && m < model::FIRST {
            return Slot::Classic(classic::build(
                |f| self.pv(pad, f),
                rate,
                pad,
                velocity,
                seed,
                self.clock,
            ));
        }
        let n = self.pad_hits[pad];
        self.pad_hits[pad] = n.wrapping_add(1);
        // Each hit moves the other way from the last: louder and a little
        // sharper, then softer and flatter, as a drum head does.
        let s = if n.is_multiple_of(2) { 1.0 } else { -1.0 };
        let signs = (s, s);
        let get = |f: u16| self.pv(pad, f);
        Slot::Current(current::Voice::build(&current::Hit {
            get: &get,
            rate,
            pad,
            velocity,
            seed,
            clock: self.clock,
            current,
            signs,
            vary: true,
        }))
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => {
                if let Some(p) = self.pad_of(note) {
                    self.hit(p, velocity);
                }
            }
            NodeEvent::NoteOff { note } => {
                if let Some(p) = self.pad_of(note) {
                    self.release(p);
                }
            }
            NodeEvent::AllOff => self.kill_where(ALL_OFF_MS, ALL_OFF_MS, |_| true),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Pressure { .. } | NodeEvent::Bend { .. } | NodeEvent::Expression { .. } => {}
            // The pedals are held by the host for this engine (`Node::hears`).
            NodeEvent::Control { .. } => {}
        }
    }

    fn run(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let rate = self.rate as f32;
        for g in &mut self.gains[from..to] {
            *g = self.level.next();
        }
        let gains = &self.gains;
        for slot in self.voices.iter_mut() {
            match slot {
                Slot::Classic(v) => {
                    if v.state == State::Free {
                        continue;
                    }
                    for i in from..to {
                        let x = v.tick(rate) * gains[i];
                        left[i] += x * v.gain_l;
                        right[i] += x * v.gain_r;
                        if v.state == State::Free {
                            break;
                        }
                    }
                }
                Slot::Current(v) => {
                    if v.state == State::Free {
                        continue;
                    }
                    for i in from..to {
                        let x = v.tick(rate) * gains[i];
                        left[i] += x * v.gain_l;
                        right[i] += x * v.gain_r;
                        if v.state == State::Free {
                            break;
                        }
                    }
                }
            }
        }
    }
}

impl Node for Drums {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = param_info(id) else {
            return;
        };
        if p.readonly {
            return;
        }
        self.values[id as usize] = p.clamp(value);
        if id == LEVEL {
            self.level.target = self.level_target();
        }
    }

    fn param(&self, id: u16) -> f32 {
        if id == METER_VOICES {
            return self.active_voices() as f32;
        }
        match param_info(id) {
            Some(_) => self.values[id as usize],
            None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        self.rate = rate;
        self.gains = vec![0.0; max_block.max(1)];
        self.level.set_time(rate, 10.0);
        self.level.snap(self.level_target());
        self.reset();
    }

    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.free();
        }
        self.pad_hits = [0; PADS];
        self.level.snap(self.level_target());
    }

    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len()).min(self.gains.len());
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            while i < events.len() && (events[i].offset as usize) <= at {
                let ev = events[i].ev;
                self.apply(&ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            if next > at {
                self.run(at, next, left, right);
            }
            at = next;
        }
        for e in &events[i..] {
            let ev = e.ev;
            self.apply(&ev);
        }
    }

    fn tail_frames(&self) -> u32 {
        (5.2 * self.rate as f32) as u32
    }

    fn sounding_voices(&self) -> usize {
        self.active_voices()
    }

    fn cull_candidate(&self) -> Option<Cull> {
        self.cull_pick().map(|(_, c)| c)
    }

    fn cull(&mut self) -> bool {
        let Some((i, _)) = self.cull_pick() else {
            return false;
        };
        let rate = self.rate as f32;
        self.voices[i].fade_out(KILL_MS, CHOKE_MS, rate);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::classic::floor;

    #[test]
    fn the_quick_floor_matches_floorf_for_every_input() {
        let same = |x: f32| floor(x).to_bits() == x.floor().to_bits() || x.is_nan();
        for x in [
            0.0,
            -0.0,
            0.5,
            -0.5,
            1.0,
            8_388_607.5,
            8_388_608.0,
            f32::MIN_POSITIVE,
        ] {
            assert!(same(x) && same(-x), "{x:e}");
        }
        for bits in (0..=u32::MAX).step_by(997) {
            let x = f32::from_bits(bits);
            assert!(same(x), "{bits:#x}");
        }
    }
}
