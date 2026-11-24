//! A sound: a native engine with its own effects and macros, as one node.
//!
//! Players judge a sound with its effects and play it through its macros,
//! so a factory or user preset can carry both ([Architecture, section
//! 5](../../../docs/dev/design/synth-architecture.md#5-what-a-preset-carries)).
//! A [`Sound`] holds the engine's node, up to [`MAX_FX`] effects from
//! `mpfree-fx` in a chain after it, and up to [`MAX_MACROS`] macros. To
//! whatever hosts it (the track host, the pad synth, locks, the mod
//! matrix, MIDI Learn, the Q-Links) it is one node with more settings:
//!
//! - **The engine's settings** keep their ids, so everything that names
//!   them keeps working, and [`Node::info`] is the engine's: the Synth tab
//!   draws the engine's own pages.
//! - **Each effect's settings** are `FX_BASE + slot × FX_SPAN + id`, in a
//!   group named after the slot ("FX 1"), so a step lock on the ensemble's
//!   depth or an LFO on a delay's feedback needs no new code.
//! - **Each macro** is a setting from [`MACRO_BASE`] up (0 to 100 %, group
//!   "Macros"). Moving it moves every setting it maps, each from one value
//!   to another along a curve (in knob positions, so a cutoff sweeps
//!   evenly in pitch). A setting turned by hand afterwards stays where the
//!   hand left it until its macro moves again: the last move wins. A macro
//!   moved on a frame (a lock, automation) moves its settings on that frame.
//!
//! Building a sound allocates (the effects, the parameter list); after
//! [`Node::prepare`] nothing does. A parameter list is made once per
//! design (engine, effects and macro names) and kept for the program's
//! life, as a plugin's is, since nodes hand out `&'static` lists.
//!
//! **In a preset file** a sound's design is a few lines beside its
//! settings ([`Sound::design_lines`], [`Sound::from_preset`]):
//!
//! ```text
//! fx.1 = mpfree.fx.ensemble
//! macro.1 = Brightness
//! macro.1.map = tone.tone -40 40 linear
//! macro.1.map = tone.hardness 30 80 exp
//! ```
//!
//! and its effects' and macros' settings are ordinary settings
//! (`fx-1.depth = 60`, `macros.brightness = 50`).
//!
//! The Synth tab gives a node's "Macros" group its first page
//! (`mpfree_synth_ui::pages::plan`). Not here yet: send hints and the
//! category (they ride along as lines the preset keeps), and the app's
//! tracks and pad synth holding sounds.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use mpfree_fx::Chain;
use mpfree_mod::MapCurve;
use mpfree_node::{
    Ctl, Ctx, Cull, Curve, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Tuning, Unit,
};

use crate::preset::{id_of, key_of, Error as PresetError, Preset, VERSION as PRESET_VERSION};

/// The first effect setting's id.
pub const FX_BASE: u16 = 61_440;
/// Ids each effect slot has for its settings.
pub const FX_SPAN: u16 = 1_024;
/// Effects a sound holds at most.
pub const MAX_FX: usize = 3;
/// The first macro's id.
pub const MACRO_BASE: u16 = FX_BASE + MAX_FX as u16 * FX_SPAN;
/// Macros a sound holds at most.
pub const MAX_MACROS: usize = 6;
/// Settings one macro moves at most.
pub const MAX_MAPS: usize = 8;
/// The longest macro name, in characters.
pub const MAX_MACRO_NAME: usize = 16;
/// Events a block may carry to the engine (after macros are spread out).
const MAX_EVENTS: usize = 1_024;

/// One setting a macro moves: as the macro goes from 0 to 100 %, the
/// setting goes from `from` to `to` (its own units), along `curve` in knob
/// positions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacroMap {
    /// The sound's setting id (an engine's or an effect's).
    pub target: u16,
    pub from: f32,
    pub to: f32,
    pub curve: MapCurve,
}

/// A macro: its name and the settings it moves.
#[derive(Clone, Debug, PartialEq)]
pub struct Macro {
    pub name: String,
    pub maps: Vec<MacroMap>,
}

/// Why a sound can't be built as described.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundError {
    /// More than [`MAX_FX`] effects, or more than [`MAX_MACROS`] macros.
    TooMany(&'static str),
    /// No effect with this node id.
    NoEffect(String),
    /// An effect has a setting id past [`FX_SPAN`].
    EffectIds(String),
    /// The engine has a setting at or past [`FX_BASE`].
    EngineIds,
    /// A macro moves a setting the sound doesn't have, or a meter.
    NoTarget(String),
    /// A macro with no name, or two with one name.
    MacroName(String),
    /// The preset itself (its format, its engine).
    Preset(PresetError),
}

impl std::fmt::Display for SoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SoundError::TooMany(what) => write!(f, "too many {what} for one sound"),
            SoundError::NoEffect(id) => write!(f, "no effect {id}"),
            SoundError::EffectIds(id) => write!(f, "effect {id} has too many settings"),
            SoundError::EngineIds => write!(f, "the engine's settings overlap the sound's"),
            SoundError::NoTarget(t) => write!(f, "a macro moves {t}, which the sound lacks"),
            SoundError::MacroName(n) => write!(f, "macro name \"{n}\" is empty or taken"),
            SoundError::Preset(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SoundError {}

/// What a sound is built from besides its engine: its effects (node ids,
/// in order) and its macros.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Design {
    pub fx: Vec<String>,
    pub macros: Vec<Macro>,
}

/// A native engine with its effects and macros, as one node. `E` is the
/// engine's type: a host that needs the engine's own calls (a factory
/// preset, a drum kit's pads) holds a `Sound<Keys>` and reaches them
/// through `Deref`; any other holds a `Sound` (of `dyn Node`). Settings
/// changed on the engine directly are the engine's: the sound's own
/// settings (its effects', its macros') go through [`Node::set_param`].
pub struct Sound<E: Node + ?Sized = dyn Node> {
    engine: Box<E>,
    fx: Chain,
    fx_ids: Vec<&'static str>,
    macros: Vec<Macro>,
    /// Each macro's position, as set (0 to 100 %).
    macro_at: [f32; MAX_MACROS],
    params: &'static [ParamInfo],
    /// The engine's events for this block (notes, its own settings, the
    /// macros' moves on its settings).
    events: Vec<Timed>,
    dropped_events: u64,
}

