//! The sampler's real-time engine: a fixed voice pool that plays the pads
//! of an [`EngineView`]. `render` and the event calls never allocate, lock,
//! log or touch files; the view is built off the audio thread with
//! [`EngineView::build`] and swapped in with [`Engine::set_view`].
//!
//! Each voice reads its region with 4-point Hermite interpolation at a
//! 32.32 fixed-point position, loops forward (with an equal-power
//! crossfade) or ping-pong, applies fades, an AHDSR amp envelope, an
//! optional 2- or 4-pole state-variable filter with its own envelope, and
//! equal-power pan. Stolen and choked voices fade out instead of clicking.
//!
//! **Clean pitching.** Once a voice reads its sound more than two
//! semitones faster than the sound's own rate ([`CLEAN_ABOVE`]: pitched
//! up, or a file recorded at a higher rate), it reads it band-limited
//! instead, for the rest of its life: a windowed sinc stretched to the
//! speed, from the sound or, past an octave, from one of its octave copies
//! ([`crate::bandlimit`]), so nothing folds back as aliasing. Up to two
//! semitones what folds lands above 19 kHz, so a voice there, or at or
//! below its pitch, reads as before, bit for bit, at the Hermite read's
//! cost. A pad set to Vintage ([`crate::model::Pad::vintage`]) keeps the
//! Hermite read whatever its pitch, aliasing and all.
//!
//! A region in [`PlayMode::Granular`] plays through the voice's own
//! [`GrainVoice`] (from `mpfree-grain`) instead of the Hermite read; its
//! grains read the sound through [`GrainClip`], clean when pitched up past
//! two semitones as a voice is (not on a Vintage pad). The envelope, gain,
//! pan, filter and fades after it are the same. One grain
//! [`Budget`] ([`GRAIN_BUDGET`]) caps the grains of all voices together.
//!
//! The master effect is a [`GrainFx`] on everything the voices play: grains
//! from the last [`MASTER_MEMORY_S`] seconds of the output, mixed back over
//! it, with freeze ([`Event::Freeze`]). Its settings come with the view
//! ([`crate::model::MasterFx`]); the effect itself, which holds the memory,
//! is made off the audio thread with [`Engine::make_master_fx`] and put in
//! with [`Engine::set_master_fx`]. Its grains come out of the same budget.
//!
//! [`Engine::render`] mixes every pad into one stereo pair.
//! [`Engine::render_routed`] keeps them apart instead: each voice adds its
//! sound into its own pad's pair from a [`PadOut`] (a mixer's pad
//! channels), so each pad gets its own fader, pan, mute, solo, sends and
//! output. The master effect works on the mixed pair, so it only runs in
//! [`Engine::render`]; on a mixer, a granular insert on the pads' track
//! does its job.
//!
//! **Live settings.** [`Event::PadLive`] changes one of a pad's settings
//! while it plays (its level, pan, tune, filter or envelope), as a knob, a
//! p-lock or a modulation would, without building a new view; the value
//! replaces the pad's own until it is cleared (NaN) or a new view comes
//! in. [`Event::PadLock`] is a step's parameter lock: a pad's setting
//! from the sequencer, under any live value, kept through a new view (a
//! locked Decay also takes the pad's sustain to 0, so the sound decays).
//! [`Event::ProgramLive`] moves every pad at once by an offset on its
//! own level, tune or cutoff. New voices start with all of them; playing
//! voices follow the level, pan, cutoff and resonance and the program's
//! offsets, gliding there in steps of 16 frames so nothing clicks. Both go
//! through the event queue, so they land on their frame, and split
//! ([`Engine::split`]) they reach both halves of the voices.
//! [`Engine::stop_all`] is the hard stop: every voice and grain idle at
//! once and the events waiting dropped.
//!
//! **Edits.** [`Engine::edit_view`] swaps in an edit of the kit playing
//! now, and the voices playing hear it: a voice whose layer still plays
//! the same sound takes its new loop points at once (jumping into a loop
//! moved behind it, the step faded out over [`JUMP_FRAMES`]), its tune at
//! once, its grain settings, and glides to its new level, pan, cutoff and
//! resonance as it does for live settings. [`Engine::set_view`] is for
//! another kit: its voices finish as they began.
//!
//! **Kits.** An engine made with [`Engine::with_kits`] plays several kits
//! at once from one voice pool, as a kit per track does: each kit has its
//! own view ([`Engine::set_kit_view`], [`Engine::edit_kit_view`]), live
//! settings and locks, and the events for it go in with
//! [`Engine::schedule_for`] or [`Engine::apply_for`]. A voice knows its
//! kit and pad, so mute groups, chokes, links, polyphony and a pad's
//! release stay within its kit, and [`PadOut`] gets each voice's kit with
//! its pad. Stealing looks at every voice, whatever its kit. Kit 0 is the
//! one [`Engine::new`] makes and the calls without a kit address, so an
//! engine of one kit works as it always has. [`Event::AllOff`],
//! [`Event::Freeze`] and [`Event::Tape`] are for the whole engine, from
//! any kit.

