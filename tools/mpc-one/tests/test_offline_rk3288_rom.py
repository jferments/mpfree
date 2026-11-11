#!/usr/bin/env python3
"""Offline synthetic-fixture tests; NEVER opens an MPC or a disk device."""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import os
import struct
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "offline-rk3288-rom.py"
spec = importlib.util.spec_from_file_location("rom_preflight", SCRIPT)
rom = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rom)

EXPORT_SCRIPT = Path(__file__).resolve().parents[1] / "extract-p1-sram-for-ghidra.py"
export_spec = importlib.util.spec_from_file_location("mpfree_sram_export", EXPORT_SCRIPT)
exporter = importlib.util.module_from_spec(export_spec)
export_spec.loader.exec_module(exporter)


class RK3288RomTests(unittest.TestCase):
    def test_rc4_known_vector_and_roundtrip(self):
        # Published RC4 test vector for independent basic algorithm sanity.
        self.assertEqual(rom.rc4(b"Plaintext", b"Key").hex(),
                         "bbf316e8d940af0ad3")
        blob = bytes(range(256)) + bytes(range(256))
        self.assertEqual(rom.rc4(rom.rc4(blob)), blob)

    def make_backup(self, parent, *, bad_magic=False, encoded_payload=False):
        header = bytearray(512)
        struct.pack_into("<I", header, 0, 0 if bad_magic else rom.RK_HEADER_MAGIC)
        struct.pack_into("<I", header, 8, 0 if encoded_payload else 1)
        struct.pack_into("<H", header, 12, 4)
        struct.pack_into("<H", header, 506, 4)
        struct.pack_into("<H", header, 508, 12)
        p1 = bytearray(rom.P1_SIZE)
        p1[:512] = rom.rc4(bytes(header))
        payload = b"RK32" + b"\0" * 508
        p1[2048:2560] = rom.rc4(payload) if encoded_payload else payload
        backup = parent / "simulated-emmc-backup.img"
        with backup.open("wb") as fd:
            fd.write(bytes(rom.IDBLOCK_LBA * rom.SECTOR))
            fd.write(p1)
        return backup, hashlib.sha256(p1).hexdigest()

    def test_idblock_plain_and_encoded(self):
        for encrypt in (False, True):
            with self.subTest(encoded_payload=encrypt):
                with tempfile.TemporaryDirectory() as root:
                    backup, sha = self.make_backup(Path(root), encoded_payload=encrypt)
                    output = io.StringIO()
                    with contextlib.redirect_stdout(output):
                        rom.inspect_idblock(backup, sha)
                    self.assertIn("baseline hash: VERIFIED", output.getvalue())
                    self.assertIn("First-stage identifier: RK32", output.getvalue())
                    self.assertIn("no file or hardware was modified", output.getvalue())

    def test_upstream_second_stage_allowance(self):
        self.assertEqual(rom.RK_MAX_BOOT_SIZE, 512 * 1024)
        with tempfile.TemporaryDirectory() as root:
            backup, sha = self.make_backup(Path(root))
            # This deliberately represents the exact owner's 64-sector
            # init + 1024-sector default maximum boot-stage allowance.
            with backup.open("r+b") as fd:
                fd.seek(rom.IDBLOCK_LBA * rom.SECTOR)
                encrypted_header = fd.read(rom.SECTOR)
                header = bytearray(rom.rc4(encrypted_header))
                struct.pack_into("<H", header, 506, 64)
                struct.pack_into("<H", header, 508, 1088)
                fd.seek(rom.IDBLOCK_LBA * rom.SECTOR)
                fd.write(rom.rc4(bytes(header)))
            # The modified fixture no longer matches the original fixture hash.
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                rom.inspect_idblock(backup, None)
            text = output.getvalue()
            self.assertIn("Declared init+boot size: 1088 sectors (557056 bytes)", text)
            self.assertIn("Nominal second-stage allowance: 524288 bytes", text)
            self.assertIn("not proof that 512 KiB", text)

    def test_private_sram_export_is_readonly_to_source(self):
        with tempfile.TemporaryDirectory() as root:
            work = Path(root)
            backup, _ = self.make_backup(work)
            # Apply exact reference metadata to synthetic fixtures; never
            # substitute a real proprietary p1 file into a unit test.
            with backup.open("r+b") as f:
                f.seek(rom.IDBLOCK_LBA * rom.SECTOR)
                header = bytearray(rom.rc4(f.read(512)))
                struct.pack_into("<H", header, 506, 64)
                struct.pack_into("<H", header, 508, 1088)
                f.seek(rom.IDBLOCK_LBA * rom.SECTOR)
                f.write(rom.rc4(bytes(header)))
            original_digest = hashlib.sha256(backup.read_bytes()).hexdigest()
            with backup.open("rb") as f:
                f.seek(rom.IDBLOCK_LBA * rom.SECTOR)
                sha = hashlib.sha256(f.read(rom.P1_SIZE)).hexdigest()

            old = exporter.rk.MPC_REFERENCE_P1_SHA256
            try:
                exporter.rk.MPC_REFERENCE_P1_SHA256 = sha
                extracted = work / "private" / "rk32-sram.bin"
                with contextlib.redirect_stdout(io.StringIO()):
                    exporter.extract(backup, extracted)
                self.assertEqual(len(extracted.read_bytes()), 32768)
                self.assertEqual(extracted.read_bytes()[:4], b"RK32")
                self.assertEqual(original_digest, hashlib.sha256(backup.read_bytes()).hexdigest())
                with self.assertRaisesRegex(ValueError, "Destination exists"):
                    exporter.extract(backup, extracted)
                with self.assertRaisesRegex(ValueError, "device or symlink"):
                    exporter.extract(Path("/dev/null"), work / "dev.bin")
            finally:
                exporter.rk.MPC_REFERENCE_P1_SHA256 = old

    def test_idblock_bad_magic_and_wrong_hash(self):
        with tempfile.TemporaryDirectory() as root:
            backup, sha = self.make_backup(Path(root), bad_magic=True)
            with self.assertRaisesRegex(ValueError, "signature mismatch"):
                rom.inspect_idblock(backup, sha)
        with tempfile.TemporaryDirectory() as root:
            backup, sha = self.make_backup(Path(root))
            with self.assertRaisesRegex(ValueError, "differs from expected"):
                rom.inspect_idblock(backup, "0" * 64)

    def test_plain_and_encoded_rkimage(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "test.rkimage"
            payload = b"RK32" + bytes(range(100))
            for image in (payload, rom.rc4(payload)):
                path.write_bytes(image)
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    rom.inspect_rkimage(path)
                self.assertIn("No image was loaded", output.getvalue())

    def test_device_and_symlink_are_rejected(self):
        with self.assertRaises(ValueError):
            rom.inspect_idblock(Path("/dev/null"), None)
        with tempfile.TemporaryDirectory() as root:
            original = Path(root) / "original"
            link = Path(root) / "link"
            original.write_bytes(b"RK32")
            link.symlink_to(original)
            with self.assertRaisesRegex(ValueError, "symlink"):
                rom.inspect_rkimage(link)

    def test_rkimage_max_size_limit(self):
        with tempfile.TemporaryDirectory() as root:
            file = Path(root) / "maxsize.rkimage"
            file.write_bytes(b"RK32" + bytes(32768 - 4))
            with contextlib.redirect_stdout(io.StringIO()):
                rom.inspect_rkimage(file)
            file.write_bytes(b"RK32" + bytes(32769 - 4))
            with self.assertRaisesRegex(ValueError, "32768 bytes"):
                rom.inspect_rkimage(file)

    def test_invalid_rkimage(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "bad.rkimage"
            path.write_bytes(b"INVALID")
            with self.assertRaisesRegex(ValueError, "Neither plain nor RC4"):
                rom.inspect_rkimage(path)


if __name__ == "__main__":
    unittest.main()
