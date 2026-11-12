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

Saved settings (partition 4, "mpfree-test-data", made by offline-kexec-stick.py
--app): it fills the rest of the stick (plan decision 1, Jay 2026-11-12). The
privileged step writes a partition table made for this stick, in which
partitions 1 to 3 are the image's and partition 4 starts where the image's
does and runs to the end of the stick; the MPC grows the filesystem to fill it
at boot. What goes in partition 4:
  - the stick already holds MPFree's data partition where this image's starts
    (an ext4 labelled MPFREE-DATA there): it is left in place, untouched, so
    settings and files carry over however big they are. This is every round
    after the first with this layout;
  - the stick has MPFree's data partition somewhere else (the old 64 MiB
    layout): a copy is saved in the build folder first
    (data-from-old-stick-<UTC>.img), then it is written where the new one
    starts;
  - the stick has none (the first time): the image's empty one.
A partition 4 with that name that is not MPFree's stops the tool before
anything is written. So does a stick with two different MPFree data
partitions (one where it goes, another in its partition table, from a round
written with an older tool): a copy of the table's one is saved first, for
--data-from.
  --fresh-data         erase the saved settings too: partition 4 starts empty
  --data-from FILE     use FILE (such as a saved data-from-old-stick-*.img)
                       for partition 4 instead of the stick's

The write order keeps the saved settings safe if the write is cut off (the
stick pulled, the workstation stopped):
  1. a new partition 4, if one is written, where it now starts. A moved one
     is copied to the build folder before this (and if the old one ends
     before where the new one starts, it is still whole on the stick too).
     Its first 4 KiB, which name it, are cleared first and written last, the
     superblock's first sector very last; a superblock with a wrong checksum
     is never taken for MPFree's;
  2. partitions 1 to 3, and the zeros before partition 1;
  3. the backup partition table at the end of the stick, then the main one at
     the start. Until then the stick keeps its old table.
A data partition left in place is never written. If a write is cut off,
running this again finds the saved settings: in place (left there, or moved
there whole in step 1), or where they were (cut off in step 1), or in the
saved copy.

Undo: the stick's old contents are gone; nothing else changes. Reformat the
stick (for example with GNOME Disks) to use it for something else. If the write
is cut off, the old settings are on the stick still (left in place) or in the
saved copy (moved): run this again, or with --data-from that file.
"""
import fcntl
import hashlib
import os
import re
import stat
import struct
import subprocess
import sys
import time
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
MAX_SIZE = 256 << 30
BLKFLSBUF = 0x1261
BLKRRPART = 0x125F
CHUNK = 4 << 20
NAME = re.compile(r"^(sd[a-z]{1,3})$")
SECTOR = 512
DATA_NAME = "mpfree-test-data"   # partition 4's GPT name (offline-kexec-stick.py)
DATA_LABEL = b"MPFREE-DATA"      # its ext4 label
ALIGN = 2048                      # partitions end on 1 MiB boundaries
HEAD = 4096                       # a data partition's first bytes, which hold its ext4 superblock
MAX_CARRY = 256 << 20             # the most of an old data partition read into memory to move it (old: 64 MiB)
# Tests point this at a fake /sys, /dev and /proc. In real use it is empty.
TEST_ROOT = os.environ.get("MPFREE_TEST_ROOT", "")
# Partition 4 is at least this big on the stick (plan decision 1); the tests use small fake sticks.
MIN_DATA = (1 << 20) if TEST_ROOT else (1 << 30)


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


def read_all(fd, length, offset):
    out = bytearray()
    while len(out) < length:
        chunk = os.pread(fd, length - len(out), offset + len(out))
        if not chunk:
            break
        out += chunk
    return bytes(out)


def gpt_raw(read):
    """(protective MBR, header, entries) of the GPT that `read(length, offset)` sees, or None when there is
    no valid one (signature, sizes and both checksums)."""
    head = read(SECTOR, SECTOR)
    if len(head) < 92 or head[:8] != b"EFI PART":
        return None
    size, crc = struct.unpack_from("<II", head, 12)
    if not 92 <= size <= SECTOR or zlib.crc32(head[:16] + bytes(4) + head[20:size]) != crc:
        return None
    table_lba, count, entry_size, table_crc = struct.unpack_from("<QIII", head, 72)
    if not (128 <= entry_size <= 1024 and entry_size % 8 == 0 and 0 < count <= 1024):
        return None
    table = read(count * entry_size, table_lba * SECTOR)
    if len(table) != count * entry_size or zlib.crc32(table) != table_crc:
        return None
    return read(SECTOR, 0), head[:size], table


def entries(head, table):
    """[(number, first sector, sectors, name)] of a GPT's used entries, or None if one is not sane."""
    count, entry_size = struct.unpack_from("<II", head, 80)
    parts = []
    for i in range(count):
        entry = table[i * entry_size:(i + 1) * entry_size]
        if entry[:16] == bytes(16):
            continue
        first, last = struct.unpack_from("<QQ", entry, 32)
        if last < first:
            return None
        parts.append((i + 1, first, last - first + 1, entry[56:128].decode("utf-16-le", "replace").split("\0")[0]))
    return parts


