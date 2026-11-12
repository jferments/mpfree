//! A node whose work is done on another computer, with a stand-in here for
//! when the link fails.
//!
//! [`Offloaded`] is an ordinary [`Node`], so it goes in a mixer return's
//! insert chain like any effect. Each block, it hands its input and its
//! parameter changes to the [`Remote`] and plays the server's answer from L
//! periods ago. When answers stop coming:
//!
//! - **A lost block is faded out.** The last block heard is played
//!   backwards from its last sample, fading to silence over the block, so
//!   nothing clicks. The next answer fades in over a block.
//! - **A stand-in starts at the first miss.** It is the same kind of node,
//!   run here at a cheaper quality (Cloud Eco for a Cloud send). For its
//!   first L periods it is silent by construction: its output is held back
//!   by the link's latency, or its pre-delay is at least that long. So if
//!   the link comes back in that time, the stand-in stops again unheard.
//! - **After L periods without answers, the stand-in is heard.** It fades in
//!   over a block and the song keeps playing. Its tail starts from the first
//!   miss; the server's tail from before then is lost.
//! - **The server takes over again after [`RECOVER_AFTER`] answers in a
//!   row,** with an equal-power crossfade over [`RETURN_FRAMES`]. Then the
//!   stand-in stops.
//!
//! **Latency.** The server's answer is L periods late. Some nodes have a
//! parameter that delays their whole output, such as a reverb's pre-delay.
//! Such a node can take the link's latency off that parameter
//! ([`Offloaded::absorb_into`]):
//! - the server renders it with the pre-delay shortened by L periods;
//! - the stand-in renders it with the pre-delay at least L periods long;
//! - both land where the node would have landed here, and [`Node::latency`]
//!   is 0.
//!
//! A pre-delay set shorter than L periods (5.8 ms at L = 2) acts as L
//! periods. A node without such a parameter reports L periods of latency
//! for the mixer to compensate, and its stand-in's output is held back by
//! as much to line up.
//!
//! **Parameters** go to the server as events in the requests. A change made
//! with `set_param` goes at the next block's start; timed events keep their
//! frames. Each block also resends one parameter, in turn, so a change lost
//! with a datagram is put right within one cycle (15 blocks, 44 ms, for
//! Cloud). The stand-in gets the same changes, except for parameters pinned
//! on it, such as its quality. [`Node::reset`] asks the server to reset its
//! node too ([`Remote::clear`]), and plays the answers still on their way
//! as silence.
//!
//! **Blocks** must be exactly the link's period (`Config::frames`), which
//! is what the audio thread processes. A block of another length can't be
//! sent; it counts as a miss, and the stand-in covers it.
//!
//! [`Offloaded::process`] never blocks, locks or allocates, like
//! `Remote::process` (`tests/no_alloc.rs` checks). The stand-in's buffers
//! come from [`Node::prepare`].
//!
//! Not yet:
//! - a warm stand-in that runs all the time, so a fallback keeps the tail
//!   (at the cost of the stand-in's CPU, always);
//! - tempo and the sidechain on the server (the wire carries neither);
//! - checking that the server runs the same node (the control channel will).

use std::f32::consts::FRAC_PI_2;

use mpfree_fx::{ids, Cloud};
use mpfree_node::{Ctx, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Tuning};

use crate::client::{Outcome, Remote};
use crate::wire::MAX_EVENTS;

/// Answers on time in a row before the server takes over again from the
/// stand-in: 93 ms at 128 frames and 44.1 kHz.
pub const RECOVER_AFTER: u32 = 32;
/// The crossfade from the stand-in back to the server, in frames (46 ms at
/// 44.1 kHz).
pub const RETURN_FRAMES: usize = 2048;
/// Events a block may bring; more are left out and counted.
pub const MAX_BLOCK_EVENTS: usize = 256;
/// Parameter ids an offloaded node may use: 0 to 63.
pub const MAX_PARAMS: usize = 64;

/// Cloud's quality choices.
const FULL: f32 = 0.0;
const ECO: f32 = 2.0;

