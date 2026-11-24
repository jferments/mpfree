R=''
# MPFree and the SD card, step S2a of the SD card plan (docs/dev/hardware/sd-card-and-internal-drive.md):
# open the card in the MPC's SD slot for writing INSIDE ITS MPFree FOLDER ONLY, then close it cleanly.
# STATE-CHANGING ON THE CARD: its MPFree folder (made if missing), and the in-use mark exFAT keeps while the
# card is open for writing, cleared again by the clean unmount at the close. Nothing else on the card.
# (The R='' line is an empty root prefix; only tests change it.)
#
# Run as root, once per opening, when asked: /run/mpfree-app/sd-card-write.request holds a word (the disk
# test asks now; the pad player's saves will). Never at the same time as the read-only window
# (setup-sd-card.sh): each refuses while any mmcblk device is there.
#   1. checks, the read-only window's: the startup switch-off finished, no eMMC or SD card block device, both
#      controllers unbound, the backup not running, the internal drive not open, nothing mounted at the
#      card's places; and the system has fsck.exfat;
#   2. opens the card read-only first, exactly as the read-only window does: a udev rule in /run marks each
#      mmcblk disk read-only as it appears, only the SD card controller (ff0c0000) is bound, every mmcblk
#      device must be the card's (under the SD controller, no boot or RPMB area), ro and force_ro 1, one
#      partition or none;
#   3. reads its first sector, once, into RAM: it must be exFAT (FAT's rename can cross-link two files when a
#      card is pulled at the wrong moment, so FAT stays read-only until the pull test shows a safe way), the
#      kernel must have exfat, and the in-use mark (VolumeFlags bit 1) and the disk-error bit (bit 2) must be
#      clear. A marked card is refused: check it on a computer (tools/mpc-one/repair-sd-card.py);
#   4. checks its filesystem read-only: fsck.exfat -n on the disk the kernel still holds read-only (no
#      program can open it for writing), its version noted; anything but a clean answer refuses. The card's
#      write and discard counters and the kernel's refused writes must not have moved;
#   5. makes the card's disk writable: force_ro 0 on that disk alone (ro must follow; if it stays 1, the
#      card's lock switch is on, and it is refused), mounts it rw,nosuid,nodev,noexec,noatime,dirsync,
#      errors=remount-ro (files root's and the pad player's group's) at a private place, makes its MPFree
#      folder if missing, and shows it as two bind mounts: the whole card READ-ONLY at /run/mpfree-sd, and
#      its MPFree folder writable at /run/mpfree-sd/MPFree. The private mount goes, so nothing on the
#      system, root included, can write outside MPFree/;
#   6. waits, once a second, until asked to close (sd-card-write.close), the asker's counter in
#      sd-card-write.alive stands still for QUIET seconds, the card is taken out, or the system stops this.
#      It fails and closes if the eMMC controller is bound, a device that is not the card's appears, or the
#      filesystem stops writing (errors=remount-ro after an error);
#   7. closes: sync, both mounts unmounted (LETGO seconds for files to be let go, then whatever still holds a
#      file or folder inside the card is stopped, as a shutdown would; if even that leaves it mounted, the
#      MPC shuts down cleanly, which unmounts it), the disk read-only again, the first sector read again
#      (the mark must be clear: a clean unmount clears it), fsck.exfat -n again, the sectors written noted,
#      then the read-only window's close (unbind, no device left, the rule, links and mount points gone). A
#      controller that won't let go turns the MPC off, as the read-only window does.
# The only writes outside /run (RAM, the kernel log, the controller's sysfs bind and unbind files and the
# card's force_ro files): the card's MPFree folder, which only the asker writes in, the folder itself if it
# was missing, and the exFAT in-use mark the kernel sets while the card is open and clears at the unmount.
# Every program of Akai's system it runs is in NEEDS (fsck.exfat is checked when asked, so a system without
# it still makes a stick: the card then stays read-only).
# Progress and the result: /run/mpfree-setup/sd-card-write-status (key=value lines), read by the asker:
# state (checking, opening, checking-card, open, closing, closed, none, refused, failed), message, ask (the
# request's word), while open card, fs, sectors, mount, write and fsck, and once closed written (MiB).
# Notes: /run/mpfree-setup/sd-card-write-log.txt and the kernel log.
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
SD=ff0c0000
EMMC=ff0f0000
S="$R/run/mpfree-setup"
ASK="$R/run/mpfree-app"
MNT=/run/mpfree-sd
MPF=/run/mpfree-sd/MPFree
PRIV=/run/mpfree-sd-write
RULE="$R/run/udev/rules.d/00-mpfree-sd-ro.rules"
APP_GID=960
# Seconds: for a card to answer after the bind, for files on the card to be let go at the close, and for the
# asker's counter to stand still before the window closes (a card open for writing is marked in use, so not
# for long).
APPEAR=10
LETGO=10
QUIET=30
ALIVE="$ASK/sd-card-write.alive"
NEEDS='cat cut dd dmesg grep mkdir mount mv od readlink rm rmdir sleep sync systemctl tr udevadm umount'
opened=''
mounted=''
card=''
dev=''
fs=''
sectors=''
name=''
devdir=''
version=''
written=''

