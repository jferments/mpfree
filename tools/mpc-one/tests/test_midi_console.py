#!/usr/bin/env python3
"""Tests for midi_console.py (collect-over-ssh.sh --midi-console). No device.

The fake ssh runs the tool's real remote loop under BusyBox applets (as on the
MPC) against a fake root folder, setting the script's R='' root prefix to it.
It accepts only the test build's host key alias and `sh -s`. The tests write
the app's log in that folder as MPFree would (lines, a line half written, a
file set aside) and read what the console prints.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import importlib.util
import os
import re
import select
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(TOOLS))
HELPER = os.path.join(TOOLS, "midi_console.py")
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
_spec = importlib.util.spec_from_file_location("midi_console", HELPER)
console = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(console)

FAKE_SSH = r'''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
opts, i = [], 0
while i < len(args) and args[i].startswith("-"):
    if args[i] in ("-o", "-i", "-p", "-l", "-F"):
        opts.append(args[i + 1]); i += 2
    else:
        i += 1
command = args[-1]
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write(f"ssh {' '.join(opts)}: {command}\n")
if "HostKeyAlias=mpfree-kexec-test" not in opts or command != "sh -s":
    print("Host key verification failed.", file=sys.stderr)
    sys.exit(255)
script = sys.stdin.read()
assert script.startswith("R=''"), "every remote script starts with R=''"
script = f"R='{os.environ['FAKE_ROOT']}'" + script[len("R=''"):]
env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
sh = os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh")
sys.exit(subprocess.run([sh, "-c", script], env=env, stdin=subprocess.DEVNULL).returncode)
'''


def line(n, t, port="MIDI", way="in", ch="1", kind="notes", words="Note on · C4 · vel 100",
         went="Bass, the armed track", data="90 3C 64"):
    return "\t".join([str(n), f"{t:.3f}", port, way, ch, kind, words, went, data])


def write(path, text, mode=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(text)
    if mode is not None:
        os.chmod(path, mode)


def opts(*argv):
    return console.options(["--file", "x", *argv])


class Words(unittest.TestCase):
    def test_a_line_in_words_with_where_it_went(self):
        l = console.parse(line(7, 62.5, ch="10", words="Note on · C1 · vel 100", went="Drums, pad 1"))
        self.assertEqual((l.n, l.t, l.port, l.way, l.ch, l.kind), (7, 62.5, "MIDI", "in", 10, "notes"))
        text = console.words(l, opts(), False)
        self.assertEqual(text, "  1:02.500  MIDI in            ch 10  Note on · C1 · vel 100  -> Drums, pad 1")
        self.assertTrue(console.words(l, opts("--hex"), False).endswith("[90 3C 64]"))
        # Colour only when asked for: the message by its kind, Learn purple, "nothing" yellow.
        painted = console.words(console.parse(line(1, 1, kind="cc", went="Learn: Cutoff (Filter, Drums)")),
                                opts(), True)
        self.assertIn("\x1b[36m", painted)
        self.assertIn("\x1b[35mLearn: Cutoff", painted)
        self.assertIn("\x1b[33mnothing takes it",
                      console.words(console.parse(line(1, 1, went="nothing takes it")), opts(), True))
        # No channel, nowhere it went, no bytes: dashes in the file, nothing printed.
        quiet = console.parse(line(2, 3, ch="-", kind="system", words="USB device plugged in", went="-", data="-"))
        self.assertIsNone(quiet.ch)
        self.assertNotIn("->", console.words(quiet, opts("--hex"), False))

    def test_markers_and_cut_lines_are_not_lines(self):
        for text in (console.ROTATED, console.IDLE, "1\t2", "x\t1.0\tMIDI\tin\t1\tnotes\ta\tb\tc"):
            self.assertIsNone(console.parse(text))

    def test_filters_keep_only_what_is_asked(self):
        a = console.parse(line(1, 1))
        cc = console.parse(line(2, 1, kind="cc", ch="2", port="Keystep 37"))
        out = console.parse(line(3, 1, way="out", kind="clock", ch="-"))
        every = [a, cc, out]
        keep = lambda *argv: [l.n for l in every if console.shown(l, opts(*argv))]
        self.assertEqual(keep(), [1, 2, 3])
        self.assertEqual(keep("--notes"), [1])
        self.assertEqual(keep("--notes", "--cc"), [1, 2])
        self.assertEqual(keep("--midi-port", "keystep 37"), [2])
        self.assertEqual(keep("--ch", "2"), [2])
        self.assertEqual(keep("--out"), [3])
        self.assertEqual(keep("--in", "--clock"), [])

    def test_the_kinds_are_the_apps(self):
        """The console's kinds are the names the app gives them (mpfree_midi_ui::log::kind_name)."""
        with open(os.path.join(REPO, "rust", "mpfree-midi-ui", "src", "log.rs")) as fh:
            src = fh.read()
        body = src.split("pub fn kind_name(")[1].split("\n}\n")[0]
        self.assertEqual(sorted(re.findall(r'=> "([a-z]+)"', body)), sorted(console.KINDS))

    def test_a_copied_file_is_printed_once_with_gaps_said(self):
        with tempfile.TemporaryDirectory() as t:
            path = os.path.join(t, "midi.log")
            report = os.path.join(t, "report.txt")
            write(path, "\n".join([line(1, 0.5), line(2, 0.6, kind="cc", words="CC 74 Cutoff · 60"),
                                   line(5, 0.9, words="Note off · C4"), console.ROTATED]) + "\n")
            res = subprocess.run([sys.executable, "-I", HELPER, "--file", path, "--notes", "--report", report],
                                 capture_output=True, text=True, timeout=30)
            self.assertEqual(res.returncode, 0, res.stderr)
            out = res.stdout.splitlines()
            self.assertEqual(len(out), 3, res.stdout)
            self.assertIn("Note on · C4", out[0])
            self.assertEqual(out[1], "  ... 2 lines not kept")
            self.assertIn("Note off · C4", out[2])
            self.assertNotIn("\x1b[", res.stdout, "no colour when not on a terminal")
            with open(report) as fh:
                self.assertEqual(fh.read().splitlines(), out)

    def test_it_needs_a_key_or_a_file(self):
        res = subprocess.run([sys.executable, "-I", HELPER], capture_output=True, text=True, timeout=30)
        self.assertEqual(res.returncode, 2)
        self.assertIn("--key", res.stderr)


