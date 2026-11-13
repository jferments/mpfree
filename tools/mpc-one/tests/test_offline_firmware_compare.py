#!/usr/bin/env python3
"""Tests for offline-firmware-compare.py, on hand-written survey reports and on
reports offline-rootfs-survey.sh makes from small fake rootfs images. No device.

Set MPFREE_REQUIRE_OFFLINE_TOOLS=1 (CI does) to fail instead of skip when the
survey's tools are missing.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
TOOL = os.path.join(os.path.dirname(HERE), "offline-firmware-compare.py")
SURVEY = os.path.join(os.path.dirname(HERE), "offline-rootfs-survey.sh")
A, B, C = "a" * 64, "b" * 64, "c" * 64


def report(source, fw_lines, version="9.9.9", section=True):
    text = ["MPFree offline rootfs survey (no MPC contact)", "work folder: /x", f"source: {source}", "sha256 0", "",
            "=== 1. Identity ===", "--- /etc/version", version, ""]
    if section:
        text.append("=== 10. Peripheral firmware files (sha256; offline-firmware-compare.py compares two reports) ===")
        text += fw_lines
        text.append("")
    text += ["DONE. Report: /x/report.txt", "It quotes Akai's files: keep it private and commit only findings.", ""]
    return "\n".join(text)


class CompareTests(unittest.TestCase):
    def setUp(self):
        self.base = tempfile.mkdtemp(prefix="mpfree-fwcmp-test-")
        self.work = os.path.join(self.base, "mpfree-work")

    def tearDown(self):
        shutil.rmtree(self.base, ignore_errors=True)

    def put(self, folder, text):
        path = os.path.join(self.work, folder, "report.txt")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as fh:
            fh.write(text)
        return path

    def run_tool(self, *args):
        env = dict(os.environ, MPFREE_WORK_DIR=self.work, LC_ALL="C")
        return subprocess.run([sys.executable, "-I", TOOL, *args], env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=60)

    def snapshot(self):
        out = {}
        for root, _, files in os.walk(self.work):
            for name in files:
                path = os.path.join(root, name)
                with open(path, "rb") as fh:
                    out[path] = (fh.read(), os.stat(path).st_mtime_ns)
        return out

    def test_same_files_pass(self):
        old = self.put("rootfs-survey-20261113T100000Z-aaaa",
                       report("p6 of /b/mpc-one-emmc-1", [f"fw {A} /usr/lib/firmware/ilitek/ili2117.bin",
                                                          f"fw {B} /usr/share/Akai/SME0/Firmware/fw.bin",
                                                          f"fw {C} /usr/lib/firmware/old-only.bin"], version="3.6.0"))
        new = self.put("rootfs-survey-20261113T110000Z-bbbb",
                       report("/w/akai-unpacked-1/rootfs.img", [f"fw {A} /usr/lib/firmware/ilitek/ili2117.bin",
                                                                f"fw {B} /usr/share/Akai/SME0/Firmware/fw.bin",
                                                                "fw-link /usr/lib/firmware/x.bin -> ilitek/ili2117.bin"],
                              version="3.10.1"))
        before = self.snapshot()
        res = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"old: {old}", res.stdout)
        self.assertIn(f"new: {new}", res.stdout)
        self.assertIn("version 3.6.0; 3 firmware files", res.stdout)
        self.assertIn("version 3.10.1; 2 firmware files", res.stdout)
        self.assertIn("same      /usr/lib/firmware/ilitek/ili2117.bin", res.stdout)
        self.assertIn("gone      /usr/lib/firmware/old-only.bin", res.stdout)
        self.assertIn("2 same, 0 different, 0 new, 1 gone, 0 in the new version not compared", res.stdout)
        self.assertIn("VERDICT: SAME.", res.stdout)
        self.assertEqual(self.snapshot(), before, "the reports must not change")

    def test_differences_stop(self):
        self.put("rootfs-survey-20261113T100000Z-aaaa",
                 report("p6 of /b/mpc-one-emmc-1", [f"fw {A} /usr/share/Akai/SME0/Firmware/fw.bin"]))
        self.put("rootfs-survey-20261113T110000Z-bbbb",
                 report("/w/rootfs.img", [f"fw {B} /usr/share/Akai/SME0/Firmware/fw.bin",
                                          f"fw {C} /usr/lib/firmware/xmos/new.bin"]))
        res = self.run_tool()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("DIFFERENT /usr/share/Akai/SME0/Firmware/fw.bin", res.stdout)
        self.assertIn("NEW       /usr/lib/firmware/xmos/new.bin", res.stdout)
        self.assertIn("VERDICT: STOP.", res.stdout)

    def test_skipped_lines_in_the_new_version_stop(self):
        old = self.put("o", report("p6 of /b", [f"fw {A} /usr/lib/firmware/a.bin"]))
        new = self.put("n", report("/w/rootfs.img", [f"fw {A} /usr/lib/firmware/a.bin",
                                                     "fw-skipped /usr/lib/firmware/xmos/a/b/ (too deep)"]))
        res = self.run_tool(old, new)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("not compared (new): fw-skipped /usr/lib/firmware/xmos/a/b/ (too deep)", res.stdout)
        self.assertIn("1 in the new version not compared", res.stdout)
        self.assertIn("VERDICT: STOP.", res.stdout)
        # A malformed line (no hash: the survey could not read the file) is not compared either.
        new = self.put("n2", report("/w/rootfs.img", [f"fw {A} /usr/lib/firmware/a.bin", "fw  /usr/lib/firmware/b.bin"]))
        res = self.run_tool(old, new)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("not compared (new): fw  /usr/lib/firmware/b.bin", res.stdout)

    def test_an_empty_new_list_stops(self):
        old = self.put("o", report("p6 of /b", [f"fw {A} /usr/lib/firmware/a.bin"]))
        new = self.put("n", report("/w/rootfs.img", []))
        res = self.run_tool(old, new)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("lists no firmware files at all", res.stdout)

    def test_errors(self):
        res = self.run_tool()
        self.assertEqual(res.returncode, 2, res.stdout)
        self.assertIn("made with --from-backup", res.stdout)
        self.put("rootfs-survey-20261113T100000Z-aaaa", report("p6 of /b", [f"fw {A} /x"]))
        res = self.run_tool()
        self.assertEqual(res.returncode, 2, res.stdout)
        self.assertIn("made with --image", res.stdout)
        self.put("rootfs-survey-20261113T110000Z-bbbb", report("/w/rootfs.img", [], section=False))
        res = self.run_tool()
        self.assertEqual(res.returncode, 2, res.stdout)
        self.assertIn("has no section 10", res.stdout)
        res = self.run_tool(os.path.join(self.base, "missing.txt"), os.path.join(self.base, "missing2.txt"))
        self.assertEqual(res.returncode, 2, res.stdout)
        self.assertIn("cannot read", res.stdout)
        junk = self.put("junk", "hello\n")
        res = self.run_tool(junk, junk)
        self.assertEqual(res.returncode, 2, res.stdout)
        self.assertIn("not an offline-rootfs-survey.sh report", res.stdout)
        twice = self.put("twice", report("/w", [f"fw {A} /x", f"fw {B} /x"]))
        res = self.run_tool(twice, twice)
        self.assertEqual(res.returncode, 2, res.stdout)
        self.assertIn("lists /x twice", res.stdout)
        res = self.run_tool(junk)
        self.assertEqual(res.returncode, 2, res.stdout)
        self.assertIn("give both reports or neither", res.stdout)

    def test_newest_reports_are_picked(self):
        self.put("rootfs-survey-20261101T000000Z-aaaa", report("p6 of /b/old", [f"fw {C} /x"]))
        newest_old = self.put("rootfs-survey-20261112T000000Z-bbbb", report("p6 of /b/new", [f"fw {A} /x"]))
        self.put("rootfs-survey-20261102T000000Z-cccc", report("/w/old.img", [f"fw {C} /x"]))
        newest_new = self.put("rootfs-survey-20261113T000000Z-dddd", report("/w/new.img", [f"fw {A} /x"]))
        res = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"old: {newest_old}", res.stdout)
        self.assertIn(f"new: {newest_new}", res.stdout)


def survey_tools_present():
    from test_offline_akai_updater import tools_present
    return tools_present()


@unittest.skipUnless(survey_tools_present(), "needs e2fsprogs, squashfs-tools, file and binutils")
class WithTheSurveyTests(unittest.TestCase):
    """The survey's section 10 and this tool agree on the format."""

    def test_reports_from_the_survey(self):
        from test_offline_akai_updater import SBIN_PATH, run, write
        from test_offline_rootfs_survey import build_image
        base = tempfile.mkdtemp(prefix="mpfree-fwcmp-survey-")
        try:
            os.chmod(base, 0o755)
            first = build_image(base, "one.img")
            root = os.path.join(base, "root-one.img")
            write(os.path.join(root, "usr/share/Akai/SME0/Firmware/fw.bin"), b"a newer controller firmware")
            second = os.path.join(base, "two.img")
            with open(second, "wb") as fh:
                fh.truncate(16 << 20)
            run(["mkfs.ext4", "-q", "-F", "-b", "1024", "-O", "^has_journal", "-d", root, second])
            home = os.path.join(base, "home")
            os.makedirs(home)
            env = {k: v for k, v in os.environ.items() if k not in ("MPC_BACKUP_DIR", "MPFREE_WORK_DIR")}
            env.update(HOME=home, LC_ALL="C", PATH=SBIN_PATH)
            reports = []
            for image in (first, second, first):
                res = subprocess.run(["bash", SURVEY, "--image", image], env=env, stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, text=True, timeout=300)
                self.assertEqual(res.returncode, 0, res.stdout)
                line = [x for x in res.stdout.splitlines() if x.startswith("DONE. Report: ")][-1]
                reports.append(line[len("DONE. Report: "):])
            cmp = [sys.executable, "-I", TOOL]
            res = subprocess.run(cmp + [reports[0], reports[2]], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                 text=True, timeout=60, env=env)
            # The fixture has a too-deep folder and an odd name, so the same image still stops; the rest is same.
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("same      /usr/share/Akai/SME0/Firmware/fw.bin", res.stdout)
            self.assertIn("same      /usr/lib/firmware/ilitek/fw.bin", res.stdout)
            self.assertIn("0 different, 0 new", res.stdout)
            res = subprocess.run(cmp + [reports[0], reports[1]], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                 text=True, timeout=60, env=env)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("DIFFERENT /usr/share/Akai/SME0/Firmware/fw.bin", res.stdout)
            self.assertIn("VERDICT: STOP.", res.stdout)
        finally:
            shutil.rmtree(base, ignore_errors=True)


class StaticTests(unittest.TestCase):
    def test_tool_only_reads(self):
        with open(TOOL) as fh:
            code = fh.read()
        for forbidden in ("subprocess", "socket", "open(path, \"w", "os.remove", "shutil"):
            self.assertNotIn(forbidden, code)


if __name__ == "__main__":
    unittest.main(verbosity=2)
