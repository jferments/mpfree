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

use std::sync::Arc;

use mpfree_grain::{Budget, Clip, GrainFx, GrainVoice};

use crate::dsp::{
    db_to_gain, equal_power, hermite, pan_gains, Ahdsr, Envelope, Filter, FilterKind, Rng, Svf,
};
use crate::model::{
    ChannelPick, Fade, GrainParams, Keys, LayerSelect, Levels, LoopMode, MasterFx, PadsPlay,
    PlayMode, PressureRoute, PressureTarget, Project, WarpMethod, PADS,
};
use crate::warp::Warps;

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
/// Filter coefficients, and anything pressure moves, are recomputed every
/// this many frames.
const FILTER_STEP: u32 = 16;
/// Pressure is smoothed with this time constant, so steps between pressure
/// messages don't zipper.
const PRESSURE_SMOOTH_S: f32 = 0.005;
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
                        let g = db_to_gain(r.gain_db) * db_to_gain(l.level_db) * pad_gain;
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
                            step: ((ratio * ONE as f64).round() as i64).max(1),
                            vel_lo: l.vel_lo,
                            vel_hi: l.vel_hi,
                            src_rate: s.rate(),
                            grain,
                            warp_bpm: 0.0,
                            copy: false,
                        };
                        let warp = r.warp.filter(|_| r.mode != PlayMode::Granular);
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
                                let at = |x: u64| ((x - r.start) as f64 * scale).round() as i64;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Counters for the debug page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub hits: u64,
    pub steals: u64,
    pub dropped_events: u64,
    pub peak_voices: usize,
    /// Grains refused because the grain budget was full.
    pub skipped_grains: u64,
}

pub struct Engine {
    rate: f32,
    view: EngineView,
    voices: Vec<Voice>,
    events: Vec<(u32, Event)>,
    cycle: Vec<u8>,
    rng: Rng,
    clock: u64,
    fixed_velocity: Option<u8>,
    stats: Stats,
    /// One grain voice per voice slot, used while that slot is granular.
    grains: Vec<GrainVoice>,
    budget: Budget,
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
    /// Where the voices of a pad with no output play, so they end on time.
    unheard_l: Vec<f32>,
    unheard_r: Vec<f32>,
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
            voices: vec![Voice::default(); voices.max(1)],
            events: Vec::with_capacity(MAX_EVENTS),
            cycle: vec![0; PADS],
            rng: Rng::new(0x5A4D_5052),
            clock: 0,
            fixed_velocity: None,
            stats: Stats::default(),
            grains: (0..voices.max(1))
                .map(|i| GrainVoice::new(rate, i as u64))
                .collect(),
            budget: Budget::new(GRAIN_BUDGET),
            scratch_l: vec![0.0; GRAIN_CHUNK],
            scratch_r: vec![0.0; GRAIN_CHUNK],
            unheard_l: vec![0.0; GRAIN_CHUNK],
            unheard_r: vec![0.0; GRAIN_CHUNK],
            frames: 0,
            master: None,
            tempo: 120.0,
            tape: 1.0,
            tape_target: 1.0,
            tape_rate: 0.0,
            tape_moved: false,
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
        for v in &mut self.voices {
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
            o.kill(&mut self.budget);
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
        self.budget.live()
    }

