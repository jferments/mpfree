#!/usr/bin/env python3
"""Tests for read-stick-logs.py against a fake /dev and /sys. No real device is opened.

MPFREE_TEST_ROOT makes the tool look for /dev and /sys under a temporary
folder and skip sudo, so the "partition" here is a plain file holding an ext4
filesystem made with mke2fs. Needs e2fsprogs; set MPFREE_REQUIRE_OFFLINE_TOOLS=1
to fail instead of skip when it is missing.
"""
import glob
import hashlib
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "read-stick-logs.py")
BUILDER = os.path.join(os.path.dirname(HERE), "offline-kexec-stick.py")
ROOT_UUID = "0f0e0d0c-0b0a-4908-8706-050403020100"
LOG_UUID = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d"
BOOT = "boot-11111111-2222-4333-8444-555555555555"
LOG_SIZE = 32 << 20
HAVE_TOOLS = all(shutil.which(t) for t in ("mke2fs", "debugfs", "e2fsck"))
if not HAVE_TOOLS and os.environ.get("MPFREE_REQUIRE_OFFLINE_TOOLS") == "1":
    raise SystemExit("e2fsprogs is required (MPFREE_REQUIRE_OFFLINE_TOOLS=1)")


def write(path, data=""):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)


def digest(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


@unittest.skipUnless(HAVE_TOOLS, "needs e2fsprogs (mke2fs, debugfs, e2fsck)")
class ReaderTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="mpfree-stick-logs-test-")
        self.work = os.path.join(self.root, "work")
        build = os.path.join(self.work, "kexec-stick-20261110T000000Z-abc")
        write(os.path.join(build, "stick.img"), b"\0" * 1024)
        write(os.path.join(build, "report.txt"),
              f"stick.img: 1 bytes (1 MiB); partition 1 root PARTUUID {ROOT_UUID}, partition 2 logs PARTUUID "
              f"{LOG_UUID}\n")
        self.logs = {
            "cmdline.txt": "root=PARTUUID=x ro rootwait=60\n",
            "failed-units.txt": "0 loaded units listed.\n",
            "network.txt": "2: eth0: <UP>\n    link/ether 02:11:22:AA:bb:cc brd ff:ff:ff:ff:ff:ff\n",
            "dmesg.txt": "usb 2-1: SerialNumber: ABC123XYZ\nmpfree-test: blocked /usr/sbin/az01-update\n",
            "journal.txt": "[    5.0] mpc systemd[1]: Started sshd.\n",
            "zzz-extra.txt": "extra\n",
        }

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def make_fs(self, path, logs):
        src = os.path.join(self.root, "src")
        shutil.rmtree(src, ignore_errors=True)
        os.makedirs(src)
        for boot, files in logs.items():
            for name, text in files.items():
                write(os.path.join(src, boot, name), text)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        res = subprocess.run(["mke2fs", "-q", "-F", "-t", "ext4", "-L", "MPFREE-LOG", "-E", "root_owner=0:0",
                              "-d", src, path, f"{LOG_SIZE // 1024}k"], stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, text=True)
        self.assertEqual(res.returncode, 0, res.stdout)

    def plug(self, name="sdc2", usb=True, sectors=LOG_SIZE // 512, logs=None):
        """A fake partition /dev/<name> holding the log filesystem, with its sysfs entry and PARTUUID link."""
        dev = os.path.join(self.root, "dev", name)
        self.make_fs(dev, {BOOT: self.logs} if logs is None else logs)
        bus = "usb2/2-1/2-1:1.0/host6/target6:0:0/6:0:0:0" if usb else "0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0"
        disk = name.rstrip("0123456789")
        real = os.path.join(self.root, "sys/devices/pci0000:00", bus, "block", disk, name)
        write(os.path.join(real, "size"), f"{sectors}\n")
        os.makedirs(os.path.join(self.root, "sys/class/block"), exist_ok=True)
        os.symlink(real, os.path.join(self.root, "sys/class/block", name))
        os.makedirs(os.path.join(self.root, "dev/disk/by-partuuid"), exist_ok=True)
        os.symlink(f"../../{name}", os.path.join(self.root, "dev/disk/by-partuuid", LOG_UUID))
        return dev

    def reader(self, *args):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "MPFREE_TEST_ROOT": self.root,
               "MPFREE_WORK_DIR": self.work, "HOME": self.root}
        return subprocess.run([sys.executable, "-I", TOOL, *args], env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)

    def outputs(self):
        return glob.glob(os.path.join(self.work, "stick-logs-*"))

    def report(self):
        folders = self.outputs()
        self.assertEqual(len(folders), 1, folders)
        with open(os.path.join(folders[0], "report.txt")) as fh:
            return fh.read()

    def test_reads_every_log_and_leaves_the_stick_alone(self):
        dev = self.plug()
        before = digest(dev)
        res = self.reader()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(digest(dev), before, "the partition is only read")
        self.assertIn("found: /dev/sdc2 on USB, 32 MiB", res.stdout)
        self.assertIn("test boots found: 1", res.stdout)
        self.assertIn(f"{BOOT}/failed-units.txt:\n0 loaded units listed.", res.stdout)
        report = self.report()
        for name, text in self.logs.items():
            self.assertIn(f"===== {BOOT}/{name} ({len(text)} bytes) =====\n", report)
        heads = [line for line in report.splitlines() if line.startswith("===== ")]
        self.assertEqual([h.split("/")[1].split()[0] for h in heads],
                         ["cmdline.txt", "failed-units.txt", "network.txt", "dmesg.txt", "journal.txt",
                          "zzz-extra.txt"])
        self.assertIn("mpfree-test: blocked /usr/sbin/az01-update", report)
        self.assertIn("Started sshd.", report)
        self.assertNotIn("02:11:22:AA:bb:cc", report)
        self.assertIn("link/ether xx:xx:xx:xx:xx:xx", report)
        self.assertNotIn("ABC123XYZ", report)
        self.assertIn("SerialNumber: (removed)", report)
        self.assertIn("check of the copy (e2fsck -fn): exit code 0", report)
        folder = self.outputs()[0]
        self.assertEqual(digest(os.path.join(folder, "log-partition.img")), before)
        self.assertEqual(oct(os.stat(folder).st_mode & 0o777), "0o700")

    def test_two_boots_and_no_boots(self):
        second = "boot-99999999-2222-4333-8444-555555555555"
        self.plug(logs={BOOT: self.logs, second: {"cmdline.txt": "second\n"}})
        res = self.reader()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("test boots found: 2", res.stdout)
        self.assertIn(f"===== {second}/cmdline.txt (7 bytes) =====\nsecond", self.report())

    def test_no_logs_is_reported(self):
        self.plug(logs={})
        res = self.reader()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("No test boot left logs", res.stdout)
        self.assertIn("test boots found: 0", self.report())

    def test_stops_before_reading(self):
        res = self.reader()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("not on this computer", res.stdout)
        for kw, reason in (({"usb": False}, "not on USB"), ({"sectors": 2048}, "not the log partition's"),
                           ({"name": "sdc1"}, "not partition 2 of a USB disk")):
            shutil.rmtree(os.path.join(self.root, "dev"), ignore_errors=True)
            shutil.rmtree(os.path.join(self.root, "sys"), ignore_errors=True)
            self.plug(**kw)
            res = self.reader()
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(reason, res.stdout)
            self.assertIn("nothing was read", res.stdout)
        self.assertEqual(self.outputs(), [], "nothing is copied when a check fails")
        os.remove(os.path.join(self.work, "kexec-stick-20261110T000000Z-abc", "stick.img"))
        res = self.reader()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("no kexec-stick folder with a stick.img", res.stdout)
        self.assertEqual(self.reader("--help").returncode, 2)

    @unittest.skipUnless(os.geteuid() == 0 and shutil.which("mount"), "needs root to loop-mount")
    def test_replays_the_journal_of_a_partition_cut_off_while_mounted(self):
        dev = self.plug(logs={})
        mnt = os.path.join(self.root, "mnt")
        os.makedirs(mnt)
        res = subprocess.run(["mount", "-o", "loop,rw,sync", dev, mnt], stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, text=True)
        if res.returncode != 0:
            self.skipTest("loop mount failed: " + res.stdout)
        try:
            write(os.path.join(mnt, BOOT, "journal.txt"), "".join(f"line {i}\n" for i in range(5000)))
            os.sync()
            cut = os.path.join(self.root, "cut.img")
            shutil.copyfile(dev, cut)  # the partition as a power cut leaves it: still marked mounted
        finally:
            subprocess.run(["umount", mnt])
        shutil.copyfile(cut, dev)
        res = self.reader()
        self.assertEqual(res.returncode, 0, res.stdout)
        report = self.report()
        self.assertIn("recovering journal", report)
        self.assertIn("line 4999\n", report)


class StaticTests(unittest.TestCase):
    def test_log_size_matches_the_builder(self):
        spec = importlib.util.spec_from_file_location("offline_kexec_stick", BUILDER)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        self.assertEqual(mod.LOG_SIZE, LOG_SIZE)
        with open(TOOL) as fh:
            self.assertIn("LOG_SIZE = 32 << 20", fh.read())

    def test_only_reads_the_stick(self):
        with open(TOOL) as fh:
            code = fh.read().split('"""', 2)[2]
        self.assertNotIn('"-w"', code, "debugfs is never opened for writing")
        self.assertEqual(code.count('["sudo"]'), 1)
        self.assertIn('sudo + ["cat", "--", dev]', code)
        for needle in ("ssh", '"mount"', '"dd"', "O_WRONLY", "O_RDWR", "rmtree", "os.remove", "unlink"):
            self.assertNotIn(needle, code, needle)


if __name__ == "__main__":
    unittest.main(verbosity=2)
