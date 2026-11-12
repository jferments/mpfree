"""Synthetic legacy U-Boot script parser tests, no device or vendor code."""
import importlib.util
from pathlib import Path
import struct
import unittest
import zlib

SOURCE = Path(__file__).resolve().parents[1] / "verify_uboot_script_wrapper.py"
SPEC = importlib.util.spec_from_file_location("mpfree_script_wrapper_test", SOURCE)
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


def make_script(commands: str, timestamp: int = 4) -> bytes:
    raw = commands.encode("utf-8")
    payload = struct.pack(">II", len(raw), 0) + raw
    crc = zlib.crc32(payload) & 0xffffffff
    header = struct.pack(">7I4B32s", M.IH_MAGIC, 0, timestamp,
                         len(payload), 0, 0, crc,
                         5, 2, 6, 0, b"test-script")
    header_crc = zlib.crc32(header) & 0xffffffff
    header = header[:4] + struct.pack(">I", header_crc) + header[8:]
    return header + payload


def simulated_od(image: bytes) -> str:
    return ("~ # od -An -v -tx1 /boot/boot.scr.uimg\n" +
            "\n".join(" " + image[i:i + 16].hex(" ")
                      for i in range(0, len(image), 16)) +
            "\n~ # echo MPFREE_VM_ACK\n")


class ScriptWrapperTests(unittest.TestCase):
    def test_matching_image_and_payload(self):
        commands = "\nbootz 0x2000000 - 0x1f00000\n"
        image = make_script(commands)
        report = M.compare_guest_to_update(simulated_od(image), {
            "commands": commands,
            "sha256": __import__("hashlib").sha256(image).hexdigest(),
        })
        self.assertTrue(report["header_crc32_verified"])
        self.assertTrue(report["payload_crc32_verified"])
        self.assertTrue(report["script_commands_match_official"])
        self.assertTrue(report["full_image_matches_official"])

    def test_only_header_timestamp_differs(self):
        commands = "\nbootz 0x2000000 - 0x1f00000\n"
        first = make_script(commands, timestamp=10)
        other = make_script(commands, timestamp=11)
        report = M.compare_guest_to_update(simulated_od(other), {
            "commands": commands,
            "sha256": __import__("hashlib").sha256(first).hexdigest(),
        })
        self.assertTrue(report["script_commands_match_official"])
        self.assertFalse(report["full_image_matches_official"])
        self.assertTrue(report["different_image_even_if_commands_match"])

    def test_different_commands_are_not_hidden(self):
        original = "\nbootz 0x2000000\n"
        guest = make_script("\nbootm 0x2000000\n")
        report = M.compare_guest_to_update(simulated_od(guest), {
            "commands": original,
            "sha256": "f" * 64,
        })
        self.assertFalse(report["script_commands_match_official"])
        self.assertTrue(report["command_diff_excerpt"])

    def test_bad_header_crc_fails_closed(self):
        image = bytearray(make_script("bootz\n"))
        image[16] ^= 0x5
        with self.assertRaises(ValueError):
            M.inspect_script_image(bytes(image))

    def test_bad_payload_crc_fails_closed(self):
        image = bytearray(make_script("bootz\n"))
        image[-1] ^= 0x5
        with self.assertRaises(ValueError):
            M.inspect_script_image(bytes(image))

    def test_bad_image_type_rejected(self):
        image = bytearray(make_script("bootz\n"))
        image[30] = 5
        with self.assertRaises(ValueError):
            M.inspect_script_image(bytes(image))

    def test_no_hex_report_fails_closed(self):
        with self.assertRaises(ValueError):
            M.decode_od_hex("~ # command not found\n")

    def test_od_dump_does_not_parse_partial_shell_noise(self):
        image = make_script("bootz\n")
        text = simulated_od(image) + "xx 11 22 33\n"
        self.assertEqual(M.decode_od_hex(text), image)

if __name__ == "__main__":
    unittest.main()
