#!/usr/bin/env python3
"""mpfree, the app, on the test build: the USB stick's own system. RAM ONLY on the MPC.

Run it through `collect-over-ssh.sh --app` while the test build runs. On the
installed MPC OS 3.10.1 the boot hook starts it: with the hook installed
(`--hook-install`), switch the MPC on with the stick in, and the hook
soft-reboots into the stick's system on Akai's kernel
(docs/dev/hardware/stick-autoboot.md). On 3.6.0, `--kexec-boot` starts it.
It talks only to the test build: it uses
the newest ~/mpfree-work/kexec-stick-* folder's known_hosts, so Akai's system (a
different host key) is refused before anything runs. Step by step, each
started by you pressing Enter:

  1. Workstation, read-only: checks that the program
     (rust/target/armv7-unknown-linux-gnueabihf/release/mpfree) is a
     static ARM EABI5 hard-float executable, and finds the test build's folder.
  2. Test build, read-only: checks it is the test build (release, no MMC
     device, kexec off, / read-only; it also says whether the boot hook
     started it), that /run has room and allows programs,
     that Akai's app is not running, and lists who holds the sound, MIDI,
     touch and screen devices.
  3. Test build RAM: copies the program to /run/mpfree-app and checks its
     hash there. Nothing else is written; a power-cycle clears it. Skipped
     when the only steps are app and line, which need no copy.
  app: READ-ONLY. The pad player that a stick built with --app starts at
     power-on. First, with no question, how it started (from its log and
     the services' state), partition 4 and its privileges. Then only the
     checks MPFREE_APP_CHECKS names (app_checks.py: `list` shows them all),
     "Check 1 of N" and so on: a hands check shows its steps, waits while
     you do them, says line by line what the app's log showed in that time,
     and asks at most one question, about what only you can see or hear; an
     auto check only reads. With no checks named it asks nothing. Last, the
     whole session from the log (status lines with underruns, the engines'
     counters). It only reads the log, the stick's settings and the
     services' state; it copies, starts and stops nothing.
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
     service; started again at the end) and draws the mpfree card.
  6. touch: shows your touches on the screen for 10 s (reads only, no grab).
  7. pads: listens on all three of the controller's MIDI ports for 15 s and
     shows every message in hex. Only if no pad hit arrived, it offers a
     second try that also sends Akai's keep-alive message every 5 s.
  8. lights: sends Akai's startup pad colours and button lights, then your
     project's pad colours, and lights each pad brighter while you hold it.
  9. drums: plays the first factory drum kit made by offline-factory-kit.py
     from the pads for 30 s. The kits file is copied into /run/mpfree-app
     with the program. Turn the volume knob down first.
 10. setup: asks the controller for a session, as Akai's app does, then sends
     the app's setup messages byte for byte (you type SEND first), lights the
     pads again and listens for 20 s, pinging every 4 s. It comes after drums
     so that the drums play in the pad mode round 2 already saw working.
 11. play: the pad player, MPFree's first GUI (you type SEND first): the same
     session and setup as step 10, then up to 120 s with every kit: the pads
     or the screen play, PAD BANK or SHIFT + wheel picks the kit, Q-Link 1 or the
     slider sets the level, STOP ends it. If the session is not accepted
     within 20 s it plays without Akai's setup. Turn the volume knob down first.
 12. click: plays a 120 BPM click on the ACVA output for 10 s, quietly, and
     counts underruns and late periods. Turn the volume knob down first.
  MPFREE_APP_CHECKS names the app step's checks, comma-separated, in the
  order they run (for example transport,basics,load). Nothing else is asked.
  MPFREE_APP_STEPS picks the steps after the copy, by name, comma-separated
  (default: inventory,screen,touch,pads,click). They always run in the order
  app, line, then the numbered ones above. After lights, drums, setup and play it
  checks that the controller still has its normal USB id. The first step that
  draws on the screen stops the log display or the pad player (our own
  services), since both would draw on /dev/fb0. At the end it starts again
  whichever it stopped, and copies the program's own report
  (/run/mpfree-app/report.txt) into this report.

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
# The device checks a round names (python3 -I leaves this folder off sys.path, so it is loaded by path).
_spec = importlib.util.spec_from_file_location("app_checks", os.path.join(HERE, "app_checks.py"))
checks = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(checks)

PROGRAM_BUILT = os.path.join(REPO_ROOT, "rust", "target", "armv7-unknown-linux-gnueabihf", "release", "mpfree")
PROGRAM_MAX = 16 << 20
KIT_MAX = 48 << 20
KIT_MAGIC = b"MPFKITS1"
REMOTE_DIR = "/run/mpfree-app"
STEPS = ("app", "line", "inventory", "screen", "touch", "pads", "lights", "drums", "setup", "play", "click")
# The steps that use the pad player started at power-on: no copy, and they run before anything stops it.
APP_STEPS = ("app", "line")
DEFAULT_STEPS = "inventory,screen,touch,pads,click"
CONTROLLER_ID = "09e8:0046"
SETUP_WORD = "SEND"
LINE_WORD = "LINE"
# Tests set this so the prompts can be answered without a terminal.
TESTING = os.environ.get("MPFREE_APP_TEST") == "1"

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
# What to do when port 2222 does not answer (the other tools on the test build reuse it).
NOT_RUNNING = ("the test build did not answer on port 2222. Start it first: with the boot hook installed "
               "(--hook-install), switch the MPC off, wait 10 s and switch it on with the stick in (on 3.6.0, "
               "the --kexec-boot block). On Akai's system this tool runs nothing")


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
if [ -e "$R/run/mpfree-hook/soft-reboot" ]; then
  say "started by: the boot hook's soft-reboot (asked at $(cat "$R/run/mpfree-hook/soft-reboot") s after power-on)"
else
  say "started by: no soft-reboot marker: a kexec (the boot hook's opt-in kexec_file route or its chain, or --kexec-boot on 3.6.0), or a direct boot"
fi
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
# Writes only into /run/mpfree-app (RAM). A report from an earlier run in this boot is kept.
PREPARE = r"""R=''
umask 077
d="$R/run/mpfree-app"
[ -d "$d" ] || mkdir "$d" || exit 1
rm -f "$d/mpfree" "$d/mpfree.part" "$d/kit.bin" "$d/kit.bin.part" "$d/kits.bin" "$d/kits.bin.part"
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-app/mpfree.part" && chmod 700 "$R/run/mpfree-app/mpfree.part" && mv "$R/run/mpfree-app/mpfree.part" "$R/run/mpfree-app/mpfree" """
RECEIVE_KIT = r"""R=''; umask 077; cat > "$R/run/mpfree-app/kits.bin.part" && mv "$R/run/mpfree-app/kits.bin.part" "$R/run/mpfree-app/kits.bin" """
HASH = r"""R=''
sha256sum "$R/run/mpfree-app/mpfree"
"""
HASH_KIT = r"""R=''
sha256sum "$R/run/mpfree-app/kits.bin"
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
cd "$R/run/mpfree-app" || exit 1
exec "$R/run/mpfree-app/mpfree" @@ARGS@@
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
# Read-only: the startup screen's unit (only on a stick built with --splash), which plays once per power-on.
SPLASH_STATE = r"""R=''
u=mpfree-test-splash.service
if [ "$(systemctl show -p LoadState --value $u 2>/dev/null)" = loaded ]; then
  echo "startup screen ($u): $(systemctl show -p ActiveState --value $u 2>/dev/null), result $(systemctl show -p Result --value $u 2>/dev/null)"
