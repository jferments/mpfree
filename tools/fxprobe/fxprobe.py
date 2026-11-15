#!/usr/bin/env python3
"""fxprobe.py: drive FXProbe Source and Tap on an MPC (over SSH) or in a
local folder (a desktop VST host, or the chain test).

The plugins read FXProbe/control.txt on the drive they are installed on and
write FXProbe/status/ and FXProbe/captures/ (see common/fxprobe.h). This
script writes the control file, waits on the status files and copies
captures back. Everything it writes on the MPC is inside the FXProbe folder
of the drive you name; `clean` deletes that folder again.

  fxprobe.py [--ssh HOST] [--port N] [--identity KEY] [--local DIR] [--drive PATH] COMMAND ...

  drives                     list mounted drives (read-only)
  setup [--programme WAV]... create FXProbe/ on the drive, upload programmes  (state-changing; undo: clean)
  status                     every instance's status (read-only)
  capture NAME [--programme short.wav] [--gains 0,-12] [--slots 1,2] [--tail S] [--timing]
                             one capture per gain of every slot as it is set now
  pull [--out DIR]           copy captures here
  clean                      delete FXProbe/ from the drive  (state-changing; undo: setup again)

--local DIR uses DIR as the FXProbe folder itself (no SSH). With --ssh the
FXProbe folder is DRIVE/FXProbe, DRIVE being the drive the plugins are
installed on (its "VST2 Plugins" folder sits beside FXProbe). The MPC's
shell is BusyBox: only mkdir, cat, ls, mv and rm are used there.

Status: written for MPC OS 3.10.1 with developer-mode SSH; tested against the
desktop chain test (test/run_chain_test.py), not yet on a device.
"""

import argparse
import os
import shlex
import struct
import subprocess
import sys
import time


class Local:
    """The FXProbe folder on this computer."""

    def __init__(self, folder):
        self.folder = os.path.abspath(folder)

    def path(self, rel=""):
        return os.path.join(self.folder, rel) if rel else self.folder

    def mkdirs(self, rel):
        os.makedirs(self.path(rel), exist_ok=True)

    def write_text(self, rel, text):
        tmp = self.path(rel) + ".tmp"
        with open(tmp, "w") as f:
            f.write(text)
        os.replace(tmp, self.path(rel))

    def read_text(self, rel):
        try:
            with open(self.path(rel)) as f:
                return f.read()
        except OSError:
            return None

    def listdir(self, rel):
        try:
            return sorted(os.listdir(self.path(rel)))
        except OSError:
            return []

    def put(self, local, rel):
        import shutil
        shutil.copyfile(local, self.path(rel))

    def get(self, rel, local):
        import shutil
        shutil.copyfile(self.path(rel), local)

    def remove_tree(self):
        import shutil
        shutil.rmtree(self.folder, ignore_errors=True)


