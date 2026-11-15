R=''
# MPFree boot-hook canary: INSTALL. STATE-CHANGING on the MPC's data partition only.
# Streamed over SSH stdin by `collect-over-ssh.sh --hook-canary` (developer mode, port 22, an
# installed Akai 3.10.1 or later). The R='' line above is an empty root prefix; only tests change it.
#
# Question it answers: does udev, at power-on, read a rule kept in Akai's writable /etc layer
# (the etc.mount overlay, whose upper directory is /data/system/etc/overlay), early enough to act on a
# USB stick before Akai's app starts? systemd ignores start-up files kept there at boot; udev re-reads
# its rule folders while it runs, so it may not.
#
# It writes exactly two files, and the folders they need:
#   /data/mpfree-hook/canary.sh        (and /data/mpfree-hook/created-folders.txt, for the undo)
#   /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules
# The rule matches every USB disk partition that appears and runs canary.sh, which only writes one
# line to the kernel log and to /run/mpfree-canary.txt (RAM), then asks systemd to run canary.sh once
# more as a short service (the way a real hook would start the jump) to log a second line from there.
# A second line in the rule notes every other device event (add or change) in /run/mpfree-canary.txt,
# which shows when udev loaded the rule and which later events a real hook could wait for.
# Nothing else is read, written, mounted, loaded or started.
#
# It never writes the root filesystem, the boot partitions, the U-Boot environment or anything else in
# /etc. A rule udev cannot parse is skipped with a log message; it cannot stop the MPC from booting.
# Undo: `collect-over-ssh.sh --hook-canary-remove`, which deletes the two files and the folders this
# made (by hand: rm /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules; rm -r /data/mpfree-hook).
# If the write is cut off: the script is written first and the rule last, each to a temporary name that
# udev ignores and then renamed, so the rule never points at a missing or half-written script.
set -u
umask 022
say() { printf '%s\n' "$*"; }
stop() { say "STOP: $*"; say "Nothing more was written."; exit 1; }

UPPER="$R/data/system/etc/overlay"
HOOK="$R/data/mpfree-hook"
RULES="$UPPER/udev/rules.d"
RULE="$RULES/99-mpfree-canary.rules"
SCRIPT="$HOOK/canary.sh"
CREATED="$HOOK/created-folders.txt"

say "MPFREE BOOT-HOOK CANARY - INSTALL (changes the data partition only: two small files)"
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

# The exact bytes of both files. canary.sh starts with its own R='' line, like this script.
SCRIPT_TEXT=$(cat <<'MPFREE_EOF'
R=''
# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.
# Run by udev: "udev" for every USB disk partition that appears, "event" for every other device event
# once this rule is loaded; and once more by systemd as a short service ("service"). It only writes
# lines to the kernel log and to /run/mpfree-canary.txt (RAM). Undo: collect-over-ssh.sh --hook-canary-remove.
PATH=/usr/sbin:/usr/bin:/sbin:/bin   # udev starts programs with no PATH
export PATH
kind=${1:-}
name=$(printf '%s' "${2:-}" | tr -cd 'A-Za-z0-9:._-' | cut -c1-40)
up=$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)
case "$kind" in
  event)
    echo "mpfree-canary: event ${ACTION:-?} ${SUBSYSTEM:-?} $name at ${up:-?} s" >> "$R/run/mpfree-canary.txt" 2>/dev/null
    exit 0 ;;
  udev|service)
    case "$name" in sd[a-z][0-9]|sd[a-z][0-9][0-9]) ;; *) exit 0 ;; esac ;;
  *) exit 0 ;;
esac
app=$(systemctl is-active acvs.service 2>/dev/null)
if pidof MPC >/dev/null 2>&1; then mpc=running; else mpc='not running'; fi
line="mpfree-canary: $kind saw $name at ${up:-?} s; acvs.service: ${app:-unknown}; MPC program: $mpc; partition name: ${ID_PART_ENTRY_NAME:-not given}"
echo "$line" > "$R/dev/kmsg" 2>/dev/null
echo "$line" >> "$R/run/mpfree-canary.txt" 2>/dev/null
if [ "$kind" = udev ]; then
  systemd-run --no-block --collect --quiet --unit="mpfree-canary-$name" /bin/sh /data/mpfree-hook/canary.sh service "$name" \
    > /dev/null 2>&1 || echo "mpfree-canary: systemd-run failed for $name" > "$R/dev/kmsg" 2>/dev/null
fi
exit 0
MPFREE_EOF
)
RULE_TEXT=$(cat <<'MPFREE_EOF'
# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.
# Logs when a USB disk partition appears, and every device event once udev has loaded this file.
# Undo: collect-over-ssh.sh --hook-canary-remove.
ACTION=="add", SUBSYSTEM=="block", KERNEL=="sd[a-z][0-9]*", RUN+="/bin/sh /data/mpfree-hook/canary.sh udev $kernel"
ACTION=="add|change", RUN+="/bin/sh /data/mpfree-hook/canary.sh event $kernel"
MPFREE_EOF
)

# Already installed (the same bytes) or something else in the way?
same() { [ -f "$1" ] && [ "$(cat "$1")" = "$2" ]; }
if same "$SCRIPT" "$SCRIPT_TEXT" && same "$RULE" "$RULE_TEXT"; then
  say "already in place, unchanged:"
  sha256sum "$SCRIPT" "$RULE" | sed "s|$R/|/|"
  say "RESULT: the canary is installed."
  exit 0
fi
[ ! -e "$RULE" ] || same "$RULE" "$RULE_TEXT" || stop "$RULE exists and is not this canary's rule"
[ ! -e "$SCRIPT" ] || same "$SCRIPT" "$SCRIPT_TEXT" || stop "$SCRIPT exists and is not this canary's script"
for d in "$HOOK" "$UPPER/udev" "$RULES"; do
  [ ! -e "$d" ] || [ -d "$d" ] || stop "$d exists and is not a folder"
done

# Folders, recording the ones made here (in order) for the undo.
new=""
for d in "$HOOK" "$UPPER/udev" "$RULES"; do
  if [ ! -d "$d" ]; then
    mkdir "$d" || stop "could not make $d"
    new="$new${d#$R}
"
  fi
done
if [ -n "$new" ]; then
  printf '%s' "$new" >> "$CREATED" || stop "could not record the new folders in $CREATED"
fi

printf '%s\n' "$SCRIPT_TEXT" > "$HOOK/.canary.sh.new" && chmod 0755 "$HOOK/.canary.sh.new" &&
  mv -f "$HOOK/.canary.sh.new" "$SCRIPT" || stop "could not write $SCRIPT"
printf '%s\n' "$RULE_TEXT" > "$RULES/.99-mpfree-canary.rules.new" && chmod 0644 "$RULES/.99-mpfree-canary.rules.new" &&
  mv -f "$RULES/.99-mpfree-canary.rules.new" "$RULE" || stop "could not write $RULE"
sync

same "$SCRIPT" "$SCRIPT_TEXT" || stop "$SCRIPT does not read back as written"
same "$RULE" "$RULE_TEXT" || stop "$RULE does not read back as written"
say "written and read back:"
sha256sum "$SCRIPT" "$RULE" | sed "s|$R/|/|"
[ -n "$new" ] && { say "folders made (the undo removes them again):"; printf '%s' "$new" | sed 's/^/  /'; }
say "RESULT: the canary is installed. It acts from the next power-on (udev may also pick it up now)."
