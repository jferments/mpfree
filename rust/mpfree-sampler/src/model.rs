//! The sampler's project model: sources, regions, chop sets, layers, pads
//! and the undo log. Owned by the control thread; never touched by the
//! audio thread, which gets an [`crate::engine::EngineView`] built from it.
//! Design: `docs/project/sampler-design.md` and `sampler-architecture.md`.

use std::sync::Arc;

pub use mpfree_grain::{Params as GrainParams, TempoSync, Window as GrainWindow};

pub use crate::dsp::{Ahdsr, Filter, FilterKind};

/// Pads in a program: 8 banks of 16.
pub const PADS: usize = 128;
/// Layers per pad.
pub const MAX_LAYERS: usize = 16;
/// Slices per chop set.
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
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for ModelError {}

/// A source's identity: a 64-bit FNV-1a hash of its rate, channel count and
/// samples, so the same audio is stored once however many pads use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub u64);

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{:016x}", self.0) }
}

/// Recorded or imported audio. Never changed after it is made.
#[derive(Clone, Debug)]
pub struct Source {
    id: SourceId,
    pub name: String,
    rate: u32,
    channels: u8,
    /// Interleaved when stereo.
    data: Arc<[f32]>,
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
        if !(8000..=192_000).contains(&rate) { return Err(ModelError::BadSource("rate")); }
        if channels != 1 && channels != 2 { return Err(ModelError::BadSource("channels")); }
        if data.is_empty() || !data.len().is_multiple_of(usize::from(channels)) { return Err(ModelError::BadSource("length")); }
        let mut data = data;
        for x in &mut data { if !x.is_finite() { *x = 0.0; } }
        let mut h = 0xCBF2_9CE4_8422_2325u64;
        fnv1a(&mut h, &rate.to_le_bytes());
        fnv1a(&mut h, &[channels]);
        for x in &data { fnv1a(&mut h, &x.to_bits().to_le_bytes()); }
        Ok(Source { id: SourceId(h), name: name.to_string(), rate, channels, data: data.into() })
    }

    /// From the existing WAV loader's prepared frames (always stereo).
    pub fn from_prepared(name: &str, p: &crate::PreparedSample) -> Source {
        let mut data = Vec::with_capacity(p.frames() * 2);
        for f in p.frame_slice() { data.push(f[0]); data.push(f[1]); }
        Source::new(name, p.sample_rate(), 2, data).expect("prepared samples are valid")
    }

    pub fn id(&self) -> SourceId { self.id }
    pub fn rate(&self) -> u32 { self.rate }
    pub fn channels(&self) -> u8 { self.channels }
    pub fn frames(&self) -> u64 { (self.data.len() / usize::from(self.channels)) as u64 }
    pub fn data(&self) -> &Arc<[f32]> { &self.data }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopMode { Forward, PingPong }

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
pub enum FadeCurve { Linear, Log, Exp }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade { pub frames: u32, pub curve: FadeCurve }

impl Fade {
    pub const NONE: Fade = Fade { frames: 0, curve: FadeCurve::Linear };
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
pub enum ChannelPick { Stereo, MonoSum, Left, Right }

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
}

impl Region {
    /// All of a source, one-shot, untouched.
    pub fn whole(source: &Source) -> Region {
        Region { source: source.id(), start: 0, end: source.frames(), sustain_loop: None,
            fade_in: Fade::NONE, fade_out: Fade::NONE, gain_db: 0.0, reverse: false,
            channels: ChannelPick::Stereo, tune_cents: 0, mode: PlayMode::OneShot, grain: GrainParams::default() }
    }

    pub fn frames(&self) -> u64 { self.end.saturating_sub(self.start) }

