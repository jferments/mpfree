#!/usr/bin/env python3
"""Conservative host-only ARM ELF vs az0x libc/loader evidence assessor.

Inputs: an authorized read-only MPC ABI report and a locally compiled ELF.
Does NOT contact the MPC, modify a sysroot, or execute an ARM binary.
Exit 0: necessary checks supported by explicit glibc version, NOT deployment.
Exit 2: known version/ELF mismatch.
Exit 4: only a symbol-tag bound / missing library evidence: NOT QUALIFIED.
Exit 3: unsupported/malformed inputs or missing tools.
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

GLIBC_VERSION = re.compile(r"\bglibc\s+(\d+)\.(\d+)(?:\.(\d+))?\b", re.I)
GLIBC_RELEASE = re.compile(
    r"\b(?:GNU C Library|GNU libc)[^\n]*\brelease version\s+(\d+)\.(\d+)(?:\.(\d+))?",
    re.I,
)
GLIBC_SYMBOL = re.compile(r"\bGLIBC_(\d+)\.(\d+)(?:\.(\d+))?\b")
INTERPRETER = re.compile(r"Requesting program interpreter:\s*([^\]\n]+)")
LIBRARY = re.compile(r"Shared library:\s*\[([^\]]+)\]")
KNOWN_DIRS = ("/lib/", "/usr/lib/", "/lib32/", "/usr/lib32/")


def version_tuple(match):
    return tuple(int(match.group(i) or 0) for i in (1, 2, 3))


def extract_target_glibc(report):
    """Explicit getconf result or release banner only; tags are NOT a version."""
    for line in report.splitlines():
        if line.startswith("getconf GNU_LIBC_VERSION:"):
            match = GLIBC_VERSION.search(line)
            if match:
                return version_tuple(match)
        if line.startswith("GNU libc release banner:"):
            match = GLIBC_RELEASE.search(line)
            if match:
                return version_tuple(match)
    return None


def highest_reported_glibc_symbol(report):
    section = report.partition("=== Available GLIBC symbol tag maximum")[2]
    if not section:
        return None
    end = section.partition("\nEND MPFREE ARMV7 ABI SURVEY")[0]
    tags = [version_tuple(m) for m in GLIBC_SYMBOL.finditer(end)]
    return max(tags) if tags else None


def highest_glibc_required(version_info):
    matches = [version_tuple(m) for m in GLIBC_SYMBOL.finditer(version_info)]
    return max(matches) if matches else None


def interpreter(program_headers):
    m = INTERPRETER.search(program_headers)
    return m.group(1).strip() if m else None


def needed_libraries(dynamic):
    return sorted(set(LIBRARY.findall(dynamic)))


def reported_libraries(report):
    out = set()
    for line in report.splitlines():
        if any(line.startswith(d) for d in KNOWN_DIRS):
            out.add(line.split(" -> ", 1)[0].rsplit("/", 1)[-1])
    return out


def readelf(binary, command, args):
    proc = subprocess.run(
        [command, *args, str(binary)], capture_output=True, text=True, timeout=15
    )
    if proc.returncode:
        raise RuntimeError(f"{command} failed: {proc.stderr[:800]}")
    return proc.stdout


def assess(report, elf_header, program, version_info, dynamic):
    """Return (exit_code, human-readable lines). No target device operations."""
    if "MPFREE ARMV7 USERSPACE ABI SURVEY - READ ONLY" not in report:
        raise ValueError("not an MPFree ABI report")
    if not re.search(r"Class:\s*ELF32", elf_header) or not re.search(
        r"Machine:\s*ARM", elf_header
    ):
        return 2, ["FAIL: candidate is not ARM ELF32"]
    if not re.search(r"Flags:.*Version5 EABI.*hard-float ABI", elf_header):
        return 2, ["FAIL: candidate is not ARM EABI5 hard-float"]

    interp = interpreter(program)
    if interp != "/lib/ld-linux-armhf.so.3":
        return 2, [f"FAIL: unexpected PT_INTERP {interp!r}"]
    if "/lib/ld-linux-armhf.so.3 ->" not in report:
        return 4, ["NOT QUALIFIED: loader file not confirmed by target report"]

    required = highest_glibc_required(version_info)
    if required is None:
        return 4, ["NOT QUALIFIED: no versioned GLIBC requirements found"]

    target = extract_target_glibc(report)
    tag_ceiling = highest_reported_glibc_symbol(report)
    needed = needed_libraries(dynamic)
    available = reported_libraries(report)
    missing = sorted(lib for lib in needed if lib not in available)
    fmt = lambda ver: ".".join(map(str, ver[:2]))
    lines = [
        f"ELF32 ARM EABI5 hard-float; PT_INTERP={interp}",
        f"Candidate maximum GLIBC requirement: {fmt(required)}",
        "DT_NEEDED: " + (", ".join(needed) or "(none)"),
        "Libraries not explicitly established by report: "
        + (", ".join(missing) if missing else "(none)"),
    ]
    if target is not None:
        lines.append(f"Target glibc explicit version: {fmt(target)}")
        if required > target:
            return 2, lines + ["FAIL: newer glibc required than device explicitly reports"]
        if missing:
            return 4, lines + ["NOT QUALIFIED: required shared-library names not confirmed"]
        return 0, lines + [
            "NECESSARY CHECKS PASS: ELF, interpreter, glibc version, named libraries.",
            "NOT runtime-qualified: ABI behavior, CPU baseline, drivers and recovery still untested.",
        ]

    lines.append("Target glibc version: UNKNOWN (getconf missing; no release banner)")
    if tag_ceiling is not None:
        lines.append(f"Highest observed libc GLIBC_* string: {fmt(tag_ceiling)}")
        if required > tag_ceiling:
            lines.append("No evidence for candidate's newer required symbols.")
        else:
            lines.append("Candidate requirements fit within observed symbol tags.")
    else:
        lines.append("Target libc symbol version evidence unavailable.")
    return 4, lines + [
        "NOT QUALIFIED: observed strings are not an authoritative libc version",
        "and do not establish that exact required symbols and DSOs are exported.",
        "Collect an explicit GNU libc release banner or inspect target ELF symbols",
        "offline before any nonpersistent ARM launch; never deploy on this result.",
    ]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--report", type=Path, required=True)
    p.add_argument("--elf", type=Path, required=True)
    p.add_argument("--readelf", default="arm-linux-gnueabihf-readelf")
    args = p.parse_args()
    report = args.report.read_text(encoding="utf-8", errors="replace")
    header = readelf(args.elf, args.readelf, ["-h"])
    prog = readelf(args.elf, args.readelf, ["-l"])
    versions = readelf(args.elf, args.readelf, ["-W", "--version-info"])
    dynamic = readelf(args.elf, args.readelf, ["-d"])
    code, messages = assess(report, header, prog, versions, dynamic)
    for msg in messages:
        print(msg)
    return code


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"ABORT / NOT QUALIFIED: {exc}", file=sys.stderr)
        sys.exit(3)
