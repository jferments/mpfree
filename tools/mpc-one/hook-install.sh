R=''
# MPFree boot hook: INSTALL. STATE-CHANGING on the MPC's data partition only, then a check that changes
# only RAM. Streamed over SSH stdin by `collect-over-ssh.sh --hook-install` (developer mode, port 22, an
# installed Akai 3.10.1 or later). The R='' line above is an empty root prefix; only tests change it.
#
# What the hook does, from the next power-on: with MPFree's stick in (a USB disk partition named
# mpfree-test-root, written by write-test-stick.py from a build of offline-kexec-stick.py), the MPC starts
# MPFree from the stick before Akai's app starts. When the Wi-Fi module cfg80211 loads, about 6 s after
# power-on, a modprobe line runs the hook; it starts a short service that Akai's app and its firmware
# updaters wait for. The service mounts the stick's partition 1 read-only, copies its /boot/mpfree-kexec
# files (kernel, device tree, command line) to /run, checks them against their SHA256SUMS and the stick,
# loads them with kexec and has systemd restart into them. A udev rule is the backup trigger. Without the
# stick, or on any problem, it does nothing and Akai's system starts as usual. This was tried first with a
# harmless canary (hook-canary-install.sh): the modprobe line ran at 5.96 s, its service at 6.09 s, and
# Akai's app started at 6.36 s.
#
# It writes exactly three files, and the folders they need:
#   /data/mpfree-hook/hook.sh            (and /data/mpfree-hook/created-folders.txt, for the undo)
#   /data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules
#   /data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf
# and deletes the canary's three files, if they are there (only files carrying the canary's marker line).
# It never writes the root filesystem, the boot partitions, the U-Boot environment or anything else in
# /etc. A rule udev cannot parse is skipped with a log message, and a modprobe line it cannot parse is
# skipped with a warning; neither can stop the MPC from booting. If hook.sh were missing, the modprobe line
# still loads cfg80211 (its parts are separated by ";").
#
# Then, with the stick in the MPC now, it runs the hook's own check (`hook.sh check`): the same steps as at
# power-on, with the kernel loaded into memory and unloaded again (kexec --load, kexec --unload) instead
# of the restart. That changes only RAM; the stick's partition is mounted read-only, or read where Akai's
# system has already mounted it.
# Undo: `collect-over-ssh.sh --hook-remove`, which deletes the three files and the folders this made (by
# hand: rm /data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules
# /data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf; rm -r /data/mpfree-hook). Without the stick
# the hook does nothing, so taking the stick out is enough to get Akai's system at power-on.
# If the write is cut off: the script is written first, the rule and modprobe line after it, each to a
# temporary name that udev and modprobe ignore and then renamed, so neither points at a missing or
# half-written script. Running this again finishes the job.
set -u
umask 022
say() { printf '%s\n' "$*"; }
stop() { say "STOP: $*"; say "Nothing more was written."; exit 1; }

UPPER="$R/data/system/etc/overlay"
HOOK="$R/data/mpfree-hook"
RULES="$UPPER/udev/rules.d"
RULE="$RULES/99-mpfree-hook.rules"
MODS="$UPPER/modprobe.d"
CONF="$MODS/99-mpfree-hook.conf"
SCRIPT="$HOOK/hook.sh"
CREATED="$HOOK/created-folders.txt"
MARK='# MPFree boot hook, written by tools/mpc-one/hook-install.sh.'
CANARY_MARK='# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.'
CANARY="$MODS/99-mpfree-canary.conf $MODS/.99-mpfree-canary.conf.new $RULES/99-mpfree-canary.rules \
$RULES/.99-mpfree-canary.rules.new $HOOK/canary.sh $HOOK/.canary.sh.new"

say "MPFREE BOOT HOOK - INSTALL (changes the data partition only: three small files; then a check, RAM only)"
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
[ -x "$R/usr/sbin/kexec" ] || stop "no /usr/sbin/kexec on this system"
[ "$(cat "$R/proc/sys/kernel/kexec_load_disabled" 2>/dev/null)" = 0 ] || stop "kexec is turned off on this system"
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

