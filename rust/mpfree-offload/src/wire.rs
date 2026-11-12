//! The datagram: one block of one partition, each way.
//!
//! Everything is little-endian and fits one UDP datagram under a 1500-byte
//! MTU, so a block is never split and a lost datagram loses one block.
//!
//! ```text
//! offset size  field
//!  0     4     magic "MPFO"
//!  4     1     version (1)
//!  5     1     kind: 1 request (to the server), 2 reply (back)
//!  6     2     partition
//!  8     8     seq: the block's number; it counts up by one each period
//! 16     8     frame: the block's first frame on the MPC's clock
//! 24     2     frames in the block, 1 to MAX_FRAMES
//! 26     1     channels of audio that follow: 0 or 2
//! 27     1     events that follow, 0 to MAX_EVENTS (a reply carries none)
//! 28     2     flags (a request's CLEAR; a reply's GAP and RESET)
//! 30     2     the server's render time for this block in µs (a reply's)
//! 32           events, EVENT_BYTES each, in frame order
//!  .           audio: the left channel's frames, then the right's, as f32
//! ```
//!
//! [`parse`] refuses anything that isn't exactly this: a wrong length, an
//! unknown kind, too many frames or events, an event past the block's end,
//! a value that isn't finite. Nothing a datagram says can make the reader
//! allocate, panic or run anything.

use mpfree_node::{Expr, NodeEvent, Timed};

pub const MAGIC: [u8; 4] = *b"MPFO";
pub const VERSION: u8 = 1;
pub const HEADER_BYTES: usize = 32;
pub const EVENT_BYTES: usize = 12;
/// The largest datagram: an Ethernet MTU of 1500 less the IPv4 and UDP
/// headers.
pub const MAX_DATAGRAM: usize = 1472;
/// The most frames in a block: the MPC's period.
pub const MAX_FRAMES: usize = 128;
/// The most events a request carries: what fits beside a stereo block of
/// `MAX_FRAMES`.
pub const MAX_EVENTS: usize = (MAX_DATAGRAM - HEADER_BYTES - 2 * MAX_FRAMES * 4) / EVENT_BYTES;

/// A reply's flag: the server never saw one or more blocks before this one
/// (a request was lost), so the partition skipped those frames.
pub const FLAG_GAP: u16 = 1;
/// A reply's flag: the node gave a sample that isn't finite, so the server
/// reset it and sends silence for this block.
pub const FLAG_RESET: u16 = 2;
/// A request's flag: reset the node before this block (the host stopped
/// or loaded a project). A client sets it on a few requests in a row, so
/// one lost datagram doesn't lose it, and the server resets on the first
/// of a run.
pub const FLAG_CLEAR: u16 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Request = 1,
    Reply = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// Shorter than a header, longer than `MAX_DATAGRAM`, or not the length
    /// its header promises.
    Length,
    Magic,
    Version,
    Kind,
    /// No frames, or more than `MAX_FRAMES`.
    Frames,
    /// Channels other than 0 or 2.
    Channels,
    /// More than `MAX_EVENTS`, or events in a reply.
    Events,
    /// An event of an unknown kind, past the block's end, or out of order.
    Event,
    /// A value (an event's or a sample) that isn't finite.
    NotFinite,
    /// The buffer given to [`encode`] is too small, or the audio slices are
    /// not `frames` long.
    Buffer,
    /// A block number or first frame of 2^62 or more (3 million years of
    /// periods): never real, and near enough to overflow that a node's
    /// frame arithmetic could trip on it.
    Range,
}

/// The first block number and frame [`parse`] refuses.
pub const MAX_COUNT: u64 = 1 << 62;

/// What a datagram's first 32 bytes say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub partition: u16,
    pub seq: u64,
    pub frame: u64,
    pub frames: u16,
    pub channels: u8,
    pub events: u8,
    pub flags: u16,
    pub server_us: u16,
}

impl Header {
    /// A request's header, with `events` and `channels` filled in by
    /// [`encode`].
    pub fn request(partition: u16, seq: u64, frame: u64, frames: usize) -> Header {
        Header {
            kind: Kind::Request,
            partition,
            seq,
            frame,
            frames: frames as u16,
            channels: 0,
            events: 0,
            flags: 0,
            server_us: 0,
        }
    }

    /// The reply to `req`.
    pub fn reply_to(req: &Header, flags: u16, server_us: u16) -> Header {
        Header {
            kind: Kind::Reply,
            channels: 2,
            events: 0,
            flags,
            server_us,
            ..*req
        }
    }

