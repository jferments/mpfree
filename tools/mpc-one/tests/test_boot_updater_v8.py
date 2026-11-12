"""Synthetic checks: no source image, live device or Akai binary executed."""
import ast
import importlib.util
import json
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[3]
def load(path,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/path)
    obj=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj
LAB=load('tools/mpc-one/arm_qemu_lab.py','mpfree_v8_lab')
AUDIT=load('tools/mpc-one/run_boot_updater_v8.py','mpfree_v8_updater')
class MPFreeV8Tests(unittest.TestCase):
    def test_sources_parse(self):
        for name in ('tools/ghidra/MPFreeDirectedBootUpdaterV8.py',
                     'tools/mpc-one/run_boot_updater_v8.py',
                     'tools/mpc-one/arm_qemu_lab.py'):
            with self.subTest(name=name):
                f=ROOT/name
                ast.parse(f.read_text(encoding='utf8'),filename=str(f))
    def test_ghidra_pyghidra_globals_are_locally_scoped(self):
        # Ghidra 12.2 headless maps script-level assignments into a Java
        # attribute proxy; for-loop targets may collide with its properties.
        g=ROOT/'tools/ghidra/MPFreeDirectedBootUpdaterV8.py'
        tree=ast.parse(g.read_text(encoding='utf8'),filename=str(g))
        names=[n.name for n in tree.body if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef))]
        self.assertIn('mpfree_v8_entrypoint',names)
        self.assertFalse(any(isinstance(n,(ast.For,ast.While)) for n in tree.body))
        self.assertIn('mpfree_v8_entrypoint()',g.read_text(encoding='utf8'))

    def test_ramfs_includes_full_path_inventory(self):
        source=(ROOT/'tools/mpc-one/run_boot_updater_v8.py').read_text()
        self.assertIn("result['all_paths']",source)
        self.assertIn("result['total_listed_paths']",source)
        self.assertIn('inspect_paths[:120]',source)

    def test_kernel_builder_bash_valid(self):
        f=ROOT/'tools/mpc-one/build_qemu_arm_kernel.sh'
        p=subprocess.run(['bash','-n',str(f)],capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr)
    def test_reject_symlinks(self):
        with tempfile.TemporaryDirectory() as folder:
            f=Path(folder)/'root.img'
            with f.open('wb') as stream:
                stream.truncate(25*1024*1024)
                stream.seek(1080)
                stream.write(b'\x53\xef')
            link=Path(folder)/'link.img';link.symlink_to(f)
            self.assertEqual(LAB.ext4(f),f)
            with self.assertRaises(ValueError):LAB.ext4(link)
            with self.assertRaises(ValueError):AUDIT.regular(link)
    def test_ghidra_roles_safety(self):
        g=(ROOT/'tools/ghidra/MPFreeDirectedBootUpdaterV8.py').read_text()
        self.assertIn("'spl','uboot','az01-update','libaz01-prog','ramfs-updater'",g)
        self.assertIn('never commit',g.lower())
    def test_vendor_paths_fixed(self):
        self.assertEqual(AUDIT.FILES['az01-update'],'/usr/sbin/az01-update')
        self.assertTrue(AUDIT.FILES['libaz01-prog'].endswith('.so.1.3'))
        self.assertEqual(AUDIT.P3_SHA,
                         'b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a')
    def test_vm_commands_not_networked(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            base=root/'base.ext4'
            base.write_bytes(b'\0'*1000)
            kernel=root/'kernel';kernel.write_bytes(b'kernel')
            overlay=root/'overlay.qcow2';overlay.write_bytes(b'qcow2')
            with patch.object(LAB,'verify_overlay',return_value={}):
                with patch.object(LAB,'OVERLAY',overlay):
                    cmd=LAB.qemu_args(kernel)
            self.assertIn('-nic',cmd)
            self.assertIn('none',cmd)
            self.assertIn('init=/bin/sh', ' '.join(cmd))
            self.assertNotIn('-device usb-host',' '.join(cmd))
if __name__=='__main__':unittest.main()