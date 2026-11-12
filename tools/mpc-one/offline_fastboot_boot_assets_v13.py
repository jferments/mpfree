#!/usr/bin/env python3
"""V13 read-only verification of private MPC 3.10 /boot script, FDT and kernel ABI.

Uses debugfs -R cat on a regular-file ext4 image (NO mount, NO live devices).
Writes only JSON to a private output directory. Does not execute vendor code,
change the input filesystem, or publish proprietary firmware artifacts.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import zlib

MAGIC_UIMAGE = 0x27051956
MAGIC_FDT = 0xd00dfeed
MAX_INPUT = 2 * 1024 * 1024 * 1024
BOOT_FILES = [
    "/boot/boot.scr.uimg",
    "/boot/rk3288-az01-acva.dtb",
    "/boot/rk3288-az01-acva-c.dtb",
    "/boot/zImage-6.18.44-az01-2026-08-10-rt6",
]
QUERY_PATHS = [
    "/boot",
    "/lib/modules",
    "/etc/systemd/system",
    "/etc/systemd/system/sysinit.target.wants",
    "/etc/systemd/system/multi-user.target.wants",
]

def regular(path):
    path = Path(path).expanduser()
    if (path.is_symlink() or not path.is_file() or
            not stat.S_ISREG(path.lstat().st_mode) or
            str(path).startswith(("/dev/", "/proc/", "/sys/", "/run/media/"))):
        raise ValueError("Require a regular-file image, not a device: " + str(path))
    if path.stat().st_size > MAX_INPUT:
        raise ValueError("Unexpectedly large input; require explicit offline inspection")
    with path.open("rb") as handle:
        handle.seek(1080)
        if handle.read(2) != b"\x53\xef":
            raise ValueError("No ext4 filesystem superblock")
    return path

def debugfs_read(image, expression, max_bytes=12_000_000):
    if not shutil.which("debugfs"):
        raise RuntimeError("debugfs is not installed")
    p = subprocess.run(["debugfs", "-R", expression, str(image)],
                       capture_output=True, timeout=45, check=False)
    if p.returncode:
        raise RuntimeError("debugfs failure for requested read-only operation: " + expression)
    if len(p.stdout) > max_bytes:
        raise RuntimeError("Unexpectedly large debugfs output for " + expression)
    return p.stdout

def checksum(data):
    return hashlib.sha256(data).hexdigest()

def uimage(data):
    if len(data) < 64: return {"valid": False, "reason": "image shorter than header"}
    values = struct.unpack(">7I4B32s", data[:64])
    magic, header_crc, epoch, data_size, load, entry, payload_crc = values[:7]
    os_code, arch_code, type_code, compress_code = values[7:11]
    name = values[11].split(b"\x00")[0].decode("ascii", "replace")
    payload = data[64:64 + data_size]
    head = bytearray(data[:64])
    head[4:8] = b"\x00" * 4
    result = {
        "valid": magic == MAGIC_UIMAGE and len(payload) == data_size,
        "uimage_magic": "%08x" % magic,
        "header_crc_valid": (zlib.crc32(head) & 0xffffffff) == header_crc,
        "payload_crc_valid": (zlib.crc32(payload) & 0xffffffff) == payload_crc,
        "timestamp": epoch,
        "data_size": data_size, "load_address": "0x%08x" % load,
        "entry_address": "0x%08x" % entry,
        "image_os": os_code, "arch": arch_code, "type": type_code,
        "compression": compress_code,
        "description": name[:64],
        "payload_sha256": checksum(payload),
        "script": None,
    }
    if type_code == 6 and data_size >= 8:
        n = struct.unpack(">I", payload[:4])[0]
        terminator = struct.unpack(">I", payload[4:8])[0]
        body = payload[8:8+n]
        result["script"] = {
            "expected_bytes": n, "zero_length_terminator": terminator == 0,
            "actual_bytes": len(body), "sha256": checksum(body),
        }
        if n <= 8192 and len(body) == n:
            contents = body.decode("utf-8", "replace")
            # Avoid exposing unique owner identifiers or auth data.
            contents = re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+",
                              "inmusic-unlock-magic-[REDACTED]", contents)
            result["script"]["normalized_command_lines"] = [
                line.strip() for line in contents.splitlines()
                if line.strip() and not line.lstrip().startswith("#")
            ][:90]
    return result

def fdt_properties(data, max_properties=15000):
    if len(data) < 40: return {"valid": False, "reason": "FDT header missing"}
    header = struct.unpack(">10I", data[:40])
    magic, total, struct_off, strings_off, reserve_off, version, last, bootcpuid, strings_size, struct_size = header
    output = {
        "valid": magic == MAGIC_FDT and 40 <= total <= len(data),
        "header": {
            "magic": "0x%08x" % magic, "totalsize": total,
            "version": version, "last_compatible_version": last,
            "struct_offset": struct_off, "strings_offset": strings_off,
            "strings_size": strings_size, "struct_size": struct_size,
        },
        "selected_properties": {}, "nodes": 0, "properties": 0,
    }
    if not output["valid"] or (struct_off + struct_size > total or
                               strings_off + strings_size > total):
        return output
    pos = struct_off
    end = struct_off + struct_size
    names = data[strings_off:strings_off + strings_size]
    stack = []
    selected_re = re.compile(
        r"(?:^/$|chosen|aliases|reserved.memory|memory@|mmc|sdmmc|"
        r"dwmmc|reboot.mode|pinctrl|serial|ethernet|usb@ff580000|sound)",
        re.I,
    )
    while pos + 4 <= end and output["properties"] < max_properties:
        token = struct.unpack_from(">I", data, pos)[0]
        pos += 4
        if token == 1:
            zero = data.find(b"\x00", pos, end)
            if zero < 0: raise ValueError("Unterminated device-tree node name")
            stack.append(data[pos:zero].decode("utf-8", "replace"))
            pos = (zero + 4) & ~3
            output["nodes"] += 1
        elif token == 2:
            if not stack: raise ValueError("Extra FDT END_NODE")
            stack.pop()
        elif token == 3:
            if pos + 8 > end: raise ValueError("Truncated FDT property")
            size, offset = struct.unpack_from(">II", data, pos)
            pos += 8
            if size > end - pos or offset >= len(names):
                raise ValueError("Invalid FDT property range/name")
            terminator = names.find(b"\x00", offset)
            if terminator < 0: raise ValueError("Unterminated FDT property name")
            key = names[offset:terminator].decode("utf-8", "replace")
            value = data[pos:pos + size]
            pos = (pos + size + 3) & ~3
            output["properties"] += 1
            path = "/" + "/".join(x for x in stack if x)
            if selected_re.search(path) and key in (
                "compatible", "status", "reg", "bootargs", "stdout-path",
                "fdtfile", "serial-number", "boot-targets", "boot_targets",
                "mode-normal", "mode-loader", "mode-recovery",
                "mode-bootloader", "mode-update", "label", "clock-frequency",
                "rotation", "device_type", "aliases", "inmusic,az01-pcb-rev",
                "enable-method", "memory-region", "non-removable",
                "bus-width", "max-frequency", "pinctrl-names",
            ):
                if key in ("serial-number", "bootargs", "inmusic,az01-pcb-rev"):
                    representation = {"redacted": True, "length": len(value),
                                      "sha256": checksum(value)}
                elif value and all((32 <= x <= 126 or x == 0) for x in value):
                    representation = [x for x in value.decode("ascii", "replace").split("\x00") if x][:24]
                elif size <= 48 and size % 4 == 0:
                    representation = ["0x%08x" % n for n in struct.unpack(">" + "I" * (size // 4), value)]
                else:
                    representation = {"size": size, "sha256": checksum(value)}
                output["selected_properties"].setdefault(path or "/", {})[key] = representation
        elif token == 4:
            pass
        elif token == 9:
            break
        else:
            raise ValueError("Unrecognized FDT token " + str(token))
    return output

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("image", type=Path)
    ap.add_argument("destination", type=Path)
    args = ap.parse_args()
    image = regular(args.image)
    if not args.destination.is_absolute() or args.destination.is_symlink():
        ap.error("Require absolute output file, not symlink")
    report = {"schema": "mpfree-v13-offline-boot-assets", "source_image_size": image.stat().st_size,
              "read_only": True, "files": {}, "directories": {},
              "limitations": [
                  "FDT template does not include all U-Boot runtime fixups.",
                  "An intact vendor kernel and script do not demonstrate an owner kernel boot.",
                  "No physical storage or vendor executable was accessed.",
              ]}
    for folder in QUERY_PATHS:
        try:
            data = debugfs_read(image, "ls -l " + folder, 2_000_000)
            report["directories"][folder] = {
                "listing": data.decode("utf-8", "replace")[:12000],
                "sha256": checksum(data),
            }
        except Exception as e:
            report["directories"][folder] = {"error": str(e)}
    for filename in BOOT_FILES:
        try:
            data = debugfs_read(image, "cat " + filename)
            entry = {"bytes": len(data), "sha256": checksum(data)}
            if filename.endswith(".uimg"): entry["uimage"] = uimage(data)
            if filename.endswith(".dtb"): entry["dtb"] = fdt_properties(data)
            if "zImage-" in filename:
                entry["kernel_arm_zimage_magic"] = (
                    len(data) >= 0x28 and data[0x24:0x28] == b"\x18\x28\x6f\x01")
                entry["kernel_size"] = len(data)
            report["files"][filename] = entry
        except Exception as e:
            report["files"][filename] = {"error": str(e)}
    old = os.umask(0o077)
    try:
        args.destination.parent.mkdir(parents=True, exist_ok=True)
        args.destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    finally:
        os.umask(old)
    print("V13 read-only /boot verification:", args.destination,
          "files inspected:", len(report["files"]))

if __name__ == "__main__":
    main()