/// What a sound adds to its engine, built from a [`Design`].
struct Parts {
    fx: Chain,
    fx_ids: Vec<&'static str>,
    macros: Vec<Macro>,
    params: &'static [ParamInfo],
}

impl Parts {
    /// Builds the effects (prepared at `rate` and `max_block`) and checks
    /// the macros against the sound's settings. Allocates.
    fn build(
        engine: &NodeInfo,
        engine_params: &'static [ParamInfo],
        design: &Design,
        rate: u32,
        max_block: usize,
    ) -> Result<Parts, SoundError> {
        if design.fx.len() > MAX_FX {
            return Err(SoundError::TooMany("effects"));
        }
        if design.macros.len() > MAX_MACROS {
            return Err(SoundError::TooMany("macros"));
        }
        if engine_params.iter().any(|p| p.id >= FX_BASE) {
            return Err(SoundError::EngineIds);
        }
        let mut fx = Chain::with_capacity(MAX_FX);
        fx.prepare(rate, max_block);
        let mut fx_ids = Vec::with_capacity(design.fx.len());
        for id in &design.fx {
            let node =
                mpfree_fx::registry::make(id).ok_or_else(|| SoundError::NoEffect(id.clone()))?;
            if node.params().iter().any(|p| p.id >= FX_SPAN) {
                return Err(SoundError::EffectIds(id.clone()));
            }
            fx_ids.push(node.info().id);
            fx.push(node);
        }
        let mut names: Vec<&str> = Vec::new();
        for m in &design.macros {
            let name = m.name.trim();
            if name.is_empty() || names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                return Err(SoundError::MacroName(m.name.clone()));
            }
            if m.maps.len() > MAX_MAPS {
                return Err(SoundError::TooMany("settings on one macro"));
            }
            names.push(name);
        }
        let params = params_for(engine.id, engine_params, &fx, &fx_ids, &names);
        let macros: Vec<Macro> = design
            .macros
            .iter()
            .map(|m| Macro {
                name: clean_macro_name(&m.name),
                maps: m.maps.clone(),
            })
            .collect();
        for m in &macros {
            for map in &m.maps {
                let ok = params
                    .iter()
                    .any(|p| p.id == map.target && !p.readonly && p.id < MACRO_BASE);
                if !ok {
                    return Err(SoundError::NoTarget(format!("setting {}", map.target)));
                }
            }
        }
        Ok(Parts {
            fx,
            fx_ids,
            macros,
            params,
        })
    }
}

impl<E: Node + ?Sized> Sound<E> {
    /// A sound from `engine`, the effects named in `design` (their node ids
    /// in `mpfree-fx`'s registry) and its macros. Allocates; the effects'
    /// settings start at their defaults and every macro at 0, and the
    /// effects are prepared for 48 kHz until [`Node::prepare`] or
    /// [`Sound::prepare_effects`].
    pub fn new(engine: Box<E>, design: &Design) -> Result<Sound<E>, SoundError> {
        let parts = Parts::build(
            engine.info(),
            engine.params(),
            design,
            48_000,
            mpfree_node::MAX_BLOCK,
        )?;
        Ok(Sound {
            engine,
            fx: parts.fx,
            fx_ids: parts.fx_ids,
            macros: parts.macros,
            macro_at: [0.0; MAX_MACROS],
            params: parts.params,
            events: Vec::with_capacity(MAX_EVENTS),
            dropped_events: 0,
        })
    }

    /// Prepares the effects alone, for a host that made the engine at its
    /// rate and never prepares it (the app's pad synth). Allocates.
    pub fn prepare_effects(&mut self, rate: u32, max_block: usize) {
        self.fx.prepare(rate, max_block);
    }

    /// Puts a new design in: new effects (at their defaults), new macros
    /// (at 0); the engine and its settings stay. On an error nothing
    /// changes. Allocates, and the old effects are dropped here: the
    /// control thread's job, or a moment the host allows it (a preset).
    pub fn set_design(&mut self, design: &Design) -> Result<(), SoundError> {
        let parts = Parts::build(
            self.engine.info(),
            self.engine.params(),
            design,
            self.fx.rate(),
            self.fx.max_block(),
        )?;
        self.put(parts);
        Ok(())
    }

    fn put(&mut self, parts: Parts) {
        self.fx = parts.fx;
        self.fx_ids = parts.fx_ids;
        self.macros = parts.macros;
        self.params = parts.params;
        self.macro_at = [0.0; MAX_MACROS];
    }

    /// A sound from a preset's text: its design lines build the sound on
    /// `engine`, then its settings are applied (each missing one at its
    /// default). Returns the preset too, with the design lines taken out
    /// of its extra lines.
    pub fn from_preset(engine: Box<E>, text: &str) -> Result<(Sound<E>, Preset), SoundError> {
        let mut sound = Sound::new(engine, &Design::default())?;
        let preset = sound.load_text(text)?;
        Ok((sound, preset))
    }

    /// Plays a preset's text from now on: its design (none for a plain
    /// preset) and every setting (each missing one at its default; the
    /// macros at their saved places, without moving what they map, which
    /// the file holds as saved). Notes keep sounding on the engine. Returns
    /// the preset, with the design lines taken out of its extra lines. On
    /// an error nothing changes. Allocates.
    pub fn load_text(&mut self, text: &str) -> Result<Preset, SoundError> {
        let (parts, preset) = self.read_text(text)?;
        // The same design keeps its effects running (no restart, no click).
        if parts.fx_ids != self.fx_ids || parts.macros != self.macros {
            self.put(parts);
        }
        for p in self.params {
            if p.readonly || p.id >= MACRO_BASE {
                continue;
            }
            let v = preset.value(p.id).unwrap_or(p.default);
            self.set_param(p.id, p.clamp(v));
        }
        for (k, p) in self.macro_params().iter().enumerate() {
            let v = preset.value(p.id).unwrap_or(p.default);
            self.macro_at[k] = p.clamp(v);
        }
        Ok(preset)
    }

