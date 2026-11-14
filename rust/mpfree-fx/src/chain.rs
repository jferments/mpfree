//! A serial chain of inserts with click-free bypass and spillover.
//!
//! Bypass fades a slot's *input* out over 20 ms and its latency-matched dry
//! signal in, so a delay or reverb keeps ringing out after it is bypassed
//! (spillover) and a filter or drive crossfades to dry without a click. A
//! bypassed slot keeps being run on silence until its tail is over, then
//! rests (and is reset, so it starts clean when switched back on). A node
//! with an endless tail (a frozen delay) keeps playing while bypassed until
//! its tail becomes finite (freeze off); that change is checked once per
//! block, the one place where the block size can move a frame.
//!
//! The dry path of every slot is delayed by the slot's latency whether the
//! slot is on or off, so the chain's latency never changes when a slot is
//! bypassed and nothing moves in time.
//!
//! If a node ever puts out a non-finite sample, the chain resets it, plays
//! the slot's dry signal for that block, and counts it; nothing else in the
//! chain is touched.
//!
//! Building the chain ([`Chain::push`], [`Chain::prepare`]) allocates and
//! belongs on the control thread; everything else is real-time safe. To
//! change a chain while it plays, build an [`Insert`] on the control thread
//! and hand it over: [`Chain::insert`], [`Chain::replace`],
//! [`Chain::remove`] and [`Chain::move_slot`] never allocate as long as
//! the chain has room ([`Chain::with_capacity`]), and what they take out
//! comes back as an `Insert` to be dropped off the audio thread. They do
//! not fade by themselves: the mixer ducks a strip around them.
//!
//! Every slot has a **slot id** ([`Chain::uid`]): a number the chain gives
//! the insert when it goes in, which stays with it when it is moved,
//! bypassed or taken out and put back, so a parameter lock, a macro or a
//! Q-Link binding that names an insert (`mpfree_param::Owner::Insert`)
//! follows the effect, not the position. A project saves the ids and gives
//! them back on load ([`Insert::with_uid`]).
//!
//! Each slot can listen to its own sidechain ([`Chain::set_key`] and
//! [`Chain::process_keyed`]), and, while profiling is on
//! ([`Chain::set_profiling`]), times its block into an `mpfree_prof`
//! probe that moves with it.
//!
//! One slot at a time can be watched ([`Chain::watch`]): the chain keeps
//! its input and output peak levels, falling 20 dB a second, for the
//! level meters on the effect's page ([`Chain::levels`]). It follows the
//! slot id, so the meters stay on the effect when it moves.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_prof::{Probe, Span};

use crate::FixedDelay;

/// Events that can be queued for one slot per block.
pub const MAX_SLOT_EVENTS: usize = 256;
const FADE_MS: f32 = 20.0;
/// `tail_left` for a node whose tail never ends (`tail_frames` of
/// `u32::MAX`, such as a frozen delay): it keeps playing while bypassed.
const FOREVER: u64 = u64::MAX;

/// Counters kept by a [`Chain`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChainStats {
    /// Times a node put out a non-finite sample and was reset.
    pub nan_resets: u64,
    /// Queued events dropped because a slot's queue was full.
    pub dropped_events: u64,
}

struct Slot {
    node: Box<dyn Node>,
    on: bool,
    fade: f32,
    tail_left: u64,
    resting: bool,
    dry: [FixedDelay; 2],
    fade_delay: FixedDelay,
    events: Vec<Timed>,
    /// Which sidechain this slot listens to, a tag only the chain's owner
    /// understands (see [`Chain::process_keyed`]).
    key: Option<u16>,
    /// The slot's time per block while the chain is profiling; it moves
    /// with the slot.
    probe: Box<Probe>,
    /// The slot id (see the module docs); 0 until the chain gives one.
    uid: u16,
}

impl Slot {
    fn new(node: Box<dyn Node>) -> Slot {
        let lat = node.latency() as usize;
        Slot {
            node,
            on: true,
            fade: 1.0,
            tail_left: 0,
            resting: false,
            dry: [FixedDelay::new(lat), FixedDelay::new(lat)],
            fade_delay: fade_line(lat),
            events: Vec::with_capacity(MAX_SLOT_EVENTS),
            key: None,
            probe: Box::new(Probe::new(u64::MAX)),
            uid: 0,
        }
    }
}

