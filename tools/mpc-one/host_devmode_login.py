#!/usr/bin/env python3
"""Logging in to an installed MPC OS 3.10.1 through Akai's developer mode: trust, status, lock, unlock.

Run it through `collect-over-ssh.sh --devmode-trust|--devmode-status|--devmode-lock|--devmode-unlock`,
which first checks the cable, the route and the MPC's network identity (MAC). Developer mode
(hold SHIFT + MENU on the MPC, choose Developer) starts Akai's SSH server on port 22. As Akai ships
it, it lets root in with no key and no password (`AuthenticationMethods none`), so anyone on the
same network can get a root shell while it is on. Akai's login banner says how to lock it to a key:
empty /data/ssh/sshd_config.d/allow-access.conf and put the key in /data/ssh/authorized_keys.

  trust   READ-ONLY on the MPC. Reads the ed25519 host key of the SSH server on --port
          (ssh-keyscan), prints its fingerprint and compares it with --known-hosts (your
          ~/.ssh/known_hosts by default, the file every other tool here checks with
          StrictHostKeyChecking=yes). Only after you type TRUST it adds that one line to the file:
          the only write, and it is on the workstation. A different key already recorded for the
          MPC stops it: it never replaces a key by itself.
  status  READ-ONLY on the MPC. Which system runs (release, /etc/version, where / and /data come
          from), whether developer mode's overlay is on /etc, Akai's SSH server settings as it reads
          them (`sshd -T`), allow-access.conf, how many keys /data/ssh/authorized_keys holds (types
          only), root's password field kind (never its value), the listening TCP ports, whether
          a lock of ours is in place, and the SSH host key's fingerprint and date with the /etc
          mount and the time since start (run before and after a restart, they show whether the
          host key and the lock survive it). Then two logins that run `id -u` only: with no key,
          and with your key.
  lock    STATE-CHANGING, on /data (the eMMC's p8 data partition, which survives a reboot; never
          a boot partition, the U-Boot environment or the root filesystem). Asks you to type LOCK.
          It keeps one SSH connection open for the whole step (an OpenSSH control socket on the
          workstation), and over it:
            1. checks it is an installed MPC OS with developer mode on: an eMMC, /data mounted
               from mmcblk0p8, the developer mode overlay on /etc, allow-access.conf present, and
               no earlier lock of ours (/data/ssh/mpfree-lock-backup);
            2. copies allow-access.conf and authorized_keys into /data/ssh/mpfree-lock-backup
               (an `absent` mark when there was no authorized_keys);
            3. adds your public key (--key's .pub, one ssh-ed25519 line) to authorized_keys,
               empties allow-access.conf, and asks Akai's SSH server to read its settings again
               (`systemctl reload sshd.service`, the step Akai's banner gives).
          Then it tries two new logins: with no key it must be refused, with your key it must get
          in. If anything else happens, it puts both files back from the copy over the connection
          it kept open, reloads the server again, and checks that the login with no key gets in as
          before. Existing logins are not cut by a reload.
  unlock  STATE-CHANGING, on /data. The undo of lock. Asks you to type UNLOCK. Logs in with your
          key, puts both files back from /data/ssh/mpfree-lock-backup, removes that folder,
          reloads Akai's SSH server and checks that the login with no key gets in again (as Akai
          ships it).

Undo, if this tool cannot run: the developer panel's own terminal on the MPC's screen is a root
shell, where these three lines put the files back (they are printed again by lock):
    cp /data/ssh/mpfree-lock-backup/allow-access.conf /data/ssh/sshd_config.d/allow-access.conf
    cp /data/ssh/mpfree-lock-backup/authorized_keys /data/ssh/authorized_keys
    systemctl reload sshd.service
(when the copy holds the `absent` mark instead of authorized_keys, the second line is
`rm /data/ssh/authorized_keys`). Akai's files say switching developer mode off deletes /data/ssh
and /data/root and reboots, which also removes the lock; that has not been seen on this unit.
"""
import argparse
import base64
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile

# Tests set this so a question can be answered without a terminal. The typed word is still required.
TESTING = os.environ.get("MPFREE_DEVMODE_LOGIN_TEST") == "1"
WORDS = {"trust": "TRUST", "lock": "LOCK", "unlock": "UNLOCK"}
KEY_LINE = re.compile(r"^ssh-ed25519 [A-Za-z0-9+/]+={0,3}(?: [A-Za-z0-9@._+-]*)?$")
BACKUP = "/data/ssh/mpfree-lock-backup"

# sshd -T prints every setting in lower case; these are the ones that decide who gets in.
SSHD_SETTINGS = ("port|listenaddress|permitrootlogin|permitemptypasswords|authenticationmethods|"
                 "passwordauthentication|pubkeyauthentication|kbdinteractiveauthentication|authorizedkeysfile|"
                 "hostkey|usepam")

# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
# The checks every changing step makes before it writes anything. They stop with exit code 3.
PRECHECK = r"""
say() { printf '%s\n' "$*"; }
stop() { say "STOP: $*; nothing was changed"; exit 3; }
ls "$R/sys/block" | grep -q '^mmcblk0$' || stop "no eMMC (mmcblk0) here: this is not an installed MPC OS"
src=""
while read -r id parent majmin base mnt mopts rest; do
  if [ "$mnt" = /data ]; then src=${rest#*- }; fi
done < "$R/proc/self/mountinfo"
case "$src" in *" /dev/mmcblk0p8 "*) ;; *) stop "/data is not mounted from mmcblk0p8 ($src)" ;; esac
grep ' /etc ' "$R/proc/self/mountinfo" | grep -q 'user-access/etc-overlay' ||
  stop "developer mode's overlay is not on /etc (is developer mode on?)"
d="$R/data/ssh"
conf="$d/sshd_config.d/allow-access.conf"
keys="$d/authorized_keys"
b="$R@@BACKUP@@"
[ -f "$conf" ] || stop "there is no /data/ssh/sshd_config.d/allow-access.conf"
"""

STATUS = r"""R=''
say() { printf '%s\n' "$*"; }
say "release: $(cat "$R/proc/sys/kernel/osrelease")"
say "MPC OS version (/etc/version): $(head -n 1 "$R/etc/version" 2>/dev/null)"
say "block devices: $(ls "$R/sys/block" | tr '\n' ' ')"
while read -r id parent majmin base mnt mopts rest; do
  case "$mnt" in /|/data) say "$mnt: ${rest#*- } (options $mopts)" ;; esac
done < "$R/proc/self/mountinfo"
if grep ' /etc ' "$R/proc/self/mountinfo" | grep -q 'user-access/etc-overlay'; then
  say "developer mode overlay on /etc: mounted"
else
  say "developer mode overlay on /etc: not mounted"
fi
while read -r id parent majmin base mnt mopts rest; do
  [ "$mnt" = /etc ] && say "  /etc mount: ${rest#*- }"
done < "$R/proc/self/mountinfo"
say "seconds since the MPC started: $(cut -d. -f1 "$R/proc/uptime")"
h="$R/etc/ssh/ssh_host_ed25519_key.pub"
if [ -f "$h" ]; then
  say "SSH host key: $(ssh-keygen -l -f "$h" 2>/dev/null || echo '(ssh-keygen could not read it)')"
  say "  made or changed: $(date -r "$h" '+%Y-%m-%d %H:%M:%S' 2>/dev/null)"
else
  say "SSH host key: $h is not there"
fi
say "Akai's SSH server (sshd.service): $(systemctl is-active sshd.service 2>&1)"
say "/data/ssh/sshd_config.d/allow-access.conf:"
if [ -f "$R/data/ssh/sshd_config.d/allow-access.conf" ]; then
  [ -s "$R/data/ssh/sshd_config.d/allow-access.conf" ] || say "  (empty)"
  sed 's/^/  | /' "$R/data/ssh/sshd_config.d/allow-access.conf"
else
  say "  (not there)"
fi
say "keys in /data/ssh/authorized_keys (their type only): $(cat "$R/data/ssh/authorized_keys" 2>/dev/null | grep -c '^ssh-')"
grep '^ssh-' "$R/data/ssh/authorized_keys" 2>/dev/null | while read -r kind rest; do say "  $kind"; done
if [ -d "$R@@BACKUP@@" ]; then say "our lock: in place (@@BACKUP@@ holds Akai's files)"; else say "our lock: not in place"; fi
f=$(grep '^root:' "$R/etc/shadow" 2>/dev/null | cut -d: -f2)
[ -r "$R/etc/shadow" ] || f="unreadable"
case "$f" in
  unreadable) k="not readable here" ;;
  "") k="empty: no password is needed" ;;
  '!'*|'*'*) k="locked: no password works" ;;
  *) k="a password hash" ;;
esac
f=""
say "root's password field: $k"
say "Akai's SSH server settings as it reads them (sshd -T):"
"$R/usr/sbin/sshd" -T 2>&1 | grep -i -E '^(@@SETTINGS@@) |rror|fatal|no hostkeys' | sed 's/^/  /'
ports=$(for t in "$R/proc/net/tcp" "$R/proc/net/tcp6"; do
  [ -r "$t" ] || continue
  while read -r sl local remote st rest; do
    [ "$st" = 0A ] && printf '%d\n' "0x${local##*:}"
  done < "$t"
done | sort -n -u | tr '\n' ' ')
say "listening TCP ports: $ports"
say STATUS-DONE
"""

