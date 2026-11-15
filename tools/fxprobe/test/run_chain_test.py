#!/usr/bin/env python3
"""End-to-end desktop test of the FXProbe plugins, no MPC needed.

Builds FXProbe Source, FXProbe Tap and chain_sim natively, renders the fxlab
programme, then drives the chain through the same control file the device
controller uses:

1. Source -> Tap (a wire): the capture must contain the programme bit for bit,
   and fxlab must find the programme in it at the frame where it really is.
2. Source -> the SDK's StereoDelay example -> Tap, with the gain and timing
   options: the capture must be complete, louder than nothing after the dry
   signal stops (the delay tail), with a timing log and a host-call log.
3. fxprobe.py setup, capture at two gains and pull, read back by fxlab
   analyse as a flat wire.
4. The kit on a stick: stage.py makes it, fxprobe.sh installs the plugins to a
   fake internal drive, the plugins load from there and find the work folder
   on the stick's data partition through their pointer, also after the stick
   comes back under another mount name, without rerunning the old command.

Usage: MPC_SDK=/path/to/mpc-plugin-builder-sdk python3 tools/fxprobe/test/run_chain_test.py
Needs a C++17 compiler (c++) and a release build of fxlab
(cargo build --release -p mpfree-fxlab in rust/).
"""

import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
FXPROBE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(FXPROBE))
FXLAB = os.path.join(REPO, "rust", "target", "release", "fxlab")


def fail(msg):
    print("FAIL " + msg)
    sys.exit(1)


