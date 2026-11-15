#!/usr/bin/env python3
"""STATE-CHANGING, RAM ONLY: boot the test build of another firmware version
once on the MPC One by kexec, from the USB stick made by write-test-stick.py.

Run it through `collect-over-ssh.sh --kexec-boot`, which first checks the
MPC's network identity. It uses the newest ~/mpfree-work/kexec-stick-* folder
(the same one write-test-stick.py wrote) and goes step by step:

  1. Workstation, read-only: checks that folder's files against SHA256SUMS,
     that its kexec.dtb hides the eMMC and SD controllers, and its command line.
  2. MPC, read-only: checks the running release, that no kexec kernel is loaded,
     the kernel lockdown state, that LoadPin does not enforce (on the installed
     MPC OS 3.10.1 it does, and the boot hook starts the test build instead),
     that this build's stick is plugged into the MPC,
     and that no shutdown hook or unit on the MPC reacts to kexec. Records the
     eMMC write counters.
  3. MPC RAM: copies zImage and kexec.dtb to /run/mpfree-kexec and checks their
     hashes there. Undo: they are deleted on any later stop, and /run is RAM.
  4. MPC kernel memory: `kexec --load`, then checks kexec_loaded. Undo: kexec --unload.
  5. Asks you to type BOOT 3.10. Anything else unloads the kernel and stops.
  6. `systemctl kexec`: Akai's system shuts down cleanly and jumps to the
     loaded kernel. If the jump fails, systemd reboots normally into it.
  7. Waits for SSH from the test build (only the build's own host key is
     accepted), then runs read-only checks there: the release, that no MMC
     device exists, that kexec is off, that / is the stick, that /data is RAM.
     A build made with --akai-app (its report says "akai app: on") runs Akai's
     app, so acvs.service is expected there and its state is printed; every
     other masked unit must still be inactive.

Nothing here writes to the eMMC, the U-Boot environment or the SD card, and
the test build cannot see either controller. To go back: switch the MPC off
with its power switch, take out the stick, wait 10 s, switch it on. U-Boot
boots Akai's system from the eMMC as before.

Two SSH servers, two ports. Akai's system (3.6.0 with MOD-001 answered on
2222; an installed 3.10.1 answers through developer mode on 22) is reached on
--stock-port with your own known_hosts, which `collect-over-ssh.sh
--devmode-trust` fills for 3.10.1. The test build always answers on --port
(2222) with the host key the stick was made with.
"""
import argparse
import hashlib
import importlib.util
import os
import re
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
BOARD = "inmusic,acva"
ALIAS = "mpfree-kexec-test"
CONFIRM = "BOOT 3.10"
STOCK_PORT = 22    # Akai's system: 3.10.1's developer mode SSH
TEST_PORT = 2222   # the test build's own key-only SSH
BUILD_FILES = ("zImage", "kexec.dtb", "cmdline.txt", "known_hosts")
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
RELEASE = re.compile(r"^[A-Za-z0-9._+-]+$")
CMDLINE_CHARS = re.compile(r"^[A-Za-z0-9=._,:/ -]+$")
NEEDED_ARGS = ("ro", "rootwait=60", "panic=10", "sysctl.kernel.kexec_load_disabled=1")
AKAI_APP_MARK = "akai app: on (--akai-app)"   # offline-kexec-stick.py's line for a build that runs Akai's app
MASKED = ("acvs", "touch-fw-update", "xmos-update", "az0x-webserver", "az01-script-runner", "az0x-data-mkfs",
          "acvs-content-resize", "az01-usbsata-fixer")
# Tests set this so the prompt can be answered without a terminal. The typed answer is still required.
TESTING = os.environ.get("MPFREE_KEXEC_BOOT_TEST") == "1"


class Stop(Exception):
    pass


def interrupt_once(signum, frame):
    """Ctrl-C while the MPC's RAM holds our files: stop once, then ignore further Ctrl-C so the undo runs."""
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    raise KeyboardInterrupt


class Out:
    def __init__(self, path):
        self.fh = open(path, "a") if path else None

    def say(self, text=""):
        print(text, flush=True)
        if self.fh:
            self.fh.write(text + "\n")
            self.fh.flush()


# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
CHECK_STOCK = r"""R=''
set -u
fail=0
say() { printf '%s\n' "$*"; }
bad() { printf 'PROBLEM: %s\n' "$*"; fail=1; }
rel=$(cat "$R/proc/sys/kernel/osrelease")
say "running release: $rel"
[ "$rel" = '@@RUNNING@@' ] || bad "the MPC runs $rel, not @@RUNNING@@"
v=$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)
say "kexec_loaded: $v"
[ "$v" = 0 ] || bad "a kexec kernel is already loaded (or kexec_loaded is unreadable)"
v=$(cat "$R/proc/sys/kernel/kexec_load_disabled" 2>/dev/null)
say "kexec_load_disabled: $v"
[ "$v" = 0 ] || bad "kexec is disabled on the running system"
[ -x "$R/usr/sbin/kexec" ] || bad "no /usr/sbin/kexec on the MPC"
if [ -r "$R/sys/kernel/security/lockdown" ]; then
  v=$(cat "$R/sys/kernel/security/lockdown")
  say "lockdown: $v"
  case "$v" in *"[none]"*) ;; *) bad "kernel lockdown is on, so kexec --load would be refused" ;; esac
else
  say "lockdown: not readable here (no lockdown file; a refused load would only print an error)"
fi
if [ -r "$R/proc/sys/kernel/loadpin/enforce" ]; then
  v=$(cat "$R/proc/sys/kernel/loadpin/enforce")
  say "LoadPin enforce: $v"
  [ "$v" = 0 ] || bad "LoadPin enforces, so the kernel refuses kexec --load (as on the installed MPC OS 3.10.1). There the boot hook starts the test build: docs/dev/hardware/stick-autoboot.md"
else
  say "LoadPin: no enforce file (not active here)"
fi
link="$R/dev/disk/by-partuuid/@@P1@@"
if [ -e "$link" ]; then
  dev=$(readlink -f "$link")
  name=${dev##*/}
  disk=${name%1}
  say "test stick: partition 1 is /dev/$name"
  case "$name" in sd[a-z]1) ;; *) bad "partition 1 of the stick is $name, not a USB disk partition" ;; esac
  case "$(readlink -f "$R/sys/class/block/$name")" in
    */usb*) say "  on USB: yes" ;;
    *) bad "/dev/$name is not on USB" ;;
  esac
  if grep -q "^/dev/$disk" "$R/proc/mounts"; then
    say "  mounted by the running system (it unmounts them when it shuts down):"
    grep "^/dev/$disk" "$R/proc/mounts" | while read -r d m t o rest; do say "    $d on $m ($t, $o)"; done
  else
    say "  not mounted"
  fi
else
  bad "this build's USB stick (partition 1 PARTUUID @@P1@@) is not plugged into the MPC"
fi
avail=$(df -k "$R/run" | tail -n 1 | while read -r a b c d rest; do echo "$d"; done)
say "free in /run: $avail KiB"
[ "${avail:-0}" -ge 65536 ] 2>/dev/null || bad "less than 64 MiB free in /run"
say "units and hooks that mention kexec (systemd's own kexec.target and systemd-kexec.service are expected):"
found=0
for d in /etc/systemd/system /run/systemd/system /usr/lib/systemd/system /lib/systemd/system; do
  [ -d "$R$d" ] || continue
  for f in $(find "$R$d" 2>/dev/null); do
    [ -f "$f" ] || [ -L "$f" ] || continue
    case "$f" in */kexec.target|*/systemd-kexec.service) continue ;; esac
    case "$f" in
      *kexec.target.wants/*|*kexec.target.d/*) say "  $f"; found=1; continue ;;
    esac
    [ -f "$f" ] && grep -qi kexec "$f" 2>/dev/null || continue
    # systemd's own fsck unit (255.17 and later) stops a running fsck at any reboot, kexec or power-off.
    other=$(grep -i kexec "$f" | while IFS= read -r l; do
      [ "$l" = 'Conflicts=reboot.target kexec.target poweroff.target halt.target' ] || printf '%s\n' "$l"
    done)
    if [ -z "$other" ]; then
      say "  $f: only systemd's own line 'Conflicts=reboot.target kexec.target poweroff.target halt.target'"
      say "    (it stops a running file system check at any reboot, kexec or power-off; nothing else)"
    else
      say "  $f:"
      grep -n -i kexec "$f" | while IFS= read -r l; do say "      $l"; done
      found=1
    fi
  done
done
[ "$found" = 0 ] && say "  no other unit reacts to kexec"
[ "$found" = 0 ] || bad "a unit other than systemd's own reacts to kexec: review it before booting"
say "shutdown hooks (they run on every reboot too):"
hooks=0
for d in /etc/systemd/system-shutdown /usr/lib/systemd/system-shutdown /lib/systemd/system-shutdown; do
  [ -d "$R$d" ] || continue
  for f in $(find "$R$d" 2>/dev/null); do
    [ -f "$f" ] || continue
    hooks=1
    say "  $f"
    if grep -qi kexec "$f" 2>/dev/null; then bad "shutdown hook $f mentions kexec: review it before booting"; fi
  done
done
[ "$hooks" = 0 ] && say "  none"
say "eMMC sectors written since boot (field 7), for the record:"
for p in 1 2 3 4 5 6; do
  s=$(cat "$R/sys/class/block/mmcblk0p$p/stat" 2>/dev/null)
  set -- $s
  say "  mmcblk0p$p: ${7:-?}"
done
[ "$fail" = 0 ] && say CHECKS-OK || say CHECKS-STOP
"""

