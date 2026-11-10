#!/usr/bin/env python3
"""Record what Akai's app sends to the controller while it starts. STOCK 3.6 ONLY.

Run it through `collect-over-ssh.sh --app-start-capture` (MPFREE_ACCEPT_APP_RESTART=YES),
with stock 3.6 running and no USB stick in the MPC. Each step waits for you:

  1. Read-only checks on the MPC: it runs stock 3.6 (its kernel release), Akai's
     app (acvs.service) is running, ALSA card 0 is the controller in its normal
     MIDI mode (USB 09e8:0046, not a firmware-update mode), no USB drive is
     attached, and a usbmon reader exists for the controller's USB bus. It
     notes the controller's USB address and firmware version (bcdDevice).
  2. Passive capture: reads the kernel's usbmon copy of the transfers on the
     controller's USB bus (binary /dev/usbmonN, which keeps whole transfers;
     the debugfs text file if that is missing, which keeps 32 bytes of each).
     usbmon only copies what crosses the bus; it never delays, changes or
     sends anything. The reader stops by itself after 240 s at most.
  3. STATE-CHANGING, after you type RESTART: `systemctl restart acvs.service`.
     Akai's app quits and starts again, as at every power-on, and may save its
     usual settings to /data. systemd does the restart, so a dropped SSH
     session cannot leave it half done.
  4. You press Enter when Akai's app shows its main screen again.
  5. You hit some pads and turn a Q-Link and the wheel for 8 s.
  6. Read-only checks: Akai's app runs again (a new process), and the
     controller is the same USB device as before (same address, same
     firmware version, still in MIDI mode). You say what you saw.

The report lists every MIDI message the app sent to the controller and every
message the controller sent back, decoded and in order, and every captured USB
transfer in hex. Nothing is written on the MPC by this tool. If Akai's app does
not come back, switch the MPC off and on.
"""
import argparse
import importlib.util
import os
import re
import signal
import struct
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))


def _load(name):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, f"{name}.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


kexec = _load("host_kexec_boot")
midi = _load("host_midi_capture")

STOCK_RELEASE = "6.6.95-az01-2025-06-27-rt55"
CONTROLLER = "09e8:0046"
CONFIRM = "RESTART"
TESTING = os.environ.get("MPFREE_APP_CAPTURE_TEST") == "1"
CAPTURE_SECONDS = 10 if TESTING else 240
BEFORE_SECONDS = 0.5 if TESTING else 6.0
CONTROLS_SECONDS = 0.5 if TESTING else 8.0
AFTER_WAIT = 2 if TESTING else 60
MAX_LINES = 20000

Stop = kexec.Stop

# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
CHECK = r"""R=''
set -u
fail=0
say() { printf '%s\n' "$*"; }
bad() { printf 'PROBLEM: %s\n' "$*"; fail=1; }
rel=$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)
say "release: $rel"
[ "$rel" = '@@RELEASE@@' ] || bad "this is not stock 3.6 (kernel @@RELEASE@@)"
st=$(systemctl is-active acvs.service 2>/dev/null)
say "Akai's app (acvs.service): $st"
[ "$st" = active ] || bad "Akai's app is not running"
systemctl show acvs.service -p MainPID 2>/dev/null
say "script runner (az01-script-runner.service): $(systemctl is-active az01-script-runner.service 2>/dev/null)"
id=$(cat "$R/proc/asound/card0/usbid" 2>/dev/null)
say "card 0 USB id: $id"
[ "$id" = '@@CONTROLLER@@' ] || bad "card 0 is not the controller in its normal MIDI mode"
busdev=$(cat "$R/proc/asound/card0/usbbus" 2>/dev/null)
case "$busdev" in
  [0-9][0-9][0-9]/[0-9][0-9][0-9]) ;;
  *) bad "unexpected /proc/asound/card0/usbbus: $busdev"; busdev=000/000 ;;
esac
bus=${busdev%%/*}; bus=${bus#0}; bus=${bus#0}
dev=${busdev##*/}; dev=${dev#0}; dev=${dev#0}
say "bus: $bus device: $dev"
@@IDENTITY@@
for b in "$R"/sys/block/sd*; do
  [ -e "$b" ] && bad "a USB drive is attached (${b##*/}): take it out first"
done
dfs=$(awk '$3=="debugfs"{print $2; exit}' "$R/proc/mounts" 2>/dev/null)
if [ -c "$R/dev/usbmon$bus" ]; then
  say "capture: binary /dev/usbmon$bus"
elif [ -n "$dfs" ] && [ -r "$R$dfs/usb/usbmon/${bus}u" ]; then
  say "capture: text $dfs/usb/usbmon/${bus}u"
else
  bad "no usbmon reader for bus $bus (needs /dev/usbmon$bus, or debugfs with usb/usbmon/${bus}u)"
fi
[ "$fail" = 0 ] && say CHECKS-OK
"""

