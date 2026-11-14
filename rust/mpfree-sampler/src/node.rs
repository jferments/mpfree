//! The sampler as an instrument node: a kit or a keygroup program that any
//! track can hold, played by the sequencer, the pads and the arp like a
//! synth (`mpfree-node`'s [`Node`]).
//!
//! [`SamplerNode`] owns a real-time [`Engine`]. Every note goes through
//! [`Engine::schedule`], so it lands on its frame for any block size:
//!
//! - **A drum kit** ([`PadsPlay::Pads`] or 16 Levels): note `Pad 1 note +
//!   n` hits pad `n` (pad 1 is note 36 by default, as on Drums), its
//!   pressure goes to that pad, and its note-off lets go of that pad. A
//!   note below the pads goes to the keygroups, so a kit with a keygroup
//!   pad still plays it.
//! - **A keygroup program** ([`PadsPlay::Keys`]): every note goes to the
//!   keygroups as itself.
//!
//! A note-off goes where its note-on went, even if the kit or "Pad 1 note"
//! changed in between.
//!
//! **New sounds** (new samples, an edited pad, another kit) arrive as an
//! [`EngineView`] built on the control thread and sent with
//! [`SamplerControl::send_view`]. The node takes it in at the start of a
//! block, never more than one per block, and hands the view it replaces
//! back to [`SamplerControl::take_old`] to be dropped there: the audio
//! thread never allocates or frees. Playing voices finish as they started.
//!
//! **Parameters.** The kit's follow Drums' ids: Level (dB, smoothed, on
//! its frame) and Pad 1 note, plus Tune and Cutoff, which move every pad
//! at once ([`Event::ProgramLive`]). Pads 1 to 16 each have Level, Pan,
//! Tune, Cutoff, Resonance, Attack, Decay and Release at ids `pad * 16 +
//! field` ([`pad_id`]), as Drums lays out its pads. These are **offsets
//! on the pad's own settings** (the ones edited on the Sample tab): at
//! their defaults the pad plays as it is, and a knob, a p-lock or a
//! modulation moves it from there. Each change goes to the engine as an
//! [`Event::PadLive`] on its frame; playing voices glide to Level, Pan,
//! Cutoff and Resonance, and the rest reach the next hit. When a new view
//! comes in, the offsets go onto its pads' own settings.

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError};

use mpfree_node::{
    Ctx, Cull, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit,
};

use crate::engine::{Engine, EngineView, Event, PadField, ProgramField};
use crate::model::{PadsPlay, PADS};

/// The node's id and name.
pub static INFO: NodeInfo = NodeInfo {
    id: "mpfree.sampler",
    name: "Sampler",
    kind: Kind::Instrument,
};

/// Kit parameter ids, the same as Drums' kit settings where they share one.
pub mod ids {
    /// The kit's level, in dB.
    pub const LEVEL: u16 = 300;
    /// The note pad 1 plays; pad n plays `BASE_NOTE + n - 1`.
    pub const BASE_NOTE: u16 = 301;
    /// Every pad's tune moves by this many semitones (playing voices too).
    pub const TUNE: u16 = 302;
    /// Every filter's cutoff moves by this many octaves; below 0 a pad
    /// without a filter gets a low-pass, so the whole kit darkens.
    pub const CUTOFF: u16 = 303;
    /// Read-only: voices sounding now.
    pub const METER_VOICES: u16 = 310;
}

/// Pads with settings on the node: pads 1 to 16 (bank A).
pub const LIVE_PADS: usize = 16;
/// Id stride between pads, as on Drums.
pub const STRIDE: u16 = 16;

/// A pad's settings: add to `pad * STRIDE` ([`pad_id`]). The order is
/// [`PadField::ALL`]'s.
pub mod field {
    /// dB on the pad's own level.
    pub const LEVEL: u16 = 0;
    /// Percent of the way to a side, on the pad's own pan.
    pub const PAN: u16 = 1;
    /// Semitones on the pad's own tune (new hits).
    pub const TUNE: u16 = 2;
    /// Octaves on the pad's own cutoff. Below 0, a pad whose filter is off
    /// gets a low-pass; above 0 it stays open.
    pub const CUTOFF: u16 = 3;
    /// Percent added to the pad's own resonance (where there is a filter).
    pub const RESONANCE: u16 = 4;
    /// The amp envelope's attack, decay and release, as a percentage of
    /// the pad's own (new hits). A time of 0 is taken as 1 ms, so a pad
    /// with no attack can still be given one.
    pub const ATTACK: u16 = 5;
    pub const DECAY: u16 = 6;
    pub const RELEASE: u16 = 7;
    /// Fields per pad.
    pub const COUNT: usize = 8;
}

