#!/usr/bin/env python3
"""WORKSTATION ONLY: make drum kits for Hello MPC from Akai's factory content,
read from the owner's eMMC backup. It never contacts the MPC.

  python3 tools/mpc-one/offline-factory-kit.py [--backup FOLDER] [--program TEXT] [--kits N]

1. Finds the newest backup folder ~/mpfree-backups/mpc-one-emmc-*/ that has
   mmcblk0.img (made by `collect-over-ssh.sh --backup`), unless --backup names one.
2. Reads the GPT in mmcblk0.img and finds the partition named "content" (p7,
   mounted at /media/acvs-content on the MPC).
3. Lists that ext4 file system with debugfs, which opens the image read-only
   (no -w) at the partition's offset. Nothing is mounted and no sudo is needed.
4. Finds drum programs (.xpm files whose XML says type="Drum"). Kit 1 is the
   first one, by path, whose pads 1 to 16 have at least 8 samples it can find
   (--program picks the first whose path contains TEXT instead). Kits 2 to N
   (--kits, default 8) are the next programs by path with at least 12 samples
   found, one per name family (the part of the file name before "-Kit", or its
   first word), while the file stays under 48 MiB.
5. Copies each kit's WAV files out with debugfs into a temporary folder,
   converts them to 44.1 kHz stereo 16-bit (at most 10 s each), and deletes
   the copies.
6. Writes a new folder ~/mpfree-work/factory-kit-<UTC>-XXXXXXXX/ with kits.bin
   (for mpfree-hello's drums and play steps; format in
   rust/mpfree-hello/src/kit.rs) and report.txt (file and sample names,
   formats and kit 1's program XML; no audio).

The backup is opened only for reading. The kit is Akai's copyrighted content:
it stays in ~/mpfree-work and on the MPC's RAM. Never commit it or share it;
results.py copies only report.txt.

MPFREE_BACKUPS_DIR and MPFREE_WORK_DIR change the folders (tests use them).
"""
import argparse
import array
import datetime
import glob
import hashlib
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
import zlib

SECTOR = 512
RATE = 44_100
MAX_SECONDS = 10
MAX_XPM = 4 << 20
MAX_WAV = 64 << 20
MAX_KIT = 48 << 20
MAX_ENTRIES = 300_000
MAX_DEPTH = 16
MIN_PADS = 8
# Samples found on pads 1-16 for kits after the first.
MIN_EXTRA_PADS = 12
DEFAULT_KITS = 8
MAX_KITS = 16
MAX_KIT_NAME = 120
MAX_REPORTED_PROGRAMS = 200
# The controller's note for pads 1 to 16 (rust/mpfree-controller/src/mpc_one.rs
# PAD_NOTES). Assumed, not checked: programs map pads to these same notes by
# default. The tool tries both readings and reports which one it used.
PAD_NOTES = (37, 36, 42, 82, 40, 38, 46, 44, 48, 47, 45, 43, 49, 55, 51, 53)
MAGIC = b"MPFKIT01"
KITS_MAGIC = b"MPFKITS1"
# Words that make a folder of loose WAVs look like drums, when no program is found.
DRUM_WORDS = ("kick", "snare", "clap", "hat", "hh", "tom", "perc", "rim", "crash", "ride", "cymbal", "shaker", "snap")


class Stop(Exception):
    pass


def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")


# ------------------------------------------------------------------ backup and GPT

def newest_backup(root):
    folders = sorted(d for d in glob.glob(os.path.join(root, "mpc-one-emmc-*"))
                     if os.path.isfile(os.path.join(d, "mmcblk0.img")))
    if not folders:
        raise Stop(f"no backup folder with mmcblk0.img under {root} (make one with collect-over-ssh.sh --backup)")
    return folders[-1]