# The controller's USB identity (address, ids, firmware version), from sysfs. Shared by CHECK and AFTER.
IDENTITY = r"""for d in "$R"/sys/bus/usb/devices/*; do
  [ -r "$d/busnum" ] && [ -r "$d/devnum" ] || continue
  [ "$(cat "$d/busnum")" = "$bus" ] && [ "$(cat "$d/devnum")" = "$dev" ] || continue
  say "usb device: ${d##*/} $(cat "$d/idVendor"):$(cat "$d/idProduct") bcdDevice $(cat "$d/bcdDevice") devnum $(cat "$d/devnum")"
done"""

# Passive: copies the usbmon stream to stdout, markers to stderr. Bounded by its own sleep.
CAPTURE = r"""R=''
set -u
src='@@SOURCE@@'
printf 'MPFREE-CAPTURE-START\n' >&2
cat "$R$src" &
pid=$!
sleep @@SECONDS@@
kill "$pid" 2>/dev/null
wait "$pid" 2>/dev/null
printf 'MPFREE-CAPTURE-END\n' >&2
"""

# The only state-changing script.
RESTART = r"""R=''
systemctl restart acvs.service
echo "systemctl restart acvs.service: exit code $?"
"""

AFTER = r"""R=''
set -u
say() { printf '%s\n' "$*"; }
i=0
while [ "$i" -lt @@WAIT@@ ]; do
  [ "$(systemctl is-active acvs.service 2>/dev/null)" = active ] && break
  sleep 1
  i=$((i + 1))
done
say "Akai's app (acvs.service): $(systemctl is-active acvs.service 2>/dev/null)"
systemctl show acvs.service -p MainPID 2>/dev/null
say "script runner (az01-script-runner.service): $(systemctl is-active az01-script-runner.service 2>/dev/null)"
say "card 0 USB id: $(cat "$R/proc/asound/card0/usbid" 2>/dev/null)"
busdev=$(cat "$R/proc/asound/card0/usbbus" 2>/dev/null)
bus=@@BUS@@
dev=${busdev##*/}; dev=${dev#0}; dev=${dev#0}
say "bus: $bus device: $dev"
@@IDENTITY@@
"""

SOURCE = re.compile(r"^(/dev/usbmon\d{1,3}|/[a-z/]{1,64}/usb/usbmon/\d{1,3}u)$")
HDR = struct.Struct("<QBBBBHBBqiiII8s")  # struct mon_bin_hdr, API 0: 48 bytes on every architecture
assert HDR.size == 48
MAX_CAP = 300 * 1024 // 5  # the kernel's default buffer / 5: the largest data part of one event
XFER = {0: "iso", 1: "intr", 2: "ctrl", 3: "bulk"}
TEXT_XFER = {"Z": 0, "I": 1, "C": 2, "B": 3}


class Event:
    __slots__ = ("phase", "ts", "kind", "xfer", "ep", "dev", "status", "length", "data", "setup", "cut")

    def __init__(self, phase, ts, kind, xfer, ep, dev, status, length, data, setup=None, cut=False):
        self.phase, self.ts, self.kind, self.xfer, self.ep, self.dev = phase, ts, kind, xfer, ep, dev
        self.status, self.length, self.data, self.setup, self.cut = status, length, data, setup, cut

    @property
    def direction(self):
        return "IN" if self.ep & 0x80 else "OUT"

    def midi_data(self):
        """USB-MIDI packets carried by this event: OUT on submission, IN on completion."""
        if self.xfer != 3 or not self.data:
            return None
        if (self.direction == "OUT" and self.kind == "S") or (self.direction == "IN" and self.kind == "C"):
            return self.data
        return None

    def raw_line(self, t0):
        setup = f" setup {self.setup.hex(' ')}" if self.setup else ""
        cut = " (cut short)" if self.cut else ""
        return (f"{self.phase:9s} +{self.ts - t0:9.6f} {self.kind} {XFER.get(self.xfer, '?'):4s} ep {self.ep:02x} "
                f"dev {self.dev} status {self.status} len {self.length}{setup} = {self.data.hex(' ')}{cut}")


