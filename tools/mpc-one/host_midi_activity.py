#!/usr/bin/env python3
"""Guided, read-only MIDI activity mapping for the MPC One internal controller.

Only invoke through collect-over-ssh.sh --midi-activity, whose identity guard
first checks the direct Ethernet link, route and device MAC.

For each phase the owner performs one kind of physical action (pads, knobs,
buttons...) while this tool takes two snapshots of the kernel's cumulative
RawMIDI counters (/proc/asound/card0/midi0, /proc/asound/card0/usbmidi). The
per-substream byte deltas show which internal MIDI port carries which control.

Safety: every snapshot streams the reviewed read-only-midi-counters.sh to
`sh -s`; it reads two procfs text files. Nothing on the MPC is opened, sent,
started, stopped or written. The factory app keeps running normally.
"""
import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

SENTINEL = "MPFREE MIDI COUNTERS V1"

# Each phase: (key, instruction). Actions were chosen to change nothing
# persistent in the factory app: no saving, deleting or loading.
PHASES = (
    ("idle", "Touch NOTHING on the MPC (baseline: background LED/clock traffic)."),
    ("pads", "Tap several different pads, soft and hard."),
    ("pad_pressure", "Press and HOLD one pad, varying pressure (aftertouch), then release."),
    ("qlinks", "Turn the four Q-Link knobs back and forth (touching them also counts)."),
    ("data_wheel", "Turn the main data wheel left and right."),
    ("buttons", "Press SHIFT a few times, then press FULL LEVEL twice (on, then off again)."),
    ("transport", "Press PLAY, wait a second, then press STOP."),
    ("volume", "Turn the master volume knob slightly up, then back to where it was."),
)

LINE_HEADER = re.compile(r"^(Output|Input)\s+(\d+)\s*$")
LINE_VALUE = re.compile(r"^\s*([A-Za-z][A-Za-z ]*?)\s*:\s*(-?\d+)\s*$")


def parse_counters(text):
    """Return {(file, 'Output 0', 'Tx bytes'): int} from a counters snapshot."""
    if SENTINEL not in text:
        raise ValueError("counter snapshot sentinel missing")
    if "@@END" not in text:
        raise ValueError("counter snapshot truncated")
    values = {}
    current_file = None
    section = None
    for raw in text.splitlines():
        line = raw.rstrip()
        if line.startswith("@@FILE "):
            current_file = line[len("@@FILE "):].strip()
            section = None
            continue
        if line.startswith("@@"):
            continue
        m = LINE_HEADER.match(line.strip())
        if m:
            section = f"{m.group(1)} {m.group(2)}"
            continue
        m = LINE_VALUE.match(line)
        if m and current_file and section:
            key = m.group(1).strip()
            if key in ("Owner PID", "Mode", "Buffer size", "Avail"):
                continue  # not traffic counters
            values[(current_file, section, key)] = int(m.group(2))
    if not values:
        raise ValueError("no MIDI counters found in snapshot")
    return values


def deltas(before, after):
    out = {}
    for key in sorted(set(before) | set(after)):
        if key in before and key in after:
            out[key] = after[key] - before[key]
    return out


def format_phase(name, seconds, d):
    lines = [f"PHASE {name} window={seconds:.1f}s"]
    active = [(k, v) for k, v in d.items() if v != 0]
    if not active:
        lines.append("  no counter changed")
    for (f, section, key), v in active:
        short = f.rsplit("/", 1)[-1]
        lines.append(f"  {short:8s} {section:9s} {key:12s} +{v}  ({v / seconds:.1f}/s)")
    return "\n".join(lines)


def ssh_command(ip, port, user, key):
    return [
        "ssh", "-T", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes",
        "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=8",
        "-o", "ForwardAgent=no", "-o", "ClearAllForwardings=yes",
        "-i", key, "-p", str(port), f"{user}@{ip}", "sh -s",
    ]


def snapshot(args, script_bytes):
    p = subprocess.run(ssh_command(args.ip, args.port, args.user, args.key),
                       input=script_bytes, stdout=subprocess.PIPE,
                       stderr=subprocess.PIPE, timeout=20, check=False)
    if p.returncode != 0:
        raise RuntimeError("ssh snapshot failed: " + p.stderr.decode("utf-8", "replace")[:300])
    text = p.stdout.decode("utf-8", "replace")
    return text, parse_counters(text)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ip", required=True)
    ap.add_argument("--port", required=True, type=int)
    ap.add_argument("--user", required=True)
    ap.add_argument("--key", required=True)
    ap.add_argument("--script", required=True, help="read-only-midi-counters.sh")
    ap.add_argument("--report", required=True)
    ap.add_argument("--seconds", type=float, default=8.0)
    args = ap.parse_args(argv)
    if not 2.0 <= args.seconds <= 30.0:
        ap.error("--seconds must be 2..30")
    script_bytes = Path(args.script).read_bytes()

    report = open(args.report, "w", encoding="utf-8")

    def out(msg=""):
        print(msg)
        report.write(msg + "\n")
        report.flush()

    out("MPFREE MPC ONE GUIDED MIDI ACTIVITY MAP - READ ONLY")
    out(f"phases={len(PHASES)} window={args.seconds:.1f}s source=/proc/asound/card0/{{midi0,usbmidi}}")
    first_text, _ = snapshot(args, script_bytes)
    out("\nInitial counter snapshot:")
    out(first_text.rstrip())
    for i, (name, instruction) in enumerate(PHASES, 1):
        print(f"\n[{i}/{len(PHASES)}] {name}: {instruction}", file=sys.stderr)
        try:
            reply = input("Press Enter to start this phase (or type s then Enter to skip): ")
        except EOFError:
            out("stdin closed; stopping")
            break
        if reply.strip().lower() == "s":
            out(f"\nPHASE {name} skipped by owner")
            continue
        _, before = snapshot(args, script_bytes)
        t0 = time.monotonic()
        print(f"GO: do it now for {args.seconds:.0f} seconds...", file=sys.stderr, flush=True)
        time.sleep(args.seconds)
        _, after = snapshot(args, script_bytes)
        elapsed = time.monotonic() - t0
        print("STOP.", file=sys.stderr)
        out("\n" + format_phase(name, elapsed, deltas(before, after)))
    out("\nEND GUIDED MIDI ACTIVITY MAP")
    report.close()
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as e:
        print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)
