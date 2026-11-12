"""MPFree V10 offline synthetic regression checks (never contacts devices)."""
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
def load():
    spec=importlib.util.spec_from_file_location('mpfree_v10_test_module',
                  ROOT/'tools/mpc-one/run_boot_updater_v10.py')
    mod=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod
M=load()
class TestV10(unittest.TestCase):
    def test_python_syntax_and_scoped_ghidra(self):
        for path in ['tools/mpc-one/run_boot_updater_v10.py',
                     'tools/ghidra/MPFreeDirectedUpdaterV10.py']:
            ast.parse((ROOT/path).read_text(),filename=path)
        tree=ast.parse((ROOT/'tools/ghidra/MPFreeDirectedUpdaterV10.py').read_text())
        self.assertIn('mpfree_v10_run',[n.name for n in tree.body
                                       if isinstance(n,ast.FunctionDef)])
        self.assertFalse(any(isinstance(n,(ast.For,ast.While)) for n in tree.body))
    def test_qemu_can_discard_all_guest_writes(self):
        source=(ROOT/'tools/mpc-one/arm_qemu_lab.py').read_text()
        self.assertIn("--launch-ephemeral",source)
        self.assertIn("cmd.insert(1,'-snapshot')",source)
        self.assertIn("'-nic','none'",source)
        self.assertIn("'-monitor','none'",source)

    def test_stage_report_requires_schema_source_role(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/'v10-ghidra-spl.json'
            self.assertFalse(M.stage_result(p,'spl',0)['success'])
            payload={'schema':M.SCHEMA,'role':'spl',
                     'source_sha256_prechecked_by_launcher':M.SOURCES['spl'],
                     'boot_stage_memory_sha_verified':True,'decompiled':5}
            p.write_text(json.dumps(payload))
            self.assertTrue(M.stage_result(p,'spl',0)['success'])
            self.assertFalse(M.stage_result(p,'spl',1)['success'])
            payload['source_sha256_prechecked_by_launcher']='bad'
            p.write_text(json.dumps(payload))
            self.assertFalse(M.stage_result(p,'spl',0)['success'])
    def test_lineage_checks_archive_hashes_and_sources(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/'prior-v9.zip'
            data={f'v9-ghidra-{r}.json':json.dumps({
                'role':r,'source_sha256_verified_offline':s}).encode()
                for r,s in M.SOURCES.items()}
            for i in range(6):
                data[f'other-{i}.json']=b'{}'
            index=[{'name':n,'bytes':len(b),
                    'sha256':hashlib.sha256(b).hexdigest()} for n,b in data.items()]
            with zipfile.ZipFile(p,'w') as z:
                z.writestr('ZIP_INDEX.json',json.dumps({'files':index}))
                for n,b in data.items():z.writestr(n,b)
            result=M.lineage(p)
            self.assertEqual(result['verified_members'],len(index))
            index[2]['sha256']='0'*64
            with zipfile.ZipFile(p,'w') as z:
                z.writestr('ZIP_INDEX.json',json.dumps({'files':index}))
                for n,b in data.items():z.writestr(n,b)
            with self.assertRaisesRegex(ValueError,'SHA'):
                M.lineage(p)
if __name__=='__main__':unittest.main()