class BinaryStream:
    """Splits the binary usbmon stream (read(2) API: 48-byte header, then len_cap data bytes)."""

    def __init__(self):
        self.buf = bytearray()
        self.broken = None

    def feed(self, chunk, phase):
        self.buf.extend(chunk)
        events = []
        while not self.broken and len(self.buf) >= HDR.size:
            (_id, kind, xfer, ep, dev, _bus, flag_setup, flag_data, sec, usec, status, length, cap,
             setup) = HDR.unpack_from(self.buf)
            if kind not in b"SCE" or xfer > 3 or cap > MAX_CAP:
                self.broken = f"unexpected header {bytes(self.buf[:HDR.size]).hex()}"
                break
            if len(self.buf) < HDR.size + cap:
                break
            data = bytes(self.buf[HDR.size:HDR.size + cap]) if flag_data == 0 else b""
            del self.buf[:HDR.size + cap]
            events.append(Event(phase, sec + usec / 1e6, chr(kind), xfer, ep, dev, status, length, data,
                                setup if flag_setup == 0 else None, cut=len(data) < length and bool(data)))
        return events


class TextStream:
    """Splits the usbmon text interface (Documentation/usb/usbmon.rst); at most 32 data bytes per event."""

    ADDR = re.compile(r"^([CIBZ])([io]):(\d+):(\d+):(\d+)$")

    def __init__(self):
        self.buf = bytearray()
        self.broken = None

    def feed(self, chunk, phase):
        self.buf.extend(chunk)
        events = []
        while b"\n" in self.buf:
            line, _, rest = bytes(self.buf).partition(b"\n")
            self.buf = bytearray(rest)
            ev = self.parse(line.decode("ascii", "replace"), phase)
            if ev:
                events.append(ev)
        return events

    def parse(self, line, phase):
        parts = line.split()
        if len(parts) < 5:
            return None
        m = self.ADDR.match(parts[3])
        if not m or parts[2] not in ("S", "C", "E"):
            return None
        xfer, direction, dev, epn = TEXT_XFER[m.group(1)], m.group(2), int(m.group(4)), int(m.group(5))
        ep = epn | (0x80 if direction == "i" else 0)
        rest, setup, status = parts[4:], None, 0
        if rest and rest[0] == "s":
            setup = bytes.fromhex("".join(rest[1:6])) if len(rest) >= 6 else None
            rest = rest[6:]
        elif rest:
            try:
                status = int(rest[0].split(":")[0])
            except ValueError:
                status = 0
            rest = rest[1:]
        length = int(rest[0]) if rest and rest[0].isdigit() else 0
        data = bytearray()
        if "=" in rest:
            for word in rest[rest.index("=") + 1:]:
                if not re.fullmatch(r"[0-9a-fA-F]+", word) or len(word) % 2:
                    break
                data.extend(bytes.fromhex(word))
        try:
            ts = int(parts[1]) / 1e6
        except ValueError:
            ts = 0.0
        return Event(phase, ts, parts[2], xfer, ep, dev, status, length, bytes(data), setup,
                     cut=len(data) < length and bool(data))


def ask(prompt):
    if not TESTING:
        if not sys.stdin.isatty():
            raise Stop("run this from a terminal: each step waits for you")
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)  # drop anything typed before the question
    try:
        return input(prompt).strip()
    except EOFError:
        return "q"


def field(text, name):
    m = re.search(rf"^{re.escape(name)}(.*)$", text, re.M)
    return m.group(1).strip() if m else None


class Capture:
    """The passive usbmon reader on the MPC, read in two threads (stream on stdout, markers on stderr)."""

    def __init__(self, mpc, source, mode):
        self.parser = BinaryStream() if mode == "binary" else TextStream()
        self.lock = threading.Lock()
        self.phase = "before"
        self.events = []
        self.markers = []
        self.started = threading.Event()
        self.ended = threading.Event()
        cmd = mpc.stock + [mpc.target, "sh -s"]
        self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.proc.stdin.write(kexec.fill(CAPTURE, SOURCE=source, SECONDS=str(CAPTURE_SECONDS)).encode())
        self.proc.stdin.close()
        self.threads = [threading.Thread(target=self.read_stream, daemon=True),
                        threading.Thread(target=self.read_markers, daemon=True)]
        for t in self.threads:
            t.start()

    def set_phase(self, name):
        with self.lock:
            self.phase = name

    def read_stream(self):
        while True:
            chunk = self.proc.stdout.read1(65536)
            if not chunk:
                break
            with self.lock:
                self.events.extend(self.parser.feed(chunk, self.phase))
        self.ended.set()

    def read_markers(self):
        for raw in self.proc.stderr:
            line = raw.decode("utf-8", "replace").rstrip("\n")
            with self.lock:
                self.markers.append(line)
            if line == "MPFREE-CAPTURE-START":
                self.started.set()
        self.started.set()

    def stop(self):
        if self.proc.poll() is None:
            self.proc.terminate()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        for t in self.threads:
            t.join(timeout=5)


