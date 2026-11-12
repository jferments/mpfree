#!/usr/bin/env python3
"""Regression tests: V14 archive SHA, full-scan evidence, non-inflated ledger."""
from pathlib import Path
import hashlib, importlib.util, json, tempfile, unittest, zipfile
ROOT=Path(__file__).resolve().parents[3]
spec=importlib.util.spec_from_file_location(
    "mpfree_v14_test_runner",ROOT/"tools/mpc-one/run_bootchain_deep_v14.py")
v14=importlib.util.module_from_spec(spec);spec.loader.exec_module(v14)

class V14Integrity(unittest.TestCase):
    def test_valid_private_zip(self):
        with tempfile.TemporaryDirectory() as td:
            f=Path(td)/"out.zip"
            payload=b'{"schema":"fixture"}\n'
            with zipfile.ZipFile(f,"w") as z:
                z.writestr("ZIP_INDEX.json",json.dumps({"files":[{
                    "name":"test.json","bytes":len(payload),
                    "sha256":hashlib.sha256(payload).hexdigest()}]}))
                z.writestr("test.json",payload)
            self.assertEqual(v14.verified_archive(f)["test.json"],payload)

    def test_rejects_mismatched_hash(self):
        with tempfile.TemporaryDirectory() as td:
            f=Path(td)/"out.zip"
            with zipfile.ZipFile(f,"w") as z:
                z.writestr("ZIP_INDEX.json",json.dumps({"files":[{
                    "name":"test.json","bytes":4,"sha256":"0"*64}]}))
                z.writestr("test.json",b"test")
            with self.assertRaises(ValueError):v14.verified_archive(f)

    def test_rejects_path_traversal(self):
        with tempfile.TemporaryDirectory() as td:
            f=Path(td)/"out.zip"
            with zipfile.ZipFile(f,"w") as z:
                z.writestr("ZIP_INDEX.json",json.dumps({"files":[{
                    "name":"../escaping.json","bytes":1,
                    "sha256":hashlib.sha256(b'x').hexdigest()}]}))
                z.writestr("../escaping.json",b"x")
            with self.assertRaises(ValueError):v14.verified_archive(f)

    def test_question_not_answered_by_many_functions(self):
        with tempfile.TemporaryDirectory() as td:
            d=Path(td)
            v14.write(d/"v14-full-p3-ghidra.json",{
                "statistics":{"ghidra_functions_total":500,
                              "functions_decompiled":500,
                              "unique_direct_call_edges":500,
                              "indirect_or_unresolved_call_sites":0,
                              "elapsed_s":100},
                "owner_hash_verified":True})
            questions=[{"id":"FASTBOOT_RUNTIME-01","question":"Can real Fastboot authorize?"}]
            _,counts=v14.analyze(d,{"full_p3":{"success":True}},questions,None)
            row=json.loads((d/"V14-85-QUESTION-EVIDENCE-LEDGER.json").read_text())["questions"][0]
            self.assertEqual(row["classification"],"NEEDS_HARDWARE_CONFIRMATION")
            self.assertTrue(row["probe_executed"])
            self.assertEqual(counts["NEEDS_HARDWARE_CONFIRMATION"],1)

    def test_never_claim_answer_if_probe_failed(self):
        with tempfile.TemporaryDirectory() as td:
            d=Path(td)
            v14.analyze(d,{"full_p3":{"success":False}},
                        [{"id":"FASTBOOT-06","question":"RAM boot FDT?"}],None)
            row=json.loads((d/"V14-85-QUESTION-EVIDENCE-LEDGER.json").read_text())["questions"][0]
            self.assertEqual(row["classification"],"NOT_PROBED_STAGE_FAILED")

    def test_static_script_answers_are_specific(self):
        with tempfile.TemporaryDirectory() as td:
            d=Path(td)
            v14.write(d/"offline-boot-assets-v13.json",{
                "files":{"/boot/boot.scr.uimg":{"uimage":{
                    "valid":True,"header_crc_valid":True,"payload_crc_valid":True,
                    "script":{"normalized_command_lines":["bootz kernel - fdt"]}}}}})
            rows=[{"id":"ROOTFS_COMPATIBILITY-02","question":"Boot script present?"},
                  {"id":"REMOVABLE_DIRECTBOOT-07","question":"Hardcoded UUID?"}]
            v14.analyze(d,{"boot_assets":{"success":True}},rows,None)
            ledger=json.loads((d/"V14-85-QUESTION-EVIDENCE-LEDGER.json").read_text())
            self.assertEqual([x["classification"] for x in ledger["questions"]],
                             ["ANSWERED_STATIC","ANSWERED_STATIC"])
            self.assertTrue(all(x["evidence"] for x in ledger["questions"]))
if __name__=="__main__":unittest.main(verbosity=2)
