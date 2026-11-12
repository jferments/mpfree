#!/usr/bin/env python3
"""mpfree-disktest on the kexec test build: how fast and how steadily the stick's partition 4 streams audio. WRITES TEST FILES ON THE STICK'S PARTITION 4, THEN DELETES THEM.

Run it after `collect-over-ssh.sh --kexec-boot` has started the test build
from the USB stick. Like host_bench_mpc.py, it talks only to the test build:
it uses the newest ~/mpfree-work/kexec-stick-* folder's known_hosts, so stock
3.6 (a different host key) is refused before anything runs. Each step after
the first waits for you to press Enter:

  1. Workstation, read-only: checks that the program is a static ARM EABI5
     hard-float executable:
       rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-disktest
  2. Test build, read-only: host_app_mpc.py's checks (the test build's
     release, no MMC device, kexec off, / read-only, /run has room and allows
     programs, Akai's app is not running, who holds the sound, MIDI, touch
     and screen devices), then `df -k /run/mpfree-data`.
  3. Test build RAM: copies the program to /run/mpfree-disktest and checks
     its hash there. A power-cycle clears it.
  4. STICK, PARTITION 4: runs `mpfree-disktest` with no arguments, about 5
     minutes. It refuses (exit code 3, nothing written) unless
     /run/mpfree-data is the mount of an ext4 file system labelled
     MPFREE-DATA on partition 4 of a USB disk, no MMC device exists, and at
     least 64 MiB is free. Then it writes only inside
     /run/mpfree-data/mpfree-disktest (files it creates itself, O_EXCL and
     O_NOFOLLOW; at most 448 MiB at once and at most half the free space),
     and deletes that folder at the end, after an error, on Ctrl-C, or when
     the ssh session closes. It prints the free space before and after.
     It reads the partition's superblock (read-only) to check the label.
     It runs as an ordinary process, so the pad player's audio thread, if
     it is running, comes first.

What it measures, in 128 KiB blocks (the audio tracks' chunk): a sequential
write, a write paced at 3.1 MB/s (8 stereo tracks recording) for 60 s with
fdatasync every second, reads with the file's pages dropped from the cache
(sequential, then 8 paced streams), and 4 writing plus 4 reading streams at
once. For each: p50, p99 and max time per call, how many calls took over 50,
100, 250 and 500 ms, and how far behind schedule it fell (the RAM buffer a
recording or playback would need).

If it is stopped hard (power off) mid-run, the folder stays on partition 4:
the next run deletes it first (only the tool's own file names; anything else
in it stops the tool). To delete it by hand on the test build:
rm -r /run/mpfree-data/mpfree-disktest

Nothing here writes to the eMMC (the test build cannot see it), the U-Boot
environment, the SD card, or the stick's partitions 1 to 3. To go back: switch
the MPC off with its power switch, take out the stick, wait 10 s, switch it on.
"""
import argparse
import hashlib
import importlib.util
import os
import re
import signal
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
_spec = importlib.util.spec_from_file_location("host_app_mpc", os.path.join(HERE, "host_app_mpc.py"))
app = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(app)
kexec = app.kexec
Stop = kexec.Stop

PROGRAM = os.path.join(REPO_ROOT, "rust", "target", "armv7-unknown-linux-gnueabihf", "release", "mpfree-disktest")
REMOTE_DIR = "/run/mpfree-disktest"
# The one run: no arguments (the program picks /run/mpfree-data itself and refuses anything else).
TIMEOUT = 900
REFUSED = 3
DEFAULT_SSH = {
    "ip": os.environ.get("MPC_IP", "10.42.0.141"),
    "port": int(os.environ.get("MPC_PORT", "2222")),
    "user": os.environ.get("MPC_USER", "root"),
    "key": os.environ.get("MPC_KEY", "/home/jesse/.ssh/mpc_one_autorun_ed25519"),
}

# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
# Read-only.
DF = r"""R=''
if [ -d "$R/run/mpfree-data" ]; then df -k "$R/run/mpfree-data"; else echo "no /run/mpfree-data on the test build"; fi
"""
# Writes only into /run/mpfree-disktest (RAM).
PREPARE = r"""R=''
umask 077
d="$R/run/mpfree-disktest"
[ -d "$d" ] || mkdir "$d" || exit 1
rm -f "$d/mpfree-disktest" "$d/mpfree-disktest.part"
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-disktest/mpfree-disktest.part" && chmod 700 "$R/run/mpfree-disktest/mpfree-disktest.part" && mv "$R/run/mpfree-disktest/mpfree-disktest.part" "$R/run/mpfree-disktest/mpfree-disktest" """
HASH = r"""R=''
sha256sum "$R/run/mpfree-disktest/mpfree-disktest"
"""
RUN = r"""R=''
cd "$R/run/mpfree-disktest" || exit 1
exec "$R/run/mpfree-disktest/mpfree-disktest"
"""


