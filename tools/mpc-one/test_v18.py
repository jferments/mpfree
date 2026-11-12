#!/usr/bin/env python3
"""V18 host regression: no uninitialized Ghidra analyzer, distinct new cases,
strict coverage, prior provenance, no fabricated experimental answers.
"""
import ast
import importlib.util
from pathlib import Path
import json
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
if not (ROOT/"mpc-one/run_v18.py").exists():
    ROOT=Path(__file__).resolve().parents[3]/"tools"
spec=importlib.util.spec_from_file_location(
    "mpfree_v18_test",ROOT/"mpc-one/run_v18.py")
v18=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v18)

class V18EvidenceContracts(unittest.TestCase):
    def test_16_new_vendor_arm_cases_and_no_v17_repeat(self):
        fresh=v18.v18_cases()
        old=set(v18.V17.tested_cases())
        self.assertEqual(len(fresh),16)
        self.assertEqual(len(set(fresh)),16)
        self.assertFalse(set(fresh)&old)
        self.assertEqual(sum("getvar" in c for c in fresh),12)

    def test_ghidra_dataflow_function_defined_before_main_sweep(self):
        source=(ROOT/"ghidra/MPFreeV17FixedPointDive.py").read_text()
        tree=ast.parse(source)
        lines={node.name:node.lineno for node in tree.body
               if isinstance(node,(ast.FunctionDef,ast.AsyncFunctionDef))}
        self.assertIn("possible_constant_targets",lines)
        self.assertIn("varnode",lines)
        usages=[node.lineno for node in ast.walk(tree)
                if isinstance(node,ast.Call) and isinstance(node.func,ast.Name)
                and node.func.id=="possible_constant_targets"]
        self.assertTrue(usages)
        # Ignore recursive uses inside the function itself; actual call from
        # full High P-code loop must occur AFTER definition.
        loop_uses=[x for x in usages if x<lines["possible_constant_targets"]]
        self.assertFalse(loop_uses,"V17's exact NameError regression is back")

    def test_static_coverage_fails_closed(self):
        with tempfile.TemporaryDirectory() as root:
            d=Path(root)
            folder=d/"deep-p3";folder.mkdir()
            (folder/"V17-p3-SUMMARY.json").write_text(json.dumps({
                "functions_attempted":1050,
                "statistics":{"function_decompiled":480,"decompile_failed":570}}))
            (folder/"V17-p3-INDIRECT-AND-MMIO.jsonl").write_text("")
            with patch.object(v18.V17,"ghidra_saved",return_value={"success":True}):
                data=v18.ghidra("p3",d)
            self.assertFalse(data["success"])
            self.assertEqual(data["dataflow_effect_rows"],0)
            self.assertIn("threshold",data["error"])

    def test_new_pass_has_real_pcode_effects(self):
        with tempfile.TemporaryDirectory() as root:
            d=Path(root)
            folder=d/"deep-p1";folder.mkdir()
            (folder/"V17-p1-SUMMARY.json").write_text(json.dumps({
                "functions_attempted":213,
                "statistics":{"function_decompiled":212,"decompile_failed":1}}))
            (folder/"V17-p1-INDIRECT-AND-MMIO.jsonl").write_text(
                '{"function":"0xff705208","kind":"CALLIND"}\n')
            with patch.object(v18.V17,"ghidra_saved",return_value={"success":True}):
                data=v18.ghidra("p1",d)
            self.assertTrue(data["success"])
            self.assertEqual(data["dataflow_effect_rows"],1)

    def test_proprietary_bytes_only_guest_temporary_file(self):
        runner=(ROOT/"mpc-one/run_v18.py").read_text()
        self.assertIn("tempfile.TemporaryDirectory",runner)
        self.assertIn("V17.package(output)",runner)
        self.assertIn("raw=temp/",runner)
        self.assertNotIn("usb-host",runner)

if __name__=="__main__":
    unittest.main(verbosity=2)
