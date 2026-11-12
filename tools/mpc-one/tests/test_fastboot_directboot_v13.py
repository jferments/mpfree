#!/usr/bin/env python3
"""Host-only tests for V13 evidence classification and prior ZIP integrity."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "tools/mpc-one/run_fastboot_directboot_v13.py"
spec = importlib.util.spec_from_file_location("mpfree_v13_regression", SCRIPT)
v13 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v13)

class EvidenceTests(unittest.TestCase):
    def test_pointer_alias_and_fastboot_call_evidence(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            fns = {}
            for source, target in [
                ("0x00122980", "00122834"),
                ("0x00122c5c", "00122834"),
                ("0x00122bf8", "00122834"),
                ("0x001226b4", "00101744"),
            ]:
                fns[source] = {"calls": [{"targets": [target]}]}
            v13.record(root, "ghidra-p3-v13.json", {
                "oem_permission_alias": {
                    "same_nonzero_address": True,
                    "gate_pointer": "0x0014d690",
                    "unlock_writer_pointer": "0x0014d690",
                },
                "pointer_cells": {},
                "dispatch_table": {"entries": [
                    {"index": k, "command": "synthetic", "handler_pointer": "0x00100000"}
                    for k in range(8)]},
                "function_details": fns,
                "constant_occurrences": {},
            })
            v13.record(root, "offline-rootfs-and-boot-map-v12.json", {
                "observations": {"boot_script_flags": {
                    "contains_bootz": True, "sha256": "test-script-hash"}}
            })
            rows = [{"id": "X", "question": "Does download require permission?",
                     "status": "OPEN", "evidence": []}]
            findings = v13.evaluate(root, {}, rows)
            self.assertEqual(findings["oem_unlock_permission_pointer_alias"]["status"],
                             "ANSWERED_STATIC")
            self.assertEqual(findings["eight_slot_fastboot_dispatch"]["status"],
                             "ANSWERED_STATIC")
            self.assertEqual(findings["fastboot_download_permission_check"]["status"],
                             "ANSWERED_STATIC")
            self.assertEqual(findings["fastboot_flash_permission_check"]["status"],
                             "ANSWERED_STATIC")
            self.assertEqual(findings["fastboot_erase_permission_check"]["status"],
                             "ANSWERED_STATIC")
            self.assertEqual(findings["fastboot_bootm_direct_call"]["status"],
                             "ANSWERED_STATIC")
            self.assertEqual(rows[0]["status"], "ANSWERED_STATIC")
            self.assertEqual(findings["pmu_bootcount_at_0xff7300a0"]["status"],
                             "UNRESOLVED")

    def test_no_proof_when_static_inputs_missing(self):
        with tempfile.TemporaryDirectory() as folder:
            findings = v13.evaluate(Path(folder), {}, [])
            self.assertEqual(findings["oem_unlock_permission_pointer_alias"]["status"],
                             "UNRESOLVED")
            self.assertEqual(findings["eight_slot_fastboot_dispatch"]["status"],
                             "UNRESOLVED")
            self.assertEqual(findings["fastboot_download_permission_check"]["status"],
                             "UNRESOLVED")
            self.assertEqual(findings["maskrom_independent_restore"]["status"],
                             "NEEDS_HARDWARE")

    def test_verified_v12_question_lineage(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            directory = root / "bootchain-wide-v12-testing"
            directory.mkdir()
            v13.record(directory, "V12-RESEARCH-QUESTION-MATRIX.json", {
                "questions": [{"id": "Q-1", "question": "Is boot possible?",
                               "next_probe": "inspect"}]})
            privatepack = v13.load("tools/mpc-one/package-private-boot-audit.py",
                                   "mpfree_v13_private_test")
            archive, _ = privatepack.package(directory)
            dest = root / "result"
            dest.mkdir()
            items = v13.load_previous_questions(dest, archive)
            self.assertEqual(len(items), 1)
            self.assertEqual(items[0]["status"], "OPEN")
            self.assertTrue(json.loads(
                (dest / "V13-PRIOR-QUESTION-LINEAGE.json").read_text())["present"])

    def test_invalid_prior_zip_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = root / "invalid.zip"
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("ZIP_INDEX.json", json.dumps({"files": [
                    {"name": "report.json", "bytes": 7,
                     "sha256": "0" * 64}]}))
                z.writestr("report.json", "invalid")
            with self.assertRaises(ValueError):
                v13.load_previous_questions(root, archive)

if __name__ == "__main__":
    unittest.main(verbosity=2)
