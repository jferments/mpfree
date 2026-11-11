//! The mod engine: one place that decides the value of every modulated,
//! macro-mapped, morphed or locked parameter, and sends it on as a
//! sample-timed parameter event.
//!
//! See the crate docs for the order the layers apply in.

use mpfree_node::{Ctx, Curve, NodeEvent, ParamInfo, Timed, Unit};
use mpfree_param::{Owner, ParamAddr};

use crate::follow::{Follower, FOLLOWER_PARAMS};
use crate::lfo::{Lfo, LFO_PARAMS};

/// Macro knobs per engine.
pub const MACROS: usize = 8;
/// Targets one macro can move.
pub const MACRO_TARGETS: usize = 8;
/// LFOs per engine: `Owner::Mod(0)` to `Owner::Mod(7)`.
pub const LFOS: usize = 8;
/// Envelope followers per engine: `Owner::Mod(8)` to `Owner::Mod(11)`.
pub const FOLLOWERS: usize = 4;
/// Values the host sets (velocity, pressure, a touch strip...).
pub const HOST_SOURCES: usize = 8;
/// Parameters one engine can look after (internal ones included).
pub const MAX_TARGETS: usize = 512;
/// Rows of the mod matrix.
pub const MAX_ROWS: usize = 64;
/// Frames between control ticks, unless the host picks another.
pub const DEFAULT_TICK: u32 = 32;

/// The internal targets come first: the morph position, then the macros,
/// so everything that reads them sees this tick's value.
const MORPH: usize = 0;
const FIRST_MACRO: usize = 1;
const INTERNAL: usize = FIRST_MACRO + MACROS;

/// Where a mod matrix row takes its signal from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// An LFO, -1 to 1.
    Lfo(u8),
    /// An envelope follower, 0 to 1.
    Follower(u8),
    /// A macro knob's value, 0 to 1.
    Macro(u8),
    /// A value the host sets ([`ModEvent::Source`]), 0 to 1.
    Host(u8),
}

impl Source {
    fn valid(self) -> bool {
        match self {
            Source::Lfo(n) => (n as usize) < LFOS,
            Source::Follower(n) => (n as usize) < FOLLOWERS,
            Source::Macro(n) => (n as usize) < MACROS,
            Source::Host(n) => (n as usize) < HOST_SOURCES,
        }
    }
}

/// How a macro's travel maps onto a target's range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MapCurve {
    #[default]
    Linear,
    /// Slow at first, fast at the end.
    Exp,
    /// Fast at first, slow at the end.
    Log,
    /// Slow at both ends.
    S,
}

impl MapCurve {
    pub const NAMES: &'static [&'static str] = &["Linear", "Exp", "Log", "S-curve"];

    #[inline]
    pub fn apply(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            MapCurve::Linear => x,
            MapCurve::Exp => x * x,
            MapCurve::Log => 1.0 - (1.0 - x) * (1.0 - x),
            MapCurve::S => x * x * (3.0 - 2.0 * x),
        }
    }
}

/// One target of a macro: as the macro goes 0 to 1, the target goes from
/// `from` to `to` (knob positions, 0 to 1, so `from` above `to` turns it
/// down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacroMap {
    pub target: ParamAddr,
    pub from: f32,
    pub to: f32,
    pub curve: MapCurve,
}

/// One row of the mod matrix: `source` × `depth` (× `via`, when there is
/// one) is added to the target's knob position. Depth -1 to 1 is the
/// whole travel of the knob down or up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub source: Source,
    /// Scales the row by a second source, 0 to 1 (an LFO's depth on the
    /// mod wheel). A bipolar `via` counts from its middle.
    pub via: Option<Source>,
    pub target: ParamAddr,
    pub depth: f32,
}

/// Something that happens on an exact frame of a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModEvent {
    /// Sets a parameter's own value (a knob turned, automation). Also sets
    /// a macro (`Owner::Macro`), the morph (`Owner::Morph`) or an LFO's or
    /// follower's settings (`Owner::Mod`).
    Set { addr: ParamAddr, value: f32 },
    /// A parameter lock: replaces the value until [`ModEvent::Unlock`].
    Lock { addr: ParamAddr, value: f32 },
    Unlock { addr: ParamAddr },
    /// A host source's value, 0 to 1.
    Source { n: u8, value: f32 },
    /// A note started: LFOs with Retrigger on restart.
    Trigger,
}