/// The id of `field` on pad `pad` (0 to 15).
pub const fn pad_id(pad: usize, field: u16) -> u16 {
    pad as u16 * STRIDE + field
}

/// The default for "Pad 1 note" (C1 on the MPC's pads, as on Drums).
pub const DEFAULT_BASE_NOTE: u8 = 36;
/// Views waiting to go in, and old views waiting to come back.
pub const VIEW_QUEUE: usize = 2;
/// How long a Level change takes, in ms.
const LEVEL_MS: f32 = 10.0;

const GROUPS: [&str; LIVE_PADS] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16",
];

/// The shortest time a time setting scales from, in ms.
const TIME_FLOOR_MS: f32 = 1.0;
/// Where a pad without a filter gets its low-pass from (the engine's open
/// filter).
const OPEN_HZ: f32 = 22_000.0;

/// A pad setting's name, range, default, unit and curve.
type Spec = (&'static str, f32, f32, f32, Unit, Curve);

const fn pad_param(pad: usize, f: u16) -> ParamInfo {
    let (name, min, max, default, unit, curve): Spec = match f {
        field::LEVEL => ("Level", -60.0, 12.0, 0.0, Unit::Db, Curve::Linear),
        field::PAN => ("Pan", -100.0, 100.0, 0.0, Unit::Percent, Curve::Linear),
        field::TUNE => ("Tune", -24.0, 24.0, 0.0, Unit::Semitones, Curve::Linear),
        field::CUTOFF => ("Cutoff", -8.0, 8.0, 0.0, Unit::Octaves, Curve::Linear),
        field::RESONANCE => (
            "Resonance",
            -100.0,
            100.0,
            0.0,
            Unit::Percent,
            Curve::Linear,
        ),
        field::ATTACK => ("Attack", 1.0, 10_000.0, 100.0, Unit::Percent, Curve::Log),
        field::DECAY => ("Decay", 1.0, 10_000.0, 100.0, Unit::Percent, Curve::Log),
        _ => ("Release", 1.0, 10_000.0, 100.0, Unit::Percent, Curve::Log),
    };
    ParamInfo {
        id: pad_id(pad, f),
        name,
        group: GROUPS[pad],
        min,
        max,
        default,
        unit,
        curve,
        readonly: false,
    }
}

const KIT: &[ParamInfo] = &[
    ParamInfo {
        id: ids::LEVEL,
        name: "Level",
        group: "Kit",
        min: -60.0,
        max: 6.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::BASE_NOTE,
        name: "Pad 1 note",
        group: "Kit",
        min: 0.0,
        max: 112.0,
        default: DEFAULT_BASE_NOTE as f32,
        unit: Unit::None,
        curve: Curve::Stepped,
        readonly: false,
    },
    ParamInfo {
        id: ids::TUNE,
        name: "Tune",
        group: "Kit",
        min: -24.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Semitones,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::CUTOFF,
        name: "Cutoff",
        group: "Kit",
        min: -8.0,
        max: 8.0,
        default: 0.0,
        unit: Unit::Octaves,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::METER_VOICES,
        name: "Voices on",
        group: "Kit",
        min: 0.0,
        max: 128.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Stepped,
        readonly: true,
    },
];

const N_PAD_PARAMS: usize = LIVE_PADS * field::COUNT;
const N_PARAMS: usize = N_PAD_PARAMS + KIT.len();

const fn build() -> [ParamInfo; N_PARAMS] {
    let mut out = [KIT[0]; N_PARAMS];
    let mut i = 0;
    while i < N_PAD_PARAMS {
        out[i] = pad_param(i / field::COUNT, (i % field::COUNT) as u16);
        i += 1;
    }
    let mut k = 0;
    while k < KIT.len() {
        out[N_PAD_PARAMS + k] = KIT[k];
        k += 1;
    }
    out
}

/// Every parameter: Pad 1 to Pad 16, then Kit.
pub static PARAMS: [ParamInfo; N_PARAMS] = build();

/// The pad and field of a pad setting's id.
fn pad_field(id: u16) -> Option<(usize, usize)> {
    let (pad, f) = (usize::from(id / STRIDE), usize::from(id % STRIDE));
    (pad < LIVE_PADS && f < field::COUNT).then_some((pad, f))
}

/// A parameter's description, by id.
fn info(id: u16) -> Option<&'static ParamInfo> {
    match pad_field(id) {
        Some((pad, f)) => Some(&PARAMS[pad * field::COUNT + f]),
        None => PARAMS[N_PAD_PARAMS..].iter().find(|p| p.id == id),
    }
}

