#!/usr/bin/env python3
"""Tests for write-autorun-drive.py, against a fake /proc and /sys (MPFREE_TEST_ROOT)."""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "write-autorun-drive.py")


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


class WriteAutorunDriveTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="mpfree-autorun-test-")
        self.mounts = []
        self.disk("sda", usb=False)
        self.mount("sda2", "/", "ext4")
        self.disk("sdc")
        self.mount("sdc1", "/media/jesse/SPARE DRIVE", "vfat")

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def disk(self, name, usb=True, partnames=("",)):
        bus = "usb1/1-1/1-1:1.0/host6/target6:0:0/6:0:0:0" if usb else "ata1/host0/target0:0:0/0:0:0:0"
        real = f"{self.root}/sys/devices/pci0000:00/{bus}/block/{name}"
        write(f"{real}/size", "60000000\n")
        write(f"{real}/device/vendor", "Generic\n" if usb else "ATA\n")
        write(f"{real}/device/model", "Flash Disk\n" if usb else "SSD\n")
        os.makedirs(f"{self.root}/sys/block", exist_ok=True)
        os.makedirs(f"{self.root}/sys/class/block", exist_ok=True)
        os.symlink(real, f"{self.root}/sys/block/{name}")
        for n, partname in enumerate(partnames, 1):
            write(f"{real}/{name}{n}/uevent", f"DEVNAME={name}{n}\n" + (f"PARTNAME={partname}\n" if partname else ""))
            write(f"{real}/{name}{n}/partition", f"{n}\n")
            os.symlink(f"{real}/{name}{n}", f"{self.root}/sys/class/block/{name}{n}")

    def mount(self, node, where, fs):
        os.makedirs(self.root + where, exist_ok=True)
        escaped = where.replace(" ", "\\040")
        self.mounts.append(f"{40 + len(self.mounts)} 1 8:{len(self.mounts)} / {escaped} rw,relatime shared:1 - "
                           f"{fs} /dev/{node} rw\n")
        write(f"{self.root}/proc/self/mountinfo", "".join(self.mounts))

    def run_tool(self, answer="WRITE\n"):
        env = dict(os.environ, MPFREE_TEST_ROOT=self.root)
        return subprocess.run([sys.executable, "-I", TOOL], input=answer, env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)

    def drive(self, name="autorun.sh"):
        return os.path.join(self.root, "media/jesse/SPARE DRIVE", name)

    def test_writes_the_script_to_the_one_spare_drive(self):
        res = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn("/dev/sda2 on /: ATA SSD, ext4", out)
        self.assertIn("not used: not on USB; ext4, not FAT32 or exFAT", out)
        self.assertIn("/dev/sdc1 on /media/jesse/SPARE DRIVE: Generic Flash Disk, vfat, 30.7 GB\n      CAN be used", out)
        self.assertIn("It adds autorun.sh to /media/jesse/SPARE DRIVE (/dev/sdc1).", out)
        self.assertIn("and read it back: it matches", out)
        with open(self.drive()) as fh:
            script = fh.read()
        self.assertTrue(script.startswith("#!/bin/sh\n# MPFree autorun test (write-autorun-drive.py)\n"))
        self.assertIn("> /dev/kmsg", script)
        self.assertEqual(sorted(os.listdir(os.path.dirname(self.drive()))), ["autorun.sh"])
        self.assertEqual(subprocess.run(["sh", "-n", self.drive()]).returncode, 0)
        # Again, after a run on the MPC left its result: the old result goes, the script is rewritten.
        write(self.drive("mpfree-autorun-result.txt"), "mpfree-autorun result\nid: uid=0(root)\n")
        res = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("removes mpfree-autorun-result.txt from an earlier run.", res.stdout)
        self.assertEqual(sorted(os.listdir(os.path.dirname(self.drive()))), ["autorun.sh"])

    def test_writes_only_after_the_typed_word(self):
        for answer in ("\n", "write\n", "yes\n", ""):
            res = self.run_tool(answer)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("not confirmed; nothing was written", res.stdout)
            self.assertEqual(os.listdir(os.path.dirname(self.drive())), [])

    def test_leaves_other_files_alone(self):
        write(self.drive(), "#!/bin/sh\necho someone else's\n")
        res = self.run_tool()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("already holds autorun.sh, which this tool did not write", res.stdout)
        with open(self.drive()) as fh:
            self.assertEqual(fh.read(), "#!/bin/sh\necho someone else's\n")
        os.unlink(self.drive())
        write(self.drive("mpfree-autorun-result.txt"), "something else\n")
        res = self.run_tool()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("already holds mpfree-autorun-result.txt, which this tool did not write", res.stdout)
        self.assertFalse(os.path.exists(self.drive()))

    def test_never_the_test_stick_and_never_a_guess(self):
        self.disk("sdb", partnames=("mpfree-test-root", "mpfree-test-log", "mpfree-test-kits", "mpfree-test-data"))
        self.mount("sdb4", "/media/jesse/MPFREE-DATA", "vfat")
        res = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("/dev/sdb4 on /media/jesse/MPFREE-DATA: Generic Flash Disk, vfat, 30.7 GB\n"
                      "      not used: the MPFree test stick", res.stdout)
        self.assertFalse(os.path.exists(os.path.join(self.root, "media/jesse/MPFREE-DATA/autorun.sh")))
        self.disk("sdd")
        self.mount("sdd1", "/media/jesse/OTHER", "exfat")
        res = self.run_tool()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("more than one drive could be used", res.stdout)
        self.assertFalse(os.path.exists(os.path.join(self.root, "media/jesse/OTHER/autorun.sh")))

    def test_no_drive(self):
        self.mounts = self.mounts[:1]
        write(f"{self.root}/proc/self/mountinfo", "".join(self.mounts))
        res = self.run_tool()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("no spare drive to use", res.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
