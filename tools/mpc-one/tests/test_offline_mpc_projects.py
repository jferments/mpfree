#!/usr/bin/env python3
"""Tests for offline-mpc-projects.py against a fake backup: a disk image with a
real GPT whose "data" partition is an ext4 file system made by mke2fs -d,
holding made-up projects (not Akai files) next to private files that must
never be copied.

Needs mke2fs and debugfs (e2fsprogs). They are skipped without them unless
MPFREE_REQUIRE_OFFLINE_TOOLS=1, which CI sets.
"""
import gzip
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "offline-mpc-projects.py")
sys.path.insert(0, HERE)
from test_offline_factory_kit import DEBUGFS, HAVE, MKE2FS, gpt_disk  # noqa: E402

FAKE_HEAD = b"FAKEHDR\n9.9.9.9\nTestObject\njson\nTestOS\n"
FILES = {
    "MPC Documents/Beat One.xpj": gzip.compress(FAKE_HEAD + b'{"data": {}}'),
    "MPC Documents/Beat One_[ProjectData]/kick.wav": b"RIFF" + b"k" * 3000,
    "MPC Documents/Beat One_[ProjectData]/sub/snare.wav": b"RIFF" + b"s" * 5000,
    "Old/Song.xpj": '<?xml version="1.0"?>\n<MPCVObject><Version><File_Version>1.0</File_Version>'
                    '<Application>Test</Application></Version></MPCVObject>\n',
    "Old/Song_[ProjectData]/Song.xal": "<x/>",
    "Projects/Lonely.xpj": b"\x00\x01\x02 not a project",
    "Arch.xpa": b"PK" + b"a" * 100,
    "etc/upper/secret.key": "PRIVATE",
    "mpfree/ssh/id_ed25519": "PRIVATE",
    "MPC Documents/loose.wav": b"RIFF" + b"l" * 10,
}
SECRETS = ("secret.key", "id_ed25519", "loose.wav")


