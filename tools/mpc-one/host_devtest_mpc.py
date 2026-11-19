#!/usr/bin/env python3
"""The device round's workflows, played by mpfree itself on the test build. RAM ONLY on the MPC.

Run it while the test build runs from the USB stick, after host_bench_mpc.py
(the round's order is in docs/dev/hardware/device-suite.md). Like the other
host tools it talks only to the test build: it uses the newest
~/mpfree-work/kexec-stick-* or creator-stick-* folder's known_hosts, so Akai's
system (a different host key) is refused before anything runs. It asks for
Enter three times; everything else runs by itself.

  1. Workstation, read-only: checks that the program
     (rust/target/armv7-unknown-linux-gnueabihf/release/mpfree, built from
     the same commit as the workflows) is a static ARM EABI5 hard-float
     executable, lists the workflows (tools/mpc-one/devtest/workflows/*.mpw)
     with their hashes, and finds the test build's folder.
  2. Test build, read-only: host_app_mpc.py's checks (the test build's
     release, no MMC device, kexec off, / read-only, /run has room and allows
     programs, Akai's app is not running, who holds the sound, MIDI, touch
     and screen devices), and that the stick's kits are mounted where the
     pad player reads them (/run/mpfree-kits/kits.bin, read-only).
  3. Test build RAM: copies the program and the workflows to
     /run/mpfree-devtest and checks every hash there. Then it stops our own
     pad player and log display (mpfree-test-app and mpfree-test-screen, RAM
     state only; Akai's services are never touched), because the workflows'
     mpfree needs the screen, the pads and the sound card; whichever was
     running starts again at the end, also when a run fails or you stop it.
  4. Every workflow, one after the other, on the default sound cores:
     `mpfree play --script WORKFLOW --kits /run/mpfree-kits/kits.bin
     --data /run/mpfree-devtest/data-NAME --gain-db -12`. Each plays its
     steps by itself (buttons, pads, Q-Links, the wheel and taps on the screen
     go in where the hardware's do), says each step in its log, and stops on
     its own a few seconds after its last step. Its whole log goes into this
     report. A workflow that fails is reported and the next one starts. Then
     one question: did anything click, drop out or sound wrong.
  5. heavy-load again on each sound-core setting (--sound-cores shared,
     core3, two, three), the same steps each time, so the report can pick the
     default from the numbers.

What mpfree writes in these runs goes to /run/mpfree-devtest (RAM): each
workflow gets an empty data folder of its own there, so partition 4 (your
projects and settings on the stick, /run/mpfree-data) is never opened, and
the stick's kits are only read. Nothing here writes to the eMMC (the test
build cannot see it), the U-Boot environment, the SD card or the stick. To go
back: switch the MPC off with its power switch, take out the stick, wait 10 s,
switch it on.

Read the report with tools/mpc-one/devtest_report.py (the bench report and
this one together).

MPFREE_DEVTEST_ONLY=NAME,NAME plays only those workflows in step 4;
--no-cores skips step 5.
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

WORKFLOWS = os.path.join(HERE, "devtest", "workflows")
REMOTE_DIR = "/run/mpfree-devtest"
NAME = re.compile(r"^[a-z0-9][a-z0-9-]{0,40}$")
WORKFLOW_MAX = 64 << 10
# The workflow played again on each sound-core setting, and the settings, in order.
CORES_WORKFLOW = "heavy-load"
CORES = ("shared", "core3", "two", "three")
# How long one run may take before it is stopped: the longest script CI allows (600 s), the app's start and
# its few seconds after the last step.
RUN_TIMEOUT = 900
LISTEN = ("Did anything click, drop out, stick or sound wrong while they played? Name the workflow and the "
          "part the terminal showed (or just press Enter if nothing did).")

# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
# Writes only into /run/mpfree-devtest (RAM).
PREPARE = r"""R=''
umask 077
d="$R/run/mpfree-devtest"
[ -d "$d" ] || mkdir "$d" || exit 1
rm -rf "$d/mpfree" "$d/mpfree.part" "$d/workflows" "$d"/data-*
mkdir "$d/workflows" || exit 1
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-devtest/mpfree.part" && chmod 700 "$R/run/mpfree-devtest/mpfree.part" && mv "$R/run/mpfree-devtest/mpfree.part" "$R/run/mpfree-devtest/mpfree" """
RECEIVE_WORKFLOW = r"""R=''; umask 077; cat > "$R/run/mpfree-devtest/workflows/@@NAME@@.mpw" """
HASHES = r"""R=''
cd "$R/run/mpfree-devtest" || exit 1
sha256sum mpfree workflows/*.mpw
"""
# Read-only: the stick's kits, where its pad player reads them (partition 3, mounted read-only at start).
KITS = r"""R=''
k="$R/run/mpfree-kits/kits.bin"
if [ -s "$k" ]; then echo "kits: at /run/mpfree-kits/kits.bin"; else echo "kits: missing"; fi
"""
RUN = r"""R=''
d="$R/run/mpfree-devtest"
rm -rf "$d/data-@@NAME@@" && mkdir "$d/data-@@NAME@@" || exit 1
cd "$d" || exit 1
exec "$R/run/mpfree-devtest/mpfree" play --script "$d/workflows/@@WORKFLOW@@.mpw" --kits "$R/run/mpfree-kits/kits.bin" --data "$d/data-@@NAME@@" --gain-db -12@@CORES@@
"""
# After an interruption: ends a workflow's mpfree still running from this folder (its ssh is gone), so the
# pad player can have the sound card back. Only that program; nothing else is signalled.
END_RUN = r"""R=''
for p in "$R"/proc/[0-9]*; do
  [ "$(readlink "$p/exe" 2>/dev/null)" = "$R/run/mpfree-devtest/mpfree" ] || continue
  kill "${p##*/}" 2>/dev/null && echo "ended a workflow still running (pid ${p##*/})"
done
"""


