#!/usr/bin/env python3
"""Phase 1 "Hello MPC" on the kexec test build. RAM ONLY on the MPC.

Run it through `collect-over-ssh.sh --hello`, after `--kexec-boot` has started
the test build from the USB stick. It talks only to the test build: it uses
the newest ~/mpfree-work/kexec-stick-* folder's known_hosts, so stock 3.6 (a
different host key) is refused before anything runs. Step by step, each
started by you pressing Enter:

  1. Workstation, read-only: checks that the program
     (rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-hello) is a
     static ARM EABI5 hard-float executable, and finds the test build's folder.
  2. Test build, read-only: checks it is the test build (release, no MMC
     device, kexec off, / read-only), that /run has room and allows programs,
     that Akai's app is not running, and lists who holds the sound, MIDI,
     touch and screen devices.
  3. Test build RAM: copies the program to /run/mpfree-hello and checks its
     hash there. Nothing else is written; a power-cycle clears it.
  4. inventory: the program lists sound cards, MIDI and input devices, the
     framebuffer and real-time limits (read-only).
  5. screen: stops the test build's own log display (mpfree-test-screen, our
     service; started again at the end) and draws the Hello MPC card.
  6. touch: shows your touches on the screen for 10 s (reads only, no grab).
  7. pads: listens on all three of the controller's MIDI ports for 15 s and
     shows every message in hex. Only if no pad hit arrived, it offers a
     second try that also sends Akai's keep-alive message every 5 s.
  8. lights: sends Akai's startup pad colours and button lights, then your
     project's pad colours, and lights each pad brighter while you hold it.
  9. drums: plays the factory drum kit made by offline-factory-kit.py from
     the pads for 30 s. The kit is copied into /run/mpfree-hello with the
     program. Turn the volume knob down first.
 10. setup: asks the controller for a session, as Akai's app does, then sends
     the app's setup messages byte for byte (you type SEND first), lights the
     pads again and listens for 20 s, pinging every 4 s. It comes after drums
     so that the drums play in the pad mode round 2 already saw working.
 11. click: plays a 120 BPM click on the ACVA output for 10 s, quietly, and
     counts underruns and late periods. Turn the volume knob down first.
  MPFREE_HELLO_STEPS picks the steps after the copy, by name, comma-separated
  (default: inventory,screen,touch,pads,click). They always run in the order
  above. After lights, drums and setup it checks that the controller still
  has its normal USB id. The first step that draws on the screen stops the
  log display, since both would draw on /dev/fb0. At the end it starts the
  log display again (if it was stopped) and copies the program's own report
  (/run/mpfree-hello/hello.txt) into this report.

lights, drums and setup send the controller only what Akai's app sent it in
the capture of 2026-11-10 (see docs/hardware/controller-protocol.md), plus pad
colours made by the app's own rule for a held pad.

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
import struct
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
_spec = importlib.util.spec_from_file_location("host_kexec_boot", os.path.join(HERE, "host_kexec_boot.py"))
kexec = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(kexec)

PROGRAM_BUILT = os.path.join(REPO_ROOT, "rust", "target", "armv7-unknown-linux-gnueabihf", "release", "mpfree-hello")
PROGRAM_MAX = 16 << 20
KIT_MAX = 48 << 20
KIT_MAGIC = b"MPFKIT01"
REMOTE_DIR = "/run/mpfree-hello"
STEPS = ("inventory", "screen", "touch", "pads", "lights", "drums", "setup", "click")
DEFAULT_STEPS = "inventory,screen,touch,pads,click"
CONTROLLER_ID = "09e8:0046"
SETUP_WORD = "SEND"
# Tests set this so the prompts can be answered without a terminal.
TESTING = os.environ.get("MPFREE_HELLO_TEST") == "1"

# The only program runs this tool asks for: fixed arguments, nothing from the command line.
PHASES = {
    "inventory": ("inventory", 60),
    "screen": ("screen", 30),
    "touch": ("touch --seconds 10", 40),
    "pads": ("pads --seconds 15", 45),
    "pads-keepalive": ("pads --seconds 15 --keepalive", 45),
    "lights": ("lights --seconds 15", 60),
    "setup": ("setup --seconds 20", 90),
    "drums": ("drums --seconds 30 --gain-db -12", 90),
    "click": ("click --seconds 10 --gain-db -12", 45),
}

Stop = kexec.Stop


def arm_static_problem(data):
    """None if `data` is a static 32-bit ARM EABI5 hard-float executable, else what is wrong."""
    if len(data) > PROGRAM_MAX or len(data) < 52 or data[:7] != b"\x7fELF\x01\x01\x01":
        return "not a 32-bit little-endian ELF file of a plausible size"
    e_type, e_machine = struct.unpack_from("<HH", data, 16)
    phoff, = struct.unpack_from("<I", data, 28)
    flags, = struct.unpack_from("<I", data, 36)
    phentsize, phnum = struct.unpack_from("<HH", data, 42)
    if e_type != 2 or e_machine != 40:
        return "not an ARM executable"
    if flags & 0xFF000000 != 0x05000000 or not flags & 0x400:
        return "not EABI5 hard-float"
    if phnum == 0 or phentsize < 32 or phoff + phnum * phentsize > len(data):
        return "missing program headers, or they run past the end"
    if any(struct.unpack_from("<I", data, phoff + i * phentsize)[0] in (2, 3) for i in range(phnum)):
        return "dynamically linked (it must be static: the test build's C library may differ)"
    return None


# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
CHECK = r"""R=''
fail=0
say() { printf '%s\n' "$*"; }
bad() { printf 'PROBLEM: %s\n' "$*"; fail=1; }
rel=$(cat "$R/proc/sys/kernel/osrelease")
say "release: $rel"
[ "$rel" = '@@NEW@@' ] || bad "running $rel, not the test build's @@NEW@@"
if ls "$R/sys/block" | grep -q '^mmcblk'; then bad "an MMC block device exists"; else say "no MMC block device"; fi
v=$(cat "$R/proc/sys/kernel/kexec_load_disabled" 2>/dev/null)
say "kexec_load_disabled: $v"
[ "$v" = 1 ] || bad "kexec is not disabled"
ropts=""; runopts=""
while read -r dev mnt type opts rest; do
  [ "$mnt" = / ] && ropts=$opts
  [ "$mnt" = /run ] && runopts="$type $opts"
