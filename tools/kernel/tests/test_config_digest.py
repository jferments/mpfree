#!/usr/bin/env python3
"""Tests for tools/kernel/config-digest.py."""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(os.path.dirname(HERE), "config-digest.py")

CONFIG = "#\n# Automatically generated file; DO NOT EDIT.\n#\n" + "".join(
    f"CONFIG_Y{i:03d}=y\n" for i in range(30)) + "".join(
    f"CONFIG_M{i:03d}=m\n" for i in range(13)) + """\
# CONFIG_KEXEC is not set
CONFIG_CMDLINE="isolcpus=2-3 regulator_ignore_unused clk_ignore_unused"
CONFIG_LOG_BUF_SHIFT=17
CONFIG_ARM_PATCH_PHYS_VIRT=y
CONFIG_PHYS_OFFSET=0x60000000
CONFIG_LSM="landlock,lockdown,yama,loadpin"
"""


def run(*args):
    return subprocess.run([sys.executable, "-I", SCRIPT, *args], capture_output=True, text=True)


def run_on(mode, text):
    with tempfile.TemporaryDirectory() as tmp:
        p = os.path.join(tmp, "in")
        with open(p, "w") as fh:
            fh.write(text)
        return run(mode, p)


def set_lines(text):
    return "".join(sorted(line + "\n" for line in text.splitlines()
                          if line.startswith("CONFIG_") and "=" in line))


class DigestTests(unittest.TestCase):
    def setUp(self):
        r = run_on("print", CONFIG)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.out = r.stdout

    def test_the_layout(self):
        lines = self.out.splitlines()
        self.assertRegex(lines[0], r"^config-digest 1: 48 set \(31 =y, 13 =m, 4 other\); sha256 [0-9a-f]{64}$")
        self.assertEqual(lines[-1], "config-digest end")
        ys = [x for x in lines if x.startswith("y ")]
        self.assertEqual([x.split()[1] for x in ys], ["001", "002", "003"])
        self.assertEqual(len(ys[0].split()) - 3, 12)
        self.assertIn("ARM_PATCH_PHYS_VIRT", ys[0])
        ms = [x for x in lines if x.startswith("m ")]
        self.assertEqual(len(ms), 2)
        vs = [x for x in lines if x.startswith("v ")]
        self.assertEqual([x.split(" ", 3)[3] for x in vs], [
            'CMDLINE="isolcpus=2-3 regulator_ignore_unused clk_ignore_unused"', "LOG_BUF_SHIFT=17",
            'LSM="landlock,lockdown,yama,loadpin"', "PHYS_OFFSET=0x60000000"])
        self.assertNotIn("KEXEC", self.out)

    def test_read_gives_the_set_lines_back(self):
        r = run_on("read", self.out)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout, set_lines(CONFIG))

    def test_read_takes_the_job_logs_time_stamps(self):
        stamped = "".join(f"2026-11-17T09:20:01.1234567Z {x}\n" for x in self.out.splitlines())
        r = run_on("read", "noise before\n" + stamped + "2026-11-17T09:20:02.0000000Z noise after\n")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout, set_lines(CONFIG))

    def test_a_mistyped_name_names_its_line(self):
        r = run_on("read", self.out.replace("Y005", "Y05", 1))
        self.assertEqual(r.returncode, 1)
        self.assertIn("y 001: the text does not match its hash", r.stderr)
        self.assertIn("DIGEST-FAILED", r.stderr)
        self.assertEqual(r.stdout, "")

    def test_a_changed_value_fails(self):
        r = run_on("read", self.out.replace("LOG_BUF_SHIFT=17", "LOG_BUF_SHIFT=18"))
        self.assertEqual(r.returncode, 1)
        self.assertIn("v 002: the text does not match its hash", r.stderr)

    def test_a_missing_line_fails(self):
        lines = self.out.splitlines()
        cut = [x for x in lines if not x.startswith("y 002 ")]
        r = run_on("read", "\n".join(cut) + "\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("y 003 follows y 001", r.stderr)
        self.assertIn("the header says", r.stderr)

    def test_a_copy_cut_short_fails(self):
        r = run_on("read", self.out.replace("config-digest end\n", ""))
        self.assertEqual(r.returncode, 1)
        self.assertIn("cut short", r.stderr)

    def test_no_header_fails(self):
        r = run_on("read", "y 001 00000000 A\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("no `config-digest 1:` line", r.stderr)

    def test_usage_and_missing_file(self):
        self.assertEqual(run().returncode, 2)
        self.assertEqual(run("other", "/nonexistent").returncode, 2)
        r = run("print", "/nonexistent/config")
        self.assertEqual(r.returncode, 2)
        self.assertIn("STOP:", r.stderr)

    def test_an_empty_config_stops(self):
        r = run_on("print", "# nothing\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("no CONFIG_ lines", r.stderr)


if __name__ == "__main__":
    unittest.main()