def check_program(out, path):
    """Step 1: the program's size, hash and ELF kind. Returns (path, sha256)."""
    path = os.path.realpath(path)
    if not os.path.isfile(path):
        raise Stop(f"missing {path}: build it first (see docs/dev/design/audio-tracks.md)")
    with open(path, "rb") as fh:
        data = fh.read(app.PROGRAM_MAX + 1)
    problem = app.arm_static_problem(data)
    if problem:
        raise Stop(f"{path} is {problem}; rebuild it as docs/dev/design/audio-tracks.md shows")
    digest = hashlib.sha256(data).hexdigest()
    out.say(f"  {path}")
    out.say(f"    {len(data)} bytes, sha256 {digest}, static ARM EABI5 hard-float")
    return path, digest


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", default=DEFAULT_SSH["ip"])
    p.add_argument("--port", type=int, default=DEFAULT_SSH["port"])
    p.add_argument("--user", default=DEFAULT_SSH["user"])
    p.add_argument("--key", default=DEFAULT_SSH["key"])
    p.add_argument("--work", default=os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    p.add_argument("--program", default=PROGRAM)
    p.add_argument("--report")
    args = p.parse_args(argv)
    if os.environ.get("MPFREE_ACCEPT_DISKTEST") != "YES":
        raise Stop("opt-in required (writes test files on the stick's partition 4, then deletes them): "
                   "MPFREE_ACCEPT_DISKTEST=YES python3 tools/mpc-one/host_disktest_mpc.py")
    if not app.TESTING and not sys.stdin.isatty():
        raise Stop("run this from a terminal: each step waits for you to press Enter")
    out = kexec.Out(args.report)
    out.say("MPFree disk test on the test build (RAM, and test files on the stick's partition 4 that it deletes)")

    out.say("\n[1/4] Workstation checks (read-only)")
    path, digest = check_program(out, args.program)
    folder = kexec.newest_build(os.path.realpath(args.work))
    out.say(f"  test build: {folder}")
    with open(os.path.join(folder, "report.txt")) as fh:
        found = re.search(r"^its kernel release: (\S+)$", fh.read(), re.M)
    if not found or not kexec.RELEASE.match(found.group(1)):
        raise Stop(f"{folder}/report.txt does not name the test build's kernel release")
    mpc = kexec.Mpc(args, os.path.join(folder, "known_hosts"))

    out.say("\n[2/4] Checks on the test build (read-only; only its own host key is accepted)")
    rc, text = mpc.run(kexec.fill(app.CHECK, NEW=found.group(1)), test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc == 255 and "CHECKS-" not in text:
        raise Stop("the test build did not answer. Start it first with the --kexec-boot block; "
                   "on stock 3.6 this tool runs nothing")
    if rc != 0 or text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the test build is not ready; nothing was changed on it")
    rc, text = mpc.run(DF, test=True)
    for line in text.splitlines():
        out.say(f"  {line}")

    signal.signal(signal.SIGINT, signal.default_int_handler)
    result = 1
    try:
        out.say(f"\n[3/4] Copy the program into the test build's RAM ({REMOTE_DIR})")
        if app.ready(out, "Copies one program of about 1.1 MB.") != "go":
            raise Stop("stopped by you")
        rc, text = mpc.run(PREPARE, test=True)
        if rc != 0:
            raise Stop(f"could not make {REMOTE_DIR} ({text.strip()})")
        rc, text = mpc.run(None, test=True, stdin_path=path, command=RECEIVE)
        if rc != 0:
            raise Stop(f"copying failed ({text.strip()})")
        rc, text = mpc.run(HASH, test=True)
        if rc != 0 or text.split()[:1] != [digest]:
            raise Stop("the program on the MPC does not match the build")
        out.say("  copied; its hash on the MPC matches")

        out.say("\n[4/4] The disk test: WRITES test files in /run/mpfree-data/mpfree-disktest "
                "(stick partition 4), then deletes them")
        answer = app.ready(out, "Takes about 5 minutes; the MPC can stay as it is.")
        if answer == "quit":
            raise Stop("stopped by you")
        if answer == "skip":
            out.say("  skipped: nothing was written on the stick")
            result = 0
        else:
            rc, _ = app.stream(mpc, RUN, TIMEOUT, out)
            if rc == REFUSED:
                out.say("  the disk test refused (reason above); nothing was written on the stick")
                result = 2
            elif rc != 0:
                raise Stop(f"mpfree-disktest ended with exit code {rc}; if it was cut off, the next run "
                           "deletes its folder first (or: rm -r /run/mpfree-data/mpfree-disktest)")
            else:
                result = 0
    except (Stop, KeyboardInterrupt) as err:
        out.say(f"\nSTOP: {str(err) or 'interrupted (Ctrl-C)'}")
        result = 1
    rc, text = mpc.run(DF, test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    out.say("\nRESULT: " + {0: "the disk test ran (numbers above).",
                            2: "refused by the disk test; nothing was written on the stick.",
                            1: "stopped early."}[result])
    out.say("NEXT: carry on with the round, or switch the MPC off with its power switch, take out the stick,")
    out.say("wait 10 s, switch it on, and run the boot-map block to confirm nothing on the eMMC changed.")
    return 0 if result == 0 else 1


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:  # only reaches here in steps 1 and 2, before anything is copied
        print("\nSTOP: interrupted (Ctrl-C). Nothing was copied to the MPC.")
        sys.exit(130)