use crate::bandlimit::{self, OctaveCell, Reach, NEAR_HALF, NEAR_TAPS};
use crate::dsp::{
    db_to_gain, equal_power, hermite, pan_gains, vowel_formants, vowel_gain, vowel_resonance,
    Ahdsr, Comb, Envelope, Filter, FilterKind, Ladder, Rng, Sample, Svf, FORMANT_2, TINY,
};
use crate::grain_clip::GrainClip;
use crate::grain_share::{self, Claim, Rank};
use crate::hits::{self, HitCell};
use crate::model::{
    edge_gain, ChannelPick, Fade, GrainMode, GrainParams, Keys, LayerSelect, Levels, LfoShape,
    LfoTarget, LoopMode, MasterFx, Mono, PadLfo, PadsPlay, Pcm, PitchEnv, PlayMode, PressureRoute,
    PressureTarget, Project, SliceSet, WarpMethod, GLIDE_MS, GRAIN_GLIDE_MAX_MS, LFO_BEATS,
    LFO_RATE_HZ, PADS, PITCH_DECAY_MS, PITCH_ENV_ST,
};
use crate::warp::{CopyMap, Warps};
use mpfree_grain::{Budget, GrainFx, GrainSource, GrainVoice, MIN_SPAN};
pub use mpfree_grain::{GrainView, MAX_GRAINS};
/// The lane types [`Engine::split`] takes and reports, so callers need no
/// dependency of their own on `mpfree-lanes`.
pub use mpfree_lanes::{Config as LaneConfig, Setup as LaneSetup, Stats as LaneStats};
use mpfree_lanes::{Lane, Setup};
use mpfree_node::Cull;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod counters;
mod events;
mod follow;
mod grains;
mod master;
mod notes;
mod render;
mod routing;
#[cfg(test)]
mod tests;
mod view;
mod voices;

pub use counters::*;
pub use events::*;
pub use follow::*;
pub use grains::*;
pub use master::*;
use notes::*;
pub use render::*;
pub use routing::*;
pub use view::*;
pub use voices::*;

/// The most kits one engine plays ([`Engine::with_kits`]): one for each of
/// the sequencer's 128 tracks.
pub const MAX_KITS: usize = 128;

