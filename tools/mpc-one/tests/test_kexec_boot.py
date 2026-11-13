#!/usr/bin/env python3
"""Tests for host_kexec_boot.py (collect-over-ssh.sh --kexec-boot) against a fake MPC. No device.

The fake ssh runs the tool's real remote scripts under BusyBox applets (as on
the MPC), against two fake root folders: one for the running 3.6 and one for
the test build. It sets the scripts' R='' root prefix to the fake root, which
is the only change made to them. Stand-ins for kexec and systemctl log their
calls and move the fake MPC between "3.6", "jumped to the test build", and the
failure cases. A connection using the test build's host key alias only
succeeds once the fake MPC has jumped, as with the real host keys.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import hashlib
import os
import pty
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_offline_dtb import board_tree, dtb  # noqa: E402

TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_kexec_boot.py")
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
OLD, NEW = "6.6.95-fake-rt55", "6.18.44-fake-rt6"
P1 = "11111111-2222-4333-8444-555555555555"
CMDLINE = f"root=PARTUUID={P1} rootwait=60 ro panic=10 rfkill.default_state=0 sysctl.kernel.kexec_load_disabled=1"
APPLETS = ("sh", "cat", "grep", "readlink", "find", "df", "tail", "sha256sum", "rm", "rmdir", "mkdir",
           "sed", "tr", "ls", "head", "dmesg", "ip")
TEST_IP, TEST_IFACE, TEST_MAC = "192.0.2.141", "mpfreetest0", "02:00:00:00:00:01"

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
if stage == "dead" or (test and stage != "310") or (not test and stage == "310"):
    print("Host key verification failed." if stage != "dead" else "ssh: connect to host: No route to host",
          file=sys.stderr)
    sys.exit(255)
root = os.environ["FAKE_ROOT310" if test else "FAKE_ROOT36"]
env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
with open(os.path.join(root, "proc/sys/kernel/osrelease"), "w") as fh:
    fh.write(os.environ["FAKE_NEW" if test else "FAKE_OLD"] + "\n")
sh = os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh")
if command == "sh -s":
    script = sys.stdin.read()
    assert script.startswith("R=''\n"), "every remote script starts with R=''"
    script = f"R='{root}'\n" + script[len("R=''\n"):]
    sys.exit(subprocess.run([sh, "-c", script], env=env).returncode)
if command.startswith("R=''"):
    command = f"R='{root}'" + command[len("R=''"):]
sys.exit(subprocess.run([sh, "-c", command], env=env).returncode)
'''

FAKE_KEXEC = r'''#!/bin/sh
echo "kexec: $*" >> "$FAKE_LOG"
case "$1" in
  --load)
    if [ "$FAKE_SCENARIO" = load-fails ]; then echo "kexec_load failed: Operation not permitted" >&2; exit 1; fi
    echo 1 > "$FAKE_ROOT36/sys/kernel/kexec_loaded" ;;
  --unload)
    [ "$*" = "--unload --kexec-syscall" ] || { echo "kexec stand-in: refusing $*" >&2; exit 99; }
    echo 0 > "$FAKE_ROOT36/sys/kernel/kexec_loaded" ;;
  *) echo "kexec stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  is-system-running) echo running ;;
  kexec)
    [ "$FAKE_SCENARIO" = no-jump ] && exit 0
    [ "$(cat "$FAKE_ROOT36/sys/kernel/kexec_loaded")" = 1 ] || { echo "no kernel loaded" >&2; exit 1; }
    case "$FAKE_SCENARIO" in
      stock-back) echo 0 > "$FAKE_ROOT36/sys/kernel/kexec_loaded"; echo "12.00 10.00" > "$FAKE_ROOT36/proc/uptime" ;;
      nothing) echo dead > "$FAKE_STAGE" ;;
      *) echo 310 > "$FAKE_STAGE" ;;
    esac ;;
  --failed) echo "  (none)" ;;
  is-active) case " $FAKE_ACTIVE " in *" $2 "*) echo active ;; *) echo inactive ;; esac ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

# BusyBox runs its own applets (such as uname, dmesg and ip) even when PATH has a stand-in,
# so the fake MPC gets its release from proc/sys/kernel/osrelease, as the scripts read it.
# systemd 255.17's own fsck unit, as on the reference unit: its only kexec line is allowed.
FSCK_UNIT = ("[Unit]\nDescription=File System Check on %f\nDefaultDependencies=no\nBindsTo=%i.device\n"
             "IgnoreOnIsolate=yes\nConflicts=reboot.target kexec.target poweroff.target halt.target\n"
             "After=%i.device systemd-fsck-root.service local-fs-pre.target\n"
             "Before=systemd-quotacheck.service shutdown.target\n\n[Service]\nType=oneshot\n"
             "RemainAfterExit=yes\nExecStart=/usr/lib/systemd/systemd-fsck %f\nTimeoutSec=infinity\n")

FAKE_SIMPLE = {
    "journalctl": '#!/bin/sh\necho "kernel: mpfree-test: blocked /usr/bin/dfu-util -l"\n',
}


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


class KexecBootTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-kexec-boot-test-")
        t = self.tmp
        self.work, self.log, self.stage = os.path.join(t, "work"), os.path.join(t, "log"), os.path.join(t, "stage")
        self.r36, self.r310 = os.path.join(t, "root36"), os.path.join(t, "root310")
        self.report, self.arp = os.path.join(t, "report.txt"), os.path.join(t, "arp")
        write(self.stage, "36\n")
        write(self.log, "")
        write(self.arp, "IP address HW type Flags HW address Mask Device\n"
                        f"{TEST_IP} 0x1 0x2 {TEST_MAC} * {TEST_IFACE}\n")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        write(os.path.join(fakebin, "ping"), "#!/bin/sh\nexit 0\n", 0o755)
        for name in APPLETS:
            os.makedirs(remote, exist_ok=True)
            os.symlink(self.busybox, os.path.join(remote, name))
        write(os.path.join(remote, "systemctl"), FAKE_SYSTEMCTL, 0o755)
        for name, text in FAKE_SIMPLE.items():
            write(os.path.join(remote, name), text, 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
                    "HOME": t, "MPFREE_KEXEC_BOOT_TEST": "1", "FAKE_STAGE": self.stage, "FAKE_LOG": self.log,
                    "FAKE_ROOT36": self.r36, "FAKE_ROOT310": self.r310, "FAKE_REMOTE_PATH": remote,
                    "FAKE_OLD": OLD, "FAKE_NEW": NEW, "FAKE_SCENARIO": "ok", "FAKE_ACTIVE": ""}
        self.key = os.path.join(t, "key")
        write(self.key, "fake key\n", 0o600)
        self.folder = self.build()
        self.root36()
        self.root310()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def build(self, emmc="disabled", akai=False):
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        tree = board_tree(serial="FAKE-SERIAL", mac=bytes.fromhex("020000000001"))
        tree.find("/dwmmc@ff0f0000").set_prop("status", (emmc + "\0").encode())
        tree.find("/dwmmc@ff0c0000").set_prop("status", b"disabled\0")
        files = {"stick.img": b"\x00" * 4096, "zImage": os.urandom(300000), "kexec.dtb": dtb.build(tree),
                 "cmdline.txt": CMDLINE + "\n", "known_hosts": "mpfree-kexec-test ssh-ed25519 AAAAfake\n"}
        for name, data in files.items():
            write(os.path.join(folder, name), data)
        with open(os.path.join(folder, "SHA256SUMS"), "w") as fh:
            for name, data in files.items():
                raw = data if isinstance(data, bytes) else data.encode()
                fh.write(f"{hashlib.sha256(raw).hexdigest()}  {name}\n")
        write(os.path.join(folder, "report.txt"), f"its kernel release: {NEW}\n"
                                                  f"running release: {OLD} (the only other boot-files release)\n"
              + ("akai app: on (--akai-app)\n" if akai else ""))
        return folder

    def root36(self):
        r = self.r36
        write(f"{r}/sys/kernel/kexec_loaded", "0\n")
        write(f"{r}/proc/sys/kernel/kexec_load_disabled", "0\n")
        write(f"{r}/proc/uptime", "5000.00 4000.00\n")
        write(f"{r}/proc/mounts", "/dev/mmcblk0p6 / ext4 ro 0 0\n/dev/sda2 /media/MPFREE-LOG ext4 rw 0 0\n")
        write(f"{r}/usr/sbin/kexec", FAKE_KEXEC, 0o755)
        write(f"{r}/etc/systemd/system/acvs.service", "[Service]\nExecStart=/usr/bin/az01-launch-MPC\n")
        write(f"{r}/usr/lib/systemd/system/kexec.target", "[Unit]\nRequires=systemd-kexec.service\n")
        write(f"{r}/usr/lib/systemd/system/systemd-kexec.service", "[Service]\nExecStart=systemctl --force kexec\n")
        write(f"{r}/usr/lib/systemd/system/systemd-fsck@.service", FSCK_UNIT)
        write(f"{r}/usr/lib/systemd/system-shutdown/flush.sh", "#!/bin/sh\nsync\n", 0o755)
        os.makedirs(f"{r}/run")
        usb = f"{r}/sys/devices/platform/ff500000.usb/usb1/1-1/host0/block/sda/sda1"
        os.makedirs(usb)
        os.makedirs(f"{r}/sys/class/block")
        os.symlink(usb, f"{r}/sys/class/block/sda1")
        write(f"{r}/dev/sda1", "")
        os.makedirs(f"{r}/dev/disk/by-partuuid")
        os.symlink("../../sda1", f"{r}/dev/disk/by-partuuid/{P1}")
        for n in range(1, 9):
            write(f"{r}/sys/class/block/mmcblk0p{n}/stat", f"10 0 80 1 {0 if n < 7 else 5} 0 {0 if n < 7 else 40} 2\n")

    def root310(self, mmc=False, data="tmpfs"):
        r = self.r310
        shutil.rmtree(r, ignore_errors=True)
        write(f"{r}/proc/cmdline", CMDLINE + " isolcpus=2-3\n")
        write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        for name in ("sda", "loop0") + (("mmcblk0",) if mmc else ()):
            os.makedirs(f"{r}/sys/block/{name}")
        write(f"{r}/sys/class/block/sda1/dev", "8:1\n")
        write(f"{r}/dev/sda1", "")
        os.makedirs(f"{r}/dev/disk/by-partuuid")
        os.symlink("../../sda1", f"{r}/dev/disk/by-partuuid/{P1}")
        write(f"{r}/proc/self/mountinfo",
              "20 1 8:1 / / ro,relatime shared:1 - ext4 /dev/root ro\n"
              f"30 20 0:25 / /data rw,relatime shared:9 - {data} {data} rw,size=262144k\n")

    def helper_cmd(self):
        return [sys.executable, "-I", HELPER, "--ip", TEST_IP, "--key", self.key, "--iface", TEST_IFACE,
                "--expect-mac", TEST_MAC, "--work", self.work, "--report", self.report, "--wait", "3",
                "--poll", "0.05", "--still-up-after", "0.3", "--arp", self.arp]

    def run_helper(self, answer="BOOT 3.10\n", scenario="ok", active=""):
        env = dict(self.env, FAKE_SCENARIO=scenario, FAKE_ACTIVE=active)
        return subprocess.run(self.helper_cmd(), input=answer, env=env, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def commands(self):
        with open(self.log) as fh:
            return fh.read()

    def loaded(self):
        with open(f"{self.r36}/sys/kernel/kexec_loaded") as fh:
            return fh.read().strip()

    def assert_untouched(self, loaded="0"):
        self.assertEqual(self.loaded(), loaded)
        self.assertFalse(os.path.exists(f"{self.r36}/run/mpfree-kexec"))
        self.assertNotIn("systemctl: kexec", self.commands())

    def test_boots_and_checks_the_test_build(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("zImage, kexec.dtb, cmdline.txt, known_hosts match SHA256SUMS",
                     "kexec.dtb: for inmusic,acva, eMMC and SD card controllers disabled",
                     f"running release: {OLD}", "test stick: partition 1 is /dev/sda1", "on USB: yes",
                     "/dev/sda2 on /media/MPFREE-LOG (ext4, rw)", "lockdown: not readable here",
                     "/usr/lib/systemd/system-shutdown/flush.sh", "mmcblk0p6: 0", "CHECKS-OK",
                     "systemd-fsck@.service: only systemd's own line", "no other unit reacts to kexec",
                     "their hashes on the MPC match the build", "kexec --load exit code: 0", "kexec_loaded: 1",
                     "answer: 'BOOT 3.10'", "the test build answered after", f"release: {NEW}",
                     "/ is device 8:1, options ro,relatime", "/data: tmpfs tmpfs", "mpfree-test: blocked",
                     f"network identity: {TEST_MAC}",
                     "RESULT: the test build is running from the stick, and it cannot see the eMMC or SD card."):
            self.assertIn(text, out)
        log = self.commands()
        load = re.search(r"^kexec: (--load .*)$", log, re.M).group(1)
        self.assertEqual(load, f"--load --kexec-syscall --dtb={self.r36}/run/mpfree-kexec/kexec.dtb "
                               f"--command-line={CMDLINE} {self.r36}/run/mpfree-kexec/zImage")
        for name in ("zImage", "kexec.dtb"):
            copied = f"{self.r36}/run/mpfree-kexec/{name}"
            with open(os.path.join(self.folder, name), "rb") as a, open(copied, "rb") as b:
                self.assertEqual(a.read(), b.read())
        self.assertIn("systemctl: kexec", log)
        with open(self.report) as fh:
            self.assertIn("RESULT: the test build is running", fh.read(), "the report gets the same lines")

    def test_build_that_runs_akais_app(self):
        res = self.run_helper(active="acvs.service")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("PROBLEM: acvs.service is active", res.stdout, "a normal build must not run Akai's app")
        self.assertNotIn("this build runs Akai's app", res.stdout)
        self.folder = self.build(akai=True)
        write(self.stage, "36\n")
        write(f"{self.r36}/sys/kernel/kexec_loaded", "0\n")
        res = self.run_helper(active="acvs.service")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("this build runs Akai's app (made with --akai-app); every flashing program in it is a stub",
                     "Akai's app starting", "Akai's app (acvs.service, expected in this build): active",
                     "CHECKS-OK", "RESULT: the test build is running from the stick"):
            self.assertIn(text, out)
        self.assertNotIn("PROBLEM", out)
        write(self.stage, "36\n")
        write(f"{self.r36}/sys/kernel/kexec_loaded", "0\n")
        res = self.run_helper(active="acvs.service xmos-update.service")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("PROBLEM: xmos-update.service is active", res.stdout, "the other masks still count")

    def test_anything_but_the_phrase_unloads_and_stops(self):
        for answer in ("boot 3.10\n", "yes\n", "\n", ""):
            with self.subTest(answer=answer):
                res = self.run_helper(answer)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("STOP: not confirmed", res.stdout)
                self.assertIn("unloaded; /run/mpfree-kexec removed. The MPC is as before.", res.stdout)
                self.assert_untouched()
                self.assertIn("kexec: --unload --kexec-syscall\n", self.commands())

    def test_stops_before_changing_anything(self):
        cases = (
            ("stick missing", lambda: os.unlink(f"{self.r36}/dev/disk/by-partuuid/{P1}"),
             "is not plugged into the MPC"),
            ("already loaded", lambda: write(f"{self.r36}/sys/kernel/kexec_loaded", "1\n"),
             "a kexec kernel is already loaded"),
            ("lockdown", lambda: write(f"{self.r36}/sys/kernel/security/lockdown",
                                       "none [integrity] confidentiality\n"),
             "kernel lockdown is on"),
            ("wrong release", lambda: self.env.update(FAKE_OLD="6.6.94-other"), "not 6.6.95-fake-rt55"),
            ("kexec unit", lambda: write(f"{self.r36}/etc/systemd/system/akai-kexec-hook.service",
                                         "[Install]\nWantedBy=kexec.target\n"), "reacts to kexec"),
            ("kexec wants", lambda: os.symlink("/dev/null", self.wants()), "reacts to kexec"),
            ("fsck unit changed", lambda: write(f"{self.r36}/usr/lib/systemd/system/systemd-fsck@.service",
                                                FSCK_UNIT + "ExecStopPost=/usr/sbin/kexec -e\n"),
             "      15:ExecStopPost=/usr/sbin/kexec -e"),
            ("fsck line indented", lambda: write(f"{self.r36}/usr/lib/systemd/system/systemd-fsck@.service",
                                                 FSCK_UNIT.replace("Conflicts=", " Conflicts=")),
             "reacts to kexec"),
            ("hook", lambda: write(f"{self.r36}/usr/lib/systemd/system-shutdown/akai.sh",
                                   '#!/bin/sh\n[ "$1" = kexec ] && do_something\n', 0o755), "mentions kexec"),
            ("not usb", self.not_usb, "is not on USB"),
        )
        for label, change, reason in cases:
            with self.subTest(label):
                self.tearDown()
                self.setUp()
                change()
                res = self.run_helper()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn(reason, res.stdout)
                self.assertIn("CHECKS-STOP", res.stdout)
                self.assertIn("STOP: the MPC is not ready; nothing was changed on it", res.stdout)
                self.assert_untouched("1" if label == "already loaded" else "0")
                self.assertNotIn("kexec: ", self.commands())

    def wants(self):
        os.makedirs(f"{self.r36}/etc/systemd/system/kexec.target.wants")
        return f"{self.r36}/etc/systemd/system/kexec.target.wants/akai-prepare.service"

    def not_usb(self):
        os.unlink(f"{self.r36}/sys/class/block/sda1")
        os.makedirs(f"{self.r36}/sys/devices/platform/ata/block/sda/sda1")
        os.symlink(f"{self.r36}/sys/devices/platform/ata/block/sda/sda1", f"{self.r36}/sys/class/block/sda1")

    def test_bad_builds_stop_before_contacting_the_mpc(self):
        cases = (
            ("changed zImage", lambda: write(os.path.join(self.folder, "zImage"), b"changed"),
             "does not match SHA256SUMS"),
            ("eMMC visible", lambda: self.build(emmc="okay"), "the eMMC controller is not disabled"),
            ("odd cmdline", lambda: self.rewrite_cmdline(CMDLINE.replace(" panic=10", "")), "not the builder's"),
        )
        for label, change, reason in cases:
            with self.subTest(label):
                self.tearDown()
                self.setUp()
                change()
                res = self.run_helper()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn(reason, res.stdout)
                self.assertEqual(self.commands(), "", "no ssh at all")

    def rewrite_cmdline(self, text):
        path = os.path.join(self.folder, "cmdline.txt")
        write(path, text + "\n")
        sums = os.path.join(self.folder, "SHA256SUMS")
        with open(sums) as fh:
            lines = [ln for ln in fh if not ln.endswith(" cmdline.txt\n")]
        lines.append(f"{hashlib.sha256((text + chr(10)).encode()).hexdigest()}  cmdline.txt\n")
        write(sums, "".join(lines))

    def test_ctrl_c_at_the_question_undoes_the_load(self):
        proc = subprocess.Popen(self.helper_cmd(), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, env=dict(self.env, FAKE_SCENARIO="ok"), text=True)
        seen = []
        for line in proc.stdout:
            seen.append(line)
            if "wait 10 s, switch it on." in line:
                break
        time.sleep(0.3)
        self.assertEqual(self.loaded(), "1")
        proc.send_signal(signal.SIGINT)
        time.sleep(0.05)
        proc.send_signal(signal.SIGINT)  # the second Ctrl-C (or timeout's forwarded one) is ignored
        out = "".join(seen) + proc.communicate(timeout=60)[0]
        self.assertEqual(proc.returncode, 1, out)
        self.assertIn("STOP: interrupted (Ctrl-C)", out)
        self.assertIn("unloaded; /run/mpfree-kexec removed. The MPC is as before.", out)
        self.assertNotIn("Traceback", out)
        self.assert_untouched()

    def test_failed_load_is_undone(self):
        res = self.run_helper(scenario="load-fails")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("kexec_load failed: Operation not permitted", res.stdout)
        self.assertIn("STOP: kexec --load did not load the kernel", res.stdout)
        self.assertNotIn("Type BOOT 3.10", res.stdout)
        self.assert_untouched()

    def test_jump_outcomes(self):
        cases = (("no-jump", "RESULT: the jump did not start. 3.6 is running as before.", True),
                 ("stock-back", "RESULT: the MPC is on 3.6 again. If you did not switch it off", False),
                 ("nothing", "RESULT: nothing answered. Switch the MPC off", False))
        for scenario, result, undone in cases:
            with self.subTest(scenario):
                self.tearDown()
                self.setUp()
                res = self.run_helper(scenario=scenario)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn(result, res.stdout)
                if undone:
                    self.assertIn("kexec: --unload --kexec-syscall\n", self.commands())
                    self.assertEqual(self.loaded(), "0")

    def test_problems_on_the_test_build_are_reported(self):
        self.root310(mmc=True, data="ext4")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("PROBLEM: an MMC block device exists", res.stdout)
        self.assertIn("PROBLEM: /data is not a tmpfs", res.stdout)
        self.assertIn("RESULT: the test build is running, but some checks failed", res.stdout)
        self.assertIn("NEXT: leave it running and do this round's steps on the test build", res.stdout)
        self.assertIn("When they are done, switch the MPC off with its power switch", res.stdout)

    def test_wrapper_needs_a_terminal_and_the_opt_in(self):
        env = dict(self.env, MPC_KEY=self.key)
        res = subprocess.run(["bash", WRAPPER, "--kexec-boot"], input="", env=env, text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("run it from a terminal", res.stdout)
        master, slave = pty.openpty()
        try:
            res = subprocess.run(["bash", WRAPPER, "--kexec-boot"], stdin=slave, env=env, text=True,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        finally:
            os.close(slave)
            os.close(master)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("MPFREE_ACCEPT_KEXEC_BOOT=YES", res.stdout)
        self.assertEqual(self.commands(), "")


class StaticTests(unittest.TestCase):
    def test_remote_scripts_change_only_ram(self):
        sys.path.insert(0, TOOLS)
        import importlib.util
        spec = importlib.util.spec_from_file_location("host_kexec_boot", HELPER)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        scripts = {name: getattr(mod, name) for name in
                   ("CHECK_36", "PREPARE", "RECEIVE", "HASHES", "LOAD", "UNDO", "JUMP", "STATE", "CHECK_310")}
        for akai in (False, True):
            filled = mod.check_310({"new": NEW, "p1": P1, "akai": akai})
            self.assertNotIn("@@", filled)
            loop = re.search(r"^for u in (.*); do$", filled, re.M).group(1).split()
            self.assertEqual("acvs" in loop, not akai, loop)
            self.assertEqual(len(loop), 7 if akai else 8, loop)
            self.assertEqual("is-active acvs.service" in filled, akai)
            scripts[f"CHECK_310 akai={akai}"] = filled
        for name, text in scripts.items():
            self.assertTrue(text.startswith("R=''"), name)
            self.assertIsNone(re.search(r"(?<![0-9])>\s*\S*/dev/", text), f"{name}: writes to a device")
            self.assertIsNone(re.search(r"(^|[;&|(]\s*)(mkfs|mke2fs|sfdisk|fdisk|parted|wipefs)", text, re.M), name)
            for needle in ("of=", " dd ", "fw_setenv", "mount ", "kexec -e", "--exec",
                           "flash", "/boot/", "echo 1 >", "tee "):
                self.assertNotIn(needle, text, f"{name}: {needle}")
            self.assertIsNone(re.search(r"(^|[;&|(]\s*|systemctl\s+)(reboot|poweroff|halt)\b", text, re.M), name)
        writes = [ln for t in scripts.values() for ln in t.splitlines() if re.search(r"(^|[^2])>[^&]", ln)]
        self.assertEqual(writes, ['R=\'\'; umask 077; cat > "$R/run/mpfree-kexec/@@NAME@@" '], writes)
        changes = " ".join(scripts[n] for n in ("PREPARE", "UNDO"))
        self.assertEqual(sorted(set(re.findall(r"\b(rm -f|rmdir|mkdir)\b", changes))), ["mkdir", "rm -f", "rmdir"])
        self.assertEqual(scripts["JUMP"].count("systemctl kexec"), 1)
        for name, text in scripts.items():
            if name != "JUMP":
                self.assertNotIn("systemctl kexec", text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
