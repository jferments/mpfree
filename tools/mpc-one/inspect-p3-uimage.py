#!/usr/bin/env python3
"""Inspect the legacy U-Boot uImage at LBA 256 in an OFFLINE disk backup.

Regular files only; no device/symlink input, writes, SSH, USB, eFuse operations,
or proprietary payload export. Validate both header and payload CRC32 and show
the true load/entry addresses required for reproducing the SPL handoff.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import stat
import struct
import zlib
from pathlib import Path

SECTOR = 512
P3_LBA = 256
P3_SECTORS = 768
UIMAGE_HEADER_SIZE = 64
UIMAGE_MAGIC = 0x27051956
STRUCT = struct.Struct(">7I4B32s")


class ImageValidationError(ValueError):
    """A regular-file backup does not contain a valid p3 legacy image."""


def open_regular_readonly(path: Path):
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ImageValidationError("Source must be a regular file, not a device")
        return os.fdopen(fd, "rb", closefd=True)
    except BaseException:
        os.close(fd)
        raise


def inspect(path: Path) -> dict:
    with open_regular_readonly(Path(path)) as source:
        file_size = os.fstat(source.fileno()).st_size
        offset = P3_LBA * SECTOR
        if file_size < offset + UIMAGE_HEADER_SIZE:
            raise ImageValidationError("Input too short for sector-256 uImage header")
        source.seek(offset)
        header = source.read(UIMAGE_HEADER_SIZE)
        if len(header) != UIMAGE_HEADER_SIZE:
            raise ImageValidationError("Short uImage header read")
        (magic, hcrc, timestamp, payload_size, load, entry, dcrc,
         os_id, arch_id, type_id, comp_id, name_bytes) = STRUCT.unpack(header)
        if magic != UIMAGE_MAGIC:
            raise ImageValidationError(
                f"Not a legacy U-Boot uImage at LBA 256: 0x{magic:08x}")
        capacity = P3_SECTORS * SECTOR - UIMAGE_HEADER_SIZE
        if payload_size == 0 or payload_size > capacity:
            raise ImageValidationError(
                f"Declared payload size {payload_size} exceeds p3 payload capacity {capacity}")
        if file_size < offset + UIMAGE_HEADER_SIZE + payload_size:
            raise ImageValidationError("Declared payload extends past end of file")
        hcrc_calculated = zlib.crc32(
            header[:4] + b"\x00" * 4 + header[8:]) & 0xFFFFFFFF
        remaining = payload_size
        data_crc = 0
        hasher = hashlib.sha256()
        while remaining:
            block = source.read(min(65536, remaining))
            if not block:
                raise ImageValidationError("Unexpected end of image payload")
            remaining -= len(block)
            data_crc = zlib.crc32(block, data_crc)
            hasher.update(block)
        data_crc &= 0xFFFFFFFF
    return {
        "schema": "mpfree-offline-p3-uimage-v1",
        "source_kind": "local regular file; never uploaded",
        "p3_start_lba": P3_LBA,
        "p3_partition_sectors": P3_SECTORS,
        "uimage_header_bytes": UIMAGE_HEADER_SIZE,
        "magic": f"0x{magic:08x}",
        "payload_size_bytes": payload_size,
        "total_image_bytes": UIMAGE_HEADER_SIZE + payload_size,
        "load_address": f"0x{load:08x}",
        "entry_address": f"0x{entry:08x}",
        "timestamp_utc": dt.datetime.fromtimestamp(
            timestamp, tz=dt.timezone.utc).isoformat(),
        "os_id": os_id, "arch_id": arch_id,
        "image_type_id": type_id, "compression_id": comp_id,
        "image_name": name_bytes.split(b"\x00", 1)[0].decode(
            "utf-8", errors="replace"),
        "declared_header_crc32": f"0x{hcrc:08x}",
        "calculated_header_crc32": f"0x{hcrc_calculated:08x}",
        "header_crc32_valid": hcrc == hcrc_calculated,
        "declared_payload_crc32": f"0x{dcrc:08x}",
        "calculated_payload_crc32": f"0x{data_crc:08x}",
        "payload_crc32_valid": dcrc == data_crc,
        "payload_sha256": hasher.hexdigest(),
        "security_note": (
            "CRC32 detects corruption; it is not a cryptographic signature. "
            "This inspection does not prove BootROM acceptance or image execution."),
        "offline_only": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "backup", type=Path, help="Existing whole-disk backup regular file")
    parser.add_argument(
        "--require-crc", action="store_true",
        help="Exit nonzero if header or payload CRC fails")
    args = parser.parse_args()
    try:
        report = inspect(args.backup)
    except (OSError, ValueError, OverflowError) as exc:
        parser.exit(2, f"MPFree p3 inspection FAILED: {exc}\n")
    print(json.dumps(report, indent=2))
    return 0 if (
        not args.require_crc or (
            report["header_crc32_valid"] and report["payload_crc32_valid"]
        )
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
