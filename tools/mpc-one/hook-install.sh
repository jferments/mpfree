R=''
# MPFree boot hook: INSTALL. STATE-CHANGING on the MPC's data partition only, then a read-only check of the
# stick. Streamed over SSH stdin by `collect-over-ssh.sh --hook-install` (developer mode, port 22, an
# installed Akai 3.10.1 or later). The R='' line above is an empty root prefix; only tests change it.
#
# What the hook does, from the next power-on: with MPFree's stick in (a USB disk partition named
# mpfree-test-root, written by write-test-stick.py from a build of offline-kexec-stick.py), the MPC starts
# MPFree from the stick before Akai's app starts. When the Wi-Fi module cfg80211 loads, about 6 s after
# power-on, a modprobe line runs the hook; it starts a short service that Akai's app and its firmware
# updaters wait for. The service mounts the stick's partition 1 read-only at /run/nextroot, checks that it
# is a system for the running kernel that switches the MPC's eMMC and SD card off before anything can
# touch them, and asks systemd for a soft-reboot: systemd stops Akai's programs, unmounts /data and starts
# the stick's system on the kernel that is already running. No kernel is loaded, so LoadPin (which refuses
# kexec on 3.10.1) has nothing to refuse. A udev rule is the backup trigger.
# A stick built with `offline-kexec-stick.py --kexec-file` asks for the second, opt-in route instead: the
# service loads Akai's own kernel from Akai's root (the only kernel LoadPin lets the file-based kexec load)
# with the stick's command line, and systemd restarts into it. If that load is refused, or this system's
# kernel is not the one the stick was built with, it soft-reboots as above. A stick without that option
# always takes the soft-reboot. Without the stick, or on any
# problem, it does nothing and Akai's system starts as usual. The trigger was tried first with a harmless
# canary (hook-canary-install.sh): the modprobe line ran at 5.96 s, its service at 6.09 s, and Akai's app
# started at 6.36 s.
#
# It writes exactly three files, and the folders they need:
#   /data/mpfree-hook/hook.sh            (and /data/mpfree-hook/created-folders.txt, for the undo)
#   /data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules
#   /data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf
# and deletes the canary's three files, if they are there (only files carrying the canary's marker line).
# An earlier hook (the kexec version) in the same places is replaced. It never writes the root filesystem,
# the boot partitions, the U-Boot environment or anything else in /etc. A rule udev cannot parse is skipped
# with a log message, and a modprobe line it cannot parse is skipped with a warning; neither can stop the
# MPC from booting. If hook.sh were missing, the modprobe line still loads cfg80211 (its parts are
# separated by ";").
#
# Then, with the stick in the MPC now, it runs the hook's own check (`hook.sh check`): the same checks on
# the stick as at power-on, without the soft-reboot. It changes nothing: the stick's partition is mounted
# read-only in /run and unmounted again, or read where Akai's system has already mounted it, and modules the
# stick names are only looked up (modprobe --dry-run). For a --kexec-file stick it also loads Akai's kernel
# into RAM with the file-based kexec and unloads it again (RAM only, never started), to show whether the
# kernel accepts the load with the stick's command line.
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

say "MPFREE BOOT HOOK - INSTALL (changes the data partition only: three small files; then a read-only check of the stick)"
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
# soft-reboot came with systemd 254; 3.10.1 has 255. Its two units are what `systemctl soft-reboot` starts.
for u in soft-reboot.target systemd-soft-reboot.service; do
  [ -f "$R/usr/lib/systemd/system/$u" ] || stop "this system's systemd has no $u (soft-reboot needs systemd 254 or later)"
