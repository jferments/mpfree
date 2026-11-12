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
//! **Parameters** follow Drums' kit ids: Level (dB, smoothed, on its
//! frame) and Pad 1 note. Per-pad settings (level, pan, tune, filter,
//! envelope) come next, through the engine's live settings.

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError};

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::engine::{Engine, EngineView, Event};
use crate::model::{PadsPlay, PADS};

/// The node's id and name.
pub static INFO: NodeInfo = NodeInfo {
    id: "mpfree.sampler",
    name: "Sampler",
    kind: Kind::Instrument,
};

/// Parameter ids, the same as Drums' kit settings.
pub mod ids {
    /// The kit's level, in dB.
    pub const LEVEL: u16 = 300;
    /// The note pad 1 plays; pad n plays `BASE_NOTE + n - 1`.
    pub const BASE_NOTE: u16 = 301;
    /// Read-only: voices sounding now.
    pub const METER_VOICES: u16 = 310;
}

/// The default for "Pad 1 note" (C1 on the MPC's pads, as on Drums).
pub const DEFAULT_BASE_NOTE: u8 = 36;
/// Views waiting to go in, and old views waiting to come back.
pub const VIEW_QUEUE: usize = 2;
/// How long a Level change takes, in ms.
const LEVEL_MS: f32 = 10.0;

pub static PARAMS: &[ParamInfo] = &[
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
}

impl SamplerNode {
    /// A node playing `view` with `voices` voices, and its control side.
    /// Allocates: call it on the control thread.
    pub fn new(rate: u32, voices: usize, view: EngineView) -> (SamplerNode, SamplerControl) {
        let (views_tx, views_rx) = sync_channel(VIEW_QUEUE);
        let (old_tx, old_rx) = sync_channel(VIEW_QUEUE);
        let mut engine = Engine::new(rate, voices);
        let play = view.pads_play();
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
                let old = self.engine.set_view(view);
                if let Err(TrySendError::Full(o) | TrySendError::Disconnected(o)) =
                    self.old.try_send(old)
                {
                    self.parked = Some(o);
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
            // Level is applied on its frame by `process`; the rest now.
            NodeEvent::Param { id, value } if id != ids::LEVEL => self.set(id, value),
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

    fn set(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.iter().find(|p| p.id == id && !p.readonly) else {
            return;
        };
        let v = p.clamp(value);
        match id {
            ids::LEVEL => {
                self.level_db = v;
                self.gain.target = db_gain(v);
            }
            ids::BASE_NOTE => self.base = v as u8,
            _ => {}
        }
    }
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
        PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        self.set(id, value);
    }

    fn param(&self, id: u16) -> f32 {
        match id {
            ids::LEVEL => self.level_db,
            ids::BASE_NOTE => f32::from(self.base),
            ids::METER_VOICES => self.engine.active_voices() as f32,
            _ => 0.0,
        }
    }

    /// Nothing to do: the engine and its views are made for one rate, so a
    /// host at another rate makes a new node.
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}

    /// Releases every voice at once (the engine has no harder stop yet) and
    /// forgets the held notes.
    fn reset(&mut self) {
        self.engine.apply(Event::AllOff);
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
                    self.set(id, value);
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
}