else
  echo "startup screen ($u): not on this stick"
fi
"""
# Read-only: what partition 4 keeps for the app (its Prefs and the buttons Map controls found).
SETTINGS = r"""R=''
echo "--- app"
cat "$R/run/mpfree-data/settings/app.conf" 2>/dev/null
echo "--- map"
cat "$R/run/mpfree-data/settings/map.conf" 2>/dev/null
true
"""
# Read-only: the stick's partition 4 (saved settings and files): its size and free space, and what the
# online grow at power-on said (it fills the stick since #169; whether 3.10's resize2fs grows it is checked here).
DATA_STATE = r"""R=''
if grep -q ' /run/mpfree-data ' "$R/proc/mounts"; then
  df -k "$R/run/mpfree-data" | tail -n 1 | while read -r dev size used avail rest; do
    echo "partition 4: $size KiB, $used KiB used, $avail KiB free"
  done
else
  echo "partition 4: not mounted"
fi
if [ -f "$R/run/mpfree-data-grow.txt" ]; then
  echo "partition 4 grow:"
  n=0
  while IFS= read -r l && [ "$n" -lt 8 ]; do echo "  $l"; n=$((n + 1)); done < "$R/run/mpfree-data-grow.txt"
else
  echo "partition 4 grow: no /run/mpfree-data-grow.txt"
