#!/usr/bin/env python3
"""WORKSTATION ONLY: list and copy out your own MPC projects from your eMMC
backup, for the MPC project importer. It never contacts the MPC.

  python3 tools/mpc-one/offline-mpc-projects.py                  # list only
  python3 tools/mpc-one/offline-mpc-projects.py --copy 1,3       # copy projects 1 and 3
  python3 tools/mpc-one/offline-mpc-projects.py --copy all       # copy every project
  (--backup FOLDER picks a backup; the default is the newest one)

1. Finds the newest backup folder ~/mpfree-backups/mpc-one-emmc-*/ that has
   mmcblk0.img (made by `collect-over-ssh.sh --backup`), unless --backup names one.
2. Reads the GPT and finds the partition named "data" (p8: /data and the MPC's
   "Internal" drive, /media/az01-internal).
3. Lists that ext4 file system with debugfs, which opens the image read-only
   (no -w) at the partition's offset. Nothing is mounted and no sudo is needed.
4. Lists every project: each NAME.xpj with its NAME_[ProjectData] folder, and
   each .xpa archive. For each .xpj it reads only the first bytes to say which
   format it is (MPC 3 gzip with its 5 header lines, or MPC 2 XML).
5. With --copy, copies only the chosen projects' .xpj (or .xpa) and their
   ProjectData folders into a new folder ~/mpfree-work/mpc-projects-<UTC>-XXXXXXXX/,
   checks every file's size against the listing, writes SHA256SUMS and
   mpc-projects.zip (the file to upload to the project thread).

Nothing else on the partition is copied: not /etc, /var, SSH keys or settings.
report.txt holds paths, sizes and header lines only, no audio.
Projects can hold Akai factory or expansion sounds: keep the copies private and
never commit them.

MPFREE_BACKUPS_DIR and MPFREE_WORK_DIR change the folders (tests use them).
"""
import argparse
import gzip
import hashlib
import importlib.util
import io
import os
import re
import shutil
import sys
import tempfile
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("offline_factory_kit", os.path.join(HERE, "offline-factory-kit.py"))
fk = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fk)
Stop = fk.Stop
SECTOR = fk.SECTOR

DATA_PART = "data"
MAX_HEAD = 64 << 10
PD_SUFFIX = "_[ProjectData]"


def projects(tree):
    """[(kind, path, folder or None, files {path: size})], sorted by path."""
    files = {p: s for p, (d, s) in tree.items() if not d}
    out = []
    for p in sorted(files):
        low = p.lower()
        if low.endswith(".xpj"):
            folder = p[:-4] + PD_SUFFIX
            if not tree.get(folder, (False, 0))[0]:
                folder = None
            own = {p: files[p]}
            if folder:
                own.update({f: s for f, s in files.items() if f.startswith(folder + "/")})
            out.append(("project", p, folder, own))
        elif low.endswith(".xpa"):
            out.append(("archive", p, None, {p: files[p]}))
    return out


def printable(raw):
    return "".join(c if 32 <= ord(c) < 127 else "?" for c in raw)[:100]


def header(data):
    """What format a .xpj's first bytes say it is, as report lines."""
    if data[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=io.BytesIO(data)) as gz:
                text = gz.read(4096)
        except (OSError, EOFError) as err:
            return [f"gzip, but its start would not unpack ({err})"]
        lines = text.decode("utf-8", "replace").split("\n")[:5]
        return ["gzip (MPC 3 style); first lines: " + " | ".join(printable(x) for x in lines)]
    if data.lstrip()[:5] == b"<?xml":
        text = data.decode("utf-8", "replace")
        tags = re.findall(r"<(File_Version|Application|Application_Version|Platform)>([^<]{0,60})</", text)
        return ["XML (MPC 2 style)" + ("; " + ", ".join(f"{k} {printable(v)}" for k, v in tags[:6]) if tags else "")]
    return ["unknown format; first bytes " + data[:16].hex(" ")]


def pick(arg, count):
    if arg == "all":
        return list(range(1, count + 1))
    try:
        nums = sorted({int(x) for x in arg.split(",") if x.strip()})
    except ValueError:
        raise Stop(f"--copy takes numbers from the list, like 1,3, or all (got {arg!r})")
    bad = [n for n in nums if not 1 <= n <= count]
    if bad or not nums:
        raise Stop(f"--copy: no project numbered {bad or arg} (the list has 1 to {count})")
    return nums


def safe_rel(path):
    """A path on the partition as a relative local path, or None if it could escape."""
    parts = path.strip("/").split("/")
    if not parts or any(p in ("", ".", "..") for p in parts):
        return None
    return os.path.join(*parts)