def gpt_partitions(fd):
    """[(number, first sector, sectors, name)] from the GPT read through fd (an image file or a disk),
    or None when there is no valid GPT."""
    raw = gpt_raw(lambda length, offset: read_all(fd, length, offset))
    return entries(raw[1], raw[2]) if raw else None


def data_partition(parts):
    """(first sector, sectors) of the data partition, None if there is none; Stop if it is not number 4."""
    found = [part for part in parts or [] if part[3] == DATA_NAME]
    if not found:
        return None
    if len(found) > 1 or found[0][0] != 4:
        raise Stop(f"the {DATA_NAME} partition is not the one partition 4")
    return found[0][1], found[0][2]


def crc32c(data, crc=0xFFFFFFFF):
    """CRC-32C (Castagnoli) as the kernel's crc32c(): no final inversion."""
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0x82F63B78 & -(crc & 1))
    return crc


def is_data_head(head):
    """True for the first bytes of an ext4 labelled MPFREE-DATA, whose superblock checksum is right when it
    has one (metadata_csum), so a superblock cut off half written is not taken for one."""
    sb = head[1024:2048]
    if len(sb) != 1024 or sb[0x38:0x3A] != b"\x53\xef" or sb[0x78:0x88].rstrip(b"\0") != DATA_LABEL:
        return False
    if struct.unpack_from("<I", sb, 0x64)[0] & 0x400:      # metadata_csum
        return crc32c(sb[:0x3FC]) == struct.unpack_from("<I", sb, 0x3FC)[0]
    return True


def is_data_fs(data, size):
    """True for `size` bytes of an ext4 labelled MPFREE-DATA."""
    return len(data) == size and is_data_head(data)


def fs_bytes(head):
    """The size of the ext4 whose first bytes are `head`, from its superblock."""
    sb = head[1024:2048]
    blocks = struct.unpack_from("<I", sb, 0x04)[0]
    if struct.unpack_from("<I", sb, 0x60)[0] & 0x80:     # 64bit: the high half of the block count
        blocks |= struct.unpack_from("<I", sb, 0x150)[0] << 32
    return blocks * (1024 << struct.unpack_from("<I", sb, 0x18)[0])


def image_data_partition(image):
    fd = os.open(image, os.O_RDONLY | getattr(os, "O_CLOEXEC", 0))
    try:
        return data_partition(gpt_partitions(fd))
    finally:
        os.close(fd)


def image_parts(image):
    fd = os.open(image, os.O_RDONLY | getattr(os, "O_CLOEXEC", 0))
    try:
        raw = gpt_raw(lambda length, offset: read_all(fd, length, offset))
        if raw is None:
            raise Stop("stick.img has no valid partition table")
        return raw
    finally:
        os.close(fd)


