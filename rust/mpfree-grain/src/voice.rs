//! The grain voice: schedules grains and mixes them.

use crate::buffer::{Aim, GrainSource};
use crate::motion::{env_level, moved, GrainEnv, GrainLfo, LfoState, MotionTarget};
use crate::rng::Rng;
use crate::window::{Window, WindowTable};

/// Grains one voice can play at once.
pub const MAX_GRAINS: usize = 64;
/// Frame positions below this (2^31 less a margin for rounding) take the
/// `i32` path in [`mix`].
const SMALL_FRAMES: f64 = 2_147_000_000.0;
/// A voice renders in parts of at most this many frames, through its own
/// scratch buffers (see [`GrainVoice::render`], "Level").
const PART: usize = 64;
/// How fast the voice follows how its grains add up (one-pole, seconds).
const COHERENCE_SECS: f64 = 0.05;
/// How fast the floor under the level's divisor rises (seconds). It falls
/// over a quarter of a grain, so a cloud's first grains, and grains cut
/// short by a live buffer that has only just started, keep their windows.
const FLOOR_SECS: f64 = 0.01;
/// The floor in a dense cloud: below it, a lone grain's window shows again.
const FLOOR_DENSE: f32 = 0.3;
/// The smoothed window power a voice needs before it judges coherence.
const COVER: f32 = 0.25;
/// Coherence is measured in one part of every this many (parts are
/// counted on the voice's own clock), which keeps most grain frames to
/// the four sums the level needs.
const MEASURE_EVERY: u64 = 4;
/// Grains one voice aims to keep alive at most: when a cloud wants more
/// (a long size at a high density) the gaps widen to fit, so it thins
/// evenly instead of skipping grains at random. Under [`MAX_GRAINS`] by
/// enough that random timing's shorter gaps rarely reach the cap.
const CAP_FIT: f64 = 54.0;
/// A cloud starts with about this many grains under way (or fewer, as its
/// density sets): enough to sound at its level at once, without the cost
/// of the whole cloud for a short hit.
const START_GRAINS: f64 = 8.0;
/// A cloud that starts already running fades in over this long, so its
/// first frame doesn't click (seconds).
const START_FADE_SECS: f64 = 0.004;
/// Mixed into the seed of the timing's own random sequence, so timing
/// draws never shift the other parameters' draws (and timing 0 plays
/// exactly as before it existed).
const TIMING_SEED: u64 = 0x7A3E_D1CE_0F7E_57A3;
/// Mixed into the seed of the size and level variation's own random
/// sequence, for the same reason: with no variation a cloud plays exactly
/// as before variation existed.
const VARY_SEED: u64 = 0x5EED_0B1A_C0DE_2611;
/// Mixed into the seed of the LFO's own random sequence (its Hold and
/// Drift values), for the same reason.
const LFO_SEED: u64 = 0x1F0D_5EED_0A7E_B3B3;
/// A grain varied all the way down in level (`Params::level_spray` 1) is
/// this many decibels down.
const LEVEL_SPRAY_DB: f32 = 24.0;

/// A grain stopped at a hit shorter than this much of its size is not
/// made (see [`GrainVoice::render`], "Hits").
const MIN_CUT: f64 = 0.25;

/// How fast the grains playing fade as a hit starts.
const HIT_FADE_SECS: f64 = 0.003;

/// How [`Params::spray`] spreads grains around the position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SprayShape {
    /// Anywhere in the spray's range, as likely at its edges as at the
    /// position.
    #[default]
    Even,
    /// Most grains near the position and a few far out (the offset is the
    /// even draw cubed): a focused cloud with stray grains.
    Peaked,
}

/// Locks density and size to the tempo. Grains then start on exact frames
/// of the transport's timeline, so a synced cloud stays on the grid however
/// the audio is cut into periods.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoSync {
    /// Output frames per beat (60 × rate / BPM).
    pub frames_per_beat: f64,
    /// Grains per beat (4 = sixteenths). `None` keeps `Params::density`.
    pub grains_per_beat: Option<f32>,
    /// Grain length in beats. `None` keeps `Params::size_ms`.
    pub size_beats: Option<f32>,
}

/// Everything a performer can turn. Out-of-range values are clamped when
/// grains are made, so any value is safe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Where grains read, 0 to 1 across the source's span.
    pub position: f32,
    /// Random offset of each grain's start, 0 to 1 of the span.
    pub spray: f32,
    /// Grain length, 1 ms to 4 s (the design's range is 2 ms to 2 s).
    pub size_ms: f32,
    /// Grains per second, 0.1 to 1000.
    pub density: f32,
    /// Semitones, ±48.
    pub pitch: f32,
    /// Random pitch per grain, ± this many semitones (0 to 24).
    pub pitch_spray: f32,
    /// Allowed semitones as a 12-bit mask relative to the sample's own pitch
    /// (bit 0 is the original pitch). 0 turns quantising off.
    /// `0b1010_1011_0101` is a major scale.
    pub scale: u16,
    /// How fast the reading position moves through the source: 1 is the
    /// original speed, 0 frozen, negative backwards (−4 to 4).
    pub scan: f32,
    pub window: Window,
    /// Chance that a grain plays backwards, 0 to 1.
    pub reverse: f32,
    /// Random pan per grain, 0 (centre) to 1 (anywhere).
    pub spread: f32,
    /// Linear output gain.
    pub gain: f32,
    /// The tempo (BPM) the source was played at, for a stretch that keeps
    /// time: with a tempo set on the voice ([`GrainVoice::set_tempo`]) the
    /// scan moves the transport's tempo over this times as fast, so scan 1
    /// plays the source in time at any tempo. None: the scan is as set.
    pub scan_tempo: Option<f32>,
    pub sync: Option<TempoSync>,
    /// How the gaps between grains vary, 0 to 1: 0 regular (every gap one
    /// density period: grains in lockstep, which buzzes at the grain rate
    /// and makes a frozen cloud comb, its level rising and falling with
    /// the density; each grain starts at its exact time, between two
    /// frames if it falls there, so the rate is exact and a cloud of short
    /// grains can play a tone at the density), up to 1 random (each gap anywhere from none to two
    /// periods). The density on average is the same at any setting.
    /// Synced density keeps its grains on the beat grid and ignores it.
    pub timing: f32,
    /// How each grain's length varies, 0 to 1: at 1 a grain is anywhere
    /// from half to twice the size (2^(±1), even in octaves). The cloud
    /// keeps its density.
    pub size_spray: f32,
    /// How each grain's level varies, 0 to 1: at 1 a grain is anywhere from
    /// its full level to 24 dB down (even in decibels). The cloud as a
    /// whole keeps its level (see [`GrainVoice::render`], "Level"), so
    /// variation changes its texture, with some grains standing out.
    pub level_spray: f32,
    /// How the position spray spreads grains: evenly, or peaked at the
    /// position.
    pub spray_shape: SprayShape,
    /// The voice's LFO, moving one setting (see [`crate::motion`]).
    pub lfo: GrainLfo,
    /// The voice's movement envelope, moving one setting from the voice's
    /// start (see [`crate::motion`]).
    pub env: GrainEnv,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            position: 0.0,
            spray: 0.0,
            size_ms: 80.0,
            density: 25.0,
            pitch: 0.0,
            pitch_spray: 0.0,
            scale: 0,
            scan: 0.0,
            window: Window::Hann,
            reverse: 0.0,
            spread: 0.0,
            gain: 1.0,
            scan_tempo: None,
            sync: None,
            timing: 0.3,
            size_spray: 0.0,
            level_spray: 0.0,
            spray_shape: SprayShape::Even,
            lfo: GrainLfo::default(),
            env: GrainEnv::default(),
        }
    }
}