class ReadOnly(unittest.TestCase):
    def test_the_remote_loop_only_reads(self):
        script = console.REMOTE
        self.assertTrue(script.startswith("R=''\n"))
        # No output redirection at all: it writes nothing on the MPC, only to its own standard output.
        self.assertNotIn(">", script)
        # Its only redirections: the log opened for reading as fd 3, and reads from it.
        self.assertEqual(re.findall(r"\d?<[&\d]*", script), ["<&3", "<&3", "3<"])
        # Only shell built-ins, its own two functions and variables, and sleep.
        words = set(re.findall(r"^\s*([a-z_]+)", script, re.M)) - {"do", "done", "fi", "then", "else", "return"}
        self.assertEqual(words, {"echo", "follow", "idle", "if", "line", "part", "printf", "quiet", "said", "sleep",
                                 "while"})
        self.assertNotIn("SKIP=0", console.remote_script(False).split("follow()")[0])
        self.assertIn("SKIP=0", console.remote_script(True).split("follow()")[0])

    def test_the_wrapper_runs_it_read_only_on_the_test_builds_port(self):
        with open(WRAPPER) as fh:
            w = fh.read()
        self.assertIn('"--midi-console") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-midi-console" ;;', w)
        self.assertIn("--midi-console (read-only", w)
        self.assertIn('python3 -I "$SCRIPT_DIR/midi_console.py"', w)
        call = w.split('python3 -I "$SCRIPT_DIR/midi_console.py"')[1].split("then")[0]
        self.assertIn('--port "$MPC_TEST_PORT"', call)
        self.assertIn('--work "$work_abs"', call)
        # The filter words are checked one by one against a fixed list before they reach the tool.
        self.assertIn("MPFREE_MIDI_CONSOLE", w)
        self.assertIn("MPFREE_MIDI_PORT", w)


