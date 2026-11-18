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

use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_grain::{Budget, GrainFx, GrainSource, GrainVoice};
pub use mpfree_grain::{GrainView, MAX_GRAINS};

/// A layer's cloud as it plays now ([`Engine::cloud_of`]), for drawing it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CloudState {
    /// The spray its grains start in, 0 to 1 of the region: its setting as
    /// movement and pressure have moved it.
    pub spray: f32,
    /// More grains wanted at once than a voice keeps
    /// ([`mpfree_grain::MAX_GRAINS`]): the voice spaces them out to fit.
    pub thinned: bool,
}
/// The lane types [`Engine::split`] takes and reports, so callers need no
/// dependency of their own on `mpfree-lanes`.
pub use mpfree_lanes::{Config as LaneConfig, Setup as LaneSetup, Stats as LaneStats};
use mpfree_lanes::{Lane, Setup};
use mpfree_node::Cull;

use crate::bandlimit::{self, OctaveCell, Reach};
use crate::dsp::{
    db_to_gain, equal_power, hermite, pan_gains, Ahdsr, Envelope, Filter, FilterKind, Rng, Sample,
    Svf, TINY,
};
use crate::grain_clip::GrainClip;
use crate::hits::{self, HitCell};
use crate::model::{
    edge_gain, ChannelPick, Fade, GrainMode, GrainParams, Keys, LayerSelect, Levels, LoopMode,
    MasterFx, PadsPlay, Pcm, PlayMode, PressureRoute, PressureTarget, Project, WarpMethod,
    GRAIN_GLIDE_MAX_MS, PADS,
};
use crate::warp::{CopyMap, Warps};

/// Fade length when a voice is choked, muted by its group, or pushed out by
/// the pad's polyphony limit.
pub const KILL_FRAMES: u16 = 64;
/// Fade length of the previous sound when its voice is stolen for a new hit.
pub const STEAL_FRAMES: u16 = 32;
/// How long the step is smoothed over when an edit moves a loop behind a
/// playing voice and it jumps into the loop ([`Engine::edit_view`]).
pub const JUMP_FRAMES: u16 = 64;
/// Events that can wait for the next render.
pub const MAX_EVENTS: usize = 256;
/// The most kits one engine plays ([`Engine::with_kits`]): one for each of
/// the sequencer's 128 tracks.
pub const MAX_KITS: usize = 128;
/// What a hit on a pad plays.
#[derive(Clone, Copy)]
enum Target {
    /// The pad's own sound.
    Own(u8),
    /// A note, through the keygroups.
    Key(u8),
    /// Pad `pad` at 16 Levels step `k` of `by`.
    Level { pad: u8, by: Levels, k: u8 },
}

impl Target {
    /// Whether voice `v` was started by a hit of this pad.
    fn played(&self, v: &Voice) -> bool {
        match *self {
            // The pad's own voices and those of the pads it links to.
            Target::Own(pad) => v.by == pad && v.note == NO_NOTE,
            Target::Key(note) => v.note == note,
            Target::Level { pad, k, .. } => v.pad == pad && v.note == LEVEL_NOTE + k,
        }
    }
}

/// What a hit changes in the voices it starts: a keygroup's semitones, or
/// a 16 Levels step.
#[derive(Clone, Copy, Default)]
struct Vary {
    semis: i32,
    velocity: Option<u8>,
    /// From 0 to 1 of the region (a granular voice's grain position).
    start: Option<f32>,
    cutoff_hz: Option<f32>,
    decay_ms: Option<f32>,
    /// An audition: these source frames once, forward, without fades or a
    /// loop, instead of the layer's region.
    slice: Option<(i64, i64)>,
    /// The pad hit, when it is not the pad playing (a link).
    by: Option<u8>,
}

impl Vary {
    fn level(by: Levels, k: u8) -> Vary {
        let k = usize::from(k);
        let mut v = Vary::default();
        match by {
            Levels::Velocity => v.velocity = Some(Levels::velocity(k)),
            Levels::Tune => v.semis = Levels::semitones(k),
            Levels::Cutoff => v.cutoff_hz = Some(Levels::cutoff_hz(k)),
            Levels::Decay => v.decay_ms = Some(Levels::decay_ms(k)),
            Levels::Start => v.start = Some(Levels::start(k)),
        }
        v
    }
}

/// A voice's note when its pad was hit, not played as a keygroup.
const NO_NOTE: u8 = 255;

/// Cycle's turns count to this and start again: every set size from 1 to
/// 16 layers divides it, so each set keeps its order across the wrap.
const TURNS: u32 = 720_720;
/// A 16 Levels voice's note: this plus the pad (0 to 15) that played it.
const LEVEL_NOTE: u8 = 128;
/// Grains alive at once across every granular voice. Past it, new grains
/// are skipped. A first guess, to be set by measurement on the MPC.
pub const GRAIN_BUDGET: usize = 256;
/// Seconds of output the master effect remembers for its grains.
pub const MASTER_MEMORY_S: f32 = 8.0;
/// Granular voices render their grains in chunks of this many frames.
const GRAIN_CHUNK: usize = 256;
/// Split ([`Engine::split`]), the lane's voices render at most this many
/// frames at a time, each into its own pair (longer blocks go in parts).
pub const LANE_BLOCK: usize = 1024;
/// Parts of a block shorter than this (between two events close together)
/// play the lane's voices on the audio thread: handing them over would
/// cost more than it saves.
pub const LANE_MIN_FRAMES: usize = 16;
/// Filter coefficients, and anything pressure moves, are recomputed every
/// this many frames.
const FILTER_STEP: u32 = 16;
/// Pressure is smoothed with this time constant, so steps between pressure
/// messages don't zipper.
const PRESSURE_SMOOTH_S: f32 = 0.005;
/// How fast playing voices follow a live setting ([`Event::PadLive`],
/// [`Event::ProgramLive`]): a one-pole glide with this time constant,
/// stepped every [`FILTER_STEP`] frames (linear within each step for the
/// gain).
const LIVE_SMOOTH_S: f32 = 0.010;
/// The filter a live cutoff gives a pad without one: a 2-pole low-pass,
/// starting open (the filter clamps the cutoff below half the rate).
const OPEN_FILTER: Filter = Filter {
    kind: FilterKind::LowPass,
    poles: 2,
    cutoff_hz: 22_000.0,
    resonance: 0.0,
    env_octaves: 0.0,
};
const ONE: i64 = 1 << 32;

/// The step past which a voice takes the band-limited read: two semitones
/// up (and a hair more, so a pad tuned exactly two up stays put whatever
/// the rounding). Up to it, the highest a 44.1 kHz sound holds
/// (22.05 kHz) moves to under 24.75 kHz and folds back no lower than
/// 19.35 kHz, above hearing; the band-limited read costs several times the
/// Hermite read, so a voice detuned a little, bent or swept by vibrato
/// doesn't pay for it.
pub const CLEAN_ABOVE: i64 = 4_820_937_788 + (1 << 16);

/// A voice's [`Voice::bl_above`]: never, unless it is started clean.
#[derive(Clone, Copy, Debug)]
struct BlAbove(i64);

impl Default for BlAbove {
    fn default() -> BlAbove {
        BlAbove(i64::MAX)
    }
}

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
struct Mixed<'a> {
    left: &'a mut [f32],
    right: &'a mut [f32],
}

impl PadOut for Mixed<'_> {
    fn pad(&mut self, _kit: usize, _pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        Some((&mut *self.left, &mut *self.right))
    }
}

#[derive(Clone, Debug)]
pub struct ViewLoop {
    start: i64,
    end: i64,
    crossfade: i64,
    ping_pong: bool,
}

/// A layer resolved into plain numbers and a handle to its audio.
#[derive(Clone, Debug)]
pub struct ViewLayer {
    data: Pcm,
    stereo: bool,
    pick: ChannelPick,
    start: i64,
    end: i64,
    sustain_loop: Option<ViewLoop>,
    fade_in: Fade,
    fade_out: Fade,
    reverse: bool,
    mode: PlayMode,
    gain_l: f32,
    gain_r: f32,
    /// The region's and the layer's gain, without the pad's level and pan,
    /// and the layer's own pan: live Level and Pan start from these.
    unit_gain: f32,
    pan: f32,
    /// Source frames per output frame, 32.32 fixed point.
    step: i64,
    vel_lo: u8,
    vel_hi: u8,
    src_rate: u32,
    /// Grain settings with the layer's tune and reverse folded in.
    grain: mpfree_grain::Params,
    /// What a key or a tune moves ([`pitched_grain`]): a Tone region's
    /// density (its note), or the grains' pitch and, with the region's key
    /// tracking, their size and the scan.
    follow: KeyFollow,
    /// Frames a granular voice takes to glide to a new key (0, none).
    glide: u32,
    /// The tempo the layer's audio is at, when it is warped (0 if not):
    /// the voice plays it `tempo / warp_bpm` times as fast.
    warp_bpm: f64,
    /// It plays a stretched copy, not the source.
    copy: bool,
    /// For a stretched copy: where each source frame lands in it.
    source_at: Option<CopyMap>,
    /// The sound's octave copies, for a clean read past an octave up
    /// (none for a Vintage pad, a granular region or a stretched copy).
    octaves: Option<Arc<OctaveCell>>,
    /// The sound's hits, for a region in the Stretch grain mode
    /// ([`crate::hits`]).
    hits: Option<Arc<HitCell>>,
}

impl ViewLayer {
    /// Whether the layer plays a stretched copy (and how long it is).
    pub fn stretched_frames(&self) -> Option<i64> {
        (self.warp_bpm > 0.0 && self.copy).then_some(self.end - self.start)
    }
}

#[derive(Clone, Debug)]
pub struct ViewPad {
    layers: Vec<ViewLayer>,
    select: LayerSelect,
    polyphony: u8,
    mute_group: Option<u8>,
    chokes: Vec<usize>,
    links: Vec<usize>,
    latch: bool,
    amp: Ahdsr,
    filter: Filter,
    filter_env: Ahdsr,
    vel_to_level: f32,
    vel_to_start: f32,
    pressure: PressureRoute,
    keys: Option<Keys>,
    wow: f32,
    key_fade: u8,
    vel_fade: u8,
    /// The pad's own level, pan and tune, for live settings to replace.
    level_db: f32,
    pan: f32,
    tune_cents: i32,
    /// Pitched as older samplers do: no band-limited read.
    vintage: bool,
}

/// What the audio thread needs from the project, for every pad.
#[derive(Clone, Debug)]
pub struct EngineView {
    pub revision: u64,
    pads: Vec<Option<ViewPad>>,
    master: MasterFx,
    pads_play: PadsPlay,
}

impl EngineView {
    /// Whether a hit on pad `i` plays something.
    pub fn has_pad(&self, i: usize) -> bool {
        let sounds = |p: &Option<ViewPad>| p.as_ref().is_some_and(|p| !p.layers.is_empty());
        match self.pads_play {
            PadsPlay::Pads => self.pads.get(i).is_some_and(sounds),
            // With the pads playing notes: a keygroup holds its note.
            PadsPlay::Keys { from } => {
                let note = usize::from(from) + i;
                note < 128
                    && self.pads.iter().any(|p| {
                        sounds(p)
                            && p.as_ref()
                                .and_then(|p| p.keys)
                                .is_some_and(|k| k.contains(note as u8))
                    })
            }
            // With 16 Levels: every pad plays the one pad.
            PadsPlay::Levels { pad, .. } => self.pads.get(usize::from(pad)).is_some_and(sounds),
        }
    }

    /// What the pads play.
    pub fn pads_play(&self) -> PadsPlay {
        self.pads_play
    }

    /// Pad `pad`'s own value of `field`, as edited: what [`Event::PadLive`]
    /// with NaN goes back to, so a caller can give its changes as offsets
    /// on it. Cutoff and Resonance only where the pad has a filter; None for
    /// a pad without a sound.
    pub fn pad_own(&self, pad: usize, field: PadField) -> Option<f32> {
        let p = self
            .pads
            .get(pad)?
            .as_ref()
            .filter(|p| !p.layers.is_empty())?;
        let filtered = p.filter.kind != FilterKind::Off;
        match field {
            PadField::Level => Some(p.level_db),
            PadField::Pan => Some(p.pan),
            PadField::Tune => Some(p.tune_cents as f32),
            PadField::Cutoff => filtered.then_some(p.filter.cutoff_hz),
            PadField::Resonance => filtered.then_some(p.filter.resonance),
            PadField::Attack => Some(p.amp.attack_ms),
            PadField::Decay => Some(p.amp.decay_ms),
            PadField::Release => Some(p.amp.release_ms),
        }
    }

    /// Layer `layer` of pad `pad`, as resolved.
    pub fn layer(&self, pad: usize, layer: usize) -> Option<&ViewLayer> {
        self.pads.get(pad)?.as_ref()?.layers.get(layer)
    }

    pub fn empty() -> EngineView {
        EngineView {
            revision: 0,
            pads: vec![None; PADS],
            master: MasterFx::default(),
            pads_play: PadsPlay::Pads,
        }
    }

    /// Resolves every pad of a project for an engine running at `rate`.
    /// Call this off the audio thread; it allocates. Warped regions play
    /// as they were recorded; [`EngineView::build_warped`] plays them at
    /// the project tempo.
    pub fn build(project: &Project, rate: u32) -> EngineView {
        Self::resolve(project, rate, None)
    }

    /// As [`EngineView::build`], with warped regions at the tempo of
    /// `warps`: a region warped to stretch plays its stretched copy when
    /// `warps` holds it, and repitches to the tempo until then; one warped
    /// to repitch always repitches. The engine needs the same tempo
    /// ([`Engine::set_tempo`]).
    pub fn build_warped(project: &Project, rate: u32, warps: &mut Warps) -> EngineView {
        Self::resolve(project, rate, Some(warps))
    }

    fn resolve(project: &Project, rate: u32, mut warps: Option<&mut Warps>) -> EngineView {
        let mut pads = Vec::with_capacity(PADS);
        for i in 0..PADS {
            pads.push(project.pad(i).map(|p| {
                let pad_gain = db_to_gain(p.level_db);
                let layers = p
                    .layers
                    .iter()
                    .filter_map(|l| {
                        let s = project.source(l.region.source)?;
                        let r = &l.region;
                        let (pl, pr) = pan_gains(l.pan + p.pan);
                        let unit_gain = db_to_gain(r.gain_db) * db_to_gain(l.level_db);
                        let g = unit_gain * pad_gain;
                        let cents = f64::from(r.tune_cents + l.tune_cents + p.tune_cents);
                        let ratio = f64::from(s.rate()) / f64::from(rate) * (cents / 1200.0).exp2();
                        let follow = KeyFollow {
                            tone: r.grain_mode == GrainMode::Tone,
                            size: r.grain_keys.size,
                            scan: r.grain_keys.scan,
                        };
                        let mut grain = pitched_grain(r.grain, (cents / 100.0) as f32, follow);
                        // A stretch of a sound with a tempo keeps time.
                        if r.grain_mode == GrainMode::Stretch {
                            grain.scan_tempo = r.warp.as_ref().map(|w| w.bpm);
                        }
                        if r.reverse {
                            grain.reverse = 1.0;
                        }
                        let mut v = ViewLayer {
                            data: s.data().clone(),
                            stereo: s.channels() == 2,
                            pick: r.channels,
                            start: r.start as i64,
                            end: r.end as i64,
                            sustain_loop: r.sustain_loop.map(|lp| ViewLoop {
                                start: lp.start as i64,
                                end: lp.end as i64,
                                crossfade: i64::from(lp.crossfade),
                                ping_pong: lp.mode == LoopMode::PingPong,
                            }),
                            fade_in: r.fade_in,
                            fade_out: r.fade_out,
                            reverse: r.reverse,
                            mode: r.mode,
                            gain_l: g * pl,
                            gain_r: g * pr,
                            unit_gain,
                            pan: l.pan,
                            step: ((ratio * ONE as f64).round() as i64).max(1),
                            vel_lo: l.vel_lo,
                            vel_hi: l.vel_hi,
                            src_rate: s.rate(),
                            grain,
                            follow,
                            glide: (r.grain_glide_ms.clamp(0.0, GRAIN_GLIDE_MAX_MS) / 1000.0
                                * rate as f32) as u32,
                            warp_bpm: 0.0,
                            copy: false,
                            source_at: None,
                            octaves: None,
                            hits: None,
                        };
                        if r.mode == PlayMode::Granular && r.grain_mode == GrainMode::Stretch {
                            hits::prepare(s.hits(), s.data(), s.channels(), s.rate());
                            v.hits = Some(s.hits().clone());
                        }
                        if !p.vintage {
                            v.octaves = Some(s.octaves().clone());
                            // Grains go as far up as their pitch and its
                            // spray take them.
                            let up = if r.mode == PlayMode::Granular {
                                let g = &r.grain;
                                let semis = g.pitch.max(0.0) + g.pitch_spray.clamp(0.0, 24.0);
                                ratio * f64::from(semis / 12.0).exp2()
                            } else {
                                ratio
                            };
                            if may_pass_an_octave(p, up, project.pads_play()) {
                                bandlimit::prepare(s.octaves(), s.data(), s.channels());
                            }
                        }
                        let warp = r.warp.as_ref().filter(|_| r.mode != PlayMode::Granular);
                        if let (Some(w), Some(ws)) = (warp, warps.as_deref_mut()) {
                            // Tune is in the copy, or left out (repitch):
                            // the tempo alone sets the speed.
                            let plain = f64::from(s.rate()) / f64::from(rate);
                            v.step = ((plain * ONE as f64).round() as i64).max(1);
                            v.warp_bpm = f64::from(w.bpm);
                            let copy = match w.method {
                                WarpMethod::Stretch => ws
                                    .key_for(r, l.tune_cents + p.tune_cents)
                                    .and_then(|k| ws.copy(&k).map(|d| (k, d))),
                                WarpMethod::Repitch => None,
                            };
                            if let Some((k, data)) = copy {
                                let scale = k.scale();
                                let map = CopyMap::new(w, s.rate(), &k);
                                let at = |x: u64| map.at(x as f64).round() as i64;
                                let len = k.frames as i64;
                                v.data = Pcm::F32(data);
                                v.start = 0;
                                v.end = len;
                                v.sustain_loop = r.sustain_loop.map(|lp| {
                                    let (a, b) = (at(lp.start), at(lp.end).min(len));
                                    ViewLoop {
                                        start: a,
                                        end: b.max(a + 1),
                                        crossfade: ((f64::from(lp.crossfade) * scale) as i64)
                                            .min((b - a) / 2)
                                            .min(a),
                                        ping_pong: lp.mode == LoopMode::PingPong,
                                    }
                                });
                                let fade = |f: Fade| Fade {
                                    frames: (f64::from(f.frames) * scale).round() as u32,
                                    ..f
                                };
                                v.fade_in = fade(r.fade_in);
                                v.fade_out = fade(r.fade_out);
                                if i64::from(v.fade_in.frames) + i64::from(v.fade_out.frames) > len
                                {
                                    (v.fade_in, v.fade_out) = (r.fade_in, r.fade_out);
                                }
                                // The copy is at the tempo it was made for.
                                v.warp_bpm *= (r.frames() as f64) / (len as f64);
                                v.copy = true;
                                v.source_at = Some(map);
                                // The copies are of the source, not this.
                                v.octaves = None;
                            }
                        }
                        Some(v)
                    })
                    .collect();
                ViewPad {
                    layers,
                    select: p.select,
                    polyphony: p.polyphony,
                    mute_group: p.mute_group,
                    chokes: p.chokes.clone(),
                    links: p
                        .links
                        .iter()
                        .copied()
                        .filter(|&l| l < PADS && l != i)
                        .collect(),
                    latch: p.latch,
                    amp: p.amp,
                    filter: p.filter,
                    filter_env: p.filter_env,
                    vel_to_level: p.vel_to_level.clamp(0.0, 1.0),
                    vel_to_start: p.vel_to_start.clamp(0.0, 1.0),
                    pressure: p.pressure,
                    keys: p.keys,
                    wow: p.wow.clamp(0.0, 1.0),
                    key_fade: p.key_fade.min(crate::model::MAX_KEY_FADE),
                    vel_fade: p.vel_fade.min(crate::model::MAX_VEL_FADE),
                    level_db: p.level_db,
                    pan: p.pan,
                    tune_cents: p.tune_cents,
                    vintage: p.vintage,
                }
            }));
        }
        EngineView {
            revision: project.revision(),
            pads,
            master: *project.master(),
            pads_play: project.pads_play(),
        }
    }
}