# The exact bytes of the three files. hook.sh starts with its own R='' line, like this script.
SCRIPT_TEXT=$(cat <<'MPFREE_EOF'
R=''
# MPFree boot hook, written by tools/mpc-one/hook-install.sh.
# With MPFree's stick in at power-on (a USB disk partition named mpfree-test-root), it starts MPFree from
# the stick before Akai's app starts: it loads the stick's kernel and device tree with kexec, and systemd
# restarts into them. Without that stick it does nothing. It is run:
#   modprobe  by the module loader, when the Wi-Fi module cfg80211 loads (about 6 s after power-on);
#   udev      by udev, for a device event (the backup, if cfg80211 does not load);
#   jump      by systemd, as the service the first of those started (Akai's app and its firmware
#             updaters wait for it to finish);
#   check     by hook-install.sh: the same checks, and the kernel loaded and unloaded, without restarting.
# It writes only to /run (RAM) and the kernel log, mounts the stick's partition 1 read-only for a moment,
# and loads a kernel into memory. On any problem it stops, cleans up and leaves Akai's system to start as
# usual. Undo: collect-over-ssh.sh --hook-remove, or take the stick out.
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin   # udev and modprobe give no useful PATH (tests set R and theirs)
export PATH
LABEL=mpfree-test-root
KDIR=boot/mpfree-kexec
HOLD='acvs.service touch-fw-update.service xmos-update.service az01-usbsata-fixer.service'
UPDATERS='touch-fw-update.service xmos-update.service az01-usbsata-fixer.service'
FLASHERS='dfu-util touch-fw-upd xmosdfu midifirmup az01-update az01-usbsata-fixer'
LATE=15   # seconds after power-on; later than this it never starts the jump
kind=${1:-}
up=$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)
log() { echo "mpfree-hook: $1" >> "$R/run/mpfree-hook.txt" 2>/dev/null; }
kmsg() { echo "mpfree-hook: $1" > "$R/dev/kmsg" 2>/dev/null; log "$1"; }

# --- modprobe, udev: start the jump once per boot, with the stick in, early in the boot -------------------
start_jump() {
  before=''
  for u in $HOLD; do before="$before --property=Before=$u"; done
  # shellcheck disable=SC2086
  systemd-run --no-block --collect --quiet --unit=mpfree-jump --description="MPFree: start from the stick" \
    --property=Type=oneshot --property=TimeoutStartSec=45 $before \
    /bin/sh /data/mpfree-hook/hook.sh jump > /dev/null 2>&1 ||
    kmsg "systemd-run failed; Akai's system starts as usual"
}
trigger() {   # no call to systemd here but the service start: systemd may itself be waiting on modprobe
  [ -d "$R/run/mpfree-hook" ] && return 0
  s=${up%%.*}
  case "$s" in ''|*[!0-9]*) return 0 ;; esac
  if [ "$s" -ge "$LATE" ]; then
    mkdir "$R/run/mpfree-hook" 2>/dev/null && log "$1 at $up s: past $LATE s, so not this boot"
    return 0
  fi
  if [ ! -e "$R/dev/disk/by-partlabel/$LABEL" ]; then   # MPFree's stick is not in
    [ "$1" = modprobe ] && log "modprobe at $up s: MPFree's stick is not in"
    return 0
  fi
  mkdir "$R/run/mpfree-hook" 2>/dev/null || return 0
  kmsg "MPFree's stick is in: $1 starts the jump at $up s"
  if [ "$1" = modprobe ]; then start_jump & else start_jump; fi
}

# --- jump, check ------------------------------------------------------------------------------------------
say() { if [ "$kind" = check ]; then echo "$1"; else kmsg "$1"; fi; }
W=''; K=''; mnt=''; ours=''; loaded=''
cleanup() {
  if [ "$loaded" = yes ] && [ "$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)" != 0 ]; then
    "$R/usr/sbin/kexec" --unload --kexec-syscall && say "kernel unloaded (kexec --unload)"
  fi
  loaded=''
  if [ "$ours" = yes ]; then umount "$mnt" 2>/dev/null || say "could not unmount the stick's partition 1"; fi
  ours=''
  [ -n "$K" ] && rm -f "$K/zImage" "$K/kexec.dtb" "$K/cmdline.txt" "$K/SHA256SUMS" && rmdir "$K" 2>/dev/null
  [ -n "$W" ] && rmdir "$W/stick" 2>/dev/null
  [ "$kind" = check ] && [ -n "$W" ] && rmdir "$W" 2>/dev/null
  return 0
}
fail() { say "STOP: $1"; cleanup; say "nothing was started: Akai's system starts as usual"; exit 1; }
size() { wc -c < "$1" 2>/dev/null | tr -cd 0-9; }
busy() {   # prints what may be writing firmware right now
  for u in $UPDATERS; do
    case "$(systemctl is-active "$u" 2>/dev/null)" in activating|deactivating|reloading) echo "$u"; return 0 ;; esac
  done
  for p in $FLASHERS; do pidof "$p" > /dev/null 2>&1 && { echo "$p"; return 0; }; done
  grep -qs 'SME0/Firmware/update.sh' "$R"/proc/[0-9]*/cmdline && { echo "Akai's controller update.sh"; return 0; }
  return 1
}

