#!/usr/bin/env python3
"""D3: the whole app streaming on the stick. STATE-CHANGING: the stick's partition 4 only, in its own folder.

The disk streaming plan's device session D3 (docs/dev/design/disk-streaming.md#d3-the-app-streaming-on-the-stick):
mpfree itself plays the stream test's workflows (tools/mpc-one/streamtest/workflows/*.mpw) on the test build, as
host_devtest_mpc.py plays the device round's, with one difference: each run's data folder is on the stick's
partition 4, so the takes it records go to the stick and play back from their files there, with the stream
layer on ("Audio tracks from their files", for that run only). Like the other host tools it talks only to the
test build: it uses the newest ~/mpfree-work/kexec-stick-* or creator-stick-* folder's known_hosts, so Akai's
system (a different host key) is refused before anything runs. It asks for Enter twice; everything else runs by
itself.

  1. Workstation, read-only: checks that the program
     (rust/target/armv7-unknown-linux-gnueabihf/release/mpfree, built from the same commit as the workflows) is a
     static ARM EABI5 hard-float executable, lists the workflows with their hashes, and finds the test build.
  2. Test build, read-only: host_app_mpc.py's checks (the test build's release, no MMC device, kexec off, /
     read-only, /run has room and allows programs, Akai's app is not running, who holds the sound, MIDI, touch
     and screen devices), the stick's kits (/run/mpfree-kits/kits.bin), and partition 4: mounted at
     /run/mpfree-data, ext4, read-write, partition 4 of a USB disk, with at least 4 GiB free.
  3. Test build RAM: copies the program and the workflows to /run/mpfree-streamtest and checks every hash. Then
     it stops our own pad player and log display (mpfree-test-app and mpfree-test-screen, RAM state only; Akai's
     services are never touched), because the workflows' mpfree needs the screen, the pads and the sound card;
     whichever was running starts again at the end, also when a run fails or you stop it.
  4. Every workflow, one after the other: `mpfree play --script WORKFLOW --kits /run/mpfree-kits/kits.bin
     --data /run/mpfree-data/mpfree-streamtest/data-NAME --gain-db -12`. Each plays its steps by itself, says
     each step and every 30 s its totals in its log (the audio tracks' line counts what played from the files and
     what went silent because the stick was late), and stops a few seconds after its last step. Its whole log
     goes into this report. Then its data folder is deleted, and the free space said. A workflow that fails is
     reported and the next one starts. Then one question: did anything click, drop out or sound wrong.

What it changes:
  - RAM: /run/mpfree-streamtest (the program and the workflows), gone at the next power-off.
  - The stick's partition 4: only /run/mpfree-data/mpfree-streamtest, each workflow's own data folder (its
    library, its settings, the takes it records and the project it saves: under 1 GiB at once). It is
    deleted after each workflow, at the end, after an error and on Ctrl-C; one a cut-off run left is deleted
    before the first workflow. Nothing else on partition 4 (your projects, settings, library and backup) is
    opened: each run's mpfree has its own data folder.
  - Nothing here writes to the eMMC (the test build cannot see it), the U-Boot environment or the SD card.

To go back after a cut (power off or the stick pulled mid-run): run this again (it deletes the folder first), or
switch the MPC off with its power switch, take out the stick, wait 10 s and switch it on: the folder then stays
on the stick until the next run.

MPFREE_STREAMTEST_ONLY=NAME,NAME plays only those workflows.
"""
import argparse
import importlib.util
import os
import signal
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("host_devtest_mpc", os.path.join(HERE, "host_devtest_mpc.py"))
devtest = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(devtest)
app = devtest.app
kexec = app.kexec
Stop = kexec.Stop