fi
"""
# Read-only: who the pad player runs as and what it may do (its uid, groups, capabilities, NoNewPrivs, and its
# real-time and locked-memory limits), who owns the device nodes it opens, the framebuffer's `pan` file and the
# eMMC driver's `bind` file (the one write that would undo the switch-off), and the groups it needs. Root on a
# build without --app-user; the mpfree user, with no capabilities, on one with it (offline-kexec-stick.py).
PRIVS_STATE = r"""R=''
n=0
for p in "$R"/proc/[0-9]*; do
  [ "$(cat "$p/comm" 2>/dev/null)" = mpfree ] || continue
  n=$((n + 1))
  while IFS= read -r l; do
    case "$l" in Uid:*|Gid:*|Groups:*|CapEff:*|CapBnd:*|NoNewPrivs:*) echo "privileges: pid ${p##*/} $l" ;; esac
  done < "$p/status"
  while IFS= read -r l; do
    case "$l" in "Max realtime priority"*|"Max locked memory"*) echo "privileges: pid ${p##*/} $l" ;; esac
  done < "$p/limits"
done
[ "$n" -gt 0 ] || echo "privileges: no mpfree process"
for f in "$R"/dev/fb0 "$R"/dev/snd/* "$R"/dev/input/event* "$R"/sys/class/graphics/fb0/pan \
    "$R"/sys/bus/platform/drivers/*dwmmc*/bind; do
  [ -e "$f" ] || continue
  set -- $(ls -lnd "$f")
  echo "privileges: ${f#"$R"} $1 uid $3 gid $4"
done
if [ -f "$R/etc/group" ]; then
  grep -E '^(audio|video|input|systemd-journal|mpfree):' "$R/etc/group" | while IFS= read -r l; do
    echo "privileges: group $l"
  done
fi
"""
# Read-only (the profiling thread's GPU asks, /mnt/project-files/hello-mpc/gpu-inventory-asks.md): the GPU's
# libraries, the release, the display nodes and the panel's mode, which programs hold /dev/dri, whether the
# Panfrost driver is bound, and the display state from debugfs only where it is already mounted (nothing is
# mounted). It opens no device node and changes nothing; a path that isn't there prints "not there".
GPU_STATE = r"""R=''
n=0
for f in "$R"/usr/lib/libEGL* "$R"/usr/lib/libGLESv2* "$R"/usr/lib/libgbm* "$R"/usr/lib/libdrm* \
    "$R"/usr/lib/*/libEGL* "$R"/usr/lib/*/libGLESv2* "$R"/usr/lib/*/libgbm* "$R"/usr/lib/dri/*; do
  if [ -L "$f" ]; then echo "gpu: ${f#"$R"} -> $(readlink "$f")"; n=$((n + 1))
  elif [ -e "$f" ]; then echo "gpu: ${f#"$R"}"; n=$((n + 1)); fi
done
[ "$n" -gt 0 ] || echo "gpu: no libEGL, libGLESv2, libgbm, libdrm or /usr/lib/dri"
if [ -f "$R/etc/os-release" ]; then
  grep -E '^(NAME|VERSION|VERSION_ID|PRETTY_NAME)=' "$R/etc/os-release" | while IFS= read -r l; do echo "gpu: os-release $l"; done
else
  echo "gpu: /etc/os-release not there"
fi
if [ -d "$R/dev/dri" ]; then
  ls "$R/dev/dri" | while IFS= read -r d; do echo "gpu: /dev/dri/$d"; done
else
  echo "gpu: /dev/dri not there"
fi
n=0
for c in "$R"/sys/class/drm/card*-*; do
  [ -d "$c" ] || continue
  n=$((n + 1))
  st=$(cat "$c/status" 2>&1)
  m=""
  if [ -f "$c/modes" ]; then while IFS= read -r l; do m="$m $l"; done < "$c/modes"; fi
  echo "gpu: ${c#"$R"}: $st, modes:${m:- none}"
done
[ "$n" -gt 0 ] || echo "gpu: no /sys/class/drm/card*-* connectors"
n=0
for p in "$R"/proc/[0-9]*; do
  if ls -l "$p/fd" 2>/dev/null | grep -q '/dev/dri/'; then
    echo "gpu: /dev/dri is open in pid ${p##*/} ($(cat "$p/comm" 2>/dev/null))"; n=$((n + 1))
  fi