/// Whether a layer of pad `p` read `ratio` times its sound's rate (its
/// rate against the output's, and its tune) may be played more than an
/// octave faster than that sound, so its octave copies should be made: a
/// keygroup (its notes can reach anywhere), or tune, pitch pressure, 16
/// Levels on tune and wow together reaching twice the sound's speed.
fn may_pass_an_octave(p: &crate::model::Pad, ratio: f64, play: PadsPlay) -> bool {
    if p.keys.is_some() {
        return true;
    }
    let mut up = ratio;
    if p.pressure.target == PressureTarget::Pitch {
        up *= f64::from(p.pressure.depth.abs()).exp2();
    }
    if matches!(
        play,
        PadsPlay::Levels {
            by: Levels::Tune,
            ..
        }
    ) {
        up *= (7.0f64 / 12.0).exp2();
    }
    if p.wow > 0.0 {
        up *= 1.02;
    }
    up >= 1.9
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    NoteOn {
        pad: u8,
        velocity: u8,
    },
    NoteOff {
        pad: u8,
    },
    /// Pressure on a held pad, 0 to 127 (polyphonic aftertouch). Moves the
    /// pad's pressure target on its playing voices, smoothed over about 5 ms.
    Pressure {
        pad: u8,
        value: u8,
    },
    /// A note through the keygroups: every pad whose [`Keys`] hold it
    /// plays, a semitone higher for each note above its root.
    Key {
        note: u8,
        velocity: u8,
    },
    KeyOff {
        note: u8,
    },
    /// Releases every voice (gate-style), e.g. on STOP.
    AllOff,
    /// Freezes the master effect (on) or lets it take in new audio again
    /// (off). Frozen, its grains keep playing the audio it holds while
    /// the pads carry on dry. Does nothing without a master effect.
    Freeze {
        on: bool,
    },
    /// Tape stop (`stop`) or start: every voice slows to a halt, or comes
    /// back up to speed, over `ms` milliseconds, as a tape machine's
    /// motor would; the sound fades out as it nears a halt. Stopped, new
    /// hits wait silent until the tape starts again.
    Tape {
        stop: bool,
        ms: u16,
    },
    /// Plays source frames `start..end` of layer `layer` of pad `pad` once,
    /// forward from its first frame, with the pad's settings (level, pan,
    /// tune, channels, envelopes, filter, pressure), as a pad made from
    /// that slice plays it: the Chop tab's audition of a slice. The pad's
    /// own playing voices fade out first; mute groups and chokes are left
    /// alone. `velocity` as for [`Event::NoteOn`]. The frames are clipped to
    /// the source; nothing plays if none are left.
    Audition {
        pad: u8,
        layer: u8,
        start: u32,
        end: u32,
        velocity: u8,
    },
    /// Changes one of pad `pad`'s settings while it plays, as a knob, a
    /// p-lock or a modulation does: `value` replaces the pad's own until
    /// the next change (NaN gives the pad's own back) or a new view
    /// ([`Engine::set_view`]). New voices start with it; playing voices
    /// follow the fields [`PadField::follows`] says, gliding there. Values
    /// out of range are clamped to it ([`PadField::clamp`]).
    PadLive {
        pad: u8,
        field: PadField,
        value: f32,
    },
    /// A step's parameter lock on pad `pad` (the sequencer's
    /// `Event::Lock` on a pad's sound): `value` takes the place of the
    /// pad's own `field` until the next lock or its end (NaN), as the
    /// player sends them on their frames. Under a live value
    /// ([`Event::PadLive`], the scene crossfader), which still goes over
    /// it; a new view keeps it. New voices start with it and playing
    /// voices follow it as they follow a live value, so a lock with no
    /// note moves the sound still ringing. A locked Decay also makes the
    /// sound decay to silence (the pad's sustain at 0), as the Play page's
    /// Decay knob does, so it shortens a sound that plays out.
    PadLock {
        pad: u8,
        field: PadField,
        value: f32,
    },
    /// Moves every pad at once by an offset on its own setting (or on its
    /// live one): see [`ProgramField`]. 0 or NaN is no offset. New voices
    /// start with it and playing voices follow every field. It stays when
    /// a new view comes in.
    ProgramLive {
        field: ProgramField,
        value: f32,
    },
    /// Live chop on pad `pad`'s sound (`Some`), or off (`None`): while
    /// it is on and the pad's audition plays, [`Event::Mark`] notes where
    /// the audition is in its source ([`Engine::mark`]). Turned off, the
    /// audition stops (fading out); it ends by itself when the audition
    /// does ([`Engine::marking`]).
    LiveChop {
        pad: Option<u8>,
    },
    /// While live chop is on: a cut where the audition is now, to the
    /// frame. Nothing plays.
    Mark,
}

/// The cuts live chop keeps for reading ([`Engine::mark`]).
pub const MAX_MARKS: usize = 64;

/// A pad setting [`Event::PadLive`] changes while the pad plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadField {
    /// The pad's level, in dB (-120 to +24): replaces its own Level.
    Level = 0,
    /// The pad's pan, -1 (left) to +1 (right); each layer's own pan adds
    /// to it, as it does to the pad's own.
    Pan = 1,
    /// The pad's tune in cents (-4800 to +4800), for new voices. A warped
    /// layer keeps its pitch (its tune is in its stretched copy).
    Tune = 2,
    /// The filter's cutoff in Hz (10 to 22 000). A pad whose filter is off
    /// gets a 2-pole low-pass. It also takes the place of 16 Levels'
    /// cutoff.
    Cutoff = 3,
    /// The filter's resonance, 0 to 1 (only where there is a filter).
    Resonance = 4,
    /// The amp envelope's attack, decay and release in ms (0 to 60 000),
    /// for new voices.
    Attack = 5,
    Decay = 6,
    Release = 7,
}

impl PadField {
    pub const COUNT: usize = 8;
    pub const ALL: [PadField; PadField::COUNT] = [
        PadField::Level,
        PadField::Pan,
        PadField::Tune,
        PadField::Cutoff,
        PadField::Resonance,
        PadField::Attack,
        PadField::Decay,
        PadField::Release,
    ];

    /// Whether playing voices follow it (Level, Pan, Cutoff, Resonance);
    /// the others reach new voices only.
    pub fn follows(self) -> bool {
        matches!(
            self,
            PadField::Level | PadField::Pan | PadField::Cutoff | PadField::Resonance
        )
    }

    /// `x` within the field's range (NaN stays NaN).
    pub fn clamp(self, x: f32) -> f32 {
        let (lo, hi) = match self {
            PadField::Level => (-120.0, 24.0),
            PadField::Pan => (-1.0, 1.0),
            PadField::Tune => (-4800.0, 4800.0),
            PadField::Cutoff => (10.0, 22_000.0),
            PadField::Resonance => (0.0, 1.0),
            PadField::Attack | PadField::Decay | PadField::Release => (0.0, 60_000.0),
        };
        x.clamp(lo, hi)
    }
}

/// An offset on every pad's own setting, from [`Event::ProgramLive`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProgramField {
    /// dB added to every pad's level (-120 to +24).
    Level = 0,
    /// Cents added to every pad's tune (-4800 to +4800); playing voices
    /// glide there too. Warped layers keep their pitch.
    Tune = 1,
    /// Octaves the cutoff of every filter moves (-10 to +10). Below 0, a
    /// pad without a filter gets a 2-pole low-pass, so the whole kit
    /// darkens.
    Cutoff = 2,
}

impl ProgramField {
    pub const COUNT: usize = 3;
    pub const ALL: [ProgramField; ProgramField::COUNT] = [
        ProgramField::Level,
        ProgramField::Tune,
        ProgramField::Cutoff,
    ];

    /// `x` within the field's range, NaN as 0 (no offset).
    pub fn clamp(self, x: f32) -> f32 {
        if x.is_nan() {
            return 0.0;
        }
        let (lo, hi) = match self {
            ProgramField::Level => (-120.0, 24.0),
            ProgramField::Tune => (-4800.0, 4800.0),
            ProgramField::Cutoff => (-10.0, 10.0),
        };
        x.clamp(lo, hi)
    }
}

/// The live settings: each pad's [`PadField`]s (NaN where the pad's own
/// applies), its step locks under them, and the program's offsets.
#[derive(Clone, Copy, Debug)]
struct Live {
    pads: [[f32; PadField::COUNT]; PADS],
    locks: [[f32; PadField::COUNT]; PADS],
    program: [f32; ProgramField::COUNT],
}

impl Live {
    const OWN: [[f32; PadField::COUNT]; PADS] = [[f32::NAN; PadField::COUNT]; PADS];

    /// Pad `p`'s settings in place of its own: each field's live value,
    /// else its lock (NaN where neither is set).
    fn row(&self, p: usize) -> [f32; PadField::COUNT] {
        let (Some(live), Some(locks)) = (self.pads.get(p), self.locks.get(p)) else {
            return [f32::NAN; PadField::COUNT];
        };
        let mut out = *live;
        for (o, l) in out.iter_mut().zip(locks) {
            if o.is_nan() {
                *o = *l;
            }
        }
        out
    }
}

/// What a playing voice follows of the live settings: where its gain,
/// filter and tune are going, the steps it takes there, and the pad's own
/// settings from when it started (a voice finishes as it started, whatever
/// view comes in).
#[derive(Clone, Copy, Debug, Default)]
struct Follow {
    /// Still gliding to a target.
    moving: bool,
    /// Frames into the current step of [`FILTER_STEP`] frames.
    count: u32,
    /// The gain (left, right, before the tape's fade) it is going to, and
    /// the step a frame.
    gain_to: [f32; 2],
    gain_inc: [f32; 2],
    /// The cutoff (Hz) and resonance its filter is going to.
    fc_to: f32,
    res_to: f32,
    /// The program's tune in cents: in use, and where it is going.
    cents: f32,
    cents_to: f32,
    /// The one-pole step per [`FILTER_STEP`] frames.
    k: f32,
    /// The pad's own level, pan, cutoff and resonance when it started (the
    /// cutoff of 16 Levels if it set one; an open filter's if none).
    own_db: f32,
    own_pan: f32,
    own_fc: f32,
    own_res: f32,
}

#[derive(Clone, Debug, Default)]
struct Voice {
    active: bool,
    /// The kit whose pad started it ([`Engine::with_kits`]).
    kit: u8,
    pad: u8,
    layer: u8,
    data: Option<Pcm>,
    stereo: bool,
    pick_l: u8,
    pick_r: u8,
    pos: i64,
    step: i64,
    backwards: bool,
    reverse: bool,
    start: i64,
    end: i64,
    loop_start: i64,
    loop_end: i64,
    crossfade: i64,
    ping_pong: bool,
    looping: bool,
    fade_in: Option<Fade>,
    fade_out: Option<Fade>,
    gain_l: f32,
    gain_r: f32,
    mode: Option<PlayMode>,
    held: bool,
    age: u64,
    amp: Option<Envelope>,
    fenv: Option<Envelope>,
    filter: Option<Filter>,
    svf: [Svf; 2],
    filter_count: u32,
    /// The envelope level and pressure factor (as bits) the filter was
    /// last set for.
    filter_key: Option<(u32, u32)>,
    kill_left: u16,
    tail_left: u16,
    tail_l: f32,
    tail_r: f32,
    last_l: f32,
    last_r: f32,
    /// About how loud it is now, for load shedding: the louder of its last
    /// two output samples at the end of each render, held and falling a
    /// little each render (so a peak, not a zero crossing). Starts at its
    /// gain.
    heard: f32,
    /// Plays through the engine's grain voice in the same slot.
    granular: bool,
    src_rate: u32,
    /// Grain settings before pressure moves them.
    grain_base: GrainParams,
    /// What a key moves ([`ViewLayer::follow`]), and the semitones its hit
    /// added (a keygroup's, a live tune's), kept for a live edit and a
    /// glide; of those, the hit's key (a keygroup's, 16 Levels').
    grain_follow: KeyFollow,
    grain_hit: f32,
    grain_key: i32,
    /// Gliding to another key ([`ViewLayer::glide`]): where `grain_hit`
    /// goes, how far it moves a frame, and the frames left.
    glide_to: f32,
    glide_step: f32,
    glide_left: u32,
    route: PressureRoute,
    /// Pressure asked for and the smoothed value in use, 0 to 1.
    press_target: f32,
    press: f32,
    press_coef: f32,
    /// What pressure has made of the step, the gain and the cutoff. The
    /// gain glides to each new value over [`FILTER_STEP`] frames.
    base_step: i64,
    press_gain: f32,
    press_gain_inc: f32,
    press_fc: f32,
    press_count: u32,
    /// The layer's tempo when warped (0 if not), and its step before the
    /// tempo moves it.
    warp_bpm: f64,
    plain_step: i64,
    /// The note a keygroup voice plays ([`NO_NOTE`] for a pad hit).
    note: u8,
    /// The pad whose hit started it: its own pad, or the pad linking to it.
    by: u8,
    /// Its pad latches: lifting a pad or a key lets it play on.
    latch: bool,
    /// An audition (a slice, or the region for live chop).
    audition: bool,
    /// How fast it plays against its step: the tape's speed with wow and
    /// flutter (1 when neither moves).
    speed: f32,
    /// Its gain before the tape fades it.
    base_gain_l: f32,
    base_gain_r: f32,
    /// Wow and flutter, 0 to 1, and where its three wobbles are (cycles).
    wow: f32,
    wow_phase: [f32; 3],
    /// Its gain before the pad's level and pan (region, layer and
    /// velocity), its layer's own pan, and the tape's fade on its gain:
    /// live Level and Pan work from these.
    unit_gain: f32,
    layer_pan: f32,
    tape_gain: f32,
    /// The live settings it follows.
    follow: Follow,
    /// The program tune (cents) its grains were last set for.
    grain_cents: f32,
    /// Started as a plain one-shot ([`Engine::set_full_voices`]): it may
    /// take a voice past the full-voice limit.
    light: bool,
    /// The step past which it reads band-limited ([`Voice::bl`]):
    /// [`CLEAN_ABOVE`], or never for a Vintage pad or a granular voice.
    bl_above: BlAbove,
    /// Reads band-limited ([`bandlimit::read2`]), from the first frame its
    /// step passed [`Voice::bl_above`] on: once set, for the rest of its
    /// life, so its sound doesn't change as its pitch moves.
    bl: bool,
    /// Its sound's octave copies, when made by the time it started.
    octaves: Option<Arc<OctaveCell>>,
    /// A granular Stretch's hits, when found by the time it started.
    hits: Option<Arc<HitCell>>,
    /// A granular voice's grains read band-limited when pitched up
    /// ([`GrainClip::clean`]): not on a Vintage pad.
    grain_clean: bool,
    /// The step the band-limited read was last aimed for ([`aim_bl`]),
    /// the octave copy it reads (0: the sound) and the sinc's stretch.
    bl_step: i64,
    bl_octave: u32,
    bl_reach: Reach,
    /// What it follows of an edit ([`Engine::edit_view`]): the hit's own
    /// gain (velocity, 16 Levels) under the layer's, whether its filter's
    /// cutoff is the pad's (not one 16 Levels set), and the layer's step
    /// and grain settings it plays from now.
    hit_gain: f32,
    own_cut: bool,
    layer_step: i64,
    layer_grain: GrainParams,
    /// A jump into a moved loop: what the read was minus what it is now,
    /// fading out over [`JUMP_FRAMES`] frames so the sound doesn't step.
    jump: [f32; 2],
    jump_left: u16,
}

impl Voice {
    #[inline]
    fn at<S: Sample>(&self, data: &[S], i: i64, ch: u8) -> f32 {
        let i = i.clamp(self.start, self.end - 1) as usize;
        if self.stereo {
            data[2 * i + usize::from(ch)].f()
        } else {
            data[i].f()
        }
    }

    /// One channel at a fixed-point position, Hermite-interpolated, with
    /// every neighbour clamped to the region: the edge case of [`Voice::read2`].
    #[cold]
    #[inline(never)]
    fn read<S: Sample>(&self, data: &[S], pos: i64, pick: u8) -> f32 {
        let i = pos >> 32;
        let t = frac(pos);
        let ch = |k: i64| -> f32 {
            match pick {
                0 => self.at(data, k, 0),
                1 => self.at(data, k, 1),
                _ => 0.5 * (self.at(data, k, 0) + self.at(data, k, 1)),
            }
        };
        if t == 0.0 {
            return ch(i);
        }
        hermite(ch(i - 1), ch(i), ch(i + 1), ch(i + 2), t)
    }

