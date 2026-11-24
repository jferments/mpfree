#!/usr/bin/env python3
"""WORKSTATION ONLY: check the MPC's SD card in this computer's card reader, and repair it when asked. It is
for the card that came out of the MPC's slot, before MPFree saves to it (step S2 of
docs/dev/hardware/sd-card-and-internal-drive.md): MPFree writes only to a card that checks clean and whose
"not safely removed" mark is clear, and only a check that repairs on a computer clears that mark.

  python3 tools/mpc-one/repair-sd-card.py            check only: nothing on the card changes
  python3 tools/mpc-one/repair-sd-card.py --repair   check, then repair after you type REPAIR
  python3 tools/mpc-one/repair-sd-card.py --repair --yes
                                              check, then repair if needed without asking (the SD card
                                              round's first command passes it when you ask it to repair)

Exit codes: 0 clean (or repaired clean), 3 needs a repair (check only), 5 no card found, 1 anything else.

It never contacts the MPC. It never asks you to type a device name: it looks for the one card that is
  - a whole disk in a card reader: on USB and marked removable (sdX), or an SD card in a built-in reader
    (mmcblkN whose type is SD), not read-only,
  - between 1 GB and 2 TiB,
  - one exFAT filesystem: one partition, or the whole disk without a table, that udev reads as exfat,
  - not the MPFree stick, with nothing stacked on it (LVM, RAID, dm-crypt) and no swap on it,
  - with nothing of it mounted, except by the desktop under /run/media/YOU or /media/YOU,
and it stops if there is no such card or more than one. It lists every USB or removable disk it sees, and for
the card it shows where it is plugged in, its reader's make and model, its size and its filesystem.

The check (both ways): the desktop's mount is taken away first (udisksctl unmount, as you), so the card is
checked at rest. Then, under sudo (it asks for your password): the disk again (the same size, nothing
mounted), the filesystem's first sector (it must say EXFAT), its VolumeFlags (byte 106: bit 1 is the mark a
system leaves while it has the card in use, bit 2 says a disk error was seen), and exfatprogs' fsck.exfat -n,
which opens the card read-only and changes nothing. A card that needs a repair is mounted again for the
desktop, so you can copy files off it; a clean one is left unmounted, ready to take out.

The repair (--repair only, after the check, and only if you type REPAIR): fsck.exfat -y, which opens the card
exclusively (refused while anything has it mounted), sets the mark while it works, mends what it found, and
clears the mark at the end. Then the same check again: READY when it is clean and the mark is clear, and the
card is left unmounted so it can go straight back into the MPC. (A computer that mounts the card and writes to
it sets the mark again until it is ejected, so if you open it here first, eject it before taking it out.)

Undo: none. A repair changes only what the check found wrong, and the mark, but that can't be put back: copy
anything on the card you can't lose to this computer first. If the repair stops part way, the card stays
marked and MPFree keeps reading it read-only: run --repair again.
"""
import os
import re
import shutil
import stat
import subprocess
import sys

# Tests point this at a fake /sys, /dev, /proc and /run. In real use it is empty.
TEST_ROOT = os.environ.get("MPFREE_TEST_ROOT", "")
NAME = re.compile(r"^(sd[a-z]{1,3}|mmcblk[0-9]{1,2})$")
SECTOR = 512
MIN_SIZE = (1 << 20) * 16 if TEST_ROOT else 10 ** 9
MAX_SIZE = 2 << 40
CONFIRM = "REPAIR"
FLAGS = 106          # exFAT's VolumeFlags, in the filesystem's first sector
IN_USE = 0x02        # VolumeDirty: a system had it mounted and wrote to it, and didn't unmount it
DISK_ERROR = 0x04    # MediaFailure
# The privileged steps run only programs from the system's own folders (in tests, the fake ones first).
ROOT_PATH = os.environ.get("PATH", "") if TEST_ROOT else "/usr/sbin:/usr/bin:/sbin:/bin"


class Stop(Exception):
    pass


class NoCard(Stop):
    """No card in this computer: exit code 5, so the SD card round can tell it from a failure."""


def p(path):
    return TEST_ROOT + path


def read(path, default=""):
    try:
        with open(p(path)) as fh:
            return fh.read().strip()
    except OSError:
        return default


def unescape(field):
    """mountinfo writes a space as \\040 (and tab, newline, backslash likewise)."""
    return re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), field)