LOCK = r"""R=''
@@PRECHECK@@
[ -e "$b" ] && stop "@@BACKUP@@ is already there (an earlier lock of ours): run the unlock block first"
key='@@KEY@@'
case "$key" in ssh-ed25519\ *) ;; *) stop "the key sent is not one ssh-ed25519 line" ;; esac
[ "$(printf '%s\n' "$key" | wc -l)" = 1 ] || stop "the key sent is not one line"
umask 077
mkdir "$b" || stop "could not make @@BACKUP@@"
cp -p "$conf" "$b/allow-access.conf" || { rm -rf "$b"; stop "could not copy allow-access.conf"; }
if [ -f "$keys" ]; then
  cp -p "$keys" "$b/authorized_keys" || { rm -rf "$b"; stop "could not copy authorized_keys"; }
else
  : > "$b/absent" || { rm -rf "$b"; stop "could not mark authorized_keys as absent"; }
fi
sync
say "copied Akai's files to @@BACKUP@@: $(ls "$b" | tr '\n' ' ')"
printf '%s\n' "$key" >> "$keys" || { say "could not add your key"; exit 4; }
chmod 600 "$keys"
say "added your key to /data/ssh/authorized_keys ($(grep -c '^ssh-' "$keys") key line(s) now)"
: > "$conf" || { say "could not empty allow-access.conf"; exit 4; }
sync
say "emptied /data/ssh/sshd_config.d/allow-access.conf"
systemctl reload sshd.service
rc=$?
say "systemctl reload sshd.service: exit code $rc"
[ "$rc" = 0 ] && say LOCK-DONE || say LOCK-RELOAD-FAILED
"""

# Puts Akai's two files back from the copy. Used by unlock, and by lock when its logins fail.
RESTORE = r"""R=''
@@PRECHECK@@
[ -d "$b" ] || stop "there is no @@BACKUP@@, so there is nothing of ours to undo"
[ -f "$b/allow-access.conf" ] || stop "@@BACKUP@@ has no allow-access.conf"
if [ ! -f "$b/authorized_keys" ] && [ ! -f "$b/absent" ]; then stop "@@BACKUP@@ has neither authorized_keys nor the absent mark"; fi
cp -p "$b/allow-access.conf" "$conf" || { say "could not put allow-access.conf back"; exit 4; }
if [ -f "$b/authorized_keys" ]; then
  cp -p "$b/authorized_keys" "$keys" || { say "could not put authorized_keys back"; exit 4; }
else
  rm -f "$keys" || { say "could not remove authorized_keys"; exit 4; }
fi
rm -rf "$b" || { say "could not remove @@BACKUP@@"; exit 4; }
sync
say "@@RESTORED@@"
systemctl reload sshd.service
rc=$?
say "systemctl reload sshd.service: exit code $rc"
[ "$rc" = 0 ] && say RESTORE-DONE || say RESTORE-RELOAD-FAILED
"""

