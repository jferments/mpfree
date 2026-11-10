#!/usr/bin/env python3
"""Tests for write-test-stick.py against a fake /sys, /dev and /proc. No real device is opened.

MPFREE_TEST_ROOT makes the tool look for /sys, /dev, /proc and /run under a
temporary folder and skip sudo, so the "devices" here are plain files.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "write-test-stick.py")
IMAGE = bytes(range(256)) * 4096 * 3          # 3 MiB, not a multiple of the 4 MiB chunk
MIB = 1 << 20


def write(path, data=""):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)


class WriterTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="mpfree-stick-writer-test-")
        self.work = os.path.join(self.root, "work")
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        write(os.path.join(folder, "stick.img"), IMAGE)
        write(os.path.join(folder, "SHA256SUMS"), f"{hashlib.sha256(IMAGE).hexdigest()}  stick.img\n")
        write(os.path.join(self.root, "proc/self/mountinfo"),
              "22 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw\n")
        write(os.path.join(self.root, "proc/swaps"), "Filename\tType\tSize\tUsed\tPriority\n")
        os.makedirs(os.path.join(self.root, "sys/block"))
        self.disk("nvme0n1", "259:0", 512 << 30 >> 9, removable="0", usb=False)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def disk(self, name, dev, sectors, removable="1", usb=True, serial="FAKE123", parts=1):
        bus = "usb2/2-1/2-1:1.0/host6" if usb else "0000:00:1d.0/nvme"
        real = os.path.join(self.root, "sys/devices/pci0000:00", bus, "block", name)
        major, minor = dev.split(":")
        write(os.path.join(real, "size"), f"{sectors}\n")
        write(os.path.join(real, "dev"), dev + "\n")
        write(os.path.join(real, "removable"), removable + "\n")
        write(os.path.join(real, "ro"), "0\n")
        write(os.path.join(real, "device/vendor"), "FakeVend\n")
        write(os.path.join(real, "device/model"), "Fake Stick\n")
        os.makedirs(os.path.join(real, "holders"))
        os.symlink(real, os.path.join(self.root, "sys/block", name))
        write(os.path.join(self.root, f"run/udev/data/b{dev}"), f"E:ID_SERIAL_SHORT={serial}\n")
        os.makedirs(os.path.join(self.root, "sys/class/block"), exist_ok=True)
        os.symlink(real, os.path.join(self.root, "sys/class/block", name))
        write(os.path.join(self.root, "dev", name), b"\xee" * (sectors * 512 if usb else 16))
        for n in range(1, parts + 1):
            part = os.path.join(real, f"{name}{n}")
            write(os.path.join(part, "partition"), f"{n}\n")
            write(os.path.join(part, "dev"), f"{major}:{int(minor) + n}\n")
            os.makedirs(os.path.join(part, "holders"))
            os.symlink(part, os.path.join(self.root, "sys/class/block", f"{name}{n}"))
        return os.path.join(self.root, "dev", name)

    def writer(self, answer="WRITE\n", *args):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "MPFREE_TEST_ROOT": self.root,
               "MPFREE_WORK_DIR": self.work, "HOME": self.root}
        return subprocess.run([sys.executable, "-I", TOOL, *args], input=answer, env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)

    def contents(self, dev):
        with open(dev, "rb") as fh:
            return fh.read()

    def test_writes_the_one_stick_and_reads_it_back(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("/dev/sdb: FakeVend Fake Stick, serial FAKE123, 16777216 bytes", res.stdout)
        self.assertIn("CAN be written", res.stdout)
        self.assertIn("they match stick.img", res.stdout)
        self.assertNotIn("nvme0n1", res.stdout, "internal disks are not even listed")
        data = self.contents(dev)
        self.assertEqual(data[:len(IMAGE)], IMAGE)
        self.assertEqual(data[len(IMAGE):], b"\xee" * (16 * MIB - len(IMAGE)), "nothing past the image")

    def test_anything_but_write_stops(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        for answer in ("write\n", "\n", "yes\n", ""):
            res = self.writer(answer)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertEqual(self.contents(dev), b"\xee" * 16 * MIB)

    def refused(self, reason):
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn(reason, res.stdout)
        self.assertIn("STOP: no USB stick to write", res.stdout)
        return res

    def test_refusals(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        with self.subTest("mounted partition"):
            with open(os.path.join(self.root, "proc/self/mountinfo"), "a") as fh:
                fh.write("40 22 8:17 / /run/media/user/STICK rw - vfat /dev/sdb1 rw\n")
            self.refused("/dev/sdb1 is mounted or used as swap")
        write(os.path.join(self.root, "proc/self/mountinfo"), "22 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n")
        with self.subTest("swap"):
            write(os.path.join(self.root, "proc/swaps"), "Filename Type\n/dev/sdb partition 1 0 -2\n")
            self.refused("/dev/sdb is mounted or used as swap")
        write(os.path.join(self.root, "proc/swaps"), "Filename Type\n")
        with self.subTest("holder"):
            holders = os.path.join(self.root, "sys/class/block/sdb1/holders")
            os.symlink("../../dm-0", os.path.join(holders, "dm-0"))
            self.refused("/dev/sdb1 is in use by LVM, RAID or dm-crypt")
            os.unlink(os.path.join(holders, "dm-0"))
        with self.subTest("not removable"):
            write(os.path.join(self.root, "sys/block/sdb/removable"), "0\n")
            self.refused("not marked removable")
            write(os.path.join(self.root, "sys/block/sdb/removable"), "1\n")
        with self.subTest("too small"):
            write(os.path.join(self.root, "sys/block/sdb/size"), "1024\n")
            self.refused("smaller than stick.img")
        with self.subTest("too large"):
            write(os.path.join(self.root, "sys/block/sdb/size"), f"{(300 << 30) >> 9}\n")
            self.refused("larger than 256 GiB")
        self.assertEqual(self.contents(dev), b"\xee" * 16 * MIB)

    def test_two_sticks_stop(self):
        a = self.disk("sdb", "8:16", 16 * MIB >> 9)
        b = self.disk("sdc", "8:32", 16 * MIB >> 9, serial="FAKE456")
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("more than one USB stick could be written", res.stdout)
        self.assertEqual(self.contents(a), b"\xee" * 16 * MIB)
        self.assertEqual(self.contents(b), b"\xee" * 16 * MIB)

    def test_damaged_image_stops(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        write(os.path.join(self.work, "kexec-stick-20260101T000000Z-abc", "stick.img"), IMAGE[:-1] + b"X")
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("does not match its SHA256SUMS line", res.stdout)
        self.assertEqual(self.contents(dev), b"\xee" * 16 * MIB)

    def test_privileged_step_checks_again(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        image = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc", "stick.img")
        digest = hashlib.sha256(IMAGE).hexdigest()
        with open(os.path.join(self.root, "proc/self/mountinfo"), "a") as fh:
            fh.write("40 22 8:17 / /mnt rw - vfat /dev/sdb1 rw\n")
        res = self.writer("", "--root-write", "sdb", str(len(IMAGE)), image, digest)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("/dev/sdb no longer qualifies", res.stdout)
        write(os.path.join(self.root, "proc/self/mountinfo"), "22 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n")
        for name in ("nvme0n1", "../sdb", "sdb1"):
            res = self.writer("", "--root-write", name, str(len(IMAGE)), image, digest)
            self.assertEqual(res.returncode, 1, res.stdout)
        res = self.writer("", "--root-write", "sdb", str(len(IMAGE)), image, "0" * 64)
        self.assertIn("stick.img changed since it was checked", res.stdout)
        self.assertEqual(self.contents(dev), b"\xee" * 16 * MIB)


class StaticTests(unittest.TestCase):
    def test_no_device_name_from_the_user_and_one_write_path(self):
        with open(TOOL) as fh:
            code = fh.read()
        self.assertEqual(code.count("os.write("), 1)
        self.assertEqual(code.count("os.O_RDWR"), 1)
        self.assertIn("os.O_EXCL", code)
        self.assertNotIn("ssh", code.replace("sha", ""))
        self.assertNotIn("/dev/mmc", code)


if __name__ == "__main__":
    unittest.main(verbosity=2)