/// The sampler's real-time engine: the voice pool, each kit's view and the
/// event queue (see the module's docs; the parts are in its modules).
pub struct Engine {
    rate: f32,
    /// Each kit's view ([`Engine::with_kits`]).
    views: Vec<EngineView>,
    /// The kit the event being applied is for.
    at: usize,
    /// The kit whose view the master effect takes its settings from.
    master_kit: usize,
    /// The voices, with their grain voices, grain budgets and buffers.
    pool: Voices,
    /// Every cloud's claim on the grain budget and which cloud it is, for
    /// sharing it out ([`Engine::share_grains`]): room for every voice
    /// and the master effect's two clouds, reserved here.
    claims: Vec<Claim>,
    clouds: Vec<Cloud>,
    /// Split: the second share of the voices renders here, on another core.
    lane: Option<Lane<Carrier>>,
    /// (frame offset, kit, event).
    events: Vec<(u32, u8, Event)>,
    /// Let-gos the queue had no room for, each in its own place
    /// ([`LET_GO_PLACES`] a kit, then the one for all notes off) with the
    /// frame offset it lands on; [`NOT_DUE`] where none waits.
    late_due: Vec<u32>,
    /// The places in `late_due` with a let-go waiting, in no order.
    late: Vec<u32>,
    /// No pressure or live value in the queue has a newer one for the same
    /// pad and setting behind it, so none can make room
    /// ([`Engine::make_room`]). Cleared when one goes in.
    nothing_to_merge: bool,
    /// Each pad's turn for Cycle (see [`TURNS`]), [`PADS`] a kit.
    cycle: Vec<u32>,
    /// [`Engine::victim`]'s count of each kit's pads' held voices, all 0
    /// between calls, [`PADS`] a kit.
    held: Vec<u16>,
    /// Each pad's last granular key (its voice's semitones), which a
    /// gliding layer's next voice starts from, [`PADS`] a kit.
    last_key: Vec<Option<f32>>,
    /// Each mono pad's keys held (sampler R5.5), [`PADS`] a kit.
    mono_keys: Vec<HeldKeys>,
    /// Each mono pad's last note, which Glide on every note slides from
    /// when nothing sounds, [`PADS`] a kit.
    mono_at: Vec<Option<f32>>,
    rng: Rng,
    clock: u64,
    fixed_velocity: Option<u8>,
    stats: Stats,
    /// Frames rendered since the engine was made: the timeline synced grains use.
    frames: u64,
    master: Option<GrainFx>,
    /// The project tempo, for warped layers.
    tempo: f64,
    /// The transport's position in beats and the frame it was at, while
    /// it plays ([`Engine::set_beat`]): a synced LFO that runs on between
    /// hits keeps to it.
    beat: Option<(u64, f64)>,
    /// The frame the events being applied are on.
    event_frame: u64,
    /// The tape's speed (1 when running, 0 stopped), where it is going,
    /// and how far it moves a frame.
    tape: f32,
    tape_target: f32,
    tape_rate: f32,
    /// Whether the voices still have to follow a tape that moved.
    tape_moved: bool,
    /// Each kit's live settings ([`Event::PadLive`], [`Event::ProgramLive`]).
    lives: Vec<Live>,
    /// The one-pole step playing voices follow them with, per
    /// [`FILTER_STEP`] frames.
    live_k: f32,
    /// Split: [`Engine::stop_all`] came while the lane's share was away;
    /// it stops when it comes back.
    stop_lane: bool,
    /// Split: an edit ([`Engine::edit_view`]) of a kit came while the
    /// lane's share was away; its voices follow the view when it comes
    /// back. One a kit.
    edit_lane: Vec<bool>,
    /// Voices this old or newer (by `clock`) were started by the hit being
    /// played, so its links don't choke each other.
    spare_from: u64,
    /// Live chop: the pad whose audition marks are taken from, and its kit.
    chop_pad: Option<u8>,
    chop_kit: u8,
    /// Live chop's cuts, in source frames: cut `i` at `marks[i % MAX_MARKS]`.
    marks: [u64; MAX_MARKS],
    marks_made: u64,
    /// How many voices may be full voices (not plain one-shots) at once
    /// ([`Engine::set_full_voices`]); the whole pool unless set.
    full_limit: usize,
}

impl Engine {
    /// `voices` is the size of the fixed pool. All memory is reserved here.
    /// One kit, kit 0.
    pub fn new(rate: u32, voices: usize) -> Engine {
        Engine::with_kits(rate, voices, 1)
    }

