"""Compatibility inventory tests use fake ELF and preset fixtures only."""

from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from mpfree_tools.compat_survey import main, signature_info, survey
from mpfree_tools.content import validate_sources


def fake_arm_elf() -> bytes:
    header = bytearray(64)
    header[:4] = b"\x7fELF"
    header[4] = 1   # ELFCLASS32
    header[5] = 1   # little endian
    header[18:20] = (40).to_bytes(2, "little")  # EM_ARM
    return bytes(header)


class CompatibilitySurveyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "content-backup"
        self.source.mkdir()

    def test_detects_arm_plugin_candidate_without_executing_it(self) -> None:
        path = self.source / "synth.so"
        path.write_bytes(fake_arm_elf())
        info = signature_info(path)
        self.assertEqual(info["signature"], "ELF")
        self.assertEqual(info["elf_class"], "32-bit")
        self.assertEqual(info["elf_machine"], "ARM")
        records, errors = survey(validate_sources([self.source]))
        self.assertFalse(errors)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["kind"], "native-library-candidate")

    def test_lists_preset_and_archive_candidates_without_copying(self) -> None:
        (self.source / "program.xpm").write_text("<xml>example</xml>")
        (self.source / "expansion.xpn").write_bytes(b"PK\x03\x04" + b"\x00" * 12)
        (self.source / "audio.wav").write_bytes(b"RIFF" + b"\x00" * 16)
        records, errors = survey(validate_sources([self.source]))
        self.assertFalse(errors)
        self.assertEqual(
            {r["relative_path"] for r in records},
            {"program.xpm", "expansion.xpn"},
        )
        self.assertEqual(
            next(r for r in records if r["relative_path"] == "expansion.xpn")["signature"],
            "ZIP",
        )
        self.assertEqual((self.source / "expansion.xpn").read_bytes()[:4], b"PK\x03\x04")

    def test_skips_symlinks(self) -> None:
        (self.source / "real.so").write_bytes(fake_arm_elf())
        try:
            (self.source / "link.so").symlink_to(self.source / "real.so")
        except (OSError, NotImplementedError):
            self.skipTest("Symlinks unavailable")
        records, _ = survey(validate_sources([self.source]))
        self.assertEqual([r["relative_path"] for r in records], ["real.so"])

    def test_cli_report_contains_metadata_not_binary_data(self) -> None:
        (self.source / "synth.so").write_bytes(fake_arm_elf())
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            code = main(["--source", str(self.source), "--hash"])
        self.assertEqual(code, 0)
        report = json.loads(stdout.getvalue())
        self.assertEqual(report["command"], "compatibility-survey")
        self.assertEqual(report["candidates"][0]["elf_machine"], "ARM")
        self.assertEqual(len(report["candidates"][0]["sha256"]), 64)
        self.assertNotIn("content", report["candidates"][0])

    def test_rejects_manifest_in_source(self) -> None:
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                main([
                    "--source", str(self.source),
                    "--manifest", str(self.source / "report.json"),
                ])
        self.assertEqual(raised.exception.code, 2)
        self.assertFalse((self.source / "report.json").exists())


if __name__ == "__main__":
    unittest.main()
