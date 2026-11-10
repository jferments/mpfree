#!/usr/bin/env python3
"""OFFLINE, workstation only: read the build configuration embedded in an ARM zImage.

Kernels built with CONFIG_IKCONFIG carry their .config, gzip-compressed,
between the markers IKCFG_ST and IKCFG_ED. In a zImage that block sits
inside the compressed kernel, so this finds the compressed payload,
decompresses it in memory and pulls the config out. It is the same method
as the kernel's scripts/extract-ikconfig, without needing a kernel tree.

gzip, xz and lzma payloads are handled with Python's standard library, and
lz4 (the kernel's "legacy" lz4 format) by the lz4 command if it is installed,
else (or if the command fails) by a small decoder here. lzo and zstd payloads
need the lzop or zstd command. Commands are fed from memory and nothing is written to disk.

It only reads the file it is given and prints to stdout.

Usage:
  python3 tools/mpc-one/offline-kernel-config.py ZIMAGE            print the whole config
  python3 tools/mpc-one/offline-kernel-config.py ZIMAGE --select    print only the symbols
                                                                     that matter for a kexec boot
"""
import lzma
import re
import shutil
import subprocess
import sys
import zlib

MAX_KERNEL = 128 << 20  # refuse to inflate anything bigger than this
MARK_START, MARK_END = b"IKCFG_ST", b"IKCFG_ED"
GZIP_MAGIC = b"\x1f\x8b\x08"

# (magic, name, how to decompress). Searched in this order at every offset where the magic occurs.
PAYLOADS = (
    (b"\x1f\x8b\x08", "gzip", "zlib"),
    (b"\xfd7zXZ\x00", "xz", "xz"),
    (b"\x5d\x00\x00", "lzma", "lzma"),
    (b"\x02\x21\x4c\x18", "lz4", ("lz4", "-dc")),
    (b"\x89\x4c\x5a\x4f\x00\x0d\x0a\x1a\x0a", "lzo", ("lzop", "-dc")),
    (b"\x28\xb5\x2f\xfd", "zstd", ("zstd", "-dc")),
)

SELECT = re.compile(
    r"^(# )?CONFIG_("
    r"LOCALVERSION|PREEMPT_RT|SMP|NR_CPUS|HOTPLUG_CPU|HIGHMEM|VMSPLIT_[0-9A-Z_]+|"
    r"KEXEC[A-Z_]*|"
    r"SECURITY_LOADPIN[A-Z_]*|LSM|MODULES|MODULE_SIG[A-Z_]*|"
    r"BLK_DEV_INITRD|RD_[A-Z0-9]+|INITRAMFS_SOURCE|"
    r"USB_STORAGE|USB_UAS|SCSI|BLK_DEV_SD|USB_DWC2[A-Z_]*|USB_EHCI[A-Z_]*|USB_OHCI[A-Z_]*|"
    r"USB_XHCI[A-Z_]*|PHY_ROCKCHIP_USB|USB_ONBOARD_DEV|"
    r"EXT4_FS|VFAT_FS|TMPFS|DEVTMPFS[A-Z_]*|OVERLAY_FS|"
    r"MMC_DW[A-Z_]*|MMC_BLOCK|PROC_SYSCTL|STRICT_DEVMEM|IO_STRICT_DEVMEM|DEVMEM|OF_OVERLAY|"
    r"ARM_APPENDED_DTB|ARM_ATAG_DTB_COMPAT|CMDLINE[A-Z_]*|IKCONFIG[A-Z_]*|"
    r"STMMAC_ETH|DWMAC_ROCKCHIP|DRM_ROCKCHIP|ROCKCHIP_VOP[A-Z0-9_]*"
    r")(=| is not set)"
)


class ConfigError(Exception):
    pass


LZ4_LEGACY_MAGIC = 0x184C2102
LZ4_LEGACY_BLOCK = 8 << 20
LZ4_LEGACY_BOUND = LZ4_LEGACY_BLOCK + (LZ4_LEGACY_BLOCK >> 7) + 64  # largest compressed block


