#!/usr/bin/env python3
"""WORKSTATION: the SD card round (step S2a of docs/dev/hardware/sd-card-and-internal-drive.md) in two commands.
Each runs every step by itself, logs each with results.py, and stops only where your hands are needed (moving
the stick or the card, switching the MPC on); it waits for the MPC by itself instead of asking for Enter.

  MPFREE_ACCEPT_ROUND=YES python3 tools/mpc-one/sd-card-round.py prepare [--repair-card] [--from STEP]
  MPFREE_ACCEPT_ROUND=YES python3 tools/mpc-one/sd-card-round.py run [--sd-only]

Before `prepare`: the MPFree stick and the SD card in this computer, and the MPC switched on into Akai's system
with developer mode on and nothing in its USB port or SD slot (Akai's system marks a card it writes to, and a
marked card is never written by MPFree).

prepare (about 30 minutes; it asks for your sudo password once, at the start):
  1. checks    WORKSTATION ONLY: this clone's tests for every tool the round uses.
  2. build     WORKSTATION ONLY: the MPC's programs, the helpers and the USB creator.
  3. stick     STATE-CHANGING, THE STICK ONLY: a look at the stick (read-only; it must be an MPFree stick with
               partition 4 MPFREE-DATA ext4 and developer SSH on), then the USB creator remakes partitions 1, 2,
               3 and 5 and the table, keeping partition 4 (--keep-data: your settings, files and backups), with
               the newest ~/mpfree-work/dev-tools-* folder if there is one, then a second look: partition 4 and
               its backups as before. Undo: run prepare again from an earlier commit.
  4. card      THE SD CARD IN THIS COMPUTER: checked read-only (repair-sd-card.py). With --repair-card, a card
               that is marked "not safely removed" or doesn't check clean is repaired (STATE-CHANGING, the
               card's filesystem records only: fsck.exfat -y; no undo, the records it mends were wrong). Left
               unmounted, ready to take out. No card in this computer: said, and skipped (the MPC's write
               window checks the card anyway and refuses a marked one).
  5. boot-map  READ-ONLY, AKAI'S SYSTEM: waits for it on port 22, then collect-over-ssh.sh --boot-map (the
               boot-critical partitions' hashes). It stops if the SD card is in the MPC.
--from STEP starts at that step, after a stop that says so; the steps before it must have finished at this
commit.

run (about 1 h 15, or 30 minutes with --sd-only; your hands only to move the stick and the card):
  1. Waits for MPFree on port 2222 (you: switch the MPC off, put the stick in, switch it on).
  2. sd-card   STATE-CHANGING, THE CARD'S MPFree FOLDER AND PARTITION 4: host_disktest_mpc.py --sd-card. You
               put the SD card in the MPC's slot when it says (it waits 15 minutes for it). Its files are
               deleted at the end and the write window checks the card again after closing.
  3. Unless --sd-only, the stick tests sent on 23 Nov: stream-read (read-only), stream-recorder
               (partition 4's own folder and one kernel setting, put back), stream-app (partition 4's own
               folder), heat (the processor busy, no file written).
  4. power-off THE MPC, RAM ONLY: asks the stick's system for a clean shut down (the pad player's power
               request: one word in /run/mpfree-app/power-request) and waits for it to go off.
  5. Waits for Akai's system on port 22 (you: take the stick and the card out, switch the MPC on), then
               boot-map again, READ-ONLY, and compares every boot-critical hash with prepare's.
  6. Packs everything into one zip: upload it.
A step that fails is said and the rest still run, except that run stops before the SD card test if MPFree
doesn't answer, and the boot map is always taken.

It never writes the MPC's eMMC (the stick's system can't see it), the U-Boot environment or anything of Akai's
system. MPC_IP, MPC_KEY, MPFREE_WORK_DIR and MPFREE_RESULTS_DIR change its defaults.
"""
import argparse
import glob
import importlib.util
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
PREPARE_STEPS = ("checks", "build", "stick", "card", "boot-map")
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
        self.log.say("sudo: the stick's and the card's steps need it; it asks for your password once, now.")
        if subprocess.run(["sudo", "-v"]).returncode != 0:
            raise Stop("sudo did not accept the password; nothing was changed")
        self.on.set()
        threading.Thread(target=self.keep, daemon=True).start()

    def keep(self):
        while self.on.is_set():
            time.sleep(60)
            subprocess.run(["sudo", "-n", "-v"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


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
    if start <= PREPARE_STEPS.index("card"):
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
        rc, before = step(log, "stick-before", ["sudo", CREATOR, "inspect", "--device", "mpfree"])
        if rc != 0 or "partition 4: MPFREE-DATA ext4" not in before or "developer SSH: on" not in before:
            raise Stop("the stick is not an MPFree stick with partition 4 MPFREE-DATA ext4 and developer SSH on "
                       "(see stick-before's log); nothing was written. Is the MPFree stick (and only it) in?")
        cmd = ["sudo", CREATOR, "write", UPDATE, "--device", "mpfree", "--yes-erase", "mpfree", "--keep-data",
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
        rc, after = step(log, "stick-after", ["sudo", CREATOR, "inspect", "--device", "mpfree"])
        if rc != 0 or inspect_lines(after) != inspect_lines(before):
            raise Stop("the second look at the stick doesn't show partition 4 and its backups as before (see "
                       "stick-before's and stick-after's logs)")
        log.say("  partition 4 and its backups: as before")
        note_state("done", "stick")
        log.say("  The stick is ready: take it out of this computer (eject it first if your desktop shows it).")

    if "card" in todo:
        log.say("\n== 4. card (the SD card in this computer: read-only check"
                + (", repaired if needed" if args.repair_card else "") + ") ==")
        rc, _ = step(log, "card-check", ["python3", "tools/mpc-one/repair-sd-card.py"])
        if rc == 5:
            log.say("  no SD card in this computer: skipped. The MPC's write window checks the card itself and "
                    "refuses one that is marked or doesn't check clean.")
        elif rc == 3 and args.repair_card:
            rc, _ = step(log, "card-repair", ["python3", "tools/mpc-one/repair-sd-card.py", "--repair", "--yes"])
            if rc != 0:
                raise Stop("the card repair didn't leave it clean and unmarked (see card-repair's log). The stick is "
                           "ready; run prepare --from card --repair-card again, or send me the zip")
            log.say("  the card is repaired, clean and unmarked")
        elif rc == 3:
            raise Stop("the card needs a repair (see card-check's log). The stick is ready; to repair the card, run "
                       "prepare --from card --repair-card")
        elif rc != 0:
            raise Stop("the card check didn't finish (see card-check's log). The stick is ready; run prepare --from "
                       "card again")
        else:
            log.say("  the card is clean and unmarked")
        note_state("done", "card")
        log.say("  Take the card out of this computer. Keep it out of the MPC until command 2 says to put it in.")
    sudo.on.clear()

    log.say("\n== 5. boot-map (read-only, Akai's system) ==")
    log.say("  Switch the MPC on into Akai's system with developer mode on, with NOTHING in its USB port or SD slot.")
    hashes, card, problem = take_boot_map(log, "boot-map-before")
    if card:
        raise Stop("the SD card is in the MPC while Akai's system runs, which can mark it. Switch the MPC off, "
                   "put the card in this computer again and run: prepare --from card"
                   + (" --repair-card" if args.repair_card else ""))
    if problem:
        raise Stop(problem + "; then run prepare --from boot-map")
    note_state("done", "boot-map")
    note_state("prepare", "ok")
    log.say("\nPREPARE-OK: the stick is made, the card is ready and Akai's boot map is taken.")
    log.say("NEXT: run command 2 (sd-card-round.py run). It tells you when to switch the MPC off, put the stick in "
            "and put the card in.")
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
    log.say("  Switch the MPC off. Put the MPFree stick in its USB port (the SD card stays out), wait 10 s and "
            "switch it on. MPFree comes up in about 20 s.")
    if not wait_for(log, TEST_PORT, "MPFree's system on the stick"):
        outcome["mpfree"] = "did not answer"
        log.say("  MPFree didn't answer: nothing was run on the MPC.")
    else:
        time.sleep(SETTLE)
        log.say("\n== 2. sd-card (the card's MPFree folder and partition 4, files deleted after) ==")
        log.say("  >>> PUT THE SD CARD IN THE MPC'S SLOT NOW (the test waits up to 15 minutes for it) <<<")
        rc, _ = step(log, "sd-card", ["python3", "tools/mpc-one/host_disktest_mpc.py", "--sd-card"],
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
    log.say("  Take the stick AND the SD card out of the MPC, wait 10 s, and switch it on into Akai's system "
            "(developer mode stays on).")
    after, card, problem = take_boot_map(log, "boot-map-after")
    if card:
        outcome["note"] = ("the SD card was in the MPC under Akai's system, which can mark it: check it with "
                           "repair-sd-card.py before MPFree writes to it again")
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
    prep.add_argument("--repair-card", action="store_true")
    prep.add_argument("--from", dest="start", choices=PREPARE_STEPS, default=PREPARE_STEPS[0])
    go = sub.add_parser("run")
    go.add_argument("--sd-only", action="store_true")
    args = p.parse_args(argv)
    os.chdir(REPO)
    if os.environ.get("MPFREE_ACCEPT_ROUND") != "YES":
        raise Stop("opt-in required (prepare remakes the stick and may repair the card; run writes test files on the "
                   "card and the stick and shuts the MPC down): MPFREE_ACCEPT_ROUND=YES python3 "
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
