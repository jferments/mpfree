#!/usr/bin/env python3
"""Tests for results.py: recording steps into one folder and packing it into one zip. No device.

MPFREE_RESULTS_DIR, MPFREE_WORK_DIR and MPC_REPORT_DIR point the tool at
temporary folders, so nothing outside them is touched.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "results.py")
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))


class ResultsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-results-test-")
        self.results = os.path.join(self.tmp, "results")
        self.work = os.path.join(self.tmp, "work")
        self.research = os.path.join(self.tmp, "research")
        os.makedirs(self.work)
        os.makedirs(self.research)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def tool(self, *args, stdin="", results=None):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": self.tmp,
               "MPFREE_RESULTS_DIR": results or self.results, "MPFREE_WORK_DIR": self.work,
               "MPC_REPORT_DIR": self.research}
        return subprocess.run([sys.executable, "-I", TOOL, *args], input=stdin, env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)

    def py(self, code):
        return ["--", sys.executable, "-I", "-c", code]

    def current(self):
        return os.path.join(self.results, "current")

    def read(self, *parts):
        with open(os.path.join(*parts)) as fh:
            return fh.read()

    def write(self, path, data):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
            fh.write(data)

    def test_records_output_answers_and_exit_code(self):
        code = "import sys; print('hello'); a = input('Type WRITE: '); print('got', a); sys.exit(3)"
        res = self.tool("run", "first-step", *self.py(code), stdin="WRITE\n")
        self.assertEqual(res.returncode, 3, res.stdout)
        self.assertIn("hello", res.stdout, "the output still reaches the terminal")
        log = self.read(self.current(), "01-first-step.log")
        self.assertIn("# mpfree results: first-step\n", log)
        self.assertIn("hello\n", log)
        self.assertIn("got WRITE", log)
        self.assertNotIn("\r", log)
        self.assertTrue(log.endswith("exit code 3\n"), log)
        self.assertEqual(oct(os.stat(self.current()).st_mode & 0o777), "0o700")
        res = self.tool("run", "second", *self.py("print('two')"))
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertTrue(os.path.isfile(os.path.join(self.current(), "02-second.log")))

    def test_end_of_input_reaches_the_command(self):
        code = "import sys\ntry:\n    input('answer: ')\nexcept EOFError:\n    print('saw EOF'); sys.exit(1)\n"
        res = self.tool("run", "eof", *self.py(code), stdin="")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("saw EOF", self.read(self.current(), "01-eof.log"))

    def test_new_and_changed_reports_are_copied(self):
        old = os.path.join(self.work, "rootfs-survey-old", "report.txt")
        self.write(old, "old report\n")
        changed = os.path.join(self.research, "mpc-one-kexec-preflight-1.txt")
        self.write(changed, "before\n")
        os.utime(changed, (1, 1))
        code = (f"import os\nopen({os.path.join(self.work, 'kexec-stick-1', 'report.txt')!r}, 'w').write('stick\\n')\n"
                f"open({changed!r}, 'w').write('after\\n')\n"
                f"os.makedirs({os.path.join(self.work, 'bin-1')!r})\n"
                f"open({os.path.join(self.work, 'bin-1', 'report.txt')!r}, 'wb').write(b'\\x00\\x01')\n")
        os.makedirs(os.path.join(self.work, "kexec-stick-1"))
        res = self.tool("run", "build", *self.py(code))
        self.assertEqual(res.returncode, 0, res.stdout)
        names = sorted(os.listdir(self.current()))
        self.assertEqual(names, ["01-build--kexec-stick-1-report.txt", "01-build--mpc-one-kexec-preflight-1.txt",
                                 "01-build.log"], res.stdout)
        self.assertEqual(self.read(self.current(), "01-build--kexec-stick-1-report.txt"), "stick\n")
        self.assertEqual(self.read(self.current(), "01-build--mpc-one-kexec-preflight-1.txt"), "after\n")
        self.assertIn("not copied (not a text report)", res.stdout)

    def test_zip_holds_everything_and_starts_a_new_folder(self):
        self.tool("run", "one", *self.py("print('one')"))
        self.tool("run", "two", *self.py("print('two')"))
        res = self.tool("zip")
        self.assertEqual(res.returncode, 0, res.stdout)
        zips = [f for f in os.listdir(self.results) if f.endswith(".zip")]
        self.assertEqual(len(zips), 1, os.listdir(self.results))
        self.assertIn(f"UPLOAD THIS ONE FILE: {os.path.join(self.results, zips[0])}", res.stdout)
        self.assertFalse(os.path.exists(self.current()))
        stamp = zips[0][len("mpfree-results-"):-len(".zip")]
        kept = os.path.join(self.results, stamp)
        with zipfile.ZipFile(os.path.join(self.results, zips[0])) as zf:
            names = zf.namelist()
            want = ("MANIFEST.txt", "01-one.log", "02-two.log")
            self.assertEqual(names, [f"mpfree-results-{stamp}/{n}" for n in want])
            manifest = zf.read(names[0]).decode()
            for name in ("01-one.log", "02-two.log"):
                data = zf.read(f"mpfree-results-{stamp}/{name}")
                self.assertIn(f"{hashlib.sha256(data).hexdigest()}  {name}", manifest)
                with open(os.path.join(kept, name), "rb") as fh:
                    self.assertEqual(fh.read(), data)
        self.assertIn("clone: ", manifest)
        self.assertIn("python: ", manifest)
        res = self.tool("zip")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("nothing to pack", res.stdout)
        self.tool("run", "three", *self.py("print('three')"))
        self.assertEqual(os.listdir(self.current()), ["01-three.log"])

    def test_refusals(self):
        for args, reason in ((("run", "Bad Name", "--", "true"), "use lowercase letters"),
                             (("run", "x", "--"), "nothing to run"),
                             (("run", "x", "--", "no-such-command-mpfree"), "command not found")):
            res = self.tool(*args)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(reason, res.stdout)
        self.assertFalse(os.path.exists(self.results))
        res = self.tool("run", "x", "--", "true", results=os.path.join(REPO, "research", "results-test"))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("inside this clone", res.stdout)
        self.assertEqual(self.tool("bogus").returncode, 2)


class StaticTests(unittest.TestCase):
    def test_never_contacts_the_mpc_or_touches_devices(self):
        with open(TOOL) as fh:
            code = fh.read().split('"""', 2)[2]  # the code after the docstring
        for needle in ("ssh", "/dev/", "sudo", "rmtree", "os.remove", "unlink"):
            self.assertNotIn(needle, code.replace("sha256", ""), needle)


if __name__ == "__main__":
    unittest.main(verbosity=2)
