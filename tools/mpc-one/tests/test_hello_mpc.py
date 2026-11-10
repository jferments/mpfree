#!/usr/bin/env python3
"""Tests for host_hello_mpc.py (collect-over-ssh.sh --hello) against a fake test build. No device.

The fake ssh runs the tool's real remote scripts under BusyBox applets (as on
the MPC) against a fake root folder, setting the scripts' R='' root prefix to
it. It accepts only the test build's host key alias, and only while the fake
MPC is "on the test build", as the real host keys do. Running the program is
the one other change: the fake ssh runs a shell stand-in instead of the ARM
program, which logs its arguments and prints what the real one would.

The static tests also read the program's Rust source, to keep what it opens
for writing to a short reviewed list.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import hashlib
import os
import pty
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(TOOLS))
HELPER = os.path.join(TOOLS, "host_hello_mpc.py")
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
HELLO_SRC = os.path.join(REPO, "rust", "mpfree-hello", "src")
NEW = "6.18.44-fake-rt6"
APPLETS = ("sh", "cat", "grep", "readlink", "ls", "df", "tail", "sha256sum", "rm", "mkdir", "mv", "chmod")

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
test = "HostKeyAlias=mpfree-kexec-test" in opts
with open(os.environ["FAKE_STAGE"]) as fh:
    stage = fh.read().strip()
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write(f"ssh[{'test' if test else '3.6'}@{stage}]: {command}\n")
if not test or stage != "310":
    print("Host key verification failed.", file=sys.stderr)
    sys.exit(255)
root = os.environ["FAKE_ROOT"]
env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
sh = os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh")
script = sys.stdin.read() if command == "sh -s" else command
assert script.startswith("R=''"), "every remote script starts with R=''"
script = f"R='{root}'" + script[len("R=''"):]
program = 'exec "$R/run/mpfree-hello/mpfree-hello"'
if program in script:
    script = script.replace(program, 'exec "$FAKE_PROGRAM"')
sys.exit(subprocess.run([sh, "-c", script], env=env, stdin=None if command == "sh -s" else sys.stdin).returncode)
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  is-active)
    case "$2" in
      mpfree-test-screen.service) cat "$FAKE_SCREEN" ;;
      acvs.service) echo "$FAKE_ACVS" ;;
      *) echo inactive ;;
    esac ;;
  stop) [ "$2" = mpfree-test-screen.service ] && echo inactive > "$FAKE_SCREEN" ;;
  start) [ "$2" = mpfree-test-screen.service ] && echo active > "$FAKE_SCREEN" ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

FAKE_PROGRAM = r'''#!/bin/sh
echo "program: $*" >> "$FAKE_LOG"
echo "===== mpfree-hello $1 =====" >> "$FAKE_REPORT"
case "$1" in
  inventory) echo "audio card for the click: 1 (ACVA)"; echo "inventory done" ;;
  screen) echo "Hello MPC."; echo "screen: card drawn" ;;
  touch) echo "touch: 1 contact(s): slot 0: x 100 y 200"; echo "touch done: 40 events, 10 frames, 3 lines shown" ;;
  pads)
    case "$*" in *--keepalive*) echo "sent keep-alive 1"; h=$FAKE_PADS_KEEPALIVE ;; *) h=$FAKE_PADS ;; esac
    echo "pads done: 30 bytes, 10 events, $h pad hits, 0 keep-alives sent, 0 replies" ;;
  click) echo "click done: 3445 periods (10.0 s), 0 underruns, 0 late periods"; exit "$FAKE_CLICK_RC" ;;
esac
'''


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


def arm_elf(machine=40, flags=0x05000400, interp=False):
    """A minimal 32-bit ELF header plus one program header (PT_LOAD, or PT_INTERP if asked)."""
    header = bytearray(52)
    header[:7] = b"\x7fELF\x01\x01\x01"
    struct.pack_into("<HHI", header, 16, 2, machine, 1)
    struct.pack_into("<I", header, 28, 52)          # e_phoff
    struct.pack_into("<I", header, 36, flags)
    struct.pack_into("<HHH", header, 40, 52, 32, 1)  # e_ehsize, e_phentsize, e_phnum
    ph = bytearray(32)
    struct.pack_into("<I", ph, 0, 3 if interp else 1)
    return bytes(header + ph) + os.urandom(4000)


class HelloTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-hello-test-")
        t = self.tmp
        self.work, self.log, self.stage = os.path.join(t, "work"), os.path.join(t, "log"), os.path.join(t, "stage")
        self.root, self.report = os.path.join(t, "root310"), os.path.join(t, "report.txt")
        self.screen = os.path.join(t, "screen-state")
        write(self.stage, "310\n")
        write(self.log, "")
        write(self.screen, "active\n")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, name))
        write(os.path.join(remote, "systemctl"), FAKE_SYSTEMCTL, 0o755)
        program_standin = os.path.join(t, "program-standin")
        write(program_standin, FAKE_PROGRAM, 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
                    "HOME": t, "MPFREE_HELLO_TEST": "1", "FAKE_STAGE": self.stage, "FAKE_LOG": self.log,
                    "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SCREEN": self.screen,
                    "FAKE_PROGRAM": program_standin, "FAKE_ACVS": "inactive", "FAKE_PADS": "3",
                    "FAKE_PADS_KEEPALIVE": "2", "FAKE_CLICK_RC": "0",
                    "FAKE_REPORT": os.path.join(self.root, "run/mpfree-hello/hello.txt")}
        self.key = os.path.join(t, "key")
        write(self.key, "fake key\n", 0o600)
        self.program = os.path.join(t, "mpfree-hello")
        write(self.program, arm_elf(), 0o755)
        self.build()
        self.make_root()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def build(self):
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        write(os.path.join(folder, "stick.img"), b"\x00" * 4096)
        write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        write(os.path.join(folder, "report.txt"), f"its kernel release: {NEW}\nrunning release: 6.6.95-fake\n")

    def make_root(self, mmc=False, run_opts="rw,nosuid,nodev,mode=755"):
        r = self.root
        shutil.rmtree(r, ignore_errors=True)
        write(f"{r}/proc/sys/kernel/osrelease", NEW + "\n")
        write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        for name in ("sda", "loop0") + (("mmcblk0",) if mmc else ()):
            os.makedirs(f"{r}/sys/block/{name}")
        write(f"{r}/proc/mounts", "/dev/root / ext4 ro,relatime 0 0\n"
                                  f"tmpfs /run tmpfs {run_opts} 0 0\n"
                                  "tmpfs /data tmpfs rw,size=262144k,mode=755 0 0\n")
        write(f"{r}/proc/321/comm", "mpfree-screen-l\n")
        os.makedirs(f"{r}/proc/321/fd")
        os.symlink("/dev/fb0", f"{r}/proc/321/fd/3")
        os.symlink("/dev/null", f"{r}/proc/321/fd/0")
        os.makedirs(f"{r}/run")

    def run_helper(self, answers="\n" * 12, program=None, **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--program", program or self.program]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def log_lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def test_runs_every_step_in_order(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("static ARM EABI5 hard-float", f"release: {NEW}", "no MMC block device", "/: read-only",
                     "/run: tmpfs rw,nosuid,nodev,mode=755", "Akai's app (acvs.service): inactive",
                     "/dev/fb0: pid 321 (mpfree-screen-l)", "CHECKS-OK", "its hash on the MPC matches",
                     "inventory done", "log display: inactive", "screen: card drawn", "touch done:",
                     "3 pad hits arrived without the keep-alive, so it is not sent.", "click done:",
                     "Starting the log display again:", "log display: active", "| ===== mpfree-hello click =====",
                     "RESULT: all steps ran", "NEXT: switch the MPC off with its power switch"):
            self.assertIn(text, out)
        self.assertEqual(self.log_lines("program: "),
                         ["inventory", "screen", "touch --seconds 10", "pads --seconds 15",
                          "click --seconds 10 --gain-db -12"])
        self.assertEqual(self.log_lines("systemctl: "),
                         ["is-active acvs.service", "is-active mpfree-test-screen.service",
                          "stop mpfree-test-screen.service", "is-active mpfree-test-screen.service",
                          "start mpfree-test-screen.service", "is-active mpfree-test-screen.service"])
        with open(self.program, "rb") as a, open(f"{self.root}/run/mpfree-hello/mpfree-hello", "rb") as b:
            self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
        self.assertEqual(os.stat(f"{self.root}/run/mpfree-hello/mpfree-hello").st_mode & 0o777, 0o700)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-hello/mpfree-hello.part"))
        with open(self.report) as fh:
            self.assertIn("RESULT: all steps ran", fh.read())

    def test_keepalive_only_without_pad_hits(self):
        res = self.run_helper(FAKE_PADS="0")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("No pad hits arrived.", res.stdout)
        self.assertEqual(self.log_lines("program: "),
                         ["inventory", "screen", "touch --seconds 10", "pads --seconds 15",
                          "pads --seconds 15 --keepalive", "click --seconds 10 --gain-db -12"])

    def test_skips_and_stops(self):
        # inventory: skip; screen: skip; touch: go (and a note); pads: quit. The log display was never stopped.
        res = self.run_helper("s\ns\n\nyes, both fingers\nq\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.log_lines("program: "), ["touch --seconds 10"])
        self.assertNotIn("stop mpfree-test-screen.service", self.log_lines("systemctl: "))
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertIn("your answer: yes, both fingers", res.stdout)
        # inventory: go; screen: go (and a note); touch: quit. The log display is started again.
        write(self.log, "")
        res = self.run_helper("\n\ny\nq\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.log_lines("program: "), ["inventory", "screen"])
        self.assertEqual(self.log_lines("systemctl: ")[-2:],
                         ["start mpfree-test-screen.service", "is-active mpfree-test-screen.service"])
        with open(self.screen) as fh:
            self.assertEqual(fh.read().strip(), "active")

    def test_other_answers_ask_again(self):
        res = self.run_helper("yes\n" + "\n" * 12)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Please press just Enter, or s, or q.", res.stdout)

    def test_a_failed_step_is_reported_and_the_rest_go_on(self):
        res = self.run_helper(FAKE_CLICK_RC="1")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("click: exit code 1", res.stdout)

    def test_refuses_anything_but_the_test_build(self):
        write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-hello"))
        self.assertEqual(self.log_lines("program: "), [])

    def test_checks_stop_before_anything_is_copied(self):
        for kwargs, env, problem in (
                ({"mmc": True}, {}, "an MMC block device exists"),
                ({"run_opts": "rw,nosuid,nodev,noexec"}, {}, "/run does not allow programs (noexec)"),
                ({}, {"FAKE_ACVS": "active"}, "Akai's app is running")):
            self.make_root(**kwargs)
            res = self.run_helper(**env)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(f"PROBLEM: {problem}", res.stdout)
            self.assertIn("the test build is not ready; nothing was changed on it", res.stdout)
            self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-hello"))
        self.assertEqual(self.log_lines("program: "), [])

    def test_bad_programs_stop_before_contacting_the_mpc(self):
        for data, why in ((arm_elf(machine=62), "not an ARM executable"),
                          (arm_elf(flags=0x05000200), "not EABI5 hard-float"),
                          (arm_elf(interp=True), "dynamically linked"),
                          (b"#!/bin/sh\necho hi\n" * 10, "not a 32-bit little-endian ELF")):
            write(self.program, data, 0o755)
            res = self.run_helper()
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(why, res.stdout)
        res = self.run_helper(program=os.path.join(self.tmp, "missing"))
        self.assertIn("build it first", res.stdout)
        self.assertEqual(self.log_lines("ssh["), [])

    def test_wrapper_needs_a_terminal_and_the_opt_in(self):
        env = dict(self.env, MPC_KEY=self.key)
        res = subprocess.run(["bash", WRAPPER, "--hello"], input="", env=env, text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("run it from a terminal", res.stdout)
        master, slave = pty.openpty()
        try:
            res = subprocess.run(["bash", WRAPPER, "--hello"], stdin=slave, env=env, text=True,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        finally:
            os.close(slave)
            os.close(master)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("MPFREE_ACCEPT_HELLO=YES", res.stdout)
        self.assertEqual(self.log_lines("ssh["), [])


class StaticTests(unittest.TestCase):
    def test_remote_scripts_change_only_ram(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("host_hello_mpc", HELPER)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        names = ("CHECK", "PREPARE", "RECEIVE", "HASH", "RUN", "SCREEN_STOP", "SCREEN_START", "REPORT")
        scripts = {name: getattr(mod, name) for name in names}
        for name, text in scripts.items():
            self.assertTrue(text.startswith("R=''"), name)
            self.assertIsNone(re.search(r"(?<![0-9])>\s*\S*/dev/", text), f"{name}: writes to a device")
            self.assertIsNone(re.search(r"(^|[;&|(]\s*)(mkfs|mke2fs|sfdisk|fdisk|parted|wipefs)", text, re.M), name)
            for needle in ("of=", " dd ", "fw_setenv", "mount ", "kexec --", "systemctl kexec", "flash", "/boot/", "echo 1 >", "tee ",
                           "mmcblk0p", "/dev/sd"):
                self.assertNotIn(needle, text, f"{name}: {needle}")
            self.assertIsNone(re.search(r"(^|[;&|(]\s*|systemctl\s+)(reboot|poweroff|halt)\b", text, re.M), name)
        writes = [ln for t in scripts.values() for ln in t.splitlines() if re.search(r"(^|[^2])>[^&]", ln)]
        self.assertEqual(writes, [mod.RECEIVE], writes)
        self.assertIn('cat > "$R/run/mpfree-hello/mpfree-hello.part"', mod.RECEIVE)
        services = re.findall(r"systemctl (stop|start|restart|enable|disable|mask|unmask|kill) (\S+)",
                              " ".join(scripts.values()))
        self.assertEqual(sorted(services), [("start", "mpfree-test-screen.service"),
                                            ("stop", "mpfree-test-screen.service")])
        # The program's arguments are fixed in the tool; nothing from the command line reaches the MPC.
        self.assertEqual(sorted(mod.PHASES), ["click", "inventory", "pads", "pads-keepalive", "screen", "touch"])
        for args, _ in mod.PHASES.values():
            self.assertRegex(args, r"^[a-z]+( --[a-z-]+( -?[0-9]+)?)*$")

    def test_the_program_writes_only_where_reviewed(self):
        """Every place the program opens a file for writing, in its Rust source."""
        found = []
        for name in sorted(os.listdir(HELLO_SRC)):
            with open(os.path.join(HELLO_SRC, name)) as fh:
                for line in fh:
                    if re.search(r"\.(write|append|create|create_new|truncate)\(true\)", line):
                        found.append((name, line.strip()))
                    for needle in ("mmcblk", "/dev/sd", "/boot", "mount(", "reboot", "fw_setenv", "/dev/mtd",
                                   "/sbin/kexec", "/sys/class/block", "EVIOCGRAB", "remove_file", "remove_dir"):
                        self.assertNotIn(needle, line, f"{name}: {line.strip()}")
        texts = [line for _, line in found]
        expected = (
            # the program's own report in /run/mpfree-hello (Out::new and the STOP line in main)
            "OpenOptions::new().create(true).append(true).mode(0o644).open(Path::new(REPORT_DIR).join(\"hello.txt\")).ok()",
            "if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(Path::new(REPORT_DIR).join(\"hello.txt\")) {",
            # the framebuffer and its sysfs pan file
            "let fb = OpenOptions::new().write(true).open(&o.fb).map_err(|e| format!(\"{}: {e}\", o.fb.display()))?;",
            "let _ = OpenOptions::new().write(true).open(o.sysfs.join(\"pan\")).and_then(|mut f| f.write_all(b\"0,0\\n\"));",
            # the host-only preview file (never on the MPC)
            "OpenOptions::new().write(true).create_new(true).open(path).and_then(|mut f| f.write_all(&ppm))",
            # the ACVA playback device
            "let file = OpenOptions::new().read(true).write(true).custom_flags(sys::O_NONBLOCK).open(path)?;",
            # the controller's MIDI port, output side (keep-alive only)
            "Direction::Output => opts.write(true),",
        )
        self.assertEqual(sorted(texts), sorted(expected), "\n".join(texts))

    def test_unsafe_code_only_in_sys(self):
        """The C calls (ioctl, fcntl, sched_setscheduler, mlockall) are the only unsafe code."""
        for name in sorted(os.listdir(HELLO_SRC)):
            with open(os.path.join(HELLO_SRC, name)) as fh:
                text = fh.read()
            if name == "main.rs":
                self.assertIn("#![deny(unsafe_code)]", text)
            if name != "sys.rs":
                self.assertNotIn("unsafe {", text, name)
                self.assertNotIn("allow(unsafe_code)", text, name)
        with open(os.path.join(HELLO_SRC, "sys.rs")) as fh:
            sys_rs = fh.read()
        self.assertEqual(sorted(set(re.findall(r"\bfn (\w+)\(", sys_rs.split("extern \"C\" {")[1].split("}")[0]))),
                         ["fcntl", "ioctl", "mlockall", "sched_setscheduler"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
