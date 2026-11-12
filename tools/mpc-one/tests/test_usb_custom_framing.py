"""Synthetic-only offline tests for FF/61/7A framing evidence synthesis."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "tools/mpc-one/synthesize_custom_usb_framing.py"
spec = importlib.util.spec_from_file_location("mpfree_ff617a_framing_test", SOURCE)
FRAMING = importlib.util.module_from_spec(spec)
spec.loader.exec_module(FRAMING)


def fake_ghidra():
    funcs = {where: {
        "pseudocode": "synthetic placeholder",
        "size": 32,
        "graph": {"callees": [], "computed": []},
    } for where in FRAMING.ROLES}
    funcs["0x0001a674"]["pseudocode"] = (
        "0x5aa5fe01 0x5aa5fd02 0x5aa5fc03 0x5aa5fb04 "
        "0x5aa5fa05 0x5aa5f906 if (value < 0x5aa5fc04) synthetic"
    )
    funcs["0x0001a8d0"]["pseudocode"] = "0xa55a0101 0xa55a0202"
    funcs["0x0001d644"]["pseudocode"] = "void FUN_0001d644() { 0x802c550a 0x4004550d; }"
    funcs["0x0001a314"]["pseudocode"] = """
int FUN_0001a314(void) {
    0x5aa5fe01; 0x5aa5fd02; 0x5aa5fc03;
    0x5aa5fb04; 0x5aa5fa05; 0x5aa5f906;
    if (local_30 < 0x18 || local_30 < *(ushort *)(param_4 + 0x16)) {}
    if ((*(int *)(param_4 + 4) != *(int *)(param_2 + 4)) ||
        (*(int *)(param_4 + 0x10) != *(int *)(param_2 + 0x10))) {}
    if ((uVar1 & 1) == 0) {}
    if ((uVar1 & 2) == 0) {}
    if (*(int *)(param_4 + 0x18) == 1) return -0xb;
    return -9;
}"""
    funcs["0x0001a674"]["pseudocode"] = (
        "void FUN_0001a674(void) {0x5aa5fe01; 0x5aa5fd02; "
        "0x5aa5fc03; 0x5aa5fb04; 0x5aa5fa05; 0x5aa5f906;"
        "if (value < 0x5aa5fc04) {} }"
    )
    funcs["0x0001a8d0"]["pseudocode"] = (
        "uStack_30 = 0xa55a0101; uStack_30 = 0xa55a0202;"
    )
    funcs["0x0001aabc"]["pseudocode"] = "void FUN_0001aabc(void) {}"
    funcs["0x0001a314"]["machine_instruction_evidence"] = [
        {"address": "0001a314", "mnemonic": "CMP", "text": "cmp r0,#0x18"},
        {"address": "0001a318", "mnemonic": "BL", "text": "bl FUN_0001d644"},
    ]
    funcs["0x0001a314"]["instruction_capture_limited"] = False
    return {
        "schema": "mpfree-v11-backend-selector-and-boot-recovery-static",
        "role": "libaz01-prog",
        "source_sha256_prechecked_by_launcher": FRAMING.SOURCE_SHA,
        "verified_raw_table_candidates": [{
            "base_elf_vaddr": FRAMING.TABLE,
            "xref_source_functions": [FRAMING.CTOR],
            "observed_ghidra_slots": [
                {"relative": 8, "resolved_function": "0001a170"},
                {"relative": 24, "resolved_function": "0001b35c"},
                {"relative": 28, "resolved_function": "0001b27c"},
                {"relative": 32, "resolved_function": "0001b150"},
                {"relative": 44, "resolved_function": None},
                {"relative": 52, "resolved_function": "0001a8d0"},
            ],
        }],
        "analyzed_functions": funcs,
    }


class FramingTests(unittest.TestCase):
    def test_usbdevfs_ioctl_decoding(self):
        submit = FRAMING.ioctl_fields(0x802c550a)
        self.assertEqual(submit["type"], "0x55")
        self.assertEqual(submit["command_number"], 10)
        self.assertEqual(submit["direction"], 2)
        self.assertEqual(submit["size_bytes"], 44)
        reap = FRAMING.ioctl_fields(0x4004550d)
        self.assertEqual(reap["command_number"], 13)
        self.assertEqual(reap["direction"], 1)
        self.assertEqual(reap["size_bytes"], 4)

    def test_v11_callback_addresses_normalized(self):
        report = FRAMING.validated_report(fake_ghidra(), "synthetic-ghidra-report")
        members = report["callback_evidence"]
        self.assertEqual(len(members), 6)
        self.assertEqual(members[0]["function"], "0x0001a170")
        self.assertTrue(members[0]["decompiled"])
        self.assertFalse(members[4]["decompiled"])
        self.assertTrue(report["constructor_table_xref_verified"])

    def test_all_signature_evidence_and_provenance(self):
        report = FRAMING.validated_report(fake_ghidra(), "source-hash")
        self.assertTrue(report["magic_evidence"]["eight_known_values_present"])
        self.assertEqual(len(report["magic_evidence"]["code_points"]), 9)
        self.assertEqual(len(report["magic_evidence"]["constructed_outgoing_control_values"]), 2)
        self.assertEqual(len(report["magic_evidence"]["equality_matched_message_identifiers"]), 6)
        self.assertTrue(all(report["response_validation_checks"].values()))
        self.assertEqual(len(report["decompiler_return_type_warnings"]), 3)
        self.assertEqual(
            set(report["magic_evidence"]["equality_matched_message_identifiers"]),
            {"0x5aa5fe01", "0x5aa5fd02", "0x5aa5fc03",
             "0x5aa5fb04", "0x5aa5fa05", "0x5aa5f906"})
        self.assertEqual(
            set(report["magic_evidence"]["constructed_outgoing_control_values"]),
            {"0xa55a0101", "0xa55a0202"})
        self.assertEqual(report["magic_evidence"]["comparison_only_range_bounds"],
                         ["0x5aa5fc04"])
        self.assertTrue(all(item["seen_in_verified_pseudocode"]
                            for item in report["linux_usbdevfs_ioctls"]))
        md = FRAMING.render_markdown(report)
        self.assertIn("UNIDENTIFIED", report["protocol_family"])
        self.assertIn("0x5aa5fe01", md)
        self.assertIn("USBDEVFS_SUBMITURB", md)
        self.assertIn("Constructed outgoing value (NOT equality-tested): 0xa55a0101", md)
        self.assertIn("completion_flag_bit0_required: True", md)
        self.assertNotIn("synthetic placeholder", md)

    def test_machine_instruction_evidence_and_callsite_summary(self):
        report = FRAMING.validated_report(fake_ghidra(), "source-hash")
        parser = next(x for x in report["focused_function_evidence"]
                      if x["address"] == "0x0001a314")
        ins = parser["instruction_evidence"]
        self.assertEqual(ins["captured"], 2)
        self.assertEqual(len(ins["call_sites"]), 1)
        self.assertEqual(ins["call_sites"][0]["address"], "0001a318")
        self.assertEqual(len(ins["sha256"]), 64)
        self.assertIn("ARM callsite evidence", FRAMING.render_markdown(report))

    def test_rejects_bad_source_or_table_or_role(self):
        base = fake_ghidra()
        base["source_sha256_prechecked_by_launcher"] = "different"
        with self.assertRaises(ValueError):
            FRAMING.validated_report(base, "sha")
        base = fake_ghidra()
        base["role"] = "spl"
        with self.assertRaises(ValueError):
            FRAMING.validated_report(base, "sha")
        base = fake_ghidra()
        base["verified_raw_table_candidates"][0]["xref_source_functions"] = []
        with self.assertRaises(ValueError):
            FRAMING.validated_report(base, "sha")


if __name__ == "__main__":
    unittest.main()
