#!/usr/bin/env python3
"""Extract only the first 32-KiB RK3288 SRAM stage for offline Ghidra work.

This file is a *host-only local exporter*. It never connects to an MPC, never
opens a raw block device, and refuses to overwrite an existing destination.
The private extracted proprietary loader must remain off GitHub.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import os
import stat
import struct
import sys
from pathlib import Path

INSPECTOR = Path(__file__).with_name("offline-rk3288-rom.py")
spec = importlib.util.spec_from_file_location("mpfree_rk3288_inspector", INSPECTOR)
rk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rk)


def extract(source: Path, target: Path) -> None:
    source = rk.regular_file(source)
    offset = rk.IDBLOCK_LBA * rk.SECTOR
    if source.stat().st_size < offset + rk.P1_SIZE:
        raise ValueError("Backup does not contain the full p1 region")
    with source.open("rb") as f:
        f.seek(offset)
        data = f.read(rk.P1_SIZE)
    if len(data) != rk.P1_SIZE:
        raise ValueError("Short read of p1 in backup")
    actual_p1_sha = hashlib.sha256(data).hexdigest()
    if actual_p1_sha != rk.MPC_REFERENCE_P1_SHA256:
        raise ValueError("Wrong p1 SHA256: %s; refusing extraction" % actual_p1_sha)

    header = rk.rc4(data[:rk.SECTOR])
    signature, disable_rc4 = struct.unpack_from("<I4xI", header, 0)
    init_offset = struct.unpack_from("<H", header, 12)[0]
    init_size = struct.unpack_from("<H", header, 506)[0]
    init_boot_size = struct.unpack_from("<H", header, 508)[0]
    if signature != rk.RK_HEADER_MAGIC:
        raise ValueError("Invalid decoded Rockchip header")
    if disable_rc4 != 1:
        raise ValueError("Unexpected encrypted first-stage payload")
    if init_offset != 4 or init_size != 64 or init_boot_size != 1088:
        raise ValueError("MPC One p1 metadata differs from verified baseline")
    stage_offset = init_offset * rk.SECTOR
    stage_size = init_size * rk.SECTOR
    stage = data[stage_offset:stage_offset + stage_size]
    if len(stage) != 32768 or stage[:4] != rk.RK32:
        raise ValueError("Expected 32-KiB RK32 SRAM stage missing")
    stage_sha256 = hashlib.sha256(stage).hexdigest()
    target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    # No symlink handling ambiguity, no accidental overwrites.
    if target.exists() or target.is_symlink():
        raise ValueError("Destination exists; select a new output path: %s" % target)
    fd = os.open(str(target), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "wb") as dest:
            dest.write(stage)
    except Exception:
        try:
            target.unlink()
        except OSError:
            pass
        raise

    print("Source p1: baseline SHA256 VERIFIED")
    print("Created local private file: %s" % target)
    print("Payload size: %d bytes" % len(stage))
    print("Payload RK32 magic: confirmed")
    print("SPL stage SHA256: %s" % stage_sha256)
    print("Ghidra ARMv7 raw import base: 0xFF704000")
    print("Candidate first-instruction address: 0xFF704004")
    print("NO writes to eMMC, USB, physical memory or firmware")
    print("Do not distribute extracted vendor binary.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("backup", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        extract(args.backup, args.output)
        return 0
    except (OSError, ValueError, struct.error) as exc:
        print("ERROR: %s" % exc, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