WORKFLOWS = os.path.join(HERE, "streamtest", "workflows")
REMOTE_DIR = "/run/mpfree-streamtest"
DATA_DIR = "/run/mpfree-data/mpfree-streamtest"
# The free space partition 4 needs before the first workflow: every workflow's data folder is deleted before the
# next starts, and the biggest (eight-tracks: about 190 MB of takes, then the project's copy of them) stays under
# 1 GiB; the rest is room to spare.
NEED_KIB = 4 << 20
RUN_TIMEOUT = 1200
LISTEN = ("Did anything click, drop out, stick or sound wrong while they played? Name the workflow and the "
          "part the terminal showed (or just press Enter if nothing did).")

# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
# Read-only: partition 4 as the workflows need it: mounted at /run/mpfree-data, ext4, read-write, partition 4 of
# a USB disk, with room. Says whether a cut-off run left its folder.
DATA = r"""R=''
dev=''; type=''; opts=''
while read -r d m t o rest; do
  [ "$m" = /run/mpfree-data ] && { dev=$d; type=$t; opts=$o; }
done < "$R/proc/mounts"
[ -n "$dev" ] || { echo "partition 4: not mounted at /run/mpfree-data"; exit 1; }
echo "partition 4: $dev at /run/mpfree-data, $type, $opts"
[ "$type" = ext4 ] || { echo "partition 4: not ext4"; exit 1; }
case ",$opts," in *,rw,*) ;; *) echo "partition 4: not read-write"; exit 1 ;; esac
name=${dev#/dev/}
case "$dev" in /dev/*) ;; *) name='' ;; esac
case "$name" in ''|*/*) echo "partition 4: $dev is not a /dev block device"; exit 1 ;; esac
[ "$(cat "$R/sys/class/block/$name/partition" 2>/dev/null)" = 4 ] || { echo "partition 4: $dev is not partition 4"; exit 1; }
case "$(readlink "$R/sys/class/block/$name" 2>/dev/null)" in
  */usb*) echo "partition 4: $dev is on a USB disk" ;;
  *) echo "partition 4: $dev is not on a USB disk"; exit 1 ;;
esac
avail=$(df -k "$R/run/mpfree-data" | tail -n 1 | while read -r a b c d rest; do echo "$d"; done)
echo "partition 4: $avail KiB free"
[ "${avail:-0}" -ge @@NEED@@ ] 2>/dev/null || { echo "partition 4: less than @@NEED@@ KiB free"; exit 1; }
if [ -e "$R/run/mpfree-data/mpfree-streamtest" ]; then
  echo "partition 4: /run/mpfree-data/mpfree-streamtest is left from a run that was cut off (deleted before the first workflow)"
fi
echo DATA-OK
"""
# Writes only into /run/mpfree-streamtest (RAM) and /run/mpfree-data/mpfree-streamtest (its own folder on p4).
PREPARE = r"""R=''
umask 077
d="$R/run/mpfree-streamtest"
[ -d "$d" ] || mkdir "$d" || exit 1
rm -rf "$d/mpfree" "$d/mpfree.part" "$d/workflows"
mkdir "$d/workflows" || exit 1
rm -rf "$R/run/mpfree-data/mpfree-streamtest" || exit 1
mkdir "$R/run/mpfree-data/mpfree-streamtest" || exit 1
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-streamtest/mpfree.part" && chmod 700 "$R/run/mpfree-streamtest/mpfree.part" && mv "$R/run/mpfree-streamtest/mpfree.part" "$R/run/mpfree-streamtest/mpfree" """
RECEIVE_WORKFLOW = r"""R=''; umask 077; cat > "$R/run/mpfree-streamtest/workflows/@@NAME@@.mpw" """
HASHES = r"""R=''
cd "$R/run/mpfree-streamtest" || exit 1
sha256sum mpfree workflows/*.mpw
"""
RUN = r"""R=''
d="$R/run/mpfree-data/mpfree-streamtest"
[ -d "$d" ] || exit 1
rm -rf "$d/data-@@NAME@@" && mkdir "$d/data-@@NAME@@" || exit 1
cd "$R/run/mpfree-streamtest" || exit 1
exec "$R/run/mpfree-streamtest/mpfree" play --script "$R/run/mpfree-streamtest/workflows/@@NAME@@.mpw" --kits "$R/run/mpfree-kits/kits.bin" --data "$d/data-@@NAME@@" --gain-db -12
"""
# After each workflow: its data folder goes; then what partition 4 has free.
CLEAR_RUN = r"""R=''
rm -rf "$R/run/mpfree-data/mpfree-streamtest/data-@@NAME@@"
if [ -e "$R/run/mpfree-data/mpfree-streamtest/data-@@NAME@@" ]; then echo "data folder: NOT deleted"; exit 1; fi
df -k "$R/run/mpfree-data" | tail -n 1 | while read -r a b c d rest; do echo "data folder: deleted; partition 4: $d KiB free"; done
"""
# At the end, after an error and on Ctrl-C: the whole folder goes.
CLEAR_ALL = r"""R=''
rm -rf "$R/run/mpfree-data/mpfree-streamtest"
if [ -e "$R/run/mpfree-data/mpfree-streamtest" ]; then echo "/run/mpfree-data/mpfree-streamtest: NOT deleted"; exit 1; fi
df -k "$R/run/mpfree-data" | tail -n 1 | while read -r a b c d rest; do echo "/run/mpfree-data/mpfree-streamtest: deleted; partition 4: $d KiB free"; done
"""
# After an interruption: ends a workflow's mpfree still running from this folder (its ssh is gone), so its data
# folder can go and the pad player can have the sound card back. Only that program; nothing else is signalled.
END_RUN = r"""R=''
for p in "$R"/proc/[0-9]*; do
  [ "$(readlink "$p/exe" 2>/dev/null)" = "$R/run/mpfree-streamtest/mpfree" ] || continue
  kill "${p##*/}" 2>/dev/null && echo "ended a workflow still running (pid ${p##*/})"
done
sleep 2
"""


