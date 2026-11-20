"""Synthetic tests for single-file PRIVATE boot audit ZIP output.

No proprietary sources or MPC hardware are ever used.
"""
import ast
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[3]
MODULE = ROOT / "tools/mpc-one/package-private-boot-audit.py"
SPEC = importlib.util.spec_from_file_location("mpfree_private_boot_audit_zip", MODULE)
zipper = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(zipper)

class SingleZipArchiveTests(unittest.TestCase):
    def test_packages_all_reports_with_integrity_manifest(self):
        with tempfile.TemporaryDirectory(prefix="mpfree-test-audit-") as temp:
            path = Path(temp)/"bootchain-wide-test"
            path.mkdir()
            fixtures={"manifest.json":'{"schema":"test"}\n',
                      "p1-sram-ghidra.json":"{}\n",
                      "p3-ghidra.log":"mock ghidra log\n",
                      "firmware-3.10-container.txt":"test only\n",
                      "nested/forensics.json":"{}\n"}
            for name,data in fixtures.items():
                p=path/name;p.parent.mkdir(parents=True,exist_ok=True)
                p.write_text(data,encoding="utf-8")
            archive, index=zipper.package(path)
            self.assertEqual(len(index["files"]),len(fixtures))
            self.assertTrue(archive.is_file())
            with zipfile.ZipFile(archive) as z:
                self.assertIsNone(z.testzip())
                self.assertEqual(set(z.namelist()),set(fixtures)|{"ZIP_INDEX.json"})
                internal=json.loads(z.read("ZIP_INDEX.json"))
                self.assertEqual(len(internal["files"]),len(fixtures))
                for record in internal["files"]:
                    self.assertEqual(
                        hashlib.sha256(z.read(record["name"])).hexdigest(), record["sha256"])
            checksum=archive.with_suffix(".zip.sha256")
            self.assertIn(hashlib.sha256(archive.read_bytes()).hexdigest(),
                          checksum.read_text())

    def test_archive_must_not_include_backup_or_binary(self):
        with tempfile.TemporaryDirectory(prefix="mpfree-test-audit-") as temp:
            path=Path(temp)/"bootchain-wide-test"
            path.mkdir()
            (path/"owner-emmc.img").write_bytes(b"\0"*1024)
            with self.assertRaisesRegex(ValueError,"extension"):
                zipper.package(path)

    def test_archive_refuses_links(self):
        with tempfile.TemporaryDirectory(prefix="mpfree-test-audit-") as temp:
            path=Path(temp)/"bootchain-wide-test"
            path.mkdir()
            (path/"manifest.json").write_text("{}")
            (path/"alias.json").symlink_to(path/"manifest.json")
            with self.assertRaisesRegex(ValueError,"symlink"):
                zipper.package(path)

    def test_scripts_parse_without_live_tools(self):
        for rel in (
            "tools/ghidra/MPFreeBootChainForensicsPass2.py",
            "tools/mpc-one/offline-boot-version-compare.py",
            "tools/mpc-one/offline-spl-fdt.py",
            "tools/mpc-one/package-private-boot-audit.py",
            "tools/ghidra/MPFreeBootChainWideAudit.py",
        ):
            with self.subTest(path=rel):
                f=ROOT/rel;ast.parse(f.read_text(),filename=str(f))

    def test_bash_launcher_parses(self):
        f=ROOT/"tools/ghidra/run_wide_bootchain_audit.sh"
        p=subprocess.run(["bash","-n",str(f)],capture_output=True,text=True)
        self.assertEqual(p.returncode,0,p.stderr)

if __name__=="__main__":
    unittest.main()
