#!/usr/bin/env python3
"""Pure-memory parser tests for read-only V13 rootfs boot-asset inventory."""
import importlib.util
from pathlib import Path
import struct
import unittest
import zlib

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "tools/mpc-one/offline_fastboot_boot_assets_v13.py"
spec = importlib.util.spec_from_file_location("mpfree_v13_assets_test", SCRIPT)
assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assets)

def make_uimage(commands):
    data = commands.encode("utf-8")
    payload = struct.pack(">II", len(data), 0) + data
    h = bytearray(struct.pack(">7I4B32s",
        assets.MAGIC_UIMAGE, 0, 0, len(payload), 0, 0,
        zlib.crc32(payload) & 0xffffffff,
        5, 2, 6, 0, b"test-script\x00"))
    crc = zlib.crc32(h) & 0xffffffff
    h[4:8] = struct.pack(">I", crc)
    return bytes(h) + payload

def make_fdt():
    # Small valid FDT: root -> chosen with stdout-path property.
    def align(b):
        return b + b"\x00" * ((-len(b)) % 4)
    strings = b"stdout-path\x00"
    struct_block = b"".join([
        struct.pack(">I", 1), align(b"\x00"),
        struct.pack(">I", 1), align(b"chosen\x00"),
        struct.pack(">III", 3, len(b"serial2:115200n8\x00"), 0),
        align(b"serial2:115200n8\x00"),
        struct.pack(">I", 2), struct.pack(">I", 2),
        struct.pack(">I", 9),
    ])
    off_res = 40
    off_struct = off_res + 16
    off_strings = off_struct + len(struct_block)
    total = off_strings + len(strings)
    header = struct.pack(">10I", assets.MAGIC_FDT, total,
                         off_struct, off_strings, off_res, 17, 16,
                         0, len(strings), len(struct_block))
    return header + bytes(16) + struct_block + strings

class OfflineParsers(unittest.TestCase):
    def test_uimage_crc_and_script_parsing(self):
        blob = make_uimage("echo test\nbootz 0x02000000\n")
        r = assets.uimage(blob)
        self.assertTrue(r["valid"])
        self.assertTrue(r["header_crc_valid"])
        self.assertTrue(r["payload_crc_valid"])
        self.assertEqual(r["script"]["normalized_command_lines"],
                         ["echo test", "bootz 0x02000000"])

    def test_uimage_detects_corruption(self):
        blob = bytearray(make_uimage("echo test\n"))
        blob[-1] ^= 1
        r = assets.uimage(bytes(blob))
        self.assertTrue(r["header_crc_valid"])
        self.assertFalse(r["payload_crc_valid"])

    def test_fdt_selected_property_decode(self):
        result = assets.fdt_properties(make_fdt())
        self.assertTrue(result["valid"])
        self.assertEqual(result["nodes"], 2)
        self.assertEqual(result["properties"], 1)
        self.assertEqual(result["selected_properties"]["/chosen"]["stdout-path"],
                         ["serial2:115200n8"])

    def test_fdt_rejects_wrong_magic_and_truncated_data(self):
        data = bytearray(make_fdt())
        data[0] = 0
        self.assertFalse(assets.fdt_properties(bytes(data))["valid"])
        self.assertFalse(assets.fdt_properties(b"short")["valid"])

if __name__ == "__main__":
    unittest.main(verbosity=2)
