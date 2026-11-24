R=''
# MPFree and the SD card, step S1 of the SD card plan (docs/dev/hardware/sd-card-and-internal-drive.md):
# open the card in the MPC's SD slot READ-ONLY for the pad player, then close it again. READ-ONLY on the card.
# (The R='' line is an empty root prefix; only tests change it.)
#
# Run as root, once per opening, when the pad player asks for the card (it writes
# /run/mpfree-app/sd-card.request). At startup the stick's system switched the eMMC and SD card controllers
# off (internal-off.sh). This opens a read-only window on the SD card only, and keeps it open until the pad
# player asks to close it (/run/mpfree-app/sd-card.close) or stops answering (sd-card.alive):
#   1. checks: the startup switch-off finished, no eMMC or SD card block device present, both controllers
#      unbound (so the backup's window is not open; the eMMC's stays unbound throughout), the backup not
#      running, nothing mounted at the card's place;
#   2. opens the window: a udev rule in /run (RAM) marks each mmcblk disk read-only as it appears, then only
#      the SD card controller (ff0c0000) is bound again (/sys/bus/platform/drivers_probe). Every mmcblk
#      device must sit under the SD controller: a device under any other controller, or a boot or RPMB
#      area (which only an eMMC has), closes the window at once. The card is marked read-only again here and
#      checked (ro = 1 on the disk and each partition, force_ro = 1). No card within APPEAR seconds: the
#      window closes and the status says there is none;
#   3. finds the filesystem: the card holds one partition, or none (a card formatted without a table), and
#      its first sector says FAT (12, 16 or 32) or exFAT, which the kernel must have (/proc/filesystems:
#      no module can load after the soft-reboot, so the stick loads them before it). One sector is read,
#      from a disk the kernel already holds read-only, into RAM; it also says whether the card was safely
#      removed last time (the in-use mark FAT and exFAT keep), which the status gives as dirty;
#   4. mounts it read-only (ro,nosuid,nodev,noexec,noatime; files root's and readable by the pad player's
#      group) at /run/mpfree-sd, and records the card's write and discard counters (block layer stat fields
#      5-7 and 12-14) and the kernel's count of refused writes to read-only disks;
#   5. waits, once a second, until the pad player asks to close it, the card is taken out or the system
#      stops this. While it reads, the pad player moves a counter in /run/mpfree-app/sd-card.alive; once the
#      counter stands still for QUIET seconds (the pad player stopped), the window closes as if asked. If
#      the card's write or discard counters move, or the card stops being read-only, it fails and closes;
#   6. closes: checks the write and discard counters and the refused writes again, unmounts (after LETGO
#      seconds of the pad player still holding a file, lazily: the card is read-only, so nothing can be
#      lost), unbinds the controller, waits until no mmcblk device is left, removes the pad player's
#      counter, the udev rule, stale /dev/disk links and the empty mount point. If the controller will not
#      let go, the MPC powers off, as the backup's window does (MPFree never runs with the internal storage
#      reachable).
# Nothing is ever written to the card: no fsck, no write, a read-only mount on a disk the kernel holds
# read-only (no program can open it for writing, and no filesystem mounts it read-write). Underneath that,
# the block layer of this kernel (6.x) only warns about a write to a read-only disk, once per disk ("Trying
# to write to read-only block-device mmcblk1p1"), and lets it through; so both the counters and that
# warning are checked, and either fails the window.
# The only writes are /run (RAM: the status, the notes, the udev rule, the mount point, the sector's copy
# and the requests, removed), the kernel log, the controller's sysfs bind and unbind files and the card's
# force_ro files (which only switch read-only on).
# Every program of Akai's system it runs is in NEEDS: it stops before anything else when one is missing.
# Progress and the result: /run/mpfree-setup/sd-card-status (key=value lines), read by the pad player:
# state (checking, opening, open, closing, closed, none, failed), message, ask (the request's word), and while
# open card, fs, sectors, mount and dirty. Notes: /run/mpfree-setup/sd-card-log.txt and the kernel log.
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
SD=ff0c0000
EMMC=ff0f0000
S="$R/run/mpfree-setup"
ASK="$R/run/mpfree-app"
MNT=/run/mpfree-sd
RULE="$R/run/udev/rules.d/00-mpfree-sd-ro.rules"
APP_GID=960
# Seconds: for a card to answer after the bind, for the pad player to let go of the card's files, and for
# its counter to stand still before the window closes (setup-backup.sh's QUIET).
APPEAR=10
LETGO=10
QUIET=120
ALIVE="$ASK/sd-card.alive"
NEEDS='cat cut dd dmesg grep mkdir mount mv od readlink rm rmdir sleep sync systemctl tr udevadm umount'
opened=''
mounted=''
dirty=''
card=''
fs=''
sectors=''
name=''
devdir=''