def check_program(out, path):
    """Step 1 for the program: its size, hash and ELF kind. Returns (path, sha256)."""
    path = os.path.realpath(path)
    if not os.path.isfile(path):
        raise Stop(f"missing {path}: build it first (cross-build mpfree-app, docs/dev/hardware/device-suite.md)")
    with open(path, "rb") as fh:
        data = fh.read(app.PROGRAM_MAX + 1)
    problem = app.arm_static_problem(data)
    if problem:
        raise Stop(f"{path} is {problem}; rebuild it as docs/dev/hardware/device-suite.md shows")
    digest = hashlib.sha256(data).hexdigest()
    out.say(f"  program: {path}")
    out.say(f"    {len(data)} bytes, sha256 {digest}, static ARM EABI5 hard-float")
    return path, digest


def seconds(text):
    """About how long a workflow plays: its waits, holds and taps (the app prints the exact length)."""
    total = 0.0
    for raw in text.splitlines():
        words = raw.split("#", 1)[0].split()
        if not words:
            continue
        verb, rest = words[0], words[1:]
        try:
            if verb == "wait":
                total += float(rest[0])
            elif verb == "hold":
                total += float(rest[0]) + 0.05
            elif verb == "pad":
                total += (float(rest[2]) if len(rest) > 2 else 0.1) + 0.03
            elif verb == "tap":
                total += 0.25
            elif verb in ("button", "qlink", "wheel", "pad-down", "pad-up", "button-down", "button-up", "gear"):
                total += 0.1
        except (IndexError, ValueError):
            raise Stop(f"a workflow's {verb} line does not read: {raw.strip()!r}")
    return total


def workflows(out, folder, only):
    """Step 1 for the workflows: [(name, path, sha256, seconds)], sorted, or only those named."""
    found = sorted(f[:-4] for f in os.listdir(folder) if f.endswith(".mpw")) if os.path.isdir(folder) else []
    if not found:
        raise Stop(f"no workflows in {folder}")
    for name in found:
        if not NAME.match(name):
            raise Stop(f"{name}.mpw: a workflow's name is lower-case letters, digits and dashes")
    if only:
        missing = [n for n in only if n not in found]
        if missing:
            raise Stop("MPFREE_DEVTEST_ONLY names no workflow called " + ", ".join(missing))
        found = [n for n in found if n in only]
    listed = []
    for name in found:
        path = os.path.join(folder, f"{name}.mpw")
        with open(path, "rb") as fh:
            data = fh.read(WORKFLOW_MAX + 1)
        if len(data) > WORKFLOW_MAX:
            raise Stop(f"{name}.mpw is over {WORKFLOW_MAX >> 10} KiB")
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            raise Stop(f"{name}.mpw is not UTF-8 text")
        digest = hashlib.sha256(data).hexdigest()
        length = seconds(text)
        title = next((ln[6:].strip() for ln in text.splitlines() if ln.startswith("title ")), "(no title)")
        out.say(f"  {name}: about {length:.0f} s, sha256 {digest[:16]}: {title}")
        listed.append((name, path, digest, length))
    return listed


