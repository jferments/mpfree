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
//!   with [`TrackHost::param_value`]. A lock's value put back when it ends
//!   is [`TrackHost::unlock`], asked for with [`TrackHost::hold`] before
//!   each new lock.
//! - **Endings always get in** (step sequencer refactor A2b): each slot's
//!   queue takes events by the rule of `mpfree_node::room` ([`ROOM`]). New
//!   notes and values have room of their own and are left out of a period
//!   too busy for them (counted, [`SlotStats::dropped`]); a note-off for a
//!   note the slot owes, a pedal coming up, a lock put back and a panic
//!   always find a place ([`SlotStats::ends_lost`] counts one that didn't:
//!   a bug).
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
//! - **Other cores** ([`TrackHost::set_lanes`]): the slots can be split into
//!   groups, one rendered on the calling thread and one on each lane
//!   (`mpfree-lanes`, a worker thread on its own core). Each period the
//!   lanes are posted with their slots' instruments, buffers and events,
//!   the calling thread renders its own group, joins the lanes by the
//!   period's deadline ([`TrackHost::set_deadline`]), and then adds every
//!   slot's sound into its track in slot order, so the sound is the same
//!   to the bit with or without lanes. A slot moves between groups only
//!   between periods: twice a second the slots are placed again by what
//!   their renders cost, the most costly first, each where the least is
//!   placed (the lanes before the calling thread on a tie, as it has the
//!   rest of the period's work), and only if the busiest group gets
//!   clearly lighter. A lane
//!   that misses the deadline leaves its slots out of that period
//!   (counted, [`TrackHost::lane_counts`]) and finishes on its own;
//!   [`TrackHost::settle`] takes them back after the write.
//!
//! # Real-time rules
//!
//! [`TrackHost::new`] and [`Instrument::new`] allocate and belong on the
//! control thread. Everything else never allocates, locks or panics: the
//! event queues ([`MAX_EVENTS`] per slot per period, all its [`ROOM`]
//! lets in), the render buffers and the retired list are all made in
//! `new`. The output is the same, sample for sample, for any block size;
//! the one exception is the moment a ringing-out instrument is retired,
//! which lands on a block boundary (it is inaudible by then).
#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use mpfree_fx::Mixer;
use mpfree_lanes::{Config as LaneConfig, Job, Lane, Setup as LaneSetup};
use mpfree_node::pedal::{NoteSet, Pedals};
use mpfree_node::room::{Limits, NodeRoom};
use mpfree_node::{Ctl, Ctx, Cull, Node, NodeEvent, ParamInfo, Timed, Tuning};
use mpfree_play::{Event, Out, Sink, ALL};
use mpfree_prof::{Probe, Span};

/// What one period of a slot's queue takes (refactor A2b): new notes and
/// values have room of their own; an ending always finds a place. Unlocks:
/// two for each of the 128 locks a lock table holds at once (the app's
/// `locks::HOLDS`), as its host asks [`TrackHost::hold`] before each new
/// one.
pub const ROOM: Limits = Limits {
    starts: 128,
    values: 256,
    ends: 256,
    unlocks: 256,
    all_offs: 16,
};
/// Events one slot can take in one period: all its room lets in, and the
/// `AllOff` a resting instrument gets as it plays again.
pub const MAX_EVENTS: usize = ROOM.room() + 1;
/// STOP's fade, in frames (10 ms at 44.1 kHz).
pub const STOP_FADE: u32 = 441;
/// A ringing-out instrument is retired after this many frames in a row
/// below [`QUIET_LEVEL`] (about 93 ms at 44.1 kHz).
pub const QUIET_FRAMES: u32 = 4096;
/// The level a ringing-out instrument counts as quiet below (-100 dBFS).
pub const QUIET_LEVEL: f32 = 1e-5;
/// The longest an old instrument rings out (20 s at 44.1 kHz).
pub const MAX_RING_FRAMES: u32 = 882_000;
/// With lanes, the slots are placed again every this many renders (about
/// half a second at 128 frames and 44.1 kHz).
pub const PLACE_EVERY: u32 = 172;
/// A new placement is taken only if its busiest group costs at most this
/// share of the busiest group now.
pub const PLACE_GAIN: f32 = 0.8;
/// What a slot's render counts as at least when it is placed, ns: an
/// instrument not timed yet still has a place of its own.
const PLACE_FLOOR_NS: f32 = 1_000.0;

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
    /// New notes, values and locks left out of a period too busy for them
    /// (refactor A2b): something new gave way so nothing hangs.
    pub dropped: u64,
    /// Endings (note-offs, pedals up, locks put back, panics) that found no
    /// room: a bug, which must stay 0.
    pub ends_lost: u64,
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