    /// Both output channels at a fixed-point position, Hermite-interpolated:
    /// what [`Voice::read`] gives for `pick_l` and `pick_r`, bit for bit.
    /// Away from the region's edges (nearly always) the four neighbours are
    /// read straight from the data, once for both channels, with no clamps.
    #[inline(always)]
    fn read2<S: Sample>(&self, data: &[S], pos: i64) -> (f32, f32) {
        let i = pos >> 32;
        if i - 1 < self.start || i + 2 >= self.end {
            return (
                self.read(data, pos, self.pick_l),
                self.read(data, pos, self.pick_r),
            );
        }
        let t = frac(pos);
        let k = (i - 1) as usize;
        if !self.stereo {
            // Mono plays the same on both sides.
            let w: &[S; 4] = data[k..k + 4].try_into().expect("four frames");
            let w = w.map(S::f);
            let x = if t == 0.0 {
                w[1]
            } else {
                hermite(w[0], w[1], w[2], w[3], t)
            };
            return (x, x);
        }
        let w: &[S; 8] = data[2 * k..2 * k + 8].try_into().expect("four frames");
        let w = w.map(S::f);
        let side = |pick: u8| -> [f32; 4] {
            match pick {
                0 => [w[0], w[2], w[4], w[6]],
                1 => [w[1], w[3], w[5], w[7]],
                _ => [
                    0.5 * (w[0] + w[1]),
                    0.5 * (w[2] + w[3]),
                    0.5 * (w[4] + w[5]),
                    0.5 * (w[6] + w[7]),
                ],
            }
        };
        let (a, b) = (side(self.pick_l), side(self.pick_r));
        if t == 0.0 {
            return (a[1], b[1]);
        }
        (
            hermite(a[0], a[1], a[2], a[3], t),
            hermite(b[0], b[1], b[2], b[3], t),
        )
    }
}

/// The fraction of a 32.32 fixed-point position, 0 to 1. Converted from
/// its low 32 bits as a `u32`, which ARMv7 does in one instruction (a
/// 64-bit integer needs a library call); exact either way.
#[inline(always)]
fn frac(pos: i64) -> f32 {
    (pos as u32) as f32 * (1.0 / ONE as f32)
}

fn picks(stereo: bool, pick: ChannelPick) -> (u8, u8) {
    if !stereo {
        return (0, 0);
    }
    match pick {
        ChannelPick::Stereo => (0, 1),
        ChannelPick::MonoSum => (2, 2),
        ChannelPick::Left => (0, 0),
        ChannelPick::Right => (1, 1),
    }
}

/// A share of the voice pool, with their grain voices, grain budget and
/// buffers. The engine has one; split ([`Engine::split`]), a second one
/// renders on a lane.
struct Pool {
    voices: Vec<Voice>,
    /// One grain voice per voice slot, used while that slot is granular.
    grains: Vec<GrainVoice>,
    budget: Budget,
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
    /// Where the voices of a pad with no output play, so they end on time.
    unheard_l: Vec<f32>,
    unheard_r: Vec<f32>,
    /// The lane's share only: each voice's sound for the part of the block
    /// it renders ([`LANE_BLOCK`] frames a voice), and the voices that
    /// played in it.
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    played: Vec<bool>,
}

/// What a playing voice costs to render, in grains (see [`Pool::cost`]).
/// A rough figure from the workstation bench, where a sample voice (read,
/// envelope, filter) costs about as much as 8 to 16 live grains; the
/// device bench may move it.
const VOICE_COST: usize = 8;
/// What a plain one-shot costs in the same units: the drum kit bench
/// cases put a plain voice at about a fifth of a full one.
const LIGHT_COST: usize = 2;

impl Pool {
    /// About what this share costs to render now: its playing voices and
    /// their live grains. A new voice goes to the share that costs less, so
    /// two granular clouds don't pile up on one core.
    fn cost(&self) -> usize {
        let cost = |v: &Voice| if v.light { LIGHT_COST } else { VOICE_COST };
        self.voices
            .iter()
            .filter(|v| v.active)
            .map(cost)
            .sum::<usize>()
            + self.budget.live()
    }

    /// `n` voices, the first of them slot `first` of the whole pool (it
    /// seeds their grain voices). `lane`: with the buffers the lane's share
    /// renders into.
    fn new(rate: u32, first: usize, n: usize, lane: bool) -> Pool {
        let out = if lane { n * LANE_BLOCK } else { 0 };
        Pool {
            voices: vec![Voice::default(); n],
            grains: (first..first + n)
                .map(|i| GrainVoice::new(rate, i as u64))
                .collect(),
            budget: Budget::new(GRAIN_BUDGET),
            scratch_l: vec![0.0; GRAIN_CHUNK],
            scratch_r: vec![0.0; GRAIN_CHUNK],
            unheard_l: vec![0.0; GRAIN_CHUNK],
            unheard_r: vec![0.0; GRAIN_CHUNK],
            out_l: vec![0.0; out],
            out_r: vec![0.0; out],
            played: vec![false; if lane { n } else { 0 }],
        }
    }

    /// Frames `a..b` of the block (`frame` its first) of every voice, each
    /// into its pad's pair from `out`, or played unheard.
    fn render_into(&mut self, a: usize, b: usize, frame: u64, rate: f32, out: &mut impl PadOut) {
        let Pool {
            voices,
            grains,
            budget,
            scratch_l,
            scratch_r,
            unheard_l,
            unheard_r,
            ..
        } = self;
        for (v, g) in voices.iter_mut().zip(grains.iter_mut()) {
            if !v.active {
                continue;
            }
            match out.pad(usize::from(v.kit), usize::from(v.pad)) {
                Some((l, r)) => play(
                    v,
                    g,
                    budget,
                    frame,
                    rate,
                    &mut l[a..b],
                    &mut r[a..b],
                    scratch_l,
                    scratch_r,
                ),
                None => {
                    let mut k = a;
                    while k < b && v.active {
                        let m = (b - k).min(unheard_l.len());
                        let (ul, ur) = (&mut unheard_l[..m], &mut unheard_r[..m]);
                        ul.fill(0.0);
                        ur.fill(0.0);
                        play(
                            v,
                            g,
                            budget,
                            frame + (k - a) as u64,
                            rate,
                            ul,
                            ur,
                            scratch_l,
                            scratch_r,
                        );
                        k += m;
                    }
                }
            }
        }
    }

    /// The lane's share: `frames` frames from `frame` of every voice, each
    /// into its own pair in `out_l` and `out_r`.
    fn render_own(&mut self, frames: usize, frame: u64, rate: f32) {
        let Pool {
            voices,
            grains,
            budget,
            scratch_l,
            scratch_r,
            out_l,
            out_r,
            played,
            ..
        } = self;
        for (k, (v, g)) in voices.iter_mut().zip(grains.iter_mut()).enumerate() {
            played[k] = v.active;
            if !v.active {
                continue;
            }
            let at = k * LANE_BLOCK;
            let (l, r) = (&mut out_l[at..at + frames], &mut out_r[at..at + frames]);
            l.fill(0.0);
            r.fill(0.0);
            play(v, g, budget, frame, rate, l, r, scratch_l, scratch_r);
        }
    }

    /// Adds what [`Pool::render_own`] rendered into frames `a..a + frames`
    /// of each voice's pad pair, in slot order.
    fn add_own(&self, a: usize, frames: usize, out: &mut impl PadOut) {
        for (k, v) in self.voices.iter().enumerate() {
            if !self.played[k] {
                continue;
            }
            if let Some((l, r)) = out.pad(usize::from(v.kit), usize::from(v.pad)) {
                let at = k * LANE_BLOCK;
                for (o, x) in l[a..a + frames]
                    .iter_mut()
                    .zip(&self.out_l[at..at + frames])
                {
                    *o += *x;
                }
                for (o, x) in r[a..a + frames]
                    .iter_mut()
                    .zip(&self.out_r[at..at + frames])
                {
                    *o += *x;
                }
            }
        }
    }
}

/// The whole voice pool: slots `0..a.voices.len()` in `a`, the rest in `b`
/// once split. `b` is away (None) while the lane renders it.
struct Voices {
    a: Pool,
    b: Option<Box<Pool>>,
    /// The lane share's grain limit when it left.
    b_limit: usize,
    /// The lane share's skipped grains when it left, so the count does not
    /// go back while it is away ([`Engine::stats`]).
    b_skipped: u64,
    split: bool,
}

impl Voices {
    /// Every voice here with its grain voice, in slot order.
    fn iter(&self) -> impl Iterator<Item = (&Voice, &GrainVoice)> + '_ {
        let b = self.b.iter().flat_map(|p| p.voices.iter().zip(&p.grains));
        self.a.voices.iter().zip(&self.a.grains).chain(b)
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = (&mut Voice, &mut GrainVoice)> + '_ {
        let b = self
            .b
            .iter_mut()
            .flat_map(|p| p.voices.iter_mut().zip(p.grains.iter_mut()));
        self.a
            .voices
            .iter_mut()
            .zip(self.a.grains.iter_mut())
            .chain(b)
    }

    fn voice(&self, slot: usize) -> Option<&Voice> {
        let n = self.a.voices.len();
        match slot.checked_sub(n) {
            None => self.a.voices.get(slot),
            Some(k) => self.b.as_ref()?.voices.get(k),
        }
    }

    fn voice_mut(&mut self, slot: usize) -> Option<&mut Voice> {
        let n = self.a.voices.len();
        match slot.checked_sub(n) {
            None => self.a.voices.get_mut(slot),
            Some(k) => self.b.as_mut()?.voices.get_mut(k),
        }
    }

    fn grain_mut(&mut self, slot: usize) -> Option<&mut GrainVoice> {
        let n = self.a.voices.len();
        match slot.checked_sub(n) {
            None => self.a.grains.get_mut(slot),
            Some(k) => self.b.as_mut()?.grains.get_mut(k),
        }
    }

    /// Silences slot `slot`'s grains, giving them back to its budget.
    fn kill_grain(&mut self, slot: usize) {
        let n = self.a.voices.len();
        match slot.checked_sub(n) {
            None => {
                if let Some(g) = self.a.grains.get_mut(slot) {
                    g.kill(&mut self.a.budget);
                }
            }
            Some(k) => {
                if let Some(p) = self.b.as_mut() {
                    let Pool { grains, budget, .. } = &mut **p;
                    if let Some(g) = grains.get_mut(k) {
                        g.kill(budget);
                    }
                }
            }
        }
    }

    /// A free slot for a new voice. Split, from the share playing fewer
    /// voices, so the two cores get about the same work.
    fn free_slot(&self) -> Option<usize> {
        let n = self.a.voices.len();
        let free_a = self.a.voices.iter().position(|v| !v.active);
        let Some(b) = self.b.as_ref() else {
            return free_a;
        };
        let free_b = b.voices.iter().position(|v| !v.active).map(|k| n + k);
        if b.cost() < self.a.cost() {
            free_b.or(free_a)
        } else {
            free_a.or(free_b)
        }
    }

    /// Shares the grain budget out before a part of a block: each share
    /// keeps its grains and gets half of what is left, so together they
    /// never pass [`GRAIN_BUDGET`].
    fn share_budget(&mut self) {
        if !self.split {
            return;
        }
        match self.b.as_mut() {
            Some(b) => {
                let (la, lb) = (self.a.budget.live(), b.budget.live());
                let free = GRAIN_BUDGET.saturating_sub(la + lb);
                self.a.budget.limit = la + free.div_ceil(2);
                b.budget.limit = lb + free / 2;
            }
            None => self.a.budget.limit = GRAIN_BUDGET.saturating_sub(self.b_limit),
        }
    }
}

/// What the lane runs: the second share of the voices, for one part of a
/// block.
struct Carrier {
    pool: Option<Box<Pool>>,
    frames: usize,
    frame: u64,
    rate: f32,
}

impl mpfree_lanes::Job for Carrier {
    fn run(&mut self, _block: u64) {
        if let Some(p) = self.pool.as_mut() {
            p.render_own(self.frames, self.frame, self.rate);
        }
    }
}

/// Counters for the debug page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub hits: u64,
    pub steals: u64,
    pub dropped_events: u64,
    pub peak_voices: usize,
    /// Grains refused because the grain budget was full.
    pub skipped_grains: u64,
    /// Split: parts of blocks the lane's voices were handed over for, and
    /// the ones it missed its deadline on (those voices are unheard for
    /// that part, and events wait until they are back).
    pub lane_parts: u64,
    pub lane_late: u64,
    /// Voices given up under load ([`Engine::cull`]).
    pub culls: u64,
}

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
    /// Split: the second share of the voices renders here, on another core.
    lane: Option<Lane<Carrier>>,
    /// (frame offset, kit, event).
    events: Vec<(u32, u8, Event)>,
    /// Each pad's turn for Cycle (see [`TURNS`]), [`PADS`] a kit.
    cycle: Vec<u32>,
    /// [`Engine::victim`]'s count of each kit's pads' held voices, all 0
    /// between calls, [`PADS`] a kit.
    held: Vec<u16>,
    /// Each pad's last granular key (its voice's semitones), which a
    /// gliding layer's next voice starts from, [`PADS`] a kit.
    last_key: Vec<Option<f32>>,
    rng: Rng,
    clock: u64,
    fixed_velocity: Option<u8>,
    stats: Stats,
    /// Frames rendered since the engine was made: the timeline synced grains use.
    frames: u64,
    master: Option<GrainFx>,
    /// The project tempo, for warped layers.
    tempo: f64,
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