mkdir -p "$S"
# The asker's word for this opening (hex digits from the request), given back in every status.
ask=$(cut -c1-32 "$ASK/sd-card-write.request" 2>/dev/null | tr -cd '0-9a-f')
ask=$(printf '%s' "$ask" | cut -c1-32)
rm -f "$ASK/sd-card-write.request" "$ASK/sd-card-write.close" "$ALIVE"
status() {   # state message
  { echo "state=$1"; echo "message=$2"; echo "ask=$ask"
    if [ "$1" = open ]; then
      echo "card=$card"; echo "fs=$fs"; echo "sectors=$sectors"; echo "mount=$MNT"; echo "write=$MPF"
      echo "fsck=$version"
    fi
    if [ -n "$written" ]; then echo "written=$written"; fi
  } > "$S/sd-card-write-status.new" && mv "$S/sd-card-write-status.new" "$S/sd-card-write-status"
}
say() { { echo "mpfree-sd-card-write: $*" > "$R/dev/kmsg"; } 2>/dev/null; echo "$*" >> "$S/sd-card-write-log.txt"; }
devices() { for b in "$R"/sys/block/mmcblk*; do [ -e "$b" ] && echo "${b##*/}"; done; }
setro() {   # every mmcblk disk, read-only (its partitions with it)
  for b in $(devices); do
    [ -e "$R/sys/block/$b/force_ro" ] && echo 1 > "$R/sys/block/$b/force_ro" 2>/dev/null
  done
}
refused() { dmesg 2>/dev/null | grep -c 'read-only block-device mmcblk'; }
counters() {   # writes and discards of the card and its partitions (block layer stat fields 5-7 and 12-14)
  for s in "$R/sys/block/$card/stat" "$R"/sys/block/"$card"/"$card"p*/stat; do
    [ -e "$s" ] || continue
    # shellcheck disable=SC2046 # word splitting collapses sysfs padding on purpose
    set -- $(cat "$s")
    d=${s%/stat}
    echo "${d##*/} ${5:-} ${6:-} ${7:-} ${12:-} ${13:-} ${14:-}"
  done
}
sectors_written() {   # the card's disk's sectors written so far (stat field 7)
  # shellcheck disable=SC2046
  set -- $(cat "$R/sys/block/$card/stat" 2>/dev/null)
  echo "${7:-0}"
}
on() { grep -q "^[^ ]* $1 " "$R/proc/mounts" 2>/dev/null; }   # something is mounted at $1
card_mounted() { [ -n "$dev" ] && grep -q "^/dev/$dev " "$R/proc/mounts" 2>/dev/null; }
holders() {   # the processes with a file, folder or root inside the card's places
  for p in "$R"/proc/[0-9]*; do
    [ -d "$p" ] || continue
    for l in "$p/cwd" "$p/root" "$p"/fd/*; do
      case "$(readlink "$l" 2>/dev/null)" in
        "$MNT"|"$MNT"/*|"$PRIV"|"$PRIV"/*) echo "${p##*/}"; break ;;
      esac
    done
  done
}
unmount_one() {   # $1: wait up to $2 seconds for a plain unmount
  n=0
  while on "$1"; do
    umount "$R$1" 2>/dev/null && break
    [ "$n" -ge "$2" ] && return 1
    sleep 1; n=$((n + 1))
  done
  return 0
}
unmount() {
  [ -n "$mounted" ] || return 0
  sync
  ok=yes
  for m in "$MPF" "$MNT" "$PRIV"; do
    on "$m" || continue
    unmount_one "$m" "$LETGO" && continue
    for p in $(holders); do
      say "stopping process $p ($(cat "$R/proc/$p/comm" 2>/dev/null)): it still held a file on the card"
      kill -TERM "$p" 2>/dev/null
    done
    unmount_one "$m" 5 && continue
    for p in $(holders); do kill -KILL "$p" 2>/dev/null; done
    unmount_one "$m" 5 || ok=''
  done
  if [ -z "$ok" ] || card_mounted; then
    say "STOP: the card could not be unmounted; shutting down cleanly, which unmounts it"
    status failed "The SD card could not be closed, so the MPC shuts down to close it cleanly."
    sync
    systemctl poweroff || poweroff
    exit 2
  fi
  mounted=''
}
close_window() {
  trap '' TERM INT
  unmount
  rm -f "$ALIVE" "$S/sd-card-write-sector"
  [ -n "$opened" ] || return 0
  setro
  [ -e "$devdir/driver" ] && echo "$name" > "$devdir/driver/unbind" 2>/dev/null
  n=0
  while [ -n "$(devices)" ] || [ -e "$devdir/driver" ]; do
    if [ "$n" -ge 10 ]; then
      say "STOP: the SD card controller did not let go; powering off (MPFree never runs with it reachable)"
      status failed "The SD card could not be switched off again, so the MPC turns off. Take the stick out to start Akai's system."
      sync
      systemctl poweroff || poweroff -f
      exit 2
    fi
    sleep 1; n=$((n + 1))
  done
  for l in "$R"/dev/disk/by-*/*; do
    case "$(readlink "$l" 2>/dev/null)" in *mmcblk*) rm -f "$l" ;; esac
  done
  rm -f "$RULE"
  udevadm control --reload > /dev/null 2>&1
  for m in "$MPF" "$MNT" "$PRIV"; do
    [ -d "$R$m" ] && ! on "$m" && rmdir "$R$m" 2>/dev/null
  done
  opened=''
  say "window closed: no SD card block device is left"
}
fail() {
  trap - TERM INT
  say "STOP: $1"
  close_window
  status failed "$1"
  exit 1
}
refuse() {   # the card can't be opened for writing; nothing was written to it
  trap - TERM INT
  say "refused: $1"
  close_window
  status refused "$1"
  exit 3
}
stopped() {
  trap - TERM INT
  say "stopped by the system"
  close_window
  status closed "Closed: the system stopped it."
  exit 0
}
only_the_card() {
  for b in $(devices); do
    l=$(readlink "$R/sys/block/$b" 2>/dev/null)
    # Gone between the listing and the look: the card going out, never a stranger (the loop sees it next).
    [ -n "$l" ] || [ -e "$R/sys/block/$b" ] || [ -L "$R/sys/block/$b" ] || continue
    case "$l" in
      */"$name"/*) ;;
      *) fail "a block device that is not the SD card's appeared: $b" ;;
    esac
    case "$b" in *boot*|*rpmb*) fail "an eMMC area appeared under the SD controller: $b" ;; esac
  done
  for d in "$R"/sys/bus/platform/devices/"$EMMC".*; do
    [ -e "$d/driver" ] && fail "the eMMC controller ${d##*/} is switched on"
  done
  return 0
}
SECTOR="$S/sd-card-write-sector"
byte() {   # the byte at offset $1 of the sector's copy, in octal (Akai's BusyBox has no od -t)
  dd if="$SECTOR" bs=1 skip="$1" count=1 2>/dev/null | od -b | { read -r _ b _; echo "${b:-0}"; }
}
fsck_card() {   # fsck.exfat -n on the card's filesystem; its words go to the notes
  echo "--- fsck.exfat -n /dev/$dev ($1)" >> "$S/sd-card-write-log.txt"
  fsck.exfat -n "$R/dev/$dev" >> "$S/sd-card-write-log.txt" 2>&1 < /dev/null
}

