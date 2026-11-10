#!/usr/bin/env python3
"""WORKSTATION ONLY, READ-ONLY: after the kexec test boot, copy the logs that
the test build saved on the USB stick into one report.

Plug the stick into this computer, not the MPC. The tool takes the log
partition's PARTUUID from the newest ~/mpfree-work/kexec-stick-*/report.txt
(the build that write-test-stick.py wrote) and stops unless that partition is
on a USB disk and has the size the builder gave it. It reads the partition
once, with `sudo cat` (sudo asks for your password), into a new folder
~/mpfree-work/stick-logs-<UTC>-*/, and from then on works only on that copy:
e2fsck replays the copy's journal (the MPC was switched off with the partition
still mounted), and debugfs reads the files out of the copy. It never writes
to the stick and never contacts the MPC.

It writes report.txt in that folder: every file of every test boot, with MAC
addresses and USB serial numbers replaced. Run it through results.py, which
copies report.txt into the round's folder. The partition copy stays in the
folder; keep it private like the other work folders.

Your desktop may mount the stick's two partitions when you plug it in. That
does no harm, and this tool does not need it.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

LOG_SIZE = 32 << 20          # offline-kexec-stick.py's LOG_SIZE (a test checks they match)
PARTUUID = re.compile(r"partition 2 logs PARTUUID ([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})")
PART_NAME = re.compile(r"^sd[a-z]{1,3}2$")
BOOT_DIR = re.compile(r"^boot-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
FILE_NAME = re.compile(r"^[A-Za-z0-9._-]{1,64}$")
FIRST = ("cmdline.txt", "failed-units.txt", "network.txt", "units.txt", "dmesg.txt", "journal.txt")
MAC = re.compile(r"\b[0-9A-Fa-f]{2}(?::[0-9A-Fa-f]{2}){5}\b")
SERIAL = re.compile(r"(SerialNumber: |ID_SERIAL_SHORT=|ID_SERIAL=)\S+")
# Tests point this at a fake /dev and /sys, and then sudo is not used. In real use it is empty.
TEST_ROOT = os.environ.get("MPFREE_TEST_ROOT", "")


class Stop(Exception):
    pass


def p(path):
    return TEST_ROOT + path


def run(cmd):
    return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)


def newest_build(work):
    """The newest kexec-stick folder with a stick.img (the rule write-test-stick.py uses), and its log PARTUUID."""
    folders = sorted(d for d in os.listdir(work) if d.startswith("kexec-stick-")) if os.path.isdir(work) else []
    folders = [d for d in folders if os.path.isfile(os.path.join(work, d, "stick.img"))]
    if not folders:
        raise Stop(f"no kexec-stick folder with a stick.img under {work}")
    folder = os.path.join(work, folders[-1])
    with open(os.path.join(folder, "report.txt")) as fh:
        found = PARTUUID.findall(fh.read())
    if len(found) != 1:
        raise Stop(f"{folder}/report.txt does not name one log partition")
    return folder, found[0]


def find_partition(uuid):
    link = p(f"/dev/disk/by-partuuid/{uuid}")
    if not os.path.lexists(link):
        raise Stop("the stick's log partition is not on this computer. Plug the stick into this computer "
                   "(not the MPC), wait 10 s, and run this again")
    dev = os.path.realpath(link)
    name = os.path.basename(dev)
    if not PART_NAME.match(name) or dev != os.path.realpath(p("/dev/" + name)):
        raise Stop(f"the log partition is {dev}, not partition 2 of a USB disk; nothing was read")
    sys_path = os.path.realpath(p(f"/sys/class/block/{name}"))
    if "/usb" not in sys_path:
        raise Stop(f"/dev/{name} is not on USB; nothing was read")
    try:
        with open(os.path.join(sys_path, "size")) as fh:
            size = int(fh.read().strip()) * 512
    except (OSError, ValueError):
        size = -1
    if size != LOG_SIZE:
        raise Stop(f"/dev/{name} is {size} bytes, not the log partition's {LOG_SIZE}; nothing was read")
    return dev, name


def copy_partition(dev, dest):
    """The one read of the stick: sudo cat into a new file. Returns the bytes copied."""
    sudo = [] if TEST_ROOT else ["sudo"]
    with open(dest, "xb") as fh:
        res = subprocess.run(sudo + ["cat", "--", dev], stdout=fh)
    size = os.path.getsize(dest)
    if res.returncode != 0 or size != LOG_SIZE:
        raise Stop(f"reading {dev} failed (exit code {res.returncode}, {size} of {LOG_SIZE} bytes)")
    return size


class Copy:
    """The working copy of the log filesystem, read with debugfs (read-only without -w)."""

    def __init__(self, path):
        self.path = path

    def cmd(self, command):
        res = subprocess.run(["debugfs", "-R", command, self.path], stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL)
        return res.stdout.decode("utf-8", "replace")

    def entries(self, path):
        """[(inode, is_dir, name, size)] of one directory, from `ls -p`."""
        out = []
        for line in self.cmd(f'ls -p "{path}"').splitlines():
            f = line.split("/")
            if len(f) != 8 or not f[1].isdigit() or f[5] in (".", ".."):
                continue
            out.append((int(f[1]), f[2].startswith("04"), f[5], int(f[6]) if f[6].isdigit() else 0))
        return out


def redact(text):
    return SERIAL.sub(lambda m: m.group(1) + "(removed)", MAC.sub("xx:xx:xx:xx:xx:xx", text))


def main(argv):
    if argv:
        print(__doc__.strip())
        return 2
    for tool in ("debugfs", "e2fsck") + (() if TEST_ROOT else ("sudo",)):
        if not shutil.which(tool):
            raise Stop(f"missing '{tool}'. Install e2fsprogs with your package manager")
    work = os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    build, uuid = newest_build(work)
    print("MPFree: read the test boot's logs from the USB stick (read-only, no MPC contact)")
    print(f"build: {build}")
    print(f"log partition PARTUUID: {uuid}")
    dev, name = find_partition(uuid)
    print(f"found: /dev/{name} on USB, {LOG_SIZE >> 20} MiB")
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    folder = tempfile.mkdtemp(prefix=f"stick-logs-{stamp}-", dir=work)
    raw = os.path.join(folder, "log-partition.img")
    print(f"copying /dev/{name} to {raw} (sudo may ask for your password) ...", flush=True)
    copy_partition(dev, raw)
    work_copy = os.path.join(folder, "log-partition-replayed.img")
    shutil.copyfile(raw, work_copy)
    replay = run(["e2fsck", "-y", "-E", "journal_only", work_copy])
    check = run(["e2fsck", "-fn", work_copy])
    fs = Copy(work_copy)
    boots = sorted((e for e in fs.entries("/") if e[1] and BOOT_DIR.match(e[2])), key=lambda e: e[0])

    lines = ["MPFree kexec test boot: logs from the USB stick's log partition",
             f"read {time.strftime('%Y-%m-%d %H:%M:%S UTC', time.gmtime())} from /dev/{name} (PARTUUID {uuid})",
             f"build: {os.path.basename(build)}",
             "MAC addresses and USB serial numbers are replaced.", "",
             f"journal replay on the copy (e2fsck -E journal_only): exit code {replay.returncode}",
             replay.stdout.rstrip(),
             f"check of the copy (e2fsck -fn): exit code {check.returncode}",
             check.stdout.rstrip(), "",
             f"test boots found: {len(boots)}"]
    print(f"test boots found: {len(boots)}")
    order = {name: i for i, name in enumerate(FIRST)}
    files = {}
    for _, _, boot, _ in boots:
        found = [e for e in fs.entries(f"/{boot}") if not e[1] and FILE_NAME.match(e[2])]
        files[boot] = sorted(found, key=lambda e: (order.get(e[2], len(FIRST)), e[2]))
        lines.append(f"  {boot}: " + ", ".join(f"{e[2]} ({e[3]} bytes)" for e in files[boot]))
        print(lines[-1])
    for _, _, boot, _ in boots:
        for _, _, fname, size in files[boot]:
            text = redact(fs.cmd(f'cat "/{boot}/{fname}"')).rstrip()
            lines += ["", f"===== {boot}/{fname} ({size} bytes) =====", text]
            if fname == "failed-units.txt":
                print(f"\n{boot}/failed-units.txt:\n{text}")
    if not boots:
        lines.append("No test boot left logs: the test build did not get as far as starting its log service, "
                     "or it could not mount this partition.")
        print(lines[-1])
    report = os.path.join(folder, "report.txt")
    with open(report, "w") as fh:
        fh.write("\n".join(lines) + "\n")
    print(f"\nreport: {report}")
    print("Nothing was written to the stick. Unplug it.")
    return 0


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
