//! The track host (roadmap H1): each track's instrument, fed by the
//! sequencer and the pads, rendered into its mixer track.
//!
//! - **Slots.** The host owns the destination list that a track's
//!   `mpfree_seq::DestId` indexes: slot `d` is destination `d`. A slot
//!   holds one instrument (any [`Node`] of kind Instrument: Poly, FM, a
//!   drum synth, later a sandboxed plugin) and the mixer track it feeds
//!   (slot `i` feeds track `i` until [`TrackHost::route`] says otherwise).
//!   Effects are not here: every mixer strip has its own insert chain.
//! - **Notes in.** The sequencer hands the host its notes through
//!   [`mpfree_play::Sink`] ([`Out::dest`] picks the slot); the pads, the
//!   arp and anything else call [`TrackHost::note_on`],
//!   [`TrackHost::note_off`] and [`TrackHost::event`]. Notes are counted
//!   per slot: when a clip and a pad play the same note, its note-off goes
//!   out when the last of them lets go, so one never cuts the other.
//! - **Parameters in.** [`TrackHost::param`] is a parameter change on an
//!   exact frame (automation, a parameter lock, the mod matrix), read back
//!   with [`TrackHost::param_value`].
//! - **Sound out.** [`TrackHost::render`] adds each slot's instrument into
//!   its track's input ([`Outputs`], which the mixer implements) before
//!   the mixer runs. A block with a non-finite sample is dropped, the node
//!   reset and the reset counted.
//! - **Swapping an instrument** ([`TrackHost::swap`]): the new one takes
//!   the slot at once; the old one gets `AllOff` and rings out into the
//!   same track until it has been quiet for [`QUIET_FRAMES`] (or its
//!   [`Node::tail_frames`] and a margin have passed, at most
//!   [`MAX_RING_FRAMES`]), then comes back through
//!   [`TrackHost::pop_retired`] to be dropped off the audio thread.
//! - **STOP** ([`TrackHost::stop`]): every instrument fades out over
//!   [`STOP_FADE`] frames, then is reset, so nothing rings on.
//! - **Tuning** ([`TrackHost::set_tuning`]): each slot has its own tuning
//!   (12-TET at A4 = 440 Hz until set). The slot keeps it, so an
//!   instrument swapped in later plays in it too.
//!
//! # Real-time rules
//!
//! [`TrackHost::new`] and [`Instrument::new`] allocate and belong on the
//! control thread. Everything else never allocates, locks or panics: the
//! event queues ([`MAX_EVENTS`] per slot per period, more are counted and
//! dropped), the render buffers and the retired list are all made in
//! `new`. The output is the same, sample for sample, for any block size;
//! the one exception is the moment a ringing-out instrument is retired,
//! which lands on a block boundary (it is inaudible by then).
#![forbid(unsafe_code)]

use mpfree_fx::Mixer;
use mpfree_node::{Ctx, Node, NodeEvent, ParamInfo, Timed, Tuning};
use mpfree_play::{Event, Out, Sink, ALL};
use mpfree_prof::{Probe, Span};

/// Events one slot can take in one period; more are counted and dropped.
pub const MAX_EVENTS: usize = 256;
/// STOP's fade, in frames (10 ms at 44.1 kHz).
pub const STOP_FADE: u32 = 441;
/// A ringing-out instrument is retired after this many frames in a row
/// below [`QUIET_LEVEL`] (about 93 ms at 44.1 kHz).
pub const QUIET_FRAMES: u32 = 4096;
/// The level a ringing-out instrument counts as quiet below (-100 dBFS).
pub const QUIET_LEVEL: f32 = 1e-5;
/// The longest an old instrument rings out (20 s at 44.1 kHz).
pub const MAX_RING_FRAMES: u32 = 882_000;

/// An instrument made ready for the audio thread: the node, prepared for
/// the host's rate and block size, and its timing probe. Making one
/// allocates; do it on the control thread and hand it to
/// [`TrackHost::swap`].
pub struct Instrument {
    node: Box<dyn Node>,
    probe: Box<Probe>,
}

impl std::fmt::Debug for Instrument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instrument")
            .field("node", &self.node.info().id)
            .finish()
    }
}

impl Instrument {
    pub fn new(mut node: Box<dyn Node>, rate: u32, max_block: usize) -> Instrument {
        node.prepare(rate, max_block);
        Instrument {
            node,
            probe: Box::new(Probe::new(u64::MAX)),
        }
    }

    pub fn node(&self) -> &dyn Node {
        self.node.as_ref()
    }

    pub fn node_mut(&mut self) -> &mut (dyn Node + 'static) {
        self.node.as_mut()
    }

