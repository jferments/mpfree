"""Synthetic tests of the offline ARM return-contract analyzer (no firmware)."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "tools/mpc-one/analyze_arm_return_contracts.py"
spec = importlib.util.spec_from_file_location("mpfree_arm_contract_analyzer_tests", SOURCE)
LAB = importlib.util.module_from_spec(spec)
spec.loader.exec_module(LAB)


class ArmReturnContractTests(unittest.TestCase):
    def test_arm_status_values(self):
        ins = [{"address": "1d744", "text": "mov r0,#0x0"},
               {"address": "1d79c", "text": "mvn r0,#0x8"},
               {"address": "1d7bc", "text": "mvn r0,#0x5"}]
        self.assertEqual([x["value_int32"] for x in
                          LAB.detect_return_register_writes(ins)], [0, -9, -6])

    def test_direct_call_result_checked(self):
        ins = [{"address": "1b3fc", "text": "bl 0x0001a674"},
               {"address": "1b400", "text": "cmp r0,#0x0"},
               {"address": "1b404", "text": "blt 0x0001b444"}]
        found = LAB.scan_callsite("0x0001b35c", ins, "0x0001a674")
        self.assertEqual(len(found), 1)
        self.assertTrue(found[0]["conditional_branch_after_compare"])
        self.assertTrue(found[0]["immediate_r0_test"])
        self.assertEqual(found[0]["r0_test_kind"], "explicit_flags_test")
        ins[1]["text"] = "mov r1,r0"
        self.assertFalse(LAB.scan_callsite(
            "0x0001b35c", ins, "0x0001a674")[0]["immediate_r0_compare"])

    def test_subs_zero_is_a_direct_status_check(self):
        # Four actual verified callsites use this ARM idiom, not CMP.
        examples = [
            ("1a270", "subs r3,r0,#0x0", "beq 0x0001a1f4", "r3"),
            ("1a3bc", "subs r5,r0,#0x0", "bne 0x0001a4f8", "r5"),
            ("1b054", "subs r4,r0,#0x0", "beq 0x0001b0a0", "r4"),
            ("1b108", "subs r4,r0,#0x0", "bne 0x0001b060", "r4"),
        ]
        for address, operation, branch, destination in examples:
            with self.subTest(callsite=address):
                instructions = [
                    {"address": address, "text": "bl 0x0001d644"},
                    {"address": "1a3c0", "text": operation},
                    {"address": "1a3c4", "text": branch},
                ]
                result = LAB.scan_callsite("0x0001a314", instructions,
                                           "0x0001d644")[0]
                self.assertTrue(result["immediate_r0_test"])
                self.assertFalse(result["immediate_r0_compare"])
                self.assertEqual(result["r0_test_kind"], "subs_zero_flags_test")
                self.assertEqual(result["sub_zero_destination"], destination)
                self.assertTrue(result["conditional_branch_after_compare"])

    def test_copy_of_return_is_capture_not_a_status_test(self):
        instructions = [
            {"address": "1b0c4", "text": "bl 0x0001a314"},
            {"address": "1b0c8", "text": "vmov.f32 s0,0x3f800000"},
            {"address": "1b0cc", "text": "cpy r4,r0"},
        ]
        result = LAB.scan_callsite("0x0001afd4", instructions,
                                   "0x0001a314")[0]
        self.assertFalse(result["immediate_r0_test"])
        self.assertEqual(len(result["r0_copies"]), 1)
        self.assertEqual(result["r0_copies"][0]["destination_register"], "r4")
        self.assertEqual(result["r0_copies"][0]["instructions_after_call"], 2)

    def test_capture_after_call_does_not_imply_success(self):
        instructions = [
            {"address": "1b134", "text": "bl 0x0001a314"},
            {"address": "1b138", "text": "mov r4,r0"},
            {"address": "1b13c", "text": "b 0x0001b060"},
        ]
        result = LAB.scan_callsite("0x0001afd4", instructions,
                                   "0x0001a314")[0]
        self.assertFalse(result["conditional_branch_after_compare"])
        self.assertTrue(result["r0_copies"])
        self.assertIn("downstream use not yet traced", result["observation"])

    def test_archive_index_rejects_unindexed_members(self):
        with tempfile.TemporaryDirectory(prefix="mpfree-arm-contract-test-") as td:
            p = Path(td) / "fixture.zip"
            blob = b"test"
            index = {"schema": "mpfree-bootchain-integrated-zip-index-v1", "files": [
                {"name": "one.txt", "bytes": len(blob),
                 "sha256": hashlib.sha256(blob).hexdigest()}]}
            with zipfile.ZipFile(p, "w") as z:
                z.writestr("one.txt", blob)
                z.writestr(LAB.INDEX, json.dumps(index))
            with zipfile.ZipFile(p) as z:
                self.assertEqual(LAB.validate_index(z)["indexed_members_verified"], 1)
            with zipfile.ZipFile(p, "a") as z:
                z.writestr("unindexed.txt", "unexpected")
            with zipfile.ZipFile(p) as z:
                with self.assertRaises(ValueError):
                    LAB.validate_index(z)

    def test_requires_verified_library_role(self):
        doc = {"schema": "mpfree-v11-backend-selector-and-boot-recovery-static",
               "role": "az01-update",
               "source_sha256_prechecked_by_launcher": LAB.EXPECTED_LIBRARY_SHA}
        with self.assertRaises(ValueError):
            LAB.summarize(doc, {"indexed_members_verified": 1})


if __name__ == "__main__":
    unittest.main()
