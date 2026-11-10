#!/usr/bin/env python3
"""OFFLINE, workstation only: list what an Akai update image contains.

Akai's MPC update images (and inMusic's other az0x devices) are flattened
device tree (FDT/FIT) containers: a tree of nodes whose properties hold the
partition payloads, their hashes and signatures. This prints the tree with
every property's name and size, short values in full, and a SHA-256 and
format guess for every large payload. It answers "which partitions does
this update write?" without decompressing, mounting or running anything.

It only reads the files it is given. It never contacts the MPC. A .zip
download is unpacked into a temporary folder next to it, which is deleted
afterwards.

Usage: python3 tools/mpc-one/offline-update-image.py PATH [PATH ...]
PATH may be an .img file, a .zip holding one, or a folder (every .img and
.zip directly inside it is read).
"""
import hashlib
import os
import struct
import sys
import tempfile
import zipfile

FDT_MAGIC = 0xD00DFEED
FDT_BEGIN_NODE, FDT_END_NODE, FDT_PROP, FDT_NOP, FDT_END = 1, 2, 3, 4, 9
HEADER = struct.Struct(">10I")
SEARCH_PREFIX = 1 << 20  # look this far into a file for the FDT header
SHORT_VALUE = 256
MAX_DEPTH = 64
CHUNK = 1 << 20

MAGICS = (
    (b"\x1f\x8b", "gzip"),
    (b"\xfd7zXZ\x00", "xz"),
    (b"\x28\xb5\x2f\xfd", "zstd"),
    (b"\x04\x22\x4d\x18", "lz4 frame"),
    (b"\x02\x21\x4c\x18", "lz4 legacy"),
    (b"BZh", "bzip2"),
    (b"\x5d\x00\x00", "lzma"),
    (b"hsqs", "squashfs"),
    (b"\x27\x05\x19\x56", "U-Boot legacy image"),
    (b"\xd0\x0d\xfe\xed", "FDT"),
    (b"\x7fELF", "ELF"),
    (b"RKNS", "Rockchip idbloader (RKNS)"),
    (b"LDR ", "Rockchip loader"),
    (b"BM", "BMP image"),
    (b"\x89PNG", "PNG image"),
)


class ImageError(Exception):
    pass


def guess(head):
    for magic, name in MAGICS:
        if head.startswith(magic):
            return name
    if not head.strip(b"\x00"):
        return "zeros at the start"
    return "unknown"


def guess_region(fh, offset, length):
    fh.seek(offset)
    head = fh.read(min(length, 2048))
    # An ext2/3/4 image starts with 1 KiB of zeros; its magic sits at byte 1080.
    if len(head) >= 1082 and head[1080:1082] == b"\x53\xef":
        kind = "ext2/3/4 filesystem"
    else:
        kind = guess(head[:16])
    return kind, head[:16].hex()


def sha256_region(fh, offset, length):
    digest = hashlib.sha256()
    fh.seek(offset)
    left = length
    while left:
        block = fh.read(min(CHUNK, left))
        if not block:
            raise ImageError(f"file ends inside a payload at offset {offset + length - left}")
        digest.update(block)
        left -= len(block)
    return digest.hexdigest()


def printable_strings(value):
    """Return a list of strings if value is one or more NUL-terminated strings."""
    if not value or value[-1:] != b"\x00":
        return None
    parts = value[:-1].split(b"\x00")
    if any(not p for p in parts):
        return None
    try:
        texts = [p.decode("ascii") for p in parts]
    except UnicodeDecodeError:
        return None
    if all(t.isprintable() for t in texts):
        return texts
    return None


def show_value(fh, name, offset, length):
    if length == 0:
        return "(empty)"
    if length <= SHORT_VALUE:
        fh.seek(offset)
        value = fh.read(length)
        texts = printable_strings(value)
        if texts is not None:
            return ", ".join(f'"{t}"' for t in texts)
        if length % 4 == 0 and length <= 16:
            cells = struct.unpack(f">{length // 4}I", value)
            return "<" + " ".join(f"0x{c:x}" for c in cells) + ">" + (
                f" ({cells[0]})" if length == 4 else "")
        return f"[{length} bytes] {value.hex()}"
    kind, head = guess_region(fh, offset, length)
    digest = sha256_region(fh, offset, length)
    return f"[{length} bytes, {kind}, starts {head}, sha256 {digest}]"


def find_fdt(fh, size):
    fh.seek(0)
    head = fh.read(min(size, SEARCH_PREFIX))
    start = 0
    while True:
        at = head.find(b"\xd0\x0d\xfe\xed", start)
        if at < 0:
            raise ImageError(f"no FDT header in the first {len(head)} bytes; starts {head[:16].hex()} "
                             f"({guess(head[:16])})")
        if at + HEADER.size <= size:
            fields = HEADER.unpack_from(head, at) if at + HEADER.size <= len(head) else None
            if fields and fields[1] >= HEADER.size and at + fields[1] <= size and fields[5] >= 16:
                return at, fields
        start = at + 1