    /// Swaps in a new view at once and hands back the old one, so the caller
    /// frees it off the audio thread. Playing voices keep their own handle to
    /// their audio and finish normally. The master effect takes the new
    /// view's settings (a new mix glides there over 10 ms).
    pub fn set_view(&mut self, view: EngineView) -> EngineView {
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
            skipped_grains: self.budget.skipped,
            ..self.stats
        }
    }
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
    /// (pad, layer, source frame) of every playing voice, for playheads on screen.
    /// A granular voice reports its reading position (before spray).
    pub fn playheads(&self) -> impl Iterator<Item = (u8, u8, u64)> + '_ {
        self.voices
            .iter()
            .zip(&self.grains)
            .filter(|(v, _)| v.active)
            .map(|(v, g)| {
                let at = if v.granular {
                    v.start + (f64::from(g.playhead()) * (v.end - v.start) as f64) as i64
                } else {
                    v.pos >> 32
                };
                (v.pad, v.layer, at.max(0) as u64)
            })
    }
    pub fn pad_voices(&self, pad: u8) -> usize {
        self.voices
            .iter()
            .filter(|v| v.active && v.pad == pad && v.kill_left == 0)
            .count()
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
                for v in &mut self.voices {
                    if v.active && t.played(v) && v.kill_left == 0 {
                        v.press_target = f32::from(value.min(127)) / 127.0;
                    }
                }
            }
            Event::AllOff => {
                for (v, g) in self.voices.iter_mut().zip(&mut self.grains) {
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
        for v in &mut self.voices {
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
        for (v, g) in self.voices.iter_mut().zip(&mut self.grains) {
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
        for v in &mut self.voices {
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
        for v in &mut self.voices {
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
                let mut oldest: Option<usize> = None;
                for (i, v) in self.voices.iter().enumerate() {
                    if v.active && v.pad == pad && v.kill_left == 0 {
                        n += 1;
                        if oldest.is_none_or(|o| self.voices[o].age > v.age) {
                            oldest = Some(i);
                        }
                    }
                }
                if n < polyphony {
                    break;
                }
                match oldest {
                    Some(o) => Self::kill(&mut self.voices[o]),
                    None => break,
                }
            }
            self.start_voice(p, li, level, vel, note, vary);
        }
    }

    fn start_voice(&mut self, p: usize, li: usize, level: f32, vel: f32, note: u8, vary: Vary) {
        let semis = vary.semis;
        let Some(vp) = self.view.pads[p].as_ref() else {
            return;
        };
        let l = &vp.layers[li];
        // A free voice, else the oldest releasing one, else the oldest.
        let slot = self
            .voices
            .iter()
            .position(|v| !v.active)
            .unwrap_or_else(|| {
                let mut best = 0;
                for (i, v) in self.voices.iter().enumerate() {
                    let b = &self.voices[best];
                    let rel = |x: &Voice| {
                        x.kill_left > 0 || x.amp.as_ref().is_some_and(|a| a.is_releasing())
                    };
                    if (rel(v), std::cmp::Reverse(v.age)) > (rel(b), std::cmp::Reverse(b.age)) {
                        best = i;
                    }
                }
                best
            });
        let stolen = self.voices[slot].active;
        if self.voices[slot].granular {
            self.grains[slot].kill(&mut self.budget);
        }
        let granular = l.mode == PlayMode::Granular;
        let (tail_l, tail_r) = if stolen {
            self.stats.steals += 1;
            (self.voices[slot].last_l, self.voices[slot].last_r)
        } else {
            (0.0, 0.0)
        };
        let frames = l.end - l.start;
        let skip = if l.reverse {
            0
        } else {
            let from = vary.start.unwrap_or(vp.vel_to_start * (1.0 - vel));
            (from as f64 * frames as f64) as i64
        };
        let start_frame = if l.reverse {
            l.end - 1
        } else {
            (l.start + skip).min(l.end - 1)
        };
        let (pick_l, pick_r) = picks(l.stereo, l.pick);
        let lp = l.sustain_loop.as_ref();
        let mut amp = vp.amp;
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
        let mut grain = l.grain;
        grain.pitch += semis as f32;
        if let Some(at) = vary.start {
            grain.position = (at * 16.0 / 15.0).min(1.0);
        }
        // A keygroup a few semitones from its root reads that much faster.
        let plain = if semis == 0 {
            l.step
        } else {
            ((l.step as f64 * (f64::from(semis) / 12.0).exp2()).round() as i64).max(1)
        };
        let step = if l.warp_bpm > 0.0 {
            warped_step(plain, self.tempo, l.warp_bpm)
        } else {
            plain
        };
        self.clock += 1;
        let v = &mut self.voices[slot];
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
            backwards: l.reverse,
            reverse: l.reverse,
            start: l.start,
            end: l.end,
            loop_start: lp.map_or(0, |x| x.start),
            loop_end: lp.map_or(0, |x| x.end),
            crossfade: lp.map_or(0, |x| x.crossfade),
            ping_pong: lp.is_some_and(|x| x.ping_pong),
            looping: !granular
                && lp.is_some()
                && !l.reverse
                && matches!(l.mode, PlayMode::Loop | PlayMode::Toggle | PlayMode::Gate),
            fade_in: (l.fade_in.frames > 0).then_some(l.fade_in),
            fade_out: (l.fade_out.frames > 0).then_some(l.fade_out),
            gain_l: l.gain_l * level,
            gain_r: l.gain_r * level,
            mode: Some(l.mode),
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
            base_gain_l: l.gain_l * level,
            base_gain_r: l.gain_r * level,
            wow: vp.wow,
            // Each voice wobbles from its own place, so a chord doesn't
            // wander in step.
            wow_phase: [
                (self.clock as f32 * 0.618_034).fract(),
                (self.clock as f32 * 0.414_214).fract(),
                (self.clock as f32 * 0.732_051).fract(),
            ],
        };
        if self.tape != 1.0 {
            // A hit on a stopped (or slowing) tape plays at its speed.
            v.speed = self.tape;
            let gain = (self.tape / TAPE_FADE).min(1.0);
            (v.gain_l, v.gain_r) = (v.base_gain_l * gain, v.base_gain_r * gain);
            retime(v);
        }
        if granular {
            let g = &mut self.grains[slot];
            g.set_params(grain);
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
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        assert_eq!(
            left.len(),
            right.len(),
            "left and right must be the same length"
        );
        let n = left.len();
        left.fill(0.0);
        right.fill(0.0);
        self.run(n, &mut Mixed { left, right }, true);
    }

    /// Renders the next `frames` frames with each pad apart: every voice
    /// adds its sound into its pad's pair from `out` (see [`PadOut`]).
    /// Events, voices and grains run exactly as in [`Engine::render`], and
    /// the pads' pairs add up to what it renders. The master effect, which
    /// works on the mixed pair, doesn't run. Panics if `out` hands back a
    /// pair shorter than `frames` (a programming error).
    pub fn render_routed(&mut self, frames: usize, out: &mut impl PadOut) {
        self.run(frames, out, false);
    }

    /// The render loop: events on their frames, then every voice into its
    /// pad's pair, then (`master`) the master effect over pad 0's pair,
    /// which is the mixed pair when `out` is [`Mixed`].
    fn run(&mut self, frames: usize, out: &mut impl PadOut, master: bool) {
        let n = frames as u32;
        let mut at = 0u32;
        while at < n {
            // Apply every event due now.
            while let Some(&(o, ev)) = self.events.first() {
                if o > at {
                    break;
                }
                self.events.remove(0);
                self.apply(ev);
            }
            let next = self.events.first().map_or(n, |&(o, _)| o.min(n));
            let (a, b) = (at as usize, next as usize);
            let frame = self.frames + at as u64;
            self.follow_tape(b - a);
            for (v, g) in self.voices.iter_mut().zip(&mut self.grains) {
                if !v.active {
                    continue;
                }
                match out.pad(usize::from(v.pad)) {
                    Some((l, r)) => play(
                        v,
                        g,
                        &mut self.budget,
                        frame,
                        self.rate,
                        &mut l[a..b],
                        &mut r[a..b],
                        &mut self.scratch_l,
                        &mut self.scratch_r,
                    ),
                    None => {
                        let mut k = a;
                        while k < b && v.active {
                            let m = (b - k).min(self.unheard_l.len());
                            let (ul, ur) = (&mut self.unheard_l[..m], &mut self.unheard_r[..m]);
                            ul.fill(0.0);
                            ur.fill(0.0);
                            play(
                                v,
                                g,
                                &mut self.budget,
                                frame + (k - a) as u64,
                                self.rate,
                                ul,
                                ur,
                                &mut self.scratch_l,
                                &mut self.scratch_r,
                            );
                            k += m;
                        }
                    }
                }
            }
            if master {
                if let (Some(fx), Some((l, r))) = (&mut self.master, out.pad(0)) {
                    fx.process(&mut self.budget, frame, &mut l[a..b], &mut r[a..b]);
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
        if v.route.target != PressureTarget::Off {
            g.set_params(pressed_grain(v.grain_base, v.route, v.press));
        }
        g.render(&clip, budget, frame + at as u64, bl, br);
        for k in 0..n {
            let (l, r) = finish(v, rate, bl[k], br[k], 1.0);
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

/// The voice's step from its base step, its pitch pressure and its speed
/// (the tape's, with wow and flutter). A granular voice's grains keep their
/// own pitch.
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
fn finish(v: &mut Voice, rate: f32, mut l: f32, mut r: f32, mut g: f32) -> (f32, f32) {
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

fn render_voice(v: &mut Voice, rate: f32, left: &mut [f32], right: &mut [f32]) {
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
        let (l, r) = finish(v, rate, l, r, g);
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