done
[ "$n" -gt 0 ] || echo "gpu: nothing has /dev/dri open"
if [ -d "$R/sys/bus/platform/drivers/panfrost" ]; then
  b=$(ls "$R/sys/bus/platform/drivers/panfrost" | grep -v -E '^(bind|unbind|uevent|module|new_id|remove_id)$')
  echo "gpu: panfrost driver bound to: ${b:-nothing}"
else
  echo "gpu: panfrost driver not there"
fi
if [ -d "$R/sys/kernel/debug/dri/0" ]; then
  echo "gpu: debugfs dri/0 state (first 80 lines):"
  n=0
  while IFS= read -r l && [ "$n" -lt 80 ]; do echo "gpu:   $l"; n=$((n + 1)); done < "$R/sys/kernel/debug/dri/0/state"
else
  echo "gpu: debugfs dri/0 not there (it is not mounted for this)"
fi
"""
# Read-only, during play (the profiling thread's round 8 asks 2 and 4): the cores, the cpufreq governor and
# clock, the clock source, which core each of the app's threads runs on and its priority, two snapshots of
# the per-core counters 2 s apart, and the interrupt table. It reads /sys and /proc only.
CPU_PROBE = r"""R=''
for f in "$R"/sys/devices/system/cpu/online "$R"/sys/devices/system/cpu/isolated \
    "$R"/sys/devices/system/cpu/cpu[0-3]/cpufreq/scaling_governor \
    "$R"/sys/devices/system/cpu/cpu[0-3]/cpufreq/scaling_cur_freq \
    "$R"/sys/devices/system/cpu/cpu[0-3]/cpufreq/scaling_max_freq \
    "$R"/sys/devices/system/clocksource/clocksource0/current_clocksource; do
  if [ -e "$f" ]; then echo "${f#"$R"}: $(cat "$f" 2>&1)"; else echo "${f#"$R"}: not there"; fi