trap stopped TERM INT
: >> "$S/sd-card-write-log.txt"
missing=''
for p in $NEEDS; do command -v "$p" > /dev/null 2>&1 || missing="$missing $p"; done
[ -z "$missing" ] || fail "this system lacks programs the SD card needs:$missing"
status checking "Checking"
say "started at $(cut -d' ' -f1 "$R/proc/uptime") s"

# 1. Checks, before the window opens.
command -v fsck.exfat > /dev/null 2>&1 || refuse "this system has no fsck.exfat, so the card can't be checked before writing"
grep -q "no eMMC or SD card block device is left" "$R/run/mpfree-internal-off.txt" 2>/dev/null ||
  fail "the startup switch-off of the internal storage did not finish"
[ -z "$(devices)" ] || fail "an eMMC or SD card block device is already there ($(devices | tr '\n' ' '))"
for d in "$R"/sys/bus/platform/devices/"$SD".*; do
  [ -e "$d" ] || continue
  [ -z "$devdir" ] || fail "more than one SD card controller at $SD"
  devdir=$d
done
[ -n "$devdir" ] || fail "no SD card controller at $SD"
name=${devdir##*/}
[ ! -e "$devdir/driver" ] || fail "the SD card controller $name is switched on already"
for d in "$R"/sys/bus/platform/devices/"$EMMC".*; do
  [ -e "$d/driver" ] && fail "the eMMC controller ${d##*/} is switched on"
done
grep -q '^state=running$' "$S/status" 2>/dev/null && fail "the first-start backup is running"
grep -q '^state=open$' "$S/status" 2>/dev/null && fail "the MPC's internal drive is open for Akai's kits and samples"
for m in "$MNT" "$MPF" "$PRIV"; do
  on "$m" && fail "something is mounted at $m already"
done

# 2. Open read-only, as the read-only window does.
mkdir -p "${RULE%/*}"
echo 'SUBSYSTEM=="block", KERNEL=="mmcblk*", ENV{DEVTYPE}=="disk", ACTION=="add", ATTR{force_ro}="1"' > "$RULE" ||
  fail "could not write the read-only udev rule in /run"
udevadm control --reload > /dev/null 2>&1
refused_before=$(refused)
say "opening the card read-only first: binding $name"
status opening "Switching on the SD card, read-only first"
opened=yes
echo "$name" > "$R/sys/bus/platform/drivers_probe" 2>/dev/null || fail "could not switch the SD card controller on"
n=0
while :; do
  setro
  only_the_card
  card=$(devices)
  [ -n "$card" ] && break
  [ "$n" -ge "$APPEAR" ] && break
  sleep 1; n=$((n + 1))
done
[ -e "$devdir/driver" ] || fail "the SD card controller did not switch on"
if [ -z "$card" ]; then
  say "no card answered within $APPEAR s"
  close_window
  status none "There is no card in the SD slot."
  exit 0
fi
only_the_card
case "$card" in *[!a-z0-9]*) fail "more than one disk appeared under the SD controller: $(echo $card)" ;; esac
sleep 1
setro
only_the_card
[ -e "$R/sys/block/$card" ] || fail "the card went away as it was opened"
[ "$(cat "$R/sys/block/$card/ro")" = 1 ] || fail "$card is not read-only"
[ "$(cat "$R/sys/block/$card/force_ro" 2>/dev/null)" = 1 ] || fail "$card's force_ro is not 1"
parts=''
for p in "$R/sys/block/$card/$card"p*; do
  [ -e "$p" ] || continue
  [ "$(cat "$p/ro")" = 1 ] || fail "${p##*/} is not read-only"
  parts="$parts ${p##*/}"