    pub fn into_node(self) -> Box<dyn Node> {
        self.node
    }
}

/// Where the host adds each track's sound: the mixer's track inputs, or
/// anything else that hands out a pair of buffers per track. Each pair is
/// at least as long as the block; the host adds into its start.
pub trait Outputs {
    fn track(&mut self, track: usize) -> Option<(&mut [f32], &mut [f32])>;

    /// Whether `track` wants its instrument's sound this block. False while
    /// the mixer plays a frozen take of the track in its place
    /// ([`Mixer::needs_input`]): the instrument then rests (see
    /// [`TrackHost::render`]).
    fn wants(&self, _track: usize) -> bool {
        true
    }
}

impl Outputs for Mixer {
    fn track(&mut self, track: usize) -> Option<(&mut [f32], &mut [f32])> {
        self.track_input(track)
    }

    fn wants(&self, track: usize) -> bool {
        self.needs_input(mpfree_fx::StripId::Track(track))
    }
}

/// Every track into one stereo pair (a host with no mixer, and tests).
pub struct Sum<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

impl Outputs for Sum<'_> {
    fn track(&mut self, _track: usize) -> Option<(&mut [f32], &mut [f32])> {
        Some((&mut *self.left, &mut *self.right))
    }
}

/// One slot's counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotStats {
    /// Notes started.
    pub notes: u64,
    /// Events that did not fit in a period's queue (should stay 0).
    pub dropped: u64,
    /// Note-offs for a note nobody held (harmless; counted to spot bugs).
    pub stray_offs: u64,
    /// Blocks dropped for a non-finite sample (should stay 0).
    pub resets: u64,
    /// Instruments swapped out and rung out on this slot.
    pub swaps: u64,
    /// Blocks the slot's instrument rested through, its track frozen.
    pub rested: u64,
}

/// What [`TrackHost::swap`] gives back when it cannot take the instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapError {
    /// No such slot.
    NoSlot,
    /// An instrument is still ringing out and the retired list is full:
    /// call [`TrackHost::pop_retired`] first.
    RetiredFull,
}

/// An old instrument ringing out after a swap.
struct Ringing {
    inst: Instrument,
    /// AllOff still to be sent (at the start of the next render).
    off: bool,
    frames: u32,
    quiet: u32,
    limit: u32,
}

struct Slot {
    inst: Option<Instrument>,
    ringing: Option<Ringing>,
    track: Option<usize>,
    events: Vec<Timed>,
    /// How many notes each note number has sounding (pads, clips, the arp).
    held: [u8; 128],
    /// The slot's tuning, given to each instrument that takes the slot.
    tuning: Tuning,
    left: Vec<f32>,
    right: Vec<f32>,
    stats: SlotStats,
    /// Its track wanted nothing last block (frozen): the instrument
    /// rested, and gets an AllOff before it plays again.
    resting: bool,
}

/// The track host. See the crate docs.
pub struct TrackHost {
    slots: Vec<Slot>,
    rate: u32,
    max_block: usize,
    retired: Vec<Instrument>,
    /// STOP's fade: frames left, while `fading`.
    fade: u32,
    fading: bool,
    profiling: bool,
    /// Frames a render was asked for past `max_block` (should stay 0).
    clipped_frames: u64,
}

impl TrackHost {
    /// A host with `slots` destinations (slot `i` feeding track `i`), for
    /// blocks of at most `max_block` frames. Allocates.
    pub fn new(slots: usize, rate: u32, max_block: usize) -> TrackHost {
        let slots = (0..slots)
            .map(|i| Slot {
                inst: None,
                ringing: None,
                track: Some(i),
                events: Vec::with_capacity(MAX_EVENTS),
                held: [0; 128],
                tuning: Tuning::standard(),
                left: vec![0.0; max_block],
                right: vec![0.0; max_block],
                stats: SlotStats::default(),
                resting: false,
            })
            .collect::<Vec<_>>();
        let retired = Vec::with_capacity(2 * slots.len() + 2);
        TrackHost {
            slots,
            rate,
            max_block,
            retired,
            fade: 0,
            fading: false,
            profiling: false,
            clipped_frames: 0,
        }
    }

