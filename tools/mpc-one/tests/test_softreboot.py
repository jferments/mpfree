#!/usr/bin/env python3
"""Tests for MPFree's soft-reboot route (collect-over-ssh.sh --softreboot-check / --softreboot-try /
--softreboot-report, and softreboot-try.sh / read-only-softreboot-report.sh). No device, no network.

Like test_boot_hook.py, the wrapper runs against a fake ssh that streams the scripts under BusyBox applets
against a fake root (the first line R='' becomes that folder). A fake `mount` copies a fake stick ROOT tree
into /run/nextroot; `umount` empties it. `systemctl soft-reboot` is a stand-in that only logs. Nothing here
restarts anything.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox or dash.
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
from test_hook_canary import FAKE_SSH, MOUNTS, write  # noqa: E402
from test_kexec_preflight import FAKE_IP, TEST_HOST, TEST_IFACE, TEST_IP, TEST_MAC  # noqa: E402

TOOLS = os.path.dirname(HERE)
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
TRY = os.path.join(TOOLS, "softreboot-try.sh")
REPORT = os.path.join(TOOLS, "read-only-softreboot-report.sh")
RELEASE = "6.18.44-az01-2026-08-10-rt6"
APPLETS = ("cat", "grep", "sed", "tr", "wc", "ls", "uname", "head", "tail", "cut", "mkdir", "chmod", "mv", "rm",
           "rmdir", "sync", "df", "basename", "dirname", "stat", "date", "readlink", "cp", "find")
STICK_UUID = "1b2c3d4e-0000-4000-8000-00000000abcd"
NEXTROOT = "run/nextroot"
SVC = "usr/lib/systemd/system/systemd-soft-reboot.service"

# systemctl: version, is-active (a unit is "activating" iff it is FAKE_BUSY_UNIT), and soft-reboot logs only.
FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  --version) echo "systemd ${FAKE_SYSTEMD_VER:-255} (${FAKE_SYSTEMD_VER:-255}.fake)" ;;
  is-active) if [ "$2" = "${FAKE_BUSY_UNIT:-}" ]; then echo activating; else echo inactive; fi ;;
  soft-reboot) : ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''
FAKE_PIDOF = '#!/bin/sh\n[ "$1" = "${FAKE_PIDOF_HIT:-}" ] && { echo 4321; exit 0; }\nexit 1\n'
# mount copies the fake stick root into /run/nextroot; umount empties it. Both only inside the fake root.
FAKE_MOUNT = r'''#!/bin/sh
echo "mount: $*" >> "$FAKE_LOG"
[ -n "${FAKE_MOUNT_FAIL:-}" ] && exit 32
for dir; do :; done
case "$dir" in "$FAKE_ROOT"/run/nextroot) ;; *) echo "fake mount: unexpected target $dir" >&2; exit 99 ;; esac
cp -a "$FAKE_STICK_ROOT/." "$dir/"
'''
FAKE_UMOUNT = r'''#!/bin/sh
echo "umount: $*" >> "$FAKE_LOG"
case "$1" in "$FAKE_ROOT"/run/nextroot) ;; *) echo "fake umount: unexpected $1" >&2; exit 99 ;; esac
rm -rf "$1"/* "$1"/.[!.]* 2>/dev/null
exit 0
'''
FAKE_JOURNALCTL = '#!/bin/sh\nexit 0\n'
FAKE_DMESG = '#!/bin/sh\nexit 0\n'


class SoftRebootTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox, cls.dash = shutil.which("busybox"), shutil.which("dash")
        if not cls.busybox or not cls.dash:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox and dash are required for these tests")
            raise unittest.SkipTest("BusyBox or dash not installed")
        if not shutil.which("ssh-keygen"):
            raise unittest.SkipTest("ssh-keygen not installed")
        cls.keys = tempfile.mkdtemp(prefix="mpfree-softreboot-keys-")
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "", "-f",
                        os.path.join(cls.keys, "host")], check=True)
        with open(os.path.join(cls.keys, "host.pub")) as fh:
            cls.hostkey = " ".join(fh.read().split()[:2])

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.keys, ignore_errors=True)

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-softreboot-test-")
        self.home = os.path.join(self.tmp, "home")
        self.reports = os.path.join(self.tmp, "research")
        self.log = os.path.join(self.tmp, "commands.log")
        self.root = os.path.join(self.tmp, "root")
        self.stickroot = os.path.join(self.tmp, "stickroot")
        fakebin, remote = os.path.join(self.tmp, "fakebin"), os.path.join(self.tmp, "remote-bin")
        for d in (self.home, fakebin, remote):
            os.makedirs(d)
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        write(os.path.join(fakebin, "ip"), FAKE_IP, 0o755)
        write(os.path.join(fakebin, "ping"), "#!/bin/sh\nexit 0\n", 0o755)
        os.symlink(self.busybox, os.path.join(remote, "sh"))
        for applet in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, applet))
        for name, text in (("systemctl", FAKE_SYSTEMCTL), ("pidof", FAKE_PIDOF), ("mount", FAKE_MOUNT),
                           ("umount", FAKE_UMOUNT), ("journalctl", FAKE_JOURNALCTL), ("dmesg", FAKE_DMESG)):
            write(os.path.join(remote, name), text, 0o755)
        self.remote = remote
        r = self.root
        write(os.path.join(r, "proc/sys/kernel/osrelease"), RELEASE + "\n")
        write(os.path.join(r, "proc/mounts"), MOUNTS)
        write(os.path.join(r, "proc/uptime"), "900.50 9.10\n")
        write(os.path.join(r, "usr/lib/systemd/system/systemd-soft-reboot.service"), "[Unit]\n")
        os.makedirs(os.path.join(r, "sys/block/mmcblk0"))
        os.makedirs(os.path.join(r, "run"))
        self.build_stick_root()
        self.key = os.path.join(self.tmp, "key")
        write(self.key, "not a real key\n", 0o600)
        self.env = {
            "HOME": self.home, "LC_ALL": "C", "PATH": fakebin + os.pathsep + "/usr/bin:/bin",
            "MPC_KEY": self.key, "MPC_REPORT_DIR": self.reports, "MPC_PORT": "22",
            "MPC_IP": TEST_IP, "MPC_IFACE": TEST_IFACE, "MPC_HOST_IP_CIDR": TEST_HOST, "MPC_EXPECT_MAC": TEST_MAC,
            "FAKE_LOG": self.log, "FAKE_REMOTE_PATH": remote, "FAKE_ROOT": self.root,
            "FAKE_STICK_ROOT": self.stickroot, "FAKE_HOSTKEY": self.hostkey, "FAKE_RELEASE": RELEASE,
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def build_stick_root(self):
        """A valid MPFree stick root: an init, the marker, a startup service, matching modules, and the two
        masks (Akai's app and the eMMC content-mount generator) that keep the internal eMMC untouched."""
        s = self.stickroot
        write(os.path.join(s, "sbin/init"), "#!/bin/sh\n", 0o755)
        write(os.path.join(s, "etc/mpfree-test"), "mpfree\n")
        write(os.path.join(s, "etc/systemd/system/mpfree-test-splash.service"), "[Unit]\n")
        os.makedirs(os.path.join(s, "lib/modules/" + RELEASE))
        os.makedirs(os.path.join(s, "etc/systemd/system"), exist_ok=True)
        os.makedirs(os.path.join(s, "etc/systemd/system-generators"), exist_ok=True)
        os.symlink("/dev/null", os.path.join(s, "etc/systemd/system/acvs.service"))
        os.symlink("/dev/null", os.path.join(s, "etc/systemd/system-generators/acva-emmc-content-generator"))

    def plug_in(self):
        r = self.root
        block = os.path.join(r, "sys/devices/platform/ff540000.usb/usb2/2-1/2-1:1.0/host0/target0:0:0/0:0:0:0/"
                                "block/sda/sda1")
        os.makedirs(block)
        os.makedirs(os.path.join(r, "sys/class/block"), exist_ok=True)
        os.symlink(os.path.relpath(block, os.path.join(r, "sys/class/block")),
                   os.path.join(r, "sys/class/block/sda1"))
        write(os.path.join(r, "dev/sda1"), "")
        for folder, name in (("by-partlabel", "mpfree-test-root"), ("by-partuuid", STICK_UUID)):
            os.makedirs(os.path.join(r, "dev/disk", folder), exist_ok=True)
            os.symlink("../../sda1", os.path.join(r, "dev/disk", folder, name))

    def run_wrapper(self, mode, **extra):
        return subprocess.run(["bash", WRAPPER, mode], env={**self.env, **extra}, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=120)

    def calls(self, prefix):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [line for line in fh.read().splitlines() if line.startswith(prefix)]

    def report_text(self):
        paths = glob.glob(os.path.join(self.reports, "mpc-one-softreboot-*-*.txt"))
        self.assertTrue(paths, "no report was written")
        with open(max(paths, key=os.path.getmtime)) as fh:
            return fh.read()

    def nextroot_entries(self):
        p = os.path.join(self.root, NEXTROOT)
        return sorted(os.listdir(p)) if os.path.isdir(p) else None

    # --- the wrapper's gates --------------------------------------------------------------------------
    def test_try_needs_the_opt_in_and_port_22(self):
        res = self.run_wrapper("--softreboot-try")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("MPFREE_ACCEPT_SOFTREBOOT=YES", res.stdout)
        self.assertIn("--softreboot-check first", res.stdout)
        res = self.run_wrapper("--softreboot-check", MPC_PORT="2222")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("run it with MPC_PORT=22", res.stdout)
        self.assertEqual(self.calls("ssh: "), [], "nothing may reach the MPC")

    # --- check: the happy path ------------------------------------------------------------------------
    def test_check_sets_up_validates_and_tears_down(self):
        self.plug_in()
        res = self.run_wrapper("--softreboot-check")
        self.assertEqual(res.returncode, 0, res.stdout)
        text = self.report_text()
        self.assertIn("MPFREE SOFT-REBOOT INTO THE STICK - check", text)
        self.assertIn("systemd 255 can restart userspace into /run/nextroot", text)
        self.assertIn("stick: /dev/sda1 (partition 1 of a USB disk", text)
        self.assertIn("stick root: init present, /etc/mpfree-test marker, startup service "
                      "mpfree-test-splash.service, modules for " + RELEASE, text)
        self.assertIn("stick masks in place: Akai's app (acvs.service) and the eMMC content-mount generator", text)
        self.assertIn("CHECK-OK: torn down, nothing left behind", text)
        work = os.path.join(self.root, NEXTROOT)
        self.assertEqual(self.calls("mount: "),
                         [f"mount: -t ext4 -o ro,noload,nodev,nosuid {self.root}/dev/sda1 {work}"])
        self.assertEqual(self.calls("umount: "), [f"umount: {work}"])
        self.assertIsNone(self.nextroot_entries(), "/run/nextroot is removed after the check")
        self.assertEqual(self.calls("systemctl: soft-reboot"), [], "the check never restarts")

    # --- try: the real restart (stand-in logs it) -----------------------------------------------------
    def test_try_sets_up_then_asks_systemd_to_restart(self):
        self.plug_in()
        res = self.run_wrapper("--softreboot-try", MPFREE_ACCEPT_SOFTREBOOT="YES")
        self.assertEqual(res.returncode, 0, res.stdout)
        text = self.report_text()
        self.assertIn("MPFREE SOFT-REBOOT INTO THE STICK - try", text)
        self.assertIn("all checks passed", text)
        self.assertIn("starting MPFree now", text)
        self.assertEqual(self.calls("systemctl: soft-reboot"), ["systemctl: soft-reboot"])
        self.assertEqual(self.calls("mount: ").__len__(), 1)

    # --- check: every refusal, and none of them restart or leave /run/nextroot behind -----------------
    def expect_stop(self, mode, message, **extra):
        open(self.log, "w").close()
        res = self.run_wrapper(mode, **extra)
        self.assertEqual(res.returncode, 1, res.stdout)
        text = self.report_text() if glob.glob(os.path.join(self.reports, "mpc-one-softreboot-*")) else res.stdout
        self.assertIn("STOP:", text)
        self.assertIn(message, text)
        self.assertEqual(self.calls("systemctl: soft-reboot"), [], "a refusal never restarts")
        self.assertIsNone(self.nextroot_entries(), "a refusal leaves no /run/nextroot")
        shutil.rmtree(self.reports, ignore_errors=True)

    def test_check_refuses_cleanly_on_every_problem(self):
        self.plug_in()
        with self.subTest("wrong release"):
            write(os.path.join(self.root, "proc/sys/kernel/osrelease"), "6.6.0-other\n")
            self.expect_stop("--softreboot-check", "this is for Akai's 3.10.1")
            write(os.path.join(self.root, "proc/sys/kernel/osrelease"), RELEASE + "\n")
        with self.subTest("systemd too old"):
            self.expect_stop("--softreboot-check", "is too old for a userspace-only restart", FAKE_SYSTEMD_VER="250")
        with self.subTest("no soft-reboot unit"):
            os.unlink(os.path.join(self.root, SVC))
            self.expect_stop("--softreboot-check", "is not present, so this systemd cannot restart userspace")
            write(os.path.join(self.root, SVC), "[Unit]\n")
        with self.subTest("nextroot already present"):
            # A pre-existing /run/nextroot must stop the run AND be left untouched (something else may have
            # staged it), so this case is not run through expect_stop (which asserts /run/nextroot is gone).
            os.makedirs(os.path.join(self.root, NEXTROOT))
            open(self.log, "w").close()
            res = self.run_wrapper("--softreboot-check")
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("/run/nextroot already exists", self.report_text())
            self.assertEqual(self.calls("systemctl: soft-reboot"), [])
            self.assertEqual(self.nextroot_entries(), [], "a pre-existing /run/nextroot is left as it was")
            self.assertEqual(self.calls("umount: "), [], "it never unmounts what it did not mount")
            os.rmdir(os.path.join(self.root, NEXTROOT))
            shutil.rmtree(self.reports, ignore_errors=True)
        with self.subTest("stick not mountable"):
            self.expect_stop("--softreboot-check", "could not mount /dev/sda1 at /run/nextroot", FAKE_MOUNT_FAIL="1")
        with self.subTest("no init on the stick root"):
            os.unlink(os.path.join(self.stickroot, "sbin/init"))
            self.expect_stop("--softreboot-check", "has no init")
            write(os.path.join(self.stickroot, "sbin/init"), "#!/bin/sh\n", 0o755)
        with self.subTest("no marker"):
            os.unlink(os.path.join(self.stickroot, "etc/mpfree-test"))
            self.expect_stop("--softreboot-check", "has no /etc/mpfree-test marker")
            write(os.path.join(self.stickroot, "etc/mpfree-test"), "mpfree\n")
        with self.subTest("no startup service"):
            os.unlink(os.path.join(self.stickroot, "etc/systemd/system/mpfree-test-splash.service"))
            self.expect_stop("--softreboot-check", "has no MPFree startup service")
            write(os.path.join(self.stickroot, "etc/systemd/system/mpfree-test-splash.service"), "[Unit]\n")
        with self.subTest("modules for a different kernel"):
            shutil.rmtree(os.path.join(self.stickroot, "lib/modules/" + RELEASE))
            self.expect_stop("--softreboot-check", "was built for a different kernel")
            os.makedirs(os.path.join(self.stickroot, "lib/modules/" + RELEASE))
        with self.subTest("Akai's app not masked"):
            os.unlink(os.path.join(self.stickroot, "etc/systemd/system/acvs.service"))
            self.expect_stop("--softreboot-check", "Akai's app (acvs.service) is not masked")
            os.symlink("/dev/null", os.path.join(self.stickroot, "etc/systemd/system/acvs.service"))
        with self.subTest("content generator not masked"):
            os.unlink(os.path.join(self.stickroot, "etc/systemd/system-generators/acva-emmc-content-generator"))
            self.expect_stop("--softreboot-check", "eMMC content-mount generator is not masked")
            os.symlink("/dev/null",
                       os.path.join(self.stickroot, "etc/systemd/system-generators/acva-emmc-content-generator"))
        with self.subTest("a firmware updater is running"):
            self.expect_stop("--softreboot-check", "touch-fw-update.service is running now",
                             FAKE_BUSY_UNIT="touch-fw-update.service")
        with self.subTest("no stick"):
            shutil.rmtree(os.path.join(self.root, "dev/disk"))
            self.expect_stop("--softreboot-check", "MPFree's stick is not in")

    def test_try_refuses_without_restarting_when_a_check_fails(self):
        self.plug_in()
        shutil.rmtree(os.path.join(self.stickroot, "lib/modules/" + RELEASE))
        res = self.run_wrapper("--softreboot-try", MPFREE_ACCEPT_SOFTREBOOT="YES")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: ", self.report_text())
        self.assertEqual(self.calls("systemctl: soft-reboot"), [])
        self.assertIsNone(self.nextroot_entries())

    # --- report: read-only ----------------------------------------------------------------------------
    def test_report_reads_only_and_names_the_running_system(self):
        self.plug_in()
        res = self.run_wrapper("--softreboot-report")
        self.assertEqual(res.returncode, 0, res.stdout)
        text = self.report_text()
        self.assertIn("MPFREE SOFT-REBOOT REPORT - READ ONLY", text)
        self.assertIn("this is Akai's system (no /etc/mpfree-test", text)
        self.assertIn("systemd-soft-reboot.service: present", text)
        self.assertIn("/run/nextroot: absent", text)
        self.assertIn("stick in: yes (/dev/sda1)", text)
        self.assertIn("END OF SOFT-REBOOT REPORT", text)
        # Read-only: it mounts, starts and restarts nothing.
        self.assertEqual(self.calls("mount: ") + self.calls("umount: "), [])
        self.assertEqual([c for c in self.calls("systemctl: ") if "soft-reboot" in c], [])


class StaticTests(unittest.TestCase):
    def code(self, path):
        with open(path) as fh:
            return "\n".join(line for line in fh.read().splitlines() if not line.lstrip().startswith("#"))

    def test_scripts_parse(self):
        for path in (TRY, REPORT):
            subprocess.run(["sh", "-n", path], check=True)
            if shutil.which("busybox"):
                subprocess.run(["busybox", "sh", "-n", path], check=True)

    def test_try_writes_no_storage_but_run_nextroot(self):
        code = self.code(TRY)
        # The only mount is the stick's root read-only at /run/nextroot (ro, no dev/suid); the only restart is
        # systemctl's userspace-only one. The mmcblk glob is a read (reports the block devices), not a write.
        self.assertIn("mount -t ext4 -o ro,noload,nodev,nosuid ", code)
        self.assertEqual(code.count("mount -t"), 1)
        self.assertNotIn("rw,", code, "the stick is only ever mounted read-only")
        self.assertIn('systemctl "$verb"', code)
        for forbidden in ("mkfs", "fw_setenv", "dd ", "mmcblk0p", " > /data", ">> /data", " > /etc", ">> /etc",
                          "kexec", "rm -rf /", "systemctl start", "systemctl stop", "flashcp"):
            self.assertNotIn(forbidden, code)
        # Every redirection writes only /dev/null; nothing is written to storage.
        writes = [line.strip() for line in code.splitlines() if (" > " in line or ">>" in line)]
        for line in writes:
            self.assertIn("> /dev/null", line, line)

    def test_report_only_reads(self):
        code = self.code(REPORT)
        # Forbid command forms that would change state; the words "mount"/"mounted" appear only in strings and
        # /proc/mounts reads, which is why we match command shapes (a dash or quote), not the bare word.
        for forbidden in ("rm ", "mv ", "cp ", "mkdir", "chmod", "mount -", "umount ", "systemctl soft-re",
                          "systemctl start", "systemctl stop", "systemd-run", "kexec", " > ", ">>"):
            self.assertNotIn(forbidden, code)
        self.assertEqual(code.count("mount -t") + code.count("umount "), 0, "the report never (un)mounts")


if __name__ == "__main__":
    unittest.main(verbosity=2)
