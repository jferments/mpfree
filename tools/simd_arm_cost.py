#!/usr/bin/env python3
"""Instructions per 128-frame block for each mpfree-simd kernel and its plain
Rust twin, as ARMv7 code (the MPC One's), counted under qemu.

    python3 tools/simd_arm_cost.py [--blocks N] [--no-build] [--gate]

- It builds `rust/mpfree-simd/examples/simd_cost.rs` for
  armv7-unknown-linux-gnueabihf (linker arm-linux-gnueabihf-gcc, as CI's
  ARMv7 job), then runs it under
  `qemu-arm -L /usr/arm-linux-gnueabihf -one-insn-per-tb -d exec,nochain`,
  where every instruction run prints one `Trace` line. That is the method
  of tools/inst_arm_cost.py.
- Each path (plain and NEON) is counted twice, with N blocks after the
  warm-up and with none; the difference over N is its instructions per
  block. The four runs go side by side.
- Each run prints the path that ran and a hash of its last block. The two
  paths must print the same hash (the kernel matches its twin bit for bit)
  and the NEON run must say NEON, or the script fails.
- With --gate it also fails when the NEON path doesn't take fewer
  instructions than the plain one: a kernel that runs more instructions
  than its twin can't be winning. Fewer instructions don't prove it wins:
  NEON's loads and moves cost more cycles each, so the device round's
  timing (mpfree-bench's "Supersaw saws" cases) decides whether it stays.

Needs qemu-user (qemu-arm), the arm-linux-gnueabihf cross toolchain and the
Rust armv7-unknown-linux-gnueabihf target. It writes nothing outside
rust/target. Under qemu's trace about 300k instructions run a second; 50
blocks take about a minute.
"""

import argparse
import concurrent.futures
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = "armv7-unknown-linux-gnueabihf"
EXAMPLE = os.path.join(ROOT, "rust", "target", TARGET, "release", "examples", "simd_cost")
QEMU = ["qemu-arm", "-L", "/usr/arm-linux-gnueabihf"]
PATHS = ["plain", "neon"]


def build():
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER", "arm-linux-gnueabihf-gcc")
    subprocess.run(
        ["cargo", "build", "--manifest-path", os.path.join(ROOT, "rust", "Cargo.toml"),
         "--release", "--locked", "-p", "mpfree-simd", "--example", "simd_cost",
         "--target", TARGET],
        env=env, check=True)


def trace(path, blocks):
    """Instructions qemu ran for `path` with `blocks` measured blocks, and
    the line the program printed."""
    env = dict(os.environ)
    env["SIMD_COST_PATH"] = path
    env["SIMD_COST_BLOCKS"] = str(blocks)
    p = subprocess.Popen(QEMU + ["-one-insn-per-tb", "-d", "exec,nochain", EXAMPLE],
                         env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    count = 0
    # qemu writes its log to stderr; the program's own line goes to stdout.
    for chunk in iter(lambda: p.stderr.read(1 << 22), b""):
        count += chunk.count(b"Trace ")
    out = p.stdout.read().decode().strip()
    if p.wait() != 0:
        raise RuntimeError(f"{path}: qemu exited with {p.returncode}")
    return count, out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--blocks", type=int, default=50, help="measured blocks (default 50)")
    ap.add_argument("--no-build", action="store_true", help="use the example as built")
    ap.add_argument("--gate", action="store_true",
                    help="fail unless NEON runs fewer instructions than plain")
    args = ap.parse_args()
    if args.blocks < 1:
        sys.exit("--blocks takes a number above 0")
    if not args.no_build:
        build()
    runs = [(path, blocks) for path in PATHS for blocks in (0, args.blocks)]
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(runs)) as ex:
        got = dict(zip(runs, ex.map(lambda r: trace(*r), runs)))
    per = {}
    lines = {}
    for path in PATHS:
        (n0, _), (n1, line) = got[(path, 0)], got[(path, args.blocks)]
        per[path] = (n1 - n0) / args.blocks
        lines[path] = line
    print(f"ARMv7 instructions per 128-frame block, counted under qemu over {args.blocks} blocks:")
    print(f"  {'case':<36} {'plain':>10} {'NEON':>10} {'NEON/plain':>11}")
    ratio = per["neon"] / per["plain"]
    print(f"  {'Supersaw saws: 8 voices':<36} {per['plain']:>10.0f} {per['neon']:>10.0f}"
          f" {ratio:>10.1%}")
    hashes = {path: lines[path].rsplit(None, 1)[-1] for path in PATHS}
    failed = False
    if " NEON " not in f" {lines['neon']} ":
        print(f"FAIL: the NEON run ran the twin: {lines['neon']!r}")
        failed = True
    if hashes["plain"] != hashes["neon"]:
        print(f"FAIL: the paths' samples differ: {lines['plain']!r} vs {lines['neon']!r}")
        failed = True
    else:
        print(f"  same samples on both paths (hash {hashes['plain']})")
    if args.gate and per["neon"] >= per["plain"]:
        print("FAIL: the NEON kernel runs no fewer instructions than its twin")
        failed = True
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