def udev_props(dev):
    """udev's record of the block device with numbers `dev` (ID_FS_TYPE, ID_FS_LABEL, ...), read without root."""
    props = {}
    for line in read(f"/run/udev/data/b{dev}").splitlines() if dev else []:
        if line.startswith("E:") and "=" in line:
            key, _, value = line[2:].partition("=")
            props[key] = value
    return props


def mounts():
    """Everything mounted or used as swap: [(major:minor, resolved source, mount point, type)]."""
    found = []
    with open(p("/proc/self/mountinfo")) as fh:
        for line in fh:
            left, _, right = line.partition(" - ")
            left, right = left.split(), right.split()
            if len(left) < 5 or len(right) < 2:
                continue
            source = os.path.realpath(p(right[1])) if right[1].startswith("/dev/") else right[1]
            found.append((left[2], source, unescape(left[4]), right[0]))
    with open(p("/proc/swaps")) as fh:
        for line in fh.readlines()[1:]:
            if line.split():
                found.append(("", os.path.realpath(p(line.split()[0])), "[swap]", "swap"))
    return found


def size_text(n):
    return f"{n / 1e9:.1f} GB ({n} bytes)"


def location(name):
    """Where the disk is plugged in: its USB port (bus-port path) and speed, or the built-in reader."""
    real = os.path.realpath(p(f"/sys/block/{name}"))
    ports = [part for part in real.split("/") if re.match(r"^\d+-\d+(\.\d+)*$", part)]
    if ports:
        port = ports[-1]
        speed = read(f"/sys/bus/usb/devices/{port}/speed")
        return f"USB port {port}" + (f" ({speed} Mbit/s)" if speed else "")
    return "a built-in card reader" if name.startswith("mmcblk") else "not on USB"


def user_media(mount_point, user):
    return any(mount_point.startswith(f"{base}/{user}/") for base in ("/run/media", "/media"))


def partname(node):
    for line in read(f"/sys/class/block/{node}/uevent").splitlines():
        if line.startswith("PARTNAME="):
            return line.split("=", 1)[1]
    return ""


def describe(name, user):
    """Facts about /sys/block/<name>, its partitions and their mounts, and every reason it isn't the card."""
    base = f"/sys/block/{name}"
    sectors = read(base + "/size", "0")
    mmc = name.startswith("mmcblk")
    info = {
        "name": name,
        "dev": read(base + "/dev"),
        "size": int(sectors) * SECTOR if sectors.isdigit() else 0,
        "removable": read(base + "/removable"),
        "usb": "/usb" in os.path.realpath(p(base)),
        "sd": mmc and read(base + "/device/type") == "SD",
        "vendor": read(base + "/device/vendor", "") or ("SD card" if mmc else "?"),
        "model": read(base + "/device/model", "") or read(base + "/device/name", "?"),
        "ro": read(base + "/ro"),
    }
    info["where"] = location(name)
    folder = p(base)
    parts = sorted((d for d in os.listdir(folder) if d.startswith(name) and os.path.isfile(p(f"{base}/{d}/partition"))),
                   key=lambda d: int(re.search(r"(\d+)$", d).group(1))) if os.path.isdir(folder) else []
    seen = mounts()
    reasons, info["parts"], info["desktop"] = [], [], []
    if not NAME.match(name):
        reasons.append("not a card reader's disk name")
    if mmc and not info["sd"]:
        reasons.append("not an SD card (an eMMC or another MMC device)")
    if not mmc and not info["usb"]:
        reasons.append("not on USB")
    if not mmc and info["removable"] != "1":
        reasons.append("not marked removable (a card reader's disk is)")
    if info["ro"] != "0":
        reasons.append("read-only (the card's lock switch may be on)")
    if os.path.isfile(p(base + "/partition")):
        reasons.append("a partition, not a whole disk")
    if info["size"] < MIN_SIZE:
        reasons.append("smaller than 1 GB" if info["size"] else "no card in it")
    if info["size"] > MAX_SIZE:
        reasons.append("larger than 2 TiB, so not an SD card")
    for node in [name] + parts:
        dev = read(f"/sys/class/block/{node}/dev")
        props = udev_props(dev)
        here = [(mp, kind) for number, source, mp, kind in seen
                if (dev and number == dev) or source == os.path.realpath(p("/dev/" + node))]
        if node != name or props.get("ID_FS_TYPE"):
            part_sectors = read(f"/sys/class/block/{node}/size", "0")
            info["parts"].append({
                "node": node,
                "dev": dev,
                "size": int(part_sectors) * SECTOR if part_sectors.isdigit() else 0,
                "fs": props.get("ID_FS_TYPE", ""),
                "label": props.get("ID_FS_LABEL", ""),
                "uuid": props.get("ID_FS_UUID", ""),
                "mounted": [mp for mp, _ in here],
            })
        for mp, kind in here:
            if kind != "swap" and user_media(mp, user):
                info["desktop"].append(node)
            else:
                reasons.append(f"/dev/{node} is in use at {mp}" + (" (swap)" if kind == "swap" else ""))
        if os.path.isdir(p(f"/sys/class/block/{node}/holders")) and os.listdir(p(f"/sys/class/block/{node}/holders")):
            reasons.append(f"/dev/{node} is in use by LVM, RAID or dm-crypt")
        if (props.get("ID_PART_ENTRY_NAME", "") or partname(node)).startswith("mpfree-test") or \
                props.get("ID_FS_LABEL", "") == "MPFREE-DATA":
            reasons.append("the MPFree stick")
    filesystems = [part for part in info["parts"] if part["fs"]]
    if len(info["parts"]) > 1:
        reasons.append(f"{len(info['parts'])} partitions (the MPC's card has one)")
    elif not filesystems:
        reasons.append("no filesystem")
    elif filesystems[0]["fs"] != "exfat":
        reasons.append(f"its filesystem is {filesystems[0]['fs']}, not exFAT")
    info["fs_node"] = filesystems[0]["node"] if len(info["parts"]) == 1 and filesystems else ""
    info["desktop"] = sorted(set(info["desktop"]))
    return info, sorted(set(reasons), key=reasons.index)