def analyse(events, out, mode, broken):
    out.say(f"\nCAPTURE: {len(events)} usbmon events ({mode})")
    if broken:
        out.say(f"  WARNING: the binary stream went out of step and was not read further: {broken}")
    if not events:
        out.say("  nothing was captured")
        return
    devs = sorted({e.dev for e in events})
    out.say(f"  USB devices seen on the bus: {', '.join(str(d) for d in devs)}")
    cut = sum(1 for e in events if e.cut)
    if cut:
        out.say(f"  {cut} transfers were cut short by the capture (text mode keeps 32 bytes of each)")
    t0 = events[0].ts

    decoders = {}
    messages = []  # (phase, ts, direction, cable, msg)
    for e in events:
        data = e.midi_data()
        if data is None:
            continue
        dec = decoders.setdefault(e.dev, midi.MidiDecoder())
        for direction, cable, msg in dec.feed(e.direction, data):
            messages.append((e.phase, e.ts, direction, cable, msg))

    out.say("\nMESSAGES BY PHASE (count, direction, cable, kind)")
    phases = []
    for m in messages:
        if m[0] not in phases:
            phases.append(m[0])
    for phase in phases:
        these = [m for m in messages if m[0] == phase]
        out.say(f"\n== {phase}: {len(these)} messages")
        counts = {}
        for _p, _t, d, c, msg in these:
            k = midi.summary_key(d, c, msg)
            counts[k] = counts.get(k, 0) + 1
        for (d, c, k), n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))[:60]:
            out.say(f"  {n:5d}x {d:3s} cable{c} {k}")

    restart = [m for m in messages if m[0] != "before"]
    out.say("\nEVERY MESSAGE FROM THE RESTART ON, IN ORDER (pad pressure left out)")
    out.say("  (OUT = from Akai's app to the controller, IN = from the controller; time from the capture's start)")
    shown = 0
    for phase, ts, d, c, msg in restart:
        if d == "IN" and msg and msg[0] & 0xF0 == 0xA0:
            continue
        shown += 1
        if shown > MAX_LINES:
            out.say(f"  (more than {MAX_LINES} messages: see the counts above)")
            break
        out.say(f"  {phase:9s} +{ts - t0:9.6f} {d:3s} cable{c} {midi.describe(msg)}")

    ctrl = [e for e in events if e.xfer == 2 and e.kind == "S" and e.setup]
    out.say(f"\nUSB CONTROL REQUESTS: {len(ctrl)}")
    for e in ctrl[:200]:
        out.say(f"  {e.phase:9s} +{e.ts - t0:9.6f} dev {e.dev} setup {e.setup.hex(' ')}")

    out.say("\nEVERY CAPTURED USB TRANSFER (phase, time, S=submitted C=completed, type, endpoint, device, data)")
    for i, e in enumerate(events):
        if i >= MAX_LINES * 3:
            out.say("  (stopped listing here)")
            break
        out.say("  " + e.raw_line(t0))


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", required=True)
    p.add_argument("--port", type=int, default=2222)
    p.add_argument("--user", default="root")
    p.add_argument("--key", required=True)
    p.add_argument("--report")
    args = p.parse_args(argv)
    out = kexec.Out(args.report)
    mpc = kexec.Mpc(args, os.devnull)  # only the stock host key (the test build is refused by it)
    out.say("MPFree: record what Akai's app sends to the controller while it starts (stock 3.6)")

    out.say("\n[1/6] Checks on the MPC (read-only)")
    rc, text = mpc.run(kexec.fill(CHECK, RELEASE=STOCK_RELEASE, CONTROLLER=CONTROLLER, IDENTITY=IDENTITY))
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc != 0 or text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the checks did not pass; nothing was started or changed")
    bus = field(text, "bus: ").split()[0]
    before_usb = field(text, "usb device: ")
    before_pid = field(text, "MainPID=")
    mode, source = field(text, "capture: ").split()
    if mode not in ("binary", "text") or not SOURCE.match(source) or not bus.isdigit() or not before_usb:
        raise Stop("could not read the capture source, bus or controller identity from the checks")

    out.say(f"\n[2/6] Passive capture of USB bus {bus} ({mode}: {source}), at most {CAPTURE_SECONDS} s")
    a = ask("  Press Enter to start the capture (q stops here): ").lower()
    if a == "q":
        raise Stop("stopped by you; nothing was started")
    cap = Capture(mpc, source, mode)
    restarted = False
    result = 1
    signal.signal(signal.SIGINT, signal.default_int_handler)
    try:
        if not cap.started.wait(25) or "MPFREE-CAPTURE-START" not in cap.markers:
            raise Stop(f"the capture did not start: {' / '.join(cap.markers) or 'no answer'}")
        out.say(f"  capture running; recording Akai's app as it is for {BEFORE_SECONDS:.0f} s")
        time.sleep(BEFORE_SECONDS)

        out.say("\n[3/6] Restart Akai's app (STATE-CHANGING)")
        out.say("  systemd stops Akai's app and starts it again, as at every power-on. It may save its usual")
        out.say("  settings to /data. If it does not come back, switch the MPC off and on.")
        while True:
            a = ask(f"  Type {CONFIRM} and press Enter to restart it now (q stops here, nothing restarted): ")
            if a == CONFIRM:
                break
            if a.lower() == "q":
                raise Stop("stopped by you; Akai's app was not restarted")
            print(f"  Please type {CONFIRM} exactly, or q.")
        cap.set_phase("restart")
        restarted = True
        rc, text = mpc.run(RESTART, timeout=150)
        out.say(f"  {text.strip() or f'(no answer, exit code {rc})'}")
        if "exit code 0" not in text:
            out.say("  WARNING: the restart did not report success; the checks in step 6 show the app's state.")
        cap.set_phase("starting")

        out.say("\n[4/6] Wait for Akai's app")
        ask("  When Akai's app shows its main screen again, press Enter: ")
        cap.set_phase("app-up")

        out.say(f"\n[5/6] Controls: {CONTROLS_SECONDS:.0f} s")
        ask("  Press Enter, then hit pads 1 to 4, turn Q-Link 1 and turn the data wheel: ")
        cap.set_phase("controls")
        out.say("  GO")
        time.sleep(CONTROLS_SECONDS)
        cap.set_phase("after")
        out.say("  STOP")
        result = 0
    except (Stop, KeyboardInterrupt) as err:
        out.say(f"\nSTOP: {str(err) or 'interrupted (Ctrl-C)'}")
    signal.signal(signal.SIGINT, signal.SIG_IGN)  # no Ctrl-C may cut the checks short
    cap.stop()

    out.say("\n[6/6] Checks after the restart (read-only)")
    if not restarted:
        out.say("  Akai's app was not restarted.")
    else:
        rc, text = mpc.run(kexec.fill(AFTER, WAIT=str(AFTER_WAIT), BUS=bus, IDENTITY=IDENTITY), timeout=AFTER_WAIT + 30)
        for line in text.splitlines():
            out.say(f"  {line}")
        after_usb, after_pid = field(text, "usb device: "), field(text, "MainPID=")
        running = field(text, "Akai's app (acvs.service): ") == "active"
        out.say(f"  Akai's app: {'running again' if running else 'NOT RUNNING'}"
                f" (process {before_pid} before, {after_pid} now)")
        if after_usb == before_usb:
            out.say(f"  controller: the same USB device as before ({after_usb})")
        else:
            out.say(f"  WARNING: the controller changed: before '{before_usb}', now '{after_usb}'")
        if not running:
            out.say("  Akai's app is not running: switch the MPC off with its power switch and on again.")
            result = 1
        try:
            answer = ask("  Is Akai's app back as before, and did the pads light when you hit them? "
                         "Type a short answer and press Enter: ")
        except Stop:
            answer = ""
        out.say(f"  your answer: {answer or '(none)'}")

    with cap.lock:
        events, markers, broken = list(cap.events), list(cap.markers), cap.parser.broken
    extra = [m for m in markers if not m.startswith("MPFREE-CAPTURE-")]
    if extra:
        out.say("\nthe capture's own messages: " + " / ".join(extra[:20]))
    analyse(events, out, mode, broken)
    out.say("\nRESULT: " + ("all steps ran." if result == 0 else "stopped early or a check failed."))
    return result


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:
        print("\nSTOP: interrupted (Ctrl-C). Akai's app was not restarted.")
        sys.exit(130)
