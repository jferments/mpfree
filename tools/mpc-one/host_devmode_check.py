#!/usr/bin/env python3
"""Akai's developer mode on the 3.10.1 test build: what it opens, and Akai's own lock-down.

Run it through `collect-over-ssh.sh --devmode-check`, after `--kexec-boot` has
started a test build made with `offline-kexec-stick.py --akai-app`, and after
you have switched developer mode on in Akai's app on the MPC. It talks to the
test build only. On port 2222 (our test SSH) it accepts only the build's own
host key, so stock 3.6 is refused before anything runs. On port 22 (Akai's
SSH) it accepts only the host key it has just read from the test build over
port 2222.

  1. Workstation, read-only: finds the newest ~/mpfree-work/kexec-stick-*
     folder and checks that it was made with --akai-app.
  2. Test build, read-only: that it is the test build (release, no MMC
     device, / read-only from the stick, /data in RAM) and whether Akai's
     app runs; then what developer mode changed: the overlay it mounts on
     /etc, /data and /data/ssh, the SSH server's settings as it reads them
     (`sshd -T`), its host key, the TCP ports that listen, the SSH server
     processes, what kind root's password field is (never its value), the
     drives that are mounted, what Akai's app and its SSH server wrote to
     the journal, blocked program calls, and the result of the autorun.sh
     test (write-autorun-drive.py) if a drive holding it is mounted.
  3. Workstation to port 22, read-only: logs in offering no key and no
     password and runs `id -u` only; then the same with your key.
  4. Only after you type LOCK. STATE-CHANGING, RAM only: Akai's own way to
     lock developer mode's SSH to a key (its login banner and
     allow-access.conf say how): copies
     /data/ssh/sshd_config.d/allow-access.conf and /data/ssh/authorized_keys
     to /run/mpfree-devmode, empties allow-access.conf, adds your public key
     (the build's /etc/mpfree-test/authorized_keys) to
     /data/ssh/authorized_keys and reloads Akai's SSH server
     (`systemctl reload sshd.service`). It first checks again that it is the
     test build and that /data is in RAM. Then step 3 again: the login with
     no key must be refused and your key must work. Anything other than LOCK
     skips it.

Nothing here writes to the eMMC (the test build cannot see it), the U-Boot
environment, the SD card or the stick. Our SSH on port 2222 is a separate
server with its own settings, so it keeps working whatever happens on port 22.
Undo for step 4, and for developer mode itself: switch the MPC off with its
power switch, take out the stick, wait 10 s, switch it on. /data and /run are
RAM on the test build, so none of it survives.
"""
import argparse
import base64
import hashlib
import importlib.util
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("host_kexec_boot", os.path.join(HERE, "host_kexec_boot.py"))
kexec = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(kexec)

Stop = kexec.Stop
LOCK_WORD = "LOCK"
DEVMODE_ALIAS = "mpfree-devmode-test"
HOST_KEY = re.compile(r"^akai host key: (ssh-ed25519 [A-Za-z0-9+/]+={0,3})$", re.M)
# Tests set this so the question can be answered without a terminal. The typed word is still required.
TESTING = os.environ.get("MPFREE_DEVMODE_TEST") == "1"

# sshd -T prints every setting in lower case; these are the ones that decide who gets in.
SSHD_SETTINGS = ("port|listenaddress|permitrootlogin|permitemptypasswords|authenticationmethods|"
                 "passwordauthentication|pubkeyauthentication|kbdinteractiveauthentication|authorizedkeysfile|"
                 "hostkey|usepam")

# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
STATE = r"""R=''
fail=0
say() { printf '%s\n' "$*"; }
bad() { printf 'PROBLEM: %s\n' "$*"; fail=1; }
rel=$(cat "$R/proc/sys/kernel/osrelease")
say "release: $rel"
[ "$rel" = '@@NEW@@' ] || bad "running $rel, not @@NEW@@"
if ls "$R/sys/block" | grep -q '^mmcblk'; then bad "an MMC block device exists"; fi
root=$(readlink -f "$R/dev/disk/by-partuuid/@@P1@@")
want=$(cat "$R/sys/class/block/${root##*/}/dev" 2>/dev/null)
got=""; opts=""
while read -r id parent majmin base mnt mopts rest; do
  if [ "$mnt" = / ]; then got=$majmin; opts=$mopts; fi
done < "$R/proc/self/mountinfo"
[ -n "$want" ] && [ "$got" = "$want" ] || bad "/ is not the stick's partition 1"
case ",$opts," in *,ro,*) ;; *) bad "/ is not read-only" ;; esac
line=$(grep ' /data ' "$R/proc/self/mountinfo" | tail -n 1)
say "/data: ${line#* - }"
case "$line" in *" - tmpfs "*) ;; *) bad "/data is not a tmpfs" ;; esac
say "Akai's app (acvs.service): $(systemctl is-active acvs.service 2>&1)"
say "Akai's SSH server (sshd.service): $(systemctl is-active sshd.service 2>&1)"
say "mounts on /etc:"
grep ' /etc ' "$R/proc/self/mountinfo" | sed 's/^/  /'
if grep ' /etc ' "$R/proc/self/mountinfo" | grep -q 'user-access/etc-overlay'; then
  say "developer mode overlay: mounted"
else
  say "developer mode overlay: not mounted"
fi
say "/data:"
ls -lan "$R/data" 2>&1 | sed 's/^/  /'
say "/data/ssh:"
ls -lanR "$R/data/ssh" 2>&1 | sed 's/^/  /'
say "/data/ssh/sshd_config.d/allow-access.conf:"
sed 's/^/  | /' "$R/data/ssh/sshd_config.d/allow-access.conf" 2>&1
say "keys in /data/ssh/authorized_keys (their type only): $(grep -c '^[^#]' "$R/data/ssh/authorized_keys" 2>/dev/null)"
grep '^[^#]' "$R/data/ssh/authorized_keys" 2>/dev/null | while read -r kind rest; do say "  $kind"; done
f=$(grep '^root:' "$R/etc/shadow" | cut -d: -f2)
case "$f" in
  "") k="empty: no password is needed" ;;
  '!'*|'*'*) k="locked: no password works" ;;
  *) k="a password hash" ;;
esac
f=""
say "root's password field: $k"
say "Akai's SSH server settings as it reads them (sshd -T):"
"$R/usr/sbin/sshd" -T 2>&1 | grep -i -E '^(@@SETTINGS@@) |rror|fatal|no hostkeys' | sed 's/^/  /'
for key in $("$R/usr/sbin/sshd" -T 2>/dev/null | grep -i '^hostkey ' | cut -d' ' -f2); do
  case "$key" in *ed25519*) ;; *) continue ;; esac
  if [ -r "$R$key.pub" ]; then pub=$(cut -d' ' -f1,2 "$R$key.pub"); else pub=$(ssh-keygen -y -f "$R$key" 2>/dev/null | cut -d' ' -f1,2); fi
  [ -n "$pub" ] && say "akai host key: $pub"
done
ports=$(for t in "$R/proc/net/tcp" "$R/proc/net/tcp6"; do
  [ -r "$t" ] || continue
  while read -r sl local remote st rest; do
    [ "$st" = 0A ] && printf '%d\n' "0x${local##*:}"
  done < "$t"
done | sort -n -u | tr '\n' ' ')
say "listening TCP ports: $ports"
say "SSH server processes:"
for p in "$R"/proc/[0-9]*; do
  c=$(tr '\0' ' ' < "$p/cmdline" 2>/dev/null)
  case "$c" in *sshd*) say "  ${p##*/}: $c" ;; esac
done
say "drives mounted:"
grep '^/dev/sd' "$R/proc/mounts" | sed 's/^/  /'
say "autorun.sh test result (from write-autorun-drive.py's script):"
found=0
while read -r dev mnt rest; do
  case "$dev" in /dev/sd*) ;; *) continue ;; esac
  mnt=$(printf '%b' "$mnt")
  r="$R$mnt/mpfree-autorun-result.txt"
  if [ -r "$r" ]; then
    found=1
    say "  $mnt/mpfree-autorun-result.txt:"
    head -c 8192 "$r" | sed 's/^/  | /'
  fi
done < "$R/proc/mounts"
if [ -r "$R/run/mpfree-autorun-result.txt" ]; then
  found=1
  say "  /run/mpfree-autorun-result.txt (the drive was read-only):"
  head -c 8192 "$R/run/mpfree-autorun-result.txt" | sed 's/^/  | /'
fi
[ "$found" = 0 ] && say "  no result file on a mounted drive or in /run"
say "its kernel log lines:"
journalctl -b -k --no-pager -o short-monotonic 2>/dev/null | grep 'mpfree-autorun' | sed 's/^/  /'
say "Akai's app in the journal (developer mode, SSH, scripts, drives):"
journalctl -b -u acvs.service --no-pager -o short-monotonic 2>/dev/null |
  grep -i -E 'develop|user-access|autorun|script|ssh|drive|mount' | tail -n 80 | sed 's/^/  /'
say "Akai's SSH server in the journal:"
journalctl -b -u sshd.service -u sshdgenkeys.service --no-pager -o short-monotonic 2>/dev/null |
  tail -n 60 | sed 's/^/  /'
say "blocked program calls:"
journalctl -b --no-pager 2>/dev/null | grep 'mpfree-test: blocked' | sed 's/^/  /'
[ "$fail" = 0 ] && say CHECKS-OK || say CHECKS-PROBLEM
"""