# The copy writes only into /run/mpfree-kexec (RAM).
PREPARE = r"""R=''
umask 077
d="$R/run/mpfree-kexec"
if [ -d "$d" ]; then rm -f "$d/zImage" "$d/kexec.dtb" && rmdir "$d" || exit 1; fi
mkdir "$d"
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-kexec/@@NAME@@" """
HASHES = r"""R=''
sha256sum "$R/run/mpfree-kexec/zImage" "$R/run/mpfree-kexec/kexec.dtb"
"""
LOAD = r"""R=''
"$R/usr/sbin/kexec" --load --kexec-syscall --dtb="$R/run/mpfree-kexec/kexec.dtb" \
  --command-line='@@CMDLINE@@' "$R/run/mpfree-kexec/zImage"
rc=$?
echo "kexec --load exit code: $rc"
echo "kexec_loaded: $(cat "$R/sys/kernel/kexec_loaded")"
"""
UNDO = r"""R=''
if [ "$(cat "$R/sys/kernel/kexec_loaded")" != 0 ]; then "$R/usr/sbin/kexec" --unload --kexec-syscall; fi
rm -f "$R/run/mpfree-kexec/zImage" "$R/run/mpfree-kexec/kexec.dtb"
[ -d "$R/run/mpfree-kexec" ] && rmdir "$R/run/mpfree-kexec"
echo "kexec_loaded: $(cat "$R/sys/kernel/kexec_loaded")"
"""
JUMP = r"""R=''
echo "starting the jump"
systemctl kexec
"""
STATE = r"""R=''
echo "uptime: $(cat "$R/proc/uptime")"
echo "state: $(systemctl is-system-running 2>&1)"
"""
CHECK_310 = r"""R=''
fail=0
say() { printf '%s\n' "$*"; }
bad() { printf 'PROBLEM: %s\n' "$*"; fail=1; }
rel=$(cat "$R/proc/sys/kernel/osrelease")
say "release: $rel"
[ "$rel" = '@@NEW@@' ] || bad "running $rel, not @@NEW@@"
say "command line: $(cat "$R/proc/cmdline")"
say "block devices: $(ls "$R/sys/block" | tr '\n' ' ')"
if ls "$R/sys/block" | grep -q '^mmcblk'; then bad "an MMC block device exists"; fi
for dev in "$R"/sys/bus/platform/devices/ff0f0000.* "$R"/sys/bus/platform/devices/ff0c0000.*; do
  [ -e "$dev" ] || continue
  if [ -e "$dev/driver" ]; then bad "${dev##*/} has a driver bound"; else say "${dev##*/}: present, no driver"; fi
done
v=$(cat "$R/proc/sys/kernel/kexec_load_disabled")
say "kexec_load_disabled: $v"
[ "$v" = 1 ] || bad "kexec is not disabled"
root=$(readlink -f "$R/dev/disk/by-partuuid/@@P1@@")
want=$(cat "$R/sys/class/block/${root##*/}/dev" 2>/dev/null)
say "stick partition 1: ${root##*/} ($want)"
got=""; opts=""
while read -r id parent majmin base mnt mopts rest; do
  if [ "$mnt" = / ]; then got=$majmin; opts=$mopts; fi
done < "$R/proc/self/mountinfo"
say "/ is device $got, options $opts"
[ -n "$want" ] && [ "$got" = "$want" ] || bad "/ is not the stick's partition 1"
case ",$opts," in *,ro,*) ;; *) bad "/ is not read-only" ;; esac
line=$(grep ' /data ' "$R/proc/self/mountinfo" | tail -n 1)
say "/data: ${line#* - }"
case "$line" in *" - tmpfs "*) ;; *) bad "/data is not a tmpfs" ;; esac
say "system state: $(systemctl is-system-running 2>&1)"
say "failed units:"
systemctl --failed --no-legend --plain 2>&1 | sed 's/^/  /'
@@AKAI@@
say "masked Akai units that are active (there should be none):"
for u in @@MASKED@@; do
  s=$(systemctl is-active "$u.service" 2>/dev/null)
  if [ "$s" = active ]; then bad "$u.service is active"; fi
done
say "blocked program calls:"
journalctl -b --no-pager 2>/dev/null | grep 'mpfree-test: blocked' | sed 's/^/  /'
say "MMC messages from the kernel:"
dmesg 2>/dev/null | grep -i -e mmc -e dwmmc | sed 's/^/  /'
say "network:"
ip -4 addr show 2>&1 | grep -e inet -e '^[0-9]' | sed 's/^/  /'
[ "$fail" = 0 ] && say CHECKS-OK || say CHECKS-PROBLEM
"""


