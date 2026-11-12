//! The offload server: the other computer's side. It holds the partitions'
//! nodes, answers each request with that block rendered, and never plays
//! anything itself, so it needs no clock and no sound card.
//!
//! [`Server::answer`] does one datagram and is what the tests drive;
//! [`serve`] is the loop around a UDP socket.

use std::io;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mpfree_node::{Ctx, Kind as NodeKind, Node, Timed};

use crate::wire::{
    self, Block, Header, Kind, WireError, FLAG_GAP, FLAG_RESET, MAX_DATAGRAM, MAX_EVENTS,
    MAX_FRAMES,
};

/// Why a datagram got no answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// It didn't parse.
    Wire(WireError),
    /// A reply, not a request.
    NotRequest,
    /// No such partition.
    Partition,
    /// A block at or up to [`WINDOW`] blocks before one already answered:
    /// a duplicate or a datagram that came in late. Answering it would play
    /// the node backwards.
    Stale,
}

/// How far behind the last block a request may be and still count as a
/// late copy of an old block. Further back, it is a new stream (the client
/// started again from 0, or the last block number was damaged or forged),
/// and the partition follows it after resetting its node, so one bad
/// datagram can't lock a partition out.
pub const WINDOW: u64 = 64;

/// Counts for one partition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartitionStats {
    pub answered: u64,
    /// Requests that arrived after one or more blocks went missing.
    pub gaps: u64,
    /// Blocks skipped by those gaps.
    pub skipped: u64,
    pub stale: u64,
    /// New streams: a block far behind the last one, after which the node
    /// starts again from silence.
    pub restarts: u64,
    /// Times the node gave a sample that wasn't finite and was reset.
    pub resets: u64,
}

struct Partition {
    node: Box<dyn Node>,
    next: Option<u64>,
    l: Vec<f32>,
    r: Vec<f32>,
    events: Vec<Timed>,
    stats: PartitionStats,
}

/// The partitions this server renders.
pub struct Server {
    rate: u32,
    parts: Vec<Partition>,
    out: Vec<u8>,
    refused: u64,
}

impl Server {
    pub fn new(rate: u32) -> Server {
        Server {
            rate,
            parts: Vec::new(),
            out: vec![0; MAX_DATAGRAM],
            refused: 0,
        }
    }

    /// Adds a partition and returns its number. The node is prepared here,
    /// off any audio path.
    pub fn add(&mut self, mut node: Box<dyn Node>) -> u16 {
        node.prepare(self.rate, MAX_FRAMES);
        self.parts.push(Partition {
            node,
            next: None,
            l: vec![0.0; MAX_FRAMES],
            r: vec![0.0; MAX_FRAMES],
            events: Vec::with_capacity(MAX_EVENTS),
            stats: PartitionStats::default(),
        });
        (self.parts.len() - 1) as u16
    }

    pub fn partitions(&self) -> usize {
        self.parts.len()
    }

    pub fn stats(&self, partition: u16) -> Option<PartitionStats> {
        self.parts.get(partition as usize).map(|p| p.stats)
    }

    /// Datagrams refused so far, for any reason.
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// Renders the block a request asks for and returns the reply's bytes.
    pub fn answer(&mut self, datagram: &[u8]) -> Result<&[u8], Refused> {
        let got = self.answer_inner(datagram);
        if got.is_err() {
            self.refused += 1;
        }
        let n = got?;
        Ok(&self.out[..n])
    }