prepare() {   # $1 = work folder in /run. Finds the stick, copies and checks its kexec files.
  W=$1; K="$W/kexec"
  dev=$(readlink -f "$R/dev/disk/by-partlabel/$LABEL" 2>/dev/null)
  [ -n "$dev" ] && [ -e "$dev" ] || fail "MPFree's stick is not in (no partition named $LABEL)"
  name=${dev##*/}
  case "$name" in sd[a-z]1) ;; *) fail "the partition named $LABEL is $name, not partition 1 of a USB disk" ;; esac
  case "$(readlink -f "$R/sys/class/block/$name")" in */usb*) ;; *) fail "/dev/$name is not on USB" ;; esac
  uuid=''
  for l in "$R"/dev/disk/by-partuuid/*; do
    [ "$(readlink -f "$l")" = "$dev" ] && uuid=${l##*/}
  done
  case "$uuid" in
    [0-9a-f]*-*-*-*-*) ;;
    *) fail "/dev/$name has no partition UUID link" ;;
  esac
  say "stick: /dev/$name (partition 1 of a USB disk), at $up s"
  mkdir -p "$W" && mkdir -p "$K" || fail "could not make $K"
  avail=$(df -k "$R/run" 2>/dev/null | tail -n 1 | while read -r a b c d rest; do echo "$d"; done)
  [ "${avail:-0}" -ge 65536 ] 2>/dev/null || fail "less than 64 MiB free in /run"
  mnt=''
  while read -r d m rest; do
    [ "$d" = "/dev/$name" ] && [ -z "$mnt" ] && mnt=$m
  done < "$R/proc/mounts"
  if [ -n "$mnt" ]; then
    case "$mnt" in *[!A-Za-z0-9/._-]*) fail "/dev/$name is mounted at an unusual path" ;; esac
    mnt="$R$mnt"
    say "partition 1 is mounted already at ${mnt#$R}; read from there"
  else
    mkdir -p "$W/stick" || fail "could not make $W/stick"
    mnt="$W/stick"
    mount -t ext4 -o ro,noload,nodev,nosuid,noexec "$dev" "$mnt" || fail "could not mount /dev/$name read-only"
    ours=yes
  fi
  B="$mnt/$KDIR"
  [ -f "$B/SHA256SUMS" ] || fail "the stick has no /$KDIR (it was made before offline-kexec-stick.py added it)"
  link=$(readlink "$B/zImage")
  rel=${link#../zImage-}
  case "$link" in ../zImage-*) ;; *) fail "/$KDIR/zImage is not a link to ../zImage-<release>" ;; esac
  case "$rel" in ''|*[!A-Za-z0-9._+-]*) fail "/$KDIR/zImage links to an unusual name" ;; esac
  [ -f "$mnt/boot/zImage-$rel" ] && [ ! -L "$mnt/boot/zImage-$rel" ] || fail "the stick has no /boot/zImage-$rel"
  for n in zImage:33554432 kexec.dtb:1048576 cmdline.txt:1024 SHA256SUMS:1024; do
    f=${n%%:*}; max=${n#*:}
    [ -f "$B/$f" ] || fail "the stick has no /$KDIR/$f"
    bytes=$(size "$B/$f")
    [ -n "$bytes" ] && [ "$bytes" -gt 0 ] && [ "$bytes" -le "$max" ] || fail "/$KDIR/$f has an unusual size (${bytes:-?} bytes)"
    cp "$B/$f" "$K/$f" || fail "could not copy /$KDIR/$f"
  done
  if [ "$ours" = yes ]; then
    ours=''
    umount "$mnt" || fail "could not unmount the stick's partition 1"
  fi
  for f in zImage kexec.dtb cmdline.txt; do
    want=''; count=0
    while read -r sum file rest; do
      [ "$file" = "$f" ] && [ -z "$rest" ] && { want=$sum; count=$((count + 1)); }
    done < "$K/SHA256SUMS"
    [ "$count" = 1 ] || fail "SHA256SUMS on the stick has no single line for $f"
    got=$(sha256sum < "$K/$f" | cut -d' ' -f1)
    [ "$got" = "$want" ] || fail "$f on the stick does not match its SHA256SUMS (damaged? write the stick again)"
  done
  say "kexec files: kernel $rel, checked against SHA256SUMS"
  [ "$(wc -l < "$K/cmdline.txt" | tr -cd 0-9)" = 1 ] || fail "cmdline.txt is not one line"
  cmdline=$(cat "$K/cmdline.txt")
  [ -z "$(printf '%s' "$cmdline" | tr -d 'A-Za-z0-9=._,:/ -')" ] || fail "cmdline.txt has unusual characters"
  case " $cmdline " in
    *" root=PARTUUID=$uuid "*) ;;
    *) fail "cmdline.txt does not start the system on this stick's partition 1" ;;
  esac
  for a in ro rootwait=60 panic=10 sysctl.kernel.kexec_load_disabled=1; do
    case " $cmdline " in *" $a "*) ;; *) fail "cmdline.txt lacks $a" ;; esac
  done
  say "command line: $(printf '%s' "$cmdline" | sed 's/PARTUUID=[^ ]*/PARTUUID=(this stick)/')"
}