def fill(script, **values):
    for key, value in values.items():
        script = script.replace(f"@@{key}@@", value)
    if "@@" in script:
        raise Stop("internal error: a remote script has an unfilled value")
    return script


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def newest_build(work):
    """The folder write-test-stick.py writes: the newest kexec-stick-* with a stick.img."""
    folders = sorted(d for d in os.listdir(work) if d.startswith("kexec-stick-")) if os.path.isdir(work) else []
    folders = [d for d in folders if os.path.isfile(os.path.join(work, d, "stick.img"))]
    if not folders:
        raise Stop(f"no kexec-stick-* folder with a stick.img under {work}")
    return os.path.join(work, folders[-1])


def check_build(folder, out):
    sums = {}
    with open(os.path.join(folder, "SHA256SUMS")) as fh:
        for line in fh:
            parts = line.split()
            if len(parts) == 2:
                sums[parts[1]] = parts[0]
    for name in BUILD_FILES:
        path = os.path.join(folder, name)
        if not os.path.isfile(path):
            raise Stop(f"{path} is missing")
        if sums.get(name) != sha256(path):
            raise Stop(f"{path} does not match SHA256SUMS")
    out.say(f"  {', '.join(BUILD_FILES)} match SHA256SUMS")
    with open(os.path.join(folder, "report.txt")) as fh:
        report = fh.read()
    new = re.search(r"^its kernel release: (\S+)$", report, re.M)
    running = re.search(r"^running release: (\S+)", report, re.M)
    if not new or not running or not RELEASE.match(new.group(1)) or not RELEASE.match(running.group(1)):
        raise Stop(f"{folder}/report.txt does not name both kernel releases")
    with open(os.path.join(folder, "cmdline.txt")) as fh:
        cmdline = fh.read().strip()
    if not CMDLINE_CHARS.match(cmdline):
        raise Stop("cmdline.txt has unexpected characters")
    args = cmdline.split()
    roots = [a[len("root=PARTUUID="):] for a in args if a.startswith("root=PARTUUID=")]
    missing = [a for a in NEEDED_ARGS if a not in args]
    if len(roots) != 1 or not UUID.match(roots[0]) or missing:
        raise Stop(f"cmdline.txt is not the builder's command line ({cmdline})")
    out.say(f"  command line: {cmdline}")
    spec = importlib.util.spec_from_file_location("offline_dtb", os.path.join(HERE, "offline-dtb.py"))
    dtb = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(dtb)
    tree = dtb.load(os.path.join(folder, "kexec.dtb"))
    reasons = dtb.verify_kexec_dtb(tree, BOARD)
    chosen = tree.find("/chosen")
    if chosen is not None and chosen.prop("inmusic,acvs-emmc-content") is not None:
        reasons.append("it still has /chosen/inmusic,acvs-emmc-content")
    if reasons:
        raise Stop("kexec.dtb must not be used: " + "; ".join(reasons))
    out.say("  kexec.dtb: for inmusic,acva, eMMC and SD card controllers disabled")
    akai = AKAI_APP_MARK in report.splitlines()
    if akai:
        out.say("  this build runs Akai's app (made with --akai-app); every flashing program in it is a stub")
    return {"new": new.group(1), "running": running.group(1), "cmdline": cmdline, "p1": roots[0], "akai": akai}