/// A [`ModEvent`] at a frame offset inside the block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModTimed {
    pub offset: u32,
    pub ev: ModEvent,
}

impl ModTimed {
    pub fn new(offset: u32, ev: ModEvent) -> ModTimed { ModTimed { offset, ev } }
}

/// Why a change to the engine was refused. Nothing changes when it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModError {
    /// No room (targets, rows or a macro's maps).
    Full,
    /// The parameter is not a target ([`ModEngine::add_target`] first).
    Unknown(ParamAddr),
    /// A switch (toggle or choice) can be moved by a macro but not
    /// swept by the matrix; a macro or the morph cannot be a macro's
    /// target; a source out of range.
    Refused,
}

/// Counters for the Dev pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModStats {
    pub ticks: u64,
    /// Parameter events sent on.
    pub sent: u64,
    /// Locks and sets for parameters that were not targets: passed on
    /// as they came (an unlock of one cannot restore anything).
    pub unregistered: u64,
}

#[derive(Clone, Copy, Debug)]
struct Target {
    addr: ParamAddr,
    info: ParamInfo,
    /// The parameter's own value (in its units).
    base: f32,
    /// The base as a knob position (worked out when it is set).
    base_u: f32,
    /// ln(min) and ln(max / min) for a log curve, so a position turns
    /// into a value with one `exp`.
    log: Option<(f32, f32)>,
    /// A lock's value and knob position.
    lock: Option<(f32, f32)>,
    /// Scene A and B knob positions.
    scene: [Option<f32>; 2],
    /// The macro and map that move it.
    mac: Option<(u8, u8)>,
    /// Matrix rows aimed at it.
    rows: u8,
    /// The sum of its rows this tick, in knob travel.
    offset: f32,
    /// The knob position it came to last (macros and morph read this).
    pos: f32,
    /// The last value sent; NaN before the first.
    sent: f32,
    dirty: bool,
}

impl Target {
    fn new(addr: ParamAddr, info: ParamInfo, base: f32) -> Target {
        let base = info.clamp(base);
        Target { addr, info, base, base_u: info.to_unit(base), log: log_consts(&info), lock: None, scene: [None; 2], mac: None, rows: 0, offset: 0.0, pos: info.to_unit(base), sent: f32::NAN, dirty: true }
    }

    fn moving(&self) -> bool { self.rows > 0 || self.mac.is_some() || self.morphs() }

    /// Both scenes hold it, so the morph moves it.
    fn morphs(&self) -> bool { self.scene[0].is_some() && self.scene[1].is_some() }

    /// The value at knob position `u`: `ParamInfo::from_unit`, faster.
    #[inline]
    fn value_at(&self, u: f32) -> f32 {
        match self.log {
            Some((a, b)) => self.info.clamp((a + b * u).exp()),
            None => self.info.from_unit(u),
        }
    }

    fn set_base(&mut self, v: f32) {
        self.base = self.info.clamp(v);
        self.base_u = self.info.to_unit(self.base);
    }

    fn is_switch(&self) -> bool { matches!(self.info.curve, Curve::Toggle | Curve::Choice(_)) }
}

#[derive(Clone, Copy, Debug)]
struct Map {
    target: u16,
    from: f32,
    to: f32,
    curve: MapCurve,
}

#[derive(Clone, Copy, Debug)]
struct RowSlot {
    row: Row,
    target: u16,
}

static MACRO_INFO: [ParamInfo; MACROS] = {
    const fn m(name: &'static str) -> ParamInfo {
        ParamInfo { id: 0, name, group: "Macros", min: 0.0, max: 1.0, default: 0.0, unit: Unit::None, curve: Curve::Linear, readonly: false }
    }
    [m("Macro 1"), m("Macro 2"), m("Macro 3"), m("Macro 4"), m("Macro 5"), m("Macro 6"), m("Macro 7"), m("Macro 8")]
};