/// What an [`Offloaded`] node plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The server's answers.
    Remote,
    /// Answers stopped. The stand-in runs, not heard yet.
    Bridge,
    /// The stand-in.
    Local,
    /// A crossfade from the stand-in back to the server.
    Return,
}

/// An [`Offloaded`] node's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OffloadStats {
    /// Misses right after a block that was heard, faded out.
    pub concealed: u64,
    /// Times the stand-in started: a miss while the server was heard.
    pub bridges: u64,
    /// Times the stand-in was heard: no answers for L periods.
    pub fallbacks: u64,
    /// Crossfades back to the server that finished.
    pub returns: u64,
    /// Blocks the stand-in rendered.
    pub local_blocks: u64,
    /// Parameter changes held over to the next block, for want of room in a
    /// request.
    pub deferred: u64,
    /// Events left out: past [`MAX_BLOCK_EVENTS`], or a note past a
    /// request's room.
    pub events_dropped: u64,
}

/// A node rendered by a server, with a stand-in here. See the module docs.
pub struct Offloaded {
    remote: Remote,
    local: Box<dyn Node>,
    params: &'static [ParamInfo],
    /// Each parameter's value as set, by id: what the server's node has,
    /// before any latency is taken off.
    v: Vec<f32>,
    /// Parameters held at a value on the stand-in.
    pinned: Vec<(u16, f32)>,
    /// A parameter in ms that takes the link's latency.
    absorb: Option<u16>,
    /// The link's latency, in frames and in ms at the prepared rate.
    delay: usize,
    delay_ms: f32,
    /// Parameters the server hasn't had since they changed, a bit per id.
    dirty: u64,
    /// The next parameter to resend, an index into `params`.
    refresh: usize,
    mode: Mode,
    /// Answers on time in a row.
    hits: u32,
    /// Frames the stand-in has rendered since it started.
    local_frames: u64,
    /// How far the crossfade back has gone, in frames.
    pos: usize,
    /// Answers still to play as silence after a reset.
    mute: u32,
    /// Whether the last block's answer was heard, so `prev` holds it.
    heard: bool,
    /// Whether the next answer fades in.
    fade_in: bool,
    prev: [Vec<f32>; 2],
    prev_len: usize,
    input: [Vec<f32>; 2],
    alt: [Vec<f32>; 2],
    /// The stand-in's hold-back, when nothing absorbs the latency.
    line: [Vec<f32>; 2],
    line_at: usize,
    to_remote: Vec<Timed>,
    to_local: Vec<Timed>,
    stats: OffloadStats,
}

impl Offloaded {
    /// `local` is the stand-in: the node the server's partition runs, set
    /// up the same way. Every parameter goes to the server with the first
    /// block. Panics if the node has a parameter id of 64 or more.
    pub fn new(remote: Remote, local: Box<dyn Node>) -> Offloaded {
        let params = local.params();
        let top = params.iter().map(|p| p.id as usize + 1).max().unwrap_or(0);
        assert!(
            top <= MAX_PARAMS,
            "an offloaded node's parameter ids must be below {MAX_PARAMS}"
        );
        let mut v = vec![f32::NAN; top];
        for p in params {
            v[p.id as usize] = local.param(p.id);
        }
        let delay = remote.latency_frames() as usize;
        Offloaded {
            remote,
            local,
            params,
            v,
            pinned: Vec::new(),
            absorb: None,
            delay,
            delay_ms: 0.0,
            dirty: all_ids(params),
            refresh: 0,
            mode: Mode::Remote,
            hits: 0,
            local_frames: 0,
            pos: 0,
            mute: 0,
            heard: false,
            fade_in: false,
            prev: [Vec::new(), Vec::new()],
            prev_len: 0,
            input: [Vec::new(), Vec::new()],
            alt: [Vec::new(), Vec::new()],
            line: [Vec::new(), Vec::new()],
            line_at: 0,
            to_remote: Vec::with_capacity(MAX_EVENTS),
            to_local: Vec::with_capacity(MAX_BLOCK_EVENTS),
            stats: OffloadStats::default(),
        }
    }

