"""Host-only experimental MPFree PCM datagram format and bounded playout model.

This is NOT a production network stack or MPC executable:
- no sockets, no listening ports, no ALSA/MIDI/DRM, no device modification;
- used to test sample-frame sequencing and deadline failures in Python;
- future native C++ implementation must use fixed preallocated ring buffers,
  authorization, clock synchronization and off-thread packet parsing.
"""
from dataclasses import dataclass
import struct

MAGIC = b"MPF1"
VERSION = 1
FORMAT_S32_LE = 1
HEADER = struct.Struct("!4sBBHIIQHH")
HEADER_BYTES = HEADER.size  # 28 bytes
ETHERNET_MTU = 1500
IPV4_UDP_HEADER = 28
MAX_DATAGRAM = ETHERNET_MTU - IPV4_UDP_HEADER  # 1472 bytes, no IP options
MAX_PCM_DATA = MAX_DATAGRAM - HEADER_BYTES
MAX_CHANNELS = 8
MAX_FRAMES = 256

class PacketError(ValueError):
    pass


@dataclass(frozen=True)
class AudioPacket:
    stream_id: int
    sequence: int
    start_frame: int
    frames: int
    channels: int
    payload: bytes

    def to_bytes(self) -> bytes:
        if not 0 <= self.stream_id < 2**32:
            raise PacketError("stream_id out of range")
        if not 0 <= self.sequence < 2**32:
            raise PacketError("sequence out of range")
        if not 0 <= self.start_frame < 2**64:
            raise PacketError("start_frame out of range")
        if not 1 <= self.frames <= MAX_FRAMES:
            raise PacketError("invalid audio frames per datagram")
        if not 1 <= self.channels <= MAX_CHANNELS:
            raise PacketError("invalid audio channels per datagram")
        if len(self.payload) != self.frames * self.channels * 4:
            raise PacketError("payload byte count disagrees with S32_LE geometry")
        if len(self.payload) > MAX_PCM_DATA:
            raise PacketError("payload exceeds IPv4/UDP datagram budget")
        return HEADER.pack(
            MAGIC, VERSION, FORMAT_S32_LE, 0,
            self.stream_id, self.sequence, self.start_frame,
            self.frames, self.channels
        ) + self.payload

    @staticmethod
    def from_bytes(blob: bytes) -> "AudioPacket":
        if len(blob) < HEADER_BYTES or len(blob) > MAX_DATAGRAM:
            raise PacketError("invalid datagram size")
        magic, version, sample_format, flags, sid, seq, frame, count, channels = (
            HEADER.unpack_from(blob)
        )
        if magic != MAGIC or version != VERSION:
            raise PacketError("invalid magic or protocol version")
        if sample_format != FORMAT_S32_LE or flags != 0:
            raise PacketError("unknown PCM format or reserved flags")
        if not 1 <= count <= MAX_FRAMES or not 1 <= channels <= MAX_CHANNELS:
            raise PacketError("invalid PCM geometry")
        payload = blob[HEADER_BYTES:]
        if len(payload) != count * channels * 4:
            raise PacketError("truncated or overlong PCM")
        return AudioPacket(sid, seq, frame, count, channels, payload)


class BoundedPlayout:
    """Host-only fixed-window deadline model, not thread-safe or real-time safe.

    start_frame refers to the local playout timeline, already aligned.
    Clock drift compensation and network sequence authentication are out of scope.
    """

    def __init__(self, stream_id: int, *,
                 channels: int = 2, block_frames: int = 128,
                 capacity_blocks: int = 8, start_frame: int = 0):
        if not 0 <= stream_id < 2**32:
            raise ValueError("invalid stream_id")
        if not 1 <= channels <= MAX_CHANNELS:
            raise ValueError("invalid channels")
        if block_frames <= 0 or block_frames > MAX_FRAMES:
            raise ValueError("invalid block length")
        if block_frames * channels * 4 > MAX_PCM_DATA:
            raise ValueError("one block would exceed Ethernet MTU")
        if not 1 <= capacity_blocks <= 64:
            raise ValueError("invalid window capacity")
        if start_frame < 0 or start_frame % block_frames:
            raise ValueError("unaligned start_frame")
        self.stream_id = stream_id
        self.channels = channels
        self.block_frames = block_frames
        self.capacity_blocks = capacity_blocks
        self.next_frame = start_frame
        self.pending: dict[int, bytes] = {}
        self.silence = bytes(block_frames * channels * 4)
        self.received = 0
        self.duplicates = 0
        self.late = 0
        self.too_early = 0
        self.rejected = 0
        self.underflows = 0

    def ingest(self, packet: AudioPacket) -> bool:
        if (packet.stream_id != self.stream_id or
            packet.frames != self.block_frames or
            packet.channels != self.channels or
            packet.start_frame % self.block_frames):
            self.rejected += 1
            return False
        if packet.start_frame < self.next_frame:
            self.late += 1
            return False
        if packet.start_frame >= (self.next_frame +
                                  self.capacity_blocks * self.block_frames):
            self.too_early += 1
            return False
        if packet.start_frame in self.pending:
            self.duplicates += 1
            return False
        self.pending[packet.start_frame] = packet.payload
        self.received += 1
        return True

    def playout(self) -> bytes:
        """Advance one playout block, returning silence if deadline was missed."""
        payload = self.pending.pop(self.next_frame, None)
        self.next_frame += self.block_frames
        if payload is None:
            self.underflows += 1
            return self.silence
        return payload
