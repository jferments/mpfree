#!/usr/bin/env python3
"""OFFLINE, workstation only: list what an Akai update image contains.

Two container formats are understood:
- older Akai MPC images (and inMusic's other az0x devices) are flattened
  device tree (FDT/FIT) containers: a tree of nodes whose properties hold
  the partition payloads, their hashes and signatures;
- newer ones (seen in MPC 3.10.0) start with "AZ01": a little-endian header
  (build, compatible boards, USB device IDs, description) followed by one
  "PART" record per partition (name, compression, hashes, then the data).
This prints every field, and a SHA-256 and format guess for every payload.
It answers "which partitions does this update write?" without
decompressing, mounting or running anything.

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


class Node:
    """One FDT node: its properties as (name, file offset, length), in order."""

    def __init__(self, name, path):
        self.name, self.path = name, path
        self.props, self.children = [], []

    def prop(self, name):
        for pname, offset, length in self.props:
            if pname == name:
                return offset, length
        return None

    def child(self, name):
        for c in self.children:
            if c.name == name:
                return c
        return None


class Fdt:
    """A parsed FDT inside an open file. Values stay in the file; only offsets are kept."""

    def __init__(self, fh, file_size):
        self.fh, self.file_size = fh, file_size
        self.base, fields = find_fdt(fh, file_size)
        (_, self.total, off_struct, off_strings, _, self.version, self.last_comp, _,
         size_strings, size_struct) = fields
        for off, length, what in ((off_struct, size_struct, "structure"), (off_strings, size_strings, "strings")):
            if off + length > self.total:
                raise ImageError(f"FDT {what} block runs past the FDT's total size")
        fh.seek(self.base + off_strings)
        self._strings = fh.read(size_strings)
        self.root = self._parse(self.base + off_struct, self.base + off_struct + size_struct)

    def _u32(self, at):
        self.fh.seek(at)
        raw = self.fh.read(4)
        if len(raw) != 4:
            raise ImageError(f"file ends inside the FDT structure at offset {at}")
        return struct.unpack(">I", raw)[0]

    def _parse(self, pos, end):
        stack, root = [], None
        while pos < end:
            token = self._u32(pos)
            pos += 4
            if token == FDT_BEGIN_NODE:
                self.fh.seek(pos)
                raw = self.fh.read(min(256, end - pos))
                nul = raw.find(b"\x00")
                if nul < 0:
                    raise ImageError(f"unterminated node name at offset {pos}")
                name = raw[:nul].decode("ascii", "replace") or "/"
                pos += (nul + 1 + 3) & ~3
                parent = stack[-1] if stack else None
                node = Node(name, (parent.path + "/" + name).lstrip("/") if parent and parent.path else
                            ("" if parent is None else name))
                if parent is None:
                    if root is not None:
                        raise ImageError("FDT has more than one root node")
                    root = node
                else:
                    parent.children.append(node)
                stack.append(node)
                if len(stack) > MAX_DEPTH:
                    raise ImageError("FDT nesting is implausibly deep")
            elif token == FDT_END_NODE:
                if not stack:
                    raise ImageError(f"unbalanced FDT_END_NODE at offset {pos - 4}")
                stack.pop()
            elif token == FDT_PROP:
                if not stack:
                    raise ImageError(f"property outside any node at offset {pos - 4}")
                length = self._u32(pos)
                name_off = self._u32(pos + 4)
                pos += 8
                name_end = self._strings.find(b"\x00", name_off)
                if name_off >= len(self._strings) or name_end < 0:
                    raise ImageError(f"property name offset {name_off} outside the strings block")
                name = self._strings[name_off:name_end].decode("ascii", "replace")
                if pos + length > end:
                    raise ImageError(f"property {name} runs past the FDT structure block")
                stack[-1].props.append((name, pos, length))
                pos += (length + 3) & ~3
            elif token == FDT_NOP:
                continue
            elif token == FDT_END:
                break
            else:
                raise ImageError(f"unknown FDT token 0x{token:x} at offset {pos - 4}")
        if stack:
            raise ImageError("FDT structure ended inside a node")
        if root is None:
            raise ImageError("FDT has no root node")
        return root

    def read(self, offset, length):
        self.fh.seek(offset)
        data = self.fh.read(length)
        if len(data) != length:
            raise ImageError(f"file ends inside a value at offset {offset}")
        return data

    def u32_prop(self, node, name):
        found = node.prop(name)
        if not found or found[1] != 4:
            return None
        return struct.unpack(">I", self.read(*found))[0]

    def payload(self, node):
        """(offset, length, where) of a node's data: inline "data", or FIT external data."""
        inline = node.prop("data")
        if inline:
            return inline[0], inline[1], "inline"
        size = self.u32_prop(node, "data-size")
        position = self.u32_prop(node, "data-position")
        offset = self.u32_prop(node, "data-offset")
        if size is None or (position is None and offset is None):
            return None
        if position is not None:  # absolute, from the start of the FIT
            start = self.base + position
        else:  # FIT external data: offset from the end of the FDT, rounded up to 4 bytes
            start = self.base + ((self.total + 3) & ~3) + offset
        return start, size, "external"


