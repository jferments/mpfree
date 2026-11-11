#!/usr/bin/env python3
"""Inspect RK3288 BootROM first-stage images OFFLINE (no hardware access).

Supports:
  idblock   - parse the RC4-obfuscated first-stage header in a private,
              whole-disk MPC One eMMC backup, at Rockchip sector 64.
  rkimage   - validate a local, owner-supplied candidate USB/SRAM SPL image,
              either plain ('RK32'...) or RC4-encoded for a ROM host tool.

This tool NEVER opens a block device, connects to USB/SSH, or writes any file.
Do not share the private eMMC image or its decrypted payload.

Format references: U-Boot tools/rkcommon.c and doc/README.rockchip.
"""
from __future__ import annotations

import argparse
import hashlib
import stat
import struct
import sys
from pathlib import Path

SECTOR = 512
IDBLOCK_LBA = 64
P1_SIZE = 128 * SECTOR
# U-Boot tools/rkcommon.h uses this default allowance when no separate boot file
# is supplied to the image builder. A header field equal to init+allowance
# does not prove a second stage occupies these sectors on the physical disk.
RK_MAX_BOOT_SIZE = 512 * 1024
RK_HEADER_MAGIC = 0x0FF0AA55
RK32 = b"RK32"
# Public Rockchip header-obfuscation key from upstream U-Boot.
ROCKCHIP_RC4_KEY = bytes.fromhex("7c4e0304550509072d2c7b38170d1711")
# Verified MPC One p1 partition hash (publicly recorded for this reference).
MPC_REFERENCE_P1_SHA256 = "aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec"


def rc4(data: bytes, key: bytes = ROCKCHIP_RC4_KEY) -> bytes:
    """Symmetric RC4 stream transform, used ONLY for offline header inspection."""
    state = list(range(256))
    j = 0
    for i in range(256):
        j = (j + state[i] + key[i % len(key)]) & 255
        state[i], state[j] = state[j], state[i]
    i = j = 0
    output = bytearray(len(data))
    for pos, b in enumerate(data):
        i = (i + 1) & 255
        j = (j + state[i]) & 255
        state[i], state[j] = state[j], state[i]
        output[pos] = b ^ state[(state[i] + state[j]) & 255]
    return bytes(output)


def regular_file(path: Path) -> Path:
    """Reject raw disks, sockets, pipes and symlinked targets."""
    mode = path.lstat().st_mode
    if not stat.S_ISREG(mode):
        raise ValueError("Expected an ordinary local file, not a device or symlink: %s" % path)
    return path