def play(mpc, out, name):
    """One workflow run, its log streamed into the report between two marker lines, then its data folder
    deleted. Returns (exit code, whether the folder went)."""
    # The device round's markers, so tools/mpc-one/devtest_report.py reads these runs too.
    out.say(f"  ----- workflow {name} (sound cores: default) -----")
    rc, _ = app.stream(mpc, kexec.fill(RUN, NAME=name), RUN_TIMEOUT, out)
    out.say(f"  ----- end of workflow {name} (sound cores: default): exit code {rc} -----")
    crc, text = mpc.run(kexec.fill(CLEAR_RUN, NAME=name), test=True)
    out.say(f"  {text.strip() or f'data folder: no answer, exit code {crc}'}")
    return rc, crc == 0


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", default=os.environ.get("MPC_IP", "10.42.0.141"))
    p.add_argument("--port", type=int, default=int(os.environ.get("MPC_TEST_PORT", "2222")))
    p.add_argument("--user", default=os.environ.get("MPC_USER", "root"))
    p.add_argument("--key", default=os.environ.get("MPC_KEY", "/home/jesse/.ssh/mpc_one_autorun_ed25519"))
    p.add_argument("--work", default=os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    p.add_argument("--program", default=app.PROGRAM_BUILT)
    p.add_argument("--workflows", default=WORKFLOWS)
    p.add_argument("--report")
    args = p.parse_args(argv)
    only = [n for n in os.environ.get("MPFREE_STREAMTEST_ONLY", "").split(",") if n]
    if os.environ.get("MPFREE_ACCEPT_STREAMTEST") != "YES":
        raise Stop("opt-in required (copies mpfree and the workflows into the test build's RAM and plays them "
                   "with their data on the stick's partition 4, in its own folder, deleted after): "
                   "MPFREE_ACCEPT_STREAMTEST=YES python3 tools/mpc-one/host_streamtest_mpc.py")
    if not app.TESTING and not app.ROUND and not sys.stdin.isatty():
        raise Stop("run this from a terminal: it asks for Enter before the copy and before the workflows")
    out = kexec.Out(args.report)
    out.say("MPFree stream test (D3) on the test build (changes RAM, and partition 4 in its own folder only)")
    total = 4

    out.say(f"\n[1/{total}] Workstation checks (read-only)")
    program, digest = devtest.check_program(out, args.program)
    listed = devtest.workflows(out, os.path.realpath(args.workflows), only)
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
    rc, text = mpc.run(devtest.KITS, test=True)
    out.say(f"  {text.strip()}")
    if rc != 0 or not text.startswith("kits: ") or "missing" in text:
        raise Stop("the stick's kits are not at /run/mpfree-kits/kits.bin (a stick made with --app mounts them "
                   "at start); nothing was changed on the test build")
    # The tests' fake partition 4 is a folder on the machine running them: only they may ask for more room.
    need = int(os.environ.get("MPFREE_STREAMTEST_NEED_KIB", NEED_KIB)) if app.TESTING else NEED_KIB
    rc, text = mpc.run(kexec.fill(DATA, NEED=str(need)), test=True)
    for line in text.strip().splitlines():
        out.say(f"  {line}")
    if rc != 0 or text.strip().splitlines()[-1:] != ["DATA-OK"]:
        raise Stop("partition 4 is not as the workflows need it (above); nothing was changed on the test build")

    signal.signal(signal.SIGINT, signal.default_int_handler)
    stopped = []
    failed = []
    played = 0
    made = False
    try:
        out.say(f"\n[3/{total}] Copy mpfree and {len(listed)} workflows into the test build's RAM ({REMOTE_DIR})")
        if app.ready(out, f"Copies the program (about 10 MB) and the workflows, and makes {DATA_DIR}.") != "go":
            raise Stop("stopped by you")
        made = True
        rc, text = mpc.run(PREPARE, test=True)
        if rc != 0:
            raise Stop(f"could not make {REMOTE_DIR} and {DATA_DIR} ({text.strip()})")
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

        minutes = sum(s for _, _, _, s in listed) / 60 + len(listed) * 0.5
        out.say(f"\n[4/{total}] The workflows, one after the other, with their data on the stick")
        out.say("  Each plays by itself: the screen changes, the pads light and the MPC sounds. Keep the volume")
        out.say("  where you can hear it. The terminal shows each part (the 'mark' lines) as it starts.")
        if app.ready(out, f"{len(listed)} workflows, about {minutes:.0f} minutes; hands off the MPC.") != "go":
            raise Stop("stopped by you")
        for name, _, _, _ in listed:
            rc, cleared = play(mpc, out, name)
            played += 1
            if rc != 0:
                failed.append(f"{name} (exit code {rc})")
            if not cleared:
                raise Stop(f"{name}'s data folder could not be deleted; the next workflows are left out")
        out.say("")
        app.note(out, LISTEN)
        result = 0 if not failed else 1
    except (Stop, KeyboardInterrupt) as err:
        out.say(f"\nSTOP: {str(err) or 'interrupted (Ctrl-C)'}")
        rc, text = mpc.run(END_RUN, test=True)
        if text.strip():
            out.say(f"  {text.strip()}")
        result = 1
    if made:
        rc, text = mpc.run(CLEAR_ALL, test=True)
        out.say(f"  {text.strip() or f'no answer, exit code {rc}'}")
        if rc != 0:
            out.say(f"  {DATA_DIR} may still be on the stick: running this again deletes it first")
            result = 1
    for script in stopped:
        rc, text = mpc.run(script, test=True)
        out.say(f"  {text.strip() or f'no answer, exit code {rc}'} (started again)")
    if failed:
        out.say("\nFAILED: " + ", ".join(failed))
    out.say(f"\nRESULT: {played} workflows played" + ("." if result == 0 else ", stopped early or with failures (above)."))
    out.say("NEXT: python3 tools/mpc-one/devtest_report.py with this report reads each part's numbers. Then")
    out.say("switch the MPC off with its power switch, take out the stick, wait 10 s, switch it on, and run the")
    out.say("boot-map block to confirm nothing on the eMMC changed.")
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
