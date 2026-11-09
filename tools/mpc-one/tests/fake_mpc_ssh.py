#!/usr/bin/env python3
"""Stand-in for `ssh` in the eMMC backup tests. Never contacts a network.

It emulates the MPC with a fake tree (FAKE_MPC_ROOT) holding dev/, sys/ and
proc/ files. It accepts only the remote commands host_emmc_backup.py is
allowed to send and exits 255 on anything else, so the tests also prove
the helper sends nothing beyond its fixed command list. Every remote
command is appended to FAKE_MPC_LOG.

Fault injection (set by the tests):
  FAKE_MPC_ON_STREAM   comma list applied after /dev/mmcblk0 is served:
                       write:<dev>, discard:<dev>, reboot
  FAKE_MPC_CORRUPT     <dev>:<byte offset> flips one byte in that stream
  FAKE_MPC_TRUNCATE    <dev>:<bytes> sends only that many bytes
  FAKE_MPC_DD_RC       <dev>:<exit status> for the remote dd
  FAKE_MPC_PAUSE       <dev>:<bytes> sends that many bytes, then hangs for 60 s
"""
import os
import re
import subprocess
import sys
import time

ROOT = os.environ["FAKE_MPC_ROOT"]
OPTIONS_WITH_VALUE = {"-o", "-i", "-p", "-l", "-F"}


def fault(name, dev):
    for item in filter(None, os.environ.get(name, "").split(",")):
        d, _, value = item.partition(":")
        if d == dev:
            return int(value)
    return None


def bump(dev, fields):
    path = os.path.join(ROOT, "sys/class/block", dev, "stat")
    with open(path) as fh:
        values = [int(x) for x in fh.read().split()]
    for i in fields:
        values[i] += 8
    with open(path, "w") as fh:
        fh.write(" ".join(f"{v:8d}" for v in values) + "\n")


def apply_stream_side_effects():
    for action in filter(None, os.environ.get("FAKE_MPC_ON_STREAM", "").split(",")):
        kind, _, dev = action.partition(":")
        if kind == "write":
            bump(dev, (4, 6))
            bump("mmcblk0", (4, 6))
        elif kind == "discard":
            bump(dev, (11, 13))
        elif kind == "reboot":
            with open(os.path.join(ROOT, "proc/sys/kernel/random/boot_id"), "w") as fh:
                fh.write("99999999-0000-4000-8000-000000000000\n")
        else:
            sys.exit(f"fake ssh: unknown side effect {action}")


def main():
    args = sys.argv[1:]
    i = 0
    while i < len(args) and args[i].startswith("-"):
        i += 2 if args[i] in OPTIONS_WITH_VALUE else 1
    if len(args) != i + 2:
        print("fake ssh: expected exactly one destination and one command", file=sys.stderr)
        return 255
    command = args[i + 1]
    with open(os.environ["FAKE_MPC_LOG"], "a") as fh:
        fh.write(command + "\n")

    if command in ("sh -s hash", "sh -s stat"):
        env = dict(os.environ, MPFREE_TEST_ROOT=ROOT,
                   PATH=os.environ["FAKE_MPC_BUSYBOX_PATH"])
        return subprocess.run(["sh", "-s", command.split()[-1]], env=env).returncode

    m = re.fullmatch(r"dd if=/dev/(mmcblk0|mmcblk0boot0|mmcblk0boot1) bs=1048576", command)
    if not m:
        print(f"fake ssh: command not allowed on the fake MPC: {command}", file=sys.stderr)
        return 255
    dev = m.group(1)
    with open(os.path.join(ROOT, "dev", dev), "rb") as fh:
        data = bytearray(fh.read())
    offset = fault("FAKE_MPC_CORRUPT", dev)
    if offset is not None:
        data[offset] ^= 0xFF
    limit = fault("FAKE_MPC_TRUNCATE", dev)
    if limit is not None:
        data = data[:limit]
    out = sys.stdout.buffer
    pause = fault("FAKE_MPC_PAUSE", dev)
    for start in range(0, len(data), 65536):
        if pause is not None and start >= pause:
            out.flush()
            time.sleep(60)
        out.write(data[start:start + 65536])
    out.flush()
    blocks = len(data) // 1048576
    partial = 1 if len(data) % 1048576 else 0
    print(f"{blocks}+{partial} records in\n{blocks}+{partial} records out", file=sys.stderr)
    if dev == "mmcblk0":
        apply_stream_side_effects()
    rc = fault("FAKE_MPC_DD_RC", dev)
    return rc if rc is not None else 0


if __name__ == "__main__":
    sys.exit(main())
