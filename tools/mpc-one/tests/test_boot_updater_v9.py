"""Offline-only MPFree V9 synthetic tests; never boots or contacts the MPC."""
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
def load(name,rel):
    spec=importlib.util.spec_from_file_location(name,ROOT/rel)
    mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
    return mod
V9=load('mpfree_v9_synthetic','tools/mpc-one/run_boot_updater_v9.py')

class V9StaticAndKernelTests(unittest.TestCase):
    def test_source_syntax(self):
        for name in ('tools/mpc-one/run_boot_updater_v9.py',
                     'tools/ghidra/MPFreeUpdaterFocusedV9.py'):
            with self.subTest(name=name):
                ast.parse((ROOT/name).read_text(),filename=name)
    def test_new_ghidra_has_no_module_level_loop_proxy_collision(self):
        t=ast.parse((ROOT/'tools/ghidra/MPFreeUpdaterFocusedV9.py').read_text())
        self.assertFalse(any(isinstance(x,(ast.For,ast.While)) for x in t.body))
        self.assertIn('mpfree_v9_run',[x.name for x in t.body if isinstance(x,ast.FunctionDef)])
    def test_kernel_config_disables_ti_keystone_not_virtio(self):
        data=(ROOT/'tools/mpc-one/build_qemu_arm_kernel.sh').read_text()
        for key in ('KEYSTONE_NAVIGATOR_QMSS','ARCH_KEYSTONE','SOC_TI'):
            self.assertIn('-d '+key,data)
        for key in ('VIRTIO_BLK','VIRTIO_MMIO','EXT4_FS','SERIAL_AMBA_PL011'):
            self.assertIn('-e '+key,data)
        p=subprocess.run(['bash','-n',str(ROOT/'tools/mpc-one/build_qemu_arm_kernel.sh')],
                         capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr)
    def test_prior_sha_reuse_checks_every_report(self):
        with tempfile.TemporaryDirectory() as td:
            d=Path(td);archive=d/'v8.zip';content={}
            for role,sha in V9.SRC_SHA.items():
                content['v8-ghidra-'+role+'.json']=json.dumps({
                    'schema':'mpfree-directed-boot-updater-v8',
                    'role':role,'source_sha256_prechecked_by_launcher':sha}).encode()
            for name in ('v8-stage-manifest.json','v8-updater-ramfs-inventory.json','v8-test.log'):
                content[name]=b'{}'
            rows=[{'name':name,'sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data)}
                  for name,data in content.items()]
            with zipfile.ZipFile(archive,'w') as z:
                z.writestr('ZIP_INDEX.json',json.dumps({'files':rows}))
                for name,data in content.items():z.writestr(name,data)
            out=V9.validate_prior(archive)
            self.assertEqual(out['files_verified'],len(rows))
            self.assertEqual(out['sha256'],hashlib.sha256(archive.read_bytes()).hexdigest())
            # Single modified member fails instead of silently becoming trusted.
            rows[0]['sha256']='0'*64
            with zipfile.ZipFile(archive,'w') as z:
                z.writestr('ZIP_INDEX.json',json.dumps({'files':rows}))
                for name,data in content.items():z.writestr(name,data)
            with self.assertRaisesRegex(ValueError,'SHA'):
                V9.validate_prior(archive)
    def test_report_requires_source_sha_and_role_even_if_ghidra_exit_zero(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/'v9-ghidra-spl.json'
            self.assertFalse(V9.verify_stage(path,'spl',0)['success'])
            path.write_text(json.dumps({'schema':'mpfree-v9-updater-backend-and-rom-static',
                                        'role':'spl','source_sha256_verified_offline':'wrong'}))
            self.assertFalse(V9.verify_stage(path,'spl',0)['success'])
            path.write_text(json.dumps({'schema':'mpfree-v9-updater-backend-and-rom-static',
                                        'role':'spl','source_sha256_verified_offline':V9.SRC_SHA['spl'],
                                        'recognized_functions':200,'decompiled':100}))
            self.assertTrue(V9.verify_stage(path,'spl',0)['success'])
            self.assertFalse(V9.verify_stage(path,'spl',1)['success'])
if __name__=='__main__':unittest.main()
