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
                 "0x00122cf8":{"pseudocode":"mpf_fastboot_response()"},
                 "0x00122d38":{"pseudocode":"mpf_fastboot_response()"},
                 "0x00122948":{"pseudocode":"mpf_fastboot_response()"},
                 "0x00100fe4":{"pseudocode":(
                     "if (uVar4 == 1) { *DAT_00101154 = 1; } "
                     "if (uVar4 != 9) {}; *puVar1 = 0;")},
                 "0x001100c8":{"pseudocode":(
                     "FUN_00110084; FUN_0010fde8; if (local_8c < uVar4) {}; "
                     "iVar1 + 0x68")},
                 "0x0010fde8":{"pseudocode":(
                     "case 0xcac1; case 0xcac2; case 0xcac3; case 0xcac4; "
                     "(uint)(param_1[1] + param_1[2]) < iVar11 + uVar2")},
                 "0x00110270":{"pseudocode":(
                     "FUN_00110084; if (local_8c < uVar5) {}; iVar2 + 0x6c")},
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
        "spl":{},"update":{},
        "usb":{"verified_backend_interfaces":[
            {"usb_descriptor":{"class":"0xff","subclass":"0x42","protocol":"0x03"},
             "initializer_references_table":True,
             "marker_interpretation":"Fastboot OKAY and FAIL response words"},
            {"usb_descriptor":{"class":"0xff","subclass":"0x61","protocol":"0x7a"},
             "initializer_references_table":True,
             "upstream_protocol_family":"Unknown vendor-specific"},
            {"usb_descriptor":{"class":"0xff","subclass":"0x06","protocol":"0x05"},
             "initializer_references_table":True}]},
        "framing":{"protocol_family":"UNIDENTIFIED; framed proprietary/vendor-specific candidate"}
    }

def run(data):
    lookup={path:key for key,path in M.SOURCE.items()}
    return M.build(lambda path:json.dumps(data[lookup[path]]).encode())

class DirectBootFocusTests(unittest.TestCase):
    def test_seven_topics_and_exact_sha(self):
        report=run(fixture())
        self.assertEqual(len(report["questions"]),40)
        self.assertEqual(
            report["nominal_memory_address_gaps"]["fdt_addr_r"][
                "from_fastboot_load_base_bytes"], 24115200)
        self.assertEqual(
            report["nominal_memory_address_gaps"]["kernel_addr_r"][
                "from_fastboot_load_base_bytes"], 25163776)
        self.assertFalse(report["fastboot_download_memory_safe_proven"])
        self.assertEqual(len({x["topic"] for x in report["questions"]}),9)
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

    def test_fastboot_dispatch_table_proves_boot_and_download_handlers(self):
        d = fixture()
        d["u"]["direct_boot_probe"] = {
            "fastboot_command_table": [
                {"prefix_text": "boot",
                 "callback_containing_function": "0x00122964"},
                {"prefix_text": "download:",
                 "callback_containing_function": "0x00122980"},
            ] + [
                {"prefix_text": "other"+str(i),
                 "callback_containing_function": None}
                for i in range(6)
            ]
        }
        report = run(d)
        self.assertTrue(report["checks"]["fastboot_dispatch_table_eight_entries"])
        self.assertTrue(report["checks"]["fastboot_boot_command_callback_resolved"])
        self.assertTrue(report["checks"]["fastboot_download_command_callback_resolved"])
        dispatch_question = next(x for x in report["questions"]
                                 if "map boot and download" in x["question"])
        self.assertEqual(dispatch_question["result"], "verified-static")
        d["u"]["direct_boot_probe"]["fastboot_command_table"][0][
            "callback_containing_function"] = "0x001226b4"
        changed = run(d)
        self.assertFalse(changed["checks"]["fastboot_boot_command_callback_resolved"])

    def test_raw_sparse_erase_guards_detected_in_verified_shapes(self):
        report = run(fixture())
        for name in (
            "flash_partition_capacity_guard",
            "flash_sparse_four_chunk_kinds",
            "sparse_raw_fill_partition_bound",
            "erase_partition_block_range_guard",
        ):
            self.assertTrue(report["checks"][name],name)
        data = fixture()
        data["u"]["analyzed_functions"]["0x0010fde8"]["pseudocode"] = ""
        changed = run(data)
        self.assertFalse(changed["checks"]["flash_sparse_four_chunk_kinds"])
        self.assertFalse(changed["checks"]["sparse_raw_fill_partition_bound"])

    def test_all_eight_exact_thumb_command_handlers(self):
        d = fixture()
        expected = {
            "reboot":"0x00122cf8", "getvar:":"0x00122d38",
            "download:":"0x00122980", "boot":"0x00122964",
            "continue":"0x00122948", "flash":"0x00122c5c",
            "erase":"0x00122bf8", "oem":"0x00122854",
        }
        d["u"]["direct_boot_probe"] = {
            "fastboot_command_table": [
                {"prefix_text": key, "callback_containing_function": value,
                 "callback_is_thumb": True}
                for key,value in expected.items()
            ]
        }
        report = run(d)
        self.assertTrue(report["checks"]["all_eight_fastboot_prefixes_exact_and_unique"])
        for key in ("getvar_has_no_local_permission_call",
                    "continue_has_no_local_permission_call",
                    "reboot_has_no_local_permission_call",
                    "reboot_mode_clear_and_one_nine_branch"):
            self.assertTrue(report["checks"][key], key)
        d["u"]["direct_boot_probe"]["fastboot_command_table"][0][
            "callback_is_thumb"] = False
        changed = run(d)
        self.assertFalse(changed["checks"]["all_eight_fastboot_prefixes_exact_and_unique"])

    def test_host_usb_backends_distinct_from_verified_uboot_fastboot(self):
        d = fixture()
        result = run(d)
        self.assertTrue(result["checks"]["all_three_host_usb_selector_families_verified"])
        self.assertTrue(result["checks"]["fastboot_host_backend_has_okay_fail_literals"])
        self.assertTrue(result["checks"]["ff617a_is_not_proven_rockusb"])
        self.assertIn("0xff/0x61/0x7a", result["host_usb_descriptor_classes"])
        d["framing"]["protocol_family"] = "RockUSB proven"
        self.assertFalse(run(d)["checks"]["ff617a_is_not_proven_rockusb"])

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
