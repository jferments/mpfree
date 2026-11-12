//! The MPC's side: a [`Remote`] for the audio thread and a [`Link`] for the
//! control thread.
//!
//! The audio thread never touches the socket. Each period,
//! [`Remote::process`] writes block n's request into a ring and takes block
//! n − L's answer from a table, both lock-free and without allocating. Two
//! threads of the link's own do the network: one sends what the ring holds,
//! the other files each answer under its block number. An answer that isn't
//! there when its turn comes is a **miss**: the block is silent, and the
//! partition carries on with the next one.
//!
//! The ring's slots and the table's samples are atomics, so none of this
//! needs `unsafe`; the table's slots are seqlocks, so a half-written answer
//! is never played.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mpfree_lanes::os;
use mpfree_node::Timed;

use crate::wire::{
    self, Header, Kind, FLAG_CLEAR, FLAG_GAP, FLAG_RESET, MAX_DATAGRAM, MAX_EVENTS, MAX_FRAMES,
};

/// Requests waiting for the sender thread.
pub const RING: usize = 16;
/// Answers kept, by block number. Larger than any latency plus the ring.
pub const TABLE: usize = 64;
/// The longest latency a link takes, in periods.
pub const MAX_LATENCY: u32 = (TABLE - RING) as u32;
/// Requests in a row that carry CLEAR after [`Remote::clear`], so a lost
/// datagram or two don't lose it.
pub const CLEAR_REQUESTS: u32 = 4;
const WORDS: usize = MAX_DATAGRAM / 4;
/// Round trips the receiver keeps for [`Link::close`] (about 48 minutes of
/// periods at 44.1 kHz); later ones are counted but not kept.
const KEEP: usize = 1 << 20;

/// One partition's link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The partition's number on the server.
    pub partition: u16,
    /// How many periods later a block's answer is played (L). 1 to
    /// [`MAX_LATENCY`].
    pub latency: u32,
    /// Frames in every block: the period.
    pub frames: usize,
    /// Whether the block's audio goes with the request: yes for an effect,
    /// no for an instrument, which only needs its events.
    pub send_audio: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            partition: 0,
            latency: 2,
            frames: 128,
            send_audio: true,
        }
    }
}

/// What [`Remote::process`] gave for a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Block n − L's answer, on time.
    Hit,
    /// Block n − L's answer wasn't there (late or lost): silence.
    Miss,
    /// One of the first L blocks, before any answer can be due: silence.
    Warmup,
}

/// The audio thread's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteStats {
    pub blocks: u64,
    pub hits: u64,
    pub misses: u64,
    pub warmup: u64,
    /// Requests dropped because the sender thread had fallen 16 behind.
    pub ring_full: u64,
    /// Events past [`MAX_EVENTS`] in one block, left out.
    pub events_dropped: u64,
    /// Blocks not sent because their input had a sample that isn't finite,
    /// or because the block wasn't `frames` long.
    pub not_sent: u64,
    /// Answers played that the server flagged: a lost request before them,
    /// or a node reset.
    pub gaps: u64,
    pub resets: u64,
}

struct Slot {
    len: AtomicU32,
    words: [AtomicU32; WORDS],
}

struct Answer {
    ver: AtomicU64,
    seq: AtomicU64,
    flags: AtomicU32,
    samples: [AtomicU32; 2 * MAX_FRAMES],
}

struct Shared {
    ring: Box<[Slot]>,
    head: AtomicU64,
    tail: AtomicU64,
    kick: AtomicU32,
    asleep: AtomicU32,
    table: Box<[Answer]>,
    sent_ns: Box<[AtomicU64]>,
    /// The next block number the audio thread will take an answer for.
    taken: AtomicU64,
    epoch: Instant,
    stop: AtomicBool,
}

impl Shared {
    fn new() -> Shared {
        Shared {
            ring: (0..RING)
                .map(|_| Slot {
                    len: AtomicU32::new(0),
                    words: std::array::from_fn(|_| AtomicU32::new(0)),
                })
                .collect(),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            kick: AtomicU32::new(0),
            asleep: AtomicU32::new(0),
            table: (0..TABLE)
                .map(|_| Answer {
                    ver: AtomicU64::new(0),
                    seq: AtomicU64::new(u64::MAX),
                    flags: AtomicU32::new(0),
                    samples: std::array::from_fn(|_| AtomicU32::new(0)),
                })
                .collect(),
            sent_ns: (0..TABLE).map(|_| AtomicU64::new(0)).collect(),
            taken: AtomicU64::new(0),
            epoch: Instant::now(),
            stop: AtomicBool::new(false),
        }
    }

    fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }
}

/// The audio thread's end of a link.
pub struct Remote {
    shared: Arc<Shared>,
    cfg: Config,
    seq: u64,
    /// Requests still to carry CLEAR.
    clear_left: u32,
    scratch: [u8; MAX_DATAGRAM],
    stats: RemoteStats,
}

impl Remote {
    pub fn config(&self) -> Config {
        self.cfg
    }

    pub fn stats(&self) -> RemoteStats {
        self.stats
    }

    /// The number the next block will carry.
    pub fn next_seq(&self) -> u64 {
        self.seq
    }

    /// The partition's latency in frames, for delay compensation.
    pub fn latency_frames(&self) -> u32 {
        self.cfg.latency * self.cfg.frames as u32
    }

    /// Asks the server to reset the node before the next block it renders
    /// (a stop or a project load). The next [`CLEAR_REQUESTS`] requests
    /// carry the CLEAR flag, and the server resets on the first of them
    /// that arrives. The answers to blocks sent before this still come
    /// back, L periods of them: [`crate::offloaded::Offloaded`] plays those
    /// as silence.
    pub fn clear(&mut self) {
        self.clear_left = CLEAR_REQUESTS;
    }

    /// Requests handed over and not yet taken by the sender thread (for
    /// tests that play blocks faster than the period and must not outrun
    /// the ring).
    pub fn queued(&self) -> u64 {
        let sh = &*self.shared;
        sh.head
            .load(Ordering::Acquire)
            .saturating_sub(sh.tail.load(Ordering::Acquire))
    }

    /// Whether block `seq`'s answer has come back and is still kept (for
    /// tests and tools that wait for it instead of a deadline).
    pub fn has_answer(&self, seq: u64) -> bool {
        let a = &self.shared.table[(seq % TABLE as u64) as usize];
        let v = a.ver.load(Ordering::Acquire);
        v & 1 == 0 && a.seq.load(Ordering::Relaxed) == seq
    }

    /// Sends this block (its events, and its audio for an effect) and puts
    /// block n − L's answer in `left` and `right`. `frame` is the block's
    /// first frame on this side's clock. Never blocks, allocates or locks:
    /// at most it wakes the sender thread.
    pub fn process(
        &mut self,
        frame: u64,
        events: &[Timed],
        left: &mut [f32],
        right: &mut [f32],
    ) -> Outcome {
        let n = self.cfg.frames;
        self.stats.blocks += 1;
        let seq = self.seq;
        self.seq += 1;
        if left.len() == n && right.len() == n {
            self.send(seq, frame, events, left, right);
        } else {
            self.stats.not_sent += 1;
        }
        if seq < self.cfg.latency as u64 {
            self.stats.warmup += 1;
            left.fill(0.0);
            right.fill(0.0);
            Outcome::Warmup
        } else {
            let want = seq - self.cfg.latency as u64;
            self.shared.taken.store(want + 1, Ordering::Relaxed);
            if left.len() == n && right.len() == n && self.take(want, left, right) {
                self.stats.hits += 1;
                Outcome::Hit
            } else {
                self.stats.misses += 1;
                left.fill(0.0);
                right.fill(0.0);
                Outcome::Miss
            }
        }
    }

    fn send(&mut self, seq: u64, frame: u64, events: &[Timed], left: &[f32], right: &[f32]) {
        let keep = events.len().min(MAX_EVENTS);
        self.stats.events_dropped += (events.len() - keep) as u64;
        let mut header = Header::request(self.cfg.partition, seq, frame, self.cfg.frames);
        if self.clear_left > 0 {
            self.clear_left -= 1;
            header.flags |= FLAG_CLEAR;
        }
        let audio = self.cfg.send_audio.then_some((left, right));
        let len = match wire::encode(&mut self.scratch, &header, &events[..keep], audio) {
            Ok(len) => len,
            Err(_) => {
                self.stats.not_sent += 1;
                return;
            }
        };
        let sh = &*self.shared;
        let head = sh.head.load(Ordering::Relaxed);
        if head - sh.tail.load(Ordering::Acquire) >= RING as u64 {
            self.stats.ring_full += 1;
            return;
        }
        let slot = &sh.ring[(head % RING as u64) as usize];
        for (w, b) in slot.words.iter().zip(self.scratch[..len].chunks(4)) {
            let mut word = [0u8; 4];
            word[..b.len()].copy_from_slice(b);
            w.store(u32::from_le_bytes(word), Ordering::Relaxed);
        }
        slot.len.store(len as u32, Ordering::Relaxed);
        sh.sent_ns[(seq % TABLE as u64) as usize].store(sh.now_ns(), Ordering::Relaxed);
        // SeqCst against the sender's `asleep`: one of the two sees the
        // other, so a request is never left waiting for a sleeping sender.
        sh.head.store(head + 1, Ordering::SeqCst);
        if sh.asleep.load(Ordering::SeqCst) != 0 {
            sh.kick.fetch_add(1, Ordering::SeqCst);
            os::wake(&sh.kick);
        }
    }