# The lock-down. Its only writes: two copies into /run/mpfree-devmode, then Akai's two files in /data/ssh,
# and only after it has checked that /data is a tmpfs.
LOCK = r"""R=''
say() { printf '%s\n' "$*"; }
rel=$(cat "$R/proc/sys/kernel/osrelease")
[ "$rel" = '@@NEW@@' ] || { say "STOP: running $rel, not @@NEW@@; nothing was changed"; exit 3; }
line=$(grep ' /data ' "$R/proc/self/mountinfo" | tail -n 1)
case "$line" in *" - tmpfs "*) ;; *) say "STOP: /data is not in RAM here; nothing was changed"; exit 3 ;; esac
d="$R/data/ssh"
conf="$d/sshd_config.d/allow-access.conf"
keys="$d/authorized_keys"
ours="$R/etc/mpfree-test/authorized_keys"
[ -f "$conf" ] || { say "STOP: there is no /data/ssh/sshd_config.d/allow-access.conf (is developer mode on?); nothing was changed"; exit 3; }
grep -q '^ssh-' "$ours" || { say "STOP: no key in /etc/mpfree-test/authorized_keys; nothing was changed"; exit 3; }
umask 077
b="$R/run/mpfree-devmode"
mkdir -p "$b" || { say "STOP: could not make /run/mpfree-devmode; nothing was changed"; exit 3; }
cp "$conf" "$b/allow-access.conf" || { say "STOP: could not copy allow-access.conf; nothing was changed"; exit 3; }
if [ -f "$keys" ]; then
  cp "$keys" "$b/authorized_keys" || { say "STOP: could not copy authorized_keys; nothing was changed"; exit 3; }
fi
say "copied to /run/mpfree-devmode: $(ls "$b" | tr '\n' ' ')"
: > "$conf" || { say "could not empty allow-access.conf"; exit 4; }
say "emptied /data/ssh/sshd_config.d/allow-access.conf"
cat "$ours" >> "$keys" || { say "could not add your key"; exit 4; }
say "added your key to /data/ssh/authorized_keys ($(grep -c '^[^#]' "$keys") key line(s) now)"
systemctl reload sshd.service
rc=$?
say "systemctl reload sshd.service: exit code $rc"
say "Akai's SSH server settings now (sshd -T):"
"$R/usr/sbin/sshd" -T 2>&1 | grep -i -E '^(@@SETTINGS@@) |rror|fatal|no hostkeys' | sed 's/^/  /'
[ "$rc" = 0 ] && say LOCK-DONE || say LOCK-RELOAD-FAILED
"""
JOURNAL = r"""R=''
journalctl -b -u sshd.service --no-pager -o short-monotonic 2>/dev/null | tail -n 12
"""


def ask(prompt):
    if not TESTING:
        if not sys.stdin.isatty():
            return ""
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)  # drop anything typed before the question
    try:
        return input(prompt).strip()
    except EOFError:
        return ""


def fingerprint(pub):
    """OpenSSH's SHA256 fingerprint of a public key line ('ssh-ed25519 AAAA...')."""
    blob = base64.b64decode(pub.split()[1])
    return "SHA256:" + base64.b64encode(hashlib.sha256(blob).digest()).decode().rstrip("=")


