#!/usr/bin/env python3
"""Tests for MPFree's boot hook (collect-over-ssh.sh --hook-install, --hook-remove, --hook-report, and the
hook.sh they install). No device, no network.

The wrapper runs against the same stand-ins as test_hook_canary.py: a fake ssh that runs the streamed
scripts under BusyBox applets against a fake root folder (their first line R='' becomes that folder).
BusyBox's shell runs its own applets before anything in PATH, so the hook's steps that mount, sleep or look
for processes (the jump and the check) run under dash here, with the same BusyBox applets for everything
but the stand-ins for mount, umount, sleep, pidof, systemctl, systemd-run and modprobe. The installer runs
the check with $R/bin/sh, which is dash in the fake root and /bin/sh on the MPC.

The fake stick's soft-reboot files are made from offline-kexec-stick.py's own constants, so the hook and the
stick builder are tested to agree.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox or dash.
"""
import glob
import hashlib
import importlib.util
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
HOLD = ("acvs.service", "touch-fw-update.service", "xmos-update.service", "az01-usbsata-fixer.service")
JUMP_START = ("systemd-run: --no-block --collect --quiet --unit=mpfree-jump --description=MPFree: start from the "
              "stick --property=Type=oneshot --property=TimeoutStartSec=45 "
              + " ".join(f"--property=Before={u}" for u in HOLD) + " /bin/sh /data/mpfree-hook/hook.sh jump")
SOFT_REBOOT = "systemctl: --no-block --job-mode=replace-irreversibly start soft-reboot.target"
KEXEC_TARGET = "systemctl: --no-block --job-mode=replace-irreversibly start kexec.target"
STICK_UUID = "0b1c2d3e-4f50-4617-8293-a4b5c6d7e8f9"
KERNEL = b"FAKE AKAI ZIMAGE " * 64
KCMD = (f"root=PARTUUID={STICK_UUID} rootwait=60 ro panic=10 rfkill.default_state=0 "
        "sysctl.kernel.kexec_load_disabled=1")
# A --chain stick's: kexec left on for the stick's system, LoadPin not enforcing, and the chain asked for.
CHAIN_KCMD = f"root=PARTUUID={STICK_UUID} rootwait=60 ro panic=10 rfkill.default_state=0 loadpin.enforce=0 mpfree.chain=1"


def stick_module():
    spec = importlib.util.spec_from_file_location("offline_kexec_stick", os.path.join(TOOLS, "offline-kexec-stick.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


STICK = stick_module()
# kmod's modprobe as the MPC has it: --version as kmod, every call logged, a module named in
# $FAKE_MODPROBE_FAIL fails (with --dry-run too: it is not there).
FAKE_MODPROBE = r'''#!/bin/sh
if [ "$1" = --version ]; then echo "kmod version 31"; echo "+ZSTD +XZ"; exit 0; fi
echo "modprobe: $*" >> "$FAKE_LOG"
for a; do [ -n "${FAKE_MODPROBE_FAIL:-}" ] && [ "$a" = "$FAKE_MODPROBE_FAIL" ] && exit 1; done
exit 0
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  is-active)
    if [ "$2" = "${FAKE_BUSY_UNIT:-}" ]; then echo activating
    elif [ "$2" = acvs.service ]; then echo "${FAKE_ACVS:-inactive}"
    else echo inactive; fi ;;
  --no-block) case "$*" in *kexec.target*) exit "${FAKE_KEXEC_START_RC:-0}" ;; esac; exit "${FAKE_START_RC:-0}" ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''
# Akai's kexec-tools under LoadPin, as the probe on 3.10.1 found it: the old buffer syscall denied (load and
# unload), a device tree refused in file mode, the file syscall accepted. kexec_loaded follows.
FAKE_KEXEC = r'''#!/bin/sh
echo "kexec: $*" >> "$FAKE_LOG"
case "$*" in *--kexec-syscall*) echo "kexec_load failed: Operation not permitted" >&2; exit 1 ;; esac
case "$1" in
  --load)
    case "$*" in *--dtb*) echo "DTB file cannot be specified in file mode" >&2; exit 1 ;; esac
    [ -n "${FAKE_KEXEC_REFUSE:-}" ] && { echo "kexec_file_load failed: Operation not permitted" >&2; exit 1; }
    echo 1 > "$FAKE_ROOT/sys/kernel/kexec_loaded"
    # A load that reports a failure but leaves the kernel loaded (kexec-tools exits 1 on a late error).
    [ -n "${FAKE_KEXEC_FAIL_LOADED:-}" ] && { echo "kexec: late error" >&2; exit 1; }
    [ -n "${FAKE_KEXEC_UPDATE:-}" ] && { mkdir -p "$FAKE_ROOT/proc/999"; printf 'sh\0/usr/share/Akai/SME0/Firmware/update.sh\0' > "$FAKE_ROOT/proc/999/cmdline"; }
    exit 0 ;;
  --unload)
    [ -n "${FAKE_UNLOAD_FAIL:-}" ] && exit 1
    echo 0 > "$FAKE_ROOT/sys/kernel/kexec_loaded"; exit 0 ;;
esac
exit 2
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


def sha(data):
    return hashlib.sha256(data).hexdigest()


