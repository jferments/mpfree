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
//! belongs on the control thread; everything else is real-time safe.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};

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
}

impl Slot {
    fn new(node: Box<dyn Node>) -> Slot {
        let lat = node.latency() as usize;
        Slot { node, on: true, fade: 1.0, tail_left: 0, resting: false, dry: [FixedDelay::new(lat), FixedDelay::new(lat)], fade_delay: fade_line(lat), events: Vec::with_capacity(MAX_SLOT_EVENTS) }
    }
}

fn fade_line(lat: usize) -> FixedDelay {
    let mut d = FixedDelay::new(lat);
    for _ in 0..lat { d.run(1.0); }
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
}

impl Default for Chain {
    fn default() -> Self { Self::new() }
}

impl Chain {
    pub fn new() -> Chain {
        let mut c = Chain { slots: Vec::new(), rate: 48_000, max_block: 0, step: 0.0, scratch: Default::default(), part: Vec::with_capacity(MAX_SLOT_EVENTS), stats: ChainStats::default() };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c
    }

    /// Adds a node at the end, prepared for the chain's rate (control
    /// thread). Returns its slot index.
    pub fn push(&mut self, mut node: Box<dyn Node>) -> usize {
        node.prepare(self.rate, self.max_block);
        self.slots.push(Slot::new(node));
        self.slots.len() - 1
    }

    /// Prepares every node and the chain's buffers (control thread).
    pub fn prepare(&mut self, rate: u32, max_block: usize) {
        self.rate = rate.max(1);
        self.max_block = max_block.max(1);
        self.step = 1.0 / (FADE_MS * 0.001 * self.rate as f32);
        self.scratch = [vec![0.0; self.max_block], vec![0.0; self.max_block], vec![0.0; self.max_block], vec![0.0; self.max_block]];
        for s in &mut self.slots {
            s.node.prepare(self.rate, self.max_block);
            let lat = s.node.latency() as usize;
            s.dry = [FixedDelay::new(lat), FixedDelay::new(lat)];
            s.fade_delay = fade_line(lat);
            let f = if s.on { 1.0 } else { 0.0 };
            s.fade = f;
            for _ in 0..lat { s.fade_delay.run(f); }
        }
    }