    /// A preset's text read for this sound's engine: the parts its design
    /// builds and the preset (design lines taken out). Changes nothing.
    fn read_text(&self, text: &str) -> Result<(Parts, Preset), SoundError> {
        let (info, engine_params) = (self.engine.info(), self.engine.params());
        // First the engine alone, for the design (its lines are extra to
        // the engine).
        let bare = Preset::parse(text, info, engine_params).map_err(SoundError::Preset)?;
        let parts = self.parts_from_lines(&bare.extra, bare.format)?;
        let mut preset = Preset::parse(text, info, parts.params).map_err(SoundError::Preset)?;
        preset.extra.retain(|(k, _)| !is_design_key(k));
        // Settings saved in an older format, in this build's meaning (the
        // macros' and effects' own ids are never the engine's).
        preset.upgrade(self);
        Ok((parts, preset))
    }

    /// The parts the design lines among `lines` build on this engine
    /// (lines that aren't a design's are passed over), with the macros'
    /// ranges saved in preset format `format` put in this build's meaning.
    /// Changes nothing.
    fn parts_from_lines(
        &self,
        lines: &[(String, String)],
        format: u32,
    ) -> Result<Parts, SoundError> {
        let (info, engine_params) = (self.engine.info(), self.engine.params());
        let macro_names: Vec<(usize, &str)> = numbered(lines, "macro");
        // The effects and the macros' names give every setting a map may
        // name; then the maps.
        let mut design = Design {
            fx: numbered(lines, "fx")
                .into_iter()
                .map(|(_, v)| v.to_string())
                .collect(),
            macros: macro_names
                .iter()
                .map(|(_, name)| Macro {
                    name: name.to_string(),
                    maps: Vec::new(),
                })
                .collect(),
        };
        let (rate, max_block) = (self.fx.rate(), self.fx.max_block());
        let shell = Parts::build(info, engine_params, &design, rate, max_block)?;
        for (k, (n, _)) in macro_names.iter().enumerate() {
            let key = format!("macro.{n}.map");
            for (_, line) in lines.iter().filter(|(k2, _)| *k2 == key) {
                let mut map = parse_map(shell.params, line)
                    .ok_or_else(|| SoundError::NoTarget(line.clone()))?;
                if format < PRESET_VERSION {
                    if let Some(p) = engine_params.iter().find(|p| p.id == map.target) {
                        map.from = p.clamp(self.engine.upgrade(format, map.target, map.from));
                        map.to = p.clamp(self.engine.upgrade(format, map.target, map.to));
                    }
                }
                if design.macros[k].maps.len() >= MAX_MAPS {
                    return Err(SoundError::TooMany("settings on one macro"));
                }
                design.macros[k].maps.push(map);
            }
        }
        if design.macros.iter().any(|m| !m.maps.is_empty()) {
            Parts::build(info, engine_params, &design, rate, max_block)
        } else {
            Ok(shell)
        }
    }

    /// Puts in the design that `lines` describe ([`Sound::design_lines`],
    /// as [`Node::save_extras`] gives them; other lines are passed over),
    /// as [`Sound::set_design`] does: the engine keeps its settings, the
    /// effects start at their defaults and the macros at 0. On an error
    /// nothing changes. Allocates.
    pub fn set_design_lines(&mut self, lines: &[(String, String)]) -> Result<(), SoundError> {
        self.set_design_lines_from(lines, PRESET_VERSION)
    }

    /// [`Sound::set_design_lines`] for lines saved in preset format
    /// `format` (a project's part from before a setting changed meaning):
    /// a macro's range on such a setting is converted ([`Node::upgrade`]).
    pub fn set_design_lines_from(
        &mut self,
        lines: &[(String, String)],
        format: u32,
    ) -> Result<(), SoundError> {
        let parts = self.parts_from_lines(lines, format)?;
        self.put(parts);
        Ok(())
    }

    /// The preset of the sound now, as text: its settings and the lines of
    /// its design, then `extra` lines (such as the Q-Link sets).
    pub fn to_text(&self, name: &str, tags: &[&str], extra: &[(String, String)]) -> String {
        let mut p = Preset::capture(self, name, tags);
        p.extra = self.design_lines();
        p.extra
            .extend(extra.iter().filter(|(k, _)| !is_design_key(k)).cloned());
        p.to_text(self.params)
    }

    /// The design as preset lines: `fx.N`, `macro.N` and `macro.N.map`.
    pub fn design_lines(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (i, id) in self.fx_ids.iter().enumerate() {
            out.push((format!("fx.{}", i + 1), id.to_string()));
        }
        for (i, m) in self.macros.iter().enumerate() {
            out.push((format!("macro.{}", i + 1), m.name.clone()));
            for map in &m.maps {
                let Some(p) = self.params.iter().find(|p| p.id == map.target) else {
                    continue;
                };
                let key = key_of(self.params, map.target).unwrap_or_default();
                out.push((
                    format!("macro.{}.map", i + 1),
                    format!(
                        "{key} {} {} {}",
                        number(p.clamp(map.from)),
                        number(p.clamp(map.to)),
                        curve_text(map.curve)
                    ),
                ));
            }
        }
        out
    }

    /// The design: effects by node id and the macros.
    pub fn design(&self) -> Design {
        Design {
            fx: self.fx_ids.iter().map(|s| s.to_string()).collect(),
            macros: self.macros.clone(),
        }
    }

