#!/usr/bin/env python3
"""Host-only regression tests for V12 packaging, safety and question coverage."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "tools/mpc-one/run_fastboot_directboot_v12.py"
spec = importlib.util.spec_from_file_location("mpfree_v12_tested", SCRIPT)
v12 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v12)


class V12OfflineTests(unittest.TestCase):
    def test_rejects_block_and_symlink_inputs(self):
        with tempfile.TemporaryDirectory() as name:
            d = Path(name)
            source = d / "regular"
            source.write_bytes(b"test")
            link = d / "link"
            link.symlink_to(source)
            self.assertEqual(v12.regular(source), source)
            with self.assertRaises(ValueError):
                v12.regular(link)
            with self.assertRaises(ValueError):
                v12.regular(Path("/dev/zero"))

    def test_rootfs_missing_still_gives_a_private_report(self):
        with tempfile.TemporaryDirectory() as name:
            d = Path(name)
            outcome = v12.run_offline_rootfs(d, None, None)
            self.assertTrue(outcome["success"])
            report = json.loads((d / "offline-rootfs-and-boot-map-v12.json").read_text())
            self.assertIn("not found", report["observations"]["rootfs"])

    def test_rejects_non_ext4(self):
        with tempfile.TemporaryDirectory() as name:
            d = Path(name)
            wrong = d / "not-an-ext4.img"
            wrong.write_bytes(bytes(4096))
            outcome = v12.run_offline_rootfs(d, wrong, None)
            self.assertFalse(outcome["success"])
            self.assertIn("ext4", outcome["error"])

    def test_question_matrix_even_when_ghidra_unavailable(self):
        with tempfile.TemporaryDirectory() as name:
            d = Path(name)
            matrix = v12.question_matrix(d, {})
            self.assertGreaterEqual(len(matrix), 60)
            self.assertTrue(all(row["status"].startswith("OPEN") for row in matrix))
            self.assertEqual(len({row["id"] for row in matrix}), len(matrix))

    def test_question_matrix_distinguishes_lead_from_proof(self):
        with tempfile.TemporaryDirectory() as name:
            d = Path(name)
            v12.write_json(d / "ghidra-p3-fastboot-v12.json",
                           {"questions": {"fastboot": ["What changes authorization?"]},
                            "markers": {"unlock": {"present": True},
                                        "boot_targets": {"present": True}},
                            "registers": {"pmu_bootcount_new_hypothesis":
                                          {"total_raw_literal_sites": 1}}})
            rows = v12.question_matrix(d, {"qemu_synthetic": {"success": True}})
            lead = next(row for row in rows if row["id"] == "FASTBOOT-01")
            self.assertIn("STATIC/SYNTHETIC", lead["status"])
            self.assertNotIn("PROVEN", lead["status"])

    def test_packager_accepts_only_report_files_and_sha_index(self):
        with tempfile.TemporaryDirectory() as name:
            folder = Path(name) / "bootchain-wide-v12-test"
            folder.mkdir()
            (folder / "report.txt").write_text("no private binaries")
            pkg = v12.load("tools/mpc-one/package-private-boot-audit.py", "privatepack_v12_unittest")
            archive, index = pkg.package(folder)
            self.assertTrue(archive.exists())
            self.assertEqual(len(index["files"]), 1)
            import zipfile
            with zipfile.ZipFile(archive) as z:
                self.assertIsNone(z.testzip())
                self.assertEqual(z.read("report.txt"), b"no private binaries")
                self.assertEqual(json.loads(z.read("ZIP_INDEX.json"))["files"][0]["sha256"],
                                 v12.digest(folder / "report.txt"))

    def test_packager_rejects_binary_and_symlink(self):
        with tempfile.TemporaryDirectory() as name:
            folder = Path(name) / "bootchain-wide-v12-danger"
            folder.mkdir()
            (folder / "firmware.img").write_bytes(b"private")
            pkg = v12.load("tools/mpc-one/package-private-boot-audit.py", "privatepack_v12_reject")
            with self.assertRaises(ValueError):
                pkg.package(folder)
            (folder / "firmware.img").unlink()
            (folder / "out.log").symlink_to(folder / "private")
            with self.assertRaises(ValueError):
                pkg.package(folder)


if __name__ == "__main__":
    unittest.main(verbosity=2)