    /// Holds the stand-in's parameter `id` at `value` (its quality). The
    /// server's node still gets what is set.
    pub fn pin_local(mut self, id: u16, value: f32) -> Offloaded {
        self.local.set_param(id, value);
        self.pinned.retain(|&(i, _)| i != id);
        self.pinned.push((id, value));
        self
    }

    /// Takes the link's latency off parameter `id`: a time in ms that
    /// delays the node's whole output, such as a reverb's pre-delay. Then
    /// [`Node::latency`] is 0.
    pub fn absorb_into(mut self, id: u16) -> Offloaded {
        self.absorb = Some(id);
        self.sync_local(id);
        self
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn stats(&self) -> OffloadStats {
        self.stats
    }

    /// The link's audio-thread end: its counts, and what tests wait on.
    pub fn remote(&self) -> &Remote {
        &self.remote
    }

    /// The stand-in, for reading its parameters.
    pub fn stand_in(&self) -> &dyn Node {
        self.local.as_ref()
    }

    fn info_of(&self, id: u16) -> Option<&'static ParamInfo> {
        self.params.iter().find(|p| p.id == id)
    }

    fn for_remote(&self, id: u16, value: f32) -> f32 {
        if self.absorb == Some(id) {
            (value - self.delay_ms).max(0.0)
        } else {
            value
        }
    }

    fn for_local(&self, id: u16, value: f32) -> Option<f32> {
        if self.pinned.iter().any(|&(i, _)| i == id) {
            None
        } else if self.absorb == Some(id) {
            Some(value.max(self.delay_ms))
        } else {
            Some(value)
        }
    }

    fn sync_local(&mut self, id: u16) {
        let Some(&v) = self.v.get(id as usize) else {
            return;
        };
        if let Some(lv) = self.for_local(id, v) {
            self.local.set_param(id, lv);
        }
    }

    /// The stand-in stops and forgets its state, ready for its next start.
    fn stop_local(&mut self) {
        self.local.reset();
        self.local_frames = 0;
        for b in &mut self.line {
            b.fill(0.0);
        }
        self.line_at = 0;
    }

    /// Fills `to_remote` and `to_local` for this block: parameters the
    /// server hasn't had, one resent in turn, then the block's own events.
    fn build_events(&mut self, events: &[Timed]) {
        self.to_remote.clear();
        self.to_local.clear();
        let room = self.to_remote.capacity().min(MAX_EVENTS);
        let mut d = self.dirty;
        while d != 0 && self.to_remote.len() < room {
            let id = d.trailing_zeros() as u16;
            d &= d - 1;
            self.dirty &= !(1u64 << id);
            let value = self.for_remote(id, self.v[id as usize]);
            self.to_remote
                .push(Timed::new(0, NodeEvent::Param { id, value }));
        }
        if !self.params.is_empty() && self.to_remote.len() < room {
            let p = &self.params[self.refresh];
            self.refresh = (self.refresh + 1) % self.params.len();
            if !p.readonly {
                let value = self.for_remote(p.id, self.v[p.id as usize]);
                self.to_remote
                    .push(Timed::new(0, NodeEvent::Param { id: p.id, value }));
            }
        }
        let take = events.len().min(MAX_BLOCK_EVENTS);
        self.stats.events_dropped += (events.len() - take) as u64;
        for e in &events[..take] {
            match e.ev {
                NodeEvent::Param { id, value } => {
                    let Some(p) = self.info_of(id) else {
                        continue;
                    };
                    let v = p.clamp(value);
                    self.v[id as usize] = v;
                    if self.to_remote.len() < room {
                        let value = self.for_remote(id, v);
                        self.to_remote
                            .push(Timed::new(e.offset, NodeEvent::Param { id, value }));
                    } else {
                        self.dirty |= 1u64 << id;
                        self.stats.deferred += 1;
                    }
                    if let Some(value) = self.for_local(id, v) {
                        self.to_local
                            .push(Timed::new(e.offset, NodeEvent::Param { id, value }));
                    }
                }
                _ => {
                    if self.to_remote.len() < room {
                        self.to_remote.push(*e);
                    } else {
                        self.stats.events_dropped += 1;
                    }
                    self.to_local.push(*e);
                }
            }
        }
    }

