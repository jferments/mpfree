#!/usr/bin/env python3
"""WORKSTATION ONLY, ERASES ONE SPARE USB STICK: give it one partition with a
new filesystem and a label, then mount it where the desktop mounts sticks. It
is for the pull tests (WP), whose runs each start on a freshly formatted spare.

  python3 tools/mpc-one/format-spare-stick.py --fat32 WP-FAT
  python3 tools/mpc-one/format-spare-stick.py --exfat WP-EXFAT
  python3 tools/mpc-one/format-spare-stick.py --ext4 WP-TEST

It never contacts the MPC. It never asks you to type a device name: it looks
for the one disk that is
  - a whole disk on USB that the kernel marks removable, and not read-only,
  - between 1 GB and 256 GiB,
  - not the MPFree stick (no partition named mpfree-test-*, no MPFREE-DATA),
  - with nothing stacked on it (LVM, RAID, dm-crypt) and no swap on it,
  - with nothing of it mounted, except by the desktop under /run/media/YOU or
    /media/YOU (it unmounts those, after you confirm),
and it stops if there is no such disk or more than one. It lists every USB
or removable disk it sees, and for the one it would erase it shows where it
is plugged in (the USB port and the by-path name), its make, model, serial
number and size, and each partition with its filesystem, label, size and
where it is mounted. It erases only after you type ERASE. It also stops if
another disk already has a filesystem with the label asked for, since the
pull tests find the stick by its label.

The erase runs under sudo (it asks for your password). That step checks the
disk again (the same serial number and size, nothing mounted now), opens it
exclusively, clears its first and last MiB (old partition tables, the GPT's
backup copy included) and the first MiB of where the new partition goes, writes an MBR with one partition from 1 MiB to the
last whole MiB (type 0c for FAT32, 07 for exFAT, 83 for ext4), has the kernel
read it, wipes any old signature from the partition (wipefs -a), makes the
filesystem (mkfs.vfat -F 32, mkfs.exfat, or mkfs.ext4 with its top folder
yours) and reads it back with blkid. Then, as you, it mounts it the way the
file manager does (udisksctl mount) and prints where.

Undo: the stick's old contents are gone; nothing else on this computer
changes. Format it again (with this or the desktop's disk tool) to use it for
something else. If the erase stops part way, the stick may have no readable
partition: run this again.
"""
import fcntl
import os
import random
import re
import shutil
import stat
import struct
import subprocess
import sys
import time

# Tests point this at a fake /sys, /dev, /proc and /run. In real use it is empty.
TEST_ROOT = os.environ.get("MPFREE_TEST_ROOT", "")
NAME = re.compile(r"^sd[a-z]{1,3}$")
LABEL = re.compile(r"^[A-Z0-9][A-Z0-9-]{0,10}$")
SECTOR = 512
ALIGN = 2048                      # 1 MiB, in sectors
MIN_SIZE = (1 << 20) * 16 if TEST_ROOT else 10 ** 9
MAX_SIZE = 256 << 30
KINDS = {  # flag: (name, MBR type, filesystem as blkid names it)
    "--fat32": ("FAT32", 0x0C, "vfat"),
    "--exfat": ("exFAT", 0x07, "exfat"),
    "--ext4": ("ext4", 0x83, "ext4"),
}
CONFIRM = "ERASE"
BLKFLSBUF = 0x1261
# The privileged step runs only programs from the system's own folders (in tests, the fake ones first).
ROOT_PATH = os.environ.get("PATH", "") if TEST_ROOT else "/usr/sbin:/usr/bin:/sbin:/bin"


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
    """mountinfo writes a space as \\040 (and tab, newline, backslash likewise)."""
    return re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), field)


def udev_props(dev):
    """udev's record of the block device with numbers `dev` (ID_FS_TYPE, ID_FS_LABEL, ...), read without root."""
    props = {}
    for line in read(f"/run/udev/data/b{dev}").splitlines() if dev else []:
        if line.startswith("E:") and "=" in line:
            key, _, value = line[2:].partition("=")
            props[key] = value
    return props