def probe(args, known_hosts, key=None):
    """Log in on Akai's port and run `id -u`. Returns (outcome, text): in, refused, closed, wrong-host or none."""
    cmd = ["ssh", "-F", "/dev/null", "-T", "-p", str(args.devmode_port),
           "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o", "StrictHostKeyChecking=yes",
           "-o", f"UserKnownHostsFile={known_hosts}", "-o", "GlobalKnownHostsFile=/dev/null",
           "-o", f"HostKeyAlias={DEVMODE_ALIAS}", "-o", "CheckHostIP=no", "-o", "UpdateHostKeys=no",
           "-o", "PasswordAuthentication=no", "-o", "KbdInteractiveAuthentication=no",
           "-o", "GSSAPIAuthentication=no", "-o", "HostbasedAuthentication=no", "-o", "IdentityAgent=none",
           "-o", "ForwardAgent=no", "-o", "ClearAllForwardings=yes", "-o", "LogLevel=ERROR"]
    if key:
        cmd += ["-o", "PubkeyAuthentication=yes", "-o", "IdentitiesOnly=yes", "-i", key]
    else:
        cmd += ["-o", "PubkeyAuthentication=no"]
    cmd += [f"{args.user}@{args.ip}", "id -u"]
    try:
        res = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, timeout=40)
    except subprocess.TimeoutExpired:
        return "none", "no answer within 40 s"
    text = (res.stdout + res.stderr).decode("utf-8", "replace").strip()
    if res.returncode == 0:
        return "in", text
    if "Host key verification failed" in text or "IDENTIFICATION HAS CHANGED" in text:
        return "wrong-host", text
    if "Permission denied" in text:
        return "refused", text
    if "Connection refused" in text:
        return "closed", text
    return "none", text or f"exit code {res.returncode}"


def show_probe(out, label, result):
    outcome, text = result
    meaning = {"in": "logged in", "refused": "refused", "closed": "nothing listens on that port",
               "wrong-host": "a different host key answered: NOT the test build's SSH server",
               "none": "no login"}[outcome]
    out.say(f"  {label}: {meaning}")
    for line in text.splitlines()[:6]:
        out.say(f"    | {line}")
    if outcome == "in":
        uid = text.splitlines()[-1].strip() if text else "?"
        out.say(f"    id -u printed {uid}" + (" (root)" if uid == "0" else ""))
    return outcome


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", required=True)
    p.add_argument("--port", type=int, default=2222, help="our test SSH port on the test build")
    p.add_argument("--devmode-port", type=int, default=22, help="the port developer mode's SSH server uses")
    p.add_argument("--user", default="root")
    p.add_argument("--key", required=True)
    p.add_argument("--work", required=True)
    p.add_argument("--report")
    args = p.parse_args(argv)
    out = kexec.Out(args.report)
    out.say("Akai's developer mode on the test build (read-only unless you type LOCK; then RAM only)")
    try:
        return check(args, out)
    except Stop as err:
        out.say(f"\nSTOP: {err}")
        return 1
    except KeyboardInterrupt:
        out.say("\nSTOP: interrupted (Ctrl-C). Power-cycle the MPC to undo anything this run changed (RAM only).")
        return 130