def show(info, reasons, full, verdict=True):
    print(f"  /dev/{info['name']}: {info['vendor']} {info['model']}, {size_text(info['size'])}")
    if full:
        print(f"      plugged in at: {info['where']}")
        for part in info["parts"]:
            what = part["fs"] or "no filesystem"
            if part["label"]:
                what += f", label {part['label']}"
            if part["uuid"]:
                what += f", UUID {part['uuid']}"
            where = "mounted at " + ", ".join(part["mounted"]) if part["mounted"] else "not mounted"
            print(f"      /dev/{part['node']}: {what}, {size_text(part['size'])}, {where}")
    if verdict:
        print("      " + ("the card" if not reasons else "not the card: " + "; ".join(reasons)))


def find_card(user):
    print("USB, removable and SD card disks seen:")
    names = sorted(os.listdir(p("/sys/block"))) if os.path.isdir(p("/sys/block")) else []
    candidates, seen = [], 0
    for name in names:
        if not NAME.match(name):
            continue
        info, reasons = describe(name, user)
        if not (info["usb"] or info["removable"] == "1" or name.startswith("mmcblk")):
            continue
        seen += 1
        show(info, reasons, full=False)
        if not reasons:
            candidates.append(info)
    if not seen:
        print("  (none)")
    if not candidates:
        raise NoCard("no exFAT SD card found: put the card from the MPC in this computer's card reader (with its lock "
                   "switch off), then run this again. Nothing was changed")
    if len(candidates) > 1:
        raise Stop("more than one disk could be the card: take out every other card and USB disk with exFAT on it, "
                   "then run this again. Nothing was changed")
    return candidates[0]


def ask(prompt):
    if not TEST_ROOT:
        if not sys.stdin.isatty():
            raise Stop("run this from a terminal: it asks you to confirm")
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)  # drop anything pasted after the command
    try:
        return input(prompt).strip()
    except EOFError:
        return ""


def program(name):
    found = shutil.which(name, path=ROOT_PATH)
    if not found:
        raise Stop(f"{name} is not installed (it comes with " +
                   {"fsck.exfat": "exfatprogs", "udisksctl": "udisks2"}.get(name, "util-linux") +
                   "); nothing was changed")
    return found


def udisks(verb, node):
    """udisksctl mount or unmount of /dev/<node> as you, like the file manager, without a password prompt."""
    res = subprocess.run([program("udisksctl"), verb, "--no-user-interaction", "-b", f"/dev/{node}"],
                         stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                         timeout=60)
    return res.returncode == 0, res.stdout.strip()


def exfatprogs_version(fsck):
    """fsck.exfat's version, if it is exfatprogs' (exfat-utils' fsck takes other options and is refused)."""
    res = subprocess.run([fsck, "-V"], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                         text=True, timeout=30)
    found = re.search(r"exfatprogs version\s*:\s*(\S+)", res.stdout)
    if not found:
        raise Stop(f"{fsck} is not exfatprogs' fsck.exfat (install exfatprogs); nothing was changed")
    return found.group(1)