static MORPH_INFO: ParamInfo = ParamInfo { id: 0, name: "Morph", group: "Morph", min: 0.0, max: 1.0, default: 0.0, unit: Unit::None, curve: Curve::Linear, readonly: false };

/// See the module docs.
pub struct ModEngine {
    targets: Vec<Target>,
    maps: [[Option<Map>; MACRO_TARGETS]; MACROS],
    rows: Vec<RowSlot>,
    lfos: [Lfo; LFOS],
    followers: [Follower; FOLLOWERS],
    host: [f32; HOST_SOURCES],
    lfo_out: [f32; LFOS],
    fol_out: [f32; FOLLOWERS],
    tick: u32,
    rate: u32,
    stats: ModStats,
    /// A set or lock for a parameter that is not a target, to send on as
    /// it came.
    pass: Option<(ParamAddr, f32)>,
}

impl ModEngine {
    /// An engine ticking every `tick` frames (at least 1). Allocates.
    pub fn new(rate: u32, tick: u32) -> ModEngine {
        let rate = rate.max(1);
        let mut targets = Vec::with_capacity(MAX_TARGETS);
        targets.push(Target::new(ParamAddr::morph_pos(0), MORPH_INFO, 0.0));
        for (m, info) in MACRO_INFO.iter().enumerate() {
            targets.push(Target::new(ParamAddr::macro_value(m as u8), *info, 0.0));
        }
        ModEngine {
            targets,
            maps: [[None; MACRO_TARGETS]; MACROS],
            rows: Vec::with_capacity(MAX_ROWS),
            lfos: std::array::from_fn(|i| Lfo::new(0x51F0 + i as u32)),
            followers: std::array::from_fn(|_| Follower::new(rate)),
            host: [0.0; HOST_SOURCES],
            lfo_out: [0.0; LFOS],
            fol_out: [0.0; FOLLOWERS],
            tick: tick.max(1),
            rate,
            stats: ModStats::default(),
            pass: None,
        }
    }

    pub fn rate(&self) -> u32 { self.rate }
    pub fn tick(&self) -> u32 { self.tick }
    pub fn stats(&self) -> ModStats { self.stats }

    /// Changes the sample rate (control thread).
    pub fn prepare(&mut self, rate: u32) {
        self.rate = rate.max(1);
        for f in &mut self.followers { f.prepare(self.rate); }
    }

    /// Restarts the LFOs and empties the followers; keeps every setting.
    pub fn reset(&mut self) {
        for l in &mut self.lfos { l.reset(); }
        for f in &mut self.followers { f.reset(); }
        self.lfo_out = [0.0; LFOS];
        self.fol_out = [0.0; FOLLOWERS];
        for t in &mut self.targets { t.dirty = true; }
    }

    // ---- Targets ----------------------------------------------------------

    fn find(&self, addr: ParamAddr) -> Option<usize> { self.targets.iter().position(|t| t.addr == addr) }

    /// Starts looking after a parameter, with its description and its
    /// value now. From here on, set it through the engine
    /// ([`ModEvent::Set`]), or the engine's next value will undo the
    /// change. Real-time safe. Adding one that is already there updates
    /// its description and value.
    pub fn add_target(&mut self, addr: ParamAddr, info: ParamInfo, value: f32) -> Result<(), ModError> {
        if matches!(addr.owner, Owner::Macro(_) | Owner::Morph(_) | Owner::Mod(_)) {
            return Err(ModError::Refused);
        }
        if let Some(i) = self.find(addr) {
            let t = &mut self.targets[i];
            t.info = info;
            t.log = log_consts(&info);
            t.set_base(value);
            t.dirty = true;
            return Ok(());
        }
        if self.targets.len() == self.targets.capacity() {
            return Err(ModError::Full);
        }
        self.targets.push(Target::new(addr, info, value));
        Ok(())
    }

    pub fn is_target(&self, addr: ParamAddr) -> bool { self.find(addr).is_some() }

