//! The sampler's project model: sources, regions, chop sets, layers, pads,
//! the master effect and the undo log. Owned by the control thread; never touched by the
//! audio thread, which gets an [`crate::engine::EngineView`] built from it.
//! Design: `docs/dev/design/sampler-design.md` and `sampler-architecture.md`.

use std::sync::Arc;

pub use mpfree_grain::{
    GrainEnv, GrainLfo, LfoShape, MotionTarget, Params as GrainParams,
    SprayShape as GrainSprayShape, TempoSync, Window as GrainWindow,
};

use crate::dsp::{as_i16, Sample};
pub use crate::dsp::{Ahdsr, Filter, FilterKind};

/// Pads in a program: 8 banks of 16.
pub const PADS: usize = 128;
/// Layers per pad.
pub const MAX_LAYERS: usize = 16;
/// Slices per chop set, and the most a sliced region plays as (R4: a
/// whole break on one pad).
pub const MAX_SLICES: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub enum ModelError {
    BadSource(&'static str),
    UnknownSource,
    BadRegion(&'static str),
    BadPad,
    TooManyLayers,
    TooManySlices,
    BadSlice,
    BadMasterFx,
    /// A name with nothing in it.
    BadName,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ModelError {}

/// A source's identity: a 64-bit FNV-1a hash of its rate, channel count and
/// samples, so the same audio is stored once however many pads use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub u64);

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// A sound's samples as it holds them, interleaved when stereo: 16-bit
/// integers when every sample is a 16-bit value exactly (a 16-bit WAV or
/// AIFF, and what is cut from one), at half the memory; 32-bit floats
/// otherwise. Either reads as the same floats ([`Sample::f`]).
#[derive(Clone, Debug)]
pub enum Pcm {
    F32(Arc<[f32]>),
    I16(Arc<[i16]>),
}

impl Pcm {
    /// As 16 bits if every sample is 16-bit exactly ([`as_i16`]), else as
    /// they are.
    pub fn new(data: Vec<f32>) -> Pcm {
        if !data.iter().all(|&x| as_i16(x).is_some()) {
            return Pcm::F32(data.into());
        }
        Pcm::I16(data.iter().map(|&x| as_i16(x).unwrap_or(0)).collect())
    }

    /// Samples, not frames.
    pub fn len(&self) -> usize {
        match self {
            Pcm::F32(d) => d.len(),
            Pcm::I16(d) => d.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Whether it is held as 16 bits.
    pub fn is_16_bit(&self) -> bool {
        matches!(self, Pcm::I16(_))
    }
    /// The bytes it takes.
    pub fn bytes(&self) -> usize {
        match self {
            Pcm::F32(d) => d.len() * 4,
            Pcm::I16(d) => d.len() * 2,
        }
    }
    /// Sample `i`. Panics past the end, as indexing does.
    #[inline]
    pub fn get(&self, i: usize) -> f32 {
        match self {
            Pcm::F32(d) => d[i],
            Pcm::I16(d) => d[i].f(),
        }
    }
    /// Every sample, as floats.
    pub fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        let (a, b) = match self {
            Pcm::F32(d) => (Some(d.iter().copied()), None),
            Pcm::I16(d) => (None, Some(d.iter().map(|x| x.f()))),
        };
        a.into_iter().flatten().chain(b.into_iter().flatten())
    }
    /// Samples `range` as floats: borrowed when it holds floats, converted
    /// (a copy) when it holds 16 bits. None past the end.
    pub fn f32s(&self, range: std::ops::Range<usize>) -> Option<std::borrow::Cow<'_, [f32]>> {
        match self {
            Pcm::F32(d) => d.get(range).map(std::borrow::Cow::Borrowed),
            Pcm::I16(d) => d
                .get(range)
                .map(|s| std::borrow::Cow::Owned(s.iter().map(|x| x.f()).collect())),
        }
    }
    /// Every sample as floats ([`Pcm::f32s`] of all of it).
    pub fn all_f32(&self) -> std::borrow::Cow<'_, [f32]> {
        self.f32s(0..self.len()).expect("the whole range")
    }
    /// A copy of every sample, as floats.
    pub fn to_vec(&self) -> Vec<f32> {
        self.all_f32().into_owned()
    }
    /// Whether both are the same shared samples (not just equal ones).
    pub fn ptr_eq(&self, other: &Pcm) -> bool {
        match (self, other) {
            (Pcm::F32(a), Pcm::F32(b)) => Arc::ptr_eq(a, b),
            (Pcm::I16(a), Pcm::I16(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    /// Where the samples are in memory: the same for every clone, so
    /// memory counts can tell shared sounds apart.
    pub fn addr(&self) -> usize {
        match self {
            Pcm::F32(d) => d.as_ptr() as usize,
            Pcm::I16(d) => d.as_ptr() as usize,
        }
    }
    /// The mix of `channels` (1 or 2, interleaved) to one: what
    /// `mpfree_analysis::mono` gives for the floats, bit for bit, without
    /// a float copy of the whole sound first.
    pub fn mono(&self, channels: usize) -> Vec<f32> {
        let channels = channels.max(1);
        match self {
            Pcm::F32(d) => mpfree_analysis::mono(d, channels).unwrap_or_default(),
            Pcm::I16(d) => d
                .chunks_exact(channels)
                .map(|f| f.iter().map(|x| x.f()).sum::<f32>() / channels as f32)
                .collect(),
        }
    }
}

impl mpfree_analysis::Samples for Pcm {
    fn len(&self) -> usize {
        Pcm::len(self)
    }
    #[inline]
    fn at(&self, i: usize) -> f32 {
        self.get(i)
    }
}

impl From<Vec<f32>> for Pcm {
    fn from(data: Vec<f32>) -> Pcm {
        Pcm::new(data)
    }
}

/// Recorded or imported audio. Its audio never changes after it is made;
/// its name can ([`Project::rename`]).
#[derive(Clone, Debug)]
pub struct Source {
    id: SourceId,
    pub name: String,
    rate: u32,
    channels: u8,
    /// Interleaved when stereo; 16 bits when it can be ([`Pcm::new`]).
    data: Pcm,
    /// Its octave copies, for reading it more than an octave up
    /// ([`crate::bandlimit`]), shared by every clone. Made when a pad
    /// that can play it that high asks for them.
    octaves: Arc<crate::bandlimit::OctaveCell>,
    /// Its hits, for a granular Stretch ([`crate::hits`]), shared by every
    /// clone. Found when a pad that stretches it asks for them.
    hits: Arc<crate::hits::HitCell>,
}

fn fnv1a(hash: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *hash ^= u64::from(*b);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
}

impl Source {
    /// `data` is interleaved for stereo. Non-finite samples become 0.
    pub fn new(name: &str, rate: u32, channels: u8, data: Vec<f32>) -> Result<Source, ModelError> {
        if !(8000..=192_000).contains(&rate) {
            return Err(ModelError::BadSource("rate"));
        }
        if channels != 1 && channels != 2 {
            return Err(ModelError::BadSource("channels"));
        }
        if data.is_empty() || !data.len().is_multiple_of(usize::from(channels)) {
            return Err(ModelError::BadSource("length"));
        }
        let mut data = data;
        for x in &mut data {
            if !x.is_finite() {
                *x = 0.0;
            }
        }
        let mut h = 0xCBF2_9CE4_8422_2325u64;
        fnv1a(&mut h, &rate.to_le_bytes());
        fnv1a(&mut h, &[channels]);
        for x in &data {
            fnv1a(&mut h, &x.to_bits().to_le_bytes());
        }
        Ok(Source {
            id: SourceId(h),
            name: name.to_string(),
            rate,
            channels,
            data: Pcm::new(data),
            octaves: Arc::default(),
            hits: Arc::default(),
        })
    }

    /// From the existing WAV loader's prepared frames (always stereo).
    pub fn from_prepared(name: &str, p: &crate::PreparedSample) -> Source {
        let mut data = Vec::with_capacity(p.frames() * 2);
        for f in p.frame_slice() {
            data.push(f[0]);
            data.push(f[1]);
        }
        Source::new(name, p.sample_rate(), 2, data).expect("prepared samples are valid")
    }

    pub fn id(&self) -> SourceId {
        self.id
    }
    pub fn rate(&self) -> u32 {
        self.rate
    }
    pub fn channels(&self) -> u8 {
        self.channels
    }
    pub fn frames(&self) -> u64 {
        (self.data.len() / usize::from(self.channels)) as u64
    }
    /// Its samples, interleaved when stereo.
    pub fn data(&self) -> &Pcm {
        &self.data
    }
    /// The same sound (the same id) held as floats, as every sound was
    /// before 16-bit sounds were kept as 16 bits: for tests and benches
    /// that compare the two.
    pub fn held_as_floats(&self) -> Source {
        Source {
            data: Pcm::F32(self.data.to_vec().into()),
            octaves: Arc::default(),
            hits: Arc::default(),
            ..self.clone()
        }
    }
    /// Where its octave copies are kept ([`crate::bandlimit::prepare`]
    /// makes them).
    pub fn octaves(&self) -> &Arc<crate::bandlimit::OctaveCell> {
        &self.octaves
    }
    /// Makes its octave copies here and now if they aren't made yet: for
    /// offline renders and tests, which mustn't wait on a thread.
    pub fn make_octaves(&self) {
        crate::bandlimit::prepare_now(&self.octaves, &self.data, self.channels);
    }
    /// Where its hits are kept ([`crate::hits::prepare`] finds them).
    pub fn hits(&self) -> &Arc<crate::hits::HitCell> {
        &self.hits
    }
    /// Finds its hits here and now if they aren't found yet: for offline
    /// renders and tests, which mustn't wait on a thread.
    pub fn find_hits(&self) {
        crate::hits::prepare_now(&self.hits, &self.data, self.channels, self.rate);
    }

    /// The same sound at `rate` (itself when it is at that rate already),
    /// converted with `mpfree-stretch`'s windowed sinc, with the
    /// [`Rescale`] that moves anything placed on it to the same moments
    /// of the new one. Heavy for a long sound: call it off the audio and
    /// screen threads.
    pub fn at_rate(&self, rate: u32) -> Result<(Source, Rescale), ModelError> {
        let data = if rate == self.rate {
            self.data.to_vec()
        } else {
            let from = self.data.all_f32();
            mpfree_stretch::convert_rate(&from, usize::from(self.channels), self.rate, rate)
        };
        let to = Source::new(&self.name, rate, self.channels, data)?;
        let r = Rescale {
            from: self.id,
            to: to.id,
            ratio: f64::from(rate) / f64::from(self.rate),
            frames: to.frames(),
            rate,
        };
        Ok((to, r))
    }
}

/// Puts every sound in `sources` that isn't at `rate` at that rate
/// ([`Source::at_rate`]), and moves what `pads` and `chops` place on them
/// along. A sound that can't be converted (too short to hold a frame at
/// the new rate) stays as it was. Returns what moved, one [`Rescale`] a
/// sound.
pub fn convert_to_rate(
    sources: &mut [Source],
    pads: &mut [Pad],
    chops: &mut [ChopSet],
    rate: u32,
) -> Vec<Rescale> {
    convert_sounds(sources, pads, chops, &mut at_rate(rate))
}

/// What [`convert_sounds`] asks of each sound: its converted copy and
/// where frames on it land, or None to keep it as it is.
pub type Convert<'a> = dyn FnMut(&Source) -> Option<(Source, Rescale)> + 'a;

/// Each sound not at `rate` put at it, as [`convert_to_rate`] does.
pub fn at_rate(rate: u32) -> impl FnMut(&Source) -> Option<(Source, Rescale)> {
    move |s: &Source| {
        if s.rate() == rate {
            None
        } else {
            s.at_rate(rate).ok()
        }
    }
}

/// [`convert_to_rate`] with the conversion the caller's (`convert`): one
/// that remembers what it made, so sounds shared between kits stay
/// shared, or that keeps a sound as it is when memory is short.
pub fn convert_sounds(
    sources: &mut [Source],
    pads: &mut [Pad],
    chops: &mut [ChopSet],
    convert: &mut Convert<'_>,
) -> Vec<Rescale> {
    let mut moved = Vec::new();
    for s in sources.iter_mut() {
        let Some((to, r)) = convert(s) else {
            continue;
        };
        for p in pads.iter_mut() {
            p.rescale(&r);
        }
        for c in chops.iter_mut() {
            c.rescale(&r);
        }
        *s = to;
        moved.push(r);
    }
    moved
}

/// Where frames on a sound land once it is converted to another rate
/// ([`Source::at_rate`]): regions, loops, fades, warp pins and chop
/// markers on the sound `from` move to the same moments of `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rescale {
    pub from: SourceId,
    pub to: SourceId,
    /// The new rate over the old.
    pub ratio: f64,
    /// The converted sound's frames and rate.
    pub frames: u64,
    pub rate: u32,
}

impl Rescale {
    /// A frame of the old sound on the new one.
    pub fn frame(&self, f: u64) -> u64 {
        ((f as f64 * self.ratio).round() as u64).min(self.frames)
    }
    /// A length in frames on the new sound.
    pub fn length(&self, n: u32) -> u32 {
        (f64::from(n) * self.ratio).round().min(f64::from(u32::MAX)) as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopMode {
    Forward,
    PingPong,
}

/// A sustain loop inside a region. It is separate from the region's end, so
/// the sound can leave the loop and play its tail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loop {
    pub start: u64,
    pub end: u64,
    /// Equal-power crossfade length at the loop point, in frames (forward loops only).
    pub crossfade: u32,
    pub mode: LoopMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeCurve {
    Linear,
    Log,
    Exp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub frames: u32,
    pub curve: FadeCurve,
}

impl Fade {
    pub const NONE: Fade = Fade {
        frames: 0,
        curve: FadeCurve::Linear,
    };
    /// Gain at `x` in 0..1 of the fade (0 = silent end).
    pub fn shape(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self.curve {
            FadeCurve::Linear => x,
            FadeCurve::Log => 1.0 - (1.0 - x) * (1.0 - x),
            FadeCurve::Exp => x * x,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelPick {
    Stereo,
    MonoSum,
    Left,
    Right,
}

/// How a granular region's Grain page starts and which of its controls it
/// shows (the granular engine plan's decision 1). The grains underneath
/// are the same in every mode: a mode is a starting point and a set of
/// controls, and its settings stay when another mode is picked until that
/// mode's own are put in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GrainMode {
    /// A scattered cloud: every control.
    #[default]
    Cloud,
    /// A time stretch and freeze: grains in step reading at a speed of
    /// their own, the pitch apart.
    Stretch,
    /// Repeats of the spot under the position, locked to the tempo: a
    /// stutter.
    Repeat,
    /// The sound as an oscillator: short grains of one spot, regular, so
    /// the grain rate (the density) is the note heard, and the keys and
    /// tune move it (with the grains' size, so each keeps as many cycles);
    /// the grains' own pitch colours it.
    Tone,
}

/// What a pad hit does with a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    /// Plays to the end whatever the pad does.
    OneShot,
    /// Plays while held (looping between the loop points, if the region
    /// has them); lifting the pad starts the amp envelope's release.
    Gate,
    /// A hit starts it (looping, if the region has loop points), the next
    /// hit releases it.
    Toggle,
    /// Loops between the loop points while held; lifting the pad leaves the
    /// loop and plays the tail to the region's end.
    Loop,
    /// Plays the region as a grain cloud (the region's `grain` settings)
    /// while held; lifting the pad stops new grains and starts the amp
    /// envelope's release. Tune adds to the grain pitch, and reverse makes
    /// every grain play backwards. The source's channels play as they are
    /// (the channel pick doesn't apply).
    Granular,
}

/// What pressing harder on a held pad changes. The MPC's pads send
/// pressure (polyphonic aftertouch) for each pad; 0 is a light touch and
/// full pressure gives the whole `depth`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PressureTarget {
    #[default]
    Off,
    /// Level, up to ±12 dB.
    Level,
    /// Pitch, up to ±12 semitones (grains too).
    Pitch,
    /// Filter cutoff, up to ±4 octaves (needs the pad's filter on).
    Filter,
    /// Grains per second, up to ×16 or ÷16 (granular regions).
    GrainDensity,
    /// Grain length, up to ×8 or ÷8 (granular regions).
    GrainSize,
    /// Where grains read, moved by up to the whole region, wrapping.
    GrainPosition,
    /// Grain spray, up to ±100 %.
    GrainSpray,
}

/// A pad's pressure routing: one target and how far full pressure moves
/// it, from −1 (down) to +1 (up).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PressureRoute {
    pub target: PressureTarget,
    pub depth: f32,
}

/// How a warped region meets the project tempo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WarpMethod {
    /// The length follows the tempo and the pitch stays (a stretched copy
    /// made on a worker; until it is ready, the region repitches).
    #[default]
    Stretch,
    /// Played faster or slower like tape: the pitch moves with the tempo.
    /// The region's tune is left out, as it would pull it off the beat.
    Repitch,
}

/// The slowest and fastest tempo a warped region may say it is at.
pub const WARP_BPM: std::ops::RangeInclusive<f32> = 20.0..=999.0;

/// A hit pinned to a beat in a warped region: source frame `frame` plays
/// on `beat`, counted in beats from the region's start. Between two pins
/// the sound is stretched evenly; the region's start stays on beat 0 and
/// its end on [`Warp::beats`], so pins move the hits inside a loop without
/// changing how long it is. Pins only move hits when the region stretches
/// ([`WarpMethod::Stretch`]); repitching, it plays straight through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    /// A source frame inside the region.
    pub frame: u64,
    pub beat: f64,
}

/// The most pins a region may hold.
pub const MAX_PINS: usize = 128;
/// Between two pins a region may play this much slower, or this much
/// faster, than its own tempo.
pub const PIN_SPEED: f64 = 4.0;

/// A region that plays at the project tempo.
#[derive(Clone, Debug, PartialEq)]
pub struct Warp {
    /// The tempo the region was played at (detected or typed).
    pub bpm: f32,
    pub method: WarpMethod,
    /// Hits pinned to beats, in order of frame and of beat. Empty: the
    /// region plays evenly at its tempo.
    pub pins: Vec<Pin>,
}

impl Warp {
    pub fn new(bpm: f32) -> Warp {
        Warp {
            bpm,
            method: WarpMethod::Stretch,
            pins: Vec::new(),
        }
    }

    /// Source frames per beat at the region's own tempo.
    pub fn frames_per_beat(&self, rate: u32) -> f64 {
        60.0 * f64::from(rate) / f64::from(self.bpm)
    }

    /// How many beats `frames` of the region last at its tempo: where the
    /// region's end plays.
    pub fn beats(&self, frames: u64, rate: u32) -> f64 {
        frames as f64 / self.frames_per_beat(rate)
    }

    /// Whether the pins move anything when the region plays: it stretches
    /// and has pins.
    pub fn pinned(&self) -> bool {
        self.method == WarpMethod::Stretch && !self.pins.is_empty()
    }

    /// The points the region plays through, as (frames from the region's
    /// start, beat): its start on beat 0, each pin, its end.
    pub fn points(&self, start: u64, end: u64, rate: u32) -> Vec<(f64, f64)> {
        let len = end.saturating_sub(start);
        let mut out = Vec::with_capacity(self.pins.len() + 2);
        out.push((0.0, 0.0));
        out.extend(
            self.pins
                .iter()
                .map(|p| (p.frame.saturating_sub(start) as f64, p.beat)),
        );
        out.push((len as f64, self.beats(len, rate)));
        out
    }

    /// The beat source frame `frame` plays on. Frames before the region's
    /// start or after its end follow its tempo from there.
    pub fn beat_at(&self, start: u64, end: u64, rate: u32, frame: f64) -> f64 {
        self.line(start, end, rate).beat_at(frame)
    }

    /// The source frame that plays on `beat` (beats before 0 or after the
    /// end follow the region's tempo from there).
    pub fn frame_at(&self, start: u64, end: u64, rate: u32, beat: f64) -> f64 {
        self.line(start, end, rate).frame_at(beat)
    }

    /// The line between the region `start..end`'s frames and beats, worked
    /// out once, for looking up many places (a picture's columns).
    pub fn line(&self, start: u64, end: u64, rate: u32) -> BeatLine {
        BeatLine {
            start: start as f64,
            fpb: self.frames_per_beat(rate),
            points: self.points(start, end, rate),
        }
    }

    /// How fast the region plays between two points, against its own tempo
    /// (1: as recorded; 2: twice as fast).
    pub fn speed(&self, a: (f64, f64), b: (f64, f64), rate: u32) -> f64 {
        (b.0 - a.0) / self.frames_per_beat(rate) / (b.1 - a.1)
    }

    /// Whether the pins fit the region `start..end`: inside it, in order of
    /// frame and beat, no more than [`MAX_PINS`], and nowhere played more
    /// than [`PIN_SPEED`] times faster or slower than the region's tempo.
    pub fn pins_ok(&self, start: u64, end: u64, rate: u32) -> bool {
        if self.pins.is_empty() {
            return true;
        }
        if self.pins.len() > MAX_PINS
            || self
                .pins
                .iter()
                .any(|p| !p.beat.is_finite() || p.frame <= start || p.frame >= end)
        {
            return false;
        }
        let pts = self.points(start, end, rate);
        pts.windows(2).all(|w| {
            let s = self.speed(w[0], w[1], rate);
            w[1].0 > w[0].0 && w[1].1 > w[0].1 && (1.0 / PIN_SPEED..=PIN_SPEED).contains(&s)
        })
    }

    /// The warp for the region moved from `old` to `new` (start, end): each
    /// pin stays on its hit, its beat counted from the new start. Pins
    /// outside the new region, or that no longer fit, are dropped.
    pub fn moved(&self, old: (u64, u64), new: (u64, u64), rate: u32) -> Warp {
        let mut w = self.clone();
        if !self.pins.is_empty() && old.0 != new.0 {
            let zero = self.beat_at(old.0, old.1, rate, new.0 as f64);
            for p in &mut w.pins {
                p.beat -= zero;
            }
        }
        w.fit(new.0, new.1, rate);
        w
    }

    /// Where pin `i` may go, as (earliest, latest) beat, between its
    /// neighbours and within [`PIN_SPEED`] of the region's tempo on either
    /// side. None if there is no pin `i` or no room.
    pub fn pin_room(&self, i: usize, start: u64, end: u64, rate: u32) -> Option<(f64, f64)> {
        let p = self.pins.get(i)?;
        let pts = self.points(start, end, rate);
        let x = p.frame.saturating_sub(start) as f64;
        self.room_between(pts[i], x, pts[i + 2], rate)
    }

    /// The beats a hit `x` frames from the region's start may go on
    /// between points `a` and `b` (frames from the start, beat).
    fn room_between(&self, a: (f64, f64), x: f64, b: (f64, f64), rate: u32) -> Option<(f64, f64)> {
        if !(a.0 < x && x < b.0) {
            return None;
        }
        let fpb = self.frames_per_beat(rate);
        let (left, right) = ((x - a.0) / fpb, (b.0 - x) / fpb);
        // A hair inside, so a pin put on an edge still passes the check.
        let lo = (a.1 + left / PIN_SPEED).max(b.1 - right * PIN_SPEED) + 1e-9;
        let hi = (a.1 + left * PIN_SPEED).min(b.1 - right / PIN_SPEED) - 1e-9;
        (lo <= hi).then_some((lo, hi))
    }

    /// The warp with pin `i` moved to `beat`, or as near it as there is
    /// room for.
    pub fn pin_moved(&self, i: usize, beat: f64, start: u64, end: u64, rate: u32) -> Warp {
        let mut w = self.clone();
        if let Some((lo, hi)) = self.pin_room(i, start, end, rate) {
            w.pins[i].beat = beat.clamp(lo, hi);
        }
        w
    }

    /// The warp with a new pin on source frame `frame`, on `beat` or as
    /// near it as there is room for, and the pin's place among the pins.
    /// None if the frame is outside the region, already pinned, there is no
    /// room, or there are [`MAX_PINS`] already.
    pub fn pin_added(
        &self,
        frame: u64,
        beat: f64,
        start: u64,
        end: u64,
        rate: u32,
    ) -> Option<(Warp, usize)> {
        if self.pins.len() >= MAX_PINS || self.pins.iter().any(|p| p.frame == frame) {
            return None;
        }
        let i = self.pins.partition_point(|p| p.frame < frame);
        let pts = self.points(start, end, rate);
        let x = frame.checked_sub(start)? as f64;
        let (lo, hi) = self.room_between(pts[i], x, pts[i + 1], rate)?;
        let mut w = self.clone();
        w.pins.insert(
            i,
            Pin {
                frame,
                beat: beat.clamp(lo, hi),
            },
        );
        Some((w, i))
    }

    /// Pins each of `hits` (source frames) that the region plays within
    /// `reach` of a grid line, every `grid` beats, onto that line (the
    /// nearest hit, where two reach one line). The pins there were go.
    /// `reach` is in beats.
    pub fn pinned_to_grid(
        &self,
        hits: &[u64],
        grid: f64,
        reach: f64,
        start: u64,
        end: u64,
        rate: u32,
    ) -> Warp {
        let mut w = Warp {
            pins: Vec::new(),
            ..self.clone()
        };
        if grid.is_nan() || grid <= 0.0 {
            return w;
        }
        let total = w.beats(end.saturating_sub(start), rate);
        let fpb = w.frames_per_beat(rate);
        // (line, hit, how far off), the nearest hit for each line.
        let mut best: Vec<(i64, u64, f64)> = Vec::new();
        for &h in hits {
            if h <= start || h >= end {
                continue;
            }
            let b = (h - start) as f64 / fpb;
            let line = (b / grid).round() as i64;
            let g = line as f64 * grid;
            let off = (b - g).abs();
            if off > reach || g <= 0.0 || g >= total {
                continue;
            }
            match best.iter_mut().find(|e| e.0 == line) {
                Some(e) if off < e.2 => *e = (line, h, off),
                Some(_) => {}
                None => best.push((line, h, off)),
            }
        }
        best.sort_by_key(|e| e.0);
        w.pins = best
            .iter()
            .map(|&(line, frame, _)| Pin {
                frame,
                beat: line as f64 * grid,
            })
            .filter(|p| p.frame > start)
            .collect();
        // Hits out of order (two lines' hits crossed) go, then any that
        // don't fit.
        let mut last = 0;
        w.pins.retain(|p| {
            let ok = p.frame > last;
            if ok {
                last = p.frame;
            }
            ok
        });
        w.fit(start, end, rate);
        w
    }

    /// Drops the pins that don't fit the region `start..end` (see
    /// [`Warp::pins_ok`]), keeping as many as it can from the start.
    pub fn fit(&mut self, start: u64, end: u64, rate: u32) {
        if self.pins_ok(start, end, rate) {
            return;
        }
        let total = self.beats(end.saturating_sub(start), rate);
        let ok = |w: &Warp, a: (f64, f64), b: (f64, f64)| {
            b.0 > a.0 && b.1 > a.1 && (1.0 / PIN_SPEED..=PIN_SPEED).contains(&w.speed(a, b, rate))
        };
        let mut kept: Vec<Pin> = Vec::new();
        let mut last = (start as f64, 0.0);
        for p in &self.pins {
            let at = (p.frame as f64, p.beat);
            if kept.len() < MAX_PINS
                && p.beat.is_finite()
                && p.frame > start
                && p.frame < end
                && p.beat < total
                && ok(self, last, at)
            {
                kept.push(*p);
                last = at;
            }
        }
        let tail = (end as f64, total);
        while let Some(p) = kept.last() {
            if ok(self, (p.frame as f64, p.beat), tail) {
                break;
            }
            kept.pop();
        }
        self.pins = kept;
    }
}

/// A warped region's line between source frames and beats (see
/// [`Warp::line`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BeatLine {
    start: f64,
    fpb: f64,
    points: Vec<(f64, f64)>,
}

impl BeatLine {
    /// The beat source frame `frame` plays on.
    pub fn beat_at(&self, frame: f64) -> f64 {
        through(&self.points, frame - self.start, |p| p, 1.0 / self.fpb)
    }

    /// The source frame that plays on `beat`.
    pub fn frame_at(&self, beat: f64) -> f64 {
        self.start + through(&self.points, beat, |(f, b)| (b, f), self.fpb)
    }
}

/// Follows the line through `pts` (each turned into (x, y) by `xy`, both
/// rising) at `x`; past either end, on with slope `slope`.
pub(crate) fn through(
    pts: &[(f64, f64)],
    x: f64,
    xy: impl Fn((f64, f64)) -> (f64, f64),
    slope: f64,
) -> f64 {
    let (first, last) = (xy(pts[0]), xy(pts[pts.len() - 1]));
    if x <= first.0 {
        return first.1 + (x - first.0) * slope;
    }
    if x >= last.0 {
        return last.1 + (x - last.0) * slope;
    }
    // The first point at or past x: x lies between it and the one before.
    let i = pts.partition_point(|p| xy(*p).0 < x).max(1);
    let (a, b) = (xy(pts[i - 1]), xy(pts[i]));
    if b.0 > a.0 {
        a.1 + (x - a.0) / (b.0 - a.0) * (b.1 - a.1)
    } else {
        b.1
    }
}

/// A recipe on part of a source. The source's audio is never changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub source: SourceId,
    pub start: u64,
    pub end: u64,
    pub sustain_loop: Option<Loop>,
    pub fade_in: Fade,
    pub fade_out: Fade,
    pub gain_db: f32,
    pub reverse: bool,
    pub channels: ChannelPick,
    pub tune_cents: i32,
    pub mode: PlayMode,
    /// Grain settings, used when `mode` is [`PlayMode::Granular`]. Kept
    /// with the region in every mode, so switching modes loses nothing.
    pub grain: GrainParams,
    /// Which controls the Grain page shows for `grain`, and where it
    /// started them.
    pub grain_mode: GrainMode,
    /// Plays at the project tempo (not in granular mode, whose grains have
    /// their own tempo sync).
    pub warp: Option<Warp>,
    /// A sliced region's slice boundaries, in source frames, sorted (at
    /// most [`MAX_SLICES`] - 1): slice i runs from boundary i to boundary
    /// i + 1, where the boundaries are its start, these, then its end. A
    /// hit plays one slice (the pad's [`Pad::slice`], or the note's on
    /// keys). Empty: not sliced. A boundary outside the region (after a
    /// trim) is passed over ([`Region::slice_bounds`]).
    pub slices: Vec<u64>,
    /// A sliced region's settings for each slice, by slice number (R4
    /// part 5): a slice past the list plays with [`SliceSet::DEFAULT`].
    /// Kept shorter than [`MAX_SLICES`] and without default entries at
    /// its end ([`Region::set_slice_set`]).
    pub slice_sets: Vec<SliceSet>,
}

/// One slice's own settings on a sliced region (R4 part 5), on top of
/// its pad's: the Play page's Slices section sets them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliceSet {
    /// Semitones from the pad's pitch, [`SLICE_TUNE`].
    pub tune_st: i8,
    /// Its level, dB, [`SLICE_LEVEL_DB`].
    pub level_db: f32,
    /// Plays backwards (from its end), or forwards on a reversed region.
    pub reverse: bool,
    /// A low-pass cutoff (Hz, [`SLICE_CUTOFF_HZ`]) in place of the pad's
    /// filter's for this slice; None: the pad's filter as it is.
    pub cutoff_hz: Option<f32>,
    /// Its fade out, ms ([`SLICE_FADE_MS`]), at most half of what it
    /// plays; None: the region's.
    pub fade_out_ms: Option<f32>,
    /// Plays on past its end to the region's end ("Plays to: Sound end").
    pub to_end: bool,
}

impl SliceSet {
    /// A slice as its region plays it.
    pub const DEFAULT: SliceSet = SliceSet {
        tune_st: 0,
        level_db: 0.0,
        reverse: false,
        cutoff_hz: None,
        fade_out_ms: None,
        to_end: false,
    };

    /// Every number is finite and in its range.
    pub fn ok(&self) -> bool {
        SLICE_TUNE.contains(&self.tune_st)
            && SLICE_LEVEL_DB.contains(&self.level_db)
            && self
                .cutoff_hz
                .is_none_or(|hz| SLICE_CUTOFF_HZ.contains(&hz))
            && self
                .fade_out_ms
                .is_none_or(|ms| SLICE_FADE_MS.contains(&ms))
    }
}

impl Default for SliceSet {
    fn default() -> Self {
        SliceSet::DEFAULT
    }
}

/// A slice's pitch, semitones.
pub const SLICE_TUNE: std::ops::RangeInclusive<i8> = -24..=24;
/// A slice's level, dB.
pub const SLICE_LEVEL_DB: std::ops::RangeInclusive<f32> = -48.0..=12.0;
/// A slice's own low-pass cutoff, Hz.
pub const SLICE_CUTOFF_HZ: std::ops::RangeInclusive<f32> = 20.0..=20_000.0;
/// A slice's own fade out, ms.
pub const SLICE_FADE_MS: std::ops::RangeInclusive<f32> = 0.0..=2000.0;

impl Region {
    /// Moves the region onto its sound converted to another rate (`r`),
    /// keeping every point at the same moment: start and end, the loop
    /// and its crossfade, the fades and the warp pins. Rounding never
    /// leaves it unplayable: points are kept inside the region and the
    /// crossfade and fades inside what holds them; a pin that no longer
    /// fits is dropped. A region on another sound is left alone.
    pub fn rescale(&mut self, r: &Rescale) {
        if self.source != r.from || r.frames == 0 {
            return;
        }
        self.source = r.to;
        self.start = r.frame(self.start).min(r.frames - 1);
        self.end = r.frame(self.end).clamp(self.start + 1, r.frames);
        if let Some(l) = &mut self.sustain_loop {
            l.start = r.frame(l.start).clamp(self.start, self.end - 1);
            l.end = r.frame(l.end).clamp(l.start + 1, self.end);
            let room = ((l.end - l.start) / 2).min(l.start - self.start);
            l.crossfade = u64::from(r.length(l.crossfade)).min(room) as u32;
        }
        // Slices at the same moments, still in order and inside the sound.
        for x in &mut self.slices {
            *x = r.frame(*x);
        }
        self.slices.retain(|&x| x > 0 && x < r.frames);
        self.slices.dedup();
        let len = self.frames();
        self.fade_in.frames = r.length(self.fade_in.frames);
        self.fade_out.frames = r.length(self.fade_out.frames);
        let over =
            (u64::from(self.fade_in.frames) + u64::from(self.fade_out.frames)).saturating_sub(len);
        let cut = over.min(u64::from(self.fade_out.frames));
        self.fade_out.frames -= cut as u32;
        self.fade_in.frames -= (over - cut) as u32;
        if let Some(w) = &mut self.warp {
            let (a, b) = (self.start, self.end);
            for p in &mut w.pins {
                p.frame = r.frame(p.frame);
            }
            w.pins.retain(|p| p.frame > a && p.frame < b);
            w.pins.dedup_by_key(|p| p.frame);
            if !w.pins_ok(a, b, r.rate) {
                w.pins.clear();
            }
        }
    }

    /// All of a source, one-shot, untouched.
    pub fn whole(source: &Source) -> Region {
        Region {
            source: source.id(),
            start: 0,
            end: source.frames(),
            sustain_loop: None,
            fade_in: Fade::NONE,
            fade_out: Fade::NONE,
            gain_db: 0.0,
            reverse: false,
            channels: ChannelPick::Stereo,
            tune_cents: 0,
            mode: PlayMode::OneShot,
            grain: GrainParams::default(),
            grain_mode: GrainMode::Cloud,
            warp: None,
            slices: Vec::new(),
            slice_sets: Vec::new(),
        }
    }

    pub fn frames(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// The loudest frame between its start and end on the channels it
    /// plays, from `source` (its sound): the larger |sample| of the two
    /// sides in Stereo, one side's for Left or Right, |(left + right) / 2|
    /// for Mono sum. Before its gain, fades and envelope; 0 for silence or
    /// a region outside the sound. What Normalize brings to 0 dBFS.
    pub fn peak(&self, source: &Source) -> f32 {
        let ch = usize::from(source.channels()).max(1);
        let (start, end) = (
            self.start as usize,
            (self.end as usize).min(source.frames() as usize),
        );
        let Some(frames) = source.data().f32s(start * ch..end.max(start) * ch) else {
            return 0.0;
        };
        let pick = if ch == 2 {
            self.channels
        } else {
            ChannelPick::Left
        };
        frames
            .chunks_exact(ch)
            .map(|f| match pick {
                ChannelPick::Stereo => f[0].abs().max(f[1].abs()),
                ChannelPick::MonoSum => (0.5 * (f[0] + f[1])).abs(),
                ChannelPick::Left => f[0].abs(),
                ChannelPick::Right => f[1].abs(),
            })
            .fold(0.0, f32::max)
    }

    /// Its slice boundaries inside it (the ones a trim left outside are
    /// passed over).
    pub fn cuts(&self) -> impl Iterator<Item = u64> + '_ {
        let (a, b) = (self.start, self.end);
        self.slices.iter().copied().filter(move |&x| x > a && x < b)
    }

    /// How many slices it plays as: 1 when it isn't sliced.
    pub fn slice_count(&self) -> usize {
        self.cuts().count() + 1
    }

    /// Slice `i`'s start and end (slice 0 of a region that isn't sliced is
    /// the whole of it); None past the last.
    pub fn slice_bounds(&self, i: usize) -> Option<(u64, u64)> {
        let mut edges = std::iter::once(self.start)
            .chain(self.cuts())
            .chain(std::iter::once(self.end));
        let a = edges.nth(i)?;
        let b = edges.next()?;
        Some((a, b))
    }

    /// Slice `i`'s own settings ([`SliceSet::DEFAULT`] when it has none).
    pub fn slice_set(&self, i: usize) -> SliceSet {
        self.slice_sets.get(i).copied().unwrap_or_default()
    }

    /// Gives slice `i` settings `s` (from 0, below [`MAX_SLICES`]), keeping
    /// the list without default entries at its end. A slice past the
    /// region's last keeps them, for a re-chop that brings it back.
    pub fn set_slice_set(&mut self, i: usize, s: SliceSet) {
        if i >= MAX_SLICES {
            return;
        }
        if self.slice_sets.len() <= i {
            self.slice_sets.resize(i + 1, SliceSet::DEFAULT);
        }
        self.slice_sets[i] = s;
        while self.slice_sets.last() == Some(&SliceSet::DEFAULT) {
            self.slice_sets.pop();
        }
    }

    pub fn check(&self, source: &Source) -> Result<(), ModelError> {
        if self.source != source.id() {
            return Err(ModelError::UnknownSource);
        }
        if self.slices.len() >= MAX_SLICES
            || self.slices.windows(2).any(|w| w[0] >= w[1])
            || self.slices.iter().any(|&x| x == 0 || x >= source.frames())
        {
            return Err(ModelError::BadRegion("slices"));
        }
        if self.slice_sets.len() > MAX_SLICES || !self.slice_sets.iter().all(SliceSet::ok) {
            return Err(ModelError::BadRegion("slice settings"));
        }
        if self.start >= self.end || self.end > source.frames() {
            return Err(ModelError::BadRegion("start/end"));
        }
        if let Some(l) = self.sustain_loop {
            if l.start < self.start || l.end > self.end || l.end <= l.start {
                return Err(ModelError::BadRegion("loop points"));
            }
            if u64::from(l.crossfade) > (l.end - l.start) / 2
                || u64::from(l.crossfade) > l.start - self.start
            {
                return Err(ModelError::BadRegion("loop crossfade"));
            }
        }
        if self.mode == PlayMode::Loop && self.sustain_loop.is_none() {
            return Err(ModelError::BadRegion("loop mode needs loop points"));
        }
        if u64::from(self.fade_in.frames) + u64::from(self.fade_out.frames) > self.frames() {
            return Err(ModelError::BadRegion("fades longer than the region"));
        }
        if !self.gain_db.is_finite() || self.tune_cents.abs() > 4800 {
            return Err(ModelError::BadRegion("gain/tune"));
        }
        if !grain_ok(&self.grain) {
            return Err(ModelError::BadRegion("grain settings"));
        }
        if let Some(w) = &self.warp {
            if !WARP_BPM.contains(&w.bpm) {
                return Err(ModelError::BadRegion("warp tempo"));
            }
            if !w.pins_ok(self.start, self.end, source.rate()) {
                return Err(ModelError::BadRegion("warp pins"));
            }
        }
        Ok(())
    }
}

/// Grain settings are usable when every number in them is finite.
fn grain_ok(g: &GrainParams) -> bool {
    let numbers = [
        g.position,
        g.spray,
        g.size_ms,
        g.density,
        g.pitch,
        g.pitch_spray,
        g.scan,
        g.reverse,
        g.spread,
        g.gain,
        g.lfo.rate_hz,
        g.lfo.depth,
        g.env.attack_ms,
        g.env.decay_ms,
        g.env.sustain,
        g.env.depth,
    ];
    if g.lfo.beats.is_some_and(|b| !b.is_finite()) {
        return false;
    }
    let sync_ok = g.sync.is_none_or(|t| {
        t.frames_per_beat.is_finite()
            && t.grains_per_beat.is_none_or(f32::is_finite)
            && t.size_beats.is_none_or(f32::is_finite)
    });
    numbers.iter().all(|x| x.is_finite()) && sync_ok
}

/// The granular effect on everything the sampler plays (its master output),
/// in the spirit of Clouds or the S-4's Mosaic: grains from the last few
/// seconds of the output, mixed back over it. Freezing it is something you
/// play, not a setting ([`crate::engine::Event::Freeze`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterFx {
    /// From 0 (only the dry output, and no grains) to 1 (only grains),
    /// equal power.
    pub mix: f32,
    /// The grains. `position` 0 is the oldest audio the effect remembers
    /// ([`crate::engine::MASTER_MEMORY_S`] seconds back) and 1 the newest;
    /// `scan` 0 keeps reading the same distance behind the live output.
    pub grain: GrainParams,
}