def walk(fh, base, fields, file_size, out):
    (_, total, off_struct, off_strings, _, version, last_comp, _,
     size_strings, size_struct) = fields
    out.append(f"FDT header at offset {base}: version {version} (compatible with {last_comp}), "
               f"total size {total} bytes, file size {file_size} bytes")
    if base + total < file_size:
        kind, head = guess_region(fh, base + total, file_size - base - total)
        out.append(f"data after the FDT: {file_size - base - total} bytes ({kind}, starts {head})")
    if base:
        kind, head = guess_region(fh, 0, base)
        out.append(f"data before the FDT: {base} bytes ({kind}, starts {head}), "
                   f"sha256 {sha256_region(fh, 0, base)}")
    for off, length, what in ((off_struct, size_struct, "structure"), (off_strings, size_strings, "strings")):
        if off + length > total:
            raise ImageError(f"FDT {what} block runs past the FDT's total size")
    fh.seek(base + off_strings)
    strings = fh.read(size_strings)

    pos = base + off_struct
    end = pos + size_struct
    depth = 0
    path = []
    external = []  # (node path, data-offset or data-position, data-size)
    node_props = {}

    def read_u32(at):
        fh.seek(at)
        raw = fh.read(4)
        if len(raw) != 4:
            raise ImageError(f"file ends inside the FDT structure at offset {at}")
        return struct.unpack(">I", raw)[0]

    while pos < end:
        token = read_u32(pos)
        pos += 4
        if token == FDT_BEGIN_NODE:
            fh.seek(pos)
            raw = fh.read(min(256, end - pos))
            nul = raw.find(b"\x00")
            if nul < 0:
                raise ImageError(f"unterminated node name at offset {pos}")
            name = raw[:nul].decode("ascii", "replace") or "/"
            pos += (nul + 1 + 3) & ~3
            out.append("  " * depth + (name if depth == 0 else name + "/"))
            path.append(name)
            node_props = {}
            depth += 1
            if depth > MAX_DEPTH:
                raise ImageError("FDT nesting is implausibly deep")
        elif token == FDT_END_NODE:
            if "data-size" in node_props and ("data-offset" in node_props or "data-position" in node_props):
                external.append(("/".join(path[1:]) or "/", node_props))
            depth -= 1
            path.pop()
            node_props = {}
            if depth < 0:
                raise ImageError(f"unbalanced FDT_END_NODE at offset {pos - 4}")
        elif token == FDT_PROP:
            length = read_u32(pos)
            name_off = read_u32(pos + 4)
            pos += 8
            name_end = strings.find(b"\x00", name_off)
            if name_off >= len(strings) or name_end < 0:
                raise ImageError(f"property name offset {name_off} outside the strings block")
            name = strings[name_off:name_end].decode("ascii", "replace")
            if pos + length > end:
                raise ImageError(f"property {name} runs past the FDT structure block")
            if length == 4 and name in ("data-size", "data-offset", "data-position"):
                node_props[name] = read_u32(pos)
            out.append("  " * depth + f"{name} = {show_value(fh, name, pos, length)}")
            pos += (length + 3) & ~3
        elif token == FDT_NOP:
            continue
        elif token == FDT_END:
            break
        else:
            raise ImageError(f"unknown FDT token 0x{token:x} at offset {pos - 4}")
    if depth != 0:
        raise ImageError("FDT structure ended inside a node")

    for node, props in external:
        size = props["data-size"]
        if "data-position" in props:  # absolute, from the start of the FIT
            start = base + props["data-position"]
        else:  # FIT external data: offset from the end of the FDT, rounded up to 4 bytes
            start = base + ((total + 3) & ~3) + props["data-offset"]
        if start + size > file_size:
            out.append(f"external data for {node}: offset {start}, {size} bytes: PAST THE END OF THE FILE")
            continue
        kind, head = guess_region(fh, start, size)
        out.append(f"external data for {node}: offset {start}, {size} bytes, {kind}, starts {head}, "
                   f"sha256 {sha256_region(fh, start, size)}")


def describe_image(path):
    out = [f"===== {path}"]
    size = os.path.getsize(path)
    out.append(f"size {size} bytes, sha256 {file_sha256(path)}")
    with open(path, "rb") as fh:
        base, fields = find_fdt(fh, size)
        walk(fh, base, fields, size, out)
    return out


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(CHUNK), b""):
            digest.update(block)
    return digest.hexdigest()


def inputs(paths):
    for path in paths:
        if os.path.isdir(path):
            found = sorted(os.path.join(path, n) for n in os.listdir(path)
                           if n.lower().endswith((".img", ".zip")) and os.path.isfile(os.path.join(path, n)))
            if not found:
                raise ImageError(f"no .img or .zip file directly inside {path}")
            yield from found
        elif os.path.isfile(path):
            yield path
        else:
            raise ImageError(f"not found: {path}")


def describe(path):
    if not zipfile.is_zipfile(path):
        return describe_image(path)
    out = [f"===== {path} (zip archive)"]
    with zipfile.ZipFile(path) as archive:
        members = [m for m in archive.infolist() if not m.is_dir()]
        for m in members:
            out.append(f"  {m.filename}: {m.file_size} bytes")
        wanted = [m for m in members if m.filename.lower().endswith(".img")]
        if not wanted:
            out.append("  (no .img inside; nothing more to read)")
            return out
        with tempfile.TemporaryDirectory(prefix=".unzipped-", dir=os.path.dirname(os.path.abspath(path))) as tmp:
            for m in wanted:
                # Extract by our own name, never the archive's path, so nothing lands outside tmp.
                target = os.path.join(tmp, f"member-{len(os.listdir(tmp))}.img")
                with archive.open(m) as src, open(target, "wb") as dst:
                    for block in iter(lambda: src.read(CHUNK), b""):
                        dst.write(block)
                lines = describe_image(target)
                lines[0] = f"===== {path} -> {m.filename}"
                out.extend(lines)
    return out


def main(argv):
    if not argv:
        print(__doc__.strip().split("\n\n")[-1], file=sys.stderr)
        return 2
    status = 0
    try:
        paths = list(inputs(argv))
    except ImageError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    for path in paths:
        try:
            lines = describe(path)
        except (ImageError, OSError, zipfile.BadZipFile) as exc:
            print(f"===== {path}\nERROR: {exc}\n")
            status = 1
            continue
        print("\n".join(lines))
        print()
    print("Akai's update files are proprietary: keep them private and commit only findings.")
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
