#!/usr/bin/env python3
"""WORKSTATION ONLY, READ-ONLY: compare two eMMC backups of the MPC One,
region by region, to show that a new backup matches an older one where it
should.

    compare-emmc-backups.py OLD_FOLDER NEW_FOLDER
    compare-emmc-backups.py --dir FOLDER

Each folder is one backup, mpc-one-emmc-<UTC>[-...]/, holding mmcblk0.img,
mmcblk0boot0.img, mmcblk0boot1.img and SHA256SUMS: as host_emmc_backup.py
makes it over SSH, or as MPFree's first-start wizard makes it on the stick
and `mpfree-stick copy-backup` (the creator's "Copy my MPC's backup") copies
it off. With --dir, the newest complete backup in FOLDER is compared with the
one before it (by the UTC time in their names); folders still being written
(.part, .partial) are passed over.

It reads every image once, and for each it:
  - checks the whole image against the folder's SHA256SUMS;
  - reads the partition table in mmcblk0.img and hashes each region: the
    MBR and primary GPT (sectors 0-63), each partition, and the backup GPT
    at the end; and the two boot areas whole.
Then it prints, for each region, whether the two backups are the same.

These must be the same between two backups of the same MPC on the same
factory software: the partition table, uboot-spl, uboot, splash,
recoverysplash, rootfs and both boot areas. MPFree never writes any of them.
env (the U-Boot environment) is normally the same too. content and data are
written by the factory software in normal use (projects, settings, and
MPFree's startup hook on data), so they differ. If the factory software was
updated between the two backups, uboot, the splashes and rootfs differ
because of the update.

Exit status: 0 when every region that must be the same is the same; 1 when
one differs or an image does not match its SHA256SUMS; 2 on a usage error or
a folder that is not a backup. It never writes anything.
"""
import argparse
import hashlib
import os
import re
import struct
import sys
import zlib

SECTOR = 512
GPT_HEAD_SECTORS = 64
CHUNK = 4 << 20
IMAGES = ("mmcblk0", "mmcblk0boot0", "mmcblk0boot1")
MUST_MATCH = {"gpt-head", "gpt-tail", "uboot-spl", "uboot", "splash", "recoverysplash", "rootfs",
              "mmcblk0boot0", "mmcblk0boot1"}
NORMALLY_SAME = {"env"}
WRITTEN_IN_USE = {"content", "data"}
NAME = re.compile(r"^mpc-one-emmc-(\d{8}T\d{6}Z)")


class NotABackup(Exception):
    pass


def partitions(path):
    """The partitions of the disk image at path: [(number, name, start, sectors)], and the disk's sectors."""
    size = os.path.getsize(path)
    with open(path, "rb") as fh:
        fh.seek(SECTOR)
        hdr = fh.read(SECTOR)
        if hdr[:8] != b"EFI PART":
            raise NotABackup(f"{path}: no GPT header at sector 1")
        hsize = struct.unpack_from("<I", hdr, 12)[0]
        if not 92 <= hsize <= SECTOR:
            raise NotABackup(f"{path}: the GPT header size {hsize} is implausible")
        check = bytearray(hdr[:hsize])
        check[16:20] = b"\0\0\0\0"
        if zlib.crc32(bytes(check)) != struct.unpack_from("<I", hdr, 16)[0]:
            raise NotABackup(f"{path}: the GPT header's CRC does not match")
        entries_lba, count, esize, ents_crc = struct.unpack_from("<QIII", hdr, 72)
        if not (0 < count <= 1024 and 128 <= esize <= 1024):
            raise NotABackup(f"{path}: the GPT entries ({count} of {esize} bytes) are implausible")
        fh.seek(entries_lba * SECTOR)
        ents = fh.read(count * esize)
        if zlib.crc32(ents) != ents_crc:
            raise NotABackup(f"{path}: the GPT entries' CRC does not match")
    parts = []
    for i in range(count):
        e = ents[i * esize:(i + 1) * esize]
        if e[:16] == b"\0" * 16:
            continue
        first, last = struct.unpack_from("<QQ", e, 32)
        name = e[56:128].decode("utf-16-le", "replace").split("\0", 1)[0]
        if last < first or (last + 1) * SECTOR > size:
            raise NotABackup(f"{path}: partition {i + 1} ({name}) is outside the image")
        parts.append((i + 1, name, first, last - first + 1))
    parts.sort(key=lambda p: p[2])
    return parts, size // SECTOR


def regions(path):
    """The regions of mmcblk0.img to hash: [(label, start, sectors)], and the partition table."""
    parts, sectors = partitions(path)
    out = [("gpt-head", 0, GPT_HEAD_SECTORS)]
    for n, name, start, count in parts:
        out.append((name or f"p{n}", start, count))
    end = max(s + c for _, s, c in out)
    if end < sectors:
        out.append(("gpt-tail", end, sectors - end))
    return out, parts


def read_sums(folder):
    path = os.path.join(folder, "SHA256SUMS")
    sums = {}
    try:
        with open(path) as fh:
            for line in fh:
                m = re.match(r"^([0-9a-f]{64})  ([A-Za-z0-9]+)\.img$", line.strip())
                if m:
                    sums[m.group(2)] = m.group(1)
    except OSError as e:
        raise NotABackup(f"{folder}: no SHA256SUMS ({e.strerror})")
    return sums