def stick_table(raw, stick_sectors):
    """The image's partition table made for a stick of `stick_sectors` sectors: partitions 1 to 3 as in the
    image, partition 4 from where the image's starts to the last 1 MiB boundary before the backup table at
    the end of the stick. Returns (main table bytes for offset 0, backup table bytes, the backup's offset,
    partition 4's (first sector, sectors), the image's partitions)."""
    mbr, head, table = raw
    parts = entries(head, table)
    data = data_partition(parts)
    if data is None:
        raise Stop("stick.img has no data partition")
    count, entry_size = struct.unpack_from("<II", head, 80)
    table_sectors = -(-count * entry_size // SECTOR)
    first_usable, = struct.unpack_from("<Q", head, 40)
    if struct.unpack_from("<Q", head, 72)[0] != 2 or first_usable < 2 + table_sectors:
        raise Stop("stick.img's partition table is not laid out as offline-kexec-stick.py makes it")
    last_usable = stick_sectors - 2 - table_sectors
    first = data[0]
    last = (last_usable + 1) // ALIGN * ALIGN - 1
    if (last - first + 1) * SECTOR < MIN_DATA:
        raise Stop(f"the stick is too small for a data partition of {MIN_DATA >> 20} MiB or more")
    if any(n != 4 and f + size > first for n, f, size, _ in parts) or len(parts) != 4:
        raise Stop("stick.img's partitions are not 1 to 3 and then the data partition")
    table = bytearray(table) + bytes(table_sectors * SECTOR - len(table))
    struct.pack_into("<Q", table, 3 * entry_size + 40, last)
    table_crc = zlib.crc32(table[:count * entry_size])

    def header(mine, other, table_lba):
        h = bytearray(head)
        struct.pack_into("<QQ", h, 24, mine, other)
        struct.pack_into("<Q", h, 48, last_usable)
        struct.pack_into("<Q", h, 72, table_lba)
        struct.pack_into("<I", h, 88, table_crc)
        struct.pack_into("<I", h, 16, 0)
        struct.pack_into("<I", h, 16, zlib.crc32(h))
        return bytes(h) + bytes(SECTOR - len(h))

    mbr = bytearray(mbr)
    if len(mbr) != SECTOR or mbr[510:512] != b"\x55\xaa" or mbr[446 + 4] != 0xEE:
        raise Stop("stick.img has no protective MBR")
    struct.pack_into("<I", mbr, 446 + 12, min(stick_sectors - 1, 0xFFFFFFFF))
    main = bytes(mbr) + header(1, stick_sectors - 1, 2) + bytes(table)
    backup_lba = stick_sectors - 1 - table_sectors
    backup = bytes(table) + header(stick_sectors - 1, 1, backup_lba)
    # Read both back as a reader of the stick would, before anything is written.
    want = [(n, f, (last - f + 1) if n == 4 else size, name) for n, f, size, name in parts]
    made = gpt_raw(lambda length, offset: main[offset:offset + length])
    if made is None or entries(made[1], made[2]) != want:
        raise Stop("the partition table made for this stick does not read back as asked")

    def read_backup(length, offset):
        # The backup header is the stick's last sector; gpt_raw asks for the header at sector 1.
        if offset == SECTOR:
            return backup[-SECTOR:]
        return backup[offset - backup_lba * SECTOR:][:length] if offset >= backup_lba * SECTOR else b""

    back = gpt_raw(read_backup)
    if back is None or back[2] != made[2] or struct.unpack_from("<Q", back[1], 24)[0] != stick_sectors - 1:
        raise Stop("the backup partition table made for this stick does not read back as asked")
    return main, backup, backup_lba * SECTOR, (first, last - first + 1), parts


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


def check_device(name, need):
    if not NAME.match(name) or not os.path.isdir(p(f"/sys/block/{name}")):
        raise Stop(f"{name} is not a disk this tool writes")
    info, reasons = describe(name)
    if info["size"] < need:
        reasons.append(f"smaller than stick.img needs ({info['size']} < {need} bytes)")
    if info["size"] > MAX_SIZE:
        reasons.append("larger than 256 GiB, so probably not a USB stick")
    return info, reasons


def needed(image, size):
    """The smallest stick this image fits: the image, or with a data partition, up to its start and then
    MIN_DATA and 1 MiB for the backup partition table."""
    data = image_data_partition(image)
    return size if data is None else max(size, data[0] * SECTOR + MIN_DATA + ALIGN * SECTOR)


def label(info):
    return (f"/dev/{info['name']}: {info['vendor']} {info['model']}, serial {info['serial']}, "
            f"{info['size']} bytes ({info['size'] / 1e9:.1f} GB)")


def find_stick(need):
    names = sorted(os.listdir(p("/sys/block")))
    candidates = []
    print("USB and removable disks seen:")
    seen = 0
    for name in names:
        info, reasons = check_device(name, need) if NAME.match(name) else (None, ["skip"])
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


def data_plan(fd, info, image, image_data, new_data, data_mode):
    """What goes in partition 4 (first sector, sectors = new_data): (bytes to write there, or None to leave
    the stick's in place; True if those bytes came from the stick and need a saved copy). Stop, with
    nothing written, when the stick's partition 4 is not something to keep or replace safely."""
    first, sectors = new_data

    def empty():
        with open(image, "rb") as fh:
            fh.seek(image_data[0] * SECTOR)
            data = fh.read(image_data[1] * SECTOR)
        if not is_data_fs(data, image_data[1] * SECTOR):
            raise Stop("stick.img's partition 4 is not an empty MPFREE-DATA ext4")
        return data

    if data_mode == "fresh":
        print("partition 4: starts empty (--fresh-data)")
        return empty(), False
    if data_mode.startswith("file:"):
        return data_file(data_mode[5:], sectors * SECTOR), False
    # Keep: the stick's own, in place if it is where this image's starts.
    head = read_all(fd, HEAD, first * SECTOR)
    old = data_partition(gpt_partitions(fd))
    if is_data_head(head):
        if fs_bytes(head) > sectors * SECTOR:
            raise Stop(f"the MPFREE-DATA filesystem at partition 4 is {fs_bytes(head)} bytes, more than the "
                       f"{sectors * SECTOR} this stick has room for; nothing written")
        if old is not None and old[0] != first:
            other = read_all(fd, HEAD, old[0] * SECTOR)
            if is_data_head(other) and fs_uuid(other) != fs_uuid(head):
                two_data_partitions(fd, image, head, old, other)
        print(f"partition 4: the stick's MPFree data partition is already where it goes, so it stays in place "
              f"({fs_bytes(head)} bytes of filesystem); it is not written")
        return None, False
    if old is None:
        print("partition 4: the stick has no MPFree data partition, so the saved settings start empty")
        return empty(), False
    old_first, old_sectors = old
    if old_first == first:
        raise Stop(f"the stick's partition 4 ({DATA_NAME}) is not an MPFREE-DATA ext4; nothing written. "
                   "Run with --fresh-data to start it empty")
    if old_sectors * SECTOR > MAX_CARRY or (old_first + old_sectors) * SECTOR > info["size"]:
        raise Stop(f"the stick's partition 4 ({DATA_NAME}) is {old_sectors} sectors, more than this tool moves; "
                   "nothing written. Run with --fresh-data to start it empty, or --data-from a saved copy")
    if old_sectors > sectors:
        raise Stop(f"the stick's partition 4 ({DATA_NAME}) is bigger than the room for it in the new layout; "
                   "nothing written. Run with --fresh-data to start it empty, or --data-from a saved copy")
    data = read_all(fd, old_sectors * SECTOR, old_first * SECTOR)
    if not is_data_fs(data, old_sectors * SECTOR) or fs_bytes(data) > len(data):
        raise Stop(f"the stick's partition 4 ({DATA_NAME}) is not an MPFREE-DATA ext4; nothing written. "
                   "Run with --fresh-data to start it empty")
    return data, True


def fs_uuid(head):
    """The ext4's own UUID (made new by mke2fs; the same in a copy or a moved one)."""
    return head[1024 + 0x68:1024 + 0x78]


def written(head):
    """When the ext4 was last written, from its superblock."""
    stamp = struct.unpack_from("<I", head, 1024 + 0x30)[0]
    return time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime(stamp)) if stamp else "never"