/// Wow and flutter at full depth: cents and cycles a second of each
/// wobble (two slow ones for the wow, one fast for the flutter).
const WOW: [(f32, f32); 3] = [(15.0, 0.53), (10.0, 0.29), (5.0, 7.3)];
/// Below this speed the tape fades the sound out, reaching silence at a halt.
const TAPE_FADE: f32 = 0.25;

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
            lane: None,
            events: Vec::with_capacity(MAX_EVENTS),
            cycle: vec![0; PADS * kits],
            held: vec![0; PADS * kits],
            last_key: vec![None; PADS * kits],
            rng: Rng::new(0x5A4D_5052),
            clock: 0,
            fixed_velocity: None,
            stats: Stats::default(),
            frames: 0,
            master: None,
            tempo: 120.0,
            tape: 1.0,
            tape_target: 1.0,
            tape_rate: 0.0,
            tape_moved: false,
            lives: vec![
                Live {
                    pads: Live::OWN,
                    locks: Live::OWN,
                    program: [0.0; ProgramField::COUNT],
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

    /// At most `n` of the pool's voices play as full voices (any setting:
    /// filters, envelopes, loops, grains, pitch); the rest of the pool is
    /// for plain one-shots, which cost a fraction as much ([`plain_now`]).
    /// A full voice past the limit takes the place of another full voice,
    /// as a voice past a full pool does. The pad player keeps its full
    /// voices at the 24 it has always had and gives the drum hits more.
    pub fn set_full_voices(&mut self, n: usize) {
        self.full_limit = n.max(1);
    }

    /// The voice slot a new voice takes when there is no free one (or no
    /// free full voice, with `full_only`): a releasing voice first, then
    /// the oldest voice of the pad holding the most voices, whatever its
    /// kit. `held` is [`Engine::held`], all 0, and left so. Never
    /// allocates.
    fn victim(pool: &Voices, held: &mut [u16], full_only: bool) -> usize {
        let rel = |x: &Voice| x.kill_left > 0 || x.amp.as_ref().is_some_and(|a| a.is_releasing());
        // One kit: a table on the stack, zeroed as it is made, as an
        // engine of one kit always had; more kits count in `held`, which
        // is zeroed again after.
        let one = held.len() <= PADS;
        let mut own = [0u16; PADS];
        let table: &mut [u16] = if one { &mut own } else { held };
        let at = |x: &Voice| usize::from(x.kit) * PADS + usize::from(x.pad);
        for (v, _) in pool.iter() {
            if v.active && !rel(v) && usize::from(v.pad) < PADS {
                if let Some(n) = table.get_mut(at(v)) {
                    *n += 1;
                }
            }
        }
        type Key = (bool, u16, std::cmp::Reverse<u64>);
        let mut best: Option<(usize, Key)> = None;
        for (i, (v, _)) in pool.iter().enumerate() {
            if full_only && (!v.active || v.light) {
                continue;
            }
            let count = if usize::from(v.pad) < PADS {
                table.get(at(v)).copied().unwrap_or(0)
            } else {
                0
            };
            let key = (rel(v), count, std::cmp::Reverse(v.age));
            if best.is_none_or(|(_, b)| key > b) {
                best = Some((i, key));
            }
        }
        if !one {
            for (v, _) in pool.iter() {
                if usize::from(v.pad) < PADS {
                    if let Some(n) = table.get_mut(at(v)) {
                        *n = 0;
                    }
                }
            }
        }
        best.map_or(0, |(i, _)| i)
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
            // A cloud synced to the tempo follows it at once.
            g.set_tempo(beat);
        }
    }

    /// Output frames per beat at the project tempo: what a granular pad's
    /// tempo sync follows, whatever its saved `frames_per_beat` says.
    pub fn frames_per_beat(&self) -> f64 {
        60.0 * f64::from(self.rate) / self.tempo
    }

    pub fn tempo(&self) -> f64 {
        self.tempo
    }

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

    fn apply_master(&mut self) {
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

    /// How many kits the engine plays ([`Engine::with_kits`]).
    pub fn kits(&self) -> usize {
        self.views.len()
    }

    /// Frames rendered so far. Synced grains start on frames of this
    /// timeline, so it should match the transport's frame count.
    pub fn frames(&self) -> u64 {
        self.frames
    }
    /// Grains playing now, across every granular voice.
    pub fn live_grains(&self) -> usize {
        self.pool.a.budget.live() + self.pool.b.as_ref().map_or(0, |b| b.budget.live())
    }

    /// Swaps in a new view at once and hands back the old one, so the caller
    /// frees it off the audio thread. Playing voices keep their own handle to
    /// their audio and finish as they started: this is for another kit,
    /// whose pads are other pads ([`Engine::edit_view`] is for an edit).
    /// The master effect takes the new view's settings (a new mix glides
    /// there over 10 ms). The pads' live settings ([`Event::PadLive`]) go
    /// back to the pads' own; the program's offsets stay.
    pub fn set_view(&mut self, view: EngineView) -> EngineView {
        self.set_kit_view(0, view)
    }

    /// [`Engine::set_view`] for kit `kit`: the other kits and their voices
    /// are left as they are. The master effect takes the new view's
    /// settings only if it follows this kit ([`Engine::set_master_kit`]).
    /// A kit out of range gets nothing, and `view` comes back.
    pub fn set_kit_view(&mut self, kit: usize, view: EngineView) -> EngineView {
        let Some(slot) = self.views.get_mut(kit) else {
            return view;
        };
        let old = std::mem::replace(slot, view);
        self.lives[kit].pads = Live::OWN;
        self.edit_lane[kit] = false;
        if kit == self.master_kit {
            self.apply_master();
        }
        old
    }

    /// Swaps in an edit of the view playing now (the same kit, edited) as
    /// [`Engine::set_view`] does, and the voices playing hear it: each
    /// voice whose layer still plays the same sound (the same pad and
    /// layer, the same audio) picks up the layer's loop points at once,
    /// and its level, pan, cutoff, resonance and tune as a live setting
    /// does (gliding, so nothing clicks; a new tune at once, as a knob
    /// would). A loop moved behind the voice takes it into the loop, the
    /// step smoothed over [`JUMP_FRAMES`]; a loop taken away lets the
    /// voice play on to its end; a granular voice takes the new grain
    /// settings. What it started with stays: its start and end, fades,
    /// play mode, direction, envelope, filter type and the sound itself
    /// (an edit that changes those, or puts another sound on the pad, is
    /// heard from the next hit). Never allocates.
    pub fn edit_view(&mut self, view: EngineView) -> EngineView {
        self.edit_kit_view(0, view)
    }

    /// [`Engine::edit_view`] for kit `kit`: its voices hear the edit, the
    /// other kits' voices are left as they are. A kit out of range gets
    /// nothing, and `view` comes back.
    pub fn edit_kit_view(&mut self, kit: usize, view: EngineView) -> EngineView {
        let Some(live) = self.lives.get(kit) else {
            return view;
        };
        // The pads whose voices follow a live value, which goes now.
        let live_pads: [bool; PADS] = std::array::from_fn(|p| {
            PadField::ALL
                .iter()
                .any(|&f| f.follows() && !live.pads[p][f as usize].is_nan())
        });
        let old = self.set_kit_view(kit, view);
        let Engine {
            pool, views, lives, ..
        } = self;
        let (view, live) = (&views[kit], &lives[kit]);
        for (v, g) in pool.iter_mut() {
            let p = usize::from(v.pad);
            if v.active && usize::from(v.kit) == kit && p < PADS {
                follow_edit(v, g, view, &live.row(p), &live.program, live_pads[p]);
            }
        }
        self.edit_lane[kit] = self.pool.split && self.pool.b.is_none();
        old
    }

    /// The revision of kit 0's view.
    pub fn view_revision(&self) -> u64 {
        self.kit_revision(0)
    }

    /// The revision of kit `kit`'s view (0 for a kit out of range).
    pub fn kit_revision(&self, kit: usize) -> u64 {
        self.views.get(kit).map_or(0, |v| v.revision)
    }

    /// Full Level (Some(127)), Half Level (Some(64)), or off (None).
    pub fn set_fixed_velocity(&mut self, v: Option<u8>) {
        self.fixed_velocity = v.map(|v| v.clamp(1, 127));
    }

    /// The counters. While the lane renders its share of the voices (or
    /// was late for it), that share's skipped grains are counted as they
    /// were when it left, so they never go back.
    pub fn stats(&self) -> Stats {
        let b = match self.pool.b.as_ref() {
            Some(b) => b.budget.skipped,
            None => self.pool.b_skipped,
        };
        Stats {
            skipped_grains: self.pool.a.budget.skipped + b,
            ..self.stats
        }
    }
    pub fn active_voices(&self) -> usize {
        self.pool.iter().filter(|(v, _)| v.active).count()
    }

    /// Load shedding: the voice here this engine would give up first, the
    /// least audible one ([`Cull::audibility`]) by how loud it has been
    /// lately. Voices already fading and auditions are left be, and so are
    /// the lane's while it has them (they are not here). No allocation.
    pub fn cull_candidate(&self) -> Option<Cull> {
        self.cull_pick().map(|(_, c)| c)
    }

    /// Gives up the voice [`Engine::cull_candidate`] names, with the
    /// steal's short fade. True if it did. No allocation.
    pub fn cull(&mut self) -> bool {
        let Some((slot, _)) = self.cull_pick() else {
            return false;
        };
        let Some(v) = self.pool.voice_mut(slot) else {
            return false;
        };
        Self::kill(v);
        self.stats.culls += 1;
        true
    }

    fn cull_pick(&self) -> Option<(usize, Cull)> {
        let mut best = None;
        for (i, (v, _)) in self.pool.iter().enumerate() {
            if !v.active || v.kill_left > 0 || v.audition {
                continue;
            }
            let c = Cull {
                level: v.heard,
                releasing: v.amp.as_ref().is_some_and(|a| a.is_releasing()),
            };
            best = Cull::first(best, Some((i, c)));
        }
        best
    }
    /// (pad, layer, source frame) of every playing voice, for playheads on screen.
    /// A granular voice reports its reading position (before spray); a
    /// voice playing a stretched copy, the source frame that part of the
    /// copy was made from.
    /// Kit 0's.
    pub fn playheads(&self) -> impl Iterator<Item = (u8, u8, u64)> + '_ {
        self.kit_playheads(0)
    }

    /// [`Engine::playheads`] of kit `kit`'s voices.
    pub fn kit_playheads(&self, kit: usize) -> impl Iterator<Item = (u8, u8, u64)> + '_ {
        self.pool
            .iter()
            .filter(move |(v, _)| v.active && usize::from(v.kit) == kit)
            .map(|(v, g)| (v.pad, v.layer, self.source_frame(v, g)))
    }

    /// Where voice `v` is in its source: see [`Engine::playheads`].
    fn source_frame(&self, v: &Voice, g: &GrainVoice) -> u64 {
        let at = if v.granular {
            v.start + (f64::from(g.playhead()) * (v.end - v.start) as f64) as i64
        } else {
            let pos = v.pos >> 32;
            match self.copy_map(v) {
                Some(map) => map.source(pos as f64).round() as i64,
                None => pos,
            }
        };
        at.max(0) as u64
    }

    /// The map of the stretched copy voice `v` plays, if it plays one.
    fn copy_map(&self, v: &Voice) -> Option<&CopyMap> {
        let l = self
            .views
            .get(usize::from(v.kit))?
            .pads
            .get(usize::from(v.pad))?
            .as_ref()?
            .layers
            .get(usize::from(v.layer))?;
        let same = v.data.as_ref().is_some_and(|d| d.ptr_eq(&l.data));
        l.source_at.as_ref().filter(|_| same)
    }
    /// The voices kit 0's pad `pad` plays now, not counting those fading out.
    pub fn pad_voices(&self, pad: u8) -> usize {
        self.kit_pad_voices(0, pad)
    }

    /// [`Engine::pad_voices`] of kit `kit`'s pad.
    pub fn kit_pad_voices(&self, kit: usize, pad: u8) -> usize {
        self.pool
            .iter()
            .filter(|(v, _)| {
                v.active && usize::from(v.kit) == kit && v.pad == pad && v.kill_left == 0
            })
            .count()
    }

    /// The pads whose hit keeps a latched voice playing, as bits (pad `k`
    /// is bit `k`): a pad lights while it is latched. Hits only: a latched
    /// keygroup note or 16 Levels step isn't a pad of its own. Kit 0's.
    pub fn latched_pads(&self) -> u128 {
        self.kit_latched_pads(0)
    }

    /// [`Engine::latched_pads`] of kit `kit`.
    pub fn kit_latched_pads(&self, kit: usize) -> u128 {
        let mut bits = 0u128;
        for (v, _) in self.pool.iter() {
            if v.active
                && usize::from(v.kit) == kit
                && v.latch
                && v.held
                && v.note == NO_NOTE
                && usize::from(v.by) < PADS
            {
                bits |= 1 << v.by;
            }
        }
        bits
    }

    /// The grains layer `layer` of pad `pad` plays now, across every voice
    /// playing it: each one's reading position (0 to 1 across the region),
    /// window level and direction, for drawing the cloud. Fills `out` from
    /// the start and returns how many it holds (grains past its end are
    /// left out). Never allocates. Kit 0's.
    pub fn grains_of(&self, pad: u8, layer: u8, out: &mut [GrainView]) -> usize {
        self.kit_grains_of(0, pad, layer, out)
    }

    /// [`Engine::grains_of`] for kit `kit`'s pad.
    pub fn kit_grains_of(&self, kit: usize, pad: u8, layer: u8, out: &mut [GrainView]) -> usize {
        let mut n = 0;
        for (v, g) in self.pool.iter() {
            if v.active
                && usize::from(v.kit) == kit
                && v.granular
                && v.pad == pad
                && v.layer == layer
                && n < out.len()
            {
                n += g.grains(&mut out[n..]);
            }
        }
        n
    }

    /// How layer `layer` of pad `pad` plays as grains now, beyond its
    /// grains ([`Engine::grains_of`]): the spray its first voice's grains
    /// start in (as its movement and pressure move it), and whether any of
    /// its voices is thinning its grains to fit (a cloud wanting more at
    /// once than a voice keeps). None while no voice plays it. Never
    /// allocates.
    pub fn cloud_of(&self, pad: u8, layer: u8) -> Option<CloudState> {
        self.kit_cloud_of(0, pad, layer)
    }

    /// [`Engine::cloud_of`] for kit `kit`'s pad.
    pub fn kit_cloud_of(&self, kit: usize, pad: u8, layer: u8) -> Option<CloudState> {
        let mut out: Option<CloudState> = None;
        for (v, g) in self.pool.iter() {
            if v.active
                && usize::from(v.kit) == kit
                && v.granular
                && v.pad == pad
                && v.layer == layer
            {
                let c = out.get_or_insert(CloudState {
                    spray: g.moved_params().spray,
                    thinned: false,
                });
                c.thinned |= g.stats().thinned;
            }
        }
        out
    }

    /// Queues an event `offset` frames into the next render (later renders if
    /// the offset is past its end). Events at the same offset keep their order.
    /// Returns false, and counts it, if the queue is full. For kit 0.
    pub fn schedule(&mut self, offset: u32, ev: Event) -> bool {
        self.schedule_for(0, offset, ev)
    }

    /// [`Engine::schedule`] for kit `kit`: its pads, its live settings. One
    /// queue holds every kit's events, in the order they come. Returns
    /// false for a kit out of range, which is not counted.
    pub fn schedule_for(&mut self, kit: usize, offset: u32, ev: Event) -> bool {
        if kit >= self.views.len() {
            return false;
        }
        if self.events.len() >= MAX_EVENTS {
            self.stats.dropped_events += 1;
            return false;
        }
        let i = self.events.partition_point(|(o, _, _)| *o <= offset);
        self.events.insert(i, (offset, kit as u8, ev));
        true
    }

    /// Applies an event now (before the next rendered frame). For kit 0.
    pub fn apply(&mut self, ev: Event) {
        self.apply_for(0, ev)
    }

    /// [`Engine::apply`] for kit `kit`. A kit out of range gets nothing.
    pub fn apply_for(&mut self, kit: usize, ev: Event) {
        if kit >= self.views.len() {
            return;
        }
        self.at = kit;
        match ev {
            // With the pads playing notes, a pad is the note it plays;
            // with 16 Levels, the one pad at its level.
            Event::NoteOn { pad, velocity } => match self.target(pad) {
                Target::Own(_) => self.hit(pad, velocity),
                Target::Key(note) => self.key(note, velocity),
                Target::Level { pad: p, by, k } => {
                    self.note_on(p, velocity, LEVEL_NOTE + k, Vary::level(by, k))
                }
            },
            Event::NoteOff { pad } => {
                let t = self.target(pad);
                self.release(|v| t.played(v));
            }
            Event::Key { note, velocity } => self.key(note.min(127), velocity),
            Event::KeyOff { note } => self.release(|v| v.note == note),
            Event::Pressure { pad, value } => {
                let t = self.target(pad);
                for (v, _) in self.pool.iter_mut() {
                    if v.active && usize::from(v.kit) == kit && t.played(v) && v.kill_left == 0 {
                        v.press_target = f32::from(value.min(127)) / 127.0;
                    }
                }
            }
            Event::AllOff => {
                for (v, g) in self.pool.iter_mut() {
                    if v.active {
                        v.held = false;
                        v.looping = false;
                        if let Some(a) = &mut v.amp {
                            a.release();
                        }
                        if v.granular {
                            g.stop();
                        }
                    }
                }
            }
            Event::Freeze { on } => {
                if let Some(fx) = &mut self.master {
                    fx.set_frozen(on);
                }
            }
            Event::Tape { stop, ms } => {
                self.tape_target = if stop { 0.0 } else { 1.0 };
                let frames = f32::from(ms) * self.rate / 1000.0;
                if frames < 1.0 {
                    self.tape = self.tape_target;
                    self.tape_rate = 0.0;
                } else {
                    self.tape_rate = 1.0 / frames;
                }
                self.tape_moved = true;
            }
            Event::Audition {
                pad,
                layer,
                start,
                end,
                velocity,
            } => self.audition(pad, layer, start, end, velocity),
            Event::PadLive { pad, field, value } => self.pad_live(pad, field, value),
            Event::PadLock { pad, field, value } => self.pad_lock(pad, field, value),
            Event::ProgramLive { field, value } => self.program_live(field, value),
            Event::LiveChop { pad } => {
                if let (None, Some(was)) = (pad, self.chop_pad) {
                    let chop_kit = self.chop_kit;
                    for (v, _) in self.pool.iter_mut() {
                        if v.active && v.audition && v.kit == chop_kit && v.pad == was {
                            Self::kill(v);
                        }
                    }
                }
                self.chop_pad = pad;
                if pad.is_some() {
                    self.chop_kit = kit as u8;
                }
            }
            Event::Mark => {
                if let Some(at) = self.chop_pad.and_then(|p| self.audition_at(p)) {
                    self.marks[(self.marks_made % MAX_MARKS as u64) as usize] = at;
                    self.marks_made += 1;
                }
            }
        }
    }

    /// Where live chop's kit's pad `pad`'s newest audition is in its
    /// source, if one plays.
    fn audition_at(&self, pad: u8) -> Option<u64> {
        let mut best: Option<(u64, u64)> = None;
        for (v, g) in self.pool.iter() {
            if v.active && v.audition && v.kit == self.chop_kit && v.pad == pad && v.kill_left == 0
            {
                let at = self.source_frame(v, g);
                if best.is_none_or(|(age, _)| v.age > age) {
                    best = Some((v.age, at));
                }
            }
        }
        best.map(|(_, at)| at)
    }

    /// The pad live chop takes cuts from, while it is on and that pad's
    /// audition plays: hits mark cuts ([`Event::Mark`]) instead of playing
    /// then. `None` once the audition ends, even before live chop is
    /// turned off.
    pub fn marking(&self) -> Option<u8> {
        self.chop_pad.filter(|&p| self.audition_at(p).is_some())
    }

    /// The kit live chop was last turned on for ([`Event::LiveChop`]
    /// through [`Engine::apply_for`] or [`Engine::schedule_for`]).
    pub fn marking_kit(&self) -> usize {
        usize::from(self.chop_kit)
    }

    /// How many cuts live chop has made.
    pub fn marks_made(&self) -> u64 {
        self.marks_made
    }

    /// Cut `i` (from 0, in the order made), in source frames, while it is
    /// among the last [`MAX_MARKS`].
    pub fn mark(&self, i: u64) -> Option<u64> {
        (i < self.marks_made && self.marks_made - i <= MAX_MARKS as u64)
            .then(|| self.marks[(i % MAX_MARKS as u64) as usize])
    }

    /// [`Event::PadLive`]: the table, then the playing voices of the pad.
    fn pad_live(&mut self, pad: u8, field: PadField, value: f32) {
        let Some(row) = self.lives[self.at].pads.get_mut(usize::from(pad)) else {
            return;
        };
        row[field as usize] = field.clamp(value);
        self.follow_pad(pad, field);
    }

    /// [`Event::PadLock`]: the table, then the playing voices of the pad.
    fn pad_lock(&mut self, pad: u8, field: PadField, value: f32) {
        let Some(row) = self.lives[self.at].locks.get_mut(usize::from(pad)) else {
            return;
        };
        row[field as usize] = field.clamp(value);
        self.follow_pad(pad, field);
    }

    /// Pad `pad`'s playing voices go where its settings now say, if they
    /// follow `field`.
    fn follow_pad(&mut self, pad: u8, field: PadField) {
        if field.follows() {
            let at = self.at;
            let (live, program) = (self.lives[at].row(usize::from(pad)), self.lives[at].program);
            for (v, _) in self.pool.iter_mut() {
                if v.active && usize::from(v.kit) == at && v.pad == pad {
                    retarget(v, &live, &program);
                }
            }
        }
    }

    /// [`Event::ProgramLive`]: the offset, then every playing voice.
    fn program_live(&mut self, field: ProgramField, value: f32) {
        let at = self.at;
        self.lives[at].program[field as usize] = field.clamp(value);
        let program = self.lives[at].program;
        let live = &self.lives[at];
        for (v, _) in self.pool.iter_mut() {
            if v.active && usize::from(v.kit) == at && usize::from(v.pad) < PADS {
                retarget(v, &live.row(usize::from(v.pad)), &program);
            }
        }
    }

    /// Pad `pad`'s live `field`, or `None` where the pad's own applies.
    /// Kit 0's.
    pub fn pad_live_value(&self, pad: u8, field: PadField) -> Option<f32> {
        self.kit_pad_live_value(0, pad, field)
    }

    /// [`Engine::pad_live_value`] of kit `kit`'s pad.
    pub fn kit_pad_live_value(&self, kit: usize, pad: u8, field: PadField) -> Option<f32> {
        let x = self.lives.get(kit)?.pads.get(usize::from(pad))?[field as usize];
        (!x.is_nan()).then_some(x)
    }

    /// Pad `pad`'s step lock on `field` ([`Event::PadLock`]), if one holds.
    /// Kit 0's.
    pub fn pad_lock_value(&self, pad: u8, field: PadField) -> Option<f32> {
        self.kit_pad_lock_value(0, pad, field)
    }

    /// [`Engine::pad_lock_value`] of kit `kit`'s pad.
    pub fn kit_pad_lock_value(&self, kit: usize, pad: u8, field: PadField) -> Option<f32> {
        let x = self.lives.get(kit)?.locks.get(usize::from(pad))?[field as usize];
        (!x.is_nan()).then_some(x)
    }

    /// The program's offset on `field` (0 when none). Kit 0's.
    pub fn program_live_value(&self, field: ProgramField) -> f32 {
        self.kit_program_live_value(0, field)
    }

    /// [`Engine::program_live_value`] of kit `kit` (0 out of range).
    pub fn kit_program_live_value(&self, kit: usize, field: ProgramField) -> f32 {
        self.lives
            .get(kit)
            .map_or(0.0, |l| l.program[field as usize])
    }

    /// The hard stop: every voice and its grains go idle at once, without a
    /// fade, the master effect's grains stop and forget what they hold, and
    /// the events waiting are dropped. [`Event::AllOff`] releases instead;
    /// this is for a reset or a panic, where a click is fine. The live
    /// settings, the tape and the view stay. Never allocates. Split, a share
    /// still away on the lane (late) stops as it comes back.
    pub fn stop_all(&mut self) {
        self.events.clear();
        silence(&mut self.pool.a);
        match self.pool.b.as_deref_mut() {
            Some(b) => silence(b),
            None => self.stop_lane = self.pool.split,
        }
        if let Some(fx) = &mut self.master {
            fx.reset(&mut self.pool.a.budget);
        }
    }

    /// The tape's speed now: 1 running, 0 stopped.
    pub fn tape_speed(&self) -> f32 {
        self.tape
    }

    /// Moves the tape on by `frames` and has every voice follow it and its
    /// own wow and flutter. Once a period (or between events), never per
    /// frame; cheap while the tape runs and no pad wobbles.
    fn follow_tape(&mut self, frames: usize) {
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

    fn kill(v: &mut Voice) {
        if v.active && v.kill_left == 0 {
            v.kill_left = KILL_FRAMES;
        }
    }

    /// What a hit on pad `pad` of the kit being played plays.
    fn target(&self, pad: u8) -> Target {
        match self.views[self.at].pads_play {
            PadsPlay::Pads => Target::Own(pad),
            PadsPlay::Keys { from } => Target::Key(from.saturating_add(pad).min(127)),
            PadsPlay::Levels { pad: p, by } => Target::Level {
                pad: p,
                by,
                k: pad % 16,
            },
        }
    }

    /// Every keygroup holding `note` plays it.
    fn key(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.release(|v| v.note == note);
            return;
        }
        for p in 0..self.views[self.at].pads.len() {
            let Some(k) = self.views[self.at].pads[p].as_ref().and_then(|vp| vp.keys) else {
                continue;
            };
            if k.contains(note) {
                let semis = i32::from(note) - i32::from(k.root);
                self.note_on(
                    p as u8,
                    velocity,
                    note,
                    Vary {
                        semis,
                        ..Vary::default()
                    },
                );
            }
        }
    }

    /// Lets go of the held voices `which` picks, of the kit being played.
    fn release(&mut self, which: impl Fn(&Voice) -> bool) {
        let at = self.at;
        for (v, g) in self.pool.iter_mut() {
            // A latched voice waits for its pad's next hit.
            if !v.active || usize::from(v.kit) != at || !which(v) || !v.held || v.latch {
                continue;
            }
            match v.mode {
                Some(PlayMode::Gate) => {
                    v.held = false;
                    if let Some(a) = &mut v.amp {
                        a.release();
                    }
                }
                Some(PlayMode::Granular) => {
                    v.held = false;
                    g.stop();
                    if let Some(a) = &mut v.amp {
                        a.release();
                    }
                }
                Some(PlayMode::Loop) => {
                    v.held = false;
                    v.looping = false;
                    v.backwards = v.reverse;
                }
                _ => {} // one-shot ignores it; toggle waits for the next hit
            }
        }
    }

    fn audition(&mut self, pad: u8, layer: u8, start: u32, end: u32, velocity: u8) {
        // A new audition ends live chop (Player::live_chop turns it on
        // again after its own).
        self.chop_pad = None;
        if velocity == 0 || start >= end {
            return;
        }
        let velocity = self.fixed_velocity.unwrap_or(velocity.min(127));
        let p = usize::from(pad);
        let at = self.at;
        let Some(Some(vp)) = self.views[at].pads.get(p) else {
            return;
        };
        let li = usize::from(layer);
        if li >= vp.layers.len() {
            return;
        }
        self.stats.hits += 1;
        let vel = f32::from(velocity) / 127.0;
        let level = (1.0 - vp.vel_to_level) + vp.vel_to_level * vel * vel;
        for (v, _) in self.pool.iter_mut() {
            if v.active && usize::from(v.kit) == at && v.pad == pad {
                Self::kill(v);
            }
        }
        let vary = Vary {
            slice: Some((i64::from(start), i64::from(end))),
            ..Vary::default()
        };
        self.start_voice(p, li, level, vel, NO_NOTE, vary);
    }

    /// A hit on pad `pad` itself: it plays, and so does every pad it
    /// links to, each as a hit on it would (its layers, chokes and latch),
    /// its voices let go when `pad` is. Velocity 0 is a lift.
    fn hit(&mut self, pad: u8, velocity: u8) {
        if velocity == 0 {
            self.release(|v| v.by == pad && v.note == NO_NOTE);
            return;
        }
        let mut links = [0u8; PADS];
        let mut n = 0;
        if let Some(Some(vp)) = self.views[self.at].pads.get(usize::from(pad)) {
            for &l in &vp.links {
                links[n] = l as u8;
                n += 1;
            }
        }
        self.spare_from = self.clock + 1;
        self.note_on(pad, velocity, NO_NOTE, Vary::default());
        for &l in &links[..n] {
            let by = Vary {
                by: Some(pad),
                ..Vary::default()
            };
            self.note_on(l, velocity, NO_NOTE, by);
        }
        self.spare_from = u64::MAX;
    }

    /// Pad `pad` plays: hit (`note` is [`NO_NOTE`]), as the keygroup for
    /// `note`, or at a 16 Levels step, with what `vary` changes.
    fn note_on(&mut self, pad: u8, velocity: u8, note: u8, vary: Vary) {
        if velocity == 0 {
            self.release(|v| v.pad == pad && v.note == note);
            return;
        }
        // 16 Levels of velocity are the pads' own, whatever Full Level says.
        let velocity = vary
            .velocity
            .unwrap_or(self.fixed_velocity.unwrap_or(velocity.min(127)));
        let p = usize::from(pad);
        let at = self.at;
        let Some(Some(vp)) = self.views[at].pads.get(p) else {
            return;
        };
        self.stats.hits += 1;

        // A second hit on a held toggle or latched pad lets it go instead
        // of starting it again.
        let mut toggled_off = false;
        for (v, g) in self.pool.iter_mut() {
            if v.active
                && usize::from(v.kit) == at
                && v.pad == pad
                && v.note == note
                && v.held
                && (v.latch || v.mode == Some(PlayMode::Toggle))
            {
                let_go(v, g);
                toggled_off = true;
            }
        }
        if toggled_off {
            return;
        }

        // Mute groups and one-way chokes (not the voices this hit started
        // on the pads it links to).
        let group = vp.mute_group;
        for (v, _) in self.pool.iter_mut() {
            if !v.active || usize::from(v.kit) != at || v.pad == pad || v.age >= self.spare_from {
                continue;
            }
            let other = usize::from(v.pad);
            let same_group = group.is_some()
                && self.views[at]
                    .pads
                    .get(other)
                    .and_then(Option::as_ref)
                    .is_some_and(|o| o.mute_group == group);
            if same_group || vp.chokes.contains(&other) {
                Self::kill(v);
            }
        }

        // Which layers play.
        let count = vp.layers.len();
        let mut chosen = [false; crate::model::MAX_LAYERS];
        let matching =
            |i: usize| vp.layers[i].vel_lo <= velocity && velocity <= vp.layers[i].vel_hi;
        let n_match = (0..count).filter(|&i| matching(i)).count();
        if n_match == 0 {
            return;
        }
        // With a velocity fade, layers with the same velocities are a set:
        // Cycle and Random take turns within each set, and overlapping sets
        // play together, crossfaded.
        let sets = vp.vel_fade > 0 && vp.select != LayerSelect::All;
        match vp.select {
            LayerSelect::All => {
                for (i, c) in chosen.iter_mut().enumerate().take(count) {
                    *c = matching(i);
                }
            }
            LayerSelect::Cycle | LayerSelect::Random if sets => {
                let k = if vp.select == LayerSelect::Cycle {
                    let turn = &mut self.cycle[at * PADS + p];
                    let k = *turn % TURNS;
                    *turn = (k + 1) % TURNS;
                    k as usize
                } else {
                    self.rng.below(TURNS) as usize
                };
                let same = |i: usize, j: usize| {
                    let (a, b) = (&vp.layers[i], &vp.layers[j]);
                    matching(j) && a.vel_lo == b.vel_lo && a.vel_hi == b.vel_hi
                };
                for i in (0..count).filter(|&i| matching(i)) {
                    let before = (0..i).filter(|&j| same(i, j)).count();
                    let size = (0..count).filter(|&j| same(i, j)).count();
                    chosen[i] = before == k % size;
                }
            }
            LayerSelect::Cycle | LayerSelect::Random => {
                let k = if vp.select == LayerSelect::Cycle {
                    let turn = &mut self.cycle[at * PADS + p];
                    let k = *turn as usize % n_match;
                    *turn = ((k + 1) % n_match) as u32;
                    k
                } else {
                    self.rng.below(n_match as u32) as usize
                };
                if let Some(i) = (0..count).filter(|&i| matching(i)).nth(k) {
                    chosen[i] = true;
                }
            }
        }

        let vel = f32::from(velocity) / 127.0;
        let level = (1.0 - vp.vel_to_level) + vp.vel_to_level * vel * vel;
        // Key and velocity fades: a level for each layer, set at the start.
        let key_gain = match vp.keys {
            Some(k) if note != NO_NOTE && k.contains(note) => {
                edge_gain(note, k.lo, k.hi, vp.key_fade, 0, 127)
            }
            _ => 1.0,
        };
        let mut gains = [key_gain; crate::model::MAX_LAYERS];
        for (li, g) in gains.iter_mut().enumerate().take(count) {
            let l = &vp.layers[li];
            *g *= edge_gain(velocity, l.vel_lo, l.vel_hi, vp.vel_fade, 1, 127);
        }
        let polyphony = usize::from(vp.polyphony);
        // A mono pad's gliding granular layers: a key played while one
        // holds moves that cloud there, without a new start (legato).
        let mut legato = [false; crate::model::MAX_LAYERS];
        if polyphony == 1 {
            for (li, l) in vp.layers.iter().enumerate().take(count) {
                legato[li] = l.glide > 0 && l.mode == PlayMode::Granular;
            }
        }
        for li in 0..count {
            if !chosen[li] {
                continue;
            }
            if legato[li] && self.glide_held(pad, li, note, vary) {
                continue;
            }
            // Polyphony: push out this pad's oldest voices past the limit.
            loop {
                let mut n = 0;
                let mut oldest: Option<(usize, u64)> = None;
                for (i, (v, _)) in self.pool.iter().enumerate() {
                    if v.active && usize::from(v.kit) == at && v.pad == pad && v.kill_left == 0 {
                        n += 1;
                        if oldest.is_none_or(|(_, age)| age > v.age) {
                            oldest = Some((i, v.age));
                        }
                    }
                }
                if n < polyphony {
                    break;
                }
                match oldest.and_then(|(o, _)| self.pool.voice_mut(o)) {
                    Some(v) => Self::kill(v),
                    None => break,
                }
            }
            self.start_voice(p, li, level * gains[li], vel, note, vary);
        }
    }

    /// A held granular voice of layer `li` of pad `pad`, if one is
    /// playing, glides to `vary`'s key (a mono pad's next key: see
    /// [`Region::grain_glide_ms`]) and plays `note` from now on. False if
    /// none holds.
    ///
    /// [`Region::grain_glide_ms`]: crate::model::Region::grain_glide_ms
    fn glide_held(&mut self, pad: u8, li: usize, note: u8, vary: Vary) -> bool {
        let p = usize::from(pad);
        let Some(glide) = self.view.pads[p]
            .as_ref()
            .and_then(|vp| vp.layers.get(li))
            .map(|l| l.glide)
        else {
            return false;
        };
        let at = self.at;
        for (v, _) in self.pool.iter_mut() {
            if v.active
                && v.granular
                && v.held
                && v.kill_left == 0
                && usize::from(v.kit) == at
                && v.pad == pad
                && usize::from(v.layer) == li
            {
                let now = if v.glide_left > 0 {
                    v.glide_to
                } else {
                    v.grain_hit
                };
                let to = now + (vary.semis - v.grain_key) as f32;
                v.glide_to = to;
                v.glide_step = (to - v.grain_hit) / glide.max(1) as f32;
                v.glide_left = glide;
                v.grain_key = vary.semis;
                v.note = note;
                v.by = vary.by.unwrap_or(pad);
                self.last_key[at * PADS + p] = Some(to);
                return true;
            }
        }
        false
    }

    /// Starts layer `li` of pad `p` with what `vary` changes. A slice, for
    /// an audition, plays those source frames instead of the layer's
    /// region: once, forward, without fades or a loop.
    fn start_voice(&mut self, p: usize, li: usize, level: f32, vel: f32, note: u8, vary: Vary) {
        let semis = vary.semis;
        let at = self.at;
        let Some(vp) = self.views[at].pads[p].as_ref() else {
            return;
        };
        let l = &vp.layers[li];
        // A slice is in source frames; a stretched copy has its own.
        let slice = vary.slice.map(|(a, b)| match &l.source_at {
            Some(map) => {
                let at = |x: i64| map.at(x as f64).round() as i64;
                (at(a), at(b))
            }
            None => (a, b),
        });
        let slice = match slice {
            Some((a, b)) => {
                let frames = (l.data.len() / if l.stereo { 2 } else { 1 }) as i64;
                let (a, b) = (a.max(0), b.min(frames));
                if a >= b {
                    return;
                }
                Some((a, b))
            }
            None => None,
        };
        // What the voice reads: the layer's region, or the slice once.
        let (start, end, reverse, mode) = match slice {
            Some((a, b)) => (a, b, false, PlayMode::OneShot),
            None => (l.start, l.end, l.reverse, l.mode),
        };
        let granular = mode == PlayMode::Granular;
        let frames = end - start;
        let skip = if reverse || slice.is_some() {
            0
        } else {
            let from = vary.start.unwrap_or(vp.vel_to_start * (1.0 - vel));
            (from as f64 * frames as f64) as i64
        };
        let start_frame = if reverse {
            end - 1
        } else {
            (start + skip).min(end - 1)
        };
        let (pick_l, pick_r) = picks(l.stereo, l.pick);
        let lp = if slice.is_some() {
            None
        } else {
            l.sustain_loop.as_ref()
        };
        let (fade_in, fade_out) = if slice.is_some() {
            (None, None)
        } else {
            (
                (l.fade_in.frames > 0).then_some(l.fade_in),
                (l.fade_out.frames > 0).then_some(l.fade_out),
            )
        };
        // The pad's live settings and locks, where set, in place of its own.
        let kit_live = &self.lives[at];
        let live = kit_live.row(p);
        let program = kit_live.program;
        let set = |f: PadField| Some(live[f as usize]).filter(|x| !x.is_nan());
        let mut amp = vp.amp;
        if let Some(ms) = set(PadField::Attack) {
            amp.attack_ms = ms;
        }
        if kit_live.pads[p][PadField::Decay as usize].is_nan() {
            // A locked decay decays to silence, as the Play page's Decay
            // knob shortens a sound.
            if let Some(ms) =
                Some(kit_live.locks[p][PadField::Decay as usize]).filter(|x| !x.is_nan())
            {
                (amp.decay_ms, amp.sustain) = (ms, 0.0);
            }
        } else if let Some(ms) = set(PadField::Decay) {
            amp.decay_ms = ms;
        }
        if let Some(ms) = set(PadField::Release) {
            amp.release_ms = ms;
        }
        if let Some(ms) = vary.decay_ms {
            (amp.decay_ms, amp.sustain) = (ms, 0.0);
        }
        let filter = match vary.cutoff_hz {
            Some(hz) if vp.filter.kind == FilterKind::Off => Some(Filter {
                kind: FilterKind::LowPass,
                poles: 2,
                cutoff_hz: hz,
                resonance: 0.0,
                env_octaves: 0.0,
            }),
            Some(hz) => Some(Filter {
                cutoff_hz: hz,
                ..vp.filter
            }),
            None => (vp.filter.kind != FilterKind::Off).then_some(vp.filter),
        };
        let own_fc = filter.map_or(OPEN_FILTER.cutoff_hz, |f| f.cutoff_hz);
        let own_res = filter.map_or(0.0, |f| f.resonance);
        let filter = live_filter(
            filter,
            set(PadField::Cutoff),
            set(PadField::Resonance),
            program[ProgramField::Cutoff as usize],
        );
        // A live tune in place of the pad's own, and the program's tune
        // (which playing voices follow). A warped layer keeps its pitch.
        let warped = l.warp_bpm > 0.0;
        let (tune, cents) = if warped {
            (0.0, 0.0)
        } else {
            (
                set(PadField::Tune).map_or(0.0, |c| c - vp.tune_cents as f32),
                program[ProgramField::Tune as usize],
            )
        };
        // A gliding layer's voice starts from the pad's last key.
        let hit = semis as f32 + tune / 100.0;
        let from = if granular && l.glide > 0 {
            self.last_key[at * PADS + p].filter(|k| *k != hit)
        } else {
            None
        };
        if granular {
            self.last_key[at * PADS + p] = Some(hit);
        }
        let mut grain = pitched_grain(l.grain, from.unwrap_or(hit), l.follow);
        if let Some(at) = vary.start {
            grain.position = (at * 16.0 / 15.0).min(1.0);
        }
        // A keygroup a few semitones from its root reads that much faster.
        let plain = if semis == 0 && tune == 0.0 {
            l.step
        } else {
            let octaves = f64::from(semis) / 12.0 + f64::from(tune) / 1200.0;
            ((l.step as f64 * octaves.exp2()).round() as i64).max(1)
        };
        // The gain: the view's, unless a live level or pan or the
        // program's level moves it.
        let unit_gain = l.unit_gain * level;
        let (live_db, live_pan) = (set(PadField::Level), set(PadField::Pan));
        let program_db = program[ProgramField::Level as usize];
        let (gain_l, gain_r) = if live_db.is_none() && live_pan.is_none() && program_db == 0.0 {
            (l.gain_l * level, l.gain_r * level)
        } else {
            live_gains(
                unit_gain,
                l.pan,
                live_db.unwrap_or(vp.level_db) + program_db,
                live_pan.unwrap_or(vp.pan),
            )
        };
        let follow = Follow {
            gain_to: [gain_l, gain_r],
            fc_to: filter.map_or(0.0, |f| f.cutoff_hz),
            res_to: filter.map_or(0.0, |f| f.resonance),
            cents,
            cents_to: cents,
            k: self.live_k,
            own_db: vp.level_db,
            own_pan: vp.pan,
            own_fc,
            own_res,
            ..Follow::default()
        };
        let step = if l.warp_bpm > 0.0 {
            warped_step(plain, self.tempo, l.warp_bpm)
        } else {
            plain
        };
        // A plain one-shot (as a kit's drum hits are) can take one of the
        // voices past the full-voice limit: it costs a fraction of a full
        // voice once it plays (`plain_now`, `render_plain`).
        let light = !granular
            && mode == PlayMode::OneShot
            && step == ONE
            && !reverse
            && fade_in.is_none()
            && fade_out.is_none()
            && filter.is_none()
            && vp.pressure.target == PressureTarget::Off
            && vp.wow == 0.0
            && cents == 0.0
            && self.tape == 1.0
            && amp_holds(&amp, self.rate)
            && (!l.stereo || (pick_l, pick_r) == (0, 1));
        // A free voice, else the oldest releasing one, else the oldest of
        // the pad holding the most voices: a pad hit over and over (Note
        // Repeat on a long sound) gives up its own voices before another
        // pad's sound is cut short. A full voice past the full-voice limit
        // takes one of the full voices the same way.
        let full_now = if light {
            0
        } else {
            self.pool
                .iter()
                .filter(|(v, _)| v.active && !v.light)
                .count()
        };
        let slot = if !light && full_now >= self.full_limit {
            Self::victim(&self.pool, &mut self.held, true)
        } else {
            match self.pool.free_slot() {
                Some(slot) => slot,
                None => Self::victim(&self.pool, &mut self.held, false),
            }
        };
        let Some(old) = self.pool.voice(slot) else {
            return;
        };
        let (stolen, was_granular, last) = (old.active, old.granular, (old.last_l, old.last_r));
        if was_granular {
            self.pool.kill_grain(slot);
        }
        let (tail_l, tail_r) = if stolen {
            self.stats.steals += 1;
            last
        } else {
            (0.0, 0.0)
        };
        self.clock += 1;
        let Some(v) = self.pool.voice_mut(slot) else {
            return;
        };
        *v = Voice {
            active: true,
            kit: at as u8,
            pad: p as u8,
            layer: li as u8,
            data: Some(l.data.clone()),
            stereo: l.stereo,
            pick_l,
            pick_r,
            pos: start_frame << 32,
            step,
            backwards: reverse,
            reverse,
            start,
            end,
            loop_start: lp.map_or(0, |x| x.start),
            loop_end: lp.map_or(0, |x| x.end),
            crossfade: lp.map_or(0, |x| x.crossfade),
            ping_pong: lp.is_some_and(|x| x.ping_pong),
            looping: !granular
                && lp.is_some()
                && !reverse
                && matches!(mode, PlayMode::Loop | PlayMode::Toggle | PlayMode::Gate),
            fade_in,
            fade_out,
            gain_l,
            gain_r,
            mode: Some(mode),
            held: true,
            age: self.clock,
            amp: Some(Envelope::start(&amp, self.rate)),
            fenv: filter.map(|_| Envelope::start(&vp.filter_env, self.rate)),
            filter,
            svf: [Svf::default(); 2],
            filter_count: 0,
            filter_key: None,
            kill_left: 0,
            tail_left: if stolen { STEAL_FRAMES } else { 0 },
            tail_l,
            tail_r,
            last_l: 0.0,
            last_r: 0.0,
            heard: gain_l.abs().max(gain_r.abs()),
            granular,
            src_rate: l.src_rate,
            grain_base: grain,
            grain_follow: l.follow,
            grain_hit: from.unwrap_or(hit),
            grain_key: semis,
            glide_to: hit,
            glide_step: from.map_or(0.0, |k| (hit - k) / l.glide as f32),
            glide_left: if from.is_some() { l.glide } else { 0 },
            route: vp.pressure,
            press_target: 0.0,
            press: 0.0,
            press_coef: 1.0 - (-1.0 / (PRESSURE_SMOOTH_S * self.rate)).exp(),
            base_step: step,
            press_gain: 1.0,
            press_gain_inc: 0.0,
            press_fc: 1.0,
            press_count: 0,
            warp_bpm: l.warp_bpm,
            plain_step: plain,
            note,
            by: vary.by.unwrap_or(p as u8),
            latch: vp.latch,
            audition: vary.slice.is_some(),
            speed: 1.0,
            base_gain_l: gain_l,
            base_gain_r: gain_r,
            wow: vp.wow,
            // Each voice wobbles from its own place, so a chord doesn't
            // wander in step.
            wow_phase: [
                (self.clock as f32 * 0.618_034).fract(),
                (self.clock as f32 * 0.414_214).fract(),
                (self.clock as f32 * 0.732_051).fract(),
            ],
            unit_gain,
            layer_pan: l.pan,
            tape_gain: 1.0,
            follow,
            grain_cents: cents,
            light,
            bl_above: BlAbove(if vp.vintage || granular {
                i64::MAX
            } else {
                CLEAN_ABOVE
            }),
            bl: false,
            // The copies if they are made by now, else none for its life:
            // what it reads doesn't hang on a thread's timing.
            octaves: if vp.vintage {
                None
            } else {
                l.octaves.as_ref().filter(|c| c.get().is_some()).cloned()
            },
            hits: l.hits.as_ref().filter(|c| c.get().is_some()).cloned(),
            grain_clean: !vp.vintage,
            bl_step: 0,
            bl_octave: 0,
            bl_reach: Reach::default(),
            hit_gain: level,
            own_cut: vary.cutoff_hz.is_none(),
            layer_step: l.step,
            layer_grain: l.grain,
            jump: [0.0; 2],
            jump_left: 0,
        };
        if self.tape != 1.0 {
            // A hit on a stopped (or slowing) tape plays at its speed.
            v.speed = self.tape;
            let gain = (self.tape / TAPE_FADE).min(1.0);
            v.tape_gain = gain;
            (v.gain_l, v.gain_r) = (v.base_gain_l * gain, v.base_gain_r * gain);
            retime(v);
        } else if cents != 0.0 {
            retime(v);
        }
        let beat = self.frames_per_beat();
        if let Some(g) = self.pool.grain_mut(slot).filter(|_| granular) {
            g.set_tempo(beat);
            g.set_params(tuned_grain(grain, cents, l.follow));
            // Seeded from the hit count, so the same performance renders the same.
            g.reseed(0x6772_6169_6e00 ^ self.clock);
            g.start();
        }
        let n = self.active_voices();
        if n > self.stats.peak_voices {
            self.stats.peak_voices = n;
        }
    }

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

    fn render_mixed(&mut self, left: &mut [f32], right: &mut [f32], deadline: Option<Instant>) {
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
    fn fetch_lane(&mut self) {
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
                    if v.active && p < PADS && self.edit_lane.get(k) == Some(&true) {
                        let live = &self.lives[k];
                        follow_edit(v, g, &self.views[k], &live.row(p), &live.program, true);
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
        }
        self.tape_moved = true;
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
                while let Some(&(o, kit, ev)) = self.events.get(applied) {
                    if o > at {
                        break;
                    }
                    applied += 1;
                    self.apply_for(usize::from(kit), ev);
                }
                next = self.events.get(applied).map_or(n, |&(o, _, _)| o.min(n));
            }
            if self.pool.split {
                next = next.min(at + LANE_BLOCK as u32);
            }
            let (a, b) = (at as usize, next as usize);
            let frame = self.frames + at as u64;
            self.follow_tape(b - a);
            self.pool.share_budget();
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
        self.events.drain(..applied);
        for e in &mut self.events {
            e.0 = e.0.saturating_sub(n);
        }
    }
}

