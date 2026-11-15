#!/usr/bin/env python3
"""Tests for host_devmode_check.py: Akai's developer mode on the 3.10.1 test build.

The remote scripts run under BusyBox sh against a fake test-build root. A fake
ssh plays both servers: port 2222 runs the scripts (only with the test build's
host key alias), port 22 plays Akai's sshd with the settings it last loaded
(a stand-in `systemctl reload sshd.service` copies them), so the lock-down is
checked end to end: the login with no key must stop working and the key must
start working.
"""
import base64
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_devmode_check.py")
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
NEW = "6.18.44-fake-rt6"
P1 = "11111111-2222-4333-8444-555555555555"
CMDLINE = f"root=PARTUUID={P1} rootwait=60 ro panic=10 sysctl.kernel.kexec_load_disabled=1"
APPLETS = ("sh", "cat", "grep", "readlink", "ls", "sed", "tail", "head", "tr", "cut", "sort", "mkdir", "cp")
TEST_IP = "192.0.2.141"


def ed25519_line(seed, comment=""):
    blob = b"\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x20" + bytes([seed]) * 32
    return f"ssh-ed25519 {base64.b64encode(blob).decode()}" + (f" {comment}" if comment else "")


AKAI_HOST_KEY = ed25519_line(7)
OTHER_HOST_KEY = ed25519_line(8)
OUR_KEY = ed25519_line(9, "mpc-test")
ALLOW_ACCESS = ("# To lock down access to SSH keys, delete the content of this file and add\n"
                "# your key to /data/ssh/authorized_keys\n\nPermitRootLogin yes\nPermitEmptyPasswords yes\n\n"
                "AuthenticationMethods none\n\nPasswordAuthentication yes\n")

FAKE_SSH = r'''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
opts, i, port, ident = [], 0, "22", None
while i < len(args) and args[i].startswith("-"):
    if args[i] in ("-o", "-i", "-p", "-l", "-F"):
        if args[i] == "-o":
            opts.append(args[i + 1])
        elif args[i] == "-p":
            port = args[i + 1]
        elif args[i] == "-i":
            ident = args[i + 1]
        i += 2
    else:
        i += 1
command = args[-1]
o = dict(x.split("=", 1) for x in opts)
root = os.environ["FAKE_ROOT"]
stage = open(os.environ["FAKE_STAGE"]).read().strip()
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write(f"ssh[{port}@{stage}] key={'yes' if ident and o.get('PubkeyAuthentication') == 'yes' else 'no'}: "
             f"{command.splitlines()[0]}\n")
if port == "2222":
    if stage == "off":
        print("Connection reset by 10.42.0.141 port 2222", file=sys.stderr)
        sys.exit(255)
    if o.get("HostKeyAlias") != "mpfree-kexec-test" or stage != "310":
        print("Host key verification failed.", file=sys.stderr)
        sys.exit(255)
    assert command == "sh -s"
    script = sys.stdin.read()
    assert script.startswith("R=''\n"), "every remote script starts with R=''"
    script = f"R='{root}'\n" + script[len("R=''\n"):]
    env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
    sys.exit(subprocess.run([os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh"), "-c", script], env=env).returncode)
# Akai's SSH server on port 22, as it was last (re)loaded.
for want in ("StrictHostKeyChecking=yes", "HostKeyAlias=mpfree-devmode-test", "IdentityAgent=none",
             "PasswordAuthentication=no", "KbdInteractiveAuthentication=no", "BatchMode=yes"):
    assert want in opts, want
assert "-F" in args and args[args.index("-F") + 1] == "/dev/null"
loaded = os.path.join(root, "run/fake-sshd-loaded")
if stage != "310" or not os.path.isdir(loaded):
    print(f"ssh: connect to host {args[-2].split('@')[1]} port 22: Connection refused", file=sys.stderr)
    sys.exit(255)
known = open(o["UserKnownHostsFile"]).read().split()
server = open(os.environ["FAKE_SERVER_KEY"]).read().split()[:2]
if known[0] != "mpfree-devmode-test" or known[1:3] != server:
    print("Host key verification failed.", file=sys.stderr)
    sys.exit(255)
conf = open(os.path.join(loaded, "allow-access.conf")).read()
keys = open(os.path.join(loaded, "authorized_keys")).read()
ok = "AuthenticationMethods none" in conf
if not ok and ident and o.get("PubkeyAuthentication") == "yes":
    pub = open(ident + ".pub").read().split()[:2]
    ok = any(line.split()[:2] == pub for line in keys.splitlines())
if not ok:
    print(f"{args[-2]}: Permission denied (publickey).", file=sys.stderr)
    sys.exit(255)
assert command == "id -u"
print("0")
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  is-active) case " $FAKE_ACTIVE " in *" $2 "*) echo active ;; *) echo inactive ;; esac ;;
  reload)
    [ "$2" = sshd.service ] || { echo "systemctl stand-in: refusing $*" >&2; exit 99; }
    d="$FAKE_ROOT/run/fake-sshd-loaded"
    [ -d "$d" ] || { echo "sshd.service is not active, cannot reload." >&2; exit 1; }
    cp "$FAKE_ROOT/data/ssh/sshd_config.d/allow-access.conf" "$d/allow-access.conf"
    cp "$FAKE_ROOT/data/ssh/authorized_keys" "$d/authorized_keys" ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

FAKE_JOURNALCTL = r'''#!/bin/sh
echo "journalctl: $*" >> "$FAKE_LOG"
case " $* " in
  *" -k "*) [ "$FAKE_AUTORUN" = 1 ] && echo "[  80.1] kernel: mpfree-autorun: ran as 0 from /media/SPARE DRIVE/autorun.sh" ;;
  *" acvs.service "*) echo "[  40.2] MPC[812]: DevMode: Developer mode on"; echo "[  40.3] MPC[812]: unrelated line" ;;
  *" sshd.service "*) echo "[  41.0] sshd[900]: Server listening on 0.0.0.0 port 22." ;;
  *) echo "[   9.0] kernel: mpfree-test: blocked /usr/bin/dfu-util -l" ;;
