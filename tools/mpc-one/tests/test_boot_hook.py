#!/usr/bin/env python3
"""Tests for MPFree's boot hook (collect-over-ssh.sh --hook-install, --hook-remove, --hook-report, and the
hook.sh they install). No device, no network.

The wrapper runs against the same stand-ins as test_hook_canary.py: a fake ssh that runs the streamed
scripts under BusyBox applets against a fake root folder (their first line R='' becomes that folder).
BusyBox's shell runs its own applets before anything in PATH, so the hook's steps that mount, sleep or look
for processes (the jump and the check) run under dash here, with the same BusyBox applets for everything
but the stand-ins for mount, umount, sleep, pidof, systemctl, systemd-run and kexec. The installer runs
the check with $R/bin/sh, which is dash in the fake root and /bin/sh on the MPC.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox or dash.
"""
import glob
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_hook_canary import FAKE_JOURNALCTL, FAKE_KMOD, FAKE_SSH, MOUNTS, write  # noqa: E402
from test_kexec_preflight import FAKE_IP, TEST_HOST, TEST_IFACE, TEST_IP, TEST_MAC  # noqa: E402

TOOLS = os.path.dirname(HERE)
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
INSTALL = os.path.join(TOOLS, "hook-install.sh")
REMOVE = os.path.join(TOOLS, "hook-remove.sh")
REPORT = os.path.join(TOOLS, "read-only-hook-report.sh")
RELEASE = "6.18.44-az01-2026-08-10-rt6"
APPLETS = ("cat", "grep", "sed", "tr", "wc", "ls", "uname", "head", "tail", "cut", "mkdir", "chmod", "mv", "rm",
           "rmdir", "sync", "sha256sum", "df", "basename", "dirname", "stat", "date", "readlink", "cp", "find")
RULE = "data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules"
CONF = "data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf"
SCRIPT = "data/mpfree-hook/hook.sh"
MARK = "# MPFree boot hook, written by tools/mpc-one/hook-install.sh."
CANARY_MARK = "# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh."
CANARY = ("data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules",
          "data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf", "data/mpfree-hook/canary.sh")
STICK_UUID = "1b2c3d4e-0000-4000-8000-00000000abcd"
KERNEL = b"FAKE-ZIMAGE-" * 4000
DTB = b"\xd0\x0d\xfe\xed" + b"FAKE-DTB" * 500
CMDLINE = (f"root=PARTUUID={STICK_UUID} rootwait=60 ro panic=10 rfkill.default_state=0 "
           "sysctl.kernel.kexec_load_disabled=1\n")
HOLD = ("acvs.service", "touch-fw-update.service", "xmos-update.service", "az01-usbsata-fixer.service")
JUMP_START = ("systemd-run: --no-block --collect --quiet --unit=mpfree-jump --description=MPFree: start from the "
              "stick --property=Type=oneshot --property=TimeoutStartSec=45 "
              + " ".join(f"--property=Before={u}" for u in HOLD) + " /bin/sh /data/mpfree-hook/hook.sh jump")

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  is-active)
    if [ "$2" = "${FAKE_BUSY_UNIT:-}" ]; then echo activating
    elif [ "$2" = acvs.service ]; then echo "${FAKE_ACVS:-inactive}"
    else echo inactive; fi ;;
  --no-block) exit "${FAKE_START_RC:-0}" ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''
FAKE_SYSTEMD_RUN = '#!/bin/sh\necho "systemd-run: $*" >> "$FAKE_LOG"\n'
FAKE_PIDOF = '#!/bin/sh\n[ "$1" = "${FAKE_PIDOF_HIT:-}" ] && { echo 4321; exit 0; }\nexit 1\n'
FAKE_SLEEP = '#!/bin/sh\necho "sleep: $*" >> "$FAKE_LOG"\n'
# mount fills the folder from the fake stick; umount empties it again. Both only inside the fake root.
FAKE_MOUNT = r'''#!/bin/sh
echo "mount: $*" >> "$FAKE_LOG"
[ -n "${FAKE_MOUNT_FAIL:-}" ] && exit 32
for dir; do :; done
case "$dir" in "$FAKE_ROOT"/*) ;; *) echo "fake mount: outside the fake root" >&2; exit 99 ;; esac
cp -a "$FAKE_STICK/." "$dir/"
'''
FAKE_UMOUNT = r'''#!/bin/sh
echo "umount: $*" >> "$FAKE_LOG"
case "$1" in "$FAKE_ROOT"/run/*) ;; *) echo "fake umount: outside the fake root's /run" >&2; exit 99 ;; esac
rm -rf "$1"/*
'''
FAKE_KEXEC = r'''#!/bin/sh
echo "kexec: $*" >> "$FAKE_LOG"
case "$1" in
  --load) [ -n "${FAKE_KEXEC_FAIL:-}" ] && exit 1; echo 1 > "$FAKE_ROOT/sys/kernel/kexec_loaded" ;;
  --unload) echo 0 > "$FAKE_ROOT/sys/kernel/kexec_loaded" ;;
esac
'''