def mounts():
    """Everything mounted or used as swap: [(major:minor, resolved source, mount point, type)]."""
    found = []
    with open(p("/proc/self/mountinfo")) as fh:
        for line in fh:
            left, _, right = line.partition(" - ")
            left, right = left.split(), right.split()
            if len(left) < 5 or len(right) < 2:
                continue
            source = os.path.realpath(p(right[1])) if right[1].startswith("/dev/") else right[1]
            found.append((left[2], source, unescape(left[4]), right[0]))
    with open(p("/proc/swaps")) as fh:
        for line in fh.readlines()[1:]:
            if line.split():
                found.append(("", os.path.realpath(p(line.split()[0])), "[swap]", "swap"))
    return found


def size_text(n):
    return f"{n / 1e9:.1f} GB ({n} bytes)"


def first_part(name):
    """The name Linux gives a disk's first partition: sdb1, but loop0p1 or nvme0n1p1 after a digit."""
    return name + ("p1" if name[-1].isdigit() else "1")


def location(name):
    """Where the disk is plugged in: its USB port (bus-port path) and speed, and its /dev/disk/by-path name."""
    real = os.path.realpath(p(f"/sys/block/{name}"))
    ports = [part for part in real.split("/") if re.match(r"^\d+-\d+(\.\d+)*$", part)]
    where = "not on USB"
    if ports:
        port = ports[-1]
        speed = read(f"/sys/bus/usb/devices/{port}/speed")
        where = f"USB port {port}" + (f" ({speed} Mbit/s)" if speed else "")
    by_path = ""
    folder = p("/dev/disk/by-path")
    for link in sorted(os.listdir(folder)) if os.path.isdir(folder) else []:
        if os.path.realpath(os.path.join(folder, link)) == os.path.realpath(p(f"/dev/{name}")):
            by_path = link
    return where, by_path


def user_media(mount_point, user):
    return any(mount_point.startswith(f"{base}/{user}/") for base in ("/run/media", "/media"))


def describe(name, user):
    """Facts about /sys/block/<name>, its partitions and their mounts, and every reason it must not be erased."""
    base = f"/sys/block/{name}"
    sectors = read(base + "/size", "0")
    info = {
        "name": name,
        "dev": read(base + "/dev"),
        "size": int(sectors) * SECTOR if sectors.isdigit() else 0,
        "removable": read(base + "/removable"),
        "usb": "/usb" in os.path.realpath(p(base)),
        "vendor": read(base + "/device/vendor", "?"),
        "model": read(base + "/device/model", "?"),
        "ro": read(base + "/ro"),
    }
    disk_props = udev_props(info["dev"])
    info["serial"] = disk_props.get("ID_SERIAL_SHORT", "?")
    info["table"] = {"dos": "MBR", "gpt": "GPT"}.get(disk_props.get("ID_PART_TABLE_TYPE", ""), "none")
    info["where"], info["by_path"] = location(name)
    folder = p(base)
    parts = sorted((d for d in os.listdir(folder) if d.startswith(name) and os.path.isfile(p(f"{base}/{d}/partition"))),
                   key=lambda d: int(re.search(r"(\d+)$", d).group(1))) if os.path.isdir(folder) else []
    seen = mounts()
    reasons, info["parts"], info["desktop"] = [], [], []
    if not NAME.match(name):
        reasons.append("not a USB disk name")
    if not info["usb"]:
        reasons.append("not on USB")
    if info["removable"] != "1":
        reasons.append("not marked removable (external hard disks usually aren't)")
    if info["ro"] != "0":
        reasons.append("read-only")
    if os.path.isfile(p(base + "/partition")):
        reasons.append("a partition, not a whole disk")
    if info["size"] < MIN_SIZE:
        reasons.append("smaller than 1 GB" if info["size"] else "no medium in it")
    if info["size"] > MAX_SIZE:
        reasons.append("larger than 256 GiB, so probably not a USB stick")
    for node in [name] + parts:
        dev = read(f"/sys/class/block/{node}/dev")
        props = udev_props(dev)
        here = [(mp, kind) for number, source, mp, kind in seen
                if (dev and number == dev) or source == os.path.realpath(p("/dev/" + node))]
        if node != name or props.get("ID_FS_TYPE"):
            part_sectors = read(f"/sys/class/block/{node}/size", "0")
            info["parts"].append({
                "node": node,
                "size": int(part_sectors) * SECTOR if part_sectors.isdigit() else 0,
                "fs": props.get("ID_FS_TYPE", ""),
                "label": props.get("ID_FS_LABEL", ""),
                "partname": props.get("ID_PART_ENTRY_NAME", "") or partname(node),
                "mounted": [mp for mp, _ in here],
            })
        for mp, kind in here:
            if kind != "swap" and user_media(mp, user):
                info["desktop"].append(node)
            else:
                reasons.append(f"/dev/{node} is in use at {mp}" + (" (swap)" if kind == "swap" else ""))
        if os.path.isdir(p(f"/sys/class/block/{node}/holders")) and os.listdir(p(f"/sys/class/block/{node}/holders")):
            reasons.append(f"/dev/{node} is in use by LVM, RAID or dm-crypt")
        if (props.get("ID_PART_ENTRY_NAME", "") or partname(node)).startswith("mpfree-test") or \
                props.get("ID_FS_LABEL", "") == "MPFREE-DATA":
            reasons.append("the MPFree stick")
    info["desktop"] = sorted(set(info["desktop"]))
    return info, sorted(set(reasons), key=reasons.index)


