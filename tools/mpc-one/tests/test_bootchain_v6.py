"""Pure synthetic V6 tests: no connected MPC, no vendor ELF execution."""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[3]
def module(path,alias):
    spec=importlib.util.spec_from_file_location(alias,ROOT/path)
    x=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(x)
    return x

V6=module('tools/mpc-one/offline-bootchain-v6-investigate.py','mpfree_v6_analysis')
RELOC=module('tools/mpc-one/offline-bootchain-v6-elf-relocations.py','mpfree_v6_rel')
SCAN=module('tools/mpc-one/offline-bootchain-v6-startup-search.py','mpfree_v6_scan')
RUN=module('tools/mpc-one/run_bootchain_v6_followup.py','mpfree_v6_run')

class MPFreeV6Tests(unittest.TestCase):
    def test_questions_are_unique_and_broad(self):
        questions=V6.questions()
        self.assertEqual(len(questions),38)
        self.assertEqual({x[0] for x in questions},
                         {'Q%d'%i for i in range(111,149)})
        self.assertTrue(any('100%' in x[2] for x in questions))
        self.assertTrue(any('fuse' in x[2].lower() for x in questions))

    def test_indirect_callsite_assembly_and_gating(self):
        lines=[
            ' 7bc: cmp r0, #0',
            ' 7c0: beq 7e4 <fallback>',
            ' 7c8: bl 760 <SHA256_Final@plt>',
            ' 7e8: bl 698 <memcmp@plt>',
            ' 7f0: bne 884 <fatal>',
            ' 884: bl 6dc <errx@plt>',
        ]
        result=V6.classify_elf(lines,['SHA256_Final','memcmp','errx'])
        self.assertEqual(result['actual_imported_call_count'],3)
        self.assertEqual([x['callee'] for x in result['callsite_candidates']],
                         ['SHA256_Final','memcmp','errx'])

    def test_elf_rel32_with_thumb_code_pointer(self):
        image=bytearray(0x800)
        image[:4]=b'\x7fELF';image[4]=1;image[5]=1
        struct.pack_into('<H',image,18,40) # EM_ARM
        struct.pack_into('<I',image,32,0x600)
        struct.pack_into('<HHH',image,46,40,5,4)
        names=b'\0.text\0.data.rel.ro\0.rel.dyn\0.shstrtab\0'
        image[0x500:0x500+len(names)]=names
        # section name offset, type, flags, vaddr, file-offset, size
        def sh(i,n,t,fl,addr,offset,size):
            struct.pack_into('<IIIIIIIIII',image,0x600+i*40,
                             n,t,fl,addr,offset,size,0,0,4,0)
        sh(1,1,1,0x6,0x700,0x200,0x80)
        sh(2,names.find(b'.data'),1,0x3,0x2000,0x300,0x40)
        sh(3,names.find(b'.rel.dyn'),9,0,0x2500,0x400,8)
        sh(4,names.find(b'.shstrtab'),3,0,0,0x500,len(names))
        struct.pack_into('<I',image,0x300,0x709)  # main|Thumb
        struct.pack_into('<II',image,0x400,0x2000,23)  # R_ARM_RELATIVE
        c=RELOC.candidates(image)
        self.assertEqual(len(c),1)
        self.assertEqual(c[0]['candidate_vaddr'],'0x00000708')
        self.assertTrue(c[0]['thumb'])

    def test_strict_regular_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'p';p.write_bytes(b'x')
            link=Path(tmp)/'l';link.symlink_to(p)
            with self.assertRaisesRegex(ValueError,'regular'):
                RUN.verified_zip(link)

    def test_debugfs_ls_p_parses_filenames_not_inode_modes(self):
        from unittest.mock import patch
        listing=(b"/2/040755/0/0/./ /\n"
                 b"/3/040755/0/0/../ /\n"
                 b"/123/100644/0/0/az0x-loadpin-trustpoints.service/283/\n"
                 b"/124/100644/0/0/example.conf/53/\n")
        with patch.object(SCAN,"query",return_value=(listing,b"",0)):
            names=SCAN.listdir(Path("/tmp/fake-readonly.ext4"),"/usr/lib/systemd/system")
        self.assertIn("az0x-loadpin-trustpoints.service",names)
        self.assertIn("example.conf",names)
        self.assertNotIn("100644",names)

    def test_startup_query_no_shell_execution(self):
        self.assertIn('az01config',SCAN.KEYWORDS['fuse_programming'].pattern.decode())
        self.assertTrue(SCAN.PRIVATE.search(b'password=xxxx'))
        self.assertFalse(SCAN.PRIVATE.search(b'boot_targets=mmc0 mmc1'))

    def test_v6_script_python_and_shell_syntax(self):
        import ast
        for rel in [
            'tools/mpc-one/offline-bootchain-v6-investigate.py',
            'tools/mpc-one/offline-bootchain-v6-elf-relocations.py',
            'tools/mpc-one/offline-bootchain-v6-startup-search.py',
            'tools/mpc-one/run_bootchain_v6_followup.py',
            'tools/ghidra/MPFreeELFMainRecoveryV6.py',
        ]:
            src=(ROOT/rel).read_text()
            with self.subTest(file=rel):ast.parse(src,filename=rel)
        proc=subprocess.run(['bash','-n',str(ROOT/'tools/ghidra/run_wide_bootchain_audit.sh')],
                            capture_output=True,text=True)
        self.assertEqual(proc.returncode,0,proc.stderr)

if __name__=='__main__':unittest.main()