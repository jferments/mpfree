#!/usr/bin/env python3
"""Tests for offline-factory-kit.py against a fake backup: a disk image with a
real GPT whose "content" partition is an ext4 file system made by mke2fs -d,
holding a made-up drum program and WAV files in several formats.

Needs mke2fs and debugfs (e2fsprogs). They are skipped without them unless
MPFREE_REQUIRE_OFFLINE_TOOLS=1, which CI sets.
"""
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import uuid
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "offline-factory-kit.py")
PAD_NOTES = (37, 36, 42, 82, 40, 38, 46, 44, 48, 47, 45, 43, 49, 55, 51, 53)


def tool(name):
    for p in (shutil.which(name), f"/usr/sbin/{name}", f"/sbin/{name}"):
        if p and os.access(p, os.X_OK):
            return p
    return None


MKE2FS, DEBUGFS = tool("mke2fs"), tool("debugfs")
HAVE = bool(MKE2FS and DEBUGFS)
if not HAVE and os.environ.get("MPFREE_REQUIRE_OFFLINE_TOOLS") == "1":
    raise SystemExit("mke2fs and debugfs are required (MPFREE_REQUIRE_OFFLINE_TOOLS=1)")


def wav(rate, channels, bits, frames, tag=1, value=0.5):
    if tag == 3:
        one = struct.pack("<f", value)
    elif bits == 16:
        one = struct.pack("<h", int(value * 32767))
    elif bits == 24:
        one = int(value * 8388607).to_bytes(3, "little", signed=True)
    else:
        raise ValueError(bits)
    data = one * channels * frames
    block = channels * bits // 8
    fmt = struct.pack("<HHIIHH", tag, channels, rate, rate * block, block, bits)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"LIST" + struct.pack("<I", 3) + b"abc\0" \
        + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


XPM = """<?xml version="1.0" encoding="UTF-8"?>
<MPCVObject>
  <Version><File_Version>2.1</File_Version><Application>MPC-V</Application></Version>
  <Program type="Drum">
    <ProgramName>Test Kit</ProgramName>
    <Instruments>
{instruments}
    </Instruments>
    <PadNoteMap>
{padnotes}
    </PadNoteMap>
  </Program>
</MPCVObject>
"""


def xpm(samples):
    inst = []
    for n in range(1, 129):
        name = samples.get(n, "")
        inst.append(f'      <Instrument number="{n}"><Layers><Layer number="1"><Active>True</Active>'
                    f'<SampleName>{name}</SampleName><SampleFile></SampleFile></Layer></Layers></Instrument>')
    notes = "\n".join(f'      <PadNote number="{p}"><Note>{PAD_NOTES[p - 1]}</Note></PadNote>' for p in range(1, 17))
    return XPM.format(instruments="\n".join(inst), padnotes=notes)