    /// Stops looking after a parameter: its macro map and matrix rows go
    /// with it. Real-time safe. False if it was not a target (or is a
    /// macro or the morph, which always are).
    pub fn remove_target(&mut self, addr: ParamAddr) -> bool {
        let Some(i) = self.find(addr) else { return false };
        if i < INTERNAL {
            return false;
        }
        if let Some((m, k)) = self.targets[i].mac {
            self.maps[m as usize][k as usize] = None;
        }
        self.rows.retain(|r| r.target as usize != i);
        let last = self.targets.len() - 1;
        self.targets.swap_remove(i);
        if i != last {
            // The last target moved into `i`: point everything at it there.
            let (from, to) = (last as u16, i as u16);
            for maps in &mut self.maps {
                for m in maps.iter_mut().flatten() {
                    if m.target == from { m.target = to; }
                }
            }
            for r in &mut self.rows {
                if r.target == from { r.target = to; }
            }
        }
        true
    }

    /// Targets with nothing moving them (no macro, row, lock or scene).
    /// The host may remove them to make room.
    pub fn idle_targets(&self) -> impl Iterator<Item = ParamAddr> + '_ {
        self.targets[INTERNAL..].iter().filter(|t| !t.moving() && t.lock.is_none()).map(|t| t.addr)
    }

    pub fn targets(&self) -> impl Iterator<Item = ParamAddr> + '_ { self.targets[INTERNAL..].iter().map(|t| t.addr) }

    /// A target's own value (not modulated).
    pub fn base(&self, addr: ParamAddr) -> Option<f32> {
        if let Some(v) = self.source_param(addr) { return Some(v); }
        self.find(addr).map(|i| self.targets[i].base)
    }

    /// The value last sent (macros and the morph: their position).
    pub fn value(&self, addr: ParamAddr) -> Option<f32> {
        if let Some(v) = self.source_param(addr) { return Some(v); }
        let t = &self.targets[self.find(addr)?];
        Some(if t.sent.is_nan() { t.base } else { t.sent })
    }

    pub fn info(&self, addr: ParamAddr) -> Option<ParamInfo> {
        match addr.owner {
            Owner::Mod(n) => source_info(n as usize, addr.param),
            _ => self.find(addr).map(|i| self.targets[i].info),
        }
    }

    pub fn is_locked(&self, addr: ParamAddr) -> bool { self.find(addr).is_some_and(|i| self.targets[i].lock.is_some()) }

    /// How far modulation can move a target's knob now, as positions
    /// (0 to 1) below and above where it would sit without the matrix,
    /// for a ring around the knob. `None` with no rows.
    pub fn mod_range(&self, addr: ParamAddr) -> Option<(f32, f32)> {
        let i = self.find(addr)?;
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for r in self.rows.iter().filter(|r| r.target as usize == i) {
            let d = r.row.depth.abs();
            let bipolar = matches!(r.row.source, Source::Lfo(_));
            let (a, b) = if bipolar { (-d, d) } else if r.row.depth < 0.0 { (-d, 0.0) } else { (0.0, d) };
            lo += a;
            hi += b;
        }
        if self.targets[i].rows == 0 { return None; }
        let p = (self.targets[i].pos - self.targets[i].offset).clamp(0.0, 1.0);
        Some(((p + lo).clamp(0.0, 1.0), (p + hi).clamp(0.0, 1.0)))
    }

    // ---- Macros -------------------------------------------------------------

    /// Gives macro `m` a target. A parameter has one macro at most: mapping
    /// it again moves it to this macro. Real-time safe.
    pub fn map(&mut self, m: usize, map: MacroMap) -> Result<(), ModError> {
        if m >= MACROS || matches!(map.target.owner, Owner::Macro(_) | Owner::Morph(_)) {
            return Err(ModError::Refused);
        }
        let i = self.find(map.target).ok_or(ModError::Unknown(map.target))?;
        let slot = Map { target: i as u16, from: map.from.clamp(0.0, 1.0), to: map.to.clamp(0.0, 1.0), curve: map.curve };
        // Already on this macro: change it in place.
        if let Some((om, ok)) = self.targets[i].mac {
            if om as usize == m {
                self.maps[m][ok as usize] = Some(slot);
                self.targets[i].dirty = true;
                return Ok(());
            }
        }
        let k = self.maps[m].iter().position(|s| s.is_none()).ok_or(ModError::Full)?;
        if let Some((om, ok)) = self.targets[i].mac {
            self.maps[om as usize][ok as usize] = None;
        }
        self.maps[m][k] = Some(slot);
        self.targets[i].mac = Some((m as u8, k as u8));
        self.targets[i].dirty = true;
        Ok(())
    }

    /// Takes a parameter off its macro. False if it had none.
    pub fn unmap(&mut self, target: ParamAddr) -> bool {
        let Some(i) = self.find(target) else { return false };
        let Some((m, k)) = self.targets[i].mac.take() else { return false };
        self.maps[m as usize][k as usize] = None;
        self.targets[i].dirty = true;
        true
    }

    /// Macro `m`'s targets, in the order they were mapped (for the screen).
    pub fn macro_maps(&self, m: usize) -> impl Iterator<Item = MacroMap> + '_ {
        self.maps.get(m).into_iter().flatten().flatten().map(|s| MacroMap { target: self.targets[s.target as usize].addr, from: s.from, to: s.to, curve: s.curve })
    }

    /// Which macro moves a parameter, if any.
    pub fn macro_of(&self, target: ParamAddr) -> Option<usize> { self.find(target).and_then(|i| self.targets[i].mac).map(|(m, _)| m as usize) }

    pub fn macro_value(&self, m: usize) -> f32 { self.targets.get(FIRST_MACRO + m).filter(|_| m < MACROS).map_or(0.0, |t| t.pos) }

    // ---- The matrix ---------------------------------------------------------

    /// Adds a matrix row. Real-time safe. Refused for a switch target, a
    /// source out of range, or a macro modulating itself.
    pub fn add_row(&mut self, row: Row) -> Result<usize, ModError> {
        if !row.source.valid() || !row.via.is_none_or(Source::valid) || !row.depth.is_finite() {
            return Err(ModError::Refused);
        }
        let i = self.find(row.target).ok_or(ModError::Unknown(row.target))?;
        if self.targets[i].is_switch() {
            return Err(ModError::Refused);
        }
        if let (Source::Macro(m), Owner::Macro(t)) = (row.source, row.target.owner) {
            if m == t { return Err(ModError::Refused); }
        }
        if self.rows.len() == self.rows.capacity() {
            return Err(ModError::Full);
        }
        let row = Row { depth: row.depth.clamp(-1.0, 1.0), ..row };
        self.rows.push(RowSlot { row, target: i as u16 });
        self.targets[i].rows += 1;
        self.targets[i].dirty = true;
        Ok(self.rows.len() - 1)
    }

    /// Changes a row's depth (-1 to 1). False if there is no such row.
    pub fn set_depth(&mut self, row: usize, depth: f32) -> bool {
        let Some(r) = self.rows.get_mut(row) else { return false };
        if depth.is_finite() { r.row.depth = depth.clamp(-1.0, 1.0); }
        true
    }

    /// Takes a row out; later rows move up one. Real-time safe.
    pub fn remove_row(&mut self, row: usize) -> Option<Row> {
        if row >= self.rows.len() { return None; }
        let r = self.rows.remove(row);
        let t = &mut self.targets[r.target as usize];
        t.rows -= 1;
        if t.rows == 0 { t.offset = 0.0; }
        t.dirty = true;
        Some(r.row)
    }

    pub fn rows(&self) -> impl Iterator<Item = Row> + '_ { self.rows.iter().map(|r| r.row) }

    /// A source's value now (-1 to 1 for an LFO, else 0 to 1).
    pub fn source(&self, s: Source) -> f32 {
        match s {
            Source::Lfo(n) => self.lfo_out.get(n as usize).copied().unwrap_or(0.0),
            Source::Follower(n) => self.fol_out.get(n as usize).copied().unwrap_or(0.0),
            Source::Macro(n) => self.macro_value(n as usize),
            Source::Host(n) => self.host.get(n as usize).copied().unwrap_or(0.0),
        }
    }

    pub fn lfo(&self, n: usize) -> Option<&Lfo> { self.lfos.get(n) }
    pub fn follower(&self, n: usize) -> Option<&Follower> { self.followers.get(n) }

    fn source_param(&self, addr: ParamAddr) -> Option<f32> {
        let Owner::Mod(n) = addr.owner else { return None };
        let n = n as usize;
        if n < LFOS {
            Some(self.lfos[n].param(addr.param)).filter(|v| !v.is_nan())
        } else {
            self.followers.get(n - LFOS).map(|f| f.param(addr.param)).filter(|v| !v.is_nan())
        }
    }

    // ---- Morph --------------------------------------------------------------

    /// Stores every target's own value (and every macro's) as scene A (0)
    /// or B (1).
    pub fn capture_scene(&mut self, which: usize) {
        let which = which.min(1);
        for t in &mut self.targets[FIRST_MACRO..] {
            t.scene[which] = Some(t.base_u);
            t.dirty = true;
        }
    }

    /// Sets or clears one target's value in a scene.
    pub fn set_scene_value(&mut self, addr: ParamAddr, which: usize, value: Option<f32>) -> Result<(), ModError> {
        if matches!(addr.owner, Owner::Morph(_)) { return Err(ModError::Refused); }
        let i = self.find(addr).ok_or(ModError::Unknown(addr))?;
        let t = &mut self.targets[i];
        t.scene[which.min(1)] = value.map(|v| t.info.to_unit(v));
        t.dirty = true;
        Ok(())
    }

    /// Forgets both scenes.
    pub fn clear_scenes(&mut self) {
        for t in &mut self.targets { t.scene = [None; 2]; t.dirty = true; }
    }

    /// The parameters whose scenes differ, with their A and B values, for
    /// the "what changes" list.
    pub fn scene_changes(&self) -> impl Iterator<Item = (ParamAddr, f32, f32)> + '_ {
        self.targets[FIRST_MACRO..].iter().filter_map(|t| {
            let [Some(a), Some(b)] = t.scene else { return None };
            ((a - b).abs() > 1e-6).then(|| (t.addr, t.info.from_unit(a), t.info.from_unit(b)))
        })
    }

    pub fn morph(&self) -> f32 { self.targets[MORPH].pos }

    // ---- Audio thread -------------------------------------------------------

    /// Runs one block of `len` frames starting at `ctx.frame`: applies
    /// `events` (sorted by offset) on their frames, ticks every
    /// [`ModEngine::tick`] frames of absolute time, and calls `out` with
    /// each parameter whose value changed and the event to send it, in
    /// time order. `follow[n]` is the audio follower `n` listens to this
    /// block (each side `len` long), or `None` for silence.
    ///
    /// Never allocates. Gives the same events, frame for frame, however
    /// the host cuts the same frames into blocks.
    pub fn process(&mut self, ctx: &Ctx, len: usize, events: &[ModTimed], follow: &[Option<(&[f32], &[f32])>; FOLLOWERS], out: &mut dyn FnMut(ParamAddr, Timed)) {
        let tick = self.tick as u64;
        let mut next_tick = ((tick - ctx.frame % tick) % tick) as usize;
        let mut followed = 0usize;
        let mut ev = 0usize;
        let beats_per_frame = ctx.bpm / 60.0 / self.rate as f64;
        loop {
            let at_ev = events.get(ev).map(|e| (e.offset as usize).min(len.saturating_sub(1)));
            match at_ev {
                // Events on a tick's frame go first, so a lock lands before
                // the tick reads it.
                Some(at) if at <= next_tick || next_tick >= len => {
                    self.apply(&events[ev].ev);
                    self.settle(false, at as u32, out);
                    ev += 1;
                }
                _ if next_tick < len => {
                    self.follow(follow, followed, next_tick);
                    followed = next_tick;
                    let beats = ctx.playing.then_some(ctx.beats + next_tick as f64 * beats_per_frame);
                    self.advance(beats);
                    self.settle(true, next_tick as u32, out);
                    next_tick += tick as usize;
                }
                _ => break,
            }
        }
        self.follow(follow, followed, len);
    }

    fn follow(&mut self, follow: &[Option<(&[f32], &[f32])>; FOLLOWERS], from: usize, to: usize) {
        if to <= from { return; }
        for (f, input) in self.followers.iter_mut().zip(follow) {
            match input {
                Some((l, r)) if l.len() >= to && r.len() >= to => f.run(&l[from..to], &r[from..to]),
                _ => f.decay(to - from),
            }
        }
    }

    fn advance(&mut self, beats: Option<f64>) {
        self.stats.ticks += 1;
        let secs = self.tick as f64 / self.rate as f64;
        for (o, l) in self.lfo_out.iter_mut().zip(&mut self.lfos) { *o = l.tick(secs, beats); }
        for (o, f) in self.fol_out.iter_mut().zip(&self.followers) { *o = f.value(); }
    }

    fn apply(&mut self, ev: &ModEvent) {
        match *ev {
            ModEvent::Set { addr, value } => {
                if let Owner::Mod(n) = addr.owner {
                    let n = n as usize;
                    if n < LFOS {
                        self.lfos[n].set_param(addr.param, value);
                    } else if let Some(f) = self.followers.get_mut(n - LFOS) {
                        f.set_param(addr.param, value);
                    }
                    return;
                }
                let Some(i) = self.find(addr) else {
                    self.stats.unregistered += 1;
                    self.pass = Some((addr, value));
                    return;
                };
                let morph = self.targets[MORPH].pos;
                let t = &mut self.targets[i];
                t.set_base(value);
                // While both scenes hold it, a turn edits the nearer scene.
                if let [Some(_), Some(_)] = t.scene {
                    t.scene[(morph >= 0.5) as usize] = Some(t.base_u);
                }
                t.dirty = true;
            }
            ModEvent::Lock { addr, value } => match self.find(addr) {
                Some(i) => {
                    let t = &mut self.targets[i];
                    let v = t.info.clamp(value);
                    t.lock = Some((v, t.info.to_unit(v)));
                    t.dirty = true;
                }
                None => {
                    self.stats.unregistered += 1;
                    self.pass = Some((addr, value));
                }
            },
            ModEvent::Unlock { addr } => {
                if let Some(i) = self.find(addr) {
                    let t = &mut self.targets[i];
                    if t.lock.take().is_some() { t.dirty = true; }
                }
            }
            ModEvent::Source { n, value } => {
                if let Some(h) = self.host.get_mut(n as usize) {
                    *h = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
                }
            }
            ModEvent::Trigger => {
                for l in &mut self.lfos { l.trigger(); }
            }
        }
    }
}

