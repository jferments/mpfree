#!/usr/bin/env python3
"""Tests for stage.py and fxprobe.sh (the on-device FXProbe tool). No MPC, no
SDK, no Rust.

stage.py makes a kit from fake plugin folders (ELF headers for the right ARM)
and a fake fxlab. fxprobe.sh runs under BusyBox applets only, against a fake
root (FXPROBE_TEST_ROOT): its proc/mounts names the stick's kits partition
(where the kit is), its data partition under a UUID mount name, the MPC's
internal drive and a noexec drive, as Akai's system mounts them. A Python
stand-in for live FXProbe Source and Tap instances answers control.txt the way
the plugins do (status files, capture reports). The real plugins are tested
end to end with fxprobe.sh in test/run_chain_test.py.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of falling back to the
system sh and tools without BusyBox.
"""
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
FXPROBE = os.path.dirname(HERE)
SCRIPT = os.path.join(FXPROBE, "fxprobe.sh")
STAGE = os.path.join(FXPROBE, "stage.py")
APPLETS = ("sh", "awk", "cat", "cp", "cut", "date", "grep", "head", "ls", "mkdir", "mv", "rm", "rmdir", "sed",
           "sha256sum", "sleep", "sync", "tr")
SOURCE = "MPFree - VST - FXProbe Source"
TAP = "MPFree - VST - FXProbe Tap"

FAKE_FXLAB = r'''#!/bin/sh
# A stand-in for fxlab programme: tiny programmes with real schedules.
[ "$1" = programme ] || exit 2
short=0
while [ $# -gt 0 ]; do case "$1" in --short) short=1 ;; --out) out=$2; shift ;; esac; shift; done
if [ "$short" = 1 ]; then w=short.wav t=short.tsv; else w=programme.wav t=schedule.tsv; fi
printf 'RIFF....WAVEfake %s\n' "$w" > "$out/$w"
printf 'rate\t44100\n0\tsync\t0\t4410\t2205\tsync\n1\tsweep -12 dB\t6615\t8820\t4410\tsweep:-12\n' > "$out/$t"
echo "wrote $w"
'''


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


def elf(cls, machine):
    """The first bytes of an ELF shared library."""
    return b"\x7fELF" + bytes([cls, 1, 1]) + b"\x00" * 9 + struct.pack("<HH", 3, machine) + b"\x00" * 44


def parse(text):
    return dict(line.split(" ", 1) for line in text.splitlines() if " " in line)


class Instances:
    """Live FXProbe instances, as the plugins behave: once the work folder holds
    the marker they report idle; a new command seq starts a capture in every
    Tap it is for, which ends with a report and state done."""

    def __init__(self, work, slots=(1, 2), overrun_slot=None, delay=0.3):
        self.work, self.slots, self.overrun_slot, self.delay = work, slots, overrun_slot, delay
        self.stop = threading.Event()
        self.seen = None
        self.thread = threading.Thread(target=self.run, daemon=True)

    def status(self, kind, slot, fields):
        text = "fxprobe %s\nversion 1\nslot %d\n" % (kind, slot) + "".join("%s %s\n" % kv for kv in fields)
        path = os.path.join(self.work, "status", "%s-s%d.txt" % (kind, slot))
        write(path + ".tmp", text)
        os.replace(path + ".tmp", path)

    def run(self):
        while not self.stop.is_set():
            time.sleep(0.05)
            if not os.path.isfile(os.path.join(self.work, ".fxprobe-work")):
                continue
            if self.seen is None:
                for s in self.slots:
                    self.status("source", s, [("state", "idle"), ("seq", "-1"), ("programme", "")])
                    self.status("tap", s, [("state", "idle"), ("rate", "44100")])
                try:
                    with open(os.path.join(self.work, "control.txt")) as fh:
                        self.seen = parse(fh.read()).get("seq", "")
                except OSError:
                    self.seen = ""
                continue
            try:
                with open(os.path.join(self.work, "control.txt")) as fh:
                    c = parse(fh.read())
            except OSError:
                continue
            if c.get("seq") == self.seen:
                continue
            self.seen = c.get("seq")
            want = [int(x) for x in c["slots"].split(",")] if "slots" in c else list(self.slots)
            for s in want:
                self.status("source", s, [("state", "playing"), ("seq", c["seq"]), ("programme", c["programme"])])
                self.status("tap", s, [("state", "recording"), ("capture", c["capture"]), ("seq", c["seq"])])
            time.sleep(self.delay)
            frames = round(float(c["seconds"]) * 44100)
            for s in want:
                over = 512 if s == self.overrun_slot else 0
                base = os.path.join(self.work, "captures", "%s-s%d" % (c["capture"], s))
                write(base + ".wav", b"RIFF")
                write(base + ".txt", "fxprobe tap\nslot %d\ncapture %s\nseq %s\nframes_requested %d\nframes_written %d\n"
                      "overrun_frames %d\nstopped_early 0\nnonfinite_samples 0\n"
                      % (s, c["capture"], c["seq"], frames, frames - over, over))
                self.status("source", s, [("state", "done"), ("seq", c["seq"]), ("programme", c["programme"])])
                self.status("tap", s, [("state", "done"), ("capture", c["capture"]), ("seq", c["seq"])])


class FxprobeShTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox and os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
            raise RuntimeError("BusyBox is required for these tests")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="fxprobe-sh-test-")
        t = self.tmp
        self.root = os.path.join(t, "root")
        r = self.root
        write(f"{r}/proc/mounts",
              "/dev/root / ext4 ro,relatime 0 0\n"
              "/dev/mmcblk0p8 /data ext4 rw,relatime 0 0\n"
              "/dev/mmcblk0p8 /media/az01-internal ext4 rw,relatime 0 0\n"
              "/dev/sdz3 /media/MPFREE-KITS ext4 rw,nosuid,nodev,noexec,relatime,nosymfollow 0 0\n"
              "/dev/sdz4 /media/4c1e9a52-data ext4 rw,nosuid,nodev,noexec,relatime,nosymfollow 0 0\n"
              "/dev/sdy1 /media/USB321FD vfat rw,nosuid,nodev,noexec,relatime,nosymfollow 0 0\n")
        for d in ("media/az01-internal", "media/4c1e9a52-data", "media/USB321FD", "media/notmounted"):
            os.makedirs(os.path.join(r, d))
        write(f"{r}/media/4c1e9a52-data/FORMAT", "mpfree-data 1\n")
        self.internal = f"{r}/media/az01-internal"
        self.data = f"{r}/media/4c1e9a52-data"
        self.work = f"{self.data}/FXProbe"
        # Fake plugin builds and fxlab, then a real stage.
        built = os.path.join(t, "built")
        for folder, slug, uid in ((SOURCE, "fxprobe-source", "MFps"), (TAP, "fxprobe-tap", "MFpt")):
            write(f"{built}/{folder}/plugin.json",
                  '{\n  "name": "%s",\n  "manufacturer": "MPFree",\n  "uid": "%s"\n}\n' % (folder[15:], uid))
            write(f"{built}/{folder}/VST/{slug}-arm32.so", elf(1, 40))
            write(f"{built}/{folder}/VST/{slug}-arm64.so", elf(2, 183))
            write(f"{built}/{folder}/Plugin Skins/TUI.json", "{}\n")
        self.built = built
        self.fxlab = os.path.join(t, "fxlab")
        write(self.fxlab, FAKE_FXLAB, 0o755)
        self.kit = f"{r}/media/MPFREE-KITS/fxprobe"
        os.makedirs(os.path.dirname(self.kit))
        res = self.stage(self.kit)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("STAGE-OK", res.stdout)
        # Only BusyBox's applets on PATH (or the system's tools without BusyBox).
        if self.busybox:
            bindir = os.path.join(t, "bin")
            os.makedirs(bindir)
            for name in APPLETS:
                os.symlink(self.busybox, os.path.join(bindir, name))
            self.shell, path = os.path.join(bindir, "sh"), bindir
        else:
            self.shell, path = "/bin/sh", os.environ.get("PATH", "/usr/bin:/bin")
        self.env = {"PATH": path, "FXPROBE_TEST_ROOT": r, "HOME": t}
        self.instances = None

    def tearDown(self):
        if self.instances:
            self.instances.stop.set()
            self.instances.thread.join(5)
        shutil.rmtree(self.tmp, ignore_errors=True)

    def stage(self, out, *extra):
        return subprocess.run([sys.executable, STAGE, "--plugins-from", self.built, "--fxlab", self.fxlab,
                               "--out", out] + list(extra), stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                              text=True, timeout=60)

    def sh(self, *args, timeout=60):
        return subprocess.run([self.shell, os.path.join(self.kit, "fxprobe.sh")] + list(args), env=self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout)

    def ok(self, res, last):
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(res.stdout.strip().splitlines()[-1], last, res.stdout)

    def stopped(self, res, text):
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: " + text, res.stdout)

    def tree(self, path):
        out = {}
        for dirpath, _, files in os.walk(path):
            for f in files:
                p = os.path.join(dirpath, f)
                with open(p, "rb") as fh:
                    out[os.path.relpath(p, path)] = hashlib.sha256(fh.read()).hexdigest()
        return out

    # ---- stage.py ------------------------------------------------------------

    def test_stage_makes_the_kit_with_sums_and_version(self):
        names = sorted(self.tree(self.kit))
        self.assertEqual(names, sorted([
            "README.txt", "SHA256SUMS", "VERSION", "fxprobe.sh",
            f"plugins/{SOURCE}/plugin.json", f"plugins/{SOURCE}/VST/fxprobe-source-arm32.so",
            f"plugins/{SOURCE}/VST/fxprobe-source-arm64.so", f"plugins/{SOURCE}/Plugin Skins/TUI.json",
            f"plugins/{TAP}/plugin.json", f"plugins/{TAP}/VST/fxprobe-tap-arm32.so",
            f"plugins/{TAP}/VST/fxprobe-tap-arm64.so", f"plugins/{TAP}/Plugin Skins/TUI.json",
            "programmes/short.tsv", "programmes/short.wav", "programmes/standard.tsv", "programmes/standard.wav"]))
        sums = self.tree(self.kit)
        with open(os.path.join(self.kit, "SHA256SUMS")) as fh:
            lines = fh.read().splitlines()
        self.assertEqual(len(lines), len(names) - 1)
        for line in lines:
            digest, rel = line.split("  ", 1)
            self.assertEqual(sums[rel], digest, rel)
        with open(os.path.join(self.kit, "VERSION")) as fh:
            version = fh.read()
        self.assertIn("sdk prebuilt (--plugins-from)", version)
        self.assertRegex(version, r"made \d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ")
        with open(os.path.join(self.kit, "fxprobe.sh")) as fh, open(SCRIPT) as orig:
            self.assertEqual(fh.read(), orig.read())

    def test_stage_refuses_an_existing_folder_and_the_wrong_arm(self):
        res = self.stage(self.kit)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("exists; give a new folder", res.stdout)
        write(f"{self.built}/{TAP}/VST/fxprobe-tap-arm64.so", elf(2, 62))   # x86-64
        out = os.path.join(self.tmp, "kit2")
        res = self.stage(out)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("is not a 64-bit ARM shared library", res.stdout)
        self.assertFalse(os.path.exists(out))
        self.assertEqual([n for n in os.listdir(self.tmp) if n.startswith(".fxprobe-kit-")], [])

    # ---- install and remove ----------------------------------------------------

    def test_install_check_reinstall_and_remove(self):
        res = self.sh("check")
        self.ok(res, "CHECK-OK")
        self.assertIn(f"  {SOURCE}: not installed", res.stdout)
        self.assertIn("data partition: /dev/sdz4 /media/4c1e9a52-data ext4", res.stdout)
        self.assertIn("  FORMAT: mpfree-data 1", res.stdout)
        self.assertIn("  not made yet (run setup)", res.stdout)
        res = self.sh("install")
        self.ok(res, "INSTALL-OK")
        self.assertIn("drive: /dev/mmcblk0p8 /media/az01-internal ext4 rw,relatime", res.stdout)
        target = f"{self.internal}/VST2 Plugins"
        self.assertEqual(sorted(os.listdir(target)), [SOURCE, TAP])
        self.assertEqual(os.listdir(self.internal), ["VST2 Plugins"])   # no staging folder left
        for folder in (SOURCE, TAP):
            with open(f"{target}/{folder}/workfolder.txt") as fh:
                self.assertEqual(fh.read(), self.root + "/media/*/FXProbe\n")
            with open(f"{target}/{folder}/fxprobe-install.txt") as fh:
                self.assertIn("made-vst2-folder 1", fh.read())
            kit = self.tree(f"{self.kit}/plugins/{folder}")
            got = self.tree(f"{target}/{folder}")
            self.assertEqual({k: v for k, v in got.items() if k in kit}, kit)
        res = self.sh("check")
        self.ok(res, "CHECK-OK")
        self.assertIn(f"  {TAP}: installed, same as the kit; pointer {self.root}/media/*/FXProbe", res.stdout)
        res = self.sh("install")
        self.ok(res, "INSTALL-OK")
        self.assertIn(f"install: replacing the installed {TAP}", res.stdout)
        res = self.sh("remove")
        self.ok(res, "REMOVE-OK")
        self.assertIn("remove: deleted the empty", res.stdout)
        self.assertEqual(os.listdir(self.internal), [])

    def test_remove_leaves_a_vst2_folder_it_did_not_make_and_other_plugins(self):
        other = f"{self.internal}/VST2 Plugins/Acme - VST - Wobble/plugin.json"
        write(other, "{}\n")
        self.ok(self.sh("install"), "INSTALL-OK")
        res = self.sh("remove")
        self.ok(res, "REMOVE-OK")
        self.assertEqual(os.listdir(f"{self.internal}/VST2 Plugins"), ["Acme - VST - Wobble"])
        self.ok(self.sh("remove"), "REMOVE-OK")   # nothing left to remove is not an error

    def test_install_refuses_noexec_unmounted_foreign_and_damaged(self):
        self.stopped(self.sh("install", "--drive", f"{self.root}/media/USB321FD"), "/media/USB321FD is mounted noexec")
        self.stopped(self.sh("install", "--drive", f"{self.root}/media/notmounted"), f"{self.root}/media/notmounted is not a mount point")
        self.assertEqual(os.listdir(f"{self.root}/media/USB321FD"), [])
        foreign = f"{self.internal}/VST2 Plugins/{TAP}/plugin.json"
        write(foreign, '{"manufacturer": "Someone", "uid": "MFpt"}\n')
        before = self.tree(self.internal)
        self.stopped(self.sh("install"), f"{self.internal}/VST2 Plugins/{TAP} exists and is not FXProbe's")
        self.assertEqual(self.tree(self.internal), before)
        res = self.sh("remove")
        self.ok(res, "REMOVE-OK")
        self.assertIn("left", res.stdout)
        self.assertTrue(os.path.isfile(foreign))
        os.remove(foreign)
        with open(f"{self.kit}/plugins/{SOURCE}/VST/fxprobe-source-arm32.so", "ab") as fh:
            fh.write(b"x")
        self.stopped(self.sh("install"), "the kit does not match its SHA256SUMS")

    # ---- setup, status, capture, clean -----------------------------------------

    def test_setup_capture_and_clean(self):
        res = self.sh("status")
        self.assertIn("no instance has reported", res.stdout)
        res = self.sh("setup")
        self.ok(res, "SETUP-OK")
        self.assertIn(f"setup: work folder {self.work}", res.stdout)
        self.assertTrue(os.path.isfile(f"{self.work}/.fxprobe-work"))
        self.assertEqual(sorted(os.listdir(f"{self.work}/programmes")),
                         ["short.tsv", "short.wav", "standard.tsv", "standard.wav"])
        res = self.sh("setup")
        self.ok(res, "SETUP-OK")
        self.assertNotIn("setup: programmes/", res.stdout)   # already there, not copied again
        self.instances = Instances(self.work)
        self.instances.thread.start()
        deadline = time.time() + 10
        while time.time() < deadline and len(os.listdir(f"{self.work}/status")) < 4:
            time.sleep(0.05)
        res = self.sh("status")
        self.assertIn("tap-s2         state idle", res.stdout)
        res = self.sh("capture", "flanger", "--gains", "0,-12", "--tail", "0")
        self.ok(res, "CAPTURE-OK")
        self.assertIn("slots: 1 2; 2 capture(s) of 1.450 s each", res.stdout)
        self.assertIn("captured flanger-g0 from 2 slot(s)", res.stdout)
        self.assertIn("captured flanger-gm12 from 2 slot(s)", res.stdout)
        with open(f"{self.work}/control.txt") as fh:
            self.assertEqual(fh.read(), "seq 2\naction run\nprogramme short.wav\ngain -12\npreroll 0.5\n"
                                        "capture flanger-gm12\nseconds 1.450\n")
        # The same name again, one slot, the standard programme, timing log: a
        # done status left from before does not end the wait (its seq is old).
        self.instances.delay = 1.5
        res = self.sh("capture", "flanger-gm12", "--slots", "2", "--programme", "standard", "--timing")
        self.ok(res, "CAPTURE-OK")
        self.assertIn("slots: 2; 1 capture(s) of 3.450 s each", res.stdout)
        with open(f"{self.work}/control.txt") as fh:
            self.assertEqual(fh.read(), "seq 3\naction run\nprogramme standard.wav\ngain 0\npreroll 0.5\n"
                                        "capture flanger-gm12\nseconds 3.450\ntiming 1\nslots 2\n")
        self.instances.delay = 0.3
        self.instances.overrun_slot = 2
        res = self.sh("capture", "bad", "--tail", "0")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("captured bad from 2 slot(s); PROBLEMS: s2: written 63433 of 63945, overrun 512", res.stdout)
        self.assertNotIn("CAPTURE-OK", res.stdout)
        self.stopped(self.sh("capture", "../x"), "capture names are letters")
        self.stopped(self.sh("capture", "x", "--programme", "long"), "--programme is short or standard")
        kit = self.tree(self.kit)
        res = self.sh("clean")
        self.ok(res, "CLEAN-OK")
        self.assertIn("(6 capture(s))", res.stdout)
        self.assertFalse(os.path.exists(self.work))
        self.assertEqual(sorted(os.listdir(self.data)), ["FORMAT"])
        self.assertEqual(self.tree(self.kit), kit)

    def test_work_folder_needs_the_stick_data_partition(self):
        with open(f"{self.root}/proc/mounts") as fh:
            mounts = fh.read()
        write(f"{self.root}/proc/mounts", "".join(l + "\n" for l in mounts.splitlines() if "sdz4" not in l))
        self.stopped(self.sh("setup"), "the stick's data partition (/dev/sdz4, the one after the kit's /dev/sdz3) is not mounted")
        other = os.path.join(self.tmp, "elsewhere", "FXProbe")
        os.makedirs(os.path.dirname(other))
        res = self.sh("--work", other, "setup")
        self.ok(res, "SETUP-OK")
        self.assertTrue(os.path.isfile(os.path.join(other, ".fxprobe-work")))

    def test_capture_without_setup_or_instances_stops(self):
        self.stopped(self.sh("capture", "x"), f"{self.work} is not set up (run setup)")
        self.ok(self.sh("setup"), "SETUP-OK")
        self.stopped(self.sh("capture", "x"), "no Source and Tap instances reporting")


if __name__ == "__main__":
    unittest.main()
