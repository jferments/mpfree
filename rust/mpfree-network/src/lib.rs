//! MPFree experimental UDP PCM framing and playout.
//! Host-only transport model. No sockets, crypto, device access or allocation
//! in the packet codec or render-side playout. This is NOT a production secure
//! audio transport: a real network session requires authentication, pairing,
//! anti-replay, time synchronization and a non-RT socket ingress thread.
#![forbid(unsafe_code)]

pub const MAGIC: &[u8; 4] = b"MPF1";
pub const VERSION: u8 = 1;
pub const FORMAT_S32_LE: u8 = 1;
pub const HEADER_BYTES: usize = 28;
pub const MAX_DATAGRAM: usize = 1472; // IPv4 UDP under 1500-byte MTU
pub const MAX_PCM_DATA: usize = MAX_DATAGRAM - HEADER_BYTES;
pub const MAX_CHANNELS: usize = 8;
pub const MAX_FRAMES: usize = 256;
pub const MAX_WINDOW_BLOCKS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketError {
    DatagramSize,
    InvalidHeader,
    InvalidGeometry,
    PayloadLength,
    MtuExceeded,
    BufferTooSmall,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AudioPacket<'a> {
    pub stream_id: u32,
    pub sequence: u32,
    pub start_frame: u64,
    pub frames: u16,
    pub channels: u16,
    pub payload: &'a [u8],
}

impl<'a> AudioPacket<'a> {
    fn validate(&self) -> Result<usize, PacketError> {
        if !(1..=MAX_FRAMES as u16).contains(&self.frames)
            || !(1..=MAX_CHANNELS as u16).contains(&self.channels)
        {
            return Err(PacketError::InvalidGeometry);
        }
        let expected = usize::from(self.frames) * usize::from(self.channels) * 4;
        if self.payload.len() != expected {
            return Err(PacketError::PayloadLength);
        }
        if expected > MAX_PCM_DATA {
            return Err(PacketError::MtuExceeded);
        }
        Ok(expected + HEADER_BYTES)
    }

    /// Encode into caller-owned bytes; never allocates.
    /// 28-byte header big-endian; payload is interleaved S32_LE.
    pub fn encode_into(&self, output: &mut [u8]) -> Result<usize, PacketError> {
        let len = self.validate()?;
        if output.len() < len {
            return Err(PacketError::BufferTooSmall);
        }
        output[0..4].copy_from_slice(MAGIC);
        output[4] = VERSION;
        output[5] = FORMAT_S32_LE;
        output[6..8].fill(0);
        output[8..12].copy_from_slice(&self.stream_id.to_be_bytes());
        output[12..16].copy_from_slice(&self.sequence.to_be_bytes());
        output[16..24].copy_from_slice(&self.start_frame.to_be_bytes());
        output[24..26].copy_from_slice(&self.frames.to_be_bytes());
        output[26..28].copy_from_slice(&self.channels.to_be_bytes());
        output[HEADER_BYTES..len].copy_from_slice(self.payload);
        Ok(len)
    }

    /// Validate fully before exposing bytes to the rest of the application.
    /// No dynamic allocations and no panics on malformed packets.
    pub fn parse(blob: &'a [u8]) -> Result<Self, PacketError> {
        if !(HEADER_BYTES..=MAX_DATAGRAM).contains(&blob.len()) {
            return Err(PacketError::DatagramSize);
        }
        if &blob[0..4] != MAGIC
            || blob[4] != VERSION
            || blob[5] != FORMAT_S32_LE
            || blob[6] != 0
            || blob[7] != 0
        {
            return Err(PacketError::InvalidHeader);
        }
        let packet = Self {
            stream_id: u32::from_be_bytes(blob[8..12].try_into().unwrap()),
            sequence: u32::from_be_bytes(blob[12..16].try_into().unwrap()),
            start_frame: u64::from_be_bytes(blob[16..24].try_into().unwrap()),
            frames: u16::from_be_bytes(blob[24..26].try_into().unwrap()),
            channels: u16::from_be_bytes(blob[26..28].try_into().unwrap()),
            payload: &blob[HEADER_BYTES..],
        };
        packet.validate()?;
        Ok(packet)
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    pub received: u64,
    pub duplicate: u64,
    pub late: u64,
    pub too_early: u64,
    pub rejected: u64,
    pub underflows: u64,
}

#[derive(Clone)]
struct Slot {
    frame: Option<u64>,
    len: usize,
    payload: [u8; MAX_PCM_DATA],
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            frame: None,
            len: 0,
            payload: [0; MAX_PCM_DATA],
        }
    }
}