    pub fn engine(&self) -> &E {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut E {
        &mut self.engine
    }

    /// The effects' chain (to watch its meters or bypass a slot).
    pub fn fx(&self) -> &Chain {
        &self.fx
    }

    pub fn fx_mut(&mut self) -> &mut Chain {
        &mut self.fx
    }

    pub fn macros(&self) -> &[Macro] {
        &self.macros
    }

    /// Engine events dropped because a block carried more than the sound
    /// has room for (should stay 0).
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    /// The macros' settings: the list's first.
    fn macro_params(&self) -> &'static [ParamInfo] {
        &self.params[..self.macros.len()]
    }

    /// The value macro `k` at position `t` gives each of its settings.
    fn macro_targets(&self, k: usize, t: f32) -> impl Iterator<Item = (u16, f32)> + '_ {
        self.macros[k].maps.iter().filter_map(move |m| {
            let p = self.params.iter().find(|p| p.id == m.target)?;
            let c = m.curve.apply(t);
            // The ends exactly as written; between, in knob positions.
            let v = if c <= 0.0 {
                p.clamp(m.from)
            } else if c >= 1.0 {
                p.clamp(m.to)
            } else {
                let (a, b) = (p.to_unit(m.from), p.to_unit(m.to));
                p.from_unit(a + (b - a) * c)
            };
            Some((m.target, v))
        })
    }
}

/// Where a sound's setting `id` lives.
enum Place {
    Engine,
    Fx(usize, u16),
    Macro(usize),
    None,
}

fn place(id: u16, fx: usize, macros: usize) -> Place {
    if id < FX_BASE {
        Place::Engine
    } else if id < MACRO_BASE {
        let slot = usize::from((id - FX_BASE) / FX_SPAN);
        if slot < fx {
            Place::Fx(slot, (id - FX_BASE) % FX_SPAN)
        } else {
            Place::None
        }
    } else {
        let k = usize::from(id - MACRO_BASE);
        if k < macros {
            Place::Macro(k)
        } else {
            Place::None
        }
    }
}