/// One voice's next `left.len()` frames, added into `left` and `right`.
/// A granular voice that ends gives its grains back to the budget.
#[allow(clippy::too_many_arguments)]
fn play(
    v: &mut Voice,
    g: &mut GrainVoice,
    budget: &mut Budget,
    frame: u64,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
    sl: &mut [f32],
    sr: &mut [f32],
) {
    if v.granular {
        render_granular(v, g, budget, frame, rate, left, right, sl, sr);
        if !v.active {
            g.kill(budget);
            v.granular = false;
        }
    } else {
        render_voice(v, rate, left, right);
    }
}

/// A granular voice: grains from the voice's region, then the same
/// envelope, gain, filter and fades as every other voice.
#[allow(clippy::too_many_arguments)]
fn render_granular(
    v: &mut Voice,
    g: &mut GrainVoice,
    budget: &mut Budget,
    frame: u64,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
    sl: &mut [f32],
    sr: &mut [f32],
) {
    // Taken out of the voice for the render and put back after (a clone
    // would touch the shared count, which both cores' voices share).
    let Some(data) = v.data.take() else {
        v.active = false;
        return;
    };
    let ch = if v.stereo { 2 } else { 1 };
    let (start, end) = (v.start.max(0) as u64, v.end.max(0) as u64);
    // Clean pitching unless the pad is Vintage, from the sound's octave
    // copies when they were made by the time the voice started.
    let clean = v.grain_clean;
    let cell = v.octaves.take();
    let octaves = cell.as_deref().and_then(OctaveCell::get);
    // A Stretch's hits, kept sharp.
    let hit_cell = v.hits.take();
    let hits = hit_cell.as_deref().and_then(HitCell::get).unwrap_or(&[]);
    match &data {
        Pcm::F32(d) => match GrainClip::new(&d[..], ch, v.src_rate) {
            Some(c) => {
                let c = c.with_range(start, end).with_hits(hits);
                let c = if clean { c.clean(octaves) } else { c };
                grains(v, g, &c, budget, frame, rate, left, right, sl, sr)
            }
            None => v.active = false,
        },
        Pcm::I16(d) => match GrainClip::new(&d[..], ch, v.src_rate) {
            Some(c) => {
                let c = c.with_range(start, end).with_hits(hits);
                let c = if clean { c.clean(octaves) } else { c };
                grains(v, g, &c, budget, frame, rate, left, right, sl, sr)
            }
            None => v.active = false,
        },
    }
    v.octaves = cell;
    v.hits = hit_cell;
    if v.active {
        v.data = Some(data);
    }
}

