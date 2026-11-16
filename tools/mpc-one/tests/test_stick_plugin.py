#!/usr/bin/env python3
"""Tests for stick-plugin.py with a fake lsblk, udisksctl and sudo, and a folder for partition 4. No device."""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "stick-plugin.py")
URL = "https://127.0.0.1:9/Fake-Voice-1.0-mpc-armv7.zip"
MEMBERS = {
    "Fake-Voice-1.0/INSTALL.md": b"read me",
    "Fake-Voice-1.0/install.sh": b"#!/bin/sh\nexit 1\n",
    "Fake-Voice-1.0/portable/fake - VST - [SYN] Fake Voice/fake.so": b"\x7fELF" + b"\0" * 64,
    "Fake-Voice-1.0/portable/fake - VST - [SYN] Fake Voice/mpc-plugin.json": b"{}",
}


def disk(name, part_label="mpfree-test-data", label="MPFREE-DATA", fstype="ext4", tran="usb", rm=True,
         mountpoint=None):
    return {"name": name, "path": f"/dev/{name}", "type": "disk", "tran": tran, "rm": rm,
            "children": [{"name": name + "4", "path": f"/dev/{name}4", "type": "part", "partlabel": part_label,
                          "label": label, "fstype": fstype, "mountpoint": mountpoint}]}