done
grep -q "^/dev/$card" "$R/proc/mounts" && fail "something mounted the card"
sectors=$(cat "$R/sys/block/$card/size")
set -- $parts
case $# in
  0) dev=$card ;;
  1) dev=$1 ;;
  *) refuse "the card has $# partitions; MPFree saves to a card with one, as a camera, a computer or Akai's system formats it" ;;
esac
[ -e "$R/dev/$dev" ] && [ ! -L "$R/dev/$dev" ] || fail "the card's device ($dev) is missing"

# 3. The first sector: exFAT, with its mark clear.
dd if="$R/dev/$dev" bs=512 count=1 2>/dev/null > "$SECTOR" || fail "the card's first sector could not be read"
case "$(tr -c 'A-Z0-9' '.' < "$SECTOR" | cut -c4-11)" in
  EXFAT...) fs=exfat ;;
  *) refuse "the card is not exFAT: MPFree saves only to exFAT for now (FAT and others stay read-only)" ;;
esac
grep -q "[[:space:]]exfat\$" "$R/proc/filesystems" 2>/dev/null ||
  refuse "this kernel has no exfat filesystem loaded (the stick must load it before the switch)"
flags=$(byte 106)
case "$flags" in
  *[2367]) refuse "the card wasn't safely removed last time (its exFAT is marked in use): check it on a computer first" ;;