AZ01_MAGIC = b"AZ01"
AZ01_MAX_COUNT = 256
AZ01_MAX_STRING = 4096
AZ01_MAX_RECORD_HEADER = 1 << 16
HASH_ALGOS = ("sha1", "sha256", "sha384", "sha512", "md5")


class Az01Part:
    """One PART record: where its data sits in the file, and what the record says about it."""

    def __init__(self, record_offset, header_len, name, compression, hashes, offset, length):
        self.record_offset, self.header_len = record_offset, header_len
        self.name, self.path, self.compression = name, name, compression
        self.hashes = hashes  # [(algo, expected bytes)]
        self.offset, self.length = offset, length


class Az01:
    """An "AZ01" update container: a header, then PART records until the end of the file.

    Layout (little-endian; every string is u32 length, the bytes, a NUL, then
    zero padding to a multiple of 4):
      "AZ01", u32 version, u32 header size,
      string build, u32 n + n strings (compatible), u32 n + n u32 (device IDs), string description
    then from the header size on, per record:
      4-byte tag, u32 record header size; for "PART": u64 data size, string name,
      string compression, u32 n + n x (string algo, u32 size, value), then the data.
    The layout is read from Akai's 3.10.0 image; anything else is reported, not guessed.
    """

    def __init__(self, fh, file_size):
        self.fh, self.file_size = fh, file_size
        head = self._read(0, 12)
        if head[:4] != AZ01_MAGIC:
            raise ImageError("not an AZ01 container")
        self.version, self.header_size = struct.unpack_from("<II", head, 4)
        if not 12 <= self.header_size <= min(file_size, AZ01_MAX_RECORD_HEADER):
            raise ImageError(f"AZ01 header size {self.header_size} is implausible")
        pos = 12
        self.build, pos = self._string(pos, self.header_size)
        count, pos = self._count(pos, self.header_size)
        self.compatible = []
        for _ in range(count):
            text, pos = self._string(pos, self.header_size)
            self.compatible.append(text)
        count, pos = self._count(pos, self.header_size)
        self.devices = list(struct.unpack(f"<{count}I", self._read(pos, 4 * count))) if count else []
        pos += 4 * count
        self.description, pos = self._string(pos, self.header_size)
        self.header_spare = self.header_size - pos
        if self.header_spare < 0:
            raise ImageError("AZ01 header fields run past the header size")
        self.parts, self.unknown, self.trailer = [], None, None
        pos = self.header_size
        while pos < file_size:
            if file_size - pos < 8:
                self.trailer = (pos, file_size - pos)
                break
            tag, header_len = self._read(pos, 4), struct.unpack("<I", self._read(pos + 4, 4))[0]
            if tag != b"PART":
                self.unknown = (pos, tag, header_len)
                break
            if not 16 <= header_len <= AZ01_MAX_RECORD_HEADER or pos + header_len > file_size:
                raise ImageError(f"PART record at offset {pos} has an implausible header size {header_len}")
            end = pos + header_len
            length = struct.unpack("<Q", self._read(pos + 8, 8))[0]
            at = pos + 16
            name, at = self._string(at, end)
            compression, at = self._string(at, end)
            count, at = self._count(at, end)
            hashes = []
            for _ in range(count):
                algo, at = self._string(at, end)
                size, at = self._count(at, end, limit=AZ01_MAX_STRING)
                hashes.append((algo, self._read(at, size)))
                at += (size + 3) & ~3
            if at != end:
                raise ImageError(f"PART record at offset {pos}: fields end at {at}, header says {end}")
            self.parts.append(Az01Part(pos, header_len, name, compression, hashes, end, length))
            if end + length > file_size:
                break
            pos = end + ((length + 3) & ~3)
            if pos > file_size:
                break

    def _read(self, offset, length):
        self.fh.seek(offset)
        data = self.fh.read(length)
        if len(data) != length:
            raise ImageError(f"file ends inside the AZ01 header at offset {offset}")
        return data

    def _count(self, pos, end, limit=AZ01_MAX_COUNT):
        if pos + 4 > end:
            raise ImageError(f"AZ01 field at offset {pos} runs past its block")
        value = struct.unpack("<I", self._read(pos, 4))[0]
        if value > limit:
            raise ImageError(f"AZ01 count {value} at offset {pos} is implausible")
        return value, pos + 4

    def _string(self, pos, end):
        length, pos = self._count(pos, end, limit=AZ01_MAX_STRING)
        stored = (length + 1 + 3) & ~3
        if pos + stored > end:
            raise ImageError(f"AZ01 string at offset {pos - 4} runs past its block")
        raw = self._read(pos, stored)
        if raw[length] != 0:
            raise ImageError(f"AZ01 string at offset {pos - 4} is not NUL-terminated")
        return raw[:length].decode("ascii", "replace"), pos + stored


