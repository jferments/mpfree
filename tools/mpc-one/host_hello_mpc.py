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
     hash there. Nothing else is written; a power-cycle clears it. Skipped
     when the only steps are app and line, which need no copy.
  app: READ-ONLY. The pad player that a stick built with --app starts at
     power-on: one phase at a time, each started by Enter and ended by Enter
     when you are done, it shows what the app wrote to the system log in
     that time (buttons, velocities, Note Repeat and Latch, menus, its
     30-second status lines with underruns and the note engine's dropped
     events) and asks what you saw. It only reads the log and the service's
     state; it copies, starts and stops nothing.
  line: STATE-CHANGING (RAM only), and never in the default steps. The
     MPC's line input, with the same pad player: first a read-only look at
     the ACVA card's capture device in /proc/asound, then, only after you
     type LINE, phases where you turn Line input on in the pad player (it
     opens the capture device), watch its meter, loop it and turn it off
     again, each followed by the same read-only look. The tool itself only
     reads; the pad player opens and closes the device when you ask it to.
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
  9. drums: plays the first factory drum kit made by offline-factory-kit.py
     from the pads for 30 s. The kits file is copied into /run/mpfree-hello
     with the program. Turn the volume knob down first.
 10. setup: asks the controller for a session, as Akai's app does, then sends
     the app's setup messages byte for byte (you type SEND first), lights the
     pads again and listens for 20 s, pinging every 4 s. It comes after drums
     so that the drums play in the pad mode round 2 already saw working.
 11. play: the pad player, MPFree's first GUI (you type SEND first): the same
     session and setup as step 10, then up to 120 s with every kit: the pads
     or the screen play, the wheel or a tap picks the kit, Q-Link 1 or the
     slider sets the level, STOP ends it. If the session is not accepted
     within 20 s it plays without Akai's setup. Turn the volume knob down first.
 12. click: plays a 120 BPM click on the ACVA output for 10 s, quietly, and
     counts underruns and late periods. Turn the volume knob down first.
  MPFREE_HELLO_STEPS picks the steps after the copy, by name, comma-separated
  (default: inventory,screen,touch,pads,click). They always run in the order
  app, line, then the numbered ones above. After lights, drums, setup and play it
  checks that the controller still has its normal USB id. The first step that
  draws on the screen stops the log display or the pad player (our own
  services), since both would draw on /dev/fb0. At the end it starts again
  whichever it stopped, and copies the program's own report
  (/run/mpfree-hello/hello.txt) into this report.