/// [`render_granular`]'s grains from `clip`.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn grains<C: GrainSource>(
    v: &mut Voice,
    g: &mut GrainVoice,
    clip: &C,
    budget: &mut Budget,
    frame: u64,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
    sl: &mut [f32],
    sr: &mut [f32],
) {
    let mut at = 0;
    while at < left.len() && v.active {
        let n = (left.len() - at).min(sl.len());
        let (bl, br) = (&mut sl[..n], &mut sr[..n]);
        bl.fill(0.0);
        br.fill(0.0);
        let gliding = v.glide_left > 0;
        if gliding {
            // On to the next key, a part at a time.
            let k = (n as u32).min(v.glide_left);
            v.glide_left -= k;
            v.grain_hit = if v.glide_left == 0 {
                v.glide_to
            } else {
                v.grain_hit + v.glide_step * k as f32
            };
            let mut p = pitched_grain(v.layer_grain, v.grain_hit, v.grain_follow);
            p.position = v.grain_base.position;
            v.grain_base = p;
        }
        if gliding || v.route.target != PressureTarget::Off || v.grain_cents != v.follow.cents {
            let cents = v.follow.cents;
            g.set_params(tuned_grain(
                pressed_grain(v.grain_base, v.route, v.press),
                cents,
                v.grain_follow,
            ));
            v.grain_cents = cents;
        }
        g.render(clip, budget, frame + at as u64, bl, br);
        let follow = v.follow.moving;
        for k in 0..n {
            let (l, r) = if follow {
                finish::<true>(v, rate, bl[k], br[k], 1.0)
            } else {
                finish::<false>(v, rate, bl[k], br[k], 1.0)
            };
            left[at + k] += l;
            right[at + k] += r;
            if !v.active {
                break;
            }
        }
        at += n;
    }
}

/// Grain settings `cents` higher (the program's tune).
fn tuned_grain(g: GrainParams, cents: f32, follow: KeyFollow) -> GrainParams {
    if cents != 0.0 {
        pitched_grain(g, cents / 100.0, follow)
    } else {
        g
    }
}

/// Grain settings `semis` higher: the grains' pitch (and with key
/// tracking their size, shorter, and the scan, faster, as much), or for a
/// Tone region the density, with the size shortened as much so each grain
/// keeps as many cycles of the note.
fn pitched_grain(mut g: GrainParams, semis: f32, follow: KeyFollow) -> GrainParams {
    let k = (semis / 12.0).exp2();
    if follow.tone {
        g.density *= k;
        g.size_ms /= k;
    } else {
        g.pitch += semis;
        if follow.size {
            g.size_ms /= k;
        }
    }
    if follow.scan {
        g.scan *= k;
    }
    g
}

/// What a key moves on a granular voice ([`pitched_grain`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct KeyFollow {
    /// A Tone region: its density (the note), with its size shortened as
    /// much so each grain keeps its cycles; else the grains' pitch.
    tone: bool,
    /// The grains' size too (key tracking, [`GrainKeys::size`]).
    size: bool,
    /// The scan too ([`GrainKeys::scan`]).
    scan: bool,
}

/// A voice's filter with the live settings on it: `cutoff` and `resonance`
/// in place of its own, then the program's offset `octaves` on the cutoff.
/// A voice without a filter gets an open 2-pole low-pass when a cutoff is
/// set or the offset darkens.
fn live_filter(
    f: Option<Filter>,
    cutoff: Option<f32>,
    resonance: Option<f32>,
    octaves: f32,
) -> Option<Filter> {
    let mut f = match f {
        Some(f) => f,
        None if cutoff.is_some() || octaves < 0.0 => OPEN_FILTER,
        None => return None,
    };
    if let Some(hz) = cutoff {
        f.cutoff_hz = hz;
    }
    if let Some(r) = resonance {
        f.resonance = r;
    }
    if octaves != 0.0 {
        f.cutoff_hz *= octaves.exp2();
    }
    Some(f)
}

/// The gain (left, right) of a voice whose gain before the pad's level
/// and pan is `unit`, at `db` and pan `pan` (its layer's pan adds to it).
fn live_gains(unit: f32, layer_pan: f32, db: f32, pan: f32) -> (f32, f32) {
    let (pl, pr) = pan_gains(layer_pan + pan);
    let g = unit * db_to_gain(db);
    (g * pl, g * pr)
}

/// Points playing voice `v` at the live settings (`live`, its pad's row)
/// and the program's offsets: it glides there from where it is. A voice
/// fading out (choked, stolen) is left alone.
/// Lets a held voice go as the next hit on a toggle or latched pad does:
/// a loop leaves its loop and plays its tail, grains stop, and anything
/// else releases.
fn let_go(v: &mut Voice, g: &mut GrainVoice) {
    v.held = false;
    match v.mode {
        Some(PlayMode::Loop) => {
            v.looping = false;
            v.backwards = v.reverse;
        }
        Some(PlayMode::Granular) => {
            g.stop();
            if let Some(a) = &mut v.amp {
                a.release();
            }
        }
        _ => {
            if let Some(a) = &mut v.amp {
                a.release();
            }
        }
    }
}

fn retarget(v: &mut Voice, live: &[f32; PadField::COUNT], program: &[f32; ProgramField::COUNT]) {
    if !v.active || v.kill_left > 0 {
        return;
    }
    let get = |f: PadField| Some(live[f as usize]).filter(|x| !x.is_nan());
    let f = &mut v.follow;
    let db = get(PadField::Level).unwrap_or(f.own_db) + program[ProgramField::Level as usize];
    let pan = get(PadField::Pan).unwrap_or(f.own_pan);
    let (l, r) = live_gains(v.unit_gain, v.layer_pan, db, pan);
    f.gain_to = [l, r];
    let cutoff = get(PadField::Cutoff);
    let octaves = program[ProgramField::Cutoff as usize];
    if v.filter.is_none() && (cutoff.is_some() || octaves < 0.0) {
        // From open, so it comes in without a click.
        v.filter = Some(OPEN_FILTER);
        v.fenv = Some(Envelope::idle());
        v.svf = [Svf::default(); 2];
        v.filter_key = None;
        f.own_fc = OPEN_FILTER.cutoff_hz;
    }
    if v.filter.is_some() {
        f.fc_to = cutoff.unwrap_or(f.own_fc) * octaves.exp2();
        f.res_to = get(PadField::Resonance).unwrap_or(f.own_res);
    }
    f.cents_to = if v.warp_bpm > 0.0 {
        0.0
    } else {
        program[ProgramField::Tune as usize]
    };
    if !f.moving {
        // A voice that had landed may have run on through the rest of
        // that render call's loop, or not: its steps start afresh on the
        // event's frame, so blocks of any size play it the same.
        f.count = 0;
    }
    f.moving = true;
}