def read_wav(path):
    """(rate, channels, frames as a list of tuples) for float32 or PCM16 WAVs."""
    with open(path, "rb") as f:
        d = f.read()
    if d[:4] != b"RIFF" or d[8:12] != b"WAVE":
        fail(path + " is not a WAV")
    pos, fmt, data = 12, None, None
    while pos + 8 <= len(d):
        cid, n = d[pos:pos + 4], struct.unpack("<I", d[pos + 4:pos + 8])[0]
        body = d[pos + 8:pos + 8 + n]
        if cid == b"fmt ":
            fmt = struct.unpack("<HHIIHH", body[:16])
            if fmt[0] == 0xFFFE:
                fmt = (struct.unpack("<H", body[24:26])[0],) + fmt[1:]
        elif cid == b"data":
            data = body
        pos += 8 + n + (n & 1)
    tag, ch, rate, _, _, bits = fmt
    if tag == 3 and bits == 32:
        vals = struct.unpack("<%df" % (len(data) // 4), data)
    elif tag == 1 and bits == 16:
        vals = [v / 32768.0 for v in struct.unpack("<%dh" % (len(data) // 2), data)]
    else:
        fail("%s: unsupported format %d/%d" % (path, tag, bits))
    return rate, ch, [vals[i:i + ch] for i in range(0, len(vals), ch)]


def build(sdk, out):
    cxx = os.environ.get("CXX", "c++")
    flags = ["-std=c++17", "-O2", "-Wall", "-Wextra", "-Wno-format-truncation", "-I" + os.path.join(sdk, "framework")]
    jobs = [
        ("fxprobe-source.so", [os.path.join(FXPROBE, "FXProbeSource", "src", "FXProbeSource.cpp")], True),
        ("fxprobe-tap.so", [os.path.join(FXPROBE, "FXProbeTap", "src", "FXProbeTap.cpp")], True),
        ("stereo-delay.so", [os.path.join(sdk, "examples", "StereoDelay", "src", "StereoDelay.cpp")], True),
        ("chain_sim", [os.path.join(HERE, "chain_sim.cpp")], False),
    ]
    for name, srcs, shared in jobs:
        extra = ["-fPIC", "-shared", "-fvisibility=hidden"] if shared else []
        cmd = [cxx] + flags + extra + ["-o", os.path.join(out, name)] + srcs + ["-lpthread", "-ldl"]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            fail("build of %s:\n%s" % (name, r.stderr))
        if "warning:" in r.stderr and name != "stereo-delay.so":
            fail("warnings building %s:\n%s" % (name, r.stderr))


def read_status(work, name):
    try:
        with open(os.path.join(work, "status", name)) as f:
            return dict(line.split(" ", 1) for line in f.read().splitlines() if " " in line)
    except OSError:
        return {}


def wait_for(cond, secs, what):
    t = time.time() + secs
    while time.time() < t:
        if cond():
            return
        time.sleep(0.05)
    fail("timed out waiting for " + what)


def write_control(work, lines):
    tmp = os.path.join(work, "control.txt.tmp")
    with open(tmp, "w") as f:
        f.write("\n".join(lines) + "\n")
    os.replace(tmp, os.path.join(work, "control.txt"))


def run_chain(bins, work, middle, seconds, control, extra=()):
    env = dict(os.environ, FXPROBE_DIR=work)
    chain = [os.path.join(bins, "fxprobe-source.so")] + middle + [os.path.join(bins, "fxprobe-tap.so")]
    for f in ("source-s1.txt", "tap-s1.txt"):
        try:
            os.remove(os.path.join(work, "status", f))
        except OSError:
            pass
    p = subprocess.Popen([os.path.join(bins, "chain_sim"), "--seconds", str(seconds), "--speed", "12", "--playing"]
                         + list(extra) + chain, env=env)
    wait_for(lambda: read_status(work, "source-s1.txt").get("state") == "idle"
             and read_status(work, "tap-s1.txt").get("state") == "idle", 10, "both plugins to report idle")
    write_control(work, control)
    if p.wait(timeout=seconds) != 0:
        fail("chain_sim exited with %d" % p.returncode)


def fxprobe_sh(root, kit, *args):
    r = subprocess.run(["sh", os.path.join(kit, "fxprobe.sh")] + list(args), env=dict(os.environ, FXPROBE_TEST_ROOT=root),
                       capture_output=True, text=True, timeout=300)
    if r.returncode != 0:
        fail("fxprobe.sh %s: %s %s" % (" ".join(args), r.stdout, r.stderr))
    return r.stdout


def stick_test(bins, tmp, rate):
    built = os.path.join(tmp, "built")
    for project, folder, slug in (("FXProbeSource", "MPFree - VST - FXProbe Source", "fxprobe-source"),
                                  ("FXProbeTap", "MPFree - VST - FXProbe Tap", "fxprobe-tap")):
        os.makedirs(os.path.join(built, folder, "VST"))
        shutil.copy(os.path.join(FXPROBE, project, "plugin.json"), os.path.join(built, folder))
        shutil.copy(os.path.join(bins, slug + ".so"), os.path.join(built, folder, "VST", slug + ".so"))
    root = os.path.join(tmp, "root")
    media = os.path.join(root, "media")
    for d in ("az01-internal", "MPFREE-KITS", "4c1e9a52-data"):
        os.makedirs(os.path.join(media, d))
    os.makedirs(os.path.join(root, "proc"))
    with open(os.path.join(media, "4c1e9a52-data", "FORMAT"), "w") as f:
        f.write("mpfree-data 1\n")   # MPFree has used the data partition (setup needs it)

    def mounts(data_name):
        with open(os.path.join(root, "proc", "mounts"), "w") as f:
            f.write("/dev/mmcblk0p8 /media/az01-internal ext4 rw,relatime 0 0\n"
                    "/dev/sdz3 /media/MPFREE-KITS ext4 rw,nosuid,nodev,noexec,relatime,nosymfollow 0 0\n"
                    "/dev/sdz4 /media/%s ext4 rw,nosuid,nodev,noexec,relatime,nosymfollow 0 0\n" % data_name)
    mounts("4c1e9a52-data")
    kit = os.path.join(media, "MPFREE-KITS", "fxprobe")
    r = subprocess.run([sys.executable, os.path.join(FXPROBE, "stage.py"), "--plugins-from", built, "--native-test",
                        "--fxlab", FXLAB, "--out", kit], capture_output=True, text=True)
    if r.returncode != 0 or "STAGE-OK" not in r.stdout:
        fail("stage.py: " + r.stdout + r.stderr)
    fxprobe_sh(root, kit, "install")
    installed = os.path.join(media, "az01-internal", "VST2 Plugins")
    env = {k: v for k, v in os.environ.items() if k != "FXPROBE_DIR"}
    p = subprocess.Popen([os.path.join(bins, "chain_sim"), "--seconds", "600", "--speed", "4",
                          os.path.join(installed, "MPFree - VST - FXProbe Source", "VST", "fxprobe-source.so"),
                          os.path.join(installed, "MPFree - VST - FXProbe Tap", "VST", "fxprobe-tap.so")], env=env)
    try:
        time.sleep(1.5)
        if any(os.path.exists(os.path.join(media, d, "FXProbe")) for d in os.listdir(media)):
            fail("a work folder appeared before setup")
        fxprobe_sh(root, kit, "setup")
        work = os.path.join(media, "4c1e9a52-data", "FXProbe")
        wait_for(lambda: read_status(work, "source-s1.txt").get("state") == "idle"
                 and read_status(work, "tap-s1.txt").get("state") == "idle", 10,
                 "the installed plugins to find the work folder through their pointer")
        out = fxprobe_sh(root, kit, "capture", "wire-stick", "--gains", "0,-12", "--tail", "1")
        if "CAPTURE-OK" not in out:
            fail("fxprobe.sh capture: " + out)
        print("ok   stage.py kit, fxprobe.sh install/setup/capture: installed plugins find the stick's work folder")
        # The stick out and back in under another mount name.
        mounts("MPFREE-DATA")
        os.rename(os.path.join(media, "4c1e9a52-data"), os.path.join(media, "MPFREE-DATA"))
        work = os.path.join(media, "MPFREE-DATA", "FXProbe")
        before = sorted(os.listdir(os.path.join(work, "captures")))
        for f in os.listdir(os.path.join(work, "status")):
            os.remove(os.path.join(work, "status", f))
        wait_for(lambda: read_status(work, "source-s1.txt") and read_status(work, "tap-s1.txt"), 10,
                 "the plugins to find the work folder under its new mount name")
        time.sleep(1.5)
        if sorted(os.listdir(os.path.join(work, "captures"))) != before:
            fail("the command already in control.txt ran again when the stick came back")
        out = fxprobe_sh(root, kit, "capture", "wire-again", "--tail", "1")
        if "CAPTURE-OK" not in out:
            fail("fxprobe.sh capture after the stick came back: " + out)
        print("ok   stick out and back under another name: found again, the old command not rerun, captures again")
    finally:
        p.terminate()
        p.wait(timeout=10)
    sched = os.path.join(kit, "programmes", "short.tsv")
    for name, gain in (("wire-stick-g0", "0"), ("wire-stick-gm12", "-12"), ("wire-again", "0")):
        wavf = os.path.join(work, "captures", name + "-s1.wav")
        an = os.path.join(tmp, "an-" + name)
        r = subprocess.run([FXLAB, "analyse", wavf, "--schedule", sched, "--gain", gain, "--out", an],
                           capture_output=True, text=True)
        card = open(os.path.join(an, "captures.json")).read() if r.returncode == 0 else ""
        m = re.search(r'"resp_1k_db": ([-0-9.e]+)', card)
        if not m or abs(float(m.group(1))) > 0.01:
            fail("fxlab analyse of %s: %s %s" % (wavf, r.stderr, card[:400]))
    out = fxprobe_sh(root, kit, "remove")
    if os.path.exists(installed) or "REMOVE-OK" not in out:
        fail("fxprobe.sh remove left %s: %s" % (installed, out))
    print("ok   fxlab analyse reads the stick's captures as a flat wire; remove leaves the internal drive as it was")


def main():
    sdk = os.environ.get("MPC_SDK")
    if not sdk or not os.path.isfile(os.path.join(sdk, "framework", "mpc_plugin.h")):
        fail("set MPC_SDK to a checkout of https://github.com/inmusic-software/mpc-plugin-builder-sdk")
    if not os.path.isfile(FXLAB):
        fail("build fxlab first: cargo build --release -p mpfree-fxlab (in rust/)")
    tmp = tempfile.mkdtemp(prefix="fxprobe-test-")
    try:
        bins = os.path.join(tmp, "bin")
        work = os.path.join(tmp, "FXProbe")
        os.makedirs(bins)
        os.makedirs(os.path.join(work, "programmes"))
        build(sdk, bins)
        print("ok   built Source, Tap, StereoDelay and chain_sim")

        r = subprocess.run([FXLAB, "programme", "--out", tmp], capture_output=True, text=True)
        if r.returncode != 0:
            fail("fxlab programme: " + r.stderr)
        shutil.copy(os.path.join(tmp, "programme.wav"), os.path.join(work, "programmes", "standard.wav"))
        rate, _, prog = read_wav(os.path.join(tmp, "programme.wav"))
        secs = len(prog) / rate

        # 1. A wire: bit for bit.
        run_chain(bins, work, [], secs + 3, [
            "seq 1", "action run", "programme standard.wav", "preroll 0.5",
            "capture wire", "seconds %.3f" % (secs + 1.5)])
        cap = os.path.join(work, "captures", "wire-s1.wav")
        crate, _, got = read_wav(cap)
        if crate != rate:
            fail("capture rate %d" % crate)
        first = next(i for i, f in enumerate(prog) if f != (0.0, 0.0))
        start = next(i for i, f in enumerate(got) if f != (0.0, 0.0)) - first
        if got[start:start + len(prog)] != prog:
            fail("the wire capture is not the programme bit for bit")
        print("ok   wire capture holds the programme bit for bit at frame %d (%.3f s)" % (start, start / rate))
        r = subprocess.run([FXLAB, "cut", cap, os.path.join(tmp, "schedule.tsv"), "--out", os.path.join(tmp, "cut")],
                           capture_output=True, text=True)
        m = re.search(r"programme found at frame (\d+)", r.stderr)
        if not m or int(m.group(1)) != start:
            fail("fxlab cut found the programme at %s, it is at %d" % (m and m.group(1), start))
        print("ok   fxlab cut finds it at the same frame and splits it")
        rep = dict(l.split(" ", 1) for l in open(cap[:-4] + ".txt").read().splitlines() if " " in l)
        if rep["overrun_frames"] != "0" or rep["stopped_early"] != "0" or rep["frames_written"] != rep["frames_requested"]:
            fail("wire report: %r" % rep)
        if rep["transport_bpm"] != "120.000" or rep["transport_playing"] != "1":
            fail("transport not reported: %r" % rep)
        print("ok   report: %s frames, no overruns, transport 120 BPM playing" % rep["frames_written"])

        # 2. Through a delay, 6 dB down, timing on, muted output.
        run_chain(bins, work, [os.path.join(bins, "stereo-delay.so")], secs + 4, [
            "seq 2", "action run", "programme standard.wav", "gain -6", "preroll 0.25",
            "capture delay", "seconds %.3f" % (secs + 2), "timing 1"], extra=["--set", "2:1=1"])
        cap = os.path.join(work, "captures", "delay-s1.wav")
        _, _, got = read_wav(cap)
        rep = dict(l.split(" ", 1) for l in open(cap[:-4] + ".txt").read().splitlines() if " " in l)
        if rep["frames_written"] != rep["frames_requested"] or rep["overrun_frames"] != "0":
            fail("delay report: %r" % rep)
        peak = float(rep["peak_dbfs"])
        if not -9.0 < peak < 0.0:
            fail("peak %.2f dBFS: the gain command did not apply" % peak)
        timing = open(cap[:-4] + ".timing.tsv").read().splitlines()
        if not timing[0].startswith("# audio thread policy") or len(timing) < 1000:
            fail("timing log: %r" % timing[:3])
        calls = open(cap[:-4] + ".hostcalls.tsv").read()
        for op in ("setSampleRate", "setBlockSize", "open", "mainsChanged", "startProcess", "setParameter"):
            if "\t" + op + "\t" not in calls:
                fail("host-call log has no %s" % op)
        status = read_status(work, "tap-s1.txt")
        if status.get("output") != "mute":
            fail("Tap output parameter not reported as mute: %r" % status)
        print("ok   delay capture: %s frames, peak %.1f dBFS, %d timing lines, host calls logged, output muted"
              % (rep["frames_written"], peak, len(timing) - 2))
        # 3. The controller script, the short programme and fxlab analyse:
        #    the whole capture-and-measure path, on a wire at two levels.
        ctl = [sys.executable, os.path.join(FXPROBE, "fxprobe.py"), "--local", work]
        subprocess.run([FXLAB, "programme", "--short", "--out", tmp], check=True, capture_output=True)
        short = os.path.join(tmp, "short.wav")
        subprocess.run(ctl + ["setup", "--programme", short], check=True, capture_output=True)
        env = dict(os.environ, FXPROBE_DIR=work)
        ssecs = len(read_wav(short)[2]) / rate
        p = subprocess.Popen([os.path.join(bins, "chain_sim"), "--seconds", "600", "--speed", "4",
                              os.path.join(bins, "fxprobe-source.so"), os.path.join(bins, "fxprobe-tap.so")], env=env)
        wait_for(lambda: read_status(work, "source-s1.txt").get("state") == "idle", 10, "the Source to report")
        r = subprocess.run(ctl + ["capture", "wire-short", "--programme-file", short, "--gains", "0,-12"],
                           capture_output=True, text=True)
        p.terminate()
        p.wait(timeout=10)
        if r.returncode != 0 or "PROBLEMS" in r.stdout:
            fail("fxprobe.py capture: %s %s" % (r.stdout, r.stderr))
        pulled = os.path.join(tmp, "pulled")
        subprocess.run(ctl + ["pull", "--out", pulled], check=True, capture_output=True)
        for tag, gain in (("0", "0"), ("m12", "-12")):
            wavf = os.path.join(pulled, "wire-short-g%s-s1.wav" % tag)
            r = subprocess.run([FXLAB, "analyse", wavf, "--schedule", os.path.join(tmp, "short.tsv"), "--gain", gain,
                                "--out", os.path.join(tmp, "an")], capture_output=True, text=True)
            card = open(os.path.join(tmp, "an", "captures.json")).read()
            m = re.search(r'"resp_1k_db": ([-0-9.e]+)', card)
            if r.returncode != 0 or not m or abs(float(m.group(1))) > 0.01:
                fail("fxlab analyse of %s: %s %s" % (wavf, r.stderr, card[:400]))
        print("ok   fxprobe.py setup/capture/pull at two gains, fxlab analyse reads both as a flat wire")

        # 4. The kit on a stick, as on the MPC under Akai's system: stage.py
        #    makes the kit (native builds), fxprobe.sh installs the plugins
        #    to a fake internal drive, the plugins load from there and find
        #    the work folder on the stick's data partition through their
        #    pointer, also after the stick comes back under another name.
        stick_test(bins, tmp, rate)
        print("PASS")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