def unlz4_block(src, out):
    """Append one decoded LZ4 block to out (bytearray). Raises ValueError on damaged input."""
    i, n, base = 0, len(src), len(out)
    while i < n:
        token = src[i]
        i += 1
        length = token >> 4
        if length == 15:
            while True:
                extra = src[i]
                i += 1
                length += extra
                if extra != 255:
                    break
        if i + length > n:
            raise ValueError("literals run past the block")
        out += src[i:i + length]
        i += length
        if i >= n:  # the last sequence has literals only
            break
        offset = src[i] | (src[i + 1] << 8)
        i += 2
        length = token & 15
        if length == 15:
            while True:
                extra = src[i]
                i += 1
                length += extra
                if extra != 255:
                    break
        length += 4
        start = len(out) - offset
        if offset == 0 or start < base:
            raise ValueError("match offset outside the block")
        if offset >= length:
            out += out[start:start + length]
        else:  # the match overlaps the bytes it produces: repeat the pattern
            pattern = bytes(out[start:])
            out += (pattern * (length // offset + 1))[:length]
        if len(out) - base > LZ4_LEGACY_BLOCK:
            raise ValueError("block decodes to more than 8 MiB")


def lz4_legacy_blocks(data):
    """(start, size) of each block of the lz4 'legacy' stream (what the kernel's lz4
    compression writes) at the start of data, or [] if data doesn't start with one.

    The format has no end marker. The stream ends where the next 4 bytes can't be a
    block size (zero, too big, or longer than what is left): in a zImage that is the
    kernel's appended length.
    """
    if len(data) < 4 or int.from_bytes(data[:4], "little") != LZ4_LEGACY_MAGIC:
        return []
    blocks, pos = [], 4
    while pos + 4 <= len(data):
        size = int.from_bytes(data[pos:pos + 4], "little")
        if size == LZ4_LEGACY_MAGIC:  # another stream follows
            pos += 4
            continue
        if size == 0 or size > len(data) - pos - 4 or size > LZ4_LEGACY_BOUND:
            break
        blocks.append((pos + 4, size))
        pos += 4 + size
    return blocks


def unlz4_legacy(data):
    """Decode an lz4 'legacy' stream. Trailing bytes, and a damaged block with everything after it, are ignored."""
    out = bytearray()
    for start, size in lz4_legacy_blocks(data):
        try:
            unlz4_block(data[start:start + size], out)
        except (ValueError, IndexError):
            break
        if len(out) > MAX_KERNEL:
            return None
    return bytes(out) if out else None


def inflate_lz4(tail):
    """Decompress the lz4 legacy stream at the start of tail, or None."""
    blocks = lz4_legacy_blocks(tail)
    if not blocks:
        return None
    start, size = blocks[-1]
    stream = tail[:start + size]
    tool = shutil.which("lz4")
    if tool:
        # Only the stream itself goes to lz4, so it ends cleanly at a block boundary. Given
        # the bytes after it, lz4 stops with a read error, and lz4 1.10 can then exit before
        # its writer thread has written the blocks it already decoded (seen in CI as a
        # missing config). Its output is used only if it exits 0.
        run = subprocess.run((tool, "-dc"), input=stream, capture_output=True, check=False)
        if run.returncode == 0 and run.stdout:
            return run.stdout
    return unlz4_legacy(stream)


def inflate(data, offset, how):
    """Decompress the stream that starts at data[offset:]; trailing bytes are ignored."""
    tail = data[offset:]
    if how == "zlib":
        d = zlib.decompressobj(16 + zlib.MAX_WBITS)
        out = d.decompress(tail, MAX_KERNEL)
        return out if d.eof else None
    if how in ("xz", "lzma"):
        d = lzma.LZMADecompressor(lzma.FORMAT_XZ if how == "xz" else lzma.FORMAT_ALONE)
        out = d.decompress(tail, MAX_KERNEL)
        return out if d.eof else None
    if how[0] == "lz4":
        return inflate_lz4(tail)
    tool = shutil.which(how[0])
    if not tool:
        raise ConfigError(f"the payload looks {how[0]}-compressed; install the '{how[0]}' command")
    run = subprocess.run((tool,) + how[1:], input=tail, capture_output=True, check=False)
    # lzop and zstd report trailing garbage as an error but still write the whole stream first.
    # The config's own gzip CRC and end marker are checked afterwards, so a cut-short
    # output can only mean "not found", never a wrong config.
    return run.stdout if run.stdout else None


def config_in(blob):
    """The text of the config embedded in blob, or None. The gzip CRC and the end marker are checked."""
    start = blob.find(MARK_START + GZIP_MAGIC)
    if start < 0:
        return None
    d = zlib.decompressobj(16 + zlib.MAX_WBITS)
    try:
        text = d.decompress(blob[start + len(MARK_START):], 4 << 20)
    except zlib.error as err:
        raise ConfigError(f"the embedded config is damaged ({err})") from None
    if not d.eof or not d.unused_data.startswith(MARK_END):
        raise ConfigError("the embedded config is cut short or not followed by IKCFG_ED")
    return text.decode("utf-8", "replace")


def extract(data):
    """(where the config was found, config text) for a zImage, an Image or a vmlinux.

    Compressed payloads are tried first. Only if none holds a config is the file
    itself searched, which is right for an uncompressed Image or vmlinux.
    """
    problems = []
    for magic, name, how in PAYLOADS:
        at = data.find(magic)
        tries = 0
        while at >= 0 and tries < 64:
            tries += 1
            try:
                kernel = inflate(data, at, how)
            except ConfigError as err:
                problems.append(str(err))
                break
            except (zlib.error, lzma.LZMAError, EOFError):
                kernel = None
            if kernel:
                try:
                    found = config_in(kernel)
                except ConfigError as err:
                    problems.append(f"{name} payload at offset {at}: {err}")
                    found = None
                if found is not None:
                    return f"{name} payload at offset {at}", found
            at = data.find(magic, at + 1)
    try:
        found = config_in(data)
    except ConfigError as err:
        problems.append(f"uncompressed: {err}")
        found = None
    if found is not None:
        return "uncompressed, in the file itself", found
    if problems:
        raise ConfigError("; ".join(dict.fromkeys(problems)))
    raise ConfigError("no embedded config found (kernel built without CONFIG_IKCONFIG?)")


def main(argv):
    args = [a for a in argv[1:] if a != "--select"]
    if len(args) != 1 or len(argv) > 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    with open(args[0], "rb") as fh:
        data = fh.read(MAX_KERNEL + 1)
    if len(data) > MAX_KERNEL:
        print(f"ERROR: {args[0]} is larger than {MAX_KERNEL} bytes", file=sys.stderr)
        return 1
    try:
        compression, text = extract(data)
    except ConfigError as err:
        print(f"ERROR: {err}", file=sys.stderr)
        return 1
    lines = text.splitlines()
    if "--select" in argv:
        print(f"(config found: {compression}; {len(lines)} lines; selected symbols follow)")
        lines = [line for line in lines if SELECT.match(line)]
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