/// Owns at most 64 preallocated packets; all allocations occur in new().
/// Ingress and playout MUST be driven from one owning thread or have an
/// externally validated SPSC ownership bridge. Never use a mutex in RT code.
pub struct BoundedPlayout {
    stream_id: u32,
    channels: u16,
    block_frames: u16,
    next_frame: u64,
    slots: Vec<Slot>,
    counters: Counters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestResult {
    Accepted,
    Duplicate,
    Late,
    TooEarly,
    Rejected,
}

impl BoundedPlayout {
    pub fn new(
        stream_id: u32,
        channels: u16,
        block_frames: u16,
        capacity_blocks: usize,
        start_frame: u64,
    ) -> Result<Self, PacketError> {
        if !(1..=MAX_CHANNELS as u16).contains(&channels)
            || !(1..=MAX_FRAMES as u16).contains(&block_frames)
            || !(1..=MAX_WINDOW_BLOCKS).contains(&capacity_blocks)
            || usize::from(channels) * usize::from(block_frames) * 4 > MAX_PCM_DATA
            || !start_frame.is_multiple_of(u64::from(block_frames))
        {
            return Err(PacketError::InvalidGeometry);
        }
        Ok(Self {
            stream_id,
            channels,
            block_frames,
            next_frame: start_frame,
            slots: vec![Slot::default(); capacity_blocks],
            counters: Counters::default(),
        })
    }
    pub fn next_frame(&self) -> u64 {
        self.next_frame
    }
    pub fn counters(&self) -> Counters {
        self.counters
    }
    pub fn capacity_blocks(&self) -> usize {
        self.slots.len()
    }
    pub fn block_bytes(&self) -> usize {
        usize::from(self.block_frames) * usize::from(self.channels) * 4
    }
    pub fn ingest(&mut self, packet: &AudioPacket<'_>) -> IngestResult {
        if packet.stream_id != self.stream_id
            || packet.frames != self.block_frames
            || packet.channels != self.channels
            || !packet
                .start_frame
                .is_multiple_of(u64::from(self.block_frames))
            || packet.validate().is_err()
        {
            self.counters.rejected += 1;
            return IngestResult::Rejected;
        }
        if packet.start_frame < self.next_frame {
            self.counters.late += 1;
            return IngestResult::Late;
        }
        let end = self
            .next_frame
            .saturating_add(self.slots.len() as u64 * u64::from(self.block_frames));
        if packet.start_frame >= end {
            self.counters.too_early += 1;
            return IngestResult::TooEarly;
        }
        // Absolute-frame-slot index is collision-free within the bounded
        // playout window. Incoming packets from previous rotations are late.
        let index =
            (packet.start_frame / u64::from(self.block_frames) % self.slots.len() as u64) as usize;
        let slot = &mut self.slots[index];
        if slot.frame == Some(packet.start_frame) {
            self.counters.duplicate += 1;
            return IngestResult::Duplicate;
        }
        slot.payload[..packet.payload.len()].copy_from_slice(packet.payload);
        slot.len = packet.payload.len();
        slot.frame = Some(packet.start_frame);
        self.counters.received += 1;
        IngestResult::Accepted
    }
    /// Copies one block into caller-provided PCM memory. Silence on a miss.
    /// No malloc, locks or syscalls. Caller must supply exact block byte count.
    pub fn playout_into(&mut self, output: &mut [u8]) -> Result<bool, PacketError> {
        if output.len() != self.block_bytes() {
            return Err(PacketError::BufferTooSmall);
        }
        let index =
            (self.next_frame / u64::from(self.block_frames) % self.slots.len() as u64) as usize;
        let slot = &mut self.slots[index];
        let played = slot.frame == Some(self.next_frame) && slot.len == output.len();
        if played {
            output.copy_from_slice(&slot.payload[..slot.len]);
        } else {
            output.fill(0);
            self.counters.underflows += 1;
        }
        slot.frame = None;
        slot.len = 0;
        self.next_frame = self.next_frame.saturating_add(u64::from(self.block_frames));
        Ok(played)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_payload(seed: u8) -> [u8; 1024] {
        let mut b = [0; 1024];
        for (i, x) in b.iter_mut().enumerate() {
            *x = seed.wrapping_add(i as u8);
        }
        b
    }
    #[test]
    fn python_wire_header_and_roundtrip() {
        let payload = test_payload(4);
        let p = AudioPacket {
            stream_id: 0x1020_3040,
            sequence: 0x5060_7080,
            start_frame: 0x0102_0304_0506_0708,
            frames: 128,
            channels: 2,
            payload: &payload,
        };
        let mut bytes = [0; MAX_DATAGRAM];
        let n = p.encode_into(&mut bytes).unwrap();
        assert_eq!(n, 28 + 1024);
        assert_eq!(
            &bytes[..28],
            &[
                0x4d, 0x50, 0x46, 0x31, 0x01, 0x01, 0x00, 0x00, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60,
                0x70, 0x80, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x00, 0x80, 0x00, 0x02
            ]
        );
        assert_eq!(AudioPacket::parse(&bytes[..n]).unwrap(), p);
        assert_eq!(
            AudioPacket::parse(&bytes[..n - 1]),
            Err(PacketError::PayloadLength)
        );
    }
    #[test]
    fn bad_packets_never_panic_and_are_rejected() {
        let data = test_payload(3);
        let p = AudioPacket {
            stream_id: 1,
            sequence: 2,
            start_frame: 0,
            frames: 128,
            channels: 2,
            payload: &data,
        };
        let mut bytes = [0; MAX_DATAGRAM];
        let n = p.encode_into(&mut bytes).unwrap();
        for len in 0..n {
            if len != n {
                assert!(AudioPacket::parse(&bytes[..len]).is_err());
            }
        }
        for index in [0, 4, 5, 6, 7, 24, 25, 26, 27] {
            let mut corrupt = bytes[..n].to_vec();
            corrupt[index] ^= 0x40;
            // A change to a valid frame count would still fail payload length.
            assert!(AudioPacket::parse(&corrupt).is_err(), "index {index}");
        }
        let oversized = AudioPacket {
            stream_id: 1,
            sequence: 0,
            start_frame: 0,
            frames: 128,
            channels: 8,
            payload: &[0; 4096],
        };
        assert_eq!(
            oversized.encode_into(&mut bytes),
            Err(PacketError::MtuExceeded)
        );
    }
    #[test]
    fn bounded_reordering_loss_and_duplicates() {
        let mut recv = BoundedPlayout::new(19, 2, 128, 8, 0).unwrap();
        let a = test_payload(0);
        let b = test_payload(1);
        let c = test_payload(2);
        fn mk(seq: u32, frame: u64, payload: &[u8]) -> AudioPacket<'_> {
            AudioPacket {
                stream_id: 19,
                sequence: seq,
                start_frame: frame,
                frames: 128,
                channels: 2,
                payload,
            }
        }
        assert_eq!(recv.ingest(&mk(2, 256, &c)), IngestResult::Accepted);
        assert_eq!(recv.ingest(&mk(0, 0, &a)), IngestResult::Accepted);
        assert_eq!(recv.ingest(&mk(0, 0, &a)), IngestResult::Duplicate);
        let mut out = [0u8; 1024];
        assert_eq!(recv.playout_into(&mut out), Ok(true));
        assert_eq!(out, a);
        assert_eq!(recv.playout_into(&mut out), Ok(false));
        assert_eq!(out, [0u8; 1024]);
        assert_eq!(recv.ingest(&mk(1, 128, &b)), IngestResult::Late);
        assert_eq!(recv.playout_into(&mut out), Ok(true));
        assert_eq!(out, c);
        assert_eq!(recv.counters().underflows, 1);
        assert_eq!(recv.counters().late, 1);
        assert_eq!(recv.counters().duplicate, 1);
        let far = mk(100, recv.next_frame() + 8 * 128, &a);
        assert_eq!(recv.ingest(&far), IngestResult::TooEarly);
    }
    #[test]
    fn reject_wrong_stream_and_buffer_without_advancing() {
        let mut recv = BoundedPlayout::new(19, 2, 128, 8, 0).unwrap();
        let data = test_payload(0);
        let wrong = AudioPacket {
            stream_id: 20,
            sequence: 0,
            start_frame: 0,
            frames: 128,
            channels: 2,
            payload: &data,
        };
        assert_eq!(recv.ingest(&wrong), IngestResult::Rejected);
        assert_eq!(
            recv.playout_into(&mut [0u8; 2]),
            Err(PacketError::BufferTooSmall)
        );
        assert_eq!(recv.next_frame(), 0);
        assert!(BoundedPlayout::new(19, 8, 128, 8, 0).is_err());
    }
}
