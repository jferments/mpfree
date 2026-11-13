#!/usr/bin/env python3
"""V22 host regressions: accurate owner provenance, 29 genuinely new
guest cases, no Thumb patch of ARM division, actual registered callback,
85-question lineage, and SHA-indexed private report.
These tests do NOT assert the actual owner ARM/QEMU execution succeeded.
"""
import ast,hashlib,importlib.util,json,tempfile,unittest,zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location(
    "mpfree_v22_regression",ROOT/"tools/mpc-one/run_v22.py")
runner=importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
def sample_v21(path,invalid=False):
    files={
     "V21-STAGE-MANIFEST.json":{"owner_P3_sha256":
        ("0"*64 if invalid else runner.PINNED)},
     "V21-REGISTRY-DISCOVERY-AND-85-QUESTIONS.json":{
        "questions":[{"id":"Q-%02d"%i,"question":"open"} for i in range(85)]},
     "deep-p3/V21-P3-BACKEND-REGISTRY-SUMMARY.json":{
        "owner_source_verified":True,"owner_input_sha256":runner.PINNED},
     "V21-ACTUAL-VENDOR-BACKEND-ARM.json":{"tests_planned":11,"passed":6},
     "V21-RUNTIME-FAULT-AND-CALLBACK-TRACES.json":{"traps":[]},
    }
    index=[]
    with zipfile.ZipFile(path,"w",compression=zipfile.ZIP_DEFLATED) as z:
        for name,row in files.items():
            data=json.dumps(row).encode()
            z.writestr(name,data)
            index.append({"name":name,"bytes":len(data),
                          "sha256":hashlib.sha256(data).hexdigest()})
        z.writestr("ZIP_INDEX.json",json.dumps({"files":index}))
class V22Contracts(unittest.TestCase):
    def test_29_new_cases_without_old_vm_duplication(self):
        c=runner.cases()
        self.assertEqual(len(c),29)
        self.assertEqual(len(set(c)),29)
        self.assertEqual(sum(n.startswith("v22_div_") for n in c),12)
        self.assertEqual(sum(n.startswith("v22_wrapper_") for n in c),8)
        self.assertEqual(sum(n.startswith("v22_write_") for n in c),9)
        self.assertFalse(set(c)&set(runner.V21.cases()))

    def test_prior_sha_and_85_questions_checked(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/"owner-v21.zip";sample_v21(p)
            self.assertEqual(len(runner.verified_prior(p)),5)

    def test_wrong_owner_source_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/"owner-v21.zip";sample_v21(p,invalid=True)
            with self.assertRaises(ValueError):runner.verified_prior(p)

    def test_real_ARM_division_unmodified_not_stubbed(self):
        c=(ROOT/"tools/mpc-one/vendor_probe_v22.c").read_text()
        self.assertIn("V22_DIVISION ((uintptr_t)0x0012ee08u)",c)
        self.assertIn("v22_udiv",c)
        self.assertIn("V22_OWNER_WRITE ((uintptr_t)0x0011de51u)",c)
        self.assertIn("V22_WRITE_POINTER_LITERAL",c)
        self.assertIn("v22_synthetic_media_write",c)
        self.assertIn("0x0011ca78u",c)
        self.assertNotIn("intercept(0x0012ee08",c)
        self.assertNotIn("replace_guest_stub(0x0012ee08",c)
        # The real hardware transport MUST be intercepted in every actual
        # vendor write scenario. No host MMC or USB raw device is opened.
        self.assertIn("!intercept(0x0011ca78u,",c)
        self.assertNotIn("system(",c)

    def test_Ghidra_handles_write_erase_and_driver_indirection(self):
        source=(ROOT/"tools/ghidra/MPFreeV22BlockTransport.py").read_text()
        tree=ast.parse(source)
        fns={node.name:node.lineno for node in tree.body if
             isinstance(node,ast.FunctionDef)}
        for name in ("trace","fixed_reach","image_u32"):
            self.assertIn(name,fns)
            usage=[node.lineno for node in ast.walk(tree) if
                   isinstance(node,ast.Call) and
                   isinstance(node.func,ast.Name) and node.func.id==name]
            self.assertTrue(usage)
            self.assertFalse([n for n in usage if n<fns[name]])
        self.assertIn("registered_backend_callbacks_verified",source)
        self.assertIn("0x0011de50",source)
        self.assertIn("0x0011dd4c",source)
        self.assertIn("0x0011ca78",source)
        self.assertIn("source_hash_verified",source)

    def test_no_answer_fabricated_from_failed_vm(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)
            v21={"V21-REGISTRY-DISCOVERY-AND-85-QUESTIONS.json":
                 json.dumps({"questions":[{"id":"T-%02d"%i,
                 "question":"Needs hardware"} for i in range(85)]}).encode()}
            report=runner.analyze(path,v21,{})
            self.assertEqual(report["total_new_vendor_cases_passed"],0)
            self.assertEqual(report["questions_with_new_evidence"],0)
            self.assertEqual(len(report["questions"]),85)
            self.assertTrue(all(not r["answered_by_v22"] for r in report["questions"]))

    def test_single_self_verified_zip(self):
        with tempfile.TemporaryDirectory() as td:
            target=Path(td)/"private";target.mkdir()
            runner.write(target/"V22-STAGE-MANIFEST.json",{"test":True})
            zipfile_path,num=runner.package(target)
            self.assertEqual(num,1)
            self.assertTrue(zipfile_path.with_suffix(".zip.sha256").exists())
            with zipfile.ZipFile(zipfile_path) as z:
                self.assertIsNone(z.testzip())
                index=json.loads(z.read("ZIP_INDEX.json"))
                self.assertEqual(index["schema"],"mpfree-private-v22")
                item=index["files"][0]
                self.assertEqual(hashlib.sha256(z.read(item["name"])).hexdigest(),
                                 item["sha256"])
if __name__=="__main__":
    unittest.main(verbosity=2)
