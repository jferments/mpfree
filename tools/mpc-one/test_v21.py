#!/usr/bin/env python3
"""Host-only V21 regressions: verified private research input, program-source
contracts, no synthetic-question inflation, exact guest-fault provenance.
End-to-end Ghidra/ARM QEMU must still run on the owner's ML-tower.
"""
import ast
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT=Path(__file__).resolve().parents[2]
SCRIPT=ROOT/"tools/mpc-one/run_v21.py"
spec=importlib.util.spec_from_file_location("mpfree_v21_regressions",SCRIPT)
v21=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v21)

def fake_v20(path,bad=False):
    source={
      "V20-STAGE-MANIFEST.json":{
        "owner_p3_sha256":v21.SHA if not bad else "0"*64},
      "V20-EXACT-FINDINGS-AND-85-QUESTIONS.json":{
        "questions":[{"id":"T-%02d"%n,"question":"An unanswered question"}
                     for n in range(85)]},
      "V20-ACTUAL-VENDOR-STORAGE-COMMAND-ARM.json":{
        "passed":20,"tests_planned":22,"success":False},
      "deep-p3/V20-P3-BACKEND-COMMAND-SUMMARY.json":{
        "owner_input_verified":True,"owner_sha256":v21.SHA,
        "storage_vtable_offset_model":{
          "backend_object_projection_expected_delta":200,
          "derived_backend_callback_offset":"0x00000130"}},
    }
    idx=[]
    with zipfile.ZipFile(path,"w",compression=zipfile.ZIP_DEFLATED) as z:
        for name,obj in source.items():
            data=json.dumps(obj).encode("utf8")
            z.writestr(name,data)
            idx.append({"name":name,"bytes":len(data),
                        "sha256":hashlib.sha256(data).hexdigest()})
        z.writestr("ZIP_INDEX.json",json.dumps({"files":idx}))
    return source

class V21Contracts(unittest.TestCase):
    def test_only_new_guest_scenarios(self):
        names=v21.cases()
        self.assertEqual(len(names),11)
        self.assertEqual(len(set(names)),11)
        self.assertFalse(set(names)&set(v21.V20.cases()))
        self.assertFalse(set(names)&set(v21.V19.selected_cases()))
        self.assertEqual(sum(n.startswith("v21_registry_") for n in names),5)
        self.assertEqual(sum(n.startswith("v21_transfer_") for n in names),6)

    def test_previous_owner_zip_verified(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"v20.zip"
            fake_v20(path)
            prior=v21.verified_prior(path)
            self.assertEqual(len(prior),4)

    def test_rejects_wrong_owner_source(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"wrong.zip"
            fake_v20(path,bad=True)
            with self.assertRaises(ValueError):
                v21.verified_prior(path)

    def test_new_Ghidra_expr_definitions_precede_callers(self):
        script=(ROOT/"tools/ghidra/MPFreeV21BackendRegistry.py").read_text()
        tree=ast.parse(script)
        functions={n.name:n.lineno for n in tree.body
                   if isinstance(n,ast.FunctionDef)}
        for name in ("expression","literal_constants","graph_depth","refs_to"):
            self.assertIn(name,functions)
            calls=[x.lineno for x in ast.walk(tree) if isinstance(x,ast.Call)
                   and isinstance(x.func,ast.Name) and x.func.id==name]
            self.assertTrue(calls)
            self.assertFalse([x for x in calls if x<functions[name]],
                             "Ghidra V17 premature-reference bug recurred")
        self.assertIn("source_value_constants",script)
        self.assertIn("stored_value_dependencies",script)
        self.assertIn("boot_command_string_pointer_inventory",script)

    def test_actual_guest_includes_registry_and_crash_PC_diagnostics(self):
        c=(ROOT/"tools/mpc-one/vendor_probe_v21.c").read_text()
        self.assertIn("V21_SENTINEL_CELL",c)
        self.assertIn("V21_BACKEND_A+0xccu",c)
        self.assertIn("V21_BACKEND_C+0x130u",c)
        self.assertIn("ucontext_t",c)
        self.assertIn("arm_pc",c)
        self.assertIn("arm_lr",c)
        self.assertIn("arm_r5",c)
        self.assertIn("v21_guest_fault",c)
        self.assertNotIn("system(",c)

    def test_fault_traces_classify_guest_addresses_not_physical(self):
        with tempfile.TemporaryDirectory() as td:
            directory=Path(td)
            log=directory/"v21-owner-ARM-guest-console.log"
            log.write_text("\n".join([
               '{"v21_guest_fault":"signal","test":"v21_transfer_0",'
               '"signal":4,"pc":"0x0010b7d6","lr":"0x0010b7da"}',
               '{"v21_guest_fault":"signal","test":"v21_transfer_1",'
               '"signal":4,"pc":"0x00900040","lr":"0x0010b7da"}',
            ])+"\n")
            results=v21.parse_faults(directory)
            self.assertEqual(len(results["traps"]),2)
            self.assertEqual(
                [x["pc_region"] for x in results["traps_by_location"]],
                ["owner_P3","guest_counter_trampoline"])

    def test_zero_VM_observation_does_not_close_any_question(self):
        with tempfile.TemporaryDirectory() as td:
            out=Path(td)
            source=fake_v20(out/"prior.zip")
            files={k:json.dumps(v).encode() for k,v in source.items()}
            results=v21.analyze(out,files,{})
            self.assertEqual(len(results["questions"]),85)
            self.assertEqual(results["new_arm_cases_observed"],0)
            self.assertEqual(results["questions_with_new_evidence"],0)
            self.assertTrue(all(not x["answered_in_v21"] for x in results["questions"]))

    def test_one_zip_is_self_verifying_and_private(self):
        with tempfile.TemporaryDirectory() as td:
            out=Path(td)/"private-evidence"
            out.mkdir()
            v21.write(out/"V21-STAGE-MANIFEST.json",{"test":True})
            archive,num=v21.package(out)
            self.assertEqual(num,1)
            self.assertTrue(archive.with_suffix(".zip.sha256").exists())
            with zipfile.ZipFile(archive) as z:
                self.assertIsNone(z.testzip())
                idx=json.loads(z.read("ZIP_INDEX.json"))
                self.assertEqual(idx["schema"],"mpfree-private-v21")
                row=idx["files"][0]
                self.assertEqual(hashlib.sha256(z.read(row["name"])).hexdigest(),
                                 row["sha256"])

if __name__=="__main__":
    unittest.main(verbosity=2)
