#!/usr/bin/env python3
"""Tests for offline-kexec-details.py against a fake ~/mpfree-work. No device."""
import gzip
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "offline-kexec-details.py")
SPEC = importlib.util.spec_from_file_location("test_offline_dtb", os.path.join(HERE, "test_offline_dtb.py"))
t = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(t)
dtb = t.dtb
BUILD = "kexec-stick-20261110T000000Z-abc"


def fake_kernel(config):
    inner = b"K" * 5000 + b"IKCFG_ST" + gzip.compress(config) + b"IKCFG_ED" + b"K" * 5000
    return b"Z" * 3000 + gzip.compress(inner) + b"Z" * 100


class DetailsTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.mkdtemp(prefix="mpfree-kexec-details-test-")
        self.build = os.path.join(self.work, BUILD)
        os.makedirs(self.build)
        old_dir, new_dir = (os.path.join(self.work, "boot-files", r) for r in ("1.0-old", "9.9-new"))
        pre = os.path.join(self.work, "kexec-preflight-20261110T000000Z-x")
        for d in (old_dir, new_dir, pre):
            os.makedirs(d)
        new = t.board_tree()
        gmac = new.find("/ethernet@ff290000")
        gmac.children.append(t.node("mdio", [], [t.node("phy@1", [("reg", t.cells(1)),
                                                                   ("reset-gpios", t.cells(1, 8, 1))])]))
        new.root.children.append(t.node("gpio4@ff790000", [("gpio-controller", b""), ("#gpio-cells", t.cells(2)),
                                                           ("phandle", t.cells(1))],
                                        [t.node("lan-hog", [("gpio-hog", b""), ("gpios", t.cells(9, 0)),
                                                            ("output-high", b"")])]))
        old = t.board_tree()
        old.find("/ethernet@ff290000").set_prop("snps,reset-gpio", t.cells(7, 8, 1))
        live = t.board_tree(serial=t.SERIAL, mac=t.MAC)
        kexec = t.board_tree(serial=t.SERIAL, mac=t.MAC)
        self.paths = {}
        for key, folder, tree in (("old", old_dir, old), ("new", new_dir, new), ("live", pre, live)):
            name = "live.dtb" if key == "live" else "rk3288-az01-acva.dtb"
            self.paths[key] = os.path.join(folder, name)
            self.save(self.paths[key], dtb.build(tree))
        self.save(os.path.join(self.build, "kexec.dtb"), dtb.build(kexec))
        self.save(os.path.join(self.build, "stick.img"), b"\0" * 512)
        self.save(os.path.join(self.build, "zImage"), fake_kernel(b"CONFIG_FAKE_NEW=y\n# CONFIG_VT is not set\n"))
        self.old_kernel = os.path.join(old_dir, "zImage-1.0-old")
        self.save(self.old_kernel, fake_kernel(b"CONFIG_FAKE_OLD=y\nCONFIG_VT=y\n"))
        self.report = os.path.join(self.build, "report.txt")
        self.save(self.report, (
            "its kernel release: 9.9-new\n"
            f"running tree: {self.paths['live']} (Akai Professional MPC One; inmusic,acva)\n"
            "running release: 1.0-old (release.txt)\n"
            f"board file for the new kernel: {self.paths['new']}\n"
            f"board file of the running release: {self.paths['old']}\n").encode())
        # An older build, and a newer folder without a stick.img: neither is used.
        os.makedirs(os.path.join(self.work, "kexec-stick-20261109T000000Z-old"))
        os.makedirs(os.path.join(self.work, "kexec-stick-20261111T000000Z-unfinished"))

    def tearDown(self):
        shutil.rmtree(self.work, ignore_errors=True)

    @staticmethod
    def save(path, data):
        with open(path, "wb") as fh:
            fh.write(data)

    def run_tool(self, *args):
        env = {"PATH": "/usr/bin:/bin", "MPFREE_WORK_DIR": self.work, "HOME": self.work}
        return subprocess.run([sys.executable, "-I", TOOL, *args], env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)

    def test_prints_nodes_differences_and_configs(self):
        res = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn(f"build: {self.build}\n", out)
        for line in (f"board file of the running release 1.0-old: {self.paths['old']}",
                     f"board file of the new release 9.9-new: {self.paths['new']}",
                     f"running tree, as U-Boot gave it to the running release: {self.paths['live']}",
                     f"test build's kexec.dtb: {os.path.join(self.build, 'kexec.dtb')}",
                     "=== 4. Nodes of the test build's kexec.dtb ===",
                     "=== 2. Nodes of the board file of the new release 9.9-new ===",
                     "      reset-gpios = <&{/gpio4@ff790000} 0x8 0x1>;",
                     "/gpio4@ff790000/lan-hog {",
                     "GPIO hogs: /gpio4@ff790000/lan-hog",
                     "GPIO hogs: none",
                     "(no node /mipi-backlight in this tree)",
                     "  local-mac-address = (6 bytes, not shown);",
                     "=== 5. Differences between the two board files, with values ===",
                     "property /ethernet@ff290000/snps,reset-gpio: only in A (12 bytes): A <0x7 0x8 0x1> | B -",
                     "node only in B: /ethernet@ff290000/mdio/phy@1",
                     "=== 6. Kernel build config of the new release 9.9-new ===",
                     "CONFIG_FAKE_NEW=y",
                     "# CONFIG_VT is not set",
                     "=== 7. Kernel build config of the running release 1.0-old ===",
                     "CONFIG_FAKE_OLD=y",
                     "DONE. Only files on this computer were read."):
            self.assertIn(line + "\n", out)
        self.assertEqual(out.count("  local-mac-address = (6 bytes, not shown);"), 2, "live.dtb and kexec.dtb")
        self.assertNotIn(t.SERIAL, out)
        self.assertNotIn(t.MAC.hex(), out)

    def test_missing_pieces(self):
        os.remove(self.old_kernel)
        os.remove(os.path.join(self.build, "kexec.dtb"))
        res = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"--- {self.old_kernel}\n(no such file)\n", res.stdout)
        self.assertIn("(not read: ", res.stdout)
        self.save(self.report, b"its kernel release: 9.9-new\n")
        res = self.run_tool()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("does not name the running release once", res.stdout)
        os.remove(os.path.join(self.build, "stick.img"))
        res = self.run_tool()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: no kexec-stick folder with a stick.img", res.stdout)
        self.assertEqual(self.run_tool("--help").returncode, 2)

    def test_only_reads(self):
        with open(TOOL) as fh:
            code = fh.read().split('"""', 2)[2]
        for needle in ("subprocess", "ssh", '"w"', '"wb"', '"xb"', "os.remove", "unlink", "rmtree", "makedirs"):
            self.assertNotIn(needle, code, needle)


if __name__ == "__main__":
    unittest.main(verbosity=2)
