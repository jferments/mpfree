"""Synthetic tests: no vendor firmware or hardware required."""
import ast
import importlib.util
import struct
import unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]

def load(file,alias):
    spec=importlib.util.spec_from_file_location(alias,ROOT/'tools/mpc-one'/file)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module
DISPATCH=load('offline-bootchain-v7-elf-dispatch.py','mpfree_dispatch_test')
UNWIND=load('offline-bootchain-v7-unwind.py','mpfree_unwind_test')
SUMMARY=load('offline-bootchain-v7-synthesis.py','mpfree_v7_summary_test')

class V7Tests(unittest.TestCase):
    def test_arm_prel31_positive_and_negative(self):
        self.assertEqual(UNWIND.prel31(0x30,0x1000),0x1030)
        self.assertEqual(UNWIND.prel31((1<<31)-0x24,0x1000),0xfdc)

    def test_static_pie_command_table_pairs_name_and_handler(self):
        image=bytearray(0x1000)
        # File offsets match their virtual addresses in synthetic data.
        sec=[{'name':'.rodata','addr':0x100,'offset':0x100,'size':0x100,
              'flags':2,'type':1},
             {'name':'.text','addr':0x300,'offset':0x300,'size':0x100,
              'flags':6,'type':1},
             {'name':'.data','addr':0x500,'offset':0x500,'size':0x100,
              'flags':3,'type':1},
             {'name':'.rel.dyn','addr':0x700,'offset':0x700,'size':48,
              'flags':2,'type':9}]
        names=['show','modify','program-fuses']
        reloc={}
        for i,name in enumerate(names):
            pos=0x100+i*28
            image[pos:pos+len(name)+1]=name.encode()+b'\0'
            struct.pack_into('<II',image,0x500+i*12,pos,0x300+i*4)
            reloc[0x500+i*12]=pos
            reloc[0x504+i*12]=0x300+i*4
        tables=DISPATCH.map_records(image,sec,reloc)
        self.assertTrue(tables)
        self.assertEqual([e['name'] for e in tables[0]['entries']],names)
        self.assertEqual(tables[0]['entries'][2]['handler']['virtual_address'],'0x00000308')

    def test_rejects_partial_table_or_unbacked_pointers(self):
        image=bytearray(0x1000)
        sec=[{'name':'.data','addr':0x500,'offset':0x500,'size':0x40,'flags':3,'type':1},
             {'name':'.text','addr':0x300,'offset':0x300,'size':0x100,'flags':6,'type':1}]
        self.assertEqual(DISPATCH.map_records(image,sec,{}),[])

    def test_new_question_ledger(self):
        self.assertEqual(len(SUMMARY.CATALOG),32)
        self.assertIn('Q180',SUMMARY.CATALOG)
        self.assertIn('Q149',SUMMARY.CATALOG)

    def test_parse_all_scripts(self):
        for name in ('offline-bootchain-v7-elf-dispatch.py',
                     'offline-bootchain-v7-unwind.py',
                     'offline-bootchain-v7-reference-scan.py',
                     'offline-bootchain-v7-synthesis.py',
                     'run_bootchain_v7_followup.py'):
            p=ROOT/'tools/mpc-one'/name
            ast.parse(p.read_text(),filename=str(p))

if __name__=='__main__':unittest.main()