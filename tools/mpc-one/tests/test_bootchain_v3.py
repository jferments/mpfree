"""Synthetic, offline-only checks for the third bootloader evidence-graph pass.

These never open the real owner's backup or contact the MPC.
"""
import ast
import importlib.util
from pathlib import Path
import subprocess
import struct
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[3]
TOOLS=ROOT/"tools"/"mpc-one"

def load(filename,name):
    spec=importlib.util.spec_from_file_location(name,TOOLS/filename)
    obj=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj

class BootchainV3Tests(unittest.TestCase):
    def test_all_new_python_files_parse(self):
        for name in [
            "tools/ghidra/MPFreeBootChainEvidenceGraphV3.py",
            "tools/mpc-one/offline-embedded-fdt-crossstage.py",
            "tools/mpc-one/find-extracted-310-rootfs.py",
        ]:
            f=ROOT/name
            with self.subTest(name=name):
                ast.parse(f.read_text(encoding="utf-8"),filename=str(f))

    def test_bash_runner_parse(self):
        sh=ROOT/"tools/ghidra/run_wide_bootchain_audit.sh"
        p=subprocess.run(["bash","-n",str(sh)],capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr)

    def test_fdt_rejects_false_short_magic(self):
        m=load("offline-embedded-fdt-crossstage.py","mpfree_fdt_v3")
        short=b"head"+m.FDT+struct.pack(">I",0xffffffff)+b"tail"
        self.assertEqual(m.find_dtbs(short,"test"),[])

    def test_fdt_records_plausible_bounded_header(self):
        m=load("offline-embedded-fdt-crossstage.py","mpfree_fdt_v3b")
        image=b"A"*17+m.FDT+struct.pack(">I",40)+b"\0"*32
        # If dtc is installed it will reject a synthetic header;
        # either way, the offline bounds check should not crash.
        res=m.find_dtbs(image,"test")
        self.assertEqual(len(res),1)
        self.assertEqual(res[0]["offset_in_test"],17)
        self.assertEqual(res[0]["bytes"],40)

    def test_rootfs_file_validator_rejects_non_ext4(self):
        m=load("find-extracted-310-rootfs.py","mpfree_310_locator")
        with tempfile.TemporaryDirectory(prefix="mpfree-test-") as temp:
            f=Path(temp)/"fake.ext4"
            f.write_bytes(b"\0"*4096)
            self.assertFalse(m.is_regular_ext4(f))
            self.assertFalse(m.is_regular_ext4(Path(temp)/"missing.ext4"))

    def test_rootfs_file_validator_accepts_sparse_regular_image(self):
        m=load("find-extracted-310-rootfs.py","mpfree_310_locator2")
        with tempfile.TemporaryDirectory(prefix="mpfree-test-") as temp:
            f=Path(temp)/"synthetic.ext4"
            with f.open("wb") as stream:
                stream.truncate(110*1024*1024)
                stream.seek(1080)
                stream.write(b"\x53\xef")
            self.assertTrue(m.is_regular_ext4(f))
            # Never actually hand this fake image to the 3.10 comparer.
if __name__=="__main__":
    unittest.main()