def partname(node):
    for line in read(f"/sys/class/block/{node}/uevent").splitlines():
        if line.startswith("PARTNAME="):
            return line.split("=", 1)[1]
    return ""


def show(info, reasons, full, verdict=True):
    print(f"  /dev/{info['name']}: {info['vendor']} {info['model']}, serial {info['serial']}, {size_text(info['size'])}")
    if full:
        print(f"      plugged in at: {info['where']}" + (f", /dev/disk/by-path/{info['by_path']}" if info["by_path"] else ""))
        print(f"      partition table: {info['table']}")
        if not info["parts"]:
            print("      no partitions and no filesystem")
        for part in info["parts"]:
            what = part["fs"] or "no filesystem"
            if part["label"]:
                what += f", label {part['label']}"
            if part["partname"]:
                what += f", partition name {part['partname']}"
            where = "mounted at " + ", ".join(part["mounted"]) if part["mounted"] else "not mounted"
            print(f"      /dev/{part['node']}: {what}, {size_text(part['size'])}, {where}")
    if verdict:
        print("      " + ("CAN be erased" if not reasons else "not erased: " + "; ".join(reasons)))


def find_stick(user):
    print("USB and removable disks seen:")
    names = sorted(os.listdir(p("/sys/block"))) if os.path.isdir(p("/sys/block")) else []
    candidates, seen = [], 0
    for name in names:
        if not NAME.match(name):
            continue
        info, reasons = describe(name, user)
        if not (info["usb"] or info["removable"] == "1"):
            continue
        seen += 1
        show(info, reasons, full=False)
        if not reasons:
            candidates.append(info)
    if not seen:
        print("  (none)")
    if not candidates:
        raise Stop("no spare USB stick to erase: plug in one USB stick whose contents may be lost, then run this "
                   "again. Nothing was changed")
    if len(candidates) > 1:
        raise Stop("more than one USB stick could be erased: unplug all but the spare one, then run this again. "
                   "Nothing was changed")
    return candidates[0]


def label_elsewhere(info, label):
    """Another disk's filesystem with this label, if any (the pull tests find the stick by its label)."""
    ours = {info["name"]} | {part["node"] for part in info["parts"]}
    folder = p("/sys/class/block")
    for node in sorted(os.listdir(folder)) if os.path.isdir(folder) else []:
        if node in ours or re.sub(r"\d+$", "", node) == info["name"]:
            continue
        if udev_props(read(f"/sys/class/block/{node}/dev")).get("ID_FS_LABEL") == label:
            return node
    return None


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