class HookFixture(unittest.TestCase):
    """The fake MPC, the stand-ins and the fake stick; test_stick_setup.py uses them too."""

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
                           ("journalctl", FAKE_JOURNALCTL), ("modprobe", FAKE_MODPROBE), ("sleep", FAKE_SLEEP),
                           ("mount", FAKE_MOUNT), ("umount", FAKE_UMOUNT)):
            write(os.path.join(remote, name), text, 0o755)
        self.remote = remote
        r = self.root
        write(os.path.join(r, "proc/sys/kernel/osrelease"), RELEASE + "\n")
        write(os.path.join(r, "proc/device-tree/compatible"), "inmusic,acva\0inmusic,az01\0rockchip,rk3288\0")
        write(os.path.join(r, "proc/mounts"), MOUNTS)
        write(os.path.join(r, "proc/uptime"), "6.05 9.10\n")
        write(os.path.join(r, "proc/sys/kernel/modprobe"), "/sbin/modprobe\n")
        write(os.path.join(r, "proc/cmdline"), "console=ttyS2 root=/dev/dm-0 ro rootwait\n")
        write(os.path.join(r, "sbin/modprobe"), FAKE_MODPROBE, 0o755)
        for unit in ("soft-reboot.target", "systemd-soft-reboot.service"):
            write(os.path.join(r, "usr/lib/systemd/system", unit), "[Unit]\n")
        os.makedirs(os.path.join(r, "bin"))
        os.symlink(self.dash, os.path.join(r, "bin/sh"))
        write(os.path.join(r, "data/system/etc/overlay/hostname"), "mpc-one\n")
        write(os.path.join(r, "dev/kmsg"), "")
        os.makedirs(os.path.join(r, "run"))
        # For the kexec-file route: Akai's kernel on its root, kexec-tools, nothing loaded, kexec allowed.
        write(os.path.join(r, "boot/zImage-" + RELEASE), KERNEL.decode())
        write(os.path.join(r, "usr/sbin/kexec"), FAKE_KEXEC, 0o755)
        write(os.path.join(r, "sys/kernel/kexec_loaded"), "0\n")
        write(os.path.join(r, "proc/sys/kernel/kexec_load_disabled"), "0\n")
        self.before = self.tree()
        self.make_stick()
        self.key = os.path.join(self.tmp, "key")
        write(self.key, "not a real key\n", 0o600)
        # Developer mode's host key, as --devmode-trust records it after a typed TRUST.
        write(os.path.join(self.home, ".ssh", "known_hosts"), f"{TEST_IP} {self.hostkey}\n")
        self.env = {
            "HOME": self.home, "LC_ALL": "C", "PATH": fakebin + os.pathsep + "/usr/bin:/bin",
            "MPC_KEY": self.key, "MPC_REPORT_DIR": self.reports, "MPC_PORT": "22",
            "MPC_IP": TEST_IP, "MPC_IFACE": TEST_IFACE, "MPC_HOST_IP_CIDR": TEST_HOST, "MPC_EXPECT_MAC": TEST_MAC,
            "FAKE_LOG": self.log, "FAKE_REMOTE_PATH": remote, "FAKE_ROOT": self.root, "FAKE_STICK": self.stick,
            "FAKE_HOSTKEY": self.hostkey, "FAKE_RELEASE": RELEASE,
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def make_stick(self, modules=(), kexec_file=None, kernel=KERNEL, chain=False):
        """MPFree's stick, partition 1: the parts of the system the hook looks at, as offline-kexec-stick.py
        writes them. With `kexec_file` (a command line), the three files of --kexec-file too; with `chain`,
        the chain's service and the stick's own kernel files (--chain)."""
        shutil.rmtree(self.stick, ignore_errors=True)
        sr = STICK.SOFTREBOOT_DIR.lstrip("/")
        files = {STICK.INTERNAL_OFF_FILES[0]: STICK.INTERNAL_OFF_SCRIPT,
                 STICK.INTERNAL_OFF_FILES[1]: STICK.INTERNAL_OFF_UNIT,
                 STICK.INTERNAL_OFF_FILES[2]: STICK.INTERNAL_OFF_RULE,
                 STICK.SOFTREBOOT_DIR + "/PROTOCOL": STICK.SOFTREBOOT_PROTOCOL + "\n",
                 STICK.SOFTREBOOT_DIR + "/release": RELEASE + "\n"}
        if modules:
            files[STICK.SOFTREBOOT_DIR + "/preload-modules.txt"] = "".join(m + "\n" for m in modules)
        if kexec_file is not None:
            for name, text in zip(STICK.KEXEC_FILE_FILES, (STICK.KEXEC_FILE_JUMP, kexec_file,
                                                           f"{sha(kernel)}  /boot/zImage-{RELEASE}")):
                files[f"{STICK.SOFTREBOOT_DIR}/{name}"] = text + "\n"
        if chain:
            files[STICK.CHAIN_FILES[0]] = STICK.CHAIN_SCRIPT
            files[STICK.CHAIN_FILES[1]] = STICK.CHAIN_UNIT
            files[STICK.KEXEC_DIR + "/kexec.dtb"] = "FAKE STICK DEVICE TREE\n"
            files[STICK.KEXEC_DIR + "/cmdline.txt"] = KCMD + "\n"
            files[STICK.KEXEC_DIR + "/SHA256SUMS"] = "0" * 64 + "  zImage\n"
        for path, text in files.items():
            write(os.path.join(self.stick, path.lstrip("/")), text, 0o755 if path.endswith(".sh") else 0o644)
        self.write_sums()
        write(os.path.join(self.stick, "usr/lib/systemd/systemd"), "#!/bin/sh\n# FAKE-SYSTEMD\n", 0o755)
        write(os.path.join(self.stick, "usr/lib/os-release"), "ID=mpfree-test\n")
        os.makedirs(os.path.join(self.stick, "etc/systemd/system/sysinit.target.wants"))
        os.symlink(STICK.INTERNAL_OFF_FILES[1],
                   os.path.join(self.stick, "etc/systemd/system/sysinit.target.wants/mpfree-test-internal-off.service"))
        if chain:
            os.symlink(STICK.CHAIN_FILES[1], os.path.join(self.stick, STICK.CHAIN_WANTS.lstrip("/")))
        self.assertTrue(os.path.isdir(os.path.join(self.stick, sr)))

    def write_sums(self, **override):
        sr = os.path.join(self.stick, STICK.SOFTREBOOT_DIR.lstrip("/"))
        listed = list(STICK.INTERNAL_OFF_FILES) + [STICK.SOFTREBOOT_DIR + "/PROTOCOL", STICK.SOFTREBOOT_DIR + "/release"]
        if os.path.exists(os.path.join(sr, "preload-modules.txt")):
            listed.append(STICK.SOFTREBOOT_DIR + "/preload-modules.txt")
        for name in STICK.KEXEC_FILE_FILES:
            if os.path.exists(os.path.join(sr, name)):
                listed.append(f"{STICK.SOFTREBOOT_DIR}/{name}")
        if os.path.exists(os.path.join(self.stick, STICK.CHAIN_FILES[0].lstrip("/"))):
            listed += [*STICK.CHAIN_FILES, STICK.KEXEC_DIR + "/kexec.dtb", STICK.KEXEC_DIR + "/cmdline.txt",
                       STICK.KEXEC_DIR + "/SHA256SUMS"]
        sums = {}
        for p in listed:
            with open(os.path.join(self.stick, p.lstrip("/")), "rb") as fh:
                sums[p] = override.get(p) or sha(fh.read())
        with open(os.path.join(sr, "SHA256SUMS"), "w") as fh:
            fh.write("".join(f"{v}  {k}\n" for k, v in sums.items() if v != "-"))

    def plug_in(self):
        r = self.root
        block = os.path.join(r, "sys/devices/platform/ff540000.usb/usb2/2-1/2-1:1.0/host0/target0:0:0/0:0:0:0/"
                                "block/sda/sda1")
        os.makedirs(block)
        os.makedirs(os.path.join(r, "sys/class/block"), exist_ok=True)
        os.symlink(os.path.relpath(block, os.path.join(r, "sys/class/block")), os.path.join(r, "sys/class/block/sda1"))
        write(os.path.join(r, "dev/sda1"), "")
        os.makedirs(os.path.join(r, "dev/disk/by-partlabel"), exist_ok=True)
        os.symlink("../../sda1", os.path.join(r, "dev/disk/by-partlabel/mpfree-test-root"))
        os.makedirs(os.path.join(r, "dev/disk/by-partuuid"), exist_ok=True)
        os.symlink("../../sda1", os.path.join(r, "dev/disk/by-partuuid", STICK_UUID))

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


class BootHookTests(HookFixture):
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
        # The check runs as `sh -c "<the hook>"`, so the hook's own text is in a process's command line. The second
        # power-on's install (2026-11-15) took that for Akai's controller update.
        with open(INSTALL) as fh:
            write(os.path.join(self.root, "proc/4242/cmdline"), "sh\0-c\0" + fh.read() + "\0")
        res = self.install()
        out = res.stdout
        self.assertNotIn("is running now", out)
        for rel in CANARY:
            self.assertIn(f"deleted the canary's /{rel}", out)
            self.assertFalse(os.path.exists(os.path.join(self.root, rel)))
        self.assertIn("stick: /dev/sda1 (partition 1 of a USB disk), at 6.05 s", out)
        self.assertIn("the stick's system switches the MPC's eMMC and SD card off first: its files check out against "
                      "SHA256SUMS", out)
        self.assertIn(f"kernel {RELEASE}; the stick's system is for it", out)
        self.assertIn("modules to load first: none listed", out)
        self.assertIn("CHECK-OK: at power-on with this stick in, the MPC would soft-reboot into it", out)
        self.assertIn("RESULT: the hook is installed, and the stick checks out. NEXT: switch the MPC off, then on "
                      "with the stick in.", out)
        self.assertNotIn("note: the kernel command line", out)
        work = os.path.join(self.root, "run/mpfree-hook-check")
        self.assertEqual(self.calls("mount: "), [f"mount: -t ext4 -o ro,noload,nodev,nosuid,noexec "
                                                 f"{self.root}/dev/sda1 {work}/stick"])
        self.assertEqual(self.calls("umount: "), [f"umount: {work}/stick"])
        self.assertEqual(self.run_files(), [], "the check leaves nothing in /run, and no /run/nextroot")
        self.assertEqual(self.calls("systemd-run: ") + self.calls("systemctl: --no-block") + self.calls("modprobe: "),
                         [])
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

    def test_install_replaces_the_kexec_hook_on_the_mpc_now(self):
        # The hook installed on the MPC today is the kexec version: same three files, same marker line.
        for rel in (SCRIPT, RULE, CONF):
            write(os.path.join(self.root, rel), f"{'R=' + chr(39) * 2 + chr(10) if rel == SCRIPT else ''}{MARK}\n"
                                                "# the kexec version\n", 0o755 if rel == SCRIPT else 0o644)
        write(os.path.join(self.root, "data/mpfree-hook/created-folders.txt"), "/data/mpfree-hook\n")
        self.before = self.tree()
        self.plug_in()
        res = self.install()
        self.assertIn("an earlier version of the hook is in place; it is replaced", res.stdout)
        self.assertIn("soft-reboot.target", self.read(SCRIPT))
        self.assertNotIn("--kexec-syscall", self.read(SCRIPT))
        self.assertIn("CHECK-OK", res.stdout)
        self.assertEqual(sorted(set(self.tree()) - set(self.before)), [])

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
        self.write_sums(**{"/etc/mpfree-test/internal-off.sh": "0" * 64})
        res = self.install(ok=False)
        self.assertIn("STOP: /etc/mpfree-test/internal-off.sh on the stick does not match its SHA256SUMS", res.stdout)
        self.assertIn("RESULT: the hook is installed, but the check stopped (above). Take the stick out before you "
                      "next switch the MPC on (without it the hook does nothing), and send the results.", res.stdout)
        self.assertEqual(self.run_files(), [])

    def test_check_looks_up_the_modules_and_notes_the_command_line(self):
        self.install(ok=False)
        self.plug_in()
        self.make_stick(modules=("snd_usb_audio", "hid_multitouch"))
        write(os.path.join(self.root, "proc/cmdline"), "root=/dev/dm-0 systemd.show_status=0 quiet\n")
        open(self.log, "w").close()
        res = self.hook("check")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("modules to load first, all found in Akai's system: snd_usb_audio hid_multitouch", res.stdout)
        self.assertIn("note: the kernel command line has systemd.show_status=0, which the stick's system also reads",
                      res.stdout)
        self.assertEqual(self.calls("modprobe: "), ["modprobe: --dry-run --quiet snd_usb_audio",
                                                    "modprobe: --dry-run --quiet hid_multitouch"])
        open(self.log, "w").close()
        res = self.hook("check", FAKE_MODPROBE_FAIL="hid_multitouch")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: Akai's system has no module hid_multitouch (the stick needs it)", res.stdout)
        self.assertEqual(self.run_files(), [])

    def test_check_reads_the_stick_where_akai_mounted_it_without_links(self):
        # Akai's edisksd mounts every partition of a USB disk at /media/..., with nosymfollow (seen on the MPC
        # on 2026-11-15). The check reads the stick there; the one link it needs (the internal-off service in
        # sysinit.target.wants) is read with readlink, never followed. The same mount is made here in a
        # private mount namespace, so the kernel enforces it as on the MPC.
        unshare, mount = shutil.which("unshare"), shutil.which("mount")
        if not unshare or not mount:
            self.skipTest("needs unshare and mount (util-linux)")
        self.install(ok=False)
        self.plug_in()
        media = "/media/c73b610c-cb57-4e47-84f5-4ff2ae94d259"
        os.makedirs(self.root + media)
        with open(os.path.join(self.root, "proc/mounts"), "a") as fh:
            fh.write(f"/dev/sda1 {media} ext4 rw,nosuid,nodev,noexec,relatime,nosymfollow 0 0\n")
        wants = "etc/systemd/system/sysinit.target.wants/mpfree-test-internal-off.service"
        akai_mount = (f'{mount} --bind "$1" "$2" && {mount} -o remount,bind,nosymfollow "$2" || exit 77; '
                      f'cat "$2/{wants}" > /dev/null 2>&1 && exit 78; shift 2; exec "$@"')
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
        self.assertEqual(self.run_files(), [])

    def test_install_stops_on_files_it_did_not_write(self):
        for rel in (RULE, CONF, CANARY[0]):
            write(os.path.join(self.root, rel), "# someone else's\n")
            self.before = self.tree()
            res = self.install(ok=False)
            self.assertIn(f"/{rel} exists and is not", res.stdout)
            self.assertEqual(self.tree(), self.before)
            os.unlink(os.path.join(self.root, rel))

    def test_install_stops_on_another_mpc(self):
        compat = os.path.join(self.root, "proc/device-tree/compatible")
        write(compat, "inmusic,acv5\0inmusic,az01\0rockchip,rk3288\0")
        res = self.install(ok=False)
        self.assertIn("STOP: this is not an MPC One (no inmusic,acva in /proc/device-tree/compatible); MPFree has only "
                      "been tested on the MPC One (Gen 1)", res.stdout)
        write(compat, "inmusic,acva\0inmusic,az01\0rockchip,rk3288\0")
        self.assertEqual(self.tree(), self.before)

    def test_install_stops_without_soft_reboot_or_kmod(self):
        os.unlink(os.path.join(self.root, "usr/lib/systemd/system/soft-reboot.target"))
        res = self.install(ok=False)
        self.assertIn("STOP: this system's systemd has no soft-reboot.target (soft-reboot needs systemd 254 or later)",
                      res.stdout)
        write(os.path.join(self.root, "usr/lib/systemd/system/soft-reboot.target"), "[Unit]\n")
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

    def test_jump_mounts_the_stick_at_nextroot_and_soft_reboots(self):
        self.install(ok=False)
        self.plug_in()
        self.make_stick(modules=("snd_usb_audio",))
        open(self.log, "w").close()
        res = self.jump()
        self.assertEqual(res.returncode, 0, res.stdout)
        kmsg = self.read("run/mpfree-hook.txt")  # every kernel log line also goes there
        for line in ("jump service started at 6.05 s; Akai's app: inactive",
                     "stick: /dev/sda1 (partition 1 of a USB disk), at 6.05 s",
                     "the stick's system switches the MPC's eMMC and SD card off first: its files check out against "
                     "SHA256SUMS", f"kernel {RELEASE}; the stick's system is for it",
                     "modules loaded from Akai's system: snd_usb_audio",
                     "soft-reboot into MPFree (soft-reboot.target) at 6.05 s"):
            self.assertIn("mpfree-hook: " + line + "\n", kmsg)
        self.assertNotIn("STOP", kmsg)
        nextroot = os.path.join(self.root, "run/nextroot")
        # Read-only and without journal replay (nothing written to the stick), but its programs may run.
        self.assertEqual(self.calls("mount: "), [f"mount: -t ext4 -o ro,noload {self.root}/dev/sda1 {nextroot}"])
        self.assertEqual(self.calls("umount: "), [], "left mounted for the soft-reboot")
        self.assertEqual(self.calls("modprobe: "), ["modprobe: --quiet snd_usb_audio"])
        self.assertEqual(self.calls("systemctl: --no-block"), [SOFT_REBOOT])
        self.assertTrue(os.path.isfile(os.path.join(nextroot, "usr/lib/systemd/systemd")))
        self.assertEqual(self.read("run/mpfree-hook/soft-reboot"), "6.05\n", "tells the stick's system how it started")
        self.assertEqual(self.calls("sleep: "), [])
        self.assertEqual(self.calls("kexec: "), [], "a stick without --kexec-file never touches kexec")
        self.assertNotIn("kexec", kmsg)

    def test_jump_does_not_wait_for_the_usb_sata_fixer_that_always_runs(self):
        # First power-on, 2026-11-15: Akai's az01-usbsata-fixer runs all the time (still running at 7799 s), so
        # waiting for it stopped every jump after 20 s. Without a JMicron bridge attached it is not waited for.
        self.install(ok=False)
        self.plug_in()
        write(os.path.join(self.root, "sys/bus/usb/devices/2-1/idVendor"), "0781\n")  # the stick itself
        open(self.log, "w").close()
        res = self.jump(FAKE_PIDOF_HIT="az01-usbsata-fixer")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertNotIn("waiting for", self.read("run/mpfree-hook.txt"))
        self.assertEqual(self.calls("sleep: "), [])
        self.assertEqual(self.calls("systemctl: --no-block"), [SOFT_REBOOT])

    def expect_stop(self, message, asked=False, **env):
        open(self.log, "w").close()
        write(os.path.join(self.root, "run/mpfree-hook.txt"), "")
        res = self.jump(**env)
        self.assertEqual(res.returncode, 1, res.stdout)
        kmsg = self.read("run/mpfree-hook.txt")  # every kernel log line also goes there
        self.assertIn("mpfree-hook: STOP: " + message, kmsg)
        self.assertIn("mpfree-hook: nothing was started: Akai's system starts as usual\n", kmsg)
        self.assertEqual(self.calls("systemctl: --no-block"), [SOFT_REBOOT] if asked else [])
        mounts = [c for c in self.calls("mount: ") if "run/nextroot" in c]
        self.assertEqual(self.calls("umount: "), [f"umount: {self.root}/run/nextroot"] if mounts and
                         not env.get("FAKE_MOUNT_FAIL") else [], "what it mounted, it unmounts")
        # Nothing staged: with /run/nextroot left behind, a later `reboot` of Akai's system would soft-reboot.
        self.assertEqual(self.run_files(), ["run/mpfree-hook", "run/mpfree-hook.txt"], "only the marker and the log")
        return res

    def test_jump_stops_and_cleans_up_on_any_problem(self):
        self.install(ok=False)
        with self.subTest("no stick"):
            self.expect_stop("MPFree's stick is not in")
        with self.subTest("a stick the USB creator set to remove MPFree (partition 1 renamed)"):
            links = os.path.join(self.root, "dev/disk/by-partlabel")
            os.makedirs(links, exist_ok=True)
            os.symlink("../../sda1", os.path.join(links, "mpfree-test-root-off"))
            self.expect_stop("MPFree's stick is not in")
            os.unlink(os.path.join(links, "mpfree-test-root-off"))
        self.plug_in()
        sr = os.path.join(self.stick, "boot/mpfree-softreboot")
        with self.subTest("a damaged switch-off script"):
            with open(os.path.join(self.stick, "etc/mpfree-test/internal-off.sh"), "a") as fh:
                fh.write("exit 0\n")
            self.expect_stop("/etc/mpfree-test/internal-off.sh on the stick does not match its SHA256SUMS")
            self.make_stick()
        with self.subTest("a stick without the switch-off service"):
            self.write_sums(**{"/etc/mpfree-test/internal-off.sh": "-"})
            self.expect_stop("the stick's system does not carry /etc/mpfree-test/internal-off.sh (it must switch the "
                             "MPC's eMMC and SD card off)")
            self.make_stick()
        with self.subTest("the switch-off service not started first"):
            os.unlink(os.path.join(self.stick, "etc/systemd/system/sysinit.target.wants/mpfree-test-internal-off.service"))
            self.expect_stop("the stick's system does not start mpfree-test-internal-off.service first")
            self.make_stick()
        with self.subTest("another MPC model (the hook moved to an MPC that is not a One)"):
            compat = os.path.join(self.root, "proc/device-tree/compatible")
            write(compat, "inmusic,acvb\0inmusic,az01\0rockchip,rk3288\0")
            self.expect_stop("this is not an MPC One (no inmusic,acva in the device tree); MPFree has only been "
                             "tested on the MPC One")
            write(compat, "inmusic,acva\0inmusic,az01\0rockchip,rk3288\0")
        with self.subTest("a stick for another firmware"):
            write(os.path.join(sr, "release"), "6.1.0-other\n")
            self.write_sums()
            self.expect_stop(f"the stick's system is not for the running kernel {RELEASE} (rebuild it for this firmware)")
            self.make_stick()
        with self.subTest("another protocol"):
            write(os.path.join(sr, "PROTOCOL"), "mpfree-softreboot-2\n")
            self.expect_stop("/boot/mpfree-softreboot/PROTOCOL is not mpfree-softreboot-1")
            self.make_stick()
        with self.subTest("an older stick without the soft-reboot files"):
            shutil.rmtree(sr)
            self.expect_stop("the stick has no /boot/mpfree-softreboot")
            self.make_stick()
        with self.subTest("a path out of the stick in SHA256SUMS"):
            with open(os.path.join(sr, "SHA256SUMS"), "a") as fh:
                fh.write(f"{'0' * 64}  /etc/../../data/x\n")
            self.expect_stop("/boot/mpfree-softreboot/SHA256SUMS names an unusual path")
            self.make_stick()
        with self.subTest("no systemd on the stick"):
            os.unlink(os.path.join(self.stick, "usr/lib/systemd/systemd"))
            self.expect_stop("the stick has no /usr/lib/systemd/systemd to start")
            self.make_stick()
        with self.subTest("systemd on the stick not executable"):
            os.chmod(os.path.join(self.stick, "usr/lib/systemd/systemd"), 0o644)
            self.expect_stop("the stick has no /usr/lib/systemd/systemd to start")
            self.make_stick()
        with self.subTest("no os-release (systemd would restart Akai's system instead)"):
            os.unlink(os.path.join(self.stick, "usr/lib/os-release"))
            self.expect_stop("the stick has no os-release file")
            self.make_stick()
        with self.subTest("a module the stick needs is missing"):
            self.make_stick(modules=("snd_usb_audio",))
            self.expect_stop("could not load the module snd_usb_audio from Akai's system",
                             FAKE_MODPROBE_FAIL="snd_usb_audio")
            self.make_stick(modules=("bad/name",))
            self.expect_stop("/boot/mpfree-softreboot/preload-modules.txt names an unusual module")
            self.make_stick()
        with self.subTest("partition not mountable"):
            self.expect_stop("could not mount /dev/sda1 read-only", FAKE_MOUNT_FAIL="1")
        with self.subTest("already mounted by Akai's system"):
            with open(os.path.join(self.root, "proc/mounts"), "a") as fh:
                fh.write("/dev/sda1 /media/abc ext4 rw,nosuid,nodev,noexec 0 0\n")
            self.expect_stop("/dev/sda1 is mounted already (at /media/abc); not this boot")
            write(os.path.join(self.root, "proc/mounts"), MOUNTS)
        with self.subTest("something staged at /run/nextroot already"):
            write(os.path.join(self.root, "run/nextroot/etc/os-release"), "ID=other\n")
            open(self.log, "w").close()
            write(os.path.join(self.root, "run/mpfree-hook.txt"), "")
            res = self.jump()
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("STOP: /run/nextroot exists and is not an empty folder", self.read("run/mpfree-hook.txt"))
            self.assertEqual(self.calls("mount: ") + self.calls("umount: ") + self.calls("systemctl: --no-block"), [])
            self.assertTrue(os.path.exists(os.path.join(self.root, "run/nextroot/etc/os-release")), "left alone")
            shutil.rmtree(os.path.join(self.root, "run/nextroot"))
        with self.subTest("a firmware updater keeps running"):
            self.expect_stop("touch-fw-update.service is still running after 20 s",
                             FAKE_BUSY_UNIT="touch-fw-update.service")
            self.assertEqual(len(self.calls("sleep: ")), 20)
            self.assertIn("mpfree-hook: waiting for touch-fw-update.service to finish\n",
                          self.read("run/mpfree-hook.txt"))
        with self.subTest("a flasher program runs"):
            self.expect_stop("dfu-util is still running after 20 s", FAKE_PIDOF_HIT="dfu-util")
        with self.subTest("the USB-SATA fixer with a JMicron bridge attached"):
            write(os.path.join(self.root, "sys/bus/usb/devices/1-1/idVendor"), "152d\n")
            self.expect_stop("az01-usbsata-fixer (a JMicron USB-SATA bridge is attached) is still running after 20 s",
                             FAKE_PIDOF_HIT="az01-usbsata-fixer")
            shutil.rmtree(os.path.join(self.root, "sys/bus/usb/devices/1-1"))
        with self.subTest("Akai's controller update"):
            write(os.path.join(self.root, "proc/321/cmdline"), "sh\0/usr/share/Akai/SME0/Firmware/update.sh\0")
            self.expect_stop("Akai's controller update.sh is still running after 20 s")
            shutil.rmtree(os.path.join(self.root, "proc/321"))
        with self.subTest("systemd refuses the soft-reboot"):
            self.expect_stop("systemctl could not start soft-reboot.target", asked=True, FAKE_START_RC="1")
            self.assertFalse(os.path.exists(os.path.join(self.root, "run/mpfree-hook/soft-reboot")))
        with self.subTest("not started by a trigger"):
            shutil.rmtree(os.path.join(self.root, "run/mpfree-hook"))
            open(self.log, "w").close()
            res = self.hook("jump")
            self.assertEqual(res.returncode, 0, res.stdout)
            self.assertEqual(self.calls("mount: ") + self.calls("systemctl: --no-block"), [])

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

    # --- the opt-in kexec-file route (offline-kexec-stick.py --kexec-file) -----------------------------------

    def kexec_load_call(self, cmdline=KCMD):
        return f"kexec: --load --kexec-file-syscall --command-line={cmdline} {self.root}/boot/zImage-{RELEASE}"

    def test_kexec_file_stick_restarts_on_akais_own_kernel(self):
        self.install(ok=False)
        self.plug_in()
        self.make_stick(modules=("snd_usb_audio",), kexec_file=KCMD + " loadpin.enforce=0")
        open(self.log, "w").close()
        res = self.jump()
        self.assertEqual(res.returncode, 0, res.stdout)
        kmsg = self.read("run/mpfree-hook.txt")
        for line in (f"kernel {RELEASE}; the stick's system is for it",
                     "this stick asks to start on Akai's own kernel (kexec_file), command line: root=PARTUUID=(this "
                     "stick) rootwait=60 ro panic=10 rfkill.default_state=0 sysctl.kernel.kexec_load_disabled=1 "
                     "loadpin.enforce=0",
                     "Akai's kernel loaded with the stick's command line (kexec_file) at 6.05 s",
                     "restart into MPFree on Akai's own kernel (kexec.target) at 6.05 s"):
            self.assertIn("mpfree-hook: " + line + "\n", kmsg)
        self.assertNotIn("STOP", kmsg)
        self.assertEqual(self.calls("kexec: "), [self.kexec_load_call(KCMD + " loadpin.enforce=0")])
        self.assertEqual(self.read("sys/kernel/kexec_loaded"), "1\n")
        self.assertEqual(self.calls("systemctl: --no-block"), [KEXEC_TARGET], "no soft-reboot as well")
        self.assertEqual(self.calls("modprobe: "), [], "modules matter only for the soft-reboot")
        self.assertFalse(os.path.exists(os.path.join(self.root, "run/mpfree-hook/soft-reboot")))

    def test_chain_stick_restarts_on_akais_kernel_with_kexec_left_on(self):
        """--chain: the hook starts Akai's kernel as for any kexec-file stick, with the chain's command line."""
        self.install(ok=False)
        self.plug_in()
        self.make_stick(kexec_file=CHAIN_KCMD, chain=True)
        open(self.log, "w").close()
        res = self.jump()
        self.assertEqual(res.returncode, 0, res.stdout)
        kmsg = self.read("run/mpfree-hook.txt")
        for line in ("this stick asks to start on Akai's own kernel (kexec_file), command line: root=PARTUUID=(this "
                     "stick) rootwait=60 ro panic=10 rfkill.default_state=0 loadpin.enforce=0 mpfree.chain=1",
                     "on that kernel its system restarts once more, into the stick's own kernel and device tree "
                     "(mpfree.chain=1; its files check out against SHA256SUMS)",
                     "restart into MPFree on Akai's own kernel (kexec.target) at 6.05 s"):
            self.assertIn("mpfree-hook: " + line + "\n", kmsg)
        self.assertNotIn("STOP", kmsg)
        self.assertEqual(self.calls("kexec: "), [self.kexec_load_call(CHAIN_KCMD)])
        self.assertEqual(self.calls("systemctl: --no-block"), [KEXEC_TARGET])

    def test_chain_stick_that_does_not_check_out_stops(self):
        self.install(ok=False)
        self.plug_in()
        base = CHAIN_KCMD
        for cmdline, message in (
                (base + " sysctl.kernel.kexec_load_disabled=1",
                 "/boot/mpfree-softreboot/cmdline has both mpfree.chain=1 and sysctl.kernel.kexec_load_disabled=1"),
                (base.replace(" loadpin.enforce=0", ""),
                 "/boot/mpfree-softreboot/cmdline has mpfree.chain=1 without loadpin.enforce=0"),
                (base.replace(" mpfree.chain=1", ""), "/boot/mpfree-softreboot/cmdline lacks "
                                                      "sysctl.kernel.kexec_load_disabled=1"),
                (base + " mpfree.chain=2", "/boot/mpfree-softreboot/cmdline has mpfree.chain=2, which the hook does "
                                           "not allow")):
            with self.subTest(cmdline):
                self.make_stick(kexec_file=cmdline, chain=True)
                self.expect_stop(message)
                self.assertEqual(self.calls("kexec: "), [])
        with self.subTest("the chain's command line without its service"):
            self.make_stick(kexec_file=base)
            self.expect_stop("the stick asks for the chain (mpfree.chain=1) but does not carry /etc/mpfree-test/chain.sh")
        with self.subTest("the chain's service not started"):
            self.make_stick(kexec_file=base, chain=True)
            os.unlink(os.path.join(self.stick, STICK.CHAIN_WANTS.lstrip("/")))
            self.expect_stop("the stick asks for the chain (mpfree.chain=1) but does not start "
                             "mpfree-test-chain.service")
        with self.subTest("the stick's own device tree not listed"):
            self.make_stick(kexec_file=base, chain=True)
            self.write_sums(**{STICK.KEXEC_DIR + "/kexec.dtb": "-"})
            self.expect_stop("the stick asks for the chain (mpfree.chain=1) but does not carry "
                             "/boot/mpfree-kexec/kexec.dtb")
        with self.subTest("the stick's own device tree damaged"):
            self.make_stick(kexec_file=base, chain=True)
            write(os.path.join(self.stick, "boot/mpfree-kexec/kexec.dtb"), "DAMAGED\n")
            self.expect_stop("/boot/mpfree-kexec/kexec.dtb on the stick does not match its SHA256SUMS")

    def test_kexec_file_falls_back_to_the_soft_reboot(self):
        self.install(ok=False)
        self.plug_in()

        def fallback(note, loads, unloads=0, **env):
            open(self.log, "w").close()
            write(os.path.join(self.root, "run/mpfree-hook.txt"), "")
            shutil.rmtree(os.path.join(self.root, "run/mpfree-hook"), ignore_errors=True)
            shutil.rmtree(os.path.join(self.root, "run/nextroot"), ignore_errors=True)
            res = self.jump(**env)
            self.assertEqual(res.returncode, 0, res.stdout)
            kmsg = self.read("run/mpfree-hook.txt")
            self.assertIn("mpfree-hook: kexec_file: " + note, kmsg)
            self.assertNotIn("STOP", kmsg)
            self.assertEqual(len([c for c in self.calls("kexec: ") if c.startswith("kexec: --load")]), loads)
            self.assertEqual(self.calls("kexec: --unload"), ["kexec: --unload --kexec-file-syscall"] * unloads)
            self.assertEqual(self.read("sys/kernel/kexec_loaded"), "0\n", "nothing left loaded")
            self.assertEqual(self.calls("systemctl: --no-block")[-1:], [SOFT_REBOOT])
            self.assertIn("mpfree-hook: soft-reboot into MPFree (soft-reboot.target) at 6.05 s\n", kmsg)
            return kmsg

        self.make_stick(kexec_file=KCMD)
        with self.subTest("the kernel refuses the load"):
            fallback("the kernel refused the load (kexec_file_load failed: Operation not permitted); soft-reboot "
                     "instead", 1, FAKE_KEXEC_REFUSE="1")
        with self.subTest("systemd will not start kexec.target"):
            fallback("systemctl could not start kexec.target; soft-reboot instead", 1, 1, FAKE_KEXEC_START_RC="1")
        with self.subTest("the load reports a failure but leaves the kernel loaded"):
            fallback("the kernel refused the load (kexec: late error); soft-reboot instead", 1, 1,
                     FAKE_KEXEC_FAIL_LOADED="1")
        with self.subTest("a kexec kernel is loaded already"):
            write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "1\n")
            open(self.log, "w").close()
            write(os.path.join(self.root, "run/mpfree-hook.txt"), "")
            shutil.rmtree(os.path.join(self.root, "run/nextroot"), ignore_errors=True)
            res = self.jump()
            self.assertEqual(res.returncode, 0, res.stdout)
            self.assertIn("kexec_file: a kexec kernel is loaded already (not by this hook); soft-reboot instead",
                          self.read("run/mpfree-hook.txt"))
            self.assertEqual(self.calls("kexec: "), [], "someone else's kernel is left alone")
            self.assertEqual(self.calls("systemctl: --no-block"), [SOFT_REBOOT])
            write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "0\n")
        with self.subTest("kexec turned off on this system"):
            write(os.path.join(self.root, "proc/sys/kernel/kexec_load_disabled"), "1\n")
            fallback("kexec is turned off on this system; soft-reboot instead", 0)
            write(os.path.join(self.root, "proc/sys/kernel/kexec_load_disabled"), "0\n")
        with self.subTest("this system's kernel is another (an Akai update with the same release)"):
            self.make_stick(kexec_file=KCMD, kernel=b"another kernel")
            fallback(f"this system's /boot/zImage-{RELEASE} is not the kernel the stick was built with; soft-reboot "
                     "instead", 0)
            self.make_stick(kexec_file=KCMD)
        with self.subTest("this system has no such kernel file"):
            os.rename(os.path.join(self.root, "boot/zImage-" + RELEASE), os.path.join(self.root, "boot/zImage-away"))
            fallback(f"this system has no /boot/zImage-{RELEASE}; soft-reboot instead", 0)
            os.rename(os.path.join(self.root, "boot/zImage-away"), os.path.join(self.root, "boot/zImage-" + RELEASE))

    def test_kexec_file_stops_when_an_updater_starts_after_the_load(self):
        self.install(ok=False)
        self.plug_in()
        self.make_stick(kexec_file=KCMD)
        self.expect_stop("Akai's controller update.sh started; no restart while firmware may be written",
                         FAKE_KEXEC_UPDATE="1")
        self.assertEqual(self.calls("kexec: "), [self.kexec_load_call(), "kexec: --unload --kexec-file-syscall"])
        self.assertEqual(self.read("sys/kernel/kexec_loaded"), "0\n")

    def test_kexec_file_stops_when_its_kernel_stays_loaded(self):
        """A kernel this hook loaded and could not unload again stops everything: no soft-reboot with it in RAM."""
        self.install(ok=False)
        self.plug_in()
        self.make_stick(kexec_file=KCMD)
        stuck = "the kernel this hook loaded for kexec_file could not be unloaded; no restart while it is in RAM"
        update = "Akai's controller update.sh started; no restart while firmware may be written"
        for name, stop, env in (("the load fails late", stuck, {"FAKE_KEXEC_FAIL_LOADED": "1"}),
                                ("systemd will not start kexec.target", stuck, {"FAKE_KEXEC_START_RC": "1"}),
                                ("an updater starts after the load", update, {"FAKE_KEXEC_UPDATE": "1"})):
            with self.subTest(name):
                shutil.rmtree(os.path.join(self.root, "proc/999"), ignore_errors=True)
                write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "0\n")
                open(self.log, "w").close()
                write(os.path.join(self.root, "run/mpfree-hook.txt"), "")
                res = self.jump(FAKE_UNLOAD_FAIL="1", **env)
                self.assertEqual(res.returncode, 1, res.stdout)
                kmsg = self.read("run/mpfree-hook.txt")
                self.assertIn("mpfree-hook: could not unload the kernel this hook loaded (it stays in RAM until the "
                              "MPC is switched off)\n", kmsg)
                self.assertIn("mpfree-hook: STOP: " + stop + "\n", kmsg)
                self.assertIn("mpfree-hook: nothing was started: Akai's system starts as usual\n", kmsg)
                self.assertEqual(self.calls("kexec: --unload"), ["kexec: --unload --kexec-file-syscall"],
                                 "tried once, not again in the cleanup")
                self.assertNotIn(SOFT_REBOOT, self.calls("systemctl: --no-block"))
                self.assertEqual(self.run_files(), ["run/mpfree-hook", "run/mpfree-hook.txt"])
        shutil.rmtree(os.path.join(self.root, "proc/999"), ignore_errors=True)
        write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "0\n")

    def test_kexec_file_stick_that_does_not_check_out_stops(self):
        self.install(ok=False)
        self.plug_in()
        sr = os.path.join(self.stick, "boot/mpfree-softreboot")
        for cmdline, message in (
                (KCMD + " init=/bin/sh", "/boot/mpfree-softreboot/cmdline has init=/bin/sh, which the hook does not "
                                         "allow"),
                (KCMD.replace(STICK_UUID, "11111111-2222-3333-4444-555555555555"),
                 "/boot/mpfree-softreboot/cmdline does not start the system on this stick's partition 1"),
                (KCMD.replace(" sysctl.kernel.kexec_load_disabled=1", ""),
                 "/boot/mpfree-softreboot/cmdline lacks sysctl.kernel.kexec_load_disabled=1"),
                (KCMD.replace(" ro ", " rw "), "/boot/mpfree-softreboot/cmdline has rw, which the hook does not allow"),
                (KCMD + " ro", "/boot/mpfree-softreboot/cmdline has ro twice"),
                (KCMD + " a;b", "/boot/mpfree-softreboot/cmdline has unusual characters")):
            with self.subTest(cmdline):
                self.make_stick(kexec_file=cmdline)
                self.expect_stop(message)
                self.assertEqual(self.calls("kexec: "), [])
        with self.subTest("jump not listed in SHA256SUMS"):
            self.make_stick(kexec_file=KCMD)
            self.write_sums(**{"/boot/mpfree-softreboot/jump": "-"})
            self.expect_stop("/boot/mpfree-softreboot/jump is not listed in /boot/mpfree-softreboot/SHA256SUMS")
        with self.subTest("another jump"):
            self.make_stick(kexec_file=KCMD)
            write(os.path.join(sr, "jump"), "kexec-old\n")
            self.write_sums()
            self.expect_stop("/boot/mpfree-softreboot/jump is not kexec-file")
        with self.subTest("kernel.sha256 names another file"):
            self.make_stick(kexec_file=KCMD)
            write(os.path.join(sr, "kernel.sha256"), f"{sha(KERNEL)}  /data/zImage\n")
            self.write_sums()
            self.expect_stop(f"/boot/mpfree-softreboot/kernel.sha256 does not name /boot/zImage-{RELEASE}")
        with self.subTest("no partition UUID link"):
            self.make_stick(kexec_file=KCMD)
            os.unlink(os.path.join(self.root, "dev/disk/by-partuuid", STICK_UUID))
            self.expect_stop("/dev/sda1 has no plain partition UUID link")
            self.assertEqual(self.calls("kexec: "), [])

    def test_check_of_a_kexec_file_stick_loads_and_unloads(self):
        self.install(ok=False)
        self.plug_in()
        self.make_stick(kexec_file=KCMD)
        open(self.log, "w").close()
        res = self.hook("check")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Akai's kernel loaded with the stick's command line (kexec_file)", res.stdout)
        self.assertIn("Akai's kernel unloaded again (kexec --unload)", res.stdout)
        self.assertIn("CHECK-OK: at power-on with this stick in, the MPC would restart into it on Akai's own kernel "
                      "(kexec_file; the test load and unload passed), and soft-reboot into it if that load fails then",
                      res.stdout)
        self.assertEqual(self.calls("kexec: "), [self.kexec_load_call(), "kexec: --unload --kexec-file-syscall"])
        self.assertEqual(self.read("sys/kernel/kexec_loaded"), "0\n")
        self.assertEqual(self.calls("systemctl: --no-block") + self.calls("systemd-run: "), [])
        self.assertEqual(self.run_files(), [])
        open(self.log, "w").close()
        res = self.hook("check", FAKE_KEXEC_REFUSE="1")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("kexec_file: the kernel refused the load", res.stdout)
        self.assertIn("CHECK-OK: at power-on with this stick in, the MPC would soft-reboot into it (the kexec_file "
                      "route it asks for is not available, above)", res.stdout)
        self.assertEqual(self.run_files(), [])
        res = self.hook("check", FAKE_UNLOAD_FAIL="1")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: the kernel this hook loaded for kexec_file could not be unloaded; no restart while it "
                      "is in RAM", res.stdout)
        self.assertNotIn("CHECK-OK", res.stdout)
        write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "0\n")
        # A soft-reboot stick: the check never touches kexec.
        self.make_stick()
        open(self.log, "w").close()
        res = self.hook("check")
        self.assertIn("CHECK-OK: at power-on with this stick in, the MPC would soft-reboot into it\n", res.stdout)
        self.assertEqual(self.calls("kexec: "), [])


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
        for forbidden in ("mount ", "kexec", "systemctl", "systemd-run", "fw_setenv", "dd ", "rm -r",
                          "/etc/systemd", "> \"$R/usr", "> $R/usr"):
            self.assertNotIn(forbidden, installer)
        # The hook itself: the stick read-only (no journal replay) in both its mounts, no kernel loaded, the
        # soft-reboot asked for one way only, nothing written but /run and the log.
        self.assertIn('mount -t ext4 -o ro,noload "$dev" "$M"', hook)
        self.assertIn('mount -t ext4 -o ro,noload,nodev,nosuid,noexec "$dev" "$M"', hook)
        self.assertEqual(hook.count("mount -t"), 2)
        # Two ways to restart: the soft-reboot, and (kexec-file sticks only) kexec.target.
        self.assertEqual(hook.count("systemctl --no-block"), 2)
        self.assertIn("systemctl --no-block --job-mode=replace-irreversibly start soft-reboot.target", hook)
        self.assertIn("systemctl --no-block --job-mode=replace-irreversibly start kexec.target", hook)
        # kexec only with the file syscall, only Akai's own kernel, never a device tree, never executed here.
        kexec_calls = [line.strip() for line in hook.splitlines() if '"$R/usr/sbin/kexec" --' in line]
        self.assertEqual(len(kexec_calls), 2, kexec_calls)
        self.assertIn('"$R/usr/sbin/kexec" --load --kexec-file-syscall --command-line="$kcmd" "$K"', kexec_calls[1])
        self.assertIn('"$R/usr/sbin/kexec" --unload --kexec-file-syscall', kexec_calls[0])
        self.assertIn('K="$R/boot/zImage-$rel"', hook)
        for forbidden in ("fw_setenv", "dd ", "mkfs", "mmcblk", "rw,", "rm -r", " > /data",
                          ">> /data", "--kexec-syscall", "--dtb", "kexec -e", "--exec", "systemctl reboot",
                          "systemctl soft-reboot", "systemctl kexec", "insmod", "--force"):
            self.assertNotIn(forbidden, hook)
        writes = [line.strip() for line in hook.splitlines() if " > " in line or ">>" in line]
        for line in writes:
            self.assertRegex(line, r'(> /dev/null|> "\$R/dev/kmsg"|>> "\$R/run/mpfree-hook.txt"|'
                                   r'> "\$R/run/mpfree-hook/soft-reboot")', line)


if __name__ == "__main__":
    unittest.main(verbosity=2)
