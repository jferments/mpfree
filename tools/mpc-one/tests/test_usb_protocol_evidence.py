"""Offline USB backend evidence-map tests using only synthetic Ghidra fixtures."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
PATH = ROOT / "tools/mpc-one/synthesize_usb_backend_protocols.py"
spec = importlib.util.spec_from_file_location("mpfree_usb_protocol_map_test", PATH)
MAP = importlib.util.module_from_spec(spec)
spec.loader.exec_module(MAP)


def fixture():
    tables = []
    functions = {}
    for proto in MAP.INTERFACES:
        ctor = proto["ctor"]
        functions[ctor] = {"name": "test_ctor", "pseudocode": "synthetic",
                           "graph": {"callees": []}, "size": 16}
        for address, code in proto["marker_functions"].items():
            functions[address] = {"name": "test_marker",
                                  "pseudocode": "TEST " + code,
                                  "graph": {"callees": []}, "size": 12}
        tables.append({
            "base_elf_vaddr": proto["table"],
            "xref_source_functions": [ctor],
            "observed_ghidra_slots": [
                {"relative": 0x18, "resolved_function": ctor},
            ],
        })
    return {
        "schema": "mpfree-v11-backend-selector-and-boot-recovery-static",
        "role": "libaz01-prog",
        "source_sha256_prechecked_by_launcher": MAP.SOURCE_SHA,
        "ghidra_image_base": "0x00010000",
        "verified_raw_table_candidates": tables,
        "analyzed_functions": functions,
    }


class UsbProtocolEvidenceTests(unittest.TestCase):
    def test_three_interfaces_and_callbacks(self):
        report = MAP.summarize(fixture(), "fake-input-hash")
        rows = report["verified_backend_interfaces"]
        self.assertEqual(len(rows), 3)
        self.assertEqual([x["upstream_protocol_family"] for x in rows],
                         ["Android Fastboot-compatible",
                          "Unknown vendor-specific",
                          "Rockchip RockUSB-compatible"])
        self.assertTrue(all(x["initializer_references_table"] for x in rows))
        self.assertTrue(all(x["backend_callback_evidence"][0]
                            ["decompilation_present"] for x in rows))
        self.assertTrue(all(marker["seen_in_decompilation"] for row in rows
                            for marker in row["marker_evidence"]))
        self.assertIn("not", MAP.render_markdown(report).lower())

    def test_rejects_wrong_role_and_source_sha(self):
        f = fixture()
        f["role"] = "az01-update"
        with self.assertRaises(ValueError):
            MAP.summarize(f, "fake-input-hash")
        f = fixture()
        f["source_sha256_prechecked_by_launcher"] = "bad"
        with self.assertRaises(ValueError):
            MAP.summarize(f, "fake-input-hash")

    def test_missing_linked_table_is_rejected(self):
        f = fixture()
        f["verified_raw_table_candidates"].pop()
        with self.assertRaises(ValueError):
            MAP.summarize(f, "fake-input-hash")


if __name__ == "__main__":
    unittest.main()