class StickPlugin(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = self.tmp.name
        self.work, self.mnt, self.bin = (os.path.join(t, d) for d in ("work", "media/jesse/MPFREE-DATA", "bin"))
        for d in (self.work, self.mnt, self.bin):
            os.makedirs(d)
        self.log = os.path.join(t, "calls.log")
        self.disks = os.path.join(t, "lsblk.json")
        self.write_disks([disk("sdz")])
        self.zip = self.make_zip(os.path.join(t, "Fake-Voice-1.0-mpc-armv7.zip"), MEMBERS)
        os.makedirs(os.path.join(self.work, "plugins"))
        self.cached = os.path.join(self.work, "plugins", "Fake-Voice-1.0-mpc-armv7.zip")
        with open(self.zip, "rb") as a, open(self.cached, "wb") as b:
            b.write(a.read())
        self.catalog = os.path.join(t, "catalog.txt")
        self.write_catalog(self.sha(self.zip))
        self.fake("lsblk", f'echo "lsblk $*" >> {self.log}; cat {self.disks}')
        self.fake("udisksctl", f'echo "udisksctl $*" >> {self.log}\n'
                               f'if [ "$1" = mount ]; then echo "Mounted $3 at {self.mnt}."; fi')
        self.fake("sudo", f'echo "sudo $1" >> {self.log}; exec "$@"')

    def tearDown(self):
        self.tmp.cleanup()

    def fake(self, name, body):
        path = os.path.join(self.bin, name)
        with open(path, "w") as fh:
            fh.write("#!/bin/sh\n" + body + "\n")
        os.chmod(path, 0o755)

    def write_disks(self, disks):
        with open(self.disks, "w") as fh:
            json.dump({"blockdevices": disks}, fh)

    def write_catalog(self, sha):
        with open(self.catalog, "w") as fh:
            fh.write("# id kind sha256 url\n")
            fh.write(f"fake-voice   instrument  {sha}  {URL}\n")

    @staticmethod
    def make_zip(path, members):
        with zipfile.ZipFile(path, "w") as z:
            for n, data in members.items():
                z.writestr(n, data)
        return path

    @staticmethod
    def sha(path):
        with open(path, "rb") as fh:
            return hashlib.sha256(fh.read()).hexdigest()

    def run_tool(self, *args):
        env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"], MPFREE_WORK_DIR=self.work,
                   MPFREE_CATALOG=self.catalog)
        res = subprocess.run([sys.executable, TOOL, *args], capture_output=True, text=True, env=env, timeout=60)
        return res.returncode, res.stdout + res.stderr

    def calls(self):
        if not os.path.exists(self.log):
            return ""
        with open(self.log) as fh:
            return fh.read()

    def test_add_unzips_the_release_into_files_plugins_and_unmounts(self):
        rc, out = self.run_tool("add", "fake-voice")
        self.assertEqual(rc, 0, out)
        self.assertIn("SHA-256 matches", out)
        self.assertIn("partition 4: /dev/sdz4, mounted at", out)
        self.assertIn("files/plugins/Fake-Voice-1.0: 4 of the release's 4 files", out)
        self.assertIn("PLUGIN-OK: fake-voice (instrument) is in files/plugins/Fake-Voice-1.0", out)
        so = os.path.join(self.mnt, "files/plugins/Fake-Voice-1.0/portable/fake - VST - [SYN] Fake Voice/fake.so")
        self.assertTrue(os.path.isfile(so))
        self.assertIn("udisksctl unmount -b /dev/sdz4", self.calls())
        # Only files/plugins on partition 4 changed.
        self.assertEqual(sorted(os.listdir(self.mnt)), ["files"])
        self.assertEqual(os.listdir(os.path.join(self.mnt, "files")), ["plugins"])

    def test_a_second_add_writes_nothing_and_remove_takes_only_its_folder(self):
        self.assertEqual(self.run_tool("add", "fake-voice")[0], 0)
        other = os.path.join(self.mnt, "files/plugins/Other-1.0")
        os.makedirs(other)
        rc, out = self.run_tool("add", "fake-voice")
        self.assertEqual(rc, 0, out)
        self.assertIn("already on the stick; nothing written", out)
        rc, out = self.run_tool("remove", "fake-voice")
        self.assertEqual(rc, 0, out)
        self.assertIn("REMOVE-OK", out)
        self.assertFalse(os.path.exists(os.path.join(self.mnt, "files/plugins/Fake-Voice-1.0")))
        self.assertTrue(os.path.isdir(other))
        rc, out = self.run_tool("remove", "fake-voice")
        self.assertEqual(rc, 0, out)
        self.assertIn("not on the stick; nothing to remove", out)

    def test_as_zip_copies_the_zip_itself(self):
        rc, out = self.run_tool("add", "fake-voice", "--as-zip")
        self.assertEqual(rc, 0, out)
        target = os.path.join(self.mnt, "files/plugins/Fake-Voice-1.0-mpc-armv7.zip")
        self.assertEqual(self.sha(target), self.sha(self.zip))
        self.assertFalse(os.path.exists(os.path.join(self.mnt, "files/plugins/Fake-Voice-1.0")))
        rc, out = self.run_tool("remove", "fake-voice", "--as-zip")
        self.assertEqual(rc, 0, out)
        self.assertFalse(os.path.exists(target))

    def test_a_partition_the_desktop_mounted_is_used_where_it_is(self):
        self.write_disks([disk("sdz", mountpoint=self.mnt)])
        rc, out = self.run_tool("add", "fake-voice")
        self.assertEqual(rc, 0, out)
        self.assertIn("(by the desktop)", out)
        self.assertNotIn("udisksctl mount", self.calls())
        self.assertIn("udisksctl unmount", self.calls())

    def test_only_one_mpfree_stick_on_usb_qualifies(self):
        self.write_disks([disk("sdz"), disk("sdy")])
        rc, out = self.run_tool("add", "fake-voice")
        self.assertEqual(rc, 1)
        self.assertIn("more than one MPFree stick", out)
        self.write_disks([disk("sda", tran="sata", rm=False), disk("sdb", label="OTHER"),
                          disk("sdc", part_label="other", fstype="vfat")])
        rc, out = self.run_tool("add", "fake-voice")
        self.assertEqual(rc, 1)
        self.assertIn("not the stick: /dev/sda4: disk /dev/sda is not a removable USB disk", out)
        self.assertIn("STOP: no MPFree stick", out)
        self.assertNotIn("sudo", self.calls())
        self.assertNotIn("udisksctl", self.calls())

    def test_a_release_that_is_not_the_pinned_one_writes_nothing(self):
        self.write_catalog("0" * 64)
        rc, out = self.run_tool("add", "fake-voice")
        self.assertEqual(rc, 1)
        self.assertIn("STOP: the download failed", out)
        self.assertNotIn("sudo", self.calls())
        rc, out = self.run_tool("add", "no-such-plugin")
        self.assertEqual(rc, 1)
        self.assertIn("is not in", out)

    def test_a_member_outside_one_top_folder_writes_nothing(self):
        bad = self.make_zip(self.cached, {"Fake-Voice-1.0/a": b"x", "../evil": b"x"})
        self.write_catalog(self.sha(bad))
        rc, out = self.run_tool("add", "fake-voice")
        self.assertEqual(rc, 1)
        self.assertIn("a member outside one top folder", out)
        self.assertNotIn("sudo", self.calls())
        two = self.make_zip(self.cached, {"A/a": b"x", "B/b": b"x"})
        self.write_catalog(self.sha(two))
        rc, out = self.run_tool("add", "fake-voice")
        self.assertEqual(rc, 1)
        self.assertIn("2 top folders", out)

    def test_usage(self):
        rc, out = self.run_tool("add")
        self.assertEqual(rc, 2)
        self.assertIn("stick-plugin.py add mono-voice", out)

    def test_the_real_catalog_names_the_rounds_releases(self):
        sys.path.insert(0, os.path.dirname(HERE))
        import importlib.util
        spec = importlib.util.spec_from_file_location("stick_plugin", TOOL)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        os.environ.pop("MPFREE_CATALOG", None)
        kind, sha, url = mod.release("mono-voice")
        self.assertEqual((kind, sha), ("instrument", "c9263c1c0389762de82ac376cca020c675487be4c7c7c5d582e33018c5570b9d"))
        self.assertTrue(url.endswith("/SYN-Mono-Voice-1.0.2-mpc-armv7.zip"))
        self.assertEqual(mod.release("tidal-verb")[0], "effect")


if __name__ == "__main__":
    unittest.main()