    /// An engine playing `kits` kits at once (1 to [`MAX_KITS`]) from one
    /// pool of `voices` voices, each kit with an empty view until it gets
    /// one ([`Engine::set_kit_view`]). All memory is reserved here.
    pub fn with_kits(rate: u32, voices: usize, kits: usize) -> Engine {
        let kits = kits.clamp(1, MAX_KITS);
        // Made here, so the audio thread's first clean read finds it.
        bandlimit::kernel();
        bandlimit::near_kernel();
        Engine {
            rate: rate as f32,
            views: vec![EngineView::empty(); kits],
            at: 0,
            master_kit: 0,
            pool: Voices {
                a: Pool::new(rate, 0, voices.max(1), false),
                b: None,
                b_limit: 0,
                b_skipped: 0,
                split: false,
            },
            claims: Vec::with_capacity(voices.max(1) + 2),
            clouds: Vec::with_capacity(voices.max(1) + 2),
            lane: None,
            events: Vec::with_capacity(MAX_EVENTS),
            late_due: vec![NOT_DUE; kits * LET_GO_PLACES + 1],
            late: Vec::with_capacity(kits * LET_GO_PLACES + 1),
            nothing_to_merge: false,
            cycle: vec![0; PADS * kits],
            held: vec![0; PADS * kits],
            last_key: vec![None; PADS * kits],
            mono_keys: vec![HeldKeys::NONE; PADS * kits],
            mono_at: vec![None; PADS * kits],
            rng: Rng::new(0x5A4D_5052),
            clock: 0,
            fixed_velocity: None,
            stats: Stats::default(),
            frames: 0,
            master: None,
            tempo: 120.0,
            beat: None,
            event_frame: 0,
            tape: 1.0,
            tape_target: 1.0,
            tape_rate: 0.0,
            tape_moved: false,
            lives: vec![
                Live {
                    pads: Live::OWN,
                    locks: Live::OWN,
                    program: [0.0; ProgramField::COUNT],
                    bend: 0.0,
                };
                kits
            ],
            live_k: 1.0 - (-(FILTER_STEP as f32) / (LIVE_SMOOTH_S * rate as f32)).exp(),
            stop_lane: false,
            edit_lane: vec![false; kits],
            spare_from: u64::MAX,
            chop_pad: None,
            chop_kit: 0,
            marks: [0; MAX_MARKS],
            marks_made: 0,
            full_limit: voices.max(1),
        }
    }

    /// The project tempo, in BPM, for warped layers: they play `tempo /
    /// their tempo` times as fast. Voices already playing follow at once
    /// (a stretched copy made for another tempo repitches until the next
    /// hit plays the new one). Cheap when nothing changes; call it every
    /// period with the transport's tempo.
    pub fn set_tempo(&mut self, bpm: f64) {
        if !bpm.is_finite() || bpm <= 0.0 || bpm == self.tempo {
            return;
        }
        self.tempo = bpm;
        let beat = self.frames_per_beat();
        for (v, g) in self.pool.iter_mut() {
            if v.active && v.warp_bpm > 0.0 {
                v.base_step = warped_step(v.plain_step, bpm, v.warp_bpm);
                retime(v);
            }
            // A synced LFO keeps its cycle in beats.
            if v.active && v.motion.beats > 0.0 {
                v.motion.retempo(bpm);
            }
            // A cloud synced to the tempo follows it at once.
            g.set_tempo(beat);
        }
    }

    /// Where the transport is, in beats, at the start of the next render,
    /// while it plays (None stopped): a synced LFO with Retrig off keeps
    /// to its beats (sampler R5 part 4). Once a period, with the tempo.
    pub fn set_beat(&mut self, beat: Option<f64>) {
        self.beat = beat.filter(|b| b.is_finite()).map(|b| (self.frames, b));
    }

    /// Output frames per beat at the project tempo: what a granular pad's
    /// tempo sync follows, whatever its saved `frames_per_beat` says.
    pub fn frames_per_beat(&self) -> f64 {
        60.0 * f64::from(self.rate) / self.tempo
    }

    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    /// The output rate it renders at, Hz.
    pub fn rate(&self) -> u32 {
        self.rate as u32
    }