impl Default for MasterFx {
    /// Fully dry, with a cloud about a second behind the output ready for
    /// when the mix comes up.
    fn default() -> Self {
        MasterFx {
            mix: 0.0,
            grain: GrainParams {
                position: 0.85,
                spray: 0.1,
                size_ms: 120.0,
                density: 20.0,
                pitch_spray: 0.1,
                spread: 0.6,
                ..GrainParams::default()
            },
        }
    }
}

impl MasterFx {
    pub fn check(&self) -> Result<(), ModelError> {
        if !(0.0..=1.0).contains(&self.mix) || !grain_ok(&self.grain) {
            return Err(ModelError::BadMasterFx);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChopMode {
    Transient,
    Grid,
    Beat,
    Manual,
    Silence,
    /// Where the notes change (R4.7).
    Pitch,
}

/// The name a chop kept for its sound goes by ([`Project::keep_chop`]).
pub const KEPT_CHOP: &str = "Kept";

/// A named list of slice points on a source. A source can have several, so
/// re-chopping never loses an earlier chop.
#[derive(Clone, Debug, PartialEq)]
pub struct ChopSet {
    pub name: String,
    pub source: SourceId,
    pub mode: ChopMode,
    /// Slice starts, sorted, strictly inside the source; slice i runs from
    /// boundary i to boundary i+1, where the boundaries are 0, markers..., end.
    markers: Vec<u64>,
    frames: u64,
}

impl ChopSet {
    pub fn new(name: &str, source: &Source, mode: ChopMode) -> ChopSet {
        ChopSet {
            name: name.to_string(),
            source: source.id(),
            mode,
            markers: Vec::new(),
            frames: source.frames(),
        }
    }

    /// `n` equal slices.
    pub fn grid(name: &str, source: &Source, n: usize) -> Result<ChopSet, ModelError> {
        if n == 0 || n > MAX_SLICES || n as u64 > source.frames() {
            return Err(ModelError::TooManySlices);
        }
        let mut c = ChopSet::new(name, source, ChopMode::Grid);
        for i in 1..n {
            c.markers.push(source.frames() * i as u64 / n as u64);
        }
        Ok(c)
    }

    /// A chop from any list of marker frames: sorted, duplicates and points
    /// outside the source dropped (frame 0 is always a boundary already).
    pub fn from_markers(
        name: &str,
        source: &Source,
        mode: ChopMode,
        markers: impl IntoIterator<Item = u64>,
    ) -> Result<ChopSet, ModelError> {
        let mut c = ChopSet::new(name, source, mode);
        c.markers = markers
            .into_iter()
            .filter(|&m| m > 0 && m < c.frames)
            .collect();
        c.markers.sort_unstable();
        c.markers.dedup();
        if c.markers.len() + 1 > MAX_SLICES {
            return Err(ModelError::TooManySlices);
        }
        Ok(c)
    }

    /// Slices of `beats_per_slice` beats at `bpm`, with the first beat at
    /// `first_beat` (a marker goes there too when it isn't 0).
    pub fn beats(
        name: &str,
        source: &Source,
        bpm: f64,
        beats_per_slice: f64,
        first_beat: u64,
    ) -> Result<ChopSet, ModelError> {
        if !(bpm.is_finite() && bpm > 0.0 && beats_per_slice.is_finite() && beats_per_slice > 0.0) {
            return Err(ModelError::BadSlice);
        }
        let step = 60.0 / bpm * beats_per_slice * f64::from(source.rate());
        if step < 1.0 || (source.frames() as f64 / step) as usize >= MAX_SLICES {
            return Err(ModelError::TooManySlices);
        }
        let first = first_beat as f64;
        let count = ((source.frames() as f64 - first) / step).ceil().max(0.0) as usize;
        ChopSet::from_markers(
            name,
            source,
            ChopMode::Beat,
            (0..count).map(|k| (first + k as f64 * step).round() as u64),
        )
    }

    pub fn markers(&self) -> &[u64] {
        &self.markers
    }

    /// Moves the slices onto their sound converted to another rate (`r`):
    /// each marker at the same moment. A chop of another sound is left
    /// alone.
    pub fn rescale(&mut self, r: &Rescale) {
        if self.source != r.from {
            return;
        }
        self.source = r.to;
        self.frames = r.frames;
        for m in &mut self.markers {
            *m = r.frame(*m);
        }
        let n = self.frames;
        self.markers.retain(|&m| m > 0 && m < n);
        self.markers.dedup();
    }
    pub fn slices(&self) -> usize {
        self.markers.len() + 1
    }

    /// Adds a marker (ignored if one is already there). Returns the new slice count.
    pub fn add_marker(&mut self, at: u64) -> Result<usize, ModelError> {
        if at == 0 || at >= self.frames {
            return Err(ModelError::BadSlice);
        }
        match self.markers.binary_search(&at) {
            Ok(_) => {}
            Err(i) => {
                if self.markers.len() + 1 >= MAX_SLICES {
                    return Err(ModelError::TooManySlices);
                }
                self.markers.insert(i, at);
            }
        }
        Ok(self.slices())
    }

    /// Joins slice `i` with the next one.
    pub fn merge(&mut self, i: usize) -> Result<(), ModelError> {
        if i >= self.markers.len() {
            return Err(ModelError::BadSlice);
        }
        self.markers.remove(i);
        Ok(())
    }

    /// Moves the boundary between slice `i` and slice `i+1`, keeping order.
    pub fn move_marker(&mut self, i: usize, to: u64) -> Result<(), ModelError> {
        if i >= self.markers.len() {
            return Err(ModelError::BadSlice);
        }
        let lo = if i == 0 { 0 } else { self.markers[i - 1] };
        let hi = self.markers.get(i + 1).copied().unwrap_or(self.frames);
        if to <= lo || to >= hi {
            return Err(ModelError::BadSlice);
        }
        self.markers[i] = to;
        Ok(())
    }

    /// (start, end) of slice `i`.
    pub fn bounds(&self, i: usize) -> Option<(u64, u64)> {
        if i >= self.slices() {
            return None;
        }
        let start = if i == 0 { 0 } else { self.markers[i - 1] };
        let end = self.markers.get(i).copied().unwrap_or(self.frames);
        Some((start, end))
    }

    /// Slice `i` as a one-shot region.
    pub fn region(&self, source: &Source, i: usize) -> Result<Region, ModelError> {
        if source.id() != self.source {
            return Err(ModelError::UnknownSource);
        }
        let (start, end) = self.bounds(i).ok_or(ModelError::BadSlice)?;
        Ok(Region {
            start,
            end,
            ..Region::whole(source)
        })
    }

    /// The whole sound as one sliced region (R4, Chop to one pad): slice
    /// i plays as [`ChopSet::region`] for slice i does. More than
    /// [`MAX_SLICES`] slices don't fit on one pad.
    pub fn sliced(&self, source: &Source) -> Result<Region, ModelError> {
        self.sliced_part(source, 0, self.slices())
    }

    /// Slices `first..first+count` as one sliced region, from the first's
    /// start to the last's end: its slice k plays as [`ChopSet::region`]
    /// for slice `first + k` does.
    pub fn sliced_part(
        &self,
        source: &Source,
        first: usize,
        count: usize,
    ) -> Result<Region, ModelError> {
        if source.id() != self.source {
            return Err(ModelError::UnknownSource);
        }
        if count == 0 || count > MAX_SLICES {
            return Err(ModelError::BadSlice);
        }
        let (start, _) = self.bounds(first).ok_or(ModelError::BadSlice)?;
        let (_, end) = self.bounds(first + count - 1).ok_or(ModelError::BadSlice)?;
        Ok(Region {
            start,
            end,
            slices: self.markers[first..first + count - 1].to_vec(),
            ..Region::whole(source)
        })
    }
}

/// A pad playing `region` (a slice, or a sliced region), named `name`:
/// `template` (a pad's level, envelopes, filter and so on) with one layer,
/// keeping the template's first region's gain, tune and channels; with no
/// template, a new pad.
fn slice_pad(name: String, mut region: Region, template: Option<&Pad>) -> Pad {
    match template {
        Some(t) => {
            if let Some(r) = t.layers.first().map(|l| &l.region) {
                (region.gain_db, region.tune_cents, region.channels) =
                    (r.gain_db, r.tune_cents, r.channels);
                // Chopped again onto itself, a sliced pad keeps its slices'
                // settings, by number (R4 part 5).
                if region.slice_count() > 1 && r.source == region.source {
                    region.slice_sets = r.slice_sets.clone();
                }
            }
            Pad {
                name,
                layers: vec![Layer::new(region)],
                // A slice is a pad; chop to keys gives it its note.
                keys: None,
                slice: 0,
                ..t.clone()
            }
        }
        None => Pad::new(&name, Layer::new(region)),
    }
}

/// How a pad chooses among layers whose velocity range contains the hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerSelect {
    /// Every matching layer plays (stacked).
    All,
    /// One matching layer per hit, in turn (round robin).
    Cycle,
    /// One matching layer per hit, at random.
    Random,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub region: Region,
    pub vel_lo: u8,
    pub vel_hi: u8,
    pub level_db: f32,
    /// -1 left .. +1 right.
    pub pan: f32,
    pub tune_cents: i32,
}

impl Layer {
    pub fn new(region: Region) -> Layer {
        Layer {
            region,
            vel_lo: 1,
            vel_hi: 127,
            level_db: 0.0,
            pan: 0.0,
            tune_cents: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    pub name: String,
    pub layers: Vec<Layer>,
    pub select: LayerSelect,
    /// Voices this pad may hold at once (1 = mono). A new hit past the limit
    /// fades out this pad's oldest voice.
    pub polyphony: u8,
    /// Pads in the same mute group (1..=16) stop each other.
    pub mute_group: Option<u8>,
    /// Pads this one stops when hit (one-way choke: the closed hat stops the open hat).
    pub chokes: Vec<usize>,
    /// Pads a hit on this one also plays, at the same velocity (one way,
    /// as the MPC's Simultaneous Play): each linked pad does what a hit on
    /// it does, with its own layers, chokes and latch, and lets go when
    /// this pad does. A linked pad's own links are not followed.
    pub links: Vec<usize>,
    /// Latch: a hit starts the pad and lifting it lets nothing go; the
    /// next hit lets it go (a loop plays its tail, grains stop, a held
    /// sound releases). For loops, grain clouds and drones in any play mode.
    pub latch: bool,
    pub amp: Ahdsr,
    pub filter: Filter,
    pub filter_env: Ahdsr,
    /// 0 = every hit at full level, 1 = level follows velocity fully.
    pub vel_to_level: f32,
    /// Fraction of the region skipped at the softest hit (0 = always from the start).
    pub vel_to_start: f32,
    pub level_db: f32,
    pub pan: f32,
    pub tune_cents: i32,
    /// What pressure on this pad changes.
    pub pressure: PressureRoute,
    /// As a keygroup: the notes it plays (see [`Keys`]). A pad without
    /// keys only plays when it is hit.
    pub keys: Option<Keys>,
    /// Wow and flutter, 0 to 1: the pitch wanders as on a worn tape
    /// machine (at 1, slow wow about ±25 cents and flutter about ±5).
    pub wow: f32,
    /// As a keygroup: the notes (0 to [`MAX_KEY_FADE`]) over which it fades
    /// in at its low key and out at its high key, equal power, so two
    /// zones overlapping by that many notes crossfade. No fade at the ends
    /// of the keyboard, or for a hit on the pad itself.
    pub key_fade: u8,
    /// The velocities (0 to [`MAX_VEL_FADE`]) over which each layer fades
    /// in at its lowest velocity and out at its highest, equal power, so
    /// two layers overlapping by that much crossfade. No fade at 1 or 127.
    /// In Cycle and Random, layers with the same velocities take turns
    /// among themselves and the overlapping ones play together.
    pub vel_fade: u8,
    /// Vintage: the pad pitches its sound as older samplers do, read
    /// straight with no low-pass, so pitching it up aliases (grit on
    /// purpose). Off, a pad pitched up is read band-limited
    /// ([`crate::bandlimit`]) and stays clean.
    pub vintage: bool,
    /// On a sliced layer ([`Region::slices`]), the slice a hit on the pad
    /// plays, from 0 (below [`MAX_SLICES`]). A step lock or a knob on
    /// [`crate::engine::PadField::Slice`] picks another for a hit; a note
    /// on keys plays its own slice.
    pub slice: u8,
}

/// The longest key fade, in notes.
pub const MAX_KEY_FADE: u8 = 24;
/// The longest velocity fade.
pub const MAX_VEL_FADE: u8 = 64;

/// Moves the edges where neighbouring ranges meet so each pair overlaps
/// by `fade` steps, centred on where they met: what makes a fade a
/// crossfade. `ranges` are distinct and sorted by their low end. A pair
/// that doesn't meet (a gap between them) or where one holds the other (a
/// stack) is left alone; an edge never passes its neighbour's far end.
/// Running it again with another fade moves the same edges again.
pub fn overlap_ranges(ranges: &mut [(u8, u8)], fade: u8) {
    for i in 1..ranges.len() {
        let ((a_lo, a_hi), (b_lo, b_hi)) = (ranges[i - 1], ranges[i]);
        let (a_lo, a_hi, b_lo, b_hi) = (
            i32::from(a_lo),
            i32::from(a_hi),
            i32::from(b_lo),
            i32::from(b_hi),
        );
        if !(a_lo < b_lo && a_hi < b_hi) || b_lo > a_hi + 1 {
            continue;
        }
        // Where they meet: the last step of the lower one before any overlap.
        let o = a_hi + 1 - b_lo;
        let meet = (a_hi + b_lo - 1 - o % 2) / 2;
        let f = i32::from(fade);
        let hi = (meet + (f + 1) / 2).clamp(a_lo, b_hi - 1);
        let lo = (meet + 1 - f / 2).clamp(a_lo + 1, b_hi);
        ranges[i - 1].1 = hi as u8;
        ranges[i].0 = lo as u8;
    }
}

/// The level, 0 to 1, of something that plays from `lo` to `hi` at `x`
/// with a fade of `fade` steps inside each edge: it rises over the first
/// `fade` steps from `lo` and falls over the last `fade` before `hi`, on
/// an equal-power curve, so a neighbour overlapping by `fade` steps and
/// fading the other way sums to the same power. An edge at `floor` or
/// `ceil` (the end of the range) does not fade. 0 outside `lo..=hi`.
pub fn edge_gain(x: u8, lo: u8, hi: u8, fade: u8, floor: u8, ceil: u8) -> f32 {
    if x < lo || x > hi {
        return 0.0;
    }
    if fade == 0 {
        return 1.0;
    }
    let ramp = |steps_in: u8| {
        let t = f32::from(steps_in) / (f32::from(fade) + 1.0);
        (t * std::f32::consts::FRAC_PI_2).sin()
    };
    let mut g = 1.0;
    if lo > floor && u16::from(x) < u16::from(lo) + u16::from(fade) {
        g *= ramp(x - lo + 1);
    }
    if hi < ceil && u16::from(x) + u16::from(fade) > u16::from(hi) {
        g *= ramp(hi - x + 1);
    }
    g
}

/// A keygroup's notes: from `lo` to `hi` (MIDI notes), its sound played
/// as recorded at `root` and a semitone higher for each note above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    pub lo: u8,
    pub hi: u8,
    pub root: u8,
}

impl Keys {
    /// One note, played as recorded.
    pub fn one(note: u8) -> Keys {
        Keys {
            lo: note,
            hi: note,
            root: note,
        }
    }

    /// The whole keyboard, played as recorded at `root`.
    pub fn all(root: u8) -> Keys {
        Keys {
            lo: 0,
            hi: 127,
            root,
        }
    }

    pub fn contains(&self, note: u8) -> bool {
        (self.lo..=self.hi).contains(&note)
    }
}

/// What the kit's pads play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadsPlay {
    /// Each pad its own sound: a drum kit.
    #[default]
    Pads,
    /// Notes: pad 1 plays note `from`, each pad after it a semitone
    /// higher, through the kit's keygroups (pads with [`Keys`]).
    Keys { from: u8 },
    /// 16 Levels: every pad of a bank plays pad `pad`, pad 1 to 16 at
    /// sixteen steps of `by` (pad 1 the lowest).
    Levels { pad: u8, by: Levels },
}

/// What 16 Levels spreads across the pads. Pad `k` (0 to 15) plays at the
/// step [`Levels::velocity`], [`Levels::semitones`] and so on give it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Levels {
    /// Pad 1 the softest (velocity 8) to pad 16 the hardest (127).
    #[default]
    Velocity,
    /// A semitone a pad: pad 9 as recorded, pad 1 eight below, pad 16
    /// seven above.
    Tune,
    /// The filter's cutoff, 100 Hz to 20 kHz evenly in octaves (a 2-pole
    /// low-pass when the pad's filter is off).
    Cutoff,
    /// The amp envelope's decay, 25 ms to about 4.5 s, falling to
    /// silence.
    Decay,
    /// Where the sound starts, pad `k` at `k / 16` of the way through (a
    /// granular pad's grain position, `k / 15`).
    Start,
}

impl Levels {
    pub const ALL: [Levels; 5] = [
        Levels::Velocity,
        Levels::Tune,
        Levels::Cutoff,
        Levels::Decay,
        Levels::Start,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Levels::Velocity => "Velocity",
            Levels::Tune => "Tune",
            Levels::Cutoff => "Cutoff",
            Levels::Decay => "Decay",
            Levels::Start => "Start",
        }
    }

