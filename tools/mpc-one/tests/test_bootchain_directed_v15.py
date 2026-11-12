#!/usr/bin/env python3
"""V15 self-tests: real graph constraints, immutable-prior lineage and honesty."""
import importlib.util
from pathlib import Path
import json
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
SCRIPT=ROOT/"tools/mpc-one/run_bootchain_directed_v15.py"
spec=importlib.util.spec_from_file_location("mpfree_v15_tests",SCRIPT)
v15=importlib.util.module_from_spec(spec);spec.loader.exec_module(v15)

class V15Regression(unittest.TestCase):
    def test_shortest_path_and_cycles(self):
        edges={"a":{"b","c"},"b":{"d"},"c":{"a"},"d":{"e"}}
        self.assertEqual(v15.graph_path(edges,"a","e"),["a","b","d","e"])
        self.assertIsNone(v15.graph_path(edges,"e","a"))
        self.assertIsNone(v15.graph_path(edges,"a","e",2))
        self.assertEqual(v15.graph_path(edges,"a","a"),["a"])

    def test_graph_does_not_invent_indirect_targets(self):
        report={"functions":{
            "0x1":{"name":"origin","calls":[
                {"target_entries":["0x2"]},{"target_entries":[]},
                {"target_entries":["0xdeadbeef"]}]},
            "0x2":{"name":"real","calls":[]}}}
        graph=v15.build_graph(report)
        self.assertEqual(graph["0x1"],{"0x2"})
        self.assertNotIn("0xdeadbeef",graph["0x1"])

    def test_pinned_prior_v14_graph_reuse_reviews_85_questions(self):
        p1={"owner_sha256":v15.P1_SHA,"owner_hash_verified":True,
            "functions":{"0x1":{"name":"dummy","calls":[]}}}
        p3={"owner_sha256":v15.P3_SHA,"owner_hash_verified":True,
            "functions":{
                "0x00122854":{"name":"mpf_fastboot_oem","calls":[
                    {"target_entries":["0x00110484"]},{"target_entries":["0x00122834"]}],
                    "pseudocode":"command parser permission"},
                "0x00110484":{"name":"mpf_run_command_candidate","calls":[]},
                "0x00122834":{"name":"mpf_fastboot_permission","calls":[]},
                "0x00122980":{"name":"mpf_fastboot_download","calls":[]},
            }}
        q=[{"id":"FASTBOOT-%02d"%n,"question":"How is permission checked?",
            "classification":"PROBED_UNRESOLVED"} for n in range(85)]
        fake={
            "v14-full-p1-ghidra.json":json.dumps(p1).encode(),
            "v14-full-p3-ghidra.json":json.dumps(p3).encode(),
            "V14-85-QUESTION-EVIDENCE-LEDGER.json":json.dumps({"questions":q}).encode(),
        }
        with tempfile.TemporaryDirectory() as td:
            folder=Path(td)
            previous=folder/"owner-v14.zip";previous.write_bytes(b"dummy")
            with patch.object(v15.v14,"verified_archive",return_value=fake):
                result=v15.prior_analysis(folder,previous)
            self.assertTrue(result["success"],result.get("error"))
            self.assertEqual(result["questions_indexed"],85)
            report=json.loads((folder/"V15-PRIOR-GRAPH-AND-85-QUESTIONS.json").read_text())
            self.assertEqual(report["question_count"],85)
            self.assertTrue(any(x["from"]=="oem_unlock" and
                                x["to"]=="command_interpreter" for x in report["routes"]))
            self.assertTrue(all(x["no_new_runtime_proof"] for x in report["individual_questions"]))

    def test_no_fake_vm_proof(self):
        with tempfile.TemporaryDirectory() as td:
            folder=Path(td)
            prior=folder/"dummy.zip";prior.write_bytes(b"dummy")
            result=v15.synthesize(folder,{"real_vendor_function_vm":{"success":False}},prior)
            self.assertEqual(result["conclusions"]["isolated_permission_execution"]["status"],
                             "UNRESOLVED")
            self.assertFalse(result["all_85_questions_closed"])

    def test_not_a_weak_SRAM_model_report(self):
        source=(ROOT/"tools/mpc-one/v15_vendor_permission_guest.c").read_text()
        self.assertIn("ENTRY_THUMB",source)
        self.assertIn("MAP_FIXED_NOREPLACE",source)
        self.assertIn("RESPONSE_ADDR",source)
        self.assertIn("mpfree_v15",str(SCRIPT))
        # The guest only runs a bounded function slice; not any update CLI.
        self.assertNotIn("fastboot flash ",source)
        self.assertNotIn("az01-update",source)

if __name__=="__main__":
    unittest.main(verbosity=2)
