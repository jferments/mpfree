#!/usr/bin/env python3
"""WORKSTATION: the SD card round (step S2a of docs/dev/hardware/sd-card-and-internal-drive.md) in two commands.
Each runs every step by itself, logs each with results.py, and stops only where your hands are needed (moving
the stick, switching the MPC on); it waits for the MPC by itself instead of asking for Enter. The SD card stays
in the MPC's slot the whole time: it is checked, repaired if it needs it, and tested there, over SSH.

  MPFREE_ACCEPT_ROUND=YES python3 tools/mpc-one/sd-card-round.py prepare [--from STEP]
  MPFREE_ACCEPT_ROUND=YES python3 tools/mpc-one/sd-card-round.py run [--sd-only]

Before `prepare`: the MPFree stick in this computer (the one MPFree stick), and the MPC switched on into Akai's
system with developer mode on, its SD card in its slot and nothing in its USB port.

prepare (about 30 minutes). Your hands only at the start, for two answers:
  0. Which disk: it finds the one MPFree stick among this computer's USB disks (lsblk, read-only) and shows
               its path, its make and model, its size and its partitions' names and labels. Type y to write
               that disk, n (or anything else, twice) to stop with nothing written. Then sudo asks for your
               password, once for the whole round (it is kept fresh while prepare runs; run needs none).
  1. checks    WORKSTATION ONLY: this clone's tests for every tool the round uses.
  2. build     WORKSTATION ONLY: the MPC's programs, the helpers and the USB creator.
  3. stick     STATE-CHANGING, THE STICK ONLY, the disk you confirmed and no other: a look again (the same
               path, make, model, size and serial as confirmed, or it stops), a look inside (read-only; it
               must be an MPFree stick with partition 4 MPFREE-DATA ext4 and developer SSH on), then the USB
               creator remakes partitions 1, 2, 3 and 5 and the table on that path, keeping partition 4
               (--keep-data: your settings, files and backups), with the newest ~/mpfree-work/dev-tools-*
               folder if there is one, then a second look: partition 4 and its backups as before. Undo: run
               prepare again from an earlier commit.
  4. boot-map  READ-ONLY, AKAI'S SYSTEM: waits for it on port 22, then collect-over-ssh.sh --boot-map (the
               boot-critical partitions' hashes). The SD card stays in.
--from STEP starts at that step, after a stop that says so; the steps before it must have finished at this
commit. Every prepare that writes the stick asks which disk again.

run (about 1 h 15, or 30 minutes with --sd-only; your hands only to move the stick and switch the MPC on):
  1. Waits for MPFree on port 2222 (you: switch the MPC off, put the stick in, switch it on).
  2. sd-card   STATE-CHANGING, THE CARD'S MPFree FOLDER AND PARTITION 4: host_disktest_mpc.py --sd-card
               --repair-card. If the card is marked "not safely removed" (Akai's system leaves that mark) or
               doesn't check clean, the write window repairs it first, in the MPC (STATE-CHANGING, the card's
               filesystem records only: fsck.exfat -y, then checked again; no undo, the records it mends were
               wrong); a card it can't leave clean stays read-only and the write steps are skipped. The test's
               files are deleted at the end and the write window checks the card again after closing.
  3. Unless --sd-only, the stick tests sent on 23 Nov: stream-read (read-only), stream-recorder
               (partition 4's own folder and one kernel setting, put back), stream-app (partition 4's own
               folder), heat (the processor busy, no file written).
  4. power-off THE MPC, RAM ONLY: asks the stick's system for a clean shut down (the pad player's power
               request: one word in /run/mpfree-app/power-request) and waits for it to go off.
  5. Waits for Akai's system on port 22 (you: take the stick out, the card stays in, switch the MPC on),
               then boot-map again, READ-ONLY, and compares every boot-critical hash with prepare's.
  6. Packs everything into one zip: upload it.
A step that fails is said and the rest still run, except that run stops before the SD card test if MPFree
doesn't answer, and the boot map is always taken.

It never writes the MPC's eMMC (the stick's system can't see it), the U-Boot environment or anything of Akai's
system. MPC_IP, MPC_KEY, MPFREE_WORK_DIR and MPFREE_RESULTS_DIR change its defaults.
"""
import argparse
import glob
import importlib.util
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