    fn take(&mut self, want: u64, left: &mut [f32], right: &mut [f32]) -> bool {
        let a = &self.shared.table[(want % TABLE as u64) as usize];
        let v1 = a.ver.load(Ordering::Acquire);
        if v1 & 1 == 1 || a.seq.load(Ordering::Relaxed) != want {
            return false;
        }
        let n = self.cfg.frames;
        for (o, s) in left.iter_mut().zip(&a.samples[..n]) {
            *o = f32::from_bits(s.load(Ordering::Relaxed));
        }
        for (o, s) in right.iter_mut().zip(&a.samples[n..2 * n]) {
            *o = f32::from_bits(s.load(Ordering::Relaxed));
        }
        let flags = a.flags.load(Ordering::Relaxed) as u16;
        fence(Ordering::Acquire);
        if a.ver.load(Ordering::Relaxed) != v1 {
            // Overwritten while we read: not this block's answer any more.
            return false;
        }
        if flags & FLAG_GAP != 0 {
            self.stats.gaps += 1;
        }
        if flags & FLAG_RESET != 0 {
            self.stats.resets += 1;
        }
        true
    }
}

/// What a link's threads saw, from [`Link::close`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measured {
    /// Each answer's round trip in µs, from the audio thread handing the
    /// request over to the receiver filing the answer.
    pub round_trip_us: Vec<u32>,
    /// The server's own render time for each answer, in µs.
    pub server_us: Vec<u16>,
    pub sent: u64,
    pub received: u64,
    /// Answers that came back after their turn had passed.
    pub late: u64,
    /// Datagrams that weren't an answer for this partition.
    pub refused: u64,
    /// Sends and reads that failed (no server listening, for one).
    pub errors: u64,
}

/// The control thread's end of a link: it owns the two network threads.
pub struct Link {
    shared: Arc<Shared>,
    sender: Option<JoinHandle<(u64, u64)>>,
    receiver: Option<JoinHandle<Measured>>,
}

/// Opens a link to partition `cfg.partition` on the server at `server`.
pub fn connect(server: SocketAddr, cfg: Config) -> io::Result<(Remote, Link)> {
    if cfg.latency == 0 || cfg.latency > MAX_LATENCY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "latency must be 1 to 48 periods",
        ));
    }
    if cfg.frames == 0 || cfg.frames > MAX_FRAMES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frames must be 1 to 128",
        ));
    }
    let local: SocketAddr = match server {
        SocketAddr::V4(a) if a.ip().is_loopback() => "127.0.0.1:0".parse().unwrap(),
        SocketAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
        SocketAddr::V6(a) if a.ip().is_loopback() => "[::1]:0".parse().unwrap(),
        SocketAddr::V6(_) => "[::]:0".parse().unwrap(),
    };
    let tx = UdpSocket::bind(local)?;
    tx.connect(server)?;
    let rx = tx.try_clone()?;
    rx.set_read_timeout(Some(Duration::from_millis(50)))?;
    let shared = Arc::new(Shared::new());
    let sender = {
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("mpfree-offload-tx".into())
            .spawn(move || send_loop(&sh, &tx))?
    };
    let receiver = {
        let sh = shared.clone();
        let partition = cfg.partition;
        std::thread::Builder::new()
            .name("mpfree-offload-rx".into())
            .spawn(move || receive_loop(&sh, &rx, partition))?
    };
    let remote = Remote {
        shared: shared.clone(),
        cfg,
        seq: 0,
        clear_left: 0,
        scratch: [0; MAX_DATAGRAM],
        stats: RemoteStats::default(),
    };
    Ok((
        remote,
        Link {
            shared,
            sender: Some(sender),
            receiver: Some(receiver),
        },
    ))
}

