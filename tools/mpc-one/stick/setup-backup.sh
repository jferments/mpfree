R=''
# MPFree first-start setup: back up the MPC's internal eMMC to the stick. READ-ONLY on the eMMC.
# (The R='' line is an empty root prefix; only tests change it.)
#
# Run once per boot, as root, by mpfree-setup-backup.service, when the pad player's first-start wizard asks
# for it (it writes /run/mpfree-app/setup-backup.request; mpfree-setup-backup.path starts this). At startup
# the stick's system switched the eMMC and SD card controllers off (internal-off.sh). This opens a
# read-only window on the eMMC only, copies it to the stick's partition 4 and closes the window again:
#   1. checks: partition 4 mounted read-write with room for the copy, no eMMC or SD block device present,
#      the eMMC controller (ff0f0000) unbound, the startup switch-off finished;
#   2. opens the window: a udev rule in /run (RAM) marks each eMMC block device read-only as it appears,
#      then only the eMMC controller is bound again (/sys/bus/platform/drivers_probe; the SD card's stays
#      off). Every eMMC block device is marked read-only again here and checked (ro = 1, and force_ro = 1
#      on boot0 and boot1); the partition table must match the MPC One's (names, starts and sizes);
#      nothing may be mounted from it. The block layer's write and discard counters are recorded;
#   3. copies the user area (mmcblk0) and the two boot areas (mmcblk0boot0, mmcblk0boot1) with dd, each
#      hashed (SHA-256) as it is read, into partition 4's backup/mpc-one-emmc-<UTC>.part/;
#   4. checks the counters did not move, then closes the window: unbinds the controller, waits until no
#      eMMC block device is left and removes the udev rule and stale /dev/disk links. If the controller
#      will not let go, it powers the MPC off (MPFree never runs with the internal storage reachable);
#   5. drops the page cache and reads the copy back from the stick: every image must match its hash.
#      Then MANIFEST.txt is written and the folder renamed to backup/mpc-one-emmc-<UTC>/, owned by root
#      and readable by the pad player's group (it reads the factory kits and samples from it).
# Nothing is ever written to the eMMC: no mount, no fsck, no write. The only writes are partition 4's
# backup folder, /run (RAM: the status, the udev rule, and the request, removed), the kernel log and the
# controller's sysfs bind and unbind files.
# Progress and the result: /run/mpfree-setup/status (key=value lines), read by the pad player.
# Any failure before step 4 closes the window first; a failed backup is deleted and the status says why.
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
EMMC=ff0f0000
DATA=/run/mpfree-data
S="$R/run/mpfree-setup"
RULE="$R/run/udev/rules.d/00-mpfree-setup-ro.rules"
APP_GID=960
# The MPC One's eMMC layout (tools/mpc-one/baselines/mpc-one-3.6.0-reference.txt; 3.10.1 keeps it):
# partition, name, start sector, sectors. The data partition (8) runs to the end of the disk.
LAYOUT='1 uboot-spl 64 128
2 env 192 16
3 uboot 256 768
4 splash 1024 8192
5 recoverysplash 9216 8192
6 rootfs 17408 1024000
7 content 1041408 8388608
8 data 9430016 -'
# Room needed on partition 4, in KiB: the 7.8 GB user area, the two 4 MiB boot areas and 256 MiB to spare.
NEED_KB=7900000
DISKS='mmcblk0 mmcblk0boot0 mmcblk0boot1'
opened=''
name=''
dir=''
devdir=''

mkdir -p "$S"
# The request has done its job: taken away so the path unit does not start this again when it ends.
rm -f "$R/run/mpfree-app/setup-backup.request"
status() {   # state step done total message
  { echo "state=$1"; echo "step=$2"; echo "done=$3"; echo "total=$4"; echo "message=$5"
    if [ -n "$dir" ]; then echo "folder=${dir#$R}"; fi; } > "$S/status.new" && mv "$S/status.new" "$S/status"
}
say() { { echo "mpfree-setup-backup: $*" > "$R/dev/kmsg"; } 2>/dev/null; echo "$*" >> "$S/backup-log.txt"; }
devices() { for b in "$R"/sys/block/mmcblk*; do [ -e "$b" ] && echo "${b##*/}"; done; }
setro() {   # every eMMC block device, read-only
  for b in $(devices); do
    for d in "$R/dev/$b" "$R/dev/$b"p*; do
      [ -b "$d" ] && blockdev --setro "$d" 2>/dev/null
    done
  done
}
close_window() {
  [ -n "$opened" ] || return 0
  [ -e "$devdir/driver" ] && echo "$name" > "$devdir/driver/unbind" 2>/dev/null
  n=0
  while [ -n "$(devices)" ] || [ -e "$devdir/driver" ]; do
    if [ "$n" -ge 10 ]; then
      say "STOP: the eMMC controller did not let go; powering off (MPFree never runs with it reachable)"
      status failed closing 0 0 "The MPC's internal storage could not be switched off again, so the MPC turns off. Take the stick out to start Akai's system."
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
  opened=''
  say "window closed: no eMMC block device is left"
}
fail() {
  say "STOP: $1"
  close_window
  [ -n "$dir" ] && [ -d "$dir.part" ] && rm -rf "$dir.part"
  status failed "${2:-checking}" 0 0 "$1"
  exit 1
}