    /// Runs the stand-in on this block's input into `alt`, held back by the
    /// link's latency when nothing absorbs it.
    fn run_local(&mut self, ctx: &Ctx, n: usize) {
        let [al, ar] = &mut self.alt;
        al[..n].copy_from_slice(&self.input[0][..n]);
        ar[..n].copy_from_slice(&self.input[1][..n]);
        self.local
            .process(ctx, &self.to_local, &mut al[..n], &mut ar[..n]);
        if self.absorb.is_none() && self.delay > 0 {
            let [ll, lr] = &mut self.line;
            for i in 0..n {
                let at = self.line_at;
                (al[i], ll[at]) = (ll[at], al[i]);
                (ar[i], lr[at]) = (lr[at], ar[i]);
                self.line_at = if at + 1 == self.delay { 0 } else { at + 1 };
            }
        }
        self.local_frames += n as u64;
        self.stats.local_blocks += 1;
    }
}

/// A Cloud reverb send. The server runs Cloud Full with the link's latency
/// taken off its pre-delay; Cloud Eco here is the stand-in. Mix is 100 %:
/// a send is all wet, and a dry part would come back late.
pub fn cloud_send(remote: Remote) -> Offloaded {
    let mut cloud = Cloud::new();
    cloud.set_param(ids::cloud::MIX, 100.0);
    let mut o = Offloaded::new(remote, Box::new(cloud))
        .pin_local(ids::cloud::QUALITY, ECO)
        .absorb_into(ids::cloud::PRE_DELAY);
    o.set_param(ids::cloud::QUALITY, FULL);
    o
}

fn all_ids(params: &[ParamInfo]) -> u64 {
    params.iter().fold(0, |d, p| d | 1u64 << p.id)
}