done
n=0
for p in "$R"/proc/[0-9]*; do
  [ "$(cat "$p/comm" 2>/dev/null)" = mpfree ] || continue
  n=$((n + 1))
  echo "app pid ${p##*/}, its threads (id name: allowed cores, last core, real-time priority, policy 1 = FIFO):"
  for t in "$p"/task/[0-9]*; do
    s=$(cat "$t/stat" 2>/dev/null) || continue
    set -- ${s##*) }
    a=$(grep '^Cpus_allowed_list:' "$t/status" 2>/dev/null)
    echo "  ${t##*/} $(cat "$t/comm" 2>/dev/null): allowed ${a#Cpus_allowed_list:} last_cpu=${37} rt_priority=${38} policy=${39}"
  done
done
echo "app processes: $n"
grep '^cpu' "$R/proc/stat"
sleep 2
grep '^cpu' "$R/proc/stat"
cat "$R/proc/interrupts"
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
cat "$R/run/mpfree-app/report.txt" 2>/dev/null || echo "(no report file)"
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
        return "quit"  # the input ended: stop, as q does
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
    return answer


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
        raise Stop(f"MPFREE_APP_STEPS: unknown step(s) {', '.join(unknown) or '(none given)'}; "
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


# The session summary at the end of the app step: every line that starts with one of these, from
# power-on to the end (the last three status lines only).
SUMMARY_PREFIXES = ("touchscreen: ", "touch-reader: ", "touch: doorbell", "touch: no doorbell",
                    "button guide done: ", "button guide: ", "developer tools: ",
                    "looper: ", "line input: ", "saving: ", "saved settings", "saved buttons", "not saved: ",
                    "storage: ", "capture", "sample: ",
                    "map: ", "map summary: ", "bank", "pad level: ", "kit ", "synth: ", "pads: ", "Latch ",
                    "arp: ", "arp order: ", "arp pattern: ", "pattern ", "recording ", "time: ", "tempo ",
                    "touch-lab-open", "touch-lab: ", "touch-lab-exit", "touch-calibrate", "touch-orientation: ",
                    "STOP: ", "track mute: ", "morph: ", "song: ", "chain: ", "macros: ", "q-link bank: ",
                    "mixer: ", "playing ", "sound cores", "status: sound cores ", "status: screen ", "browser: ",
                    "preview",
                    "MIDI in: ", "MIDI Out", "controller output", "report: ", "chain editor: ", "control: ",
                    "threads: ", "PANIC: ", "safe start: ")
APP_LINE = re.compile(r"^\[\s*(\d+\.\d+)\]\s+(.*)$")
APP_IDENTS = ("sh", "mpfree", "mpfree-hello")  # whose lines in the app unit's log are the app's (app_lines)
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
        # "sh[1246]: text" or "systemd[1]: text": keep the text, and who wrote it if not the app. The log is
        # the app unit's only. Its ExecStart is /bin/sh start-app.sh, which execs the app, so journald names
        # the app's lines "sh" (the final round's creator stick, 16 Nov); a unit that starts the app itself
        # names them "mpfree", or "mpfree-hello" on a stick built before the rename.
        ident, sep, message = rest.partition(": ")
        if not sep:
            message = rest
        elif ident.partition("[")[0] not in APP_IDENTS:
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


def state_reads(mpc, out):
    """Read-only: partition 4, the pad player's privileges and the GPU. The short lines are shown, the rest reported."""
    rc, data = mpc.run(DATA_STATE, test=True)
    for line in (data.strip() or f"partition 4: no answer, exit code {rc}").splitlines():
        out.say(f"  {line}")
    rc, data = mpc.run(PRIVS_STATE, test=True)
    lines = (data.strip() or f"privileges: no answer, exit code {rc}").splitlines()
    for line in lines:
        if re.search(r"Uid:|CapEff:|NoNewPrivs:|no mpfree process|no answer", line):
            out.say(f"  {line}")
        else:
            log_only(out, f"  {line}")
    rc, data = mpc.run(GPU_STATE, test=True)
    lines = (data.strip() or f"gpu: no answer, exit code {rc}").splitlines()
    out.say(f"  (the GPU read: {len(lines)} lines, in the report)")
    for line in lines:
        log_only(out, f"  | {line}")


def start_report(out, lines, splash):
    """Read-only, from the log: how the start went. No question: the app's own lines say it."""
    for line in lines:
        log_only(out, f"      | [{line[0]:10.3f}] {line[1]}")
    ready_at = next((t for t, m in lines if m.startswith("playing ")), None)
    out.say(f"  start-up: {f'playing {ready_at:.1f} s after power-on' if ready_at is not None else 'no playing line yet'}; "
            f"startup screen: {splash}")
    for _, m in lines:
        if m.startswith(("saved settings: ", "saved buttons: ", "sound cores: ", "PANIC: ", "safe start: ",
                         "not saved: ", "power button: ")):
            out.say(f"    {m}")
    bad = sum(1 for _, m in lines if m.startswith(("PANIC: ", "safe start: ")))
    out.say(f"  {'no PANIC or safe start lines' if not bad else f'{bad} PANIC or safe start lines (above)'}; "
            f"the whole log since power-on is in the report")


def touch_count(mpc):
    """The touchscreen's interrupts so far (all cores added up), or None."""
    rc, text = mpc.run(TOUCH_IRQ, test=True)
    m = re.match(r"^\s*\d+:((?:\s+\d+)+)", text)
    return sum(int(x) for x in m.group(1).split()) if m else None


def settings_text(mpc):
    """Partition 4's settings/app.conf and settings/map.conf (read-only), or (None, None)."""
    rc, text = mpc.run(SETTINGS, test=True)
    if rc != 0 or "--- app" not in text:
        return None, None
    app, _, rest = text.partition("--- app\n")[2].partition("--- map\n")
    return app, rest


def wait_status(mpc, out, after, seconds=40):
    """Waits (up to `seconds`) for the app's next status line after `after`, so the check is judged on it."""
    out.say(f"    waiting up to {seconds} s for the app's next status line (one comes every 30 s)...")
    deadline = time.monotonic() + (0 if TESTING else seconds)
    while True:
        lines = app_lines(mpc, after)
        if any(checks.STATUS.match(m) for _, m in lines) or time.monotonic() >= deadline:
            return
        time.sleep(3)


def run_check(mpc, out, k, total, check, settings):
    """One check: its steps (hands) or none (auto), then what the log showed, then its one question."""
    out.say(f"\n  Check {k} of {total}: {check.title} ({check.name})")
    out.say(f"    for: {check.why}")
    before, after = {}, {}
    if check.kind == "hands":
        for line in check.steps:
            out.say(f"    {line}")
        answer = ready(out, "Ready?")
        if answer == "quit":
            raise Stop("stopped by you")
        if answer == "skip":
            return (check.name, "skipped", "")
        before["touch"] = touch_count(mpc)
        start = uptime(mpc)
        while (done := ask("  Go ahead. Press Enter when you are done (q stops here): ")) in ("again", "skip"):
            print("  Please press just Enter when you are done, or q.")
        end = uptime(mpc)
        after["touch"] = touch_count(mpc)
        if check.wait_status and done != "quit":
            wait_status(mpc, out, end)
        session = app_lines(mpc, 0.0)
        window = [(t, m) for t, m in session if t >= start]
        summarize(out, window)
        if done == "quit":
            raise Stop("stopped by you")
    else:
        session = app_lines(mpc, 0.0)
        start = end = session[-1][0] if session else 0.0
        window = []
    verdicts = checks.judge(check, checks.Window(window, session, start, end, before, after, settings))
    out.say("    what the log showed:")
    for ok, what, detail in verdicts:
        out.say(f"      {checks.mark(ok)}  {what}{f' ({detail})' if detail else ''}")
    if check.probe:
        cpu_probe(mpc, out)
    seen = sum(ok is True for ok, _, _ in verdicts)
    result = f"{seen} of {len(verdicts)} seen in the log"
    answer = note(out, check.ask) if check.ask else ""
    return (check.name, result, answer)


def app_check(mpc, out, n, total):
    """The app step: reads the pad player started at power-on, then runs the checks MPFREE_APP_CHECKS names."""
    out.say(f"\n[{n}/{total}] The pad player that started at power-on (read-only: this reads its log and nothing else)")
    rc, text = mpc.run(APP_STATE, test=True)
    state = text.strip() or f"(no answer, exit code {rc})"
    out.say(f"  {state}")
    rc, text = mpc.run(SPLASH_STATE, test=True)
    splash = text.strip().partition("): ")[2] or f"not read (exit code {rc})"
    state_reads(mpc, out)
    start_report(out, app_lines(mpc, 0.0), splash)
    chosen = checks.chosen(os.environ.get("MPFREE_APP_CHECKS", ""))
    if not state.endswith(": active"):
        out.say("  The pad player is not running, so no check runs. The end of its log (all of it is in the report);")
        out.say("  if it stopped with an error, the log display started in its place:")
        for _, m in app_lines(mpc, 0.0)[-15:]:
            out.say(f"    {m}")
        return
    if not chosen:
        out.say("  No checks named (MPFREE_APP_CHECKS), so nothing is asked: the session summary follows.")
    app_text, map_text = settings_text(mpc)
    missing = checks.not_found(chosen, map_text)
    if missing:
        out.say("  Map controls has not found these buttons yet, and a step uses them:")
        for name, control in missing:
            out.say(f"    {control} (check {name})")
        if checks.BY_NAME["map-controls"] not in chosen:
            chosen = [checks.BY_NAME["map-controls"]] + chosen
            out.say("  So map-controls runs first, to find them. If you skip it (s), those steps show NOT SEEN.")
    if chosen:
        out.say(f"  Checks: {', '.join(c.name for c in chosen)}")
    results = [run_check(mpc, out, k, len(chosen), c, app_text) for k, c in enumerate(chosen, start=1)]
    if results:
        out.say("\n  The checks:")
        for name, result, answer in results:
            out.say(f"    {name}: {result}{f'; your answer: {answer}' if answer else ''}")
    session_summary(mpc, out)


def cpu_probe(mpc, out):
    """Read-only: the cores, clock and interrupts while the app plays; the whole table goes to the report."""
    rc, text = mpc.run(CPU_PROBE, test=True)
    lines = text.strip().splitlines() or [f"(no answer, exit code {rc})"]
    out.say("    cores, clock and threads now (read-only):")
    shown = [l for l in lines if not re.match(r"^\s*\d+:|^\s*CPU0", l)]
    for line in shown[:40]:
        out.say(f"      {line}")
    for line in lines:
        log_only(out, f"      | {line}")


LINE_PHASES = (
    ("Line input on, and which jack is which",
     ["Turn the MPC's volume knob down first. Have a source ready (a phone or a synth, not too loud) on a",
      "mono cable. Tap the Looper tab. Under Recording and input, tap the Line input row: it turns on, or the",
      "log says why it stays off. Plug the source into the LEFT line input only and play it:",
      "watch the Input peak row and Play's Loop view meter. Then move the plug to the RIGHT line input only and",
      "play again. (Round 7 only checked the left jack.) A stereo cable in both is fine after that."],
     "Did Line input turn on? Did the meter move with the plug in the left jack, and in the right jack?"),
    ("Loop the line input",
     ["Record from should read Line input now (tap it if not). Press PLAY, then REC: 2 bars from the next",
      "bar line. Play the source into it; then the loop plays, its layer named Line in. Add a layer with",
      "OVERDUB (or REC), end it with REC, UNDO it. Tap Monitor to hear the input through the outputs, then",
      "tap it back to off."],
     "Did the loop record the input and play it back in time? Any clicks, dropouts or noise?"),
    ("Line input off",
     ["Tap the Line input row again: it turns off and Record from goes back to the pads; the loop",
      "keeps playing. Then STOP."],
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
    out.say("  Undo: Line input Off on the Looper tab or in Prefs closes the device. Leaving the pad player, or")
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
    # Every 30 s a set of lines, the pad player's first ("status: N hits"), then the synth's, the
    # looper's, the engines' and the rest: the latest whole set.
    starts = [i for i, m in enumerate(statuses) if re.match(r"status: \d+ hits", m)]
    latest = statuses[starts[-1]:] if starts else statuses[-3:]
    for message in latest or ["(none yet: the first comes 30 s after the app starts playing)"]:
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
    p.add_argument("--program", default=os.environ.get("MPFREE_APP", PROGRAM_BUILT))
    p.add_argument("--report")
    args = p.parse_args(argv)
    out = kexec.Out(args.report)
    out.say("mpfree on the test build (changes RAM on the MPC only)")
    steps = chosen_steps(os.environ.get("MPFREE_APP_STEPS", DEFAULT_STEPS))
    # Checked before anything else, so a typo stops the run at once.
    if os.environ.get("MPFREE_APP_PHASES"):
        raise Stop("MPFREE_APP_PHASES is gone: name the checks with MPFREE_APP_CHECKS instead "
                   "(python3 tools/mpc-one/app_checks.py list)")
    try:
        names = checks.chosen(os.environ.get("MPFREE_APP_CHECKS", ""))
    except checks.Unknown as err:
        raise Stop(f"MPFREE_APP_CHECKS: {err}")
    if names and "app" not in steps:
        raise Stop("MPFREE_APP_CHECKS goes with the app step (MPFREE_APP_STEPS=app)")
    # The app and line steps use the pad player started at power-on; the program is copied only for the others.
    copy = any(n not in APP_STEPS for n in steps)
    total = (3 if copy else 2) + len(steps)

    out.say(f"\n[1/{total}] Workstation checks (read-only)")
    out.say(f"  steps: {', '.join(steps)}")
    program = os.path.realpath(args.program)
    if not os.path.isfile(program):
        raise Stop(f"missing {program}: build it first (see docs/dev/guides/mpfree-app.md)")
    with open(program, "rb") as fh:
        data = fh.read(PROGRAM_MAX + 1)
    problem = arm_static_problem(data)
    if problem:
        raise Stop(f"{program} is {problem}; rebuild it as docs/dev/guides/mpfree-app.md shows")
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
        kit = os.environ.get("MPFREE_APP_KIT") or newest_kit(os.path.realpath(args.work))
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
        raise Stop(NOT_RUNNING)
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
                if step(i, "Screen: our log display stops (it starts again at the end), then the mpfree card",
                        "Watch the MPC's screen."):
                    drawn("screen")
                    note(out, "Is the mpfree card on the screen, and the right way up?")
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
    out.say(f"\nThe program's own report ({REMOTE_DIR}/report.txt):")
    rc, text = mpc.run(REPORT, test=True)
    for line in text.splitlines():
        out.say(f"  | {line}")
    out.say("\nRESULT: " + ("all steps ran (see each step's lines above)." if result == 0 else "stopped early."))
    out.say("NEXT: switch the MPC off with its power switch, take out the stick, wait 10 s, switch it on,")
    out.say("and wait for Akai's system. Then run the boot-map block to confirm nothing on the eMMC changed.")
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