/// A cap on grains alive at once, shared by every voice the audio thread
/// renders, so the period's cost is bounded however many voices play.
/// Voices count their grains in as they start and out as they end; a voice
/// that is dropped while playing must be [`GrainVoice::kill`]ed first, or
/// its grains stay counted.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub limit: usize,
    live: usize,
    /// Grains refused by this budget since it was made.
    pub skipped: u64,
}

impl Budget {
    pub fn new(limit: usize) -> Self {
        Budget {
            limit,
            live: 0,
            skipped: 0,
        }
    }
    /// Grains alive now across the voices using this budget.
    pub fn live(&self) -> usize {
        self.live
    }
}

/// Counters a UI or test can read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub spawned: u64,
    /// Grains not made because the voice or the budget was full.
    pub skipped: u64,
    pub active: usize,
    pub peak_active: usize,
    /// True while the cloud wants more grains than a voice keeps
    /// ([`GrainVoice`] then spaces them out to fit, keeping the level).
    pub thinned: bool,
}

/// One grain, for drawing the cloud.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrainView {
    /// Reading position, 0 to 1 across the span.
    pub position: f32,
    /// The window's current value, 0 to 1.
    pub level: f32,
    pub reverse: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Grain {
    pos: f64,
    inc: f64,
    age: u32,
    len: u32,
    dphase: f32,
    /// Where the window was at its first frame: a grain due between two
    /// frames starts that part of a frame in (0 for most grains).
    phase0: f32,
    /// The grain's own level, before its pan.
    lvl: f32,
    /// Left and right pan gains.
    pl: f32,
    pr: f32,
    /// How the source reads it ([`GrainSource::aim`]).
    aim: Aim,
}

/// What a part's grains add up to, frame by frame, before the level is
/// set: the left and right sums, the sum of the grains' window levels, the
/// sum of their squares, and the sum of each grain's own power.
#[derive(Clone, Debug)]
struct Scratch {
    l: [f32; PART],
    r: [f32; PART],
    s1: [f32; PART],
    s2: [f32; PART],
    q: [f32; PART],
}

impl Scratch {
    fn new() -> Self {
        Scratch {
            l: [0.0; PART],
            r: [0.0; PART],
            s1: [0.0; PART],
            s2: [0.0; PART],
            q: [0.0; PART],
        }
    }

    fn clear(&mut self, m: usize, measure: bool) {
        self.l[..m].fill(0.0);
        self.r[..m].fill(0.0);
        self.s1[..m].fill(0.0);
        self.s2[..m].fill(0.0);
        if measure {
            self.q[..m].fill(0.0);
        }
    }
}

/// The next gap, in density periods, for `timing` (0 to 1) and a uniform
/// draw `u` in [0, 1): anywhere from `1 - timing` to `1 + timing`, so the
/// average stays one period.
fn gap_factor(timing: f32, u: f32) -> f64 {
    let t = if timing.is_finite() {
        timing.clamp(0.0, 1.0)
    } else {
        0.0
    };
    f64::from(1.0 + t * (2.0 * u - 1.0))
}

/// A grain's offset from the position, in −1 to 1 of the spray, for an
/// even draw `r` in −1 to 1.
fn spray_draw(shape: SprayShape, r: f32) -> f32 {
    match shape {
        SprayShape::Even => r,
        SprayShape::Peaked => r * r * r,
    }
}

/// Decibels as a gain.
fn db_gain(db: f32) -> f32 {
    (db * (std::f32::consts::LN_10 / 20.0)).exp()
}

/// The floor under the level's divisor for an overlap: 1 for a sparse
/// cloud (each grain keeps its window and its level), [`FLOOR_DENSE`] from
/// overlap 2 (the cloud is held at its source's level), a line between.
/// `rms` is the grains' RMS level ([`level_rms`]): grains varied in level
/// are quieter on average, so a dense cloud's floor comes down with them,
/// or it would hold a dense cloud of them under its source's level.
fn floor_for(overlap: f64, rms: f32) -> f32 {
    let o = overlap as f32;
    let dense = FLOOR_DENSE * rms;
    if o <= 1.0 {
        1.0
    } else if o >= 2.0 {
        dense
    } else {
        1.0 - (o - 1.0) * (1.0 - dense)
    }
}

/// The RMS of grain levels varied evenly in decibels from 0 to
/// `level_spray` × [`LEVEL_SPRAY_DB`] down: 1 with no variation, 0.42 at
/// full.
fn level_rms(level_spray: f32) -> f32 {
    let d = LEVEL_SPRAY_DB * level_spray.clamp(0.0, 1.0);
    if d.is_nan() || d < 1e-3 {
        return 1.0;
    }
    // The mean of 10^(-x/10) for x even in 0 to d.
    let k = d / 10.0 * std::f32::consts::LN_10;
    ((1.0 - (-k).exp()) / k).sqrt()
}

/// One-pole coefficient for a time constant.
fn coefficient(secs: f64, rate: u32) -> f32 {
    (1.0 - (-1.0 / (secs * f64::from(rate))).exp()) as f32
}

/// Zero for anything too small to matter, so the averages never decay
/// into denormals.
#[inline]
fn settle(x: f32) -> f32 {
    if x < 1e-20 {
        0.0
    } else {
        x
    }
}