    pub fn slots(&self) -> usize {
        self.slots.len()
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn max_block(&self) -> usize {
        self.max_block
    }

    // ------------------------------------------------------------ instruments

    /// Puts `inst` in slot `slot` (None empties it), in the slot's tuning.
    /// The old instrument rings out into the same track and is retired
    /// once quiet; one still ringing from an earlier swap is retired at
    /// once. Never allocates.
    pub fn swap(
        &mut self,
        slot: usize,
        inst: Option<Instrument>,
    ) -> Result<(), (SwapError, Option<Instrument>)> {
        let Some(s) = self.slots.get_mut(slot) else {
            return Err((SwapError::NoSlot, inst));
        };
        if s.ringing.is_some() && self.retired.len() == self.retired.capacity() {
            return Err((SwapError::RetiredFull, inst));
        }
        if let Some(old) = s.ringing.take() {
            self.retired.push(old.inst);
        }
        if let Some(old) = s.inst.take() {
            let tail = old.node.tail_frames();
            let limit = tail.saturating_add(QUIET_FRAMES).min(MAX_RING_FRAMES);
            s.ringing = Some(Ringing {
                inst: old,
                off: true,
                frames: 0,
                quiet: 0,
                limit,
            });
            s.stats.swaps += 1;
        }
        // Notes queued for the old instrument end with its AllOff.
        s.events.clear();
        s.held = [0; 128];
        s.inst = inst;
        if let Some(i) = s.inst.as_mut() {
            i.node.set_tuning(&s.tuning);
        }
        Ok(())
    }

    /// An instrument taken out (after its ring-out), to drop off the audio
    /// thread.
    pub fn pop_retired(&mut self) -> Option<Instrument> {
        self.retired.pop()
    }

    /// Whether slot `slot` has an old instrument still ringing out.
    pub fn ringing(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(|s| s.ringing.is_some())
    }

    pub fn instrument(&self, slot: usize) -> Option<&Instrument> {
        self.slots.get(slot)?.inst.as_ref()
    }

    /// The instrument's parameters, for the mod matrix and the screens.
    pub fn params(&self, slot: usize) -> &'static [ParamInfo] {
        self.instrument(slot).map_or(&[], |i| i.node.params())
    }

    /// A parameter's value now (None with no instrument in the slot).
    pub fn param_value(&self, slot: usize, id: u16) -> Option<f32> {
        self.instrument(slot).map(|i| i.node.param(id))
    }

    // ------------------------------------------------------------ tuning

    /// Slot `slot`'s tuning: how its instrument turns notes into pitch.
    /// The slot keeps it, so an instrument swapped in later plays in it
    /// too. It applies from the next block, and sounding notes move to it
    /// (smoothed where the engine smooths pitch). A copy into the slot and
    /// the instrument: never allocates, so it may run on the audio thread.
    /// An instrument still ringing out after a swap keeps the old tuning.
    pub fn set_tuning(&mut self, slot: usize, tuning: &Tuning) {
        let Some(s) = self.slots.get_mut(slot) else {
            return;
        };
        s.tuning.clone_from(tuning);
        if let Some(i) = s.inst.as_mut() {
            i.node.set_tuning(tuning);
        }
    }

    /// Slot `slot`'s tuning (12-TET at A4 = 440 Hz until set).
    pub fn tuning(&self, slot: usize) -> Option<&Tuning> {
        self.slots.get(slot).map(|s| &s.tuning)
    }

    // ------------------------------------------------------------ routing

