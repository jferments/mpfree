#!/usr/bin/env python3
"""Synthetic, zero-device tests for offline legacy uImage inspection."""
from __future__ import annotations

import importlib.util
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "inspect-p3-uimage.py"
spec = importlib.util.spec_from_file_location("mpfree_inspect_p3", SCRIPT)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def make_disk(path, *, payload=b"MPFREE_TEST_PAYLOAD", bad_header_crc=False,
              bad_payload_crc=False, size_override=None):
    hdr = bytearray(audit.UIMAGE_HEADER_SIZE)
    struct.pack_into(">I", hdr, 0, audit.UIMAGE_MAGIC)
    struct.pack_into(">I", hdr, 8, 1700000000)
    struct.pack_into(">I", hdr, 12, len(payload) if size_override is None else size_override)
    struct.pack_into(">I", hdr, 16, 0x00100000)
    struct.pack_into(">I", hdr, 20, 0x00100040)
    struct.pack_into(">I", hdr, 24, zlib.crc32(payload) ^ (1 if bad_payload_crc else 0))
    hdr[28:32] = bytes((5, 2, 2, 0))
    hdr[32:32 + len(b"mpfree-test")] = b"mpfree-test"
    struct.pack_into(">I", hdr, 4, zlib.crc32(hdr) ^ (1 if bad_header_crc else 0))
    with path.open("wb") as stream:
        stream.seek(audit.P3_LBA * audit.SECTOR)
        stream.write(hdr)
        stream.write(payload)


class P3UImageTests(unittest.TestCase):
    def test_good_header_payload_crc_and_source_immutable(self):
        with tempfile.TemporaryDirectory() as directory:
            disk = Path(directory) / "emmc.img"
            make_disk(disk)
            before = disk.read_bytes()
            report = audit.inspect(disk)
            self.assertEqual(report["magic"], "0x27051956")
            self.assertEqual(report["load_address"], "0x00100000")
            self.assertEqual(report["entry_address"], "0x00100040")
            self.assertTrue(report["header_crc32_valid"])
            self.assertTrue(report["payload_crc32_valid"])
            self.assertEqual(disk.read_bytes(), before)

    def test_crc_mismatch_is_reported_not_misrepresented_as_signature(self):
        with tempfile.TemporaryDirectory() as directory:
            disk = Path(directory) / "emmc.img"
            make_disk(disk, bad_header_crc=True, bad_payload_crc=True)
            result = audit.inspect(disk)
            self.assertFalse(result["header_crc32_valid"])
            self.assertFalse(result["payload_crc32_valid"])
            self.assertIn("not a cryptographic signature", result["security_note"])

    def test_rejects_symlinks_devices_and_partition_overflow(self):
        with tempfile.TemporaryDirectory() as directory:
            disk = Path(directory) / "emmc.img"
            make_disk(disk)
            alias = Path(directory) / "alias"
            alias.symlink_to(disk)
            with self.assertRaises(OSError):
                audit.inspect(alias)
            with self.assertRaises(audit.ImageValidationError):
                audit.inspect(Path("/dev/null"))
            make_disk(disk, size_override=audit.P3_SECTORS * audit.SECTOR)
            with self.assertRaisesRegex(audit.ImageValidationError, "capacity"):
                audit.inspect(disk)

    def test_rejects_truncated_and_wrong_magic(self):
        with tempfile.TemporaryDirectory() as directory:
            disk = Path(directory) / "emmc.img"
            disk.write_bytes(b"too short")
            with self.assertRaises(audit.ImageValidationError):
                audit.inspect(disk)
            make_disk(disk)
            with disk.open("r+b") as stream:
                stream.seek(audit.P3_LBA * audit.SECTOR)
                stream.write(b"BAD!")
            with self.assertRaises(audit.ImageValidationError):
                audit.inspect(disk)


if __name__ == "__main__":
    unittest.main()