/// A node wrapped and prepared as a chain slot, with its delay lines and
/// event queue, so a playing chain can take it without allocating. Build
/// it on the control thread.
pub struct Insert {
    slot: Slot,
    rate: u32,
    max_block: usize,
    /// The slot id it asks for; `None` takes the chain's next free one.
    uid: Option<u16>,
}

impl Insert {
    /// Prepares `node` for a chain running at `rate` with blocks of up to
    /// `max_block` frames. Allocates.
    pub fn new(mut node: Box<dyn Node>, rate: u32, max_block: usize) -> Insert {
        let (rate, max_block) = (rate.max(1), max_block.max(1));
        node.prepare(rate, max_block);
        Insert {
            slot: Slot::new(node),
            rate,
            max_block,
            uid: None,
        }
    }

    pub fn node(&self) -> &dyn Node {
        self.slot.node.as_ref()
    }
    pub fn node_mut(&mut self) -> &mut (dyn Node + 'static) {
        self.slot.node.as_mut()
    }
    pub fn is_on(&self) -> bool {
        self.slot.on
    }

    /// Starts the slot bypassed (it rests until switched on).
    pub fn off(mut self) -> Insert {
        let lat = self.slot.node.latency();
        self.slot.on = false;
        self.slot.fade = 0.0;
        for _ in 0..lat {
            self.slot.fade_delay.run(0.0);
        }
        self.slot.resting = true;
        self
    }

    /// Asks for slot id `uid` (loading a project). The chain refuses the
    /// insert if another slot has it. Ids start at 1.
    pub fn with_uid(mut self, uid: u16) -> Insert {
        self.uid = Some(uid.max(1));
        self
    }

    /// Lets the chain give it a new slot id (moving it to another chain).
    pub fn without_uid(mut self) -> Insert {
        self.uid = None;
        self
    }

    /// The slot id it has (taken out of a chain) or asks for.
    pub fn uid(&self) -> Option<u16> {
        self.uid
    }

    /// The node back, to drop or to wrap again elsewhere.
    pub fn into_node(self) -> Box<dyn Node> {
        self.slot.node
    }
}

fn fade_line(lat: usize) -> FixedDelay {
    let mut d = FixedDelay::new(lat);
    for _ in 0..lat {
        d.run(1.0);
    }
    d
}

/// A serial insert chain. See the module docs.
pub struct Chain {
    slots: Vec<Slot>,
    rate: u32,
    max_block: usize,
    step: f32,
    scratch: [Vec<f32>; 4],
    /// Events of a part of a block, re-timed (capacity `MAX_SLOT_EVENTS`).
    part: Vec<Timed>,
    stats: ChainStats,
    profiling: bool,
    /// Where the search for a free slot id starts.
    next_uid: u16,
    /// The watched slot's id, and its input and output peaks (linear).
    watch: Option<u16>,
    levels: [f32; 2],
}

impl Default for Chain {
    fn default() -> Self {
        Self::new()
    }
}

impl Chain {
    pub fn new() -> Chain {
        let mut c = Chain {
            slots: Vec::new(),
            rate: 48_000,
            max_block: 0,
            step: 0.0,
            scratch: Default::default(),
            part: Vec::with_capacity(MAX_SLOT_EVENTS),
            stats: ChainStats::default(),
            profiling: false,
            next_uid: 1,
            watch: None,
            levels: [0.0; 2],
        };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c
    }

    /// An empty chain with room for `slots` inserts, so [`Chain::insert`]
    /// never needs to allocate below that.
    pub fn with_capacity(slots: usize) -> Chain {
        let mut c = Chain::new();
        c.slots.reserve_exact(slots);
        c
    }

