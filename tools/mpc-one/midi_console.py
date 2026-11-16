#!/usr/bin/env python3
"""The MIDI monitor, live on the workstation. READ-ONLY on the MPC.

Run it through `collect-over-ssh.sh --midi-console` while MPFree runs on the
test build with Prefs > Show developer tools on. MPFree then writes every line
of its MIDI monitor (Mix > I/O > Monitor, and Dev > MIDI) to
/run/mpfree-app/midi.log, in RAM (rust/mpfree-app/src/midi_log.rs). This tool
follows that file over SSH and prints each line in words, with its colour:
when, the port and way, the channel, the message, where it went, and (with
--hex) its bytes.

On the MPC it only reads: one POSIX sh loop (REMOTE below, sent on stdin)
opens the file for reading, prints what is new each second, and opens the new
file when MPFree sets the old one aside (its last line is @@ROTATED). It
writes nothing, starts nothing and stops nothing. It talks only to the test
build: like `--app`, it takes the newest ~/mpfree-work/kexec-stick-* (or
creator-stick-*) folder's known_hosts, so any other system is refused before
anything runs. Ctrl-C stops it.

Filters (any number): --notes, --cc, --program, --bend, --clock, --sysex,
--cv, and in debug mode --mpc (the MPC's own pads and buttons), --lights,
--keepalive and --system (MPFree's own events); none of them: everything.
--midi-port NAME (MIDI for the 5-pin jacks, or a USB device's name),
--ch N (1 to 16), --in or --out. --from-start prints the file from its first
line instead of only what comes next. --file PATH prints a copied midi.log
once, with the same filters, and touches no network.
"""
import argparse
import importlib.util
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("host_kexec_boot", os.path.join(HERE, "host_kexec_boot.py"))
kexec = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(kexec)

LOG = "/run/mpfree-app/midi.log"
ROTATED = "@@ROTATED"
WAITING = "@@WAITING"
FOLLOWING = "@@FOLLOWING"
IDLE = "@@IDLE"
# The kinds, as mpfree_midi_ui::log::kind_name names them.
KINDS = ("notes", "cc", "program", "bend", "clock", "sysex", "mpc", "lights", "keepalive", "cv", "system")
FIELDS = 9

# READ-ONLY. Every remote script starts with R='' (an empty root prefix); only the tests' fake ssh changes it.
# Only shell built-ins and sleep. A file that is there when it starts is read to its end first (SKIP=1) unless
# --from-start; one that appears later is read from its first line. A line half written is kept until its end
# comes. The file is opened by a redirection on a function call, so a file that goes between the test and the
# open is only a failed call, and the loop goes on. Every 10 quiet seconds it prints @@IDLE: once the
# workstation has gone, that write fails and the loop ends (SSH without a terminal sends it no hang-up).
REMOTE = r"""R=''
F="$R/run/mpfree-app/midi.log"
SKIP=1
part=''
idle=0
quiet() {
  idle=$((idle + 1))
  if [ "$idle" -ge 10 ]; then
    echo '@@IDLE' || exit 0
    idle=0
  fi
  sleep 1
}
follow() {
  if [ "$SKIP" = 1 ]; then
    while IFS= read -r line <&3; do :; done
    part="$line"
  fi
  SKIP=0
  echo '@@FOLLOWING' || exit 0
  while :; do
    if IFS= read -r line <&3; then
      line="$part$line"
      part=''
      idle=0
      printf '%s\n' "$line" || exit 0
      if [ "$line" = '@@ROTATED' ]; then
        return 0
      fi
    else
      part="$part$line"
      quiet
    fi
  done
}
said=0
while :; do
  if [ -f "$F" ]; then
    said=0
    follow 3< "$F"
    part=''
  else
    SKIP=0
    if [ "$said" = 0 ]; then
      echo '@@WAITING' || exit 0
      said=1
    fi
    quiet
  fi
done
"""

COLOURS = {"notes": "32", "cc": "36", "program": "35", "bend": "33", "clock": "34", "sysex": "37",
           "cv": "36", "mpc": "2", "lights": "2", "keepalive": "2", "system": "1"}
TAKEN_BY_NOTHING = ("nothing", "not played", "not followed", "ignored", "no pad")


class Line:
    """One line of the log: its nine tab-separated fields (mpfree_midi_ui::log::console_line)."""

    def __init__(self, fields):
        n, t, port, way, ch, kind, words, went, data = fields
        self.n = int(n)
        self.t = float(t)
        self.port, self.way, self.kind, self.words = port, way, kind, words
        self.ch = None if ch == "-" else int(ch)
        self.went = None if went == "-" else went
        self.bytes = None if data == "-" else data


def parse(text):
    """A Line, or None for a line that is not one (a marker, or cut short)."""
    fields = text.rstrip("\n").split("\t")
    if len(fields) != FIELDS:
        return None
    try:
        return Line(fields)
    except ValueError:
        return None


def shown(line, opts):
    kinds = [k for k in KINDS if getattr(opts, k, False)]
    if kinds and line.kind not in kinds:
        return False
    if opts.midi_port and line.port.lower() != opts.midi_port.lower():
        return False
    if opts.ch and line.ch != opts.ch:
        return False
    if opts.way and line.way != opts.way:
        return False
    return True