load() {
  [ -x "$R/usr/sbin/kexec" ] || fail "no /usr/sbin/kexec"
  [ "$(cat "$R/proc/sys/kernel/kexec_load_disabled" 2>/dev/null)" = 0 ] || fail "kexec is turned off on this system"
  [ "$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)" = 0 ] || fail "a kexec kernel is already loaded"
  loaded=yes
  "$R/usr/sbin/kexec" --load --kexec-syscall --dtb="$K/kexec.dtb" --command-line="$cmdline" "$K/zImage" ||
    fail "kexec --load refused the stick's kernel"
  [ "$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)" = 1 ] || fail "kexec_loaded is not 1 after kexec --load"
  say "kernel loaded (kexec --load) at $(cut -d' ' -f1 "$R/proc/uptime") s"
}

case "$kind" in
  modprobe|udev) trigger "$kind" ;;
  jump)
    [ -d "$R/run/mpfree-hook" ] || exit 0   # only as started by a trigger
    say "jump service started at $up s; Akai's app: $(systemctl is-active acvs.service 2>/dev/null)"
    prepare "$R/run/mpfree-hook"
    n=0
    while what=$(busy); do
      [ "$n" -ge 20 ] && fail "$what is still running after 20 s; no restart while firmware may be written"
      [ "$n" = 0 ] && say "waiting for $what to finish"
      sleep 1; n=$((n + 1))
    done
    load
    what=$(busy) && fail "$what started; no restart while firmware may be written"
    say "restarting into MPFree (kexec.target) at $(cut -d' ' -f1 "$R/proc/uptime") s"
    systemctl --no-block --job-mode=replace-irreversibly start kexec.target ||
      fail "systemctl could not start kexec.target" ;;
  check)
    prepare "$R/run/mpfree-hook-check"
    what=$(busy) && say "note: $what is running now (at power-on the jump waits for it)"
    load
    cleanup
    [ "$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)" = 0 ] || { say "STOP: kexec_loaded is still not 0"; exit 1; }
    say "CHECK-OK: at power-on with this stick in, the MPC would restart into it" ;;
  *) exit 0 ;;