esac
case "$flags" in
  *[4567]) refuse "the card's exFAT says a disk error was seen: check it on a computer first" ;;
esac
counters > "$S/sd-card-write-counters-before.txt"

# 4. Its filesystem, checked read-only.
status checking-card "Checking the card (read-only)"
version=$(fsck.exfat -V 2>&1 < /dev/null | grep -i 'version\|exfatfsck' | tr -cd 'A-Za-z0-9 .:_-' | cut -c1-80)
say "fsck.exfat: ${version:-no version given}"
if fsck_card "before writing"; then
  say "the card checks clean (fsck.exfat -n)"
else
  refuse "the card's check found problems (fsck.exfat -n, see the notes): check it on a computer first"
fi
setro
[ "$(cat "$R/sys/block/$card/ro")" = 1 ] || fail "$card stopped being read-only during the check"
counters > "$S/sd-card-write-counters-now.txt"
[ "$(cat "$S/sd-card-write-counters-before.txt")" = "$(cat "$S/sd-card-write-counters-now.txt")" ] ||
  fail "the card's write or discard counters moved during the read-only check"
[ "$(refused)" -le "$refused_before" ] || fail "something wrote to the read-only card during the check (the kernel's log says so)"
[ -e "$R/sys/block/$card" ] || refuse "the card was taken out during the check"

# 5. Writable: that disk alone, mounted privately, then shown as the whole card read-only and MPFree/
# writable.
say "making $card writable"
echo 0 > "$R/sys/block/$card/force_ro" 2>/dev/null || fail "could not make $card writable"
sleep 1
if [ "$(cat "$R/sys/block/$card/ro")" != 0 ]; then
  refuse "the card stays read-only: its lock switch is on (slide it up, away from LOCK)"
fi
for p in $parts; do
  [ "$(cat "$R/sys/block/$card/$p/ro")" = 0 ] || fail "$p stayed read-only after its disk was made writable"
done
only_the_card
written_before=$(sectors_written)
mkdir -p -m 700 "$R$PRIV" && mkdir -p "$R$MNT" || fail "could not make the card's places in /run"
mounted=yes
mount -t exfat -o "rw,nosuid,nodev,noexec,noatime,dirsync,uid=0,gid=$APP_GID,fmask=0117,dmask=0007,iocharset=utf8,errors=remount-ro" \
  "$R/dev/$dev" "$R$PRIV" 2>> "$S/sd-card-write-log.txt" || fail "the card could not be mounted for writing"
grep -q "^/dev/$dev $PRIV exfat rw," "$R/proc/mounts" || fail "the card is not mounted writable at $PRIV"
if [ ! -d "$R$PRIV/MPFree" ]; then
  [ -e "$R$PRIV/MPFree" ] && fail "the card has a file called MPFree where MPFree's folder goes"
  mkdir "$R$PRIV/MPFree" || fail "could not make the MPFree folder on the card"
  say "made the MPFree folder on the card"
