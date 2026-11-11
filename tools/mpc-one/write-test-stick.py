#!/usr/bin/env python3
"""WORKSTATION ONLY, ERASES ONE USB STICK: write the newest stick.img made by
offline-kexec-stick.py to a USB stick, then read it back and compare.

It never contacts the MPC. It never asks you to type a device name: it looks
for the one device that is
  - a whole disk on USB that the kernel marks removable,
  - between the image's size and 256 GiB,
  - with nothing mounted from it, no swap on it and nothing stacked on it
    (LVM, RAID, dm-crypt),
and it stops if there is no such device or more than one. It shows that
device's vendor, model, serial and size and writes only after you type WRITE.

The write itself runs under sudo (it asks for your password). Before writing,
the privileged step checks the device again, then opens it exclusively, so
nothing can mount it during the write and the read-back. Afterwards it reports
whether the bytes on the stick match stick.img.

Undo: the stick's old contents are gone; nothing else changes. Reformat the
stick (for example with GNOME Disks) to use it for something else.
"""
import fcntl
import hashlib
import os
import re
import stat
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
MAX_SIZE = 256 << 30
BLKFLSBUF = 0x1261
BLKRRPART = 0x125F
CHUNK = 4 << 20
NAME = re.compile(r"^(sd[a-z]{1,3})$")
# Tests point this at a fake /sys, /dev and /proc. In real use it is empty.
TEST_ROOT = os.environ.get("MPFREE_TEST_ROOT", "")


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


def sha256(path, length=None):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        left = length
        while left is None or left > 0:
            chunk = fh.read(CHUNK if left is None else min(CHUNK, left))
            if not chunk:
                break
            h.update(chunk)
            if left is not None:
                left -= len(chunk)
    return h.hexdigest()


def newest_stick():
    work = os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    folders = sorted(d for d in os.listdir(work) if d.startswith("kexec-stick-")) if os.path.isdir(work) else []
    folders = [d for d in folders if os.path.isfile(os.path.join(work, d, "stick.img"))]
    if not folders:
        raise Stop(f"no stick.img under {work}: run offline-kexec-stick.py first")
    folder = os.path.join(work, folders[-1])
    image = os.path.join(folder, "stick.img")
    want = None
    with open(os.path.join(folder, "SHA256SUMS")) as fh:
        for line in fh:
            parts = line.split()
            if len(parts) == 2 and parts[1] == "stick.img":
                want = parts[0]
    if not want:
        raise Stop(f"{folder}/SHA256SUMS has no stick.img line")
    return image, want


def mounted_devices():
    """(major:minor numbers, resolved source paths) of everything mounted or used as swap."""
    numbers, sources = set(), set()
    with open(p("/proc/self/mountinfo")) as fh:
        for line in fh:
            left, _, right = line.partition(" - ")
            numbers.add(left.split()[2])
            fields = right.split()
            if len(fields) >= 2 and fields[1].startswith("/dev/"):
                sources.add(os.path.realpath(p(fields[1])))
    with open(p("/proc/swaps")) as fh:
        for line in fh.readlines()[1:]:
            if line.split():
                sources.add(os.path.realpath(p(line.split()[0])))
    return numbers, sources


def describe(name):
    """Facts about /sys/block/<name> and, as a list, every reason it must not be written."""
    base = f"/sys/block/{name}"
    sectors = read(base + "/size", "0")
    info = {
        "name": name,
        "dev": read(base + "/dev"),
        "size": int(sectors) * 512 if sectors.isdigit() else 0,
        "removable": read(base + "/removable"),
        "usb": "/usb" in os.path.realpath(p(base)),
        "vendor": read(base + "/device/vendor", "?"),
        "model": read(base + "/device/model", "?"),
        "serial": "?",
        "ro": read(base + "/ro"),
    }
    for line in read(f"/run/udev/data/b{info['dev']}").splitlines():
        if line.startswith("E:ID_SERIAL_SHORT="):
            info["serial"] = line.split("=", 1)[1]
    parts = sorted(d for d in os.listdir(p(base)) if d.startswith(name) and os.path.isfile(p(f"{base}/{d}/partition")))
    numbers, sources = mounted_devices()
    reasons = []
    if not NAME.match(name):
        reasons.append("not a SCSI/USB disk name")
    if not info["usb"]:
        reasons.append("not on USB")
    if info["removable"] != "1":
        reasons.append("not marked removable (external hard disks usually are not)")
    if info["ro"] != "0":
        reasons.append("read-only")
    if os.path.isfile(p(base + "/partition")):
        reasons.append("a partition, not a whole disk")
    for node in [name] + parts:
        dev = read(f"/sys/class/block/{node}/dev")
        if dev in numbers or os.path.realpath(p("/dev/" + node)) in sources:
            reasons.append(f"/dev/{node} is mounted or used as swap")
        if os.listdir(p(f"/sys/class/block/{node}/holders")):
            reasons.append(f"/dev/{node} is in use by LVM, RAID or dm-crypt")
    return info, reasons


def check_device(name, image_size):
    if not NAME.match(name) or not os.path.isdir(p(f"/sys/block/{name}")):
        raise Stop(f"{name} is not a disk this tool writes")
    info, reasons = describe(name)
    if info["size"] < image_size:
        reasons.append(f"smaller than stick.img ({info['size']} < {image_size} bytes)")
    if info["size"] > MAX_SIZE:
        reasons.append("larger than 256 GiB, so probably not a USB stick")
    return info, reasons