/// One granular voice: a fixed array of grains, a scheduler and a seeded
/// generator. Make it off the audio thread; every method after that is
/// real-time safe.
#[derive(Clone, Debug)]
pub struct GrainVoice {
    out_rate: u32,
    /// The settings as set ([`GrainVoice::set_params`]).
    params: Params,
    /// The settings the grains are made with: as set, moved by the LFO and
    /// the envelope (see [`crate::motion`]).
    moved: Params,
    /// The LFO's place and random values, and its value and the
    /// envelope's level at the start of this part.
    lfo: LfoState,
    lfo_value: f32,
    env_value: f32,
    /// Voice clock frames: the voice's start, and when the LFO was last
    /// moved on.
    started: u64,
    motion_at: u64,
    window: WindowTable,
    grains: [Grain; MAX_GRAINS],
    active: usize,
    /// Grains fading out under a hit (see [`GrainVoice::render`], "Hits"),
    /// at the level they had: outside the level's sums, so the hit plays
    /// at its own.
    fading: [Grain; MAX_GRAINS],
    fading_n: usize,
    rng: Rng,
    /// The timing's own random sequence (see [`TIMING_SEED`]).
    trng: Rng,
    /// The size and level variation's own (see [`VARY_SEED`]).
    vrng: Rng,
    /// The gap to the next free-running grain, in density periods.
    gap: f64,
    /// Set by [`GrainVoice::start`]: the next grain comes with the ones a
    /// running cloud would have, part-way through their windows.
    prefill: bool,
    /// How long the grains can be (output frames): the size, or shorter
    /// when a grain at the cloud's pitch would read past the source's
    /// span. What the cap's fit and the running start go by.
    fit_size: f64,
    /// Frames left of the fade-in after a prefilled start, and its length.
    fade_left: u32,
    fade_len: u32,
    /// Output frames rendered.
    clock: u64,
    /// Voice clock frame of the next free-running grain.
    next_at: f64,
    last_spawn: f64,
    spawning: bool,
    /// How far scanning has moved the reading position, in source frames.
    scan_offset: f64,
    span: (i64, i64),
    stats: Stats,
    /// The last hit a grain was started on (see [`GrainVoice::render`],
    /// "Hits"): its frame in the source, and the voice clock's frame.
    last_hit: Option<(f64, u64)>,
    /// The transport's frames per beat, set by [`GrainVoice::set_tempo`]:
    /// a synced cloud follows it in place of its own `frames_per_beat`.
    beat: Option<f64>,
    sc: Scratch,
    /// Smoothed: the output's power, the grains' own powers summed, the
    /// window sum squared and the window powers summed.
    ey: f32,
    eq: f32,
    es1: f32,
    es2: f32,
    /// How far the grains add in step (1) rather than as unrelated
    /// sounds (0): judged from those averages every 8 frames
    /// (`coherence_to`), and glided to every frame, so the level never
    /// steps.
    coherence: f32,
    coherence_to: f32,
    /// The floor under the level's divisor, gliding to [`floor_for`] the
    /// overlap while grains are made and to 1 after.
    floor: f32,
    /// The averages' coefficient (per measured frame) and the coherence
    /// glide's (per frame).
    k_measure: f32,
    k_coherence: f32,
    k_floor: f32,
    /// The floor's falling coefficient and the grain size it was made for.
    k_floor_down: f32,
    k_floor_size: f64,
}

impl GrainVoice {
    pub fn new(out_rate: u32, seed: u64) -> Self {
        let params = Params::default();
        GrainVoice {
            out_rate: out_rate.max(1),
            params,
            moved: params,
            lfo: LfoState::new(Rng::new(seed ^ LFO_SEED)),
            lfo_value: 0.0,
            env_value: 0.0,
            started: 0,
            motion_at: 0,
            window: WindowTable::new(params.window),
            grains: [Grain::default(); MAX_GRAINS],
            active: 0,
            fading: [Grain::default(); MAX_GRAINS],
            fading_n: 0,
            rng: Rng::new(seed),
            trng: Rng::new(seed ^ TIMING_SEED),
            vrng: Rng::new(seed ^ VARY_SEED),
            gap: 1.0,
            prefill: false,
            fit_size: 2.0,
            fade_left: 0,
            fade_len: ((START_FADE_SECS * f64::from(out_rate.max(1))) as u32).max(1),
            clock: 0,
            next_at: 0.0,
            last_spawn: 0.0,
            spawning: false,
            scan_offset: 0.0,
            span: (0, 0),
            stats: Stats::default(),
            last_hit: None,
            beat: None,
            sc: Scratch::new(),
            ey: 0.0,
            eq: 0.0,
            es1: 0.0,
            es2: 0.0,
            coherence: 0.0,
            coherence_to: 0.0,
            floor: 1.0,
            k_measure: coefficient(COHERENCE_SECS / MEASURE_EVERY as f64, out_rate.max(1)),
            k_coherence: coefficient(COHERENCE_SECS, out_rate.max(1)),
            k_floor: coefficient(FLOOR_SECS, out_rate.max(1)),
            k_floor_down: coefficient(FLOOR_SECS, out_rate.max(1)),
            k_floor_size: 0.0,
        }
    }

    /// The tempo a synced cloud follows, as output frames per beat (60 ×
    /// rate / BPM), whatever its [`TempoSync::frames_per_beat`] says: a
    /// player sets it from the transport, so a tempo change moves the
    /// grains at once. Ignored unless finite and at least one frame.
    pub fn set_tempo(&mut self, frames_per_beat: f64) {
        if frames_per_beat.is_finite() && frames_per_beat >= 1.0 {
            self.beat = Some(frames_per_beat);
        }
    }

    /// The tempo sync in force: the params', at the tempo set.
    fn sync(&self) -> Option<TempoSync> {
        let mut s = self.moved.sync?;
        if let Some(b) = self.beat {
            s.frames_per_beat = b;
        }
        Some(s)
    }

    /// The settings as set.
    pub fn params(&self) -> &Params {
        &self.params
    }

    /// The settings the next grains are made with: as set, moved by the
    /// LFO and the envelope as they are now.
    pub fn moved_params(&self) -> &Params {
        &self.moved
    }

    /// Takes effect for the next grain; grains already playing finish as
    /// they started. A new window shape rebuilds the table (1025 values).
    /// The LFO and the envelope move the new settings by as much as they
    /// moved the old (they move on at the next part of the voice's clock).
    pub fn set_params(&mut self, p: Params) {
        self.params = p;
        self.window.set(p.window);
        self.apply_motion();
    }

    /// Restarts the random sequence.
    pub fn reseed(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
        self.trng = Rng::new(seed ^ TIMING_SEED);
        self.vrng = Rng::new(seed ^ VARY_SEED);
        self.lfo = LfoState::new(Rng::new(seed ^ LFO_SEED));
    }