def two_data_partitions(fd, image, head, old, other):
    """The stick holds MPFree's data partition where it goes and a different one (not a copy of it) where its
    partition table says: from a round written with an older tool after this layout. The table's is the one
    the MPC used last. Save a copy of it and stop, nothing written on the stick."""
    first, sectors = old
    copy = None
    if sectors * SECTOR <= MAX_CARRY:
        data = read_all(fd, sectors * SECTOR, first * SECTOR)
        if is_data_fs(data, sectors * SECTOR) and fs_bytes(data) <= len(data):
            copy = save_copy(image, data)
    raise Stop("the stick has two different MPFree data partitions: one where partition 4 goes (last written "
               f"{written(head)}) and the one its partition table names, at {first * SECTOR >> 20} MiB (last written "
               f"{written(other)}), which the MPC used last. Nothing written on the stick. "
               + (f"A copy of the second is in {copy}: run with --data-from that file to keep it, "
                  if copy else "It is too big to copy here: ")
               + "or with --fresh-data to start empty")


def data_file(path, room):
    """A saved data partition (--data-from): MPFree's ext4, whole, and no bigger than partition 4."""
    with open(path, "rb") as fh:
        data = fh.read(min(room, MAX_CARRY) + 1)
    if not is_data_head(data) or len(data) > min(room, MAX_CARRY) or fs_bytes(data) > len(data):
        raise Stop(f"{path} is not a whole MPFREE-DATA partition copy of at most {min(room, MAX_CARRY)} bytes")
    return data


