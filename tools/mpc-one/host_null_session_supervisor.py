#!/usr/bin/env python3
"""M7 LOCAL HOST ONLY: independent parent supervisor of a finite Rust session.

No SSH, IP addresses, device nodes, factory systemd commands or MPC execution.
This is a research model for future separate supervisor architecture.
It runs exactly the locally built mpfree-session --self-test and validates
null-audio results. It refuses arbitrary program paths/commands.
"""
from pathlib import Path
import argparse
import os
import re
import signal
import subprocess
import sys

REPO_ROOT = Path(__file__).resolve().parents[2]
BINARY = REPO_ROOT / "rust/target/release/mpfree-session"
SENTINEL = "MPFREE_SESSION_V1"
WATCHDOG = "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED"
MAX_SECONDS = 9


def parse_session(stdout: str, stderr: str, code: int):
    if code != 0:
        raise RuntimeError(
            f"host-only session failed with status={code}; "
            f"stdout_tail={stdout[-800:]!r}; stderr_tail={stderr[-800:]!r}"
        )
    lines = stdout.splitlines()
    if lines.count(SENTINEL) != 1:
        raise RuntimeError("wrong M7 version/sentinel")
    if stderr.splitlines().count(WATCHDOG) != 1:
        raise RuntimeError("Rust session watchdog never armed")
    fields = {}
    for line in lines:
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        if key in fields:
            raise RuntimeError(f"duplicate session result: {key}")
        fields[key] = value
    exact = {
        "os": "linux", "backend": "null", "pacing": "monotonic",
        "sample_rate_hz": "44100", "period_frames": "128",
        "frames": "88200", "callback_count": "690",
        "groove_events": "12", "sampler_notes": "4",
        "checksum_fnv1a64": "3dc4d6dabbb17fbd", "status": "PASS",
    }
    for key, value in exact.items():
        if fields.get(key) != value:
            raise RuntimeError(f"unexpected {key}: {fields.get(key)!r}")
    if fields.get("arch") not in ("x86_64", "aarch64", "arm"):
        raise RuntimeError("unsupported M7 architecture")
    ints = (
        "deadline_misses", "late_starts", "max_callback_us",
        "p99_callback_us", "max_start_lateness_us",
        "total_render_us", "wall_elapsed_us",
    )
    for key in ints:
        if not re.fullmatch(r"\d+", fields.get(key, "")):
            raise RuntimeError(f"missing/invalid nonnegative metric: {key}")
    for key in ("deadline_misses", "late_starts"):
        if int(fields[key]) > 690:
            raise RuntimeError(f"invalid {key}: exceeds callback count")
    if int(fields["p99_callback_us"]) > int(fields["max_callback_us"]):
        raise RuntimeError("p99 callback duration cannot exceed maximum")
    if int(fields["wall_elapsed_us"]) < 1:
        raise RuntimeError("wall-clock runtime must be positive")
    return fields


def end_process_group(process):
    """Best-effort host cleanup. Never signals the MPC or a factory service."""
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=0.5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=1)


def run_local(binary: Path = BINARY):
    # Caller must not override this in production CLI.
    if binary.resolve() != BINARY.resolve():
        raise ValueError("only the repository's local mpfree-session is permitted")
    if not binary.is_file():
        raise ValueError(
            "local executable unavailable: build mpfree-session --release first"
        )
    p = subprocess.Popen(
        [str(binary), "--self-test"],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, start_new_session=True,
    )
    try:
        try:
            raw_stdout, raw_stderr = p.communicate(timeout=MAX_SECONDS)
        except subprocess.TimeoutExpired:
            end_process_group(p)
            raise RuntimeError("host supervisor deadline expired; child group terminated")
    finally:
        end_process_group(p)
    stdout = raw_stdout.decode("utf-8", "replace")
    stderr = raw_stderr.decode("utf-8", "replace")
    metrics = parse_session(stdout, stderr, p.returncode)
    return metrics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()  # Any custom executable/remote address is disallowed.
    metrics = run_local()
    print("MPFREE_M7_HOST_PARENT_SUPERVISOR_V1")
    print("supervised_child=mpfree-session")
    print("hardware_access=none")
    print("child_watchdog=armed")
    for key in (
        "frames", "callback_count", "deadline_misses", "late_starts",
        "max_callback_us", "p99_callback_us",
        "max_start_lateness_us", "total_render_us", "wall_elapsed_us",
        "checksum_fnv1a64",
    ):
        print(f"{key}={metrics[key]}")
    print("status=PASS")
    print(
        "Synthetic wall-clock pacing only: not a real ALSA "
        "callback, xrun monitor, or MPC hardware test."
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as exc:
        print(f"FAIL CLOSED: {exc}", file=sys.stderr)
        sys.exit(2)