    /// How many slots the chain holds without allocating.
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }
    pub fn rate(&self) -> u32 {
        self.rate
    }
    pub fn max_block(&self) -> usize {
        self.max_block
    }

    /// Puts a prepared insert in at `at` (0 to `len`), shifting the later
    /// slots along. Real-time safe. Hands the insert back if the chain is
    /// full, `at` is past the end, it was prepared for another rate or
    /// block size, or it asks for a slot id ([`Insert::with_uid`]) another
    /// slot has.
    // Handing the value back, to be dropped off the audio thread, is the point.
    #[allow(clippy::result_large_err)]
    pub fn insert(&mut self, at: usize, ins: Insert) -> Result<(), Insert> {
        if at > self.slots.len() || self.slots.len() >= self.slots.capacity() || !self.fits(&ins) {
            return Err(ins);
        }
        let Some(uid) = self.claim_uid(ins.uid, None) else {
            return Err(ins);
        };
        let mut slot = ins.slot;
        slot.uid = uid;
        self.slots.insert(at, slot);
        Ok(())
    }

    /// Swaps the insert at `at` for `ins` and hands the old one back.
    /// Real-time safe. Hands `ins` back if `at` is not a slot, it does not
    /// fit, or it asks for a slot id another slot has. The old insert comes
    /// back with its slot id; the new one gets a new id unless it asked
    /// for one.
    // Handing the value back, to be dropped off the audio thread, is the point.
    #[allow(clippy::result_large_err)]
    pub fn replace(&mut self, at: usize, ins: Insert) -> Result<Insert, Insert> {
        if at >= self.slots.len() || !self.fits(&ins) {
            return Err(ins);
        }
        // A new effect gets a new id: what named the old one's parameters
        // means nothing to it. An id asked for may be the old one's.
        let Some(uid) = self.claim_uid(ins.uid, Some(at)) else {
            return Err(ins);
        };
        let (rate, max_block) = (self.rate, self.max_block);
        let mut slot = ins.slot;
        slot.uid = uid;
        let old = std::mem::replace(&mut self.slots[at], slot);
        let old_uid = old.uid;
        Ok(Insert {
            slot: old,
            rate,
            max_block,
            uid: Some(old_uid),
        })
    }

    /// Takes the insert at `at` out and hands it back. Real-time safe.
    pub fn remove(&mut self, at: usize) -> Option<Insert> {
        if at >= self.slots.len() {
            return None;
        }
        let slot = self.slots.remove(at);
        let uid = Some(slot.uid);
        Some(Insert {
            slot,
            rate: self.rate,
            max_block: self.max_block,
            uid,
        })
    }

    /// Moves the insert at `from` to `to`, with its state and queued
    /// events. Real-time safe. False if either is not a slot.
    pub fn move_slot(&mut self, from: usize, to: usize) -> bool {
        let n = self.slots.len();
        if from >= n || to >= n {
            return false;
        }
        if from < to {
            self.slots[from..=to].rotate_left(1);
        } else {
            self.slots[to..=from].rotate_right(1);
        }
        true
    }

    fn fits(&self, ins: &Insert) -> bool {
        ins.rate == self.rate && ins.max_block == self.max_block
    }

    /// The slot id for an insert going in: the one it asks for if no other
    /// slot (leaving out `except`) has it, else the next free one. Never
    /// allocates.
    fn claim_uid(&mut self, want: Option<u16>, except: Option<usize>) -> Option<u16> {
        let taken = |c: &Chain, u: u16| {
            c.slots
                .iter()
                .enumerate()
                .any(|(i, s)| s.uid == u && Some(i) != except)
        };
        if let Some(u) = want {
            if taken(self, u) {
                return None;
            }
            self.next_uid = self.next_uid.max(u.wrapping_add(1).max(1));
            return Some(u);
        }
        // At most MAX_SLOTS-ish slots, so a free id is always close by.
        let mut u = self.next_uid.max(1);
        while taken(self, u) {
            u = u.wrapping_add(1).max(1);
        }
        self.next_uid = u.wrapping_add(1).max(1);
        Some(u)
    }

    /// A slot's id (see the module docs).
    pub fn uid(&self, slot: usize) -> Option<u16> {
        self.slots.get(slot).map(|s| s.uid)
    }

    /// The slot that has id `uid` now.
    pub fn slot_of(&self, uid: u16) -> Option<usize> {
        self.slots.iter().position(|s| s.uid == uid)
    }

    /// Adds a node at the end, prepared for the chain's rate (control
    /// thread). Returns its slot index.
    pub fn push(&mut self, mut node: Box<dyn Node>) -> usize {
        node.prepare(self.rate, self.max_block);
        let mut slot = Slot::new(node);
        slot.uid = self.claim_uid(None, None).unwrap_or(0);
        self.slots.push(slot);
        self.slots.len() - 1
    }

    /// Watches slot `slot`'s levels for its page's meters, or none. The
    /// meters start from silence. Real-time safe.
    pub fn watch(&mut self, slot: Option<usize>) {
        self.watch = slot.and_then(|i| self.uid(i)).filter(|&u| u != 0);
        self.levels = [0.0; 2];
    }

    /// The watched slot (see [`Chain::watch`]), where it is now.
    pub fn watched(&self) -> Option<usize> {
        self.slot_of(self.watch?)
    }

    /// The watched slot's input and output peak levels in dBFS, falling
    /// 20 dB a second (-120 for silence); None when no slot is watched or
    /// it has left the chain.
    pub fn levels(&self) -> Option<(f32, f32)> {
        self.watched()?;
        let db = |x: f32| crate::fast_level_db(x).max(-120.0);
        Some((db(self.levels[0]), db(self.levels[1])))
    }

    /// Prepares every node and the chain's buffers (control thread).
    pub fn prepare(&mut self, rate: u32, max_block: usize) {
        self.rate = rate.max(1);
        self.max_block = max_block.max(1);
        self.step = 1.0 / (FADE_MS * 0.001 * self.rate as f32);
        self.scratch = [
            vec![0.0; self.max_block],
            vec![0.0; self.max_block],
            vec![0.0; self.max_block],
            vec![0.0; self.max_block],
        ];
        for s in &mut self.slots {
            s.node.prepare(self.rate, self.max_block);
            let lat = s.node.latency() as usize;
            s.dry = [FixedDelay::new(lat), FixedDelay::new(lat)];
            s.fade_delay = fade_line(lat);
            let f = if s.on { 1.0 } else { 0.0 };
            s.fade = f;
            for _ in 0..lat {
                s.fade_delay.run(f);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub fn node(&self, slot: usize) -> Option<&dyn Node> {
        self.slots.get(slot).map(|s| s.node.as_ref())
    }
    pub fn node_mut(&mut self, slot: usize) -> Option<&mut (dyn Node + 'static)> {
        self.slots.get_mut(slot).map(|s| s.node.as_mut())
    }
    pub fn stats(&self) -> ChainStats {
        self.stats
    }
    pub fn is_on(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(|s| s.on)
    }

    /// Times every slot's block into its probe (two clock reads per slot
    /// per block), for the Dev pages and the bench. Off by default.
    pub fn set_profiling(&mut self, on: bool) {
        self.profiling = on;
    }
    pub fn profiling(&self) -> bool {
        self.profiling
    }

    /// A slot's timing (blocks it ran, mean, percentiles, worst), recorded
    /// while profiling is on. The probe moves with its insert.
    pub fn probe(&self, slot: usize) -> Option<&Probe> {
        self.slots.get(slot).map(|s| &*s.probe)
    }

    /// A slot's timing, to take its window ([`Probe::take_window`]).
    pub fn probe_mut(&mut self, slot: usize) -> Option<&mut Probe> {
        self.slots.get_mut(slot).map(|s| &mut *s.probe)
    }

    /// The sidechain tag of a slot (see [`Chain::process_keyed`]).
    pub fn key(&self, slot: usize) -> Option<u16> {
        self.slots.get(slot).and_then(|s| s.key)
    }

    /// Sets the sidechain tag a slot listens to; it moves with the slot.
    /// False if there is no such slot.
    pub fn set_key(&mut self, slot: usize, key: Option<u16>) -> bool {
        let Some(s) = self.slots.get_mut(slot) else {
            return false;
        };
        s.key = key;
        true
    }

    /// Total latency in frames; the same whichever slots are bypassed.
    pub fn latency(&self) -> u32 {
        self.slots.iter().map(|s| s.node.latency()).sum()
    }

    /// Turns a slot on or off, with a 20 ms fade and the tail left ringing.
    pub fn set_on(&mut self, slot: usize, on: bool) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.on = on;
        }
    }

    /// Turns a slot on or off at once, with no fade and no tail: for
    /// setting a chain up (loading a project) before it plays. A slot
    /// turned off this way is reset.
    pub fn set_on_now(&mut self, slot: usize, on: bool) {
        let Some(s) = self.slots.get_mut(slot) else {
            return;
        };
        let f = if on { 1.0 } else { 0.0 };
        s.on = on;
        s.fade = f;
        for _ in 0..s.node.latency() {
            s.fade_delay.run(f);
        }
        s.tail_left = 0;
        s.resting = !on;
        if !on {
            s.node.reset();
        }
    }

    /// Sets a parameter now (from the next frame processed).
    pub fn set_param(&mut self, slot: usize, id: u16, value: f32) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.node.set_param(id, value);
        }
    }

    /// Queues a sample-timed event for a slot's next block. Keeps the
    /// queue sorted by offset; an offset at or past the block's end applies
    /// right after the block. Returns false (and counts it) when full.
    pub fn queue(&mut self, slot: usize, t: Timed) -> bool {
        let Some(s) = self.slots.get_mut(slot) else {
            return false;
        };
        if s.events.len() == s.events.capacity() {
            self.stats.dropped_events += 1;
            return false;
        }
        let at = s.events.partition_point(|e| e.offset <= t.offset);
        s.events.insert(at, t);
        true
    }

    /// Applies every queued parameter change now and drops the other
    /// queued events, without running the chain (a frozen strip's inserts
    /// keep up with their knobs). Real-time safe.
    pub fn flush_events(&mut self) {
        for s in &mut self.slots {
            apply_params(s.node.as_mut(), &s.events);
            s.events.clear();
        }
    }

    /// Runs the chain in place on one block. Every slot gets `ctx`'s
    /// sidechain.
    pub fn process(&mut self, ctx: &Ctx, left: &mut [f32], right: &mut [f32]) {
        self.process_keyed(ctx, |_| None, left, right)
    }

    /// Runs the chain in place on one block, giving each slot that has a
    /// sidechain tag ([`Chain::set_key`]) the signal `key` returns for it,
    /// and every other slot `ctx`'s sidechain. Keys must be at least as
    /// long as the block.
    pub fn process_keyed<'k, F>(
        &mut self,
        ctx: &Ctx<'k>,
        key: F,
        left: &mut [f32],
        right: &mut [f32],
    ) where
        F: Fn(u16) -> Option<(&'k [f32], &'k [f32])>,
    {
        let n = self.slots.len();
        self.run_slots(0..n, false, ctx, key, left, right);
    }

    /// The slot that lets the chain run with its input `frames` late and
    /// its output on time ([`Node::can_lead`]): fully on, with no latency
    /// and able to hold that lead. The slot that holds a lead already, if
    /// any, or else the first that can. The slots before it then work on
    /// the late input and the slots after it on time.
    pub fn lead_slot(&self, frames: u32) -> Option<usize> {
        let ok =
            |s: &Slot| s.on && s.fade == 1.0 && s.node.latency() == 0 && s.node.can_lead(frames);
        if let Some(i) = self.slots.iter().position(|s| s.node.lead() > 0) {
            return ok(&self.slots[i]).then_some(i);
        }
        self.slots.iter().position(ok)
    }

    /// The lead some slot holds, in frames (0: the chain runs on time).
    pub fn lead(&self) -> u32 {
        self.slots.iter().map(|s| s.node.lead()).max().unwrap_or(0)
    }

    /// Starts running the chain late ([`Node::lead_in`]): this block's
    /// output comes from what slot `slot` already has, and from the next
    /// block on the chain takes its input one block late. The slots before
    /// `slot` skip the block (their queued parameter changes apply now);
    /// `slot` and the slots after it run on time. `left` and `right` come
    /// in as anything and go out as the block's output. Real-time safe.
    pub fn lead_in(&mut self, slot: usize, ctx: &Ctx, left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len()).min(self.max_block);
        let Some(s) = self.slots.get_mut(slot) else {
            left[..len].fill(0.0);
            right[..len].fill(0.0);
            return;
        };
        let (left, right) = (&mut left[..len], &mut right[..len]);
        s.node.lead_in(ctx, &s.events, left, right);
        s.events.clear();
        if !finite(left) || !finite(right) {
            s.node.reset();
            self.stats.nan_resets += 1;
            left.fill(0.0);
            right.fill(0.0);
        }
        for s in &mut self.slots[..slot] {
            apply_params(s.node.as_mut(), &s.events);
            s.events.clear();
        }
        let n = self.slots.len();
        self.run_slots(slot + 1..n, false, ctx, |_| None, left, right);
    }

    /// Stops running the chain late ([`Node::lead_out`]): `left` and
    /// `right` hold the late input the chain has not had yet (last block's);
    /// the slots before `slot` run on it and `slot` takes it without
    /// putting anything out, so the next block's input is on time. Queued
    /// events wait for that block. Real-time safe; `left` and `right` are
    /// used as scratch.
    pub fn lead_out(&mut self, slot: usize, ctx: &Ctx, left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len()).min(self.max_block);
        if slot >= self.slots.len() {
            return;
        }
        let (left, right) = (&mut left[..len], &mut right[..len]);
        self.run_slots(0..slot, true, ctx, |_| None, left, right);
        self.slots[slot].node.lead_out(ctx, left, right);
    }

    /// Runs slots `range` in place on one block. With `no_events` they run
    /// without their queued events, which stay queued.
    fn run_slots<'k, F>(
        &mut self,
        range: std::ops::Range<usize>,
        no_events: bool,
        ctx: &Ctx<'k>,
        key: F,
        left: &mut [f32],
        right: &mut [f32],
    ) where
        F: Fn(u16) -> Option<(&'k [f32], &'k [f32])>,
    {
        let len = left.len().min(right.len()).min(self.max_block);
        if len == 0 {
            return;
        }
        let (left, right) = (&mut left[..len], &mut right[..len]);
        let step = self.step;
        let [da, db, fa, fb] = &mut self.scratch;
        let (da, db, fa, fb) = (
            &mut da[..len],
            &mut db[..len],
            &mut fa[..len],
            &mut fb[..len],
        );
        let part = &mut self.part;
        let profiling = self.profiling;
        let none: Vec<Timed> = Vec::new();
        let (watch, levels) = (self.watch, &mut self.levels);
        // The meters fall 20 dB a second: a factor of 10.
        let fall = if watch.is_some() {
            (-std::f32::consts::LN_10 * len as f32 / self.rate.max(1) as f32).exp()
        } else {
            1.0
        };
        for s in &mut self.slots[range] {
            let events = if no_events { &none } else { &s.events };
            let span = profiling.then(Span::start);
            let watched = watch == Some(s.uid);
            if watched {
                levels[0] = peak(left, right).max(levels[0] * fall);
            }
            let keyed = Ctx {
                sidechain: s.key.and_then(&key).or(ctx.sidechain),
                ..*ctx
            };
            let ctx = &keyed;
            // The dry path, delayed by the slot's latency, and the fade ramp
            // with its own delayed copy for the dry side.
            let target = if s.on { 1.0 } else { 0.0 };
            let mut hit_zero_at = None;
            let mut all_on = true;
            for i in 0..len {
                da[i] = s.dry[0].run(left[i]);
                db[i] = s.dry[1].run(right[i]);
                if s.fade < target {
                    s.fade = (s.fade + step).min(1.0);
                } else if s.fade > target {
                    s.fade = (s.fade - step).max(0.0);
                    if s.fade == 0.0 {
                        hit_zero_at = Some(i);
                    }
                }
                fa[i] = s.fade;
                fb[i] = s.fade_delay.run(s.fade);
                all_on &= fa[i] == 1.0 && fb[i] == 1.0;
            }
            // The fade only moves one way inside a block, so the input side
            // is silent throughout when it starts and ends at 0.
            let all_off = fa[0] == 0.0 && s.fade == 0.0 && hit_zero_at.is_none();
            if s.fade > 0.0 || hit_zero_at.is_some() || fa[0] > 0.0 {
                s.resting = false;
            }
            if all_off {
                // Bypassed: run the node on silence while its tail lasts.
                let n = if s.resting {
                    0
                } else {
                    (s.tail_left.min(len as u64)) as usize
                };
                left.fill(0.0);
                right.fill(0.0);
                if n > 0 {
                    run_part(s.node.as_mut(), ctx, events, 0, n, left, right, part);
                    if s.tail_left != FOREVER {
                        s.tail_left -= n as u64;
                    }
                    if !finite(&left[..n]) || !finite(&right[..n]) {
                        s.node.reset();
                        self.stats.nan_resets += 1;
                        left[..n].fill(0.0);
                        right[..n].fill(0.0);
                    }
                }
                if s.tail_left == FOREVER && s.node.tail_frames() != u32::MAX {
                    // An endless tail (a frozen delay) became finite: it
                    // rings for its new tail from the end of this block.
                    s.tail_left = s.node.tail_frames() as u64 + s.node.latency() as u64;
                }
                if s.tail_left == 0 && !s.resting {
                    s.resting = true;
                    s.node.reset();
                }
                // Events after the tail apply after its end and its reset,
                // the same whichever block the end falls in.
                let k = events.partition_point(|e| (e.offset as usize) < n);
                if !no_events {
                    apply_params(s.node.as_mut(), &events[k..]);
                }
                for i in 0..len {
                    left[i] += da[i] * (1.0 - fb[i]);
                    right[i] += db[i] * (1.0 - fb[i]);
                }
            } else {
                if !all_on {
                    for i in 0..len {
                        left[i] *= fa[i];
                        right[i] *= fa[i];
                    }
                }
                let end = match hit_zero_at {
                    None => {
                        run_part(s.node.as_mut(), ctx, events, 0, len, left, right, part);
                        len
                    }
                    Some(i) => {
                        // The tail is measured from the frame the input went
                        // silent, as the node stands at that frame, and ends
                        // on its exact frame, so the block size cannot move
                        // either.
                        run_part(s.node.as_mut(), ctx, events, 0, i + 1, left, right, part);
                        let tail = s.node.tail_frames();
                        s.tail_left = if tail == u32::MAX {
                            FOREVER
                        } else {
                            tail as u64 + s.node.latency() as u64
                        };
                        let end = i + 1 + s.tail_left.min((len - i - 1) as u64) as usize;
                        run_part(s.node.as_mut(), ctx, events, i + 1, end, left, right, part);
                        if s.tail_left != FOREVER {
                            s.tail_left -= (end - i - 1) as u64;
                        }
                        left[end..].fill(0.0);
                        right[end..].fill(0.0);
                        if s.tail_left == 0 {
                            s.resting = true;
                            s.node.reset();
                        }
                        end
                    }
                };
                let k = events.partition_point(|e| (e.offset as usize) < end);
                if !no_events {
                    apply_params(s.node.as_mut(), &events[k..]);
                }
                if !finite(left) || !finite(right) {
                    s.node.reset();
                    self.stats.nan_resets += 1;
                    left.copy_from_slice(da);
                    right.copy_from_slice(db);
                } else if !all_on {
                    for i in 0..len {
                        left[i] += da[i] * (1.0 - fb[i]);
                        right[i] += db[i] * (1.0 - fb[i]);
                    }
                }
            }
            if watched {
                levels[1] = peak(left, right).max(levels[1] * fall);
            }
            if !no_events {
                s.events.clear();
            }
            if let Some(span) = span {
                s.probe.stop(span, len as u32);
            }
        }
    }
}