results = load("results", "results.py")
app = load("host_app_mpc", "host_app_mpc.py")
kexec = app.kexec


class Stop(Exception):
    pass


WORK = os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
KEY = os.environ.get("MPC_KEY", "/home/jesse/.ssh/mpc_one_autorun_ed25519")
IP = os.environ.get("MPC_IP", "10.42.0.141")
TEST_PORT = int(os.environ.get("MPC_TEST_PORT", "2222"))
AKAI_PORT = int(os.environ.get("MPC_PORT", "22"))
# Tests make the waits short.
POLL = float(os.environ.get("MPFREE_ROUND_POLL", "5"))
SETTLE = float(os.environ.get("MPFREE_ROUND_SETTLE", "30"))
BOOT_WAIT = float(os.environ.get("MPFREE_ROUND_BOOT_WAIT", "900"))
OFF_WAIT = float(os.environ.get("MPFREE_ROUND_OFF_WAIT", "180"))
WINDOW_QUIET = float(os.environ.get("MPFREE_ROUND_WINDOW_QUIET", "150"))
# Tests: every step's command goes through this program, which stands in for it.
FAKE = os.environ.get("MPFREE_ROUND_FAKE")

UPDATE = os.path.join(WORK, "akai-update-files")
ARM = "rust/target/armv7-unknown-linux-gnueabihf/release"
CREATOR = "rust/target/release/mpfree-stick"
ZIG_PYTHON = os.path.expanduser("~/.local/share/mpfree-zig/bin/python3")
STATE = "round-state.txt"
PREPARE_STEPS = ("checks", "build", "stick", "boot-map")
# The stick's root partition's names (rust/mpfree-stick/src/stick.rs ROOT_NAME, ROOT_OFF_NAME), and partition
# 4's, which a write cut off part way leaves alone in the table (stick.rs write_disk) for the next write to keep.
STICK_ROOTS = ("mpfree-test-root", "mpfree-test-root-off")
DATA_NAME = "mpfree-test-data"
HALF_WRITTEN = "not an MPFree stick (no mpfree-test-root partition)"
LSBLK = ["lsblk", "-J", "-b", "-o", "NAME,TYPE,TRAN,SIZE,VENDOR,MODEL,SERIAL,LABEL,PARTLABEL,FSTYPE"]
BOOT_PARTS = ("mmcblk0p1", "mmcblk0p2", "mmcblk0p3", "mmcblk0p4", "mmcblk0p5", "mmcblk0boot0", "mmcblk0boot1",
              "mmcblk0p6")
LOCAL_TESTS = ("test_sd_card_round.py", "test_sd_card_window.py", "test_sd_card_write.py", "test_repair_sd_card.py",
               "test_stick_setup.py", "test_creator_recipe.py", "test_write_test_stick.py", "test_app_mpc.py",
               "test_app_checks.py", "test_results.py", "test_host_disktest_mpc.py", "test_host_streamtest_mpc.py",
               "test_devtest_report.py")
# The stick's system, port 2222, as root: one word for the pad player's power path unit, written whole by a
# rename, as the pad player's power card writes it. power.sh then starts poweroff.target, which stops every
# service and unmounts partition 4 cleanly.
POWER_OFF = r"""R=''
d="$R/run/mpfree-app"
[ -d "$d" ] || { echo "no $d: this is not the stick's system with the pad player"; exit 1; }
printf 'poweroff\n' > "$d/power-request.round" && sync && mv "$d/power-request.round" "$d/power-request" &&
  echo "asked the stick's system for a clean power-off"
"""


def current_dir():
    return os.path.join(results.results_root(), "current")


