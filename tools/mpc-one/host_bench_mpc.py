#!/usr/bin/env python3
"""mpfree-bench on the kexec test build: what every MPFree engine costs on the MPC One. RAM ONLY on the MPC.

Run it after `collect-over-ssh.sh --kexec-boot` has started the test build
from the USB stick. Like host_app_mpc.py, it talks only to the test build:
it uses the newest ~/mpfree-work/kexec-stick-* folder's known_hosts, so stock
3.6 (a different host key) is refused before anything runs. Each step after
the first waits for you to press Enter:

  1. Workstation, read-only: checks that both builds of the benchmark are
     static ARM EABI5 hard-float executables, and finds the test build's
     folder:
       default: rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-bench
       a17:     rust/target/a17/armv7-unknown-linux-gnueabihf/release/mpfree-bench
                (built with -C target-cpu=cortex-a17)
  2. Test build, read-only: host_app_mpc.py's checks (the test build's
     release, no MMC device, kexec off, / read-only, /run has room and allows
     programs, Akai's app is not running, who holds the sound, MIDI, touch
     and screen devices).
  3. Test build RAM: copies both programs to /run/mpfree-bench and checks
     their hashes there. Nothing else is written; a power-cycle clears it.
  4. For each build: `mpfree-bench --calibrate` (the reference kernels, a few
     seconds), then `mpfree-bench --json <build>.json` (every engine for 20 s
     of audio each, about 2 to 5 minutes; nobody knows exactly until it has
     run once), then `mpfree-bench --afford` (each synth preset and the sampler
     at several voice counts: what a voice costs and how many fit in each
     budget; writes nothing), then `mpfree-bench --lanes --lane-cores 3,2`
     (the same work on one core and split over two, about 5 minutes; it pins
     its own main thread to core 3 and its lane workers to core 2, and prints
     "not pinned (reason)" and runs unpinned if a pin is refused; writes
     nothing). Their output, and the JSON, go into this report.

The benchmark only computes. It opens no device and writes only its JSON
file (the --afford and --lanes runs write nothing), into /run/mpfree-bench
(RAM). The --lanes run sets the CPU affinity of its own threads only, which
ends with the process. It
runs as an ordinary process (no real-time priority), so the pad player's
audio thread (SCHED_FIFO), if it is running, always comes first: the
benchmark cannot make it underrun. For a clean run, after the copy it stops
our own pad player and log display (mpfree-test-app and mpfree-test-screen,
RAM state only; Akai's services are never touched) and starts whichever was
running again at the end, also when a run fails or you stop it.

Nothing here writes to the eMMC (the test build cannot see it), the U-Boot
environment, the SD card or the stick. To go back: switch the MPC off with its
power switch, take out the stick, wait 10 s, switch it on.
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

TARGET = os.path.join("armv7-unknown-linux-gnueabihf", "release", "mpfree-bench")
# The two builds, in the order they run: (name, where cargo puts it).
BUILDS = (
    ("default", os.path.join(REPO_ROOT, "rust", "target", TARGET)),
    ("a17", os.path.join(REPO_ROOT, "rust", "target", "a17", TARGET)),
)
REMOTE_DIR = "/run/mpfree-bench"
# The only runs this tool asks for: fixed arguments, nothing from the command line. (args, timeout in s)
RUNS = (
    ("--calibrate", 120),
    ("--json @@BUILD@@.json", 1200),
    ("--afford", 600),
    ("--lanes --lane-cores 3,2", 1200),
)
DEFAULT_SSH = {
    "ip": os.environ.get("MPC_IP", "10.42.0.141"),
    "port": int(os.environ.get("MPC_PORT", "2222")),
    "user": os.environ.get("MPC_USER", "root"),
    "key": os.environ.get("MPC_KEY", "/home/jesse/.ssh/mpc_one_autorun_ed25519"),
}

# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
# Writes only into /run/mpfree-bench (RAM).
PREPARE = r"""R=''
umask 077
d="$R/run/mpfree-bench"
[ -d "$d" ] || mkdir "$d" || exit 1
rm -f "$d/default" "$d/default.part" "$d/a17" "$d/a17.part" "$d/default.json" "$d/a17.json"
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-bench/@@BUILD@@.part" && chmod 700 "$R/run/mpfree-bench/@@BUILD@@.part" && mv "$R/run/mpfree-bench/@@BUILD@@.part" "$R/run/mpfree-bench/@@BUILD@@" """
HASH = r"""R=''
sha256sum "$R/run/mpfree-bench/@@BUILD@@"
"""
RUN = r"""R=''
cd "$R/run/mpfree-bench" || exit 1
p='@@BUILD@@'
exec "$R/run/mpfree-bench/$p" @@ARGS@@
"""
JSON = r"""R=''
cat "$R/run/mpfree-bench/@@BUILD@@.json" 2>/dev/null || echo "(no JSON file)"
"""