    /// Slot `slot` feeds mixer track `track` (None: nowhere, silent).
    pub fn route(&mut self, slot: usize, track: Option<usize>) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.track = track;
        }
    }

    /// The mixer track slot `slot` feeds.
    pub fn track_of(&self, slot: usize) -> Option<usize> {
        self.slots.get(slot)?.track
    }

    /// The first slot holding an instrument that feeds mixer track
    /// `track` (an empty slot routed there doesn't count).
    pub fn slot_of_track(&self, track: usize) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.inst.is_some() && s.track == Some(track))
    }

    // ------------------------------------------------------------ events

    fn push(s: &mut Slot, t: Timed) -> bool {
        if s.events.len() < s.events.capacity() {
            s.events.push(t);
            true
        } else {
            s.stats.dropped += 1;
            false
        }
    }

    /// An event for slot `slot`, `offset` frames into the next render.
    /// Note-ons and note-offs are counted per note (see the crate docs);
    /// `AllOff` ends every note. Returns false if it was dropped.
    pub fn event(&mut self, slot: usize, t: Timed) -> bool {
        let Some(s) = self.slots.get_mut(slot) else {
            return false;
        };
        match t.ev {
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => {
                let n = usize::from(note & 0x7F);
                if !Self::push(s, t) {
                    return false;
                }
                s.held[n] = s.held[n].saturating_add(1);
                s.stats.notes += 1;
                true
            }
            NodeEvent::NoteOn { note, .. } | NodeEvent::NoteOff { note } => {
                let n = usize::from(note & 0x7F);
                match s.held[n] {
                    0 => {
                        s.stats.stray_offs += 1;
                        true
                    }
                    1 => {
                        // Only the last holder's note-off reaches the node;
                        // if it is dropped the count stays, so a later
                        // AllOff or note-off still ends it.
                        let off = Timed::new(t.offset, NodeEvent::NoteOff { note });
                        let sent = Self::push(s, off);
                        if sent {
                            s.held[n] = 0;
                        }
                        sent
                    }
                    _ => {
                        s.held[n] -= 1;
                        true
                    }
                }
            }
            NodeEvent::AllOff => {
                s.held = [0; 128];
                Self::push(s, t)
            }
            _ => Self::push(s, t),
        }
    }

    pub fn note_on(&mut self, slot: usize, offset: u32, note: u8, velocity: u8) -> bool {
        self.event(
            slot,
            Timed::new(offset, NodeEvent::NoteOn { note, velocity }),
        )
    }

    pub fn note_off(&mut self, slot: usize, offset: u32, note: u8) -> bool {
        self.event(slot, Timed::new(offset, NodeEvent::NoteOff { note }))
    }

    /// A parameter change on an exact frame.
    pub fn param(&mut self, slot: usize, offset: u32, id: u16, value: f32) -> bool {
        self.event(slot, Timed::new(offset, NodeEvent::Param { id, value }))
    }

    /// Every note on every slot ends with its release.
    pub fn all_off(&mut self) {
        for slot in 0..self.slots.len() {
            self.event(slot, Timed::new(0, NodeEvent::AllOff));
        }
    }

    /// STOP: every slot ends its notes and fades out over [`STOP_FADE`]
    /// frames, then is reset; instruments ringing out are retired when the
    /// fade ends.
    pub fn stop(&mut self) {
        self.all_off();
        if !self.fading {
            self.fading = true;
            self.fade = STOP_FADE;
        }
    }

    // ------------------------------------------------------------ sound

    /// Adds every slot's next `frames` frames into its track's buffers in
    /// `out`. `ctx` is the block's context (its `sidechain` is not passed
    /// on). At most [`TrackHost::max_block`] frames are rendered; the rest
    /// are counted. Never allocates.
    pub fn render(&mut self, ctx: &Ctx, frames: usize, out: &mut dyn Outputs) {
        let n = frames.min(self.max_block);
        self.clipped_frames += (frames - n) as u64;
        let ctx = Ctx {
            sidechain: None,
            ..*ctx
        };
        let fade0 = self.fade;
        let fading = self.fading;
        let profiling = self.profiling;
        for s in &mut self.slots {
            let events = &mut s.events;
            // Stable insertion sort by frame (few events, no allocation);
            // late events land on the last frame.
            for e in events.iter_mut() {
                e.offset = e.offset.min(n.saturating_sub(1) as u32);
            }
            for i in 1..events.len() {
                let mut j = i;
                while j > 0 && events[j - 1].offset > events[j].offset {
                    events.swap(j - 1, j);
                    j -= 1;
                }
            }
            if n == 0 {
                events.clear();
                continue;
            }
            // A frozen track: the mixer plays its take, so the instrument
            // rests. Its notes are dropped and nothing counts as held; an
            // old instrument ringing out is retired at once (its sound is
            // in the take).
            if !s.track.is_none_or(|t| out.wants(t)) {
                events.clear();
                s.held = [0; 128];
                s.resting = true;
                s.stats.rested += 1;
                if self.retired.len() < self.retired.capacity() {
                    if let Some(r) = s.ringing.take() {
                        self.retired.push(r.inst);
                    }
                }
                continue;
            }
            // Thawed: whatever it held when it went to rest ends first.
            if std::mem::take(&mut s.resting) {
                if events.len() < events.capacity() {
                    events.insert(0, Timed::new(0, NodeEvent::AllOff));
                } else if let Some(e) = events.first_mut() {
                    *e = Timed::new(0, NodeEvent::AllOff);
                }
            }
            let (sl, sr) = (&mut s.left[..n], &mut s.right[..n]);
            let mut wrote = false;
            if let Some(inst) = s.inst.as_mut() {
                sl.fill(0.0);
                sr.fill(0.0);
                let span = profiling.then(Span::start);
                inst.node.process(&ctx, events, sl, sr);
                if let Some(span) = span {
                    inst.probe.stop(span, n as u32);
                }
                if sl.iter().chain(sr.iter()).all(|x| x.is_finite()) {
                    wrote = true;
                } else {
                    inst.node.reset();
                    s.held = [0; 128];
                    s.stats.resets += 1;
                }
            }
            events.clear();
            if wrote {
                Self::add(s.track, sl, sr, fading, fade0, out);
            }
            if let Some(r) = s.ringing.as_mut() {
                sl.fill(0.0);
                sr.fill(0.0);
                let off = [Timed::new(0, NodeEvent::AllOff)];
                let ev: &[Timed] = if r.off { &off } else { &[] };
                r.off = false;
                r.inst.node.process(&ctx, ev, sl, sr);
                let peak = sl.iter().chain(sr.iter()).fold(0f32, |m, x| m.max(x.abs()));
                let finite = peak.is_finite();
                if finite {
                    Self::add(s.track, sl, sr, fading, fade0, out);
                }
                r.frames = r.frames.saturating_add(n as u32);
                r.quiet = if finite && peak < QUIET_LEVEL {
                    r.quiet.saturating_add(n as u32)
                } else {
                    0
                };
                // With the retired list full it rings on (quietly) until
                // the control thread makes room.
                let done = !finite || r.quiet >= QUIET_FRAMES || r.frames >= r.limit;
                if done && self.retired.len() < self.retired.capacity() {
                    if let Some(r) = s.ringing.take() {
                        self.retired.push(r.inst);
                    }
                }
            }
        }
        if self.fading {
            self.fade = fade0.saturating_sub(n as u32);
            if self.fade == 0 {
                self.fading = false;
                for s in &mut self.slots {
                    if let Some(inst) = s.inst.as_mut() {
                        inst.node.reset();
                    }
                    s.held = [0; 128];
                    if self.retired.len() < self.retired.capacity() {
                        if let Some(r) = s.ringing.take() {
                            self.retired.push(r.inst);
                        }
                    } else if let Some(r) = s.ringing.as_mut() {
                        r.inst.node.reset();
                    }
                }
            }
        }
    }

    fn add(
        track: Option<usize>,
        sl: &mut [f32],
        sr: &mut [f32],
        fading: bool,
        fade0: u32,
        out: &mut dyn Outputs,
    ) {
        if fading {
            for (i, (l, r)) in sl.iter_mut().zip(sr.iter_mut()).enumerate() {
                let g = fade0.saturating_sub(i as u32) as f32 / STOP_FADE as f32;
                *l *= g;
                *r *= g;
            }
        }
        let Some((ol, or)) = track.and_then(|t| out.track(t)) else {
            return;
        };
        for (o, x) in ol.iter_mut().zip(sl.iter()) {
            *o += *x;
        }
        for (o, x) in or.iter_mut().zip(sr.iter()) {
            *o += *x;
        }
    }

    // ------------------------------------------------------------ counters

    pub fn stats(&self, slot: usize) -> SlotStats {
        self.slots
            .get(slot)
            .map_or_else(SlotStats::default, |s| s.stats)
    }

    /// Frames asked of [`TrackHost::render`] past the block size (should
    /// stay 0).
    pub fn clipped_frames(&self) -> u64 {
        self.clipped_frames
    }

    /// Times each instrument's block into its probe (two clock reads per
    /// slot per block) while on.
    pub fn set_profiling(&mut self, on: bool) {
        self.profiling = on;
    }

    /// A slot's instrument timing, to take its window
    /// ([`Probe::take_window`]). The probe moves with its instrument.
    pub fn probe_mut(&mut self, slot: usize) -> Option<&mut Probe> {
        Some(&mut *self.slots.get_mut(slot)?.inst.as_mut()?.probe)
    }
}

/// The sequencer's notes: [`Out::dest`] is the slot; `AllOff` to [`ALL`]
/// reaches every slot.
impl Sink for TrackHost {
    fn send(&mut self, out: Out) {
        let ev = match out.event {
            Event::NoteOn { note, velocity } => NodeEvent::NoteOn { note, velocity },
            Event::NoteOff { note } => NodeEvent::NoteOff { note },
            Event::AllOff => NodeEvent::AllOff,
            // A clip's pad pressure: the raw value, which the node's own
            // pressure curve shapes.
            Event::Pressure { note, value } => NodeEvent::Pressure { note, value },
            // Parameter locks and moves go to the mod resolver
            // (`mpfree-mod`), which knows each parameter's own value to go
            // back to on Unlock and sends the instruments their values; not
            // straight to a node.
            Event::Lock { .. } | Event::Lane { .. } | Event::Unlock { .. } => return,
        };
        if out.dest == ALL {
            for slot in 0..self.slots.len() {
                self.event(slot, Timed::new(out.offset, ev));
            }
        } else {
            self.event(usize::from(out.dest), Timed::new(out.offset, ev));
        }
    }
}
