R=''
# MPFree boot-hook canary: INSTALL. STATE-CHANGING on the MPC's data partition only.
# Streamed over SSH stdin by `collect-over-ssh.sh --hook-canary` (developer mode, port 22, an
# installed Akai 3.10.1 or later). The R='' line above is an empty root prefix; only tests change it.
#
# Question it answers: can something kept in Akai's writable /etc layer (the etc.mount overlay, whose
# upper directory is /data/system/etc/overlay) start a short service at power-on, early enough to jump
# into MPFree before Akai's app draws its screen? systemd ignores start-up files kept there at boot, so
# it tries two other readers of /etc:
#   - udev, which re-reads its rule folders while it runs (at most every 3 s, counted from its start);
#   - the module loader (modprobe), which reads /etc/modprobe.d each time it runs: an "install" line for
#     the Wi-Fi module cfg80211, which Akai's system loads at about 6.4 s on every boot seen so far.
#
# It writes exactly three files, and the folders they need:
#   /data/mpfree-hook/canary.sh        (and /data/mpfree-hook/created-folders.txt, for the undo)
#   /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules
#   /data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf
# canary.sh only writes lines to the kernel log and to /run/mpfree-canary.txt (RAM), and asks systemd to
# run itself once more as a short service (the way a real hook would start the jump), which logs one
# more line. The rule logs every device event once udev has loaded it; the first event of a boot, and
# every USB disk partition that appears, also start that service. The modprobe line logs, starts the
# service, then loads cfg80211 exactly as before (modprobe --ignore-install). Nothing else is read,
# written, mounted, loaded or started.
#
# It never writes the root filesystem, the boot partitions, the U-Boot environment or anything else in
# /etc. A rule udev cannot parse is skipped with a log message, and a modprobe line it cannot parse is
# skipped with a warning; neither can stop the MPC from booting. If canary.sh were missing, the
# modprobe line still loads cfg80211 (its parts are separated by ";").
# Undo: `collect-over-ssh.sh --hook-canary-remove`, which deletes the three files and the folders this
# made (by hand: rm /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules
# /data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf; rm -r /data/mpfree-hook).
# If the write is cut off: the script is written first and the rule and modprobe line after it, each to
# a temporary name that udev and modprobe ignore and then renamed, so neither points at a missing or
# half-written script. An earlier version of the canary (same marker line) is replaced.
set -u
umask 022
say() { printf '%s\n' "$*"; }
stop() { say "STOP: $*"; say "Nothing more was written."; exit 1; }

UPPER="$R/data/system/etc/overlay"
HOOK="$R/data/mpfree-hook"
RULES="$UPPER/udev/rules.d"
RULE="$RULES/99-mpfree-canary.rules"
MODS="$UPPER/modprobe.d"
CONF="$MODS/99-mpfree-canary.conf"
SCRIPT="$HOOK/canary.sh"
CREATED="$HOOK/created-folders.txt"
MARK='# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.'

say "MPFREE BOOT-HOOK CANARY - INSTALL (changes the data partition only: three small files)"
rel=$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)
say "running release: $rel"
case "$rel" in
  6.18.*-az01-*) ;;
  *) stop "this is for Akai's 3.10.1 (kernel 6.18, az01) installed on the MPC; it runs $rel" ;;
esac
grep -q " /etc overlay .*upperdir=/data/system/etc/overlay," "$R/proc/mounts" ||
  stop "/etc is not Akai's overlay with its upper folder in /data/system/etc/overlay"