def check_310(build):
    """The test build's read-only check, for a build with or without Akai's app."""
    akai = build.get("akai", False)
    masked = [u for u in MASKED if not (akai and u == "acvs")]
    line = ('say "Akai\'s app (acvs.service, expected in this build): $(systemctl is-active acvs.service 2>&1)"'
            if akai else ":")
    return fill(CHECK_310, NEW=build["new"], P1=build["p1"], MASKED=" ".join(masked), AKAI=line)


class Mpc:
    def __init__(self, args, known_hosts):
        self.args = args
        base = ["-T", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=yes",
                "-o", "ForwardAgent=no", "-o", "ClearAllForwardings=yes", "-i", args.key]
        # Akai's system and the test build listen on different ports (see the module's notes).
        stock_port = getattr(args, "stock_port", None) or STOCK_PORT
        self.stock = ["ssh", *base, "-p", str(stock_port), "-o", "ConnectTimeout=8"]
        self.test = ["ssh", *base, "-p", str(args.port), "-o", "ConnectTimeout=5",
                     "-o", f"UserKnownHostsFile={known_hosts}",
                     "-o", "GlobalKnownHostsFile=/dev/null", "-o", f"HostKeyAlias={ALIAS}",
                     "-o", "CheckHostIP=no", "-o", "UpdateHostKeys=no"]
        self.target = f"{args.user}@{args.ip}"

    def run(self, script, test=False, stdin_path=None, command="sh -s", timeout=120):
        cmd = (self.test if test else self.stock) + [self.target, command]
        try:
            if stdin_path:
                with open(stdin_path, "rb") as fh:
                    res = subprocess.run(cmd, stdin=fh, capture_output=True, timeout=timeout)
            else:
                res = subprocess.run(cmd, input=script.encode(), capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return 124, ""
        return res.returncode, (res.stdout + res.stderr).decode("utf-8", "replace")


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


def undo(mpc, out):
    rc, text = mpc.run(UNDO)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc != 0 or not text.strip().endswith("kexec_loaded: 0"):
        out.say("  WARNING: could not confirm the unload. Nothing has jumped. Do not reboot from the MPC's menu or")
        out.say("  over SSH (systemd would start the loaded kernel). Switch the MPC off with its power switch,")
        out.say("  take out the stick, wait 10 s and switch it on: that clears it.")
    else:
        out.say("  unloaded; /run/mpfree-kexec removed. The MPC is as before.")


def arp_mac(path, ip, iface):
    try:
        with open(path) as fh:
            for line in fh.readlines()[1:]:
                f = line.split()
                if len(f) >= 6 and f[0] == ip and f[5] == iface and f[2] == "0x2":
                    return f[3].lower()
    except OSError:
        pass
    return None


def wait_for_test_build(mpc, out, args):
    start = time.time()
    last_note = start
    while time.time() - start < args.wait:
        rc, text = mpc.run("echo mpfree-test-build-up", test=True, command="echo mpfree-test-build-up", timeout=20)
        if rc == 0 and "mpfree-test-build-up" in text:
            out.say(f"  the test build answered after {int(time.time() - start)} s")
            return "up"
        elapsed = time.time() - start
        rc, text = mpc.run(STATE, timeout=20)
        found = re.search(r"^uptime: ([0-9.]+)", text, re.M)
        state = re.search(r"^state: (\S+)", text, re.M)
        if rc == 0 and found:
            uptime = float(found.group(1))
            if uptime < elapsed + 30:
                out.say(f"  Akai's system is running again (up {int(uptime)} s): the MPC was switched off and on, "
                        "the jump failed,"
                        " or the test build restarted.")
                return "stock-back"
            if state and state.group(1) in ("running", "degraded") and elapsed > args.still_up_after:
                out.say(f"  Akai's system is still {state.group(1)} (up {int(uptime)} s): the jump never started.")
                return "stock-still-up"
        if time.time() - last_note >= 15:
            out.say(f"  waiting ... {int(elapsed)} s")
            last_note = time.time()
        time.sleep(args.poll)
    return "nothing"


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", required=True)
    p.add_argument("--port", type=int, default=TEST_PORT, help="the test build's SSH port")
    p.add_argument("--stock-port", type=int, default=STOCK_PORT,
                   help="Akai's system's SSH port: 22 (3.10.1 developer mode), or 2222 for 3.6.0 with MOD-001")
    p.add_argument("--user", default="root")
    p.add_argument("--key", required=True)
    p.add_argument("--iface", required=True)
    p.add_argument("--expect-mac", required=True)
    p.add_argument("--work", required=True)
    p.add_argument("--report")
    p.add_argument("--wait", type=float, default=300)
    p.add_argument("--poll", type=float, default=5)
    p.add_argument("--still-up-after", type=float, default=60,
                   help="seconds after which Akai's system still running means the jump never started")
    p.add_argument("--arp", default="/proc/net/arp")
    args = p.parse_args(argv)
    out = Out(args.report)
    out.say("MPFree kexec test boot (changes RAM on the MPC only)")

    out.say("\n[1/7] Workstation checks (read-only)")
    folder = newest_build(os.path.realpath(args.work))
    out.say(f"  build: {folder}")
    build = check_build(folder, out)
    mpc = Mpc(args, os.path.join(folder, "known_hosts"))

    out.say("\n[2/7] Checks on the running MPC (read-only)")
    rc, text = mpc.run(fill(CHECK_STOCK, RUNNING=build["running"], P1=build["p1"]))
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc != 0 or text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the MPC is not ready; nothing was changed on it")

    out.say("\n[3/7] Copy the kernel and device tree into the MPC's RAM (/run/mpfree-kexec)")
    loaded = False
    signal.signal(signal.SIGINT, interrupt_once)
    try:
        rc, text = mpc.run(PREPARE)
        if rc != 0:
            raise Stop(f"could not make /run/mpfree-kexec ({text.strip()})")
        for name in ("zImage", "kexec.dtb"):
            rc, text = mpc.run(None, stdin_path=os.path.join(folder, name), command=fill(RECEIVE, NAME=name))
            if rc != 0:
                raise Stop(f"copying {name} failed ({text.strip()})")
        rc, text = mpc.run(HASHES)
        got = {}
        for line in text.splitlines():
            if len(line.split()) == 2:
                got[line.split()[1].rsplit("/", 1)[-1]] = line.split()[0]
        for name in ("zImage", "kexec.dtb"):
            if got.get(name) != sha256(os.path.join(folder, name)):
                raise Stop(f"{name} on the MPC does not match the build")
        out.say("  zImage and kexec.dtb copied; their hashes on the MPC match the build")

        out.say("\n[4/7] Load the test kernel into memory (kexec --load; undo: kexec --unload)")
        loaded = True
        rc, text = mpc.run(fill(LOAD, CMDLINE=build["cmdline"]))
        for line in text.splitlines():
            out.say(f"  {line}")
        if rc != 0 or "kexec --load exit code: 0" not in text or not text.strip().endswith("kexec_loaded: 1"):
            raise Stop("kexec --load did not load the kernel")

        out.say("\n[5/7] Your go")
        out.say("  Typing BOOT 3.10 shuts Akai's system down and starts the test build from the stick.")
        if build.get("akai"):
            out.say("  The screen should show Akai's app starting within about a minute of the jump (this build runs it).")
        else:
            out.say("  The screen should show the test build's log within about 10 s of the jump (it has no MPC app).")
        out.say("  If it stays dark or frozen, keep waiting here: the stick still keeps the logs.")
        out.say(f"  Wait up to {int(args.wait)} s for it here. To go back at any time: switch the MPC off,")
        out.say("  take out the stick, wait 10 s, switch it on.")
        answer = ask(f"  Type {CONFIRM} and press Enter to go on (anything else unloads it and stops): ")
        out.say(f"  answer: {answer!r}")
        if answer != CONFIRM:
            raise Stop("not confirmed")
    except (Stop, KeyboardInterrupt) as err:
        signal.signal(signal.SIGINT, signal.SIG_IGN)  # no Ctrl-C may cut the undo short
        out.say(f"\nSTOP: {str(err) or 'interrupted (Ctrl-C)'}")
        out.say("Undoing the changes in RAM:" if loaded else "Removing the copied files:")
        undo(mpc, out)
        return 1
    signal.signal(signal.SIGINT, signal.default_int_handler)
    try:
        return jump_and_check(mpc, out, args, build)
    except KeyboardInterrupt:
        out.say("\nInterrupted after the jump was asked for. Switch the MPC off with its power switch, take out")
        out.say("the stick, wait 10 s and switch it on. Then run the boot-map block.")
        return 1


def jump_and_check(mpc, out, args, build):
    out.say("\n[6/7] Jump (systemctl kexec)")
    rc, text = mpc.run(JUMP, timeout=60)
    for line in text.splitlines():
        out.say(f"  {line}")
    out.say(f"  systemctl kexec returned {rc} (the connection may drop as Akai's system shuts down)")

    out.say(f"\n[7/7] Wait for the test build (up to {int(args.wait)} s), then check it (read-only)")
    state = wait_for_test_build(mpc, out, args)
    if state == "stock-still-up":
        out.say("Undoing the changes in RAM:")
        undo(mpc, out)
        out.say("\nRESULT: the jump did not start. Akai's system is running as before. Send the zip.")
        return 1
    if state == "stock-back":
        out.say("\nRESULT: the MPC is on Akai's system again. If you did not switch it off, the jump failed or the test")
        out.say("build restarted by itself. Either way there is nothing to switch off now.")
        out.say("Take out the stick, then send the zip; the stick's log partition says what happened.")
        return 1
    if state == "nothing":
        out.say("\nRESULT: nothing answered. Switch the MPC off with its power switch, take out the stick,")
        out.say("wait 10 s and switch it on. Then send the zip; the stick's log partition says what happened.")
        return 1
    mac = arp_mac(args.arp, args.ip, args.iface)
    note = "" if mac == args.expect_mac.lower() else (" (not Akai's system's; the host key still proves it is "
                                                       "the test build)")
    out.say(f"  network identity: {mac or 'not in the ARP table'}{note}")
    rc, text = mpc.run(check_310(build), test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    ok = rc == 0 and text.strip().splitlines()[-1:] == ["CHECKS-OK"]
    if ok:
        out.say("\nRESULT: the test build is running from the stick, and it cannot see the eMMC or SD card.")
    else:
        out.say("\nRESULT: the test build is running, but some checks failed (see PROBLEM lines).")
    out.say("NEXT: leave it running and do this round's steps on the test build (such as the --app block).")
    out.say("When they are done, switch the MPC off with its power switch, take out the stick, wait 10 s,")
    out.say("switch it on and wait for Akai's system. Then run the boot-map block to confirm nothing on the eMMC "
            "changed.")
    return 0 if ok else 1


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:  # only reaches here in steps 1 and 2, before anything on the MPC changes
        print("\nSTOP: interrupted (Ctrl-C). Nothing on the MPC was changed.")
        sys.exit(130)