    pub fn check(&self, source: &Source) -> Result<(), ModelError> {
        if self.source != source.id() { return Err(ModelError::UnknownSource); }
        if self.start >= self.end || self.end > source.frames() { return Err(ModelError::BadRegion("start/end")); }
        if let Some(l) = self.sustain_loop {
            if l.start < self.start || l.end > self.end || l.end <= l.start { return Err(ModelError::BadRegion("loop points")); }
            if u64::from(l.crossfade) > (l.end - l.start) / 2 || u64::from(l.crossfade) > l.start - self.start {
                return Err(ModelError::BadRegion("loop crossfade"));
            }
        }
        if self.mode == PlayMode::Loop && self.sustain_loop.is_none() { return Err(ModelError::BadRegion("loop mode needs loop points")); }
        if u64::from(self.fade_in.frames) + u64::from(self.fade_out.frames) > self.frames() { return Err(ModelError::BadRegion("fades longer than the region")); }
        if !self.gain_db.is_finite() || self.tune_cents.abs() > 4800 { return Err(ModelError::BadRegion("gain/tune")); }
        let g = &self.grain;
        let numbers = [g.position, g.spray, g.size_ms, g.density, g.pitch, g.pitch_spray, g.scan, g.reverse, g.spread, g.gain];
        let sync_ok = g.sync.is_none_or(|t| t.frames_per_beat.is_finite() && t.grains_per_beat.is_none_or(f32::is_finite) && t.size_beats.is_none_or(f32::is_finite));
        if numbers.iter().any(|x| !x.is_finite()) || !sync_ok { return Err(ModelError::BadRegion("grain settings")); }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChopMode { Transient, Grid, Beat, Manual, Silence }

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
        ChopSet { name: name.to_string(), source: source.id(), mode, markers: Vec::new(), frames: source.frames() }
    }

    /// `n` equal slices.
    pub fn grid(name: &str, source: &Source, n: usize) -> Result<ChopSet, ModelError> {
        if n == 0 || n > MAX_SLICES || n as u64 > source.frames() { return Err(ModelError::TooManySlices); }
        let mut c = ChopSet::new(name, source, ChopMode::Grid);
        for i in 1..n { c.markers.push(source.frames() * i as u64 / n as u64); }
        Ok(c)
    }

    /// A chop from any list of marker frames: sorted, duplicates and points
    /// outside the source dropped (frame 0 is always a boundary already).
    pub fn from_markers(name: &str, source: &Source, mode: ChopMode, markers: impl IntoIterator<Item = u64>) -> Result<ChopSet, ModelError> {
        let mut c = ChopSet::new(name, source, mode);
        c.markers = markers.into_iter().filter(|&m| m > 0 && m < c.frames).collect();
        c.markers.sort_unstable();
        c.markers.dedup();
        if c.markers.len() + 1 > MAX_SLICES { return Err(ModelError::TooManySlices); }
        Ok(c)
    }

    /// Slices of `beats_per_slice` beats at `bpm`, with the first beat at
    /// `first_beat` (a marker goes there too when it isn't 0).
    pub fn beats(name: &str, source: &Source, bpm: f64, beats_per_slice: f64, first_beat: u64) -> Result<ChopSet, ModelError> {
        if !(bpm.is_finite() && bpm > 0.0 && beats_per_slice.is_finite() && beats_per_slice > 0.0) { return Err(ModelError::BadSlice); }
        let step = 60.0 / bpm * beats_per_slice * f64::from(source.rate());
        if step < 1.0 || (source.frames() as f64 / step) as usize >= MAX_SLICES { return Err(ModelError::TooManySlices); }
        let first = first_beat as f64;
        let count = ((source.frames() as f64 - first) / step).ceil().max(0.0) as usize;
        ChopSet::from_markers(name, source, ChopMode::Beat, (0..count).map(|k| (first + k as f64 * step).round() as u64))
    }

    pub fn markers(&self) -> &[u64] { &self.markers }
    pub fn slices(&self) -> usize { self.markers.len() + 1 }

    /// Adds a marker (ignored if one is already there). Returns the new slice count.
    pub fn add_marker(&mut self, at: u64) -> Result<usize, ModelError> {
        if at == 0 || at >= self.frames { return Err(ModelError::BadSlice); }
        match self.markers.binary_search(&at) {
            Ok(_) => {}
            Err(i) => {
                if self.markers.len() + 1 >= MAX_SLICES { return Err(ModelError::TooManySlices); }
                self.markers.insert(i, at);
            }
        }
        Ok(self.slices())
    }