def play(mpc, out, workflow, name, cores=None):
    """One workflow run, its log streamed into the report between two marker lines. Returns its exit code."""
    label = f"{workflow} (sound cores: {cores or 'default'})"
    out.say(f"  ----- workflow {label} -----")
    rc, _ = app.stream(mpc, kexec.fill(RUN, NAME=name, WORKFLOW=workflow,
                                       CORES=f" --sound-cores {cores}" if cores else ""), RUN_TIMEOUT, out)
    out.say(f"  ----- end of workflow {label}: exit code {rc} -----")
    return rc


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", default=os.environ.get("MPC_IP", "10.42.0.141"))
    p.add_argument("--port", type=int, default=int(os.environ.get("MPC_TEST_PORT", "2222")))
    p.add_argument("--user", default=os.environ.get("MPC_USER", "root"))
    p.add_argument("--key", default=os.environ.get("MPC_KEY", "/home/jesse/.ssh/mpc_one_autorun_ed25519"))
    p.add_argument("--work", default=os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    p.add_argument("--program", default=app.PROGRAM_BUILT)
    p.add_argument("--workflows", default=WORKFLOWS)
    p.add_argument("--no-cores", action="store_true", help="skip heavy-load on each sound-core setting")
    p.add_argument("--report")
    args = p.parse_args(argv)
    only = [n for n in os.environ.get("MPFREE_DEVTEST_ONLY", "").split(",") if n]
    if os.environ.get("MPFREE_ACCEPT_DEVTEST") != "YES":
        raise Stop("opt-in required (copies mpfree and the workflows into the test build's RAM and plays them): "
                   "MPFREE_ACCEPT_DEVTEST=YES python3 tools/mpc-one/host_devtest_mpc.py")
    if not app.TESTING and not sys.stdin.isatty():
        raise Stop("run this from a terminal: it asks for Enter before the copy and before each part")
    out = kexec.Out(args.report)
    out.say("MPFree device round workflows on the test build (changes RAM on the MPC only)")
    total = 5

    out.say(f"\n[1/{total}] Workstation checks (read-only)")
    program, digest = check_program(out, args.program)
    listed = workflows(out, os.path.realpath(args.workflows), only)
    cores = [] if args.no_cores else [c for c in CORES if any(n == CORES_WORKFLOW for n, _, _, _ in listed)]
    folder = kexec.newest_build(os.path.realpath(args.work))
    out.say(f"  test build: {folder}")
    releases = app.test_build_releases(folder)
    mpc = kexec.Mpc(args, os.path.join(folder, "known_hosts"))

    out.say(f"\n[2/{total}] Checks on the test build (read-only; only its own host key is accepted)")
    rc, text = mpc.run(app.check_script(*releases), test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc == 255 and "CHECKS-" not in text:
        raise Stop(app.NOT_RUNNING)
    if rc != 0 or text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the test build is not ready; nothing was changed on it")
    rc, text = mpc.run(KITS, test=True)
    out.say(f"  {text.strip()}")
    if rc != 0 or not text.startswith("kits: ") or "missing" in text:
        raise Stop("the stick's kits are not at /run/mpfree-kits/kits.bin (a stick made with --app mounts them "
                   "at start); nothing was changed on the test build")

    signal.signal(signal.SIGINT, signal.default_int_handler)
    stopped = []
    failed = []
    played = 0
    try:
        out.say(f"\n[3/{total}] Copy mpfree and {len(listed)} workflows into the test build's RAM ({REMOTE_DIR})")
        if app.ready(out, "Copies the program (about 10 MB) and the workflows.") != "go":
            raise Stop("stopped by you")
        rc, text = mpc.run(PREPARE, test=True)
        if rc != 0:
            raise Stop(f"could not make {REMOTE_DIR} ({text.strip()})")
        rc, text = mpc.run(None, test=True, stdin_path=program, command=RECEIVE)
        if rc != 0:
            raise Stop(f"copying the program failed ({text.strip()})")
        for name, path, _, _ in listed:
            rc, text = mpc.run(None, test=True, stdin_path=path, command=kexec.fill(RECEIVE_WORKFLOW, NAME=name))
            if rc != 0:
                raise Stop(f"copying {name}.mpw failed ({text.strip()})")
        rc, text = mpc.run(HASHES, test=True)
        on_mpc = {}
        for line in text.splitlines():
            parts = line.split()
            if len(parts) == 2:
                on_mpc[parts[1]] = parts[0]
        want = {"mpfree": digest, **{f"workflows/{n}.mpw": d for n, _, d, _ in listed}}
        if rc != 0 or on_mpc != want:
            raise Stop("what is on the MPC does not match the program and the workflows here")
        out.say(f"  copied; the program's and {len(listed)} workflows' hashes on the MPC match")
        rc, text = mpc.run(app.SCREEN_STOP, test=True)
        if "pad player: was active" in text:
            stopped.append(app.APP_START)
        if "log display: was active" in text:
            stopped.append(app.SCREEN_START)
        for line in text.strip().splitlines():
            out.say(f"  {line}")
        out.say("  (mpfree needs the screen, pads and sound; whichever was running starts again at the end)")

        minutes = sum(s for _, _, _, s in listed) / 60 + len(listed) * 0.2
        out.say(f"\n[4/{total}] The workflows, one after the other, on the default sound cores")
        out.say("  Each plays by itself: the screen changes, the pads light and the MPC sounds. Keep the volume")
        out.say("  where you can hear it. The terminal shows each part (the 'mark' lines) as it starts.")
        if app.ready(out, f"{len(listed)} workflows, about {minutes:.0f} minutes; hands off the MPC.") != "go":
            raise Stop("stopped by you")
        for name, _, _, _ in listed:
            rc = play(mpc, out, name, name)
            played += 1
            if rc != 0:
                failed.append(f"{name} (exit code {rc})")
        out.say("")
        app.note(out, LISTEN)

        if cores:
            length = next(s for n, _, _, s in listed if n == CORES_WORKFLOW)
            out.say(f"\n[5/{total}] {CORES_WORKFLOW} on each sound-core setting: " + ", ".join(cores))
            answer = app.ready(out, f"{len(cores)} runs, about {len(cores) * (length / 60 + 0.2):.0f} minutes; hands off the MPC.")
            if answer == "quit":
                raise Stop("stopped by you")
            for c in (cores if answer == "go" else []):
                rc = play(mpc, out, CORES_WORKFLOW, f"{CORES_WORKFLOW}-{c}", c)
                played += 1
                if rc != 0:
                    failed.append(f"{CORES_WORKFLOW} on {c} (exit code {rc})")
        result = 0 if not failed else 1
    except (Stop, KeyboardInterrupt) as err:
        out.say(f"\nSTOP: {str(err) or 'interrupted (Ctrl-C)'}")
        rc, text = mpc.run(END_RUN, test=True)
        if text.strip():
            out.say(f"  {text.strip()}")
        result = 1
    for script in stopped:
        rc, text = mpc.run(script, test=True)
        out.say(f"  {text.strip() or f'no answer, exit code {rc}'} (started again)")
    if failed:
        out.say("\nFAILED: " + ", ".join(failed))
    out.say(f"\nRESULT: {played} runs played" + ("." if result == 0 else ", stopped early or with failures (above)."))
    out.say("NEXT: python3 tools/mpc-one/devtest_report.py with this report and the bench report; then carry")
    out.say("on with the round, or switch the MPC off with its power switch, take out the stick, wait 10 s,")
    out.say("switch it on, and run the boot-map block to confirm nothing on the eMMC changed.")
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
