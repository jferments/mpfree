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

Phase sets (--phases, or MPFREE_CAPTURE_PHASES in collect-over-ssh.sh):
- controller (default): the pads, Q-Links, wheel and buttons, as in 2026-11-10.
- midi-cv: which USB-MIDI cable carries the DIN MIDI In and Out jacks, and
  whether the CV/Gate jacks are driven over USB-MIDI at all and in what
  form. The owner sets up a MIDI track and a CV track in Akai's app before
  starting (see MIDI_CV_SETUP); the capture itself only reads.
- cv-scale: the second round. A keyboard on MIDI In plays known notes into
  the CV track, so the CV messages can be matched to note numbers; one CV
  output moved; the MIDI track's output tried again (CV_SCALE_SETUP).
- midi-out: the third round, once a synth on the 5-pin MIDI Out jack has
  been heard playing from a MIDI track (Akai's MIDI Out routing set): which
  cable carries MIDI Out, and how notes, a held note, pad velocities and
  (optionally) clock leave on it (MIDI_OUT_SETUP).
- cv-ports: CV ports 5 to 8. A CV track's CV Port and Gate Port are set to
  5 and 6, then 7 and 8, and one note played each time, so the capture shows
  which control changes carry them (CC 8 to 15 by the pattern of ports 1 to
  4, never seen). MPFree sends ports 5 to 8 only once this has shown them
  (CV_PORTS_SETUP).
- transport: which light messages Akai's app sends for REC, OVERDUB, PLAY,
  PLAY START and STOP as the transport plays, records and overdubs. Their
  lights were never in Akai's startup lights, so mpfree does not send them
  until this shows them (owner, 2026-11-13). The REC phase records into the
  open project as ordinary use does, and the owner does not save it
  (TRANSPORT_SETUP).
usbmon's text interface shows at most 32 data bytes per transfer, so longer
transfers (for example big LED SysEx bursts) are marked as truncated.
"""
import argparse
import collections
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

SENTINEL = "MPFREE PASSIVE USBMON CAPTURE V1"
PHASE_SECONDS = 8.0
# Tests set this so a run takes seconds; it changes nothing else.
if os.environ.get("MPFREE_MIDI_CAPTURE_TEST") == "1":
    PHASE_SECONDS = 0.3

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

MIDI_CV_PHASES = (
    ("idle", "Touch NOTHING (baseline). Akai's app shows the MIDI track."),
    ("din_in", "Play a few notes on the device plugged into the MPC's 5-pin MIDI In jack "
               "(s + Enter to skip if nothing is plugged in)."),
    ("din_out", "With the MIDI track selected, hit pads 1, 5, 9 and 13, one at a time."),
    ("cv_notes", "Select the CV track. Hit pad 1, then pad 13, one at a time."),
    ("cv_hold", "With the CV track still selected, HOLD pad 1 for about 3 s, then let go."),
    ("cv_slider", "On the CV track's overview of CV outputs, move ONE slider slowly up, then down "
                  "(s + Enter to skip if you cannot find it)."),
)

# Second round (after the 2026-11-12 midi-cv capture: DIN In is cable 2 in;
# the CV track's notes went out as CCs on cable 1, channel 16; nothing went
# out on cable 2). The keyboard on MIDI In plays known note numbers, which
# the capture also sees on cable 2, so the CV values can be matched to
# notes without knowing Akai's pad layout.
CV_SCALE_PHASES = (
    ("idle", "Touch NOTHING (baseline). Akai's app shows the CV track."),
    ("cv_octaves", "On the keyboard on MIDI In, play one C, then the C an octave up, then the C "
                   "another octave up, one key at a time."),
    ("cv_semitones", "On the keyboard, play C, C sharp, D, D sharp, one key at a time."),
    ("cv_out2", "Set the CV track to use the next CV outputs (the guide's Routing tab: the CV "
                "field of each CV Out), then play one C on the keyboard (s + Enter to skip)."),
    ("din_out", "Select the MIDI track (output MIDI Out, channel 1) and play a few keys on the "
                "keyboard, then hit pads 1 and 13."),
)

# Third round (after the owner found the MIDI track's routing that makes a
# synth on the MIDI Out jack play, 2026-11-12). The keyboard on MIDI In
# (cable 2 in) plays known notes into the MIDI track, so each one can be
# matched to what goes out.
MIDI_OUT_PHASES = (
    ("idle", "Touch NOTHING (baseline). Akai's app shows the MIDI track."),
    ("out_keys", "With the MIDI track selected, play C, then E, then G on the keyboard, one key "
                 "at a time. The synth on MIDI Out should sound."),
    ("out_hold", "HOLD one key on the keyboard for about 3 s, then let go."),
    ("out_pads", "Hit pad 1 softly, then pad 1 hard, then pad 13 once."),
    ("out_clock", "Only if Sync is already ticked for the MIDI Out port (Preferences > MIDI / "
                  "Sync): press PLAY, wait 3 s, press STOP. Otherwise s + Enter to skip."),
)

# Shown before the capture starts: what to prepare in Akai's app. The menu
# names are from the MPC 3.6 user guide (Track types, MIDI Tracks, CV
# Tracks) and are not checked on this unit.
MIDI_CV_SETUP = (
    "Before you press Enter, in Akai's app (stock 3.6, normal use, nothing saved):",
    "  1. Make a new MIDI track and set its output to the MIDI Out jack, channel 1.",
    "  2. Make a new CV track (it drives the CV/Gate jacks; nothing needs to be plugged in).",
    "  3. Select the MIDI track. Do not save the project or change Preferences.",
    "  4. Optional: plug a keyboard or other MIDI device into the 5-pin MIDI In jack.",
)

CV_SCALE_SETUP = (
    "Before you press Enter, in Akai's app (stock 3.6, normal use, nothing saved):",
    "  1. Keep the keyboard plugged into the 5-pin MIDI In jack.",
    "  2. Keep (or make again) the MIDI track with its output on the MIDI Out jack, channel 1.",
    "     Note down exactly what its output field says.",
    "  3. Keep (or make again) the CV track, and select it.",
    "  4. Optional: plug any MIDI device into the 5-pin MIDI Out jack, to see whether it hears notes.",
    "  5. Do not save the project or change Preferences.",
)

MIDI_OUT_SETUP = (
    "Before you press Enter, in Akai's app (stock 3.6, normal use, nothing saved):",
    "  1. Keep the keyboard on the 5-pin MIDI In jack and the synth on the 5-pin MIDI Out jack.",
    "  2. Keep the MIDI track routed the way that made the synth play, and select it.",
    "  3. Do not save the project or change Preferences.",
)

# CV ports 5 to 8 (missing MPC features thread, 2026-11-14). Ports 1 to 4
# were seen as CC 0 to 7 on cable 1, channel 16 (cv-scale's cv_out2 phase
# set the CV track to ports 3 and 4). These phases do the same for 5 to 8,
# then go back to 1 and 2 to check nothing else moved. The owner sets the
# ports before pressing Enter, so each phase only holds the note.
CV_PORTS_PHASES = (
    ("idle", "Touch NOTHING (baseline). Akai's app shows the CV track."),
    ("cv_ports_5_6", "First set the CV track's CV Port to 5 and its Gate Port to 6. Then press "
                     "Enter and play one C on the keyboard (or hit pad 1 once)."),
    ("cv_ports_7_8", "First set the CV Port to 7 and the Gate Port to 8. Then press Enter and play "
                     "one C on the keyboard (or hit pad 1 once)."),
    ("cv_ports_1_2", "First set the CV Port back to 1 and the Gate Port to 2. Then press Enter and "
                     "play one C on the keyboard (or hit pad 1 once)."),
)

CV_PORTS_SETUP = (
    "Before you press Enter, in Akai's app (3.6 or 3.10.1, normal use, nothing saved):",
    "  1. Make (or keep) a CV track, and select it. Nothing needs to be plugged into the CV jacks.",
    "  2. Find its CV Port and Gate Port fields (as in the second round, when you set them to 3 and 4).",
    "  3. Optional: the keyboard on the 5-pin MIDI In jack, so the note is a known C. Pad 1 works too.",
    "  4. Do not save the project or change Preferences.",
    "The capture runs for at most 240 s in all, so set each phase's ports promptly.",
)

# Fourth set (owner, 2026-11-13): the transport buttons' lights under Akai's
# app. Each phase fits its 8 s: count, don't rush.
TRANSPORT_PHASES = (
    ("idle", "Touch NOTHING (baseline). Akai's app is stopped."),
    ("play_stop", "Press PLAY, count to three, then press STOP."),
    ("rec_play", "Press REC once, count to two, press PLAY, count to three, then press STOP."),
    ("overdub_play", "Press OVERDUB once, count to two, press PLAY, count to three, then press STOP."),
    ("play_start", "Press PLAY START, count to two, then press STOP."),
)

TRANSPORT_SETUP = (
    "Before you press Enter, in Akai's app (stock 3.6, normal use, nothing saved):",
    "  1. Stay on the Main page with the transport stopped.",
    "  2. The REC phase records a few seconds into the open project, as in ordinary use. Do not",
    "     save the project or change Preferences; power off as after any session you don't save.",
)

PHASE_SETS = {"controller": PHASES, "midi-cv": MIDI_CV_PHASES, "cv-scale": CV_SCALE_PHASES,
              "midi-out": MIDI_OUT_PHASES, "cv-ports": CV_PORTS_PHASES,
              "transport": TRANSPORT_PHASES}
SETUPS = {"midi-cv": MIDI_CV_SETUP, "cv-scale": CV_SCALE_SETUP, "midi-out": MIDI_OUT_SETUP,
          "cv-ports": CV_PORTS_SETUP, "transport": TRANSPORT_SETUP}

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
    ap.add_argument("--phases", choices=sorted(PHASE_SETS), default="controller")
    args = ap.parse_args(argv)
    phases = PHASE_SETS[args.phases]
    if args.phases in SETUPS:
        for line in SETUPS[args.phases]:
            print(line, file=sys.stderr)
        try:
            input("Press Enter when that is set up (Ctrl+C stops, nothing has run on the MPC yet): ")
        except EOFError:
            return 1

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
    out(f"phase set: {args.phases}")
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

    for i, (name, instruction) in enumerate(phases, 1):
        with lock:
            if state["ended"]:
                out("capture window ended on the MPC; stopping early")
                break
        print(f"\n[{i}/{len(phases)}] {name}: {instruction}", file=sys.stderr)
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


def cv_ports(msgs):
    """How many CV values each port got: control changes OUT on cable 1,
    channel 16, the high half of each pair (an even CC) counted once."""
    ports = collections.Counter()
    for d, cable, m in msgs:
        if d == "OUT" and cable == 1 and len(m) == 3 and m[0] == 0xBF and m[1] < 32 and m[1] % 2 == 0:
            ports[m[1] // 2 + 1] += 1
    return ports


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
    res = ["MESSAGES PER CABLE AND PHASE (beyond the idle phase's count)"]
    base = collections.Counter((d, c) for d, c, _ in per_phase.get("idle", []))
    for phase, msgs in per_phase.items():
        if phase == "gap":
            continue
        n = collections.Counter((d, c) for d, c, _ in msgs)
        cells = [f"{d} cable{c} {n[(d, c)]} ({n[(d, c)] - base[(d, c)]:+d})"
                 for d, c in sorted(set(n) | set(base))]
        res.append(f"  {phase}: " + ", ".join(cells))
    res.append("")
    res.append("CV PORTS BY PHASE (control changes on cable 1, channel 16: port = CC // 2 + 1)")
    for phase, msgs in per_phase.items():
        ports = cv_ports(msgs)
        if ports:
            res.append(f"  {phase}: " + ", ".join(f"port {p} (CC {2 * p - 2}, {2 * p - 1}) {n}x"
                                                  for p, n in sorted(ports.items())))
    res.append("")
    res.append("DECODED USB-MIDI BY PHASE")
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
