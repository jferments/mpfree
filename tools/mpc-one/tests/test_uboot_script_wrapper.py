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


def official_fixture(image: bytes, include_full_sha: bool = True) -> dict:
    import hashlib
    size = int.from_bytes(image[12:16], "big")
    assert len(image) == 64 + size
    script_len = int.from_bytes(image[64:68], "big")
    row = {
        "commands": image[72:72 + script_len].decode("utf-8").rstrip("\n"),
        "sha256": hashlib.sha256(image[72:72 + script_len]).hexdigest(),
        "header": {
            "header_sha256": hashlib.sha256(image[:64]).hexdigest(),
            "image_bytes": len(image),
            "multi_image_script_length": script_len,
        },
    }
    if include_full_sha:
        row["image_sha256"] = hashlib.sha256(image).hexdigest()
    return row


def simulated_od(image: bytes) -> str:
    lines = ["~ # od -v -b /boot/boot.scr.uimg"]
    for offset in range(0, len(image), 16):
        words = " ".join("%03o" % v for v in image[offset:offset + 16])
        lines.append("%07o %s" % (offset, words))
    lines.append("%07o" % len(image))
    lines.append("~ # echo MPFREE_VM_ACK")
    return "\n".join(lines) + "\n"

class ScriptWrapperTests(unittest.TestCase):
    def test_matching_image_and_payload(self):
        commands = "\nbootz 0x2000000 - 0x1f00000\n"
        image = make_script(commands)
        report = M.compare_guest_to_update(
            simulated_od(image), official_fixture(image))
        self.assertTrue(report["header_crc32_verified"])
        self.assertTrue(report["payload_crc32_verified"])
        self.assertTrue(report["script_commands_match_official"])
        self.assertTrue(report["full_image_matches_official"])
        self.assertTrue(report["canonical_header_and_script_bytes_match_official"])
        self.assertTrue(report["raw_script_sha256_matches_official"])

    def test_only_header_timestamp_differs(self):
        commands = "\nbootz 0x2000000 - 0x1f00000\n"
        first = make_script(commands, timestamp=10)
        other = make_script(commands, timestamp=11)
        report = M.compare_guest_to_update(
            simulated_od(other), official_fixture(first))
        self.assertTrue(report["script_commands_match_official"])
        self.assertFalse(report["full_image_matches_official"])
        self.assertTrue(report["raw_script_sha256_matches_official"])
        self.assertFalse(report["header_sha256_matches_official"])
        self.assertFalse(report["canonical_header_and_script_bytes_match_official"])
        self.assertTrue(report["different_image_even_if_commands_match"])

    def test_full_image_hash_must_not_be_misread_as_payload_hash(self):
        image = make_script("bootz 0x2000000\n")
        record = official_fixture(image, include_full_sha=False)
        report = M.compare_guest_to_update(simulated_od(image), record)
        self.assertIsNone(report["full_image_matches_official"])
        self.assertTrue(report["header_sha256_matches_official"])
        self.assertTrue(report["raw_script_sha256_matches_official"])
        self.assertTrue(report["canonical_header_and_script_bytes_match_official"])
        self.assertEqual(report["source_official_sha256_field_means"],
                         "raw_script_data_not_uimage")

    def test_official_raw_payload_hash_difference_fails_closed(self):
        image = make_script("bootz 0x2000000\n")
        record = official_fixture(image, include_full_sha=False)
        record["sha256"] = "f" * 64
        report = M.compare_guest_to_update(simulated_od(image), record)
        self.assertFalse(report["canonical_header_and_script_bytes_match_official"])

    def test_different_commands_are_not_hidden(self):
        original = "\nbootz 0x2000000\n"
        guest = make_script("\nbootm 0x2000000\n")
        report = M.compare_guest_to_update(
            simulated_od(guest), official_fixture(make_script(original)))
        self.assertFalse(report["script_commands_match_official"])
        self.assertTrue(report["command_diff_excerpt"])

    def test_busybox_offset_gap_is_rejected(self):
        image = make_script("bootz\n")
        data = simulated_od(image)
        # Skip the first block so the next row starts at offset 020.
        lines = data.splitlines()
        lines.pop(1)
        with self.assertRaisesRegex(ValueError, "offset"):
            M.decode_od_bytes("\n".join(lines))

    def test_busybox_terminal_offset_missing_rejected(self):
        image = make_script("bootz\n")
        lines = simulated_od(image).splitlines()
        lines.pop(-2)
        with self.assertRaisesRegex(ValueError, "terminal"):
            M.decode_od_bytes("\n".join(lines))

    def test_busybox_octal_byte_overflow_rejected(self):
        image = make_script("bootz\n")
        dump = simulated_od(image)
        dump = dump.replace(" 047 ", " 777 ", 1)
        with self.assertRaisesRegex(ValueError, "out-of-range"):
            M.decode_od_bytes(dump)

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

    def test_no_octals_report_fails_closed(self):
        with self.assertRaises(ValueError):
            M.decode_od_hex("~ # command not found\n")

    def test_busybox_octal_dump_ignores_shell_noise(self):
        image = make_script("bootz\n")
        text = simulated_od(image) + "xx 11 22 33\n"
        self.assertEqual(M.decode_od_hex(text), image)

if __name__ == "__main__":
    unittest.main()
