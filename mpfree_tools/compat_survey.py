"""Read-only survey of candidate MPC preset, expansion, and plugin files.

This is an inventory tool, not a plugin loader or firmware unpacker. It
reads only short file headers (and optionally SHA-256) and does not copy
proprietary assets, execute binaries, or modify the selected source.
"""

from __future__ import annotations

import argparse
import json
import os
import stat
import sys
from pathlib import Path
from typing import Any

from mpfree_tools.content import SCHEMA, sha256_file, validate_sources, within

# Names are deliberately "candidates": extension alone cannot prove format.
SUFFIX_KINDS = {
    ".so": "native-library-candidate",
    ".dll": "native-library-candidate",
    ".xpm": "mpc-program-candidate",
    ".xpn": "mpc-expansion-candidate",
    ".xpk": "mpc-package-candidate",
    ".xpr": "mpc-project-candidate",
    ".fxp": "plugin-preset-candidate",
    ".fxb": "plugin-bank-candidate",
    ".preset": "preset-candidate",
    ".aupreset": "preset-candidate",
    ".sfz": "sample-map-candidate",
    ".sf2": "soundfont-candidate",
}
ELF_MACHINES = {
    3: "x86",
    40: "ARM",
    62: "x86_64",
    183: "AArch64",
}


def candidate_kind(path: Path) -> str | None:
    name = path.name.lower()
    if ".so." in name:
        return "native-library-candidate"
    return SUFFIX_KINDS.get(path.suffix.lower())


def signature_info(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        header = handle.read(64)
    if header.startswith(b"\x7fELF") and len(header) >= 20:
        elf_class = {1: "32-bit", 2: "64-bit"}.get(header[4], "unknown")
        endian = {1: "little", 2: "big"}.get(header[5])
        machine = None
        if endian:
            number = int.from_bytes(header[18:20], byteorder=endian)
            machine = ELF_MACHINES.get(number, f"unknown ({number})")
        return {
            "signature": "ELF",
            "elf_class": elf_class,
            "elf_endian": endian or "unknown",
            "elf_machine": machine,
        }
    if header.startswith((b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08")):
        return {"signature": "ZIP"}
    if header.startswith(b"\x1f\x8b"):
        return {"signature": "GZIP"}
    if header.startswith(b"RIFF"):
        return {"signature": "RIFF"}
    if header.lstrip().startswith(b"<?xml"):
        return {"signature": "XML"}
    return {"signature": "unrecognized"}


def survey(
    sources: list[Path], include_hash: bool = False
) -> tuple[list[dict[str, Any]], list[str]]:
    records: list[dict[str, Any]] = []
    errors: list[str] = []
    for index, source in enumerate(sources, 1):
        def onerror(error: OSError) -> None:
            errors.append(f"Unable to traverse {error.filename}: {error}")

        for directory, directories, filenames in os.walk(
            source, topdown=True, followlinks=False, onerror=onerror
        ):
            root = Path(directory)
            directories[:] = sorted(
                name for name in directories if not (root / name).is_symlink()
            )
            for name in sorted(filenames):
                path = root / name
                kind = candidate_kind(path)
                if kind is None or path.is_symlink():
                    continue
                try:
                    info = path.stat()
                    if not stat.S_ISREG(info.st_mode):
                        continue
                    record: dict[str, Any] = {
                        "source_index": index,
                        "relative_path": path.relative_to(source).as_posix(),
                        "kind": kind,
                        "size_bytes": info.st_size,
                    }
                    record.update(signature_info(path))
                    if include_hash:
                        record["sha256"] = sha256_file(path)
                    records.append(record)
                except OSError as error:
                    errors.append(f"Unable to inspect {path}: {error}")
    return records, errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="mpfree-compat-survey",
        description="Read-only inventory of possible plugin, preset, and pack files.",
    )
    parser.add_argument("--source", action="append", type=Path, required=True)
    parser.add_argument("--hash", action="store_true", help="Hash candidate files")
    parser.add_argument("--manifest", type=Path, help="Write report to a new file")
    args = parser.parse_args(argv)
    try:
        sources = validate_sources(args.source)
        if args.manifest is not None:
            if args.manifest.is_symlink():
                raise ValueError("Manifest must not be a symlink")
            manifest = args.manifest.resolve()
            if any(within(manifest, source) for source in sources):
                raise ValueError("Manifest must not be written inside a source directory")
        records, errors = survey(sources, args.hash)
        report = {
            "schema": SCHEMA,
            "command": "compatibility-survey",
            "sources": [str(source) for source in sources],
            "candidates": records,
            "errors": errors,
        }
        serialized = json.dumps(report, indent=2, ensure_ascii=False)
        if args.manifest is not None:
            args.manifest.parent.mkdir(parents=True, exist_ok=True)
            with args.manifest.open("x", encoding="utf-8") as handle:
                handle.write(serialized + "\n")
        print(serialized)
        return 1 if errors else 0
    except (OSError, ValueError) as error:
        parser.error(str(error))
        return 2


if __name__ == "__main__":
    sys.exit(main())
