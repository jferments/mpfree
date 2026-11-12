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
//! A region in [`PlayMode::Granular`] plays through the voice's own
//! [`GrainVoice`] (from `mpfree-grain`) instead of the Hermite read; the
//! envelope, gain, pan, filter and fades after it are the same. One grain
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
//! in. [`Event::ProgramLive`] moves every pad at once by an offset on its
//! own level, tune or cutoff. New voices start with all of them; playing
//! voices follow the level, pan, cutoff and resonance and the program's
//! offsets, gliding there in steps of 16 frames so nothing clicks. Both go
//! through the event queue, so they land on their frame, and split
//! ([`Engine::split`]) they reach both halves of the voices.
//! [`Engine::stop_all`] is the hard stop: every voice and grain idle at
//! once and the events waiting dropped.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_grain::{Budget, Clip, GrainFx, GrainVoice};
pub use mpfree_grain::{GrainView, MAX_GRAINS};
/// The lane types [`Engine::split`] takes and reports, so callers need no
/// dependency of their own on `mpfree-lanes`.
pub use mpfree_lanes::{Config as LaneConfig, Setup as LaneSetup, Stats as LaneStats};
use mpfree_lanes::{Lane, Setup};

use crate::dsp::{
    db_to_gain, equal_power, hermite, pan_gains, Ahdsr, Envelope, Filter, FilterKind, Rng, Svf,
};
use crate::model::{
    ChannelPick, Fade, GrainParams, Keys, LayerSelect, Levels, LoopMode, MasterFx, PadsPlay,
    PlayMode, PressureRoute, PressureTarget, Project, WarpMethod, PADS,
};
use crate::warp::{CopyMap, Warps};

/// Fade length when a voice is choked, muted by its group, or pushed out by
/// the pad's polyphony limit.
pub const KILL_FRAMES: u16 = 64;
/// Fade length of the previous sound when its voice is stolen for a new hit.
pub const STEAL_FRAMES: u16 = 32;
/// Events that can wait for the next render.
pub const MAX_EVENTS: usize = 256;
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
            Target::Own(pad) => v.pad == pad && v.note == NO_NOTE,
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

/// Where [`Engine::render_routed`] puts each pad's sound: one stereo pair
/// per pad, such as a mixer's pad channels.
pub trait PadOut {
    /// Pad `pad`'s pair for this block, both sides at least the block
    /// long. The engine adds the pad's voices into the start and never
    /// clears it. It may ask for the same pad more than once in a block.
    /// `None` leaves the pad unheard: its voices still play on (into a
    /// scratch buffer) and end on time.
    fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])>;
}

/// Every pad into one pair: what [`Engine::render`] renders into.
struct Mixed<'a> {
    left: &'a mut [f32],
    right: &'a mut [f32],
}

impl PadOut for Mixed<'_> {
    fn pad(&mut self, _pad: usize) -> Option<(&mut [f32], &mut [f32])> {
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
    data: Arc<[f32]>,
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
    /// The tempo the layer's audio is at, when it is warped (0 if not):
    /// the voice plays it `tempo / warp_bpm` times as fast.
    warp_bpm: f64,
    /// It plays a stretched copy, not the source.
    copy: bool,
    /// For a stretched copy: where each source frame lands in it.
    source_at: Option<CopyMap>,
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
    amp: Ahdsr,
    filter: Filter,
    filter_env: Ahdsr,
    vel_to_level: f32,
    vel_to_start: f32,
    pressure: PressureRoute,
    keys: Option<Keys>,
    wow: f32,
    /// The pad's own level, pan and tune, for live settings to replace.
    level_db: f32,
    pan: f32,
    tune_cents: i32,
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
                        let mut grain = r.grain;
                        grain.pitch += (cents / 100.0) as f32;
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
                            warp_bpm: 0.0,
                            copy: false,
                            source_at: None,
                        };
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
                                v.data = data;
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
                    amp: p.amp,
                    filter: p.filter,
                    filter_env: p.filter_env,
                    vel_to_level: p.vel_to_level.clamp(0.0, 1.0),
                    vel_to_start: p.vel_to_start.clamp(0.0, 1.0),
                    pressure: p.pressure,
                    keys: p.keys,
                    wow: p.wow.clamp(0.0, 1.0),
                    level_db: p.level_db,
                    pan: p.pan,
                    tune_cents: p.tune_cents,
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
    /// Moves every pad at once by an offset on its own setting (or on its
    /// live one): see [`ProgramField`]. 0 or NaN is no offset. New voices
    /// start with it and playing voices follow every field. It stays when
    /// a new view comes in.
    ProgramLive {
        field: ProgramField,
        value: f32,
    },
}

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
/// applies), and the program's offsets.
#[derive(Clone, Copy, Debug)]
struct Live {
    pads: [[f32; PadField::COUNT]; PADS],
    program: [f32; ProgramField::COUNT],
}

