#!/usr/bin/env python3
"""Guided passive USB-MIDI capture of the MPC One internal controller (usbmon).

Only invoke through `collect-over-ssh.sh --midi-capture`, which checks the
direct Ethernet link, route and device MAC and requires the owner opt-in
MPFREE_ACCEPT_USBMON=YES.

The remote side (passive-usbmon-capture.sh) streams the kernel's usbmon text
interface for the controller's USB bus for at most 240 s. This tool tags each
line with the phase the owner is performing, decodes USB-MIDI event packets
(4 bytes: cable/CIN + 3 MIDI bytes), reassembles SysEx, and writes a report.

Nothing is sent to the controller and nothing is written on the MPC.
usbmon's text interface shows at most 32 data bytes per transfer, so longer
transfers (for example big LED SysEx bursts) are marked as truncated.
"""
import argparse
import collections
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

SENTINEL = "MPFREE PASSIVE USBMON CAPTURE V1"
PHASE_SECONDS = 8.0

PHASES = (
    ("idle", "Touch NOTHING (baseline)."),
    ("pad1_soft_hard", "Hit pad 1 (bottom-left) once softly, then once hard. Nothing else."),
    ("pads_1_to_16", "Hit pads 1 to 16 in order, one at a time (bottom-left to top-right, row by row)."),
    ("pad_pressure", "Press and HOLD pad 1, vary the pressure, then release."),
    ("qlink1", "Touch and turn Q-Link 1 (top knob) slowly right, then left."),
    ("qlinks_2_to_4", "Touch and turn Q-Links 2, 3 and 4, one after the other."),
    ("data_wheel", "Turn the data wheel 3 clicks right, then 3 clicks left."),
    ("shift", "Press and release SHIFT three times."),
    ("full_level", "Press FULL LEVEL twice (on, then off again)."),
    ("play_stop", "Press PLAY, wait a second, then press STOP."),
)

CIN_LEN = {0x0: 3, 0x1: 3, 0x2: 2, 0x3: 3, 0x4: 3, 0x5: 1, 0x6: 2, 0x7: 3,
           0x8: 3, 0x9: 3, 0xA: 3, 0xB: 3, 0xC: 2, 0xD: 2, 0xE: 3, 0xF: 1}
ADDR = re.compile(r"^([CIBZ])([io]):(\d+):(\d+):(\d+)$")


def parse_usbmon_line(line, want_bus, want_dev):
    """Return (direction, bytes, declared_length) or None.

    Text format (Documentation/usb/usbmon.rst):
    tag timestamp event addr status length '=' word word ...
    OUT data is on the S(ubmit) event, IN data on the C(omplete) event.
    """
    parts = line.split()
    if len(parts) < 6:
        return None
    event, addr = parts[2], parts[3]
    m = ADDR.match(addr)
    if not m:
        return None
    xfer, direction, bus, dev = m.group(1), m.group(2), int(m.group(3)), int(m.group(4))
    if xfer != "B" or bus != want_bus or dev != want_dev:
        return None
    if not ((direction == "o" and event == "S") or (direction == "i" and event == "C")):
        return None
    if "=" not in parts:
        return None
    eq = parts.index("=")
    try:
        declared = int(parts[eq - 1])
    except ValueError:
        return None
    data = bytearray()
    for word in parts[eq + 1:]:
        if not re.fullmatch(r"[0-9a-fA-F]+", word) or len(word) % 2:
            break
        data.extend(bytes.fromhex(word))
    return ("OUT" if direction == "o" else "IN"), bytes(data), declared


class MidiDecoder:
    """USB-MIDI 1.0 event packets -> MIDI messages, with SysEx reassembly."""

    def __init__(self):
        self.sysex = {}

    def feed(self, direction, data):
        out = []
        for i in range(0, len(data) - len(data) % 4, 4):
            pkt = data[i:i + 4]
            if pkt == b"\x00\x00\x00\x00":
                continue
            cable, cin = pkt[0] >> 4, pkt[0] & 0x0F
            body = pkt[1:1 + CIN_LEN[cin]]
            key = (direction, cable)
            if cin == 0x4:
                self.sysex.setdefault(key, bytearray()).extend(body)
            elif cin in (0x5, 0x6, 0x7) and key in self.sysex:
                buf = self.sysex.pop(key)
                buf.extend(body)
                out.append((direction, cable, bytes(buf)))
            else:
                out.append((direction, cable, bytes(body)))
        return out


def describe(msg):
    if not msg:
        return "empty"
    s = msg[0]
    hexs = " ".join(f"{b:02x}" for b in msg)
    kind = {0x80: "note_off", 0x90: "note_on", 0xA0: "poly_pressure", 0xB0: "cc",
            0xC0: "program", 0xD0: "chan_pressure", 0xE0: "pitch_bend"}.get(s & 0xF0)
    if s == 0xF0:
        return f"sysex len={len(msg)} [{hexs}]"
    if kind:
        args = " ".join(str(b) for b in msg[1:])
        return f"{kind} ch={(s & 0x0F) + 1} {args} [{hexs}]"
    return f"other [{hexs}]"


