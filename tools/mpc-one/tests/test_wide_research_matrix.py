"""Synthetic tests of the seven-track MPFree evidence matrix, no firmware."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

P = Path(__file__).resolve().parents[1] / "synthesize_wide_research_matrix.py"
spec = importlib.util.spec_from_file_location("mpfree_wide_question_matrix", P)
M = importlib.util.module_from_spec(spec)
spec.loader.exec_module(M)

def fixture():
    mode = {
        "///power-management@ff730000/reboot-mode": {
            "after":["mode-loader = <0x5242c301>;",
                     "mode-update = <0x5242c301>;"],
            "before":["mode-loader = <0x5242c301>;"],
        },
    }
    data = {
        "boot": {
            "p1":{"lba":64,"bytes":65536},
            "p2":{"crc_valid_for_standard_layout":True,
                  "boot_related_variables":{"boot_targets":"mmc0 mmc1 ",
                                             "bootdelay":"0",
                                             "az01_recovercmd":"fastboot 0"}},
            "p3":{"header_crc32_valid":True,"payload_crc32_valid":True},
            "compiled_uboot_environment":{"saved_comparison":{
                "bootcmd":{"same_as_saved":True},
                "bootdelay":{"same_as_saved":True},
            }},
        },
        "diff": {
            "boot_script":{"normalized_commands_equal":True},
            "board_dtbs":{name:{
                "boot_node_diff":{"changed_boot_nodes":mode}}
                for name in ("/boot/rk3288-az01-acva.dtb",
                             "/boot/rk3288-az01-acva-c.dtb")},
            "elfs":{"/usr/bin/az01-bootloader-sig":{
                "factory":{"present":False},
                "firmware310":{"present":True}}},
            "services":{"/usr/lib/systemd/system/az0x-loadpin-trustpoints.service":{
                "firmware310":{"present":True}}},
            "ramfs_updater":{"present":True},
        },
        "graph":{"bootloader_stages":{
            "p1":{"paths":{
                "initial_sram_to_boot_loop":{"direct_call_path_found":False},
                "boot_loop_to_media_load":{"direct_call_path_found":True},
                "boot_loop_to_indirect_handoff":{"direct_call_path_found":True}}},
            "p3":{"paths":{
                "fastboot_ram_boot_to_bootm":{"direct_call_path_found":True},
                "image_classifier_to_fit_verify":{"direct_call_path_found":True}}}}},
        "p1":{}, "p3":{}, "ledger":{},
        "fdt":{"uboot_stage_fdt":[{
            "selected_dts_lines":[
                'mmc0 = "/dwmmc@ff0f0000";',
                'mmc1 = "/dwmmc@ff0c0000";',
                'dwmmc@ff0c0000 {',
                'status = "disabled";',
                '};',
                'dwmmc@ff0d0000 {',
                'status = "disabled";',
                '};',
                'dwmmc@ff0e0000 {',
                'status = "disabled";',
                '};',
                'dwmmc@ff0f0000 {',
                'status = "okay";',
                'bus-width = <0x08>;',
                'non-removable;',
                '};',
            ],
        }]},
        "spl":{"analyzed_functions":{"0xff705350":{
            "pseudocode":"synthetic","machine_instruction_evidence":[{"text":"b"}]}}},
        "updater":{"analyzed_functions":{}},
        "library":{"analyzed_functions":{}},
        "uboot":{"analyzed_functions":{
            "0x00122c5c":{"pseudocode":"mpf_fastboot_permission()"},
            "0x00122bf8":{"pseudocode":"mpf_fastboot_permission()"}}},
        "usb":{"verified_backend_interfaces":[{}, {}, {}]},
        "framing":{},
    }
    return data

def build(data):
    paths = {v:k for k,v in M.SOURCE.items()}
    return M.build(lambda name: json.dumps(data[paths[name]]).encode())

class BroadMatrixTests(unittest.TestCase):
    def test_all_seven_tracks_and_at_least_fifty_one_questions(self):
        report = build(fixture())
        self.assertGreaterEqual(len(report["questions"]), 57)
        self.assertEqual(len({x["track"] for x in report["questions"]}), 7)
        self.assertEqual(report["schema"], M.SCHEMA)
        self.assertFalse(report["vendor_code_executed"])
        self.assertEqual(len(report["focused_static_functions"]), 18)
        spl = next(x for x in report["focused_static_functions"]
                   if x["address"] == "0xff705350")
        self.assertEqual(spl["captured_machine_instructions"], 1)
        self.assertTrue(spl["decompiled"])
        self.assertIn("Directed Ghidra evidence inventory", M.markdown(report))
        self.assertIn("Recovery", M.markdown(report))

    def test_update_alias_and_permission_evidence_fails_closed(self):
        data = fixture()
        base = build(data)
        alias = next(x for x in base["questions"]
                     if "mode-update" in x["question"])
        permission = next(x for x in base["questions"]
                          if "write-permission gated" in x["question"])
        self.assertEqual(alias["status"], "static-supported")
        self.assertEqual(permission["status"], "static-supported")
        data["diff"]["board_dtbs"]["/boot/rk3288-az01-acva.dtb"][
            "boot_node_diff"]["changed_boot_nodes"][
                "///power-management@ff730000/reboot-mode"]["after"] = [
                    "mode-update = <0xef08a53c>;"
                ]
        data["uboot"]["analyzed_functions"]["0x00122c5c"]["pseudocode"] = ""
        changed = build(data)
        alias = next(x for x in changed["questions"]
                     if "mode-update" in x["question"])
        permission = next(x for x in changed["questions"]
                          if "write-permission gated" in x["question"])
        self.assertEqual(alias["status"], "unresolved")
        self.assertEqual(permission["status"], "unresolved")

    def test_embedded_fdt_exposes_disabled_sdmmc(self):
        data = fixture()
        report = build(data)
        row = next(x for x in report["questions"]
                   if "enable its SDMMC boot target" in x["question"])
        self.assertEqual(row["status"], "static-supported")
        data["fdt"]["uboot_stage_fdt"][0]["selected_dts_lines"][
            3] = 'status = "okay";'
        changed = build(data)
        row = next(x for x in changed["questions"]
                   if "enable its SDMMC boot target" in x["question"])
        self.assertEqual(row["status"], "unresolved")
        # Revert first controller and enable an unused non-eMMC controller.
        data = fixture()
        data["fdt"]["uboot_stage_fdt"][0]["selected_dts_lines"][
            6] = 'status = "okay";'
        changed = build(data)
        row = next(x for x in changed["questions"]
                   if "enable its SDMMC boot target" in x["question"])
        self.assertEqual(row["status"], "unresolved")

    def test_invalid_boot_environment_does_not_claim_valid(self):
        data = fixture()
        data["boot"]["p2"]["crc_valid_for_standard_layout"] = False
        report = build(data)
        integrity = next(x for x in report["questions"]
                         if "environment CRC valid" in x["question"])
        self.assertEqual(integrity["status"], "unresolved")

    def test_zip_index_strict_integrity(self):
        with tempfile.TemporaryDirectory() as folder:
            p = Path(folder) / "synthetic.zip"
            data = b"synthetic"
            idx = {"schema":"mpfree-bootchain-integrated-zip-index-v1",
                   "files":[{"name":"mock.txt","bytes":len(data),
                             "sha256":hashlib.sha256(data).hexdigest()}]}
            with zipfile.ZipFile(p, "w") as z:
                z.writestr("mock.txt",data)
                z.writestr(M.INDEX,json.dumps(idx))
            with zipfile.ZipFile(p) as z:
                self.assertEqual(M.verify_zip(z),1)
            with zipfile.ZipFile(p,"a") as z:
                z.writestr("unindexed.txt","bad")
            with zipfile.ZipFile(p) as z:
                with self.assertRaises(ValueError):
                    M.verify_zip(z)

if __name__=="__main__":
    unittest.main()
