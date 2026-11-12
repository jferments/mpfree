#!/usr/bin/env python3
"""MPFree V20 host-only regression: archive provenance, new cases and proof scope."""
import ast,hashlib,importlib.util,json,tempfile,unittest,zipfile
from pathlib import Path
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location("mpfree_v20_host",ROOT/"tools/mpc-one/run_v20.py")
v20=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v20)
class V20(unittest.TestCase):
    def test_22_novel_case_names(self):
        s=v20.cases()
        self.assertEqual(len(s),22)
        self.assertEqual(len(set(s)),22)
        self.assertEqual(sum("storage" in x for x in s),6)
        self.assertEqual(sum("command" in x for x in s),16)

    def test_backend_offset_and_real_parser_present(self):
        program=(ROOT/"tools/mpc-one/vendor_probe_v20.c").read_text()
        self.assertIn("0x130u",program)
        self.assertIn("0x0010b74d",program)
        self.assertIn("0x00110485",program)
        self.assertIn("intercept(0x00109b68u",program)
        self.assertIn("actual",program.lower())

    def test_ghidra_static_provenance_and_expression_defs(self):
        script=(ROOT/"tools/ghidra/MPFreeV20BackendCensus.py").read_text()
        tree=ast.parse(script)
        names={s.name:s.lineno for s in tree.body if isinstance(s,ast.FunctionDef)}
        self.assertIn("op_tree",names)
        self.assertIn("op_consts",names)
        self.assertIn("xref",names)
        for symbol in ("op_tree","op_consts"):
            invoked=[n.lineno for n in ast.walk(tree) if isinstance(n,ast.Call)
                     and isinstance(n.func,ast.Name) and n.func.id==symbol]
            self.assertTrue(invoked)
            self.assertTrue(all(l>=names[symbol] for l in invoked))
        self.assertIn("owner_input_verified",script)
        self.assertIn("functions_attempted",script)

    def test_verified_prior_v19_and_85_question_lineage(self):
        f={
            "V19-STAGE-MANIFEST.json":{"branch":"test"},
            "V19-85-QUESTION-EVIDENCE.json":{
                "questions":[{"id":"Q-%02d"%i,"question":"open"} for i in range(85)]},
            "deep-p3/V19-p3-SUMMARY.json":{
                "source_hash_verified":True,"owner_sha256":v20.PINNED},
            "V19-ACTUAL-P3-ARM-RUNTIME.json":{"tests_observed":0},
        }
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"prior-v19.zip"
            index=[]
            with zipfile.ZipFile(path,"w") as archive:
                for name,data in f.items():
                    blob=json.dumps(data).encode()
                    archive.writestr(name,blob)
                    index.append({"name":name,"bytes":len(blob),
                                  "sha256":hashlib.sha256(blob).hexdigest()})
                archive.writestr("ZIP_INDEX.json",json.dumps({
                    "schema":"mpfree-private-v19","files":index}))
            data=v20.prior(path)
            self.assertEqual(len(data),4)

    def test_absence_of_runtime_proof_does_not_close_questions(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)
            old={"V19-85-QUESTION-EVIDENCE.json":json.dumps({
                "questions":[{"id":"FASTBOOT_RUNTIME-02","question":"Persistent flash?"}]
                +[{"id":"Q-%d"%i,"question":"Unknown"} for i in range(84)]
            }).encode()}
            report=v20.correlate(p,old,{})
            self.assertEqual(report["questions_with_new_probes"],0)
            self.assertEqual(report["new_vendor_cases_passed"],0)
            self.assertFalse(report["questions"][0]["answered_by_v20"])

    def test_single_private_zip_and_sha(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/"evidence";p.mkdir()
            v20.write(p/"V20-STAGE-MANIFEST.json",{"ok":True})
            archive,number=v20.package(p)
            self.assertEqual(number,1)
            with zipfile.ZipFile(archive) as z:
                self.assertIsNone(z.testzip())
                rows=json.loads(z.read("ZIP_INDEX.json"))["files"]
                self.assertEqual(len(rows),1)
                self.assertEqual(hashlib.sha256(z.read(rows[0]["name"])).hexdigest(),
                                 rows[0]["sha256"])
if __name__=="__main__":
    unittest.main(verbosity=2)