fi
mount --bind "$R$PRIV" "$R$MNT" 2>> "$S/sd-card-write-log.txt" &&
  mount -o remount,bind,ro,nosuid,nodev,noexec,noatime "$R$MNT" 2>> "$S/sd-card-write-log.txt" ||
  fail "could not show the card read-only at $MNT"
mount --bind "$R$PRIV/MPFree" "$R$MPF" 2>> "$S/sd-card-write-log.txt" &&
  mount -o remount,bind,rw,nosuid,nodev,noexec,noatime "$R$MPF" 2>> "$S/sd-card-write-log.txt" ||
  fail "could not show the card's MPFree folder writable at $MPF"
umount "$R$PRIV" 2>> "$S/sd-card-write-log.txt" || fail "could not take away the card's private mount"
rmdir "$R$PRIV" 2>/dev/null
grep -q "^/dev/$dev $MNT exfat ro," "$R/proc/mounts" || fail "the card is not read-only at $MNT"
grep -q "^/dev/$dev $MPF exfat rw," "$R/proc/mounts" || fail "the card's MPFree folder is not writable at $MPF"
[ "$(grep -c "^/dev/$dev " "$R/proc/mounts")" = 2 ] || fail "the card is mounted somewhere else too"
say "window open for writing: $card, $sectors sectors, exFAT on $dev, read-only at $MNT, writable at $MPF"
status open "The SD card is open: MPFree writes inside its MPFree folder only."

# 6. Open until asked to close, the asker stops, the card goes, or the system stops this.
why=''
beat=''
quiet=0
while [ -z "$why" ]; do
  if [ -e "$ASK/sd-card-write.close" ]; then
    rm -f "$ASK/sd-card-write.close"
    why=asked
  elif [ ! -e "$R/sys/block/$card" ]; then
    why=out
  else
    only_the_card
    if ! grep -q "^/dev/$dev $MPF exfat rw," "$R/proc/mounts"; then
      fail "the card's filesystem stopped writing (an error, see the kernel's log): check it on a computer"
    fi
    now=$(cat "$ALIVE" 2>/dev/null)
    if [ "$now" != "$beat" ]; then
      beat=$now
      quiet=0
    else
      quiet=$((quiet + 1))
    fi
    if [ "$quiet" -ge "$QUIET" ]; then
      why=quiet
    else
      sleep 1
    fi
  fi
done

# 7. Closed cleanly, then checked again.
trap '' TERM INT
status closing "Closing the SD card"
if [ "$why" = out ]; then
  say "the card was taken out while it was open for writing"
  close_window
  status closed "The card was taken out while open for writing: check it on a computer before the next save."
  exit 0
fi
unmount
written=$(( ($(sectors_written) - written_before) / 2048 ))
setro
[ "$(cat "$R/sys/block/$card/ro")" = 1 ] || fail "$card could not be made read-only again"
problem=''
dd if="$R/dev/$dev" bs=512 count=1 2>/dev/null > "$SECTOR" || problem="its first sector could not be read again"
if [ -z "$problem" ]; then
  case "$(byte 106)" in
    *[2367]) problem="it is still marked in use after the unmount" ;;
    *[4567]) problem="its exFAT says a disk error was seen" ;;
  esac
fi
if [ -z "$problem" ]; then
  fsck_card "after writing" || problem="its check after writing found problems (fsck.exfat -n, see the notes)"
fi
close_window
rm -f "$S/sd-card-write-counters-now.txt"
if [ -n "$problem" ]; then
  say "closed, but $problem"
  status failed "Closed after writing $written MiB, but $problem: check it on a computer."
  exit 1
fi
say "closed cleanly after writing $written MiB: the mark is clear and the card checks clean"
case $why in
  quiet) status closed "Closed: the writer stopped. $written MiB written; the card checks clean." ;;
  *) status closed "Closed cleanly: $written MiB written; the card checks clean." ;;
esac
exit 0