impl ModEngine {
    /// Works out targets and sends the ones that changed, at `offset`.
    /// On a tick (`full`) the matrix is summed and every moving target is
    /// worked out; after an event only what it touched (and, for a macro
    /// or the morph, what they move).
    fn settle(&mut self, full: bool, offset: u32, out: &mut dyn FnMut(ParamAddr, Timed)) {
        if let Some((addr, value)) = self.pass.take() {
            self.stats.sent += 1;
            out(addr, Timed::new(offset, NodeEvent::Param { id: addr.param, value }));
        }
        if full {
            for t in &mut self.targets {
                if t.rows > 0 { t.offset = 0.0; }
            }
            for r in &self.rows {
                let mut v = source_value(&self.lfo_out, &self.fol_out, &self.host, &self.targets, r.row.source) * r.row.depth;
                if let Some(via) = r.row.via {
                    let s = source_value(&self.lfo_out, &self.fol_out, &self.host, &self.targets, via);
                    v *= if matches!(via, Source::Lfo(_)) { 0.5 + 0.5 * s } else { s };
                }
                self.targets[r.target as usize].offset += v;
            }
        }
        // A macro or the morph that moved drags its targets along.
        let morph_moved = self.targets[MORPH].dirty || (full && self.targets[MORPH].rows > 0);
        let mut macro_moved = [false; MACROS];
        for (m, moved) in macro_moved.iter_mut().enumerate() {
            let t = &self.targets[FIRST_MACRO + m];
            *moved = t.dirty || (full && (t.rows > 0 || morph_moved && t.morphs()));
        }
        let morph_pos = self.eval(MORPH);
        self.commit(MORPH, morph_pos, offset, out);
        for (m, moved) in macro_moved.into_iter().enumerate() {
            if moved || full {
                let v = self.eval(FIRST_MACRO + m);
                self.commit(FIRST_MACRO + m, v, offset, out);
            }
        }
        for i in INTERNAL..self.targets.len() {
            let t = &self.targets[i];
            let touched = t.dirty
                || (full && t.rows > 0)
                || (morph_moved && t.morphs())
                || t.mac.is_some_and(|(m, _)| macro_moved[m as usize]);
            if touched {
                let v = self.eval(i);
                self.commit(i, v, offset, out);
            }
        }
    }