/// Playing voice `v` (with its grain voice `g`) follows an edit of its pad
/// in `view` ([`Engine::edit_view`]), if its layer there still plays its
/// sound: the loop points at once, a new tune at once, a granular voice's
/// grain settings, and the level, pan, cutoff and resonance gliding there
/// from where they are, under `live` (its pad's live row) and `program`.
/// `was_live`: it followed a live value that the new view cleared, so it
/// glides back to its own even if the edit didn't change it.
fn follow_edit(
    v: &mut Voice,
    g: &mut GrainVoice,
    view: &EngineView,
    live: &[f32; PadField::COUNT],
    program: &[f32; ProgramField::COUNT],
    was_live: bool,
) {
    if !v.active || v.kill_left > 0 || v.audition {
        return;
    }
    let Some(vp) = view.pads.get(usize::from(v.pad)).and_then(Option::as_ref) else {
        return;
    };
    let Some(l) = vp.layers.get(usize::from(v.layer)) else {
        return;
    };
    let same_sound = v.stereo == l.stereo
        && v.src_rate == l.src_rate
        && v.data.as_ref().is_some_and(|d| d.ptr_eq(&l.data));
    if !same_sound {
        return;
    }
    let f = v.follow;
    let mut moved = was_live;

    // Level and pan: the layer's gain and pan under the hit's own.
    let unit_gain = l.unit_gain * v.hit_gain;
    if unit_gain != v.unit_gain
        || l.pan != v.layer_pan
        || vp.level_db != f.own_db
        || vp.pan != f.own_pan
    {
        (v.unit_gain, v.layer_pan) = (unit_gain, l.pan);
        (v.follow.own_db, v.follow.own_pan) = (vp.level_db, vp.pan);
        moved = true;
    }

    // The filter's cutoff and resonance, where the filter is still the
    // same kind (and its cutoff the pad's, not 16 Levels'). A low-pass
    // turned on comes in from open; one turned off opens.
    let pf = vp.filter;
    let (fc, res) = match v.filter {
        _ if !v.own_cut => (f.own_fc, f.own_res),
        Some(cur) if cur.kind == pf.kind && cur.poles == pf.poles => (pf.cutoff_hz, pf.resonance),
        Some(cur) if cur.kind == FilterKind::LowPass && pf.kind == FilterKind::Off => {
            (OPEN_FILTER.cutoff_hz, 0.0)
        }
        None if pf.kind == FilterKind::LowPass => {
            v.filter = Some(Filter {
                cutoff_hz: OPEN_FILTER.cutoff_hz,
                resonance: 0.0,
                ..pf
            });
            v.fenv = Some(Envelope::idle());
            v.svf = [Svf::default(); 2];
            v.filter_key = None;
            (pf.cutoff_hz, pf.resonance)
        }
        _ => (f.own_fc, f.own_res),
    };
    if fc != f.own_fc || res != f.own_res {
        (v.follow.own_fc, v.follow.own_res) = (fc, res);
        moved = true;
    }

    // Tune: the layer's speed against the one it played at. A warped
    // layer's speed is the tempo's; a granular voice's pitch is its grains'.
    if !v.granular && v.warp_bpm == 0.0 && l.warp_bpm == 0.0 && l.step != v.layer_step {
        let ratio = l.step as f64 / v.layer_step.max(1) as f64;
        v.plain_step = ((v.plain_step as f64 * ratio).round() as i64).max(1);
        v.base_step = v.plain_step;
        v.layer_step = l.step;
        retime(v);
    }

    // A granular voice: the layer's grain settings, with what its hit
    // added (a keygroup's semitones, 16 Levels' position) kept.
    // Out of Stretch, it stops keeping hits.
    if l.hits.is_none() {
        v.hits = None;
    }
    if v.granular
        && l.mode == PlayMode::Granular
        && (l.grain != v.layer_grain || l.follow != v.grain_follow)
    {
        v.grain_follow = l.follow;
        let mut p = pitched_grain(l.grain, v.grain_hit, v.grain_follow);
        if v.grain_base.position != v.layer_grain.position {
            p.position = v.grain_base.position;
        }
        v.grain_base = p;
        v.layer_grain = l.grain;
        g.set_params(tuned_grain(
            pressed_grain(p, v.route, v.press),
            v.follow.cents,
            v.grain_follow,
        ));
        v.grain_cents = v.follow.cents;
    }

    if v.mode == Some(l.mode) && !v.granular && !v.reverse && !l.reverse {
        follow_loop(v, l);
    }
    if moved {
        retarget(v, live, program);
    }
}

/// A voice of a looping mode follows its layer's loop ([`follow_edit`]):
/// the new points, kept inside the region it plays; if it is now past
/// the loop, it jumps in, the step smoothed ([`JUMP_FRAMES`]). A loop
/// taken away lets it play on to its end; one added while it is held
/// starts looping.
fn follow_loop(v: &mut Voice, l: &ViewLayer) {
    if !matches!(
        v.mode,
        Some(PlayMode::Loop | PlayMode::Toggle | PlayMode::Gate)
    ) {
        return;
    }
    let Some(lp) = l.sustain_loop.as_ref() else {
        if v.looping {
            v.looping = false;
            v.backwards = false;
        }
        return;
    };
    let (start, end) = (lp.start.max(v.start), lp.end.min(v.end));
    if end - start < 2 {
        return;
    }
    let looping = v.looping || v.held;
    let unchanged = (v.loop_start, v.loop_end, v.ping_pong) == (start, end, lp.ping_pong);
    v.loop_start = start;
    v.loop_end = end;
    v.ping_pong = lp.ping_pong;
    // Its blend reads before the loop's start, never before the region's.
    v.crossfade = lp
        .crossfade
        .min((end - start) / 2)
        .min(start - v.start)
        .max(0);
    if !looping || (unchanged && v.looping) {
        return;
    }
    v.looping = true;
    let i = v.pos >> 32;
    let fraction = v.pos & (ONE - 1);
    let (pos, backwards) = if v.ping_pong {
        if !v.backwards && i >= end {
            (((end - 1) << 32) | fraction, true)
        } else if v.backwards && i < start {
            ((start << 32) | fraction, false)
        } else {
            return;
        }
    } else if i >= end {
        let into = (v.pos - (start << 32)) % ((end - start) << 32);
        ((start << 32) + into, false)
    } else {
        return;
    };
    // The read on each side of the jump, so the step fades out instead
    // of clicking (with what is left of an earlier jump's).
    if let Some(data) = v.data.as_ref() {
        let ((al, ar), (bl, br)) = match data {
            Pcm::F32(d) => (v.read2(d, v.pos), v.read2(d, pos)),
            Pcm::I16(d) => (v.read2(d, v.pos), v.read2(d, pos)),
        };
        let left = f32::from(v.jump_left) / f32::from(JUMP_FRAMES);
        v.jump = [v.jump[0] * left + al - bl, v.jump[1] * left + ar - br];
        v.jump_left = JUMP_FRAMES;
        if !v.follow.moving {
            v.follow.count = 0;
        }
        v.follow.moving = true;
    }
    v.pos = pos;
    v.backwards = backwards;
}

/// One frame of following the live settings: the gain's step, and every
/// [`FILTER_STEP`] frames the next steps ([`live_step`]). Only the loops of
/// a voice that is following call it ([`render_voice`]).
#[inline(always)]
fn follow_live(v: &mut Voice) {
    if v.follow.count == 0 {
        live_step(v);
    }
    v.follow.count = (v.follow.count + 1) % FILTER_STEP;
    v.base_gain_l += v.follow.gain_inc[0];
    v.base_gain_r += v.follow.gain_inc[1];
    v.gain_l = v.base_gain_l * v.tape_gain;
    v.gain_r = v.base_gain_r * v.tape_gain;
}

/// [`follow_live`]'s step every [`FILTER_STEP`] frames: each value moves
/// `k` of the way to its target (the cutoff in octaves), the gain linearly
/// across the step; one close enough lands. The voice stops following when
/// all have landed.
#[inline(never)]
fn live_step(v: &mut Voice) {
    let f = &mut v.follow;
    let k = f.k;
    let mut moving = false;
    let mut glide = |at: &mut f32, to: f32, inc: &mut f32| {
        let d = to - *at;
        if d.abs() <= 1e-5 * to.abs().max(at.abs()) + 1e-9 {
            *at = to;
            *inc = 0.0;
        } else {
            *inc = d * k / FILTER_STEP as f32;
            moving = true;
        }
    };
    glide(&mut v.base_gain_l, f.gain_to[0], &mut f.gain_inc[0]);
    glide(&mut v.base_gain_r, f.gain_to[1], &mut f.gain_inc[1]);
    if let Some(flt) = v.filter.as_mut().filter(|_| f.fc_to > 0.0) {
        let (fc, res) = (flt.cutoff_hz, flt.resonance);
        let ratio = f.fc_to / fc;
        if (ratio - 1.0).abs() <= 1e-4 {
            flt.cutoff_hz = f.fc_to;
        } else {
            flt.cutoff_hz = fc * ratio.powf(k);
            moving = true;
        }
        let d = f.res_to - res;
        if d.abs() <= 1e-4 {
            flt.resonance = f.res_to;
        } else {
            flt.resonance = res + d * k;
            moving = true;
        }
        if flt.cutoff_hz != fc || flt.resonance != res {
            v.filter_key = None;
        }
    }
    let d = f.cents_to - f.cents;
    if d != 0.0 {
        if d.abs() <= 0.01 {
            f.cents = f.cents_to;
        } else {
            f.cents += d * k;
            moving = true;
        }
        retime(v);
    }
    v.follow.moving = moving || v.jump_left > 0;
}

/// Every voice of `pool` idle at once, its grains handed back.
fn silence(pool: &mut Pool) {
    let Pool {
        voices,
        grains,
        budget,
        ..
    } = pool;
    for (v, g) in voices.iter_mut().zip(grains.iter_mut()) {
        if v.granular {
            g.kill(budget);
        }
        v.active = false;
        v.granular = false;
        v.held = false;
        v.kill_left = 0;
        v.tail_left = 0;
        v.follow.moving = false;
        v.data = None;
    }
}

/// Grain settings with pressure `p` (0 to 1) applied along `route`.
fn pressed_grain(mut g: GrainParams, route: PressureRoute, p: f32) -> GrainParams {
    let d = route.depth * p;
    match route.target {
        PressureTarget::Pitch => g.pitch += 12.0 * d,
        PressureTarget::GrainDensity => g.density *= (4.0 * d).exp2(),
        PressureTarget::GrainSize => g.size_ms *= (3.0 * d).exp2(),
        PressureTarget::GrainPosition => g.position = (g.position + d).rem_euclid(1.0),
        PressureTarget::GrainSpray => g.spray = (g.spray + d).clamp(0.0, 1.0),
        PressureTarget::Off | PressureTarget::Level | PressureTarget::Filter => {}
    }
    g
}

/// Smooths the voice's pressure and, every [`FILTER_STEP`] frames, updates
/// what it moves on the voice itself: gain, step (pitch) and cutoff.
#[inline]
fn follow_pressure(v: &mut Voice) {
    v.press += (v.press_target - v.press) * v.press_coef;
    if v.press_count == 0 {
        pressure_step(v);
    }
    v.press_gain += v.press_gain_inc;
    v.press_count = (v.press_count + 1) % FILTER_STEP;
}

/// A warped layer's step at `tempo`: `tempo / warp_bpm` times its own.
fn warped_step(step: i64, tempo: f64, warp_bpm: f64) -> i64 {
    ((step as f64 * tempo / warp_bpm).round() as i64).max(1)
}

/// [`follow_pressure`]'s update every [`FILTER_STEP`] frames, kept out of
/// the per-frame loop.
#[inline(never)]
fn pressure_step(v: &mut Voice) {
    // The smoothing lands, so pressure let go doesn't creep toward zero
    // through the denormals.
    if (v.press_target - v.press).abs() < TINY {
        v.press = v.press_target;
    }
    let d = v.route.depth * v.press;
    match v.route.target {
        PressureTarget::Level => {
            v.press_gain_inc = (db_to_gain(12.0 * d) - v.press_gain) / FILTER_STEP as f32
        }
        PressureTarget::Pitch => retime(v),
        PressureTarget::Filter => v.press_fc = (4.0 * d).exp2(),
        _ => {}
    }
}

/// The voice's step from its base step, its pitch pressure, its speed
/// (the tape's, with wow and flutter) and the program's tune. A granular
/// voice's grains keep their own pitch.
fn retime(v: &mut Voice) {
    if v.granular {
        return;
    }
    let mut step = v.base_step as f64;
    if v.route.target == PressureTarget::Pitch {
        step *= f64::from(v.route.depth * v.press).exp2();
    }
    if v.speed != 1.0 {
        step *= f64::from(v.speed);
    }
    if v.follow.cents != 0.0 {
        step *= f64::from(v.follow.cents / 1200.0).exp2();
    }
    v.step = step.round().max(1.0) as i64;
}

/// Sets the voice's filter for envelope level `e`. The coefficients only
/// depend on `e` and pressure's cutoff factor, so while neither moves (a
/// held sustain, no pressure) this skips the `exp2` and `tan`; the second
/// stage is only set for a 4-pole filter, the only one that uses it.
/// It also flushes the filter's state: ringing out on silence, the filter
/// reaches exact zeros instead of denormals (here, out of the per-frame
/// loop, it costs nothing measurable).
#[inline(never)]
fn retune(v: &mut Voice, f: Filter, e: f32, rate: f32) {
    v.svf[0].flush();
    if f.poles >= 4 {
        v.svf[1].flush();
    }
    let key = (e.to_bits(), v.press_fc.to_bits());
    if v.filter_key == Some(key) {
        return;
    }
    v.filter_key = Some(key);
    let fc = f.cutoff_hz * (f.env_octaves * e).exp2() * v.press_fc;
    v.svf[0].set(fc, f.resonance, rate);
    if f.poles >= 4 {
        v.svf[1].set(fc, f.resonance * 0.5, rate);
    }
}

/// Everything after the source read, for one frame: amp envelope, kill
/// fade, gain and pan, filter, the stolen voice's tail. `g` is the gain
/// from the region's fades. Marks the voice done when its kill fade or
/// envelope ends.
#[inline(always)]
fn finish<const FOLLOW: bool>(
    v: &mut Voice,
    rate: f32,
    mut l: f32,
    mut r: f32,
    mut g: f32,
) -> (f32, f32) {
    if FOLLOW {
        follow_live(v);
    }
    if v.route.target != PressureTarget::Off {
        follow_pressure(v);
        g *= v.press_gain;
    }
    if let Some(a) = &mut v.amp {
        g *= a.next();
    }
    if v.kill_left > 0 {
        g *= f32::from(v.kill_left) / f32::from(KILL_FRAMES);
    }

    l *= g * v.gain_l;
    r *= g * v.gain_r;

    if let (Some(f), Some(fe)) = (v.filter, v.fenv.as_mut()) {
        let e = fe.next();
        if v.filter_count == 0 {
            retune(v, f, e, rate);
        }
        v.filter_count = (v.filter_count + 1) % FILTER_STEP;
        l = v.svf[0].process(0, l, f.kind);
        r = v.svf[0].process(1, r, f.kind);
        if f.poles >= 4 {
            l = v.svf[1].process(0, l, f.kind);
            r = v.svf[1].process(1, r, f.kind);
        }
    }

    if v.tail_left > 0 {
        let t = f32::from(v.tail_left) / f32::from(STEAL_FRAMES);
        l += v.tail_l * t;
        r += v.tail_r * t;
        v.tail_left -= 1;
    }
    v.last_l = l;
    v.last_r = r;

    if v.kill_left > 0 {
        v.kill_left -= 1;
        if v.kill_left == 0 {
            v.active = false;
        }
    }
    if v.amp.as_ref().is_some_and(|a| a.is_done()) {
        v.active = false;
    }
    (l, r)
}

/// A sample voice's next `left.len()` frames, added into `left` and
/// `right`. Whether it follows a live setting is settled once a call (only
/// events start that, and they come between calls), so a voice that isn't
/// following runs the loop it always did.
fn render_voice(v: &mut Voice, rate: f32, left: &mut [f32], right: &mut [f32]) {
    if v.bl || v.step > v.bl_above.0 {
        render_voice_bl(v, rate, left, right);
    } else {
        let done = if v.follow.moving {
            render_voice_as::<true, false>(v, rate, left, right)
        } else if plain_now(v) {
            render_plain(v, left, right);
            left.len()
        } else {
            render_voice_as::<false, false>(v, rate, left, right)
        };
        if done < left.len() && v.active {
            // Its step passed CLEAN_ABOVE on a frame (a glide, pressure):
            // the rest of the block is the band-limited read's.
            render_voice_bl(v, rate, &mut left[done..], &mut right[done..]);
        }
    }
    v.heard = (v.heard * HEARD_FALL).max(v.last_l.abs().max(v.last_r.abs()));
    if v.heard < TINY {
        v.heard = 0.0;
    }
}

/// [`render_voice`] for a voice reading band-limited, from now on. Kept
/// out of line, so the plain and Hermite paths stay as small as they were.
#[inline(never)]
fn render_voice_bl(v: &mut Voice, rate: f32, left: &mut [f32], right: &mut [f32]) {
    v.bl = true;
    if v.follow.moving {
        render_voice_as::<true, true>(v, rate, left, right);
    } else {
        render_voice_as::<false, true>(v, rate, left, right);
    }
}

/// Whether an amp envelope with these settings soon holds a level (its
/// sustain, reached within a couple of frames) until a release, so a voice
/// with it plays plain ([`plain_now`]) after its first frames.
fn amp_holds(a: &Ahdsr, rate: f32) -> bool {
    let frames = |ms: f32| ms.max(0.0) * 0.001 * rate;
    frames(a.attack_ms) < 1.0
        && frames(a.hold_ms) < 1.0
        && a.sustain > 0.0
        && (a.sustain >= 1.0 || frames(a.decay_ms) < 1.0)
}

/// Whether the voice's next frames are a plain copy of its sound at a
/// fixed gain, so [`render_plain`] gives exactly what the full loop would:
/// forward at the sound's own speed from a whole frame (no Hermite
/// blend), no loop, fades, filter or pressure, its envelope holding a
/// level, and no kill fade or stolen voice's tail under way. A kit's
/// drum hits are like this from their second frame to their last.
#[inline]
fn plain_now(v: &Voice) -> bool {
    v.step == ONE
        && !v.backwards
        && !v.looping
        && v.pos & (ONE - 1) == 0
        && v.fade_in.is_none()
        && v.fade_out.is_none()
        && v.filter.is_none()
        && v.route.target == PressureTarget::Off
        && v.kill_left == 0
        && v.tail_left == 0
        && v.amp.as_ref().is_none_or(|a| a.is_steady())
        && (!v.stereo || (v.pick_l == 0 && v.pick_r == 1))
}

