#!/usr/bin/env python3
"""mpfree-disktest on the test build: how fast and how steadily the stick's partition 4 streams audio. Seven ways to run it:

  --survey-only   READ-ONLY. Steps 1 and 2 only: nothing is copied to or run on the MPC.
  --read-only     RAM ONLY on the MPC. Copies the program into the test build's RAM and runs
                  `mpfree-disktest --read-only`, which reads audio already on partition 4 and
                  WRITES NOTHING THERE. Needs MPFREE_ACCEPT_DISKTEST=READ-ONLY (or YES).
  (neither)       WRITES TEST FILES ON THE STICK'S PARTITION 4, THEN DELETES THEM.
                  Needs MPFREE_ACCEPT_DISKTEST=YES.
  --under-load    WRITES TEST FILES ON THE STICK'S PARTITION 4, THEN DELETES THEM, and keeps
                  the MPC's processor busy (it stops at 75 degrees C). Reads the audio on partition 4
                  while a recording, heavy reads and writes, busy cores and heavy task switching
                  run. Needs MPFREE_ACCEPT_DISKTEST=YES.
  --recorder      WRITES TEST TAKES ON THE STICK'S PARTITION 4, THEN DELETES THEM, and CHANGES
                  ONE SETTING IN THE RUNNING KERNEL for two of its steps (/proc/sys/vm/dirty_bytes,
                  put back as found; a restart puts it back too). Records 8 takes with MPFree's own
                  recorder while 8 streams read the audio on partition 4 (D4), the last two steps
                  with the reads given the disk ahead of the recorder. Needs
                  MPFREE_ACCEPT_DISKTEST=YES.
  --heat          KEEPS THE PROCESSOR BUSY until the temperature settles (30 minutes at the
                  most, stopping at 75 degrees C), reading the temperatures and clocks. Writes
                  no file anywhere. Needs MPFREE_ACCEPT_DISKTEST=YES.
  --sd-card       READS THE SD CARD IN THE MPC'S SLOT through its read-only window, then WRITES TEST
                  FILES IN THE CARD'S MPFree FOLDER AND ON THE STICK'S PARTITION 4 through the write
                  window, THEN DELETES THEM; the second pass keeps the processor busy (it stops at 75
                  degrees C). The write window refuses a card that is not exFAT, is marked "not safely
                  removed" or doesn't check clean, and checks it again after closing. Needs
                  MPFREE_ACCEPT_DISKTEST=YES.

Run it while the test build runs from the USB stick (started at power-on by
the boot hook on the installed 3.10.1, or by `collect-over-ssh.sh --kexec-boot`
on 3.6.0). Like host_bench_mpc.py, it talks only to the test build:
it uses the newest ~/mpfree-work/kexec-stick-* folder's known_hosts, so Akai's
system (a different host key) is refused before anything runs. Each step after
the second waits for you to press Enter:

  1. Workstation, read-only: checks that the program is a static ARM EABI5
     hard-float executable (not with --survey-only):
       rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-disktest
  2. Test build, read-only: host_app_mpc.py's checks (the test build's
     release, no MMC device, kexec off, / read-only, /run has room and allows
     programs, Akai's app is not running, who holds the sound, MIDI, touch
     and screen devices), `df -k /run/mpfree-data`, then the streaming survey
     (read-only-stream-survey.sh: the kernel, partition 4's mount, the stick's
     block queue and USB path, writeback settings, memory, the USB interrupts
     and their threads, the pad player's real-time threads, the CPUs, the
     disk's I/O counters, what else from a disk is mounted, whether Akai's
     disk service runs, and the kernel's messages about the stick; it reads
     procfs, sysfs, unit files and the kernel log only).
  3. Test build RAM: copies the program to /run/mpfree-disktest and checks
     its hash there. A power-cycle clears it.
  4a. With --read-only, READ-ONLY ON THE STICK: runs `mpfree-disktest
     --read-only`, about 7 minutes. It refuses (exit code 3) unless
     /run/mpfree-data is the mount of an ext4 file system labelled
     MPFREE-DATA on partition 4 of a USB disk, mounted noatime (or
     read-only), and no MMC device exists. It opens the audio files under
     /run/mpfree-data/library and the files under /run/mpfree-data/backup
     (a creator stick's eMMC backup, read as 64 MiB pieces) read-only
     (O_NOFOLLOW, never a link, never off that file system) and drops only
     the pages it is about to read from the cache. It creates, changes and
     deletes nothing there, and prints counts and sizes, never a file's
     name.
  4b. Otherwise, STICK, PARTITION 4: runs `mpfree-disktest` with no arguments, about 5
     minutes. It refuses (exit code 3, nothing written) unless
     /run/mpfree-data is the mount of an ext4 file system labelled
     MPFREE-DATA on partition 4 of a USB disk, no MMC device exists, and at
     least 64 MiB is free. Then it writes only inside
     /run/mpfree-data/mpfree-disktest (files it creates itself, O_EXCL and
     O_NOFOLLOW; at most 448 MiB at once and at most half the free space),
     and deletes that folder at the end, after an error, on Ctrl-C, or when
     the ssh session closes. It prints the free space before and after.
     It reads the partition's superblock (read-only) to check the label.
  4c. With --under-load, STICK, PARTITION 4, AND THE PROCESSOR: runs
     `mpfree-disktest --under-load`, about 7 minutes. It refuses (exit
     code 3, nothing written) on everything 4a and 4b refuse. Then, in 8
     steps of about 45 s, it reads the audio as 4a does (8 streams with
     128 KiB reads, then 24 with 512 KiB reads; a set that can't keep up
     stops at 60 s) while a recording writes 3.1 MB/s, and adds in turn: nothing, a heavy write, a heavy read, both,
     a busy thread on each core, heavy task switching (8 pairs of threads
     passing a byte, 32 threads waking every 200 microseconds), those two,
     and everything. It writes only rec.bin and copy.bin in
     /run/mpfree-data/mpfree-disktest, made new for each step and deleted
     after it (at most 448 MiB at once and at most half the free space;
     a few GB written in all, each step over the same space), and deletes the
     folder at the end, after an error, on Ctrl-C, or when the ssh session
     closes. It reads the temperature (/sys/class/thermal) every half
     second: the busy and switching threads stop at 75 degrees C, or the lowest
     trip point the kernel gives if that is lower, and the steps after are
     left out; with no temperature to read, steps 5 to 8 are left out.
  4d. With --recorder, STICK, PARTITION 4, AND ONE KERNEL SETTING: runs
     `mpfree-disktest --recorder`, about 9 minutes. It refuses (exit code 3,
     nothing written) on everything 4a and 4b refuse. Then, in 10 steps of
     about 45 s, MPFree's own recorder (the recording file protocol's
     writer) records 8 takes of stereo float at 48 kHz (3.07 MB/s) while 8
     streams read the audio as 4a does with 128 KiB reads: a round every
     1, 2, 5 and 10 s spread across the interval, every 2 and 5 s with the
     takes together, every 2 and 5 s with the kernel's writeback limit
     raised, and every 2 s with the streams' reads ahead of the recorder on
     the disk: at best effort 0 with the recorder's thread at 7, then in
     the real-time class (ioprio_set on the program's own threads for that
     step only, which end with it; nothing outside the program changes). It
     writes only its takes (recN_ID.wav.recording,
     recN_ID.wav.checkpoint and recN_ID.wav) in
     /run/mpfree-data/mpfree-disktest, made new for each step and deleted
     after it (about 126 MiB a step), and deletes the folder at the end,
     after an error, on Ctrl-C, or when the ssh session closes. For steps 7
     and 8 it writes 33554432 (32 MiB) to /proc/sys/vm/dirty_bytes, only if
     that holds a size under 32 MiB (1048576 on the MPC's stick at D0),
     and writes the value it found back after step 8, after an error, on
     Ctrl-C, or when the ssh session closes; otherwise those two steps are
     left out. That setting is in the running kernel only (RAM): a power
     cycle puts it back whatever happened. This tool reads the four
     writeback settings before and after the run (read-only) and says
     whether they are as they were.
  4e. With --heat, THE PROCESSOR ONLY: runs `mpfree-disktest --heat`, up
     to 30 minutes. It refuses (exit code 3) on everything 4a refuses that
     is not about the library, and when no temperature reads. One ordinary
     thread pinned to each core filters a buffer over and over (4c's step
     5), until the hottest zone has risen less than 0.5 degrees C in 5
     minutes, or for 30 minutes at the most; the threads stop at once at
     75 degrees C, or at the lowest trip point the kernel gives that is
     lower. Every 10 s it says each zone's temperature and each core's
     clock; each core's clock settings (scaling_cur_freq,
     scaling_max_freq, cpuinfo_max_freq, scaling_governor) at the start
     and the end. It reads /sys/class/thermal and /sys/devices/system/cpu
     only and writes no file. Ctrl-C or the ssh session closing stops it.
  4f. With --sd-card, THE SD CARD AND PARTITION 4: runs `mpfree-disktest
     --sd-card --card-wait 900`, about 20 minutes (step S2a of
     docs/dev/hardware/sd-card-and-internal-drive.md). It refuses (exit code
     3) on everything 4b refuses. With no card in the slot it asks again
     every 10 s for up to 15 minutes, so the card can go in after MPFree has
     started (Akai's system, starting first, can mark a card it sees). Part
     1 asks for the read-only window (step S1's, as the pad player does),
     checks the card is mounted read-only from the SD controller with no
     other MMC device, lists the card (counts only, never a name), reads
     its largest files (up to 256 MiB) and runs 8 paced streams over them,
     and needs the window to close saying nothing was written. Part 2 asks
     for the write window (setup-sd-card-write.sh), which refuses a card
     that is not one exFAT filesystem, carries the not-safely-removed mark
     or fails fsck.exfat -n, and leaves only the card's MPFree folder
     writable. In MPFree/mpfree-disktest on the card and
     /run/mpfree-data/mpfree-disktest on the stick (files it makes itself,
     O_EXCL and O_NOFOLLOW; at most a quarter of each side's free space, up
     to 128 MiB a file) it runs 14 steps, idle and then with a busy thread
     on each core: sequential writes and reads of each side, 8 streams
     playing and a recording at 3.1 MB/s on the card, both at once,
     copies each way, both sides written and read at once, playing one
     side while recording to the other, and saving 40 files of 1 MiB as
     MPFree will (written as NAME.part, fsync, renamed, the folder synced).
     It deletes both folders at the end, after an error, on Ctrl-C, or when
     the ssh session closes, and closes the window, which unmounts the card,
     checks the mark is clear and runs fsck.exfat -n again. This tool then
     prints both windows' own logs (read-only).
  Beside every set of streams (4a, 4c and 4d) it also reads the stick's
  partitions' block counters in sysfs (/sys/class/block/sdX/sdXN/stat and
  inflight, read-only) every 10 ms, and says what the other partitions
  read and wrote, and what was going on beside each read over 100 ms.
  All run as an ordinary process, so the pad player's audio thread, if
  it is running, comes first; the screen may answer slowly during steps
  5 to 8.

The read-only test (4a) reads like the streaming engine will: 4, 8, 16 and
24 streams of 24-bit stereo at 44.1 kHz, each with 64, 128, 256 and 512 KiB
reads, then 16 streams of 16-bit and of 32-bit float, 20 s each, after a
sequential read. Each stream reads its own files, or 64 MiB pieces of one,
paced, starting from a dropped cache. For each: p50, p99, p99.9, p99.99 and
max time per read, how many took over 50, 100, 250 and 500 ms, the worst
shortfall (how far a stream fell behind its schedule), the longest time a
stream stayed behind, and the read-ahead window and RAM the plan's rule
asks for.

The writing test (4b) measures, in 128 KiB blocks (the audio tracks' chunk): a sequential
write, a write paced at 3.1 MB/s (8 stereo tracks recording) for 60 s with
fdatasync every second, reads with the file's pages dropped from the cache
(sequential, then 8 paced streams), and 4 writing plus 4 reading streams at
once. For each: p50, p99 and max time per call, how many calls took over 50,
100, 250 and 500 ms, and how far behind schedule it fell (the RAM buffer a
recording or playback would need).

If the writing test is stopped hard (power off) mid-run, its folder stays on partition 4:
the next run deletes it first (only the tool's own file names; anything else
in it stops the tool). To delete it by hand on the test build:
rm -r /run/mpfree-data/mpfree-disktest

Nothing here writes to the eMMC (the test build cannot see it), the U-Boot
environment, or the stick's partitions 1 to 3, and only --sd-card writes to the
SD card, in its MPFree folder only. To go back: switch
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
# No path arguments: the program picks /run/mpfree-data itself and refuses anything else.
TIMEOUT = 900
# The run under load: about 7 minutes, longer if the stick can't keep up (each stream set stops at 60 s).
TIMEOUT_LOAD = 1500
# The recorder's run: about 9 minutes; each step's streams stop at 120 s and its takes finish within 60 s.
TIMEOUT_RECORDER = 1800
# The heat run: 30 minutes at the most, and its start and end.
TIMEOUT_HEAT = 2400
# The SD card run: up to 15 minutes waiting for the card, the write window's two checks of the whole card, and
# 14 steps twice (about 20 minutes).
TIMEOUT_SD = 3600
REFUSED = 3
DEFAULT_SSH = {
    "ip": os.environ.get("MPC_IP", "10.42.0.141"),
    "port": int(os.environ.get("MPC_TEST_PORT", "2222")),  # the test build's own SSH, not Akai's
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
# Reads only, on partition 4.
RUN_READ = r"""R=''
cd "$R/run/mpfree-disktest" || exit 1
exec "$R/run/mpfree-disktest/mpfree-disktest" --read-only
"""
# Reads partition 4's audio and writes only its own test files there, with the processor busy.
RUN_LOAD = r"""R=''
cd "$R/run/mpfree-disktest" || exit 1
exec "$R/run/mpfree-disktest/mpfree-disktest" --under-load
"""
# Reads partition 4's audio, writes only its own takes there, and raises /proc/sys/vm/dirty_bytes for two
# steps, putting it back as found.
RUN_RECORDER = r"""R=''
cd "$R/run/mpfree-disktest" || exit 1
exec "$R/run/mpfree-disktest/mpfree-disktest" --recorder
"""
# Keeps every core busy until the temperature settles; reads the temperatures and clocks, writes no file.
RUN_HEAT = r"""R=''
cd "$R/run/mpfree-disktest" || exit 1
exec "$R/run/mpfree-disktest/mpfree-disktest" --heat
"""
# The SD card: reads the card's own files through the read-only window, then writes only its own test files in
# the card's MPFree/mpfree-disktest and partition 4's mpfree-disktest through the write window, deleting them.
# A card put in up to 15 minutes after the start is waited for.
RUN_SD = r"""R=''
cd "$R/run/mpfree-disktest" || exit 1
exec "$R/run/mpfree-disktest/mpfree-disktest" --sd-card --card-wait 900
"""
# Read-only: both SD card windows' own logs and last statuses, after the SD card test.
WINDOW_LOGS = r"""R=''
for f in sd-card-log.txt sd-card-status sd-card-write-log.txt sd-card-write-status; do
  echo "--- /run/mpfree-setup/$f"
  if [ -f "$R/run/mpfree-setup/$f" ]; then tail -n 200 "$R/run/mpfree-setup/$f"; else echo "(none)"; fi