    /// Target `i`'s knob position from its layers, and its exact value
    /// when a single plain value decides it (the base or a lock with no
    /// rows), so an unmodulated parameter goes out exactly as it was set.
    #[inline]
    fn eval(&self, i: usize) -> (f32, Option<f32>) {
        let t = &self.targets[i];
        let mut exact = Some(t.base);
        let mut u = t.base_u;
        if let [Some(a), Some(b)] = t.scene {
            u = a + (b - a) * self.targets[MORPH].pos;
            exact = None;
        }
        if let Some((m, k)) = t.mac {
            if let Some(map) = self.maps[m as usize][k as usize] {
                let x = map.curve.apply(self.targets[FIRST_MACRO + m as usize].pos);
                u = map.from + (map.to - map.from) * x;
                exact = None;
            }
        }
        if let Some((l, lu)) = t.lock {
            u = lu;
            exact = Some(l);
        }
        if t.rows > 0 {
            u += t.offset;
            exact = None;
        }
        (u.clamp(0.0, 1.0), exact)
    }

    fn commit(&mut self, i: usize, (pos, exact): (f32, Option<f32>), offset: u32, out: &mut dyn FnMut(ParamAddr, Timed)) {
        let t = &mut self.targets[i];
        t.pos = pos;
        t.dirty = false;
        let v = exact.unwrap_or_else(|| t.value_at(pos));
        let eps = (t.info.max - t.info.min).abs() * 1e-5;
        if t.sent.is_nan() || (v - t.sent).abs() > eps {
            t.sent = v;
            self.stats.sent += 1;
            out(t.addr, Timed::new(offset, NodeEvent::Param { id: t.addr.param, value: v }));
        }
    }
}

fn log_consts(info: &ParamInfo) -> Option<(f32, f32)> {
    (info.curve == Curve::Log && info.min > 0.0 && info.max > info.min).then(|| (info.min.ln(), (info.max / info.min).ln()))
}

fn source_value(lfo: &[f32; LFOS], fol: &[f32; FOLLOWERS], host: &[f32; HOST_SOURCES], targets: &[Target], s: Source) -> f32 {
    match s {
        Source::Lfo(n) => lfo[n as usize],
        Source::Follower(n) => fol[n as usize],
        // Last tick's macro (macros are worked out after the matrix).
        Source::Macro(n) => targets[FIRST_MACRO + n as usize].pos,
        Source::Host(n) => host[n as usize],
    }
}

/// The description of an LFO's (`n` below [`LFOS`]) or a follower's
/// parameter.
pub fn source_info(n: usize, id: u16) -> Option<ParamInfo> {
    let table: &[ParamInfo] = if n < LFOS { &LFO_PARAMS } else if n < LFOS + FOLLOWERS { &FOLLOWER_PARAMS } else { return None };
    table.iter().find(|p| p.id == id).copied()
}

pub(crate) fn macro_info(m: usize) -> ParamInfo { MACRO_INFO[m.min(MACROS - 1)] }