    /// Pad `k`'s velocity: 8, 16, ... 127.
    pub fn velocity(k: usize) -> u8 {
        ((k.min(15) as u32 + 1) * 127).div_ceil(16) as u8
    }

    /// Pad `k`'s semitones from the sound as recorded: -8 to +7.
    pub fn semitones(k: usize) -> i32 {
        k.min(15) as i32 - 8
    }

    /// Pad `k`'s cutoff: 100 Hz to 20 kHz.
    pub fn cutoff_hz(k: usize) -> f32 {
        100.0 * 200f32.powf(k.min(15) as f32 / 15.0)
    }

    /// Pad `k`'s decay: 25 ms doubling every two pads.
    pub fn decay_ms(k: usize) -> f32 {
        25.0 * (k.min(15) as f32 / 2.0).exp2()
    }

    /// Pad `k`'s start, 0 to 15/16 of the region.
    pub fn start(k: usize) -> f32 {
        k.min(15) as f32 / 16.0
    }
}

/// Where slices go on the keyboard by default: from note 36 (C2 as
/// `note_name` writes it; C1 on keyboards that call middle C C3), as most
/// samplers' slices to notes do.
pub const SLICE_KEYS_FROM: u8 = 36;
/// Voices a keygroup spread over the keyboard may hold.
pub const KEYGROUP_POLYPHONY: u8 = 16;