    /// Moves the LFO on to the voice's clock, takes its value and the
    /// envelope's level, and moves the settings by them.
    fn tick_motion(&mut self) {
        let rate = f64::from(self.out_rate);
        let lfo = self.params.lfo;
        let period = match lfo.beats {
            Some(b) if b.is_finite() && b > 0.0 => {
                f64::from(b.clamp(1.0 / 64.0, 256.0)) * self.beat.unwrap_or(rate * 0.5)
            }
            _ => rate / f64::from(lfo.rate_hz.clamp(0.01, 40.0)),
        };
        self.lfo
            .advance((self.clock - self.motion_at) as f64 / period.max(1.0));
        self.motion_at = self.clock;
        self.lfo_value = if lfo.target == MotionTarget::Off {
            0.0
        } else {
            self.lfo.value(lfo.shape)
        };
        let env = self.params.env;
        self.env_value = if env.target == MotionTarget::Off {
            0.0
        } else {
            env_level(&env, self.clock - self.started, self.out_rate)
        };
        self.apply_motion();
    }

    /// The settings as set, moved by the LFO's value and the envelope's
    /// level as they last were.
    fn apply_motion(&mut self) {
        let mut p = self.params;
        let (lfo, env) = (p.lfo, p.env);
        moved(
            &mut p,
            lfo.target,
            lfo.depth.clamp(-1.0, 1.0) * self.lfo_value,
        );
        moved(
            &mut p,
            env.target,
            env.depth.clamp(-1.0, 1.0) * self.env_value,
        );
        self.moved = p;
    }

    /// Starts making grains, the first at the start of the next render.
    /// Scanning restarts from `position`. The cloud starts already running:
    /// with the first grain come a few that would be playing had it been
    /// running for a grain's length (up to 8, each part-way through its
    /// window, reading on from where the scan was then, never wrapped round
    /// the span), and the voice fades in over 4 ms, so
    /// a cloud speaks at its level at once at any size and fills in to its
    /// density over the next grain (a slow swell is the amp envelope's
    /// attack). A start on a hit (see [`GrainVoice::render`], "Hits") has
    /// only the hit's grain. The LFO and the envelope start again.
    pub fn start(&mut self) {
        // The LFO and the envelope start again.
        self.lfo.restart();
        self.started = self.clock;
        self.motion_at = self.clock;
        self.tick_motion();
        self.spawning = true;
        self.prefill = true;
        self.next_at = self.clock as f64;
        self.last_spawn = self.clock as f64;
        self.scan_offset = 0.0;
        self.last_hit = None;
        // Until the cloud has played long enough to tell, guess how its
        // grains add from the settings: in step only when every grain
        // reads the same audio as the one before it at the same moment.
        let p = &self.moved;
        let ratio = (p.pitch.clamp(-48.0, 48.0) / 12.0).exp2();
        let in_step = p.spray <= 0.0
            && p.pitch_spray <= 0.0
            && p.reverse <= 0.0
            && (p.scan.clamp(-4.0, 4.0) - ratio).abs() < 0.01;
        self.coherence = if in_step { 1.0 } else { 0.0 };
        self.coherence_to = self.coherence;
        self.ey = 0.0;
        self.eq = 0.0;
        self.es1 = 0.0;
        self.es2 = 0.0;
        self.floor = 1.0;
    }

    /// Stops making grains; the ones playing finish (a natural release).
    pub fn stop(&mut self) {
        self.spawning = false;
    }

    /// Silences at once and hands its grains back to the budget. Clicks
    /// unless the output is already being faded.
    pub fn kill(&mut self, budget: &mut Budget) {
        self.spawning = false;
        budget.live = budget.live.saturating_sub(self.active + self.fading_n);
        self.active = 0;
        self.fading_n = 0;
        self.stats.active = 0;
    }