/// What a pad setting at `offset` asks of the engine, given the pad's own
/// value (`NaN` where it has none): the live value, or NaN for the pad's
/// own.
fn live_value(f: usize, own: f32, offset: f32) -> f32 {
    let field = PadField::ALL[f];
    if offset == PARAMS[f].default {
        return f32::NAN;
    }
    match field {
        PadField::Level => own + offset,
        PadField::Pan => own + offset / 100.0,
        PadField::Tune => own + offset * 100.0,
        PadField::Cutoff if own.is_nan() && offset > 0.0 => f32::NAN,
        PadField::Cutoff => (if own.is_nan() { OPEN_HZ } else { own }) * offset.exp2(),
        PadField::Resonance => (if own.is_nan() { 0.0 } else { own }) + offset / 100.0,
        PadField::Attack | PadField::Decay | PadField::Release => {
            own.max(TIME_FLOOR_MS) * offset / 100.0
        }
    }
}

/// Where a held note went, so its note-off follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    No,
    Pad(u8),
    Key,
}

/// The control thread's side: sends new views in and takes old ones back.
/// Both calls are `try` calls; never wait on the node.
pub struct SamplerControl {
    views: SyncSender<EngineView>,
    old: Receiver<EngineView>,
}

impl SamplerControl {
    /// Sends a view for the node to play from its next block. Gives it back
    /// if [`VIEW_QUEUE`] views are already waiting (send it again later) or
    /// the node is gone.
    // The view comes back whole, to send again; boxing it would allocate.
    #[allow(clippy::result_large_err)]
    pub fn send_view(&self, view: EngineView) -> Result<(), EngineView> {
        match self.views.try_send(view) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(v) | TrySendError::Disconnected(v)) => Err(v),
        }
    }

    /// A view the node has replaced, to drop here (off the audio thread).
    /// Call it now and then, so the node can keep taking new views.
    pub fn take_old(&self) -> Option<EngineView> {
        self.old.try_recv().ok()
    }
}

/// The sampler as a [`Node`].
pub struct SamplerNode {
    engine: Engine,
    /// What the view playing now does with the pads.
    play: PadsPlay,
    views: Receiver<EngineView>,
    old: SyncSender<EngineView>,
    /// A replaced view that couldn't go back yet (the return queue was
    /// full); no new view is taken until it has.
    parked: Option<EngineView>,
    level_db: f32,
    gain: Smooth,
    base: u8,
    held: [Held; 128],
    /// Pads 1 to 16's settings (offsets on their own), by field.
    pads: [[f32; field::COUNT]; LIVE_PADS],
    /// The view's own values of them (NaN where a pad has none).
    own: [[f32; field::COUNT]; LIVE_PADS],
    /// The kit's Tune (semitones) and Cutoff (octaves).
    tune: f32,
    cutoff: f32,
}