grep -q ' /data ext4 rw' "$R/proc/mounts" || stop "/data is not mounted read-write"
[ -d "$UPPER" ] || stop "$UPPER is missing"
[ -w "$R/data" ] || stop "cannot write /data (not root?)"
# The modprobe line needs kmod's modprobe (BusyBox's ignores "install" lines and has no --ignore-install).
# It names the kernel's own module loader by its full path, so that a shell that prefers its BusyBox
# applets still runs kmod's.
helper=$(cat "$R/proc/sys/kernel/modprobe" 2>/dev/null)
case "$helper" in
  /*[!A-Za-z0-9/._-]*|'') stop "the kernel's module loader has an unusual path (/proc/sys/kernel/modprobe: '$helper')" ;;
  /*) ;;
  *) stop "the kernel's module loader is not set (/proc/sys/kernel/modprobe: '$helper')" ;;
esac
"$R$helper" --version 2>/dev/null | grep -q '^kmod version' ||
  stop "the kernel's module loader $helper is not kmod's modprobe"
say "modprobe: kmod's ($helper)"

# The exact bytes of the three files. canary.sh starts with its own R='' line, like this script.
SCRIPT_TEXT=$(cat <<'MPFREE_EOF'
R=''
# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.
# Run by udev ("udev" for every USB disk partition that appears, "event" for every device event once
# the rule is loaded), by modprobe when the Wi-Fi module cfg80211 loads ("modprobe"), and by systemd as
# a short service that one of those started ("service"). It only writes lines to the kernel log and to
# /run/mpfree-canary.txt, and a marker folder in /run (RAM). Undo: collect-over-ssh.sh --hook-canary-remove.
PATH=/usr/sbin:/usr/bin:/sbin:/bin   # udev starts programs with no PATH
export PATH
kind=${1:-}
name=$(printf '%s' "${2:-}" | tr -cd 'A-Za-z0-9:._-' | cut -c1-40)
up=$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)
log() { echo "$1" >> "$R/run/mpfree-canary.txt" 2>/dev/null; }
kmsg() { echo "$1" > "$R/dev/kmsg" 2>/dev/null; log "$1"; }
state() {   # read from /sys and /proc only: no call to systemd, which may itself be waiting on modprobe
  app='not started'
  for d in "$R/sys/fs/cgroup/system.slice/acvs.service" "$R/sys/fs/cgroup/systemd/system.slice/acvs.service"; do
    [ -d "$d" ] && app=started
  done
  if pidof MPC >/dev/null 2>&1; then mpc=running; else mpc='not running'; fi
  echo "acvs.service: $app; MPC program: $mpc"
}
start() {   # $1 = who started it; the unit name makes it once per boot for each starter
  systemd-run --no-block --collect --quiet --unit="mpfree-canary-$1" --description="MPFree canary ($1)" \
    /bin/sh /data/mpfree-hook/canary.sh service "$1" > /dev/null 2>&1 || kmsg "mpfree-canary: systemd-run failed for $1"
}
comm() { tr -cd 'A-Za-z0-9:._/ -' < "/proc/$1/comm" 2>/dev/null | cut -c1-20; }   # live processes: never R
ppid() { sed -n 's/^PPid:[[:space:]]*//p' "/proc/$1/status" 2>/dev/null; }
case "$kind" in
  event)
    log "mpfree-canary: event ${ACTION:-?} ${SUBSYSTEM:-?} $name at ${up:-?} s"
    if mkdir "$R/run/mpfree-canary-first-event" 2>/dev/null; then
      kmsg "mpfree-canary: first event after udev loaded the rule: ${ACTION:-?} ${SUBSYSTEM:-?} $name at ${up:-?} s; $(state)"
      start first-event
    fi ;;
  udev)
    case "$name" in sd[a-z][0-9]|sd[a-z][0-9][0-9]) ;; *) exit 0 ;; esac
    kmsg "mpfree-canary: udev saw $name at ${up:-?} s; $(state); partition name: ${ID_PART_ENTRY_NAME:-not given}"
    start "$name" ;;
  modprobe)
    [ "$name" = cfg80211 ] || exit 0
    p1=$PPID; p2=$(ppid "$p1"); p3=$(ppid "${p2:-0}"); p4=$(ppid "${p3:-0}")
    kmsg "mpfree-canary: modprobe loads $name at ${up:-?} s; $(state); called by: $(comm "$p1") < $(comm "${p2:-0}") < $(comm "${p3:-0}") < $(comm "${p4:-0}")"
    start modprobe & ;;
  service)
    case "$name" in first-event|modprobe|sd[a-z][0-9]|sd[a-z][0-9][0-9]) ;; *) exit 0 ;; esac
    labels=$(ls "$R/dev/disk/by-partlabel" 2>/dev/null | tr -cd 'A-Za-z0-9._\n-' | tr '\n' ' ' | cut -c1-200 | sed 's/ *$//')
    kmsg "mpfree-canary: service ($name) ran at ${up:-?} s; $(state); systemd: acvs.service $(systemctl is-active acvs.service 2>/dev/null); partition labels: ${labels:-none}" ;;