@unittest.skipUnless(HAVE, "needs mke2fs and debugfs")
class MpcProjectsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.backups = os.path.join(self.tmp, "backups")
        self.work = os.path.join(self.tmp, "work")
        os.makedirs(self.work)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def ext4(self, files):
        src = tempfile.mkdtemp(dir=self.tmp)
        for rel, data in files.items():
            p = os.path.join(src, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as fh:
                fh.write(data if isinstance(data, bytes) else data.encode())
        img = os.path.join(self.tmp, "data.img")
        subprocess.run([MKE2FS, "-q", "-F", "-t", "ext4", "-d", src, img, "16M"], check=True)
        with open(img, "rb") as fh:
            return fh.read()

    def backup(self, files=FILES, part="data"):
        folder = os.path.join(self.backups, "mpc-one-emmc-20261109T233604Z-abc")
        os.makedirs(folder)
        image = os.path.join(folder, "mmcblk0.img")
        gpt_disk(image, [("rootfs", 2048, b"\x55" * 4096), ("content", 4096, b"\0" * 4096),
                         (part, 8192, self.ext4(files))])
        os.chmod(image, 0o400)
        return image

    def run_tool(self, *args):
        env = dict(os.environ, MPFREE_BACKUPS_DIR=self.backups, MPFREE_WORK_DIR=self.work)
        before = set(os.listdir(self.work))
        res = subprocess.run([sys.executable, TOOL, *args], capture_output=True, text=True, env=env, timeout=120)
        runs = sorted(d for d in set(os.listdir(self.work)) - before if d.startswith("mpc-projects-"))
        out = os.path.join(self.work, runs[-1]) if runs else None
        report = ""
        if out:
            with open(os.path.join(out, "report.txt")) as fh:
                report = fh.read()
        return res, out, report

    def sha(self, path):
        with open(path, "rb") as fh:
            return hashlib.sha256(fh.read()).hexdigest()

    def test_lists_projects_and_their_formats(self):
        image = self.backup()
        before = self.sha(image)
        res, out, report = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn("projects: 4", report)
        self.assertRegex(report, r"  1\. Arch\.xpa  \(\.xpa archive")
        self.assertRegex(report, r"  2\. MPC Documents/Beat One\.xpj  \(0\.0 MiB, 2 files in ProjectData, 2 WAV\)")
        self.assertIn("gzip (MPC 3 style); first lines: FAKEHDR | 9.9.9.9 | TestObject | json | TestOS", report)
        self.assertRegex(report, r"  3\. Old/Song\.xpj .*1 files in ProjectData, 0 WAV")
        self.assertIn("XML (MPC 2 style); File_Version 1.0, Application Test", report)
        self.assertRegex(report, r"  4\. Projects/Lonely\.xpj .*NO ProjectData folder")
        self.assertIn("unknown format; first bytes 00 01 02", report)
        self.assertIn("RESULT: listed only", report)
        self.assertEqual(sorted(os.listdir(out)), ["report.txt"])
        for s in SECRETS:
            self.assertNotIn(s, report)
        self.assertEqual(self.sha(image), before)

    def test_copies_only_the_chosen_projects(self):
        image = self.backup()
        before = self.sha(image)
        res, out, report = self.run_tool("--copy", "2,3")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn("copied 5 files", report)
        with zipfile.ZipFile(os.path.join(out, "mpc-projects.zip")) as z:
            names = sorted(z.namelist())
            self.assertEqual(names, sorted([
                "MPC Documents/Beat One.xpj", "MPC Documents/Beat One_[ProjectData]/kick.wav",
                "MPC Documents/Beat One_[ProjectData]/sub/snare.wav", "Old/Song.xpj",
                "Old/Song_[ProjectData]/Song.xal"]))
            for n in names:
                want = FILES[n]
                self.assertEqual(z.read(n), want if isinstance(want, bytes) else want.encode())
        sums = open(os.path.join(out, "SHA256SUMS")).read()
        self.assertEqual(len(sums.splitlines()), 5)
        self.assertIn(hashlib.sha256(FILES["Old/Song.xpj"].encode()).hexdigest() + "  Old/Song.xpj", sums)
        self.assertEqual(self.sha(image), before)

    def test_copy_all_never_takes_private_files(self):
        self.backup()
        res, out, report = self.run_tool("--copy", "all")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        copied = [os.path.relpath(os.path.join(d, f), out) for d, _, fs in os.walk(out) for f in fs]
        for s in SECRETS:
            self.assertFalse(any(s in c for c in copied), s)
        self.assertIn("projects/Arch.xpa", copied)
        self.assertIn("projects/Projects/Lonely.xpj", copied)

    def test_stops_cleanly(self):
        res, out, _ = self.run_tool()
        self.assertEqual(res.returncode, 1)
        self.assertIn("no backup folder", res.stdout)
        self.assertIsNone(out)
        self.backup()
        for bad, says in (("9", "no project numbered [9]"), ("x", "takes numbers"), ("0,1", "no project numbered [0]")):
            res, out, report = self.run_tool("--copy", bad)
            self.assertEqual(res.returncode, 1, bad)
            self.assertIn(says, report)
            self.assertFalse(os.path.exists(os.path.join(out, "mpc-projects.zip")))
        shutil.rmtree(self.backups)
        self.backup({"etc/a.txt": "x"})
        res, out, report = self.run_tool()
        self.assertEqual(res.returncode, 1)
        self.assertIn("no .xpj project or .xpa archive", report)
        shutil.rmtree(self.backups)
        self.backup(part="other")
        res, out, report = self.run_tool()
        self.assertEqual(res.returncode, 1)
        self.assertIn("no partition named data", report)


class StaticTests(unittest.TestCase):
    def test_reads_the_backup_only(self):
        with open(TOOL) as fh:
            src = fh.read()
        self.assertNotIn('"-w"', src)
        self.assertNotIn("'-w'", src)
        # It runs nothing itself: debugfs runs through offline-factory-kit.py's read-only Image.
        self.assertNotIn("subprocess", src)
        writes = [m.group(1) for line in src.splitlines() for m in [re.search(r'open\((.+), "(w|wb|xb|a|r\+b)"\)', line)] if m]
        self.assertEqual(sorted(writes), ['os.path.join(out_dir, "SHA256SUMS")', 'os.path.join(out_dir, "report.txt")'])
        for needle in ('"mount"', '"sudo"', "ssh ", "losetup", "rmtree", "os.remove", "os.unlink", "fallocate"):
            self.assertFalse(needle in src, needle)
        # Only debugfs dump writes copies, and only under out_dir.
        self.assertIn('cmds.append(f\'dump {fk.quoted(p)} "{dest}"\')', src)
        self.assertNotIn("rdump", src)


if __name__ == "__main__":
    unittest.main(verbosity=2)