def program(name):
    found = shutil.which(name, path=ROOT_PATH)
    if not found:
        raise Stop(f"{name} is not installed (it comes with " + {
            "mkfs.vfat": "dosfstools", "mkfs.exfat": "exfatprogs", "mkfs.ext4": "e2fsprogs",
            "udisksctl": "udisks2"}.get(name, "util-linux") + "); nothing more was done")
    return found


def run(args, what):
    res = subprocess.run(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                         text=True, timeout=600)
    out = res.stdout.strip()
    if res.returncode != 0:
        raise Stop(f"{what} failed ({res.returncode}): {out}")
    return out


def udisks(verb, node):
    """udisksctl mount or unmount of /dev/<node> as you, like the file manager, without a password prompt."""
    res = subprocess.run([program("udisksctl"), verb, "--no-user-interaction", "-b", f"/dev/{node}"],
                         stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                         timeout=60)
    return res.returncode == 0, res.stdout.strip()


def chs(lba):
    """An MBR's cylinder-head-sector bytes for `lba` (255 heads, 63 sectors; past 1023 cylinders, the maximum)."""
    cyl, head, sec = lba // (255 * 63), (lba // 63) % 255, lba % 63 + 1
    if cyl > 1023:
        cyl, head, sec = 1023, 254, 63
    return bytes([head, sec | ((cyl >> 2) & 0xC0), cyl & 0xFF])