impl Live {
    const OWN: [[f32; PadField::COUNT]; PADS] = [[f32::NAN; PadField::COUNT]; PADS];
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
    pad: u8,
    layer: u8,
    data: Option<Arc<[f32]>>,
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
    /// Plays through the engine's grain voice in the same slot.
    granular: bool,
    src_rate: u32,
    /// Grain settings before pressure moves them.
    grain_base: GrainParams,
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
}

impl Voice {
    #[inline]
    fn at(&self, data: &[f32], i: i64, ch: u8) -> f32 {
        let i = i.clamp(self.start, self.end - 1) as usize;
        if self.stereo {
            data[2 * i + usize::from(ch)]
        } else {
            data[i]
        }
    }

    /// One channel at a fixed-point position, Hermite-interpolated, with
    /// every neighbour clamped to the region: the edge case of [`Voice::read2`].
    #[cold]
    #[inline(never)]
    fn read(&self, data: &[f32], pos: i64, pick: u8) -> f32 {
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
    fn read2(&self, data: &[f32], pos: i64) -> (f32, f32) {
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
            let w: &[f32; 4] = data[k..k + 4].try_into().expect("four frames");
            let x = if t == 0.0 {
                w[1]
            } else {
                hermite(w[0], w[1], w[2], w[3], t)
            };
            return (x, x);
        }
        let w: &[f32; 8] = data[2 * k..2 * k + 8].try_into().expect("four frames");
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

impl Pool {
    /// About what this share costs to render now: its playing voices and
    /// their live grains. A new voice goes to the share that costs less, so
    /// two granular clouds don't pile up on one core.
    fn cost(&self) -> usize {
        VOICE_COST * self.voices.iter().filter(|v| v.active).count() + self.budget.live()
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
            match out.pad(usize::from(v.pad)) {
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
            if let Some((l, r)) = out.pad(usize::from(v.pad)) {
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
}

pub struct Engine {
    rate: f32,
    view: EngineView,
    /// The voices, with their grain voices, grain budgets and buffers.
    pool: Voices,
    /// Split: the second share of the voices renders here, on another core.
    lane: Option<Lane<Carrier>>,
    events: Vec<(u32, Event)>,
    cycle: Vec<u8>,
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
    /// Live settings ([`Event::PadLive`], [`Event::ProgramLive`]).
    live: Live,
    /// The one-pole step playing voices follow them with, per
    /// [`FILTER_STEP`] frames.
    live_k: f32,
    /// Split: [`Engine::stop_all`] came while the lane's share was away;
    /// it stops when it comes back.
    stop_lane: bool,
}

/// Wow and flutter at full depth: cents and cycles a second of each
/// wobble (two slow ones for the wow, one fast for the flutter).
const WOW: [(f32, f32); 3] = [(15.0, 0.53), (10.0, 0.29), (5.0, 7.3)];
/// Below this speed the tape fades the sound out, reaching silence at a halt.
const TAPE_FADE: f32 = 0.25;

impl Engine {
    /// `voices` is the size of the fixed pool. All memory is reserved here.
    pub fn new(rate: u32, voices: usize) -> Engine {
        Engine {
            rate: rate as f32,
            view: EngineView::empty(),
            pool: Voices {
                a: Pool::new(rate, 0, voices.max(1), false),
                b: None,
                b_limit: 0,
                split: false,
            },
            lane: None,
            events: Vec::with_capacity(MAX_EVENTS),
            cycle: vec![0; PADS],
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
            live: Live {
                pads: Live::OWN,
                program: [0.0; ProgramField::COUNT],
            },
            live_k: 1.0 - (-(FILTER_STEP as f32) / (LIVE_SMOOTH_S * rate as f32)).exp(),
            stop_lane: false,
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
        for (v, _) in self.pool.iter_mut() {
            if v.active && v.warp_bpm > 0.0 {
                v.base_step = warped_step(v.plain_step, bpm, v.warp_bpm);
                retime(v);
            }
        }
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
        if let Some(fx) = &mut self.master {
            fx.set_params(self.view.master.grain);
            fx.set_mix(self.view.master.mix);
        }
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
    /// their audio and finish normally. The master effect takes the new
    /// view's settings (a new mix glides there over 10 ms). The pads' live
    /// settings ([`Event::PadLive`]) go back to the pads' own; the
    /// program's offsets stay.
    pub fn set_view(&mut self, view: EngineView) -> EngineView {
        self.live.pads = Live::OWN;
        let old = std::mem::replace(&mut self.view, view);
        self.apply_master();
        old
    }
    pub fn view_revision(&self) -> u64 {
        self.view.revision
    }

    /// Full Level (Some(127)), Half Level (Some(64)), or off (None).
    pub fn set_fixed_velocity(&mut self, v: Option<u8>) {
        self.fixed_velocity = v.map(|v| v.clamp(1, 127));
    }

    pub fn stats(&self) -> Stats {
        Stats {
            skipped_grains: self.pool.a.budget.skipped
                + self.pool.b.as_ref().map_or(0, |b| b.budget.skipped),
            ..self.stats
        }
    }
    pub fn active_voices(&self) -> usize {
        self.pool.iter().filter(|(v, _)| v.active).count()
    }
    /// (pad, layer, source frame) of every playing voice, for playheads on screen.
    /// A granular voice reports its reading position (before spray); a
    /// voice playing a stretched copy, the source frame that part of the
    /// copy was made from.
    pub fn playheads(&self) -> impl Iterator<Item = (u8, u8, u64)> + '_ {
        self.pool.iter().filter(|(v, _)| v.active).map(|(v, g)| {
            let at = if v.granular {
                v.start + (f64::from(g.playhead()) * (v.end - v.start) as f64) as i64
            } else {
                let pos = v.pos >> 32;
                match self.copy_map(v) {
                    Some(map) => map.source(pos as f64).round() as i64,
                    None => pos,
                }
            };
            (v.pad, v.layer, at.max(0) as u64)
        })
    }

    /// The map of the stretched copy voice `v` plays, if it plays one.
    fn copy_map(&self, v: &Voice) -> Option<&CopyMap> {
        let l = self
            .view
            .pads
            .get(usize::from(v.pad))?
            .as_ref()?
            .layers
            .get(usize::from(v.layer))?;
        let same = v.data.as_ref().is_some_and(|d| Arc::ptr_eq(d, &l.data));
        l.source_at.as_ref().filter(|_| same)
    }
    pub fn pad_voices(&self, pad: u8) -> usize {
        self.pool
            .iter()
            .filter(|(v, _)| v.active && v.pad == pad && v.kill_left == 0)
            .count()
    }

    /// The grains layer `layer` of pad `pad` plays now, across every voice
    /// playing it: each one's reading position (0 to 1 across the region),
    /// window level and direction, for drawing the cloud. Fills `out` from
    /// the start and returns how many it holds (grains past its end are
    /// left out). Never allocates.
    pub fn grains_of(&self, pad: u8, layer: u8, out: &mut [GrainView]) -> usize {
        let mut n = 0;
        for (v, g) in self.pool.iter() {
            if v.active && v.granular && v.pad == pad && v.layer == layer && n < out.len() {
                n += g.grains(&mut out[n..]);
            }
        }
        n
    }

    /// Queues an event `offset` frames into the next render (later renders if
    /// the offset is past its end). Events at the same offset keep their order.
    /// Returns false, and counts it, if the queue is full.
    pub fn schedule(&mut self, offset: u32, ev: Event) -> bool {
        if self.events.len() >= MAX_EVENTS {
            self.stats.dropped_events += 1;
            return false;
        }
        let i = self.events.partition_point(|(o, _)| *o <= offset);
        self.events.insert(i, (offset, ev));
        true
    }

    /// Applies an event now (before the next rendered frame).
    pub fn apply(&mut self, ev: Event) {
        match ev {
            // With the pads playing notes, a pad is the note it plays;
            // with 16 Levels, the one pad at its level.
            Event::NoteOn { pad, velocity } => match self.target(pad) {
                Target::Own(_) => self.note_on(pad, velocity, NO_NOTE, Vary::default()),
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
                    if v.active && t.played(v) && v.kill_left == 0 {
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
            Event::ProgramLive { field, value } => self.program_live(field, value),
        }
    }

    /// [`Event::PadLive`]: the table, then the playing voices of the pad.
    fn pad_live(&mut self, pad: u8, field: PadField, value: f32) {
        let Some(row) = self.live.pads.get_mut(usize::from(pad)) else {
            return;
        };
        row[field as usize] = field.clamp(value);
        if field.follows() {
            let (live, program) = (*row, self.live.program);
            for (v, _) in self.pool.iter_mut() {
                if v.active && v.pad == pad {
                    retarget(v, &live, &program);
                }
            }
        }
    }

    /// [`Event::ProgramLive`]: the offset, then every playing voice.
    fn program_live(&mut self, field: ProgramField, value: f32) {
        self.live.program[field as usize] = field.clamp(value);
        let program = self.live.program;
        let pads = &self.live.pads;
        for (v, _) in self.pool.iter_mut() {
            if let Some(live) = pads.get(usize::from(v.pad)).filter(|_| v.active) {
                retarget(v, live, &program);
            }
        }
    }

    /// Pad `pad`'s live `field`, or `None` where the pad's own applies.
    pub fn pad_live_value(&self, pad: u8, field: PadField) -> Option<f32> {
        let x = self.live.pads.get(usize::from(pad))?[field as usize];
        (!x.is_nan()).then_some(x)
    }

    /// The program's offset on `field` (0 when none).
    pub fn program_live_value(&self, field: ProgramField) -> f32 {
        self.live.program[field as usize]
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

    /// What a hit on pad `pad` plays.
    fn target(&self, pad: u8) -> Target {
        match self.view.pads_play {
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
        for p in 0..self.view.pads.len() {
            let Some(k) = self.view.pads[p].as_ref().and_then(|vp| vp.keys) else {
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

    /// Lets go of the held voices `which` picks.
    fn release(&mut self, which: impl Fn(&Voice) -> bool) {
        for (v, g) in self.pool.iter_mut() {
            if !v.active || !which(v) || !v.held {
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
        if velocity == 0 || start >= end {
            return;
        }
        let velocity = self.fixed_velocity.unwrap_or(velocity.min(127));
        let p = usize::from(pad);
        let Some(Some(vp)) = self.view.pads.get(p) else {
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
            if v.active && v.pad == pad {
                Self::kill(v);
            }
        }
        let vary = Vary {
            slice: Some((i64::from(start), i64::from(end))),
            ..Vary::default()
        };
        self.start_voice(p, li, level, vel, NO_NOTE, vary);
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
        let Some(Some(vp)) = self.view.pads.get(p) else {
            return;
        };
        self.stats.hits += 1;

        // A second hit on a held toggle pad stops it instead of starting it.
        let mut toggled_off = false;
        for (v, _) in self.pool.iter_mut() {
            if v.active
                && v.pad == pad
                && v.note == note
                && v.held
                && v.mode == Some(PlayMode::Toggle)
            {
                v.held = false;
                if let Some(a) = &mut v.amp {
                    a.release();
                }
                toggled_off = true;
            }
        }
        if toggled_off {
            return;
        }

        // Mute groups and one-way chokes.
        let group = vp.mute_group;
        for (v, _) in self.pool.iter_mut() {
            if !v.active || v.pad == pad {
                continue;
            }
            let other = usize::from(v.pad);
            let same_group = group.is_some()
                && self.view.pads[other]
                    .as_ref()
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
        match vp.select {
            LayerSelect::All => {
                for (i, c) in chosen.iter_mut().enumerate().take(count) {
                    *c = matching(i);
                }
            }
            LayerSelect::Cycle | LayerSelect::Random => {
                let k = if vp.select == LayerSelect::Cycle {
                    let k = usize::from(self.cycle[p]) % n_match;
                    self.cycle[p] = ((k + 1) % n_match) as u8;
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
        let polyphony = usize::from(vp.polyphony);
        for li in 0..count {
            if !chosen[li] {
                continue;
            }
            // Polyphony: push out this pad's oldest voices past the limit.
            loop {
                let mut n = 0;
                let mut oldest: Option<(usize, u64)> = None;
                for (i, (v, _)) in self.pool.iter().enumerate() {
                    if v.active && v.pad == pad && v.kill_left == 0 {
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
            self.start_voice(p, li, level, vel, note, vary);
        }
    }

    /// Starts layer `li` of pad `p` with what `vary` changes. A slice, for
    /// an audition, plays those source frames instead of the layer's
    /// region: once, forward, without fades or a loop.
    fn start_voice(&mut self, p: usize, li: usize, level: f32, vel: f32, note: u8, vary: Vary) {
        let semis = vary.semis;
        let Some(vp) = self.view.pads[p].as_ref() else {
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
        // A free voice, else the oldest releasing one, else the oldest.
        let slot = self.pool.free_slot().unwrap_or_else(|| {
            let rel =
                |x: &Voice| x.kill_left > 0 || x.amp.as_ref().is_some_and(|a| a.is_releasing());
            let mut best: Option<(usize, (bool, std::cmp::Reverse<u64>))> = None;
            for (i, (v, _)) in self.pool.iter().enumerate() {
                let key = (rel(v), std::cmp::Reverse(v.age));
                if best.is_none_or(|(_, b)| key > b) {
                    best = Some((i, key));
                }
            }
            best.map_or(0, |(i, _)| i)
        });
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
        // The pad's live settings, where set, in place of its own.
        let live = self.live.pads[p];
        let program = self.live.program;
        let set = |f: PadField| Some(live[f as usize]).filter(|x| !x.is_nan());
        let mut amp = vp.amp;
        if let Some(ms) = set(PadField::Attack) {
            amp.attack_ms = ms;
        }
        if let Some(ms) = set(PadField::Decay) {
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
        let mut grain = l.grain;
        grain.pitch += semis as f32;
        if tune != 0.0 {
            grain.pitch += tune / 100.0;
        }
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
        self.clock += 1;
        let Some(v) = self.pool.voice_mut(slot) else {
            return;
        };
        *v = Voice {
            active: true,
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
            granular,
            src_rate: l.src_rate,
            grain_base: grain,
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
        if let Some(g) = self.pool.grain_mut(slot).filter(|_| granular) {
            g.set_params(tuned_grain(grain, cents));
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
                while let Some(&(o, ev)) = self.events.first() {
                    if o > at {
                        break;
                    }
                    self.events.remove(0);
                    self.apply(ev);
                }
                next = self.events.first().map_or(n, |&(o, _)| o.min(n));
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
                if let (Some(fx), Some((l, r))) = (&mut self.master, out.pad(0)) {
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
    let Some(data) = v.data.clone() else {
        v.active = false;
        return;
    };
    let Some(clip) = Clip::new(&data, if v.stereo { 2 } else { 1 }, v.src_rate) else {
        v.active = false;
        return;
    };
    let clip = clip.with_range(v.start.max(0) as u64, v.end.max(0) as u64);
    let mut at = 0;
    while at < left.len() && v.active {
        let n = (left.len() - at).min(sl.len());
        let (bl, br) = (&mut sl[..n], &mut sr[..n]);
        bl.fill(0.0);
        br.fill(0.0);
        if v.route.target != PressureTarget::Off || v.grain_cents != v.follow.cents {
            let cents = v.follow.cents;
            g.set_params(tuned_grain(
                pressed_grain(v.grain_base, v.route, v.press),
                cents,
            ));
            v.grain_cents = cents;
        }
        g.render(&clip, budget, frame + at as u64, bl, br);
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
    if !v.active {
        v.data = None;
    }
}

/// Grain settings `cents` higher (the program's tune).
fn tuned_grain(mut g: GrainParams, cents: f32) -> GrainParams {
    if cents != 0.0 {
        g.pitch += cents / 100.0;
    }
    g
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
    v.follow.moving = moving;
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
#[inline(never)]
fn retune(v: &mut Voice, f: Filter, e: f32, rate: f32) {
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
    if v.follow.moving {
        render_voice_as::<true>(v, rate, left, right);
    } else {
        render_voice_as::<false>(v, rate, left, right);
    }
}

fn render_voice_as<const FOLLOW: bool>(
    v: &mut Voice,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
) {
    let Some(data) = v.data.clone() else {
        v.active = false;
        return;
    };
    for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
        let i = v.pos >> 32;
        if i < v.start || i >= v.end {
            v.active = false;
            break;
        }

        let (mut l, mut r) = v.read2(&data, v.pos);

        // Forward-loop crossfade: blend in the audio before the loop start.
        if v.looping && !v.ping_pong && v.crossfade > 0 && i >= v.loop_end - v.crossfade {
            let x = (i - (v.loop_end - v.crossfade)) as f32 / v.crossfade as f32;
            let (out, inn) = equal_power(x);
            let back = v.pos - ((v.loop_end - v.loop_start) << 32);
            let (bl, br) = v.read2(&data, back);
            l = l * out + bl * inn;
            r = r * out + br * inn;
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
            break;
        }
    }
    if !v.active {
        v.data = None;
    }
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