class Log:
    """The round's own lines: on the terminal and in current/round-COMMAND.txt (in the zip)."""

    def __init__(self, command):
        self.path = os.path.join(current_dir(), f"round-{command}.txt")
        self.lines = []

    def say(self, line=""):
        print(line, flush=True)
        self.lines.append(line)
        os.makedirs(os.path.dirname(self.path), mode=0o700, exist_ok=True)
        with open(self.path, "a") as fh:
            fh.write(line + "\n")


def git(*args):
    res = subprocess.run(["git", "-C", REPO, *args], capture_output=True, text=True)
    return res.stdout.strip() if res.returncode == 0 else ""


def read_state():
    path = os.path.join(current_dir(), STATE)
    state = {"done": []}
    if os.path.isfile(path):
        with open(path) as fh:
            for line in fh:
                key, _, value = line.strip().partition("=")
                if key == "done":
                    state["done"].append(value)
                elif key:
                    state[key] = value
    return state


def note_state(key, value):
    os.makedirs(current_dir(), mode=0o700, exist_ok=True)
    with open(os.path.join(current_dir(), STATE), "a") as fh:
        fh.write(f"{key}={value}\n")


def step(log, name, cmd, env=None):
    """One logged step (results.run_piped): its exit code and its log's text."""
    if FAKE:
        cmd = [FAKE, name, *cmd]
    log.say(f"\n---- {name}: {' '.join(cmd)}")
    rc = results.run_piped(name, cmd, env, zip_hint=False)
    logs = sorted(glob.glob(os.path.join(current_dir(), f"[0-9][0-9]-{name}.log")))
    with open(logs[-1], errors="replace") as fh:
        text = fh.read()
    log.say(f"---- {name}: exit code {rc}")
    if rc == 130:
        raise Stop(f"interrupted (Ctrl-C) during {name}")
    return rc, text


def bash(*parts):
    return ["bash", "-c", " && ".join(parts)]


def answers(port):
    try:
        with socket.create_connection((IP, port), timeout=3):
            return True
    except OSError:
        return False


