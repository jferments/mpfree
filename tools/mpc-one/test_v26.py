#!/usr/bin/env python3
"""V26 host-only contracts: exact V25 archive provenance, 10 original
native MMC subcommand descriptors, UART crash fix, 20 corrected prior
cases and 12 new MMCINFO cases, 85 retained questions and SHA-indexed ZIP.

DOES NOT claim that owner Ghidra or ARM QEMU has run.
"""
from __future__ import annotations
import hashlib,importlib.util,json,tempfile,unittest,zipfile
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location(
    "mpfree_v26_contract",ROOT/"tools/mpc-one/run_v26.py")
v26=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v26)
def mock_v25(path,bad=False):
    files={
      "V25-STAGE-MANIFEST.json":{
        "owner_P1_SHA256":v26.SHA_P1,
        "owner_P3_SHA256":"0"*64 if bad else v26.SHA_P3},
      "V25-OWNER-NATIVE-MMC-COMMAND-ARM.json":{
        "tests_planned":20,"tests_observed":2,"passed":2},
      "V25-RESULTS-AND-85-QUESTIONS.json":{
        "85_research_questions":[{"id":"Q-%02d"%i,"question":"Open"}
                                 for i in range(85)]},
      "deep-p1/V25-P1-SPL-BOOT-MEDIA-ORDER.json":{
        "owner_SHA256":v26.SHA_P1,
        "priority_entries":[{"literal_media_selector_value":"0xdeadbeef"}
                            for _ in range(5)]},
      "deep-p3/V25-OWNER-COMMAND-HANDLERS-SUMMARY.json":{
        "owner_SHA256":v26.SHA_P3,
        "actual_registered_commands":86},
      "V25-BOOT-TARGETS-NATIVE-COMMAND-AND-TEST-ROOTCAUSE.json":{
        "compiled_default_boot_targets_literal":["mmc0","mmc1"]}}
    index=[]
    with zipfile.ZipFile(path,"w",compression=zipfile.ZIP_DEFLATED) as z:
        for key,object_ in files.items():
            value=json.dumps(object_).encode()
            z.writestr(key,value)
            index.append({"name":key,"sha256":hashlib.sha256(value).hexdigest(),
                          "bytes":len(value)})
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v25","files":index}))
class V26Host(unittest.TestCase):
    def test_real_v25_failures_stay_failures(self):
        with tempfile.TemporaryDirectory() as td:
            archive=Path(td)/"V25.zip";mock_v25(archive)
            prior=v26.prior(archive)
            self.assertEqual(len(prior),6)

    def test_wrong_original_P3_input_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            archive=Path(td)/"V25.zip";mock_v25(archive,bad=True)
            with self.assertRaises(ValueError):v26.prior(archive)

    def test_20_UART_corrected_cases_and_12_new_MMCINFO(self):
        names=v26.cases()
        self.assertEqual(len(names),32)
        self.assertEqual(len(set(names)),32)
        self.assertEqual(sum(n.startswith("v26_native_mmc_") for n in names),20)
        self.assertEqual(sum(n.startswith("v26_mmcinfo_") for n in names),12)
        self.assertFalse(set(names)&set(v26.V25.cases()))

    def test_uart_mmio_is_intercepted_but_command_handler_real(self):
        s=(ROOT/"tools/mpc-one/vendor_probe_v26.c").read_text()
        for term in (
          "intercept(0x0011f580u,0u,&uart)",
          "intercept(0x0011f2d6u,0u,&alternate)",
          "intercept(0x0011c4e8u,1u,&controller)",
          "intercept(0x0011c912u,0u,&config)",
          "intercept(0x0011de50u,0u,&write)",
          "intercept(0x0011dd4cu,0u,&erase)",
          "intercept(0x00108110u,1u,&persist)",
          "v26_guest_mmcinfo",
          "v26_guest_mmc",
          "V26_MMC_HANDLER ((uintptr_t)0x00104f11u)",
          "V26_MMC_DESCRIPTOR ((uintptr_t)0x00144ca4u)",
        ):
            self.assertIn(term,s)
        self.assertNotIn("intercept(0x00104f10u",s)
        self.assertNotIn("system(",s)

    def test_nested_main_wrapper_is_present(self):
        parent=(ROOT/"tools/mpc-one/vendor_probe_v25.c").read_text()
        self.assertIn("#ifdef MPFREE_V26_WRAPPING",parent)
        self.assertIn("#define main mpfree_v25_legacy_main",parent)

    def test_new_Ghidra_targets_actual_subcommands_and_UART(self):
        s=(ROOT/"tools/ghidra/MPFreeV26MMCRootAndUART.py").read_text()
        for t in (
            "MMC_SUBCOMMAND_SOURCE_LITERAL=0x00104f7c",
            "for i in range(10)",
            "all_ten_descriptors_in_owner_rom",
            "V26-OWNER-MMC-SUBCOMMAND-TABLE.json",
            "real_serial_output_UART_MMIO",
            "0x0011f592",
            "source_high_pcode",
        ):
            if t=="source_high_pcode":
                self.assertIn("getHighFunction()",s)
            else:self.assertIn(t,s)

    def test_no_static_SPL_boot_order_claim(self):
        s=(ROOT/"tools/ghidra/MPFreeV26SPLDynamicMedia.py").read_text()
        for t in (
            "0xff704120","0xff705342","0xff705350",
            "preinit_array","dynamic_write_function",
            "all_entries_sentinel","exact_physical_SD_eMMC_order_proven",
            "V26-P1-DYNAMIC-BOOT-SOURCE-ORIGIN.json",
        ):self.assertIn(t,s)

    def test_old_85_questions_not_automatically_closed(self):
        with tempfile.TemporaryDirectory() as td:
            original={"V25-RESULTS-AND-85-QUESTIONS.json":
               json.dumps({"85_research_questions":[
                   {"id":"T-%02d"%n,"question":"Unknown"}
                   for n in range(85)]}).encode()}
            report=v26.analysis(Path(td),original,{})
            self.assertEqual(len(report["questions"]),85)
            self.assertEqual(report["questions_with_new_source_or_runtime_evidence"],0)
            self.assertEqual(report["V26_actual_ARM"]["v26_native_mmc_"]["passed"],0)
            self.assertFalse(report["P1_boot_selector"]["actual_physical_priority_resolved"])

    def test_hash_indexed_private_zip(self):
        with tempfile.TemporaryDirectory() as td:
            folder=Path(td)/"V26-private";folder.mkdir()
            v26.write(folder/"V26-STAGE-MANIFEST.json",{"preflight":True})
            path,n=v26.package(folder)
            self.assertEqual(n,1)
            with zipfile.ZipFile(path) as z:
                self.assertIsNone(z.testzip())
                rows=json.loads(z.read("ZIP_INDEX.json"))
                self.assertEqual(rows["schema"],"mpfree-private-v26")
                record=rows["files"][0]
                self.assertEqual(
                    hashlib.sha256(z.read(record["name"])).hexdigest(),
                    record["sha256"])
if __name__=="__main__":
    unittest.main(verbosity=2)