def region_digests(fh, offset, length, algos):
    """sha256 plus each named algorithm over a file region, in one pass."""
    digests = {"sha256": hashlib.sha256()}
    for algo in algos:
        if algo in HASH_ALGOS and algo not in digests:
            digests[algo] = hashlib.new(algo)
    fh.seek(offset)
    left = length
    while left:
        block = fh.read(min(CHUNK, left))
        if not block:
            raise ImageError(f"file ends inside a payload at offset {offset + length - left}")
        for d in digests.values():
            d.update(block)
        left -= len(block)
    return {name: d.digest() for name, d in digests.items()}


def walk_az01(fh, file_size, out):
    az = Az01(fh, file_size)
    out.append(f"AZ01 container, format version {az.version}, header {az.header_size} bytes")
    out.append(f'  build = "{az.build}"')
    out.append("  compatible = " + ", ".join(f'"{c}"' for c in az.compatible))
    out.append("  device IDs = " + ", ".join(f"0x{d:08x} ({d >> 16:04x}:{d & 0xffff:04x})" for d in az.devices))
    out.append(f'  description = "{az.description}"')
    if az.header_spare:
        spare = az._read(az.header_size - az.header_spare, az.header_spare)
        out.append(f"  {az.header_spare} more header bytes: {spare[:64].hex()}")
    for part in az.parts:
        out.append(f"partition record at offset {part.record_offset} ({part.header_len}-byte header): {part.name}")
        out.append(f"  compression = \"{part.compression}\"")
        for algo, value in part.hashes:
            out.append(f"  hash {algo} = {value.hex()}")
        if part.offset + part.length > file_size:
            out.append(f"  data: offset {part.offset}, {part.length} bytes: PAST THE END OF THE FILE")
            continue
        kind, head = guess_region(fh, part.offset, part.length)
        digests = region_digests(fh, part.offset, part.length, [a for a, _ in part.hashes])
        out.append(f"  data: offset {part.offset}, {part.length} bytes, {kind}, starts {head}, "
                   f"sha256 {digests['sha256'].hex()}")
        for algo, value in part.hashes:
            if algo not in HASH_ALGOS:
                out.append(f"  {algo}: not a hash this tool knows, not checked")
            elif digests[algo] == value:
                out.append(f"  {algo} over the stored bytes: matches")
            else:
                out.append(f"  {algo} over the stored bytes: differs (it may cover the uncompressed data; "
                           f"offline-update-extract.py checks that)")
    if az.unknown:
        pos, tag, header_len = az.unknown
        rest = file_size - pos
        out.append(f"unknown record at offset {pos}: tag {tag.hex()} ({tag.decode('ascii', 'replace')!r}), "
                   f"header size field {header_len}, {rest} bytes to the end of the file")
        out.append(f"  first bytes: {az._read(pos, min(rest, 64)).hex()}")
    if az.trailer:
        pos, length = az.trailer
        out.append(f"trailing {length} bytes at offset {pos}: {az._read(pos, length).hex()}")
    if not az.parts:
        out.append("no partition records")


def walk(fh, file_size, out):
    fh.seek(0)
    if fh.read(4) == AZ01_MAGIC:
        walk_az01(fh, file_size, out)
        return
    fdt = Fdt(fh, file_size)
    base, total = fdt.base, fdt.total
    out.append(f"FDT header at offset {base}: version {fdt.version} (compatible with {fdt.last_comp}), "
               f"total size {total} bytes, file size {file_size} bytes")
    if base + total < file_size:
        kind, head = guess_region(fh, base + total, file_size - base - total)
        out.append(f"data after the FDT: {file_size - base - total} bytes ({kind}, starts {head})")
    if base:
        kind, head = guess_region(fh, 0, base)
        out.append(f"data before the FDT: {base} bytes ({kind}, starts {head}), "
                   f"sha256 {sha256_region(fh, 0, base)}")
    external = []

    def render(node, depth):
        out.append("  " * depth + (node.name if depth == 0 else node.name + "/"))
        for name, offset, length in node.props:
            out.append("  " * (depth + 1) + f"{name} = {show_value(fh, name, offset, length)}")
        for child in node.children:
            render(child, depth + 1)
        found = fdt.payload(node)
        if found and found[2] == "external":
            external.append((node.path or "/", found))

    render(fdt.root, 0)
    for path, (start, size, _) in external:
        if start + size > file_size:
            out.append(f"external data for {path}: offset {start}, {size} bytes: PAST THE END OF THE FILE")
            continue
        kind, head = guess_region(fh, start, size)
        out.append(f"external data for {path}: offset {start}, {size} bytes, {kind}, starts {head}, "
                   f"sha256 {sha256_region(fh, start, size)}")


def describe_image(path):
    out = [f"===== {path}"]
    size = os.path.getsize(path)
    out.append(f"size {size} bytes, sha256 {file_sha256(path)}")
    with open(path, "rb") as fh:
        walk(fh, size, out)
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
