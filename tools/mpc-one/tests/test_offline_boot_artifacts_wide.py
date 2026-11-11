"""Synthetic tests for the new offline, read-only boot-chain audit.
Never accesses an MPC, Ghidra project, device, or private backup.
"""
import ast
import hashlib
import importlib.util
from pathlib import Path
import struct
import subprocess
import unittest
import zlib
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
MODULE = ROOT / "tools" / "mpc-one" / "offline-boot-artifacts-wide.py"
SPEC = importlib.util.spec_from_file_location("mpfree_boot_artifacts_wide", MODULE)
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)

class OfflineArtifactsWideTests(unittest.TestCase):
    def test_single_copy_environment_crc_and_boot_variables(self):
        body = b"bootcmd=run az01_bootcmd\0boot_targets=mmc0 mmc1\0serial#=private\0\0"
        raw = bytearray(8192)
        raw[4:4+len(body)] = body
        raw[:4] = struct.pack("<I", zlib.crc32(raw[4:]) & 0xffffffff)
        parsed = audit.valid_env(bytes(raw))
        self.assertTrue(parsed["crc_valid_for_standard_layout"])
        self.assertEqual(parsed["boot_related_variables"]["boot_targets"], "mmc0 mmc1")
        self.assertNotIn("serial#", parsed["boot_related_variables"])
        self.assertNotIn("serial#", str(parsed))

    def test_environment_bad_crc_is_reported_not_silently_accepted(self):
        raw = bytearray(8192)
        raw[4:4+len(b"bootcmd=run az01_bootcmd\0\0")] = b"bootcmd=run az01_bootcmd\0\0"
        raw[:4] = struct.pack("<I", zlib.crc32(raw[4:]) ^ 0xffffffff)
        self.assertFalse(audit.valid_env(raw)["crc_valid_for_standard_layout"])

    def make_image(self):
        body = bytes([x % 256 for x in range(2500)])
        h = bytearray(64)
        struct.pack_into(">7I", h, 0, 0x27051956, 0, 0, len(body),
                         0x00100000, 0, zlib.crc32(body) & 0xffffffff)
        h[29] = 2 # ARM
        h[30] = 5 # firmware / U-Boot image type
        h[32:38] = b"U-Boot"
        struct.pack_into(">I", h, 4, zlib.crc32(h) & 0xffffffff)
        p3 = h + body
        p3.extend(bytes(768*512-len(p3)))
        return bytes(p3), body

    def test_uimage_header_payload_checks_and_sector_count(self):
        image, body = self.make_image()
        with patch.object(audit, "EXPECTED_P3_PAYLOAD", hashlib.sha256(body).hexdigest()):
            parsed = audit.parse_uimage(image)
        self.assertEqual(parsed["entry_address"], "0x00000000")
        self.assertEqual(parsed["load_address"], "0x00100000")
        self.assertEqual(parsed["sectors_used"], (len(body)+64+511)//512)
        self.assertTrue(parsed["header_crc32_valid"])
        self.assertTrue(parsed["payload_crc32_valid"])

    def test_uimage_corrupted_body_fails_integrity_check(self):
        image, body = self.make_image()
        damaged = bytearray(image)
        damaged[75] ^= 1
        with patch.object(audit, "EXPECTED_P3_PAYLOAD", hashlib.sha256(body).hexdigest()):
            with self.assertRaisesRegex(RuntimeError, "integrity"):
                audit.parse_uimage(bytes(damaged))

    def test_all_python_scripts_parse_without_ghidra_runtime(self):
        for relative in (
            "tools/mpc-one/offline-boot-artifacts-wide.py",
            "tools/ghidra/MPFreeBootChainWideAudit.py",
        ):
            file = ROOT / relative
            with self.subTest(file=relative):
                ast.parse(file.read_text(encoding="utf-8"), filename=str(file))

    def test_bash_wrapper_parses(self):
        script = ROOT / "tools" / "ghidra" / "run_wide_bootchain_audit.sh"
        result = subprocess.run(["bash", "-n", str(script)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

if __name__ == "__main__":
    unittest.main()
