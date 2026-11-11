#!/usr/bin/env python3
"""Per-crate line coverage from `cargo llvm-cov report --json`, as a Markdown table.

    cargo llvm-cov report --json --summary-only > cov.json   (per-file totals)
    python3 tools/ci/coverage_summary.py cov.json >> "$GITHUB_STEP_SUMMARY"

Files are grouped by their crate folder under rust/ (`rust/mpfree-x/...`).
Crates are listed lowest coverage first, so the thinnest are on top. A crate
with no measured lines (no tests ever run its code) is listed as 0%.
Standard library only.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

CRATE_PREFIX = "mpfree-"


def crate_of(filename: str) -> str | None:
    parts = Path(filename).parts
    for i, part in enumerate(parts):
        if part == "rust" and i + 1 < len(parts) and parts[i + 1].startswith(CRATE_PREFIX):
            return parts[i + 1]
    return None


def per_crate(report: dict) -> dict[str, tuple[int, int]]:
    """crate -> (covered lines, total lines)."""
    totals: dict[str, list[int]] = {}
    for block in report.get("data", []):
        for entry in block.get("files", []):
            crate = crate_of(entry.get("filename", ""))
            if crate is None:
                continue
            lines = entry.get("summary", {}).get("lines", {})
            t = totals.setdefault(crate, [0, 0])
            t[0] += int(lines.get("covered", 0))
            t[1] += int(lines.get("count", 0))
    return {crate: (c, n) for crate, (c, n) in totals.items()}


def percent(covered: int, count: int) -> float:
    return 100.0 * covered / count if count else 0.0


def table(crates: dict[str, tuple[int, int]], all_crates: list[str] | None = None) -> str:
    rows = dict(crates)
    for crate in all_crates or []:
        rows.setdefault(crate, (0, 0))
    covered = sum(c for c, _ in rows.values())
    count = sum(n for _, n in rows.values())
    out = ["| Crate | Lines covered | Coverage |", "| --- | ---: | ---: |"]
    for crate, (c, n) in sorted(rows.items(), key=lambda kv: (percent(*kv[1]), kv[0])):
        out.append(f"| `{crate}` | {c} / {n} | {percent(c, n):.1f}% |")
    out.append(f"| **all** | {covered} / {count} | **{percent(covered, count):.1f}%** |")
    return "\n".join(out) + "\n"


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    report = json.loads(Path(argv[1]).read_text(encoding="utf-8"))
    rust = Path(__file__).resolve().parents[2] / "rust"
    # Workspace members only: a crate with its own [workspace] (mpfree-splash-gl)
    # is built and tested by its own job, not by this coverage run.
    members = sorted(p.name for p in rust.iterdir() if p.is_dir() and p.name.startswith(CRATE_PREFIX)
                     and (p / "Cargo.toml").is_file()
                     and "[workspace]" not in (p / "Cargo.toml").read_text(encoding="utf-8")) \
        if rust.is_dir() else []
    print("### Rust line coverage by crate (lowest first)\n")
    print(table(per_crate(report), members))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