if [ -e "$S/started" ]; then
  say "already run this boot; not again"
  exit 0
fi
: > "$S/started"
: > "$S/backup-log.txt"
status running checking 0 0 "Checking the stick"
say "started at $(cut -d' ' -f1 "$R/proc/uptime") s"

# 1. Checks, before the window opens.
grep -q " $DATA ext4 rw" "$R/proc/mounts" || fail "the stick's partition 4 is not mounted read-write at $DATA"
[ -z "$(devices)" ] || fail "an eMMC or SD card block device is already there ($(devices | tr '\n' ' '))"
grep -q "no eMMC or SD card block device is left" "$R/run/mpfree-internal-off.txt" 2>/dev/null ||
  fail "the startup switch-off of the internal storage did not finish"
for d in "$R"/sys/bus/platform/devices/"$EMMC".*; do
  [ -e "$d" ] || continue
  [ -z "$devdir" ] || fail "more than one eMMC controller at $EMMC"
  devdir=$d
done
[ -n "$devdir" ] || fail "no eMMC controller at $EMMC"
name=${devdir##*/}
[ ! -e "$devdir/driver" ] || fail "the eMMC controller $name is switched on already"
free=$(df -Pk "$R$DATA" 2>/dev/null | awk 'NR == 2 { print $4 }')
case "$free" in ''|*[!0-9]*) fail "could not read the free space on partition 4" ;; esac
[ "$free" -ge "$NEED_KB" ] ||
  fail "partition 4 has $((free / 1024)) MiB free; the backup needs $((NEED_KB / 1024)) MiB (use a stick of 16 GB or more)"

# 2. The read-only window.
bd=$(command -v blockdev)
case "$bd" in /*) ;; *) fail "this system has no blockdev program" ;; esac
mkdir -p "${RULE%/*}"
echo "SUBSYSTEM==\"block\", KERNEL==\"mmcblk*\", ACTION==\"add\", RUN+=\"$bd --setro /dev/%k\"" > "$RULE" ||
  fail "could not write the read-only udev rule in /run"
udevadm control --reload > /dev/null 2>&1
say "opening the read-only window: binding $name"
status running opening 0 0 "Switching on the MPC's internal storage, read-only"
opened=yes
echo "$name" > "$R/sys/bus/platform/drivers_probe" 2>/dev/null || fail "could not switch the eMMC controller on"
n=0
while [ ! -e "$R/sys/block/mmcblk0" ] || [ ! -e "$R/sys/block/mmcblk0boot0" ] || [ ! -e "$R/sys/block/mmcblk0boot1" ]; do
  setro
  [ "$n" -ge 10 ] && fail "the eMMC did not appear within 10 s"
  sleep 1; n=$((n + 1))
done
setro
for b in $(devices); do
  case "$b" in mmcblk0|mmcblk0boot0|mmcblk0boot1) ;; *) fail "an unexpected block device appeared: $b" ;; esac
done
grep -q '^/dev/mmcblk' "$R/proc/mounts" && fail "something mounted the eMMC"
[ "$(cat "$R/sys/block/mmcblk0/ro")" = 1 ] || fail "mmcblk0 is not read-only"
for p in "$R"/sys/block/mmcblk0/mmcblk0p*; do
  [ -e "$p" ] || continue
  [ "$(cat "$p/ro")" = 1 ] || fail "${p##*/} is not read-only"
done
for b in mmcblk0boot0 mmcblk0boot1; do
  [ "$(cat "$R/sys/block/$b/force_ro")" = 1 ] || fail "$b's force_ro is not 1"
  [ "$(cat "$R/sys/block/$b/ro")" = 1 ] || fail "$b is not read-only"
done
size=$(cat "$R/sys/block/mmcblk0/size")
count=0
for p in "$R"/sys/block/mmcblk0/mmcblk0p*; do [ -e "$p" ] && count=$((count + 1)); done
[ "$count" = 8 ] || fail "the eMMC has $count partitions, not the MPC One's 8"
echo "$LAYOUT" | while read -r n pname start sectors; do
  p="$R/sys/block/mmcblk0/mmcblk0p$n"
  got=$(sed -n 's/^PARTNAME=//p' "$p/uevent")
  [ "$got" = "$pname" ] || { echo "partition $n is named '$got', not $pname"; break; }
  [ "$(cat "$p/start")" = "$start" ] || { echo "partition $n ($pname) starts at $(cat "$p/start"), not $start"; break; }
  if [ "$sectors" = - ]; then
    [ $((start + $(cat "$p/size"))) -le "$size" ] || { echo "partition $n runs past the disk"; break; }
  else
    [ "$(cat "$p/size")" = "$sectors" ] || { echo "partition $n ($pname) has $(cat "$p/size") sectors, not $sectors"; break; }
  fi
