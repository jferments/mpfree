#!/usr/bin/env python3
"""Tests for collect-over-ssh.sh --kexec-preflight against a fake MPC. No device, no network.

ip, ping and ssh are stand-ins, and the network identity is a test-only
address on a made-up interface, so the result never depends on the
workstation's own network (such as the lab MPC in its ARP table). The fake
ssh runs the real
read-only-kexec-preflight.sh under BusyBox applets (as on the MPC), with
stand-ins for kexec and systemctl that log how they were called. It serves
`uname -r` and `cat /sys/firmware/fdt` from the test's fake unit, and refuses
every other remote command.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import glob
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_offline_dtb import board_tree, dtb  # noqa: E402

TOOLS = os.path.dirname(HERE)
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
REMOTE = os.path.join(TOOLS, "read-only-kexec-preflight.sh")
RELEASE = "6.6.95-fake-rt55"
OTHER = "6.18.44-fake-rt6"
SERIAL = "FAKE-SERIAL-7788"
MAC = bytes.fromhex("020000aabbcc")
APPLETS = ("sh", "cat", "grep", "sed", "tr", "wc", "ls", "df", "uname", "head", "zcat", "basename")

FAKE_SSH = r'''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
i = 0
while i < len(args) and args[i].startswith("-"):
    i += 2 if args[i] in ("-o", "-i", "-p", "-l", "-F") else 1
command = args[-1]
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write("ssh: " + command + "\n")
if command == "sh -s":
    env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
    sys.exit(subprocess.run([os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh"), "-s"], env=env).returncode)
if command == "uname -r":
    print(os.environ["FAKE_RELEASE"])
    sys.exit(0)
if command == "cat /sys/firmware/fdt":
    with open(os.environ["FAKE_FDT"], "rb") as fh:
        sys.stdout.buffer.write(fh.read())
    sys.exit(0)
print("fake ssh: command not allowed: " + command, file=sys.stderr)
sys.exit(255)
'''

# Test-only network identity: a documentation address (RFC 5737) on an interface name no real
# machine has, so nothing in the workstation's own ARP table (such as the lab MPC) can match.
TEST_IP, TEST_IFACE, TEST_HOST, TEST_MAC = "192.0.2.141", "mpfreetest0", "192.0.2.1/24", "02:00:00:00:00:01"
FAKE_IP = r'''#!/bin/sh
case "$*" in
  *"addr show dev mpfreetest0"*) echo "    inet 192.0.2.1/24 brd 192.0.2.255 scope global mpfreetest0" ;;
  *"route get 192.0.2.141"*) echo "192.0.2.141 dev mpfreetest0 src 192.0.2.1 uid 1000" ;;
  *"neigh show dev mpfreetest0"*) echo "192.0.2.141 lladdr 02:00:00:00:00:01 REACHABLE" ;;
  *"neigh get 192.0.2.141 dev mpfreetest0"*) echo "192.0.2.141 dev mpfreetest0 lladdr 02:00:00:00:00:01 REACHABLE" ;;
  *) exit 1 ;;
esac
'''

FAKE_KEXEC = r'''#!/bin/sh
echo "kexec: $*" >> "$FAKE_LOG"
case "$1" in
  --version) echo "kexec-tools 2.0.fake" ;;
  --help) printf '%s\n' "Usage: kexec [OPTION]... [kernel]" \
    "  -l, --load           Load the new kernel into the current kernel." \
    "  -e, --exec           Execute a currently loaded kernel." \
    "      --dtb=FILE       Use FILE as the fdt blob of the new kernel." \
    "      --command-line=STRING Set the kernel command line to STRING." \
    "  -x, --no-ifdown      Don't bring down network interfaces." ;;
  *) echo "kexec stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  --version) echo "systemd 255 (255.fake)" ;;
  show) echo "Id=kexec.target"; echo "LoadState=loaded" ;;
  is-system-running) echo "running" ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''


def write(path, data, mode=0o644):
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


class KexecPreflightTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-kexec-preflight-test-")
        self.home = os.path.join(self.tmp, "home")
        self.work = os.path.join(self.home, "mpfree-work")
        self.reports = os.path.join(self.tmp, "research")
        self.log = os.path.join(self.tmp, "commands.log")
        fakebin, remote = os.path.join(self.tmp, "fakebin"), os.path.join(self.tmp, "remote-bin")
        for d in (self.home, fakebin, remote):
            os.makedirs(d)
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        write(os.path.join(fakebin, "ip"), FAKE_IP, 0o755)
        write(os.path.join(fakebin, "ping"), "#!/bin/sh\nexit 0\n", 0o755)
        for applet in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, applet))
        write(os.path.join(remote, "kexec"), FAKE_KEXEC, 0o755)
        write(os.path.join(remote, "systemctl"), FAKE_SYSTEMCTL, 0o755)
        self.key = os.path.join(self.tmp, "key")
        write(self.key, "not a real key\n", 0o600)
        self.fdt = os.path.join(self.tmp, "fdt")
        write(self.fdt, dtb.build(board_tree(serial=SERIAL, mac=MAC)))
        for release, variants in ((RELEASE, ("acva", "acva-c")), (OTHER, ("acva",))):
            folder = os.path.join(self.work, "boot-files", release)
            os.makedirs(folder)
            for variant in variants:
                write(os.path.join(folder, f"rk3288-az01-{variant}.dtb"), dtb.build(board_tree()))
        self.env = {
            "HOME": self.home, "LC_ALL": "C", "PATH": fakebin + os.pathsep + "/usr/bin:/bin",
            "MPC_KEY": self.key, "MPC_REPORT_DIR": self.reports,
            "MPC_IP": TEST_IP, "MPC_IFACE": TEST_IFACE, "MPC_HOST_IP_CIDR": TEST_HOST, "MPC_EXPECT_MAC": TEST_MAC,
            "FAKE_LOG": self.log, "FAKE_REMOTE_PATH": remote, "FAKE_RELEASE": RELEASE, "FAKE_FDT": self.fdt,
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_wrapper(self, **extra):
        return subprocess.run(["bash", WRAPPER, "--kexec-preflight"], env={**self.env, **extra},
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=120)

    def test_report_and_device_tree_copy(self):
        res = self.run_wrapper()
        self.assertEqual(res.returncode, 0, res.stdout)
        [report] = glob.glob(os.path.join(self.reports, "mpc-one-kexec-preflight-*.txt"))
        with open(report) as fh:
            text = fh.read()
        self.assertIn("MPFREE KEXEC PREFLIGHT - READ ONLY", text)
        self.assertIn("END OF KEXEC PREFLIGHT", text)
        self.assertIn("kexec-tools 2.0.fake", text)
        self.assertIn("--dtb=FILE", text)
        self.assertNotIn("--no-ifdown", text, "only the options a kexec boot needs are listed")
        self.assertIn("systemd 255", text)
        self.assertIn("=== Running device tree, compared on the workstation ===", text)
        self.assertIn("model: Akai Professional MPC One", text)
        self.assertIn("property /serial-number: only in B (17 bytes)", text)
        self.assertIn("property /ethernet@ff290000/local-mac-address: only in B (6 bytes)", text)
        self.assertEqual(text.count("--- differences from"), 2, "full diffs only for the running release")
        self.assertIn(f"boot-files/{OTHER}/rk3288-az01-acva.dtb vs the running tree: 3 difference(s)", text)
        self.assertNotIn(SERIAL, text, "the report must not carry the serial number")
        self.assertNotIn(MAC.hex(), text)
        [copy] = glob.glob(os.path.join(self.work, "kexec-preflight-*", "live.dtb"))
        with open(copy, "rb") as a, open(self.fdt, "rb") as b:
            self.assertEqual(a.read(), b.read())
        with open(self.log) as fh:
            calls = fh.read().splitlines()
        self.assertEqual([c for c in calls if c.startswith("ssh: ")],
                         ["ssh: sh -s", "ssh: uname -r", "ssh: cat /sys/firmware/fdt"])
        self.assertEqual(sorted(c for c in calls if c.startswith("kexec: ")), ["kexec: --help", "kexec: --version"])
        for c in calls:
            if c.startswith("systemctl: "):
                self.assertRegex(c, r"^systemctl: (--version|show |is-system-running)")

    def test_without_boot_files_it_says_what_to_run(self):
        shutil.rmtree(os.path.join(self.work, "boot-files"))
        res = self.run_wrapper()
        self.assertEqual(res.returncode, 0, res.stdout)
        [report] = glob.glob(os.path.join(self.reports, "mpc-one-kexec-preflight-*.txt"))
        with open(report) as fh:
            self.assertIn("run offline-rootfs-survey.sh, with and without --from-backup, first", fh.read())

    def test_work_folder_inside_the_clone_is_refused(self):
        inside = os.path.join(os.path.dirname(TOOLS), "kexec-test-not-ignored")
        res = self.run_wrapper(MPFREE_WORK_DIR=inside)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("inside the git clone and not git-ignored", res.stdout)
        self.assertFalse(os.path.exists(inside))
        with open(self.log, "a"):
            pass
        with open(self.log) as fh:
            self.assertEqual(fh.read(), "", "nothing may reach the MPC before the checks pass")


class StaticTests(unittest.TestCase):
    def test_remote_script_only_reads(self):
        with open(REMOTE) as fh:
            lines = [line for line in fh.read().splitlines() if not line.lstrip().startswith("#")]
        code = "\n".join(lines)
        for line in lines:
            if "kexec " in line and "kexec --" in line:
                self.assertRegex(line, r"kexec --(version|help) ")
        for forbidden in ("kexec -l", "kexec -e", "kexec -p", "--load ", "--exec ", "systemctl kexec",
                          "systemctl start", "systemctl stop", "mount ", "of=", "> /dev", ">/dev/mmc", "echo 1 >"):
            self.assertNotIn(forbidden, code)
        subprocess.run(["sh", "-n", REMOTE], check=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