impl SamplerNode {
    /// A node playing `view` with `voices` voices, and its control side.
    /// Allocates: call it on the control thread.
    pub fn new(rate: u32, voices: usize, view: EngineView) -> (SamplerNode, SamplerControl) {
        let (views_tx, views_rx) = sync_channel(VIEW_QUEUE);
        let (old_tx, old_rx) = sync_channel(VIEW_QUEUE);
        let mut engine = Engine::new(rate, voices);
        let play = view.pads_play();
        let own = own_values(&view);
        engine.set_view(view);
        let mut gain = Smooth::new(1.0);
        gain.set_time(rate, LEVEL_MS);
        let node = SamplerNode {
            engine,
            play,
            views: views_rx,
            old: old_tx,
            parked: None,
            level_db: 0.0,
            gain,
            base: DEFAULT_BASE_NOTE,
            held: [Held::No; 128],
            pads: std::array::from_fn(|_| std::array::from_fn(|f| PARAMS[f].default)),
            own,
            tune: 0.0,
            cutoff: 0.0,
        };
        let control = SamplerControl {
            views: views_tx,
            old: old_rx,
        };
        (node, control)
    }

    /// The engine, for reading its playheads and stats.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Takes in at most one waiting view, if the one it replaces can go back.
    fn take_view(&mut self) {
        if let Some(p) = self.parked.take() {
            match self.old.try_send(p) {
                Ok(()) => {}
                Err(TrySendError::Full(p) | TrySendError::Disconnected(p)) => {
                    self.parked = Some(p);
                    return;
                }
            }
        }
        match self.views.try_recv() {
            Ok(view) => {
                self.play = view.pads_play();
                self.own = own_values(&view);
                let old = self.engine.set_view(view);
                if let Err(TrySendError::Full(o) | TrySendError::Disconnected(o)) =
                    self.old.try_send(old)
                {
                    self.parked = Some(o);
                }
                // The new view starts from its pads' own settings: put the
                // offsets back on them.
                for pad in 0..LIVE_PADS {
                    for f in 0..field::COUNT {
                        if let Some(ev) = self.pad_event(pad, f) {
                            self.engine.apply(ev);
                        }
                    }
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }
    }

    /// The pad a note hits on a drum kit, if it is in the pads' range.
    fn pad_of(&self, note: u8) -> Option<u8> {
        let pad = usize::from(note.checked_sub(self.base)?);
        (pad < PADS).then_some(pad as u8)
    }

    fn keys_only(&self) -> bool {
        matches!(self.play, PadsPlay::Keys { .. })
    }

    fn event(&mut self, offset: u32, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => {
                let note = note & 0x7F;
                // A note held again first lets go of where it went before.
                self.release(offset, note);
                let to = match (self.keys_only(), self.pad_of(note)) {
                    (false, Some(pad)) => {
                        self.engine
                            .schedule(offset, Event::NoteOn { pad, velocity });
                        Held::Pad(pad)
                    }
                    _ => {
                        self.engine.schedule(offset, Event::Key { note, velocity });
                        Held::Key
                    }
                };
                self.held[usize::from(note)] = to;
            }
            NodeEvent::NoteOn { note, .. } | NodeEvent::NoteOff { note } => {
                self.release(offset, note & 0x7F);
            }
            NodeEvent::Pressure { note, value } => {
                if let Held::Pad(pad) = self.held[usize::from(note & 0x7F)] {
                    self.engine.schedule(offset, Event::Pressure { pad, value });
                }
            }
            NodeEvent::AllOff => {
                self.engine.schedule(offset, Event::AllOff);
                self.held = [Held::No; 128];
            }
            // Level is applied on its frame by `process`; the rest go to
            // the engine on theirs.
            NodeEvent::Param { id, value } if id != ids::LEVEL => {
                if let Some(ev) = self.set(id, value) {
                    self.engine.schedule(offset, ev);
                }
            }
            // Bend, and anything newer, is for pitched synths.
            _ => {}
        }
    }

    fn release(&mut self, offset: u32, note: u8) {
        let n = usize::from(note);
        match std::mem::replace(&mut self.held[n], Held::No) {
            Held::Pad(pad) => {
                self.engine.schedule(offset, Event::NoteOff { pad });
            }
            Held::Key => {
                self.engine.schedule(offset, Event::KeyOff { note });
            }
            Held::No => {}
        }
    }

    /// Keeps a setting, and gives the engine event it makes, if any.
    fn set(&mut self, id: u16, value: f32) -> Option<Event> {
        let p = info(id).filter(|p| !p.readonly)?;
        let v = p.clamp(value);
        if let Some((pad, f)) = pad_field(id) {
            self.pads[pad][f] = v;
            return Some(self.pad_event(pad, f).unwrap_or(Event::PadLive {
                pad: pad as u8,
                field: PadField::ALL[f],
                value: f32::NAN,
            }));
        }
        match id {
            ids::LEVEL => {
                self.level_db = v;
                self.gain.target = db_gain(v);
                None
            }
            ids::BASE_NOTE => {
                self.base = v as u8;
                None
            }
            ids::TUNE => {
                self.tune = v;
                Some(Event::ProgramLive {
                    field: ProgramField::Tune,
                    value: v * 100.0,
                })
            }
            ids::CUTOFF => {
                self.cutoff = v;
                Some(Event::ProgramLive {
                    field: ProgramField::Cutoff,
                    value: v,
                })
            }
            _ => None,
        }
    }

    /// The engine event that puts pad `pad`'s setting `f` on its own value,
    /// or `None` if it is at its default (the pad plays as it is).
    fn pad_event(&self, pad: usize, f: usize) -> Option<Event> {
        let value = live_value(f, self.own[pad][f], self.pads[pad][f]);
        (!value.is_nan()).then_some(Event::PadLive {
            pad: pad as u8,
            field: PadField::ALL[f],
            value,
        })
    }
}

/// Pads 1 to 16's own values of every field in `view` (NaN where none).
fn own_values(view: &EngineView) -> [[f32; field::COUNT]; LIVE_PADS] {
    std::array::from_fn(|pad| {
        std::array::from_fn(|f| view.pad_own(pad, PadField::ALL[f]).unwrap_or(f32::NAN))
    })
}

fn db_gain(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 20.0)
    }
}