def volume_flags(path):
    """exFAT's VolumeFlags from the filesystem's first sector, read without writing; Stop if it isn't exFAT."""
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_CLOEXEC", 0))
    try:
        sector = os.read(fd, SECTOR)
    finally:
        os.close(fd)
    if len(sector) < SECTOR or sector[3:11] != b"EXFAT   " or sector[510:512] != b"\x55\xaa":
        raise Stop(f"{path.replace(TEST_ROOT, '')}'s first sector is not exFAT's; nothing was changed")
    return sector[FLAGS]


def say_flags(flags):
    marked = "SET (it wasn't safely removed last time)" if flags & IN_USE else "clear"
    print(f"the not-safely-removed mark: {marked}")
    if flags & DISK_ERROR:
        print("the disk-error bit: SET (a system saw a disk error on this card)")
    return bool(flags & (IN_USE | DISK_ERROR))


def fsck_run(fsck, args, node):
    """fsck.exfat with `args` on the card, its words shown as they come; its exit code (0 clean, 1 mended)."""
    print(f"running: fsck.exfat {' '.join(args)} /dev/{node}", flush=True)
    res = subprocess.run([fsck] + args + [p(f"/dev/{node}")], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, timeout=3600)
    for line in res.stdout.replace(TEST_ROOT, "").splitlines():
        print("    " + line)
    return res.returncode


def root_step(what, name, size, node, user):
    """Under sudo: the disk again, then the check, or the repair and the check after it."""
    info, reasons = describe(name, user)
    if info["size"] != size:
        raise Stop(f"/dev/{name} is not the card found a moment ago (its size changed); nothing was changed")
    if info["desktop"]:
        reasons.append("something of it is mounted again")
    if reasons:
        raise Stop(f"/dev/{name} no longer qualifies ({'; '.join(reasons)}); nothing was changed")
    if info["fs_node"] != node:
        raise Stop(f"/dev/{node} is not the card's filesystem; nothing was changed")
    path = p("/dev/" + node)
    st = os.stat(path)
    if not TEST_ROOT and not stat.S_ISBLK(st.st_mode):
        raise Stop(f"{path} is not a block device; nothing was changed")
    part = next(part for part in info["parts"] if part["node"] == node)
    if not TEST_ROOT and f"{os.major(st.st_rdev)}:{os.minor(st.st_rdev)}" != part["dev"]:
        raise Stop(f"{path} is not the device sysfs describes; nothing was changed")
    fsck = program("fsck.exfat")
    print(f"fsck.exfat: exfatprogs {exfatprogs_version(fsck)}")
    if what == "check":
        flags = volume_flags(path)
        marked = say_flags(flags)
        code = fsck_run(fsck, ["-n"], node)
        if code == 0:
            print("CHECK: clean" + (", but marked" if marked else " and unmarked"))
            return 3 if marked else 0
        print(f"CHECK: fsck.exfat found problems (exit code {code})")
        return 4
    say_flags(volume_flags(path))
    print(f"repairing /dev/{node} (fsck.exfat -y: it holds the card exclusively while it works) ...", flush=True)
    code = fsck_run(fsck, ["-y"], node)
    if code not in (0, 1):
        print(f"the repair stopped (exit code {code}); the card may still be marked")
    print("checking it again:")
    flags = volume_flags(path)
    marked = say_flags(flags)
    again = fsck_run(fsck, ["-n"], node)
    if again == 0 and not marked:
        print("CHECK: clean and unmarked")
        return 0
    print("CHECK: still " + ("marked" if marked else "not clean") + f" (fsck.exfat -n exit code {again})")
    return 4


def as_root(what, info, user):
    sudo = [] if TEST_ROOT else ["sudo"]
    res = subprocess.run(sudo + [sys.executable, "-I", os.path.abspath(__file__), f"--root-{what}", info["name"],
                                 str(info["size"]), info["fs_node"], user])
    return res.returncode


def mount_again(nodes):
    """Gives the desktop back the mounts taken away for the check. A failure here changes nothing on the card."""
    for node in nodes:
        if any(source == os.path.realpath(p(f"/dev/{node}")) for _, source, _, _ in mounts()):
            continue
        try:
            worked, message = udisks("mount", node)
        except (Stop, OSError, subprocess.SubprocessError) as err:
            worked, message = False, str(err)
        print(f"mounting /dev/{node} again for the desktop: {message or ('done' if worked else 'no answer')}" +
              ("" if worked else " (open it from the file manager instead)"))