    fn answer_inner(&mut self, datagram: &[u8]) -> Result<usize, Refused> {
        let block = wire::parse(datagram).map_err(Refused::Wire)?;
        let h = block.header;
        if h.kind != Kind::Request {
            return Err(Refused::NotRequest);
        }
        let rate = self.rate;
        let p = self
            .parts
            .get_mut(h.partition as usize)
            .ok_or(Refused::Partition)?;
        let mut flags = 0;
        match p.next {
            Some(next) if h.seq < next && next - h.seq <= WINDOW => {
                p.stats.stale += 1;
                return Err(Refused::Stale);
            }
            Some(next) if h.seq < next => {
                p.node.reset();
                flags |= FLAG_GAP;
                p.stats.restarts += 1;
            }
            Some(next) if h.seq > next => {
                flags |= FLAG_GAP;
                p.stats.gaps += 1;
                p.stats.skipped = p.stats.skipped.saturating_add(h.seq - next);
            }
            _ => {}
        }
        p.next = Some(h.seq.saturating_add(1));
        let started = Instant::now();
        let n = h.frames as usize;
        render(p, &block, rate, n);
        if !p.l[..n].iter().chain(&p.r[..n]).all(|s| s.is_finite()) {
            p.node.reset();
            p.l[..n].fill(0.0);
            p.r[..n].fill(0.0);
            flags |= FLAG_RESET;
            p.stats.resets += 1;
        }
        let us = started.elapsed().as_micros().min(u16::MAX as u128) as u16;
        p.stats.answered += 1;
        let reply = Header::reply_to(&h, flags, us);
        let len = wire::encode(&mut self.out, &reply, &[], Some((&p.l[..n], &p.r[..n])))
            .expect("a reply of a parsed request always fits");
        Ok(len)
    }
}

fn render(p: &mut Partition, block: &Block, rate: u32, n: usize) {
    p.events.clear();
    p.events.extend(block.events());
    let (l, r) = (&mut p.l[..n], &mut p.r[..n]);
    if !(block.read_channel(0, l) && block.read_channel(1, r)) {
        // No audio: an instrument adds into silence.
        l.fill(0.0);
        r.fill(0.0);
    } else if p.node.info().kind == NodeKind::Instrument {
        // An instrument given audio still starts from silence: it adds.
        l.fill(0.0);
        r.fill(0.0);
    }
    let ctx = Ctx::new(rate, block.header.frame);
    p.node.process(&ctx, &p.events, l, r);
}

/// What [`serve`] saw by the time it stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServeStats {
    pub received: u64,
    pub answered: u64,
    pub refused: u64,
}

