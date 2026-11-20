"""Synthetic, no-hardware V5 incremental ZIP verification tests."""
import ast
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
import zipfile

ROOT=Path(__file__).resolve().parents[3]
PATH=ROOT/"tools/mpc-one/run_bootchain_v5_followup.py"
spec=importlib.util.spec_from_file_location("mpfree_v5_incremental",PATH)
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class V5ZipReuseTests(unittest.TestCase):
    def make_test_zip(self,directory,corrupt=False):
        z=Path(directory)/"bootchain-wide-synthetic.zip"
        files={}
        for kind,sha in (("p1",module.SPL_SHA),("p3",module.UBOOT_SHA)):
            files[kind+"-evidence-graph-v3.json"]=json.dumps({
                "schema":"mpfree-evidence-graph-v3","kind":kind,"sha256":sha,
                "all_known_function_graph":{},
                "detailed_functions":{}}).encode()
            files[kind+"-coverage-v4.json"]=json.dumps({
                "schema":"mpfree-recognized-function-coverage-v4",
                "source_payload_sha256":sha,"coverage":{}}).encode()
        files["firmware-3.10-bootchain-v4-compare.json"]=b'{}'
        index={"schema":"mpfree-private-audit-zip-index-v1","files":[{
            "name":k,"sha256":("0"*64 if corrupt and i==0 else
                                hashlib.sha256(b).hexdigest()),
            "bytes":len(b)}
            for i,(k,b) in enumerate(files.items())]}
        with zipfile.ZipFile(z,"w") as output:
            output.writestr("ZIP_INDEX.json",json.dumps(index))
            for k,b in files.items():output.writestr(k,b)
        return z
    def test_reuses_only_hash_verified_prior_sources(self):
        with tempfile.TemporaryDirectory() as d:
            z=self.make_test_zip(d)
            sources=module.verify_zip(z)
            self.assertEqual(len(sources),len(module.REQUIRED))
            self.assertIn("p3-coverage-v4.json",sources)
    def test_corrupt_index_fails(self):
        with tempfile.TemporaryDirectory() as d:
            z=self.make_test_zip(d,corrupt=True)
            with self.assertRaisesRegex(ValueError,"hash"):
                module.verify_zip(z)
    def test_refuses_symlink_as_archive(self):
        with tempfile.TemporaryDirectory() as d:
            z=self.make_test_zip(d)
            link=Path(d)/"link.zip"
            link.symlink_to(z)
            with self.assertRaisesRegex(ValueError,"regular"):
                module.require_file_regular(link)
    def test_all_v5_python_sources_parse(self):
        for rel in (
            "tools/mpc-one/run_bootchain_v5_followup.py",
            "tools/mpc-one/offline-bootchain-v5-elf-import.py",
            "tools/ghidra/MPFreeImportedELFAuditV5.py",
            "tools/mpc-one/offline-bootchain-v5-synthesis.py",
        ):
            file=ROOT/rel
            with self.subTest(rel=rel):
                ast.parse(file.read_text(encoding="utf-8"),filename=str(file))
    def test_full_runner_bash_syntax(self):
        p=subprocess.run(["bash","-n",str(ROOT/"tools/ghidra/run_wide_bootchain_audit.sh")],
                          capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr)

if __name__=="__main__":unittest.main()