/// What a slot renders with: its instruments, the period's events and the
/// buffers they render into. It moves whole to a lane for a period and
/// back, so nothing in it is shared between threads.
#[derive(Default)]
struct Work {
    inst: Option<Instrument>,
    ringing: Option<Ringing>,
    /// The period's events, sorted (the slot's queue, swapped in).
    events: Vec<Timed>,
    left: Vec<f32>,
    right: Vec<f32>,
    /// The old instrument ringing out renders here.
    ring_left: Vec<f32>,
    ring_right: Vec<f32>,
    /// What the period's render did, for the host to act on at home.
    wrote: bool,
    ring_wrote: bool,
    reset: bool,
    ring_done: bool,
    /// The render's time, ns, smoothed over about 20 periods (timed only
    /// while the host has lanes).
    cost: f32,
}

impl Work {
    /// The period's render of `n` frames, on whichever thread holds it.
    fn render(&mut self, ctx: &Ctx, n: usize, profiling: bool, timing: bool) {
        let t = timing.then(Instant::now);
        (self.wrote, self.ring_wrote, self.reset, self.ring_done) = (false, false, false, false);
        let (sl, sr) = (&mut self.left[..n], &mut self.right[..n]);
        if let Some(inst) = self.inst.as_mut() {
            sl.fill(0.0);
            sr.fill(0.0);
            let span = profiling.then(Span::start);
            inst.node.process(ctx, &self.events, sl, sr);
            if let Some(span) = span {
                inst.probe.stop(span, n as u32);
            }
            if sl.iter().chain(sr.iter()).all(|x| x.is_finite()) {
                self.wrote = true;
            } else {
                inst.node.reset();
                self.reset = true;
            }
        }
        self.events.clear();
        if let Some(r) = self.ringing.as_mut() {
            let (sl, sr) = (&mut self.ring_left[..n], &mut self.ring_right[..n]);
            sl.fill(0.0);
            sr.fill(0.0);
            let off = [Timed::new(0, NodeEvent::AllOff)];
            let ev: &[Timed] = if r.off { &off } else { &[] };
            r.off = false;
            r.inst.node.process(ctx, ev, sl, sr);
            let peak = sl.iter().chain(sr.iter()).fold(0f32, |m, x| m.max(x.abs()));
            let finite = peak.is_finite();
            self.ring_wrote = finite;
            r.frames = r.frames.saturating_add(n as u32);
            r.quiet = if finite && peak < QUIET_LEVEL {
                r.quiet.saturating_add(n as u32)
            } else {
                0
            };
            self.ring_done = !finite || r.quiet >= QUIET_FRAMES || r.frames >= r.limit;
        }
        if let Some(t) = t {
            let ns = t.elapsed().as_nanos() as f32;
            self.cost += (ns - self.cost) * 0.05;
        }
    }

    /// Something to render: an instrument, or an old one ringing out.
    fn busy(&self) -> bool {
        self.inst.is_some() || self.ringing.is_some()
    }
}

struct Slot {
    work: Work,
    /// The group that renders it: 0 the calling thread, `k` lane `k - 1`.
    group: usize,
    /// Its work is on a lane that missed the deadline.
    away: bool,
    /// It renders this period (not frozen, the block not empty).
    go: bool,
    /// STOP's fade ended while its work was away: reset it on its return.
    reset_due: bool,
    track: Option<usize>,
    /// The events queued for the next render.
    events: Vec<Timed>,
    /// Which events go in the queue, so an ending always fits.
    room: NodeRoom,
    /// How many notes each note number has sounding (pads, clips, the arp).
    held: [u8; 128],
    /// The slot's tuning, given to each instrument that takes the slot.
    tuning: Tuning,
    stats: SlotStats,
    /// Its track wanted nothing last block (frozen): the instrument
    /// rested, and gets an AllOff before it plays again.
    resting: bool,
    /// The pedals, for an instrument that doesn't hear them
    /// (`Node::hears`): the host holds its note-offs while a pedal keeps
    /// the notes.
    pedals: Pedals,
    /// Whether the instrument hears the pedals itself, asked once when it
    /// takes the slot rather than on every event.
    hears_pedal: bool,
    /// Events reach the instrument as they came: it hears the pedals, or
    /// both are up. Kept with them, so a note checks one flag.
    plain: bool,
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
    /// The lanes, one per group past the first (none: every slot here).
    lanes: Vec<HostLane>,
    /// When the lanes' joins stop waiting (None: a second from the join).
    deadline: Option<Instant>,
    /// Renders since the slots were last placed.
    since_place: u32,
    /// Slots moved to another group since the lanes started.
    moves: u64,
    /// Scratch for placing: slot numbers, each slot's planned group, each
    /// group's load (made with the lanes).
    order: Vec<usize>,
    plan: Vec<usize>,
    loads: Vec<f32>,
}

