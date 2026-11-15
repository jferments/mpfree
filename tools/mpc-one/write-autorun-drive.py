#!/usr/bin/env python3
"""WORKSTATION ONLY: put a harmless autorun.sh on a spare USB drive, for the
autorun test in phase 3 of the 3.10.1 stick test.

In developer mode, Akai's app offers to run an autorun.sh from a drive that
is plugged in (it asks on the screen first). This tool writes the script that
shows when it runs, as which user, from where and in which service. The
script itself (AUTORUN below) only writes one line to the MPC's kernel log
and one file, mpfree-autorun-result.txt, next to itself on the drive (or in
/run, the MPC's RAM, if the drive is read-only). It changes nothing else.

It never contacts the MPC, never formats or mounts anything and never asks you
to type a device name: it looks for the one filesystem that is
  - on a USB disk (/dev/sd*, a partition or the whole disk), mounted on this
    workstation, FAT32 or exFAT,
  - not on the MPFree test stick (no partition named mpfree-test-*),
and it stops if there is none or more than one. When there is none, it lists
the USB disks it sees with nothing mounted and what is on them (from udev's
records), and says what to do: open a FAT32 stick in the file manager so it
is mounted, or format a blank stick as FAT first. It shows the drive and adds
the files only after you type WRITE. It writes autorun.sh (and removes an
mpfree-autorun-result.txt from an earlier run of this script); it stops
without writing if the drive holds an autorun.sh or result file that this
tool did not write. It then reads autorun.sh back and compares it.

Undo: delete autorun.sh (and mpfree-autorun-result.txt) from the drive.

Use: plug in one spare FAT32 or exFAT USB drive and let the desktop mount it,
then
  python3 tools/mpc-one/write-autorun-drive.py
Eject the drive, and plug it into the MPC only while the 3.10.1 test build
runs with developer mode on. The MPC has one USB-A port and the test stick
must stay in it (the test build runs from it), so this needs a USB hub that
holds the test stick from power-on (phase 3 of
docs/dev/hardware/firmware-3.10.1-stick-test.md).
"""
import os
import re
import sys

TEST_ROOT = os.environ.get("MPFREE_TEST_ROOT", "")
NAME = re.compile(r"^sd[a-z]+[0-9]*$")
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


def udev_props(node):
    """udev's record of a block device (ID_FS_TYPE, ID_FS_LABEL, ...), read without root."""
    props = {}
    dev = read(f"/sys/class/block/{node}/dev")
    for line in read(f"/run/udev/data/b{dev}").splitlines() if dev else []:
        if line.startswith("E:") and "=" in line:
            key, _, value = line[2:].partition("=")
            props[key] = value
    return props


def drives():
    """Every mounted /dev/sd* partition or whole disk: (facts, reasons it is not used)."""
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


def unmounted_usb_disks(found):
    """USB disks (/dev/sd*) with nothing of them mounted: [(name, description, [what is on them])]."""
    mounted = {re.sub(r"[0-9]+$", "", info["name"]) for info, _ in found}
    disks = []
    names = sorted(os.listdir(p("/sys/block"))) if os.path.isdir(p("/sys/block")) else []
    for disk in names:
        base = f"/sys/block/{disk}"
        if not re.match(r"^sd[a-z]+$", disk) or disk in mounted or "/usb" not in os.path.realpath(p(base)):
            continue
        size = int(read(base + "/size", "0") or 0) * 512
        if size == 0:
            continue  # a card reader slot with no card
        desc = f"{read(base + '/device/vendor', '?')} {read(base + '/device/model', '?')}, {size / 1e9:.1f} GB"
        parts = sorted((d for d in os.listdir(p(base)) if re.match(rf"^{disk}[0-9]+$", d)),
                       key=lambda d: int(d[len(disk):]))
        if any(partname(d).startswith("mpfree-test") for d in parts):
            disks.append((disk, desc + " (the MPFree test stick: never used)", None))
            continue
        held = []
        for node in [disk] + parts:
            props = udev_props(node)
            fs = props.get("ID_FS_TYPE", "")
            if node == disk and (parts or not fs):
                continue  # the whole disk counts only when it holds a filesystem itself
            label = props.get("ID_FS_LABEL", "")
            held.append((node, fs, label))
        disks.append((disk, desc, held))
    return disks


def no_drive_hint(disks):
    """Why none of the unmounted USB disks can be used yet, and what to do."""
    disks = [d for d in disks if d[2] is not None]  # never the test stick
    if not disks:
        return ("no spare drive to use: plug in one FAT32 or exFAT USB drive and let the desktop mount it "
                "(it then shows in the file manager)")
    fat = [d for d in disks if any(fs in FILESYSTEMS for _, fs, _ in d[2])]
    blank = [d for d in disks if not any(fs for _, fs, _ in d[2])]
    if fat:
        return ("a USB drive with FAT32 or exFAT is plugged in but not mounted: open it in the file manager "
                "(click it in the side bar) so the desktop mounts it, then run this again")
    if blank:
        return ("the USB drive has no filesystem yet, so the desktop cannot mount it. Format it as "
                "FAT first (this erases it): in the Disks app, pick that drive on the left, then the drive "
                "menu (top right) > Format Disk > MBR / DOS > Format; then the + under Volumes > Type: FAT > "
                "Create, and the play button to mount it. Then run this again")
    return ("the USB drive holds no FAT32 or exFAT filesystem: use a FAT32 or exFAT drive instead, "
            "then run this again")


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
    print("Mounted drives seen (/dev/sd*):")
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
        disks = unmounted_usb_disks(found)
        print("USB disks with nothing mounted (what udev says is on them):")
        for disk, desc, held in disks:
            print(f"  /dev/{disk}: {desc}")
            for node, fs, label in held or ():
                print(f"      /dev/{node}: {fs or 'no filesystem'}" + (f", label {label}" if label else ""))
            if held == []:
                print("      no partitions and no filesystem (blank)")
        if not disks:
            print("  (none)")
        raise Stop(no_drive_hint(disks))
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
    print("runs with developer mode on, and answer the MPC's question about the autorun script. The MPC has one")
    print("USB-A port and the test stick must stay in it, so the drive goes into a USB hub that holds the test")
    print("stick from power-on.")
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
