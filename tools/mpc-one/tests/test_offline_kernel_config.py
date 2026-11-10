#!/usr/bin/env python3
"""Tests for offline-kernel-config.py against fake kernels with an embedded config. No device.

gzip, xz and lzma payloads are always tested. lz4, lzo and zstd payloads are
tested when those commands are installed (lz4 also through the built-in
decoder, with the command hidden); CI installs them
(MPFREE_REQUIRE_COMPRESSORS=1 turns a missing one into a failure).
"""
import gzip
import lzma
import os
import random
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "offline-kernel-config.py")
CONFIG = (b"#\n# Automatically generated file; DO NOT EDIT.\n"
          b'CONFIG_LOCALVERSION="-az01-fake"\n'
          b"CONFIG_KEXEC=y\n"
          b"# CONFIG_SECURITY_LOADPIN is not set\n"
          b"CONFIG_USB_STORAGE=y\n"
          b"CONFIG_BLK_DEV_SD=m\n"
          b"CONFIG_NOT_SELECTED_FAKE=y\n")
EXTERNAL = {"lz4": ["lz4", "-l", "-9", "-c"], "lzo": ["lzop", "-9", "-c"], "zstd": ["zstd", "-19", "-q", "-c"]}


def noise(n, seed):
    """Bytes that never contain the start of a compressed stream by accident."""
    rnd = random.Random(seed)
    return bytes(rnd.choice(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789") for _ in range(n))


def inner_kernel():
    return noise(200000, 1) + b"IKCFG_ST" + gzip.compress(CONFIG) + b"IKCFG_ED" + noise(100000, 2)


def compress(kind, data):
    if kind == "gzip":
        return gzip.compress(data, 9)
    if kind == "xz":  # as the kernel builds it: CRC32 check and the ARM branch filter
        return lzma.compress(data, format=lzma.FORMAT_XZ, check=lzma.CHECK_CRC32,
                             filters=[{"id": lzma.FILTER_ARM}, {"id": lzma.FILTER_LZMA2, "preset": 6}])
    if kind == "lzma":
        return lzma.compress(data, format=lzma.FORMAT_ALONE)
    return subprocess.run(EXTERNAL[kind], input=data, capture_output=True, check=True).stdout


def zimage(kind):
    """A decompressor stub, the compressed kernel, then the 4-byte size the kernel appends."""
    return noise(20000, 3) + compress(kind, inner_kernel()) + b"\x00\x10\x00\x00"


class KernelConfigTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-kcfg-test-")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_tool(self, data, *args, path=None):
        path = path or os.path.join(self.tmp, "zImage")
        with open(path, "wb") as fh:
            fh.write(data)
        return subprocess.run([sys.executable, "-I", TOOL, path, *args], stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=120,
                              env={**os.environ, "PATH": os.environ.get("PATH", "/usr/bin:/bin")})

    def check(self, kind):
        res = self.run_tool(zimage(kind), "--select")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"(config found: {kind} payload at offset 20000;", res.stdout)
        self.assertIn('CONFIG_LOCALVERSION="-az01-fake"', res.stdout)
        self.assertIn("CONFIG_KEXEC=y", res.stdout)
        self.assertIn("# CONFIG_SECURITY_LOADPIN is not set", res.stdout)
        self.assertIn("CONFIG_USB_STORAGE=y", res.stdout)
        self.assertIn("CONFIG_BLK_DEV_SD=m", res.stdout)
        self.assertNotIn("NOT_SELECTED", res.stdout)

    def test_stdlib_payloads(self):
        for kind in ("gzip", "xz", "lzma"):
            with self.subTest(kind):
                self.check(kind)

    def test_external_payloads(self):
        for kind, cmd in EXTERNAL.items():
            with self.subTest(kind):
                if not shutil.which(cmd[0]):
                    if os.environ.get("MPFREE_REQUIRE_COMPRESSORS"):
                        self.fail(f"{cmd[0]} is not installed")
                    self.skipTest(f"{cmd[0]} is not installed")
                self.check(kind)

    def test_whole_config_and_uncompressed_kernel(self):
        res = self.run_tool(inner_kernel())
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(res.stdout, CONFIG.decode())

    def test_failures(self):
        res = self.run_tool(noise(100000, 4))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("no embedded config found", res.stdout)
        damaged = inner_kernel().replace(b"IKCFG_ED", b"XXXXXXXX")
        res = self.run_tool(damaged)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("not followed by IKCFG_ED", res.stdout)
        res = self.run_tool(b"", "--select", "--extra")
        self.assertEqual(res.returncode, 2, res.stdout)

    def run_without_tools(self, data):
        path = os.path.join(self.tmp, "zImage")
        with open(path, "wb") as fh:
            fh.write(data)
        empty = os.path.join(self.tmp, "empty-bin")
        os.makedirs(empty, exist_ok=True)
        return subprocess.run([sys.executable, "-I", TOOL, path, "--select"], stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=120, env={"PATH": empty})

    def test_lz4_without_the_lz4_command(self):
        if not shutil.which("lz4"):
            self.skipTest("lz4 is not installed (needed to make the test data)")
        res = self.run_without_tools(zimage("lz4"))
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("(config found: lz4 payload at offset 20000;", res.stdout)
        self.assertIn("CONFIG_USB_STORAGE=y", res.stdout)

    def test_missing_decompressor_is_named(self):
        if not shutil.which("lzop"):
            self.skipTest("lzop is not installed (needed to make the test data)")
        res = self.run_without_tools(zimage("lzo"))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("install the 'lzop' command", res.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