/// A plain voice's next frames ([`plain_now`]): each frame of its sound
/// times its gain, added in. Bit for bit what [`render_voice_as`] gives
/// for such a voice (`a_plain_voice_renders_as_the_full_loop_does`), at a
/// fraction of the cost: the loop is a multiply and an add a sample.
fn render_plain(v: &mut Voice, left: &mut [f32], right: &mut [f32]) {
    let Some(data) = v.data.take() else {
        v.active = false;
        return;
    };
    match &data {
        Pcm::F32(d) => render_plain_from(v, d, left, right),
        Pcm::I16(d) => render_plain_from(v, d, left, right),
    }
    if v.active {
        v.data = Some(data);
    }
}

/// [`render_plain`] from the samples as the sound holds them.
#[inline(always)]
fn render_plain_from<S: Sample>(v: &mut Voice, data: &[S], left: &mut [f32], right: &mut [f32]) {
    let i = v.pos >> 32;
    let n = if i < v.start {
        0
    } else {
        left.len().min((v.end - i).max(0) as usize)
    };
    // The full loop's gain: the envelope's level (its `g`), times the
    // voice's gain on each side.
    let g = v.amp.as_ref().map_or(1.0, |a| a.level());
    let (gl, gr) = (g * v.gain_l, g * v.gain_r);
    // Raw samples times the gain with the samples' scale in it: exactly
    // the scaled samples times the gain (the scale is a power of two).
    let (sl, sr) = (gl * S::SCALE, gr * S::SCALE);
    if n > 0 {
        let k = i as usize;
        let (left, right) = (&mut left[..n], &mut right[..n]);
        if v.stereo {
            let src = &data[2 * k..2 * (k + n)];
            for ((ol, or), f) in left
                .iter_mut()
                .zip(right.iter_mut())
                .zip(src.chunks_exact(2))
            {
                *ol += f[0].raw() * sl;
                *or += f[1].raw() * sr;
            }
            v.last_l = src[2 * n - 2].raw() * sl;
            v.last_r = src[2 * n - 1].raw() * sr;
        } else {
            let src = &data[k..k + n];
            for ((ol, or), x) in left.iter_mut().zip(right.iter_mut()).zip(src) {
                *ol += x.raw() * sl;
                *or += x.raw() * sr;
            }
            v.last_l = src[n - 1].raw() * sl;
            v.last_r = src[n - 1].raw() * sr;
        }
        v.pos += (n as i64) << 32;
    }
    if n < left.len() {
        // It reached its end within the block, as the full loop finds.
        v.active = false;
    }
}

/// How much a voice's [`Voice::heard`] level falls each render it is not
/// held up by its output (about 3 dB in four renders).
const HEARD_FALL: f32 = 0.84;

/// Sets which copy a band-limited voice reads and how far its sinc
/// stretches, for its step now: its sound, or the octave copy that brings
/// its speed under [`bandlimit::MAX_STRETCH`] (positions and the region
/// shift right by the copy's octave). Set again whenever its step moves,
/// on that frame, so the sound doesn't depend on how the blocks fall.
#[inline]
fn aim_bl(v: &mut Voice, octaves: Option<&bandlimit::Octaves>) {
    let mut r = v.step as f32 / ONE as f32;
    let mut octave = 0u32;
    if let Some(o) = octaves {
        while r >= bandlimit::MAX_STRETCH && o.level(octave as usize + 1).is_some() {
            octave += 1;
            r *= 0.5;
        }
    }
    v.bl_octave = octave;
    v.bl_reach = Reach::new(r);
    v.bl_step = v.step;
}

/// Both output channels of a band-limited voice at `pos` ([`aim_bl`] says
/// from what).
#[inline(always)]
fn read_bl<S: Sample>(
    v: &Voice,
    kernel: &bandlimit::Kernel,
    data: &[S],
    octaves: Option<&bandlimit::Octaves>,
    pos: i64,
) -> (f32, f32) {
    let k = v.bl_octave;
    let (lo, hi, at) = (v.start >> k, (v.end - 1) >> k, pos >> k);
    let picks = (v.pick_l, v.pick_r);
    match octaves.and_then(|o| o.level(k as usize)) {
        // The copies are floats, whatever the sound holds.
        Some(copy) if k > 0 => {
            bandlimit::read2(kernel, &copy[..], v.stereo, lo, hi, at, v.bl_reach, picks)
        }
        _ => bandlimit::read2(kernel, data, v.stereo, lo, hi, at, v.bl_reach, picks),
    }
}

/// A sample voice's next frames, added into `left` and `right`, read with
/// Hermite or (`BL`) band-limited. Returns the frames it rendered: all of
/// them, unless the voice ended, or (not `BL`) its pitch went past
/// [`CLEAN_ABOVE`] on a frame (a glide, pressure), after which the rest of
/// the block is the band-limited read's ([`render_voice`] carries on with
/// it).
fn render_voice_as<const FOLLOW: bool, const BL: bool>(
    v: &mut Voice,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
) -> usize {
    // Taken out of the voice for the render and put back after (a clone
    // would touch the shared count, which both cores' voices share).
    let Some(data) = v.data.take() else {
        v.active = false;
        return left.len();
    };
    let done = match &data {
        Pcm::F32(d) => render_voice_from::<f32, FOLLOW, BL>(v, d, rate, left, right),
        Pcm::I16(d) => render_voice_from::<i16, FOLLOW, BL>(v, d, rate, left, right),
    };
    if v.active {
        v.data = Some(data);
    }
    done
}

/// [`render_voice_as`] from the samples as the sound holds them.
#[inline(always)]
fn render_voice_from<S: Sample, const FOLLOW: bool, const BL: bool>(
    v: &mut Voice,
    data: &[S],
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
) -> usize {
    // The band-limited read's copies, taken only by a voice that reads
    // with them, and its kernel, fetched once a call.
    let octaves = if BL { v.octaves.take() } else { None };
    let copies = octaves.as_deref().and_then(OctaveCell::get);
    let kernel = bandlimit::kernel();
    let read = |v: &Voice, pos: i64| -> (f32, f32) {
        if BL {
            read_bl(v, kernel, data, copies, pos)
        } else {
            v.read2(data, pos)
        }
    };
    let mut done = left.len();
    for (k, (ol, or)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
        let i = v.pos >> 32;
        if i < v.start || i >= v.end {
            v.active = false;
            done = k;
            break;
        }
        if BL && v.step != v.bl_step {
            aim_bl(v, copies);
        }

        let (mut l, mut r) = read(v, v.pos);

        // Forward-loop crossfade: blend in the audio before the loop start.
        if v.looping && !v.ping_pong && v.crossfade > 0 && i >= v.loop_end - v.crossfade {
            let x = (i - (v.loop_end - v.crossfade)) as f32 / v.crossfade as f32;
            let (out, inn) = equal_power(x);
            let back = v.pos - ((v.loop_end - v.loop_start) << 32);
            let (bl, br) = read(v, back);
            l = l * out + bl * inn;
            r = r * out + br * inn;
        }
        // An edit's jump into a moved loop: the step fading out. A voice
        // follows while one is under way ([`live_step`]).
        if FOLLOW && v.jump_left > 0 {
            let t = f32::from(v.jump_left) / f32::from(JUMP_FRAMES);
            l += v.jump[0] * t;
            r += v.jump[1] * t;
            v.jump_left -= 1;
        }

        let mut g = 1.0f32;
        if let Some(f) = v.fade_in {
            let d = if v.reverse {
                v.end - 1 - i
            } else {
                i - v.start
            };
            if d < i64::from(f.frames) {
                g *= f.shape(d as f32 / f.frames as f32);
            }
        }
        if let Some(f) = v.fade_out {
            let d = if v.reverse {
                i - v.start
            } else {
                v.end - 1 - i
            };
            if d < i64::from(f.frames) {
                g *= f.shape(d as f32 / f.frames as f32);
            }
        }
        let (l, r) = finish::<FOLLOW>(v, rate, l, r, g);
        *ol += l;
        *or += r;

        // Advance.
        if v.backwards {
            v.pos -= v.step;
        } else {
            v.pos += v.step;
        }
        let j = v.pos >> 32;
        if v.looping {
            if v.ping_pong {
                if !v.backwards && j >= v.loop_end {
                    v.backwards = true;
                    v.pos = ((v.loop_end - 1) << 32) - (v.pos - (v.loop_end << 32));
                } else if v.backwards && j < v.loop_start {
                    v.backwards = false;
                    v.pos = (v.loop_start << 32) + ((v.loop_start << 32) - v.pos);
                }
            } else if j >= v.loop_end {
                v.pos -= (v.loop_end - v.loop_start) << 32;
            }
        }

        if !v.active {
            done = k + 1;
            break;
        }
        if !BL && v.step > v.bl_above.0 {
            // Past two semitones up from the next frame on.
            done = k + 1;
            break;
        }
    }
    if v.active && BL {
        v.octaves = octaves;
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The direct read gives what the clamped one does, bit for bit, at
    /// every kind of position: whole and fractional frames, the region's
    /// first and last frames and just outside them, for mono and stereo
    /// and every channel pick.
    #[test]
    fn the_direct_read_matches_the_clamped_one() {
        let frames = 64usize;
        let stereo: Vec<f32> = (0..frames * 2)
            .map(|i| ((i * 7919) % 1000) as f32 / 500.0 - 1.0)
            .collect();
        let mono: Vec<f32> = (0..frames)
            .map(|i| ((i * 104_729) % 977) as f32 / 488.5 - 1.0)
            .collect();
        let fracs = [0u64, 1, 1 << 31, 0x1234_5678, u64::from(u32::MAX)];
        for (data, is_stereo) in [(&stereo, true), (&mono, false)] {
            for pick in [
                ChannelPick::Stereo,
                ChannelPick::MonoSum,
                ChannelPick::Left,
                ChannelPick::Right,
            ] {
                let (pick_l, pick_r) = picks(is_stereo, pick);
                let v = Voice {
                    stereo: is_stereo,
                    pick_l,
                    pick_r,
                    start: 5,
                    end: 40,
                    ..Voice::default()
                };
                for i in 0..frames as i64 {
                    for &t in &fracs {
                        let pos = (i << 32) | t as i64;
                        let (l, r) = v.read2(data, pos);
                        let (wl, wr) = (v.read(data, pos, pick_l), v.read(data, pos, pick_r));
                        assert_eq!(
                            (l.to_bits(), r.to_bits()),
                            (wl.to_bits(), wr.to_bits()),
                            "{pick:?} stereo {is_stereo} at {i} + {t:#x}"
                        );
                    }
                }
            }
        }
    }

    /// A sound held as 16 bits reads as its floats do, bit for bit: the
    /// direct read and the clamped one, mono and stereo, every pick, at
    /// whole and fractional frames and the region's edges.
    #[test]
    fn a_16_bit_sound_reads_as_its_floats_do() {
        let frames = 64usize;
        let ints: Vec<i16> = (0..frames * 2)
            .map(|i| (((i * 7919) % 1000) as i32 * 65 - 32_500) as i16)
            .collect();
        let floats: Vec<f32> = ints.iter().map(|&k| f32::from(k) / 32_768.0).collect();
        let fracs = [0u64, 1, 1 << 31, 0x1234_5678, u64::from(u32::MAX)];
        for is_stereo in [true, false] {
            let n = if is_stereo { frames * 2 } else { frames };
            let (ints, floats) = (&ints[..n], &floats[..n]);
            for pick in [
                ChannelPick::Stereo,
                ChannelPick::MonoSum,
                ChannelPick::Left,
                ChannelPick::Right,
            ] {
                let (pick_l, pick_r) = picks(is_stereo, pick);
                let v = Voice {
                    stereo: is_stereo,
                    pick_l,
                    pick_r,
                    start: 5,
                    end: 40,
                    ..Voice::default()
                };
                for i in 0..frames as i64 {
                    for &t in &fracs {
                        let pos = (i << 32) | t as i64;
                        let (a, b) = (v.read2(ints, pos), v.read2(floats, pos));
                        assert_eq!(
                            (a.0.to_bits(), a.1.to_bits()),
                            (b.0.to_bits(), b.1.to_bits()),
                            "{pick:?} stereo {is_stereo} at {i} + {t:#x}"
                        );
                    }
                }
            }
        }
    }

    /// Pressure let go lands on zero: its smoothing never creeps toward
    /// zero through the denormals, which cost far more than a normal
    /// number (7 times, for 16 pads held after pressure).
    #[test]
    fn pressure_let_go_lands_on_exact_zero() {
        let mut v = Voice {
            route: PressureRoute {
                target: PressureTarget::Level,
                depth: 0.5,
            },
            press: 1.0,
            press_target: 0.0,
            press_coef: 1.0 - (-1.0 / (PRESSURE_SMOOTH_S * 44_100.0)).exp(),
            press_gain: 1.0,
            ..Voice::default()
        };
        for frame in 0..44_100 {
            follow_pressure(&mut v);
            assert!(
                v.press == 0.0 || v.press.abs() >= f32::MIN_POSITIVE,
                "a denormal at frame {frame}"
            );
        }
        assert_eq!(v.press, 0.0);
    }

    /// A plain voice gives the same samples, bit for bit, from
    /// `render_plain` as from the full loop: stereo and mono, from the
    /// start and from part way in, at full and at a held sustain level,
    /// running out within a block and exactly at its end; and it ends
    /// (or plays on) on the same frame.
    #[test]
    fn a_plain_voice_renders_as_the_full_loop_does() {
        let rate = 44_100.0;
        let frames = 300usize;
        let stereo: Arc<[f32]> = (0..frames * 2)
            .map(|i| ((i * 7919) % 1000) as f32 / 500.0 - 1.0)
            .collect::<Vec<_>>()
            .into();
        let mono: Arc<[f32]> = (0..frames)
            .map(|i| ((i * 104_729) % 977) as f32 / 488.5 - 1.0)
            .collect::<Vec<_>>()
            .into();
        // And the same held as 16 bits.
        let stereo16: Arc<[i16]> = (0..frames * 2)
            .map(|i| (((i * 7919) % 1000) as i32 * 65 - 32_500) as i16)
            .collect::<Vec<_>>()
            .into();
        let mono16: Arc<[i16]> = (0..frames)
            .map(|i| (((i * 104_729) % 977) as i32 * 67 - 32_700) as i16)
            .collect::<Vec<_>>()
            .into();
        let sounds = [
            (Pcm::F32(stereo), true),
            (Pcm::F32(mono), false),
            (Pcm::I16(stereo16), true),
            (Pcm::I16(mono16), false),
        ];
        for (data, is_stereo) in &sounds {
            let is_stereo = *is_stereo;
            for sustain in [1.0f32, 0.7] {
                for (start, end, from) in [(0i64, 300i64, 0i64), (10, 256, 30), (3, 131, 3)] {
                    let mut amp = Envelope::start(
                        &Ahdsr {
                            sustain,
                            ..Ahdsr::OPEN
                        },
                        rate,
                    );
                    for _ in 0..4 {
                        amp.next();
                    }
                    assert!(amp.is_steady());
                    let (pick_l, pick_r) = picks(is_stereo, ChannelPick::Stereo);
                    let v = Voice {
                        active: true,
                        data: Some(data.clone()),
                        stereo: is_stereo,
                        pick_l,
                        pick_r,
                        pos: from << 32,
                        step: ONE,
                        start,
                        end,
                        gain_l: 0.83,
                        gain_r: -0.41,
                        amp: Some(amp),
                        ..Voice::default()
                    };
                    assert!(plain_now(&v));
                    let (mut a, mut b) = (v.clone(), v);
                    for block in 0..4 {
                        let (mut al, mut ar) = ([0.25f32; 128], [-0.5f32; 128]);
                        let (mut bl, mut br) = (al, ar);
                        render_plain(&mut a, &mut al, &mut ar);
                        render_voice_as::<false, false>(&mut b, rate, &mut bl, &mut br);
                        let what = format!("stereo {is_stereo}, sustain {sustain}, {start}..{end} from {from}, block {block}");
                        assert_eq!(al.map(f32::to_bits), bl.map(f32::to_bits), "{what}");
                        assert_eq!(ar.map(f32::to_bits), br.map(f32::to_bits), "{what}");
                        assert_eq!((a.active, a.pos), (b.active, b.pos), "{what}");
                        assert_eq!(
                            (a.last_l.to_bits(), a.last_r.to_bits()),
                            (b.last_l.to_bits(), b.last_r.to_bits()),
                            "{what}"
                        );
                        if !a.active {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// What isn't plain goes through the full loop: a filter, a fade, a
    /// loop, pitch, a blend between frames, a kill fade, a stolen voice's
    /// tail, an envelope still moving, a channel pick.
    #[test]
    fn only_plain_voices_take_the_plain_path() {
        let mut amp = Envelope::start(&Ahdsr::OPEN, 44_100.0);
        for _ in 0..4 {
            amp.next();
        }
        let plain = Voice {
            active: true,
            stereo: true,
            pick_l: 0,
            pick_r: 1,
            step: ONE,
            end: 100,
            amp: Some(amp),
            ..Voice::default()
        };
        assert!(plain_now(&plain));
        type Change = (&'static str, fn(&mut Voice));
        let not: [Change; 10] = [
            ("filter", |v| v.filter = Some(OPEN_FILTER)),
            ("fade", |v| {
                v.fade_out = Some(Fade {
                    frames: 10,
                    ..Fade::NONE
                })
            }),
            ("loop", |v| v.looping = true),
            ("pitch", |v| v.step = ONE + 1),
            ("blend", |v| v.pos = 1),
            ("kill", |v| v.kill_left = 3),
            ("tail", |v| v.tail_left = 3),
            ("attack", |v| {
                v.amp = Some(Envelope::start(
                    &Ahdsr {
                        attack_ms: 5.0,
                        ..Ahdsr::OPEN
                    },
                    44_100.0,
                ))
            }),
            ("release", |v| v.amp.as_mut().unwrap().release()),
            ("left only", |v| v.pick_r = 0),
        ];
        for (what, change) in not {
            let mut v = plain.clone();
            change(&mut v);
            assert!(!plain_now(&v), "{what}");
        }
    }

    #[test]
    fn the_fraction_is_the_low_half_over_two_to_the_32() {
        for pos in [
            0i64,
            1,
            (7 << 32) | 0x8000_0000,
            -1,
            (3 << 32) | 0xFFFF_FFFF,
            i64::MIN + 12345,
        ] {
            assert_eq!(frac(pos), (pos & (ONE - 1)) as f32 / ONE as f32, "{pos:#x}");
        }
    }
}