def gpt_disk(path, parts):
    """Write a disk image with a valid primary GPT. parts: [(name, first_lba, content bytes)]."""
    entries = bytearray(128 * 128)
    end = max(first + len(data) // 512 for _, first, data in parts) + 34
    for i, (name, first, data) in enumerate(parts):
        e = uuid.uuid4().bytes_le + uuid.uuid4().bytes_le + struct.pack("<QQQ", first, first + len(data) // 512 - 1, 0)
        e += name.encode("utf-16-le").ljust(72, b"\0")
        entries[i * 128:(i + 1) * 128] = e
    head = bytearray(struct.pack("<8sIIIIQQQQ16sQIII", b"EFI PART", 0x10000, 92, 0, 0, 1, end - 1, 34, end - 34,
                                 uuid.uuid4().bytes_le, 2, 128, 128, zlib.crc32(bytes(entries))))
    struct.pack_into("<I", head, 16, zlib.crc32(bytes(head)))
    with open(path, "wb") as fh:
        fh.truncate(end * 512)
        fh.seek(512)
        fh.write(bytes(head).ljust(512, b"\0"))
        fh.write(entries)
        for _, first, data in parts:
            fh.seek(first * 512)
            fh.write(data)


def parse_kit(data):
    assert data[:8] == b"MPFKIT01"
    count, = struct.unpack_from("<I", data, 8)
    at, sounds = 12, []
    for _ in range(count):
        note = data[at]
        assert data[at + 1:at + 4] == b"\0\0\0"
        frames, nlen = struct.unpack_from("<IH", data, at + 4)
        name = data[at + 10:at + 10 + nlen].decode()
        at += 10 + nlen
        audio = data[at:at + frames * 4]
        at += frames * 4
        sounds.append((note, name, frames, audio))
    assert at == len(data)
    return sounds


@unittest.skipUnless(HAVE, "needs mke2fs and debugfs")
class FactoryKitTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.backups = os.path.join(self.tmp, "backups")
        self.work = os.path.join(self.tmp, "work")
        os.makedirs(self.work)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def content(self, files):
        src = os.path.join(self.tmp, "src")
        for rel, data in files.items():
            p = os.path.join(src, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as fh:
                fh.write(data if isinstance(data, bytes) else data.encode())
        img = os.path.join(self.tmp, "content.img")
        subprocess.run([MKE2FS, "-q", "-F", "-t", "ext4", "-d", src, img, "16M"], check=True)
        with open(img, "rb") as fh:
            return fh.read()

    def backup(self, files, name="mpc-one-emmc-20261109T233604Z-abc"):
        folder = os.path.join(self.backups, name)
        os.makedirs(folder)
        gpt_disk(os.path.join(folder, "mmcblk0.img"),
                 [("rootfs", 2048, b"\x55" * 4096), ("content", 4096, self.content(files)), ("data", 40960, b"\0" * 4096)])
        os.chmod(os.path.join(folder, "mmcblk0.img"), 0o400)
        return folder

    def run_tool(self, *args):
        env = dict(os.environ, MPFREE_BACKUPS_DIR=self.backups, MPFREE_WORK_DIR=self.work)
        before = set(os.listdir(self.work))
        res = subprocess.run([sys.executable, TOOL, *args], capture_output=True, text=True, env=env, timeout=120)
        kits = sorted(d for d in set(os.listdir(self.work)) - before if d.startswith("factory-kit-"))
        report = ""
        if kits:
            with open(os.path.join(self.work, kits[-1], "report.txt")) as fh:
                report = fh.read()
        return res, kits, report

    def test_makes_a_kit_from_a_drum_program(self):
        names = {1: "Kick", 2: "Snare", 3: "Hat", 4: "Clap", 5: "Tom", 6: "Rim", 7: "Shaker", 8: "Crash", 16: "Missing"}
        files = {
            "Drums/Test Kit/Test Kit.xpm": xpm(names),
            "Drums/Test Kit/Kick.wav": wav(44100, 1, 16, 4410),
            "Drums/Test Kit/Snare.wav": wav(48000, 2, 24, 4800),
            "Drums/Test Kit/Hat.wav": wav(44100, 2, 32, 2205, tag=3),
            "Drums/Test Kit/clap.WAV": wav(44100, 1, 16, 100),
            "Samples/Tom.wav": wav(44100, 1, 16, 100),
            "Drums/Test Kit/Rim.wav": wav(44100, 1, 16, 100),
            "Drums/Test Kit/Shaker.wav": wav(44100, 1, 16, 100),
            "Drums/Test Kit/Crash.wav": wav(22050, 1, 16, 22050 * 12),
            "Keys/Piano.xpm": XPM.replace('type="Drum"', 'type="Keygroup"').format(instruments="", padnotes=""),
            "My Folder/notes.txt": "x",
        }
        folder = self.backup(files)
        before = os.stat(os.path.join(folder, "mmcblk0.img"))
        res, kits, report = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertEqual(len(kits), 1)
        self.assertIn("chosen: /Drums/Test Kit/Test Kit.xpm (Test Kit), pads read by pad number", report)
        self.assertIn("drum programs: 1", report)
        self.assertRegex(report, r"\n +16 +53 +- \(Missing: file not found\)\n")
        with open(os.path.join(self.work, kits[0], "kit.bin"), "rb") as fh:
            sounds = parse_kit(fh.read())
        self.assertEqual([(n, name) for n, name, _, _ in sounds],
                         list(zip(PAD_NOTES[:8], ["Kick", "Snare", "Hat", "Clap", "Tom", "Rim", "Shaker", "Crash"])))
        frames = {name: f for _, name, f, _ in sounds}
        self.assertEqual(frames["Kick"], 4410)
        self.assertEqual(frames["Snare"], 4410)   # 48 kHz -> 44.1 kHz
        self.assertEqual(frames["Crash"], 441000)  # cut to 10 s
        kick = sounds[0][3]
        self.assertEqual(struct.unpack_from("<hh", kick, 0), (16383, 16383))  # mono to both channels
        hat = sounds[2][3]
        self.assertEqual(struct.unpack_from("<hh", hat, 0), (16384, 16384))
        self.assertIn("(48000 Hz, 2 ch, 24-bit int, 0.10 s)", report)
        self.assertIn("cut to 10 s", report)
        self.assertIn("| <MPCVObject>", report)
        # The backup is untouched, and nothing but the kit folder was written.
        after = os.stat(os.path.join(folder, "mmcblk0.img"))
        self.assertEqual((before.st_mtime_ns, before.st_size), (after.st_mtime_ns, after.st_size))
        self.assertEqual(sorted(os.listdir(os.path.join(self.work, kits[0]))), ["kit.bin", "report.txt"])
        self.assertEqual(sorted(os.listdir(self.work)), kits)

    def test_loose_wavs_when_there_is_no_program(self):
        files = {"Loops/loop.wav": wav(44100, 1, 16, 10),
                 "One Shots/Snare 1.wav": wav(44100, 1, 16, 10),
                 "One Shots/Kick 1.wav": wav(44100, 1, 16, 10),
                 "One Shots/HH Closed.wav": wav(44100, 1, 16, 10)}
        self.backup(files)
        res, kits, report = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn("chosen: /One Shots/ (One Shots), pads read loose WAVs", report)
        with open(os.path.join(self.work, kits[0], "kit.bin"), "rb") as fh:
            sounds = parse_kit(fh.read())
        self.assertEqual([name for _, name, _, _ in sounds], ["Kick 1.wav", "Snare 1.wav", "HH Closed.wav"])

    def test_stops_cleanly(self):
        res, kits, _ = self.run_tool()
        self.assertEqual(res.returncode, 1)
        self.assertIn("no backup folder", res.stdout)
        self.assertEqual(kits, [])
        folder = self.backup({"a.txt": "x"})
        img = os.path.join(folder, "mmcblk0.img")
        os.chmod(img, 0o600)
        with open(img, "r+b") as fh:
            fh.seek(512 + 40)
            fh.write(b"\x01")
        res, kits, report = self.run_tool()
        self.assertEqual(res.returncode, 1)
        self.assertIn("checksum is wrong", report)
        shutil.rmtree(folder)
        self.backup({"a.txt": "x"})
        res, kits, report = self.run_tool()
        self.assertEqual(res.returncode, 1)
        self.assertIn("no drum program and no folder of drum WAVs", report)


class StaticTests(unittest.TestCase):
    def test_reads_the_backup_only(self):
        with open(TOOL) as fh:
            src = fh.read()
        self.assertNotIn('"-w"', src)
        self.assertNotIn("'-w'", src)
        self.assertIn('[self.debugfs, "-f", cmdfile, self.spec]', src)
        # Every open() for writing, by its mode.
        writes = [m.group(1) for line in src.splitlines() for m in [re.search(r'open\((.+), "(w|wb|xb|a|r\+b)"\)', line)] if m]
        self.assertEqual(sorted(writes), ['kit_path', 'os.path.join(out_dir, "report.txt")'])
        self.assertIn('NamedTemporaryFile("w", suffix=".cmds"', src)
        for needle in ('"mount"', '"sudo"', "ssh ", "losetup", "rmtree", "os.remove", "fallocate"):
            self.assertFalse(needle in src, needle)
        # debugfs is the only program it runs.
        self.assertEqual(src.count("subprocess."), 1)
        self.assertIn("os.unlink(cmdfile)", src)


if __name__ == "__main__":
    unittest.main(verbosity=2)