impl Pad {
    /// [`Region::rescale`] on every layer.
    pub fn rescale(&mut self, r: &Rescale) {
        for l in &mut self.layers {
            l.region.rescale(r);
        }
    }

    pub fn new(name: &str, layer: Layer) -> Pad {
        Pad {
            name: name.to_string(),
            layers: vec![layer],
            select: LayerSelect::All,
            polyphony: 8,
            mute_group: None,
            chokes: Vec::new(),
            links: Vec::new(),
            latch: false,
            amp: Ahdsr::OPEN,
            filter: Filter::default(),
            filter_env: Ahdsr::OPEN,
            vel_to_level: 1.0,
            vel_to_start: 0.0,
            level_db: 0.0,
            pan: 0.0,
            tune_cents: 0,
            pressure: PressureRoute::default(),
            keys: None,
            wow: 0.0,
            key_fade: 0,
            vel_fade: 0,
            vintage: false,
            slice: 0,
        }
    }

    /// The velocity fade set to `fade` (at most [`MAX_VEL_FADE`]), and the
    /// layers that meet moved to overlap by it ([`overlap_ranges`]), so
    /// they crossfade. Round robins (layers with the same velocities) move
    /// together.
    pub fn set_vel_fade(&mut self, fade: u8) {
        let fade = fade.min(MAX_VEL_FADE);
        let mut ranges: Vec<(u8, u8)> = self.layers.iter().map(|l| (l.vel_lo, l.vel_hi)).collect();
        ranges.sort_unstable();
        ranges.dedup();
        let before = ranges.clone();
        overlap_ranges(&mut ranges, fade);
        for l in &mut self.layers {
            if let Some(i) = before.iter().position(|r| *r == (l.vel_lo, l.vel_hi)) {
                (l.vel_lo, l.vel_hi) = ranges[i];
            }
        }
        self.vel_fade = fade;
    }

