#!/usr/bin/env python3
"""The instruction-count gate: what every mpfree-bench case costs, counted in
instructions under callgrind, against a committed baseline.

Times on a shared VM drift by a third during the day; instruction counts do
not. So CI counts each case's instructions per 128-frame block and fails when
any case costs more than 5 % above the baseline. A case that got more than
5 % cheaper is reported too, so the baseline can be tightened to keep the gain.

    python3 tools/ci/instr_gate.py [--bench PATH] [--blocks N] [--filter TEXT]
                                   [--tolerance PCT] [--baseline FILE] [--bless]

- It runs `valgrind --tool=callgrind ... mpfree-bench --count`, which wraps each
  case's blocks in callgrind client requests (rust/mpfree-bench/src/vg.rs), and
  reads one dump per case.
- `--bless` writes the counts as the new baseline. Do it with the toolchain CI
  pins (MPFREE_CLIPPY_TOOLCHAIN in the workflow), because another compiler
  makes other code. A baseline from another toolchain still compares, with a
  warning.
- Build the bench first, in release, as CI does:
  `cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-bench`.

Exit status: 0 when nothing rose past the tolerance, 1 when something did, 2 for
a usage or tool error.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DEFAULT_BENCH = os.path.join(ROOT, "rust", "target", "release", "mpfree-bench")
DEFAULT_BASELINE = os.path.join(ROOT, "rust", "mpfree-bench", "instructions.txt")
# What mpfree-bench --count puts before each case's name (COUNT_TAG in main.rs).
TAG = "mpfree-bench case: "
DEFAULT_BLOCKS = 200


def toolchain():
    """The rustc that `cargo` would use here (RUSTUP_TOOLCHAIN respected)."""
    try:
        out = subprocess.run(["rustc", "--version"], capture_output=True, text=True, check=True)
        return out.stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def parse_dump(text):
    """(case name, instructions) from one callgrind dump, or None if the dump
    is not one of ours (the final one at program exit)."""
    name = None
    total = None
    for line in text.splitlines():
        if line.startswith("desc: Trigger: Client Request: "):
            trigger = line[len("desc: Trigger: Client Request: "):]
            if trigger.startswith(TAG):
                name = trigger[len(TAG):]
        elif line.startswith("summary:") or line.startswith("totals:"):
            # The first number is Ir, the only event counted.
            fields = line.split()
            if len(fields) >= 2:
                total = int(fields[1])
    if name is None or total is None:
        return None
    return name, total


def count(bench, blocks, filt):
    """{case: instructions per block}, counted under callgrind."""
    if shutil.which("valgrind") is None:
        sys.exit("instr_gate: valgrind is not installed (apt install valgrind)")
    if not os.access(bench, os.X_OK):
        sys.exit(f"instr_gate: no bench at {bench}; build it with "
                 "cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-bench")
    with tempfile.TemporaryDirectory(prefix="mpfree-instr-") as d:
        cmd = ["valgrind", "--tool=callgrind", f"--callgrind-out-file={d}/cg.%p",
               "--dump-instr=no", "--dump-line=no", "--compress-strings=no", "--compress-pos=no",
               bench, "--count", "--blocks", str(blocks)]
        if filt:
            cmd += ["--filter", filt]
        run = subprocess.run(cmd, capture_output=True, text=True)
        if run.returncode != 0:
            sys.stderr.write(run.stdout[-4000:] + run.stderr[-4000:])
            sys.exit(f"instr_gate: callgrind run failed (exit {run.returncode})")
        counts = {}
        for f in sorted(os.listdir(d)):
            with open(os.path.join(d, f), encoding="utf-8", errors="replace") as fh:
                got = parse_dump(fh.read())
            if got is not None:
                name, total = got
                counts[name] = round(total / blocks)
        expected = [l for l in run.stdout.splitlines()[1:] if l.strip()]
        missing = [n for n in expected if n not in counts]
        if missing:
            sys.exit(f"instr_gate: no count for {len(missing)} case(s), first {missing[0]!r}")
        return counts


def read_baseline(path):
    """({case: per block}, header lines)."""
    base, header = {}, []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line.strip():
                continue
            if line.startswith("#"):
                header.append(line)
                continue
            n, name = line.split("\t", 1)
            base[name] = int(n)
    return base, header


def write_baseline(path, counts, blocks, rustc):
    lines = [
        "# Instructions per 128-frame block for each mpfree-bench case, counted by",
        "# callgrind (tools/ci/instr_gate.py; it fails CI on a rise over 5 %).",
        f"# Toolchain: {rustc}. Blocks counted per case: {blocks}, after 50 warm-up blocks.",
        "# After an intended change, bless with CI's pinned toolchain:",
        "#   cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-bench",
        "#   python3 tools/ci/instr_gate.py --bless",
    ]
    for name in sorted(counts):
        lines.append(f"{counts[name]}\t{name}")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n")


def compare(base, now, tolerance):
    """(report lines, number of rises past the tolerance)."""
    rows, rises = [], 0
    width = max([len(n) for n in list(base) + list(now)] + [4])
    for name in sorted(set(base) | set(now)):
        b, n = base.get(name), now.get(name)
        if b is None:
            rows.append(f"  NEW   {name:<{width}} {n:>12,}  (not in the baseline; bless to add it)")
            continue
        if n is None:
            rows.append(f"  GONE  {name:<{width}} {b:>12,}  (in the baseline, no longer a case; bless to drop it)")
            continue
        change = (n - b) / b * 100 if b else 0.0
        if change > tolerance:
            word = "RISE"
            rises += 1
        elif change < -tolerance:
            word = "DROP"
        else:
            word = "ok"
        rows.append(f"  {word:<5} {name:<{width}} {b:>12,} -> {n:>12,}  {change:+6.1f} %")
    return rows, rises


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bench", default=DEFAULT_BENCH)
    ap.add_argument("--blocks", type=int, default=DEFAULT_BLOCKS)
    ap.add_argument("--filter", default=None)
    ap.add_argument("--tolerance", type=float, default=5.0, help="percent (default 5)")
    ap.add_argument("--baseline", default=DEFAULT_BASELINE)
    ap.add_argument("--bless", action="store_true", help="write the counts as the new baseline")
    a = ap.parse_args()
    if a.blocks <= 0 or a.tolerance <= 0:
        ap.error("--blocks and --tolerance must be above 0")
    if a.bless and a.filter:
        ap.error("--bless counts every case; leave out --filter")

    rustc = toolchain()
    now = count(a.bench, a.blocks, a.filter)
    if a.bless:
        write_baseline(a.baseline, now, a.blocks, rustc)
        print(f"instr_gate: wrote {len(now)} cases to {os.path.relpath(a.baseline, ROOT)} ({rustc})")
        return 0
    if not os.path.exists(a.baseline):
        print(f"instr_gate: no baseline at {a.baseline}; run with --bless to make one")
        return 2
    base, header = read_baseline(a.baseline)
    if a.filter:
        base = {k: v for k, v in base.items() if k in now}
    blocks_line = next((h for h in header if "Blocks counted per case:" in h), "")
    m = re.search(r"Blocks counted per case: (\d+)", blocks_line)
    if m and int(m.group(1)) != a.blocks:
        print(f"instr_gate: warning: the baseline counted {m.group(1)} blocks a case, this run {a.blocks}")
    tc_line = next((h for h in header if h.startswith("# Toolchain:")), "")
    if rustc not in tc_line:
        print(f"instr_gate: warning: the baseline was made with another toolchain ({tc_line[2:]}); "
              f"this is {rustc}, so counts can differ for that reason alone")
    rows, rises = compare(base, now, a.tolerance)
    print(f"Instructions per block, baseline -> now (tolerance {a.tolerance:g} %, {rustc}):")
    print("\n".join(rows))
    drops = sum(1 for r in rows if r.lstrip().startswith("DROP"))
    if drops:
        print(f"{drops} case(s) got cheaper by more than {a.tolerance:g} %: "
              "bless the baseline to keep the gain (python3 tools/ci/instr_gate.py --bless).")
    if rises:
        print(f"instr_gate: {rises} case(s) cost more than {a.tolerance:g} % above the baseline. "
              "If that is intended, say why in the PR and bless the baseline.")
        return 1
    print("instr_gate: no case rose past the tolerance.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
