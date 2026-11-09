#!/usr/bin/env python3
"""Optional bounded SSH+ICMP diagnostic. Runs on the workstation only.

The MPC receives only a read of /dev/zero or a write to /dev/null; no
firmware, configuration, audio devices, or persistent storage are modified.
Numbers are SSH goodput including startup/cipher overhead, NOT raw wire speed.
"""
import argparse
import os
import subprocess
import sys
import time

TRANSFER = 4 * 1024 * 1024  # 4 MiB each direction, hard-coded cap
CHUNK = 65536


def ssh_prefix(args):
    return [
        "timeout", "30", "ssh", "-T", "-o", "BatchMode=yes",
        "-o", "IdentitiesOnly=yes", "-o", "Compression=no",
        "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=8",
        "-o", "ForwardAgent=no", "-o", "ClearAllForwardings=yes",
        "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2",
        "-i", args.key, "-p", str(args.port),
        f"{args.user}@{args.ip}",
    ]


def goodput_mbit(total_bytes, seconds):
    if seconds <= 0:
        raise ValueError("elapsed seconds must be positive")
    return total_bytes * 8 / seconds / 1_000_000


def download(args):
    # Remote reads exactly 4 MiB from /dev/zero, writes only to SSH stdout.
    command = ssh_prefix(args) + ["dd if=/dev/zero bs=16384 count=256 2>/dev/null"]
    start = time.monotonic()
    proc = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    received = 0
    try:
        while True:
            data = proc.stdout.read(CHUNK)
            if not data:
                break
            received += len(data)
            if received > TRANSFER:
                raise RuntimeError("MPC sent more than the capped transfer")
        _, stderr = proc.communicate(timeout=10)
    except Exception:
        proc.kill()
        proc.communicate()
        raise
    elapsed = time.monotonic() - start
    if proc.returncode != 0 or received != TRANSFER:
        raise RuntimeError(
            f"MPC -> workstation SSH failed: rc={proc.returncode}, "
            f"bytes={received}, stderr={stderr.decode(errors='replace')[:1000]}"
        )
    return received, elapsed


def upload(args):
    # Remote writes ONLY to /dev/null; EOF ends a bounded host-supplied stream.
    command = ssh_prefix(args) + ["cat > /dev/null"]
    start = time.monotonic()
    proc = subprocess.Popen(command, stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        remaining = TRANSFER
        payload = os.urandom(CHUNK)
        while remaining:
            n = min(remaining, CHUNK)
            proc.stdin.write(payload[:n])
            remaining -= n
        proc.stdin.close()
        # The remote 'cat' exits when SSH sends EOF. No remote filesystem writes.
        rc = proc.wait(timeout=30)
        stderr = proc.stderr.read()
        stdout = proc.stdout.read()
    except Exception:
        proc.kill()
        proc.wait()
        raise
    elapsed = time.monotonic() - start
    if rc != 0 or stdout:
        raise RuntimeError(
            f"workstation -> MPC SSH failed: rc={rc}, "
            f"stderr={stderr.decode(errors='replace')[:1000]}"
        )
    return TRANSFER, elapsed


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ip", required=True)
    ap.add_argument("--user", required=True)
    ap.add_argument("--key", required=True)
    ap.add_argument("--iface", required=True)
    ap.add_argument("--port", required=True, type=int)
    args = ap.parse_args()

    print("MPFree OPTIONAL ACTIVE Ethernet diagnostic", flush=True)
    print("Both directions: 4 MiB over verified SSH, compression disabled.")
    print("Do not run during recording or live performance.", flush=True)

    ping = subprocess.run(
        ["timeout", "12", "ping", "-I", args.iface,
         "-n", "-c", "20", "-i", "0.2", "-W", "1", args.ip],
        text=True, capture_output=True, timeout=15,
    )
    print("\n--- ICMP round-trip times (56-byte ping payload) ---")
    for line in ping.stdout.splitlines():
        if ("icmp_seq=" in line or "packet loss" in line
                or line.startswith("rtt ") or line.startswith("round-trip")):
            print(line)
    if ping.returncode != 0:
        print(f"ICMP rc={ping.returncode}, stderr={ping.stderr.strip()}")
    else:
        print("ICMP RTT is not one-way audio latency or packet jitter under DSP load.")

    for label, fn in (
        ("MPC → workstation", download),
        ("workstation → MPC", upload),
    ):
        print(f"\n--- {label}: encrypted SSH goodput ---", flush=True)
        nbytes, elapsed = fn(args)
        print(f"{nbytes} bytes / {elapsed:.3f} seconds = "
              f"{goodput_mbit(nbytes, elapsed):.2f} Mbit/s")
        print("Includes SSH startup and CPU crypto; underestimates native link capacity.")
    print("\nDone. No changes to MPC firmware, network settings or audio devices.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.SubprocessError, RuntimeError, ValueError) as exc:
        print(f"BENCHMARK FAILED SAFELY: {exc}", file=sys.stderr)
        sys.exit(1)
