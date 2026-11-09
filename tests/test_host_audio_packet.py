#!/usr/bin/env python3
"""Deterministic/offline tests: no sockets, device operations or remote access."""
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools/mpc-one"))
from host_audio_packet import (  # noqa: E402
    AudioPacket, BoundedPlayout, HEADER_BYTES, MAX_DATAGRAM, PacketError,
)


def pcm_block(value: int, frames: int = 128, channels: int = 2) -> bytes:
    return struct.pack("<i", value) * (frames * channels)


class TestPacketCodec(unittest.TestCase):
    def packet(self, seq=0, frame=0, frames=128, channels=2):
        return AudioPacket(
            stream_id=19, sequence=seq, start_frame=frame, frames=frames,
            channels=channels, payload=pcm_block(seq + 1, frames, channels)
        )

    def test_stereo_128_frame_packet_fits_standard_mtu(self):
        pkt = self.packet()
        data = pkt.to_bytes()
        self.assertEqual(len(data), HEADER_BYTES + 1024)
        self.assertLessEqual(len(data), MAX_DATAGRAM)
        self.assertEqual(AudioPacket.from_bytes(data), pkt)
        self.assertEqual(data[:4], b"MPF1")

    def test_big_multichannel_packet_rejected_before_fragmentation(self):
        with self.assertRaisesRegex(PacketError, "MTU|datagram"):
            self.packet(channels=8).to_bytes()
        pkt = self.packet(frames=32, channels=8)
        self.assertEqual(AudioPacket.from_bytes(pkt.to_bytes()), pkt)

    def test_truncation_corruption_and_extra_bytes(self):
        pkt = self.packet()
        blob = pkt.to_bytes()
        for bad in (b"", blob[:5], blob[:-1], blob + b"\x00",
                    b"WRNG" + blob[4:],
                    blob[:4] + b"\x02" + blob[5:],
                    blob[:5] + b"\x02" + blob[6:],
                    blob[:6] + b"\x00\x01" + blob[8:]):
            with self.subTest(bad=bad[:12]):
                with self.assertRaises(PacketError):
                    AudioPacket.from_bytes(bad)

    def test_incorrect_payload_and_sequence_range(self):
        with self.assertRaises(PacketError):
            AudioPacket(1, 0, 0, 128, 2, b"\x00").to_bytes()
        with self.assertRaises(PacketError):
            AudioPacket(1, 2**32, 0, 128, 2, pcm_block(1)).to_bytes()
        with self.assertRaises(PacketError):
            AudioPacket(1, 0, -1, 128, 2, pcm_block(1)).to_bytes()


class TestBoundedPlayout(unittest.TestCase):
    def setUp(self):
        self.packets = [
            AudioPacket(19, seq, seq * 128, 128, 2, pcm_block(seq+1))
            for seq in range(6)
        ]
        self.buf = BoundedPlayout(19, capacity_blocks=8)

    def test_reordering_before_deadline(self):
        self.assertTrue(self.buf.ingest(self.packets[2]))
        self.assertTrue(self.buf.ingest(self.packets[0]))
        self.assertTrue(self.buf.ingest(self.packets[1]))
        for i in range(3):
            self.assertEqual(self.buf.playout(), self.packets[i].payload)
        self.assertEqual(self.buf.underflows, 0)

    def test_lost_packet_and_late_arrival_yield_bounded_silence(self):
        self.assertTrue(self.buf.ingest(self.packets[0]))
        self.assertTrue(self.buf.ingest(self.packets[2]))
        self.assertEqual(self.buf.playout(), self.packets[0].payload)
        self.assertEqual(self.buf.playout(), bytes(1024))
        self.assertFalse(self.buf.ingest(self.packets[1]))
        self.assertEqual(self.buf.late, 1)
        self.assertEqual(self.buf.playout(), self.packets[2].payload)
        self.assertEqual(self.buf.underflows, 1)

    def test_duplicate_and_far_future_rejected(self):
        self.assertTrue(self.buf.ingest(self.packets[0]))
        self.assertFalse(self.buf.ingest(self.packets[0]))
        self.assertEqual(self.buf.duplicates, 1)
        future = AudioPacket(19, 9, 8 * 128, 128, 2, pcm_block(2))
        self.assertFalse(self.buf.ingest(future))
        self.assertEqual(self.buf.too_early, 1)
        self.assertLessEqual(len(self.buf.pending), 8)

    def test_wrong_stream_geometry_and_alignment(self):
        self.assertFalse(self.buf.ingest(AudioPacket(20, 0, 0, 128, 2, pcm_block(1))))
        self.assertFalse(self.buf.ingest(AudioPacket(19, 0, 1, 128, 2, pcm_block(1))))
        self.assertFalse(self.buf.ingest(AudioPacket(19, 0, 0, 64, 2, pcm_block(1, 64))))
        self.assertEqual(self.buf.rejected, 3)

    def test_capacity_and_max_frames(self):
        with self.assertRaises(ValueError):
            BoundedPlayout(19, channels=8, block_frames=128)
        with self.assertRaises(ValueError):
            BoundedPlayout(19, capacity_blocks=0)
        with self.assertRaises(ValueError):
            BoundedPlayout(19, capacity_blocks=1000)


if __name__ == "__main__":
    unittest.main()
