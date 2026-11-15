#!/usr/bin/env python3
"""Tests for kexec-file-probe.sh (collect-over-ssh.sh --hook-kexec-probe).

The probe loads a kernel into RAM and unloads it, several ways, to find which load LoadPin accepts. It must
never execute or boot a kernel, and must always leave kexec_loaded at 0. These tests run it against a fake
root with a fake `kexec` that mimics LoadPin: the old buffer syscall is denied, the file syscall is accepted
for the on-disk trusted kernel and denied for a copy under /run.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip when no POSIX shell is found.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
PROBE = os.path.join(TOOLS, "kexec-file-probe.sh")

FAKE_KEXEC = r'''#!/bin/sh
# Mimic LoadPin: the old buffer syscall is denied; the file syscall is allowed for a kernel on disk and
# denied for one under /run (an untrusted filesystem).
L="$FAKE_ROOT/sys/kernel/kexec_loaded"
mode=buffer; file=
for a in "$@"; do
  case "$a" in
    --kexec-file-syscall) mode=file ;;
    -u|--unload) echo 0 > "$L"; exit 0 ;;
    /*) file=$a ;;
  esac
done
[ "${FAKE_EXECUTED:-}" ] && { echo "FAKE kexec was asked to EXECUTE" > "$FAKE_ROOT/EXECUTED"; }
case " $* " in *" -e "*|*" --exec "*) echo EXEC > "$FAKE_ROOT/EXECUTED"; exit 0 ;; esac
if [ "$mode" = buffer ]; then
  echo "LoadPin: kexec-image old-api-denied" >&2; echo "kexec_load failed: Operation not permitted" >&2; exit 1
fi
case "$file" in
  */run/*) echo "LoadPin: kexec-image loadpin-denied" >&2; echo "kexec_file_load failed: Operation not permitted" >&2; exit 1 ;;
  *) echo 1 > "$L"; exit 0 ;;
esac
'''


def write(path, text, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(text)
    os.chmod(path, mode)


def pick_shell():
    for name in ("dash", "sh", "busybox"):
        p = shutil.which(name)
        if p:
            return [p, "sh"] if name == "busybox" else [p]
    return None


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.shell = pick_shell()
        if not self.shell:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                self.fail("no POSIX shell found")
            self.skipTest("no POSIX shell")
        self.tmp = tempfile.mkdtemp()
        self.root = os.path.join(self.tmp, "root")
        self.rel = os.uname().release
        write(os.path.join(self.root, "proc/sys/kernel/kexec_load_disabled"), "0\n")
        write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "0\n")
        write(os.path.join(self.root, "sys/firmware/fdt"), "FAKEDTB\n")
        write(os.path.join(self.root, "boot/zImage-" + self.rel), "FAKE-KERNEL-BYTES\n")
        write(os.path.join(self.root, "usr/sbin/kexec"), FAKE_KEXEC, 0o755)
        os.makedirs(os.path.join(self.root, "run"))

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_probe(self):
        with open(PROBE) as fh:
            script = fh.read().replace("R=''", f"R='{self.root}'", 1)
        return subprocess.run(self.shell + ["-c", script, "probe"],
                              env={"PATH": "/usr/bin:/bin", "FAKE_ROOT": self.root},
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=60)

    def loaded(self):
        with open(os.path.join(self.root, "sys/kernel/kexec_loaded")) as fh:
            return fh.read().strip()

    def test_probe_reports_each_load_and_leaves_nothing_staged(self):
        res = self.run_probe()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        # The old buffer syscall is refused; the file syscall of the trusted kernel loads.
        self.assertIn("old buffer syscall", out)
        self.assertRegex(out, r"old buffer syscall[^\n]*: refused")
        self.assertRegex(out, r"file syscall, reuse this kernel's command line: LOADED")
        self.assertRegex(out, r"file syscall, our command line: LOADED")
        self.assertRegex(out, r"file syscall, our command line \+ a device tree: LOADED")
        # The same kernel copied to /run (untrusted) is refused.
        self.assertRegex(out, r"copied to /run[^\n]*: refused")
        self.assertIn("RESULT: probe done, nothing is staged", out)
        # Nothing left staged, and the kernel was never executed/booted.
        self.assertEqual(self.loaded(), "0")
        self.assertFalse(os.path.exists(os.path.join(self.root, "EXECUTED")), "the probe must never execute a kernel")

    def test_probe_stops_if_no_trusted_kernel(self):
        os.unlink(os.path.join(self.root, "boot/zImage-" + self.rel))
        res = self.run_probe()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: no", res.stdout)
        self.assertEqual(self.loaded(), "0")

    def test_probe_refuses_when_a_kernel_is_already_staged(self):
        write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "1\n")
        res = self.run_probe()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("already staged", res.stdout)

    def test_script_never_executes_a_kernel(self):
        with open(PROBE) as fh:
            code = fh.read()
        body = "\n".join(l for l in code.splitlines() if not l.lstrip().startswith("#"))
        for forbidden in (" -e ", "--exec", "kexec.target", "reboot", "shutdown", "systemctl"):
            self.assertNotIn(forbidden, body, f"probe must not contain {forbidden!r}")

    def test_parses(self):
        subprocess.run(["sh", "-n", PROBE], check=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