impl Node for SamplerNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        if let Some(ev) = self.set(id, value) {
            self.engine.apply(ev);
        }
    }

    fn param(&self, id: u16) -> f32 {
        if let Some((pad, f)) = pad_field(id) {
            return self.pads[pad][f];
        }
        match id {
            ids::LEVEL => self.level_db,
            ids::BASE_NOTE => f32::from(self.base),
            ids::TUNE => self.tune,
            ids::CUTOFF => self.cutoff,
            ids::METER_VOICES => self.engine.active_voices() as f32,
            _ => f32::NAN,
        }
    }

    /// Nothing to do: the engine and its views are made for one rate, so a
    /// host at another rate makes a new node.
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}

    /// Stops every voice at once ([`Engine::stop_all`]), drops the notes
    /// waiting and forgets the held ones. The settings stay.
    fn reset(&mut self) {
        self.engine.stop_all();
        self.held = [Held::No; 128];
        self.gain.snap(self.gain.target);
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        self.take_view();
        self.engine.set_tempo(ctx.bpm);
        // Notes go to the engine on their frames, in order (a "Pad 1 note"
        // change moves the notes after it); Level changes are applied on
        // their frames to the gain below.
        for t in events {
            self.event(t.offset, &t.ev);
        }
        self.engine.render(left, right);
        let mut at = 0usize;
        let n = left.len();
        let mut params = events
            .iter()
            .filter(|t| matches!(t.ev, NodeEvent::Param { id: ids::LEVEL, .. }))
            .peekable();
        while at < n {
            while let Some(t) = params.next_if(|t| t.offset as usize <= at) {
                if let NodeEvent::Param { id, value } = t.ev {
                    // Level makes no engine event: it is the gain below.
                    let _ = self.set(id, value);
                }
            }
            let to = params.peek().map_or(n, |t| (t.offset as usize).min(n));
            if self.gain.settled() && self.gain.value == 1.0 {
                at = to;
                continue;
            }
            for i in at..to {
                let g = self.gain.next();
                left[i] *= g;
                right[i] *= g;
            }
            at = to;
        }
    }

    fn sounding_voices(&self) -> usize {
        self.engine.active_voices()
    }

    fn cull_candidate(&self) -> Option<Cull> {
        self.engine.cull_candidate()
    }

    fn cull(&mut self) -> bool {
        self.engine.cull()
    }
}