done < "$R/proc/mounts"
case ",$ropts," in *,ro,*) say "/: read-only" ;; *) bad "/ is not read-only ($ropts)" ;; esac
say "/run: $runopts"
case "$runopts" in tmpfs\ *) ;; *) bad "/run is not a tmpfs" ;; esac
case ",${runopts#* }," in *,noexec,*) bad "/run does not allow programs (noexec)" ;; esac
avail=$(df -k "$R/run" | tail -n 1 | while read -r a b c d rest; do echo "$d"; done)
say "free in /run: $avail KiB"
[ "${avail:-0}" -ge 65536 ] 2>/dev/null || bad "less than 64 MiB free in /run (room for the program and the drum kit)"
s=$(systemctl is-active acvs.service 2>/dev/null)
say "Akai's app (acvs.service): ${s:-unknown}"
[ "$s" = active ] && bad "Akai's app is running"
say "log display (mpfree-test-screen.service): $(systemctl is-active mpfree-test-screen.service 2>/dev/null)"
say "programs holding sound, MIDI, touch or screen devices:"
n=0
for f in "$R"/proc/[0-9]*/fd/*; do
  t=$(readlink "$f" 2>/dev/null) || continue
  case "$t" in
    /dev/snd/pcm*|/dev/snd/midi*|/dev/input/event*|/dev/fb*)
      p=${f#"$R"/proc/}; p=${p%%/*}
      say "  $t: pid $p ($(cat "$R/proc/$p/comm" 2>/dev/null))"; n=$((n + 1)) ;;
  esac
done
[ "$n" = 0 ] && say "  none"
[ "$fail" = 0 ] && say CHECKS-OK || say CHECKS-STOP
"""
# Writes only into /run/mpfree-hello (RAM). A report from an earlier run in this boot is kept.
PREPARE = r"""R=''
umask 077
d="$R/run/mpfree-hello"
[ -d "$d" ] || mkdir "$d" || exit 1
rm -f "$d/mpfree-hello" "$d/mpfree-hello.part" "$d/kit.bin" "$d/kit.bin.part"
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-hello/mpfree-hello.part" && chmod 700 "$R/run/mpfree-hello/mpfree-hello.part" && mv "$R/run/mpfree-hello/mpfree-hello.part" "$R/run/mpfree-hello/mpfree-hello" """
RECEIVE_KIT = r"""R=''; umask 077; cat > "$R/run/mpfree-hello/kit.bin.part" && mv "$R/run/mpfree-hello/kit.bin.part" "$R/run/mpfree-hello/kit.bin" """
HASH = r"""R=''
sha256sum "$R/run/mpfree-hello/mpfree-hello"
"""
HASH_KIT = r"""R=''
sha256sum "$R/run/mpfree-hello/kit.bin"
"""
# Read-only: the controller's USB id, to see it is still in its normal MIDI mode.
CONTROLLER = r"""R=''
found=0
for d in "$R"/sys/bus/usb/devices/*; do
  [ -f "$d/idVendor" ] || continue
  v=$(cat "$d/idVendor"); [ "$v" = 09e8 ] || continue
  echo "controller: $v:$(cat "$d/idProduct")"; found=1
done
[ "$found" = 1 ] || echo "controller: not found"
"""
RUN = r"""R=''
cd "$R/run/mpfree-hello" || exit 1
exec "$R/run/mpfree-hello/mpfree-hello" @@ARGS@@
"""
# Our own log display only; Akai's units are not touched.
SCREEN_STOP = r"""R=''
systemctl stop mpfree-test-screen.service
echo "log display: $(systemctl is-active mpfree-test-screen.service)"
"""
SCREEN_START = r"""R=''
systemctl start mpfree-test-screen.service
echo "log display: $(systemctl is-active mpfree-test-screen.service)"
"""
REPORT = r"""R=''
cat "$R/run/mpfree-hello/hello.txt" 2>/dev/null || echo "(no report file)"
"""


def ask(prompt):
    """Enter goes on; 's' skips this step; 'q' stops (the log display is still started again)."""
    if not TESTING:
        if not sys.stdin.isatty():
            raise Stop("run this from a terminal: each step waits for you to press Enter")
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)  # drop anything typed before the question
    try:
        answer = input(prompt).strip().lower()
    except EOFError:
        return "q"
    return {"": "go", "s": "skip", "q": "quit"}.get(answer, "again")


def typed(prompt):
    """A typed word, exactly as typed (q if the input ends)."""
    if not TESTING:
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)
    try:
        return input(prompt).strip()
    except EOFError:
        return "q"


def note(out, question):
    """Your answer to what only you can see or hear, recorded in the report."""
    if not TESTING:
        import termios
        termios.tcflush(sys.stdin, termios.TCIFLUSH)
    try:
        answer = input(f"  {question} Type a short answer and press Enter: ").strip()
    except EOFError:
        answer = ""
    out.say(f"  your answer: {answer or '(none)'}")


def ready(out, prompt):
    while True:
        a = ask(f"  {prompt} Press Enter to start (s skips this step, q stops here): ")
        if a != "again":
            out.say(f"  answer: {a}")
            return a
        print("  Please press just Enter, or s, or q.")


def stream(mpc, script, timeout, out):
    """Run a remote script on the test build and show its output as it comes. Returns (rc, text)."""
    cmd = mpc.test + [mpc.target, "sh -s"]
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    timer = threading.Timer(timeout, proc.kill)
    timer.start()
    lines = []
    try:
        proc.stdin.write(script.encode())
        proc.stdin.close()
        for raw in proc.stdout:
            line = raw.decode("utf-8", "replace").rstrip("\n")
            lines.append(line)
            out.say(f"    {line}")
        rc = proc.wait()
    finally:
        timer.cancel()
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    if rc == -signal.SIGKILL:
        out.say(f"    (stopped after {timeout} s)")
    return rc, "\n".join(lines)


def run_phase(mpc, out, name):
    args, timeout = PHASES[name]
    rc, text = stream(mpc, kexec.fill(RUN, ARGS=args), timeout, out)
    if rc != 0:
        out.say(f"  {name}: exit code {rc}")
    return rc, text


def chosen_steps(text):
    names = [n.strip() for n in text.split(",") if n.strip()]
    unknown = [n for n in names if n not in STEPS]
    if unknown or not names:
        raise Stop(f"MPFREE_HELLO_STEPS: unknown step(s) {', '.join(unknown) or '(none given)'}; "
                   f"choose from {','.join(STEPS)}")
    return [n for n in STEPS if n in names]


def newest_kit(work):
    """The newest ~/mpfree-work/factory-kit-*/kit.bin, or None."""
    folders = sorted(d for d in os.listdir(work) if d.startswith("factory-kit-")
                     and os.path.isfile(os.path.join(work, d, "kit.bin")))
    return os.path.join(work, folders[-1], "kit.bin") if folders else None


def check_controller(mpc, out):
    rc, text = mpc.run(CONTROLLER, test=True)
    line = text.strip() or f"(no answer, exit code {rc})"
    out.say(f"  {line}")
    if line != f"controller: {CONTROLLER_ID}":
        out.say(f"  WARNING: the controller is not {CONTROLLER_ID} as before. Stop here, switch the MPC off,")
        out.say("  and tell Claude before anything else.")
        raise Stop("the controller's USB id changed")


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", required=True)
    p.add_argument("--port", type=int, default=2222)
    p.add_argument("--user", default="root")
    p.add_argument("--key", required=True)
    p.add_argument("--work", required=True)
    p.add_argument("--program", default=os.environ.get("MPFREE_HELLO", PROGRAM_BUILT))
    p.add_argument("--report")
    args = p.parse_args(argv)
    out = kexec.Out(args.report)
    out.say("MPFree Hello MPC on the test build (changes RAM on the MPC only)")
    steps = chosen_steps(os.environ.get("MPFREE_HELLO_STEPS", DEFAULT_STEPS))
    total = 3 + len(steps)

    out.say(f"\n[1/{total}] Workstation checks (read-only)")
    out.say(f"  steps: {', '.join(steps)}")
    program = os.path.realpath(args.program)
    if not os.path.isfile(program):
        raise Stop(f"missing {program}: build it first (see docs/guides/hello-mpc.md)")
    with open(program, "rb") as fh:
        data = fh.read(PROGRAM_MAX + 1)
    problem = arm_static_problem(data)
    if problem:
        raise Stop(f"{program} is {problem}; rebuild it as docs/guides/hello-mpc.md shows")
    digest = hashlib.sha256(data).hexdigest()
    out.say(f"  program: {program}")
    out.say(f"  {len(data)} bytes, sha256 {digest}, static ARM EABI5 hard-float")
    folder = kexec.newest_build(os.path.realpath(args.work))
    out.say(f"  test build: {folder}")
    with open(os.path.join(folder, "report.txt")) as fh:
        found = re.search(r"^its kernel release: (\S+)$", fh.read(), re.M)
    if not found or not kexec.RELEASE.match(found.group(1)):
        raise Stop(f"{folder}/report.txt does not name the test build's kernel release")
    new = found.group(1)
    kit = kit_digest = None
    if "drums" in steps:
        kit = os.environ.get("MPFREE_HELLO_KIT") or newest_kit(os.path.realpath(args.work))
        if not kit:
            out.say("  no factory kit found (make one with offline-factory-kit.py); the drums step is left out")
            steps.remove("drums")
            total -= 1
        else:
            kit = os.path.realpath(kit)
            with open(kit, "rb") as fh:
                kit_data = fh.read(KIT_MAX + 1)
            if len(kit_data) > KIT_MAX or kit_data[:8] != KIT_MAGIC:
                raise Stop(f"{kit} is not a kit file made by offline-factory-kit.py, or is too big")
            kit_digest = hashlib.sha256(kit_data).hexdigest()
            out.say(f"  drum kit: {kit}")
            out.say(f"  {len(kit_data)} bytes, sha256 {kit_digest}")
    mpc = kexec.Mpc(args, os.path.join(folder, "known_hosts"))

    out.say(f"\n[2/{total}] Checks on the test build (read-only; only its own host key is accepted)")
    rc, text = mpc.run(kexec.fill(CHECK, NEW=new), test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc == 255 and "CHECKS-" not in text:
        raise Stop("the test build did not answer. Start it first with the --kexec-boot block; "
                   "on stock 3.6 this tool runs nothing")
    if rc != 0 or text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the test build is not ready; nothing was changed on it")

    out.say(f"\n[3/{total}] Copy the program{' and the drum kit' if kit_digest else ''} into the test build's RAM ({REMOTE_DIR})")
    rc, text = mpc.run(PREPARE, test=True)
    if rc != 0:
        raise Stop(f"could not make {REMOTE_DIR} ({text.strip()})")
    rc, text = mpc.run(None, test=True, stdin_path=program, command=RECEIVE)
    if rc != 0:
        raise Stop(f"copying the program failed ({text.strip()})")
    rc, text = mpc.run(HASH, test=True)
    if rc != 0 or text.split()[:1] != [digest]:
        raise Stop("the program on the MPC does not match the build")
    out.say("  copied; its hash on the MPC matches")
    if kit_digest:
        rc, text = mpc.run(None, test=True, stdin_path=kit, command=RECEIVE_KIT)
        if rc != 0:
            raise Stop(f"copying the drum kit failed ({text.strip()})")
        rc, text = mpc.run(HASH_KIT, test=True)
        if rc != 0 or text.split()[:1] != [kit_digest]:
            raise Stop("the drum kit on the MPC does not match the one made on the workstation")
        out.say("  drum kit copied; its hash on the MPC matches")

    screen_stopped = False
    signal.signal(signal.SIGINT, signal.default_int_handler)

    def stop_log_display():
        """Our own log display and the program would both draw on /dev/fb0; stop ours first."""
        nonlocal screen_stopped
        if not screen_stopped:
            rc, text = mpc.run(SCREEN_STOP, test=True)
            screen_stopped = True
            out.say(f"  {text.strip() or f'(no answer, exit code {rc})'}")

    def drawn(name):
        stop_log_display()
        return run_phase(mpc, out, name)

    def step(n, header, prompt):
        out.say(f"\n[{n}/{total}] {header}")
        answer = ready(out, prompt)
        if answer == "quit":
            raise Stop("stopped by you")
        return answer == "go"

    try:
        for i, name in enumerate(steps, start=4):
            if name == "inventory":
                if step(i, "Inventory (read-only)", "Lists the sound, MIDI, input and screen devices."):
                    run_phase(mpc, out, "inventory")
            elif name == "screen":
                if step(i, "Screen: our log display stops (it starts again at the end), then the Hello MPC card",
                        "Watch the MPC's screen."):
                    drawn("screen")
                    note(out, "Is the Hello MPC card on the screen, and the right way up?")
            elif name == "touch":
                if step(i, "Touch: 10 s, one finger and then two (read-only)", "Have a finger ready."):
                    drawn("touch")
                    note(out, "Did your touches show on the screen, near where you touched?")
            elif name == "pads":
                if step(i, "Pads: 15 s of listening on all three MIDI ports (nothing is sent to the controller)",
                        "Hit some pads, press a button, turn a Q-Link and the wheel."):
                    rc, text = drawn("pads")
                    note(out, "What did you press or turn, and did anything light up or react?")
                    hits = re.search(r"^pads done: .* (\d+) pad hits", text, re.M)
                    if hits and hits.group(1) == "0":
                        out.say("  No pad hits arrived. The stock app sends a keep-alive message every 5 s;")
                        out.say("  the second try sends it too (the only message this step ever sends).")
                        answer = ready(out, "Second try, with the keep-alive: hit some pads again.")
                        if answer == "quit":
                            raise Stop("stopped by you")
                        if answer == "go":
                            drawn("pads-keepalive")
                            note(out, "What did you press or turn this time, and did anything light up or react?")
                    elif hits:
                        out.say(f"  {hits.group(1)} pad hits arrived without the keep-alive, so it is not sent.")
            elif name == "lights":
                out.say(f"\n[{i}/{total}] Lights: pad colours and button lights, no session (STATE-CHANGING:")
                out.say("  the controller's lights only, until it is switched off)")
                out.say("  Sends Akai's startup pad colours (orange on pad 1 to teal on pad 16) and button lights,")
                out.say("  then your project's pad colours. Then 15 s: each pad you hold should light brighter.")
                answer = ready(out, "Watch the pads and buttons.")
                if answer == "quit":
                    raise Stop("stopped by you")
                if answer == "go":
                    drawn("lights")
                    check_controller(mpc, out)
                    note(out, "Did the pads light up (which colours, which pads), and did a held pad get brighter? "
                              "Which buttons lit?")
            elif name == "setup":
                out.say(f"\n[{i}/{total}] Akai's setup: a session, then the app's setup messages byte for byte")
                out.say("  (STATE-CHANGING: the controller's settings until it is switched off; every byte is one")
                out.say("  Akai's app sends at each start, so it ends where it does after a factory boot)")
                out.say("  If the controller does not accept the session within 20 s, nothing else is sent.")
                out.say("  Then 20 s: hit pads, turn Q-Link 1 and the wheel, press a few buttons.")
                while True:
                    a = typed(f"  Type {SETUP_WORD} and press Enter to send them (s skips, q stops here): ")
                    if a in (SETUP_WORD, "s", "q"):
                        break
                    print(f"  Please type {SETUP_WORD} exactly, or s, or q.")
                out.say(f"  answer: {a}")
                if a == "q":
                    raise Stop("stopped by you")
                if a == SETUP_WORD:
                    drawn("setup")
                    check_controller(mpc, out)
                    note(out, "What lit up, and did the pads, Q-Link and wheel feel any different?")
            elif name == "drums":
                out.say(f"\n[{i}/{total}] Drums: 30 s of the factory kit from the pads, through the main outputs")
                out.say("  Each sound peaks at most 12 dB below full scale; hits that overlap add up. Turn the MPC's")
                out.say("  volume knob most of the way down first, and keep headphones off your ears until you have")
                out.say("  heard how loud it is.")
                answer = ready(out, "Volume down? Then play the pads.")
                if answer == "quit":
                    raise Stop("stopped by you")
                if answer == "go":
                    drawn("drums")
                    check_controller(mpc, out)
                    note(out, "Did each pad play a drum sound right away? Any crackles, gaps or lag?")
            elif name == "click":
                out.say(f"\n[{i}/{total}] Click: 10 s at 120 BPM through the main outputs and headphones")
                out.say("  It peaks about 18 dB below full scale. Turn the MPC's volume knob most of the way down")
                out.say("  first, and keep headphones off your ears until you have heard how loud it is.")
                answer = ready(out, "Volume down?")
                if answer == "quit":
                    raise Stop("stopped by you")
                if answer == "go":
                    drawn("click")
                    note(out, "Did you hear a steady click, louder every fourth beat? Any crackles or gaps?")
        result = 0
    except (Stop, KeyboardInterrupt) as err:
        out.say(f"\nSTOP: {str(err) or 'interrupted (Ctrl-C)'}")
        result = 1
    signal.signal(signal.SIGINT, signal.SIG_IGN)  # no Ctrl-C may cut the clean-up short
    if screen_stopped:
        out.say("\nStarting the log display again:")
        rc, text = mpc.run(SCREEN_START, test=True)
        out.say(f"  {text.strip() or f'(no answer, exit code {rc})'}")
    out.say(f"\nThe program's own report ({REMOTE_DIR}/hello.txt):")
    rc, text = mpc.run(REPORT, test=True)
    for line in text.splitlines():
        out.say(f"  | {line}")
    out.say("\nRESULT: " + ("all steps ran (see each step's lines above)." if result == 0 else "stopped early."))
    out.say("NEXT: switch the MPC off with its power switch, take out the stick, wait 10 s, switch it on,")
    out.say("and wait for 3.6. Then run the boot-map block to confirm nothing on the eMMC changed.")
    return result


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:  # only reaches here in steps 1 to 3, before any program runs
        print("\nSTOP: interrupted (Ctrl-C). No program was started on the MPC.")
        sys.exit(130)