/// A lane's slots for one period.
struct Group {
    works: Vec<Work>,
    /// The slot each of `works` came from.
    index: Vec<usize>,
    ctx: Ctx<'static>,
    n: usize,
    profiling: bool,
}

impl Job for Group {
    fn run(&mut self, _block: u64) {
        for w in &mut self.works {
            w.render(&self.ctx, self.n, self.profiling, true);
        }
    }
}

struct HostLane {
    lane: Lane<Group>,
    posted: bool,
    away: bool,
    counts: LaneCounts,
}

/// A lane's counts since it started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaneCounts {
    /// Periods it was posted.
    pub periods: u64,
    /// Periods it missed the deadline: its slots were left out.
    pub late: u64,
    /// Times the host had to wait past a deadline to take its slots back.
    pub waits: u64,
}

impl TrackHost {
    /// A host with `slots` destinations (slot `i` feeding track `i`), for
    /// blocks of at most `max_block` frames. Allocates.
    pub fn new(slots: usize, rate: u32, max_block: usize) -> TrackHost {
        let slots = (0..slots)
            .map(|i| Slot {
                work: Work {
                    events: Vec::with_capacity(MAX_EVENTS),
                    left: vec![0.0; max_block],
                    right: vec![0.0; max_block],
                    ring_left: vec![0.0; max_block],
                    ring_right: vec![0.0; max_block],
                    ..Work::default()
                },
                group: 0,
                away: false,
                go: false,
                reset_due: false,
                track: Some(i),
                events: Vec::with_capacity(MAX_EVENTS),
                room: NodeRoom::new(ROOM),
                held: [0; 128],
                tuning: Tuning::standard(),
                stats: SlotStats::default(),
                resting: false,
                pedals: Pedals::new(),
                hears_pedal: false,
                plain: true,
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
            lanes: Vec::new(),
            deadline: None,
            since_place: 0,
            moves: 0,
            order: Vec::new(),
            plan: Vec::new(),
            loads: Vec::new(),
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
        if slot >= self.slots.len() {
            return Err((SwapError::NoSlot, inst));
        }
        self.home();
        let least = self.least_loaded();
        let s = &mut self.slots[slot];
        if s.work.ringing.is_some() && self.retired.len() == self.retired.capacity() {
            return Err((SwapError::RetiredFull, inst));
        }
        if let Some(old) = s.work.ringing.take() {
            self.retired.push(old.inst);
        }
        if let Some(old) = s.work.inst.take() {
            let tail = old.node.tail_frames();
            let limit = tail.saturating_add(QUIET_FRAMES).min(MAX_RING_FRAMES);
            s.work.ringing = Some(Ringing {
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
        s.room.clear();
        s.pedals.clear_notes();
        // A new instrument starts where the least is placed (an old one
        // ringing out stays with it).
        if s.work.ringing.is_none() {
            s.group = least;
        }
        s.work.inst = inst;
        s.hears_pedal = s
            .work
            .inst
            .as_ref()
            .is_some_and(|i| i.node.hears(Ctl::Sustain));
        s.plain = s.hears_pedal || s.pedals.idle();
        if let Some(i) = s.work.inst.as_mut() {
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
        self.slots
            .get(slot)
            .is_some_and(|s| s.work.ringing.is_some())
    }

    /// Slot `slot`'s instrument (None while a late lane still has it).
    pub fn instrument(&self, slot: usize) -> Option<&Instrument> {
        self.slots.get(slot)?.work.inst.as_ref()
    }

    // ------------------------------------------------------------ load shedding

    /// Voices sounding in every slot's instrument ([`Node::sounding_voices`]; a
    /// plugin counts none), not counting old ones ringing out.
    pub fn voices(&self) -> usize {
        self.slots
            .iter()
            .filter_map(|s| s.work.inst.as_ref())
            .map(|i| i.node.sounding_voices())
            .sum()
    }

    /// The voice to give up first across the slots (the least audible,
    /// [`Cull::audibility`]), with its slot. Old instruments ringing out
    /// are left be: they end on their own. No allocation.
    pub fn cull_candidate(&self) -> Option<(usize, Cull)> {
        let mut best = None;
        for (i, s) in self.slots.iter().enumerate() {
            if let Some(c) = s.work.inst.as_ref().and_then(|x| x.node.cull_candidate()) {
                best = Cull::first(best, Some((i, c)));
            }
        }
        best
    }

    /// Gives up the voice slot `slot`'s instrument names
    /// ([`Node::cull`]). True if it did. No allocation.
    pub fn cull(&mut self, slot: usize) -> bool {
        self.home();
        self.slots
            .get_mut(slot)
            .and_then(|s| s.work.inst.as_mut())
            .is_some_and(|i| i.node.cull())
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
        self.home();
        let Some(s) = self.slots.get_mut(slot) else {
            return;
        };
        s.tuning.clone_from(tuning);
        if let Some(i) = s.work.inst.as_mut() {
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
            .position(|s| s.work.inst.is_some() && s.track == Some(track))
    }

    // ------------------------------------------------------------ events

    fn push(s: &mut Slot, t: Timed) -> bool {
        Self::push_to(&mut s.events, &mut s.room, &mut s.stats, t)
    }

    /// Queues `t` if the slot's room takes it: a new note or value in a
    /// period too busy is left out (counted by the room), an ending always
    /// goes in. The queue holds all the room lets in, so one that doesn't
    /// fit is an ending lost, counted as the bug it is.
    fn push_to(
        events: &mut Vec<Timed>,
        room: &mut NodeRoom,
        stats: &mut SlotStats,
        t: Timed,
    ) -> bool {
        if !room.admit(&t.ev) {
            return false;
        }
        Self::store(events, stats, t)
    }

    fn store(events: &mut Vec<Timed>, stats: &mut SlotStats, t: Timed) -> bool {
        if events.len() < events.capacity() {
            events.push(t);
            true
        } else {
            debug_assert!(false, "the room let in more than the queue holds");
            stats.ends_lost += 1;
            false
        }
    }

    /// Queues `t` for the slot's instrument: as it came, or through the
    /// slot's [`Pedals`] (`PEDALLED`), which hold note-offs while a pedal
    /// keeps the notes. False if anything was dropped.
    #[inline(always)]
    fn send<const PEDALLED: bool>(s: &mut Slot, t: Timed) -> bool {
        if !PEDALLED {
            return Self::push(s, t);
        }
        let Slot {
            pedals,
            events,
            room,
            stats,
            ..
        } = s;
        let mut all = true;
        pedals.filter(t.ev, |ev| {
            all &= Self::push_to(events, room, stats, Timed::new(t.offset, ev));
        });
        all
    }

    /// An event for slot `slot`, `offset` frames into the next render.
    /// Note-ons and note-offs are counted per note (see the crate docs);
    /// `AllOff` ends every note. Returns false if it was dropped.
    pub fn event(&mut self, slot: usize, t: Timed) -> bool {
        let Some(s) = self.slots.get_mut(slot) else {
            return false;
        };
        // The usual case first, so a plain note costs what it did before
        // the pedals: one flag. A pedal moving leaves it in `count`.
        if s.plain {
            Self::count::<false>(s, t)
        } else {
            Self::event_pedalled(s, t)
        }
    }

    /// [`TrackHost::event`] while a pedal is down or moving, out of line.
    #[cold]
    #[inline(never)]
    fn event_pedalled(s: &mut Slot, t: Timed) -> bool {
        if s.hears_pedal {
            // The instrument's own pedal: it goes as it came.
            return Self::push(s, t);
        }
        if s.pedals.idle() {
            // A pedal goes down. The quick path doesn't tell the pedals
            // which keys are down; the counts know.
            let mut keys = NoteSet::new();
            for (n, &c) in s.held.iter().enumerate() {
                if c > 0 {
                    keys.put(n as u8);
                }
            }
            s.pedals.set_keys(keys);
        }
        let sent = Self::count::<true>(s, t);
        s.plain = s.pedals.idle();
        sent
    }

    /// Counts `t` against the slot's notes and sends what reaches the
    /// instrument ([`TrackHost::send`]).
    #[inline(always)]
    fn count<const PEDALLED: bool>(s: &mut Slot, t: Timed) -> bool {
        match t.ev {
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => {
                let n = usize::from(note & 0x7F);
                // Counted as a holder even when a busy period leaves it
                // out, so its note-off pairs with it: the note ends with
                // its last holder, and the room leaves out a note-off for
                // a note it doesn't owe (refactor A2b).
                s.held[n] = s.held[n].saturating_add(1);
                let sent = Self::send::<PEDALLED>(s, t);
                if sent {
                    s.stats.notes += 1;
                }
                sent
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
                        // one for an owed note always has room.
                        s.held[n] = 0;
                        let off = Timed::new(t.offset, NodeEvent::NoteOff { note });
                        Self::send::<PEDALLED>(s, off)
                    }
                    _ => {
                        s.held[n] -= 1;
                        true
                    }
                }
            }
            NodeEvent::AllOff => {
                s.held = [0; 128];
                Self::send::<PEDALLED>(s, t)
            }
            NodeEvent::Control { .. } if !PEDALLED && Pedals::moves(&t.ev) => {
                Self::event_pedalled(s, t)
            }
            _ => Self::send::<PEDALLED>(s, t),
        }
    }

    /// Whether slot `slot` holds note `note` (a note-on not yet ended).
    pub fn holds_note(&self, slot: usize, note: u8) -> bool {
        self.slots
            .get(slot)
            .is_some_and(|s| s.held[usize::from(note & 0x7F)] > 0)
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

    /// A controller for slot `slot`'s instrument (a pedal, the mod wheel,
    /// breath) on an exact frame. For an instrument that doesn't hear the
    /// pedals the host holds its notes ([`Pedals`]).
    pub fn control(&mut self, slot: usize, offset: u32, ctl: Ctl, value: f32) -> bool {
        self.event(slot, Timed::new(offset, NodeEvent::Control { ctl, value }))
    }

    /// A parameter change on an exact frame.
    pub fn param(&mut self, slot: usize, offset: u32, id: u16, value: f32) -> bool {
        self.event(slot, Timed::new(offset, NodeEvent::Param { id, value }))
    }

    /// Whether slot `slot` may take a new lock (one not held before),
    /// asked before its first value goes in: so the value put back when it
    /// ends ([`TrackHost::unlock`]) always has a place. False in a period
    /// with as many new locks as [`ROOM`] lets in (counted).
    pub fn hold(&mut self, slot: usize) -> bool {
        self.slots.get_mut(slot).is_some_and(|s| s.room.hold())
    }

    /// A lock's end: parameter `id` back to `value` (the value under the
    /// locks) on an exact frame. An ending: it always goes in while the
    /// host asks [`TrackHost::hold`] before each new lock.
    pub fn unlock(&mut self, slot: usize, offset: u32, id: u16, value: f32) -> bool {
        let Some(s) = self.slots.get_mut(slot) else {
            return false;
        };
        if !s.room.unlock() {
            s.stats.ends_lost += 1;
            return false;
        }
        Self::store(
            &mut s.events,
            &mut s.stats,
            Timed::new(offset, NodeEvent::Param { id, value }),
        )
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
    /// are counted. With lanes ([`TrackHost::set_lanes`]) the lanes' groups
    /// render at the same time as this thread's, and a lane past the
    /// deadline leaves its slots out (see the crate docs). Never allocates.
    pub fn render(&mut self, ctx: &Ctx, frames: usize, out: &mut dyn Outputs) {
        let n = frames.min(self.max_block);
        self.clipped_frames += (frames - n) as u64;
        // A lane still out from a late period: its slots come back first.
        self.home();
        let ctx = Ctx {
            sidechain: None,
            ..*ctx
        };
        let fade0 = self.fade;
        let fading = self.fading;
        let profiling = self.profiling;
        for s in &mut self.slots {
            s.go = false;
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
            // No frames: the events wait for the next render, endings
            // and all (refactor A2b).
            if n == 0 {
                continue;
            }
            // A frozen track: the mixer plays its take, so the instrument
            // rests. Its notes are dropped and nothing counts as held; an
            // old instrument ringing out is retired at once (its sound is
            // in the take).
            if !s.track.is_none_or(|t| out.wants(t)) {
                events.clear();
                s.held = [0; 128];
                s.room.clear();
                s.room.period();
                s.pedals.clear_notes();
                s.resting = true;
                s.stats.rested += 1;
                if self.retired.len() < self.retired.capacity() {
                    if let Some(r) = s.work.ringing.take() {
                        self.retired.push(r.inst);
                    }
                }
                continue;
            }
            // Thawed: whatever it held when it went to rest ends first. Its
            // place is kept apart from the room's ([`MAX_EVENTS`]).
            if std::mem::take(&mut s.resting) {
                if events.len() < events.capacity() {
                    events.insert(0, Timed::new(0, NodeEvent::AllOff));
                } else {
                    debug_assert!(false, "the thaw's AllOff has a place");
                    s.stats.ends_lost += 1;
                }
            }
            // The period's events go with the work; the slot's queue is
            // the work's emptied one from last period, and a new period.
            std::mem::swap(&mut s.events, &mut s.work.events);
            s.room.period();
            s.go = true;
        }
        let timing = !self.lanes.is_empty();
        if timing {
            self.since_place += 1;
            if self.since_place >= PLACE_EVERY {
                self.place();
            }
        }
        // The lanes' groups go first, then this thread renders its own.
        let job_ctx = Ctx {
            rate: ctx.rate,
            frame: ctx.frame,
            bpm: ctx.bpm,
            beats: ctx.beats,
            playing: ctx.playing,
            sidechain: None,
        };
        let TrackHost { slots, lanes, .. } = self;
        for (k, l) in lanes.iter_mut().enumerate() {
            let Some(job) = l.lane.job() else {
                continue;
            };
            for (i, s) in slots.iter_mut().enumerate() {
                if s.go
                    && s.group == k + 1
                    && s.work.busy()
                    && job.works.len() < job.works.capacity()
                {
                    job.works.push(std::mem::take(&mut s.work));
                    job.index.push(i);
                    s.away = true;
                }
            }
            if job.works.is_empty() {
                continue;
            }
            (job.ctx, job.n, job.profiling) = (job_ctx, n, profiling);
            if l.lane.post(ctx.frame).is_ok() {
                l.posted = true;
                l.counts.periods += 1;
            } else {
                // Never reached (the job is home): render them here.
                Self::take_back(slots, l, false, &mut self.retired);
            }
        }
        for s in slots.iter_mut().filter(|s| s.go && !s.away) {
            s.work.render(&ctx, n, profiling, timing);
        }
        // The clock is read only when a lane was posted.
        let mut deadline = None;
        for l in lanes.iter_mut().filter(|l| l.posted) {
            l.posted = false;
            let until = *deadline.get_or_insert_with(|| {
                self.deadline
                    .unwrap_or_else(|| Instant::now() + Duration::from_secs(1))
            });
            if l.lane.join(until).is_ok() {
                Self::take_back(slots, l, false, &mut self.retired);
            } else {
                l.counts.late += 1;
                l.away = true;
            }
        }
        // Every slot's sound into its track, in slot order: the same sums
        // as with every slot here.
        for s in slots.iter_mut().filter(|s| s.go && !s.away) {
            let w = &mut s.work;
            if w.reset {
                s.held = [0; 128];
                s.room.clear();
                s.stats.resets += 1;
            }
            if w.wrote {
                Self::add(
                    s.track,
                    &mut w.left[..n],
                    &mut w.right[..n],
                    fading,
                    fade0,
                    out,
                );
            }
            if w.ring_wrote {
                Self::add(
                    s.track,
                    &mut w.ring_left[..n],
                    &mut w.ring_right[..n],
                    fading,
                    fade0,
                    out,
                );
            }
            // With the retired list full it rings on (quietly) until the
            // control thread makes room.
            if w.ring_done && self.retired.len() < self.retired.capacity() {
                if let Some(r) = w.ringing.take() {
                    self.retired.push(r.inst);
                }
            }
        }
        if self.fading {
            self.fade = fade0.saturating_sub(n as u32);
            if self.fade == 0 {
                self.fading = false;
                for s in &mut self.slots {
                    if s.away {
                        s.reset_due = true;
                    } else {
                        Self::end_fade(s, &mut self.retired);
                    }
                }
            }
        }
    }

    /// STOP's fade has ended: the slot's instrument is reset and an old
    /// one ringing out retired (reset, if the retired list is full).
    fn end_fade(s: &mut Slot, retired: &mut Vec<Instrument>) {
        if let Some(inst) = s.work.inst.as_mut() {
            inst.node.reset();
        }
        s.held = [0; 128];
        s.room.clear();
        if retired.len() < retired.capacity() {
            if let Some(r) = s.work.ringing.take() {
                retired.push(r.inst);
            }
        } else if let Some(r) = s.work.ringing.as_mut() {
            r.inst.node.reset();
        }
    }

    /// A joined lane's slots back home. `late`: their period's sound was
    /// left out, so only what the render did to the slot is kept (a reset,
    /// a ring-out that ended, STOP's reset due meanwhile).
    fn take_back(slots: &mut [Slot], l: &mut HostLane, late: bool, retired: &mut Vec<Instrument>) {
        let Some(job) = l.lane.job() else {
            return;
        };
        while let (Some(w), Some(i)) = (job.works.pop(), job.index.pop()) {
            let Some(s) = slots.get_mut(i) else {
                continue;
            };
            s.work = w;
            s.away = false;
            if late {
                if s.work.reset {
                    s.held = [0; 128];
                    s.room.clear();
                    s.stats.resets += 1;
                }
                if s.work.ring_done && retired.len() < retired.capacity() {
                    if let Some(r) = s.work.ringing.take() {
                        retired.push(r.inst);
                    }
                }
                if std::mem::take(&mut s.reset_due) {
                    Self::end_fade(s, retired);
                }
            }
        }
        l.away = false;
    }

    /// After a late period: takes the lanes' slots back, waiting until
    /// `deadline` at most (the next period's); past it this waits for them
    /// as long as it takes, since the next render needs them, and counts
    /// the wait. Call it right after each write: the write's own wait has
    /// usually given the lanes the time to finish. Does nothing after a
    /// period in time. Never allocates.
    pub fn settle(&mut self, deadline: Instant) {
        let TrackHost {
            slots,
            lanes,
            retired,
            ..
        } = self;
        for l in lanes.iter_mut().filter(|l| l.away) {
            if l.lane.join(deadline).is_err() {
                l.counts.waits += 1;
                while l
                    .lane
                    .join(Instant::now() + Duration::from_secs(1))
                    .is_err()
                {}
            }
            Self::take_back(slots, l, true, retired);
        }
    }

    /// Every slot home now, waiting for a late lane if need be (counted).
    fn home(&mut self) {
        if self.lanes.iter().any(|l| l.away) {
            self.settle(Instant::now());
        }
    }

    // ------------------------------------------------------------ lanes

    /// Splits the slots into groups: this thread's and one per lane in
    /// `configs` (none: every slot renders here again). Allocates and
    /// starts the lanes' threads: call it between renders, as the sound
    /// starts. Gives each lane's setup (where its worker runs, or why its
    /// jobs run here).
    pub fn set_lanes(&mut self, configs: Vec<LaneConfig>) -> Vec<LaneSetup> {
        self.home();
        let n = self.slots.len();
        self.lanes = configs
            .into_iter()
            .map(|c| HostLane {
                lane: Lane::new(
                    Group {
                        works: Vec::with_capacity(n),
                        index: Vec::with_capacity(n),
                        ctx: Ctx::new(self.rate, 0),
                        n: 0,
                        profiling: false,
                    },
                    c,
                ),
                posted: false,
                away: false,
                counts: LaneCounts::default(),
            })
            .collect();
        self.order = Vec::with_capacity(n);
        self.plan = vec![0; n];
        self.loads = vec![0.0; self.lanes.len() + 1];
        for s in &mut self.slots {
            s.group = 0;
        }
        // Spread at once, by what each has cost so far (nothing timed:
        // one each).
        self.place_now(true);
        self.lanes.iter().map(|l| l.lane.setup().clone()).collect()
    }

    /// When the lanes' joins stop waiting in the next renders (the
    /// period's one deadline). None: a second from the join.
    pub fn set_deadline(&mut self, at: Option<Instant>) {
        self.deadline = at;
    }

    /// Lanes beside this thread (0: every slot renders here).
    pub fn lanes(&self) -> usize {
        self.lanes.len()
    }

    /// Lane `k`'s counts since it started.
    pub fn lane_counts(&self, k: usize) -> Option<LaneCounts> {
        self.lanes.get(k).map(|l| l.counts)
    }

    /// The group rendering slot `slot`: 0 this thread, `k` lane `k - 1`.
    pub fn group_of(&self, slot: usize) -> Option<usize> {
        self.slots.get(slot).map(|s| s.group)
    }

    /// Slots moved between groups since the lanes started.
    pub fn moves(&self) -> u64 {
        self.moves
    }

    /// What slot `slot`'s render costs, ns, smoothed (timed while the host
    /// has lanes; 0 before).
    pub fn cost_ns(&self, slot: usize) -> f32 {
        self.slots.get(slot).map_or(0.0, |s| s.work.cost)
    }

    /// The group with the least placed on it now.
    fn least_loaded(&mut self) -> usize {
        if self.lanes.is_empty() {
            return 0;
        }
        self.loads.fill(0.0);
        for s in self.slots.iter().filter(|s| s.work.busy()) {
            self.loads[s.group] += s.work.cost.max(PLACE_FLOOR_NS);
        }
        least(&self.loads)
    }

    /// Places the slots again (see the crate docs), if every lane is home.
    fn place(&mut self) {
        self.since_place = 0;
        if self.lanes.iter().any(|l| l.away) {
            return;
        }
        self.place_now(false);
    }

    /// The most costly slot first, each on the group with the least placed
    /// so far; taken if the busiest group gets [`PLACE_GAIN`] lighter, or
    /// always with `force`. No allocation.
    fn place_now(&mut self, force: bool) {
        let cost = |s: &Slot| s.work.cost.max(PLACE_FLOOR_NS);
        self.loads.fill(0.0);
        for s in self.slots.iter().filter(|s| s.work.busy()) {
            self.loads[s.group] += cost(s);
        }
        let busiest_now = self.loads.iter().copied().fold(0.0, f32::max);
        self.order.clear();
        self.order
            .extend((0..self.slots.len()).filter(|&i| self.slots[i].work.busy()));
        let slots = &self.slots;
        self.order
            .sort_unstable_by(|&a, &b| cost(&slots[b]).total_cmp(&cost(&slots[a])).then(a.cmp(&b)));
        self.loads.fill(0.0);
        for &i in &self.order {
            let g = least(&self.loads);
            self.plan[i] = g;
            self.loads[g] += cost(&self.slots[i]);
        }
        let busiest = self.loads.iter().copied().fold(0.0, f32::max);
        if !force && busiest > busiest_now * PLACE_GAIN {
            return;
        }
        for &i in &self.order {
            let s = &mut self.slots[i];
            if s.group != self.plan[i] {
                s.group = self.plan[i];
                self.moves += 1;
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

    /// Slot `slot`'s counters, with what its room left out and lost.
    pub fn stats(&self, slot: usize) -> SlotStats {
        self.slots
            .get(slot)
            .map_or_else(SlotStats::default, |s| SlotStats {
                dropped: s.stats.dropped + s.room.refused,
                ends_lost: s.stats.ends_lost + s.room.lost,
                ..s.stats
            })
    }

    /// Endings every slot found no room for since the start: a bug, which
    /// must stay 0 (refactor A2b).
    pub fn ends_lost(&self) -> u64 {
        (0..self.slots.len()).map(|i| self.stats(i).ends_lost).sum()
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
        Some(&mut *self.slots.get_mut(slot)?.work.inst.as_mut()?.probe)
    }
}

/// The group with the least load; on a tie the last, so the lanes fill
/// before this thread, which has the rest of the period's work besides.
fn least(loads: &[f32]) -> usize {
    let mut best = loads.len().saturating_sub(1);
    for (g, &l) in loads.iter().enumerate().rev() {
        if l < loads[best] {
            best = g;
        }
    }
    best
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
            // straight to a node. A clip's program change is for outside
            // gear.
            Event::Lock { .. }
            | Event::Lane { .. }
            | Event::Unlock { .. }
            | Event::Program { .. } => return,
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