    /// Joins slice `i` with the next one.
    pub fn merge(&mut self, i: usize) -> Result<(), ModelError> {
        if i >= self.markers.len() { return Err(ModelError::BadSlice); }
        self.markers.remove(i);
        Ok(())
    }

    /// Moves the boundary between slice `i` and slice `i+1`, keeping order.
    pub fn move_marker(&mut self, i: usize, to: u64) -> Result<(), ModelError> {
        if i >= self.markers.len() { return Err(ModelError::BadSlice); }
        let lo = if i == 0 { 0 } else { self.markers[i - 1] };
        let hi = self.markers.get(i + 1).copied().unwrap_or(self.frames);
        if to <= lo || to >= hi { return Err(ModelError::BadSlice); }
        self.markers[i] = to;
        Ok(())
    }

    /// (start, end) of slice `i`.
    pub fn bounds(&self, i: usize) -> Option<(u64, u64)> {
        if i >= self.slices() { return None; }
        let start = if i == 0 { 0 } else { self.markers[i - 1] };
        let end = self.markers.get(i).copied().unwrap_or(self.frames);
        Some((start, end))
    }

    /// Slice `i` as a one-shot region.
    pub fn region(&self, source: &Source, i: usize) -> Result<Region, ModelError> {
        if source.id() != self.source { return Err(ModelError::UnknownSource); }
        let (start, end) = self.bounds(i).ok_or(ModelError::BadSlice)?;
        Ok(Region { start, end, ..Region::whole(source) })
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
    pub fn new(region: Region) -> Layer { Layer { region, vel_lo: 1, vel_hi: 127, level_db: 0.0, pan: 0.0, tune_cents: 0 } }
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
}

impl Pad {
    pub fn new(name: &str, layer: Layer) -> Pad {
        Pad { name: name.to_string(), layers: vec![layer], select: LayerSelect::All, polyphony: 8,
            mute_group: None, chokes: Vec::new(), amp: Ahdsr::OPEN, filter: Filter::default(),
            filter_env: Ahdsr::OPEN, vel_to_level: 1.0, vel_to_start: 0.0, level_db: 0.0, pan: 0.0, tune_cents: 0 }
    }
}

/// One step that can be undone: a pad replaced, or a source added. Each
/// carries what is needed to reverse it.
#[derive(Clone, Debug)]
enum Undo {
    Pad { index: usize, before: Option<Box<Pad>>, gesture: Option<u64> },
    AddSource { id: SourceId },
    /// Undo pops the newest chop set (keeping it here); redo pushes it back.
    AddChop { set: Option<Box<ChopSet>> },
    /// Several steps made by one command (chop to pads), undone together.
    Group(Vec<Undo>),
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
    undo: Vec<Undo>,
    redo: Vec<Undo>,
    /// Bumps on every change; the engine view records which one it was built from.
    revision: u64,
}

impl Project {
    pub fn new() -> Project { Project { pads: vec![None; PADS], ..Default::default() } }

    pub fn revision(&self) -> u64 { self.revision }

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
        if self.source(c.source).is_none() { return Err(ModelError::UnknownSource); }
        self.chop_sets.push(c);
        self.push_undo(Undo::AddChop { set: None });
        Ok(self.chop_sets.len() - 1)
    }

    pub fn source(&self, id: SourceId) -> Option<&Source> { self.sources.iter().find(|s| s.id() == id) }
    pub fn sources(&self) -> &[Source] { &self.sources }

    pub fn pad(&self, index: usize) -> Option<&Pad> { self.pads.get(index).and_then(|p| p.as_ref()) }

    /// Pads that use a source: shown before anything is deleted.
    pub fn users_of(&self, id: SourceId) -> Vec<usize> {
        (0..PADS).filter(|&i| self.pad(i).is_some_and(|p| p.layers.iter().any(|l| l.region.source == id))).collect()
    }

