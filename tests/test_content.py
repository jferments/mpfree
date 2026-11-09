"""Synthetic fixtures only: no Akai factory content is required."""

from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
import wave
from pathlib import Path

from mpfree_tools.content import (
    audio_format,
    discover,
    import_assets,
    main,
    validate_destination,
    validate_sources,
)


def make_wav(path: Path, frames: int = 16) -> bytes:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(44100)
        output.writeframes(b"\x00\x00" * frames)
    return path.read_bytes()


class ContentTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "factory-backup"
        self.source.mkdir()
        self.dest = self.root / "mpfree-library"

    def test_recognizes_audio_signatures_not_just_extensions(self) -> None:
        valid = self.source / "Drums" / "Kick.WAV"
        make_wav(valid)
        fake = self.source / "not-a-sample.wav"
        fake.write_text("this is not a WAV file")
        flac = self.source / "sound.flac"
        flac.write_bytes(b"fLaC" + b"\x00" * 16)
        aiff = self.source / "sound.aiff"
        aiff.write_bytes(b"FORM" + b"\x00" * 4 + b"AIFF")
        ogg = self.source / "sound.ogg"
        ogg.write_bytes(b"OggS" + b"\x00" * 16)
        self.assertEqual(audio_format(valid), "wav")
        self.assertIsNone(audio_format(fake))
        assets, errors = discover(validate_sources([self.source]))
        self.assertFalse(errors)
        self.assertEqual(
            {asset.relative_path.as_posix() for asset in assets},
            {"Drums/Kick.WAV", "sound.flac", "sound.aiff", "sound.ogg"},
        )

    def test_import_dry_run_does_not_create_destination(self) -> None:
        make_wav(self.source / "drums" / "kick.wav")
        assets, errors = discover(validate_sources([self.source]))
        self.assertFalse(errors)
        records, import_errors = import_assets(assets, self.dest, apply=False)
        self.assertFalse(import_errors)
        self.assertEqual(records[0]["status"], "planned")
        self.assertFalse(self.dest.exists())

    def test_import_copies_and_is_idempotent(self) -> None:
        payload = make_wav(self.source / "drums" / "kick.wav")
        assets, errors = discover(validate_sources([self.source]))
        self.assertFalse(errors)
        records, import_errors = import_assets(assets, self.dest, apply=True)
        self.assertFalse(import_errors)
        self.assertEqual(records[0]["status"], "copied")
        target = self.dest / "source-01" / "drums" / "kick.wav"
        self.assertEqual(target.read_bytes(), payload)
        self.assertEqual(len(records[0]["sha256"]), 64)
        records, import_errors = import_assets(assets, self.dest, apply=True)
        self.assertFalse(import_errors)
        self.assertEqual(records[0]["status"], "already_present")

    def test_import_refuses_conflicting_destination(self) -> None:
        make_wav(self.source / "kick.wav")
        target = self.dest / "source-01" / "kick.wav"
        target.parent.mkdir(parents=True)
        target.write_bytes(b"not the original")
        assets, _ = discover(validate_sources([self.source]))
        records, errors = import_assets(assets, self.dest, apply=True)
        self.assertEqual(records[0]["status"], "conflict")
        self.assertTrue(errors)
        self.assertEqual(target.read_bytes(), b"not the original")

    def test_refuses_overlapping_source_and_destination(self) -> None:
        sources = validate_sources([self.source])
        with self.assertRaises(ValueError):
            validate_destination(self.source / "mpfree", sources)
        with self.assertRaises(ValueError):
            validate_destination(self.root, sources)

    def test_skips_symlinked_audio_and_directories(self) -> None:
        real = self.source / "real.wav"
        make_wav(real)
        link = self.source / "linked.wav"
        outside = self.root / "outside"
        outside.mkdir()
        make_wav(outside / "other.wav")
        try:
            link.symlink_to(real)
            (self.source / "linked-folder").symlink_to(outside, target_is_directory=True)
        except (OSError, NotImplementedError):
            self.skipTest("Symlinks unavailable")
        assets, errors = discover(validate_sources([self.source]))
        self.assertFalse(errors)
        self.assertEqual([a.relative_path.as_posix() for a in assets], ["real.wav"])

    def test_rejects_symlinked_destination_parent(self) -> None:
        make_wav(self.source / "drums" / "kick.wav")
        self.dest.mkdir()
        other = self.root / "other"
        other.mkdir()
        try:
            (self.dest / "source-01").symlink_to(other, target_is_directory=True)
        except (OSError, NotImplementedError):
            self.skipTest("Symlinks unavailable")
        assets, _ = discover(validate_sources([self.source]))
        records, errors = import_assets(assets, self.dest, apply=True)
        self.assertEqual(records[0]["status"], "error")
        self.assertTrue(errors)
        self.assertFalse((other / "drums" / "kick.wav").exists())

    def test_cli_scan_produces_json_without_writes(self) -> None:
        make_wav(self.source / "kick.wav")
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            code = main(["scan", "--source", str(self.source), "--hash"])
        self.assertEqual(code, 0)
        report = json.loads(stdout.getvalue())
        self.assertEqual(report["schema"], "mpfree.content.v1")
        self.assertEqual(report["assets"][0]["format"], "wav")
        self.assertEqual(len(report["assets"][0]["sha256"]), 64)
        self.assertFalse(self.dest.exists())

    def test_cli_import_requires_apply_to_copy(self) -> None:
        make_wav(self.source / "kick.wav")
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            code = main([
                "import", "--source", str(self.source),
                "--destination", str(self.dest),
            ])
        self.assertEqual(code, 0)
        self.assertFalse(self.dest.exists())
        self.assertEqual(json.loads(stdout.getvalue())["assets"][0]["status"], "planned")

    def test_manifest_cannot_write_to_source(self) -> None:
        make_wav(self.source / "kick.wav")
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                main([
                    "scan", "--source", str(self.source),
                    "--manifest", str(self.source / "inventory.json"),
                ])
        self.assertEqual(raised.exception.code, 2)
        self.assertFalse((self.source / "inventory.json").exists())


if __name__ == "__main__":
    unittest.main()
