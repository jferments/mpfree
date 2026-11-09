#!/usr/bin/env python3
"""Compare a locally cross-compiled ARMv7 ELF with a read-only MPC ABI report.

NO device access, no writes, no copying stock libraries; this can establish
necessary compatibility conditions, but never proves runtime support.
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

GLIBC_VERSION = re.compile(r"\bglibc\s+(\d+)\.(\d+)(?:\.(\d+))?\b", re.I)
GLIBC_SYMBOL = re.compile(r"\bGLIBC_(\d+)\.(\d+)(?:\.(\d+))?\b")
INTERPRETER = re.compile(r"Requesting program interpreter:\s*([^\]\n]+)")
LIBRARY = re.compile(r"Shared library:\s*\[([^\]]+)\]")


def version_tuple(match):
    return tuple(int(match.group(i) or 0) for i in (1, 2, 3))


def extract_target_glibc(report):
    # Prefer actual getconf result, not the file's list of available GLIBC tags.
    for line in report.splitlines():
        if line.startswith("getconf GNU_LIBC_VERSION:"):
            match = GLIBC_VERSION.search(line)
            if match:
                return version_tuple(match)
    return None


def highest_glibc_required(readelf_text):
    versions = [version_tuple(m) for m in GLIBC_SYMBOL.finditer(readelf_text)]
    return max(versions) if versions else None


def interpreter(readelf_programs):
    m = INTERPRETER.search(readelf_programs)
    return m.group(1).strip() if m else None


def needed_libraries(readelf_dynamic):
    return sorted(set(LIBRARY.findall(readelf_dynamic)))


def readelf(binary, command, args):
    result = subprocess.run([command, *args, str(binary)],
                            capture_output=True, text=True, timeout=15)
    if result.returncode:
        raise RuntimeError(f"{command} failed: {result.stderr[:1000]}")
    return result.stdout


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--report", type=Path, required=True,
                   help="private mpfree --abi report generated on the workstation")
    p.add_argument("--elf", type=Path, required=True,
                   help="locally cross-compiled MPFree ARMv7 binary; never sent to MPC")
    p.add_argument("--readelf", default="arm-linux-gnueabihf-readelf",
                   help="host-only binutils readelf executable")
    args = p.parse_args()

    report = args.report.read_text(encoding="utf-8", errors="replace")
    if "MPFREE ARMV7 USERSPACE ABI SURVEY" not in report:
        raise RuntimeError("Not an MPFree read-only ABI report")
    target = extract_target_glibc(report)
    if target is None:
        raise RuntimeError("Target glibc version was not reported by getconf; compatibility UNKNOWN")

    h = readelf(args.elf, args.readelf, ["-h"])
    program = readelf(args.elf, args.readelf, ["-l"])
    versions = readelf(args.elf, args.readelf, ["-W", "--version-info"])
    dyn = readelf(args.elf, args.readelf, ["-d"])
    if not re.search(r"Class:\s*ELF32", h) or not re.search(r"Machine:\s*ARM", h):
        raise RuntimeError("Binary is not ARM ELF32")
    if not re.search(r"Flags:.*Version5 EABI.*hard-float ABI", h):
        raise RuntimeError("Binary is not ARM EABI5 hard-float")
    link = interpreter(program)
    if link != "/lib/ld-linux-armhf.so.3":
        raise RuntimeError(f"Unexpected PT_INTERP {link!r}; expected armhf loader")
    required = highest_glibc_required(versions)
    if required is None:
        raise RuntimeError("No GLIBC symbol versions found; can't establish needed runtime ABI")

    fmt = lambda x: ".".join(map(str, x[:2]))
    print(f"MPFree binary architecture: ARM ELF32 EABI5 hard-float; PT_INTERP={link}")
    print(f"Cross-built binary requires GNU libc >= {fmt(required)}; MPC reports {fmt(target)}")
    print("DT_NEEDED libraries:", ", ".join(needed_libraries(dyn)) or "(none)")
    if required > target:
        print("FAIL: binary requires newer glibc than the MPC reports.")
        return 2
    print("PASS: necessary glibc version / ELF / loader checks satisfied.")
    print("UNVERIFIED: actual library presence, exact ABI, kernel feature requirements,")
    print("CPU instruction compatibility, sysroot provenance, and nonpersistent on-device launch.")
    print("DO NOT deploy or execute on MPC based on this report alone.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"ABORT / NOT QUALIFIED: {exc}", file=sys.stderr)
        sys.exit(3)