def check(args, out):

    out.say("\n[1/4] Workstation checks (read-only)")
    folder = kexec.newest_build(os.path.realpath(args.work))
    out.say(f"  test build: {folder}")
    with open(os.path.join(folder, "report.txt")) as fh:
        report = fh.read()
    if kexec.AKAI_APP_MARK not in report.splitlines():
        raise Stop("the newest test build was not made with --akai-app, so Akai's app (and developer mode) "
                   "is not on it. Build and write one first; nothing was sent to the MPC")
    new = re.search(r"^its kernel release: (\S+)$", report, re.M)
    with open(os.path.join(folder, "cmdline.txt")) as fh:
        p1 = re.search(r"^root=PARTUUID=(\S+)", fh.read())
    if not new or not kexec.RELEASE.match(new.group(1)) or not p1 or not kexec.UUID.match(p1.group(1)):
        raise Stop(f"{folder} does not name the test build's release and partition; nothing was sent to the MPC")
    if not os.path.isfile(args.key) or not os.path.isfile(args.key + ".pub"):
        raise Stop(f"missing {args.key} or {args.key}.pub")
    out.say(f"  made with --akai-app; release {new.group(1)}")
    mpc = kexec.Mpc(args, os.path.join(folder, "known_hosts"))
    values = {"NEW": new.group(1), "P1": p1.group(1), "SETTINGS": SSHD_SETTINGS}

    out.say("\n[2/4] On the test build (read-only; only its own host key is accepted)")
    rc, text = mpc.run(kexec.fill(STATE, **values), test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc == 255 and "CHECKS-" not in text:
        raise Stop("the test build did not answer on port 2222. Start it first with the --kexec-boot block; "
                   "on stock 3.6 this tool runs nothing")
    if text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the test build's safety checks did not pass (PROBLEM lines above); nothing was changed")
    overlay = "developer mode overlay: mounted" in text.splitlines()
    ports = re.search(r"^listening TCP ports: (.*)$", text, re.M)
    listening = ports.group(1).split() if ports else []
    keys = HOST_KEY.findall(text)

    out.say(f"\n[3/4] Logging in on port {args.devmode_port} from here (read-only: runs `id -u` only)")
    port_open = str(args.devmode_port) in listening
    if not overlay:
        out.say("  developer mode's overlay is not mounted")
    if not port_open:
        out.say(f"  nothing listens on port {args.devmode_port} on the test build")
    if not overlay and not port_open:
        raise Stop("developer mode is not on. On the MPC: Menu > Preferences > Enhancements, switch Developer "
                   "Panel on; then hold SHIFT + MENU and choose Developer. Then run this block again. "
                   "Nothing was changed")
    if len(keys) != 1:
        raise Stop("could not read exactly one ed25519 host key of Akai's SSH server over port 2222, so a "
                   f"login on port {args.devmode_port} could not be checked to be the test build. Nothing was "
                   "changed")
    out.say(f"  Akai's SSH host key (read over port 2222): {fingerprint(keys[0])}")
    with tempfile.TemporaryDirectory(prefix="mpfree-devmode-") as tmp:
        known = os.path.join(tmp, "known_hosts")
        with open(known, "w") as fh:
            fh.write(f"{DEVMODE_ALIAS} {keys[0]}\n")
        before_none = show_probe(out, "no key, no password", probe(args, known))
        before_key = show_probe(out, "your key", probe(args, known, args.key))
        if "wrong-host" in (before_none, before_key):
            raise Stop(f"the server on port {args.devmode_port} is not the one the test build reported; "
                       "is another device on this cable? Nothing was changed")

        out.say("\n[4/4] Akai's lock-down (STATE-CHANGING, RAM only; skipped unless you type LOCK)")
        out.say("  It empties /data/ssh/sshd_config.d/allow-access.conf, adds your public key to")
        out.say("  /data/ssh/authorized_keys and reloads Akai's SSH server, as Akai's login banner says to.")
        out.say("  Copies of both files go to /run/mpfree-devmode first. /data is RAM on the test build, so a")
        out.say("  power-cycle undoes it. Our SSH on port 2222 is not affected.")
        locked = None
        if before_none != "in":
            out.say("  skipped: the login with no key did not get in, so there is nothing to lock")
        elif ask(f"  Type {LOCK_WORD} and press Enter to lock it, or just Enter to skip: ") != LOCK_WORD:
            out.say("  skipped (LOCK not typed)")
        else:
            out.say("  answer: LOCK")
            rc, text = mpc.run(kexec.fill(LOCK, **values), test=True)
            for line in text.splitlines():
                out.say(f"  {line}")
            if "LOCK-DONE" not in text.splitlines():
                out.say("  the lock-down did not finish (see the lines above)")
                locked = "failed"
            else:
                after_none = show_probe(out, "no key, no password, after the lock", probe(args, known))
                after_key = show_probe(out, "your key, after the lock", probe(args, known, args.key))
                locked = "works" if (after_none == "refused" and after_key == "in") else "not-as-expected"
                if locked != "works":
                    out.say("  Akai's SSH server in the journal since then:")
                    rc, text = mpc.run(JOURNAL, test=True)
                    for line in text.splitlines():
                        out.say(f"    {line}")

    out.say("\nRESULT:")
    if before_none == "in":
        out.say(f"  developer mode's SSH (port {args.devmode_port}) let a login with no key and no password in: "
                "anyone on this network can get a root shell while it is on.")
    else:
        out.say(f"  a login with no key and no password on port {args.devmode_port}: {before_none}.")
    if locked == "works":
        out.say("  Akai's lock-down works: after it, the login with no key is refused and your key gets in.")
    elif locked:
        out.say(f"  Akai's lock-down: {locked} (see step 4).")
    out.say("NEXT: switch the MPC off with its power switch, take out the stick, wait 10 s, switch it on,")
    out.say("and wait for 3.6. Then run the boot-map block to confirm nothing on the eMMC changed.")
    return 0


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except OSError as err:
        print(f"STOP: {err}")
        sys.exit(1)
