"""Static safety/syntax guards for the report-targeted Ghidra/QEMU extension."""
import ast
from pathlib import Path
import unittest

ROOT=Path(__file__).resolve().parents[3]
GHIDRA=ROOT/"tools/ghidra/MPFreeBackendSelectorV11.py"
RUNNER=ROOT/"tools/mpc-one/run_bootchain_integrated_lab.py"
FOCUS=ROOT/"tools/mpc-one/synthesize_direct_boot_focus.py"

class DirectBootSourceGuards(unittest.TestCase):
    def test_ghidra_syntax_and_existing_sha_guard(self):
        s=GHIDRA.read_text()
        ast.parse(s,filename=str(GHIDRA))
        self.assertIn("Saved project differs from verified owner boot stage",s)
        self.assertIn("direct_boot_probe",s)
        self.assertIn("permission_read_write_pointer_equal",s)
        self.assertIn("matches_published_37_byte_unlock_token",s)
        self.assertIn("pmu_boot_counter_register",s)
        self.assertIn("rom_download_magic",s)
        self.assertIn("fastboot_command_table_expected_eight_entries",s)
        self.assertIn("'0x0010fde8'",s)

    def test_other_stage_pointer_probes_are_not_cleared(self):
        source = GHIDRA.read_text()
        stage = source.index("if role in ('spl','az01-update')")
        uboot = source.index("if role=='uboot':", stage)
        self.assertNotIn("direct_boot={}", source[stage:uboot])
        self.assertIn("original_image_pointers", source[stage:uboot])
        self.assertIn("spl-updater-pointer-evidence.json", RUNNER.read_text())

    def test_adc_source_present_and_pointer_proof_is_required(self):
        source = GHIDRA.read_text()
        runner = RUNNER.read_text()
        self.assertIn("'board_setup_recovery_flag_pointer'", source)
        self.assertIn("'recovery_adc_not_found_string'", source)
        self.assertIn("recovery_adc_to_environment_pointer_equal", source)
        self.assertIn("recovery-adc-evidence.json", runner)
        self.assertIn("all_pointer_checks_passed", runner)

    def test_source_does_not_execute_an_oem_fastboot_command(self):
        s=GHIDRA.read_text()
        self.assertNotIn("fastboot oem ",s)
        self.assertNotIn("fastboot flash ",s)
        self.assertNotIn("subprocess.",s)
        self.assertNotIn("os.system(",s)
        self.assertNotIn("mmcblk",s)

    def test_integrated_runner_is_qa_gate_for_directboot(self):
        s=RUNNER.read_text()
        ast.parse(s,filename=str(RUNNER))
        for x in ("direct-boot-focus.json","direct-boot-focus.md",
                  "literal_pointer_and_token_proven",
                  "published_oem_token_binary_sha256_matches",
                  "MPFREE_VM_SECTION_direct_boot_rootfs",
                  "MPFREE_VM_SECTION_kernel_modules",
                  "MPFREE_VM_SECTION_310_trustpoint_unit",
                  "MPFREE_VM_SECTION_boot_script_hex",
                  "od -v -b /boot/boot.scr.uimg",
                  "verify_uboot_script_wrapper.py",
                  "security-310-utilities.json",
                  "partition_guard_and_sparse_path_proven"):
            self.assertIn(x,s)

    def test_script_decoder_is_strict_busybox_octal_not_gnu_od(self):
        decoder = ROOT / "tools/mpc-one/verify_uboot_script_wrapper.py"
        source = decoder.read_text()
        ast.parse(source, filename=str(decoder))
        self.assertIn("decode_od_bytes", source)
        self.assertIn("BUSYBOX_OCTAL_LINE", source)
        self.assertIn("Non-contiguous BusyBox od byte offsets", source)
        self.assertIn("terminal_offset", source)
        self.assertNotIn("od -An -v -tx1", RUNNER.read_text())

    def test_new_offline_recheck_and_hash_region_contract(self):
        preflight = ROOT / "tools/mpc-one/recheck_integrated_directboot.py"
        script_parser = ROOT / "tools/mpc-one/verify_uboot_script_wrapper.py"
        preflight_src = preflight.read_text()
        parser_src = script_parser.read_text()
        ast.parse(preflight_src, filename=str(preflight))
        ast.parse(parser_src, filename=str(script_parser))
        for value in (
            "wide.verify_zip(z)",
            "script.compare_guest_to_update(",
            "direct.build(z.read)",
            "physical_device_access",
            "vendor_binary_executed",
        ):
            self.assertIn(value, preflight_src)
        for value in (
            "source_official_sha256_field_means",
            "raw_script_data_not_uimage",
            "header_sha256_matches_official",
            "canonical_header_and_script_bytes_match_official",
            'firmware310.get("image_sha256")',
        ):
            self.assertIn(value, parser_src)
        self.assertNotIn("subprocess.", preflight_src)
        self.assertNotIn("os.system(", preflight_src)

    def test_focus_syntax_and_no_physical_fastboot(self):
        s=FOCUS.read_text()
        ast.parse(s,filename=str(FOCUS))
        self.assertIn("physical_fastboot_commands_sent",s)
        self.assertIn("vendor_code_executed",s)
        self.assertNotIn("subprocess.",s)
        self.assertNotIn("os.system(",s)
        self.assertNotIn("fastboot flash ",s)

if __name__=="__main__":
    unittest.main()
