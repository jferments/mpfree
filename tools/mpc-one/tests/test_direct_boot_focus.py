"""Pure synthetic fixtures for the MPFree direct-boot evidence analyzer."""
import importlib.util
import json
from pathlib import Path
import unittest

SOURCE=Path(__file__).resolve().parents[1]/"synthesize_direct_boot_focus.py"
spec=importlib.util.spec_from_file_location("mpfree_direct_boot_focus",SOURCE)
M=importlib.util.module_from_spec(spec)
spec.loader.exec_module(M)

def fixture():
    dtb=("mmc0 = \"/dwmmc@ff0f0000\";",
         "mmc1 = \"/dwmmc@ff0c0000\";",
         "dwmmc@ff0c0000 {", 'status = "disabled";', "};")
    return {
        "u":{"role":"uboot",
             "source_sha256_prechecked_by_launcher":M.UBOOT_SHA,
             "analyzed_functions":{
                 "0x00122854":{"pseudocode":(
                     "FUN_0012dc78(DAT_00122924,iVar3,0x25);"
                     "*DAT_00122928 = 1;")},
                 "0x00122834":{"pseudocode":"*DAT_0012284c"},
                 "0x00122980":{"pseudocode":"mpf_fastboot_permission()"},
                 "0x00122c5c":{"pseudocode":"mpf_fastboot_permission()"},
                 "0x00122bf8":{"pseudocode":"mpf_fastboot_permission()"},
                 "0x001226b4":{"pseudocode":"mpf_bootm_command();mpf_reset_cpu_candidate();"},
             }},
        "p3":{"globals":{"0x0014d690":{"references":[
            {"from":"0012289e","type":"WRITE"},
            {"from":"00122838","type":"READ"}]}}},
        "boot":{"boot_script":{"firmware310":{
            "commands":"load fake zImage; bootz 0x02000000"},
            "normalized_commands_equal":True},
            "elfs":{"/usr/bin/az01-bootloader-sig":{"firmware310":{"present":True}}},
            "services":{"/usr/lib/systemd/system/az0x-loadpin-trustpoints.service":{
                "firmware310":{"present":True}}}},
        "env":{"p2":{"boot_related_variables":{
            "boot_targets":"mmc0 mmc1",
            "loadaddr":"0x00800800",
            "fdt_addr_r":"0x01f00000",
            "kernel_addr_r":"0x02000000",
            "ramdisk_addr_r":"0x04000000"}}},
        "fdt":{"uboot_stage_fdt":[{"selected_dts_lines":dtb}]},
        "spl":{},"update":{}
    }

def run(data):
    lookup={path:key for key,path in M.SOURCE.items()}
    return M.build(lambda path:json.dumps(data[lookup[path]]).encode())

class DirectBootFocusTests(unittest.TestCase):
    def test_seven_topics_and_exact_sha(self):
        report=run(fixture())
        self.assertEqual(len(report["questions"]),26)
        self.assertEqual(
            report["nominal_memory_address_gaps"]["fdt_addr_r"][
                "from_fastboot_load_base_bytes"], 24115200)
        self.assertEqual(
            report["nominal_memory_address_gaps"]["kernel_addr_r"][
                "from_fastboot_load_base_bytes"], 25163776)
        self.assertFalse(report["fastboot_download_memory_safe_proven"])
        self.assertEqual(len({x["topic"] for x in report["questions"]}),7)
        self.assertEqual(report["private_source_sha256"],M.UBOOT_SHA)
        self.assertFalse(report["vendor_code_executed"])
        self.assertFalse(report["physical_fastboot_commands_sent"])

    def test_oem_grant_and_shared_permission_global(self):
        report=run(fixture())
        for key in ("token_prefix_comparison_37_bytes",
                    "oem_grants_permission_pointer",
                    "global_0x14d690_written_at_0x12289e",
                    "global_0x14d690_read_at_0x122838",
                    "download_gate_present","flash_gate_present","erase_gate_present"):
            self.assertTrue(report["checks"][key],key)
        report["checks"]["runtime_permission_initialization_proven"]=False
        self.assertNotIn("runtime_permission_initialization_proven",
                         M.build(lambda p:json.dumps(fixture()[{
                             v:k for k,v in M.SOURCE.items()}[p]]).encode())["checks"])

    def test_missing_grant_or_wrong_write_xref_fails_closed(self):
        d=fixture()
        d["u"]["analyzed_functions"]["0x00122854"]["pseudocode"]="not present"
        d["p3"]["globals"]["0x0014d690"]["references"]=[]
        report=run(d)
        self.assertFalse(report["checks"]["oem_grants_permission_pointer"])
        self.assertFalse(report["checks"]["global_0x14d690_written_at_0x12289e"])
        row=report["questions"][0]
        self.assertEqual(row["result"],"needs-evidence")

    def test_bootm_is_distinct_from_bootz(self):
        report=run(fixture())
        self.assertTrue(report["checks"]["boot_completion_calls_bootm"])
        self.assertTrue(report["checks"]["normal_script_bootz"])
        self.assertFalse(report["checks"]["boot_completion_has_local_permission_check"])
        self.assertIn("## RAM boot",M.render_md(report))

    def test_wrong_original_uboot_rejected(self):
        d=fixture()
        d["u"]["source_sha256_prechecked_by_launcher"]="different"
        with self.assertRaises(ValueError):
            run(d)

    def test_mmc1_disabled_negative(self):
        d=fixture()
        self.assertTrue(run(d)["checks"]["mmc1_disabled_in_embedded_fdt"])
        d["fdt"]["uboot_stage_fdt"][0]["selected_dts_lines"]=(
            "mmc1 = \"/dwmmc@ff0c0000\";",
            "dwmmc@ff0c0000 {",'status = "okay";',"};")
        self.assertFalse(run(d)["checks"]["mmc1_disabled_in_embedded_fdt"])

if __name__=="__main__":
    unittest.main()