def leave(info):
    """A clean card stays unmounted, ready to go back in the MPC (a desktop that writes to it marks it again)."""
    if any(source in {os.path.realpath(p(f"/dev/{part['node']}")) for part in info["parts"]}
           for _, source, _, _ in mounts()):
        print("Your desktop has mounted the card again: eject it in the file manager (Eject or Safely remove) "
              "before you take it out, or the mark comes back.")
    else:
        print("The card is left unmounted: take it out now and put it back in the MPC's slot.")


def main(argv):
    if argv[:1] in (["--root-check"], ["--root-repair"]):
        if len(argv) != 5 or not argv[2].isdigit():
            raise Stop("usage: --root-check|--root-repair NAME SIZE NODE USER (run without it instead)")
        if not TEST_ROOT and os.geteuid() != 0:
            raise Stop(f"{argv[0]} runs under sudo")
        if not NAME.match(argv[1]):
            raise Stop(f"{argv[1]} is not a disk this tool checks")
        return root_step(argv[0][len("--root-"):], argv[1], int(argv[2]), argv[3], argv[4])
    if argv not in ([], ["--repair"], ["--repair", "--yes"]):
        print(__doc__.strip())
        return 2
    repair = argv[:1] == ["--repair"]
    yes = argv[1:] == ["--yes"]
    user = os.environ.get("USER") or os.environ.get("LOGNAME") or ""
    if not TEST_ROOT and os.geteuid() == 0:
        raise Stop("run this as yourself, not under sudo: it asks for sudo itself, for the card's steps only")
    print("Check" + (" and repair" if repair else "") + " the MPC's SD card in this computer (workstation only, "
          "no MPC contact)")
    info = find_card(user)
    print()
    print("The card:")
    show(info, [], full=True, verdict=False)
    node = info["fs_node"]
    for desk in info["desktop"]:
        worked, message = udisks("unmount", desk)
        answer = message or ("done" if worked else "no answer")
        print(f"unmounting /dev/{desk} from the desktop for the check: {answer}")
        if not worked:
            raise Stop(f"/dev/{desk} could not be unmounted: close any window or program showing the card, then run "
                       "this again. Nothing was changed")
    print("The check reads the card under sudo, read-only:", flush=True)
    code = as_root("check", info, user)
    if code not in (0, 3, 4):
        mount_again(info["desktop"])
        print("The check did not finish; see above. Nothing was changed.")
        return 1
    if not repair:
        if code == 0:
            print("RESULT: the card is clean and its mark is clear. Nothing was changed.")
            leave(info)
        else:
            mount_again(info["desktop"])
            print("RESULT: the card needs a repair (see above). Nothing was changed. To repair it, copy anything "
                  "you can't lose off the card first, then run this again with --repair.")
        return 0 if code == 0 else 3
    if code == 0:
        print("RESULT: the card is clean and its mark is clear: nothing to repair. Nothing was changed.")
        leave(info)
        return 0
    print()
    print("The repair changes the card's records where the check found them wrong, and clears the mark. It can't")
    print("be undone: anything on the card you can't lose should be copied to this computer first.")
    if yes:
        print(f"Repairing /dev/{node}: asked for on the command line (--yes).")
    elif ask(f"Type {CONFIRM} and press Enter to repair /dev/{node} (anything else stops): ") != CONFIRM:
        mount_again(info["desktop"])
        print("Stopped. Nothing was changed.")
        return 1
    code = as_root("repair", info, user)
    if code == 0:
        print(f"READY: /dev/{node} is clean and unmarked. MPFree can save to it now.")
        leave(info)
        return 0
    mount_again(info["desktop"])
    print("RESULT: the repair did not leave the card clean and unmarked (see above). MPFree keeps reading it "
          "read-only. Run --repair again; if it says the same, send me the output.")
    return 1


if __name__ == "__main__":
    os.umask(0o022)
    sys.stdout.reconfigure(line_buffering=True)  # in order with sudo's and the programs' own output
    try:
        sys.exit(main(sys.argv[1:]))
    except NoCard as err:
        print(f"STOP: {err}")
        sys.exit(5)
    except (Stop, OSError, subprocess.SubprocessError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:
        print("\nSTOP: interrupted (Ctrl-C).")
        sys.exit(130)
