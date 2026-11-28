"""Offline synthetic tests for V5 graph synthesis and hardware evidence gates."""
import ast
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[3]
MODULE=ROOT/"tools/mpc-one/offline-bootchain-v5-synthesis.py"
spec=importlib.util.spec_from_file_location("mpfree_v5_synthesis",MODULE)
v5=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v5)

class V5EvidenceTests(unittest.TestCase):
    def test_shortest_direct_call_path(self):
        graph={
            "0x00000001":{"calls":["0x00000003"]},
            "0x00000003":{"calls":["0x00000002"]},
            "0x00000002":{"calls":[]}
        }
        self.assertEqual(v5.shortest(graph,"0x00000001","0x00000002"),
                         ["0x00000001","0x00000003","0x00000002"])
        self.assertIsNone(v5.shortest(graph,"0x00000002","0x00000001"))

    def test_complete_synthetic_cross_stage_evidence(self):
        with tempfile.TemporaryDirectory() as td:
            folder=Path(td)/"bootchain-wide-test";folder.mkdir()
            for stage in ("p1","p3"):
                sha=v5.GRAPH_SHA[stage]
                if stage=="p1":
                    a="0xff704060"; b="0xff705350"
                else:
                    a="0x001226b4"; b="0x00101744"
                graph={
                   a:{"name":"source","calls":[b],
                      "computed_calls":[]},
                   b:{"name":"target","calls":[],
                      "computed_calls":["indirect_call_site"]},
                }
                previous={a:{"pseudocode":"void test(void) { *DAT_00122928 = 1; }",
                             "name":"source"}}
                later={b:{"pseudocode":"void target(void) {}",
                           "name":"target"}}
                v3={"schema":"mpfree-evidence-graph-v3","kind":stage,"sha256":sha,
                    "all_known_function_graph":graph,"detailed_functions":previous}
                v4={"schema":"mpfree-recognized-function-coverage-v4",
                    "source_payload_sha256":sha,"coverage":later}
                (folder/(stage+"-evidence-graph-v3.json")).write_text(json.dumps(v3))
                (folder/(stage+"-coverage-v4.json")).write_text(json.dumps(v4))
            out=v5.report(folder)
            self.assertEqual(out["bootloader_stages"]["p3"]["recognized_functions"],2)
            self.assertTrue(out["bootloader_stages"]["p3"]["paths"][
                "fastboot_ram_boot_to_bootm"]["direct_call_path_found"])
            self.assertEqual(len(out["hardware_proof_gates"]),6)
            self.assertIn("Hardware",v5.summarize(out).replace("hardware","Hardware"))

    def test_rejects_wrong_image_signature(self):
        with tempfile.TemporaryDirectory() as td:
            folder=Path(td)
            (folder/"p1-evidence-graph-v3.json").write_text(json.dumps({
                "schema":"mpfree-evidence-graph-v3","kind":"p1",
                "sha256":"bad"}))
            (folder/"p1-coverage-v4.json").write_text(json.dumps({
                "schema":"mpfree-recognized-function-coverage-v4",
                "source_payload_sha256":v5.GRAPH_SHA["p1"]}))
            with self.assertRaises(RuntimeError):v5.get_all("p1",folder)

    def test_script_and_launcher_syntax(self):
        ast.parse(MODULE.read_text(),filename=str(MODULE))
        sh=ROOT/"tools/ghidra/run_wide_bootchain_audit.sh"
        p=subprocess.run(["bash","-n",str(sh)],capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr)

if __name__=="__main__": unittest.main()
