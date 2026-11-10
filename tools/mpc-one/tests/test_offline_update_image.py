#!/usr/bin/env python3
"""Tests for offline-update-image.py against hand-built FDT/FIT images. No device.

The images are built with a small FDT encoder below (the format is the
devicetree specification's flattened tree), so the tests need only Python.
"""
import gzip
import hashlib
import os
import random
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "offline-update-image.py")


def pad4(data):
    return data + b"\x00" * (-len(data) % 4)


def s(*texts):
    return b"".join(t.encode() + b"\x00" for t in texts)


def u32(*cells):
    return struct.pack(f">{len(cells)}I", *cells)


def fdt_blob(root):
    """root = (name, [(prop, bytes), ...], [children])"""
    strings, offsets, body = bytearray(), {}, bytearray()

    def name_offset(name):
        if name not in offsets:
            offsets[name] = len(strings)
            strings.extend(name.encode() + b"\x00")
        return offsets[name]

    def emit(node):
        name, props, children = node
        body.extend(u32(1) + pad4(name.encode() + b"\x00"))
        for prop, value in props:
            body.extend(u32(3, len(value), name_offset(prop)) + pad4(value))
        for child in children:
            emit(child)
        body.extend(u32(2))

    emit(root)
    body.extend(u32(9))
    off_rsv = 40
    off_struct = off_rsv + 16
    off_strings = off_struct + len(body)
    total = off_strings + len(strings)
    header = struct.pack(">10I", 0xD00DFEED, total, off_struct, off_strings, off_rsv,
                         17, 16, 0, len(strings), len(body))
    return header + bytes(16) + bytes(body) + bytes(strings)


def ext4_like(size=4096):
    data = bytearray(random.Random(1).randbytes(size))
    data[1080:1082] = b"\x53\xef"
    data[:16] = bytes(16)
    return bytes(data)


ROOTFS = gzip.compress(random.Random(2).randbytes(6000))
SPLASH = ext4_like()


def mpc_like_tree(extra_images=()):
    rootfs = ("rootfs", [("description", s("rootfs")), ("data", ROOTFS), ("compression", s("gzip"))],
              [("hash", [("algo", s("sha1")), ("value", hashlib.sha1(ROOTFS).digest())], [])])
    splash = ("splash", [("data", SPLASH), ("compression", s("none"))], [])
    return ("", [("description", s("MPC upgrade image")), ("timestamp", u32(0x639B5F86)),
                 ("compatible", s("inmusic,acv5", "inmusic,acva"))],
            [("images", [], [rootfs, splash, *extra_images]),
             ("configurations", [("default", s("conf"))],
              [("conf", [("partitions", s("rootfs", "splash"))], [])])])


def sha(data):
    return hashlib.sha256(data).hexdigest()


class OfflineUpdateImageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="mpfree-img-test-")
        self.dir = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def put(self, name, data):
        path = os.path.join(self.dir, name)
        with open(path, "wb") as fh:
            fh.write(data)
        return path

    def run_tool(self, *paths):
        return subprocess.run([sys.executable, "-I", TOOL, *paths], stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=120)

    def test_lists_partitions_hashes_and_payload_types(self):
        blob = fdt_blob(mpc_like_tree())
        path = self.put("MPC-3.6.0.img", blob)
        before = (sha(blob), os.stat(path).st_mtime_ns)
        res = self.run_tool(path)
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn(f"size {len(blob)} bytes, sha256 {sha(blob)}", out)
        self.assertIn('  description = "MPC upgrade image"', out)
        self.assertIn("  timestamp = <0x639b5f86> (1671126918)", out)
        self.assertIn('  compatible = "inmusic,acv5", "inmusic,acva"', out)
        self.assertIn("    rootfs/\n", out)
        self.assertIn(f"      data = [{len(ROOTFS)} bytes, gzip, starts {ROOTFS[:16].hex()}, sha256 {sha(ROOTFS)}]", out)
        self.assertIn(f"        value = [20 bytes] {hashlib.sha1(ROOTFS).hexdigest()}", out)
        self.assertIn(f"      data = [{len(SPLASH)} bytes, ext2/3/4 filesystem,", out)
        self.assertIn('      partitions = "rootfs", "splash"', out)
        with open(path, "rb") as fh:
            self.assertEqual((sha(fh.read()), os.stat(path).st_mtime_ns), before, "input must not change")

    def test_external_data_prefix_and_trailer(self):
        boot = b"RKNS" + random.Random(3).randbytes(3000)
        rootfs_ext = gzip.compress(b"x" * 9000)
        tree = ("", [("description", s("external"))],
                [("images", [], [
                    ("uboot", [("data-position", u32(0)), ("data-size", u32(len(boot)))], []),
                    ("rootfs", [("data-offset", u32(0)), ("data-size", u32(len(rootfs_ext)))], []),
                    ("broken", [("data-offset", u32(10 ** 6)), ("data-size", u32(16))], []),
                ])])
        fdt = fdt_blob(tree)
        # data-position is absolute from the FIT start, so patch it to point past the FDT.
        store = len(fdt) + (-len(fdt) % 4)
        position = store + len(rootfs_ext)
        tree[2][0][2][0][1][0] = ("data-position", u32(position))
        fdt = fdt_blob(tree)
        self.assertEqual(len(fdt) + (-len(fdt) % 4), store)
        prefix = b"SIGN" + bytes(508)
        image = prefix + pad4(fdt) + rootfs_ext + boot
        res = self.run_tool(self.put("signed.img", image))
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn("FDT header at offset 512:", out)
        self.assertIn(f"data before the FDT: 512 bytes (unknown, starts {prefix[:16].hex()}), sha256 {sha(prefix)}", out)
        self.assertIn(f"external data for images/rootfs: offset {512 + store}, {len(rootfs_ext)} bytes, gzip, "
                      f"starts {rootfs_ext[:16].hex()}, sha256 {sha(rootfs_ext)}", out)
        self.assertIn(f"external data for images/uboot: offset {512 + position}, {len(boot)} bytes, "
                      f"Rockchip idbloader (RKNS),", out)
        self.assertIn(f"sha256 {sha(boot)}", out)
        self.assertIn("external data for images/broken: offset", out)
        self.assertIn("PAST THE END OF THE FILE", out)
        self.assertIn("data after the FDT:", out)

    def test_zip_download_is_unpacked_safely(self):
        blob = fdt_blob(mpc_like_tree())
        zpath = os.path.join(self.dir, "MPC One Firmware.zip")
        with zipfile.ZipFile(zpath, "w") as archive:
            archive.writestr("readme.txt", "hello")
            archive.writestr("../escape/update.img", blob)
        res = self.run_tool(zpath)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("  readme.txt: 5 bytes", res.stdout)
        self.assertIn(f"===== {zpath} -> ../escape/update.img", res.stdout)
        self.assertIn(f"sha256 {sha(ROOTFS)}", res.stdout)
        self.assertFalse(os.path.exists(os.path.join(os.path.dirname(self.dir), "escape")))
        self.assertEqual(sorted(os.listdir(self.dir)), ["MPC One Firmware.zip"], "temporary files left behind")

    def test_folder_reads_every_img_and_zip(self):
        self.put("a.img", fdt_blob(mpc_like_tree()))
        with zipfile.ZipFile(os.path.join(self.dir, "b.zip"), "w") as archive:
            archive.writestr("b.img", fdt_blob(mpc_like_tree()))
        self.put("notes.txt", b"ignored")
        res = self.run_tool(self.dir)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("a.img\n", res.stdout)
        self.assertIn("b.zip -> b.img", res.stdout)
        self.assertNotIn("notes.txt", res.stdout)

    def test_not_an_image(self):
        path = self.put("MPC Updater.exe", b"MZ" + bytes(200))
        res = self.run_tool(path)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("ERROR: no FDT header in the first 202 bytes; starts 4d5a", res.stdout)

    def test_truncated_image(self):
        blob = fdt_blob(mpc_like_tree())
        res = self.run_tool(self.put("short.img", blob[:2000]))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("ERROR:", res.stdout)

    def test_empty_folder_and_missing_path(self):
        res = self.run_tool(self.dir)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("no .img or .zip file directly inside", res.stdout)
        res = self.run_tool(os.path.join(self.dir, "nope.img"))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("not found:", res.stdout)

    def test_tool_stays_offline(self):
        with open(TOOL) as fh:
            code = fh.read()
        for forbidden in ("import socket", "import subprocess", "urllib", "http.client", "os.system",
                          "ssh", "extractall", "os.remove", "shutil.rmtree"):
            self.assertNotIn(forbidden, code)


if __name__ == "__main__":
    unittest.main(verbosity=2)
