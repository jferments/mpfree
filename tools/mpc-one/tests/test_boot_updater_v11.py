"""V11 offline-only tests: source SHA, ELF table alignment, no firmware exec."""
import ast
import importlib.util
from pathlib import Path
import subprocess
import unittest
ROOT=Path(__file__).resolve().parents[3]
def module(rel,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/rel)
    obj=importlib.util.module_from_spec(spec);spec.loader.exec_module(obj)
    return obj
T=module('tools/mpc-one/offline-bootchain-v11-backend-tables.py','mpfree_v11_tbl_synthetic')
V11=module('tools/mpc-one/run_boot_updater_v11.py','mpfree_v11_runner_synthetic')
class Tests(unittest.TestCase):
    def test_ghidra_scoped_and_runner_syntax(self):
        for path in ('tools/ghidra/MPFreeBackendSelectorV11.py',
                     'tools/mpc-one/offline-bootchain-v11-backend-tables.py',
                     'tools/mpc-one/run_boot_updater_v11.py'):
            tree=ast.parse((ROOT/path).read_text(),filename=path)
            if path.endswith('.py') and '/ghidra/' in path:
                self.assertFalse(any(isinstance(n,(ast.For,ast.While)) for n in tree.body))
                self.assertIn('mpfree_v11_entrypoint',
                    [n.name for n in tree.body if isinstance(n,ast.FunctionDef)])
    def test_offline_candidate_alignment_uses_original_elf_vaddr(self):
        base=0x12a74
        mapping={base:2,base+4:0,
                 base+8:0x54b8,
                 base+0x18:0x5bec,
                 base+0x1c:0x6c14,
                 base+0x20:0x6dbc,
                 base+0x2c:0x6028,
                 base+0x34:0x5da8}
        pointers={where:{'target':where_value,'thumb':False}
                  for where,where_value in mapping.items()
                  if where not in (base,base+4)}
        found=T.candidate_tables(pointers,mapping.get)
        match=[f for f in found if f['base_vaddr']==f'0x{base:08x}']
        self.assertEqual(len(match),1)
        self.assertEqual(match[0]['matched_callback_count'],6)
        self.assertEqual(match[0]['callbacks']['write_bootloader'],'0x00005bec')
        self.assertEqual(match[0]['callbacks']['write_partition'],'0x00006c14')
    def test_rockusb_like_candidate_allows_missing_stream(self):
        base=0x12bd8
        m={base:1,base+4:0,base+8:0xc73c,base+0x18:0xc49c,
           base+0x1c:0xc3fc,base+0x34:0xc8c8}
        ptr={k:{'target':v} for k,v in m.items() if k>=base+8}
        found=T.candidate_tables(ptr,m.get)
        table=next(f for f in found if int(f['base_vaddr'],16)==base)
        self.assertEqual(table['matched_callback_count'],4)
        self.assertNotIn('stream_partition',table['callbacks'])
    def test_requires_verified_v10_prior_and_two_elves(self):
        txt=(ROOT/'tools/mpc-one/run_boot_updater_v11.py').read_text()
        for string in ('--prior-v10','check_previous','verify_source',
                       'mpfree-v11-private-updater-backend-audit'):
            self.assertIn(string,txt)
        self.assertIn('MPFreeBackendSelectorV11.py',txt)
        self.assertEqual(V11.SOURCES if hasattr(V11,'SOURCES') else V11.SHA['spl'],
                         V11.SHA['spl'])
if __name__=='__main__':unittest.main()
