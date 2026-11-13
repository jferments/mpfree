#!/usr/bin/env python3
"""WORKSTATION ONLY: put a harmless autorun.sh on a spare USB drive, for the
autorun test in phase 3 of the 3.10.1 stick test.

In developer mode, Akai's app offers to run an autorun.sh from a drive that
is plugged in (it asks on the screen first). This tool writes the script that
shows when it runs, as which user, from where and in which service. The
script itself (AUTORUN below) only writes one line to the MPC's kernel log
and one file, mpfree-autorun-result.txt, next to itself on the drive (or in
/run, the MPC's RAM, if the drive is read-only). It changes nothing else.

It never contacts the MPC and never asks you to type a device name: it looks
for the one partition that is
  - on a USB disk (/dev/sd*), mounted on this workstation, FAT32 or exFAT,
  - not on the MPFree test stick (no partition named mpfree-test-*),
and it stops if there is none or more than one. It shows the drive and adds
the files only after you type WRITE. It writes autorun.sh (and removes an
mpfree-autorun-result.txt from an earlier run of this script); it stops
without writing if the drive holds an autorun.sh or result file that this
tool did not write. It then reads autorun.sh back and compares it.

Undo: delete autorun.sh (and mpfree-autorun-result.txt) from the drive.

Use: plug in one spare FAT32 or exFAT USB drive and let the desktop mount it,
then
  python3 tools/mpc-one/write-autorun-drive.py
Eject the drive, and plug it into the MPC only while the 3.10.1 test build
runs with developer mode on (phase 3 of
docs/dev/hardware/firmware-3.10.1-stick-test.md).
"""
import os
import re
import sys

TEST_ROOT = os.environ.get("MPFREE_TEST_ROOT", "")
NAME = re.compile(r"^sd[a-z]+[0-9]+$")
FILESYSTEMS = ("vfat", "exfat")
SCRIPT = "autorun.sh"
RESULT = "mpfree-autorun-result.txt"
MARK = "# MPFree autorun test (write-autorun-drive.py)"
RESULT_MARK = "mpfree-autorun result"
CONFIRM = "WRITE"

AUTORUN = f"""#!/bin/sh
{MARK}
# Phase 3 of the 3.10.1 stick test. Harmless: it writes one line to the kernel
# log and one file, {RESULT}, next to itself (or in /run if the drive is
# read-only), and changes nothing else.
here=$(cd "$(dirname "$0")" 2>/dev/null && pwd)
out="$here/{RESULT}"
( : > "$out" ) 2>/dev/null || out=/run/{RESULT}
{{
  echo "{RESULT_MARK}"
  echo "time (UTC): $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uptime: $(cut -d' ' -f1 /proc/uptime) s"
  echo "release: $(cat /proc/sys/kernel/osrelease)"
  echo "id: $(id)"
  echo "script: $0"
  echo "arguments: $*"
  echo "working folder: $(pwd)"
  echo "parent: $(tr '\\0' ' ' < /proc/$PPID/cmdline | head -c 300)"
  echo "cgroup: $(cat /proc/self/cgroup)"
  echo "PATH: $PATH"
  echo "drives:"
  grep '^/dev/sd' /proc/mounts
}} > "$out" 2>&1
echo "mpfree-autorun: ran as $(id -u) from $0, result in $out" > /dev/kmsg 2>/dev/null
exit 0
"""


class Stop(Exception):
    pass


def p(path):
    return TEST_ROOT + path


def read(path, default=""):
    try:
        with open(p(path)) as fh:
            return fh.read().strip()
    except OSError:
        return default


def unescape(field):
    """Mount points in mountinfo write a space as \\040 (and tab, newline, backslash likewise)."""
    return re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), field)


def partname(node):
    for line in read(f"/sys/class/block/{node}/uevent").splitlines():
        if line.startswith("PARTNAME="):
            return line.split("=", 1)[1]
    return ""