    /// The pad `old` with `layer` as its only layer and named `name` (its
    /// envelopes, filter, mute group and the rest kept), or a new pad when
    /// there was none: what [`Project::put_source`] puts.
    pub fn replacing(name: &str, layer: Layer, old: Option<&Pad>) -> Pad {
        match old {
            Some(old) => Pad {
                name: name.to_string(),
                layers: vec![layer],
                select: LayerSelect::All,
                ..old.clone()
            },
            None => Pad::new(name, layer),
        }
    }
}

/// One step that can be undone: a pad replaced, a source added, or the
/// master effect changed. Each carries what is needed to reverse it.
#[derive(Clone, Debug)]
enum Undo {
    Pad {
        index: usize,
        before: Option<Box<Pad>>,
        gesture: Option<u64>,
    },
    Master {
        before: MasterFx,
        gesture: Option<u64>,
    },
    AddSource {
        id: SourceId,
    },
    /// Undo pops the newest chop set (keeping it here); redo pushes it back.
    AddChop {
        set: Option<Box<ChopSet>>,
    },
    /// What the pads play changed.
    PadsPlay {
        before: PadsPlay,
    },
    /// A sound named: its name before.
    Rename {
        id: SourceId,
        before: String,
    },
    /// Several steps made by one command (chop to pads), undone together.
    Group(Vec<Undo>),
}