def sha(data):
    return hashlib.sha256(data).hexdigest()


class BootHookTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox, cls.dash = shutil.which("busybox"), shutil.which("dash")
        if not cls.busybox or not cls.dash:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox and dash are required for these tests")
            raise unittest.SkipTest("BusyBox or dash not installed")
        if not shutil.which("ssh-keygen"):
            raise unittest.SkipTest("ssh-keygen not installed")
        cls.keys = tempfile.mkdtemp(prefix="mpfree-boot-hook-keys-")
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "", "-f",
                        os.path.join(cls.keys, "host")], check=True)
        with open(os.path.join(cls.keys, "host.pub")) as fh:
            cls.hostkey = " ".join(fh.read().split()[:2])

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.keys, ignore_errors=True)

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-boot-hook-test-")
        self.home = os.path.join(self.tmp, "home")
        self.reports = os.path.join(self.tmp, "research")
        self.log = os.path.join(self.tmp, "commands.log")
        self.root = os.path.join(self.tmp, "root")
        self.stick = os.path.join(self.tmp, "stick")
        fakebin, remote = os.path.join(self.tmp, "fakebin"), os.path.join(self.tmp, "remote-bin")
        for d in (self.home, fakebin, remote):
            os.makedirs(d)
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        write(os.path.join(fakebin, "ip"), FAKE_IP, 0o755)
        write(os.path.join(fakebin, "ping"), "#!/bin/sh\nexit 0\n", 0o755)
        os.symlink(self.busybox, os.path.join(remote, "sh"))
        for applet in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, applet))
        for name, text in (("systemctl", FAKE_SYSTEMCTL), ("systemd-run", FAKE_SYSTEMD_RUN), ("pidof", FAKE_PIDOF),
                           ("journalctl", FAKE_JOURNALCTL), ("modprobe", FAKE_KMOD), ("sleep", FAKE_SLEEP),
                           ("mount", FAKE_MOUNT), ("umount", FAKE_UMOUNT)):
            write(os.path.join(remote, name), text, 0o755)
        self.remote = remote
        r = self.root
        write(os.path.join(r, "proc/sys/kernel/osrelease"), RELEASE + "\n")
        write(os.path.join(r, "proc/mounts"), MOUNTS)
        write(os.path.join(r, "proc/uptime"), "6.05 9.10\n")
        write(os.path.join(r, "proc/sys/kernel/modprobe"), "/sbin/modprobe\n")
        write(os.path.join(r, "proc/sys/kernel/kexec_load_disabled"), "0\n")
        write(os.path.join(r, "sys/kernel/kexec_loaded"), "0\n")
        write(os.path.join(r, "sbin/modprobe"), FAKE_KMOD, 0o755)
        write(os.path.join(r, "usr/sbin/kexec"), FAKE_KEXEC, 0o755)
        os.makedirs(os.path.join(r, "bin"))
        os.symlink(self.dash, os.path.join(r, "bin/sh"))
        write(os.path.join(r, "data/system/etc/overlay/hostname"), "mpc-one\n")
        write(os.path.join(r, "dev/kmsg"), "")
        os.makedirs(os.path.join(r, "run"))
        self.before = self.tree()
        # MPFree's stick: partition 1 of a USB disk, and what is on it.
        files = {"boot/zImage-" + RELEASE: KERNEL, "boot/mpfree-kexec/kexec.dtb": DTB,
                 "boot/mpfree-kexec/cmdline.txt": CMDLINE.encode()}
        for rel, data in files.items():
            os.makedirs(os.path.dirname(os.path.join(self.stick, rel)), exist_ok=True)
            with open(os.path.join(self.stick, rel), "wb") as fh:
                fh.write(data)
        os.symlink("../zImage-" + RELEASE, os.path.join(self.stick, "boot/mpfree-kexec/zImage"))
        self.write_sums()
        self.key = os.path.join(self.tmp, "key")
        write(self.key, "not a real key\n", 0o600)
        self.env = {
            "HOME": self.home, "LC_ALL": "C", "PATH": fakebin + os.pathsep + "/usr/bin:/bin",
            "MPC_KEY": self.key, "MPC_REPORT_DIR": self.reports, "MPC_PORT": "22",
            "MPC_IP": TEST_IP, "MPC_IFACE": TEST_IFACE, "MPC_HOST_IP_CIDR": TEST_HOST, "MPC_EXPECT_MAC": TEST_MAC,
            "FAKE_LOG": self.log, "FAKE_REMOTE_PATH": remote, "FAKE_ROOT": self.root, "FAKE_STICK": self.stick,
            "FAKE_HOSTKEY": self.hostkey, "FAKE_RELEASE": RELEASE,
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write_sums(self, **override):
        sums = {"zImage": sha(KERNEL), "kexec.dtb": sha(DTB), "cmdline.txt": sha(CMDLINE.encode()), **override}
        with open(os.path.join(self.stick, "boot/mpfree-kexec/SHA256SUMS"), "w") as fh:
            fh.write("".join(f"{v}  {k}\n" for k, v in sums.items()))

    def plug_in(self):
        r = self.root
        block = os.path.join(r, "sys/devices/platform/ff540000.usb/usb2/2-1/2-1:1.0/host0/target0:0:0/0:0:0:0/"
                                "block/sda/sda1")
        os.makedirs(block)
        os.makedirs(os.path.join(r, "sys/class/block"), exist_ok=True)
        os.symlink(os.path.relpath(block, os.path.join(r, "sys/class/block")), os.path.join(r, "sys/class/block/sda1"))
        write(os.path.join(r, "dev/sda1"), "")
        for folder, name in (("by-partlabel", "mpfree-test-root"), ("by-partuuid", STICK_UUID)):
            os.makedirs(os.path.join(r, "dev/disk", folder), exist_ok=True)
            os.symlink("../../sda1", os.path.join(r, "dev/disk", folder, name))

    def tree(self):
        found = {}
        for folder, dirs, files in os.walk(os.path.join(self.root, "data")):
            rel = os.path.relpath(folder, self.root)
            found[rel + "/"] = None
            for name in files:
                with open(os.path.join(folder, name)) as fh:
                    found[os.path.join(rel, name)] = fh.read()
        return found

    def run_wrapper(self, mode, **extra):
        return subprocess.run(["bash", WRAPPER, mode], env={**self.env, **extra}, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=120)

    def calls(self, prefix):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [line for line in fh.read().splitlines() if line.startswith(prefix)]

    def read(self, rel):
        with open(os.path.join(self.root, rel)) as fh:
            return fh.read()

    def install(self, ok=True):
        res = self.run_wrapper("--hook-install", MPFREE_ACCEPT_HOOK="YES")
        self.assertEqual(res.returncode, 0 if ok else 1, res.stdout)
        return res

    def hook(self, *args, shell=None, wrap=(), **env):
        """hook.sh as installed, run as udev, modprobe or systemd would, with R set to the fake root."""
        script = self.read(SCRIPT).replace("R=''", f"R='{self.root}'", 1)
        return subprocess.run([*wrap, shell or self.dash, "-c", script, "hook.sh", *args],
                              env={"PATH": self.remote, "FAKE_LOG": self.log, "FAKE_ROOT": self.root,
                                   "FAKE_STICK": self.stick, **env},
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=60)

    def run_files(self):
        return sorted(os.path.relpath(os.path.join(d, n), self.root)
                      for d, dirs, files in os.walk(os.path.join(self.root, "run")) for n in dirs + files)

    def test_install_needs_the_opt_in_and_port_22(self):
        res = self.run_wrapper("--hook-install")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("MPFREE_ACCEPT_HOOK=YES", res.stdout)
        res = self.run_wrapper("--hook-install", MPC_PORT="2222", MPFREE_ACCEPT_HOOK="YES")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("run it with MPC_PORT=22", res.stdout)
        self.assertEqual(self.calls("ssh: "), [], "nothing may reach the MPC")
        self.assertEqual(self.tree(), self.before)

    def test_install_replaces_the_canary_and_checks_the_stick(self):
        # The canary as installed on the MPC now: three files and the folders it made.
        for rel in CANARY:
            write(os.path.join(self.root, rel), f"{'R=' + chr(39) * 2 + chr(10) if rel.endswith('.sh') else ''}"
                                                f"{CANARY_MARK}\n# canary\n")
        write(os.path.join(self.root, "data/mpfree-hook/created-folders.txt"),
              "/data/mpfree-hook\n/data/system/etc/overlay/udev\n/data/system/etc/overlay/udev/rules.d\n"
              "/data/system/etc/overlay/modprobe.d\n")
        self.plug_in()
        res = self.install()
        out = res.stdout
        for rel in CANARY:
            self.assertIn(f"deleted the canary's /{rel}", out)
            self.assertFalse(os.path.exists(os.path.join(self.root, rel)))
        self.assertIn("stick: /dev/sda1 (partition 1 of a USB disk), at 6.05 s", out)
        self.assertIn("kexec files: kernel " + RELEASE + ", checked against SHA256SUMS", out)
        self.assertIn("command line: root=PARTUUID=(this stick) rootwait=60 ro panic=10", out)
        self.assertIn("kernel loaded (kexec --load)", out)
        self.assertIn("kernel unloaded (kexec --unload)", out)
        self.assertIn("CHECK-OK: at power-on with this stick in, the MPC would restart into it", out)
        self.assertIn("RESULT: the hook is installed, and the stick checks out. NEXT: switch the MPC off, then on "
                      "with the stick in.", out)
        work = os.path.join(self.root, "run/mpfree-hook-check")
        self.assertEqual(self.calls("kexec: "), [
            f"kexec: --load --kexec-syscall --dtb={work}/kexec/kexec.dtb --command-line={CMDLINE.strip()} "
            f"{work}/kexec/zImage",
            "kexec: --unload --kexec-syscall"])
        self.assertEqual(self.calls("mount: "), [f"mount: -t ext4 -o ro,noload,nodev,nosuid,noexec "
                                                 f"{self.root}/dev/sda1 {work}/stick"])
        self.assertEqual(self.calls("umount: "), [f"umount: {work}/stick"])
        self.assertEqual(self.read("sys/kernel/kexec_loaded"), "0\n")
        self.assertEqual(self.run_files(), [], "the check leaves nothing in /run")
        self.assertEqual(self.calls("systemd-run: ") + self.calls("systemctl: --no-block"), [])
        # Exactly the three files; the canary's folder list is kept for the undo.
        after = self.tree()
        self.assertEqual(sorted(set(after) - set(self.before)), [
            "data/mpfree-hook/", "data/mpfree-hook/created-folders.txt", SCRIPT,
            "data/system/etc/overlay/modprobe.d/", CONF, "data/system/etc/overlay/udev/",
            "data/system/etc/overlay/udev/rules.d/", RULE])
        self.assertTrue(self.read(SCRIPT).startswith("R=''\n" + MARK + "\n"))
        self.assertEqual(self.read(RULE).splitlines()[0], MARK)
        self.assertEqual([line for line in self.read(CONF).splitlines() if not line.startswith("#")],
                         ["install cfg80211 export PATH=/usr/sbin:/usr/bin:/sbin:/bin; /bin/sh "
                          "/data/mpfree-hook/hook.sh modprobe; /sbin/modprobe --ignore-install cfg80211 $CMDLINE_OPTS"])
        if shutil.which("udevadm"):
            check = subprocess.run(["udevadm", "verify", os.path.join(self.root, RULE)],
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            self.assertEqual(check.returncode, 0, check.stdout)
        # The undo takes everything away, the canary's folders too.
        res = self.run_wrapper("--hook-remove")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("RESULT: the hook is gone", res.stdout)
        self.assertEqual(self.tree(), self.before)

    def test_install_without_the_stick_then_again_with_it(self):
        res = self.install(ok=False)
        self.assertIn("written and read back:", res.stdout)
        self.assertIn("MPFree's stick is not in the MPC", res.stdout)
        self.assertIn("RESULT: the hook is installed; with no stick checked. NEXT: put MPFree's stick in the MPC and "
                      "run this again", res.stdout)
        self.assertEqual(self.read("data/mpfree-hook/created-folders.txt").split(),
                         ["/data/mpfree-hook", "/data/system/etc/overlay/udev",
                          "/data/system/etc/overlay/udev/rules.d", "/data/system/etc/overlay/modprobe.d"])
        installed = self.tree()
        self.plug_in()
        res = self.install()
        self.assertIn("already in place, unchanged:", res.stdout)
        self.assertIn("CHECK-OK", res.stdout)
        self.assertEqual(self.tree(), installed)
        res = self.run_wrapper("--hook-remove")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.tree(), self.before)

    def test_install_reports_a_stick_that_does_not_check_out(self):
        self.plug_in()
        self.write_sums(**{"kexec.dtb": "0" * 64})
        res = self.install(ok=False)
        self.assertIn("STOP: kexec.dtb on the stick does not match its SHA256SUMS", res.stdout)
        self.assertIn("RESULT: the hook is installed, but the check stopped (above). Take the stick out before you "
                      "next switch the MPC on (without it the hook does nothing), and send the results.", res.stdout)
        self.assertEqual(self.calls("kexec: "), [])
        self.assertEqual(self.run_files(), [])

    def test_check_reads_the_stick_where_akai_mounted_it_without_links(self):
        # Akai's edisksd mounts every partition of a USB disk at /media/..., with nosymfollow (seen on the MPC
        # on 2026-11-15), so /boot/mpfree-kexec/zImage cannot be followed there. The same mount is made here in
        # a private mount namespace, so the kernel enforces it as on the MPC.
        unshare, mount = shutil.which("unshare"), shutil.which("mount")
        if not unshare or not mount:
            self.skipTest("needs unshare and mount (util-linux)")
        self.install(ok=False)
        self.plug_in()
        media = "/media/c73b610c-cb57-4e47-84f5-4ff2ae94d259"
        os.makedirs(self.root + media)
        with open(os.path.join(self.root, "proc/mounts"), "a") as fh:
            fh.write(f"/dev/sda1 {media} ext4 rw,nosuid,nodev,noexec,relatime,nosymfollow 0 0\n")
        akai_mount = (f'{mount} --bind "$1" "$2" && {mount} -o remount,bind,nosymfollow "$2" || exit 77; '
                      '[ -f "$2/boot/mpfree-kexec/zImage" ] && exit 78; shift 2; exec "$@"')
        ns = ["-m"] if os.geteuid() == 0 else ["-r", "-m"]
        wrap = (unshare, *ns, "--propagation", "private", "/bin/sh", "-c", akai_mount, "sh", self.stick,
                self.root + media)
        open(self.log, "w").close()
        res = self.hook("check", wrap=wrap)
        if res.returncode in (77, 78) or "unshare: " in res.stdout:
            self.skipTest("cannot make a nosymfollow mount here: " + res.stdout.strip())
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"partition 1 is mounted already at {media}; read from there", res.stdout)
        self.assertIn("CHECK-OK", res.stdout)
        self.assertEqual(self.calls("mount: ") + self.calls("umount: "), [], "Akai's mount is used and left alone")
        self.assertEqual(self.calls("kexec: --unload"), ["kexec: --unload --kexec-syscall"])
        self.assertEqual(self.read("sys/kernel/kexec_loaded"), "0\n")
        self.assertEqual(self.run_files(), [], "the copies in /run are removed again")

    def test_install_stops_on_files_it_did_not_write(self):
        for rel in (RULE, CONF, CANARY[0]):
            write(os.path.join(self.root, rel), "# someone else's\n")
            self.before = self.tree()
            res = self.install(ok=False)
            self.assertIn(f"/{rel} exists and is not", res.stdout)
            self.assertEqual(self.tree(), self.before)
            os.unlink(os.path.join(self.root, rel))

    def test_install_stops_without_kexec_or_kmod(self):
        write(os.path.join(self.root, "proc/sys/kernel/kexec_load_disabled"), "1\n")
        res = self.install(ok=False)
        self.assertIn("STOP: kexec is turned off on this system", res.stdout)
        write(os.path.join(self.root, "proc/sys/kernel/kexec_load_disabled"), "0\n")
        write(os.path.join(self.root, "sbin/modprobe"), "#!/bin/sh\nexit 1\n", 0o755)
        res = self.install(ok=False)
        self.assertIn("is not kmod's modprobe", res.stdout)
        self.assertEqual(self.tree(), self.before)

    def test_triggers_start_the_jump_once_and_only_with_the_stick(self):
        self.install(ok=False)
        open(self.log, "w").close()
        # No stick: the modprobe line notes it and starts nothing; udev's rule would not even run the hook.
        for shell in (os.path.join(self.remote, "sh"), self.dash):
            res = self.hook("modprobe", shell=shell)
            self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.read("run/mpfree-hook.txt"), "mpfree-hook: modprobe at 6.05 s: MPFree's stick is not in\n" * 2)
        self.assertFalse(os.path.exists(os.path.join(self.root, "run/mpfree-hook")))
        self.assertEqual(self.calls("systemd-run: "), [])
        # With the stick (BusyBox's shell, as on the MPC): the first trigger starts the jump, in the background.
        self.plug_in()
        res = self.hook("modprobe", shell=os.path.join(self.remote, "sh"))
        self.assertEqual(res.returncode, 0, res.stdout)
        for _ in range(100):
            if self.calls("systemd-run: "):
                break
            time.sleep(0.05)
        self.assertEqual(self.calls("systemd-run: "), [JUMP_START])
        self.assertEqual(self.read("dev/kmsg"), "mpfree-hook: MPFree's stick is in: modprobe starts the jump at 6.05 s\n")
        self.assertTrue(os.path.isdir(os.path.join(self.root, "run/mpfree-hook")))
        self.hook("udev", shell=os.path.join(self.remote, "sh"))
        self.hook("modprobe")
        self.assertEqual(len(self.calls("systemd-run: ")), 1, "once per boot")
        self.assertEqual(self.calls("systemctl: "), [], "the triggers ask systemd for nothing else")
        # Late in the boot (a stick plugged in later, a module loaded later): never.
        os.rmdir(os.path.join(self.root, "run/mpfree-hook"))
        write(os.path.join(self.root, "proc/uptime"), "15.20 9.10\n")
        self.hook("udev", shell=os.path.join(self.remote, "sh"))
        self.assertEqual(len(self.calls("systemd-run: ")), 1)
        self.assertIn("mpfree-hook: udev at 15.20 s: past 15 s, so not this boot\n", self.read("run/mpfree-hook.txt"))
        self.assertTrue(os.path.isdir(os.path.join(self.root, "run/mpfree-hook")))

    def test_the_modprobe_line_still_loads_the_module(self):
        self.install(ok=False)
        self.plug_in()
        open(self.log, "w").close()
        [line] = [l for l in self.read(CONF).splitlines() if l.startswith("install cfg80211 ")]
        command = line[len("install cfg80211 "):].replace("$CMDLINE_OPTS", "")
        command = command.replace("PATH=/usr/sbin:/usr/bin:/sbin:/bin", f"PATH={self.remote}")
        command = command.replace("; /sbin/modprobe ", f"; {os.path.join(self.remote, 'modprobe')} ")
        script = os.path.join(self.tmp, "hook-copy.sh")
        write(script, self.read(SCRIPT).replace("R=''", f"R='{self.root}'", 1), 0o755)
        command = command.replace("/bin/sh /data/mpfree-hook/hook.sh", f"{self.remote}/sh {script}")
        for present in (True, False):
            res = subprocess.run([os.path.join(self.remote, "sh"), "-c", command],
                                 env={"FAKE_LOG": self.log, "FAKE_ROOT": self.root}, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, text=True, timeout=30)
            self.assertEqual(res.returncode, 0, res.stdout)
            self.assertEqual(self.calls("modprobe: "), ["modprobe: --ignore-install cfg80211"])
            open(self.log, "w").close()
            if present:
                os.unlink(script)  # without the script the module loads all the same

    def jump(self, **env):
        os.makedirs(os.path.join(self.root, "run/mpfree-hook"), exist_ok=True)
        return self.hook("jump", **env)

    def test_jump_loads_the_stick_and_restarts_into_it(self):
        self.install(ok=False)
        self.plug_in()
        open(self.log, "w").close()
        res = self.jump()
        self.assertEqual(res.returncode, 0, res.stdout)
        kmsg = self.read("run/mpfree-hook.txt")  # every kernel log line also goes there
        for line in ("jump service started at 6.05 s; Akai's app: inactive",
                     "stick: /dev/sda1 (partition 1 of a USB disk), at 6.05 s",
                     f"kexec files: kernel {RELEASE}, checked against SHA256SUMS",
                     "kernel loaded (kexec --load) at 6.05 s", "restarting into MPFree (kexec.target) at 6.05 s"):
            self.assertIn("mpfree-hook: " + line + "\n", kmsg)
        self.assertNotIn("STOP", kmsg)
        work = os.path.join(self.root, "run/mpfree-hook")
        self.assertEqual(self.calls("kexec: "), [
            f"kexec: --load --kexec-syscall --dtb={work}/kexec/kexec.dtb --command-line={CMDLINE.strip()} "
            f"{work}/kexec/zImage"])
        self.assertEqual(self.calls("systemctl: --no-block"),
                         ["systemctl: --no-block --job-mode=replace-irreversibly start kexec.target"])
        self.assertEqual(self.calls("umount: "), [f"umount: {work}/stick"])
        self.assertEqual(self.read("sys/kernel/kexec_loaded"), "1\n", "left loaded for the restart")
        with open(os.path.join(work, "kexec/zImage"), "rb") as fh:
            self.assertEqual(fh.read(), KERNEL)
        self.assertEqual(self.calls("sleep: "), [])

    def expect_stop(self, message, loaded=False, asked=False, **env):
        open(self.log, "w").close()
        write(os.path.join(self.root, "run/mpfree-hook.txt"), "")
        res = self.jump(**env)
        self.assertEqual(res.returncode, 1, res.stdout)
        kmsg = self.read("run/mpfree-hook.txt")  # every kernel log line also goes there
        self.assertIn("mpfree-hook: STOP: " + message, kmsg)
        self.assertIn("mpfree-hook: nothing was started: Akai's system starts as usual\n", kmsg)
        self.assertEqual(self.calls("systemctl: --no-block"),
                         ["systemctl: --no-block --job-mode=replace-irreversibly start kexec.target"] if asked else [])
        self.assertEqual(self.read("sys/kernel/kexec_loaded"), "0\n")
        self.assertEqual(self.calls("kexec: --unload"), ["kexec: --unload --kexec-syscall"] if loaded else [])
        self.assertEqual(self.run_files(), ["run/mpfree-hook", "run/mpfree-hook.txt"], "only the marker and the log")
        return res

    def test_jump_stops_and_cleans_up_on_any_problem(self):
        self.install(ok=False)
        with self.subTest("no stick"):
            self.expect_stop("MPFree's stick is not in")
        self.plug_in()
        with self.subTest("damaged file"):
            self.write_sums(zImage="1" * 64)
            self.expect_stop("zImage on the stick does not match its SHA256SUMS")
            self.write_sums()
        with self.subTest("another stick's command line"):
            other = CMDLINE.replace(STICK_UUID, "99999999-0000-4000-8000-000000000000").encode()
            with open(os.path.join(self.stick, "boot/mpfree-kexec/cmdline.txt"), "wb") as fh:
                fh.write(other)
            self.write_sums(**{"cmdline.txt": sha(other)})
            self.expect_stop("cmdline.txt does not start the system on this stick's partition 1")
            with open(os.path.join(self.stick, "boot/mpfree-kexec/cmdline.txt"), "wb") as fh:
                fh.write(CMDLINE.encode())
            self.write_sums()
        with self.subTest("an older stick without the kexec files"):
            os.rename(os.path.join(self.stick, "boot/mpfree-kexec"), os.path.join(self.tmp, "kexec-aside"))
            self.expect_stop("the stick has no /boot/mpfree-kexec")
            os.rename(os.path.join(self.tmp, "kexec-aside"), os.path.join(self.stick, "boot/mpfree-kexec"))
        with self.subTest("a kernel link out of the stick"):
            link = os.path.join(self.stick, "boot/mpfree-kexec/zImage")
            os.unlink(link)
            os.symlink("/etc/shadow", link)
            self.expect_stop("/boot/mpfree-kexec/zImage is not a link to ../zImage-<release>")
            os.unlink(link)
            os.symlink("../zImage-" + RELEASE, link)
        with self.subTest("partition not mountable"):
            self.expect_stop("could not mount /dev/sda1 read-only", FAKE_MOUNT_FAIL="1")
        with self.subTest("kexec refuses"):
            self.expect_stop("kexec --load refused the stick's kernel", loaded=False, FAKE_KEXEC_FAIL="1")
        with self.subTest("a firmware updater keeps running"):
            res = self.expect_stop("touch-fw-update.service is still running after 20 s",
                                   FAKE_BUSY_UNIT="touch-fw-update.service")
            self.assertEqual(len(self.calls("sleep: ")), 20)
            self.assertIn("mpfree-hook: waiting for touch-fw-update.service to finish\n",
                          self.read("run/mpfree-hook.txt"))
        with self.subTest("a flasher program runs"):
            self.expect_stop("dfu-util is still running after 20 s", FAKE_PIDOF_HIT="dfu-util")
        with self.subTest("Akai's controller update"):
            write(os.path.join(self.root, "proc/321/cmdline"), "sh\0/usr/share/Akai/SME0/Firmware/update.sh\0")
            self.expect_stop("Akai's controller update.sh is still running after 20 s")
            shutil.rmtree(os.path.join(self.root, "proc/321"))
        with self.subTest("systemd refuses the restart"):
            self.expect_stop("systemctl could not start kexec.target", loaded=True, asked=True, FAKE_START_RC="1")
        with self.subTest("not started by a trigger"):
            shutil.rmtree(os.path.join(self.root, "run/mpfree-hook"))
            open(self.log, "w").close()
            res = self.hook("jump")
            self.assertEqual(res.returncode, 0, res.stdout)
            self.assertEqual(self.calls("mount: ") + self.calls("kexec: "), [])

    def test_report_reads_only(self):
        self.install(ok=False)
        installed = self.tree()
        write(os.path.join(self.root, "run/mpfree-hook.txt"), "mpfree-hook: modprobe at 6.05 s: MPFree's stick is not in\n")
        res = self.run_wrapper("--hook-report")
        self.assertEqual(res.returncode, 0, res.stdout)
        [report] = glob.glob(os.path.join(self.reports, "mpc-one-hook-report-*.txt"))
        with open(report) as fh:
            text = fh.read()
        self.assertIn("MPFREE BOOT HOOK REPORT - READ ONLY", text)
        self.assertIn("MPFree's stick in now: no\n", text)
        self.assertIn("hook installed before this boot: NO, ", text)
        self.assertIn("NEXT: switch the MPC off and on", text)
        self.assertIn("=== What the hook did at this power-on (/run/mpfree-hook.txt) ===\n"
                      "mpfree-hook: modprobe at 6.05 s: MPFree's stick is not in\n", text)
        self.assertIn("  /data/mpfree-hook/hook.sh\n", text)
        self.assertIn("Started InMusic MPC Application", text)
        self.assertIn("END OF BOOT HOOK REPORT", text)
        for rel in (SCRIPT, RULE, CONF):
            old = time.time() - 86400
            os.utime(os.path.join(self.root, rel), (old, old))
        write(os.path.join(self.root, "proc/uptime"), "3600.50 9.10\n")
        self.plug_in()
        res = self.run_wrapper("--hook-report")
        self.assertIn("hook installed before this boot: yes\n", res.stdout)
        self.assertIn("MPFree's stick in now: yes (/dev/sda1)\n", res.stdout)
        self.assertEqual(self.tree(), installed)