def paint(text, code, colour):
    return f"\x1b[{code}m{text}\x1b[0m" if colour and code else text


def went_colour(went):
    if went.startswith("Learn"):
        return "35"
    if went.startswith(TAKEN_BY_NOTHING):
        return "33"
    if went.startswith("clock"):
        return "32"
    return "2"


def words(line, opts, colour):
    """The line as the console prints it."""
    clock = f"{int(line.t // 60)}:{line.t % 60:06.3f}"
    port = f"{line.port} {line.way}"
    ch = f"ch {line.ch:<2}" if line.ch else " " * 5
    out = f"{clock:>10}  {port:<18} {ch}  " + paint(line.words, COLOURS.get(line.kind), colour)
    if line.went:
        out += "  -> " + paint(line.went, went_colour(line.went), colour)
    if opts.hex and line.bytes:
        out += "  " + paint(f"[{line.bytes}]", "2", colour)
    return out


class Printer:
    """Prints the lines the filters keep, and says when numbers were skipped (lines MPFree didn't keep)."""

    def __init__(self, opts, colour, report=None):
        self.opts, self.colour, self.report = opts, colour, report
        self.last = None

    def say(self, text, code="2"):
        print(paint(text, code, self.colour), flush=True)
        if self.report:
            self.report.write(text + "\n")
            self.report.flush()

    def take(self, text):
        if text in (ROTATED, IDLE):
            return
        if text == WAITING:
            self.say(f"Waiting for {LOG}: MPFree writes it while Prefs > Show developer tools is on.")
            return
        if text == FOLLOWING:
            self.say(f"Following {LOG} (read only; Ctrl-C stops).")
            return
        line = parse(text)
        if line is None:
            return
        if self.last is not None and line.n > self.last + 1:
            self.say(f"  ... {line.n - self.last - 1} lines not kept")
        self.last = line.n
        if shown(line, self.opts):
            print(words(line, self.opts, self.colour), flush=True)
            if self.report:
                self.report.write(words(line, self.opts, False) + "\n")
                self.report.flush()


def remote_script(from_start):
    return REMOTE.replace("SKIP=1\n", "SKIP=0\n", 1) if from_start else REMOTE


def ssh_command(args):
    """The test build's SSH, as --app reaches it: its own known_hosts, nothing else accepted."""
    folder = kexec.newest_build(os.path.realpath(args.work))
    mpc = kexec.Mpc(args, os.path.join(folder, "known_hosts"))
    return mpc.test + [mpc.target, "sh -s"]


def follow(args, printer):
    cmd = ssh_command(args)
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    proc.stdin.write(remote_script(args.from_start).encode())
    proc.stdin.close()
    try:
        for raw in proc.stdout:
            printer.take(raw.decode("utf-8", "replace").rstrip("\n"))
    except KeyboardInterrupt:
        proc.terminate()
        proc.wait(timeout=10)
        printer.say("Stopped.")
        return 0
    rc = proc.wait()
    err = proc.stderr.read().decode("utf-8", "replace").strip()
    printer.say(f"The SSH session ended (exit {rc}){': ' + err[:300] if err else ''}", "33")
    return 1


def options(argv):
    p = argparse.ArgumentParser(description="The MIDI monitor's lines, live from MPFree on the test build "
                                            "(read-only on the MPC).")
    p.add_argument("--ip", default="10.42.0.141")
    p.add_argument("--port", type=int, default=kexec.TEST_PORT, help="the test build's SSH port")
    p.add_argument("--user", default="root")
    p.add_argument("--key")
    p.add_argument("--work", default=os.path.expanduser("~/mpfree-work"),
                   help="where the stick builds are (their known_hosts)")
    p.add_argument("--file", help="print a copied midi.log once; no network")
    p.add_argument("--report", help="also write the lines shown here, without colour")
    for k in KINDS:
        p.add_argument(f"--{k}", action="store_true", help=f"show {k} lines")
    p.add_argument("--midi-port", help="only this port: MIDI (the 5-pin jacks), or a USB device's name")
    p.add_argument("--ch", type=int, choices=range(1, 17), metavar="1-16", help="only this channel")
    way = p.add_mutually_exclusive_group()
    way.add_argument("--in", dest="way", action="store_const", const="in", help="only what came in")
    way.add_argument("--out", dest="way", action="store_const", const="out", help="only what went out")
    p.add_argument("--hex", action="store_true", help="add each message's bytes")
    p.add_argument("--from-start", action="store_true", help="print the file from its first line")
    p.add_argument("--no-colour", action="store_true")
    args = p.parse_args(argv)
    if not args.file and not args.key:
        p.error("--key is needed to reach the MPC (or --file to read a copy)")
    return args


def main(argv):
    args = options(argv)
    colour = not args.no_colour and sys.stdout.isatty()
    report = open(args.report, "a") if args.report else None
    try:
        printer = Printer(args, colour, report)
        if args.file:
            with open(args.file, encoding="utf-8", errors="replace") as fh:
                for text in fh:
                    printer.take(text.rstrip("\n"))
            return 0
        try:
            return follow(args, printer)
        except kexec.Stop as e:
            printer.say(f"Not started: {e}", "31")
            return 1
    finally:
        if report:
            report.close()


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except KeyboardInterrupt:
        sys.exit(0)
