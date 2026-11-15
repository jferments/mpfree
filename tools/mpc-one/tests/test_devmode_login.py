#!/usr/bin/env python3
"""host_devmode_login.py against a fake installed MPC OS 3.10.1 with developer mode on. No device.

A fake `ssh` stands in for the client and for Akai's SSH server: it checks the host key against the
known_hosts file it is given, lets a login in the way the server's loaded settings say (no key while
allow-access.conf said `AuthenticationMethods none` at the last reload, otherwise only a key listed in
/data/ssh/authorized_keys), keeps an OpenSSH-style control connection, and runs the remote scripts
under BusyBox against a fake root folder. A fake `systemctl reload sshd.service` loads the settings.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_devmode_login.py")
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
TEST_IP = "192.0.2.141"
APPLETS = ("sh", "cat", "grep", "ls", "head", "sed", "cut", "sort", "tr", "cp", "mkdir", "rm", "chmod", "wc",
           "sync")
OPEN_CONF = "AuthenticationMethods none\nPermitEmptyPasswords yes\n"

FAKE_SSH = r'''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
opts, flags, i = {}, set(), 0
while i < len(args) and args[i].startswith("-"):
    a = args[i]
    if a in ("-o", "-i", "-p", "-l", "-F", "-O"):
        v = args[i + 1]
        if a == "-o":
            k, _, val = v.partition("=")
            opts[k.lower()] = val
        else:
            opts[a] = v
        i += 2
    else:
        flags.add(a); i += 1
rest = args[i:]
root, state = os.environ["FAKE_ROOT"], os.environ["FAKE_SSHD_STATE"]
log = open(os.environ["FAKE_LOG"], "a")
control = opts.get("controlpath", "none")
if opts.get("-O") == "exit":
    log.write("ssh: close control\n")
    if control != "none" and os.path.exists(control):
        os.unlink(control)
    sys.exit(0)
port = opts.get("-p", "22")
host = rest[0].split("@", 1)[1]
name = host if port == "22" else f"[{host}]:{port}"
known = opts.get("userknownhostsfile", "")
want = f"{name} ssh-ed25519 {os.environ['FAKE_HOST_KEY']}"
lines = open(known).read().splitlines() if known and os.path.isfile(known) else []
if want not in lines:
    print("Host key verification failed.", file=sys.stderr)
    sys.exit(255)
def allowed():
    with open(state) as fh:
        loaded = fh.read()
    if "AuthenticationMethods none" in loaded:
        return True
    if opts.get("pubkeyauthentication") == "yes" and "-i" in opts and os.environ.get("FAKE_KEYS") != "ignored":
        mine = open(opts["-i"] + ".pub").read().split()[:2]
        path = os.path.join(root, "data/ssh/authorized_keys")
        held = [l.split()[:2] for l in open(path).read().splitlines()] if os.path.isfile(path) else []
        return mine in held
    return False
if "-N" in flags:
    if not allowed():
        print("root@host: Permission denied (publickey).", file=sys.stderr)
        sys.exit(255)
    open(control, "w").close()
    log.write("ssh: open control\n")
    sys.exit(0)
via = control != "none" and os.path.exists(control)
if not via and not allowed():
    print("root@host: Permission denied (publickey).", file=sys.stderr)
    sys.exit(255)
command = rest[1]
log.write(f"ssh[{'control' if via else 'new'}]: {command}\n")
log.flush()
env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
if command == "id -u":
    print("0"); sys.exit(0)
if command == "sh -s":
    script = sys.stdin.read()
    assert script.startswith("R=''\n"), "every remote script starts with R=''"
    script = f"R='{root}'\n" + script[len("R=''\n"):]
    sys.exit(subprocess.run([os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh"), "-c", script], env=env).returncode)
print(f"fake ssh: command not allowed: {command}", file=sys.stderr)
sys.exit(255)
'''

FAKE_KEYSCAN = r'''#!/bin/sh
[ "$FAKE_KEYSCAN" = none ] && exit 1
port=22
while [ $# -gt 1 ]; do case "$1" in -p) port=$2; shift 2 ;; -T|-t) shift 2 ;; *) shift ;; esac; done
if [ "$port" = 22 ]; then name=$1; else name="[$1]:$port"; fi
echo "# $name:$port SSH-2.0-OpenSSH_9.6" >&2
echo "$name ssh-ed25519 $FAKE_HOST_KEY"
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1 $2" in
  "reload sshd.service")
    [ "$FAKE_RELOAD" = ignored ] && exit 0
    [ "$FAKE_RELOAD" = fails ] && exit 1
    cat "$FAKE_ROOT/data/ssh/sshd_config.d/allow-access.conf" > "$FAKE_SSHD_STATE" ;;
  "is-active sshd.service") echo active ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

FAKE_SSHD = r'''#!/bin/sh
[ "$1" = -T ] || exit 1
echo "port 22"
echo "permitrootlogin yes"
echo "passwordauthentication yes"
if grep -q 'AuthenticationMethods none' "$FAKE_ROOT/data/ssh/sshd_config.d/allow-access.conf"; then
  echo "authenticationmethods none"; echo "permitemptypasswords yes"
elif [ "$FAKE_EMPTY_PASSWORDS" = always ]; then
  echo "authenticationmethods any"; echo "permitemptypasswords yes"
else
  echo "authenticationmethods any"; echo "permitemptypasswords no"
fi
echo "hostkey /etc/ssh/ssh_host_ed25519_key"
'''


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


def read(path):
    with open(path) as fh:
        return fh.read()


def tools_present():
    return bool(shutil.which("busybox") and shutil.which("ssh-keygen"))


@unittest.skipUnless(tools_present() or os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1",
                     "needs BusyBox and ssh-keygen")
class DevmodeLoginTests(unittest.TestCase):
    def setUp(self):
        if not tools_present():
            self.fail("BusyBox or ssh-keygen is missing")
        self.tmp = tempfile.mkdtemp(prefix="mpfree-devmode-login-test-")
        t = self.tmp
        self.root, self.log = os.path.join(t, "root"), os.path.join(t, "log")
        self.state, self.known = os.path.join(t, "sshd-state"), os.path.join(t, "home", ".ssh", "known_hosts")
        self.report = os.path.join(t, "report.txt")
        self.key = os.path.join(t, "mpc_key")
        for path, comment in ((self.key, "jay@workstation"), (os.path.join(t, "hostkey"), "host")):
            res = subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", comment, "-f", path],
                                 capture_output=True, text=True)
            self.assertEqual(res.returncode, 0, res.stderr)
        self.host_key = read(os.path.join(t, "hostkey.pub")).split()[1]
        self.pub = read(self.key + ".pub").strip()
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        write(os.path.join(fakebin, "ssh-keyscan"), FAKE_KEYSCAN, 0o755)
        os.symlink(shutil.which("ssh-keygen"), os.path.join(fakebin, "ssh-keygen"))
        os.makedirs(remote)
        for name in APPLETS:
            os.symlink(shutil.which("busybox"), os.path.join(remote, name))
        write(os.path.join(remote, "systemctl"), FAKE_SYSTEMCTL, 0o755)
        os.symlink(shutil.which("ssh-keygen"), os.path.join(remote, "ssh-keygen"))
        self.make_root()
        write(self.log, "")
        write(self.state, OPEN_CONF)
        self.env = {"PATH": fakebin + os.pathsep + "/usr/bin:/bin", "HOME": os.path.join(t, "home"),
                    "MPFREE_DEVMODE_LOGIN_TEST": "1", "FAKE_ROOT": self.root, "FAKE_SSHD_STATE": self.state,
                    "FAKE_LOG": self.log, "FAKE_REMOTE_PATH": remote, "FAKE_HOST_KEY": self.host_key}

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def make_root(self, data_source="/dev/mmcblk0p8", overlay=True):
        r = self.root
        shutil.rmtree(r, ignore_errors=True)
        os.makedirs(f"{r}/sys/block/mmcblk0")
        os.makedirs(f"{r}/sys/block/sda")
        etc = ("40 20 0:40 / /etc rw,relatime - overlay overlay rw,lowerdir=/etc,"
               "upperdir=/data/root/user-access/etc-overlay/upper\n") if overlay else ""
        write(f"{r}/proc/self/mountinfo",
              "20 1 179:6 / / ro,relatime shared:1 - ext4 /dev/mmcblk0p6 ro\n"
              f"30 20 179:8 / /data rw,relatime shared:9 - ext4 {data_source} rw\n" + etc)
        write(f"{r}/proc/sys/kernel/osrelease", "6.18.44-fake-rt6\n")
        write(f"{r}/proc/uptime", "1234.56 4000.00\n")
        write(f"{r}/etc/ssh/ssh_host_ed25519_key.pub", read(os.path.join(self.tmp, "hostkey.pub")))
        write(f"{r}/etc/version", "3.10.1.fake\n")
        write(f"{r}/etc/shadow", "root::19000:0:99999:7:::\n")
        write(f"{r}/data/ssh/sshd_config.d/allow-access.conf", OPEN_CONF)
        write(f"{r}/usr/sbin/sshd", FAKE_SSHD, 0o755)
        write(f"{r}/proc/net/tcp", "  sl  local_address rem_address   st\n   0: 00000000:0016 00000000:0000 0A\n")

    def run_helper(self, action, answer="", **extra):
        cmd = [sys.executable, "-I", HELPER, action, "--ip", TEST_IP, "--key", self.key, "--known-hosts", self.known,
               "--report", self.report]
        return subprocess.run(cmd, input=answer, env=dict(self.env, **extra), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def trust(self):
        res = self.run_helper("trust", "TRUST\n")
        self.assertEqual(res.returncode, 0, res.stdout)
        return res

    def files(self):
        out = {}
        for base, _, names in os.walk(os.path.join(self.root, "data")):
            for name in names:
                path = os.path.join(base, name)
                out[os.path.relpath(path, self.root)] = read(path)
        return out

    def test_trust_records_the_key_only_after_trust(self):
        res = self.run_helper("trust", "no\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("not confirmed; nothing was written", res.stdout)
        self.assertFalse(os.path.exists(self.known))
        res = self.trust()
        self.assertEqual(read(self.known), f"{TEST_IP} ssh-ed25519 {self.host_key}\n")
        self.assertEqual(os.stat(self.known).st_mode & 0o777, 0o600)
        self.assertIn("no key, no password: logged in", res.stdout)
        self.assertIn("anyone on this network can get a root shell", res.stdout)
        fp = subprocess.run(["ssh-keygen", "-lf", os.path.join(self.tmp, "hostkey.pub")], capture_output=True,
                            text=True).stdout.split()[1]
        self.assertIn(fp, res.stdout, "the fingerprint is OpenSSH's")
        res = self.run_helper("trust", "")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("already recorded: nothing to add", res.stdout)
        self.assertEqual(read(self.known).count("\n"), 1)

    def test_trust_never_replaces_another_key(self):
        other = subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", os.path.join(self.tmp, "other")],
                               capture_output=True)
        self.assertEqual(other.returncode, 0)
        old = f"{TEST_IP} ssh-ed25519 {read(os.path.join(self.tmp, 'other.pub')).split()[1]}\n"
        write(self.known, old, 0o600)
        res = self.run_helper("trust", "TRUST\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("already holds a different key", res.stdout)
        self.assertIn(f"ssh-keygen -R '{TEST_IP}' -f {self.known}", res.stdout)
        self.assertEqual(read(self.known), old)

    def test_trust_needs_developer_mode(self):
        res = self.run_helper("trust", "TRUST\n", FAKE_KEYSCAN="none")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("Is developer mode on?", res.stdout)
        self.assertFalse(os.path.exists(self.known))

    def test_an_unrecorded_host_key_stops_everything_else(self):
        for action in ("status", "lock", "unlock"):
            with self.subTest(action):
                res = self.run_helper(action, "LOCK\n" if action == "lock" else "UNLOCK\n")
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("the --devmode-trust block shows it and says what to do", res.stdout)
        self.assertEqual(read(f"{self.root}/data/ssh/sshd_config.d/allow-access.conf"), OPEN_CONF)
        self.assertNotIn("sh -s", read(self.log))

    def test_lock_status_and_unlock(self):
        self.trust()
        before = self.files()
        res = self.run_helper("status")
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("release: 6.18.44-fake-rt6", "MPC OS version (/etc/version): 3.10.1.fake",
                     "/data: ext4 /dev/mmcblk0p8 rw", "developer mode overlay on /etc: mounted",
                     "| AuthenticationMethods none", "keys in /data/ssh/authorized_keys (their type only): 0",
                     "our lock: not in place", "root's password field: empty: no password is needed",
                     "authenticationmethods none", "listening TCP ports: 22", "OPEN: a login with no key",
                     "  /etc mount: overlay overlay rw,lowerdir=/etc,upperdir=/data/root/user-access/etc-overlay/upper",
                     "seconds since the MPC started: 1234", "SSH host key: 256 SHA256:", "  made or changed: 20"):
            self.assertIn(text, res.stdout)

        res = self.run_helper("lock", "nope\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.files(), before, "nothing changes without LOCK")

        res = self.run_helper("lock", "LOCK\n")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("LOCKED: the login with no key is refused and your key gets in", res.stdout)
        for line in ("cp /data/ssh/mpfree-lock-backup/allow-access.conf /data/ssh/sshd_config.d/allow-access.conf",
                     "systemctl reload sshd.service"):
            self.assertIn(line, res.stdout)
        after = self.files()
        self.assertEqual(after["data/ssh/sshd_config.d/allow-access.conf"], "")
        self.assertEqual(after["data/ssh/authorized_keys"], self.pub + "\n")
        self.assertEqual(after["data/ssh/mpfree-lock-backup/allow-access.conf"], OPEN_CONF)
        self.assertEqual(after["data/ssh/mpfree-lock-backup/absent"], "")
        self.assertEqual(set(after) - set(before), {"data/ssh/authorized_keys", "data/ssh/mpfree-lock-backup/absent",
                                                    "data/ssh/mpfree-lock-backup/allow-access.conf"})
        log = read(self.log)
        self.assertIn("ssh: open control", log)
        self.assertIn("ssh: close control", log)
        self.assertRegex(log, r"ssh\[control\]: sh -s\n(.*\n)*systemctl: reload sshd.service")

        res = self.run_helper("status")
        self.assertIn("LOCKED: only your key gets in.", res.stdout)
        self.assertIn("our lock: in place", res.stdout)
        res = self.run_helper("lock", "LOCK\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("there is nothing to lock", res.stdout)

        res = self.run_helper("unlock", "UNLOCK\n")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("UNLOCKED: Akai's files are back", res.stdout)
        self.assertEqual(self.files(), before, "unlock puts every file back as it was")

    def test_lock_keeps_an_existing_authorized_keys(self):
        self.trust()
        theirs = "ssh-ed25519 AAAAexisting someone\n"
        write(f"{self.root}/data/ssh/authorized_keys", theirs, 0o600)
        before = self.files()
        self.assertEqual(self.run_helper("lock", "LOCK\n").returncode, 0)
        self.assertEqual(self.files()["data/ssh/authorized_keys"], theirs + self.pub + "\n")
        self.assertEqual(self.files()["data/ssh/mpfree-lock-backup/authorized_keys"], theirs)
        self.assertEqual(self.run_helper("unlock", "UNLOCK\n").returncode, 0)
        self.assertEqual(self.files(), before)

    def test_a_lock_that_does_not_work_undoes_itself(self):
        cases = (("reload ignored", {"FAKE_RELOAD": "ignored"}),
                 ("keys ignored", {"FAKE_KEYS": "ignored"}),
                 ("reload fails", {"FAKE_RELOAD": "fails"}),
                 ("empty passwords still taken", {"FAKE_EMPTY_PASSWORDS": "always"}))
        for label, extra in cases:
            with self.subTest(label):
                self.make_root()
                write(self.state, OPEN_CONF)
                write(self.log, "")
                self.trust()
                before = self.files()
                res = self.run_helper("lock", "LOCK\n", **extra)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("so it is undone now over the connection kept open", res.stdout)
                self.assertIn("NOT LOCKED: the lock-down did not work as Akai's banner says, and it was undone",
                              res.stdout)
                self.assertEqual(self.files(), before)
                self.assertIn("ssh: close control", read(self.log))

    def test_lock_checks_it_is_an_installed_mpc_os(self):
        cases = (("data in RAM", {"data_source": "tmpfs"}, "/data is not mounted from mmcblk0p8"),
                 ("no overlay", {"overlay": False}, "developer mode's overlay is not on /etc"))
        for label, kwargs, message in cases:
            with self.subTest(label):
                self.make_root(**kwargs)
                write(self.state, OPEN_CONF)
                self.trust()
                before = self.files()
                res = self.run_helper("lock", "LOCK\n")
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn(message, res.stdout)
                self.assertIn("the MPC's checks stopped it before any change", res.stdout)
                self.assertEqual(self.files(), before)
        self.make_root()
        os.makedirs(f"{self.root}/sys/block/mmcblk0", exist_ok=True)
        shutil.rmtree(f"{self.root}/sys/block/mmcblk0")
        write(self.state, OPEN_CONF)
        res = self.run_helper("lock", "LOCK\n")
        self.assertIn("no eMMC (mmcblk0) here", res.stdout)

    def test_a_bad_public_key_is_never_sent(self):
        self.trust()
        write(self.key + ".pub", "ssh-rsa AAAAB3 someone\n")
        res = self.run_helper("lock", "LOCK\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("is not one ssh-ed25519 public key line; nothing was sent", res.stdout)
        self.assertNotIn("sh -s", read(self.log))

    def test_wrapper_modes(self):
        wrapper = read(WRAPPER)
        for mode in ("--devmode-trust", "--devmode-status", "--devmode-lock", "--devmode-unlock"):
            self.assertIn(f'"{mode}")', wrapper)
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": self.tmp, "MPC_KEY": self.key}
        res = subprocess.run(["bash", WRAPPER, "--devmode-lock"], stdin=subprocess.DEVNULL, env=env, text=True,
                             capture_output=True, timeout=60)
        self.assertEqual(res.returncode, 1)
        self.assertIn("run it from a terminal", res.stderr)


class StaticTests(unittest.TestCase):
    def test_changing_scripts_write_only_under_data_ssh(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("host_devmode_login", HELPER)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        made = mod.scripts("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 jay@workstation")
        self.assertNotRegex(made["status"], r"(?m)^[^#]*\b(?:cp|rm|mkdir|chmod|reload)\b|>\s*\"?\$")
        for name in ("lock", "restore"):
            text = made[name]
            for target in re.findall(r">>?\s*(\"?\$\w+[^\s;)]*)", text):
                self.assertIn(target.strip('"').split("/")[0], ("$keys", "$conf", "$b"), f"{name}: > {target}")
            for line in text.splitlines():
                self.assertNotRegex(line, r"(^|[;&|]\s*)(?:dd|mount|umount|fw_setenv|mkfs\S*|reboot)\b", f"{name}: {line}")
                for word in ("systemctl stop", "systemctl restart", "systemctl start"):
                    self.assertNotIn(word, line, f"{name}: {line}")
            self.assertEqual(set(re.findall(r"systemctl \w+ [\w.]+", text)), {"systemctl reload sshd.service"})
        self.assertIn('b="$R/data/ssh/mpfree-lock-backup"', made["lock"])
        self.assertIn('keys="$d/authorized_keys"', made["lock"])
        self.assertIn('d="$R/data/ssh"', made["lock"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