    fn check_pad(&self, pad: &Pad) -> Result<(), ModelError> {
        if pad.layers.len() > MAX_LAYERS { return Err(ModelError::TooManyLayers); }
        if pad.polyphony == 0 || pad.chokes.iter().any(|&c| c >= PADS) || pad.mute_group.is_some_and(|g| !(1..=16).contains(&g)) {
            return Err(ModelError::BadPad);
        }
        for l in &pad.layers {
            let s = self.source(l.region.source).ok_or(ModelError::UnknownSource)?;
            l.region.check(s)?;
            if l.vel_lo > l.vel_hi || l.vel_hi > 127 { return Err(ModelError::BadPad); }
        }
        Ok(())
    }

    /// Replaces a pad (None clears it). One undo step, unless `gesture` is
    /// the same as the previous step's on the same pad (a knob turn), which
    /// merges into that step.
    pub fn set_pad(&mut self, index: usize, pad: Option<Pad>, gesture: Option<u64>) -> Result<(), ModelError> {
        if index >= PADS { return Err(ModelError::BadPad); }
        if let Some(p) = &pad { self.check_pad(p)?; }
        let before = std::mem::replace(&mut self.pads[index], pad);
        let merge = matches!(self.undo.last(), Some(Undo::Pad { index: i, gesture: Some(g), .. }) if *i == index && gesture == Some(*g));
        if !merge { self.push_undo(Undo::Pad { index, before: before.map(Box::new), gesture }); } else { self.redo.clear(); self.revision += 1; }
        Ok(())
    }

    /// Edits a pad in place through a closure; the change is checked and undoable.
    pub fn edit_pad(&mut self, index: usize, gesture: Option<u64>, f: impl FnOnce(&mut Pad)) -> Result<(), ModelError> {
        let mut p = self.pad(index).cloned().ok_or(ModelError::BadPad)?;
        f(&mut p);
        self.set_pad(index, Some(p), gesture)
    }

    /// Puts slices `first..first+count` of a chop set on consecutive pads
    /// from `to_pad`, as one undo step.
    pub fn slices_to_pads(&mut self, chop: usize, first: usize, count: usize, to_pad: usize) -> Result<(), ModelError> {
        let c = self.chop_sets.get(chop).ok_or(ModelError::BadSlice)?.clone();
        let s = self.source(c.source).ok_or(ModelError::UnknownSource)?.clone();
        if count == 0 || first + count > c.slices() || to_pad + count > PADS { return Err(ModelError::BadSlice); }
        // Build and check every pad first, so a failure changes nothing.
        let mut pads = Vec::with_capacity(count);
        for k in 0..count {
            let name = format!("{} {}", c.name, first + k + 1);
            let pad = Pad::new(&name, Layer::new(c.region(&s, first + k)?));
            self.check_pad(&pad)?;
            pads.push(pad);
        }
        let mut steps = Vec::with_capacity(count);
        for (k, pad) in pads.into_iter().enumerate() {
            let before = self.pads[to_pad + k].replace(pad);
            steps.push(Undo::Pad { index: to_pad + k, before: before.map(Box::new), gesture: None });
        }
        self.push_undo(Undo::Group(steps));
        Ok(())
    }

    fn push_undo(&mut self, u: Undo) {
        self.undo.push(u);
        self.redo.clear();
        self.revision += 1;
    }

    fn reverse(&mut self, u: Undo) -> Undo {
        match u {
            Undo::Pad { index, before, gesture } => {
                let now = std::mem::replace(&mut self.pads[index], before.map(|b| *b));
                Undo::Pad { index, before: now.map(Box::new), gesture }
            }
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
            Undo::AddChop { set: None } => Undo::AddChop { set: self.chop_sets.pop().map(Box::new) },
            Undo::AddChop { set: Some(c) } => {
                self.chop_sets.push(*c);
                Undo::AddChop { set: None }
            }
            Undo::Group(steps) => Undo::Group(steps.into_iter().rev().map(|u| self.reverse(u)).collect()),
        }
    }

    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }

    pub fn undo(&mut self) -> bool {
        let Some(u) = self.undo.pop() else { return false };
        let r = self.reverse(u);
        self.redo.push(r);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(u) = self.redo.pop() else { return false };
        let r = self.reverse(u);
        self.undo.push(r);
        self.revision += 1;
        true
    }
}
