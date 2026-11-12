#!/usr/bin/env python3
"""Host-only V17 preflight, no owner firmware/USB/VM access."""
import hashlib,importlib.util,json,struct,tempfile,unittest,zipfile
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location("mpfree_v17_runner",HERE/"run_v17.py")
runner=importlib.util.module_from_spec(spec);spec.loader.exec_module(runner)
spec2=importlib.util.spec_from_file_location("mpfree_v17_dtb",HERE/"offline_boot_assets_v17.py")
dtb=importlib.util.module_from_spec(spec2);spec2.loader.exec_module(dtb)

class V17ResearchSelfTests(unittest.TestCase):
    def test_dynamic_cases_are_many_and_unique(self):
        cases=runner.tested_cases()
        self.assertGreater(len(cases),1100)
        self.assertEqual(len(cases),len(set(cases)))
        for essential in ("v17_oem_shell_locked","v17_oem_shell_unlocked",
                          "v17_oem_env_locked","v17_oem_env_unlocked",
                          "v17_boot_completion","v17_download_70000000",
                          "v17_download_70000001","v17_receive_4096_4096",
                          "v17_receive_8192_1","v17_receive_0_0"):
            self.assertIn(essential,cases)

    def test_memory_model_does_not_claim_hardware_proof(self):
        crossings=dtb.ranges({})
        self.assertGreaterEqual(len(crossings),12)
        self.assertFalse(any(v["proof_of_physical_overlap"] for v in crossings))
        self.assertTrue(any(x["would_cross_anchor"] for x in crossings))
        self.assertTrue(any(not x["would_cross_anchor"] for x in crossings))

    def test_full_dtb_parse_and_diff(self):
        def pad(b):return b+b"\0"*((-len(b))%4)
        props=b"compatible\0"
        def node(name):return struct.pack(">I",1)+pad(name.encode()+b"\0")
        def prop(value):
            return struct.pack(">III",3,len(value),0)+pad(value)
        structblk=node("")+prop(b"rockchip,rk3288\0")+struct.pack(">II",2,9)
        hdr=struct.pack(">10I",0xd00dfeed,40+16+len(structblk)+len(props),
            56,56+len(structblk),40,17,16,0,len(props),len(structblk))
        parsed=dtb.parse_dtb(hdr+bytes(16)+structblk+props)
        self.assertEqual(parsed["node_count"],1)
        self.assertEqual(parsed["property_count"],1)
        self.assertEqual(parsed["nodes"]["/"]["compatible"]["value"],
                         ["rockchip,rk3288"])
        changed=json.loads(json.dumps(parsed))
        changed["nodes"]["/"]["compatible"]["value"]=["rockchip,rk3288-c"]
        self.assertEqual(len(dtb.difference(parsed,changed)),1)
        blob=bytearray(hdr+bytes(16)+structblk+props);blob[0]=0
        with self.assertRaises(ValueError):dtb.parse_dtb(bytes(blob))

    def test_package_verifies_every_written_member(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/"private-result";p.mkdir()
            (p/"proof.json").write_text(json.dumps({"real_vendor":False}))
            (p/"detail.jsonl").write_text(json.dumps({"function":"0x00100000"})+"\n")
            zip_path,n=runner.package(p)
            self.assertEqual(n,2)
            data=runner.load_zip(zip_path)
            self.assertIn("proof.json",data)
            self.assertIn("detail.jsonl",data)
            self.assertEqual(runner.digest(zip_path),zip_path.with_suffix(".zip.sha256").read_text().split()[0])

    def test_archive_rejects_invalid_hash(self):
        with tempfile.TemporaryDirectory() as td:
            a=Path(td)/"bad.zip"
            with zipfile.ZipFile(a,"w") as z:
                z.writestr("ZIP_INDEX.json",json.dumps({"files":[
                    {"name":"sample.json","bytes":3,"sha256":"0"*64},
                    {"name":"another.json","bytes":1,"sha256":"0"*64},
                    {"name":"third.json","bytes":1,"sha256":"0"*64},
                    {"name":"fourth.json","bytes":1,"sha256":"0"*64},
                    {"name":"fifth.json","bytes":1,"sha256":"0"*64}]}))
                for name,blob in (("sample.json",b"foo"),("another.json",b"a"),
                                  ("third.json",b"b"),("fourth.json",b"c"),
                                  ("fifth.json",b"d")):
                    z.writestr(name,blob)
            with self.assertRaises(ValueError):runner.load_zip(a)

    def test_all_major_stage_names_are_present(self):
        source=(HERE/"run_v17.py").read_text()
        for name in ("deep_p1","deep_p3","updater_sources",
                     "ghidra_updater","offline_boot","real_vendor_guest",
                     "evidence_ledger","ThreadPoolExecutor"):
            self.assertIn(name,source)
        self.assertNotIn("fastboot flash ",source)
        self.assertNotIn("fastboot erase ",source)

if __name__=="__main__":
    unittest.main(verbosity=2)