esac
exit 0
MPFREE_EOF
)
RULE_TEXT=$(cat <<'MPFREE_EOF'
# MPFree boot hook, written by tools/mpc-one/hook-install.sh.
# The backup trigger: with MPFree's stick in, the first device event that udev handles after it has read
# this file runs the hook, unless the modprobe line has already started it (then /run/mpfree-hook exists).
# Undo: collect-over-ssh.sh --hook-remove.
TEST=="/run/mpfree-hook", GOTO="mpfree_hook_end"
TEST!="/dev/disk/by-partlabel/mpfree-test-root", GOTO="mpfree_hook_end"
RUN+="/bin/sh /data/mpfree-hook/hook.sh udev"
LABEL="mpfree_hook_end"
MPFREE_EOF
)
CONF_TEXT=$(cat <<'MPFREE_EOF'
# MPFree boot hook, written by tools/mpc-one/hook-install.sh.
# When the Wi-Fi module cfg80211 loads (about 6 s after power-on), the hook looks for MPFree's stick and,
# with it in, starts the jump in the background; then the module loads as before.
# Undo: collect-over-ssh.sh --hook-remove.
install cfg80211 export PATH=/usr/sbin:/usr/bin:/sbin:/bin; /bin/sh /data/mpfree-hook/hook.sh modprobe; MPFREE_HELPER --ignore-install cfg80211 $CMDLINE_OPTS
MPFREE_EOF
)
CONF_TEXT=$(printf '%s\n' "$CONF_TEXT" | sed "s|MPFREE_HELPER|$helper|")

same() { [ -f "$1" ] && [ "$(cat "$1")" = "$2" ]; }
ours() { [ -f "$1" ] && grep -qxF "$MARK" "$1"; }
canary() { [ -f "$1" ] && grep -qxF "$CANARY_MARK" "$1"; }
if same "$SCRIPT" "$SCRIPT_TEXT" && same "$RULE" "$RULE_TEXT" && same "$CONF" "$CONF_TEXT"; then
  wrote=no
  say "already in place, unchanged:"
else
  wrote=yes
  for f in "$SCRIPT" "$RULE" "$CONF"; do
    [ ! -e "$f" ] || ours "$f" || stop "${f#$R} exists and is not this hook's file"
  done
  for f in $CANARY; do
    [ ! -e "$f" ] || canary "$f" || stop "${f#$R} exists and is not the canary's file"
  done
  for d in "$HOOK" "$UPPER/udev" "$RULES" "$MODS"; do
    [ ! -e "$d" ] || [ -d "$d" ] || stop "${d#$R} exists and is not a folder"
  done
  ours "$SCRIPT" && say "an earlier version of the hook is in place; it is replaced"

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
  # The canary goes before the new rule and modprobe line: its own first, then its script.
  for f in $CANARY; do
    [ -e "$f" ] || continue
    rm -f "$f" && say "deleted the canary's ${f#$R}" || stop "could not delete ${f#$R}"
  done
  put "$RULE" "$RULE_TEXT" 0644
  put "$CONF" "$CONF_TEXT" 0644
  sync
  same "$SCRIPT" "$SCRIPT_TEXT" || stop "${SCRIPT#$R} does not read back as written"
  same "$RULE" "$RULE_TEXT" || stop "${RULE#$R} does not read back as written"
  same "$CONF" "$CONF_TEXT" || stop "${CONF#$R} does not read back as written"
  say "written and read back:"
fi
sha256sum "$SCRIPT" "$RULE" "$CONF" | sed "s|$R/|/|"
[ "$wrote" = yes ] && [ -n "$new" ] && { say "folders made (the undo removes them again):"; printf '%s' "$new" | sed 's/^/  /'; }

say ""
say "=== Check with MPFree's stick (RAM only: the kernel is loaded and unloaded again) ==="
if [ ! -e "$R/dev/disk/by-partlabel/mpfree-test-root" ]; then
  say "MPFree's stick is not in the MPC (no partition named mpfree-test-root), so it was not checked."
  say "RESULT: the hook is installed; with no stick checked. NEXT: put MPFree's stick in the MPC and run this again (it writes nothing more)."
  exit 1
fi
# Run as an argument, with no input: nothing in it can read the rest of this script from SSH.
if "$R/bin/sh" -c "$(sed "1s|^R=''\$|R='$R'|" "$SCRIPT")" hook.sh check < /dev/null; then
  say "RESULT: the hook is installed, and the stick checks out. NEXT: switch the MPC off, then on with the stick in."
else
  say "RESULT: the hook is installed, but the check stopped (above). Until that is fixed, power-on starts Akai's system as usual."
  exit 1
fi