impl Link {
    /// Stops the network threads and returns what they saw.
    pub fn close(mut self) -> Measured {
        self.stop()
    }

    fn stop(&mut self) -> Measured {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.kick.fetch_add(1, Ordering::SeqCst);
        os::wake(&self.shared.kick);
        let (sent, send_errors) = self
            .sender
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        let mut m = self
            .receiver
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        m.sent = sent;
        m.errors += send_errors;
        m
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        if self.sender.is_some() || self.receiver.is_some() {
            self.stop();
        }
    }
}

fn send_loop(sh: &Shared, socket: &UdpSocket) -> (u64, u64) {
    let mut buf = [0u8; MAX_DATAGRAM];
    let (mut sent, mut errors) = (0u64, 0u64);
    loop {
        let tail = sh.tail.load(Ordering::Relaxed);
        if tail == sh.head.load(Ordering::Acquire) {
            if sh.stop.load(Ordering::SeqCst) {
                return (sent, errors);
            }
            sh.asleep.store(1, Ordering::SeqCst);
            let k = sh.kick.load(Ordering::SeqCst);
            if tail == sh.head.load(Ordering::SeqCst) && !sh.stop.load(Ordering::SeqCst) {
                os::wait(&sh.kick, k, Some(Duration::from_millis(50)));
            }
            sh.asleep.store(0, Ordering::SeqCst);
            continue;
        }
        let slot = &sh.ring[(tail % RING as u64) as usize];
        let len = (slot.len.load(Ordering::Relaxed) as usize).min(MAX_DATAGRAM);
        for (b, w) in buf.chunks_mut(4).zip(&slot.words) {
            b.copy_from_slice(&w.load(Ordering::Relaxed).to_le_bytes());
        }
        sh.tail.store(tail + 1, Ordering::Release);
        match socket.send(&buf[..len]) {
            Ok(_) => sent += 1,
            Err(_) => errors += 1,
        }
    }
}

fn receive_loop(sh: &Shared, socket: &UdpSocket, partition: u16) -> Measured {
    let mut m = Measured {
        round_trip_us: Vec::with_capacity(4096),
        server_us: Vec::with_capacity(4096),
        ..Measured::default()
    };
    let mut buf = [0u8; MAX_DATAGRAM + 1];
    let mut left = [0f32; MAX_FRAMES];
    let mut right = [0f32; MAX_FRAMES];
    while !sh.stop.load(Ordering::SeqCst) {
        let n = match socket.recv(&mut buf) {
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => {
                // Nobody listening (ICMP port unreachable) and the like:
                // count it, wait a little and carry on.
                m.errors += 1;
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
        };
        let now = sh.now_ns();
        let block = match wire::parse(&buf[..n]) {
            Ok(b) if b.header.kind == Kind::Reply && b.header.partition == partition => b,
            _ => {
                m.refused += 1;
                continue;
            }
        };
        let h = block.header;
        let frames = h.frames as usize;
        if !(block.read_channel(0, &mut left[..frames])
            && block.read_channel(1, &mut right[..frames]))
        {
            m.refused += 1;
            continue;
        }
        m.received += 1;
        if h.seq < sh.taken.load(Ordering::Relaxed) {
            m.late += 1;
            continue;
        }
        let a = &sh.table[(h.seq % TABLE as u64) as usize];
        let v = a.ver.load(Ordering::Relaxed);
        a.ver.store(v + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        for (s, x) in a.samples[..frames].iter().zip(&left[..frames]) {
            s.store(x.to_bits(), Ordering::Relaxed);
        }
        for (s, x) in a.samples[frames..2 * frames].iter().zip(&right[..frames]) {
            s.store(x.to_bits(), Ordering::Relaxed);
        }
        a.flags.store(h.flags as u32, Ordering::Relaxed);
        a.seq.store(h.seq, Ordering::Relaxed);
        a.ver.store(v + 2, Ordering::Release);
        if m.round_trip_us.len() < KEEP {
            let sent = sh.sent_ns[(h.seq % TABLE as u64) as usize].load(Ordering::Relaxed);
            m.round_trip_us
                .push((now.saturating_sub(sent) / 1000).min(u32::MAX as u64) as u32);
            m.server_us.push(h.server_us);
        }
    }
    m
}
