#!/usr/bin/env python3
"""Inspect a U-Boot legacy script from a read-only QEMU od transcript.

No firmware execution, physical device, USB, /dev/mem or storage writes.
The entire decoded image and commands remain in the owner-private ZIP;
only independent parser source belongs in the public repository.
"""
from __future__ import annotations
import hashlib
import re
import struct
import zlib

IH_MAGIC = 0x27051956
IH_TYPE_SCRIPT = 6
MAX_IMAGE_SIZE = 1024 * 1024
# BusyBox 1.36 `od -v -b` prints an OCTAL byte address,
# followed by up to 16 three-digit OCTAL bytes per line; final address
# line has no bytes. Unlike GNU od, this build has no -A/-t options.
BUSYBOX_OCTAL_LINE = re.compile(r"^\s*([0-7]{7,11})(?:\s+([0-7]{3}(?:\s+[0-7]{3})*))?\s*$")


def decode_od_bytes(section: str) -> bytes:
    """Reassemble an actual BusyBox od -v -b transcript with offset checks.

    Reject missing chunks, duplicated lines, masked truncation, unexpected
    byte values and missing terminal length. Shell prompts/ACKs are ignored;
    unsupported od options will result in a hard error, not empty success.
    """
    if not isinstance(section, str):
        raise TypeError("Guest od transcript must be text")
    result = bytearray()
    found_data = False
    terminal_offset = None
    for line in section.splitlines():
        match = BUSYBOX_OCTAL_LINE.fullmatch(line)
        if not match:
            continue
        offset = int(match.group(1), 8)
        words = match.group(2)
        if words is None:
            if found_data:
                if terminal_offset is not None:
                    raise ValueError("Duplicate terminal offset")
                terminal_offset = offset
            continue
        if terminal_offset is not None:
            raise ValueError("Hex/oct dump data after terminal offset")
        if offset != len(result):
            raise ValueError("Non-contiguous BusyBox od byte offsets")
        numbers = words.split()
        if not (1 <= len(numbers) <= 16):
            raise ValueError("Unexpected octal bytes per output row")
        decoded = [int(item, 8) for item in numbers]
        if any(item > 255 for item in decoded):
            raise ValueError("BusyBox od produced out-of-range octal byte")
        result.extend(decoded)
        found_data = True
        if len(result) > MAX_IMAGE_SIZE:
            raise ValueError("Guest image exceeds offline analysis bound")
    if not found_data:
        raise ValueError("No U-Boot script bytes found in guest BusyBox od output")
    if terminal_offset is None or terminal_offset != len(result):
        raise ValueError("Missing or inconsistent BusyBox od terminal offset")
    return bytes(result)


# Backward-compatible public name; input is explicitly BusyBox octal, not hex.
decode_od_hex = decode_od_bytes