def wait_for(log, port, what, up=True, limit=None):
    limit = BOOT_WAIT if limit is None else limit
    minutes = max(1, int(limit // 60))
    log.say(f"  waiting for {what} ({IP} port {port}, up to {minutes} minute{'s' if minutes > 1 else ''})...")
    start = last = time.monotonic()
    while time.monotonic() - start < limit:
        if answers(port) == up:
            log.say(f"  {'it answers' if up else 'it has stopped answering'} (after {time.monotonic() - start:.0f} s)")
            return True
        time.sleep(POLL)
        if time.monotonic() - last >= 60:
            last = time.monotonic()
            log.say(f"  still waiting ({last - start:.0f} s)")
    log.say(f"  no change within {int(limit)} s")
    return False


class Sudo:
    """Asks for the password once, then keeps it fresh for this terminal while the command runs."""

    def __init__(self, log):
        self.log = log
        self.on = threading.Event()

    def start(self):
        self.log.say("sudo: the stick's steps need it; it asks for your password once, now, for the whole round "
                     "(command 2 needs none).")
        if subprocess.run(["sudo", "-v"]).returncode != 0:
            raise Stop("sudo did not accept the password; nothing was changed")
        self.on.set()
        threading.Thread(target=self.keep, daemon=True).start()

    def keep(self):
        while self.on.is_set():
            time.sleep(60)
            subprocess.run(["sudo", "-n", "-v"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def look(log, name, cmd):
    """A read-only look whose words the round reads: (exit code, output), also kept in the round's log."""
    if FAKE:
        cmd = [FAKE, name, *cmd]
    try:
        res = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as err:
        raise Stop(f"{cmd[0]} didn't run ({err}); nothing was written")
    with open(log.path, "a") as fh:
        fh.write(f"---- {name}: {' '.join(cmd)}: exit code {res.returncode}\n{res.stdout}{res.stderr}")
    return res.returncode, res.stdout


def text_of(value):
    return " ".join(str(value or "").split())


def sticks(listing):
    """The USB disks holding an MPFree stick (a partition named mpfree-test-root, or partition 4 alone after a
    write cut off part way), from lsblk -J."""
    try:
        disks = json.loads(listing).get("blockdevices", [])
    except (ValueError, AttributeError):
        raise Stop("lsblk's list of disks could not be read; nothing was written")
    found = []
    for d in disks:
        if d.get("type") != "disk" or text_of(d.get("tran")) != "usb":
            continue
        names = [text_of(p.get("partlabel")) for p in d.get("children") or []]
        if any(n in STICK_ROOTS for n in names) or names == [DATA_NAME]:
            found.append(d)
    return found


def identity(d):
    """What must stay the same between the y and the write: path, make, model, size, serial, partitions."""
    return (text_of(d.get("name")), text_of(d.get("vendor")), text_of(d.get("model")), text_of(d.get("size")),
            text_of(d.get("serial")), tuple(text_of(p.get("partlabel")) for p in d.get("children") or []))


def disk_lines(d):
    name = " ".join(x for x in (text_of(d.get("vendor")), text_of(d.get("model"))) if x) or "USB disk"
    size = int(text_of(d.get("size")) or 0)
    lines = [f"    path:       /dev/{text_of(d.get('name'))}",
             f"    device:     {name}, {size / 1e9:.1f} GB ({size} bytes), on USB"]
    for p in d.get("children") or []:
        number = re.search(r"(\d+)$", text_of(p.get("name")))
        label = text_of(p.get("label"))
        lines.append(f"    partition {number.group(1) if number else '?'}: {text_of(p.get('partlabel')) or '-':<18} "
                     f"label {label or '(none)':<12} {text_of(p.get('fstype')) or '-'}")
    return lines, f"/dev/{text_of(d.get('name'))} ({name}, {size / 1e9:.1f} GB)"


def find_stick(log, name):
    rc, out = look(log, name, LSBLK)
    if rc != 0:
        raise Stop(f"lsblk didn't list this computer's disks (exit code {rc}); nothing was written")
    found = sticks(out)
    if not found:
        raise Stop("no MPFree stick among this computer's USB disks (a disk with a partition named "
                   "mpfree-test-root): put the MPFree stick in this computer; nothing was written")
    if len(found) > 1:
        raise Stop("more than one MPFree stick is in this computer (" + ", ".join(
            f"/dev/{text_of(d.get('name'))}" for d in found) + "): leave only the one to write in; nothing was "
            "written")
    return found[0]


def ask(log, question):
    """y or n, typed; anything else asks again, twice. No answer (no terminal) is no."""
    for _ in range(3):
        log.say(question)
        try:
            answer = input("> ").strip().lower()
        except EOFError:
            answer = ""
            log.say("  (no answer: this needs a terminal)")
            return False
        log.say(f"  typed: {answer!r}")
        if answer in ("y", "yes"):
            return True
        if answer in ("n", "no"):
            return False
        log.say("  Type y or n.")
    return False


def confirm_stick(log):
    """Shows the one MPFree stick and asks for y before anything is written: its identity, once confirmed."""
    log.say("\n== 0. which disk (read-only: lsblk) ==")
    d = find_stick(log, "disk-look")
    lines, short = disk_lines(d)
    log.say("  The USB creator will write THIS disk: partitions 1, 2, 3 and 5 and the partition table, keeping "
            "partition 4 (your settings, files and backups). No other disk is touched.")
    for line in lines:
        log.say(line)
    if [text_of(p.get("partlabel")) for p in d.get("children") or []] == [DATA_NAME]:
        log.say("    (partition 4 alone: an earlier write was cut off part way; the creator finds and keeps it)")
    if not ask(log, f"  Write to {short}? Type y or n:"):
        raise Stop("not confirmed: nothing was written. Check which disk is the MPFree stick, then run prepare "
                   "again")
    log.say(f"  confirmed: {short}")
    return d


def boot_map(text):
    """{partition: hash} from a boot map report, and whether the SD card (mmcblk1) was in."""
    hashes = {}
    section = text.split("=== Boot-critical partition hashes", 1)[1:] or [""]
    for line in section[0].split("\n===", 1)[0].splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[0] in BOOT_PARTS and re.fullmatch(r"[0-9a-f]{64}", parts[1]):
            hashes[parts[0]] = parts[1]
    table = text.split("=== Partition table", 1)[1:] or [""]
    card = bool(re.search(r"\bmmcblk1\b", table[0].split("\n===", 1)[0]))
    return hashes, card


def boot_map_report(name):
    found = sorted(glob.glob(os.path.join(current_dir(), f"[0-9][0-9]-{name}--*.txt")))
    if not found:
        return None
    with open(found[-1], errors="replace") as fh:
        return fh.read()


def take_boot_map(log, name):
    """Waits for Akai's system and takes the boot map: (hashes, card in, problem or None)."""
    if not wait_for(log, AKAI_PORT, "Akai's system with developer mode on"):
        return {}, False, ("Akai's system didn't answer on port 22: is developer mode on (SHIFT + MENU, then "
                           "Developer)?")
    time.sleep(SETTLE)
    rc, _ = step(log, name, ["bash", "tools/mpc-one/collect-over-ssh.sh", "--boot-map"],
                 {"MPC_PORT": str(AKAI_PORT), "MPC_IP": IP})
    text = boot_map_report(name)
    if rc != 0 or text is None or "END MPFREE BOOT/RECOVERY MAP" not in text:
        return {}, False, f"the boot map did not finish (exit code {rc}); see {name}'s log"
    hashes, card = boot_map(text)
    missing = [p for p in BOOT_PARTS if p not in hashes]
    if missing:
        return hashes, card, "the boot map has no hash for " + ", ".join(missing)
    return hashes, card, None


def inspect_lines(text):
    """The stick's partition 4 and backups, as the creator's inspect says them."""
    return [ln for ln in text.splitlines() if ln.startswith(("partition 4:", "backup:"))]


def newest_dev_tools():
    found = [d for d in glob.glob(os.path.join(WORK, "dev-tools-*")) if os.path.isdir(d)]
    return max(found, key=os.path.getmtime) if found else None


def pack_earlier():
    """An earlier run's files in current/ go into their own zip first, so this round's zip is its own."""
    current = current_dir()
    files = os.listdir(current) if os.path.isdir(current) else []
    state = read_state()
    if files and state.get("commit") != git("rev-parse", "HEAD"):
        print("Earlier files in the results folder, not this round's: packing them into their own zip first.")
        results.pack()


def prepare(args, log):
    head = git("rev-parse", "HEAD")
    log.say(f"MPFree SD card round, command 1 of 2 (prepare), at {git('log', '-1', '--format=%h %s')}")
    state = read_state()
    start = PREPARE_STEPS.index(args.start)
    if start:
        if state.get("commit") != head:
            raise Stop(f"--from {args.start}: the steps before it ran at another commit; run prepare without --from")
        missing = [s for s in PREPARE_STEPS[:start] if s not in state["done"]]
        if missing:
            raise Stop(f"--from {args.start}: {', '.join(missing)} didn't finish; run prepare --from {missing[0]}")
    else:
        note_state("commit", head)
    if not os.path.isdir(UPDATE):
        raise Stop(f"no Akai update files in {UPDATE}; nothing was changed")
    if not os.path.isfile(KEY + ".pub"):
        raise Stop(f"no {KEY}.pub (the round's SSH key); nothing was changed")
    sudo = Sudo(log)
    disk = None
    if start <= PREPARE_STEPS.index("stick"):
        disk = confirm_stick(log)
        sudo.start()
    todo = PREPARE_STEPS[start:]

    if "checks" in todo:
        log.say("\n== 1. checks (workstation only) ==")
        tests = ["export MPFREE_REQUIRE_BUSYBOX=1"] + [f"python3 tools/mpc-one/tests/{t}" for t in LOCAL_TESTS] + [
            "cargo test --manifest-path rust/Cargo.toml --release --locked -p mpfree-audiotracks --test "
            "disktest_sd_card", "echo LOCAL-CHECKS-OK"]
        rc, text = step(log, "local-checks", bash(*tests))
        if rc != 0 or "LOCAL-CHECKS-OK" not in text:
            raise Stop("a local check failed (see local-checks' log); nothing was changed")
        note_state("done", "checks")

    if "build" in todo:
        log.say("\n== 2. build (workstation only) ==")
        rc, text = step(log, "build-arm", bash(
            "export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc "
            "RUSTFLAGS='-C target-feature=+crt-static -C relocation-model=static'",
            "cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf "
            "-p mpfree-screen-log -p mpfree-startup -p mpfree-app",
            "cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf "
            "-p mpfree-audiotracks --bin mpfree-disktest",
            f"cd {ARM}", "file mpfree mpfree-disktest",
            "sha256sum mpfree-screen-log mpfree-startup mpfree mpfree-disktest"))
        if rc != 0:
            raise Stop("the MPC's programs didn't build (see build-arm's log); nothing was changed")
        rc, _ = step(log, "build-helpers", bash(
            f"CARGO_ZIGBUILD_PYTHON_PATH={ZIG_PYTHON} cargo zigbuild --manifest-path rust/Cargo.toml --release "
            "--locked --target armv7-unknown-linux-gnueabihf.2.34 -p mpfree-mpcstore --bin mpfree-fetch "
            "-p mpfree-plughost --bin mpfree-plughost",
            f"sha256sum {ARM}/mpfree-fetch {ARM}/mpfree-plughost"))
        if rc != 0:
            raise Stop("the helpers didn't build (see build-helpers' log); nothing was changed")
        rc, _ = step(log, "build-creator", bash(
            "cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-stick",
            f"sha256sum {CREATOR}"))
        if rc != 0:
            raise Stop("the USB creator didn't build (see build-creator's log); nothing was changed")
        note_state("done", "build")

    if "stick" in todo:
        log.say("\n== 3. stick (STATE-CHANGING, the stick only: partitions 1, 2, 3 and 5; partition 4 kept) ==")
        again = find_stick(log, "disk-again")
        if identity(again) != identity(disk):
            raise Stop(f"the MPFree stick in this computer is not the disk you confirmed ({disk_lines(disk)[1]}, now "
                       f"{disk_lines(again)[1]}); nothing was written. Run prepare again")
        path = f"/dev/{text_of(disk.get('name'))}"
        log.say(f"  the disk you confirmed, still the same: {disk_lines(disk)[1]}")
        rc, before = step(log, "stick-before", ["sudo", CREATOR, "inspect", "--device", path])
        table = re.findall(r"(?m)^partition (\d): (\S+)", before)
        if rc == 0 and HALF_WRITTEN in before and table == [("4", DATA_NAME)]:
            # The creator's own half-written stick: partition 4 alone. It keeps it; the look before the first try
            # (this results folder, this commit) is what partition 4 and its backups must be after.
            kept = read_state().get("partition4")
            if not kept:
                raise Stop(f"{path} holds partition 4 alone (a write cut off part way), and this results folder has "
                           "no look at it from before; nothing was written. Send me the results folder's logs")
            baseline = json.loads(kept)
            log.say("  partition 4 alone, as a write cut off part way leaves it: the creator keeps it, and the look "
                    "after is checked against the one before the first try")
        elif rc != 0 or "partition 4: MPFREE-DATA ext4" not in before or "developer SSH: on" not in before:
            raise Stop(f"{path} is not an MPFree stick with partition 4 MPFREE-DATA ext4 and developer SSH on "
                       "(see stick-before's log); nothing was written")
        else:
            baseline = inspect_lines(before)
            note_state("partition4", json.dumps(baseline))
        cmd = ["sudo", CREATOR, "write", UPDATE, "--device", path, "--yes-erase", path, "--keep-data",
               "--no-backup", "--no-akai-kits", "--programs", ARM, "--ssh-key", KEY + ".pub", "--work", WORK,
               "--record", WORK]
        tools = newest_dev_tools()
        if tools:
            cmd += ["--dev-tools", tools]
            log.say(f"  developer tools for Akai's system: {tools}")
        else:
            log.say(f"  no {WORK}/dev-tools-* folder: the stick goes without developer tools")
        rc, text = step(log, "write-stick", cmd)
        if rc != 0 or "done the stick is ready" not in text:
            raise Stop("the USB creator didn't finish (see write-stick's log). If it stopped before 'Writing the "
                       "stick', nothing changed; if it was cut off while writing, run prepare --from stick")
        rc, after = step(log, "stick-after", ["sudo", CREATOR, "inspect", "--device", path])
        if rc != 0 or inspect_lines(after) != baseline:
            raise Stop("the second look at the stick doesn't show partition 4 and its backups as before (see "
                       "stick-before's and stick-after's logs)")
        log.say("  partition 4 and its backups: as before")
        note_state("done", "stick")
        log.say("  The stick is ready: take it out of this computer (eject it first if your desktop shows it).")

    sudo.on.clear()

    log.say("\n== 4. boot-map (read-only, Akai's system) ==")
    log.say("  The MPC on Akai's system with developer mode on: nothing in its USB port, the SD card in its slot.")
    hashes, card, problem = take_boot_map(log, "boot-map-before")
    if problem:
        raise Stop(problem + "; then run prepare --from boot-map")
    log.say("  the SD card is in the MPC's slot" if card else
            "  note: no SD card in the MPC's slot. Put it in before command 2; it stays in from then on.")
    note_state("done", "boot-map")
    note_state("prepare", "ok")
    log.say("\nPREPARE-OK: the stick is made and Akai's boot map is taken.")
    log.say("NEXT: run command 2 (sd-card-round.py run). It tells you when to switch the MPC off, put the stick in "
            "and switch it on. The SD card stays in the MPC.")
    return 0


def power_off_now(args):
    """Internal (run's power-off step): the clean power-off request, over the stick's own SSH."""
    folder = kexec.newest_build(WORK)
    mpc = kexec.Mpc(argparse.Namespace(ip=IP, port=TEST_PORT, user="root", key=KEY),
                    os.path.join(folder, "known_hosts"))
    rc, text = mpc.run(POWER_OFF, test=True)
    print(text.strip())
    return rc


def run(args, log):
    state = read_state()
    head = git("rev-parse", "HEAD")
    log.say(f"MPFree SD card round, command 2 of 2 (run), at {git('log', '-1', '--format=%h %s')}")
    if state.get("prepare") != "ok":
        raise Stop("command 1 (prepare) hasn't finished in this results folder: run it first")
    if state.get("commit") != head:
        raise Stop("this clone is at another commit than command 1 made the stick from: switch back to it")
    before, _ = boot_map(boot_map_report("boot-map-before") or "")
    outcome = {}

    log.say("\n== 1. MPFree from the stick ==")
    log.say("  Switch the MPC off. Put the MPFree stick in its USB port (the SD card stays in its slot), wait 10 s "
            "and switch it on. MPFree comes up in about 20 s.")
    if not wait_for(log, TEST_PORT, "MPFree's system on the stick"):
        outcome["mpfree"] = "did not answer"
        log.say("  MPFree didn't answer: nothing was run on the MPC.")
    else:
        time.sleep(SETTLE)
        log.say("\n== 2. sd-card (the card's MPFree folder and partition 4, files deleted after; the card repaired "
                "first, in the MPC, if it is marked or doesn't check clean) ==")
        rc, _ = step(log, "sd-card", ["python3", "tools/mpc-one/host_disktest_mpc.py", "--sd-card", "--repair-card"],
                     {"MPFREE_ACCEPT_DISKTEST": "YES", "MPFREE_ROUND": "1"})
        outcome["sd-card"] = "ran" if rc == 0 else f"exit code {rc}"
        if not args.sd_only:
            if rc != 0:
                log.say(f"  letting the SD card's windows close by themselves first ({WINDOW_QUIET:.0f} s)")
                time.sleep(WINDOW_QUIET)
            log.say("\n== 3. the stick tests (partition 4's own folders; the processor busy) ==")
            for name, cmd, env in (
                    ("stream-read", ["--read-only"], {"MPFREE_ACCEPT_DISKTEST": "READ-ONLY"}),
                    ("stream-recorder", ["--recorder"], {"MPFREE_ACCEPT_DISKTEST": "YES"}),
                    ("stream-app", None, {"MPFREE_ACCEPT_STREAMTEST": "YES"}),
                    ("heat", ["--heat"], {"MPFREE_ACCEPT_DISKTEST": "YES"})):
                tool = ["python3", "tools/mpc-one/host_streamtest_mpc.py"] if cmd is None else \
                    ["python3", "tools/mpc-one/host_disktest_mpc.py", *cmd]
                rc, _ = step(log, name, tool, dict(env, MPFREE_ROUND="1"))
                outcome[name] = "ran" if rc == 0 else f"exit code {rc}"
        log.say("\n== 4. power-off (a clean shut down of the stick's system) ==")
        rc, _ = step(log, "power-off", [sys.executable, os.path.abspath(__file__), "power-off"])
        if rc == 0 and wait_for(log, TEST_PORT, "the MPC to switch off", up=False, limit=OFF_WAIT):
            log.say("  the MPC is off")
        else:
            log.say("  it didn't switch off by itself: hold its power button until it goes off")

    log.say("\n== 5. boot-map (read-only, Akai's system) ==")
    log.say("  Take the stick out of the MPC (the SD card stays in), wait 10 s, and switch it on into Akai's system "
            "(developer mode stays on).")
    after, _, problem = take_boot_map(log, "boot-map-after")
    if problem:
        outcome["boot map"] = problem
    else:
        changed = [p for p in BOOT_PARTS if before.get(p) != after.get(p)]
        for p in BOOT_PARTS:
            log.say(f"  {p:<13} {'same' if p not in changed else 'DIFFERENT'}")
        outcome["boot map"] = "every boot-critical hash as before" if not changed else \
            "CHANGED: " + ", ".join(changed)
    log.say("\nRESULT:")
    for k, v in outcome.items():
        log.say(f"  {k}: {v}")
    passed = outcome.get("sd-card") == "ran" and outcome.get("boot map") == "every boot-critical hash as before"
    log.say("ROUND-OK" if passed else "ROUND-NOT-OK: see the steps above")
    log.say("Packing the zip:")
    results.pack()
    return 0 if passed else 1


def main(argv):
    if argv == ["power-off"]:
        return power_off_now(None)
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="command", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("--from", dest="start", choices=PREPARE_STEPS, default=PREPARE_STEPS[0])
    go = sub.add_parser("run")
    go.add_argument("--sd-only", action="store_true")
    args = p.parse_args(argv)
    os.chdir(REPO)
    if os.environ.get("MPFREE_ACCEPT_ROUND") != "YES":
        raise Stop("opt-in required (prepare remakes the stick after you confirm the disk; run may repair the SD "
                   "card in the MPC, writes test files on the card and the stick and shuts the MPC down): "
                   "MPFREE_ACCEPT_ROUND=YES python3 "
                   f"tools/mpc-one/sd-card-round.py {args.command}")
    if os.geteuid() == 0 and not FAKE:
        raise Stop("run it as yourself, not under sudo: it asks for sudo itself, once")
    if args.command == "prepare":
        pack_earlier()
    log = Log(args.command)
    try:
        return prepare(args, log) if args.command == "prepare" else run(args, log)
    except (Stop, results.Stop, kexec.Stop) as err:
        log.say(f"\nSTOP: {err}")
        return 1


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, results.Stop, kexec.Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:
        print("\nSTOP: interrupted (Ctrl-C).")
        sys.exit(130)