def inspect_idblock(backup: Path, expected_p1_sha256: str | None) -> None:
    regular_file(backup)
    offset = IDBLOCK_LBA * SECTOR
    if backup.stat().st_size < offset + P1_SIZE:
        raise ValueError("Backup too short for complete p1: %s" % backup)
    with backup.open("rb") as fh:
        fh.seek(offset)
        p1 = fh.read(P1_SIZE)
    if len(p1) != P1_SIZE:
        raise ValueError("Short read from backup p1")

    sha = hashlib.sha256(p1).hexdigest()
    print("Mode: offline Rockchip first-stage ID block")
    print("Source: locally selected regular file (never uploaded)")
    print("Sector LBA: %d, partition span: %d bytes" % (IDBLOCK_LBA, P1_SIZE))
    print("p1 SHA256: %s" % sha)
    if expected_p1_sha256 is not None:
        if sha != expected_p1_sha256.lower():
            raise ValueError("p1 SHA256 differs from expected baseline; inspect this backup separately")
        print("p1 baseline hash: VERIFIED")

    header = rc4(p1[:SECTOR])
    signature = struct.unpack_from("<I", header, 0)[0]
    if signature != RK_HEADER_MAGIC:
        raise ValueError(
            "Decrypted header signature mismatch: got 0x%08x, expected 0x%08x"
            % (signature, RK_HEADER_MAGIC)
        )
    disable_rc4 = struct.unpack_from("<I", header, 8)[0]
    init_offset = struct.unpack_from("<H", header, 12)[0]
    init_size = struct.unpack_from("<H", header, 506)[0]
    init_boot_size = struct.unpack_from("<H", header, 508)[0]
    payload_pos = init_offset * SECTOR
    payload_len = init_size * SECTOR

    if disable_rc4 not in (0, 1):
        raise ValueError("Unrecognized disable_rc4 field: %d" % disable_rc4)
    if not init_offset or not init_size:
        raise ValueError("Empty Rockchip image offset/size")
    if payload_pos + payload_len > len(p1):
        raise ValueError("Declared SPL extends beyond first-stage p1 partition")
    if init_boot_size < init_size:
        raise ValueError("init_boot_size smaller than init_size")
    nominal_boot_bytes = (init_boot_size - init_size) * SECTOR
    is_upstream_default_allowance = nominal_boot_bytes == RK_MAX_BOOT_SIZE
    # Rockchip's upstream tool requires a 2-KiB aligned init image.
    if init_size % 4 != 0:
        print("WARNING: Init size not 2-KiB aligned (vendor-specific image?)")

    first_block = p1[payload_pos:payload_pos + SECTOR]
    if not disable_rc4:
        # Rockchip rkcommon.c resets the cipher for each payload sector.
        first_block = rc4(first_block)
    candidate_magic = first_block[:4]
    print("Decoded header signature: 0x%08x (VALID)" % signature)
    print("Payload RC4: %s" % ("enabled" if disable_rc4 == 0 else "disabled"))
    print("Initial-loader offset: %d sectors (%d bytes from ID block)"
          % (init_offset, payload_pos))
    print("Declared init size: %d sectors (%d bytes)" % (init_size, payload_len))
    print("Declared init+boot size: %d sectors (%d bytes)"
          % (init_boot_size, init_boot_size * SECTOR))
    print("Nominal second-stage allowance: %d bytes (%d KiB)"
          % (nominal_boot_bytes, nominal_boot_bytes // 1024))
    if is_upstream_default_allowance:
        print("Interpretation: exactly matches the 512 KiB default allowance in")
        print("U-Boot tools/rkcommon.h; this is not proof that 512 KiB of")
        print("second-stage data exists contiguously after p1 on disk.")
    else:
        print("Interpretation: nondefault second-stage size metadata; actual")
        print("second-stage storage location still requires SPL analysis.")
    print("First-stage identifier: %s" %
          (candidate_magic.decode("ascii") if candidate_magic == RK32 else
           "nonstandard/not RK32 (raw bytes withheld)"))
    print("Interpretation: Rockchip BootROM ID header; no ROM or SDRAM execution proven.")
    print("OFFLINE ONLY; no file or hardware was modified.")


def inspect_rkimage(path: Path) -> None:
    regular_file(path)
    size = path.stat().st_size
    # 32 KiB RK3288 SPL envelope per U-Boot rkcommon.c, including the 4-byte RK32 header.
    if not 4 <= size <= 32768:
        raise ValueError("Unexpected RK3288 RKIMAGE size %d (must be 4..32768 bytes)" % size)
    raw = path.read_bytes()
    plain = raw[:4] == RK32
    wrapped = rc4(raw[:4]) == RK32
    print("Mode: offline candidate RK3288 ROM USB image")
    print("Local file SHA256: %s" % hashlib.sha256(raw).hexdigest())
    print("Length: %d bytes" % size)
    if plain:
        print("RK32 format: plain image (may require RC4 wrapping by USB host tool)")
    elif wrapped:
        print("RK32 format: RC4 encoded image (verify host protocol expectation)")
    else:
        raise ValueError("Neither plain nor RC4-encoded image begins with RK32")
    print("No image was loaded, executed, transmitted, altered or flashed.")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    idb = sub.add_parser("idblock", help="Inspect p1 at sector 64 of an existing OFFLINE eMMC backup")
    idb.add_argument("backup", type=Path)
    idb.add_argument("--expected-sha256", default=MPC_REFERENCE_P1_SHA256,
                     help="Expected 64-KiB p1 SHA256; use 'none' for non-reference boards")
    rk = sub.add_parser("rkimage", help="Inspect a local USB-ROM SPL candidate (not upload it)")
    rk.add_argument("image", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "idblock":
            expect = None if args.expected_sha256.lower() == "none" else args.expected_sha256
            inspect_idblock(args.backup, expect)
        else:
            inspect_rkimage(args.image)
    except (OSError, ValueError, struct.error) as exc:
        print("ERROR: %s" % exc, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