    /// The datagram's whole length.
    pub fn len(&self) -> usize {
        HEADER_BYTES
            + self.events as usize * EVENT_BYTES
            + self.channels as usize * self.frames as usize * 4
    }

    /// Never true: a header alone is 32 bytes (clippy wants the pair).
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// A datagram that [`parse`] accepted, borrowing its bytes.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    pub header: Header,
    events: &'a [u8],
    audio: &'a [u8],
}

impl<'a> Block<'a> {
    /// The block's events, already checked.
    pub fn events(&self) -> impl Iterator<Item = Timed> + 'a {
        self.events
            .chunks_exact(EVENT_BYTES)
            .map(|e| read_event(e).expect("checked by parse"))
    }

    /// Copies channel `ch` (0 left, 1 right) into `out`, which must be
    /// `frames` long. Returns false if the block has no audio.
    pub fn read_channel(&self, ch: usize, out: &mut [f32]) -> bool {
        let n = self.header.frames as usize;
        if self.header.channels != 2 || ch > 1 || out.len() != n {
            return false;
        }
        let bytes = &self.audio[ch * n * 4..(ch + 1) * n * 4];
        for (o, b) in out.iter_mut().zip(bytes.chunks_exact(4)) {
            *o = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
        true
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// Reads and checks one datagram.
pub fn parse(d: &[u8]) -> Result<Block<'_>, WireError> {
    if d.len() < HEADER_BYTES || d.len() > MAX_DATAGRAM {
        return Err(WireError::Length);
    }
    if d[0..4] != MAGIC {
        return Err(WireError::Magic);
    }
    if d[4] != VERSION {
        return Err(WireError::Version);
    }
    let kind = match d[5] {
        1 => Kind::Request,
        2 => Kind::Reply,
        _ => return Err(WireError::Kind),
    };
    let header = Header {
        kind,
        partition: u16_at(d, 6),
        seq: u64_at(d, 8),
        frame: u64_at(d, 16),
        frames: u16_at(d, 24),
        channels: d[26],
        events: d[27],
        flags: u16_at(d, 28),
        server_us: u16_at(d, 30),
    };
    let frames = header.frames as usize;
    if frames == 0 || frames > MAX_FRAMES {
        return Err(WireError::Frames);
    }
    if header.seq >= MAX_COUNT || header.frame >= MAX_COUNT {
        return Err(WireError::Range);
    }
    if header.channels != 0 && header.channels != 2 {
        return Err(WireError::Channels);
    }
    if header.events as usize > MAX_EVENTS || (kind == Kind::Reply && header.events != 0) {
        return Err(WireError::Events);
    }
    if d.len() != header.len() {
        return Err(WireError::Length);
    }
    let ev_end = HEADER_BYTES + header.events as usize * EVENT_BYTES;
    let events = &d[HEADER_BYTES..ev_end];
    let audio = &d[ev_end..];
    let mut last = 0u32;
    for e in events.chunks_exact(EVENT_BYTES) {
        let t = read_event(e)?;
        if t.offset as usize >= frames || t.offset < last {
            return Err(WireError::Event);
        }
        last = t.offset;
    }
    for s in audio.chunks_exact(4) {
        if !f32::from_le_bytes([s[0], s[1], s[2], s[3]]).is_finite() {
            return Err(WireError::NotFinite);
        }
    }
    Ok(Block {
        header,
        events,
        audio,
    })
}

// An event, 12 bytes: offset u16, kind u8, x u8, id u16, y u8, a zero byte,
// value f32. What x, id, y and value mean depends on the kind:
//
//   kind 1 NoteOn     x note, y velocity
//   kind 2 NoteOff    x note
//   kind 3 Pressure   x note, y value
//   kind 4 Bend       value
//   kind 5 Expression x note, y the expression (0 slide, 1 tone, 2 release), value
//   kind 6 AllOff
//   kind 7 Param      id, value

fn expr_code(e: Expr) -> u8 {
    match e {
        Expr::Slide => 0,
        Expr::Tone => 1,
        Expr::Release => 2,
    }
}

fn write_event(t: &Timed, out: &mut [u8]) -> Result<(), WireError> {
    let (kind, x, id, y, value) = match t.ev {
        NodeEvent::NoteOn { note, velocity } => (1u8, note, 0u16, velocity, 0.0f32),
        NodeEvent::NoteOff { note } => (2, note, 0, 0, 0.0),
        NodeEvent::Pressure { note, value } => (3, note, 0, value, 0.0),
        NodeEvent::Bend { value } => (4, 0, 0, 0, value),
        NodeEvent::Expression { note, kind, value } => (5, note, 0, expr_code(kind), value),
        NodeEvent::AllOff => (6, 0, 0, 0, 0.0),
        NodeEvent::Param { id, value } => (7, 0, id, 0, value),
    };
    if !value.is_finite() {
        return Err(WireError::NotFinite);
    }
    if t.offset > u16::MAX as u32 {
        return Err(WireError::Event);
    }
    out[0..2].copy_from_slice(&(t.offset as u16).to_le_bytes());
    out[2] = kind;
    out[3] = x;
    out[4..6].copy_from_slice(&id.to_le_bytes());
    out[6] = y;
    out[7] = 0;
    out[8..12].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn read_event(e: &[u8]) -> Result<Timed, WireError> {
    let offset = u16_at(e, 0) as u32;
    let x = e[3];
    let id = u16_at(e, 4);
    let y = e[6];
    let value = f32::from_le_bytes([e[8], e[9], e[10], e[11]]);
    if !value.is_finite() {
        return Err(WireError::NotFinite);
    }
    let ev = match e[2] {
        1 => NodeEvent::NoteOn {
            note: x,
            velocity: y,
        },
        2 => NodeEvent::NoteOff { note: x },
        3 => NodeEvent::Pressure { note: x, value: y },
        4 => NodeEvent::Bend { value },
        5 => NodeEvent::Expression {
            note: x,
            kind: match y {
                0 => Expr::Slide,
                1 => Expr::Tone,
                2 => Expr::Release,
                _ => return Err(WireError::Event),
            },
            value,
        },
        6 => NodeEvent::AllOff,
        7 => NodeEvent::Param { id, value },
        _ => return Err(WireError::Event),
    };
    Ok(Timed { offset, ev })
}

/// Writes one datagram into `out` and returns its length. `events` go in a
/// request only; `audio` (left, right) must each be `frames` long. The
/// header's `events` and `channels` are set from what is given.
pub fn encode(
    out: &mut [u8],
    header: &Header,
    events: &[Timed],
    audio: Option<(&[f32], &[f32])>,
) -> Result<usize, WireError> {
    let frames = header.frames as usize;
    if frames == 0 || frames > MAX_FRAMES {
        return Err(WireError::Frames);
    }
    if events.len() > MAX_EVENTS || (header.kind == Kind::Reply && !events.is_empty()) {
        return Err(WireError::Events);
    }
    if header.seq >= MAX_COUNT || header.frame >= MAX_COUNT {
        return Err(WireError::Range);
    }
    let h = Header {
        events: events.len() as u8,
        channels: if audio.is_some() { 2 } else { 0 },
        ..*header
    };
    let len = h.len();
    if out.len() < len || len > MAX_DATAGRAM {
        return Err(WireError::Buffer);
    }
    if let Some((l, r)) = audio {
        if l.len() != frames || r.len() != frames {
            return Err(WireError::Buffer);
        }
    }
    out[0..4].copy_from_slice(&MAGIC);
    out[4] = VERSION;
    out[5] = h.kind as u8;
    out[6..8].copy_from_slice(&h.partition.to_le_bytes());
    out[8..16].copy_from_slice(&h.seq.to_le_bytes());
    out[16..24].copy_from_slice(&h.frame.to_le_bytes());
    out[24..26].copy_from_slice(&h.frames.to_le_bytes());
    out[26] = h.channels;
    out[27] = h.events;
    out[28..30].copy_from_slice(&h.flags.to_le_bytes());
    out[30..32].copy_from_slice(&h.server_us.to_le_bytes());
    let mut at = HEADER_BYTES;
    let mut last = 0u32;
    for t in events {
        if t.offset as usize >= frames || t.offset < last {
            return Err(WireError::Event);
        }
        last = t.offset;
        write_event(t, &mut out[at..at + EVENT_BYTES])?;
        at += EVENT_BYTES;
    }
    if let Some((l, r)) = audio {
        for s in l.iter().chain(r) {
            if !s.is_finite() {
                return Err(WireError::NotFinite);
            }
            out[at..at + 4].copy_from_slice(&s.to_le_bytes());
            at += 4;
        }
    }
    debug_assert_eq!(at, len);
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, k: f32) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * k).sin() * 0.5).collect()
    }

    #[test]
    fn a_full_request_fits_one_datagram_and_reads_back() {
        assert_eq!(MAX_EVENTS, 34);
        let (l, r) = (tone(128, 0.1), tone(128, 0.2));
        let mut events: Vec<Timed> = (0..MAX_EVENTS as u32)
            .map(|i| {
                Timed::new(
                    i * 3,
                    NodeEvent::Param {
                        id: i as u16,
                        value: i as f32 * 0.5,
                    },
                )
            })
            .collect();
        events[0] = Timed::new(
            0,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 100,
            },
        );
        events[1] = Timed::new(
            3,
            NodeEvent::Expression {
                note: 61,
                kind: Expr::Release,
                value: 0.25,
            },
        );
        let mut buf = [0u8; MAX_DATAGRAM];
        let h = Header::request(7, 1234, 99_000, 128);
        let n = encode(&mut buf, &h, &events, Some((&l, &r))).unwrap();
        assert_eq!(n, HEADER_BYTES + MAX_EVENTS * EVENT_BYTES + 2 * 128 * 4);
        assert!(n <= MAX_DATAGRAM);
        let b = parse(&buf[..n]).unwrap();
        assert_eq!(
            (b.header.partition, b.header.seq, b.header.frame),
            (7, 1234, 99_000)
        );
        assert_eq!(b.events().collect::<Vec<_>>(), events);
        let (mut l2, mut r2) = (vec![0.0; 128], vec![0.0; 128]);
        assert!(b.read_channel(0, &mut l2) && b.read_channel(1, &mut r2));
        assert_eq!((l2, r2), (l, r));
    }

    #[test]
    fn a_reply_carries_audio_flags_and_the_server_time() {
        let (l, r) = (tone(64, 0.3), tone(64, 0.4));
        let req = Header::request(1, 5, 640, 64);
        let mut buf = [0u8; MAX_DATAGRAM];
        let n = encode(
            &mut buf,
            &Header::reply_to(&req, FLAG_GAP, 321),
            &[],
            Some((&l, &r)),
        )
        .unwrap();
        let b = parse(&buf[..n]).unwrap();
        assert_eq!(b.header.kind, Kind::Reply);
        assert_eq!(
            (b.header.flags, b.header.server_us, b.header.seq),
            (FLAG_GAP, 321, 5)
        );
    }

    #[test]
    fn what_does_not_fit_or_is_not_finite_is_refused() {
        let mut buf = [0u8; MAX_DATAGRAM];
        let l = tone(128, 0.1);
        let h = Header::request(0, 0, 0, 128);
        let many = vec![Timed::new(0, NodeEvent::AllOff); MAX_EVENTS + 1];
        assert_eq!(encode(&mut buf, &h, &many, None), Err(WireError::Events));
        let late = [Timed::new(128, NodeEvent::AllOff)];
        assert_eq!(encode(&mut buf, &h, &late, None), Err(WireError::Event));
        let nan = [Timed::new(0, NodeEvent::Bend { value: f32::NAN })];
        assert_eq!(encode(&mut buf, &h, &nan, None), Err(WireError::NotFinite));
        let mut bad = l.clone();
        bad[5] = f32::INFINITY;
        assert_eq!(
            encode(&mut buf, &h, &[], Some((&l, &bad))),
            Err(WireError::NotFinite)
        );
        assert_eq!(
            encode(&mut buf, &h, &[], Some((&l, &l[..64]))),
            Err(WireError::Buffer)
        );
        let big = Header::request(0, 0, 0, MAX_FRAMES + 1);
        assert_eq!(encode(&mut buf, &big, &[], None), Err(WireError::Frames));
    }

    #[test]
    fn damaged_headers_are_named() {
        let mut buf = [0u8; MAX_DATAGRAM];
        let n = encode(&mut buf, &Header::request(0, 0, 0, 16), &[], None).unwrap();
        let ok = buf[..n].to_vec();
        let with = |at: usize, v: u8| {
            let mut d = ok.clone();
            d[at] = v;
            parse(&d).map(|b| b.header)
        };
        assert_eq!(with(0, b'X').unwrap_err(), WireError::Magic);
        assert_eq!(with(4, 2).unwrap_err(), WireError::Version);
        assert_eq!(with(5, 3).unwrap_err(), WireError::Kind);
        assert_eq!(with(24, 0).unwrap_err(), WireError::Frames);
        assert_eq!(with(26, 1).unwrap_err(), WireError::Channels);
        assert_eq!(with(26, 2).unwrap_err(), WireError::Length);
        assert_eq!(with(27, 1).unwrap_err(), WireError::Length);
        assert_eq!(parse(&ok[..n - 1]).unwrap_err(), WireError::Length);
        let mut far = ok.clone();
        far[15] = 0x40;
        assert_eq!(parse(&far).unwrap_err(), WireError::Range);
        assert!(parse(&ok).is_ok());
    }
}