def inspect_script_image(image: bytes) -> dict:
    """Validate uImage header/data CRC32 and extract multi-image script.

    This parser interprets documented legacy uImage format. CRC32 is
    an integrity check, not RSA authentication. No image is executed.
    """
    if not isinstance(image, bytes) or not 72 <= len(image) <= MAX_IMAGE_SIZE:
        raise ValueError("Invalid read-only legacy script size")
    head = image[:64]
    magic, hcrc, timestamp, payload_size, load, entry, dcrc, osid, arch, typ, comp, name = (
        struct.unpack(">7I4B32s", head))
    if magic != IH_MAGIC or typ != IH_TYPE_SCRIPT:
        raise ValueError("Not a legacy U-Boot script image")
    if payload_size < 8 or payload_size > MAX_IMAGE_SIZE - 64:
        raise ValueError("Invalid script payload length")
    if len(image) != 64 + payload_size:
        raise ValueError("Image byte count differs from uImage header")
    payload = image[64:]
    if (zlib.crc32(head[:4] + b"\0\0\0\0" + head[8:]) & 0xffffffff) != hcrc:
        raise ValueError("Invalid uImage header CRC32")
    if (zlib.crc32(payload) & 0xffffffff) != dcrc:
        raise ValueError("Invalid uImage payload CRC32")
    script_len, sentinel = struct.unpack(">II", payload[:8])
    if sentinel != 0 or script_len > len(payload) - 8:
        raise ValueError("Unsupported multi-image script length table")
    if script_len != len(payload) - 8:
        raise ValueError("Unexpected trailing multi-image script bytes")
    try:
        decoded = payload[8:8 + script_len].decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ValueError("Script bytes are not UTF-8") from exc
    return {
        "schema": "mpfree-legacy-boot-script-verified-v1",
        "full_image_sha256": hashlib.sha256(image).hexdigest(),
        "header_sha256": hashlib.sha256(head).hexdigest(),
        "multi_image_prefix_sha256": hashlib.sha256(payload[:8]).hexdigest(),
        "image_total_bytes": len(image),
        "header_crc32_verified": True,
        "payload_crc32_verified": True,
        "script_type": typ,
        "os": osid,
        "arch": arch,
        "compression": comp,
        "header_timestamp": timestamp,
        "header_load_address": "0x%08x" % load,
        "header_entry_address": "0x%08x" % entry,
        "image_name_ascii": name.rstrip(b"\0").decode("ascii", errors="replace"),
        "script_commands_sha256": hashlib.sha256(decoded.encode()).hexdigest(),
        "script_length": script_len,
        "script_commands": decoded,
    }


def compare_guest_to_update(section: str, firmware310: dict) -> dict:
    """Compare hashes of the SAME semantic byte regions.

    In the verified 3.10 source report, sha256 hashes the raw script
    data (including its terminal newline), NOT the complete uImage.
    The header.header_sha256 key hashes the 64-byte header.
    An original full-image hash must use explicit image_sha256.
    """
    guest = inspect_script_image(decode_od_bytes(section))
    official = str(firmware310["commands"])
    observed = guest.pop("script_commands")
    def normalize(x):
        return x.replace("\r\n", "\n").strip()
    expected_script = firmware310["sha256"].lower()
    expected_header = firmware310["header"]["header_sha256"].lower()
    expected_size = firmware310["header"]["image_bytes"]
    expected_script_length = firmware310["header"]["multi_image_script_length"]
    if (not isinstance(expected_size, int) or
        not isinstance(expected_script_length, int) or
        any(len(x) != 64 for x in (expected_script, expected_header))):
        raise ValueError("Incomplete official 3.10 script/header provenance")
    script_match = guest["script_commands_sha256"] == expected_script
    header_match = guest["header_sha256"] == expected_header
    size_match = guest["image_total_bytes"] == expected_size
    script_length_match = guest["script_length"] == expected_script_length
    layout_match = script_match and header_match and size_match and script_length_match
    original_image_hash = firmware310.get("image_sha256")
    if original_image_hash is not None and not isinstance(original_image_hash, str):
        raise ValueError("Invalid optional full image hash")
    exact_full_hash_match = (guest["full_image_sha256"] ==
                             original_image_hash.lower()
                             if original_image_hash else None)
    result = {
        **guest,
        "source_official_sha256_field_means": "raw_script_data_not_uimage",
        "official_raw_script_sha256": expected_script,
        "official_header_sha256": expected_header,
        "original_full_image_sha256_available": original_image_hash is not None,
        "full_image_matches_official": exact_full_hash_match,
        "header_sha256_matches_official": header_match,
        "raw_script_sha256_matches_official": script_match,
        "image_length_matches_official": size_match,
        "raw_script_length_matches_official": script_length_match,
        "canonical_header_and_script_bytes_match_official": layout_match,
        "script_commands_match_official": normalize(observed) == normalize(official),
        "payload_command_lines": len(observed.splitlines()),
        "different_image_even_if_commands_match": (
            exact_full_hash_match is False
            and normalize(observed) == normalize(official)),
        "analysis_only_no_vendor_execution": True,
    }
    if not result["script_commands_match_official"]:
        import difflib
        result["command_diff_excerpt"] = list(
            difflib.unified_diff(official.splitlines(), observed.splitlines(),
                                 fromfile="official-3.10", tofile="qemu-clone"))[:64]
    return result