def gpt_partitions(path):
    """[(name, first_lba, last_lba)] from the primary GPT, checking its CRCs."""
    with open(path, "rb") as fh:
        fh.seek(SECTOR)
        head = fh.read(SECTOR)
        if head[:8] != b"EFI PART":
            raise Stop(f"{path}: no GPT header at sector 1")
        size, crc = struct.unpack_from("<II", head, 12)
        if not 92 <= size <= SECTOR:
            raise Stop(f"{path}: odd GPT header size {size}")
        check = bytearray(head[:size])
        check[16:20] = b"\0\0\0\0"
        if zlib.crc32(bytes(check)) != crc:
            raise Stop(f"{path}: the GPT header's checksum is wrong")
        entries_lba, count, entry_size, entries_crc = struct.unpack_from("<QIII", head, 72)
        if not (128 <= entry_size <= 1024 and count <= 1024):
            raise Stop(f"{path}: odd GPT entry layout")
        fh.seek(entries_lba * SECTOR)
        table = fh.read(count * entry_size)
    if len(table) != count * entry_size or zlib.crc32(table) != entries_crc:
        raise Stop(f"{path}: the GPT partition table's checksum is wrong")
    parts = []
    for i in range(count):
        e = table[i * entry_size:(i + 1) * entry_size]
        if e[:16] == b"\0" * 16:
            continue
        first, last = struct.unpack_from("<QQ", e, 32)
        name = e[56:128].decode("utf-16-le", "replace").split("\0")[0]
        parts.append((name, first, last))
    return parts


# ------------------------------------------------------------------ debugfs (read-only)

def find_debugfs():
    for p in (shutil.which("debugfs"), "/usr/sbin/debugfs", "/sbin/debugfs"):
        if p and os.access(p, os.X_OK):
            return p
    raise Stop("debugfs not found (it is in the e2fsprogs package)")


class Image:
    """The content file system inside the backup, through debugfs without -w."""

    def __init__(self, debugfs, image, offset):
        if any(c in image for c in "? \t\n\"'"):
            raise Stop(f"the backup path {image} has a character debugfs cannot take")
        self.debugfs = debugfs
        self.spec = f"{image}?offset={offset}"

    def batch(self, commands, timeout=600):
        """Run debugfs commands; returns [(command, output lines)]."""
        with tempfile.NamedTemporaryFile("w", suffix=".cmds", delete=False) as fh:
            fh.write("\n".join(commands) + "\n")
            cmdfile = fh.name
        try:
            res = subprocess.run([self.debugfs, "-f", cmdfile, self.spec], capture_output=True, text=True,
                                 errors="replace", timeout=timeout)
        finally:
            os.unlink(cmdfile)
        out, current = [], None
        for line in res.stdout.splitlines():
            if line.startswith("debugfs: "):
                current = (line[len("debugfs: "):], [])
                out.append(current)
            elif current is not None:
                current[1].append(line)
        return out, res

    def stats(self):
        out, res = self.batch(["stats"], timeout=120)
        lines = out[0][1] if out else []
        info = {}
        for line in lines:
            k, sep, v = line.partition(":")
            if sep and k.strip() in ("Filesystem volume name", "Block count", "Block size", "Inode count",
                                     "Free blocks", "Filesystem features", "Last mount time"):
                info[k.strip()] = v.strip()
        if "Block count" not in info:
            raise Stop("debugfs could not read the content partition at its offset "
                       f"({res.stderr.strip()[:300] or 'no output'}); this debugfs may be too old for ?offset=")
        return info


def quoted(path):
    if '"' in path or "\\" in path or any(ord(c) < 32 for c in path):
        return None
    return f'"{path}"'


def walk(img):
    """Every file and folder: {path: (is_dir, size)}, breadth first, one debugfs run per level."""
    tree = {"/": (True, 0)}
    level = ["/"]
    skipped = 0
    for _depth in range(MAX_DEPTH):
        cmds, dirs = [], []
        for d in level:
            q = quoted(d)
            if q is None:
                skipped += 1
                continue
            cmds.append(f"ls -p {q}")
            dirs.append(d)
        if not cmds:
            break
        out, _ = img.batch(cmds)
        nxt = []
        for (cmd, lines), d in zip(out, dirs):
            for line in lines:
                f = line.split("/")
                if len(f) != 8 or f[0] or f[7]:
                    continue
                mode, name, size = f[2], f[5], f[6]
                if name in (".", "..", "lost+found"):
                    continue
                path = d.rstrip("/") + "/" + name
                is_dir = mode.startswith("04")
                tree[path] = (is_dir, 0 if is_dir else int(size or 0))
                if is_dir:
                    nxt.append(path)
                if len(tree) > MAX_ENTRIES:
                    raise Stop(f"more than {MAX_ENTRIES} entries in the content partition; stopping the listing")
        level = nxt
    return tree, skipped