class Follows(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise AssertionError("BusyBox is required (MPFREE_REQUIRE_BUSYBOX=1)")
            raise unittest.SkipTest("BusyBox is not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-console-")
        t = self.tmp
        self.root, self.ssh_log = os.path.join(t, "root"), os.path.join(t, "ssh.log")
        self.dir = os.path.join(self.root, "run", "mpfree-app")
        self.file = os.path.join(self.dir, "midi.log")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in ("sh", "sleep"):
            os.symlink(self.busybox, os.path.join(remote, name))
        write(self.ssh_log, "")
        work = os.path.join(t, "work")
        write(os.path.join(work, "kexec-stick-20260101T000000Z-abc", "stick.img"), "x")
        write(os.path.join(work, "kexec-stick-20260101T000000Z-abc", "known_hosts"),
              "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        self.key = os.path.join(t, "key")
        write(self.key, "fake key\n", 0o600)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"), "HOME": t,
                    "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_LOG": self.ssh_log}
        self.cmd = [sys.executable, "-I", HELPER, "--ip", "10.42.0.141", "--port", "2222", "--user", "root",
                    "--key", self.key, "--work", work]
        self.proc = None

    def tearDown(self):
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def start(self, *argv):
        self.proc = subprocess.Popen(self.cmd + list(argv), env=self.env, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, text=True)
        self.buf = ""

    def read_until(self, text, timeout=15):
        """Printed lines until one holds `text`."""
        end = time.time() + timeout
        out = []
        while time.time() < end:
            if "\n" in self.buf:
                first, self.buf = self.buf.split("\n", 1)
                out.append(first)
                if text in first:
                    return out
                continue
            ready, _, _ = select.select([self.proc.stdout], [], [], 0.2)
            if ready:
                chunk = os.read(self.proc.stdout.fileno(), 65536).decode()
                if not chunk:
                    break
                self.buf += chunk
        self.fail(f"no line with {text!r} in {timeout} s; printed: {out}; stderr: "
                  f"{self.proc.stderr.read() if self.proc.poll() is not None else ''}")

    def append(self, text):
        with open(self.file, "a") as fh:
            fh.write(text)

    def test_it_follows_new_lines_and_opens_the_next_file(self):
        # A file there before it starts: only what comes next is printed.
        write(self.file, line(1, 0.1, words="Note on · old") + "\n")
        self.start("--no-colour")
        self.read_until("Following /run/mpfree-app/midi.log")
        self.append(line(2, 0.2, words="Note on · C4 · vel 90") + "\n")
        got = self.read_until("C4 · vel 90")
        self.assertFalse(any("old" in g for g in got), got)
        # A line half written is printed whole once its end comes.
        self.append(line(3, 0.3, words="Note on · E4 · vel 88")[:20])
        time.sleep(1.5)
        self.append(line(3, 0.3, words="Note on · E4 · vel 88")[20:] + "\n")
        got = self.read_until("E4 · vel 88")
        self.assertEqual(len([g for g in got if "E4" in g]), 1)
        # MPFree sets the file aside (its last line says so) and starts a new one: read from its first line.
        self.append(line(4, 0.4, words="Note on · G4 · vel 90") + "\n" + console.ROTATED + "\n")
        os.rename(self.file, self.file + ".1")
        self.read_until("G4 · vel 90")
        time.sleep(1.5)
        write(self.file, line(9, 0.9, words="Note off · C4") + "\n")
        got = self.read_until("Note off · C4")
        self.assertIn("  ... 4 lines not kept", got)
        self.assertFalse(any("ROTATED" in g for g in got), got)
        with open(self.ssh_log) as fh:
            calls = fh.read().splitlines()
        self.assertEqual(len(calls), 1, calls)
        self.assertIn("HostKeyAlias=mpfree-kexec-test", calls[0])
        self.assertTrue(calls[0].endswith(": sh -s"), calls[0])

    def test_it_waits_for_the_file_and_reads_it_from_its_first_line(self):
        # No file yet (the developer tools are off, or MPFree hasn't listed a line): it says what it waits for.
        self.start("--no-colour")
        self.read_until("Waiting for /run/mpfree-app/midi.log")
        os.makedirs(self.dir)
        write(self.file, line(1, 0.1, words="Note on · first") + "\n")
        got = self.read_until("first")
        self.assertTrue(any("Following" in g for g in got), got)

    def test_from_start_prints_the_whole_file(self):
        write(self.file, line(1, 0.1, words="Note on · first") + "\n" + line(2, 0.2, kind="cc", words="CC 1") + "\n")
        self.start("--no-colour", "--from-start", "--notes")
        got = self.read_until("first")
        self.assertTrue(any("Following" in g for g in got), got)
        self.append(line(3, 0.3, words="Note on · next") + "\n")
        got = self.read_until("next")
        self.assertFalse(any("CC 1" in g for g in got), got)

    def test_the_loop_ends_on_the_mpc_once_the_workstation_has_gone(self):
        """SSH without a terminal sends no hang-up: the loop's next @@IDLE fails to write, and it ends."""
        os.makedirs(self.dir)
        write(self.file, line(1, 0.1) + "\n")
        read_end, write_end = os.pipe()
        script = f"R='{self.root}'" + console.REMOTE[len("R=''"):]
        proc = subprocess.Popen([self.busybox, "sh", "-c", script], stdout=write_end, stdin=subprocess.DEVNULL,
                                env={"PATH": self.env["FAKE_REMOTE_PATH"]})
        os.close(write_end)
        self.assertEqual(os.read(read_end, 100), b"@@FOLLOWING\n")
        os.close(read_end)
        try:
            proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            proc.kill()
            self.fail("the loop went on with nobody reading")

    def test_the_mpc_sees_nothing_written(self):
        write(self.file, line(1, 0.1) + "\n")
        before = sorted(os.listdir(self.dir))
        with open(self.file) as fh:
            content = fh.read()
        self.start("--no-colour")
        self.read_until("Following")
        self.proc.terminate()
        self.proc.wait(timeout=10)
        self.assertEqual(sorted(os.listdir(self.dir)), before)
        with open(self.file) as fh:
            self.assertEqual(fh.read(), content)


if __name__ == "__main__":
    unittest.main(verbosity=2)
