#!/usr/bin/env python3
"""Host self-tests: verified real V14/V15 evidence, ZIP safety, V16 plan."""
import importlib.util,hashlib,json,tempfile,unittest,zipfile
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('v16_evidence_tests',HERE/'evidence_v16.py')
e=importlib.util.module_from_spec(spec);spec.loader.exec_module(e)
spec2=importlib.util.spec_from_file_location('v16_runner_tests',HERE/'run_v16.py')
r=importlib.util.module_from_spec(spec2);spec2.loader.exec_module(r)

class V16(unittest.TestCase):
    def test_graph_routes_and_indirect_dont_count(self):
        f={'0x1':{'calls':[{'target_entries':['0x2']},{'target_entries':[]}]},
           '0x2':{'calls':[{'target_entries':['0x3']}]},'0x3':{'calls':[]}}
        graph=e.build_graph(f)
        self.assertEqual(e.route(graph,'0x1','0x3'),['0x1','0x2','0x3'])
        self.assertIsNone(e.route(graph,'0x3','0x1'))
        self.assertEqual(sum(map(len,graph.values())),2)

    def test_real_case_plan_spans_seven_function_families(self):
        tests=r.CASES
        self.assertEqual(len(tests),len(set(tests)))
        self.assertGreaterEqual(len(tests),100)
        self.assertEqual(len([x for x in tests if x.startswith('oem_mutate_')]),37)
        self.assertEqual(len([x for x in tests if x.startswith('oem_trunc_')]),38)
        for x in ('receive_clip','oem_exact','oem_shell_denied',
                  'classify_fdt','boot_callback','usb_exit_state','permission_0'):
            self.assertIn(x,tests)

    def test_archive_rejects_hidden_unindexed_entry(self):
        with tempfile.TemporaryDirectory() as name:
            p=Path(name)/'archive.zip'
            data=b'test'
            index={'files':[{'name':'entry.txt','bytes':len(data),
                             'sha256':hashlib.sha256(data).hexdigest()}]}
            with zipfile.ZipFile(p,'w') as z:
                z.writestr('ZIP_INDEX.json',json.dumps(index))
                z.writestr('entry.txt',data)
                z.writestr('secret.img',b'bad')
            with self.assertRaises(ValueError):e.load_verified_zip(p)

    def test_archive_sha_mismatch_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            p=Path(name)/'archive.zip'
            index={'files':[{'name':'entry.txt','bytes':4,'sha256':'0'*64}]}
            with zipfile.ZipFile(p,'w') as z:
                z.writestr('ZIP_INDEX.json',json.dumps(index))
                z.writestr('entry.txt',b'test')
            with self.assertRaises(ValueError):e.load_verified_zip(p)

    def test_offline_replay_against_user_v14_v15_when_present(self):
        files14=list(Path('/mnt/data').glob('*v14-*.zip'))
        files15=list(Path('/mnt/data').glob('*v15-*.zip'))
        if not files14 or not files15:self.skipTest('Private owner test archives are not present')
        result=e.evidence(e.load_verified_zip(files14[0]),e.load_verified_zip(files15[0]))
        self.assertEqual(result['summary']['questions'],85)
        self.assertEqual(len({x['id'] for x in result['questions']}),85)
        self.assertTrue(all(x['status'] in ('ANSWERED_STATIC','PARTIAL_STATIC',
                                           'PARTIAL_RUNTIME_ISOLATED','UNRESOLVED') for x in result['questions']))
        self.assertGreaterEqual(len(result['facts']),7)
        self.assertEqual(len(result['direct_paths']),9)
        self.assertEqual(result['summary']['actual_new_hardware_proofs'],0)

if __name__=='__main__':unittest.main(verbosity=2)