def dump(img, wanted, folder):
    """Copy files out with debugfs dump; returns {path: local file}."""
    cmds, local = [], {}
    for i, path in enumerate(wanted):
        q = quoted(path)
        if q is None:
            continue
        dest = os.path.join(folder, f"{i:03d}.bin")
        cmds.append(f'dump {q} "{dest}"')
        local[path] = dest
    if cmds:
        img.batch(cmds)
    return {p: f for p, f in local.items() if os.path.isfile(f)}


# ------------------------------------------------------------------ drum programs

def program_info(text):
    """(program name, {instrument number: (sample name, sample file)}, {pad: note}) or None if not a drum program."""
    try:
        root = ET.fromstring(text)
    except ET.ParseError:
        return None
    prog = None
    for el in root.iter("Program"):
        if el.get("type", "").lower() == "drum":
            prog = el
            break
    if prog is None:
        return None
    name = (prog.findtext("ProgramName") or prog.findtext("Name") or "").strip()
    instruments = {}
    for inst in prog.iter("Instrument"):
        try:
            number = int(inst.get("number", ""))
        except ValueError:
            continue
        for layer in inst.iter("Layer"):
            if (layer.findtext("Active") or "True").strip().lower() == "false":
                continue
            sample = (layer.findtext("SampleName") or "").strip()
            file = (layer.findtext("SampleFile") or "").strip()
            if sample or file:
                instruments[number] = (sample, file)
                break
    padnotes = {}
    for pn in prog.iter("PadNote"):
        try:
            padnotes[int(pn.get("number", ""))] = int((pn.findtext("Note") or "").strip())
        except ValueError:
            continue
    return name, instruments, padnotes


def mappings(instruments, padnotes):
    """Two readings of which instrument a pad plays, pads 1 to 16: by pad number, and by the pad's note."""
    base = min(instruments) if instruments else 1
    base = 0 if base == 0 else 1
    by_pad = {p: instruments.get(p - 1 + base) for p in range(1, 17)}
    by_note = {p: instruments.get(padnotes.get(p, PAD_NOTES[p - 1])) for p in range(1, 17)}
    return by_pad, by_note


def resolve(sample, file, xpm_dir, files, by_name):
    """The content path of a sample, or None."""
    cands = []
    if file:
        f = file.replace("\\", "/")
        cands += [f, xpm_dir.rstrip("/") + "/" + f.lstrip("./"), "/" + f.lstrip("/")]
        for prefix in ("/media/acvs-content", "/media/az01-internal"):
            if f.startswith(prefix):
                cands.append(f[len(prefix):])
    if sample:
        cands += [xpm_dir.rstrip("/") + "/" + sample + ext for ext in (".wav", ".WAV")]
    for c in cands:
        if c in files:
            return c
    keys = []
    if file:
        keys.append(os.path.basename(file.replace("\\", "/")).lower())
    if sample:
        keys.append((sample + ".wav").lower())
    for k in keys:
        hits = by_name.get(k)
        if hits:
            same_dir = [h for h in hits if os.path.dirname(h) == xpm_dir]
            return (same_dir or sorted(hits))[0]
    return None


# ------------------------------------------------------------------ WAV to 44.1 kHz stereo 16-bit