def hash_backup(folder, say):
    """{region: sha256} for one backup folder, the partition table, and the images whose SHA256SUMS differ."""
    sums = read_sums(folder)
    for d in IMAGES:
        if not os.path.isfile(os.path.join(folder, f"{d}.img")):
            raise NotABackup(f"{folder}: no {d}.img")
        if d not in sums:
            raise NotABackup(f"{folder}: SHA256SUMS has no line for {d}.img")
    hashes, bad = {}, []
    regs, parts = regions(os.path.join(folder, "mmcblk0.img"))
    for d in IMAGES:
        path = os.path.join(folder, f"{d}.img")
        size = os.path.getsize(path)
        say(f"  reading {d}.img ({size / 1e9:.2f} GB)")
        whole = hashlib.sha256()
        mine = [(label, start * SECTOR, (start + count) * SECTOR, hashlib.sha256())
                for label, start, count in (regs if d == "mmcblk0" else [])]
        pos = 0
        with open(path, "rb") as fh:
            while True:
                buf = fh.read(CHUNK)
                if not buf:
                    break
                whole.update(buf)
                end = pos + len(buf)
                for _, a, b, h in mine:
                    if a < end and b > pos:
                        h.update(buf[max(a, pos) - pos:min(b, end) - pos])
                pos = end
        if whole.hexdigest() != sums[d]:
            bad.append(d)
        if d == "mmcblk0":
            for label, _, _, h in mine:
                hashes[label] = h.hexdigest()
        else:
            hashes[d] = whole.hexdigest()
    return hashes, parts, bad


def complete_backups(folder):
    found = []
    for name in sorted(os.listdir(folder)):
        path = os.path.join(folder, name)
        m = NAME.match(name)
        if not m or name.endswith((".part", ".partial")) or not os.path.isdir(path):
            continue
        if all(os.path.isfile(os.path.join(path, f)) for f in ("SHA256SUMS",) + tuple(f"{d}.img" for d in IMAGES)):
            found.append((m.group(1), name, path))
    found.sort()
    return [p for _, _, p in found]


def compare(old, new, say):
    say(f"older backup: {old}")
    o, oparts, obad = hash_backup(old, say)
    say(f"newer backup: {new}")
    n, nparts, nbad = hash_backup(new, say)
    failed = False
    say("")
    for label, folder, bad in (("older", old, obad), ("newer", new, nbad)):
        if bad:
            failed = True
            verb = "does" if len(bad) == 1 else "do"
            say(f"DAMAGED: in the {label} backup, {', '.join(b + '.img' for b in bad)} {verb} not match SHA256SUMS")
        else:
            say(f"SHA256SUMS: every image of the {label} backup matches")
    layout = lambda parts: [(name, start, count) for _, name, start, count in parts]
    if layout(oparts) != layout(nparts):
        failed = True
        say("DIFFERENT: the partition tables differ (names, starts or sizes):")
        say(f"  older: {layout(oparts)}")
        say(f"  newer: {layout(nparts)}")
    else:
        say(f"partition table: the same layout, {len(nparts)} partitions")
    say("")
    say(f"  {'region':<15} {'result':<10} sha256 (newer)")
    for label in list(dict.fromkeys(list(o) + list(n))):
        a, b = o.get(label), n.get(label)
        same = a is not None and a == b
        allowed = label in WRITTEN_IN_USE or label in NORMALLY_SAME
        word = "same" if same else "differs" if allowed else "DIFFERS"
        failed = failed or not (same or allowed)
        why = ""
        if label in WRITTEN_IN_USE:
            why = "  (written in normal use: expected)"
        elif label in NORMALLY_SAME and not same:
            why = "  (the U-Boot environment: worth a look, not a failure)"
        elif label not in MUST_MATCH and not same:
            why = "  (a region this tool does not know)"
        say(f"  {label:<15} {word:<10} {b or '-'}{why}")
    say("")
    if failed:
        say("RESULT: DIFFERENT where the two backups must be the same (see above). If the factory software was "
            "updated between them, uboot, the splashes and rootfs differ because of that: compare backups made "
            "on the same version.")
    else:
        say("RESULT: the same everywhere they must be: partition table, boot areas, U-Boot, splashes and rootfs.")
    return 1 if failed else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description="Compare two eMMC backups of the MPC One, region by region "
                                             "(read-only).")
    ap.add_argument("folders", nargs="*", metavar="FOLDER", help="the older and the newer backup folder")
    ap.add_argument("--dir", help="compare the newest backup in this folder with the one before it")
    a = ap.parse_args(argv)
    say = lambda s: print(s, flush=True)
    try:
        if a.dir:
            if a.folders:
                ap.error("give --dir or two folders, not both")
            found = complete_backups(a.dir)
            if len(found) < 2:
                raise NotABackup(f"{a.dir}: needs two complete mpc-one-emmc-* backups, found {len(found)}")
            say(f"{len(found)} complete backups in {a.dir}; comparing the newest two")
            return compare(found[-2], found[-1], say)
        if len(a.folders) != 2:
            ap.error("give two backup folders (older, newer), or --dir FOLDER")
        return compare(a.folders[0], a.folders[1], say)
    except NotABackup as e:
        say(f"STOP: {e}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