esac
'''

FAKE_SSHD = r'''#!/bin/sh
[ "$1" = -T ] || { echo "sshd stand-in: refusing $*" >&2; exit 99; }
echo "port 22"
if grep -q 'AuthenticationMethods none' "$FAKE_ROOT/data/ssh/sshd_config.d/allow-access.conf" 2>/dev/null; then
  printf 'permitrootlogin yes\npermitemptypasswords yes\nauthenticationmethods none\npasswordauthentication yes\n'
else
  printf 'permitrootlogin without-password\npermitemptypasswords no\nauthenticationmethods any\npasswordauthentication no\n'
fi
echo "authorizedkeysfile /data/ssh/authorized_keys"
echo "ciphers chacha20-poly1305@openssh.com"
echo "hostkey /etc/ssh/ssh_host_ed25519_key"
'''


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


def read(path):
    with open(path) as fh:
        return fh.read()


class DevmodeCheckTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = t = tempfile.mkdtemp(prefix="mpfree-devmode-test-")
        self.work, self.log, self.stage = os.path.join(t, "work"), os.path.join(t, "log"), os.path.join(t, "stage")
        self.root, self.report = os.path.join(t, "root310"), os.path.join(t, "report.txt")
        self.server_key = os.path.join(t, "server_key.pub")
        write(self.stage, "310\n")
        write(self.log, "")
        write(self.server_key, AKAI_HOST_KEY + "\n")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, name))
        write(os.path.join(remote, "systemctl"), FAKE_SYSTEMCTL, 0o755)
        write(os.path.join(remote, "journalctl"), FAKE_JOURNALCTL, 0o755)
        self.key = os.path.join(t, "key")
        write(self.key, "fake private key\n", 0o600)
        write(self.key + ".pub", OUR_KEY + "\n")
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"), "HOME": t,
                    "MPFREE_DEVMODE_TEST": "1", "FAKE_STAGE": self.stage, "FAKE_LOG": self.log,
                    "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SERVER_KEY": self.server_key,
                    "FAKE_ACTIVE": "acvs.service sshd.service", "FAKE_AUTORUN": "1"}
        self.build()
        self.make_root()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def build(self, akai=True):
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        shutil.rmtree(folder, ignore_errors=True)
        write(os.path.join(folder, "stick.img"), b"\0" * 512)
        write(os.path.join(folder, "cmdline.txt"), CMDLINE + "\n")
        write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        write(os.path.join(folder, "report.txt"), f"its kernel release: {NEW}\n"
              + ("akai app: on (--akai-app)\n" if akai else ""))

    def make_root(self, devmode=True, data="tmpfs"):
        r = self.root
        shutil.rmtree(r, ignore_errors=True)
        write(f"{r}/proc/sys/kernel/osrelease", NEW + "\n")
        for name in ("sda", "sdb", "loop0"):
            os.makedirs(f"{r}/sys/block/{name}")
        write(f"{r}/sys/class/block/sda1/dev", "8:1\n")
        write(f"{r}/dev/sda1", "")
        os.makedirs(f"{r}/dev/disk/by-partuuid")
        os.symlink("../../sda1", f"{r}/dev/disk/by-partuuid/{P1}")
        mountinfo = ("20 1 8:1 / / ro,relatime shared:1 - ext4 /dev/root ro\n"
                     f"30 20 0:25 / /data rw,relatime shared:9 - {data} {data} rw,size=262144k\n"
                     "31 20 0:26 / /etc rw,relatime shared:10 - overlay overlay "
                     "rw,lowerdir=/etc,upperdir=/data/system/etc/overlay,workdir=/data/system/etc/.work\n")
        if devmode:
            mountinfo += ("90 31 0:40 / /etc rw,relatime shared:40 - overlay overlay "
                          "ro,lowerdir=/usr/lib/az0x/user-access/etc-overlay:/etc\n")
        write(f"{r}/proc/self/mountinfo", mountinfo)
        write(f"{r}/proc/mounts", "/dev/sda2 /media/MPFREE-LOG ext4 rw 0 0\n"
                                  "/dev/sdb1 /media/SPARE\\040DRIVE vfat rw,nosuid,nodev 0 0\n"
                                  "tmpfs /data tmpfs rw 0 0\n")
        write(f"{r}/media/SPARE DRIVE/mpfree-autorun-result.txt", "id: uid=0(root) gid=0(root)\nscript: autorun.sh\n")
        write(f"{r}/usr/sbin/sshd", FAKE_SSHD, 0o755)
        write(f"{r}/etc/ssh/ssh_host_ed25519_key.pub", AKAI_HOST_KEY + " root@mpc\n")
        write(f"{r}/etc/mpfree-test/authorized_keys", OUR_KEY + "\n")
        write(f"{r}/data/system/etc/overlay/.keep", "")
        tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n"
        tcp += "   0: 00000000:08AE 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1\n"
        tcp += "   1: 8D002AC0:08AE 01002AC0:D2F0 01 00000000:00000000 02:00000000 00000000     0        0 2\n"
        write(f"{r}/proc/1200/cmdline", "sshd: /usr/sbin/sshd -D -e -f /etc/mpfree-test/sshd_config [listener]\0")
        write(f"{r}/proc/812/cmdline", "/usr/bin/MPC\0")
        if devmode:
            tcp += "   2: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 3\n"
            write(f"{r}/etc/shadow", "root::15069:0:99999:7:::\ndaemon:*:15069:0:99999:7:::\n")
            write(f"{r}/data/ssh/sshd_config.d/allow-access.conf", ALLOW_ACCESS)
            write(f"{r}/data/ssh/authorized_keys", "")
            write(f"{r}/data/root/profile", "/usr/lib/az0x/user-access/banner\ncd /data\n")
            write(f"{r}/run/fake-sshd-loaded/allow-access.conf", ALLOW_ACCESS)
            write(f"{r}/run/fake-sshd-loaded/authorized_keys", "")
            write(f"{r}/proc/1300/cmdline", "sshd: /usr/sbin/sshd -D [listener] 0 of 10-100 startups\0")
        else:
            write(f"{r}/etc/shadow", "root:*:15069:0:99999:7:::\n")
        write(f"{r}/proc/net/tcp", tcp)
        write(f"{r}/proc/net/tcp6", tcp.splitlines()[0] + "\n")

    def run_helper(self, answer="LOCK\n", **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", TEST_IP, "--key", self.key, "--work", self.work,
               "--report", self.report]
        return subprocess.run(cmd, input=answer, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def commands(self):
        return read(self.log)

    def assert_unchanged(self):
        self.assertEqual(read(f"{self.root}/data/ssh/sshd_config.d/allow-access.conf"), ALLOW_ACCESS)
        self.assertEqual(read(f"{self.root}/data/ssh/authorized_keys"), "")
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-devmode"))
        self.assertNotIn("systemctl: reload", self.commands())

    def test_reads_logs_in_and_locks_down(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        blob = base64.b64decode(AKAI_HOST_KEY.split()[1])
        import hashlib
        fp = "SHA256:" + base64.b64encode(hashlib.sha256(blob).digest()).decode().rstrip("=")
        for text in ("made with --akai-app; release " + NEW, "/data: tmpfs tmpfs rw,size=262144k",
                     "Akai's app (acvs.service): active", "Akai's SSH server (sshd.service): active",
                     "developer mode overlay: mounted", "lowerdir=/usr/lib/az0x/user-access/etc-overlay:/etc",
                     "  | AuthenticationMethods none", "keys in /data/ssh/authorized_keys (their type only): 0",
                     "root's password field: empty: no password is needed", "  authenticationmethods none",
                     "  hostkey /etc/ssh/ssh_host_ed25519_key", "listening TCP ports: 22 2222 ",
                     "  1300: sshd: /usr/sbin/sshd -D [listener] 0 of 10-100 startups",
                     "  /dev/sdb1 /media/SPARE\\040DRIVE vfat", "  /media/SPARE DRIVE/mpfree-autorun-result.txt:",
                     "  | id: uid=0(root) gid=0(root)", "mpfree-autorun: ran as 0 from /media/SPARE DRIVE/autorun.sh",
                     "MPC[812]: DevMode: Developer mode on", "sshd[900]: Server listening on 0.0.0.0 port 22.",
                     "mpfree-test: blocked /usr/bin/dfu-util -l", "CHECKS-OK",
                     f"Akai's SSH host key (read over port 2222): {fp}",
                     "no key, no password: logged in", "id -u printed 0 (root)", "your key: logged in",
                     "answer: LOCK", "copied to /run/mpfree-devmode: allow-access.conf authorized_keys",
                     "emptied /data/ssh/sshd_config.d/allow-access.conf",
                     "added your key to /data/ssh/authorized_keys (1 key line(s) now)",
                     "systemctl reload sshd.service: exit code 0", "  authenticationmethods any", "LOCK-DONE",
                     "no key, no password, after the lock: refused", "Permission denied (publickey).",
                     "your key, after the lock: logged in",
                     "let a login with no key and no password in: anyone on this network can get a root shell",
                     "Akai's lock-down works: after it, the login with no key is refused and your key gets in."):
            self.assertIn(text, out)
        self.assertNotIn("ciphers", out, "only the settings that decide who gets in")
        self.assertNotIn("unrelated line", out)
        self.assertNotIn("PROBLEM", out)
        r = self.root
        self.assertEqual(read(f"{r}/data/ssh/sshd_config.d/allow-access.conf"), "")
        self.assertEqual(read(f"{r}/data/ssh/authorized_keys"), OUR_KEY + "\n")
        self.assertEqual(read(f"{r}/run/mpfree-devmode/allow-access.conf"), ALLOW_ACCESS)
        self.assertEqual(read(f"{r}/run/mpfree-devmode/authorized_keys"), "")
        self.assertEqual(oct(os.stat(f"{r}/run/mpfree-devmode").st_mode & 0o777), "0o700")
        log = self.commands()
        self.assertEqual(re.findall(r"^systemctl: (\S+ \S+)", log, re.M),
                         ["is-active acvs.service", "is-active sshd.service", "reload sshd.service"])
        self.assertEqual(re.findall(r"^ssh\[(\d+)@310\] key=(\w+): (.*)$", log, re.M),
                         [("2222", "no", "sh -s"), ("22", "no", "id -u"), ("22", "yes", "id -u"),
                          ("2222", "no", "sh -s"), ("22", "no", "id -u"), ("22", "yes", "id -u")])
        self.assertIn("Akai's lock-down works", read(self.report), "the report gets the same lines")

    def test_lock_down_needs_the_typed_word(self):
        for answer in ("\n", "lock\n", "yes\n", ""):
            res = self.run_helper(answer)
            self.assertEqual(res.returncode, 0, res.stdout)
            self.assertIn("skipped (LOCK not typed)", res.stdout)
            self.assertIn("no key, no password: logged in", res.stdout)
            self.assert_unchanged()

    def test_lock_down_that_does_not_work_is_reported(self):
        # The reload fails: the files change, but the server keeps what it had.
        system = os.path.join(self.tmp, "remote-bin", "systemctl")
        write(system, FAKE_SYSTEMCTL.replace("  reload)\n", "  reload)\n    exit 1\n"), 0o755)
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("LOCK-RELOAD-FAILED", res.stdout)
        self.assertIn("the lock-down did not finish", res.stdout)
        self.assertIn("Akai's lock-down: failed (see step 4).", res.stdout)

    def test_stops_on_the_wrong_build_or_machine(self):
        self.build(akai=False)
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("was not made with --akai-app", res.stdout)
        self.assertEqual(self.commands(), "", "nothing is sent to the MPC")
        self.assertIn("STOP:", read(self.report))
        self.build()
        write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("port 2222 answered with another host key, so the MPC is not running this test build", res.stdout)
        self.assertNotRegex(self.commands(), r"ssh\[22@")
        write(self.stage, "off\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer on port 2222", res.stdout)
        self.assertNotRegex(self.commands(), r"ssh\[22@")
        write(self.stage, "310\n")
        self.make_root(data="ext4")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("PROBLEM: /data is not a tmpfs", res.stdout)
        self.assertIn("safety checks did not pass", res.stdout)
        self.assertNotRegex(self.commands(), r"ssh\[22@")
        self.assert_unchanged()
        self.make_root()
        write(self.server_key, OTHER_HOST_KEY + "\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("a different host key answered", res.stdout)
        self.assertIn("is not the one the test build reported", res.stdout)
        self.assert_unchanged()

    def test_developer_mode_off(self):
        self.make_root(devmode=False)
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        for text in ("developer mode overlay: not mounted", "listening TCP ports: 2222 ",
                     "root's password field: locked: no password works", "nothing listens on port 22",
                     "developer mode is not on. On the MPC: Menu > Preferences > Enhancements"):
            self.assertIn(text, res.stdout)
        self.assertNotRegex(self.commands(), r"ssh\[22@")

    def test_wrapper_needs_a_terminal(self):
        res = subprocess.run(["bash", WRAPPER, "--devmode-check"], input="", env=dict(self.env, MPC_KEY=self.key),
                             text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("--devmode-check asks you before the lock-down: run it from a terminal", res.stdout)
        self.assertEqual(self.commands(), "")


class StaticTests(unittest.TestCase):
    def test_remote_scripts(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("host_devmode_check", HELPER)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        scripts = {n: getattr(mod, n) for n in ("STATE", "LOCK", "JOURNAL")}
        for name, text in scripts.items():
            self.assertTrue(text.startswith("R=''\n"), name)
            self.assertIsNone(re.search(r"(?<![0-9])>\s*\S*/dev/", text), f"{name}: writes to a device")
            for needle in ("of=", " dd ", "fw_setenv", "mount ", "kexec", "flash", "/boot/", "tee ", "rm ",
                           "mkfs", "reboot", "poweroff", "halt", "start ", "stop ", "restart", "/etc/ssh/",
                           "chmod", "chown", "mv "):
                self.assertNotIn(needle, text, f"{name}: {needle}")
        for name in ("STATE", "JOURNAL"):
            writes = [ln for ln in scripts[name].splitlines() if re.search(r"(^|[^2])>[^&]", ln)]
            self.assertEqual(writes, [], name)
            self.assertNotIn("systemctl reload", scripts[name])
            self.assertNotIn("cp ", scripts[name])
            self.assertNotIn("mkdir", scripts[name])
        lock = scripts["LOCK"]
        writes = sorted(re.findall(r'(?:^|[^2])(>>?) ?"?(\$[a-z]+)"?', lock, re.M))
        self.assertEqual(writes, [(">", "$conf"), (">>", "$keys")], writes)
        self.assertEqual(re.findall(r'\bcp ("\$\w+") ("[^"]+")', lock),
                         [('"$conf"', '"$b/allow-access.conf"'), ('"$keys"', '"$b/authorized_keys"')])
        self.assertIn('d="$R/data/ssh"', lock)
        self.assertIn('b="$R/run/mpfree-devmode"', lock)
        self.assertEqual(re.findall(r"^.*\bsystemctl\b.*$", lock, re.M),
                         ["systemctl reload sshd.service", 'say "systemctl reload sshd.service: exit code $rc"'])
        self.assertIn("systemctl reload sshd.service\n", lock)
        # The checks that it is the test build with /data in RAM come before the first change.
        first_change = lock.index("mkdir")
        self.assertLess(lock.index("not @@NEW@@"), first_change)
        self.assertLess(lock.index('*" - tmpfs "*'), first_change)


if __name__ == "__main__":
    unittest.main(verbosity=2)