class StaticTests(unittest.TestCase):
    def code(self, path):
        with open(path) as fh:
            return "\n".join(line for line in fh.read().splitlines() if not line.lstrip().startswith("#"))

    def test_scripts_parse(self):
        for path in (INSTALL, REMOVE, REPORT):
            subprocess.run(["sh", "-n", path], check=True)

    def test_report_only_reads(self):
        code = self.code(REPORT)
        for forbidden in ("rm ", "mv ", "cp ", "mkdir", "chmod", "mount", "systemctl start", "systemctl stop",
                          "systemd-run", "kexec --", "/usr/sbin/kexec", "tee", " > ", ">>"):
            self.assertNotIn(forbidden, code)

    def test_install_writes_only_its_three_files(self):
        code = self.code(INSTALL)
        start, end = code.index("SCRIPT_TEXT=$(cat <<'MPFREE_EOF'"), code.index("MPFREE_EOF\n)\nRULE_TEXT")
        installer, hook = code[:start] + code[end:], code[start:end]
        for forbidden in ("mount ", "kexec --", "kexec\" --", "systemctl", "systemd-run", "fw_setenv", "dd ", "rm -r",
                          "/usr/lib/systemd", "/etc/systemd"):
            self.assertNotIn(forbidden, installer)
        # The hook itself: the stick read-only, kexec by its full path, nothing written but /run and the log.
        self.assertIn("mount -t ext4 -o ro,noload,nodev,nosuid,noexec ", hook)
        self.assertEqual(hook.count("mount -t"), 1)
        for forbidden in ("fw_setenv", "dd ", "mkfs", "mmcblk", "rw,", "rm -r", "/etc/systemd", " > /data",
                          ">> /data", "systemctl kexec", "reboot"):
            self.assertNotIn(forbidden, hook)
        writes = [line.strip() for line in hook.splitlines() if " > " in line or ">>" in line]
        for line in writes:
            self.assertRegex(line, r'(> /dev/null|> "\$R/dev/kmsg"|>> "\$R/run/mpfree-hook.txt")', line)


if __name__ == "__main__":
    unittest.main(verbosity=2)