mkdir -p "$S"
# The pad player's word for this opening (hex digits from the request), given back in every status, so it
# knows the answer is to its own request and not an earlier opening's.
ask=$(cut -c1-32 "$ASK/sd-card.request" 2>/dev/null | tr -cd '0-9a-f')
ask=$(printf '%s' "$ask" | cut -c1-32)
# The request has done its job: taken away so the path unit does not start this again when it ends. A close
# or a counter left from an earlier opening is not for this one.
rm -f "$ASK/sd-card.request" "$ASK/sd-card.close" "$ALIVE"
status() {   # state message
  { echo "state=$1"; echo "message=$2"; echo "ask=$ask"
    if [ "$1" = open ]; then
      echo "card=$card"; echo "fs=$fs"; echo "sectors=$sectors"; echo "mount=$MNT"; echo "dirty=$dirty"
    fi
  } > "$S/sd-card-status.new" && mv "$S/sd-card-status.new" "$S/sd-card-status"
}
say() { { echo "mpfree-sd-card: $*" > "$R/dev/kmsg"; } 2>/dev/null; echo "$*" >> "$S/sd-card-log.txt"; }
devices() { for b in "$R"/sys/block/mmcblk*; do [ -e "$b" ] && echo "${b##*/}"; done; }
setro() {   # every mmcblk disk, read-only (its partitions with it)
  for b in $(devices); do
    [ -e "$R/sys/block/$b/force_ro" ] && echo 1 > "$R/sys/block/$b/force_ro" 2>/dev/null
  done
}
# The kernel's warnings about a write to a read-only mmcblk disk, this boot (each names the disk).
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
is_mounted() { grep -q "^[^ ]* $MNT " "$R/proc/mounts" 2>/dev/null; }
unmount() {
  [ -n "$mounted" ] || return 0
  n=0
  while is_mounted; do
    if [ "$n" -ge "$LETGO" ]; then
      umount -l "$R$MNT" 2>/dev/null && say "unmounted lazily: MPFree still held a file on the card (read-only)"
      break
    fi
    umount "$R$MNT" 2>/dev/null && break
    sleep 1; n=$((n + 1))
  done
  is_mounted && say "the card is still mounted at $MNT"
  mounted=''
}
close_window() {
  trap '' TERM INT   # a stop from the system waits for the close: it takes LETGO + 10 s at the most
  unmount
  rm -f "$ALIVE" "$S/sd-card-sector"
  [ -n "$opened" ] || return 0
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
  [ -d "$R$MNT" ] && ! is_mounted && rmdir "$R$MNT" 2>/dev/null
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
stopped() {
  trap - TERM INT
  say "stopped by the system"
  close_window
  status closed "Closed: the system stopped it. Nothing was written to the card."
  exit 0
}
# Every mmcblk device is the card's: under the SD controller, and not a boot or RPMB area (only an eMMC has
# those). The eMMC controller has no driver.
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

trap stopped TERM INT
: >> "$S/sd-card-log.txt"
missing=''
for p in $NEEDS; do command -v "$p" > /dev/null 2>&1 || missing="$missing $p"; done
[ -z "$missing" ] || fail "this system lacks programs the SD card needs:$missing"
status checking "Checking"
say "started at $(cut -d' ' -f1 "$R/proc/uptime") s"

# 1. Checks, before the window opens.
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
! is_mounted || fail "something is mounted at $MNT already"

# 2. The read-only window.
mkdir -p "${RULE%/*}"
echo 'SUBSYSTEM=="block", KERNEL=="mmcblk*", ENV{DEVTYPE}=="disk", ACTION=="add", ATTR{force_ro}="1"' > "$RULE" ||
  fail "could not write the read-only udev rule in /run"
udevadm control --reload > /dev/null 2>&1
refused_before=$(refused)
say "opening the read-only window: binding $name"
status opening "Switching on the SD card, read-only"
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
# Its partitions come with it; a second look once they have.
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
  *) fail "the card has $# partitions; MPFree reads a card with one, as a camera, a computer or Akai's system formats it" ;;
esac
[ -e "$R/dev/$dev" ] && [ ! -L "$R/dev/$dev" ] || fail "the card's device ($dev) is missing"

# 3. The filesystem, from its first sector, read once into RAM: the names FAT and exFAT write there (every
# other byte a dot here), and the flag each keeps while a system has it mounted for writing.
SECTOR="$S/sd-card-sector"
dd if="$R/dev/$dev" bs=512 count=1 2>/dev/null > "$SECTOR" || fail "the card's first sector could not be read"
first=$(tr -c 'A-Z0-9' '.' < "$SECTOR")
byte() {   # the byte at offset $1 of the sector, in octal (Akai's BusyBox has no od -t)
  dd if="$SECTOR" bs=1 skip="$1" count=1 2>/dev/null | od -b | { read -r _ b _; echo "${b:-0}"; }
}
kind=''
case "$(printf '%s' "$first" | cut -c4-11)" in
  EXFAT...) fs=exfat; kind=exFAT ;;
  NTFS....) kind=NTFS ;;
esac
if [ -z "$kind" ]; then
  case "$(printf '%s' "$first" | cut -c83-87)" in FAT32) fs=vfat; kind=FAT32 ;; esac
fi
if [ -z "$kind" ]; then
  case "$(printf '%s' "$first" | cut -c55-59)" in FAT12|FAT16) fs=vfat; kind=$(printf '%s' "$first" | cut -c55-59) ;; esac