    pub fn len(&self) -> usize { self.slots.len() }
    pub fn is_empty(&self) -> bool { self.slots.is_empty() }
    pub fn node(&self, slot: usize) -> Option<&dyn Node> { self.slots.get(slot).map(|s| s.node.as_ref()) }
    pub fn node_mut(&mut self, slot: usize) -> Option<&mut (dyn Node + 'static)> { self.slots.get_mut(slot).map(|s| s.node.as_mut()) }
    pub fn stats(&self) -> ChainStats { self.stats }
    pub fn is_on(&self, slot: usize) -> bool { self.slots.get(slot).is_some_and(|s| s.on) }

    /// Total latency in frames; the same whichever slots are bypassed.
    pub fn latency(&self) -> u32 { self.slots.iter().map(|s| s.node.latency()).sum() }

    /// Turns a slot on or off, with a 20 ms fade and the tail left ringing.
    pub fn set_on(&mut self, slot: usize, on: bool) {
        if let Some(s) = self.slots.get_mut(slot) { s.on = on; }
    }

    /// Turns a slot on or off at once, with no fade and no tail: for
    /// setting a chain up (loading a project) before it plays. A slot
    /// turned off this way is reset.
    pub fn set_on_now(&mut self, slot: usize, on: bool) {
        let Some(s) = self.slots.get_mut(slot) else { return };
        let f = if on { 1.0 } else { 0.0 };
        s.on = on;
        s.fade = f;
        for _ in 0..s.node.latency() { s.fade_delay.run(f); }
        s.tail_left = 0;
        s.resting = !on;
        if !on { s.node.reset(); }
    }

    /// Sets a parameter now (from the next frame processed).
    pub fn set_param(&mut self, slot: usize, id: u16, value: f32) {
        if let Some(s) = self.slots.get_mut(slot) { s.node.set_param(id, value); }
    }

    /// Queues a sample-timed event for a slot's next block. Keeps the
    /// queue sorted by offset; an offset at or past the block's end applies
    /// right after the block. Returns false (and counts it) when full.
    pub fn queue(&mut self, slot: usize, t: Timed) -> bool {
        let Some(s) = self.slots.get_mut(slot) else { return false };
        if s.events.len() == s.events.capacity() {
            self.stats.dropped_events += 1;
            return false;
        }
        let at = s.events.partition_point(|e| e.offset <= t.offset);
        s.events.insert(at, t);
        true
    }

    /// Runs the chain in place on one block.
    pub fn process(&mut self, ctx: &Ctx, left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len()).min(self.max_block);
        if len == 0 {
            return;
        }
        let (left, right) = (&mut left[..len], &mut right[..len]);
        let step = self.step;
        let [da, db, fa, fb] = &mut self.scratch;
        let (da, db, fa, fb) = (&mut da[..len], &mut db[..len], &mut fa[..len], &mut fb[..len]);
        let part = &mut self.part;
        for s in &mut self.slots {
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
                    if s.fade == 0.0 { hit_zero_at = Some(i); }
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
                let n = if s.resting { 0 } else { (s.tail_left.min(len as u64)) as usize };
                left.fill(0.0);
                right.fill(0.0);
                if n > 0 {
                    run_part(s.node.as_mut(), ctx, &s.events, 0, n, left, right, part);
                    if s.tail_left != FOREVER { s.tail_left -= n as u64; }
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
                let k = s.events.partition_point(|e| (e.offset as usize) < n);
                apply_params(s.node.as_mut(), &s.events[k..]);
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
                        run_part(s.node.as_mut(), ctx, &s.events, 0, len, left, right, part);
                        len
                    }
                    Some(i) => {
                        // The tail is measured from the frame the input went
                        // silent, as the node stands at that frame, and ends
                        // on its exact frame, so the block size cannot move
                        // either.
                        run_part(s.node.as_mut(), ctx, &s.events, 0, i + 1, left, right, part);
                        let tail = s.node.tail_frames();
                        s.tail_left = if tail == u32::MAX { FOREVER } else { tail as u64 + s.node.latency() as u64 };
                        let end = i + 1 + s.tail_left.min((len - i - 1) as u64) as usize;
                        run_part(s.node.as_mut(), ctx, &s.events, i + 1, end, left, right, part);
                        if s.tail_left != FOREVER { s.tail_left -= (end - i - 1) as u64; }
                        left[end..].fill(0.0);
                        right[end..].fill(0.0);
                        if s.tail_left == 0 {
                            s.resting = true;
                            s.node.reset();
                        }
                        end
                    }
                };
                let k = s.events.partition_point(|e| (e.offset as usize) < end);
                apply_params(s.node.as_mut(), &s.events[k..]);
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
            s.events.clear();
        }
    }
}

/// Runs `node` on frames `from..to` of the block, with the events that fall
/// there (their offsets made relative to `from`) and a context that starts
/// at `from`.
#[allow(clippy::too_many_arguments)]
fn run_part(node: &mut dyn Node, ctx: &Ctx, events: &[Timed], from: usize, to: usize, left: &mut [f32], right: &mut [f32], part: &mut Vec<Timed>) {
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
    part.extend(events[lo..hi].iter().map(|e| Timed { offset: e.offset - from as u32, ev: e.ev }));
    let sidechain = ctx.sidechain.and_then(|(a, b)| (a.len() >= to && b.len() >= to).then(|| (&a[from..to], &b[from..to])));
    let beats = if ctx.playing { ctx.beats + from as f64 * ctx.bpm / (60.0 * ctx.rate.max(1) as f64) } else { ctx.beats };
    let sub = Ctx { frame: ctx.frame + from as u64, beats, sidechain, ..*ctx };
    node.process(&sub, part, &mut left[from..to], &mut right[from..to]);
}

#[inline]
fn finite(x: &[f32]) -> bool { x.iter().all(|v| v.is_finite()) }

/// Applies the parameter changes among `events` now (a node that is not
/// being run, or events past the end of the block).
fn apply_params(node: &mut dyn Node, events: &[Timed]) {
    for e in events {
        if let NodeEvent::Param { id, value } = e.ev { node.set_param(id, value); }
    }
}