def mbr(total_sectors, kind, signature):
    """512 bytes: an MBR whose one partition runs from 1 MiB to the last whole MiB of the disk."""
    first = ALIGN
    count = (total_sectors // ALIGN) * ALIGN - first
    if count < ALIGN:
        raise Stop("the stick is too small for a partition")
    sector = bytearray(SECTOR)
    struct.pack_into("<I", sector, 440, signature)
    entry = bytes([0x00]) + chs(first) + bytes([kind]) + chs(first + count - 1) + struct.pack("<II", first, count)
    sector[446:462] = entry
    sector[510:512] = b"\x55\xaa"
    return bytes(sector), first, count


def write_at(fd, offset, data):
    os.lseek(fd, offset, os.SEEK_SET)
    view = memoryview(data)
    while view:
        done = os.write(fd, view)
        view = view[done:]


def wait_for(path, seconds, check=os.path.exists):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if check(path):
            return True
        time.sleep(0.2)
    return check(path)


def root_format(name, serial, size, flag, label, uid, gid, user):
    """The privileged step: check again, open exclusively, clear the ends, one partition, its filesystem, read back."""
    kind_name, kind, fs = KINDS[flag]
    info, reasons = describe(name, user)
    if info["serial"] != serial or info["size"] != size:
        raise Stop(f"/dev/{name} is not the stick you confirmed (its serial number or size changed); nothing written")
    if info["desktop"]:
        reasons.append("something of it is mounted again")
    if reasons:
        raise Stop(f"/dev/{name} no longer qualifies ({'; '.join(reasons)}); nothing written")
    dev = p("/dev/" + name)
    st = os.stat(dev)
    if not TEST_ROOT and not stat.S_ISBLK(st.st_mode):
        raise Stop(f"{dev} is not a block device; nothing written")
    if not TEST_ROOT and f"{os.major(st.st_rdev)}:{os.minor(st.st_rdev)}" != info["dev"]:
        raise Stop(f"{dev} is not the device sysfs describes; nothing written")
    mkfs_name = {"vfat": "mkfs.vfat", "exfat": "mkfs.exfat", "ext4": "mkfs.ext4"}[fs]
    tools = {t: program(t) for t in ("blockdev", "wipefs", "blkid", mkfs_name)}
    udevadm = shutil.which("udevadm", path=ROOT_PATH)
    sector, first, count = mbr(size // SECTOR, kind, random.SystemRandom().randrange(1, 1 << 32))
    # Exclusive: refused while anything of the disk is mounted or held, and nothing can mount it meanwhile.
    fd = os.open(dev, os.O_RDWR | os.O_EXCL | getattr(os, "O_CLOEXEC", 0))
    try:
        print(f"clearing the first 2 MiB and the last MiB of /dev/{name}, then writing one {kind_name} partition "
              f"of {count * SECTOR} bytes from 1 MiB ...", flush=True)
        # The old table and whatever began at 1 MiB, so nothing old shows (or is mounted) at the new partition.
        write_at(fd, 0, bytes(2 * ALIGN * SECTOR))
        write_at(fd, size - ALIGN * SECTOR, bytes(ALIGN * SECTOR))
        write_at(fd, 0, sector)
        os.fsync(fd)
        if not TEST_ROOT:
            fcntl.ioctl(fd, BLKFLSBUF)
        os.lseek(fd, 0, os.SEEK_SET)
        if os.read(fd, SECTOR) != sector:
            raise Stop(f"/dev/{name}'s first sector does not read back as written")
    finally:
        os.close(fd)
    node = first_part(name)
    part = p("/dev/" + node)

    def ready(path):
        if not os.path.exists(path):
            return False
        if TEST_ROOT:
            return True
        st = os.stat(path)
        return stat.S_ISBLK(st.st_mode) and \
            f"{os.major(st.st_rdev)}:{os.minor(st.st_rdev)}" == read(f"/sys/class/block/{node}/dev")
    # The kernel reads the new table. udev may be probing the disk after the write (it re-reads a disk written
    # to itself), which makes the kernel's re-read busy for a moment: settle, try three times, and if the
    # partition still isn't there, partx adds it from the table.
    settle = (lambda: subprocess.run([udevadm, "settle", "--timeout=10"], stdin=subprocess.DEVNULL, check=False)) \
        if udevadm else (lambda: None)
    for _ in range(3):
        settle()
        res = subprocess.run([tools["blockdev"], "--rereadpt", dev], stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=60)
        if res.returncode == 0:
            break
        print(f"the kernel's re-read of /dev/{name} was refused ({res.stdout.strip()}); trying again", flush=True)
        time.sleep(1)
    settle()
    partx = shutil.which("partx", path=ROOT_PATH)
    if not wait_for(part, 5, ready) and partx:
        print(f"/dev/{node} not there yet: adding it from the new table (partx -u)", flush=True)
        subprocess.run([partx, "-u", dev], stdin=subprocess.DEVNULL, check=False)
        settle()
    if not wait_for(part, 10, ready):
        raise Stop(f"/dev/{node} did not appear after the new partition table; unplug the stick, plug it back in "
                   "and run this again")
    if read(f"/sys/class/block/{node}/start") not in ("", str(first)) or \
            read(f"/sys/class/block/{node}/size") not in ("", str(count)):
        raise Stop(f"/dev/{node} is not where the new table puts it; run this again")
    run([tools["wipefs"], "-a", part], f"clearing old signatures from /dev/{node}")
    mkfs = [tools[mkfs_name]] + {"vfat": ["-F", "32", "-n", label],
                                 "exfat": ["-L", label],
                                 "ext4": ["-q", "-L", label, "-E", f"root_owner={uid}:{gid}"]}[fs] + [part]
    print(f"making {kind_name} on /dev/{node}, labelled {label} ...", flush=True)
    run(mkfs, f"making {kind_name}")
    found = dict(line.split("=", 1) for line in run([tools["blkid"], "-p", "-o", "export", part],
                                                    "reading it back").splitlines() if "=" in line)
    if found.get("TYPE") != fs or found.get("LABEL") != label:
        raise Stop(f"/dev/{node} reads back as {found.get('TYPE', 'nothing')} labelled {found.get('LABEL', '')!r}, "
                   f"not {fs} labelled {label}")
    if udevadm:
        subprocess.run([udevadm, "settle", "--timeout=10"], stdin=subprocess.DEVNULL, check=False)
    print(f"/dev/{node} reads back as {fs}, labelled {label}")
    return 0


def main(argv):
    if argv[:1] == ["--root-format"]:
        if len(argv) != 9 or argv[4] not in KINDS or not argv[3].isdigit() or not LABEL.match(argv[5]) or \
                not argv[6].isdigit() or not argv[7].isdigit():
            raise Stop("usage: --root-format NAME SERIAL SIZE KIND LABEL UID GID USER (run without it instead)")
        if not TEST_ROOT and os.geteuid() != 0:
            raise Stop("--root-format runs under sudo")
        if not NAME.match(argv[1]):
            raise Stop(f"{argv[1]} is not a disk this tool erases")
        return root_format(argv[1], argv[2], int(argv[3]), argv[4], argv[5], argv[6], argv[7], argv[8])
    if len(argv) != 2 or argv[0] not in KINDS:
        print(__doc__.strip())
        return 2
    flag, label = argv
    kind_name, _, fs = KINDS[flag]
    if not LABEL.match(label):
        raise Stop("the label is 1 to 11 capital letters, digits or dashes, starting with a letter or digit")
    user = os.environ.get("USER") or os.environ.get("LOGNAME") or ""
    if not TEST_ROOT and os.geteuid() == 0:
        raise Stop("run this as yourself, not under sudo: it asks for sudo itself, for the erase only")
    print(f"Format a spare USB stick as {kind_name}, labelled {label} (workstation only, no MPC contact)")
    info = find_stick(user)
    other = label_elsewhere(info, label)
    if other:
        raise Stop(f"/dev/{other} already has a filesystem labelled {label}: unplug that disk (or rename its "
                   "filesystem), then run this again. Nothing was changed")
    print()
    print("This ERASES everything on this stick:")
    show(info, [], full=True, verdict=False)
    print(f"It becomes one {kind_name} partition labelled {label}, mounted where your desktop mounts sticks.")
    print("Nothing else on this computer is touched.")
    if info["desktop"]:
        print("Your desktop has it mounted: it is unmounted first (udisksctl unmount, as you).")
    if ask(f"Type {CONFIRM} and press Enter to erase it (anything else stops): ") != CONFIRM:
        print("Stopped. Nothing was changed.")
        return 1
    for node in info["desktop"]:
        worked, message = udisks("unmount", node)
        print(f"unmounting /dev/{node}: {message or ('done' if worked else 'no answer')}")
        if not worked:
            raise Stop(f"/dev/{node} could not be unmounted: close any window showing it, then run this again. "
                       "Nothing was erased")
    sudo = [] if TEST_ROOT else ["sudo"]
    res = subprocess.run(sudo + [sys.executable, "-I", os.path.abspath(__file__), "--root-format", info["name"],
                                 info["serial"], str(info["size"]), flag, label, str(os.getuid()), str(os.getgid()),
                                 user])
    if res.returncode != 0:
        print("The erase did not finish; see above. The stick may have no readable partition: run this again.")
        return 1
    node = first_part(info["name"])
    link = p(f"/dev/disk/by-label/{label}")
    if not wait_for(link, 15, lambda path: os.path.realpath(path) == os.path.realpath(p(f"/dev/{node}"))):
        raise Stop(f"/dev/disk/by-label/{label} did not appear for /dev/{node}; unplug the stick, plug it back in, "
                   "and it mounts as usual")
    mounted = [mp for _, source, mp, _ in mounts() if source == os.path.realpath(p(f"/dev/{node}"))]
    if not mounted:
        worked, message = udisks("mount", node)
        print(f"mounting /dev/{node}: {message or ('done' if worked else 'no answer')}")
        mounted = [mp for _, source, mp, _ in mounts() if source == os.path.realpath(p(f"/dev/{node}"))]
        if not worked and not mounted:  # (the desktop may have mounted it first)
            raise Stop(f"/dev/{node} is ready but could not be mounted: open it in the file manager")
    if not mounted:
        raise Stop(f"udisksctl answered, but /dev/{node} does not show as mounted: open it in the file manager")
    print(f"READY: /dev/{node}, {kind_name}, labelled {label}, mounted at {mounted[0]}")
    return 0


if __name__ == "__main__":
    os.umask(0o022)
    sys.stdout.reconfigure(line_buffering=True)  # in order with sudo's and the programs' own output
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError, subprocess.SubprocessError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:
        print("\nSTOP: interrupted (Ctrl-C).")
        sys.exit(130)