done
"""
# Read-only: the kernel's four writeback settings, before and after the recorder's run.
WRITEBACK = r"""R=''
for n in dirty_bytes dirty_ratio dirty_background_bytes dirty_background_ratio; do
  printf '%s=%s\n' "$n" "$(cat "$R/proc/sys/vm/$n" 2>/dev/null)"
done
"""
# Read-only: the streaming survey (step D0 of docs/dev/design/disk-streaming.md).
SURVEY = os.path.join(HERE, "read-only-stream-survey.sh")


def check_program(out, path):
    """Step 1: the program's size, hash and ELF kind. Returns (path, sha256)."""
    path = os.path.realpath(path)
    if not os.path.isfile(path):
        raise Stop(f"missing {path}: build it first (see docs/dev/design/disk-streaming.md)")
    with open(path, "rb") as fh:
        data = fh.read(app.PROGRAM_MAX + 1)
    problem = app.arm_static_problem(data)
    if problem:
        raise Stop(f"{path} is {problem}; rebuild it as docs/dev/design/disk-streaming.md shows")
    digest = hashlib.sha256(data).hexdigest()
    out.say(f"  {path}")
    out.say(f"    {len(data)} bytes, sha256 {digest}, static ARM EABI5 hard-float")
    return path, digest


def survey_script():
    """The streaming survey's text; every remote script starts with R=''."""
    with open(SURVEY) as fh:
        text = fh.read()
    if not text.startswith("R=''\n"):
        raise Stop(f"{SURVEY} does not start with R=''")
    return text


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", default=DEFAULT_SSH["ip"])
    p.add_argument("--port", type=int, default=DEFAULT_SSH["port"])
    p.add_argument("--user", default=DEFAULT_SSH["user"])
    p.add_argument("--key", default=DEFAULT_SSH["key"])
    p.add_argument("--work", default=os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    p.add_argument("--program", default=PROGRAM)
    p.add_argument("--report")
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--survey-only", action="store_true",
                      help="read-only: the checks and the streaming survey; nothing is copied or run")
    mode.add_argument("--read-only", action="store_true",
                      help="copy the program to the test build's RAM and run its read-only test")
    mode.add_argument("--under-load", action="store_true",
                      help="copy the program to the test build's RAM and run its test under load "
                           "(writes test files on partition 4, then deletes them; keeps the processor busy)")
    mode.add_argument("--recorder", action="store_true",
                      help="copy the program to the test build's RAM and run its recorder test (writes test "
                           "takes on partition 4, then deletes them; raises /proc/sys/vm/dirty_bytes for two "
                           "steps and puts it back)")
    mode.add_argument("--heat", action="store_true",
                      help="copy the program to the test build's RAM and keep every core busy until the "
                           "temperature settles (30 minutes at the most, stopping at 75 degrees C); writes no file")
    mode.add_argument("--sd-card", action="store_true",
                      help="copy the program to the test build's RAM and run its SD card test (reads the card's "
                           "files; writes test files in the card's MPFree folder and on partition 4, then deletes "
                           "them; keeps the processor busy for its second pass)")
    args = p.parse_args(argv)
    accept = os.environ.get("MPFREE_ACCEPT_DISKTEST")
    if args.read_only and accept not in ("YES", "READ-ONLY"):
        raise Stop("opt-in required (copies a program into the test build's RAM; reads audio on the stick's "
                   "partition 4 and writes nothing there): "
                   "MPFREE_ACCEPT_DISKTEST=READ-ONLY python3 tools/mpc-one/host_disktest_mpc.py --read-only")
    if args.under_load and accept != "YES":
        raise Stop("opt-in required (writes test files on the stick's partition 4, then deletes them, and keeps "
                   "the processor busy): "
                   "MPFREE_ACCEPT_DISKTEST=YES python3 tools/mpc-one/host_disktest_mpc.py --under-load")
    if args.recorder and accept != "YES":
        raise Stop("opt-in required (writes test takes on the stick's partition 4, then deletes them, and raises "
                   "one writeback setting in the running kernel for two steps, putting it back): "
                   "MPFREE_ACCEPT_DISKTEST=YES python3 tools/mpc-one/host_disktest_mpc.py --recorder")
    if args.heat and accept != "YES":
        raise Stop("opt-in required (keeps the processor busy until the temperature settles, 30 minutes at the "
                   "most, stopping at 75 degrees C; writes no file): "
                   "MPFREE_ACCEPT_DISKTEST=YES python3 tools/mpc-one/host_disktest_mpc.py --heat")
    if args.sd_card and accept != "YES":
        raise Stop("opt-in required (reads the SD card's files, writes test files in its MPFree folder and on the "
                   "stick's partition 4, then deletes them, and keeps the processor busy for a while): "
                   "MPFREE_ACCEPT_DISKTEST=YES python3 tools/mpc-one/host_disktest_mpc.py --sd-card")
    if not args.read_only and not args.survey_only and accept != "YES":
        raise Stop("opt-in required (writes test files on the stick's partition 4, then deletes them): "
                   "MPFREE_ACCEPT_DISKTEST=YES python3 tools/mpc-one/host_disktest_mpc.py")
    if not args.survey_only and not app.TESTING and not app.ROUND and not sys.stdin.isatty():
        raise Stop("run this from a terminal: each step waits for you to press Enter")
    out = kexec.Out(args.report)
    steps = 2 if args.survey_only else 4
    if args.survey_only:
        out.say("MPFree streaming survey on the test build (read-only; nothing is copied or run)")
    elif args.read_only:
        out.say("MPFree disk test on the test build, read-only (RAM, and reads of the audio on the stick's partition 4)")
    elif args.under_load:
        out.say("MPFree disk test under load on the test build (RAM, reads of the audio and test files on the "
                "stick's partition 4 that it deletes, and a busy processor)")
    elif args.recorder:
        out.say("MPFree recorder test on the test build (RAM, reads of the audio and test takes on the stick's "
                "partition 4 that it deletes, and one writeback setting in the running kernel, put back)")
    elif args.heat:
        out.say("MPFree heat test on the test build (RAM, and the processor kept busy until the temperature "
                "settles; no file written)")
    elif args.sd_card:
        out.say("MPFree SD card test on the test build (RAM, reads of the SD card, and test files in the card's "
                "MPFree folder and on the stick's partition 4 that it deletes)")
    else:
        out.say("MPFree disk test on the test build (RAM, and test files on the stick's partition 4 that it deletes)")

    out.say(f"\n[1/{steps}] Workstation checks (read-only)")
    survey = survey_script()
    if not args.survey_only:
        path, digest = check_program(out, args.program)
    folder = kexec.newest_build(os.path.realpath(args.work))
    out.say(f"  test build: {folder}")
    releases = app.test_build_releases(folder)
    mpc = kexec.Mpc(args, os.path.join(folder, "known_hosts"))

    out.say(f"\n[2/{steps}] Checks and the streaming survey on the test build (read-only; only its own host key is accepted)")
    rc, text = mpc.run(app.check_script(*releases), test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc == 255 and "CHECKS-" not in text:
        raise Stop(app.NOT_RUNNING)
    if rc != 0 or text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the test build is not ready; nothing was changed on it")
    rc, text = mpc.run(DF, test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    rc, text = mpc.run(survey, test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc != 0 or "END OF STREAM SURVEY" not in text:
        out.say(f"  (the survey ended early, exit code {rc}; nothing was changed)")
    if args.survey_only:
        out.say("\nRESULT: the survey ran (above); nothing was copied to or run on the MPC.")
        return 0 if rc == 0 else 1

    signal.signal(signal.SIGINT, signal.default_int_handler)
    result = 1
    writeback_before = None
    if args.recorder:
        rc, text = mpc.run(WRITEBACK, test=True)
        writeback_before = text.strip().splitlines()
        out.say("  writeback settings now: " + ", ".join(writeback_before))
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

        if args.read_only:
            out.say("\n[4/4] The read-only disk test: READS the audio in /run/mpfree-data/library and the "
                    "backup in /run/mpfree-data/backup (stick partition 4); writes nothing there")
            answer = app.ready(out, "Takes about 7 minutes; the MPC can stay as it is.")
        elif args.under_load:
            out.say("\n[4/4] The disk test under load: READS the audio in /run/mpfree-data/library and "
                    "/run/mpfree-data/backup, WRITES test files in /run/mpfree-data/mpfree-disktest (stick "
                    "partition 4) and deletes them, and keeps the processor busy (it stops at 75 degrees C)")
            answer = app.ready(out, "Takes about 7 minutes; the screen may answer slowly in steps 5 to 8.")
        elif args.recorder:
            out.say("\n[4/4] The recorder test: READS the audio in /run/mpfree-data/library and "
                    "/run/mpfree-data/backup, WRITES test takes in /run/mpfree-data/mpfree-disktest (stick "
                    "partition 4) and deletes them, and for steps 7 and 8 sets /proc/sys/vm/dirty_bytes to "
                    "33554432, then back to the value it found (RAM only)")
            answer = app.ready(out, "Takes about 9 minutes; the MPC can stay as it is.")
        elif args.heat:
            out.say("\n[4/4] The heat test: keeps every core busy (ordinary priority) until the hottest zone "
                    "rises less than 0.5 degrees C in 5 minutes, 30 minutes at the most, stopping at 75 degrees C; "
                    "reads the temperatures and clocks and writes no file")
            answer = app.ready(out, "Takes up to 30 minutes; the screen may answer slowly. The MPC can stay as it is.")
        elif args.sd_card:
            out.say("\n[4/4] The SD card test: READS the card's own files through the read-only window, then "
                    "WRITES test files in the card's MPFree/mpfree-disktest and in /run/mpfree-data/mpfree-disktest "
                    "(stick partition 4) through the write window and deletes them; the second pass keeps the "
                    "processor busy (it stops at 75 degrees C). With no card in the slot it waits up to 15 minutes "
                    "for one")
            answer = app.ready(out, "Takes about 20 minutes; put the SD card in the MPC's slot now if it isn't.")
        else:
            out.say("\n[4/4] The disk test: WRITES test files in /run/mpfree-data/mpfree-disktest "
                    "(stick partition 4), then deletes them")
            answer = app.ready(out, "Takes about 5 minutes; the MPC can stay as it is.")
        if answer == "quit":
            raise Stop("stopped by you")
        if answer == "skip":
            out.say("  skipped: nothing was written on the stick")
            result = 0
        else:
            script = RUN_READ if args.read_only else RUN_LOAD if args.under_load \
                else RUN_RECORDER if args.recorder else RUN_HEAT if args.heat else RUN_SD if args.sd_card else RUN
            timeout = TIMEOUT_LOAD if args.under_load else TIMEOUT_RECORDER if args.recorder \
                else TIMEOUT_HEAT if args.heat else TIMEOUT_SD if args.sd_card else TIMEOUT
            rc, _ = app.stream(mpc, script, timeout, out)
            if args.sd_card:
                out.say("  the SD card windows' own logs (read-only):")
                _, text = mpc.run(WINDOW_LOGS, test=True)
                for line in text.splitlines():
                    out.say(f"    {line}")
            if rc == REFUSED and args.sd_card:
                out.say("  the SD card test refused (reason above); it wrote no test file")
                result = 2
            elif rc != 0 and args.sd_card:
                raise Stop(f"mpfree-disktest --sd-card ended with exit code {rc} (reason above); if it was cut off, "
                           "the next run deletes its folders first")
            elif rc == REFUSED:
                out.say("  the disk test refused (reason above); nothing was written on the stick")
                result = 2
            elif rc != 0 and args.read_only:
                raise Stop(f"mpfree-disktest --read-only ended with exit code {rc}; it writes nothing on the stick")
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
    if writeback_before is not None:
        rc, text = mpc.run(WRITEBACK, test=True)
        after = text.strip().splitlines()
        if rc == 0 and after == writeback_before:
            out.say("  writeback settings as before the run: " + ", ".join(after))
        else:
            out.say("  WRITEBACK SETTINGS CHANGED: " + ", ".join(after) + " (before: " + ", ".join(writeback_before)
                    + "). They are in the running kernel only: switching the MPC off and on puts them back.")
            result = 1
    ran = "the read-only disk test ran (numbers above); nothing was written on the stick." if args.read_only \
        else "the disk test under load ran (numbers above)." if args.under_load \
        else "the recorder test ran (numbers above)." if args.recorder \
        else "the heat test ran (numbers above); it wrote no file." if args.heat \
        else "the SD card test ran (numbers above); the card checked clean after." if args.sd_card \
        else "the disk test ran (numbers above)."
    stopped = "stopped early; the read-only test writes nothing on the stick." if args.read_only else "stopped early."
    out.say("\nRESULT: " + {0: ran, 2: "refused by the disk test; nothing was written on the stick.",
                            1: stopped}[result])
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