def drives():
    """Every mounted /dev/sd* partition: (facts, reasons it is not used)."""
    found = []
    with open(p("/proc/self/mountinfo")) as fh:
        lines = fh.read().splitlines()
    for line in lines:
        left, _, right = line.partition(" - ")
        left, right = left.split(), right.split()
        if len(left) < 5 or len(right) < 2 or not right[1].startswith("/dev/"):
            continue
        name = right[1][len("/dev/"):]
        if not NAME.match(name):
            continue
        disk = re.sub(r"[0-9]+$", "", name)
        base = f"/sys/block/{disk}"
        info = {"name": name, "fs": right[0], "mount": unescape(left[4]),
                "vendor": read(base + "/device/vendor", "?"), "model": read(base + "/device/model", "?"),
                "size": int(read(base + "/size", "0") or 0) * 512}
        reasons = []
        if "/usb" not in os.path.realpath(p(base)):
            reasons.append("not on USB")
        if info["fs"] not in FILESYSTEMS:
            reasons.append(f"{info['fs']}, not FAT32 or exFAT")
        parts = sorted(d for d in os.listdir(p(base)) if d.startswith(disk)) if os.path.isdir(p(base)) else []
        if any(partname(d).startswith("mpfree-test") for d in parts):
            reasons.append("the MPFree test stick")
        if not os.path.isdir(p(info["mount"])):
            reasons.append("its folder is not there")
        if any(f["name"] == name for f, _ in found):
            continue  # mounted twice: list it once
        found.append((info, reasons))
    return found


def own_file(path, mark):
    with open(path, errors="replace") as fh:
        return mark in fh.read(4096).splitlines()[:2]


def ask(prompt):
    if not TEST_ROOT:
        if not sys.stdin.isatty():
            raise Stop("run this from a terminal: it asks you to confirm")
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)  # drop anything pasted after the command
    try:
        return input(prompt).strip()
    except EOFError:
        return ""


def main():
    print("Harmless autorun.sh for the 3.10.1 developer mode test (workstation only, no MPC contact)")
    print("Mounted USB drives seen:")
    candidates = []
    found = drives()
    for info, reasons in found:
        print(f"  /dev/{info['name']} on {info['mount']}: {info['vendor']} {info['model']}, {info['fs']}, "
              f"{info['size'] / 1e9:.1f} GB")
        print("      " + ("CAN be used" if not reasons else "not used: " + "; ".join(reasons)))
        if not reasons:
            candidates.append(info)
    if not found:
        print("  (none)")
    if not candidates:
        raise Stop("no spare drive to use: plug in one FAT32 or exFAT USB drive and let the desktop mount it")
    if len(candidates) > 1:
        raise Stop("more than one drive could be used: unplug all but the spare one, then run this again")
    info = candidates[0]
    folder = p(info["mount"])
    script, result = os.path.join(folder, SCRIPT), os.path.join(folder, RESULT)
    for path, mark, what in ((script, MARK, SCRIPT), (result, RESULT_MARK, RESULT)):
        if os.path.lexists(path) and not (os.path.isfile(path) and own_file(path, mark)):
            raise Stop(f"{info['mount']} already holds {what}, which this tool did not write. Use another drive, "
                       "or move that file off this one first. Nothing was written")
    print(f"\nIt adds {SCRIPT} to {info['mount']} (/dev/{info['name']})"
          + (f" and removes {RESULT} from an earlier run." if os.path.exists(result) else "."))
    print("The script only writes one kernel log line on the MPC and its result file; nothing else changes.")
    if ask(f"Type {CONFIRM} and press Enter to add it: ") != CONFIRM:
        raise Stop("not confirmed; nothing was written")
    if os.path.exists(result):
        os.unlink(result)
        print(f"removed {RESULT} from an earlier run")
    temp = script + ".mpfree-new"
    with open(temp, "w", newline="\n") as fh:
        fh.write(AUTORUN)
        fh.flush()
        os.fsync(fh.fileno())
    os.replace(temp, script)
    os.sync()
    with open(script, newline="") as fh:
        if fh.read() != AUTORUN:
            raise Stop(f"{SCRIPT} on the drive does not read back as written; delete it and run this again")
    print(f"wrote {info['mount']}/{SCRIPT} ({len(AUTORUN)} bytes) and read it back: it matches")
    print("\nNEXT: eject the drive (in the file manager). Plug it into the MPC only while the 3.10.1 test build")
    print("runs with developer mode on, and answer the MPC's question about the autorun script.")
    print(f"Undo: delete {SCRIPT} (and {RESULT}) from the drive.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:
        print("\nSTOP: interrupted (Ctrl-C).")
        sys.exit(130)