def read_wav(data, max_seconds=MAX_SECONDS):
    """(rate, channels, bits, kind, [left floats], [right floats], cut): at most max_seconds (+1 frame) are decoded."""
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError("not a RIFF WAVE file")
    pos, fmt, pcm = 12, None, None
    while pos + 8 <= len(data):
        cid, size = data[pos:pos + 4], struct.unpack_from("<I", data, pos + 4)[0]
        body = data[pos + 8:pos + 8 + size]
        if cid == b"fmt ":
            fmt = body
        elif cid == b"data":
            pcm = body
        pos += 8 + size + (size & 1)
    if fmt is None or pcm is None or len(fmt) < 16:
        raise ValueError("no fmt or data chunk")
    tag, ch, rate, _, block, bits = struct.unpack_from("<HHIIHH", fmt, 0)
    if tag == 0xFFFE and len(fmt) >= 26:
        tag = struct.unpack_from("<H", fmt, 24)[0]
    if tag not in (1, 3) or ch < 1 or not 8000 <= rate <= 192000 or block != ch * bits // 8:
        raise ValueError(f"unsupported format (tag {tag}, {ch} channels, {rate} Hz, {bits} bits)")
    n = len(pcm) // block
    cut = n > max_seconds * rate + 1
    n = min(n, max_seconds * rate + 1)
    kind = "float" if tag == 3 else "int"
    if tag == 3 and bits == 32:
        a = array.array("f")
        a.frombytes(pcm[:n * block])
        flat = a
        scale = 1.0
    elif tag == 1 and bits == 16:
        a = array.array("h")
        a.frombytes(pcm[:n * block])
        flat, scale = a, 1 / 32768
    elif tag == 1 and bits == 32:
        a = array.array("i")
        a.frombytes(pcm[:n * block])
        flat, scale = a, 1 / 2147483648
    elif tag == 1 and bits == 24:
        flat = [int.from_bytes(pcm[i:i + 3], "little", signed=True) for i in range(0, n * block, 3)]
        scale = 1 / 8388608
    elif tag == 1 and bits == 8:
        flat = [b - 128 for b in pcm[:n * block]]
        scale = 1 / 128
    else:
        raise ValueError(f"unsupported sample size {bits} bits ({kind})")
    if sys.byteorder != "little" and isinstance(flat, array.array):
        flat.byteswap()
    left = [flat[i] * scale for i in range(0, n * ch, ch)]
    right = [flat[i + 1] * scale for i in range(0, n * ch, ch)] if ch > 1 else left
    return rate, ch, bits, kind, left, right, cut


def resample(x, rate):
    if rate == RATE or not x:
        return x
    n = int(len(x) * RATE / rate)
    step = rate / RATE
    out = []
    last = len(x) - 1
    for k in range(n):
        p = k * step
        i = int(p)
        if i >= last:
            out.append(x[last])
        else:
            f = p - i
            out.append(x[i] + (x[i + 1] - x[i]) * f)
    return out


def to_s16_stereo(left, right):
    a = array.array("h", bytes(4 * len(left)))
    for i, (l, r) in enumerate(zip(left, right)):
        a[2 * i] = max(-32768, min(32767, round(l * 32767)))
        a[2 * i + 1] = max(-32768, min(32767, round(r * 32767)))
    if sys.byteorder != "little":
        a.byteswap()
    return a.tobytes()


def utf8_cut(text, limit):
    """At most `limit` bytes of UTF-8, never splitting a character."""
    b = text.encode("utf-8")
    if len(b) <= limit:
        return b
    return b[:limit].decode("utf-8", "ignore").encode("utf-8")