done
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
# the stick before Akai's app starts: it mounts the stick's system read-only at /run/nextroot and asks
# systemd for a soft-reboot, which stops Akai's programs and starts the stick's system on the kernel that
# is already running. No kernel is loaded. Without that stick it does nothing.
# Opt-in second route: a stick whose /boot/mpfree-softreboot/jump says kexec-file (offline-kexec-stick.py
# --kexec-file) asks for a restart into Akai's OWN kernel, /boot/zImage-<release> on Akai's root, loaded
# with the file-based kexec and the stick's command line (root= the stick). That is the only kexec load
# LoadPin accepts on 3.10.1. If this system's kernel is not the one the stick was built with, or the kernel
# refuses the load, or systemd will not start kexec.target, the hook soft-reboots as for any other stick.
# It is run:
#   modprobe  by the module loader, when the Wi-Fi module cfg80211 loads (about 6 s after power-on);
#   udev      by udev, for a device event (the backup, if cfg80211 does not load);
#   jump      by systemd, as the service the first of those started (Akai's app and its firmware
#             updaters wait for it to finish);
#   check     by hook-install.sh: the same checks on the stick, without the soft-reboot.
# It writes only to /run (RAM) and the kernel log, mounts the stick's partition 1 read-only, and may load
# kernel modules the stick names, from Akai's own system, or (kexec-file sticks) Akai's own kernel into RAM. On any problem it stops, cleans up and leaves
# Akai's system to start as usual. Undo: collect-over-ssh.sh --hook-remove, or take the stick out.
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin   # udev and modprobe give no useful PATH (tests set R and theirs)
export PATH
LABEL=mpfree-test-root
SDIR=boot/mpfree-softreboot
PROTOCOL=mpfree-softreboot-1
# What the stick's system must carry: the service that switches the MPC's eMMC and SD card off before
# anything can touch them (offline-kexec-stick.py writes these and lists them in its SHA256SUMS).
NEED='/etc/mpfree-test/internal-off.sh /etc/systemd/system/mpfree-test-internal-off.service /etc/udev/rules.d/00-mpfree-internal-off.rules'
WANTS=etc/systemd/system/sysinit.target.wants/mpfree-test-internal-off.service
HOLD='acvs.service touch-fw-update.service xmos-update.service az01-usbsata-fixer.service'
UPDATERS='touch-fw-update.service xmos-update.service az01-usbsata-fixer.service'
FLASHERS='dfu-util touch-fw-upd xmosdfu midifirmup az01-update'
LATE=15   # seconds after power-on; later than this it never starts the jump
# The only words a kexec-file stick's command line may hold (root= must name the stick's own partition 1),
# and the ones it must hold: read-only root, a wait for the stick, a restart on panic, no further kexec.
KWORDS='ro rootwait=60 panic=10 rfkill.default_state=0 sysctl.kernel.kexec_load_disabled=1 loadpin.enforce=0'
KNEED='ro rootwait=60 panic=10 sysctl.kernel.kexec_load_disabled=1'
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
M=''; ours=''; W=''; loaded=''; jump=soft-reboot; kcmd=''; ksum=''
STUCK="the kernel this hook loaded for kexec_file could not be unloaded; no restart while it is in RAM"
unload() {   # takes out again a kernel this hook loaded (with the file syscall: LoadPin denies the old one).
             # Returns 1 while it stays loaded: then nothing is started, and it is not tried a second time.
  case "$loaded" in yes) ;; stuck) return 1 ;; *) return 0 ;; esac
  if [ "$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)" = 0 ]; then loaded=''; return 0; fi
  if "$R/usr/sbin/kexec" --unload --kexec-file-syscall > /dev/null 2>&1 &&
      [ "$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)" = 0 ]; then
    loaded=''
    say "Akai's kernel unloaded again (kexec --unload)"
    return 0
  fi
  loaded=stuck
  say "could not unload the kernel this hook loaded (it stays in RAM until the MPC is switched off)"
  return 1
}
cleanup() {
  unload
  if [ "$ours" = yes ]; then
    umount "$M" 2>/dev/null || say "could not unmount the stick's partition 1 from ${M#$R}"
  fi
  ours=''
  [ "$kind" = jump ] && rmdir "$R/run/nextroot" 2>/dev/null
  [ -n "$W" ] && { rmdir "$W/stick" 2>/dev/null; rmdir "$W" 2>/dev/null; }
  return 0
}
fail() { say "STOP: $1"; cleanup; say "nothing was started: Akai's system starts as usual"; exit 1; }
busy() {   # prints what may be writing firmware right now
  for u in $UPDATERS; do
    case "$(systemctl is-active "$u" 2>/dev/null)" in activating|deactivating|reloading) echo "$u"; return 0 ;; esac
  done
  for p in $FLASHERS; do pidof "$p" > /dev/null 2>&1 && { echo "$p"; return 0; }; done
  # az01-usbsata-fixer runs all the time on 3.10.1 (its service stays active; seen still running two hours
  # after power-on). It writes only to a JMicron USB-SATA bridge (USB vendor 152d), so it counts only while
  # one is attached. Assumption from its name, its log ("USB-SATA bridge fixer") and its unit; not read in code.
  if pidof az01-usbsata-fixer > /dev/null 2>&1; then
    for v in "$R"/sys/bus/usb/devices/*/idVendor; do
      grep -qx 152d "$v" 2>/dev/null && { echo "az01-usbsata-fixer (a JMicron USB-SATA bridge is attached)"; return 0; }
    done
  fi
  # [.] so that this line's own text (check mode runs as `sh -c "<this script>"`) does not match itself.
  grep -qs 'SME0/Firmware/update[.]sh' "$R"/proc/[0-9]*/cmdline && { echo "Akai's controller update.sh"; return 0; }
  return 1
}
mounted_at() {   # $1 = mount point (with $R): is something mounted there?
  while read -r d m rest; do [ "$R$m" = "$1" ] && return 0; done < "$R/proc/mounts"
  return 1
}
regular() { [ -f "$1" ] && [ ! -L "$1" ]; }
helper=''
modules() {   # $1 = load or try: the stick's preload list, from Akai's own system (the stick's are refused)
  list="$M/$SDIR/preload-modules.txt"
  [ -e "$list" ] || { say "modules to load first: none listed"; return 0; }
  regular "$list" || fail "/$SDIR/preload-modules.txt is not a plain file"
  [ "$(wc -l < "$list" | tr -cd 0-9)" -le 64 ] || fail "/$SDIR/preload-modules.txt has more than 64 lines"
  names=''
  while read -r m rest; do
    [ -z "$m$rest" ] && continue
    [ -z "$rest" ] || fail "/$SDIR/preload-modules.txt has a line with more than one word"
    case "$m" in *[!A-Za-z0-9_-]*) fail "/$SDIR/preload-modules.txt names an unusual module" ;; esac
    names="$names $m"
  done < "$list"
  [ -n "$names" ] || { say "modules to load first: none listed"; return 0; }
  helper=$(cat "$R/proc/sys/kernel/modprobe" 2>/dev/null)
  case "$helper" in /*) ;; *) fail "the kernel's module loader is not set" ;; esac
  for m in $names; do
    if [ "$1" = try ]; then
      "$R$helper" --dry-run --quiet "$m" > /dev/null 2>&1 || fail "Akai's system has no module $m (the stick needs it)"
    else
      "$R$helper" --quiet "$m" > /dev/null 2>&1 || fail "could not load the module $m from Akai's system"
    fi
  done
  if [ "$1" = try ]; then say "modules to load first, all found in Akai's system:$names"
  else say "modules loaded from Akai's system:$names"; fi
}

prepare() {   # Finds the stick, mounts its partition 1 read-only and checks it is a system to start.
  dev=$(readlink -f "$R/dev/disk/by-partlabel/$LABEL" 2>/dev/null)
  [ -n "$dev" ] && [ -e "$dev" ] || fail "MPFree's stick is not in (no partition named $LABEL)"
  name=${dev##*/}
  case "$name" in sd[a-z]1) ;; *) fail "the partition named $LABEL is $name, not partition 1 of a USB disk" ;; esac
  case "$(readlink -f "$R/sys/class/block/$name")" in */usb*) ;; *) fail "/dev/$name is not on USB" ;; esac
  say "stick: /dev/$name (partition 1 of a USB disk), at $up s"
  where=''
  while read -r d m rest; do
    [ "$d" = "/dev/$name" ] && [ -z "$where" ] && where=$m
  done < "$R/proc/mounts"
  if [ "$kind" = jump ]; then
    # The stick's system is mounted at /run/nextroot for the soft-reboot: read-only, no journal replay (so
    # nothing is written to the stick), with its programs allowed to run.
    [ -z "$where" ] || fail "/dev/$name is mounted already (at $where); not this boot"
    M="$R/run/nextroot"
    if [ -e "$M" ]; then
      mounted_at "$M" && fail "something is mounted at /run/nextroot already"
      [ -d "$M" ] && [ ! -L "$M" ] && [ -z "$(ls -A "$M")" ] || fail "/run/nextroot exists and is not an empty folder"
    fi
    mkdir -p "$M" || fail "could not make /run/nextroot"
    mount -t ext4 -o ro,noload "$dev" "$M" || fail "could not mount /dev/$name read-only"
    ours=yes
  elif [ -n "$where" ]; then
    case "$where" in *[!A-Za-z0-9/._-]*) fail "/dev/$name is mounted at an unusual path" ;; esac
    M="$R$where"
    say "partition 1 is mounted already at $where; read from there"
  else
    W="$R/run/mpfree-hook-check"
    M="$W/stick"
    mkdir -p "$M" || fail "could not make ${M#$R}"
    mount -t ext4 -o ro,noload,nodev,nosuid,noexec "$dev" "$M" || fail "could not mount /dev/$name read-only"
    ours=yes
  fi
  B="$M/$SDIR"
  regular "$B/PROTOCOL" || fail "the stick has no /$SDIR (it was made before offline-kexec-stick.py added it)"
  [ "$(cat "$B/PROTOCOL")" = "$PROTOCOL" ] || fail "/$SDIR/PROTOCOL is not $PROTOCOL (stick and hook from different versions)"
  rel=$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)
  regular "$B/release" && [ "$(cat "$B/release")" = "$rel" ] ||
    fail "the stick's system is not for the running kernel $rel (rebuild it for this firmware)"
  regular "$B/SHA256SUMS" || fail "the stick has no /$SDIR/SHA256SUMS"
  [ "$(wc -l < "$B/SHA256SUMS" | tr -cd 0-9)" -le 32 ] || fail "/$SDIR/SHA256SUMS is unusually long"
  listed=' '
  while read -r sum file rest; do
    [ -z "$rest" ] || fail "/$SDIR/SHA256SUMS has an unusual line"
    case "$sum" in *[!0-9a-f]*|'') fail "/$SDIR/SHA256SUMS has an unusual line" ;; esac
    case "$file" in /*) ;; *) fail "/$SDIR/SHA256SUMS names a file that is not a full path" ;; esac
    case "$file" in *[!A-Za-z0-9/._-]*|*..*) fail "/$SDIR/SHA256SUMS names an unusual path" ;; esac
    regular "$M$file" || fail "the stick has no $file (named in /$SDIR/SHA256SUMS)"
    [ "$(sha256sum < "$M$file" | cut -d' ' -f1)" = "$sum" ] ||
      fail "$file on the stick does not match its SHA256SUMS (damaged? write the stick again)"
    listed="$listed$file "
  done < "$B/SHA256SUMS"
  for f in $NEED; do
    case "$listed" in *" $f "*) ;; *) fail "the stick's system does not carry $f (it must switch the MPC's eMMC and SD card off)" ;; esac
  done
  [ "$(readlink "$M/$WANTS")" = /etc/systemd/system/mpfree-test-internal-off.service ] ||
    fail "the stick's system does not start mpfree-test-internal-off.service first (/$WANTS)"
  say "the stick's system switches the MPC's eMMC and SD card off first: its files check out against SHA256SUMS"
  # Its mode bits, not [ -x ]: the check's mount is noexec, where [ -x ] is false even for root.
  case "$(ls -ln "$M/usr/lib/systemd/systemd" 2>/dev/null)" in -??x*) ;; *) false ;; esac &&
    regular "$M/usr/lib/systemd/systemd" || fail "the stick has no /usr/lib/systemd/systemd to start"
  # systemd switches to /run/nextroot only if it holds an os-release file; otherwise it would restart
  # Akai's own system instead.
  regular "$M/usr/lib/os-release" || regular "$M/etc/os-release" || fail "the stick has no os-release file"
  say "kernel $rel; the stick's system is for it"
  [ -e "$B/jump" ] || return 0
  # The opt-in second route (offline-kexec-stick.py --kexec-file): jump, cmdline and kernel.sha256, all
  # three listed in SHA256SUMS, so checked above.
  for f in jump cmdline kernel.sha256; do
    case "$listed" in *" /$SDIR/$f "*) ;; *) fail "/$SDIR/$f is not listed in /$SDIR/SHA256SUMS" ;; esac
  done
  [ "$(cat "$B/jump")" = kexec-file ] || fail "/$SDIR/jump is not kexec-file (stick and hook from different versions)"
  uuid=''
  for l in "$R"/dev/disk/by-partuuid/*; do
    [ "$(readlink -f "$l")" = "$dev" ] && uuid=${l##*/}
  done
  case "$uuid" in
    *[!0-9a-f-]*|'') fail "/dev/$name has no plain partition UUID link (the kexec-file command line names it)" ;;
  esac
  [ "$(wc -l < "$B/cmdline" | tr -cd 0-9)" = 1 ] || fail "/$SDIR/cmdline is not one line"
  kcmd=$(cat "$B/cmdline")
  [ "${#kcmd}" -le 512 ] || fail "/$SDIR/cmdline is unusually long"
  [ -z "$(printf '%s' "$kcmd" | tr -d 'A-Za-z0-9=._/ -')" ] || fail "/$SDIR/cmdline has unusual characters"
  seen=' '
  for w in $kcmd; do
    case "$seen" in *" $w "*) fail "/$SDIR/cmdline has $w twice" ;; esac
    case "$w" in
      "root=PARTUUID=$uuid") ;;
      root=*) fail "/$SDIR/cmdline does not start the system on this stick's partition 1" ;;
      *) case " $KWORDS " in *" $w "*) ;; *) fail "/$SDIR/cmdline has $w, which the hook does not allow" ;; esac ;;
    esac
    seen="$seen$w "
  done
  for w in "root=PARTUUID=$uuid" $KNEED; do
    case "$seen" in *" $w "*) ;; *) fail "/$SDIR/cmdline lacks ${w%%=$uuid}" ;; esac
  done
  [ "$(wc -l < "$B/kernel.sha256" | tr -cd 0-9)" = 1 ] || fail "/$SDIR/kernel.sha256 is not one line"
  read -r ksum kfile krest < "$B/kernel.sha256"
  [ -z "$krest" ] && [ "$kfile" = "/boot/zImage-$rel" ] || fail "/$SDIR/kernel.sha256 does not name /boot/zImage-$rel"
  case "$ksum" in *[!0-9a-f]*|'') fail "/$SDIR/kernel.sha256 has an unusual hash" ;; esac
  [ "${#ksum}" = 64 ] || fail "/$SDIR/kernel.sha256 has an unusual hash"
  jump=kexec-file
  say "this stick asks to start on Akai's own kernel (kexec_file), command line: $(printf '%s' "$kcmd" | sed 's/PARTUUID=[^ ]*/PARTUUID=(this stick)/')"
}
kexec_file() {   # $1 = what happens instead. Loads Akai's own kernel with the stick's command line (RAM only).
                 # Returns 1, with a note, where it did not: the MPC then soft-reboots instead.
  K="$R/boot/zImage-$rel"
  regular "$K" || { say "kexec_file: this system has no /boot/zImage-$rel; $1"; return 1; }
  [ "$(sha256sum < "$K" | cut -d' ' -f1)" = "$ksum" ] ||
    { say "kexec_file: this system's /boot/zImage-$rel is not the kernel the stick was built with; $1"; return 1; }
  [ -x "$R/usr/sbin/kexec" ] || { say "kexec_file: this system has no /usr/sbin/kexec; $1"; return 1; }
  [ "$(cat "$R/proc/sys/kernel/kexec_load_disabled" 2>/dev/null)" = 0 ] ||
    { say "kexec_file: kexec is turned off on this system; $1"; return 1; }
  [ "$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)" = 0 ] ||
    { say "kexec_file: a kexec kernel is loaded already (not by this hook); $1"; return 1; }
  loaded=yes
  # The file syscall only: LoadPin denies the old buffer one ("kexec-image old-api-denied"), and refuses a
  # device tree in file mode, so the kernel keeps the running one. The command line points root= at the stick.
  if ! out=$("$R/usr/sbin/kexec" --load --kexec-file-syscall --command-line="$kcmd" "$K" 2>&1); then
    say "kexec_file: the kernel refused the load ($(printf '%s' "$out" | tr -c 'A-Za-z0-9 ._:/()=-' ' ' | cut -c1-200)); $1"
    unload || fail "$STUCK"
    return 1
  fi
  [ "$(cat "$R/sys/kernel/kexec_loaded" 2>/dev/null)" = 1 ] ||
    { say "kexec_file: kexec_loaded is not 1 after the load; $1"; unload || fail "$STUCK"; return 1; }
  say "Akai's kernel loaded with the stick's command line (kexec_file) at $(cut -d' ' -f1 "$R/proc/uptime") s"
}