    /// True while making grains or while any still play.
    pub fn is_active(&self) -> bool {
        self.spawning || self.active > 0 || self.fading_n > 0
    }
    pub fn is_spawning(&self) -> bool {
        self.spawning
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The reading position (before spray), 0 to 1 across the span of the
    /// last render.
    pub fn playhead(&self) -> f32 {
        let len = (self.span.1 - self.span.0) as f64;
        if len <= 0.0 {
            return 0.0;
        }
        (self.base_offset(len) / len) as f32
    }

    /// Fills `out` with the grains playing now and returns how many.
    pub fn grains(&self, out: &mut [GrainView]) -> usize {
        let (start, end) = self.span;
        let len = ((end - start) as f64).max(1.0);
        let n = self.active.min(out.len());
        for (o, g) in out.iter_mut().zip(&self.grains[..n]) {
            *o = GrainView {
                position: ((g.pos - start as f64) / len).clamp(0.0, 1.0) as f32,
                level: self.window.at(g.age as f32 * g.dphase + g.phase0),
                reverse: g.inc < 0.0,
            };
        }
        n
    }

    /// How many grains overlap at once at these settings.
    fn overlap(&self) -> f64 {
        self.density() * self.size_frames() / f64::from(self.out_rate)
    }

    fn base_offset(&self, len: f64) -> f64 {
        (f64::from(self.moved.position.clamp(0.0, 1.0)) * len + self.scan_offset).rem_euclid(len)
    }

    /// How much longer than the size the varied grains are on average:
    /// the mean of 2^(s·u) for u even in −1 to 1, which is 1.08 at full
    /// variation.
    /// How much faster the scan moves for the tempo ([`Params::scan_tempo`]):
    /// the transport's tempo over the source's, 1 without both (held to
    /// 1/8 to 8).
    fn scan_tempo_ratio(&self) -> f64 {
        match (self.moved.scan_tempo, self.beat) {
            (Some(bpm), Some(beat)) if bpm.is_finite() && bpm > 0.0 => {
                let now = 60.0 * f64::from(self.out_rate) / beat;
                (now / f64::from(bpm)).clamp(0.125, 8.0)
            }
            _ => 1.0,
        }
    }

    /// Free-running grains at regular times (timing 0), which keep their
    /// exact times.
    fn exact(&self) -> bool {
        self.moved.timing <= 0.0 && self.sync_interval().is_none()
    }

    fn size_spray_mean(&self) -> f64 {
        let s = f64::from(self.moved.size_spray.clamp(0.0, 1.0));
        if s < 1e-6 {
            return 1.0;
        }
        (s.exp2() - (-s).exp2()) / (2.0 * s * std::f64::consts::LN_2)
    }

    fn size_frames(&self) -> f64 {
        let p = &self.moved;
        let rate = f64::from(self.out_rate);
        let frames = match self
            .sync()
            .and_then(|s| s.size_beats.map(|b| f64::from(b) * s.frames_per_beat))
        {
            Some(f) => f,
            None => f64::from(p.size_ms) * rate / 1000.0,
        };
        frames.clamp(rate * 0.001, rate * 4.0).max(2.0)
    }

    /// Grains per second, synced or not.
    fn density(&self) -> f64 {
        let p = &self.moved;
        let d = match self.sync().and_then(|s| s.grains_per_beat.map(|g| (s, g))) {
            Some((s, g)) if s.frames_per_beat > 0.0 => {
                f64::from(g) * f64::from(self.out_rate) / s.frames_per_beat
            }
            _ => f64::from(p.density),
        };
        d.clamp(0.1, 1000.0)
    }

    /// Synced grain interval in output frames, if density is synced.
    fn sync_interval(&self) -> Option<f64> {
        let s = self.sync()?;
        let g = s.grains_per_beat?;
        if s.frames_per_beat <= 0.0 || g <= 0.0 {
            return None;
        }
        Some((s.frames_per_beat / f64::from(g)).max(1.0))
    }

    /// The average gap between free-running grains, in output frames: the
    /// density's, or wider when the cloud wants more grains than
    /// [`CAP_FIT`] (it thins evenly).
    fn period(&self) -> f64 {
        (f64::from(self.out_rate) / self.density()).max(self.fit_size / CAP_FIT)
    }

    /// Offset in this block (at or after `from`, before `n`) of the next grain.
    fn next_spawn(&mut self, transport_frame: u64, from: usize, n: usize) -> Option<usize> {
        if !self.spawning || from >= n {
            return None;
        }
        if let Some(iv) = self.sync_interval() {
            // Grain k starts on frame round(k × interval) of the transport.
            let t = transport_frame + from as u64;
            let mut k = ((t as f64 - 0.5) / iv).ceil().max(0.0) as u64;
            while ((k as f64) * iv).round() < t as f64 {
                k += 1;
            }
            let f = ((k as f64) * iv).round() as u64;
            let off = f.checked_sub(transport_frame)? as usize;
            return (off < n).then_some(off);
        }
        // A faster density takes effect at once, not after a slow wait.
        let gap = self.period();
        self.next_at = self.next_at.min(self.last_spawn + gap * self.gap);
        let off = (self.next_at - self.clock as f64).ceil().max(from as f64);
        (off < n as f64).then_some(off as usize)
    }

    /// Mixes this voice into `left` and `right` (it adds; clear them first
    /// if this is the only voice). `transport_frame` is the absolute frame of
    /// `left[0]`, used only for tempo sync.
    ///
    /// **Level.** Overlapping grains add up, and how depends on what they
    /// read: grains of unrelated audio add in power (`n` of them are `√n`
    /// louder), grains reading the same audio in step (a granular time
    /// stretch, a steady tone) add in amplitude (`n` louder). The voice
    /// keeps, frame by frame, the sum of the grains' window levels (`S1`)
    /// and of their squares (`S2`), and divides the cloud by `c × S1 +
    /// (1 - c) × √S2`, where `c` is how far the grains are in step: what
    /// the output's power (smoothed over 50 ms) is against the grains'
    /// own powers summed, between 1 (unrelated) and `S1² / S2` (in step).
    /// So a dense cloud plays at its source's level whatever its size,
    /// density, scan, spray or window, and the window sum's ripple is gone.
    /// The divisor never goes below a floor: 1 in a sparse cloud (overlap
    /// 1 or less), so lone grains keep their windows and today's level;
    /// 0.3 from overlap 2. It starts at 1 and falls over a quarter of a
    /// grain, so a cloud's first grains keep their windows, and it glides
    /// back to 1 (in 10 ms) once grains stop, so a cloud's tail fades as
    /// its last grains do.
    ///
    /// **Hits.** A source with hits ([`GrainSource::next_hit`]: the sampler
    /// gives a sound's drum hits to a pad in Stretch mode) keeps them sharp
    /// while the scan moves forward. Overlapping grains each play a hit
    /// from their own place, so a stretched drum flams (early when it is
    /// slowed, late when it is sped up) and loses its attack. Instead, a
    /// grain reading forward stops at the next hit: its window is fitted
    /// to end there, and a grain that would be under a quarter of its size
    /// is not made. When the scan reaches a hit, between frames if it falls
    /// there, one grain starts on it at its window's peak and plays the
    /// rest of its window over at least half the size, at the cloud's
    /// pitch, in the centre and at full level; the free-running grains
    /// start again a period after it. So each hit plays once, at its time
    /// and at its own attack; what a slowed stretch has too little of
    /// just before a hit thins out, as a beat-slicing stretch gaps.
    ///
    /// **Movement.** At the start of each 64-frame part of the voice's
    /// clock the LFO moves on and the envelope is read ([`crate::motion`]),
    /// and the part's grains are made with the settings they move
    /// ([`GrainVoice::moved_params`]).
    pub fn render<S: GrainSource>(
        &mut self,
        src: &S,
        budget: &mut Budget,
        transport_frame: u64,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let n = left.len().min(right.len());
        let (start, end) = src.span();
        if end - start < 8 {
            // Nothing readable: let playing grains fall silent with the source.
            budget.live = budget.live.saturating_sub(self.active + self.fading_n);
            self.active = 0;
            self.fading_n = 0;
            self.stats.active = 0;
            self.span = (start, end);
            self.clock += n as u64;
            return;
        }
        self.span = (start, end);
        let len = (end - start) as f64;
        let src_ratio = f64::from(src.rate()) / f64::from(self.out_rate);
        let mut at = 0usize;
        while at < n {
            // Parts follow the voice's clock, not the call, so the output
            // doesn't depend on how the audio is cut into periods.
            let into = (self.clock % PART as u64) as usize;
            let m = (n - at).min(PART - into);
            if into == 0 {
                self.tick_motion();
            }
            let (scan, floor_to) = self.part_settings(len, src_ratio);
            let measure = (self.clock / PART as u64).is_multiple_of(MEASURE_EVERY);
            self.sc.clear(m, measure);
            let mut done = 0usize;
            let mut search = 0usize;
            // At most a few hits a part (they are 30 ms apart or more).
            let mut hits_left = 4u8;
            while done < m {
                let next = self.next_spawn(transport_frame + at as u64, search, m);
                let mut seg = next.unwrap_or(m);
                let mut hit = None;
                if scan > 0.0 && hits_left > 0 {
                    if let Some((k, h, frac)) =
                        self.hit_between(src, done, seg.min(m - 1), scan, len)
                    {
                        seg = k;
                        hit = Some((h, frac));
                    }
                }
                mix::<S, true>(
                    &mut self.grains,
                    &mut self.active,
                    &self.window,
                    src,
                    budget,
                    &mut self.sc,
                    done,
                    seg,
                    measure,
                );
                if self.fading_n > 0 {
                    mix::<S, false>(
                        &mut self.fading,
                        &mut self.fading_n,
                        &self.window,
                        src,
                        budget,
                        &mut self.sc,
                        done,
                        seg,
                        false,
                    );
                }
                self.scan_offset = (self.scan_offset + scan * (seg - done) as f64).rem_euclid(len);
                done = seg;
                if let Some((h, frac)) = hit {
                    // A start on a hit needs no grains from before it.
                    self.prefill = false;
                    self.fade_for_hit(budget);
                    self.spawn_hit(src, budget, src_ratio, h, frac);
                    let born = self.clock as f64 + seg as f64 - frac;
                    self.last_spawn = born;
                    self.gap = gap_factor(self.moved.timing, self.trng.unit());
                    self.next_at = born + self.period() * self.gap;
                    self.last_hit = Some((h, self.clock + seg as u64));
                    hits_left -= 1;
                    search = seg + 1;
                    continue;
                }
                if let Some(off) = next {
                    if self.prefill {
                        self.prefill = false;
                        self.prefill_grains(src, budget, src_ratio, scan);
                    }
                    let now = self.clock as f64 + off as f64;
                    // Regular grains keep their exact times, between frames
                    // when the period isn't a whole number of frames, so
                    // their rate is exact (a tone's pitch); others start
                    // on the frame.
                    let due = now - self.next_at;
                    let born = if self.exact() && (0.0..1.0).contains(&due) {
                        self.next_at
                    } else {
                        now
                    };
                    self.spawn(src, budget, src_ratio, 0, scan, now - born);
                    self.last_spawn = born;
                    self.gap = gap_factor(self.moved.timing, self.trng.unit());
                    self.next_at = born + self.period() * self.gap;
                    search = off + 1;
                }
            }
            self.level(
                &mut left[at..at + m],
                &mut right[at..at + m],
                floor_to,
                measure,
            );
            self.clock += m as u64;
            at += m;
        }
        self.stats.active = self.active;
        self.stats.thinned =
            self.spawning && self.density() * self.fit_size / f64::from(self.out_rate) > CAP_FIT;
    }

    /// What this part's grains go by, from the settings (as moved): the
    /// scan's speed in source frames an output frame, and the floor the
    /// level's divisor glides to. Sets the grains' fitted size and the
    /// floor's falling coefficient.
    fn part_settings(&mut self, len: f64, src_ratio: f64) -> (f64, f32) {
        let scan =
            f64::from(self.moved.scan.clamp(-4.0, 4.0)) * src_ratio * self.scan_tempo_ratio();
        // As `spawn` shortens a grain whose read wouldn't fit.
        let speed = f64::from((self.moved.pitch.clamp(-48.0, 48.0) / 12.0).exp2()) * src_ratio;
        let size = self.size_frames();
        self.fit_size = (size * self.size_spray_mean())
            .min((len - 4.0) / speed)
            .max(2.0);
        let floor_to = if self.spawning {
            floor_for(self.overlap(), level_rms(self.moved.level_spray))
        } else {
            1.0
        };
        if size != self.k_floor_size {
            // A quarter of a grain, at least the rising time.
            let secs = (size / f64::from(self.out_rate) / 4.0).max(FLOOR_SECS);
            self.k_floor_down = coefficient(secs, self.out_rate);
            self.k_floor_size = size;
        }
        (scan, floor_to)
    }

    /// The first hit the scan reaches on a frame from `from` to `last` of
    /// this part (see [`GrainVoice::render`], "Hits"): the frame, the hit's
    /// frame in the source, and how far past it the scan is by then, in
    /// output frames (0 to 1). The scan moves `scan` source frames an
    /// output frame, forward. A hit started on this frame or the one
    /// before is not found again.
    fn hit_between<S: GrainSource>(
        &self,
        src: &S,
        from: usize,
        last: usize,
        scan: f64,
        len: f64,
    ) -> Option<(usize, f64, f64)> {
        if last < from {
            return None;
        }
        let (start, end) = self.span;
        let now = self.clock + from as u64;
        let just = self
            .last_hit
            .filter(|&(_, at)| now <= at + 1)
            .map(|(h, _)| h);
        let find = |lo: f64, hi: f64| match src.next_hit(lo, hi) {
            Some(h) if Some(h) == just => src.next_hit(h, hi),
            h => h,
        };
        // Where the scan is on frame `from`, and was the frame before.
        let p = start as f64 + self.base_offset(len);
        let (lo, hi) = (p - scan, p + scan * (last - from) as f64);
        let (h, at) = match find(lo, hi) {
            Some(h) => (h, h),
            // Past the span's end the scan reads from its start again.
            None if hi >= end as f64 => {
                let h = find(lo - len, hi - len)?;
                (h, h + len)
            }
            None => return None,
        };
        let k = from + ((at - p) / scan).ceil().max(0.0) as usize;
        if k > last {
            return None;
        }
        let frac = ((p + scan * (k - from) as f64 - at) / scan).clamp(0.0, 0.999_999);
        Some((k, h, frac))
    }

    /// As a hit starts, the grains playing fade out over [`HIT_FADE_SECS`]
    /// along the rest of their windows (one still rising from where its
    /// falling side has the same level), at the level the cloud gave them,
    /// and leave the level's sums: the hit plays alone at its own level.
    fn fade_for_hit(&mut self, budget: &mut Budget) {
        let f = (HIT_FADE_SECS * f64::from(self.out_rate)).max(2.0) as u32;
        let peak = self.window.peak();
        // The divisor the level law gives them now (see `level`).
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for g in &self.grains[..self.active] {
            let a = self.window.at(g.age as f32 * g.dphase + g.phase0) * g.lvl;
            s1 += a;
            s2 += a * a;
        }
        let c = self.coherence;
        let divisor = (c * s1 + (1.0 - c) * s2.sqrt()).max(self.floor);
        for i in 0..self.active {
            let mut g = self.grains[i];
            if g.len - g.age > f {
                let now = g.age as f32 * g.dphase + g.phase0;
                let q = if now < peak {
                    peak + (peak - now) * (1.0 - peak) / peak
                } else {
                    now
                };
                g.dphase = (1.0 - q) / f as f32;
                g.phase0 = q - g.age as f32 * g.dphase;
                g.len = g.age + f;
            }
            g.lvl /= divisor;
            if self.fading_n < MAX_GRAINS {
                self.fading[self.fading_n] = g;
                self.fading_n += 1;
            } else {
                budget.live = budget.live.saturating_sub(1);
            }
        }
        self.active = 0;
    }

    /// Starts the grain on a hit at source frame `hit`, `frac` of an output
    /// frame ago (see [`GrainVoice::render`], "Hits").
    fn spawn_hit<S: GrainSource>(
        &mut self,
        src: &S,
        budget: &mut Budget,
        src_ratio: f64,
        hit: f64,
        frac: f64,
    ) {
        if self.active >= MAX_GRAINS || budget.live >= budget.limit {
            self.stats.skipped += 1;
            budget.skipped += 1;
            return;
        }
        let p = self.moved;
        let (start, end) = src.span();
        let hi = end as f64 - 3.0;
        let mut semis = p.pitch.clamp(-48.0, 48.0);
        if p.scale & 0xFFF != 0 {
            semis = quantise(semis, p.scale);
        }
        let speed = f64::from((semis / 12.0).exp2()) * src_ratio;
        if hit < start as f64 || hit >= hi {
            return;
        }
        // From the window's peak to its end, over at least half the size.
        let peak = f64::from(self.window.peak()).min(0.999);
        let size = self.size_frames();
        let mut frames = ((1.0 - peak).max(0.5) * (size - 1.0)).floor();
        // Not through the next hit, nor past the span.
        let reach = |f: f64| hit + (f + frac) * speed;
        if let Some(next) = src.next_hit(hit, reach(frames)) {
            frames = frames.min(((next - hit) / speed - frac).floor());
        }
        if reach(frames) > hi {
            frames = ((hi - hit) / speed - frac).floor();
        }
        if frames < 2.0 {
            return;
        }
        let dphase = ((1.0 - peak) / frames) as f32;
        self.grains[self.active] = Grain {
            pos: hit + speed * frac,
            inc: speed,
            age: 0,
            len: frames as u32 + 1,
            dphase,
            phase0: peak as f32 + frac as f32 * dphase,
            lvl: 1.0,
            pl: 1.0,
            pr: 1.0,
            aim: src.aim(speed),
        };
        self.active += 1;
        budget.live += 1;
        self.stats.spawned += 1;
        self.stats.peak_active = self.stats.peak_active.max(self.active);
    }

    /// Grains a running cloud would have now, besides the one being born:
    /// walking back one gap at a time (drawn as the timing draws them, or
    /// the synced interval; at least a [`START_GRAINS`]th of a grain) while
    /// a grain born then would still play, each made with that age.
    fn prefill_grains<S: GrainSource>(
        &mut self,
        src: &S,
        budget: &mut Budget,
        src_ratio: f64,
        scan: f64,
    ) {
        let size = self.fit_size;
        let synced = self.sync_interval();
        // A thinner cloud than the density's: enough for the level and the
        // sound from the first frame, while the full density fills in
        // over the next grain.
        let thin = size / START_GRAINS;
        let period = self.period().max(thin);
        let mut age = 0.0;
        let mut made = false;
        loop {
            age += match synced {
                Some(iv) => iv.max(thin),
                None => period * gap_factor(self.moved.timing, self.trng.unit()),
            };
            if age >= size - 1.0 {
                break;
            }
            self.spawn(src, budget, src_ratio, age as u32, scan, 0.0);
            made = true;
        }
        if made {
            self.fade_left = self.fade_len;
        }
    }

    /// Sets the level of the part in the scratch buffers (see
    /// [`GrainVoice::render`]) and adds it to `left` and `right`.
    fn level(&mut self, left: &mut [f32], right: &mut [f32], floor_to: f32, measure: bool) {
        let m = left.len().min(right.len());
        let gain = self.moved.gain;
        let (k, km) = (self.k_coherence, self.k_measure);
        // Grains varied in level cover less window power.
        let cover = COVER * level_rms(self.moved.level_spray).powi(2);
        let kf = if floor_to < self.floor {
            self.k_floor_down
        } else {
            self.k_floor
        };
        let sc = &self.sc;
        for i in 0..m {
            let (yl, yr, s1, s2) = (sc.l[i], sc.r[i], sc.s1[i], sc.s2[i]);
            self.floor += kf * (floor_to - self.floor);
            if measure {
                self.ey = settle(self.ey + km * (yl * yl + yr * yr - self.ey));
                self.eq = settle(self.eq + km * (sc.q[i] - self.eq));
                self.es1 = settle(self.es1 + km * (s1 * s1 - self.es1));
                self.es2 = settle(self.es2 + km * (s2 - self.es2));
                // Judged every 8 frames of the voice's own clock.
                if (self.clock + i as u64) & 7 == 0 && self.es2 > cover && self.eq > 1e-12 {
                    // What grains fully in step would give; near 1 the
                    // two laws agree anyway.
                    let in_step = self.es1 / self.es2;
                    if in_step > 1.05 {
                        self.coherence_to =
                            ((self.ey / self.eq - 1.0) / (in_step - 1.0)).clamp(0.0, 1.0);
                    }
                }
            }
            self.coherence += k * (self.coherence_to - self.coherence);
            let c = self.coherence;
            let divisor = (c * s1 + (1.0 - c) * s2.sqrt()).max(self.floor);
            let mut g = gain / divisor;
            if self.fade_left > 0 {
                g *= 1.0 - self.fade_left as f32 / self.fade_len as f32;
                self.fade_left -= 1;
            }
            left[i] += yl * g;
            right[i] += yr * g;
        }
    }

    /// Makes one grain. `age` (frames) makes it as if it had been born
    /// that long ago: read from where the scan was then (`scan`, source
    /// frames an output frame) and already that far through its window.
    fn spawn<S: GrainSource>(
        &mut self,
        src: &S,
        budget: &mut Budget,
        src_ratio: f64,
        age: u32,
        scan: f64,
        frac: f64,
    ) {
        // Draw the same random numbers every time, used or not, so changing
        // one parameter doesn't reshuffle the others.
        let r_spray = self.rng.bipolar();
        let r_pitch = self.rng.bipolar();
        let r_rev = self.rng.unit();
        let r_pan = self.rng.bipolar();
        let r_size = self.vrng.bipolar();
        let r_level = self.vrng.unit();
        if self.active >= MAX_GRAINS || budget.live >= budget.limit {
            self.stats.skipped += 1;
            budget.skipped += 1;
            return;
        }
        let p = self.moved;
        let (start, end) = src.span();
        let len = (end - start) as f64;

        let mut semis = p.pitch.clamp(-48.0, 48.0) + p.pitch_spray.clamp(0.0, 24.0) * r_pitch;
        if p.scale & 0xFFF != 0 {
            semis = quantise(semis, p.scale);
        }
        let ratio = f64::from((semis / 12.0).exp2());
        let reverse = r_rev < p.reverse.clamp(0.0, 1.0);
        let speed = ratio * src_ratio;

        // Keep the whole read (with the interpolator's neighbours) in the span.
        let lo = start as f64 + 1.0;
        let hi = end as f64 - 3.0;
        let mut size = self.size_frames();
        let size_spray = p.size_spray.clamp(0.0, 1.0);
        if size_spray > 0.0 {
            size *= f64::from((size_spray * r_size).exp2());
        }
        if size * speed > hi - lo {
            size = (hi - lo) / speed;
        }
        if size < 2.0 {
            self.stats.skipped += 1;
            return;
        }
        let size = size.floor();
        let extent = (size - 1.0 + frac) * speed;
        if age as f64 >= size - 1.0 {
            return;
        }
        let spray = f64::from(p.spray.clamp(0.0, 1.0) * spray_draw(p.spray_shape, r_spray));
        // A grain of the running start, born `age` frames ago, began where
        // the scan was then, not wrapped round the span: only what it still
        // has to play is read, which keeps the start in step with the
        // cloud to come rather than opening with the sound's far end.
        let past = f64::from(age);
        let pos = start as f64 + (self.base_offset(len) + spray * len * 0.5).rem_euclid(len)
            - scan * past;
        let pos = if reverse {
            pos.clamp(lo + extent, hi + speed * past)
        } else {
            pos.clamp(lo - speed * past, hi - extent)
        };
        let inc = if reverse { -speed } else { speed };
        // In a forward scan, a grain reading forward stops at the next hit
        // (see `render`, "Hits"): its window fitted to end there.
        let mut size = size;
        if scan > 0.0 && !reverse {
            if let Some(h) = src.next_hit(pos, pos + extent - 1e-6) {
                let cut = ((h - pos) / speed - frac).floor() + 1.0;
                if cut < (size * MIN_CUT).max(2.0) || f64::from(age) >= cut - 1.0 {
                    return;
                }
                size = cut;
            }
        }

        // Equal-power pan, 1 on both sides in the centre, so a spread cloud
        // of unrelated grains keeps its level. At full spread the pans
        // lean toward the sides (|r|^0.5), which widens the cloud.
        let lean = r_pan.abs().sqrt().copysign(r_pan);
        let pan = p.spread.clamp(0.0, 1.0) * lean;
        let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        // The cloud's level is set as the grains add up (`level`), so each
        // grain plays at full level here, or its varied level.
        let level_spray = p.level_spray.clamp(0.0, 1.0);
        let lvl = if level_spray > 0.0 {
            db_gain(-LEVEL_SPRAY_DB * level_spray * r_level)
        } else {
            1.0
        };
        self.grains[self.active] = Grain {
            pos: pos + inc * (f64::from(age) + frac),
            inc,
            age,
            len: size as u32,
            dphase: 1.0 / (size as f32 - 1.0),
            phase0: frac as f32 / (size as f32 - 1.0),
            lvl,
            pl: angle.cos() * std::f32::consts::SQRT_2,
            pr: angle.sin() * std::f32::consts::SQRT_2,
            aim: src.aim(speed),
        };
        self.active += 1;
        budget.live += 1;
        self.stats.spawned += 1;
        self.stats.peak_active = self.stats.peak_active.max(self.active);
    }
}

/// Mixes the grains playing into frames `from..to` of the scratch buffers,
/// and retires the ones that finish.
#[allow(clippy::too_many_arguments)]
fn mix<S: GrainSource, const LEVEL: bool>(
    grains: &mut [Grain; MAX_GRAINS],
    active: &mut usize,
    window: &WindowTable,
    src: &S,
    budget: &mut Budget,
    sc: &mut Scratch,
    from: usize,
    to: usize,
    measure: bool,
) {
    let n = to.saturating_sub(from);
    if n == 0 {
        return;
    }
    let (sl_, sr_) = (&mut sc.l[from..to], &mut sc.r[from..to]);
    let (s1_, s2_, q_) = (
        &mut sc.s1[from..to],
        &mut sc.s2[from..to],
        &mut sc.q[from..to],
    );
    let mut i = 0;
    while i < *active {
        let g = &mut grains[i];
        let m = ((g.len - g.age) as usize).min(n);
        let (mut pos, mut age) = (g.pos, g.age);
        let (lvl, pl, pr, ph0) = (g.lvl, g.pl, g.pr, g.phase0);
        // Grains are kept inside the span, which never starts below
        // frame 0, so truncation is floor (and avoids a libm call).
        // Below 2^31 frames (13 hours at 44.1 kHz, so every sample and
        // nearly every live buffer) the frame goes through an `i32`,
        // which ARMv7 converts to and from a double in one instruction
        // each; an `i64` takes two library calls. Same result either way.
        let last = pos + g.inc * m as f64;
        // One frame: read, window, and add to the sums (the grain's own
        // power too, in a measured part).
        macro_rules! frame {
            ($k:expr, $read:expr, $measure:expr) => {{
                let [a_l, a_r] = $read;
                let a = window.at(age as f32 * g.dphase + ph0) * lvl;
                let (vl, vr) = (a_l * a * pl, a_r * a * pr);
                sl_[$k] += vl;
                sr_[$k] += vr;
                if LEVEL {
                    s1_[$k] += a;
                    s2_[$k] += a * a;
                    if $measure {
                        q_[$k] += vl * vl + vr * vr;
                    }
                }
                pos += g.inc;
                age += 1;
            }};
        }
        let small = pos.min(last) >= 0.0 && pos.max(last) < SMALL_FRAMES;
        let aim = g.aim;
        match (small, measure) {
            // A grain the source reads its own way (band-limited).
            _ if aim.clean => {
                if measure {
                    for k in 0..m {
                        frame!(k, src.read_aimed(pos, aim), true);
                    }
                } else {
                    for k in 0..m {
                        frame!(k, src.read_aimed(pos, aim), false);
                    }
                }
            }
            (true, false) => {
                for k in 0..m {
                    let f = pos as i32;
                    frame!(
                        k,
                        src.read(i64::from(f), (pos - f64::from(f)) as f32),
                        false
                    );
                }
            }
            (true, true) => {
                for k in 0..m {
                    let f = pos as i32;
                    frame!(k, src.read(i64::from(f), (pos - f64::from(f)) as f32), true);
                }
            }
            (false, false) => {
                for k in 0..m {
                    let f = pos as i64;
                    frame!(k, src.read(f, (pos - f as f64) as f32), false);
                }
            }
            (false, true) => {
                for k in 0..m {
                    let f = pos as i64;
                    frame!(k, src.read(f, (pos - f as f64) as f32), true);
                }
            }
        }
        g.pos = pos;
        g.age = age;
        if g.age >= g.len {
            budget.live = budget.live.saturating_sub(1);
            *active -= 1;
            grains[i] = grains[*active];
        } else {
            i += 1;
        }
    }
}

/// The nearest allowed semitone to `semis`. `mask` bit n allows semitone n
/// of every octave (bit 0 is the original pitch). Ties go down. A mask with
/// no bits set returns `semis` unchanged.
pub fn quantise(semis: f32, mask: u16) -> f32 {
    if mask & 0xFFF == 0 {
        return semis;
    }
    let mut best = semis;
    let mut dist = f32::INFINITY;
    let lo = semis.floor() as i32 - 12;
    for n in lo..=lo + 25 {
        if mask & (1 << n.rem_euclid(12)) != 0 {
            let d = (n as f32 - semis).abs();
            if d < dist {
                dist = d;
                best = n as f32;
            }
        }
    }
    best
}