/// Answers requests on `socket` until `stop` is set. It checks `stop` at
/// least every 100 ms.
pub fn serve(socket: &UdpSocket, server: &mut Server, stop: &AtomicBool) -> io::Result<ServeStats> {
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    let mut st = ServeStats::default();
    while !stop.load(Ordering::Relaxed) {
        let (n, from) = match socket.recv_from(&mut buf) {
            Ok(x) => x,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            // A reply that bounced (ICMP port unreachable) shows up as an
            // error on the next read on some systems; it says nothing about
            // the next request.
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => continue,
            Err(e) => return Err(e),
        };
        st.received += 1;
        match server.answer(&buf[..n]) {
            Ok(reply) => {
                // A send that fails (the client went away) is not the
                // server's problem; the next request decides.
                let _ = socket.send_to(reply, from);
                st.answered += 1;
            }
            Err(_) => st.refused += 1,
        }
    }
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::{NodeEvent, NodeInfo, ParamInfo};

    /// Multiplies by a gain that a Param event sets; counts what it saw.
    struct Gain(f32);
    static GAIN_INFO: NodeInfo = NodeInfo {
        id: "test.gain",
        name: "Gain",
        kind: NodeKind::Effect,
    };
    impl Node for Gain {
        fn info(&self) -> &'static NodeInfo {
            &GAIN_INFO
        }
        fn params(&self) -> &'static [ParamInfo] {
            &[]
        }
        fn set_param(&mut self, _id: u16, v: f32) {
            self.0 = v;
        }
        fn param(&self, _id: u16) -> f32 {
            self.0
        }
        fn prepare(&mut self, _rate: u32, _max: usize) {}
        fn reset(&mut self) {}
        fn process(&mut self, _ctx: &Ctx, events: &[Timed], l: &mut [f32], r: &mut [f32]) {
            let mut at = 0;
            for t in events.iter().chain(std::iter::once(&Timed::new(
                l.len() as u32,
                NodeEvent::AllOff,
            ))) {
                let end = (t.offset as usize).min(l.len());
                for i in at..end {
                    l[i] *= self.0;
                    r[i] *= self.0;
                }
                at = end;
                if let NodeEvent::Param { value, .. } = t.ev {
                    self.0 = value;
                }
            }
        }
    }

    fn request(seq: u64, events: &[Timed]) -> Vec<u8> {
        let l: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let mut buf = vec![0u8; MAX_DATAGRAM];
        let n = wire::encode(
            &mut buf,
            &Header::request(0, seq, seq * 16, 16),
            events,
            Some((&l, &l)),
        )
        .unwrap();
        buf.truncate(n);
        buf
    }

    fn left(reply: &[u8]) -> (Header, Vec<f32>) {
        let b = wire::parse(reply).unwrap();
        let mut l = vec![0.0; b.header.frames as usize];
        assert!(b.read_channel(0, &mut l));
        (b.header, l)
    }

    #[test]
    fn a_request_is_rendered_with_its_events_on_their_frames() {
        let mut s = Server::new(44_100);
        let p = s.add(Box::new(Gain(1.0)));
        assert_eq!(p, 0);
        let ev = [Timed::new(8, NodeEvent::Param { id: 0, value: 2.0 })];
        let (h, l) = left(s.answer(&request(0, &ev)).unwrap());
        assert_eq!((h.kind, h.seq, h.flags), (Kind::Reply, 0, 0));
        assert_eq!(l[7], 7.0);
        assert_eq!(l[8], 16.0);
        assert_eq!(s.stats(0).unwrap().answered, 1);
    }

    #[test]
    fn a_lost_request_is_flagged_and_a_late_one_is_refused() {
        let mut s = Server::new(44_100);
        s.add(Box::new(Gain(1.0)));
        s.answer(&request(0, &[])).unwrap();
        let (h, _) = left(s.answer(&request(3, &[])).unwrap());
        assert_eq!(h.flags, FLAG_GAP);
        assert_eq!(s.answer(&request(2, &[])).unwrap_err(), Refused::Stale);
        assert_eq!(s.answer(&request(3, &[])).unwrap_err(), Refused::Stale);
        let st = s.stats(0).unwrap();
        assert_eq!((st.answered, st.gaps, st.skipped, st.stale), (2, 1, 2, 2));
        assert_eq!(s.refused(), 2);
    }

    #[test]
    fn a_block_far_behind_is_a_new_stream_so_one_bad_number_cannot_lock_out_the_rest() {
        let mut s = Server::new(44_100);
        s.add(Box::new(Gain(1.0)));
        s.answer(&request(5, &[])).unwrap();
        // A damaged or forged block number, far ahead: followed as a gap.
        s.answer(&request(1 << 40, &[])).unwrap();
        // The real stream carries on far "behind" it: a new stream.
        let (h, _) = left(s.answer(&request(6, &[])).unwrap());
        assert_eq!(h.flags, FLAG_GAP);
        let (h, _) = left(s.answer(&request(7, &[])).unwrap());
        assert_eq!(h.flags, 0);
        let st = s.stats(0).unwrap();
        assert_eq!((st.answered, st.restarts, st.stale), (4, 1, 0));
    }

    #[test]
    fn junk_replies_and_unknown_partitions_get_no_answer() {
        let mut s = Server::new(44_100);
        s.add(Box::new(Gain(1.0)));
        assert_eq!(
            s.answer(b"hello").unwrap_err(),
            Refused::Wire(WireError::Length)
        );
        let mut other = request(0, &[]);
        other[6] = 9;
        assert_eq!(s.answer(&other).unwrap_err(), Refused::Partition);
        let reply = s.answer(&request(0, &[])).unwrap().to_vec();
        assert_eq!(s.answer(&reply).unwrap_err(), Refused::NotRequest);
    }
}