impl<E: Node + ?Sized> Node for Sound<E> {
    fn info(&self) -> &'static NodeInfo {
        self.engine.info()
    }

    fn params(&self) -> &'static [ParamInfo] {
        self.params
    }

    fn set_param(&mut self, id: u16, value: f32) {
        match place(id, self.fx_ids.len(), self.macros.len()) {
            Place::Engine => self.engine.set_param(id, value),
            Place::Fx(slot, inner) => self.fx.set_param(slot, inner, value),
            Place::Macro(k) => {
                let Some(p) = self.params.iter().find(|p| p.id == id) else {
                    return;
                };
                let t = p.to_unit(value);
                self.macro_at[k] = p.clamp(value);
                for i in 0..self.macros[k].maps.len() {
                    let Some((target, v)) = self.macro_targets(k, t).nth(i) else {
                        break;
                    };
                    self.set_param(target, v);
                }
            }
            Place::None => {}
        }
    }

    fn param(&self, id: u16) -> f32 {
        match place(id, self.fx_ids.len(), self.macros.len()) {
            Place::Engine => self.engine.param(id),
            Place::Fx(slot, inner) => self.fx.node(slot).map_or(f32::NAN, |n| n.param(inner)),
            Place::Macro(k) => self
                .params
                .iter()
                .find(|p| p.id == id)
                .map_or(f32::NAN, |_| self.macro_at[k]),
            Place::None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        self.engine.prepare(rate, max_block);
        self.fx.prepare(rate, max_block);
    }

    fn reset(&mut self) {
        self.engine.reset();
        for slot in 0..self.fx.len() {
            if let Some(n) = self.fx.node_mut(slot) {
                n.reset();
            }
        }
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        self.events.clear();
        let (n_fx, n_macros) = (self.fx_ids.len(), self.macros.len());
        for t in events {
            let NodeEvent::Param { id, value } = t.ev else {
                self.push_engine(*t);
                continue;
            };
            match place(id, n_fx, n_macros) {
                Place::Engine => self.push_engine(*t),
                Place::Fx(slot, inner) => {
                    self.fx.queue(
                        slot,
                        Timed::new(t.offset, NodeEvent::Param { id: inner, value }),
                    );
                }
                Place::Macro(k) => {
                    let Some(p) = self.params.iter().find(|p| p.id == id) else {
                        continue;
                    };
                    let at = p.to_unit(value);
                    self.macro_at[k] = p.clamp(value);
                    for i in 0..self.macros[k].maps.len() {
                        let Some((target, v)) = self.macro_targets(k, at).nth(i) else {
                            break;
                        };
                        let ev = NodeEvent::Param {
                            id: target,
                            value: v,
                        };
                        match place(target, n_fx, n_macros) {
                            Place::Engine => self.push_engine(Timed::new(t.offset, ev)),
                            Place::Fx(slot, inner) => {
                                self.fx.queue(
                                    slot,
                                    Timed::new(
                                        t.offset,
                                        NodeEvent::Param {
                                            id: inner,
                                            value: v,
                                        },
                                    ),
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Place::None => {}
            }
        }
        let events = std::mem::take(&mut self.events);
        self.engine.process(ctx, &events, left, right);
        self.events = events;
        if !self.fx.is_empty() {
            self.fx.process(ctx, left, right);
        }
    }

    fn tail_frames(&self) -> u32 {
        let fx = (0..self.fx.len())
            .filter_map(|s| self.fx.node(s))
            .map(|n| n.tail_frames())
            .fold(0u32, |a, t| a.saturating_add(t));
        self.engine.tail_frames().saturating_add(fx)
    }

    fn latency(&self) -> u32 {
        self.engine.latency().saturating_add(self.fx.latency())
    }

    fn rest_frames(&self) -> u32 {
        self.engine.rest_frames()
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        self.engine.set_tuning(tuning);
    }

    fn follows_tuning(&self) -> bool {
        self.engine.follows_tuning()
    }

    fn hears(&self, ctl: Ctl) -> bool {
        self.engine.hears(ctl)
    }

    fn sounding_voices(&self) -> usize {
        self.engine.sounding_voices()
    }

    fn cull_candidate(&self) -> Option<Cull> {
        self.engine.cull_candidate()
    }

    fn cull(&mut self) -> bool {
        self.engine.cull()
    }

    fn save_extras(&self) -> Vec<(String, String)> {
        let mut out = self.engine.save_extras();
        out.extend(self.design_lines());
        out
    }

    fn upgrade(&self, format: u32, id: u16, value: f32) -> f32 {
        self.engine.upgrade(format, id, value)
    }
}

impl<E: Node + ?Sized> std::ops::Deref for Sound<E> {
    type Target = E;
    fn deref(&self) -> &E {
        &self.engine
    }
}

impl<E: Node + ?Sized> std::ops::DerefMut for Sound<E> {
    fn deref_mut(&mut self) -> &mut E {
        &mut self.engine
    }
}

/// A sound of any engine, for a host that holds several behind one type
/// (the app's pad synth, which keeps each engine's concrete type): what
/// [`Sound`] adds to a node, as [`Sound`]'s own calls of the same names.
pub trait Designed: Node {
    fn design(&self) -> Design;
    fn set_design(&mut self, design: &Design) -> Result<(), SoundError>;
    fn set_design_lines(&mut self, lines: &[(String, String)]) -> Result<(), SoundError>;
    fn load_text(&mut self, text: &str) -> Result<Preset, SoundError>;
    fn to_text(&self, name: &str, tags: &[&str], extra: &[(String, String)]) -> String;
    /// Whether the sound adds nothing to its engine.
    fn is_plain(&self) -> bool;
}

impl<E: Node + ?Sized> Designed for Sound<E> {
    fn design(&self) -> Design {
        Sound::design(self)
    }
    fn set_design(&mut self, design: &Design) -> Result<(), SoundError> {
        Sound::set_design(self, design)
    }
    fn set_design_lines(&mut self, lines: &[(String, String)]) -> Result<(), SoundError> {
        Sound::set_design_lines(self, lines)
    }
    fn load_text(&mut self, text: &str) -> Result<Preset, SoundError> {
        Sound::load_text(self, text)
    }
    fn to_text(&self, name: &str, tags: &[&str], extra: &[(String, String)]) -> String {
        Sound::to_text(self, name, tags, extra)
    }
    fn is_plain(&self) -> bool {
        self.fx_ids.is_empty() && self.macros.is_empty()
    }
}

impl<E: Node + ?Sized> Sound<E> {
    fn push_engine(&mut self, t: Timed) {
        if self.events.len() < self.events.capacity() {
            self.events.push(t);
        } else {
            self.dropped_events += 1;
        }
    }
}

/// The sound's parameter list: the engine's, each effect's (re-numbered,
/// grouped by slot) and the macros. Made once per design and kept.
fn params_for(
    engine: &str,
    engine_params: &'static [ParamInfo],
    fx: &Chain,
    fx_ids: &[&'static str],
    macros: &[&str],
) -> &'static [ParamInfo] {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static [ParamInfo]>>> = OnceLock::new();
    let key = format!(
        "{}|{}|{}",
        engine,
        fx_ids.join(","),
        macros
            .iter()
            .map(|m| clean_macro_name(m))
            .collect::<Vec<_>>()
            .join("\u{1f}")
    );
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = match cache.lock() {
        Ok(c) => c,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(p) = cache.get(&key) {
        return p;
    }
    // The macros first: a host that sets every value in the list's order
    // (a project, a copy) sets each macro before what it moves, so the
    // settings end as saved.
    let mut list: Vec<ParamInfo> = macros
        .iter()
        .enumerate()
        .map(|(k, m)| ParamInfo {
            id: MACRO_BASE + k as u16,
            name: leak(clean_macro_name(m)),
            group: "Macros",
            min: 0.0,
            max: 100.0,
            default: 0.0,
            unit: Unit::Percent,
            curve: Curve::Linear,
            readonly: false,
        })
        .collect();
    list.extend_from_slice(engine_params);
    for (slot, id) in fx_ids.iter().enumerate() {
        let Some(node) = fx.node(slot) else { continue };
        let group: &'static str = leak(format!("FX {}", slot + 1));
        let own = node.params();
        for p in own {
            // A name two of the effect's groups share takes its group.
            let shared = own.iter().filter(|o| o.name == p.name).count() > 1;
            let name: &'static str = if shared {
                leak(format!("{} {}", p.group, p.name))
            } else {
                p.name
            };
            list.push(ParamInfo {
                id: FX_BASE + slot as u16 * FX_SPAN + p.id,
                name,
                group,
                ..*p
            });
        }
        let _ = id;
    }
    let list: &'static [ParamInfo] = Box::leak(list.into_boxed_slice());
    cache.insert(key, list);
    list
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn clean_macro_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_MACRO_NAME)
        .collect()
}

/// `prefix.N = value` lines (N from 1), by N, in order, without gaps
/// counted: `fx.1`, `fx.2` ...
fn numbered<'a>(extra: &'a [(String, String)], prefix: &str) -> Vec<(usize, &'a str)> {
    let mut out: Vec<(usize, &str)> = extra
        .iter()
        .filter_map(|(k, v)| {
            let n = k.strip_prefix(prefix)?.strip_prefix('.')?;
            let n: usize = n.parse().ok()?;
            (n >= 1).then_some((n, v.as_str()))
        })
        .collect();
    out.sort_by_key(|(n, _)| *n);
    out.dedup_by_key(|(n, _)| *n);
    out
}

/// A design line's key: `fx.N`, `macro.N` or `macro.N.map`.
pub fn is_design_key(k: &str) -> bool {
    let rest = k
        .strip_prefix("fx.")
        .or_else(|| k.strip_prefix("macro."))
        .map(|r| r.strip_suffix(".map").unwrap_or(r));
    rest.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether preset `text` has design lines (effects or macros of its own),
/// so it loads as a sound rather than on the bare engine. Reads the text
/// only; never allocates.
pub fn has_design(text: &str) -> bool {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .any(|(k, _)| is_design_key(k.trim()))
}

/// `key from to curve` (the curve may be left out: linear).
fn parse_map(params: &[ParamInfo], line: &str) -> Option<MacroMap> {
    let mut w = line.split_whitespace();
    let target = id_of(params, w.next()?)?;
    let p = params.iter().find(|p| p.id == target)?;
    if p.readonly || target >= MACRO_BASE {
        return None;
    }
    let from: f32 = w.next()?.parse().ok()?;
    let to: f32 = w.next()?.parse().ok()?;
    let curve = match w.next() {
        None => MapCurve::Linear,
        Some(c) => parse_curve(c)?,
    };
    (from.is_finite() && to.is_finite()).then_some(MacroMap {
        target,
        from: p.clamp(from),
        to: p.clamp(to),
        curve,
    })
}

fn curve_text(c: MapCurve) -> &'static str {
    match c {
        MapCurve::Linear => "linear",
        MapCurve::Exp => "exp",
        MapCurve::Log => "log",
        MapCurve::S => "s",
    }
}

fn parse_curve(s: &str) -> Option<MapCurve> {
    Some(match s.to_ascii_lowercase().as_str() {
        "linear" => MapCurve::Linear,
        "exp" => MapCurve::Exp,
        "log" => MapCurve::Log,
        "s" => MapCurve::S,
        _ => return None,
    })
}

/// The shortest text that reads back to the same `f32`.
fn number(v: f32) -> String {
    format!("{v}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_synth::keys_params as kp;
    use mpfree_synth::Keys;

    const RATE: u32 = 48_000;

    fn keys() -> Box<dyn Node> {
        let mut k = Keys::new(RATE);
        k.prepare(RATE, 128);
        Box::new(k)
    }

    fn design() -> Design {
        Design {
            fx: vec!["mpfree.fx.ensemble".into()],
            macros: vec![Macro {
                name: "Brightness".into(),
                maps: vec![
                    MacroMap {
                        target: kp::TONE,
                        from: -40.0,
                        to: 40.0,
                        curve: MapCurve::Linear,
                    },
                    MacroMap {
                        target: kp::HARDNESS,
                        from: 30.0,
                        to: 80.0,
                        curve: MapCurve::Exp,
                    },
                ],
            }],
        }
    }

    fn chord() -> Vec<Timed> {
        [48u8, 55, 64]
            .iter()
            .map(|&note| {
                Timed::new(
                    3,
                    NodeEvent::NoteOn {
                        note,
                        velocity: 100,
                    },
                )
            })
            .collect()
    }

    /// Renders `blocks` of 128 frames, a chord at the start and `more` in
    /// block 1; both channels, block by block.
    fn play_with(n: &mut dyn Node, blocks: u64, more: &[Timed]) -> Vec<f32> {
        let mut out = Vec::new();
        let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
        for b in 0..blocks {
            let ev: Vec<Timed> = match b {
                0 => chord(),
                1 => more.to_vec(),
                _ => Vec::new(),
            };
            l.fill(0.0);
            r.fill(0.0);
            n.process(&Ctx::new(RATE, b * 128), &ev, &mut l, &mut r);
            out.extend(l.iter().chain(r.iter()));
        }
        out
    }

    fn play(n: &mut dyn Node, blocks: u64) -> Vec<f32> {
        play_with(n, blocks, &[])
    }

    fn id_named(s: &Sound, group: &str, name: &str) -> u16 {
        s.params()
            .iter()
            .find(|p| p.group == group && p.name == name)
            .unwrap_or_else(|| panic!("{group} {name}"))
            .id
    }

    #[test]
    fn a_sound_with_nothing_added_is_its_engine() {
        let mut s = Sound::new(keys(), &Design::default()).unwrap();
        let mut k = keys();
        assert_eq!(s.params().len(), k.params().len());
        assert_eq!(s.info().id, "mpfree.keys");
        assert_eq!(play(&mut s, 40), play(k.as_mut(), 40));
    }

    #[test]
    fn the_effects_follow_the_engine() {
        let mut s = Sound::new(keys(), &design()).unwrap();
        s.prepare(RATE, 128);
        let mut k = keys();
        let mut chain = Chain::with_capacity(1);
        chain.push(mpfree_fx::registry::make("mpfree.fx.ensemble").unwrap());
        chain.prepare(RATE, 128);
        let mut want = Vec::new();
        let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
        for b in 0..40u64 {
            let ev = if b == 0 { chord() } else { Vec::new() };
            l.fill(0.0);
            r.fill(0.0);
            let ctx = Ctx::new(RATE, b * 128);
            k.process(&ctx, &ev, &mut l, &mut r);
            chain.process(&ctx, &mut l, &mut r);
            want.extend(l.iter().chain(r.iter()));
        }
        let got = play(&mut s, 40);
        assert_eq!(got, want);
        assert!(got.iter().any(|v| v.abs() > 1e-3));
    }

    #[test]
    fn effect_settings_have_their_own_ids_and_groups() {
        let mut s = Sound::new(keys(), &design()).unwrap();
        mpfree_node::check_params(s.params()).unwrap();
        let mix = id_named(&s, "FX 1", "Mix");
        assert!((FX_BASE..FX_BASE + FX_SPAN).contains(&mix));
        s.set_param(mix, 25.0);
        assert_eq!(s.param(mix), 25.0);
        assert_eq!(s.fx().node(0).unwrap().param(mix - FX_BASE), 25.0);
        // The engine's own keep theirs.
        s.set_param(kp::TONE, 12.0);
        assert_eq!(s.param(kp::TONE), 12.0);
        assert!(s.param(FX_BASE + FX_SPAN).is_nan(), "no second effect");
        // One list per design, kept.
        let again = Sound::new(keys(), &design()).unwrap();
        assert!(std::ptr::eq(s.params(), again.params()));
    }

    #[test]
    fn a_macro_moves_its_settings_along_their_curves() {
        let mut s = Sound::new(keys(), &design()).unwrap();
        let m = id_named(&s, "Macros", "Brightness");
        assert_eq!(m, MACRO_BASE);
        s.set_param(m, 0.0);
        assert_eq!(s.param(kp::TONE), -40.0);
        assert_eq!(s.param(kp::HARDNESS), 30.0);
        s.set_param(m, 100.0);
        assert_eq!(s.param(kp::TONE), 40.0);
        assert_eq!(s.param(kp::HARDNESS), 80.0);
        s.set_param(m, 50.0);
        assert!((s.param(kp::TONE) - 0.0).abs() < 1e-4);
        // Exp: a quarter of the way at half travel.
        assert!(
            (s.param(kp::HARDNESS) - 42.5).abs() < 1e-3,
            "{}",
            s.param(kp::HARDNESS)
        );
        assert_eq!(s.param(m), 50.0);
        // A hand on a mapped setting wins until the macro moves again.
        s.set_param(kp::TONE, -90.0);
        assert_eq!(s.param(kp::TONE), -90.0);
        s.set_param(m, 100.0);
        assert_eq!(s.param(kp::TONE), 40.0);
    }

    #[test]
    fn a_macro_moved_on_a_frame_moves_its_settings_on_that_frame() {
        let mut a = Sound::new(keys(), &design()).unwrap();
        let mut b = Sound::new(keys(), &design()).unwrap();
        a.set_param(MACRO_BASE, 0.0);
        b.set_param(MACRO_BASE, 0.0);
        let moved = [Timed::new(
            64,
            NodeEvent::Param {
                id: MACRO_BASE,
                value: 100.0,
            },
        )];
        let by_hand = [
            Timed::new(
                64,
                NodeEvent::Param {
                    id: kp::TONE,
                    value: 40.0,
                },
            ),
            Timed::new(
                64,
                NodeEvent::Param {
                    id: kp::HARDNESS,
                    value: 80.0,
                },
            ),
        ];
        assert_eq!(
            play_with(&mut a, 30, &moved),
            play_with(&mut b, 30, &by_hand)
        );
        assert_eq!(a.param(MACRO_BASE), 100.0);
        // An effect's setting on a frame reaches the effect.
        let mix = id_named(&a, "FX 1", "Mix");
        let ev = [Timed::new(
            5,
            NodeEvent::Param {
                id: mix,
                value: 10.0,
            },
        )];
        play_with(&mut a, 2, &ev);
        assert_eq!(a.param(mix), 10.0);
    }

    #[test]
    fn a_sound_saves_and_loads_to_the_bit() {
        let mut s = Sound::new(keys(), &design()).unwrap();
        s.set_param(MACRO_BASE, 35.0);
        s.set_param(id_named(&s, "FX 1", "Mix"), 42.0);
        s.set_param(kp::DECAY, 4321.5);
        let extra = vec![("qset".to_string(), "10 11 12 13 Tone".to_string())];
        let text = s.to_text("Bright Ens", &["keys", "wide"], &extra);
        assert!(text.contains("fx.1 = mpfree.fx.ensemble\n"), "{text}");
        assert!(text.contains("macro.1 = Brightness\n"));
        assert!(
            text.contains("macro.1.map = tone.tone -40 40 linear\n"),
            "{text}"
        );
        assert!(text.contains("macro.1.map = tone.hardness 30 80 exp\n"));
        assert!(text.contains("macros.brightness = 35\n"), "{text}");
        assert!(text.contains("fx-1.mix = 42\n"), "{text}");
        let (mut back, preset) = Sound::from_preset(keys(), &text).unwrap();
        assert_eq!(back.design(), s.design());
        assert_eq!(preset.name, "Bright Ens");
        assert_eq!(preset.extra, extra, "only the other owners' lines are left");
        for p in s.params() {
            assert_eq!(
                back.param(p.id).to_bits(),
                s.param(p.id).to_bits(),
                "{}",
                p.name
            );
        }
        assert_eq!(play(&mut back, 30), play(&mut s, 30));
        // Saved again, the same text.
        assert_eq!(
            back.to_text("Bright Ens", &["keys", "wide"], &preset.extra),
            text
        );
    }

    #[test]
    fn a_plain_preset_loads_as_a_sound_with_nothing_added() {
        let k = keys();
        let text = Preset::capture(k.as_ref(), "Plain", &[]).to_text(k.params());
        let (s, _) = Sound::from_preset(keys(), &text).unwrap();
        assert_eq!(s.design(), Design::default());
        assert_eq!(s.params().len(), k.params().len());
    }

    #[test]
    fn what_cannot_be_built_says_why() {
        let bad = |d: Design| Sound::new(keys(), &d).err().unwrap();
        assert_eq!(
            bad(Design {
                fx: vec!["mpfree.fx.nothing".into()],
                ..Design::default()
            }),
            SoundError::NoEffect("mpfree.fx.nothing".into())
        );
        assert_eq!(
            bad(Design {
                fx: vec!["mpfree.fx.ensemble".into(); 4],
                ..Design::default()
            }),
            SoundError::TooMany("effects")
        );
        let named = |name: &str| Macro {
            name: name.into(),
            maps: Vec::new(),
        };
        assert!(matches!(
            bad(Design {
                macros: vec![named("Tone"), named("tone")],
                ..Design::default()
            }),
            SoundError::MacroName(_)
        ));
        assert!(matches!(
            bad(Design {
                macros: vec![Macro {
                    name: "Odd".into(),
                    maps: vec![MacroMap {
                        target: FX_BASE,
                        from: 0.0,
                        to: 1.0,
                        curve: MapCurve::Linear,
                    }],
                }],
                ..Design::default()
            }),
            SoundError::NoTarget(_)
        ));
        let k = keys();
        let mut text = Preset::capture(k.as_ref(), "Odd", &[]).to_text(k.params());
        text.push_str("macro.1 = Odd\nmacro.1.map = no.such 0 1\n");
        assert!(matches!(
            Sound::from_preset(keys(), &text).err().unwrap(),
            SoundError::NoTarget(_)
        ));
    }

    /// A sound loads another preset in place: a plain one takes its
    /// effects and macros away, a designed one brings its own; one that
    /// can't be read changes nothing.
    #[test]
    fn a_sound_loads_presets_in_place_and_a_bad_one_changes_nothing() {
        let mut s = Sound::new(keys(), &Design::default()).unwrap();
        let designed = {
            let mut d = Sound::new(keys(), &design()).unwrap();
            d.set_param(MACRO_BASE, 60.0);
            d.set_param(kp::DECAY, 2500.0);
            d.to_text("Designed", &[], &[])
        };
        s.load_text(&designed).unwrap();
        assert_eq!(s.design(), design());
        assert_eq!(s.param(MACRO_BASE), 60.0);
        assert_eq!(s.param(kp::DECAY), 2500.0);
        let before: Vec<u32> = s.params().iter().map(|p| s.param(p.id).to_bits()).collect();
        let mut broken = designed.clone();
        broken.push_str("fx.2 = mpfree.fx.nothing\n");
        assert_eq!(
            s.load_text(&broken).err(),
            Some(SoundError::NoEffect("mpfree.fx.nothing".into()))
        );
        assert_eq!(s.design(), design(), "nothing changed");
        let after: Vec<u32> = s.params().iter().map(|p| s.param(p.id).to_bits()).collect();
        assert_eq!(before, after);
        // A plain preset: the engine's settings only, the design gone.
        let k = keys();
        let plain = Preset::capture(k.as_ref(), "Plain", &[]).to_text(k.params());
        s.load_text(&plain).unwrap();
        assert_eq!(s.design(), Design::default());
        assert_eq!(s.params().len(), k.params().len());
        assert_eq!(s.param(kp::DECAY), k.param(kp::DECAY));
    }

    /// A preset saved before Poly's envelope times changed meaning (preset
    /// format 1) loads converted, its macros' ranges too, so it plays as
    /// it did; saved again, it is in this build's format and meaning.
    #[test]
    fn a_sound_saved_in_an_older_format_plays_as_it_did() {
        use mpfree_synth::params as pp;
        let poly = || -> Box<dyn Node> {
            let mut p = mpfree_synth::Poly::new(RATE);
            p.prepare(RATE, 128);
            Box::new(p)
        };
        let old = "mpfree-preset = 1\nengine = mpfree.poly\nname = Old\n\
                   amp-env.attack = 100\namp-env.decay = 300\n\
                   macro.1 = Swell\nmacro.1.map = amp-env.attack 10 1000 exp\n\
                   macros.swell = 0\n";
        let (s, preset) = Sound::from_preset(poly(), old).unwrap();
        assert_eq!(preset.format, PRESET_VERSION);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3 * b;
        assert!(near(s.param(pp::AMP_A), 31.877), "{}", s.param(pp::AMP_A));
        assert_eq!(s.param(pp::AMP_D), 300.0);
        let map = s.design().macros[0].maps[0];
        assert!(near(map.from, 3.1877) && near(map.to, 318.77), "{map:?}");
        let again = s.to_text("Old", &[], &[]);
        assert!(again.starts_with("mpfree-preset = 2\n"), "{again}");
        let (back, _) = Sound::from_preset(poly(), &again).unwrap();
        assert_eq!(back.param(pp::AMP_A), s.param(pp::AMP_A), "converted once");
        assert_eq!(back.design(), s.design());
    }

    /// `set_design` keeps the engine's settings and starts the effects and
    /// macros afresh; a sound of a concrete engine reaches its own calls.
    #[test]
    fn a_new_design_keeps_the_engine_and_the_engine_is_reachable() {
        let mut k = Keys::new(RATE);
        k.prepare(RATE, 128);
        let mut s: Sound<Keys> = Sound::new(Box::new(k), &Design::default()).unwrap();
        s.set_param(kp::DECAY, 3000.0);
        s.set_design(&design()).unwrap();
        assert_eq!(s.param(kp::DECAY), 3000.0);
        assert_eq!(s.param(MACRO_BASE), 0.0);
        assert_eq!(s.fx().len(), 1);
        // The engine's own calls, through the sound.
        let init = &Keys::presets()[0];
        s.load_preset(init);
        assert_eq!(s.active_voices(), 0);
        assert_eq!(s.engine().param(kp::DECAY), s.param(kp::DECAY));
        assert!(s
            .set_design(&Design {
                fx: vec!["mpfree.fx.nothing".into()],
                ..Design::default()
            })
            .is_err());
        assert_eq!(s.design(), design(), "a failed design changes nothing");
    }

    /// A host that copies a sound by setting every value in the list's
    /// order (a project, a track's copy) gets the same sound, even when a
    /// setting was turned after its macro: the macros come first.
    #[test]
    fn every_value_set_in_order_copies_the_sound() {
        let mut a = Sound::new(keys(), &design()).unwrap();
        a.set_param(MACRO_BASE, 70.0);
        a.set_param(kp::TONE, -12.0);
        assert_eq!(a.params()[0].id, MACRO_BASE);
        let mut b = Sound::new(keys(), &design()).unwrap();
        for p in a.params() {
            if !p.readonly {
                b.set_param(p.id, a.param(p.id));
            }
        }
        for p in a.params() {
            assert_eq!(
                b.param(p.id).to_bits(),
                a.param(p.id).to_bits(),
                "{}",
                p.name
            );
        }
    }

    #[test]
    fn design_keys() {
        for k in ["fx.1", "fx.3", "macro.2", "macro.6.map"] {
            assert!(is_design_key(k), "{k}");
        }
        for k in ["fx-1.mix", "fx.", "macro.x", "macros.brightness", "qset"] {
            assert!(!is_design_key(k), "{k}");
        }
    }
}
