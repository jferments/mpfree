#!/usr/bin/env python3
"""WORKSTATION ONLY, STATE-CHANGING ON THE STICK ONLY: put one Open MPC plugin
release that CI already runs into files/plugins/ on the MPFree stick's
partition 4 (MPFREE-DATA), where MPFree finds plugins. It never contacts the
MPC, and writes nothing anywhere else.

  python3 tools/mpc-one/stick-plugin.py add mono-voice            # unzipped: MPFree runs it
  python3 tools/mpc-one/stick-plugin.py add tidal-verb --as-zip   # the zip as it is: BROWSE > Plugins installs it
  python3 tools/mpc-one/stick-plugin.py remove mono-voice         # the undo
  python3 tools/mpc-one/stick-plugin.py remove tidal-verb --as-zip

The id is one of tools/mpc-plugins/catalog-check.txt: MIT-licensed releases
from the Open MPC catalog that CI pins by SHA-256. The release is downloaded
once into ~/mpfree-work/plugins/ and checked against that sum before any use;
it is never committed or shipped with MPFree.

How it finds the stick: the one partition named mpfree-test-data (what
offline-kexec-stick.py and write-test-stick.py make) with an ext4 labelled
MPFREE-DATA, on a removable USB disk. No other partition qualifies, and it
stops if there are none or more than one. If the desktop has not mounted it,
it mounts it with udisksctl, as the file manager would.

What it writes, with sudo (partition 4's files belong to root): the folder
files/plugins/ if missing, then the release's own top folder unzipped into it
(Python's zipfile, which keeps every member inside that folder), or with
--as-zip the zip file itself. It refuses a release whose members are not all
under one top folder, and changes nothing when that folder or zip is already
there. Then it syncs, checks what landed, and unmounts the partition so the
stick can go into the MPC.

Undo: `remove` deletes exactly that top folder (or the zip) and nothing else;
files/plugins/ stays, empty or not. If it is cut off halfway, run the same
`add` again after `remove`.

MPFREE_WORK_DIR and MPFREE_CATALOG change the folders (tests use them).
"""
import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.request
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DATA_NAME = "mpfree-test-data"   # partition 4's GPT name (offline-kexec-stick.py)
DATA_LABEL = "MPFREE-DATA"       # its ext4 label
PLUGINS = "files/plugins"        # where MPFree looks, on partition 4
MAX_ZIP = 64 << 20
ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
SHA = re.compile(r"^[0-9a-f]{64}$")


class Stop(Exception):
    pass


def catalog_path():
    return os.environ.get("MPFREE_CATALOG", os.path.join(REPO, "tools", "mpc-plugins", "catalog-check.txt"))


def work_dir():
    work = os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    if work == REPO or work.startswith(REPO + os.sep):
        raise Stop("the work folder must be outside this clone")
    return work


def release(pid):
    """(kind, sha256, url) for a catalog id."""
    with open(catalog_path()) as fh:
        for line in fh:
            f = line.split()
            if len(f) == 4 and not f[0].startswith("#") and f[0] == pid:
                kind, sha, url = f[1], f[2].lower(), f[3]
                if not SHA.match(sha) or not url.startswith("https://") or not url.endswith(".zip"):
                    raise Stop(f"{pid}: its line in {catalog_path()} is not id, kind, sha256, https url of a .zip")
                return kind, sha, url
    raise Stop(f"{pid} is not in {catalog_path()}; the ids there are the releases CI runs")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def fetch(sha, url):
    """The release zip in ~/mpfree-work/plugins, downloaded if needed, always checked against `sha`."""
    folder = os.path.join(work_dir(), "plugins")
    os.makedirs(folder, mode=0o700, exist_ok=True)
    path = os.path.join(folder, os.path.basename(url))
    if os.path.isfile(path) and sha256(path) == sha:
        print(f"release: {path} (already here, SHA-256 matches)")
        return path
    part = path + ".part"
    print(f"release: downloading {url}")
    try:
        with urllib.request.urlopen(url, timeout=60) as r, open(part, "wb") as out:
            got = 0
            while True:
                chunk = r.read(1 << 16)
                if not chunk:
                    break
                got += len(chunk)
                if got > MAX_ZIP:
                    raise Stop(f"the download is bigger than {MAX_ZIP >> 20} MiB; a plugin release is not")
                out.write(chunk)
    except OSError as err:
        if os.path.exists(part):
            os.unlink(part)
        raise Stop(f"the download failed: {err}")
    if sha256(part) != sha:
        os.unlink(part)
        raise Stop("the download's SHA-256 is not the one CI pins; nothing written")
    os.replace(part, path)
    print(f"release: {path} (SHA-256 matches the catalog's)")
    return path