impl Node for Offloaded {
    fn info(&self) -> &'static NodeInfo {
        self.local.info()
    }

    fn params(&self) -> &'static [ParamInfo] {
        self.params
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = self.info_of(id) else {
            return;
        };
        self.v[id as usize] = p.clamp(value);
        self.dirty |= 1u64 << id;
        self.sync_local(id);
    }

    fn param(&self, id: u16) -> f32 {
        match self.info_of(id) {
            Some(p) if p.readonly => self.local.param(id),
            Some(_) => self.v[id as usize],
            None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        self.delay_ms = self.delay as f32 * 1000.0 / rate.max(1) as f32;
        self.local.prepare(rate, max_block);
        let n = max_block.max(self.remote.config().frames);
        for b in self
            .prev
            .iter_mut()
            .chain(&mut self.input)
            .chain(&mut self.alt)
        {
            *b = vec![0.0; n];
        }
        let hold = if self.absorb.is_none() { self.delay } else { 0 };
        self.line = [vec![0.0; hold], vec![0.0; hold]];
        self.line_at = 0;
        if let Some(id) = self.absorb {
            self.sync_local(id);
        }
        self.dirty = all_ids(self.params);
    }

    fn reset(&mut self) {
        self.stop_local();
        self.mode = Mode::Remote;
        self.hits = 0;
        self.pos = 0;
        self.heard = false;
        self.fade_in = false;
        self.mute = self.remote.config().latency;
        self.remote.clear();
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        if n > self.input[0].len() || right.len() != n {
            // Not prepared for this block (the contract rules it out).
            left.fill(0.0);
            right.fill(0.0);
            return;
        }
        self.input[0][..n].copy_from_slice(left);
        self.input[1][..n].copy_from_slice(right);
        self.build_events(events);

        // The server's side, in place: block n − L's answer, faded in after
        // a miss; or the last answer heard, faded out, on a miss.
        let outcome = self.remote.process(ctx.frame, &self.to_remote, left, right);
        let (hit, miss) = if self.mute > 0 {
            self.mute -= 1;
            left.fill(0.0);
            right.fill(0.0);
            (false, false)
        } else {
            (outcome == Outcome::Hit, outcome == Outcome::Miss)
        };
        if hit {
            self.hits += 1;
            if self.fade_in {
                for i in 0..n {
                    let g = (i + 1) as f32 / n as f32;
                    left[i] *= g;
                    right[i] *= g;
                }
                self.fade_in = false;
            }
            self.prev[0][..n].copy_from_slice(left);
            self.prev[1][..n].copy_from_slice(right);
            self.prev_len = n;
            self.heard = true;
        } else {
            if miss {
                self.hits = 0;
                if self.heard {
                    let k = n.min(self.prev_len);
                    for i in 0..k {
                        let g = (k - 1 - i) as f32 / k as f32;
                        left[i] = self.prev[0][k - 1 - i] * g;
                        right[i] = self.prev[1][k - 1 - i] * g;
                    }
                    self.stats.concealed += 1;
                }
                self.fade_in = true;
            }
            self.heard = false;
        }

        // Where the node goes this block.
        let mut fade_local_in = false;
        let mut cut_short = None;
        match self.mode {
            Mode::Remote => {
                if miss {
                    self.mode = Mode::Bridge;
                    self.stats.bridges += 1;
                }
            }
            Mode::Bridge => {
                if hit {
                    self.mode = Mode::Remote;
                    self.stop_local();
                } else if self.local_frames >= self.delay as u64 {
                    self.mode = Mode::Local;
                    self.stats.fallbacks += 1;
                    fade_local_in = true;
                }
            }
            Mode::Local => {
                if hit && self.hits >= RECOVER_AFTER {
                    self.mode = Mode::Return;
                    self.pos = 0;
                }
            }
            Mode::Return => {
                if miss {
                    cut_short = Some(self.pos);
                    self.mode = Mode::Local;
                }
            }
        }

        if self.mode == Mode::Remote {
            // The stand-in is stopped: keep its parameters in step.
            for e in &self.to_local {
                if let NodeEvent::Param { id, value } = e.ev {
                    self.local.set_param(id, value);
                }
            }
            return;
        }
        self.run_local(ctx, n);
        let [al, ar] = &self.alt;
        match self.mode {
            Mode::Remote | Mode::Bridge => {}
            Mode::Local => {
                if let Some(pos) = cut_short {
                    // A miss during the crossfade back: the server's side
                    // fades out where it was, the stand-in comes back up.
                    let t = pos as f32 / RETURN_FRAMES as f32;
                    let (gr, gl0) = ((t * FRAC_PI_2).sin(), (t * FRAC_PI_2).cos());
                    for i in 0..n {
                        let gl = gl0 + (1.0 - gl0) * (i + 1) as f32 / n as f32;
                        left[i] = left[i] * gr + al[i] * gl;
                        right[i] = right[i] * gr + ar[i] * gl;
                    }
                } else if fade_local_in {
                    for i in 0..n {
                        let g = (i + 1) as f32 / n as f32;
                        left[i] = al[i] * g;
                        right[i] = ar[i] * g;
                    }
                } else {
                    left.copy_from_slice(&al[..n]);
                    right.copy_from_slice(&ar[..n]);
                }
            }
            Mode::Return => {
                for i in 0..n {
                    let t = ((self.pos + i + 1) as f32 / RETURN_FRAMES as f32).min(1.0);
                    let (gr, gl) = ((t * FRAC_PI_2).sin(), (t * FRAC_PI_2).cos());
                    left[i] = left[i] * gr + al[i] * gl;
                    right[i] = right[i] * gr + ar[i] * gl;
                }
                self.pos += n;
                if self.pos >= RETURN_FRAMES {
                    self.mode = Mode::Remote;
                    self.stop_local();
                    self.stats.returns += 1;
                }
            }
        }
    }

    fn tail_frames(&self) -> u32 {
        self.local.tail_frames().saturating_add(self.delay as u32)
    }

    fn latency(&self) -> u32 {
        if self.absorb.is_some() {
            0
        } else {
            self.delay as u32
        }
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        self.local.set_tuning(tuning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{connect, Config};
    use crate::Echo;
    use std::net::UdpSocket;

    /// A link to an address nobody listens on: every block is a miss.
    fn dead() -> (Remote, crate::client::Link) {
        let gone = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        connect(gone, Config::default()).unwrap()
    }

    fn block(node: &mut Offloaded, k: u64, events: &[Timed]) {
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        node.process(&Ctx::new(44_100, k * 128), events, &mut l, &mut r);
    }

    fn param(t: &Timed) -> (u32, u16, f32) {
        match t.ev {
            NodeEvent::Param { id, value } => (t.offset, id, value),
            other => panic!("not a parameter: {other:?}"),
        }
    }

    #[test]
    fn every_parameter_goes_first_then_one_in_turn() {
        let (remote, _link) = dead();
        let mut node = cloud_send(remote);
        node.prepare(44_100, 128);
        let d_ms = 256.0f32 * 1000.0 / 44_100.0;
        block(&mut node, 0, &[]);
        let sent: Vec<_> = node.to_remote.iter().map(param).collect();
        // 15 parameters at the block's start, then the first resent.
        assert_eq!(sent.len(), 16);
        assert!(sent.iter().all(|&(at, _, _)| at == 0));
        assert_eq!(sent[ids::cloud::QUALITY as usize].2, FULL);
        assert_eq!(sent[ids::cloud::MIX as usize].2, 100.0);
        assert_eq!(sent[ids::cloud::PRE_DELAY as usize].2, 10.0 - d_ms);
        // The stand-in keeps Eco and the full pre-delay.
        assert_eq!(node.stand_in().param(ids::cloud::QUALITY), ECO);
        assert_eq!(node.stand_in().param(ids::cloud::PRE_DELAY), 10.0);
        assert_eq!(node.param(ids::cloud::QUALITY), FULL);
        block(&mut node, 1, &[]);
        let sent: Vec<_> = node.to_remote.iter().map(param).collect();
        assert_eq!(sent, vec![(0, 1, node.param(1))]);
    }

    #[test]
    fn a_pre_delay_shorter_than_the_link_acts_as_the_link() {
        let (remote, _link) = dead();
        let mut node = cloud_send(remote);
        node.prepare(44_100, 128);
        let d_ms = 256.0f32 * 1000.0 / 44_100.0;
        node.set_param(ids::cloud::PRE_DELAY, 1.0);
        assert_eq!(node.param(ids::cloud::PRE_DELAY), 1.0);
        assert_eq!(node.stand_in().param(ids::cloud::PRE_DELAY), d_ms);
        assert_eq!(node.for_remote(ids::cloud::PRE_DELAY, 1.0), 0.0);
        assert_eq!(node.latency(), 0);
    }

    #[test]
    fn a_crowded_block_holds_its_last_changes_over_to_the_next() {
        let (remote, _link) = dead();
        let mut node = cloud_send(remote);
        node.prepare(44_100, 128);
        block(&mut node, 0, &[]);
        let size = ids::cloud::SIZE;
        let events: Vec<Timed> = (0..40)
            .map(|i| {
                Timed::new(
                    i * 3,
                    NodeEvent::Param {
                        id: size,
                        value: 10.0 + i as f32,
                    },
                )
            })
            .collect();
        block(&mut node, 1, &events);
        // One resent parameter and 33 of the 40 fit; the stand-in has all.
        assert_eq!(node.to_remote.len(), MAX_EVENTS);
        assert_eq!(node.stats().deferred, 7);
        assert_eq!(node.to_local.len(), 40);
        assert_eq!(node.param(size), 49.0);
        block(&mut node, 2, &[]);
        assert_eq!(param(&node.to_remote[0]), (0, size, 49.0));
    }

    #[test]
    fn without_a_parameter_to_absorb_it_the_latency_is_reported() {
        let (remote, _link) = dead();
        let mut node = Offloaded::new(remote, Box::new(Echo));
        node.prepare(44_100, 128);
        assert_eq!(node.latency(), 256);
        assert_eq!(node.tail_frames(), 256);
    }
}