/// The highest absolute sample on either side.
fn peak(left: &[f32], right: &[f32]) -> f32 {
    left.iter().chain(right).fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Runs `node` on frames `from..to` of the block, with the events that fall
/// there (their offsets made relative to `from`) and a context that starts
/// at `from`.
#[allow(clippy::too_many_arguments)]
fn run_part(
    node: &mut dyn Node,
    ctx: &Ctx,
    events: &[Timed],
    from: usize,
    to: usize,
    left: &mut [f32],
    right: &mut [f32],
    part: &mut Vec<Timed>,
) {
    if to <= from {
        return;
    }
    let lo = events.partition_point(|e| (e.offset as usize) < from);
    let hi = events.partition_point(|e| (e.offset as usize) < to);
    if from == 0 {
        node.process(ctx, &events[..hi], &mut left[..to], &mut right[..to]);
        return;
    }
    part.clear();
    part.extend(events[lo..hi].iter().map(|e| Timed {
        offset: e.offset - from as u32,
        ev: e.ev,
    }));
    let sidechain = ctx
        .sidechain
        .and_then(|(a, b)| (a.len() >= to && b.len() >= to).then(|| (&a[from..to], &b[from..to])));
    let beats = if ctx.playing {
        ctx.beats + from as f64 * ctx.bpm / (60.0 * ctx.rate.max(1) as f64)
    } else {
        ctx.beats
    };
    let sub = Ctx {
        frame: ctx.frame + from as u64,
        beats,
        sidechain,
        ..*ctx
    };
    node.process(&sub, part, &mut left[from..to], &mut right[from..to]);
}

#[inline]
fn finite(x: &[f32]) -> bool {
    x.iter().all(|v| v.is_finite())
}

/// Applies the parameter changes among `events` now (a node that is not
/// being run, or events past the end of the block).
fn apply_params(node: &mut dyn Node, events: &[Timed]) {
    for e in events {
        if let NodeEvent::Param { id, value } = e.ev {
            node.set_param(id, value);
        }
    }
}