lights, drums, setup and play send the controller only what Akai's app sent it in
the capture of 2026-11-10 (see docs/dev/hardware/controller-protocol.md), plus pad
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
KIT_MAGIC = b"MPFKITS1"
REMOTE_DIR = "/run/mpfree-hello"
STEPS = ("app", "line", "inventory", "screen", "touch", "pads", "lights", "drums", "setup", "play", "click")
# The steps that use the pad player started at power-on: no copy, and they run before anything stops it.
APP_STEPS = ("app", "line")
DEFAULT_STEPS = "inventory,screen,touch,pads,click"
CONTROLLER_ID = "09e8:0046"
SETUP_WORD = "SEND"
LINE_WORD = "LINE"
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
    "play": ("play --seconds 120 --gain-db -12", 200),
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
[ "${avail:-0}" -ge 65536 ] 2>/dev/null || bad "less than 64 MiB free in /run (room for the program and the drum kits)"
s=$(systemctl is-active acvs.service 2>/dev/null)
say "Akai's app (acvs.service): ${s:-unknown}"
[ "$s" = active ] && bad "Akai's app is running"
say "log display (mpfree-test-screen.service): $(systemctl is-active mpfree-test-screen.service 2>/dev/null)"
say "pad player (mpfree-test-app.service): $(systemctl is-active mpfree-test-app.service 2>/dev/null)"
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
rm -f "$d/mpfree-hello" "$d/mpfree-hello.part" "$d/kit.bin" "$d/kit.bin.part" "$d/kits.bin" "$d/kits.bin.part"
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-hello/mpfree-hello.part" && chmod 700 "$R/run/mpfree-hello/mpfree-hello.part" && mv "$R/run/mpfree-hello/mpfree-hello.part" "$R/run/mpfree-hello/mpfree-hello" """
RECEIVE_KIT = r"""R=''; umask 077; cat > "$R/run/mpfree-hello/kits.bin.part" && mv "$R/run/mpfree-hello/kits.bin.part" "$R/run/mpfree-hello/kits.bin" """
HASH = r"""R=''
sha256sum "$R/run/mpfree-hello/mpfree-hello"
"""
HASH_KIT = r"""R=''
sha256sum "$R/run/mpfree-hello/kits.bin"
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
# Our own log display and pad player only (whichever runs); Akai's units are not touched.
SCREEN_STOP = r"""R=''
a=$(systemctl is-active mpfree-test-app.service 2>/dev/null)
[ "$a" = active ] && systemctl stop mpfree-test-app.service
echo "pad player: was ${a:-unknown}, now $(systemctl is-active mpfree-test-app.service 2>/dev/null)"
s=$(systemctl is-active mpfree-test-screen.service 2>/dev/null)
[ "$s" = active ] && systemctl stop mpfree-test-screen.service
echo "log display: was ${s:-unknown}, now $(systemctl is-active mpfree-test-screen.service 2>/dev/null)"
"""
SCREEN_START = r"""R=''
systemctl start mpfree-test-screen.service
echo "log display: $(systemctl is-active mpfree-test-screen.service)"
"""
APP_START = r"""R=''
systemctl start mpfree-test-app.service
echo "pad player: $(systemctl is-active mpfree-test-app.service)"
"""
# Read-only: the pad player's state, the test build's clock since boot, and the pad player's log.
APP_STATE = r"""R=''
echo "pad player (mpfree-test-app.service): $(systemctl is-active mpfree-test-app.service 2>/dev/null)"
"""
UPTIME = r"""R=''
cat "$R/proc/uptime"
"""
APP_LOG = r"""R=''
journalctl -b -u mpfree-test-app.service -o short-monotonic --no-pager --no-hostname -n 50000 2>&1
"""
# The line input (read-only): the ACVA card's capture device node and its ALSA state (closed, or the
# settings and position while the pad player has it open), next to the playback device's state.
LINE_PROBE = r"""R=''
found=0
for d in "$R"/proc/asound/card[0-9]*; do
  [ "$(cat "$d/id" 2>/dev/null)" = ACVA ] || continue
  found=1
  n=${d##*/card}
  echo "ACVA is card $n"
  ls -l "$R/dev/snd/pcmC${n}D0c" 2>&1
  for f in pcm0c/info pcm0c/sub0/status pcm0c/sub0/hw_params pcm0p/sub0/hw_params; do
    echo "--- card$n/$f"
    cat "$d/$f" 2>&1
  done
done
[ "$found" = 1 ] || echo "no sound card with the id ACVA"
"""
# The touchscreen's interrupt count (read-only): shows whether the ILI2117 sends anything at all.
TOUCH_IRQ = r"""R=''
grep -i 'ili' "$R/proc/interrupts" 2>/dev/null || echo "no ILI2117 line in /proc/interrupts"
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
    """The newest ~/mpfree-work/factory-kit-*/kits.bin, or None."""
    folders = sorted(d for d in os.listdir(work) if d.startswith("factory-kit-")
                     and os.path.isfile(os.path.join(work, d, "kits.bin")))
    return os.path.join(work, folders[-1], "kits.bin") if folders else None


def send_word(out):
    """Ask for SEND before a step that sends Akai's setup. True to go on; Stop on q."""
    while True:
        a = typed(f"  Type {SETUP_WORD} and press Enter to send them (s skips, q stops here): ")
        if a in (SETUP_WORD, "s", "q"):
            break
        print(f"  Please type {SETUP_WORD} exactly, or s, or q.")
    out.say(f"  answer: {a}")
    if a == "q":
        raise Stop("stopped by you")
    return a == SETUP_WORD


def check_controller(mpc, out):
    rc, text = mpc.run(CONTROLLER, test=True)
    line = text.strip() or f"(no answer, exit code {rc})"
    out.say(f"  {line}")
    if line != f"controller: {CONTROLLER_ID}":
        out.say(f"  WARNING: the controller is not {CONTROLLER_ID} as before. Stop here, switch the MPC off,")
        out.say("  and tell Claude before anything else.")
        raise Stop("the controller's USB id changed")


# The app step's phases: (title, what to do, the question afterwards). Each is started by Enter and
# ended by Enter; the app's log lines from that time are shown. A skipped phase loses nothing: the
# summary at the end reads the whole session.
APP_PHASES = (
    ("Developer tools and Map controls",
     ["Press SHIFT + MENU for Prefs, turn the wheel to \"Map controls\" and press +. This turns on the developer tools",
      "and opens the list of the MPC's buttons: found (green), skipped (grey) or still to find.",
      "Tap a control in the list (or pick it with the wheel and press +), then press that button on the MPC:",
      "its number appears. Start with REC, OVERDUB, PLAY START, UNDO and the four PAD BANK buttons (they",
      "work as soon as they are found), then as many others as you like, at your pace. - skips one that is",
      "not on your MPC; Other 1-10 take buttons that are not listed. STOP stops listening."],
     "Which controls could you not find, and did any name in the list not match your MPC's panel?"),
    ("Developer pages: Performance, CPU by engine, Processes",
     ["SHIFT + wheel steps through the Dev tab's pages. Performance shows CPU per thread, memory,",
      "temperature, and the audio loop's render time against its 2.9 ms period, with the last minute drawn.",
      "CPU by engine shows each audio stage (sampler, synth, looper, time, mix) against its budget, and",
      "Processes every process on the MPC with its CPU and memory. Play some pads and the Grains kit for",
      "about 30 s on each page and watch the numbers move."],
     "Did the three pages fill in? What did the render time, the engines and the busiest processes show?"),
    ("Kits, pad banks, Full Level and Half Level",
     ["Tap the Play tab (or press MENU twice). Hit pad 1 softly, medium and hard: it should feel like round 5.",
      "Press FULL LEVEL (it lights) and hit pad 1 softly twice: both loud. SHIFT + FULL LEVEL is",
      "Half Level: soft hits come out medium. Press FULL LEVEL until its light goes out.",
      "Press PAD BANK A to D: banks A-D are kits 1-4, and the kit chip shows the bank. Press the same",
      "bank button again (or SHIFT + it) for E-H, kits 5-8 if the stick has them. Go back to bank A.",
      "Hold a long sound, turn the wheel to the next kit: the sound finishes rather than cutting off.",
      "Hit a few pads and press STOP while they ring: they fade out quickly, with no click."],
     "Did the banks, Full and Half Level work? Did the bank buttons light? Did kit switching and STOP sound clean?"),
    ("Grains",
     ["Turn the wheel to the kit called \"Grains: ...\". Hold a pad: a cloud of grains. Press harder:",
      "pads 1-4 get denser, 5-8 longer grains, 9-12 move where they read, 13-16 rise up to an octave.",
      "Let go: it fades out. Then hold NOTE REPEAT and 8 pads together for about 10 s, and let go."],
     "Did the grains respond to pressure and fade out? Any crackles or dropouts in the 8-pad test?"),
    ("Synth",
     ["Turn the wheel to the last entry, \"Synth: Poly\": the pads play notes from C3 up. SHIFT + wheel on",
      "Play moves them by octaves. Tap the Synth tab: Q-Links 1-4 turn the marked knobs, the wheel turns the",
      "selected knob, - / + change pages. Pick the Supersaw preset, hold 8 pads for about 10 s (the hardest",
      "case for the CPU), let go, and press STOP. Then turn the wheel back to kit 1."],
     "Did Poly play from the pads, did the knobs change the sound, any crackles in the 8-pad test?"),
    ("Note Repeat and the arp",
     ["Hold NOTE REPEAT: the quick band shows the rates and Latch. While holding it, - / + switch",
      "Repeat, Arp and Pattern, the wheel changes the rate, order or pattern, and Q-Links 1-4 set",
      "rate, gate, swing and octaves. Hold a few pads to hear each mode. SHIFT + NOTE REPEAT is Latch, and",
      "so is pressing NOTE REPEAT twice quickly; press it again to unlatch.",
      "NOTE REPEAT + MENU opens the Arp panel: - / + walk Direct, Repeat, Arp and Pattern, and the",
      "pads play through it. NOTE REPEAT + MENU again closes it. STOP silences and unlatches."],
     "Did the quick band and the Arp panel show up and change what the pads played?"),
    ("Arrange: record, overdub and undo",
     ["Uses the buttons found in phase 1. Press PLAY: the transport bar counts bars and beats. Press REC,",
      "play a short beat on the pads for a bar or two, and press REC again: the take loops. Press OVERDUB",
      "(it switches Overdub and Replace), REC, and add more hits on the next pass; REC again. UNDO takes the",
      "last take back, SHIFT + UNDO puts it back. Tap the Arrange tab: + makes a 2-bar clip and records into",
      "it, and tapping a clip launches or stops it on the next bar. PLAY START plays from bar 1. Last,",
      "Capture: with REC off, play a short phrase on the pads, then tap Capture on the Arrange tab. The phrase",
      "becomes a take (\"Capture 1\"), and UNDO takes it back. STOP stops."],
     "Did recording, overdub, undo, the clips and Capture work in time? Anything out of time or lost?"),
    ("Audio looper: loop the pads",
     ["Tap the Looper tab. It records what the pads and the synth play (\"Records from: Pads and synth\");",
      "the line input stays off. Press PLAY, then REC (or tap Record): recording starts on the next bar line",
      "and lasts 2 bars, so play a beat on the pads through them. Then the loop plays. REC (or OVERDUB) adds",
      "a layer from the next pass, REC again ends it, UNDO takes it back and SHIFT + UNDO puts it back.",
      "Tap Mute twice, then Clear. MENU on the Looper tab has its options (bars, level). STOP stops."],
     "Did the loop start on the bar and play back in time, without clicks? Did overdub, undo and Clear work?"),
    ("Sample tab: Wave and Chop",
     ["Hit pad 5 on bank A, then tap the Sample tab: the Wave editor shows the pad hit last. Q-Links 1-4",
      "move Start, End and the loop; drag a marker by its tab; the wheel zooms; tap the wave to hear it.",
      "SHIFT + wheel (or - / +) picks another pad. UNDO takes an edit back, SHIFT + UNDO puts it back.",
      "Tap Chop in the pad bar, pick slices, then Chop to pads: the Play tab's pad names follow. Play",
      "the pads, then UNDO the chop. The edits stay in memory only; nothing is saved."],
     "Did the Wave and Chop tabs show the sound, play it, and change what the pads played? Did undo work?"),
    ("Prefs, System info and saving",
     ["Press SHIFT + MENU for Prefs. Turn the wheel to System info and press +: the version, the system",
      "and the hardware. Back. Change \"Pad velocity curve\" once with +. The Prefs rows",
      "and the buttons you found are now kept on the stick's partition 4; the summary at the end shows the",
      "saving line. They should come back after the second boot. Line input is never kept."],
     "Did System info fill in? Which pad velocity curve did you leave it on?"),
    ("Touch lab",
     ["Press SHIFT + MENU for Prefs, turn the wheel to the last row, Touch screen, and press +. Turn the",
      "wheel to Touch lab and press +. Follow the screen through all 9 steps (Start, a countdown, then",
      "do what it says; Next after each), about 3 minutes. Then press Exit at the top right: you are",
      "back in the Touch screen list. Turn the wheel up one row to Calibrate touch and press +, then tap",
      "each corner's target as it asks. It only reports whether the screen matches; nothing is saved",
      "or changed. Exit is in the middle of the screen."],
     "Did the Touch lab open and see your touches? What did Calibrate touch say (match or differs)?"),
    ("Menus, console and STOP",
     ["Press MENU (the page's options) and MENU again to close. SHIFT + STOP shows the system log;",
      "SHIFT + STOP again goes back. Tap a few pads on the screen and a kit in the list.",
      "Last: SHIFT + NOTE REPEAT, hold pads 1 and 2, let go so they repeat, then press STOP."],
     "Did the menus, the console and the screen pads work, and did STOP silence everything?"),
)
# The session summary at the end of the app step: every line that starts with one of these, from
# power-on to the end (the last three status lines only).
SUMMARY_PREFIXES = ("touchscreen: ", "touch-reader: ", "button guide done: ", "button guide: ", "developer tools: ",
                    "looper: ", "line input: ", "saving: ", "saved settings", "saved buttons", "not saved: ",
                    "storage: ", "capture", "sample: ",
                    "map: ", "map summary: ", "bank", "pad level: ", "kit ", "synth: ", "pads: ", "Latch ",
                    "arp: ", "arp order: ", "arp pattern: ", "pattern ", "recording ", "time: ", "tempo ",
                    "touch-lab-open", "touch-lab: ", "touch-lab-exit", "touch-calibrate", "touch-orientation: ",
                    "STOP: ")
APP_LINE = re.compile(r"^\[\s*(\d+\.\d+)\]\s+(.*)$")
MIDI_LINE = re.compile(r"^\+\d+\.\d+ s (->\s*)?(Public|Private|MIDI Port)\b")
SHOWN_PER_PHASE = 60


def log_only(out, text):
    """A line for the report file only (the full log of each phase)."""
    if out.fh:
        out.fh.write(text + "\n")
        out.fh.flush()


def uptime(mpc):
    rc, text = mpc.run(UPTIME, test=True)
    try:
        return float(text.split()[0])
    except (IndexError, ValueError):
        raise Stop(f"could not read the test build's uptime ({text.strip() or f'exit code {rc}'})")


def app_lines(mpc, since):
    """The pad player's log lines from `since` (seconds since boot) on, as (time, message)."""
    rc, text = mpc.run(APP_LOG, test=True)
    lines = []
    for raw in text.splitlines():
        m = APP_LINE.match(raw)
        if not m or float(m.group(1)) < since:
            continue
        rest = m.group(2)
        # "mpfree-hello[211]: text" or "systemd[1]: text": keep the text, and who wrote it if not the app.
        ident, sep, message = rest.partition(": ")
        if not sep:
            message = rest
        elif not ident.startswith("mpfree-hello"):
            message = f"({ident}) {message}"
        lines.append((float(m.group(1)), message))
    return lines


def summarize(out, lines):
    """Show a phase's log: every event line, and the MIDI lines counted. Every line goes in the report."""
    events, pads, buttons, wheel = [], [], [], 0
    other = {"pressure": 0, "session": 0, "Q-Link": 0, "other": 0}
    for t, message in lines:
        log_only(out, f"      | [{t:10.3f}] {message}")
        if not MIDI_LINE.match(message):
            events.append(message)
            continue
        meaning = message.rpartition(" = ")[2]
        pad = re.fullmatch(r"pad (\d+) down, velocity (\d+)", meaning)
        button = re.fullmatch(r"button (\d+) pressed", meaning)
        turned = re.fullmatch(r"data wheel ([+-]\d+)", meaning)
        if pad:
            pads.append(f"{pad.group(1)}:{pad.group(2)}")
        elif button:
            buttons.append(button.group(1))
        elif turned:
            wheel += int(turned.group(1))
        elif "pressure" in meaning:
            other["pressure"] += 1
        elif "session" in meaning:
            other["session"] += 1
        elif "Q-Link" in meaning:
            other["Q-Link"] += 1
        elif not re.fullmatch(r"pad \d+ up|button \d+ released", meaning):
            other["other"] += 1
    out.say(f"    the app's log: {len(lines)} lines (all in the report)")
    for message in events[:SHOWN_PER_PHASE]:
        out.say(f"      {message}")
    if len(events) > SHOWN_PER_PHASE:
        out.say(f"      ... and {len(events) - SHOWN_PER_PHASE} more")
    out.say(f"    pads hit (pad:velocity as played): {' '.join(pads) or 'none'}")
    out.say(f"    buttons pressed (note numbers): {' '.join(buttons) or 'none'}")
    out.say(f"    data wheel: {wheel:+d}; Q-Link messages: {other['Q-Link']}; pressure messages: {other['pressure']}; "
            f"session pings and replies: {other['session']}; other MIDI: {other['other']}")


def app_check(mpc, out, n, total):
    """The app step: read-only, paced phases against the pad player started at power-on."""
    out.say(f"\n[{n}/{total}] The pad player that started at power-on (read-only: this reads its log and nothing else)")
    rc, text = mpc.run(APP_STATE, test=True)
    out.say(f"  {text.strip() or f'(no answer, exit code {rc})'}")
    answer = ready(out, "First, what the screen did from power-on.")
    if answer == "quit":
        raise Stop("stopped by you")
    if answer == "skip":
        return
    touch_interrupts(mpc, out, "at the start")
    out.say("  (s skips a phase. You can also play freely: the summary at the end reads the whole session.)")
    out.say("\n  Start-up: everything the app wrote since power-on")
    summarize(out, app_lines(mpc, 0.0))
    note(out, "Did the loading screen show over the galaxy with a progress bar and messages, then the pad player, "
              "filling the whole screen with no log at the edges?")
    if not text.strip().endswith(": active"):
        out.say("  The pad player is not running, so the other phases are left out. Its log is above; if it")
        out.say("  stopped with an error, the log display started in its place.")
        return
    for k, (title, todo, question) in enumerate(APP_PHASES, start=1):
        out.say(f"\n  Phase {k} of {len(APP_PHASES)}: {title}")
        for line in todo:
            out.say(f"    {line}")
        answer = ready(out, "Ready?")
        if answer == "quit":
            raise Stop("stopped by you")
        if answer == "skip":
            continue
        since = uptime(mpc)
        while (done := ask("  Go ahead. Press Enter when you are done (q stops here): ")) in ("again", "skip"):
            print("  Please press just Enter when you are done, or q.")
        summarize(out, app_lines(mpc, since))
        if done == "quit":
            raise Stop("stopped by you")
        note(out, question)
    session_summary(mpc, out)


LINE_PHASES = (
    ("Line input on, and its meter",
     ["Turn the MPC's volume knob down first. Plug a source into the line inputs (a phone or a synth, not",
      "too loud), or leave them empty. Tap the Looper tab, press MENU, turn the wheel to \"Line input\" and",
      "press +. The pad player opens the line input: \"Records from\" changes to Line input, or the log",
      "says why it stays off. Play the source: the meter on the right moves. Unplug the right-hand plug,",
      "then the left, to see which jack is L and which is R. Press MENU to close the options."],
     "Did Line input turn on, and did the meter move? Which jack moved L, and which R?"),
    ("Loop the line input",
     ["Press PLAY, then REC (or tap Record): 2 bars from the next bar line. Play the source into it; then",
      "the loop plays. Add a layer with OVERDUB (or REC), end it with REC, UNDO it. Tap Monitor to hear the",
      "input through the outputs, then tap it again. The pads are not recorded now, only the line input."],
     "Did the loop record the input and play it back in time? Any clicks, dropouts or noise?"),
    ("Line input off",
     ["Press MENU on the Looper tab, turn the wheel to \"Line input\" and press -. The pad player closes the",
      "line input and \"Records from\" goes back to the pads; the loop keeps playing. Press MENU, then STOP."],
     "Did Line input turn off, and did the pads and the loop keep playing?"),
)


def line_probe(mpc, out, when):
    rc, text = mpc.run(LINE_PROBE, test=True)
    out.say(f"  the line input {when} (read-only):")
    for line in (text.strip() or f"(no answer, exit code {rc})").splitlines():
        out.say(f"    {line}")


def line_check(mpc, out, n, total):
    """The line step: you open the line input from the pad player; the tool reads its state and log."""
    out.say(f"\n[{n}/{total}] The line input (STATE-CHANGING on the MPC, RAM only: the pad player opens its capture "
            "device when you turn Line input on)")
    rc, text = mpc.run(APP_STATE, test=True)
    out.say(f"  {text.strip() or f'(no answer, exit code {rc})'}")
    line_probe(mpc, out, "before")
    if not text.strip().endswith(": active"):
        out.say("  The pad player is not running, so this step is left out. Nothing was opened.")
        return
    out.say("  What changes: the pad player opens the ACVA card's capture device (stereo, 44.1 kHz, 128-frame")
    out.say("  periods) and reads it without waiting, so playback never waits on it. Nothing is written to the")
    out.say("  eMMC, the U-Boot environment or the mixer settings, and nothing is saved: Line input is off at")
    out.say("  every start.")
    out.say("  Undo: Line input Off in the Looper options or Prefs closes the device. Leaving the pad player, or")
    out.say("  switching the MPC off (the test build lives in RAM), closes it too.")
    out.say("  If it fails: if the card refuses to open or start the input, the pad player logs why and the row")
    out.say("  stays Off; after more than 3 read errors in a row it closes the input itself and says so. Which")
    out.say("  jacks the input reads, and whether Akai's driver records on the test build at all, are not")
    out.say("  checked yet: that is what this step finds out.")
    while True:
        a = typed(f"  Type {LINE_WORD} and press Enter to go on (s skips this step, q stops here): ")
        if a in (LINE_WORD, "s", "q"):
            break
        print(f"  Please type {LINE_WORD} exactly, or s, or q.")
    out.say(f"  answer: {a}")
    if a == "q":
        raise Stop("stopped by you")
    if a != LINE_WORD:
        return
    for k, (title, todo, question) in enumerate(LINE_PHASES, start=1):
        out.say(f"\n  Phase {k} of {len(LINE_PHASES)}: {title}")
        for line in todo:
            out.say(f"    {line}")
        answer = ready(out, "Ready?")
        if answer == "quit":
            raise Stop("stopped by you")
        if answer == "skip":
            continue
        since = uptime(mpc)
        while (done := ask("  Go ahead. Press Enter when you are done (q stops here): ")) in ("again", "skip"):
            print("  Please press just Enter when you are done, or q.")
        summarize(out, app_lines(mpc, since))
        line_probe(mpc, out, "now")
        if done == "quit":
            raise Stop("stopped by you")
        note(out, question)
    lines = app_lines(mpc, 0.0)
    out.say("\n  The line input and the looper, the whole session:")
    for message in [m for _, m in lines if m.startswith(("line input: ", "looper: ", "status: looper: "))][-40:]:
        out.say(f"    {message}")


def touch_interrupts(mpc, out, when):
    rc, text = mpc.run(TOUCH_IRQ, test=True)
    for line in (text.strip() or f"(no answer, exit code {rc})").splitlines()[:4]:
        out.say(f"  touch interrupts {when}: {' '.join(line.split())}")


def session_summary(mpc, out):
    """What the whole session showed, so a phase that was skipped or played freely still counts."""
    lines = app_lines(mpc, 0.0)
    out.say("\n  Session summary (the whole run since power-on; every line is in the report):")
    picked = [m for _, m in lines if m.startswith(SUMMARY_PREFIXES)]
    shown = 150
    for message in picked[:shown]:
        out.say(f"    {message}")
    if len(picked) > shown:
        out.say(f"    ... and {len(picked) - shown} more")
    for _, message in lines:
        log_only(out, f"      | {message}")
    hits = sum(1 for _, m in lines if re.search(r"= pad \d+ down, velocity \d+$", m))
    buttons = sorted({int(b) for _, m in lines for b in re.findall(r"= button (\d+) pressed$", m)})
    out.say(f"    pad hits from the controller: {hits}; buttons pressed: {' '.join(map(str, buttons)) or 'none'}")
    touch_interrupts(mpc, out, "now")
    out.say("\n  The app's latest status (every 30 s: hits, underruns, timing, voices, the engines' counters):")
    statuses = [m for _, m in lines if m.startswith("status: ")]
    # Every 30 s: the pad player's line, the synth's and the looper's.
    for message in statuses[-3:] or ["(none yet: the first comes 30 s after the app starts playing)"]:
        out.say(f"    {message}")
    perf = [m for _, m in lines if m.startswith("perf: ")]
    out.say(f"  The developer tools' process numbers ({len(perf)} perf lines; the latest):")
    out.say(f"    {perf[-1] if perf else '(none: the developer tools were not on, or not for 30 s)'}")


def copy_program(mpc, out, total, program, digest, kit, kit_digest):
    """Step 3: the program (and the kits, for drums and play) into the test build's RAM."""
    out.say(f"\n[3/{total}] Copy the program{' and the drum kits' if kit_digest else ''} into the test build's RAM ({REMOTE_DIR})")
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
            raise Stop(f"copying the drum kits failed ({text.strip()})")
        rc, text = mpc.run(HASH_KIT, test=True)
        if rc != 0 or text.split()[:1] != [kit_digest]:
            raise Stop("the drum kits on the MPC do not match the ones made on the workstation")
        out.say("  drum kits copied; their hash on the MPC matches")


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
    # The app and line steps use the pad player started at power-on; the program is copied only for the others.
    copy = any(n not in APP_STEPS for n in steps)
    total = (3 if copy else 2) + len(steps)

    out.say(f"\n[1/{total}] Workstation checks (read-only)")
    out.say(f"  steps: {', '.join(steps)}")
    program = os.path.realpath(args.program)
    if not os.path.isfile(program):
        raise Stop(f"missing {program}: build it first (see docs/dev/guides/hello-mpc.md)")
    with open(program, "rb") as fh:
        data = fh.read(PROGRAM_MAX + 1)
    problem = arm_static_problem(data)
    if problem:
        raise Stop(f"{program} is {problem}; rebuild it as docs/dev/guides/hello-mpc.md shows")
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
    need_kits = [n for n in ("drums", "play") if n in steps]
    if need_kits:
        kit = os.environ.get("MPFREE_HELLO_KIT") or newest_kit(os.path.realpath(args.work))
        if not kit:
            out.say("  no factory kits found (make them with offline-factory-kit.py); left out: "
                    + ", ".join(need_kits))
            for n in need_kits:
                steps.remove(n)
            total -= len(need_kits)
            if not steps:
                raise Stop("no steps left to run; nothing was sent to the MPC")
        else:
            kit = os.path.realpath(kit)
            with open(kit, "rb") as fh:
                kit_data = fh.read(KIT_MAX + 1)
            if len(kit_data) > KIT_MAX or kit_data[:8] != KIT_MAGIC:
                raise Stop(f"{kit} is not a kits file made by offline-factory-kit.py, or is too big")
            kit_digest = hashlib.sha256(kit_data).hexdigest()
            count = struct.unpack_from("<I", kit_data, 8)[0] if len(kit_data) >= 12 else 0
            out.say(f"  drum kits: {kit}")
            out.say(f"  {count} kits, {len(kit_data)} bytes, sha256 {kit_digest}")
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

    if copy:
        copy_program(mpc, out, total, program, digest, kit, kit_digest)

    stopped = []
    signal.signal(signal.SIGINT, signal.default_int_handler)

    def stop_log_display():
        """Our own log display or pad player and the program would both draw on /dev/fb0; stop ours first."""
        if stopped:
            return
        rc, text = mpc.run(SCREEN_STOP, test=True)
        for line in text.strip().splitlines() or [f"(no answer, exit code {rc})"]:
            out.say(f"  {line}")
        for name, script in (("pad player", APP_START), ("log display", SCREEN_START)):
            if re.search(rf"^{name}: was active,", text, re.M):
                stopped.append((name, script))
        if not stopped:
            stopped.append(("nothing", None))

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
        for i, name in enumerate(steps, start=4 if copy else 3):
            if name == "app":
                app_check(mpc, out, i, total)
            elif name == "line":
                line_check(mpc, out, i, total)
            elif name == "inventory":
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
                if send_word(out):
                    drawn("setup")
                    check_controller(mpc, out)
                    note(out, "What lit up, and did the pads, Q-Link and wheel feel any different?")
            elif name == "play":
                out.say(f"\n[{i}/{total}] Pad player: Akai's session and setup, then up to 120 s with the kits on the screen")
                out.say("  (STATE-CHANGING: the controller's settings and lights until it is switched off; the same")
                out.say("  bytes as the setup step, then pad colours for the kit in use)")
                out.say("  If the session is not accepted within 20 s, it plays without Akai's setup: no setup message")
                out.say("  is sent, only pad colours, as in the lights step.")
                out.say("  Play the pads or the pads on the screen; the wheel or a tap on the list picks the kit;")
                out.say("  Q-Link 1 or the slider sets the level (it starts 6 dB below the most, which is 12 dB below")
                out.say("  full scale). The tabs at the top switch between Play and Debug. STOP ends it early.")
                out.say("  Turn the MPC's volume knob most of the way down first.")
                if send_word(out):
                    drawn("play")
                    check_controller(mpc, out)
                    note(out, "Did the screen follow the pads, kits and level? Did touching the screen's pads, kit "
                              "list, slider and tabs work? Any lag, crackles or gaps?")
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
    for name, script in stopped:
        if script:
            out.say(f"\nStarting the {name} again:")
            rc, text = mpc.run(script, test=True)
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