case "$kind" in
  modprobe|udev) trigger "$kind" ;;
  jump)
    [ -d "$R/run/mpfree-hook" ] || exit 0   # only as started by a trigger
    say "jump service started at $up s; Akai's app: $(systemctl is-active acvs.service 2>/dev/null)"
    prepare
    n=0
    while what=$(busy); do
      [ "$n" -ge 20 ] && fail "$what is still running after 20 s; no restart while firmware may be written"
      [ "$n" = 0 ] && say "waiting for $what to finish"
      sleep 1; n=$((n + 1))
    done
    if [ "$jump" = kexec-file ] && kexec_file "soft-reboot instead"; then
      what=$(busy) && fail "$what started; no restart while firmware may be written"
      say "restart into MPFree on Akai's own kernel (kexec.target) at $(cut -d' ' -f1 "$R/proc/uptime") s"
      systemctl --no-block --job-mode=replace-irreversibly start kexec.target && exit 0
      say "kexec_file: systemctl could not start kexec.target; soft-reboot instead"
      unload || fail "$STUCK"
    fi
    modules load
    what=$(busy) && fail "$what started; no restart while firmware may be written"
    echo "$up" > "$R/run/mpfree-hook/soft-reboot" 2>/dev/null   # tells the stick's system how it was started
    say "soft-reboot into MPFree (soft-reboot.target) at $(cut -d' ' -f1 "$R/proc/uptime") s"
    systemctl --no-block --job-mode=replace-irreversibly start soft-reboot.target || {
      rm -f "$R/run/mpfree-hook/soft-reboot"
      fail "systemctl could not start soft-reboot.target"
    } ;;
  check)
    prepare
    modules try
    what=$(busy) && say "note: $what is running now (at power-on the jump waits for it)"
    for w in $(cat "$R/proc/cmdline" 2>/dev/null); do
      case "$w" in init=*|systemd.*) say "note: the kernel command line has $w, which the stick's system also reads" ;; esac
    done
    if [ "$jump" = kexec-file ]; then
      if kexec_file "at power-on this stick would soft-reboot instead"; then
        unload || fail "$STUCK"
        cleanup
        say "CHECK-OK: at power-on with this stick in, the MPC would restart into it on Akai's own kernel (kexec_file; the test load and unload passed), and soft-reboot into it if that load fails then"
        exit 0
      fi
      cleanup
      say "CHECK-OK: at power-on with this stick in, the MPC would soft-reboot into it (the kexec_file route it asks for is not available, above)"
      exit 0
    fi
    cleanup
    say "CHECK-OK: at power-on with this stick in, the MPC would soft-reboot into it" ;;
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
say "=== Check with MPFree's stick (read-only: mounted read-only, looked at, unmounted) ==="
if [ ! -e "$R/dev/disk/by-partlabel/mpfree-test-root" ]; then
  say "MPFree's stick is not in the MPC (no partition named mpfree-test-root), so it was not checked."
  say "RESULT: the hook is installed; with no stick checked. NEXT: put MPFree's stick in the MPC and run this again (it writes nothing more)."
  exit 1
fi
# Run as an argument, with no input: nothing in it can read the rest of this script from SSH.
if "$R/bin/sh" -c "$(sed "1s|^R=''\$|R='$R'|" "$SCRIPT")" hook.sh check < /dev/null; then
  say "RESULT: the hook is installed, and the stick checks out. NEXT: switch the MPC off, then on with the stick in."
else
  say "RESULT: the hook is installed, but the check stopped (above). Take the stick out before you next switch the MPC on (without it the hook does nothing), and send the results."
  exit 1
fi
