#!/usr/bin/env python3
"""Tests for compare-emmc-backups.py on small fake backups: a disk image with
the MPC One's partition names in a GPT, two boot areas and SHA256SUMS."""
import contextlib
import hashlib
import importlib.util
import io
import os
import shutil
import struct
import tempfile
import unittest
import uuid
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("compare", os.path.join(os.path.dirname(HERE),
                                                                      "compare-emmc-backups.py"))
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)

SECTORS = 2048
# The MPC One's partitions, at small sizes (name, start, sectors).
PARTS = [("uboot-spl", 64, 128), ("env", 192, 16), ("uboot", 256, 768), ("splash", 1024, 64),
         ("recoverysplash", 1088, 64), ("rootfs", 1152, 256), ("content", 1408, 256), ("data", 1664, 340)]


def gpt_disk(seed):
    disk = bytearray(SECTORS * 512)
    for i, (_, start, count) in enumerate(PARTS):
        disk[start * 512:(start + count) * 512] = bytes([(seed[i] + i) % 256]) * count * 512
    ents = bytearray(128 * 128)
    for i, (name, start, count) in enumerate(PARTS):
        e = memoryview(ents)[i * 128:(i + 1) * 128]
        e[0:16] = uuid.UUID("0fc63daf-8483-4772-8e79-3d69d8477de4").bytes_le
        e[16:32] = uuid.UUID(int=i + 1).bytes_le
        struct.pack_into("<QQQ", e, 32, start, start + count - 1, 0)
        n = name.encode("utf-16-le")
        e[56:56 + len(n)] = n
    hdr = bytearray(92)
    hdr[0:8] = b"EFI PART"
    struct.pack_into("<IIIIQQQQ", hdr, 8, 0x10000, 92, 0, 0, 1, SECTORS - 1, 34, SECTORS - 34)
    hdr[56:72] = uuid.UUID(int=99).bytes_le
    struct.pack_into("<QIII", hdr, 72, 2, 128, 128, zlib.crc32(bytes(ents)))
    struct.pack_into("<I", hdr, 16, zlib.crc32(bytes(hdr)))
    disk[512:512 + 92] = hdr
    disk[1024:1024 + len(ents)] = ents
    disk[(SECTORS - 33) * 512:(SECTORS - 1) * 512] = ents[:32 * 512]
    return bytes(disk)


class CompareTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp)

    def backup(self, name, seed=(0,) * 8, boot=b"\x11", damage=None):
        d = os.path.join(self.tmp, name)
        os.makedirs(d)
        images = {"mmcblk0": gpt_disk(seed), "mmcblk0boot0": boot * 4096, "mmcblk0boot1": boot * 4096}
        sums = ""
        for k, v in images.items():
            sums += f"{hashlib.sha256(v).hexdigest()}  {k}.img\n"
            if damage == k:
                v = v[:-1] + b"\xff"
            with open(os.path.join(d, f"{k}.img"), "wb") as fh:
                fh.write(v)
        with open(os.path.join(d, "SHA256SUMS"), "w") as fh:
            fh.write(sums)
        return d

    def run_tool(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = tool.main(list(args))
        return rc, out.getvalue()

    def test_the_same_where_it_must_be_with_content_and_data_written(self):
        old = self.backup("mpc-one-emmc-20261110T090000Z-abc123")
        new = self.backup("mpc-one-emmc-20261115T230000Z", seed=(0, 0, 0, 0, 0, 0, 7, 9))
        rc, out = self.run_tool(old, new)
        self.assertEqual(rc, 0, out)
        self.assertIn("SHA256SUMS: every image of the older backup matches", out)
        self.assertIn("partition table: the same layout, 8 partitions", out)
        for label in ("gpt-head", "uboot-spl", "env", "uboot", "rootfs", "gpt-tail", "mmcblk0boot0",
                      "mmcblk0boot1"):
            self.assertRegex(out, rf"\n  {label} +same ")
        self.assertRegex(out, r"\n  content +differs .*\(written in normal use: expected\)")
        self.assertRegex(out, r"\n  data +differs ")
        self.assertIn("RESULT: the same everywhere they must be", out)

    def test_dir_takes_the_newest_two_and_passes_over_unfinished_ones(self):
        self.backup("mpc-one-emmc-20261101T090000Z-old", seed=(5,) * 8)
        old = self.backup("mpc-one-emmc-20261110T090000Z-abc123")
        new = self.backup("mpc-one-emmc-20261115T230000Z")
        os.makedirs(os.path.join(self.tmp, "mpc-one-emmc-20261116T000000Z.part"))
        os.makedirs(os.path.join(self.tmp, "mpc-one-emmc-20261117T000000Z"))  # no images
        rc, out = self.run_tool("--dir", self.tmp)
        self.assertEqual(rc, 0, out)
        self.assertIn("3 complete backups", out)
        self.assertIn(f"older backup: {old}\n", out)
        self.assertIn(f"newer backup: {new}\n", out)

    def test_differences_where_it_must_be_the_same_fail(self):
        old = self.backup("mpc-one-emmc-20261110T090000Z")
        new = self.backup("mpc-one-emmc-20261115T230000Z", seed=(0, 0, 0, 0, 0, 3, 0, 0), boot=b"\x22")
        rc, out = self.run_tool(old, new)
        self.assertEqual(rc, 1, out)
        self.assertRegex(out, r"\n  rootfs +DIFFERS ")
        self.assertRegex(out, r"\n  mmcblk0boot0 +DIFFERS ")
        self.assertIn("RESULT: DIFFERENT", out)
        # The U-Boot environment alone is worth a look, not a failure.
        env = self.backup("mpc-one-emmc-20261116T230000Z", seed=(0, 4, 0, 0, 0, 0, 0, 0))
        rc, out = self.run_tool(old, env)
        self.assertEqual(rc, 0, out)
        self.assertRegex(out, r"\n  env +differs .*worth a look")

    def test_damage_and_non_backups_are_said(self):
        old = self.backup("mpc-one-emmc-20261110T090000Z")
        new = self.backup("mpc-one-emmc-20261115T230000Z", damage="mmcblk0boot1")
        rc, out = self.run_tool(old, new)
        self.assertEqual(rc, 1, out)
        self.assertIn("DAMAGED: in the newer backup, mmcblk0boot1.img does not match SHA256SUMS", out)
        os.unlink(os.path.join(new, "SHA256SUMS"))
        rc, out = self.run_tool(old, new)
        self.assertEqual(rc, 2, out)
        self.assertIn("STOP:", out)
        rc, out = self.run_tool("--dir", self.tmp)
        self.assertEqual(rc, 2, out)
        self.assertIn("needs two complete mpc-one-emmc-* backups, found 1", out)


if __name__ == "__main__":
    unittest.main()