SETTINGS_AFTER = r"""R=''
"$R/usr/sbin/sshd" -T 2>&1 | grep -i -E '^(authenticationmethods|permitemptypasswords|passwordauthentication) '
"""
RESTORED = f"put Akai's allow-access.conf and authorized_keys back; removed {BACKUP}"
BY_HAND = (f"cp {BACKUP}/allow-access.conf /data/ssh/sshd_config.d/allow-access.conf",
           f"cp {BACKUP}/authorized_keys /data/ssh/authorized_keys",
           "systemctl reload sshd.service")


class Stop(Exception):
    pass


class Out:
    def __init__(self, path):
        self.fh = open(path, "a") if path else None

    def say(self, text=""):
        print(text, flush=True)
        if self.fh:
            self.fh.write(text + "\n")
            self.fh.flush()


def fill(script, **values):
    for key, value in values.items():
        script = script.replace(f"@@{key}@@", value)
    if "@@" in script:
        raise Stop("internal error: a remote script has an unfilled value")
    return script


def scripts(key="ssh-ed25519 AAAA"):
    """The remote scripts, filled. The key is checked against KEY_LINE first, so it is safe in single quotes."""
    if not KEY_LINE.match(key):
        raise Stop("internal error: not one ssh-ed25519 key line")
    pre = fill(PRECHECK, BACKUP=BACKUP)
    return {"status": fill(STATUS, BACKUP=BACKUP, SETTINGS=SSHD_SETTINGS),
            "lock": fill(LOCK, PRECHECK=pre, BACKUP=BACKUP, KEY=key),
            "restore": fill(RESTORE, PRECHECK=pre, BACKUP=BACKUP, RESTORED=RESTORED)}


def ask(prompt):
    if not TESTING:
        if not sys.stdin.isatty():
            raise Stop("run this from a terminal: it asks you to confirm")
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)  # drop anything typed before the question
    try:
        return input(prompt).strip()
    except EOFError:
        return ""


def fingerprint(blob_b64):
    """OpenSSH's SHA256 fingerprint of a key's base64 blob."""
    blob = base64.b64decode(blob_b64)
    return "SHA256:" + base64.b64encode(hashlib.sha256(blob).digest()).decode().rstrip("=")


def host_name(args):
    """How OpenSSH names this server in known_hosts: the address alone on port 22, [address]:port otherwise."""
    return args.ip if args.port == 22 else f"[{args.ip}]:{args.port}"