def save_copy(image, data):
    """A copy of the carried data partition in the build folder, before the stick is erased. A new file,
    never one that is there already."""
    stamp = time.strftime("data-from-old-stick-%Y%m%dT%H%M%SZ", time.gmtime())
    for n in range(1, 100):
        path = os.path.join(os.path.dirname(image), stamp + ("" if n == 1 else f"-{n}") + ".img")
        try:
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0), 0o600)
            break
        except FileExistsError:
            continue
    else:
        raise Stop("could not make a new file for the copy of partition 4; nothing written")
    with os.fdopen(fd, "wb") as fh:
        fh.write(data)
        fh.flush()
        os.fsync(fh.fileno())
    if os.geteuid() == 0 and os.environ.get("SUDO_UID", "").isdigit() and os.environ.get("SUDO_GID", "").isdigit():
        os.chown(path, int(os.environ["SUDO_UID"]), int(os.environ["SUDO_GID"]))
    return path


# Tests only (with MPFREE_TEST_ROOT): cut the write off after this many writes, half way through the next.
CUT_AFTER = int(os.environ.get("MPFREE_TEST_CUT_AFTER", "-1")) if TEST_ROOT else -1
WRITES = [0]


def write_at(fd, offset, data):
    """The one place that writes the stick: `data` at byte `offset`, then waits until it is on the stick."""
    os.lseek(fd, offset, os.SEEK_SET)
    view, cut = memoryview(data), False
    if CUT_AFTER >= 0:
        WRITES[0] += 1
        if WRITES[0] > CUT_AFTER:
            # Half way, in whole sectors: a stick writes a sector whole or not at all.
            view, cut = view[:len(view) // 2 // SECTOR * SECTOR], True
    while view:
        n = os.write(fd, view)
        view = view[n:]
    os.fsync(fd)
    if cut:
        raise OSError("the write was cut off (test)")


def read_back(fd, regions):
    """sha256 of what the stick holds at [(offset, length)], in order."""
    h = hashlib.sha256()
    for offset, length in regions:
        os.lseek(fd, offset, os.SEEK_SET)
        left = length
        while left:
            chunk = os.read(fd, min(CHUNK, left))
            if not chunk:
                raise Stop("the stick ended before the image did")
            h.update(chunk)
            left -= len(chunk)
    return h.hexdigest()


def root_write(name, size, image, want, data_mode="none"):
    """The privileged step: check again, open exclusively, decide partition 4, write, read back, compare."""
    if os.path.getsize(image) != size or sha256(image) != want:
        raise Stop("stick.img changed since it was checked; nothing written")
    image_data = image_data_partition(image)
    if (image_data is None) != (data_mode == "none"):
        raise Stop("the data partition mode does not match stick.img; nothing written")
    info, reasons = check_device(name, needed(image, size))
    if reasons:
        raise Stop(f"/dev/{name} no longer qualifies ({'; '.join(reasons)}); nothing written")
    dev = p("/dev/" + name)
    st = os.stat(dev)
    if not TEST_ROOT and not stat.S_ISBLK(st.st_mode):
        raise Stop(f"{dev} is not a block device; nothing written")
    if not TEST_ROOT and f"{os.major(st.st_rdev)}:{os.minor(st.st_rdev)}" != info["dev"]:
        raise Stop(f"{dev} is not the device sysfs describes; nothing written")
    fd = os.open(dev, os.O_RDWR | os.O_EXCL | getattr(os, "O_CLOEXEC", 0))
    copy, started = None, False
    try:
        if image_data is None:
            # No data partition: the image as it is, from the first byte.
            writes = [(0, size, None)]
            checks = writes
            note = f"they match stick.img (sha256 {want})"
        else:
            main, backup, backup_at, new_data, parts = stick_table(image_parts(image), info["size"] // SECTOR)
            content, from_stick = data_plan(fd, info, image, image_data, new_data, data_mode)
            if from_stick:
                copy = save_copy(image, content)
                print(f"partition 4: moving {len(content)} bytes (sha256 {hashlib.sha256(content).hexdigest()}) "
                      f"to where it now starts; a copy is in {copy}")
            elif data_mode.startswith("file:"):
                copy = data_mode[5:]
                print(f"partition 4: {len(content)} bytes from {copy}")
            # From the end of the main table (so the gap before partition 1 holds the image's zeros, not what
            # the stick had) to the end of partition 3. The main table itself is written last.
            start = len(main)
            end = max(f + count for n, f, count, _ in parts if n != 4) * SECTOR
            writes, checks = [], []
            if content is not None:
                at = new_data[0] * SECTOR
                # A new partition 4 first, before partitions 1 to 3 can overwrite the old one. Its first 4 KiB
                # are cleared first and written last, with the superblock's first sector (its magic and name)
                # the very last write, so a cut-off write never leaves something taken for MPFree's there.
                writes += [(at, HEAD, bytes(HEAD)), (at + HEAD, len(content) - HEAD, content[HEAD:]),
                           (at, 1024, content[:1024]), (at + 2048, HEAD - 2048, content[2048:HEAD]),
                           (at + 1536, 512, content[1536:2048]), (at + 1024, 512, content[1024:1536])]
                checks.append((at, len(content), content))
            writes.append((start, end - start, None))     # partitions 1 to 3, from the image
            checks.append((start, end - start, None))
            writes += [(backup_at, len(backup), backup), (0, len(main), main)]
            checks += [(backup_at, len(backup), backup), (0, len(main), main)]
            print(f"partition 4: {new_data[1] * SECTOR} bytes, from {new_data[0] * SECTOR >> 20} MiB to the end of "
                  "the stick; the MPC grows its filesystem to fill it")
            note = "they match stick.img's partitions 1 to 3, partition 4 and a partition table made for this stick"
        total = sum(length for _, length, _ in writes)
        print(f"writing {total} bytes to /dev/{name} ...", flush=True)
        done, last = 0, time.time()
        started = True
        with open(image, "rb") as src:
            for offset, length, data in writes:
                if data is not None:
                    write_at(fd, offset, data)
                    continue
                src.seek(offset)
                pos = 0
                while pos < length:
                    chunk = src.read(min(CHUNK, length - pos))
                    if not chunk:
                        raise Stop("stick.img ended early")
                    write_at(fd, offset + pos, chunk)
                    pos += len(chunk)
                    done += len(chunk)
                    if time.time() - last > 5:
                        print(f"  {done >> 20} of {total >> 20} MiB", flush=True)
                        last = time.time()
        # What should now be on the stick, in order.
        expected = hashlib.sha256()
        with open(image, "rb") as src:
            for offset, length, data in checks:
                if data is not None:
                    expected.update(data)
                    continue
                src.seek(offset)
                left = length
                while left:
                    chunk = src.read(min(CHUNK, left))
                    expected.update(chunk)
                    left -= len(chunk)
        if not TEST_ROOT:
            fcntl.ioctl(fd, BLKFLSBUF)   # drop cached blocks, so the read-back comes from the stick
        if read_back(fd, [(offset, length) for offset, length, _ in checks]) != expected.hexdigest():
            raise Stop("READ-BACK MISMATCH: the stick does not hold what was written. Do not boot from it.")
        print(f"read back {sum(length for _, length, _ in checks)} bytes: {note}")
        if not TEST_ROOT:
            try:
                fcntl.ioctl(fd, BLKRRPART)  # show the new partitions to the workstation
            except OSError:
                pass
    except (Stop, OSError):
        if not started:
            pass                    # stopped before the first write: the stick is as it was
        elif copy:
            print(f"Your saved settings are in {copy}: write again with --data-from that file.")
        elif data_mode == "keep" and image_data is not None:
            print("Your saved settings, if the stick had them where they go, are still on it: run this again.")
        raise
    finally:
        os.close(fd)
    return 0


def main(argv):
    if argv[:1] == ["--root-write"]:
        if len(argv) != 6 or not argv[2].isdigit() or \
                not (argv[5] in ("none", "keep", "fresh") or argv[5].startswith("file:/")):
            raise Stop("usage: --root-write NAME SIZE IMAGE SHA256 DATA (run without arguments instead)")
        if not TEST_ROOT and os.geteuid() != 0:
            raise Stop("--root-write runs under sudo")
        return root_write(argv[1], int(argv[2]), argv[3], argv[4], argv[5])
    if argv == ["--fresh-data"]:
        mode = "fresh"
    elif len(argv) == 2 and argv[0] == "--data-from":
        mode = "file:" + os.path.realpath(argv[1])
    elif not argv:
        mode = "keep"
    else:
        print(__doc__.strip())
        return 2
    image, want = newest_stick()
    size = os.path.getsize(image)
    print(f"image: {image} ({size} bytes)")
    print("checking its hash ...", flush=True)
    if sha256(image) != want:
        raise Stop("stick.img does not match its SHA256SUMS line; build it again")
    new_data = image_data_partition(image)
    if new_data is None:
        if mode != "keep":
            raise Stop("this stick.img has no data partition (built without --app); nothing to keep or start empty")
        mode = "none"
    elif mode.startswith("file:"):
        data_file(mode[5:], MAX_CARRY)
    info = find_stick(needed(image, size))
    print()
    print(f"This ERASES everything on {label(info)}")
    print(f"and writes stick.img there ({size >> 20} MiB). Nothing else on this computer is touched.")
    if mode != "none":
        print("Partition 4 (saved settings and files) fills the rest of the stick.")
    if mode == "keep":
        print("It is kept from this stick if it has MPFree's: left in place where it already goes, or moved with")
        print("a copy saved in the build folder first. Otherwise it starts empty. (--fresh-data: start empty.)")
    elif mode == "fresh":
        print("Partition 4 starts empty (--fresh-data): the saved settings on this stick are erased too.")
    elif mode != "none":
        print(f"Partition 4 comes from {mode[5:]} (--data-from).")
    if ask("Type WRITE and press Enter to go on (anything else stops): ") != "WRITE":
        print("Stopped. Nothing was written.")
        return 1
    sudo = [] if TEST_ROOT else ["sudo"]
    res = subprocess.run(sudo + [sys.executable, "-I", os.path.abspath(__file__), "--root-write",
                                 info["name"], str(size), image, want, mode])
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
