#!/usr/bin/env python3
"""Tests for the boot-hook canary (collect-over-ssh.sh --hook-canary, --hook-canary-remove,
--hook-canary-report) and the wrapper's developer mode port 22 handling. No device, no network.

ip, ping and ssh are stand-ins (the network identity is the same test-only one as in
test_kexec_preflight.py). The fake ssh keeps a known_hosts file the way OpenSSH does for the options
it is given (accept-new saves an unknown key, yes refuses one), and runs the streamed scripts under
BusyBox applets (as on the MPC) against a fake root folder: their first line R='' becomes that folder.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import glob
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_kexec_preflight import FAKE_IP, TEST_HOST, TEST_IFACE, TEST_IP, TEST_MAC  # noqa: E402

TOOLS = os.path.dirname(HERE)
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
INSTALL = os.path.join(TOOLS, "hook-canary-install.sh")
REMOVE = os.path.join(TOOLS, "hook-canary-remove.sh")
REPORT = os.path.join(TOOLS, "read-only-hook-canary.sh")
RELEASE = "6.18.44-az01-2026-08-10-rt6"
APPLETS = ("sh", "cat", "grep", "sed", "tr", "wc", "ls", "uname", "head", "cut", "mkdir", "chmod", "mv",
           "rm", "rmdir", "sync", "sha256sum", "df", "zcat", "basename")
RULE = "data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules"
CONF = "data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf"
SCRIPT = "data/mpfree-hook/canary.sh"
MARK = "# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh."
MOUNTS = ("/dev/mmcblk0p8 /data ext4 rw,relatime 0 0\n"
          "overlay /etc overlay rw,relatime,lowerdir=/etc,upperdir=/data/system/etc/overlay,"
          "workdir=/data/system/etc/.work,uuid=null 0 0\n"
          "overlay /etc overlay ro,relatime,lowerdir=/usr/lib/az0x/user-access/etc-overlay:/etc,redirect_dir=on 0 0\n")

FAKE_SSH = r'''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
opts, i = {}, 0
while i < len(args) and args[i].startswith("-"):
    if args[i] == "-o":
        key, _, value = args[i + 1].partition("=")
        opts[key] = value
        i += 2
    elif args[i] in ("-i", "-p", "-l", "-F"):
        opts[args[i]] = args[i + 1]
        i += 2
    else:
        i += 1
target, command = args[i], args[-1]
host = target.split("@")[-1]
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write(f"ssh: port={opts.get('-p')} F={opts.get('-F')} strict={opts.get('StrictHostKeyChecking')} "
             f"known={opts.get('UserKnownHostsFile')} {command}\n")
known = opts.get("UserKnownHostsFile")
if known:
    key = os.environ["FAKE_HOSTKEY"]
    lines = open(known).read().splitlines() if os.path.exists(known) else []
    saved = [l.split(None, 1)[1] for l in lines if l.split(None, 1)[0] == host]
    if saved and saved[0] != key:
        print("@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@", file=sys.stderr)
        print("Host key verification failed.", file=sys.stderr)
        sys.exit(255)
    if not saved:
        if opts.get("StrictHostKeyChecking") != "accept-new":
            print("Host key verification failed.", file=sys.stderr)
            sys.exit(255)
        with open(known, "a") as fh:
            fh.write(f"{host} {key}\n")
        print(f"Warning: Permanently added '{host}' (ED25519) to the list of known hosts.", file=sys.stderr)
if command == "sh -s":
    script = sys.stdin.read()
    first, _, rest = script.partition("\n")
    if first == "R=''":
        script = "R='" + os.environ["FAKE_ROOT"] + "'\n" + rest
    env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
    sys.exit(subprocess.run([os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh"), "-s"], input=script.encode(),
                            env=env).returncode)
if command == "uname -r":
    print(os.environ["FAKE_RELEASE"])
    sys.exit(0)
if command == "cat /sys/firmware/fdt":
    sys.stdout.write("not a device tree")
    sys.exit(0)
print("fake ssh: command not allowed: " + command, file=sys.stderr)
sys.exit(255)
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  is-active) echo "${FAKE_ACVS:-inactive}"; [ "${FAKE_ACVS:-inactive}" = active ] ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''
FAKE_SYSTEMD_RUN = '#!/bin/sh\necho "systemd-run: $*" >> "$FAKE_LOG"\n'
# kmod's modprobe as far as the install checks it (--version), and as the modprobe line calls it.
FAKE_KMOD = '#!/bin/sh\nif [ "$1" = --version ]; then echo "kmod version 31"; echo "+ZSTD +XZ"; exit 0; fi\n' \
            'echo "modprobe: $*" >> "$FAKE_LOG"\n'
FAKE_BUSYBOX_MODPROBE = '#!/bin/sh\necho "BusyBox v1.36.1 multi-call binary." >&2\nexit 1\n'
FAKE_PIDOF = '#!/bin/sh\necho "pidof: $*" >> "$FAKE_LOG"\nexit 1\n'
FAKE_JOURNALCTL = r'''#!/bin/sh
echo "journalctl: $*" >> "$FAKE_LOG"
printf '%s\n' "[    2.900000] mpc-one systemd[1]: Mounted Writable /etc via OverlayFS." \
  "[    3.400000] mpc-one kernel: sd 0:0:0:0: [sda] Attached SCSI removable disk" \
  "[    6.300000] mpc-one systemd[1]: Started InMusic MPC Application."
'''


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


class HookCanaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")
        if not shutil.which("ssh-keygen"):
            raise unittest.SkipTest("ssh-keygen not installed")
        cls.keys = tempfile.mkdtemp(prefix="mpfree-hook-canary-keys-")
        for name in ("host", "other"):
            subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "", "-f",
                            os.path.join(cls.keys, name)], check=True)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.keys, ignore_errors=True)

    @classmethod
    def pub(cls, name):
        with open(os.path.join(cls.keys, name + ".pub")) as fh:
            return " ".join(fh.read().split()[:2])

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-hook-canary-test-")
        self.home = os.path.join(self.tmp, "home")
        self.reports = os.path.join(self.tmp, "research")
        self.log = os.path.join(self.tmp, "commands.log")
        self.root = os.path.join(self.tmp, "root")
        fakebin, remote = os.path.join(self.tmp, "fakebin"), os.path.join(self.tmp, "remote-bin")
        for d in (self.home, fakebin, remote):
            os.makedirs(d)
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        write(os.path.join(fakebin, "ip"), FAKE_IP, 0o755)
        write(os.path.join(fakebin, "ping"), "#!/bin/sh\nexit 0\n", 0o755)
        for applet in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, applet))
        for name, text in (("systemctl", FAKE_SYSTEMCTL), ("systemd-run", FAKE_SYSTEMD_RUN), ("pidof", FAKE_PIDOF),
                           ("journalctl", FAKE_JOURNALCTL), ("modprobe", FAKE_KMOD)):
            write(os.path.join(remote, name), text, 0o755)
        self.remote = remote
        write(os.path.join(self.root, "proc/sys/kernel/osrelease"), RELEASE + "\n")
        write(os.path.join(self.root, "proc/mounts"), MOUNTS)
        write(os.path.join(self.root, "proc/uptime"), "4.25 9.10\n")
        write(os.path.join(self.root, "proc/sys/kernel/modprobe"), "/sbin/modprobe\n")
        write(os.path.join(self.root, "proc/modules"), "cfg80211 1000000 0 - Live 0x00000000\n")
        write(os.path.join(self.root, "sbin/modprobe"), FAKE_KMOD, 0o755)
        write(os.path.join(self.root, "data/system/etc/overlay/hostname"), "mpc-one\n")
        write(os.path.join(self.root, "data/system/etc/overlay/systemd/system/old.service"), "[Unit]\n")
        write(os.path.join(self.root, "dev/kmsg"), "")
        os.makedirs(os.path.join(self.root, "run"))
        self.before = self.tree()
        self.key = os.path.join(self.tmp, "key")
        write(self.key, "not a real key\n", 0o600)
        self.known = os.path.join(self.home, "mpfree-work", "mpc-devmode-known_hosts")
        self.env = {
            "HOME": self.home, "LC_ALL": "C", "PATH": fakebin + os.pathsep + "/usr/bin:/bin",
            "MPC_KEY": self.key, "MPC_REPORT_DIR": self.reports, "MPC_PORT": "22",
            "MPC_IP": TEST_IP, "MPC_IFACE": TEST_IFACE, "MPC_HOST_IP_CIDR": TEST_HOST, "MPC_EXPECT_MAC": TEST_MAC,
            "FAKE_LOG": self.log, "FAKE_REMOTE_PATH": remote, "FAKE_ROOT": self.root,
            "FAKE_HOSTKEY": self.pub("host"), "FAKE_RELEASE": RELEASE,
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

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

    def calls(self, prefix="ssh: "):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [line for line in fh.read().splitlines() if line.startswith(prefix)]

    def read(self, rel):
        with open(os.path.join(self.root, rel)) as fh:
            return fh.read()

    def install(self):
        res = self.run_wrapper("--hook-canary", MPFREE_ACCEPT_HOOK_CANARY="YES")
        self.assertEqual(res.returncode, 0, res.stdout)
        return res

    def test_install_needs_the_opt_in_and_port_22(self):
        res = self.run_wrapper("--hook-canary")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("MPFREE_ACCEPT_HOOK_CANARY=YES", res.stdout)
        res = self.run_wrapper("--hook-canary", MPC_PORT="2222", MPFREE_ACCEPT_HOOK_CANARY="YES")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("run it with MPC_PORT=22", res.stdout)
        self.assertEqual(self.calls(), [], "nothing may reach the MPC")
        self.assertEqual(self.tree(), self.before)

    def test_install_writes_three_files_and_pins_the_host_key(self):
        res = self.install()
        self.assertIn("first contact: its host key will be saved in " + self.known, res.stdout)
        self.assertIn("RESULT: the canary is installed", res.stdout)
        self.assertIn("Developer mode SSH host key: ", res.stdout)
        [call] = self.calls()
        self.assertEqual(call, f"ssh: port=22 F=/dev/null strict=accept-new known={self.known} sh -s")
        with open(self.known) as fh:
            self.assertEqual(fh.read(), f"{TEST_IP} {self.pub('host')}\n")
        self.assertIn("modprobe: kmod's (/sbin/modprobe)", res.stdout)
        rule = self.read(RULE)
        self.assertIn('RUN+="/bin/sh /data/mpfree-hook/canary.sh udev $kernel"', rule)
        self.assertIn('\nRUN+="/bin/sh /data/mpfree-hook/canary.sh event $kernel"\n', rule)
        conf = self.read(CONF)
        self.assertEqual(conf.splitlines()[0], MARK)
        self.assertEqual([line for line in conf.splitlines() if not line.startswith("#")],
                         ["install cfg80211 export PATH=/usr/sbin:/usr/bin:/sbin:/bin; /bin/sh "
                          "/data/mpfree-hook/canary.sh modprobe cfg80211; /sbin/modprobe --ignore-install cfg80211 "
                          "$CMDLINE_OPTS"])
        self.assertTrue(self.read(SCRIPT).startswith("R=''\n" + MARK + "\n"))
        self.assertEqual(self.read("data/mpfree-hook/created-folders.txt").split(),
                         ["/data/mpfree-hook", "/data/system/etc/overlay/udev",
                          "/data/system/etc/overlay/udev/rules.d", "/data/system/etc/overlay/modprobe.d"])
        after = self.tree()
        added = sorted(set(after) - set(self.before))
        self.assertEqual(added, ["data/mpfree-hook/", "data/mpfree-hook/canary.sh",
                                 "data/mpfree-hook/created-folders.txt", "data/system/etc/overlay/modprobe.d/", CONF,
                                 "data/system/etc/overlay/udev/", "data/system/etc/overlay/udev/rules.d/", RULE])
        for path, text in self.before.items():
            self.assertEqual(after[path], text, f"{path} changed")
        if shutil.which("udevadm"):
            check = subprocess.run(["udevadm", "verify", os.path.join(self.root, RULE)],
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            self.assertEqual(check.returncode, 0, check.stdout)

    def test_second_install_changes_nothing_and_checks_the_key(self):
        self.install()
        installed = self.tree()
        res = self.install()
        self.assertIn("must match the one saved in", res.stdout)
        self.assertIn("already in place, unchanged", res.stdout)
        self.assertEqual(self.calls()[-1], f"ssh: port=22 F=/dev/null strict=yes known={self.known} sh -s")
        self.assertEqual(self.tree(), installed)

    def test_another_host_key_is_refused_before_anything_runs(self):
        os.makedirs(os.path.dirname(self.known))
        write(self.known, f"{TEST_IP} {self.pub('other')}\n")
        res = self.run_wrapper("--hook-canary", MPFREE_ACCEPT_HOOK_CANARY="YES")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("another host key than the one saved in", res.stdout)
        self.assertIn(f"ssh-keygen -R {TEST_IP} -f {self.known}", res.stdout)
        self.assertEqual(self.tree(), self.before)

    def test_install_stops_on_another_release_or_layout(self):
        write(os.path.join(self.root, "proc/sys/kernel/osrelease"), "6.6.95-az01-2025-06-27-rt55\n")
        res = self.run_wrapper("--hook-canary", MPFREE_ACCEPT_HOOK_CANARY="YES")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: this is for Akai's 3.10.1", res.stdout)
        write(os.path.join(self.root, "proc/sys/kernel/osrelease"), RELEASE + "\n")
        write(os.path.join(self.root, "proc/mounts"), "/dev/mmcblk0p8 /data ext4 rw,relatime 0 0\n")
        res = self.run_wrapper("--hook-canary", MPFREE_ACCEPT_HOOK_CANARY="YES")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: /etc is not Akai's overlay", res.stdout)
        self.assertEqual(self.tree(), self.before)

    def test_install_stops_on_a_file_it_did_not_write(self):
        for path in (RULE, CONF):
            write(os.path.join(self.root, path), "# someone else's\n")
            self.before = self.tree()
            res = self.run_wrapper("--hook-canary", MPFREE_ACCEPT_HOOK_CANARY="YES")
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(f"/{path} exists and is not this canary's file", res.stdout)
            self.assertEqual(self.tree(), self.before)
            os.unlink(os.path.join(self.root, path))

    def test_install_needs_kmods_modprobe(self):
        # BusyBox's modprobe ignores "install" lines: nothing is written.
        write(os.path.join(self.root, "sbin/modprobe"), FAKE_BUSYBOX_MODPROBE, 0o755)
        res = self.run_wrapper("--hook-canary", MPFREE_ACCEPT_HOOK_CANARY="YES")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: the kernel's module loader /sbin/modprobe is not kmod's modprobe", res.stdout)
        for odd in ("/sbin/mod probe", "/sbin/x;y", "modprobe", ""):
            write(os.path.join(self.root, "proc/sys/kernel/modprobe"), odd + "\n")
            res = self.run_wrapper("--hook-canary", MPFREE_ACCEPT_HOOK_CANARY="YES")
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("STOP: the kernel's module loader ", res.stdout)
        self.assertEqual(self.tree(), self.before)

    def write_first_version(self):
        """The canary as it was first installed on 2026-11-15: two files, three folders."""
        write(os.path.join(self.root, SCRIPT), f"R=''\n{MARK}\nexit 0\n", 0o755)
        write(os.path.join(self.root, RULE), f"{MARK}\nACTION==\"add\", RUN+=\"/bin/true\"\n")
        write(os.path.join(self.root, "data/mpfree-hook/created-folders.txt"),
              "/data/mpfree-hook\n/data/system/etc/overlay/udev\n/data/system/etc/overlay/udev/rules.d\n")

    def test_install_replaces_the_first_version(self):
        self.write_first_version()
        res = self.install()
        self.assertIn("an earlier version of the canary is in place; it is replaced", res.stdout)
        self.assertIn("RESULT: the canary is installed", res.stdout)
        self.assertIn("canary.sh modprobe cfg80211", self.read(CONF))
        self.assertIn("first-event", self.read(SCRIPT))
        self.assertEqual(self.read("data/mpfree-hook/created-folders.txt").split(),
                         ["/data/mpfree-hook", "/data/system/etc/overlay/udev",
                          "/data/system/etc/overlay/udev/rules.d", "/data/system/etc/overlay/modprobe.d"])
        res = self.run_wrapper("--hook-canary-remove")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.tree(), self.before)

    def test_remove_also_takes_the_first_version(self):
        self.write_first_version()
        res = self.run_wrapper("--hook-canary-remove")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("RESULT: the canary is gone", res.stdout)
        self.assertEqual(self.tree(), self.before)

    def canary(self, *args, **env):
        script = self.read(SCRIPT).replace("R=''", f"R='{self.root}'", 1)
        return subprocess.run([os.path.join(self.remote, "sh"), "-c", script, "canary.sh", *args],
                              env={"FAKE_LOG": self.log, "FAKE_ROOT": self.root, **env},
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)

    def test_the_canary_only_logs(self):
        self.install()
        installed = self.tree()
        open(self.log, "w").close()
        # udev starts it with no PATH; the script sets its own (here the stand-ins come first).
        script = self.read(SCRIPT)
        self.assertIn("PATH=/usr/sbin:/usr/bin:/sbin:/bin", script)
        write(os.path.join(self.root, SCRIPT),
              script.replace("PATH=/usr/sbin:/usr/bin:/sbin:/bin", f"PATH={self.remote}"), 0o755)
        res = self.canary("udev", "sda1", ID_PART_ENTRY_NAME="mpfree-test-root")
        self.assertEqual(res.returncode, 0, res.stdout)
        line = ("mpfree-canary: udev saw sda1 at 4.25 s; acvs.service: not started; MPC program: not running; "
                "partition name: mpfree-test-root")
        self.assertEqual(self.read("dev/kmsg"), line + "\n")
        self.assertEqual(self.read("run/mpfree-canary.txt"), line + "\n")
        self.assertEqual(self.calls("systemd-run: "), [
            "systemd-run: --no-block --collect --quiet --unit=mpfree-canary-sda1 --description=MPFree canary (sda1) "
            "/bin/sh /data/mpfree-hook/canary.sh service sda1"])
        self.assertEqual(self.calls("systemctl: "), [], "no call to systemd but the service start")
        # Akai's app started: seen from its cgroup, still without asking systemd.
        os.makedirs(os.path.join(self.root, "sys/fs/cgroup/system.slice/acvs.service"))
        write(os.path.join(self.root, "dev/disk/by-partlabel/mpfree-test-root"), "")
        self.canary("service", "sda1", FAKE_ACVS="active")
        self.assertIn("mpfree-canary: service (sda1) ran at 4.25 s; acvs.service: started; MPC program: "
                      "not running; systemd: acvs.service active; partition labels: mpfree-test-root\n",
                      self.read("dev/kmsg"))
        self.assertEqual(len(self.calls("systemd-run: ")), 1, "the service run starts nothing more")
        # Every device event is logged; only the first of the boot starts the service.
        self.canary("event", "card0;rm -rf /", ACTION="add", SUBSYSTEM="sound")
        self.canary("event", "regulatory", ACTION="add", SUBSYSTEM="faux")
        run = self.read("run/mpfree-canary.txt")
        self.assertIn("mpfree-canary: event add sound card0rm-rf at 4.25 s\n", run)
        self.assertIn("mpfree-canary: first event after udev loaded the rule: add sound card0rm-rf at 4.25 s; "
                      "acvs.service: started; MPC program: not running", self.read("dev/kmsg"))
        self.assertIn("mpfree-canary: event add faux regulatory at 4.25 s\n", run)
        self.assertEqual(self.read("dev/kmsg").count("first event"), 1)
        self.assertEqual(self.calls("systemd-run: ")[1:], [
            "systemd-run: --no-block --collect --quiet --unit=mpfree-canary-first-event "
            "--description=MPFree canary (first-event) /bin/sh /data/mpfree-hook/canary.sh service first-event"])
        for bad in ("../../etc", "sda", "mmcblk0p1", ""):
            self.canary("udev", bad)
            self.canary("service", bad)
        self.canary("modprobe", "mac80211")
        self.assertEqual(len(self.calls("systemd-run: ")), 2, "only sdXN names and cfg80211 are acted on")
        after = self.tree()
        self.assertEqual({k: v for k, v in after.items() if k != SCRIPT},
                         {k: v for k, v in installed.items() if k != SCRIPT}, "the canary writes nothing on /data")

    def test_the_modprobe_line_logs_starts_the_service_and_loads_the_module(self):
        self.install()
        installed = self.tree()
        open(self.log, "w").close()
        script = self.read(SCRIPT).replace("PATH=/usr/sbin:/usr/bin:/sbin:/bin", f"PATH={self.remote}")
        write(os.path.join(self.root, SCRIPT), script.replace("R=''", f"R='{self.root}'", 1), 0o755)
        # The command kmod runs for "install cfg80211": the rest of the line, $CMDLINE_OPTS filled in, by sh -c.
        [line] = [l for l in self.read(CONF).splitlines() if l.startswith("install cfg80211 ")]
        command = line[len("install cfg80211 "):].replace("$CMDLINE_OPTS", "")
        command = command.replace("PATH=/usr/sbin:/usr/bin:/sbin:/bin", f"PATH={self.remote}")
        command = command.replace("; /sbin/modprobe ", f"; {os.path.join(self.remote, 'modprobe')} ")
        command = command.replace("/data/mpfree-hook/canary.sh", os.path.join(self.root, SCRIPT))
        res = subprocess.run([os.path.join(self.remote, "sh"), "-c", command], env={"FAKE_LOG": self.log},
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.calls("modprobe: "), ["modprobe: --ignore-install cfg80211"])
        kmsg = self.read("dev/kmsg")
        self.assertRegex(kmsg, r"^mpfree-canary: modprobe loads cfg80211 at 4\.25 s; acvs\.service: not started; "
                               r"MPC program: not running; called by: sh < [^\n]* < [^\n]* < [^\n]*\n$")
        for _ in range(100):  # the service start runs in the background, so that modprobe never waits for it
            if self.calls("systemd-run: "):
                break
            time.sleep(0.05)
        self.assertEqual(self.calls("systemd-run: "), [
            "systemd-run: --no-block --collect --quiet --unit=mpfree-canary-modprobe "
            "--description=MPFree canary (modprobe) /bin/sh /data/mpfree-hook/canary.sh service modprobe"])
        self.assertEqual(self.calls("systemctl: "), [])
        # Without the script the module still loads.
        os.unlink(os.path.join(self.root, SCRIPT))
        open(self.log, "w").close()
        res = subprocess.run([os.path.join(self.remote, "sh"), "-c", command], env={"FAKE_LOG": self.log},
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.calls("modprobe: "), ["modprobe: --ignore-install cfg80211"])
        self.assertEqual({k: v for k, v in self.tree().items() if k != SCRIPT},
                         {k: v for k, v in installed.items() if k != SCRIPT})

    def test_remove_undoes_the_install_exactly(self):
        self.install()
        res = self.run_wrapper("--hook-canary-remove")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("RESULT: the canary is gone", res.stdout)
        self.assertEqual(self.tree(), self.before)
        res = self.run_wrapper("--hook-canary-remove")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.tree(), self.before)

    def test_remove_keeps_what_it_did_not_write(self):
        self.install()
        write(os.path.join(self.root, "data/system/etc/overlay/udev/rules.d/50-other.rules"), "# other\n")
        res = self.run_wrapper("--hook-canary-remove")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("kept the folder /data/system/etc/overlay/udev/rules.d (not empty)", res.stdout)
        self.assertFalse(os.path.exists(os.path.join(self.root, RULE)))
        self.assertTrue(os.path.exists(os.path.join(self.root, "data/system/etc/overlay/udev/rules.d/50-other.rules")))
        self.assertFalse(os.path.exists(os.path.join(self.root, "data/mpfree-hook")))
        write(os.path.join(self.root, RULE), "# not the canary\n")
        res = self.run_wrapper("--hook-canary-remove")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("is not the canary's; left in place", res.stdout)
        self.assertTrue(os.path.exists(os.path.join(self.root, RULE)))

    def test_report_reads_only(self):
        self.install()
        installed = self.tree()
        # (BusyBox's shell runs its own dmesg applet, so the kernel log part cannot be faked here.)
        write(os.path.join(self.root, "run/mpfree-canary.txt"), "mpfree-canary: udev saw sda1 at 4.25 s\n")
        res = self.run_wrapper("--hook-canary-report")
        self.assertEqual(res.returncode, 0, res.stdout)
        [report] = glob.glob(os.path.join(self.reports, "mpc-one-hook-canary-report-*.txt"))
        with open(report) as fh:
            text = fh.read()
        self.assertIn("MPFREE BOOT-HOOK CANARY REPORT - READ ONLY", text)
        self.assertIn("=== Canary lines in the kernel log", text)
        self.assertIn("=== /run/mpfree-canary.txt ===\nmpfree-canary: udev saw sda1 at 4.25 s", text)
        self.assertIn("Started InMusic MPC Application", text)
        self.assertIn("  /data/mpfree-hook/canary.sh", text)
        self.assertIn("  /data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf", text)
        self.assertIn("kernel's module loader: /sbin/modprobe\ncfg80211 loaded: yes\n", text)
        self.assertIn("END OF BOOT-HOOK CANARY REPORT", text)
        self.assertEqual(self.tree(), installed)

    def test_kexec_preflight_on_port_22_checks_the_key_on_every_connection(self):
        res = self.run_wrapper("--kexec-preflight")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.calls(), [
            f"ssh: port=22 F=/dev/null strict=accept-new known={self.known} sh -s",
            f"ssh: port=22 F=/dev/null strict=yes known={self.known} uname -r",
            f"ssh: port=22 F=/dev/null strict=yes known={self.known} cat /sys/firmware/fdt"])
        [copy] = glob.glob(os.path.join(self.home, "mpfree-work", "kexec-preflight-*", "release.txt"))
        with open(copy) as fh:
            self.assertEqual(fh.read(), RELEASE + "\n")

    def test_state_changing_modes_are_refused_on_port_22(self):
        res = self.run_wrapper("--kexec-boot", MPFREE_ACCEPT_KEXEC_BOOT="YES")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.calls(), [])


class StaticTests(unittest.TestCase):
    def test_scripts_parse(self):
        for path in (INSTALL, REMOVE, REPORT):
            subprocess.run(["sh", "-n", path], check=True)

    def test_report_only_reads(self):
        with open(REPORT) as fh:
            code = "\n".join(line for line in fh.read().splitlines() if not line.lstrip().startswith("#"))
        for forbidden in ("rm ", "mv ", "cp ", "mkdir", "chmod", "mount", "systemctl start", "systemctl stop",
                          "systemd-run", "kexec", "tee", " > ", ">>"):
            self.assertNotIn(forbidden, code)

    def test_install_writes_only_its_three_files(self):
        with open(INSTALL) as fh:
            code = "\n".join(line for line in fh.read().splitlines() if not line.lstrip().startswith("#"))
        for forbidden in ("mount ", "kexec ", "systemctl start", "systemctl stop", "fw_setenv", "dd ", "rm -",
                          "/usr/lib/systemd", "/etc/systemd"):
            self.assertNotIn(forbidden, code)


if __name__ == "__main__":
    unittest.main(verbosity=2)