def top_folder(zpath):
    """The one top folder every member of the release sits under, and how many files it holds."""
    with zipfile.ZipFile(zpath) as z:
        names = z.namelist()
    tops, files = set(), 0
    for n in names:
        parts = n.replace("\\", "/").split("/")
        if n.startswith(("/", "\\")) or ".." in parts or ":" in parts[0] or len(parts) < 2 or not parts[0]:
            raise Stop(f"the release has a member outside one top folder: {n!r}; nothing written")
        tops.add(parts[0])
        if not n.endswith("/"):
            files += 1
    if len(tops) != 1:
        raise Stop(f"the release has {len(tops)} top folders, not one; nothing written")
    return tops.pop(), files


def lsblk():
    try:
        res = subprocess.run(["lsblk", "-J", "-o", "NAME,PATH,TYPE,PARTLABEL,LABEL,FSTYPE,TRAN,RM,MOUNTPOINT"],
                             capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired) as err:
        raise Stop(f"lsblk did not run: {err}")
    if res.returncode != 0:
        raise Stop(f"lsblk failed: {res.stderr.strip()}")
    return json.loads(res.stdout).get("blockdevices", [])


def removable(v):
    return v in (True, 1, "1", "true")


def find_partition():
    """(device path, mount point or None) of the one MPFree data partition on a removable USB disk."""
    found, near = [], []
    for disk in lsblk():
        for part in disk.get("children") or []:
            if part.get("partlabel") != DATA_NAME and part.get("label") != DATA_LABEL:
                continue
            why = []
            if part.get("partlabel") != DATA_NAME:
                why.append(f"partition name {part.get('partlabel')!r}, not {DATA_NAME}")
            if part.get("label") != DATA_LABEL or part.get("fstype") != "ext4":
                why.append(f"filesystem {part.get('fstype')!r} labelled {part.get('label')!r}, not ext4 {DATA_LABEL}")
            if disk.get("tran") != "usb" or not removable(disk.get("rm")):
                why.append(f"disk {disk.get('path')} is not a removable USB disk")
            (near if why else found).append((part.get("path"), part.get("mountpoint"), "; ".join(why)))
    for path, _, why in near:
        print(f"not the stick: {path}: {why}")
    if not found:
        raise Stop(f"no MPFree stick: no {DATA_NAME} partition with an ext4 {DATA_LABEL} on a removable USB disk. "
                   "Put the stick into the workstation and run this again")
    if len(found) > 1:
        raise Stop("more than one MPFree stick: " + ", ".join(f[0] for f in found) + "; leave only one in")
    return found[0][0], found[0][1]


def run(cmd, what):
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    except (OSError, subprocess.TimeoutExpired) as err:
        raise Stop(f"{what} did not run: {err}")
    if res.returncode != 0:
        raise Stop(f"{what} failed: {(res.stderr or res.stdout).strip()}")
    return res.stdout


def sudo(cmd, what):
    """Run cmd with sudo, its password prompt on this terminal."""
    try:
        rc = subprocess.call(["sudo", *cmd], timeout=300)
    except (OSError, subprocess.TimeoutExpired) as err:
        raise Stop(f"{what} did not run: {err}")
    if rc != 0:
        raise Stop(f"{what} failed (exit code {rc})")


def sudo_out(cmd, what):
    """cmd's output, run with sudo: partition 4's folders may be readable by root only."""
    try:
        res = subprocess.run(["sudo", *cmd], stdout=subprocess.PIPE, text=True, timeout=300)
    except (OSError, subprocess.TimeoutExpired) as err:
        raise Stop(f"{what} did not run: {err}")
    return res.returncode, res.stdout


