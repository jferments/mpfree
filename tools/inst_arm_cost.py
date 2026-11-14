#!/usr/bin/env python3
"""Instructions per 128-frame block for Keys, Drawbar, Squelch and Analog as
ARMv7 code (the MPC One's), counted under qemu.

    python3 tools/inst_arm_cost.py [--blocks N] [--jobs J] [--no-build] [CASE ...]

- It builds `rust/mpfree-synth/examples/inst_cost.rs` for
  armv7-unknown-linux-gnueabihf (linker arm-linux-gnueabihf-gcc, as CI's
  ARMv7 step), then runs each case under
  `qemu-arm -L /usr/arm-linux-gnueabihf -one-insn-per-tb -d exec,nochain`,
  where every instruction run prints one `Trace` line.
- Each case is counted twice, with N blocks after the warm-up and with none;
  the difference over N is its instructions per block. That is the method
  of docs/dev/design/fx.md for the effects.
- The cases are the bench's own (mpfree-bench), so the counts sit beside
  the x86 callgrind counts in rust/mpfree-bench/instructions.txt.
- Instructions, not cycles: cache misses and VFP stalls come on top, so a
  run on the device still decides voice counts.
- It also prints each case's output hash under qemu, which must match the
  native build's (`cargo run --release -p mpfree-synth --example inst_cost`).

Needs qemu-user (qemu-arm), the arm-linux-gnueabihf cross toolchain and the
Rust armv7-unknown-linux-gnueabihf target. It only builds and runs the
example; it writes nothing outside rust/target. Under qemu's trace about
300k instructions run a second, so 200 blocks of a 16-voice case take
minutes; the cases run J at a time (4 by default).
"""

import argparse
import concurrent.futures
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = "armv7-unknown-linux-gnueabihf"
EXAMPLE = os.path.join(ROOT, "rust", "target", TARGET, "release", "examples", "inst_cost")
QEMU = ["qemu-arm", "-L", "/usr/arm-linux-gnueabihf"]


def build():
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER", "arm-linux-gnueabihf-gcc")
    subprocess.run(
        ["cargo", "build", "--manifest-path", os.path.join(ROOT, "rust", "Cargo.toml"),
         "--release", "--locked", "-p", "mpfree-synth", "--example", "inst_cost",
         "--target", TARGET],
        env=env, check=True)


def run_env(case, blocks):
    env = dict(os.environ)
    env["INST_COST_BLOCKS"] = str(blocks)
    if case is not None:
        env["INST_COST_ONLY"] = case
    return env


def cases():
    """(name, hash) of every case, from one run with no measured blocks."""
    out = subprocess.run(QEMU + [EXAMPLE], env=run_env(None, 0), capture_output=True,
                         text=True, check=True).stdout
    return [line.rsplit(None, 1)[0].strip() for line in out.splitlines() if line.strip()]


def trace_lines(case, blocks):
    """Instructions qemu ran for `case` with `blocks` measured blocks, and the
    output hash it printed."""
    p = subprocess.Popen(QEMU + ["-one-insn-per-tb", "-d", "exec,nochain", EXAMPLE],
                         env=run_env(case, blocks), stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE)
    count = 0
    # qemu writes its log to stderr; the program's own line goes to stdout.
    for chunk in iter(lambda: p.stderr.read(1 << 22), b""):
        count += chunk.count(b"Trace ")
    out = p.stdout.read().decode().strip()
    if p.wait() != 0:
        raise RuntimeError(f"{case}: qemu exited with {p.returncode}")
    return count, out.rsplit(None, 1)[-1] if out else ""


def measure(case, blocks):
    base, _ = trace_lines(case, 0)
    full, digest = trace_lines(case, blocks)
    return case, (full - base) / blocks, digest


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--blocks", type=int, default=200)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("case", nargs="*", help="exact case names (all when none)")
    a = ap.parse_args()
    if a.blocks < 1:
        ap.error("--blocks must be 1 or more")
    if not a.no_build:
        build()
    names = a.case or cases()
    print(f"ARMv7 instructions per 128-frame block ({a.blocks} blocks after 50 to warm up)")
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as pool:
        for case, per, digest in pool.map(lambda c: measure(c, a.blocks), names):
            print(f"{case:<30} {per / 1000:8.1f}k   {digest}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
