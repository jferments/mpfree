"""Local, non-destructive content discovery and audio import for mpfree.

This utility intentionally does not parse firmware images, decrypt assets,
load proprietary plugins, bypass activation, or modify MPC system partitions.
It can run on a desktop with a mounted/backup content directory, or on a
device with a compatible Python 3 interpreter (not yet verified on MPC One).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

SCHEMA = "mpfree.content.v1"
EXTENSIONS = {
    ".wav": "wav",
    ".wave": "wav",
    ".aif": "aiff",
    ".aiff": "aiff",
    ".aifc": "aiff",
    ".flac": "flac",
    ".ogg": "ogg",
    ".oga": "ogg",
    ".opus": "ogg",
}
CHUNK_SIZE = 1024 * 1024


@dataclass(frozen=True)
class Asset:
    source_index: int
    source: Path
    relative_path: Path
    format: str
    size_bytes: int

    @property
    def path(self) -> Path:
        return self.source / self.relative_path

    @property
    def destination_relative_path(self) -> Path:
        return Path(f"source-{self.source_index:02d}") / self.relative_path

    def record(self) -> dict[str, Any]:
        return {
            "source_index": self.source_index,
            "relative_path": self.relative_path.as_posix(),
            "format": self.format,
            "size_bytes": self.size_bytes,
        }


def audio_format(path: Path) -> str | None:
    """Accept known audio extensions only when their file signature matches."""
    expected = EXTENSIONS.get(path.suffix.lower())
    if expected is None:
        return None
    try:
        with path.open("rb") as handle:
            header = handle.read(12)
    except OSError:
        return None

    if expected == "wav" and len(header) >= 12:
        return "wav" if header[:4] in (b"RIFF", b"RF64") and header[8:12] == b"WAVE" else None
    if expected == "aiff" and len(header) >= 12:
        return "aiff" if header[:4] == b"FORM" and header[8:12] in (b"AIFF", b"AIFC") else None
    if expected == "flac":
        return "flac" if header[:4] == b"fLaC" else None
    if expected == "ogg":
        return "ogg" if header[:4] == b"OggS" else None
    return None


def within(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def validate_sources(sources: list[Path]) -> list[Path]:
    result: list[Path] = []
    for source in sources:
        if source.is_symlink():
            raise ValueError(f"Source directory must not be a symlink: {source}")
        if not source.is_dir():
            raise ValueError(f"Source directory does not exist: {source}")
        resolved = source.resolve()
        if resolved in result:
            raise ValueError(f"Duplicate source directory: {source}")
        result.append(resolved)
    return result


def discover(sources: list[Path]) -> tuple[list[Asset], list[str]]:
    assets: list[Asset] = []
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
                candidate = root / name
                if candidate.is_symlink():
                    continue
                try:
                    info = candidate.stat()
                    if not stat.S_ISREG(info.st_mode):
                        continue
                    kind = audio_format(candidate)
                    if kind is None:
                        continue
                    assets.append(
                        Asset(
                            source_index=index,
                            source=source,
                            relative_path=candidate.relative_to(source),
                            format=kind,
                            size_bytes=info.st_size,
                        )
                    )
                except OSError as error:
                    errors.append(f"Unable to inspect {candidate}: {error}")
    return assets, errors


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(CHUNK_SIZE):
            digest.update(chunk)
    return digest.hexdigest()


def validate_destination(destination: Path, sources: list[Path]) -> Path:
    if destination.is_symlink():
        raise ValueError("Destination directory must not be a symlink")
    resolved = destination.resolve()
    if destination.exists() and not destination.is_dir():
        raise ValueError("Destination is not a directory")
    for source in sources:
        if within(resolved, source) or within(source, resolved):
            raise ValueError("Source and destination directories must not overlap")
    return resolved


def ensure_parent_directories(destination: Path, relative_path: Path) -> None:
    """Reject symlink parents and never create directories in source."""
    current = destination
    if current.is_symlink():
        raise ValueError(f"Destination is a symlink: {current}")
    current.mkdir(parents=True, exist_ok=True)
    for component in relative_path.parts[:-1]:
        current = current / component
        if current.is_symlink():
            raise ValueError(f"Destination parent is a symlink: {current}")
        if current.exists() and not current.is_dir():
            raise ValueError(f"Destination parent is not a directory: {current}")
        current.mkdir(exist_ok=True)


def copy_exclusive(source: Path, destination: Path, expected_size: int) -> str:
    """Create only a new file; do not overwrite pre-existing destinations.

    On failure, remove only a file created by this invocation. A partially
    written file can exist during the copy; imports are not yet transactional.
    """
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    source_fd = os.open(source, flags)
    created = False
    try:
        with os.fdopen(source_fd, "rb") as reader:
            source_fd = -1
            info = os.fstat(reader.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size != expected_size:
                raise ValueError(f"Source changed during scan: {source}")
            digest = hashlib.sha256()
            count = 0
            with destination.open("xb") as writer:
                created = True
                while chunk := reader.read(CHUNK_SIZE):
                    writer.write(chunk)
                    digest.update(chunk)
                    count += len(chunk)
                writer.flush()
                os.fsync(writer.fileno())
            if count != expected_size:
                raise ValueError(f"Source size changed while copying: {source}")
            return digest.hexdigest()
    except Exception:
        if created:
            destination.unlink(missing_ok=True)
        raise
    finally:
        if source_fd >= 0:
            os.close(source_fd)


def import_assets(
    assets: list[Asset], destination: Path, apply: bool
) -> tuple[list[dict[str, Any]], list[str]]:
    records: list[dict[str, Any]] = []
    errors: list[str] = []
    for asset in assets:
        record = asset.record()
        relative = asset.destination_relative_path
        record["destination_relative_path"] = relative.as_posix()
        target = destination / relative
        if not apply:
            record["status"] = "planned"
            records.append(record)
            continue
        try:
            ensure_parent_directories(destination, relative)
            if target.is_symlink():
                record["status"] = "conflict"
                errors.append(f"Refusing symlink destination: {target}")
            elif target.exists():
                if target.is_file() and sha256_file(target) == sha256_file(asset.path):
                    record["status"] = "already_present"
                else:
                    record["status"] = "conflict"
                    errors.append(f"Different file already exists: {target}")
            else:
                try:
                    record["sha256"] = copy_exclusive(
                        asset.path, target, asset.size_bytes
                    )
                    record["status"] = "copied"
                except FileExistsError:
                    record["status"] = "conflict"
                    errors.append(f"File appeared during import: {target}")
        except (OSError, ValueError) as error:
            if "status" not in record:
                record["status"] = "error"
            errors.append(f"Unable to import {asset.path}: {error}")
        records.append(record)
    return records, errors


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="mpfree-content",
        description="Inventory or import accessible audio assets without changing MPC firmware.",
    )
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("scan", "import"):
        sub = commands.add_parser(command)
        sub.add_argument(
            "--source", action="append", type=Path, required=True,
            help="Existing content directory (repeatable).",
        )
        sub.add_argument(
            "--manifest", type=Path,
            help="Write metadata-only JSON report to a new file.",
        )
        if command == "scan":
            sub.add_argument(
                "--hash", action="store_true",
                help="Compute SHA-256 for each discovered audio file (slower).",
            )
        else:
            sub.add_argument("--destination", type=Path, required=True)
            sub.add_argument(
                "--apply", action="store_true",
                help="Actually copy files. Without this, only print the plan.",
            )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        sources = validate_sources(args.source)
        destination = None
        if args.command == "import":
            destination = validate_destination(args.destination, sources)
        if args.manifest is not None:
            manifest_path = args.manifest.resolve()
            if any(within(manifest_path, source) for source in sources):
                raise ValueError("Manifest must not be written inside a source directory")
            if manifest_path.is_symlink():
                raise ValueError("Manifest must not be a symlink")

        assets, errors = discover(sources)
        if args.command == "scan":
            records = [asset.record() for asset in assets]
            if args.hash:
                for asset, record in zip(assets, records):
                    try:
                        record["sha256"] = sha256_file(asset.path)
                    except OSError as error:
                        errors.append(f"Unable to hash {asset.path}: {error}")
        else:
            records, import_errors = import_assets(assets, destination, args.apply)
            errors.extend(import_errors)

        report = {
            "schema": SCHEMA,
            "command": args.command,
            "applied": bool(getattr(args, "apply", False)),
            "sources": [str(source) for source in sources],
            "assets": records,
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