impl Undo {
    /// What this step changes, for the undo history's names.
    fn info(&self) -> StepInfo {
        let mut i = StepInfo::default();
        self.add_info(&mut i);
        i
    }

    fn add_info(&self, i: &mut StepInfo) {
        match self {
            Undo::Pad { index, .. } => {
                i.pads += 1;
                i.first_pad = Some(i.first_pad.map_or(*index, |f| f.min(*index)));
            }
            Undo::Master { .. } => i.master = true,
            Undo::AddSource { .. } => i.sources += 1,
            Undo::AddChop { .. } => i.chop = true,
            Undo::PadsPlay { .. } => i.pads_play = true,
            Undo::Rename { .. } => i.named += 1,
            Undo::Group(steps) => steps.iter().for_each(|u| u.add_info(i)),
        }
    }
}

/// What an undo step changes, so the undo history can name it (Jay after
/// live test 3: hold UNDO to see the page's history).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepInfo {
    /// How many pads it changes, and the lowest of them.
    pub pads: usize,
    pub first_pad: Option<usize>,
    /// The master effect.
    pub master: bool,
    /// How many samples it adds.
    pub sources: usize,
    /// It adds a chop set.
    pub chop: bool,
    /// It changes what the pads play.
    pub pads_play: bool,
    /// How many sounds it names.
    pub named: usize,
}

/// The sampler's project: its sources, chop sets, pads and undo history.
#[derive(Clone, Debug, Default)]
pub struct Project {
    sources: Vec<Source>,
    /// Sources taken out by undo, kept so redo can bring them back.
    removed: Vec<Source>,
    /// Chop sets. Add them with [`Project::add_chop_set`] so the step can be
    /// undone; undo removes the newest one.
    pub chop_sets: Vec<ChopSet>,
    pads: Vec<Option<Pad>>,
    master: MasterFx,
    pads_play: PadsPlay,
    undo: Vec<Undo>,
    redo: Vec<Undo>,
    /// Bumps on every change; the engine view records which one it was built from.
    revision: u64,
    /// Scenes A and B: the pads as stored, to morph between and go back
    /// to ([`crate::scene`]). Storing one is not an edit (no undo step).
    pub scenes: [Option<crate::scene::Scene>; 2],
    /// The Chop tab's slices as they were last left, one set a sound
    /// ([`Project::keep_chop`]), so a chop isn't lost before it goes on
    /// pads. Like the scenes, keeping one is not an edit (no undo step);
    /// it is saved with the project and moves with its sound when the
    /// sound is converted to another rate.
    kept_chops: Vec<ChopSet>,
}