def kit_bytes(sounds):
    out = bytearray(MAGIC)
    out += struct.pack("<I", len(sounds))
    for note, name, audio in sounds:
        nb = utf8_cut(name, 120)
        out += bytes([note, 0, 0, 0]) + struct.pack("<IH", len(audio) // 4, len(nb)) + nb + audio
    return bytes(out)


def kit_entry(name, kit):
    """One kit inside kits.bin: its name, its length, then the kit."""
    nb = utf8_cut(name, MAX_KIT_NAME)
    return struct.pack("<H", len(nb)) + nb + struct.pack("<I", len(kit)) + kit


def kits_bytes(entries):
    return KITS_MAGIC + struct.pack("<I", len(entries)) + b"".join(entries)


def family(path):
    """A program's name family: the file name before "-Kit", or its first word."""
    base = os.path.splitext(os.path.basename(path))[0]
    cut = re.split(r"[-_ ]kit", base, maxsplit=1, flags=re.IGNORECASE)[0]
    if cut != base and cut:
        return cut.lower()
    return re.split(r"[-_ ]", base, maxsplit=1)[0].lower()


def pick_extra(candidates, first, count):
    """Kits after the first: by path, at least MIN_EXTRA_PADS found, one per family."""
    seen = {family(first["path"])}
    out = []
    for c in candidates:
        if len(out) >= count:
            break
        if c["path"] == first["path"] or c["found"] < MIN_EXTRA_PADS:
            continue
        f = family(c["path"])
        if f in seen:
            continue
        seen.add(f)
        out.append(c)
    return out


def convert_kit(img, files, chosen, out_dir, say):
    """Copy out and convert one kit's samples; returns [(note, name, audio)]."""
    wanted = sorted({w for _, w in chosen["pads"].values() if w})
    sounds = []
    with tempfile.TemporaryDirectory(dir=out_dir) as tmp:
        got = dump(img, [w for w in wanted if files.get(w, 0) <= MAX_WAV], tmp)
        say("pad  note  sample")
        for pad in range(1, 17):
            ref, wav = chosen["pads"].get(pad, (None, None))
            note = PAD_NOTES[pad - 1]
            label = (ref[0] or os.path.basename(ref[1])) if ref else ""
            if not wav:
                say(f"  {pad:2d}  {note:3d}  -" + (f" ({label}: file not found)" if label else ""))
                continue
            if wav not in got:
                say(f"  {pad:2d}  {note:3d}  {label}: could not copy {wav}")
                continue
            with open(got[wav], "rb") as fh:
                data = fh.read(MAX_WAV + 1)
            try:
                rate, ch, bits, kind, left, right, cut = read_wav(data)
            except (ValueError, struct.error) as err:
                say(f"  {pad:2d}  {note:3d}  {label}: {wav}: {err}")
                continue
            left, right = resample(left, rate), resample(right, rate)
            left, right = left[:RATE * MAX_SECONDS], right[:RATE * MAX_SECONDS]
            if not left:
                say(f"  {pad:2d}  {note:3d}  {label}: {wav}: no audio")
                continue
            audio = to_s16_stereo(left, right)
            sounds.append((note, label or os.path.basename(wav), audio))
            say(f"  {pad:2d}  {note:3d}  {label}: {wav} ({rate} Hz, {ch} ch, {bits}-bit {kind}, "
                f"{len(left) / RATE:.2f} s{', cut to 10 s' if cut else ''})")
    return sounds


# ------------------------------------------------------------------ main

def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--backup", help="backup folder (default: the newest one)")
    p.add_argument("--program", help="kit 1: the first drum program whose path contains this text")
    p.add_argument("--kits", type=int, default=DEFAULT_KITS, help=f"how many kits, 1 to {MAX_KITS} (default {DEFAULT_KITS})")
    args = p.parse_args(argv)
    if not 1 <= args.kits <= MAX_KITS:
        raise Stop(f"--kits must be 1 to {MAX_KITS}")
    backups = os.path.realpath(os.environ.get("MPFREE_BACKUPS_DIR", os.path.expanduser("~/mpfree-backups")))
    work = os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    if work == repo or work.startswith(repo + os.sep):
        raise Stop("the work folder must be outside this clone")

    folder = os.path.realpath(args.backup) if args.backup else newest_backup(backups)
    image = os.path.join(folder, "mmcblk0.img")
    if not os.path.isfile(image):
        raise Stop(f"{image} does not exist")
    debugfs = find_debugfs()
    os.makedirs(work, exist_ok=True)
    out_dir = tempfile.mkdtemp(prefix=f"factory-kit-{utc()}-", dir=work)
    lines = []

    def say(text=""):
        print(text, flush=True)
        lines.append(text)

    try:
        say("MPFree: make drum kits from Akai's factory content in your eMMC backup (no MPC contact)")
        say(f"backup: {folder}")
        say(f"image: {image} ({os.path.getsize(image)} bytes, opened read-only)")
        parts = gpt_partitions(image)
        content = [x for x in parts if x[0] == "content"]
        say("partitions: " + ", ".join(f"{n} {a}-{b}" for n, a, b in parts))
        if not content:
            raise Stop("no partition named content in the GPT")
        _, first, last = content[0]
        say(f"content partition: sectors {first} to {last} ({(last - first + 1) * SECTOR >> 20} MiB)")
        img = Image(debugfs, image, first * SECTOR)
        info = img.stats()
        for k, v in info.items():
            say(f"  {k}: {v}")

        say("\nlisting the content file system ...")
        tree, skipped = walk(img)
        files = {pth: size for pth, (is_dir, size) in tree.items() if not is_dir}
        say(f"{len(tree) - 1} entries: {len(files)} files, {len(tree) - 1 - len(files)} folders"
            + (f", {skipped} folders skipped (odd characters in their names)" if skipped else ""))
        exts = {}
        for pth, size in files.items():
            e = os.path.splitext(pth)[1].lower() or "(none)"
            n, s = exts.get(e, (0, 0))
            exts[e] = (n + 1, s + size)
        say("files by type:")
        for e, (n, s) in sorted(exts.items(), key=lambda kv: -kv[1][1])[:25]:
            say(f"  {e:10s} {n:7d} files {s / 1048576:10.1f} MiB")
        say("top folders (two levels):")
        for pth in sorted(t for t, (d, _) in tree.items() if d and 1 <= t.count("/") <= 2 and t != "/")[:80]:
            say(f"  {pth}/")

        by_name = {}
        for pth in files:
            by_name.setdefault(os.path.basename(pth).lower(), []).append(pth)
        xpms = sorted(pth for pth, size in files.items() if pth.lower().endswith(".xpm") and 0 < size <= MAX_XPM)
        say(f"\n.xpm programs: {len(xpms)}")
        chosen = None
        drum_programs = []
        candidates = []
        if xpms:
            with tempfile.TemporaryDirectory(dir=out_dir) as tmp:
                got = dump(img, xpms, tmp)
                for pth in xpms:
                    if pth not in got:
                        continue
                    with open(got[pth], "rb") as fh:
                        text = fh.read(MAX_XPM)
                    parsed = program_info(text)
                    if parsed is None:
                        continue
                    name, instruments, padnotes = parsed
                    by_pad, by_note = mappings(instruments, padnotes)
                    xdir = os.path.dirname(pth)
                    found_pad = {p: resolve(*v, xdir, files, by_name) if v else None for p, v in by_pad.items()}
                    found_note = {p: resolve(*v, xdir, files, by_name) if v else None for p, v in by_note.items()}
                    n_pad = sum(1 for v in found_pad.values() if v)
                    n_note = sum(1 for v in found_note.values() if v)
                    drum_programs.append((pth, name, n_pad, n_note))
                    use_note = n_note > n_pad
                    entry = {
                        "path": pth, "name": name or os.path.splitext(os.path.basename(pth))[0],
                        "reading": "by note" if use_note else "by pad number",
                        "pads": {p: ((by_note if use_note else by_pad)[p], (found_note if use_note else found_pad)[p])
                                 for p in range(1, 17)},
                        "xml": "", "instruments": len(instruments), "padnotes": padnotes,
                        "found": max(n_pad, n_note),
                    }
                    candidates.append(entry)
                    if chosen is None and entry["found"] >= MIN_PADS and (not args.program or args.program in pth):
                        entry["xml"] = text.decode("utf-8", "replace")
                        chosen = entry
        say(f"drum programs: {len(drum_programs)} (pads 1-16 with samples found: by pad number / by note)")
        for pth, name, n_pad, n_note in drum_programs[:MAX_REPORTED_PROGRAMS]:
            say(f"  {n_pad:2d} / {n_note:2d}  {pth}  ({name})")
        if len(drum_programs) > MAX_REPORTED_PROGRAMS:
            say(f"  ... and {len(drum_programs) - MAX_REPORTED_PROGRAMS} more")

        if chosen is None:
            say("\nno drum program with enough samples" + (f" matching '{args.program}'" if args.program else "")
                + "; looking for a folder of drum WAVs instead")
            folders = {}
            for pth in files:
                if pth.lower().endswith(".wav") and any(w in os.path.basename(pth).lower() for w in DRUM_WORDS):
                    folders.setdefault(os.path.dirname(pth), []).append(pth)
            if not folders:
                raise Stop("no drum program and no folder of drum WAVs found; see the listing above")
            best = max(sorted(folders), key=lambda d: len(folders[d]))

            def rank(pth):
                b = os.path.basename(pth).lower()
                return (next((i for i, w in enumerate(DRUM_WORDS) if w in b), len(DRUM_WORDS)), b)
            wavs = sorted(folders[best], key=rank)[:16]
            chosen = {"path": best + "/", "name": os.path.basename(best), "reading": "loose WAVs",
                      "pads": {p: ((os.path.basename(w), ""), w) for p, w in enumerate(wavs, 1)},
                      "xml": "", "instruments": 0, "padnotes": {}, "found": len(wavs)}

        plan = [chosen] + pick_extra(candidates, chosen, args.kits - 1)
        say(f"\nkits: {len(plan)} (asked for {args.kits})")
        for k, c in enumerate(plan, 1):
            say(f"  {k:2d}. {c['path']} ({c['name']}), {c['found']} pads found, read {c['reading']}")
        entries, kit_lines = [], []
        total = len(KITS_MAGIC) + 4
        for k, c in enumerate(plan, 1):
            say(f"\nkit {k}: {c['path']} ({c['name']}), pads read {c['reading']}")
            sounds = convert_kit(img, files, c, out_dir, say)
            if not sounds:
                if k == 1:
                    raise Stop("none of kit 1's samples could be read")
                say(f"kit {k}: no sample could be read; left out")
                continue
            entry = kit_entry(c["name"], kit_bytes(sounds))
            if total + len(entry) > MAX_KIT:
                if k == 1:
                    raise Stop(f"kit 1 alone would be {total + len(entry)} bytes; at most {MAX_KIT} fit")
                say(f"kit {k}: {len(entry)} bytes would take the file over {MAX_KIT} bytes; left out")
                continue
            total += len(entry)
            entries.append(entry)
            kit_lines.append(f"  {len(entries):2d}. {c['name']}: {len(sounds)} sounds, {len(entry)} bytes")
        data = kits_bytes(entries)
        assert len(data) == total
        kit_path = os.path.join(out_dir, "kits.bin")
        with open(kit_path, "xb") as fh:
            fh.write(data)
        say(f"\nkits.bin: {kit_path}: {len(entries)} kits, {len(data)} bytes, sha256 {hashlib.sha256(data).hexdigest()}")
        for line in kit_lines:
            say(line)
        if chosen["xml"]:
            say(f"\nkit 1's program XML ({chosen['instruments']} instruments, "
                f"{len(chosen['padnotes'])} pad notes), first 400 lines:")
            for line in chosen["xml"].splitlines()[:400]:
                say("  | " + line)
        say("\nRESULT: kits made. The Hello MPC tool copies the newest kits.bin into the test build's RAM.")
        say("The kits and this report hold Akai's content: keep them private, never commit them.")
        return 0
    except Stop as err:
        say(f"\nSTOP: {err}")
        return 1
    finally:
        with open(os.path.join(out_dir, "report.txt"), "w") as fh:
            fh.write("\n".join(lines) + "\n")
        print(f"report: {os.path.join(out_dir, 'report.txt')}")


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except Stop as err:
        print(f"STOP: {err}")
        sys.exit(1)