class Ssh:
    """The ssh commands, all with strict host key checking against --known-hosts and nothing forwarded."""

    def __init__(self, args, tmp):
        self.args = args
        self.socket = os.path.join(tmp, "control")
        self.base = ["ssh", "-F", "/dev/null", "-T", "-p", str(args.port),
                     "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o", "StrictHostKeyChecking=yes",
                     "-o", f"UserKnownHostsFile={args.known_hosts}", "-o", "GlobalKnownHostsFile=/dev/null",
                     "-o", "UpdateHostKeys=no", "-o", "PasswordAuthentication=no",
                     "-o", "KbdInteractiveAuthentication=no", "-o", "GSSAPIAuthentication=no",
                     "-o", "HostbasedAuthentication=no", "-o", "IdentityAgent=none", "-o", "ForwardAgent=no",
                     "-o", "ClearAllForwardings=yes", "-o", "LogLevel=ERROR"]
        self.target = f"{args.user}@{args.ip}"

    def auth(self, key):
        if key:
            return ["-o", "PubkeyAuthentication=yes", "-o", "IdentitiesOnly=yes", "-i", self.args.key]
        return ["-o", "PubkeyAuthentication=no"]

    def run(self, command, stdin=b"", key=True, master=False, timeout=60):
        cmd = list(self.base)
        if master:
            cmd += ["-o", f"ControlPath={self.socket}"]
        else:
            cmd += ["-o", "ControlPath=none"]
        cmd += self.auth(key) + [self.target, command]
        try:
            res = subprocess.run(cmd, input=stdin, capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return 124, f"no answer within {timeout} s"
        return res.returncode, (res.stdout + res.stderr).decode("utf-8", "replace")

    def open_master(self, key):
        cmd = self.base + ["-o", f"ControlPath={self.socket}", "-o", "ControlMaster=yes", "-o",
                           "ControlPersist=600", "-f", "-N"] + self.auth(key) + [self.target]
        try:
            res = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, timeout=40)
        except subprocess.TimeoutExpired:
            return False, "no answer within 40 s"
        return res.returncode == 0, (res.stdout + res.stderr).decode("utf-8", "replace").strip()

    def close_master(self):
        subprocess.run(self.base + ["-o", f"ControlPath={self.socket}", "-O", "exit", self.target],
                       stdin=subprocess.DEVNULL, capture_output=True, timeout=20)

    def probe(self, key):
        """A new login that runs `id -u`: in, refused, wrong-host, closed or none."""
        rc, text = self.run("id -u", key=key, timeout=40)
        text = text.strip()
        if rc == 0:
            return "in", text
        if "Host key verification failed" in text or "IDENTIFICATION HAS CHANGED" in text:
            return "wrong-host", text
        if "Permission denied" in text:
            return "refused", text
        if "Connection refused" in text:
            return "closed", text
        return "none", text or f"exit code {rc}"


def show_probe(out, label, result):
    outcome, text = result
    meaning = {"in": "logged in", "refused": "refused", "closed": "nothing listens on that port",
               "wrong-host": "the host key is not the one recorded for the MPC", "none": "no login"}[outcome]
    out.say(f"  {label}: {meaning}")
    for line in text.splitlines()[:4]:
        out.say(f"    | {line}")
    return outcome


def wrong_host(args):
    return Stop(f"the SSH server on port {args.port} did not show the host key recorded for {host_name(args)} in "
                f"{args.known_hosts}. Nothing was changed. If developer mode was switched on again or the MPC "
                "restarted, it may have made a new host key: the --devmode-trust block shows it and says what to do")


def recorded_keys(args):
    """The ed25519 keys --known-hosts holds for this server (hashed entries too), via ssh-keygen -F."""
    if not os.path.isfile(args.known_hosts):
        return []
    res = subprocess.run(["ssh-keygen", "-F", host_name(args), "-f", args.known_hosts],
                         capture_output=True, text=True, timeout=20)
    keys = []
    for line in res.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 3 and not line.startswith("#"):
            keys.append((parts[1], parts[2]))
    return keys


