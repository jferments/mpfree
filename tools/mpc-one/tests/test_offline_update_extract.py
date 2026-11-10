#!/usr/bin/env python3
"""Tests for offline-update-extract.py against hand-built FDT/FIT images. No device."""
import bz2
import gzip
import hashlib
import lzma
import os
import random
import stat
import subprocess
import sys
import tempfile
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_offline_update_image import az01_blob, fdt_blob, pad4, s, u32  # noqa: E402

TOOL = os.path.join(os.path.dirname(HERE), "offline-update-extract.py")
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))

ROOTFS = random.Random(7).randbytes(300_000) + bytes(2_000_000)  # mostly zeros, like a filesystem
SPLASH = random.Random(8).randbytes(50_000)
KERNEL = random.Random(9).randbytes(40_000)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def node(name, stored, compression, hashes=("sha1",), extra=()):
    children = []
    for i, algo in enumerate(hashes):
        if algo == "crc32":
            value = zlib.crc32(stored).to_bytes(4, "big")
        elif algo == "bad-sha1":
            algo, value = "sha1", bytes(20)
        else:
            value = hashlib.new(algo, stored).digest()
        children.append((f"hash-{i + 1}", [("algo", s(algo)), ("value", value)], []))
    return (name, [("data", stored), ("compression", s(compression)), *extra], children)


def image_tree(images):
    return ("", [("description", s("MPC upgrade image"))],
            [("images", [], images),
             ("signature", [("value", bytes(256)), ("algo", s("sha256,rsa2048"))], [])])


class OfflineUpdateExtractTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="mpfree-extract-test-")
        self.dir = self.tmp.name
        self.work = os.path.join(self.dir, "work")

    def tearDown(self):
        self.tmp.cleanup()

    def put(self, name, data):
        path = os.path.join(self.dir, name)
        with open(path, "wb") as fh:
            fh.write(data)
        return path

    def run_tool(self, *args, work=None):
        env = dict(os.environ, MPFREE_WORK_DIR=work or self.work)
        return subprocess.run([sys.executable, "-I", TOOL, *args], stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=300, env=env)

    def out_dir(self):
        [d] = os.listdir(self.work)
        return os.path.join(self.work, d)

    def read_out(self, name):
        with open(os.path.join(self.out_dir(), name), "rb") as fh:
            return fh.read()

    def test_extracts_and_checks_every_payload(self):
        images = [
            node("rootfs", gzip.compress(ROOTFS), "gzip", ("sha1", "crc32")),
            node("splash", lzma.compress(SPLASH, format=lzma.FORMAT_XZ), "xz", ("sha256",)),
            node("kernel", bz2.compress(KERNEL), "bzip2", ("md5",)),
            node("recoverysplash", SPLASH, "none", ()),
        ]
        blob = fdt_blob(image_tree(images))
        path = self.put("update.img", blob)
        before = (sha(blob), os.stat(path).st_mtime_ns)
        res = self.run_tool(path)
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn("  hash-1: sha1 matches", out)
        self.assertIn("  hash-2: crc32 matches", out)
        self.assertIn("  hash-1: sha256 matches", out)
        self.assertIn("  hash-1: md5 matches", out)
        self.assertIn("recoverysplash: 50000 bytes stored (inline)\n  no hash in the image for this payload", out)
        self.assertIn(f"images/rootfs: compression gzip -> images_rootfs.img, {len(ROOTFS)} bytes", out)
        self.assertEqual(self.read_out("images_rootfs.img"), ROOTFS)
        self.assertEqual(self.read_out("images_splash.img"), SPLASH)
        self.assertEqual(self.read_out("images_kernel.img"), KERNEL)
        self.assertEqual(self.read_out("images_recoverysplash.img"), SPLASH)
        self.assertIn(f"sha256 {sha(ROOTFS)}", out)
        mode = stat.S_IMODE(os.stat(os.path.join(self.out_dir(), "images_rootfs.img")).st_mode)
        self.assertEqual(mode, 0o400)
        with open(path, "rb") as fh:
            self.assertEqual((sha(fh.read()), os.stat(path).st_mtime_ns), before, "image must not change")

    def test_hash_mismatch_is_not_written(self):
        images = [node("rootfs", gzip.compress(ROOTFS), "gzip", ("bad-sha1",)),
                  node("splash", SPLASH, "none", ("sha1",))]
        res = self.run_tool(self.put("bad.img", fdt_blob(image_tree(images))))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("hash-1: sha1 MISMATCH", res.stdout)
        self.assertIn("images/rootfs: NOT written, because a hash doesn't match", res.stdout)
        self.assertEqual(sorted(os.listdir(self.out_dir())), ["images_splash.img"])

    def test_truncated_and_trailing_streams(self):
        whole = gzip.compress(ROOTFS)
        images = [node("cut", whole[:-50], "gzip", ()),
                  node("extra", lzma.compress(SPLASH, format=lzma.FORMAT_XZ) + b"junk", "xz", ())]
        res = self.run_tool(self.put("cut.img", fdt_blob(image_tree(images))))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("images/cut: NOT written: gzip stream is truncated", res.stdout)
        self.assertIn("images/extra: NOT written:", res.stdout)
        self.assertEqual(os.listdir(self.out_dir()), [], "no partial files may remain")

    def test_external_payload_and_node_selection(self):
        stored = gzip.compress(KERNEL)
        tree = ("", [], [("images", [], [
            ("kernel", [("data-offset", u32(0)), ("data-size", u32(len(stored))), ("compression", s("gzip"))],
             [("hash-1", [("algo", s("sha256")), ("value", hashlib.sha256(stored).digest())], [])]),
            node("splash", SPLASH, "none", ()),
        ])])
        fdt = fdt_blob(tree)
        res = self.run_tool(self.put("ext.img", pad4(fdt) + stored), "--node", "images/kernel")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("images/kernel: ", res.stdout)
        self.assertIn("(external)", res.stdout)
        self.assertEqual(os.listdir(self.out_dir()), ["images_kernel.img"])
        self.assertEqual(self.read_out("images_kernel.img"), KERNEL)
        res = self.run_tool(self.put("ext2.img", pad4(fdt) + stored), "--node", "images/nope")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("ERROR: no payload at images/nope; payloads: images/kernel, images/splash", res.stdout)

    def test_az01_image_hashed_over_uncompressed_data(self):
        packed = lzma.compress(ROOTFS, format=lzma.FORMAT_XZ)
        blob = az01_blob([("rootfs", "xz", [("sha1", hashlib.sha1(ROOTFS).digest())], packed),
                          ("splash", "none", [("sha256", hashlib.sha256(SPLASH[:999]).digest())], SPLASH[:999])],
                         trailer=bytes(16))
        res = self.run_tool(self.put("MPC-3.10.0-Gen1-update.img", blob))
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn(f"note: 16 bytes at offset {len(blob) - 16} are not a PART record", out)
        self.assertIn(f"rootfs: {len(packed)} bytes stored (AZ01 PART record)", out)
        self.assertIn("  hash-1: sha1 differs over the stored bytes", out)
        self.assertIn(f"  hash-1: sha1 matches the uncompressed data ({hashlib.sha1(ROOTFS).hexdigest()})", out)
        self.assertIn(f"rootfs: compression xz -> rootfs.img, {len(ROOTFS)} bytes", out)
        self.assertIn("  hash-1: sha256 matches", out)
        self.assertEqual(self.read_out("rootfs.img"), ROOTFS)
        self.assertEqual(self.read_out("splash.img"), SPLASH[:999])

    def test_az01_hash_matching_neither_is_not_written(self):
        packed = lzma.compress(ROOTFS, format=lzma.FORMAT_XZ)
        blob = az01_blob([("rootfs", "xz", [("sha1", bytes(20))], packed)])
        res = self.run_tool(self.put("bad.img", blob), "--node", "rootfs")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("  hash-1: sha1 MISMATCH (image says 0000", res.stdout)
        self.assertIn("rootfs: NOT written, because a hash doesn't match", res.stdout)
        self.assertEqual(os.listdir(self.out_dir()), [], "no partial files may remain")

    def test_large_zero_payload_uses_bounded_memory(self):
        zeros = gzip.compress(bytes(400 << 20), compresslevel=1)  # 400 MiB of zeros, ~400 KiB stored
        path = self.put("zeros.img", fdt_blob(image_tree([node("rootfs", zeros, "gzip", ())])))
        probe = ("import resource, subprocess, sys; r = subprocess.run(sys.argv[1:]); "
                 "print('RSS', resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss); sys.exit(r.returncode)")
        res = subprocess.run([sys.executable, "-c", probe, sys.executable, "-I", TOOL, path],
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=300,
                             env=dict(os.environ, MPFREE_WORK_DIR=self.work))
        self.assertEqual(res.returncode, 0, res.stdout)
        rss_kib = int(res.stdout.rsplit("RSS ", 1)[1].split()[0])
        self.assertLess(rss_kib, 150 * 1024, res.stdout)
        self.assertEqual(os.path.getsize(os.path.join(self.out_dir(), "images_rootfs.img")), 400 << 20)

    def test_refuses_output_inside_the_clone(self):
        target = os.path.join(REPO, "tools", "mpc-one", "extract-test-not-ignored")
        res = self.run_tool(self.put("a.img", fdt_blob(image_tree([node("splash", SPLASH, "none", ())]))),
                            work=target)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("inside the git clone and not git-ignored", res.stdout)
        self.assertFalse(os.path.exists(target))

    def test_not_an_image(self):
        res = self.run_tool(self.put("x.img", b"MZ" + bytes(100)))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("ERROR: no FDT header", res.stdout)
        self.assertFalse(os.path.exists(self.work))


if __name__ == "__main__":
    unittest.main(verbosity=2)