def label(info):
    return (f"/dev/{info['name']}: {info['vendor']} {info['model']}, serial {info['serial']}, "
            f"{info['size']} bytes ({info['size'] / 1e9:.1f} GB)")


def find_stick(image_size):
    names = sorted(os.listdir(p("/sys/block")))
    candidates = []
    print("USB and removable disks seen:")
    seen = 0
    for name in names:
        info, reasons = check_device(name, image_size) if NAME.match(name) else (None, ["skip"])
        if info is None or not (info["usb"] or info["removable"] == "1"):
            continue
        seen += 1
        print(f"  {label(info)}")
        print("      " + ("CAN be written" if not reasons else "not written: " + "; ".join(reasons)))
        if not reasons:
            candidates.append(info)
    if not seen:
        print("  (none)")
    if not candidates:
        raise Stop("no USB stick to write: plug in one USB stick whose contents may be erased")
    if len(candidates) > 1:
        raise Stop("more than one USB stick could be written: unplug all but the one to erase, then run this again")
    return candidates[0]


def ask(prompt):
    if not TEST_ROOT:
        if not sys.stdin.isatty():
            raise Stop("run this from a terminal: it asks you to confirm")
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)  # drop anything pasted after the command
    return input(prompt).strip()


def root_write(name, size, image, want):
    """The privileged step: check again, open exclusively, write, flush, read back, compare."""
    if os.path.getsize(image) != size or sha256(image) != want:
        raise Stop("stick.img changed since it was checked; nothing written")
    info, reasons = check_device(name, size)
    if reasons:
        raise Stop(f"/dev/{name} no longer qualifies ({'; '.join(reasons)}); nothing written")
    dev = p("/dev/" + name)
    st = os.stat(dev)
    if not TEST_ROOT and not stat.S_ISBLK(st.st_mode):
        raise Stop(f"{dev} is not a block device; nothing written")
    if not TEST_ROOT and f"{os.major(st.st_rdev)}:{os.minor(st.st_rdev)}" != info["dev"]:
        raise Stop(f"{dev} is not the device sysfs describes; nothing written")
    fd = os.open(dev, os.O_RDWR | os.O_EXCL | getattr(os, "O_CLOEXEC", 0))
    try:
        print(f"writing {size} bytes to /dev/{name} ...", flush=True)
        done, last = 0, time.time()
        with open(image, "rb") as src:
            while True:
                chunk = src.read(CHUNK)
                if not chunk:
                    break
                view = memoryview(chunk)
                while view:
                    n = os.write(fd, view)
                    view = view[n:]
                done += len(chunk)
                if time.time() - last > 5:
                    print(f"  {done >> 20} of {size >> 20} MiB", flush=True)
                    last = time.time()
        os.fsync(fd)
        if not TEST_ROOT:
            fcntl.ioctl(fd, BLKFLSBUF)   # drop cached blocks, so the read-back comes from the stick
        os.lseek(fd, 0, os.SEEK_SET)
        h, left = hashlib.sha256(), size
        while left:
            chunk = os.read(fd, min(CHUNK, left))
            if not chunk:
                raise Stop("the stick ended before the image did")
            h.update(chunk)
            left -= len(chunk)
        if h.hexdigest() != want:
            raise Stop("READ-BACK MISMATCH: the stick does not hold stick.img. Do not boot from it.")
        print(f"read back {size} bytes: they match stick.img (sha256 {want})")
        if not TEST_ROOT:
            try:
                fcntl.ioctl(fd, BLKRRPART)  # show the new partitions to the workstation
            except OSError:
                pass
    finally:
        os.close(fd)
    return 0


def main(argv):
    if argv[:1] == ["--root-write"]:
        if len(argv) != 5 or not argv[2].isdigit():
            raise Stop("usage: --root-write NAME SIZE IMAGE SHA256 (run without arguments instead)")
        if not TEST_ROOT and os.geteuid() != 0:
            raise Stop("--root-write runs under sudo")
        return root_write(argv[1], int(argv[2]), argv[3], argv[4])
    if argv:
        print(__doc__.strip())
        return 2
    image, want = newest_stick()
    size = os.path.getsize(image)
    print(f"image: {image} ({size} bytes)")
    print("checking its hash ...", flush=True)
    if sha256(image) != want:
        raise Stop("stick.img does not match its SHA256SUMS line; build it again")
    info = find_stick(size)
    print()
    print(f"This ERASES everything on {label(info)}")
    print(f"and writes stick.img there ({size >> 20} MiB). Nothing else on this computer is touched.")
    if ask("Type WRITE and press Enter to go on (anything else stops): ") != "WRITE":
        print("Stopped. Nothing was written.")
        return 1
    sudo = [] if TEST_ROOT else ["sudo"]
    res = subprocess.run(sudo + [sys.executable, "-I", os.path.abspath(__file__), "--root-write",
                                 info["name"], str(size), image, want])
    if res.returncode != 0:
        print("The write did not finish cleanly; see above. Do not boot from this stick.")
        return 1
    print()
    print("Done. Your desktop may now show the stick's partitions; do not open or change them.")
    print("Unplug the stick and keep it for the boot test.")
    return 0


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