def there(path):
    rc, _ = sudo_out(["sh", "-c", 'test -e "$1" || test -L "$1"', "sh", path], "looking on the stick")
    return rc == 0


def mount(dev, where):
    if where:
        print(f"partition 4: {dev}, mounted at {where} (by the desktop)")
        return where
    out = run(["udisksctl", "mount", "-b", dev], "udisksctl mount")
    m = re.search(r" at (/\S.*?)\.?\s*$", out.strip())
    if not m or not os.path.isdir(m.group(1)):
        raise Stop(f"udisksctl did not say where it mounted {dev}: {out.strip()}")
    print(f"partition 4: {dev}, mounted at {m.group(1)} (udisksctl, as the file manager does)")
    return m.group(1)


def unmount(dev):
    run(["sync"], "sync")
    run(["udisksctl", "unmount", "-b", dev], "udisksctl unmount")
    print("partition 4: unmounted; the stick can come out")


def add(pid, as_zip):
    kind, sha, url = release(pid)
    zpath = fetch(sha, url)
    top, files = top_folder(zpath)
    dev, where = find_partition()
    root = mount(dev, where)
    plugins = os.path.join(root, PLUGINS)
    target = os.path.join(plugins, os.path.basename(zpath) if as_zip else top)
    shown = f"{PLUGINS}/{os.path.basename(target)}"
    if there(target):
        print(f"{shown}: already on the stick; nothing written")
    else:
        if not there(plugins):
            sudo(["mkdir", "-p", "--", plugins], f"making {PLUGINS}")
            print(f"made {PLUGINS}/ on partition 4")
        if as_zip:
            sudo(["cp", "--", zpath, target], f"copying the zip to {shown}")
        else:
            sudo([sys.executable, "-I", "-m", "zipfile", "-e", zpath, plugins], f"unzipping into {PLUGINS}")
        run(["sync"], "sync")
    if as_zip:
        rc, out = sudo_out(["sha256sum", "--", target], "checking the zip")
        ok = rc == 0 and out.split()[:1] == [sha]
        detail = "SHA-256 matches" if ok else "its SHA-256 does not match the release's"
    else:
        rc, out = sudo_out(["find", target, "-type", "f", "-printf", "."], "counting the files")
        landed = len(out) if rc == 0 else 0
        ok = landed == files
        detail = f"{landed} of the release's {files} files"
    print(f"{shown}: {detail}")
    unmount(dev)
    if not ok:
        raise Stop(f"{shown} is not the whole release; run remove {pid}{' --as-zip' if as_zip else ''}, then add again")
    print(f"PLUGIN-OK: {pid} ({kind}) is in {shown} on the stick's partition 4")
    return 0


def remove(pid, as_zip):
    _, sha, url = release(pid)
    zpath = fetch(sha, url)
    top, _ = top_folder(zpath)
    dev, where = find_partition()
    root = mount(dev, where)
    target = os.path.join(root, PLUGINS, os.path.basename(zpath) if as_zip else top)
    shown = f"{PLUGINS}/{os.path.basename(target)}"
    if not there(target):
        print(f"{shown}: not on the stick; nothing to remove")
    else:
        sudo(["rm", "-r", "--", target], f"removing {shown}")
        print(f"removed {shown}")
    gone = not there(target)
    unmount(dev)
    if not gone:
        raise Stop(f"{shown} is still there")
    print(f"REMOVE-OK: {pid} is not in {PLUGINS} on the stick's partition 4")
    return 0


def main(argv):
    args = [a for a in argv if a != "--as-zip"]
    as_zip = len(args) != len(argv)
    if len(args) != 2 or args[0] not in ("add", "remove") or not ID.match(args[1]):
        print(__doc__.split("\n\n")[1])
        return 2
    try:
        return (add if args[0] == "add" else remove)(args[1], as_zip)
    except Stop as err:
        print(f"STOP: {err}")
        return 1


if __name__ == "__main__":
    os.umask(0o022)
    sys.exit(main(sys.argv[1:]))
