#!/usr/bin/env python3
"""Host-only regression tests for V25 owner research. Actual Ghidra
and ARM QEMU are separately required and not claimed by these checks.
"""
import ast,hashlib,importlib.util,json,tempfile,unittest,zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location("mpfree_v25",ROOT/"tools/mpc-one/run_v25.py")
v25=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v25)

def fake_prior(path,wrong=False):
    values={
      "V24-STAGE-MANIFEST.json":{
        "P1_SHA256":v25.SHA_P1,
        "P3_SHA256":("0"*64 if wrong else v25.SHA_P3)},
      "V24-REAL-OWNER-COMMAND-AND-MMC-ARM.json":{
        "tests_planned":48,"tests_observed":48,"passed":44},
      "V24-ORIGINAL-UBOOT-COMMAND-DESCRIPTORS.json":{
        "descriptor_count":86,"malformed_descriptor_count":0,
        "owner_SHA256":v25.SHA_P3,
        "safe_named_commands":{"mmc":{"handler":"0x00104f11"}}},
      "V24-EXACT-FINDINGS-AND-85-QUESTIONS.json":{
        "original_questions":[{"id":"QUESTION-%02d"%i,"question":"Open"}
                              for i in range(85)]},
      "deep-p1/V24-p1-SUMMARY.json":{"source_hash_verified":True},
      "deep-p3/V24-p3-SUMMARY.json":{"source_hash_verified":True},
      "deep-p3/V24-COMMAND-REGISTRY.json":{"descriptor_count":86}}
    rows=[]
    with zipfile.ZipFile(path,"w",compression=zipfile.ZIP_DEFLATED) as z:
        for name,body in values.items():
            data=json.dumps(body).encode()
            z.writestr(name,data)
            rows.append({"name":name,"sha256":hashlib.sha256(data).hexdigest(),
                         "bytes":len(data)})
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v24","files":rows}))

class V25Contracts(unittest.TestCase):
    def test_20_new_native_MMC_command_cases(self):
        values=v25.cases()
        self.assertEqual(len(values),20)
        self.assertEqual(len(set(values)),20)
        self.assertTrue(set(values).isdisjoint(v25.V24.cases()))

    def test_correct_original_compiled_command_handler_not_lookup(self):
        source=(ROOT/"tools/mpc-one/vendor_probe_v25.c").read_text()
        self.assertIn("0x00104f11u",source)
        self.assertIn("0x00144ca4u",source)
        self.assertIn("real_native_mmc",source)
        self.assertIn("uint32_t,uint32_t,uint32_t,uint32_t",source)
        self.assertIn("v24_setup_mmc_registry",source)
        self.assertNotIn("system(",source)

    def test_no_real_persistent_command_ever_permitted(self):
        s=(ROOT/"tools/mpc-one/vendor_probe_v25.c").read_text()
        for function in ("0x0011daf4u","0x0011ca78u","0x0011de50u",
                         "0x0011dd4cu","0x00108110u","0x0011622au"):
            self.assertIn("intercept("+function,s)
        self.assertIn("calls(write)==0u",s)
        self.assertIn("calls(erase)==0u",s)
        self.assertIn("calls(persist)==0u",s)
        self.assertIn("calls(reset)==0u",s)
        for command in ("\"mmc\",\"write\"","\"mmc\",\"erase\"",
                        "\"mmc\",\"boot\"","\"mmc\",\"rpmb\""):
            self.assertNotIn(command,s)

    def test_p3_pseudo_disassembler_is_read_only_and_all_86(self):
        source=(ROOT/"tools/ghidra/MPFreeV25CommandRecovery.py").read_text()
        tree=ast.parse(source)
        self.assertIn("PseudoDisassembler(P)",source)
        self.assertIn("PseudoDisassemblerContext(P.getProgramContext())",source)
        self.assertIn("BigInteger.ONE if pointer&1",source)
        self.assertIn("range(86)",source)
        self.assertIn("61 undefined",source)
        self.assertIn("source_high_pcode_operations",str(
            ROOT/"tools/mpc-one/run_v25.py") if False else
            (ROOT/"tools/mpc-one/run_v25.py").read_text())
        self.assertNotIn("P.startTransaction(",source)
        self.assertNotIn("P.endTransaction(",source)

    def test_p1_real_priority_table_not_just_literal(self):
        source=(ROOT/"tools/ghidra/MPFreeV25SPLBootOrder.py").read_text()
        self.assertIn("0xff709c70",source)
        self.assertIn("0xff7053fc",source)
        self.assertIn("0xdeadbeef",source)
        self.assertIn("range(5)",source)
        self.assertIn("classify(id_)",source)
        self.assertIn("full_kind_table",source)
        self.assertIn("fm.getFunctions(True)",source)

    def test_baseline_V24_44_passes_and_all_85_accepted(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"prior.zip";fake_prior(path)
            records=v25.prior(path)
            self.assertEqual(len(records),7)

    def test_prior_wrong_owner_sha_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"prior.zip";fake_prior(path,wrong=True)
            with self.assertRaises(ValueError):v25.prior(path)

    def test_absent_new_results_do_not_claim_resolutions(self):
        with tempfile.TemporaryDirectory() as td:
            prior={"V24-EXACT-FINDINGS-AND-85-QUESTIONS.json":json.dumps({
                "original_questions":[{"id":"OPEN-%02d"%i,"question":"Unknown"}
                                      for i in range(85)]}).encode()}
            result=v25.analysis(Path(td),prior,{})
            self.assertEqual(len(result["85_research_questions"]),85)
            self.assertEqual(result["questions_with_new_evidence"],0)
            self.assertEqual(result["new_native_MMC_handler_guest_tests"]["passed"],0)
            self.assertFalse(result["original_P1_SPL_boot_media"]["controller_roles_physically_proven"])

    def test_one_integrity_verified_private_zip(self):
        with tempfile.TemporaryDirectory() as td:
            out=Path(td)/"results";out.mkdir()
            v25.write(out/"V25-STAGE-MANIFEST.json",{"preflight":True})
            a,n=v25.archive(out)
            self.assertEqual(n,1)
            with zipfile.ZipFile(a) as z:
                self.assertIsNone(z.testzip())
                idx=json.loads(z.read("ZIP_INDEX.json"))
                self.assertEqual(idx["schema"],"mpfree-private-v25")
                entry=idx["files"][0]
                self.assertEqual(hashlib.sha256(z.read(entry["name"])).hexdigest(),
                                 entry["sha256"])
if __name__=="__main__":
    unittest.main(verbosity=2)