def summary_key(direction, cable, msg):
    if not msg:
        return (direction, cable, "empty")
    s = msg[0]
    if s == 0xF0:
        return (direction, cable, "sysex " + " ".join(f"{b:02x}" for b in msg[:6]))
    if s & 0xF0 in (0x80, 0x90, 0xA0, 0xB0) and len(msg) > 1:
        return (direction, cable, f"{s:02x} {msg[1]}")
    return (direction, cable, f"{s:02x}")


def ssh_command(ip, port, user, key):
    return [
        "ssh", "-T", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes",
        "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=8",
        "-o", "ForwardAgent=no", "-o", "ClearAllForwardings=yes",
        "-i", key, "-p", str(port), f"{user}@{ip}", "sh -s",
    ]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    for a in ("--ip", "--user", "--key", "--script", "--report"):
        ap.add_argument(a, required=True)
    ap.add_argument("--port", required=True, type=int)
    args = ap.parse_args(argv)

    report = open(args.report, "w", encoding="utf-8")

    def out(msg=""):
        print(msg)
        report.write(msg + "\n")
        report.flush()

    proc = subprocess.Popen(ssh_command(args.ip, args.port, args.user, args.key),
                            stdin=open(args.script, "rb"), stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE)
    lock = threading.Lock()
    state = {"phase": "gap", "ended": False, "started": False, "abort": None,
             "bus": None, "dev": None}
    lines = []  # (phase, line)

    def reader():
        for raw in proc.stdout:
            line = raw.decode("utf-8", "replace").rstrip("\n")
            with lock:
                if line.startswith("@@ABORT"):
                    state["abort"] = line
                elif line.startswith("@@BUS "):
                    f = line.split()
                    state["bus"], state["dev"] = int(f[1]), int(f[3])
                elif line == "@@CAPTURE START":
                    state["started"] = True
                elif line == "@@CAPTURE END":
                    state["ended"] = True
                lines.append((state["phase"], line))
        with lock:
            state["ended"] = True

    t = threading.Thread(target=reader, daemon=True)
    t.start()

    out("MPFREE MPC ONE GUIDED PASSIVE USB-MIDI CAPTURE")
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        with lock:
            if state["started"] or state["abort"] or state["ended"]:
                break
        time.sleep(0.1)
    with lock:
        started, abort = state["started"], state["abort"]
    if not started:
        proc.kill()
        err = proc.stderr.read().decode("utf-8", "replace")[:400] if proc.stderr else ""
        out(f"CAPTURE DID NOT START: {abort or 'no start marker'} {err}")
        report.close()
        return 1
    out(f"capture running on usb bus {state['bus']} device {state['dev']} (max 240 s)")

    for i, (name, instruction) in enumerate(PHASES, 1):
        with lock:
            if state["ended"]:
                out("capture window ended on the MPC; stopping early")
                break
        print(f"\n[{i}/{len(PHASES)}] {name}: {instruction}", file=sys.stderr)
        try:
            reply = input("Press Enter to start (s + Enter to skip): ")
        except EOFError:
            break
        if reply.strip().lower() == "s":
            continue
        with lock:
            state["phase"] = name
        print(f"GO ({PHASE_SECONDS:.0f} s)...", file=sys.stderr, flush=True)
        time.sleep(PHASE_SECONDS)
        with lock:
            state["phase"] = "gap"
        print("STOP.", file=sys.stderr)

    # Closing our side ends the remote cat at its next USB event (sub-second
    # given the controller's idle traffic); the remote script is bounded anyway.
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
    t.join(timeout=5)

    out("")
    out(analyze(lines, state["bus"], state["dev"]))
    out("\nRAW USBMON LINES (tagged by phase):")
    for phase, line in lines:
        out(f"{phase}\t{line}")
    out("\nEND GUIDED PASSIVE USB-MIDI CAPTURE")
    report.close()
    return 0


def analyze(lines, bus, dev):
    dec = MidiDecoder()
    per_phase = collections.OrderedDict()
    truncated = collections.Counter()
    for phase, line in lines:
        r = parse_usbmon_line(line, bus, dev)
        if r is None:
            continue
        direction, data, declared = r
        if declared > len(data):
            truncated[phase] += 1
        for d, cable, msg in dec.feed(direction, data):
            per_phase.setdefault(phase, []).append((d, cable, msg))
    res = ["DECODED USB-MIDI BY PHASE"]
    for phase, msgs in per_phase.items():
        res.append(f"\n== {phase}: {len(msgs)} messages"
                   + (f", {truncated[phase]} truncated transfers" if truncated[phase] else ""))
        counts = collections.Counter(summary_key(d, c, m) for d, c, m in msgs)
        for (d, c, k), n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))[:40]:
            res.append(f"  {n:5d}x {d:3s} cable{c} {k}")
        res.append("  first messages:")
        for d, c, m in msgs[:60]:
            res.append(f"    {d:3s} cable{c} {describe(m)}")
    return "\n".join(res)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as e:
        print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)
