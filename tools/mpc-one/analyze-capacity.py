#!/usr/bin/env python3
"""Offline interpretation of MPC's read-only --capacity report.

No SSH, hardware access or side effects. Uses /proc/stat and per-TID user+
system CPU tick deltas, sampled over the same interval. All CPU numbers are
fractions of sampled capacity, not claims that any core is "reserved".
"""
from pathlib import Path
import argparse
import re
import sys

CORE = re.compile(r"^(cpu\d+) total_ticks=(\d+) idle_iowait_ticks=(\d+)$")
APP = re.compile(r"^factory_thread_ticks=(\d+) sampled_threads=(\d+)$")
MARK_A = "=== CPU counters snapshot A"
MARK_B = "=== CPU counters snapshot B"


def parse(text):
    if "MPFREE STOCK RESOURCE BUDGET SURVEY - READ ONLY" not in text:
        raise ValueError("not a read-only MPFree capacity report")
    if MARK_A not in text or MARK_B not in text:
        raise ValueError("CPU snapshots incomplete")
    before_b = text.split(MARK_A, 1)[1].split(MARK_B, 1)[0]
    after_b = text.split(MARK_B, 1)[1]
    def extract(section):
        cores = {}
        app = None
        for line in section.splitlines():
            m = CORE.fullmatch(line.strip())
            if m:
                if m.group(1) in cores:
                    raise ValueError("duplicate core counter")
                cores[m.group(1)] = (int(m.group(2)), int(m.group(3)))
            m = APP.fullmatch(line.strip())
            if m:
                if app is not None:
                    raise ValueError("duplicate app counter")
                app = (int(m.group(1)), int(m.group(2)))
        if not cores or app is None:
            raise ValueError("missing CPU or factory counters")
        return cores, app
    old, old_app = extract(before_b)
    new, new_app = extract(after_b)
    if old.keys() != new.keys():
        raise ValueError("different CPU topology between snapshots")
    ratios = {}
    total_capacity_ticks = 0
    for core in sorted(old):
        t0, idle0 = old[core]
        t1, idle1 = new[core]
        dt = t1 - t0
        di = idle1 - idle0
        if dt <= 0 or di < 0 or di > dt:
            raise ValueError(f"nonmonotonic CPU counters on {core}")
        busy = dt - di
        ratios[core] = 100.0 * busy / dt
        total_capacity_ticks += dt
    app_delta = new_app[0] - old_app[0]
    if app_delta < 0:
        raise ValueError("factory process or counter reset during sample")
    total_percent = 100.0 * app_delta / total_capacity_ticks
    return {
        "cores": ratios,
        "all_core_factory_percent": total_percent,
        "single_core_equivalent_percent": total_percent * len(ratios),
        "thread_count_before": old_app[1],
        "thread_count_after": new_app[1],
    }


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("report", type=Path)
    args = p.parse_args(argv)
    result = parse(args.report.read_text(encoding="utf-8", errors="replace"))
    print("Observed per-core *total* busy fraction during brief sample:")
    for core, pct in result["cores"].items():
        print(f"  {core}: {pct:.1f}% busy (ALL processes and interrupts)")
    print(f"Factory process CPU: {result['single_core_equivalent_percent']:.1f}% "
          "of one core's capacity")
    print(f"Factory process CPU: {result['all_core_factory_percent']:.1f}% "
          "of total four-core capacity")
    print(f"Task count: {result['thread_count_before']} -> {result['thread_count_after']}")
    print("CAUTION: short snapshot; pins/affinity do not indicate sustained utilization.")
    print("CAUTION: process CPU total is not a per-core breakdown; no kernel/OS reservation is implied.")
    print("Profile idle and representative playing/synthesis loads separately.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError) as e:
        print(f"ABORT: {e}", file=sys.stderr)
        sys.exit(2)