def copy_out(img, chosen, out_dir, say):
    """Copy the chosen projects file by file with debugfs dump; returns [(rel, size)]."""
    wanted = []
    for _, _, _, own in chosen:
        for p, size in sorted(own.items()):
            rel = safe_rel(p)
            if rel is None or fk.quoted(p) is None:
                raise Stop(f"a path has characters this tool will not copy: {printable(p)}")
            wanted.append((p, rel, size))
    dest_root = os.path.join(out_dir, "projects")
    cmds = []
    for p, rel, _ in wanted:
        dest = os.path.join(dest_root, rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        if '"' in dest or "\\" in dest:
            raise Stop(f"the work folder path {dest_root} has a character debugfs cannot take")
        cmds.append(f'dump {fk.quoted(p)} "{dest}"')
    for i in range(0, len(cmds), 500):
        img.batch(cmds[i:i + 500], timeout=3600)
    done = []
    for p, rel, size in wanted:
        dest = os.path.join(dest_root, rel)
        got = os.path.getsize(dest) if os.path.isfile(dest) else -1
        if got != size:
            raise Stop(f"{p}: copied {got} bytes, the listing says {size}")
        done.append((rel, size))
    say(f"copied {len(done)} files, {sum(s for _, s in done) / 1048576:.1f} MiB; every size matches the listing")
    return dest_root, done


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--backup", help="backup folder (default: the newest one)")
    p.add_argument("--copy", help="projects to copy, by their number in the list (1,3) or all")
    args = p.parse_args(argv)
    backups = os.path.realpath(os.environ.get("MPFREE_BACKUPS_DIR", os.path.expanduser("~/mpfree-backups")))
    work = os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    repo = os.path.dirname(os.path.dirname(HERE))
    if work == repo or work.startswith(repo + os.sep):
        raise Stop("the work folder must be outside this clone")

    folder = os.path.realpath(args.backup) if args.backup else fk.newest_backup(backups)
    image = os.path.join(folder, "mmcblk0.img")
    if not os.path.isfile(image):
        raise Stop(f"{image} does not exist")
    debugfs = fk.find_debugfs()
    os.makedirs(work, exist_ok=True)
    out_dir = tempfile.mkdtemp(prefix=f"mpc-projects-{fk.utc()}-", dir=work)
    lines = []

    def say(text=""):
        print(text, flush=True)
        lines.append(text)

    try:
        say("MPFree: list and copy your MPC projects from your eMMC backup (no MPC contact)")
        say(f"backup: {folder}")
        say(f"image: {image} ({os.path.getsize(image)} bytes, opened read-only)")
        parts = fk.gpt_partitions(image)
        data = [x for x in parts if x[0] == DATA_PART]
        if not data:
            raise Stop(f"no partition named {DATA_PART} in the GPT ({', '.join(n for n, _, _ in parts)})")
        _, first, last = data[0]
        say(f"{DATA_PART} partition: sectors {first} to {last} ({(last - first + 1) * SECTOR >> 20} MiB)")
        img = fk.Image(debugfs, image, first * SECTOR)
        img.stats()

        say("\nlisting the data file system ...")
        tree, skipped = fk.walk(img)
        found = projects(tree)
        say(f"{len(tree) - 1} entries" + (f", {skipped} folders skipped (odd characters in their names)" if skipped else ""))
        if not found:
            raise Stop("no .xpj project or .xpa archive on the data partition (projects saved to an SD card "
                       "or USB drive are not in the eMMC backup)")

        heads = {}
        xpjs = [pth for kind, pth, _, _ in found if kind == "project"]
        with tempfile.TemporaryDirectory(dir=out_dir) as tmp:
            got = fk.dump(img, xpjs, tmp)
            for pth, local in got.items():
                with open(local, "rb") as fh:
                    heads[pth] = header(fh.read(MAX_HEAD))

        say(f"\nprojects: {len(found)}")
        for n, (kind, pth, pd, own) in enumerate(found, 1):
            size = sum(own.values()) / 1048576
            shown = pth[len("/"):]
            if kind == "archive":
                say(f"{n:3d}. {shown}  (.xpa archive, {size:.1f} MiB)")
                continue
            wavs = sum(1 for f in own if f.lower().endswith(".wav"))
            say(f"{n:3d}. {shown}  ({size:.1f} MiB, {len(own) - 1} files in ProjectData, {wavs} WAV)"
                + ("" if pd else "  NO ProjectData folder"))
            for h in heads.get(pth, ["could not read its first bytes"]):
                say(f"       {h}")

        if not args.copy:
            say("\nRESULT: listed only. Copy some with --copy 1,3 (numbers above) or --copy all.")
            return 0

        chosen = [found[n - 1] for n in pick(args.copy, len(found))]
        total = sum(sum(own.values()) for _, _, _, own in chosen)
        free = shutil.disk_usage(out_dir).free
        if free < total * 2 + (64 << 20):
            raise Stop(f"not enough free space in {work}: need about {total * 2 >> 20} MiB, have {free >> 20} MiB")
        say(f"\ncopying {len(chosen)} project(s), {total / 1048576:.1f} MiB ...")
        dest_root, done = copy_out(img, chosen, out_dir, say)

        sums = []
        zpath = os.path.join(out_dir, "mpc-projects.zip")
        with zipfile.ZipFile(zpath, "w") as z:
            for rel, _ in done:
                local = os.path.join(dest_root, rel)
                h = hashlib.sha256()
                with open(local, "rb") as fh:
                    for chunk in iter(lambda: fh.read(1 << 20), b""):
                        h.update(chunk)
                sums.append(f"{h.hexdigest()}  {rel}")
                stored = rel.lower().endswith((".wav", ".xpj", ".xpa"))
                z.write(local, rel, compress_type=zipfile.ZIP_STORED if stored else zipfile.ZIP_DEFLATED)
        with open(os.path.join(out_dir, "SHA256SUMS"), "w") as fh:
            fh.write("\n".join(sums) + "\n")
        with zipfile.ZipFile(zpath) as z:
            bad = z.testzip()
        if bad:
            raise Stop(f"the zip failed its check at {bad}")
        say(f"zip: {zpath} ({os.path.getsize(zpath) / 1048576:.1f} MiB, checked)")
        say("\nRESULT: copied. Upload mpc-projects.zip to the project thread (or a few at a time if it is large).")
        say("These are your private files: never commit them.")
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