def trust(args, out, ssh):
    out.say(f"\n[1/3] The host key on port {args.port} (read-only: ssh-keyscan)")
    try:
        res = subprocess.run(["ssh-keyscan", "-T", "8", "-t", "ed25519", "-p", str(args.port), args.ip],
                             capture_output=True, text=True, timeout=40)
    except subprocess.TimeoutExpired:
        raise Stop("ssh-keyscan got no answer within 40 s. Is developer mode on? (hold SHIFT + MENU, choose "
                   "Developer.) Nothing was changed")
    found = []
    for line in res.stdout.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[0] == host_name(args) and parts[1] == "ssh-ed25519":
            found.append(parts[2])
    if len(found) != 1:
        raise Stop(f"expected one ed25519 host key from {host_name(args)}, got {len(found)}. Is developer mode on? "
                   "(hold SHIFT + MENU, choose Developer.) Nothing was changed")
    key = found[0]
    out.say(f"  {host_name(args)} ssh-ed25519 {fingerprint(key)}")

    out.say(f"\n[2/3] Compare with {args.known_hosts}")
    recorded = recorded_keys(args)
    if ("ssh-ed25519", key) in recorded:
        out.say("  already recorded: nothing to add")
    elif recorded:
        for kind, blob in recorded:
            out.say(f"  recorded now: {kind} {fingerprint(blob) if kind == 'ssh-ed25519' else '(another type)'}")
        raise Stop(f"{args.known_hosts} already holds a different key for {host_name(args)}. Developer mode may make a "
                   "new host key when it is switched on again or the MPC restarts, but this is also what another "
                   "device on the cable would look like. If the fingerprint above is the MPC's (see the notes on comparing it), remove "
                   f"the old line with: ssh-keygen -R '{host_name(args)}' -f {args.known_hosts}   and run this block "
                   "again. Nothing was changed")
    else:
        out.say("  no key recorded for the MPC yet")
        out.say("  The cable, the route and the MPC's network identity (MAC) were checked before this. To check")
        out.say("  the key itself too, open the developer panel's terminal on the MPC and compare the")
        out.say("  fingerprint above with what `ssh-keygen -lf` prints for the ed25519 host key file that")
        out.say("  `sshd -T | grep hostkey` names (the status block prints that file's name).")
        answer = ask(f"  Type {WORDS['trust']} and press Enter to record this key (anything else stops): ")
        out.say(f"  answer: {answer!r}")
        if answer != WORDS["trust"]:
            raise Stop("not confirmed; nothing was written")
        folder = os.path.dirname(os.path.abspath(args.known_hosts))
        os.makedirs(folder, mode=0o700, exist_ok=True)
        fd = os.open(args.known_hosts, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        with os.fdopen(fd, "a") as fh:
            fh.write(f"{host_name(args)} ssh-ed25519 {key}\n")
        out.say(f"  added to {args.known_hosts}: {host_name(args)} ssh-ed25519 {fingerprint(key)}")

    out.say("\n[3/3] Logins (read-only: `id -u` only)")
    none = show_probe(out, "no key, no password", ssh.probe(False))
    mine = show_probe(out, "your key", ssh.probe(True))
    if "wrong-host" in (none, mine):
        raise wrong_host(args)
    out.say("\nRESULT:")
    out.say(f"  the MPC's developer mode SSH host key is recorded in {args.known_hosts}.")
    if none == "in":
        out.say("  A login with no key and no password gets in: anyone on this network can get a root shell while")
        out.say("  developer mode is on. The --devmode-lock block locks it to your key.")
    return 0


def status(args, out, ssh):
    out.say("\n[1/2] The running system (read-only)")
    rc, text = ssh.run("sh -s", scripts()["status"].encode())
    if rc == 255 and ("Host key verification failed" in text or "IDENTIFICATION HAS CHANGED" in text):
        raise wrong_host(args)
    for line in text.splitlines():
        if line != "STATUS-DONE":
            out.say(f"  {line}")
    if "STATUS-DONE" not in text.splitlines():
        raise Stop("could not read the running system's state (see above)")
    out.say("\n[2/2] Logins (read-only: `id -u` only)")
    none = show_probe(out, "no key, no password", ssh.probe(False))
    mine = show_probe(out, "your key", ssh.probe(True))
    out.say("\nRESULT:")
    if none == "in":
        out.say("  OPEN: a login with no key and no password gets in.")
    elif none == "refused" and mine == "in":
        out.say("  LOCKED: only your key gets in.")
    else:
        out.say(f"  no key: {none}; your key: {mine}.")
    return 0


def public_key(args):
    with open(args.key + ".pub") as fh:
        line = fh.read().strip()
    if "\n" in line or not KEY_LINE.match(line):
        raise Stop(f"{args.key}.pub is not one ssh-ed25519 public key line; nothing was sent to the MPC")
    return line


def lock(args, out, ssh):
    key = public_key(args)
    out.say("\n[1/4] Before (read-only)")
    none = show_probe(out, "no key, no password", ssh.probe(False))
    mine = show_probe(out, "your key", ssh.probe(True))
    if "wrong-host" in (none, mine):
        raise wrong_host(args)
    if none != "in":
        raise Stop("the login with no key does not get in, so there is nothing to lock (the status block shows "
                   "why). Nothing was changed")

    out.say("\n[2/4] Your go")
    out.say("  This changes two files on the MPC's /data partition (p8), which survive a reboot:")
    out.say("  allow-access.conf is emptied and your key goes into /data/ssh/authorized_keys, after copies of both")
    out.say(f"  go to {BACKUP}. Then Akai's SSH server reads its settings again. The undo is the --devmode-unlock")
    out.say("  block; if the new logins below fail, this step undoes itself.")
    answer = ask(f"  Type {WORDS['lock']} and press Enter to lock it (anything else stops): ")
    out.say(f"  answer: {answer!r}")
    if answer != WORDS["lock"]:
        raise Stop("not confirmed; nothing was changed")

    out.say("\n[3/4] Lock (STATE-CHANGING on /data), over one connection kept open")
    ok, text = ssh.open_master(key=False)
    if not ok:
        raise Stop(f"could not open the connection to keep ({text}); nothing was changed")
    try:
        rc, text = ssh.run("sh -s", scripts(key)["lock"].encode(), key=False, master=True)
        for line in text.splitlines():
            out.say(f"  {line}")
        if rc == 3:
            raise Stop("the MPC's checks stopped it before any change (see above)")
        done = "LOCK-DONE" in text.splitlines()

        out.say("\n[4/4] New logins (read-only: `id -u` only)")
        after_none = show_probe(out, "no key, no password", ssh.probe(False)) if done else "not tried"
        after_key = show_probe(out, "your key", ssh.probe(True)) if done else "not tried"
        empty_ok = False
        if done:
            # The probes offer no password. Root's password is empty under developer mode, so also make sure
            # the server no longer takes an empty one.
            rc, text = ssh.run("sh -s", SETTINGS_AFTER.encode(), key=False, master=True)
            settings = [line.strip().lower() for line in text.splitlines() if line.strip()]
            out.say("  Akai's SSH server settings now (sshd -T): " + "; ".join(settings))
            empty_ok = "permitemptypasswords yes" in settings and "passwordauthentication yes" in settings
            if empty_ok:
                out.say("  it still takes an empty password, and root's password is empty: not locked")
        if done and after_none == "refused" and after_key == "in" and not empty_ok:
            out.say("\nRESULT:")
            out.say("  LOCKED: the login with no key is refused and your key gets in. Akai's own files are kept in")
            out.say(f"  {BACKUP}; the --devmode-unlock block puts them back. If neither this tool nor your")
            out.say("  key can log in later, these lines in the developer panel's terminal on the MPC undo it:")
            for line in BY_HAND:
                out.say(f"    {line}")
            return 0
        out.say("\n  The lock did not end as it should, so it is undone now over the connection kept open:")
        rc, text = ssh.run("sh -s", scripts()["restore"].encode(), key=False, master=True)
        for line in text.splitlines():
            out.say(f"  {line}")
        back = show_probe(out, "no key, no password, after the undo", ssh.probe(False))
        restored = RESTORED in text.splitlines()
        out.say("\nRESULT:")
        if restored and back == "in":
            out.say("  NOT LOCKED: the lock-down did not work as Akai's banner says, and it was undone; developer")
            out.say("  mode's SSH is as it was (open). Send the report.")
        elif restored:
            out.say("  NOT LOCKED: Akai's files are back, but the login with no key does not get in yet. In the")
            out.say("  developer panel's terminal on the MPC run:  systemctl reload sshd.service   Send the report.")
        else:
            out.say("  NOT LOCKED, AND THE UNDO DID NOT FINISH. In the developer panel's terminal on the MPC run:")
            for line in BY_HAND:
                out.say(f"    {line}")
            out.say("  Send the report.")
        return 1
    finally:
        ssh.close_master()


def unlock(args, out, ssh):
    out.say("\n[1/3] Before (read-only)")
    none = show_probe(out, "no key, no password", ssh.probe(False))
    mine = show_probe(out, "your key", ssh.probe(True))
    if "wrong-host" in (none, mine):
        raise wrong_host(args)
    if mine != "in":
        raise Stop("your key does not get in, so this tool cannot undo the lock. Use the developer panel's "
                   "terminal on the MPC (the lines in this tool's notes). Nothing was changed")
    out.say("\n[2/3] Your go")
    out.say(f"  This puts Akai's allow-access.conf and authorized_keys back from {BACKUP} on /data, removes")
    out.say("  that folder and reloads Akai's SSH server: developer mode's SSH is then open again, as Akai ships it.")
    answer = ask(f"  Type {WORDS['unlock']} and press Enter to undo the lock (anything else stops): ")
    out.say(f"  answer: {answer!r}")
    if answer != WORDS["unlock"]:
        raise Stop("not confirmed; nothing was changed")
    out.say("\n[3/3] Unlock (STATE-CHANGING on /data)")
    rc, text = ssh.run("sh -s", scripts()["restore"].encode(), key=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc == 3:
        raise Stop("the MPC's checks stopped it before any change (see above)")
    back = show_probe(out, "no key, no password, after the undo", ssh.probe(False))
    out.say("\nRESULT:")
    if RESTORED in text.splitlines() and back == "in":
        out.say("  UNLOCKED: Akai's files are back; a login with no key gets in again, as before the lock.")
        return 0
    if RESTORED in text.splitlines():
        out.say("  Akai's files are back, but the login with no key does not get in yet. In the developer panel's")
        out.say("  terminal on the MPC run:  systemctl reload sshd.service   Send the report.")
        return 1
    out.say("  the undo did not end as it should (see above). Send the report.")
    return 1


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("action", choices=("trust", "status", "lock", "unlock"))
    p.add_argument("--ip", required=True)
    p.add_argument("--port", type=int, default=22, help="developer mode's SSH port")
    p.add_argument("--user", default="root")
    p.add_argument("--key", required=True)
    p.add_argument("--known-hosts", default=os.path.expanduser("~/.ssh/known_hosts"))
    p.add_argument("--report")
    args = p.parse_args(argv)
    out = Out(args.report)
    label = {"trust": "READ-ONLY on the MPC; one line in known_hosts after a typed TRUST",
             "status": "READ-ONLY", "lock": "STATE-CHANGING on /data after a typed LOCK",
             "unlock": "STATE-CHANGING on /data after a typed UNLOCK"}[args.action]
    out.say(f"Developer mode login on the MPC: {args.action} ({label})")
    for tool in ("ssh", "ssh-keyscan", "ssh-keygen"):
        if not shutil.which(tool):
            out.say(f"\nSTOP: {tool} is missing on the workstation")
            return 1
    if not os.path.isfile(args.key):
        out.say(f"\nSTOP: missing {args.key}")
        return 1
    with tempfile.TemporaryDirectory(prefix="mpfree-devmode-login-") as tmp:
        ssh = Ssh(args, tmp)
        try:
            return {"trust": trust, "status": status, "lock": lock, "unlock": unlock}[args.action](args, out, ssh)
        except Stop as err:
            out.say(f"\nSTOP: {err}")
            return 1
        except KeyboardInterrupt:
            out.say("\nSTOP: interrupted (Ctrl-C). Run the --devmode-status block to see where the MPC stands.")
            return 130


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except OSError as err:
        print(f"STOP: {err}")
        sys.exit(1)