impl Project {
    pub fn new() -> Project {
        Project {
            pads: vec![None; PADS],
            ..Default::default()
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Adds a source (or finds the identical one already there) and returns its id.
    pub fn add_source(&mut self, s: Source) -> SourceId {
        let id = s.id();
        if self.source(id).is_none() {
            self.sources.push(s);
            self.push_undo(Undo::AddSource { id });
        }
        id
    }

    /// Adds a chop set (one undo step) and returns its index.
    pub fn add_chop_set(&mut self, c: ChopSet) -> Result<usize, ModelError> {
        if self.source(c.source).is_none() {
            return Err(ModelError::UnknownSource);
        }
        self.chop_sets.push(c);
        self.push_undo(Undo::AddChop { set: None });
        Ok(self.chop_sets.len() - 1)
    }

    pub fn source(&self, id: SourceId) -> Option<&Source> {
        self.sources.iter().find(|s| s.id() == id)
    }

    /// Keeps `c` as its sound's chop, in place of the one kept before (not
    /// an undo step; see [`Project::kept_chop`]).
    pub fn keep_chop(&mut self, c: ChopSet) -> Result<(), ModelError> {
        if self.source(c.source).is_none() {
            return Err(ModelError::UnknownSource);
        }
        match self.kept_chops.iter_mut().find(|k| k.source == c.source) {
            Some(k) => *k = c,
            None => self.kept_chops.push(c),
        }
        Ok(())
    }

    /// The chop kept for sound `id`: its slices as the Chop tab last left
    /// them.
    pub fn kept_chop(&self, id: SourceId) -> Option<&ChopSet> {
        self.kept_chops.iter().find(|k| k.source == id)
    }

    /// Every chop kept, a sound each.
    pub fn kept_chops(&self) -> &[ChopSet] {
        &self.kept_chops
    }
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Every source the project holds in memory: its sources, and those
    /// undo took out and redo may bring back.
    pub fn held(&self) -> impl Iterator<Item = &Source> {
        self.sources.iter().chain(&self.removed)
    }

    pub fn pad(&self, index: usize) -> Option<&Pad> {
        self.pads.get(index).and_then(|p| p.as_ref())
    }

    /// Pads that use a source: shown before anything is deleted.
    pub fn users_of(&self, id: SourceId) -> Vec<usize> {
        (0..PADS)
            .filter(|&i| {
                self.pad(i)
                    .is_some_and(|p| p.layers.iter().any(|l| l.region.source == id))
            })
            .collect()
    }

    pub(crate) fn check_pad(&self, pad: &Pad) -> Result<(), ModelError> {
        if pad.layers.len() > MAX_LAYERS {
            return Err(ModelError::TooManyLayers);
        }
        let depth = pad.pressure.depth;
        if pad
            .keys
            .is_some_and(|k| k.lo > k.hi || k.hi > 127 || k.root > 127)
        {
            return Err(ModelError::BadPad);
        }
        if pad.polyphony == 0
            || usize::from(pad.slice) >= MAX_SLICES
            || !(0.0..=1.0).contains(&pad.wow)
            || pad.key_fade > MAX_KEY_FADE
            || pad.vel_fade > MAX_VEL_FADE
            || pad.chokes.iter().any(|&c| c >= PADS)
            || pad.links.iter().any(|&c| c >= PADS)
            || pad.mute_group.is_some_and(|g| !(1..=16).contains(&g))
            || !depth.is_finite()
            || depth.abs() > 1.0
        {
            return Err(ModelError::BadPad);
        }
        for l in &pad.layers {
            let s = self
                .source(l.region.source)
                .ok_or(ModelError::UnknownSource)?;
            l.region.check(s)?;
            if l.vel_lo > l.vel_hi || l.vel_hi > 127 {
                return Err(ModelError::BadPad);
            }
        }
        Ok(())
    }

    /// Replaces a pad (None clears it). One undo step, unless `gesture` is
    /// the same as the previous step's on the same pad (a knob turn), which
    /// merges into that step.
    pub fn set_pad(
        &mut self,
        index: usize,
        pad: Option<Pad>,
        gesture: Option<u64>,
    ) -> Result<(), ModelError> {
        if index >= PADS {
            return Err(ModelError::BadPad);
        }
        if let Some(p) = &pad {
            self.check_pad(p)?;
        }
        let before = std::mem::replace(&mut self.pads[index], pad);
        let merge = matches!(self.undo.last(), Some(Undo::Pad { index: i, gesture: Some(g), .. }) if *i == index && gesture == Some(*g));
        if !merge {
            self.push_undo(Undo::Pad {
                index,
                before: before.map(Box::new),
                gesture,
            });
        } else {
            self.redo.clear();
            self.revision += 1;
        }
        Ok(())
    }

    /// Edits a pad in place through a closure; the change is checked and undoable.
    pub fn edit_pad(
        &mut self,
        index: usize,
        gesture: Option<u64>,
        f: impl FnOnce(&mut Pad),
    ) -> Result<(), ModelError> {
        let mut p = self.pad(index).cloned().ok_or(ModelError::BadPad)?;
        f(&mut p);
        self.set_pad(index, Some(p), gesture)
    }

    /// Names pad `pad`, the sound `sound`, or both, as one undo step (the
    /// Sample tab's text keyboard). Each name is trimmed of spaces at its
    /// ends and must keep a character; a name already the same changes
    /// nothing. A sound's name is every pad's that plays it, and goes with
    /// the sound when the project is saved.
    pub fn rename(
        &mut self,
        pad: Option<(usize, &str)>,
        sound: Option<(SourceId, &str)>,
    ) -> Result<(), ModelError> {
        let pad = pad.map(|(i, n)| (i, n.trim()));
        let sound = sound.map(|(id, n)| (id, n.trim()));
        if let Some((i, n)) = pad {
            if self.pad(i).is_none() {
                return Err(ModelError::BadPad);
            }
            if n.is_empty() {
                return Err(ModelError::BadName);
            }
        }
        if let Some((id, n)) = sound {
            if self.source(id).is_none() {
                return Err(ModelError::UnknownSource);
            }
            if n.is_empty() {
                return Err(ModelError::BadName);
            }
        }
        let mut steps = Vec::new();
        if let Some((index, n)) = pad {
            if let Some(p) = self.pads[index].as_mut().filter(|p| p.name != n) {
                let before = p.clone();
                p.name = n.to_string();
                steps.push(Undo::Pad {
                    index,
                    before: Some(Box::new(before)),
                    gesture: None,
                });
            }
        }
        if let Some((id, n)) = sound {
            if let Some(s) = self
                .sources
                .iter_mut()
                .find(|s| s.id() == id && s.name != n)
            {
                let before = std::mem::replace(&mut s.name, n.to_string());
                steps.push(Undo::Rename { id, before });
            }
        }
        match steps.len() {
            0 => {}
            1 => self.push_undo(steps.remove(0)),
            _ => self.push_undo(Undo::Group(steps)),
        }
        Ok(())
    }

    pub fn master(&self) -> &MasterFx {
        &self.master
    }

    /// Changes the master effect. One undo step, unless `gesture` is the
    /// same as the previous step's (a knob turn), which merges into it.
    pub fn set_master(&mut self, fx: MasterFx, gesture: Option<u64>) -> Result<(), ModelError> {
        fx.check()?;
        let before = std::mem::replace(&mut self.master, fx);
        let merge = matches!(self.undo.last(), Some(Undo::Master { gesture: Some(g), .. }) if gesture == Some(*g));
        if !merge {
            self.push_undo(Undo::Master { before, gesture });
        } else {
            self.redo.clear();
            self.revision += 1;
        }
        Ok(())
    }

    /// Puts slices `first..first+count` of a chop set on consecutive pads
    /// from `to_pad`, as one undo step.
    pub fn slices_to_pads(
        &mut self,
        chop: usize,
        first: usize,
        count: usize,
        to_pad: usize,
    ) -> Result<(), ModelError> {
        let c = self
            .chop_sets
            .get(chop)
            .ok_or(ModelError::BadSlice)?
            .clone();
        let pads = self.slice_pads(&c, first, count, to_pad, None, first + 1)?;
        let steps = self.place_pads(pads, to_pad);
        self.push_undo(Undo::Group(steps));
        Ok(())
    }

    /// Adds chop set `c` and puts its slices `first..first+count` on
    /// consecutive pads from `to_pad`, all as one undo step. Each slice pad
    /// is `template` (a pad's level, envelopes, filter and so on) with one
    /// layer playing the slice once, keeping the template's first region's
    /// gain, tune and channels; pads are named after the chop and numbered
    /// from 1. Returns the chop set's index.
    pub fn chop_to_pads(
        &mut self,
        c: ChopSet,
        first: usize,
        count: usize,
        to_pad: usize,
        template: Option<&Pad>,
    ) -> Result<usize, ModelError> {
        if self.source(c.source).is_none() {
            return Err(ModelError::UnknownSource);
        }
        let pads = self.slice_pads(&c, first, count, to_pad, template, 1)?;
        self.chop_sets.push(c);
        let mut steps = vec![Undo::AddChop { set: None }];
        steps.extend(self.place_pads(pads, to_pad));
        self.push_undo(Undo::Group(steps));
        Ok(self.chop_sets.len() - 1)
    }

    /// The pads for slices `first..first+count`, each checked, so a failure
    /// changes nothing.
    fn slice_pads(
        &self,
        c: &ChopSet,
        first: usize,
        count: usize,
        to_pad: usize,
        template: Option<&Pad>,
        number: usize,
    ) -> Result<Vec<Pad>, ModelError> {
        let s = self.source(c.source).ok_or(ModelError::UnknownSource)?;
        if count == 0 || first + count > c.slices() || to_pad + count > PADS {
            return Err(ModelError::BadSlice);
        }
        let mut pads = Vec::with_capacity(count);
        for k in 0..count {
            let name = format!("{} {}", c.name, number + k);
            let pad = slice_pad(name, c.region(s, first + k)?, template);
            self.check_pad(&pad)?;
            pads.push(pad);
        }
        Ok(pads)
    }

    /// Adds chop set `c` and puts its slices `first..first+count` on pad
    /// `to_pad` as one sliced pad (R4), all as one undo step. The pad is
    /// as [`Project::chop_to_pads`] makes each of its pads, playing its
    /// region from the first slice's start to the last's end, sliced at
    /// the chop's markers, named after the chop; a hit plays slice 1, and
    /// on keys from [`SLICE_KEYS_FROM`] each note plays a slice of its own.
    /// The kit's pads keep playing their sounds. Returns the chop set's
    /// index.
    pub fn chop_to_one_pad(
        &mut self,
        c: ChopSet,
        first: usize,
        count: usize,
        to_pad: usize,
        template: Option<&Pad>,
    ) -> Result<usize, ModelError> {
        let s = self.source(c.source).ok_or(ModelError::UnknownSource)?;
        if first + count > c.slices() || to_pad >= PADS {
            return Err(ModelError::BadSlice);
        }
        let mut pad = slice_pad(c.name.clone(), c.sliced_part(s, first, count)?, template);
        let last = (usize::from(SLICE_KEYS_FROM) + count - 1).min(127) as u8;
        pad.keys = Some(Keys {
            lo: SLICE_KEYS_FROM,
            hi: last,
            root: SLICE_KEYS_FROM,
        });
        // Each slice a note of its own: no fade across the slices' keys.
        pad.key_fade = 0;
        pad.slice = 0;
        self.check_pad(&pad)?;
        self.chop_sets.push(c);
        let mut steps = vec![Undo::AddChop { set: None }];
        steps.extend(self.place_pads(vec![pad], to_pad));
        self.push_undo(Undo::Group(steps));
        Ok(self.chop_sets.len() - 1)
    }

    /// Puts processed audio in place of layer `layer` of pad `pad`: `s` is
    /// that layer's region stretched (or pitch shifted) on a worker, with
    /// `scale` output frames per input frame (through its pins, if it
    /// stretches with pins: the new region has none). The layer then plays all of
    /// `s`, with its loop points and fades moved to match and its warp
    /// tempo too. The old source stays, so one undo brings it all back.
    pub fn use_processed(
        &mut self,
        pad: usize,
        layer: usize,
        s: Source,
        scale: f64,
    ) -> Result<SourceId, ModelError> {
        if !(scale.is_finite() && scale > 0.0) {
            return Err(ModelError::BadRegion("scale"));
        }
        let mut p = self.pad(pad).cloned().ok_or(ModelError::BadPad)?;
        let r = p
            .layers
            .get(layer)
            .ok_or(ModelError::BadPad)?
            .region
            .clone();
        let len = s.frames();
        let rate = self.source(r.source).map_or(s.rate(), |o| o.rate());
        let pinned = r.warp.as_ref().filter(|w| w.pinned());
        let at = |x: u64| match pinned {
            // Through the pins, as the new audio was made.
            Some(w) => {
                let beat = w.beat_at(r.start, r.end, rate, x as f64);
                (beat / w.beats(r.frames(), rate) * len as f64).round() as u64
            }
            None => ((x.saturating_sub(r.start)) as f64 * scale).round() as u64,
        };
        let sustain_loop = r.sustain_loop.and_then(|l| {
            let (a, b) = (at(l.start), at(l.end).min(len));
            (b > a).then(|| Loop {
                start: a,
                end: b,
                crossfade: ((f64::from(l.crossfade) * scale) as u64)
                    .min((b - a) / 2)
                    .min(a) as u32,
                ..l
            })
        });
        let fade = |f: Fade| Fade {
            frames: (f64::from(f.frames) * scale).round() as u32,
            ..f
        };
        let (mut fade_in, mut fade_out) = (fade(r.fade_in), fade(r.fade_out));
        if u64::from(fade_in.frames) + u64::from(fade_out.frames) > len {
            (fade_in, fade_out) = (Fade::NONE, Fade::NONE);
        }
        let mode = if sustain_loop.is_none() && r.mode == PlayMode::Loop {
            PlayMode::OneShot
        } else {
            r.mode
        };
        // A sliced region's slices on the same hits in the new audio.
        let mut slices: Vec<u64> = r.cuts().map(at).filter(|&x| x > 0 && x < len).collect();
        slices.dedup();
        p.layers[layer].region = Region {
            source: s.id(),
            start: 0,
            end: len,
            sustain_loop,
            slices,
            fade_in,
            fade_out,
            mode,
            warp: r.warp.as_ref().map(|w| Warp {
                bpm: (f64::from(w.bpm) / scale) as f32,
                // A stretch's pins are in the new audio (see
                // `warp::Process`); a repitch's stay on their hits.
                pins: if w.pinned() {
                    Vec::new()
                } else {
                    w.pins
                        .iter()
                        .map(|p| Pin {
                            frame: at(p.frame),
                            ..*p
                        })
                        .filter(|p| p.frame > 0 && p.frame < len)
                        .collect()
                },
                ..w.clone()
            }),
            ..r
        };
        let id = s.id();
        let added = self.source(id).is_none();
        if added {
            self.sources.push(s);
        }
        if let Err(e) = self.check_pad(&p) {
            if added {
                self.sources.pop();
            }
            return Err(e);
        }
        let before = self.pads[pad].replace(p);
        let mut steps = Vec::with_capacity(2);
        if added {
            steps.push(Undo::AddSource { id });
        }
        steps.push(Undo::Pad {
            index: pad,
            before: before.map(Box::new),
            gesture: None,
        });
        self.push_undo(Undo::Group(steps));
        Ok(id)
    }

    /// Adds source `s` and puts it on pad `pad`, played whole and once, as
    /// one undo step; the pad takes the source's name. A pad already there
    /// keeps its other settings (level, envelopes, filter, keys) with this
    /// one layer. Returns the source's id.
    pub fn put_source(&mut self, pad: usize, s: Source) -> Result<SourceId, ModelError> {
        self.put_source_as(pad, s, Pad::replacing)
    }

    /// [`Project::put_source`], with the pad made by `make` from the
    /// source's name, its whole-region layer and the pad there before (if
    /// any): one undo step. Returns the source's id.
    pub fn put_source_as(
        &mut self,
        pad: usize,
        s: Source,
        make: impl FnOnce(&str, Layer, Option<&Pad>) -> Pad,
    ) -> Result<SourceId, ModelError> {
        if pad >= PADS {
            return Err(ModelError::BadPad);
        }
        let layer = Layer::new(Region::whole(&s));
        let p = make(&s.name, layer, self.pad(pad));
        let id = s.id();
        let added = self.source(id).is_none();
        if added {
            self.sources.push(s);
        }
        if let Err(e) = self.check_pad(&p) {
            if added {
                self.sources.pop();
            }
            return Err(e);
        }
        let before = self.pads[pad].replace(p);
        let mut steps = Vec::with_capacity(2);
        if added {
            steps.push(Undo::AddSource { id });
        }
        steps.push(Undo::Pad {
            index: pad,
            before: before.map(Box::new),
            gesture: None,
        });
        self.push_undo(Undo::Group(steps));
        Ok(id)
    }

    /// Puts `pad` on pad `index` with the sounds it plays and chop sets on
    /// them (a pad read back from its files: [`crate::export`]), as one
    /// undo step. Sounds already here are used as they are; a chop set on
    /// a sound that isn't among them is refused, and nothing changes.
    pub fn put_pad_with(
        &mut self,
        index: usize,
        sources: Vec<Source>,
        pad: Pad,
        chops: Vec<ChopSet>,
    ) -> Result<(), ModelError> {
        if index >= PADS {
            return Err(ModelError::BadPad);
        }
        let mut added = Vec::new();
        for s in sources {
            let id = s.id();
            if self.source(id).is_none() {
                self.sources.push(s);
                added.push(id);
            }
        }
        let checked = self.check_pad(&pad).and_then(|()| {
            chops
                .iter()
                .all(|c| self.source(c.source).is_some())
                .then_some(())
                .ok_or(ModelError::UnknownSource)
        });
        if let Err(e) = checked {
            self.sources.retain(|s| !added.contains(&s.id()));
            return Err(e);
        }
        let mut steps: Vec<Undo> = added.into_iter().map(|id| Undo::AddSource { id }).collect();
        for c in chops {
            self.chop_sets.push(c);
            steps.push(Undo::AddChop { set: None });
        }
        let before = self.pads[index].replace(pad);
        steps.push(Undo::Pad {
            index,
            before: before.map(Box::new),
            gesture: None,
        });
        self.push_undo(Undo::Group(steps));
        Ok(())
    }

    /// The pads back as `scene` stored them, as one undo step: every pad
    /// that differs from it is put back (or cleared). A stored pad whose
    /// sounds the kit no longer has is left as it is. Returns how many
    /// pads changed and how many were left for that.
    pub fn return_to(&mut self, scene: &crate::scene::Scene) -> (usize, usize) {
        let mut steps = Vec::new();
        let mut left = 0;
        for i in 0..PADS {
            let want = scene.pad(i);
            if self.pad(i) == want {
                continue;
            }
            if let Some(p) = want {
                if self.check_pad(p).is_err() {
                    left += 1;
                    continue;
                }
            }
            let before = std::mem::replace(&mut self.pads[i], want.cloned());
            steps.push(Undo::Pad {
                index: i,
                before: before.map(Box::new),
                gesture: None,
            });
        }
        let n = steps.len();
        if n > 0 {
            self.push_undo(Undo::Group(steps));
        }
        (n, left)
    }

    pub fn pads_play(&self) -> PadsPlay {
        self.pads_play
    }

    /// What the pads play: their own sounds, notes through the
    /// keygroups, or 16 Levels of one pad. One undo step.
    pub fn set_pads_play(&mut self, play: PadsPlay) -> Result<(), ModelError> {
        match play {
            PadsPlay::Keys { from } if from > 127 - 15 => return Err(ModelError::BadPad),
            PadsPlay::Levels { pad, .. } if usize::from(pad) >= PADS => {
                return Err(ModelError::BadPad)
            }
            _ => {}
        }
        if play == self.pads_play {
            return Ok(());
        }
        let before = std::mem::replace(&mut self.pads_play, play);
        self.push_undo(Undo::PadsPlay { before });
        Ok(())
    }

    /// Adds chop set `c` and puts its slices `first..first+count` on
    /// consecutive pads from `to_pad` as keygroups of one note each, from
    /// note `from_note` up, and the pads then play those notes (pad 1 the
    /// first slice). One undo step. Each pad is as
    /// [`Project::chop_to_pads`] makes it. Returns the chop set's index.
    pub fn chop_to_keys(
        &mut self,
        c: ChopSet,
        first: usize,
        count: usize,
        to_pad: usize,
        from_note: u8,
        template: Option<&Pad>,
    ) -> Result<usize, ModelError> {
        if self.source(c.source).is_none() {
            return Err(ModelError::UnknownSource);
        }
        if usize::from(from_note) + count > 128 || from_note > 127 - 15 {
            return Err(ModelError::BadSlice);
        }
        let mut pads = self.slice_pads(&c, first, count, to_pad, template, 1)?;
        for (k, p) in pads.iter_mut().enumerate() {
            p.keys = Some(Keys::one(from_note + k as u8));
        }
        self.chop_sets.push(c);
        let mut steps = vec![Undo::AddChop { set: None }];
        steps.extend(self.place_pads(pads, to_pad));
        let play = PadsPlay::Keys { from: from_note };
        if self.pads_play != play {
            steps.push(Undo::PadsPlay {
                before: std::mem::replace(&mut self.pads_play, play),
            });
        }
        self.push_undo(Undo::Group(steps));
        Ok(self.chop_sets.len() - 1)
    }

    /// The key fade the kit's keygroups share, None when they differ or
    /// there are none.
    pub fn key_fade(&self) -> Option<u8> {
        let mut fades = self.pads.iter().flatten().filter(|p| p.keys.is_some());
        let first = fades.next()?.key_fade;
        fades.all(|p| p.key_fade == first).then_some(first)
    }

    /// Key fade for the whole kit: every keygroup fades over `fade` notes
    /// (at most [`MAX_KEY_FADE`]) and the zones that meet move to overlap
    /// by it ([`overlap_ranges`]), so they crossfade. Keygroups over the
    /// same keys (a stack) move together. One undo step.
    pub fn set_key_fade(&mut self, fade: u8) -> Result<(), ModelError> {
        let fade = fade.min(MAX_KEY_FADE);
        let keygroups: Vec<usize> = (0..PADS)
            .filter(|&i| self.pad(i).is_some_and(|p| p.keys.is_some()))
            .collect();
        if keygroups.is_empty() {
            return Err(ModelError::BadPad);
        }
        let zone = |p: &Pad| p.keys.map_or((0, 0), |k| (k.lo, k.hi));
        let mut ranges: Vec<(u8, u8)> = keygroups
            .iter()
            .filter_map(|&i| self.pad(i).map(zone))
            .collect();
        ranges.sort_unstable();
        ranges.dedup();
        let before = ranges.clone();
        overlap_ranges(&mut ranges, fade);
        let mut changed = Vec::new();
        for &i in &keygroups {
            let Some(mut p) = self.pad(i).cloned() else {
                continue;
            };
            if let Some(at) = before.iter().position(|r| *r == zone(&p)) {
                if let Some(k) = p.keys.as_mut() {
                    (k.lo, k.hi) = ranges[at];
                }
            }
            p.key_fade = fade;
            if self.pads[i].as_ref() != Some(&p) {
                self.check_pad(&p)?;
                changed.push((i, p));
            }
        }
        let mut steps = Vec::with_capacity(changed.len());
        for (i, p) in changed {
            let before = self.pads[i].replace(p);
            steps.push(Undo::Pad {
                index: i,
                before: before.map(Box::new),
                gesture: None,
            });
        }
        if !steps.is_empty() {
            self.push_undo(Undo::Group(steps));
        }
        Ok(())
    }

    /// Makes pad `pad` a keygroup over the whole keyboard with its sound
    /// as recorded at `root` (its detected root note, say), and the pads
    /// play notes from the C at or below the root. One undo step.
    pub fn spread_to_keys(&mut self, pad: usize, root: u8) -> Result<(), ModelError> {
        if root > 127 {
            return Err(ModelError::BadPad);
        }
        let mut p = self.pad(pad).cloned().ok_or(ModelError::BadPad)?;
        p.keys = Some(Keys::all(root));
        p.polyphony = p.polyphony.max(KEYGROUP_POLYPHONY);
        self.check_pad(&p)?;
        let before = self.pads[pad].replace(p);
        let mut steps = vec![Undo::Pad {
            index: pad,
            before: before.map(Box::new),
            gesture: None,
        }];
        let play = PadsPlay::Keys {
            from: (root - root % 12).min(127 - 15),
        };
        if self.pads_play != play {
            steps.push(Undo::PadsPlay {
                before: std::mem::replace(&mut self.pads_play, play),
            });
        }
        self.push_undo(Undo::Group(steps));
        Ok(())
    }

    /// Adds `sources` and puts `pads` (keygroups that use them) on
    /// consecutive pads from `to_pad`, and the pads then play notes from
    /// `from`: an instrument made by [`crate::autosample`], as one undo
    /// step. Nothing changes if any pad is refused.
    pub fn put_instrument(
        &mut self,
        to_pad: usize,
        sources: Vec<Source>,
        pads: Vec<Pad>,
        from: u8,
    ) -> Result<(), ModelError> {
        if pads.is_empty() || to_pad + pads.len() > PADS || from > 127 - 15 {
            return Err(ModelError::BadPad);
        }
        let had = self.sources.len();
        let mut steps = Vec::with_capacity(sources.len() + pads.len() + 1);
        for s in sources {
            let id = s.id();
            if self.source(id).is_none() {
                self.sources.push(s);
                steps.push(Undo::AddSource { id });
            }
        }
        if let Some(e) = pads.iter().find_map(|p| self.check_pad(p).err()) {
            self.sources.truncate(had);
            return Err(e);
        }
        steps.extend(self.place_pads(pads, to_pad));
        let play = PadsPlay::Keys { from };
        if self.pads_play != play {
            steps.push(Undo::PadsPlay {
                before: std::mem::replace(&mut self.pads_play, play),
            });
        }
        self.push_undo(Undo::Group(steps));
        Ok(())
    }

    /// Map to keys: each `(pad, root)` becomes a keygroup played as
    /// recorded at `root` (its sound's own note, see [`crate::keymap`]),
    /// reaching halfway to the next root below and above (the lowest and
    /// highest to the ends of the keyboard, as [`crate::autosample::zones`]
    /// makes them); pads with the same root share its zone and play
    /// together. Each keeps its sounds and settings, with at least
    /// [`KEYGROUP_POLYPHONY`] voices, and the pads then play notes from the
    /// C at or below the lowest root. One undo step; nothing changes if a
    /// pad is empty or refused. Returns each pad's keys, in `roots` order.
    pub fn map_to_keys(&mut self, roots: &[(usize, u8)]) -> Result<Vec<Keys>, ModelError> {
        if roots.is_empty() || roots.iter().any(|&(pad, root)| pad >= PADS || root > 127) {
            return Err(ModelError::BadPad);
        }
        let mut notes: Vec<u8> = roots.iter().map(|&(_, r)| r).collect();
        notes.sort_unstable();
        notes.dedup();
        let zones = crate::autosample::zones(&notes);
        let keys_of = |root: u8| zones[notes.binary_search(&root).unwrap_or(0)];
        let mut pads = Vec::with_capacity(roots.len());
        for &(index, root) in roots {
            let mut p = self.pad(index).cloned().ok_or(ModelError::BadPad)?;
            if p.layers.is_empty() {
                return Err(ModelError::BadPad);
            }
            p.keys = Some(keys_of(root));
            p.polyphony = p.polyphony.max(KEYGROUP_POLYPHONY);
            self.check_pad(&p)?;
            pads.push((index, p));
        }
        let keys = pads
            .iter()
            .map(|(_, p)| p.keys.unwrap_or(Keys::one(0)))
            .collect();
        let mut steps = Vec::with_capacity(pads.len() + 1);
        for (index, p) in pads {
            let before = self.pads[index].replace(p);
            steps.push(Undo::Pad {
                index,
                before: before.map(Box::new),
                gesture: None,
            });
        }
        let low = notes[0];
        let play = PadsPlay::Keys {
            from: (low - low % 12).min(127 - 15),
        };
        if self.pads_play != play {
            steps.push(Undo::PadsPlay {
                before: std::mem::replace(&mut self.pads_play, play),
            });
        }
        self.push_undo(Undo::Group(steps));
        Ok(keys)
    }

    /// The keygroups (pad indices) that play `note`, in pad order.
    pub fn keygroups_for(&self, note: u8) -> Vec<usize> {
        (0..PADS)
            .filter(|&i| {
                self.pad(i)
                    .and_then(|p| p.keys)
                    .is_some_and(|k| k.contains(note))
            })
            .collect()
    }

    fn place_pads(&mut self, pads: Vec<Pad>, to_pad: usize) -> Vec<Undo> {
        let mut steps = Vec::with_capacity(pads.len());
        for (k, pad) in pads.into_iter().enumerate() {
            let before = self.pads[to_pad + k].replace(pad);
            steps.push(Undo::Pad {
                index: to_pad + k,
                before: before.map(Box::new),
                gesture: None,
            });
        }
        steps
    }

    fn push_undo(&mut self, u: Undo) {
        self.undo.push(u);
        self.redo.clear();
        self.revision += 1;
    }

    fn reverse(&mut self, u: Undo) -> Undo {
        match u {
            Undo::Pad {
                index,
                before,
                gesture,
            } => {
                let now = std::mem::replace(&mut self.pads[index], before.map(|b| *b));
                Undo::Pad {
                    index,
                    before: now.map(Box::new),
                    gesture,
                }
            }
            Undo::Master { before, gesture } => Undo::Master {
                before: std::mem::replace(&mut self.master, before),
                gesture,
            },
            Undo::AddSource { id } => {
                // Undoing an add removes the source only when nothing uses it,
                // which holds because later pad steps are undone first.
                if let Some(i) = self.sources.iter().position(|s| s.id() == id) {
                    let s = self.sources.remove(i);
                    self.removed.push(s);
                } else if let Some(i) = self.removed.iter().position(|s| s.id() == id) {
                    let s = self.removed.remove(i);
                    self.sources.push(s);
                }
                Undo::AddSource { id }
            }
            Undo::AddChop { set: None } => Undo::AddChop {
                set: self.chop_sets.pop().map(Box::new),
            },
            Undo::AddChop { set: Some(c) } => {
                self.chop_sets.push(*c);
                Undo::AddChop { set: None }
            }
            Undo::PadsPlay { before } => Undo::PadsPlay {
                before: std::mem::replace(&mut self.pads_play, before),
            },
            Undo::Rename { id, before } => {
                let s = self
                    .sources
                    .iter_mut()
                    .chain(self.removed.iter_mut())
                    .find(|s| s.id() == id);
                let before = match s {
                    Some(s) => std::mem::replace(&mut s.name, before),
                    None => before,
                };
                Undo::Rename { id, before }
            }
            Undo::Group(steps) => {
                Undo::Group(steps.into_iter().rev().map(|u| self.reverse(u)).collect())
            }
        }
    }

    /// Forgets every undo and redo step (and the sources only a redo could
    /// bring back), so undo can't take apart what is there now, such as a
    /// project just built from a kit. Nothing else changes.
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.removed.clear();
    }

    /// Puts every sound at another rate than `rate` at it
    /// ([`convert_to_rate`]), with the pads, the stored scenes and the
    /// chop sets moved along, for a project or kit just read from a file.
    /// When anything moved, the undo history is forgotten
    /// ([`Project::clear_history`]): its steps hold frames on the sounds as
    /// they were. Heavy for long sounds: call it off the audio and screen
    /// threads.
    pub fn convert_to_rate(&mut self, rate: u32) -> Vec<Rescale> {
        self.convert_sounds(&mut at_rate(rate))
    }

    /// [`Project::convert_to_rate`] with the conversion the caller's
    /// ([`convert_sounds`]).
    pub fn convert_sounds(&mut self, convert: &mut Convert<'_>) -> Vec<Rescale> {
        let mut pads: Vec<Pad> = Vec::new();
        let mut at = Vec::new();
        let scenes = self.scenes.iter_mut().flatten();
        let slots = std::iter::once(&mut self.pads).chain(scenes.map(|s| &mut s.pads));
        for (i, list) in slots.enumerate() {
            for (j, p) in list.iter_mut().enumerate() {
                if let Some(p) = p.take() {
                    at.push((i, j));
                    pads.push(p);
                }
            }
        }
        let moved = convert_sounds(&mut self.sources, &mut pads, &mut self.chop_sets, convert);
        for r in &moved {
            for c in &mut self.kept_chops {
                c.rescale(r);
            }
        }
        let scenes = self.scenes.iter_mut().flatten();
        let mut slots: Vec<&mut Vec<Option<Pad>>> = std::iter::once(&mut self.pads)
            .chain(scenes.map(|s| &mut s.pads))
            .collect();
        for ((i, j), p) in at.into_iter().zip(pads) {
            slots[i][j] = Some(p);
        }
        if !moved.is_empty() {
            self.clear_history();
            self.revision += 1;
        }
        moved
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The undo history: the steps undo takes back, oldest first, and the
    /// ones redo brings back, the next one first.
    pub fn history(&self) -> (Vec<StepInfo>, Vec<StepInfo>) {
        (
            self.undo.iter().map(Undo::info).collect(),
            self.redo.iter().rev().map(Undo::info).collect(),
        )
    }

    pub fn undo(&mut self) -> bool {
        let Some(u) = self.undo.pop() else {
            return false;
        };
        let r = self.reverse(u);
        self.redo.push(r);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(u) = self.redo.pop() else {
            return false;
        };
        let r = self.reverse(u);
        self.undo.push(r);
        self.revision += 1;
        true
    }
}
