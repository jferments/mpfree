#!/usr/bin/env python3
"""Offline, read-only MPFree boot-artifact cross-check.

Never opens block devices, only an existing regular-file owner backup.
Copies p6 to a temporary regular file only to let debugfs read its ext4
contents (no mount, no loop device, no on-device operation).
All reports are PRIVATE. Personal variables / unlock tokens are redacted.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import tempfile
import zlib
from datetime import datetime, timezone
from pathlib import Path

SECTOR = 512
P1 = (64, 128)
P2 = (192, 16)
P3 = (256, 768)
P6 = (17408, 1024000)
EXPECTED_P1 = "aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec"
EXPECTED_P3_PAYLOAD = "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
SENSITIVE = re.compile(r"serial|ethaddr|macaddr|uuid|partuuid|unlock|secret|password|token|key|credential|guid", re.I)
INTERESTING = re.compile(r"^(?:boot|az01_|distro_|scan_|script|kernel|fdt|ramdisk|stdin|stdout|stderr|"
                         r"mmc|usb|loadaddr|dorecovery|recovery|root|devtype|devnum|"
                         r"console|baudrate|partitions|preboot|verify|secure|fastboot)", re.I)
DTB_HINTS = ("reboot-mode", "mode-bootloader", "mode-fastboot", "mode-recovery",
             "mode-maskrom", "rockchip,rk3288", "usb@ff580000",
             "usb@ff540000", "gpio-keys", "efuse", "otp", "vqps",
             "serial@ff690000", "dwmmc", "mmc@", "chosen", "pinctrl")

def region(file, partition, length=None):
    lba, count = partition
    size = count * SECTOR if length is None else length
    file.seek(lba * SECTOR)
    data = file.read(size)
    if len(data) != size:
        raise RuntimeError("Truncated backup at LBA %s" % lba)
    return data

def valid_env(raw):
    if len(raw) < 16:
        return {"valid": False, "reason": "Too small"}
    expected_le = struct.unpack_from("<I", raw)[0]
    expected_be = struct.unpack_from(">I", raw)[0]
    actual = zlib.crc32(raw[4:]) & 0xffffffff
    # Standard single-copy U-Boot env layout. A custom/redundant
    # layout will not match and must be investigated separately.
    valid = actual in (expected_le, expected_be)
    body = raw[4:]
    entries = body.split(b"\0\0", 1)[0].split(b"\0")
    values = {}
    names = []
    for entry in entries:
        if not entry or b"=" not in entry:
            continue
        kb, vb = entry.split(b"=", 1)
        key = kb.decode("ascii", "replace")
        names.append(key if not SENSITIVE.search(key) else "[private variable]")
        if SENSITIVE.search(key):
            continue
        if INTERESTING.search(key):
            value = vb.decode("utf-8", "replace")
            if SENSITIVE.search(value):
                # Do not leak PARTUUIDs or embedded access tokens.
                value = "[redacted: sensitive variable contents]"
            values[key] = value[:2500]
    return {"format_assumption": "standard U-Boot environment CRC32 + NUL-separated key=value",
            "header_crc_le": "%08x" % expected_le,
            "header_crc_be": "%08x" % expected_be,
            "computed_crc": "%08x" % actual,
            "crc_valid_for_standard_layout": valid,
            "variable_names_redacted": names[:300], "boot_related_variables": values}

def parse_uimage(data):
    if len(data) < 64:
        raise RuntimeError("p3 truncated")
    magic, hcrc, timestamp, size, load, entry, dcrc = struct.unpack(">7I", data[:28])
    if magic != 0x27051956:
        raise RuntimeError("No legacy U-Boot uImage at p3 LBA256")
    if size + 64 > len(data):
        raise RuntimeError("uImage exceeds p3")
    header = bytearray(data[:64])
    header[4:8] = bytes(4)
    payload = data[64:64+size]
    c_h = zlib.crc32(header) & 0xffffffff
    c_d = zlib.crc32(payload) & 0xffffffff
    sha = hashlib.sha256(payload).hexdigest()
    if (hcrc, dcrc) != (c_h, c_d) or sha != EXPECTED_P3_PAYLOAD:
        raise RuntimeError("p3 header/payload integrity check failed")
    return {"valid": True, "size_bytes": size, "load_address": "0x%08x" % load,
            "entry_address": "0x%08x" % entry, "header_crc32_valid": True,
            "payload_crc32_valid": True, "sha256": sha,
            "name": data[32:64].split(b"\0")[0].decode("ascii", "replace"),
            "legacy_type_id": data[30], "arch_id": data[29],
            "sectors_used": (64 + size + 511) // 512,
            "warning": "Valid CRC32 is not a cryptographic signature."}

def embedded_uboot_environment_candidates(raw_image, saved_env):
    """Locate likely compiled-in defaults in the verified U-Boot payload.

    This is heuristic; the variables might be near command help text,
    so a candidate is NEVER asserted to be the active runtime env.
    Only a bounded NUL-separated region after the known default bootcmd
    marker is considered, and sensitive values are redacted.
    """
    if raw_image[:4] != b"\x27\x05\x19\x56":
        return {"status": "unrecognized image"}
    size = struct.unpack(">I", raw_image[12:16])[0]
    payload = raw_image[64:64+size]
    marker = b"bootcmd=run az01_bootcmd"
    at = payload.find(marker)
    if at == -1:
        return {"status": "no candidate default bootcmd in verified payload"}
    after = payload[at:at+16384]
    end = after.find(b"\0\0")
    if end < 0:
        return {"status": "candidate boundary absent", "offset": at}
    values = {}
    for item in after[:end].split(b"\0"):
        if b"=" not in item: continue
        kb, vb = item.split(b"=", 1)
        key = kb.decode("ascii", "replace")
        value = vb.decode("utf-8", "replace")
        if not INTERESTING.search(key) or SENSITIVE.search(key) or SENSITIVE.search(value):
            continue
        values[key] = value[:2500]
    comparison = {}
    for key, candidate in values.items():
        if key in saved_env:
            comparison[key] = {
                "same_as_saved": candidate == saved_env[key],
                "compiled_candidate": candidate[:1000],
                "saved": saved_env[key][:1000],
            }
    return {
        "status": "candidate embedded U-Boot environment (not runtime proof)",
        "payload_offset": at,
        "variables": values,
        "saved_comparison": comparison,
        "limitations": [
            "The match may be inside compiled default environment or diagnostic text.",
            "Do not assume a change to the on-disk environment is safe or necessary.",
            "Only runtime U-Boot console or validated boot behavior can resolve fallback.",
        ],
    }


def debugfs_cat(partition_path, image_path):
    p = subprocess.run(["debugfs", "-R", "cat " + image_path, str(partition_path)],
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    # debugfs may put an informational banner on stderr.
    if p.returncode or b"File not found" in p.stderr or b"not found" in p.stderr:
        return None, p.stderr.decode("utf-8", "replace")[:400]
    return p.stdout, None

def inspect_rootfs(backup_path, out):
    if not shutil.which("debugfs"):
        out["rootfs"] = {"status": "skipped", "reason": "debugfs not installed"}
        return
    with tempfile.TemporaryDirectory(prefix="mpfree-p6-offline-") as td:
        p6 = Path(td) / "p6-readonly-copy.ext4"
        with backup_path.open("rb") as source, p6.open("wb") as sink:
            source.seek(P6[0]*SECTOR)
            remaining = P6[1]*SECTOR
            while remaining:
                buf = source.read(min(8*1024*1024, remaining))
                if not buf:
                    raise RuntimeError("Truncated p6 partition in existing backup")
                sink.write(buf)
                remaining -= len(buf)
        rootfs = {"status": "parsed from TEMPORARY COPY, not mounted", "boot_scripts": {},
                  "device_trees": {}, "limitations": [
                      "No U-Boot runtime environment, no device access.",
                      "DTB inspection only covers likely filenames and selected properties."
                  ]}
        for name in ("/boot/boot.scr.uimg", "/boot/boot.scr"):
            data, error = debugfs_cat(p6, name)
            if data is None:
                rootfs["boot_scripts"][name] = {"status": "unavailable", "error": error}
                continue
            script = data
            image = None
            if len(data) >= 64 and data[:4] == b"\x27\x05\x19\x56":
                declared_size = struct.unpack(">I", data[12:16])[0]
                script = data[64:64+declared_size]
                image = {"uimage_header": True, "declared_payload_bytes": declared_size}
            content = script.decode("utf-8", "replace")
            # Strip embedded unique identifiers before putting content in report.
            content = re.sub(r"(?:PART)?UUID=[^\s\"']+", "[private-id-redacted]", content, flags=re.I)
            content = re.sub(r"(?im)^.*(?:unlock|password|token|secret).*$",
                             "[private line redacted]", content)
            rootfs["boot_scripts"][name] = {
                "status": "found", "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                "legacy_image": image, "script_excerpt": content[:12000],
                "excerpt_truncated": len(content) > 12000,
            }
        # The reference MPC One is az01. Also scan alternate board DTBs for clues.
        names = ["/boot/rk3288-az01.dtb", "/boot/rk3288-az01b.dtb",
                 "/boot/rk3288-az01-rev0.dtb", "/boot/rk3288-az05.dtb"]
        # debugfs ls -p gives enough names without requiring a loop mount.
        ls = subprocess.run(["debugfs", "-R", "ls -p /boot", str(p6)],
                            capture_output=True, text=True)
        for match in re.findall(r"rk3288-[A-Za-z0-9_.-]+\.dtb", ls.stdout):
            candidate = "/boot/" + match
            if candidate not in names: names.append(candidate)
        for name in names[:30]:
            raw, error = debugfs_cat(p6, name)
            if not raw or len(raw) < 40 or raw[:4] != b"\xd0\x0d\xfe\xed":
                continue
            dt = {"size_bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
            if shutil.which("dtc"):
                with tempfile.NamedTemporaryFile(prefix="mpfree-dtb-",suffix=".dtb",
                                                 dir=td, delete=False) as f:
                    f.write(raw)
                    tmp = f.name
                p = subprocess.run(["dtc", "-I", "dtb", "-O", "dts", tmp],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                text = p.stdout.decode("utf-8", "replace")
                lines = text.splitlines()
                hits = []
                seen_indices = set()
                for i, row in enumerate(lines):
                    if any(k in row.lower() for k in DTB_HINTS):
                        for j in range(max(0, i-2), min(len(lines), i+4)):
                            if j in seen_indices:
                                continue
                            seen_indices.add(j)
                            excerpt_line = lines[j]
                            if re.search(r"(?:local-)?mac-address|serial-number|"
                                         r"partuuid|device-uuid|password|secret",
                                         excerpt_line, re.I):
                                excerpt_line = "[PRIVATE DEVICE IDENTIFIER REDACTED]"
                            hits.append(excerpt_line)
                # Deduplicate by *line index*, retaining repeated property
                # names in distinct nodes and preserving the original order.
                # Old reports wasted most of their 22-KiB cap repeating
                # context and truncated potentially relevant board metadata.
                dt["relevant_dts_excerpt"] = "\n".join(hits)[:70000]
                dt["dtc_lines_scanned"] = len(lines)
                dt["excerpt_truncated"] = len("\n".join(hits)) > 70000
                dt["dtc_success"] = p.returncode == 0
            else:
                strings = re.findall(rb"[\x20-\x7e]{5,}", raw)
                dt["relevant_strings"] = [
                    x.decode("ascii", "replace")[:100] for x in strings
                    if any(k.encode() in x.lower() for k in DTB_HINTS)
                ][:120]
            rootfs["device_trees"][name] = dt
        # User-space transition and updater code: read from the same temporary
        # P6 copy and collect small, redacted keyword contexts. Vendor ELF
        # programs are NEVER executed or written into the repository.
        rootfs["startup_and_update_binaries"] = {}
        programs = (
            "/usr/sbin/az01-update",
            "/usr/bin/az01-launch-MPC",
            "/usr/bin/az01-script-runner",
            "/usr/lib/libaz01-prog.so.1.3",
            "/usr/bin/az01-bootloader-sig",
            "/usr/sbin/az01config",
            "/etc/fw_env.config",
        )
        patterns = ("bootloader", "maskrom", "brom", "rockusb", "fastboot",
                    "reboot", "kexec", "update", "factory-reset",
                    "dorecovery", "loader", "rootfs", "signature",
                    "verify", "efuse", "secureboot", "sdcard", "usb",
                    "mmcblk", "firmware", "fw_env", "u-boot")
        for path in programs:
            binary, error = debugfs_cat(p6, path)
            if not binary:
                rootfs["startup_and_update_binaries"][path] = {"present": False}
                continue
            leads = []
            for match in re.finditer(rb"[\x20-\x7e]{5,}", binary):
                literal = match.group().decode("ascii", "replace")
                if not any(k in literal.lower() for k in patterns):
                    continue
                if SENSITIVE.search(literal):
                    display = "[sensitive literal redacted]"
                else:
                    display = literal[:160]
                leads.append({"offset": match.start(), "text": display})
                if len(leads) >= 100:
                    break
            rootfs["startup_and_update_binaries"][path] = {
                "present": True, "bytes": len(binary),
                "sha256": hashlib.sha256(binary).hexdigest(),
                "is_elf": binary.startswith(b"\x7fELF"),
                "interesting_string_leads": leads,
                "leads_truncated": len(leads) >= 100,
                "warning": "Strings are hints, not a proven executable path.",
            }
        out["rootfs"] = rootfs

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--backup", required=True, type=Path)
    ap.add_argument("--report", required=True, type=Path)
    ap.add_argument("--skip-rootfs", action="store_true")
    args = ap.parse_args()
    backup = args.backup.expanduser()
    if not backup.exists() or not stat.S_ISREG(backup.stat().st_mode):
        ap.error("Backup must exist and be a regular file (never /dev/mmcblk*)")
    report = args.report.expanduser()
    if not report.is_absolute():
        ap.error("Report path must be absolute")
    with backup.open("rb") as f:
        p1 = region(f, P1)
        if hashlib.sha256(p1).hexdigest() != EXPECTED_P1:
            raise RuntimeError("p1 partition does not match reference unit; stopped")
        p2 = region(f, P2)
        p3 = region(f, P3)
    out = {
        "schema": "mpfree-wide-boot-artifacts-v1",
        "status": "OFFLINE READ-ONLY SOURCE FILE ONLY",
        "date_utc": datetime.now(timezone.utc).isoformat(),
        "source": "regular-file owner eMMC backup; path withheld",
        "p1": {"lba": P1[0], "bytes": len(p1), "sha256": EXPECTED_P1},
        "p2": valid_env(p2),
        "p3": parse_uimage(p3),
        "compiled_uboot_environment": embedded_uboot_environment_candidates(
            p3, valid_env(p2)["boot_related_variables"]),
        "limitations": [
            "Does not establish runtime environment/permission/boot choice.",
            "Does not read immutable silicon MaskROM or verify recovery availability.",
            "Does not determine whether a source-built unsigned SPL executes.",
            "Private report: may contain proprietary boot-script excerpts.",
        ]
    }
    if not args.skip_rootfs:
        inspect_rootfs(backup, out)
    else:
        out["rootfs"] = {"status": "skipped by operator"}
    report.parent.mkdir(parents=True, exist_ok=True)
    old_umask = os.umask(0o077)
    try:
        report.write_text(json.dumps(out, indent=2) + "\n", encoding="utf-8")
    finally:
        os.umask(old_umask)
    print("MPFree PRIVATE offline artifact report:", report)
    print("p1 hash verified; p3 SHA and CRC32 verified.")
    print("U-Boot environment standard-layout CRC:", out["p2"]["crc_valid_for_standard_layout"])
    print("Rootfs:", out["rootfs"]["status"])

if __name__ == "__main__":
    main()