fi
[ -n "$kind" ] || fail "the card's filesystem is not FAT or exFAT; MPFree reads those"
[ -n "$fs" ] || fail "the card is $kind; MPFree reads FAT and exFAT"
grep -q "[[:space:]]$fs\$" "$R/proc/filesystems" 2>/dev/null ||
  fail "the card is $kind, and this kernel has no $fs filesystem loaded (the stick must load it before the switch)"
# Left marked as in use: the last system that wrote to it did not unmount it (a cut, or a card pulled out).
# Linux sets the mark when it mounts FAT for writing and exFAT at its first write, and clears it at the
# unmount; a read-only mount leaves it as it is (fs/fat/inode.c fat_set_state, fs/exfat/super.c
# exfat_set_vol_flags). FAT32 keeps it in bit 0 of byte 65, FAT12/16 of byte 37, exFAT in bit 1 of its
# VolumeFlags (byte 106); exFAT's bit 2 says a disk error was seen.
case $kind in
  FAT32) flags=$(byte 65); mark='[1357]' ;;
  FAT12|FAT16) flags=$(byte 37); mark='[1357]' ;;
  exFAT) flags=$(byte 106); mark='[2367]' ;;
esac
case "$flags" in *$mark) dirty=yes ;; *) dirty=no ;; esac
[ "$kind" = exFAT ] && case "$flags" in *[4567]) dirty=yes; say "the card's exFAT says a disk error was seen" ;; esac
[ "$dirty" = yes ] && say "the card was not unmounted cleanly last time (its $kind says it is still in use)"

# 4. Mounted read-only.
mkdir -p "$R$MNT" || fail "could not make $MNT"
mounted=yes
# Names as UTF-8, as the pad player reads them: Akai's kernel turns FAT's names into ISO 8859-1 unless told
# (CONFIG_FAT_DEFAULT_IOCHARSET), which no name outside ASCII survives; exFAT's default is UTF-8 already, said
# here as Akai's own mount says it.
case $fs in vfat) names=utf8 ;; *) names=iocharset=utf8 ;; esac
mount -t "$fs" -o "ro,nosuid,nodev,noexec,noatime,uid=0,gid=$APP_GID,fmask=0137,dmask=0027,$names" "$R/dev/$dev" "$R$MNT" \
  2>> "$S/sd-card-log.txt" || fail "the card ($kind) could not be mounted read-only"
grep -q "^/dev/$dev $MNT $fs ro," "$R/proc/mounts" || fail "the card is not mounted read-only at $MNT"
counters > "$S/sd-card-counters-before.txt"
say "window open: $card, $sectors sectors, $kind on $dev, read-only at $MNT"
if [ "$dirty" = yes ]; then
  status open "The SD card is open, read-only. It wasn't safely removed last time: check it on a computer."
else
  status open "The SD card is open, read-only."
fi

# 5. Open until the pad player asks to close it or stops answering, the card goes or the system stops this.
why=''
beat=''
quiet=0
while [ -z "$why" ]; do
  if [ -e "$ASK/sd-card.close" ]; then
    rm -f "$ASK/sd-card.close"
    why=asked
  elif [ ! -e "$R/sys/block/$card" ]; then
    why=out
  else
    counters > "$S/sd-card-counters-now.txt"
    ro=$(cat "$R/sys/block/$card/ro" 2>/dev/null)
    if [ ! -e "$R/sys/block/$card" ]; then
      why=out   # taken out as it was read
    elif [ "$(cat "$S/sd-card-counters-before.txt")" != "$(cat "$S/sd-card-counters-now.txt")" ]; then
      fail "the card's write or discard counters moved while it was open (see $S/sd-card-counters-*.txt)"
    elif [ "$ro" != 1 ]; then
      fail "$card stopped being read-only"
    else
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
  fi
done

# 6. Closed again.
trap '' TERM INT
status closing "Closing the SD card"
[ "$(refused)" -le "$refused_before" ] ||
  fail "something wrote to the read-only card while it was open (the kernel's log says so)"
if [ "$why" = out ]; then
  say "the card was taken out while it was open"
  close_window
  status closed "The card was taken out while it was open. Nothing was written to it."
  exit 0
fi
counters > "$S/sd-card-counters-now.txt"
[ "$(cat "$S/sd-card-counters-before.txt")" = "$(cat "$S/sd-card-counters-now.txt")" ] ||
  fail "the card's write or discard counters moved while it was open (see $S/sd-card-counters-*.txt)"
unmount
counters > "$S/sd-card-counters-now.txt"
[ "$(cat "$S/sd-card-counters-before.txt")" = "$(cat "$S/sd-card-counters-now.txt")" ] ||
  fail "the card's write or discard counters moved as it was unmounted (see $S/sd-card-counters-*.txt)"
close_window
rm -f "$S/sd-card-counters-now.txt"
if [ "$why" = quiet ]; then
  say "closed: MPFree stopped reading the card (its counter stood still for $QUIET s)"
  status closed "Closed: MPFree stopped reading the card. Nothing was written to it."
else
  status closed "Closed. Nothing was written to the card."
fi
exit 0