esac
exit 0
MPFREE_EOF
)
RULE_TEXT=$(cat <<'MPFREE_EOF'
# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.
# Logs when a USB disk partition appears, and every device event once udev has loaded this file; the
# first of those also starts a short service. Undo: collect-over-ssh.sh --hook-canary-remove.
ACTION=="add", SUBSYSTEM=="block", KERNEL=="sd[a-z][0-9]*", RUN+="/bin/sh /data/mpfree-hook/canary.sh udev $kernel"
RUN+="/bin/sh /data/mpfree-hook/canary.sh event $kernel"
MPFREE_EOF
)
CONF_TEXT=$(cat <<'MPFREE_EOF'
# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.
# When the Wi-Fi module cfg80211 loads, it logs a line and starts a short service, then loads the
# module as before. Undo: collect-over-ssh.sh --hook-canary-remove.
install cfg80211 export PATH=/usr/sbin:/usr/bin:/sbin:/bin; /bin/sh /data/mpfree-hook/canary.sh modprobe cfg80211; MPFREE_HELPER --ignore-install cfg80211 $CMDLINE_OPTS
MPFREE_EOF
)
CONF_TEXT=$(printf '%s\n' "$CONF_TEXT" | sed "s|MPFREE_HELPER|$helper|")

# Already installed (the same bytes)? Something that is not the canary's in the way?
same() { [ -f "$1" ] && [ "$(cat "$1")" = "$2" ]; }
ours() { [ -f "$1" ] && grep -qxF "$MARK" "$1"; }
if same "$SCRIPT" "$SCRIPT_TEXT" && same "$RULE" "$RULE_TEXT" && same "$CONF" "$CONF_TEXT"; then
  say "already in place, unchanged:"
  sha256sum "$SCRIPT" "$RULE" "$CONF" | sed "s|$R/|/|"
  say "RESULT: the canary is installed."
  exit 0
fi
for f in "$SCRIPT" "$RULE" "$CONF"; do
  [ ! -e "$f" ] || ours "$f" || stop "${f#$R} exists and is not this canary's file"
done
for d in "$HOOK" "$UPPER/udev" "$RULES" "$MODS"; do
  [ ! -e "$d" ] || [ -d "$d" ] || stop "${d#$R} exists and is not a folder"
done
if ours "$SCRIPT" || ours "$RULE"; then
  say "an earlier version of the canary is in place; it is replaced"
fi

# Folders, recording the ones made here (in order) for the undo.
new=""
for d in "$HOOK" "$UPPER/udev" "$RULES" "$MODS"; do
  if [ ! -d "$d" ]; then
    mkdir "$d" || stop "could not make ${d#$R}"
    new="$new${d#$R}
"
  fi
done
if [ -n "$new" ]; then
  printf '%s' "$new" >> "$CREATED" || stop "could not record the new folders in ${CREATED#$R}"
fi

put() {   # $1 = file, $2 = text, $3 = mode: written to a temporary name, then renamed over the file
  tmp="$(dirname "$1")/.$(basename "$1").new"
  printf '%s\n' "$2" > "$tmp" && chmod "$3" "$tmp" && mv -f "$tmp" "$1" || stop "could not write ${1#$R}"
}
put "$SCRIPT" "$SCRIPT_TEXT" 0755
put "$RULE" "$RULE_TEXT" 0644
put "$CONF" "$CONF_TEXT" 0644
sync

same "$SCRIPT" "$SCRIPT_TEXT" || stop "${SCRIPT#$R} does not read back as written"
same "$RULE" "$RULE_TEXT" || stop "${RULE#$R} does not read back as written"
same "$CONF" "$CONF_TEXT" || stop "${CONF#$R} does not read back as written"
say "written and read back:"
sha256sum "$SCRIPT" "$RULE" "$CONF" | sed "s|$R/|/|"
[ -n "$new" ] && { say "folders made (the undo removes them again):"; printf '%s' "$new" | sed 's/^/  /'; }
say "RESULT: the canary is installed. It acts from the next power-on."