class Ssh:
    """The FXProbe folder on an MPC, over SSH (developer mode, root)."""

    def __init__(self, host, port, identity, drive):
        self.host = host
        self.base = ["-p", str(port), "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]
        if identity:
            self.base += ["-i", identity]
        self.folder = drive.rstrip("/") + "/FXProbe" if drive else ""

    def path(self, rel=""):
        return self.folder + ("/" + rel if rel else "")

    def sh(self, command, data=None, check=True):
        r = subprocess.run(["ssh"] + self.base + ["root@" + self.host, command], input=data, capture_output=True)
        if check and r.returncode != 0:
            sys.exit("fxprobe: on the MPC, %r failed: %s" % (command, r.stderr.decode(errors="replace").strip()))
        return r

    def mkdirs(self, rel):
        self.sh("mkdir -p %s" % shlex.quote(self.path(rel)))

    def write_text(self, rel, text):
        p = shlex.quote(self.path(rel))
        self.sh("cat > %s.tmp && mv %s.tmp %s" % (p, p, p), data=text.encode())

    def read_text(self, rel):
        r = self.sh("cat %s" % shlex.quote(self.path(rel)), check=False)
        return r.stdout.decode(errors="replace") if r.returncode == 0 else None

    def listdir(self, rel):
        r = self.sh("ls %s" % shlex.quote(self.path(rel)), check=False)
        return sorted(r.stdout.decode(errors="replace").split()) if r.returncode == 0 else []

    def put(self, local, rel):
        with open(local, "rb") as f:
            p = shlex.quote(self.path(rel))
            self.sh("cat > %s.tmp && mv %s.tmp %s" % (p, p, p), data=f.read())

    def get(self, rel, local):
        r = self.sh("cat %s" % shlex.quote(self.path(rel)))
        with open(local, "wb") as f:
            f.write(r.stdout)

    def remove_tree(self):
        if not self.folder.endswith("/FXProbe"):
            sys.exit("fxprobe: refusing to delete %r" % self.folder)
        self.sh("rm -rf %s" % shlex.quote(self.folder))


def wav_seconds(path):
    """Length of a WAV file in seconds (any format with a fmt and data chunk)."""
    with open(path, "rb") as f:
        d = f.read(1 << 16)
        size = os.path.getsize(path)
    pos, rate, align, data_len = 12, None, None, None
    while pos + 8 <= len(d):
        cid, n = d[pos:pos + 4], struct.unpack("<I", d[pos + 4:pos + 8])[0]
        if cid == b"fmt ":
            rate, align = struct.unpack("<I", d[pos + 12:pos + 16])[0], struct.unpack("<H", d[pos + 20:pos + 22])[0]
        elif cid == b"data":
            data_len = min(n, size - pos - 8)
            break
        pos += 8 + n + (n & 1)
    if not rate or not align or data_len is None:
        sys.exit("fxprobe: %s is not a WAV I can read" % path)
    return data_len / align / rate


def parse_status(text):
    return dict(line.split(" ", 1) for line in (text or "").splitlines() if " " in line)


def statuses(dev):
    out = {}
    for name in dev.listdir("status"):
        if name.endswith(".txt"):
            out[name[:-4]] = parse_status(dev.read_text("status/" + name))
    return out


def next_seq(dev):
    cur = parse_status(dev.read_text("control.txt")).get("seq", "0")
    try:
        return int(cur) + 1
    except ValueError:
        return int(time.time())


def cmd_setup(dev, args):
    for d in ("", "programmes", "captures", "status"):
        dev.mkdirs(d)
    for p in args.programme or []:
        dev.put(p, "programmes/" + os.path.basename(p))
        print("uploaded programmes/%s (%.1f s)" % (os.path.basename(p), wav_seconds(p)))
    print("FXProbe folder ready at %s" % dev.path())
    print("Instances report in status/ within a second once they see it (load or reload the plugins if not).")


def cmd_status(dev, _args):
    st = statuses(dev)
    if not st:
        print("no instance has reported (is the folder on the drive the plugins are installed on?)")
    for name, s in sorted(st.items()):
        extra = "  error: " + s["error"] if "error" in s else ""
        print("%-14s state %-10s seq %-6s %s%s" % (name, s.get("state", "?"), s.get("seq", "-"),
                                                   s.get("capture", s.get("programme", "")), extra))


def cmd_capture(dev, args):
    progs = dev.listdir("programmes")
    if args.programme not in progs:
        sys.exit("fxprobe: programmes/%s is not on the drive (have: %s); run setup --programme" % (args.programme, ", ".join(progs) or "none"))
    local_prog = args.programme_file or args.programme
    if not os.path.isfile(local_prog):
        sys.exit("fxprobe: need the programme here too, to know its length: --programme-file PATH")
    secs = wav_seconds(local_prog)
    preroll = 0.5
    seconds = preroll + secs + args.tail + 0.5
    gains = [float(g) for g in args.gains.split(",")]
    st = statuses(dev)
    taps = sorted(k for k in st if k.startswith("tap-"))
    sources = sorted(k for k in st if k.startswith("source-"))
    if args.slots:
        want = {"s" + s.strip() for s in args.slots.split(",")}
        taps = [t for t in taps if t.split("-", 1)[1] in want]
        sources = [s for s in sources if s.split("-", 1)[1] in want]
    if not taps or not sources:
        sys.exit("fxprobe: no Source/Tap instances reporting (status: %s)" % (", ".join(st) or "none"))
    print("slots: %s; %d capture(s) of %.1f s each" % (", ".join(t[4:] for t in taps), len(gains), seconds))
    for gain in gains:
        seq = next_seq(dev)
        tag = ("m%g" % -gain) if gain < 0 else ("%g" % gain)   # names allow only letters, digits, - _ .
        name = "%s-g%s" % (args.name, tag) if len(gains) > 1 else args.name
        lines = ["seq %d" % seq, "action run", "programme " + args.programme, "gain %g" % gain,
                 "preroll %g" % preroll, "capture " + name, "seconds %.3f" % seconds]
        if args.timing:
            lines.append("timing 1")
        if args.slots:
            lines.append("slots " + args.slots)
        dev.write_text("control.txt", "\n".join(lines) + "\n")
        deadline = time.time() + 3 * seconds + 15
        while True:
            time.sleep(0.5)
            st = statuses(dev)
            done = [t for t in taps if st.get(t, {}).get("capture") == name and st[t].get("state") in ("done", "stopped")]
            errs = ["%s: %s" % (k, st[k]["error"]) for k in taps + sources if "error" in st.get(k, {})]
            if errs:
                sys.exit("fxprobe: " + "; ".join(errs))
            if len(done) == len(taps):
                break
            if time.time() > deadline:
                sys.exit("fxprobe: timed out; status: %r" % {k: st.get(k) for k in taps + sources})
        bad = []
        for t in taps:
            rep = parse_status(dev.read_text("captures/%s-%s.txt" % (name, t[4:])))
            if rep.get("overrun_frames") != "0" or rep.get("frames_written") != rep.get("frames_requested") or rep.get("nonfinite_samples") != "0":
                bad.append("%s: %r" % (t, rep))
        print("captured %s from %d slot(s)%s" % (name, len(taps), "" if not bad else "; PROBLEMS: " + "; ".join(bad)))


def cmd_pull(dev, args):
    os.makedirs(args.out, exist_ok=True)
    n = 0
    for name in dev.listdir("captures"):
        target = os.path.join(args.out, name)
        if os.path.exists(target) and not args.overwrite:
            continue
        dev.get("captures/" + name, target)
        n += 1
    print("copied %d file(s) to %s" % (n, args.out))


def cmd_drives(dev, _args):
    if not isinstance(dev, Ssh):
        sys.exit("fxprobe: drives needs --ssh")
    r = dev.sh("cat /proc/mounts")
    for line in r.stdout.decode(errors="replace").splitlines():
        f = line.split()
        if len(f) > 2 and (f[1].startswith("/media") or f[1].startswith("/run/media") or f[1].startswith("/mnt")):
            plugins = dev.sh("ls %s" % shlex.quote(f[1] + "/VST2 Plugins"), check=False).returncode == 0
            print("%-40s %-6s %s" % (f[1], f[2], "has VST2 Plugins" if plugins else ""))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ssh", metavar="HOST")
    ap.add_argument("--port", type=int, default=22)
    ap.add_argument("--identity", metavar="KEY")
    ap.add_argument("--local", metavar="DIR")
    ap.add_argument("--drive", metavar="PATH")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("drives")
    p = sub.add_parser("setup")
    p.add_argument("--programme", action="append")
    sub.add_parser("status")
    p = sub.add_parser("capture")
    p.add_argument("name")
    p.add_argument("--programme", default="short.wav")
    p.add_argument("--programme-file", help="the same programme on this computer (default: --programme)")
    p.add_argument("--gains", default="0")
    p.add_argument("--slots")
    p.add_argument("--tail", type=float, default=2.0)
    p.add_argument("--timing", action="store_true")
    p = sub.add_parser("pull")
    p.add_argument("--out", default="captures")
    p.add_argument("--overwrite", action="store_true")
    sub.add_parser("clean")
    args = ap.parse_args()
    if bool(args.ssh) == bool(args.local):
        ap.error("give exactly one of --ssh HOST or --local DIR")
    if args.ssh and not args.drive and args.cmd != "drives":
        ap.error("--drive is needed with --ssh (list them with: fxprobe.py --ssh HOST drives)")
    dev = Ssh(args.ssh, args.port, args.identity, args.drive) if args.ssh else Local(args.local)
    if args.cmd == "clean":
        dev.remove_tree()
        print("deleted %s" % dev.path())
        return
    {"drives": cmd_drives, "setup": cmd_setup, "status": cmd_status, "capture": cmd_capture, "pull": cmd_pull}[args.cmd](dev, args)


if __name__ == "__main__":
    main()