def check_program(out, name, path):
    """Step 1 for one build: its size, hash and ELF kind. Returns its sha256."""
    path = os.path.realpath(path)
    if not os.path.isfile(path):
        raise Stop(f"missing {path}: build it first (see docs/dev/design/profiling.md)")
    with open(path, "rb") as fh:
        data = fh.read(app.PROGRAM_MAX + 1)
    problem = app.arm_static_problem(data)
    if problem:
        raise Stop(f"{path} is {problem}; rebuild it as docs/dev/design/profiling.md shows")
    digest = hashlib.sha256(data).hexdigest()
    out.say(f"  {name}: {path}")
    out.say(f"    {len(data)} bytes, sha256 {digest}, static ARM EABI5 hard-float")
    return path, digest


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", default=DEFAULT_SSH["ip"])
    p.add_argument("--port", type=int, default=DEFAULT_SSH["port"])
    p.add_argument("--user", default=DEFAULT_SSH["user"])
    p.add_argument("--key", default=DEFAULT_SSH["key"])
    p.add_argument("--work", default=os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    p.add_argument("--default-program", default=BUILDS[0][1])
    p.add_argument("--a17-program", default=BUILDS[1][1])
    p.add_argument("--report")
    args = p.parse_args(argv)
    if os.environ.get("MPFREE_ACCEPT_BENCH") != "YES":
        raise Stop("opt-in required (copies the benchmark into the test build's RAM and runs it): "
                   "MPFREE_ACCEPT_BENCH=YES python3 tools/mpc-one/host_bench_mpc.py")
    if not app.TESTING and not sys.stdin.isatty():
        raise Stop("run this from a terminal: each step waits for you to press Enter")
    out = kexec.Out(args.report)
    out.say("MPFree benchmark on the test build (changes RAM on the MPC only)")
    total = 3 + len(BUILDS)

    out.say(f"\n[1/{total}] Workstation checks (read-only)")
    programs = [(name, *check_program(out, name, path))
                for name, path in (("default", args.default_program), ("a17", args.a17_program))]
    folder = kexec.newest_build(os.path.realpath(args.work))
    out.say(f"  test build: {folder}")
    with open(os.path.join(folder, "report.txt")) as fh:
        found = re.search(r"^its kernel release: (\S+)$", fh.read(), re.M)
    if not found or not kexec.RELEASE.match(found.group(1)):
        raise Stop(f"{folder}/report.txt does not name the test build's kernel release")
    mpc = kexec.Mpc(args, os.path.join(folder, "known_hosts"))

    out.say(f"\n[2/{total}] Checks on the test build (read-only; only its own host key is accepted)")
    rc, text = mpc.run(kexec.fill(app.CHECK, NEW=found.group(1)), test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc == 255 and "CHECKS-" not in text:
        raise Stop("the test build did not answer. Start it first with the --kexec-boot block; "
                   "on stock 3.6 this tool runs nothing")
    if rc != 0 or text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the test build is not ready; nothing was changed on it")

    signal.signal(signal.SIGINT, signal.default_int_handler)
    stopped = []
    try:
        out.say(f"\n[3/{total}] Copy both builds into the test build's RAM ({REMOTE_DIR})")
        if app.ready(out, "Copies two programs of about 1.3 MB each.") != "go":
            raise Stop("stopped by you")
        rc, text = mpc.run(PREPARE, test=True)
        if rc != 0:
            raise Stop(f"could not make {REMOTE_DIR} ({text.strip()})")
        for name, path, digest in programs:
            rc, text = mpc.run(None, test=True, stdin_path=path, command=kexec.fill(RECEIVE, BUILD=name))
            if rc != 0:
                raise Stop(f"copying {name} failed ({text.strip()})")
            rc, text = mpc.run(kexec.fill(HASH, BUILD=name), test=True)
            if rc != 0 or text.split()[:1] != [digest]:
                raise Stop(f"{name} on the MPC does not match the build")
            out.say(f"  {name}: copied; its hash on the MPC matches")

        # A clean run (the profiling thread's ask): our own pad player is stopped so its real-time audio
        # thread does not preempt the benchmark. It is started again at the end, whatever happens.
        stopped = []
        rc, text = mpc.run(app.SCREEN_STOP, test=True)
        if "pad player: was active" in text:
            stopped.append(app.APP_START)
        if "log display: was active" in text:
            stopped.append(app.SCREEN_START)
        for line in text.strip().splitlines():
            out.say(f"  {line}")
        out.say("  (for a clean run; whichever was running starts again at the end)")
        for n, (name, _, _) in enumerate(programs, start=4):
            out.say(f"\n[{n}/{total}] The {name} build: the reference kernels, every engine, then one core vs two (only computes)")
            answer = app.ready(out, "Takes about 10 to 15 minutes; the MPC can stay as it is.")
            if answer == "quit":
                raise Stop("stopped by you")
            if answer == "skip":
                continue
            for run_args, timeout in RUNS:
                rc, _ = app.stream(mpc, kexec.fill(RUN, BUILD=name, ARGS=kexec.fill(run_args, BUILD=name)),
                                     timeout, out)
                if rc != 0:
                    raise Stop(f"{name} {run_args.split()[0]} ended with exit code {rc}")
            rc, text = mpc.run(kexec.fill(JSON, BUILD=name), test=True)
            out.say(f"  ----- {name}.json -----")
            for line in text.splitlines():
                out.say(f"  {line}")
            out.say(f"  ----- end of {name}.json -----")
        result = 0
    except (Stop, KeyboardInterrupt) as err:
        out.say(f"\nSTOP: {str(err) or 'interrupted (Ctrl-C)'}")
        result = 1
    for script in stopped:
        rc, text = mpc.run(script, test=True)
        out.say(f"  {text.strip() or f'no answer, exit code {rc}'} (started again)")
    out.say("\nRESULT: " + ("both builds ran (numbers above)." if result == 0 else "stopped early."))
    out.say("NEXT: carry on with the round, or switch the MPC off with its power switch, take out the stick,")
    out.say("wait 10 s, switch it on, and run the boot-map block to confirm nothing on the eMMC changed.")
    return result


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
