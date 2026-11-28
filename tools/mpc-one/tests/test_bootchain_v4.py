"""Synthetic tests for read-only V4 firmware comparison and Ghidra coverage.

No real firmware, Ghidra runtime, MPC, disks or network are accessed.
"""
import ast
import importlib.util
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[3]
def imported(path,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/path)
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
AUDIT=imported("tools/mpc-one/offline-bootchain-v4-compare.py",
               "mpfree_bootchain_v4_compare")
LEDGER=imported("tools/mpc-one/bootchain-evidence-ledger.py",
                "mpfree_bootchain_evidence_ledger")
class V4OfflineAuditTests(unittest.TestCase):
    def test_uimage_multi_component_script_is_normalized(self):
        script=b"\nsetenv bootargs test\nbootz 0x2000000 - 0x1f00000\n"
        payload=struct.pack(">II",len(script),0)+script
        header=bytearray(64)
        header[:4]=b"\x27\x05\x19\x56"
        struct.pack_into(">I",header,12,len(payload))
        a=AUDIT.boot_script(bytes(header)+payload)
        self.assertEqual(a["commands"],script.decode().rstrip("\n"))
        self.assertEqual(a["header"]["multi_image_script_length"],len(script))
        b=AUDIT.boot_script(bytes(header)+payload+b"unused-padding")
        self.assertEqual(a["normalized_sha256"],b["normalized_sha256"])
    def test_script_strips_only_trailing_newline(self):
        a=AUDIT.boot_script(b"echo hello")
        b=AUDIT.boot_script(b"echo hello\n")
        self.assertEqual(a["normalized_sha256"],b["normalized_sha256"])
    def test_dtb_property_diff_is_structural(self):
        a={"sha256":"old","nodes":{"/board/reboot-mode":["mode-loader = <0x1>;"],
                                     "/board/mmc":["status = \"okay\";"]}}
        b={"sha256":"new","nodes":{"/board/reboot-mode":["mode-loader = <0x2>;"],
                                     "/board/mmc":["status = \"okay\";"]}}
        d=AUDIT.diff_dtb(a,b)
        self.assertEqual(d["nodes_changed"],1)
        self.assertIn("/board/reboot-mode",d["changed_boot_nodes"])
        self.assertNotIn("/board/mmc",d["changed_boot_nodes"])
    def test_symlink_as_input_refused(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/"regular.img";p.write_bytes(b"\0"*100)
            link=Path(td)/"indirect.img";link.symlink_to(p)
            self.assertEqual(AUDIT.verify_regular(p),p)
            with self.assertRaises(ValueError):
                AUDIT.verify_regular(link)
    def test_non_elf_is_not_decompiled(self):
        with tempfile.TemporaryDirectory() as td:
            d=AUDIT.get_elf_details(b"not-ELF",td)
            self.assertFalse(d["elf"])
    def test_evidence_ledger_handles_partial_failing_passes(self):
        with tempfile.TemporaryDirectory() as td:
            folder=Path(td)/"bootchain-wide-synthetic"
            folder.mkdir()
            import json
            (folder/"p1-evidence-graph-v3.json").write_text(json.dumps({
                "all_known_function_graph":{"0xff704020":{}},
                "detailed_functions":{}}))
            (folder/"p1-coverage-v4.json").write_text(json.dumps({
                "newly_detailed":1}))
            (folder/"storage-env-boot-dtb.json").write_text(json.dumps({
                "p2":{"crc_valid_for_standard_layout":True}}))
            result=LEDGER.make(folder)
            self.assertTrue(len(result["findings"])>=8)
            self.assertIn("requires hardware",LEDGER.render(result))
            self.assertFalse(result["passes"]["p3_coverage"]["available"])

    def test_all_v4_sources_parse(self):
        for relative in (
          "tools/ghidra/MPFreeBootChainCoverageV4.py",
          "tools/mpc-one/offline-bootchain-v4-compare.py",
          "tools/mpc-one/bootchain-evidence-ledger.py",
        ):
            path=ROOT/relative
            with self.subTest(path=str(path)):
                ast.parse(path.read_text(encoding="utf-8"),filename=str(path))
    def test_full_runner_bash_syntax(self):
        f=ROOT/"tools/ghidra/run_wide_bootchain_audit.sh"
        p=subprocess.run(["bash","-n",str(f)],capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr)
if __name__=="__main__":unittest.main()
