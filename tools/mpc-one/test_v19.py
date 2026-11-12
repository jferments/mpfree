#!/usr/bin/env python3
"""V19 host preflight. Deliberately tests provenance, source contracts and ZIP
safety rather than falsely claiming to exercise real vendor ARM/Ghidra.
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
spec=importlib.util.spec_from_file_location(
    "mpfree_v19_host_regression",ROOT/"tools/mpc-one/run_v19.py")
v19=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v19)

def mock_prior(location,corrupt=False,large=False):
    required={
      "V18-STAGE-MANIFEST.json":{"stages":{}},
      "deep-p1/V17-p1-SUMMARY.json":{"verified_owner":True,
                                     "source_sha256":v19.SHA_P1},
      "deep-p3/V17-p3-SUMMARY.json":{"verified_owner":True,
                                     "source_sha256":v19.SHA_P3},
      "V18-REPAIRED-DATAFLOW-AND-85-QUESTIONS.json":{
          "question_evidence":[{"id":"Q-%02d"%i,"question":"test evidence"}
                               for i in range(85)]},
      "V18-ACTUAL-VENDOR-ARM-CASES.json":{"cases":[]},
    }
    names=[]
    with zipfile.ZipFile(location,"w",compression=zipfile.ZIP_DEFLATED,
                         compresslevel=1) as z:
        for name,value in required.items():
            content=json.dumps(value).encode("utf8")
            digest=hashlib.sha256(content).hexdigest()
            if corrupt and name=="V18-STAGE-MANIFEST.json":
                digest="0"*64
            z.writestr(name,content)
            names.append({"name":name,"sha256":digest,"bytes":len(content)})
        if large:
            # The actual V18 owner P3 report is 80 MB and V17's old
            # read-all archive reader rejected it at its 64 MB limit.
            content=b"x"*(65*1024*1024)
            extra="deep-p3/V17-p3-HIGH-PCODE-FUNCTIONS.jsonl"
            z.writestr(extra,content,compress_type=zipfile.ZIP_DEFLATED,
                       compresslevel=1)
            names.append({"name":extra,"sha256":hashlib.sha256(content).hexdigest(),
                          "bytes":len(content)})
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v18","files":names}))

class V19Contracts(unittest.TestCase):
    def test_24_distinct_real_vendor_cases_not_repeats(self):
        names=v19.selected_cases()
        self.assertEqual(len(names),24)
        self.assertEqual(len(set(names)),24)
        self.assertTrue(any("recovery_adc_99" in x for x in names))
        self.assertTrue(any("recovery_adc_100" in x for x in names))
        self.assertTrue(any("recovery_mode_9" in x for x in names))
        self.assertTrue(any("getvar_4_1_2" in x for x in names))
        self.assertFalse(set(names)&set(v19.V17.tested_cases()))

    def test_v18_private_prior_integrity_and_owner_lineage(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"prior.zip";mock_prior(path)
            report=v19.verified_prior(path)
            self.assertEqual(len(report),5)

    def test_streaming_old_large_p3_report_not_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"prior-large.zip"
            mock_prior(path,large=True)
            report=v19.verified_prior(path)
            self.assertIn("deep-p3/V17-p3-SUMMARY.json",report)
            self.assertNotIn("deep-p3/V17-p3-HIGH-PCODE-FUNCTIONS.jsonl",report)

    def test_prior_rejects_mismatched_sha(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"bad.zip";mock_prior(path,corrupt=True)
            with self.assertRaises(ValueError):v19.verified_prior(path)

    def test_ghidra_recursion_declared_before_first_use(self):
        script=(ROOT/"tools/ghidra/MPFreeV19PathWitness.py").read_text()
        tree=ast.parse(script)
        funcs={n.name:n.lineno for n in tree.body if isinstance(n,ast.FunctionDef)}
        self.assertIn("trace",funcs)
        self.assertIn("fixed_reach",funcs)
        uses=[n.lineno for n in ast.walk(tree)
              if isinstance(n,ast.Call) and isinstance(n.func,ast.Name)
              and n.func.id=="trace"]
        self.assertTrue(uses)
        self.assertFalse([n for n in uses if n<funcs["trace"]])
        self.assertIn("node-budget-reached",script)
        self.assertIn("source_image_word_at_location",script)
        self.assertIn("static",script.lower())

    def test_one_private_zip_is_sha_verified(self):
        with tempfile.TemporaryDirectory() as td:
            src=Path(td)/"mpfree-v19-private-reports"
            src.mkdir()
            v19.write(src/"V19-STAGE-MANIFEST.json",{"test":True})
            path,n=v19.package(src)
            self.assertEqual(n,1)
            self.assertTrue(path.is_file())
            self.assertTrue(path.with_suffix(".zip.sha256").is_file())
            with zipfile.ZipFile(path) as z:
                self.assertIsNone(z.testzip())
                index=json.loads(z.read("ZIP_INDEX.json"))
                self.assertEqual(index["schema"],"mpfree-private-v19")
                member=index["files"][0]
                self.assertEqual(hashlib.sha256(z.read(member["name"])).hexdigest(),
                                 member["sha256"])

    def test_weak_report_cannot_close_85_questions(self):
        with tempfile.TemporaryDirectory() as td:
            src=Path(td)
            previous={"V18-REPAIRED-DATAFLOW-AND-85-QUESTIONS.json":json.dumps({
              "question_evidence":[{"id":"Q-%02d"%i,"question":"Unknown"}
                                   for i in range(85)]}).encode()}
            ledger=v19.correlate(src,previous,{"ghidra_p1":{"success":False},
                                                 "ghidra_p3":{"success":False},
                                                 "actual_arm_vm":{"success":False}})
            self.assertEqual(len(ledger["questions"]),85)
            self.assertEqual(ledger["questions_with_new_probes"],0)
            self.assertEqual(ledger["new_runtime_cases_passed"],0)

if __name__=="__main__":
    unittest.main(verbosity=2)