    /// Frames rendered so far. Synced grains start on frames of this
    /// timeline, so it should match the transport's frame count.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The render loop: events on their frames, then every voice into its
    /// pad's pair, then (`master`) the master effect over pad 0's pair,
    /// which is the mixed pair when `out` is [`Mixed`]. Split, each part of
    /// the block between events hands the lane's share to the lane, renders
    /// the engine's own and adds the lane's in (waiting until `deadline`).
    /// While the lane's share is away (late), events wait in the queue.
    fn run(
        &mut self,
        frames: usize,
        out: &mut impl PadOut,
        master: bool,
        deadline: Option<Instant>,
    ) {
        let n = frames as u32;
        let mut at = 0u32;
        // The events applied so far this block, taken off the queue's front
        // once at the end (not one at a time, which would move the rest
        // each time). `apply` never queues events.
        let mut applied = 0usize;
        // Without a deadline, the lane gets as long as the block lasts.
        let deadline = match (&self.lane, deadline) {
            (Some(_), None) => {
                let ns = u64::from(n) * 1_000_000_000 / u64::from(self.rate as u32).max(1);
                Some(Instant::now() + Duration::from_nanos(ns))
            }
            (_, d) => d,
        };
        while at < n {
            self.fetch_lane();
            let home = !self.pool.split || self.pool.b.is_some();
            let mut next = n;
            if home {
                // Apply every event due now.
                self.event_frame = self.frames + u64::from(at);
                while let Some(&(o, kit, ev)) = self.events.get(applied) {
                    if o > at {
                        break;
                    }
                    applied += 1;
                    self.apply_for(usize::from(kit), ev);
                }
                next = self.events.get(applied).map_or(n, |&(o, _, _)| o.min(n));
                if !self.late.is_empty() {
                    next = next.min(self.land_late(at, n));
                }
            }
            if self.pool.split {
                next = next.min(at + LANE_BLOCK as u32);
            }
            let (a, b) = (at as usize, next as usize);
            let frame = self.frames + at as u64;
            self.follow_tape(b - a);
            self.pool.share_budget();
            self.share_grains(b - a, master);
            let rate = self.rate;
            let lane_busy = self
                .pool
                .b
                .as_ref()
                .is_some_and(|p| p.voices.iter().any(|v| v.active));
            match (self.lane.as_mut(), deadline) {
                (Some(lane), Some(deadline)) if lane_busy && b - a >= LANE_MIN_FRAMES => {
                    let pool = self.pool.b.take();
                    self.pool.b_limit = pool.as_ref().map_or(0, |p| p.budget.limit);
                    self.pool.b_skipped = pool.as_ref().map_or(0, |p| p.budget.skipped);
                    if let Some(c) = lane.job() {
                        (c.pool, c.frames, c.frame, c.rate) = (pool, b - a, frame, rate);
                    }
                    let _ = lane.post(frame);
                    self.stats.lane_parts += 1;
                    self.pool.a.render_into(a, b, frame, rate, out);
                    match lane.join(deadline) {
                        Ok(c) => {
                            if let Some(p) = c.pool.take() {
                                p.add_own(a, b - a, out);
                                self.pool.b = Some(p);
                            }
                        }
                        Err(_) => self.stats.lane_late += 1,
                    }
                }
                _ => {
                    self.pool.a.render_into(a, b, frame, rate, out);
                    if let Some(p) = self.pool.b.as_mut() {
                        p.render_into(a, b, frame, rate, out);
                    }
                }
            }
            if master {
                if let (Some(fx), Some((l, r))) = (&mut self.master, out.pad(0, 0)) {
                    let budget = &mut self.pool.a.budget;
                    if self.pool.split {
                        let others = self
                            .pool
                            .b
                            .as_ref()
                            .map_or(self.pool.b_limit, |p| p.budget.live());
                        budget.limit = GRAIN_BUDGET.saturating_sub(others);
                    }
                    fx.process(budget, frame, &mut l[a..b], &mut r[a..b]);
                }
            }
            at = next;
        }
        self.frames += u64::from(n);
        // Events applied between renders are on the next frame.
        self.event_frame = self.frames;
        self.events.drain(..applied);
        for e in &mut self.events {
            e.0 = e.0.saturating_sub(n);
        }
        for &i in &self.late {
            let d = &mut self.late_due[i as usize];
            *d = d.saturating_sub(n);
        }
    }
}