done > "$S/layout.txt"
[ -s "$S/layout.txt" ] && fail "this is not the MPC One's eMMC layout: $(cat "$S/layout.txt")"
emmc=$(cat "$R/sys/block/mmcblk0/device/name" 2>/dev/null)
counters() {   # writes and discards of every eMMC block device (block layer stat fields 5-7 and 12-14)
  for b in mmcblk0 mmcblk0boot0 mmcblk0boot1; do
    for s in "$R/sys/block/$b/stat" "$R"/sys/block/"$b"/"$b"p*/stat; do
      [ -e "$s" ] || continue
      # shellcheck disable=SC2046 # word splitting collapses sysfs padding on purpose
      set -- $(cat "$s")
      d=${s%/stat}
      echo "${d##*/} ${5:-} ${6:-} ${7:-} ${12:-} ${13:-} ${14:-}"
    done
  done
}
counters > "$S/counters-before.txt"
total=$(( (size + $(cat "$R/sys/block/mmcblk0boot0/size") + $(cat "$R/sys/block/mmcblk0boot1/size")) * 512 ))
say "window open: eMMC ${emmc:-?}, $size sectors, all read-only; layout checked"

# 3. The copy.
stamp=$(date -u +%Y%m%dT%H%M%SZ)
dir="$R$DATA/backup/mpc-one-emmc-$stamp"
[ ! -e "$dir" ] || fail "$dir exists already"
mkdir -p "$dir.part" || fail "could not make the backup folder" copying
chown 0:"$APP_GID" "$R$DATA/backup" "$dir.part" && chmod 0750 "$R$DATA/backup" "$dir.part" ||
  fail "could not set the backup folder's owner" copying
done_before=0
for b in $DISKS; do
  bytes=$(( $(cat "$R/sys/block/$b/size") * 512 ))
  out="$dir.part/$b.img"
  status running copying "$done_before" "$total" "Backing up the MPC's internal storage"
  ( dd if="$R/dev/$b" bs=1048576 2>/dev/null | tee "$out" | sha256sum > "$dir.part/$b.sha" ) &
  pid=$!
  while kill -0 "$pid" 2>/dev/null; do
    got=$(stat -c %s "$out" 2>/dev/null || echo 0)
    status running copying $((done_before + got)) "$total" "Backing up the MPC's internal storage"
    sleep 1
  done
  wait "$pid" || fail "reading $b failed" copying
  got=$(stat -c %s "$out" 2>/dev/null || echo 0)
  [ "$got" = "$bytes" ] || fail "$b: $got bytes copied, $bytes expected" copying
  sum=$(cut -d' ' -f1 "$dir.part/$b.sha")
  case "$sum" in *[!0-9a-f]*|'') fail "$b: no hash" copying ;; esac
  echo "$sum  $b.img" >> "$dir.part/SHA256SUMS"
  rm -f "$dir.part/$b.sha"
  done_before=$((done_before + bytes))
  say "copied $b: $bytes bytes, sha256 $sum"
done
counters > "$S/counters-after.txt"
[ "$(cat "$S/counters-before.txt")" = "$(cat "$S/counters-after.txt")" ] ||
  fail "the eMMC's write or discard counters moved during the copy (see $S/counters-*.txt)" copying
for b in $DISKS; do
  [ "$(cat "$R/sys/block/$b/ro")" = 1 ] || fail "$b stopped being read-only" copying
done

# 4. Close the window.
status running closing "$total" "$total" "Switching the MPC's internal storage off again"
close_window

# 5. Read the copy back from the stick.
status running verifying 0 "$total" "Checking the backup"
sync
{ echo 3 > "$R/proc/sys/vm/drop_caches"; } 2>/dev/null
( cd "$dir.part" && sha256sum -c SHA256SUMS > "$S/verify.txt" 2>&1 ) ||
  fail "the backup on the stick does not read back as copied: $(grep -v ': OK$' "$S/verify.txt" | head -n 3 | tr '\n' ' ')" verifying
{
  echo "MPFree eMMC backup, made on the MPC by MPFree's first-start setup (setup-backup.sh)"
  echo "made: $stamp (the MPC's clock, UTC)"
  echo "kernel: $(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)"
  echo "boot id: $(cat "$R/proc/sys/kernel/random/boot_id" 2>/dev/null)"
  echo "eMMC: ${emmc:-?}, $size sectors"
  echo "read-only window: every eMMC block device was read-only (ro = 1, force_ro = 1 on boot0/1);"
  echo "its write and discard counters did not move during the copy:"
  sed 's/^/  /' "$S/counters-after.txt"
  echo "images (also in SHA256SUMS), read back from the stick after the copy:"
  sed 's/^/  /' "$dir.part/SHA256SUMS"
  echo "partitions of mmcblk0.img (name, start sector, sectors):"
  echo "$LAYOUT" | sed 's/^/  p/'
  echo "Restore: only with MPFree's recovery tools and the owner's go; never write it back by hand."
} > "$dir.part/MANIFEST.txt"
chown 0:"$APP_GID" "$dir.part"/* && chmod 0640 "$dir.part"/* || fail "could not set the backup files' owner" verifying
mv "$dir.part" "$dir" || fail "could not name the backup folder" verifying
sync
say "backup done: ${dir#$R}"
status done done "$total" "$total" "Backup done"
exit 0
