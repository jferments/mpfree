"""Synthetic fail-closed Fastboot receive/bootm path verification tests."""
import importlib.util
import json
from pathlib import Path
import unittest

SOURCE=Path(__file__).resolve().parents[1]/"synthesize_fastboot_ram_path.py"
SPEC=importlib.util.spec_from_file_location("mpfree_fastboot_ram_path", SOURCE)
M=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)

def fixture():
    fn={
        "0x00122a10":(
            "uVar6 = *DAT_00122ad4 - *DAT_00122ac4;"
            "if ((uint)param_2[9] <= uVar6) uVar6=param_2[9];"
            "mpf_memcpy_candidate(&DAT_00800800 + *DAT_00122ac4,*param_2,uVar6);"
            "*puVar1 = uVar6; if (*puVar1 < *puVar2) {}"
            "else {*puVar2 = 0;}"),
        "0x00122980":(
            "mpf_fastboot_permission();"
            "if (*puVar1 == 0) return;"
            "if (*puVar1 < 0x70000001) {}"),
        "0x00122964":"DAT_00122978; mpf_fastboot_response(uVar1);",
        "0x001226b4":(
            "mpf_bootm_command(0,0,2,&local_14);"
            "mpf_reset_cpu_candidate(0,0,0,0);"),
        "0x0010e390":(
            "mpf_fdt_header_checker();"
            "if (legacy) uVar2 = 1; else uVar2 = 2;"),
        "0x0010a6a4":(
            "mpf_legacy_header_crc(piVar5);"
            "mpf_legacy_payload_crc(piVar5);"
            "FUN_0010f8a4(iVar7,piVar5);"),
    }
    return {
        "role":"uboot",
        "source_sha256_prechecked_by_launcher":M.EXPECTED_SHA,
        "analyzed_functions":{
            addr:{"pseudocode":body} for addr,body in fn.items()},
    }

def build(data):
    return M.build(lambda filename:json.dumps(data).encode())

class RamPathTests(unittest.TestCase):
    def test_original_contract_synthetic_success(self):
        report=build(fixture())
        self.assertTrue(report["all_static_evidence_passed"])
        self.assertEqual(report["ram_receive_base"],0x00800800)
        self.assertEqual(report["max_advertised_download_bytes"],0x70000000)
        self.assertEqual(report["mathematical_unchecked_end_address"],0x70800800)
        self.assertFalse(report["safe_nonpersistent_kernel_initramfs_FDT_layout_proven"])
        self.assertFalse(report["FIT_signature_required_proven"])

    def test_wrong_uboot_sha_is_fatal(self):
        d=fixture()
        d["source_sha256_prechecked_by_launcher"]="f"*64
        with self.assertRaisesRegex(ValueError,"authenticated"):
            build(d)

    def test_missing_permission_gate_fails_closed(self):
        d=fixture()
        d["analyzed_functions"]["0x00122980"]["pseudocode"]="download"
        report=build(d)
        self.assertFalse(report["checks"]["download_permission_gate"])
        self.assertFalse(report["all_static_evidence_passed"])

    def test_wrong_receive_base_fails_closed(self):
        d=fixture()
        addr="0x00122a10"
        d["analyzed_functions"][addr]["pseudocode"]=(
            d["analyzed_functions"][addr]["pseudocode"].replace(
                "DAT_00800800","DAT_10800800"))
        self.assertFalse(build(d)["checks"][
            "receive_copies_to_fixed_ram_base"])

    def test_missing_legacy_crc_does_not_prove_bootm_integrity(self):
        d=fixture()
        d["analyzed_functions"]["0x0010a6a4"]["pseudocode"]="no CRC"
        self.assertFalse(build(d)["checks"][
            "bootm_legacy_header_and_data_crc_paths"])

    def test_boot_fdt_classifier_does_not_claim_required_signature(self):
        report=build(fixture())
        self.assertTrue(report["checks"][
            "image_classifier_distinguishes_legacy_and_FDT"])
        self.assertFalse(report["FIT_signature_required_proven"])
        self.assertFalse(report["raw_zImage_via_fastboot_boot_proven"])

if __name__=="__main__":
    unittest.main()
