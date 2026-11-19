#!/bin/bash
# WORKSTATION ONLY. READ-ONLY ON THE STICK: is the MPFree stick that is in this computer the one the next device
# round keeps, and is the backup of the MPC on it good?
#
#   bash tools/mpc-one/check-stick-backup.sh        (from the clone, after Q0 has built mpfree-stick)
#
# Put only the stick to check in this computer, and keep the MPC's SD card and any other MPFree stick out.
# It asks for your password once (sudo), for the two commands that read the stick.
#
#   [1/4] mpfree-stick inspect: the stick's partitions, what made it, and the backups on its partition 4.
#   [2/4] mpfree-stick copy-backup: copies the newest backup off the stick into a new folder,
#         ~/mpfree-work/stick-check-<UTC>/, and checks every image against the backup's SHA256SUMS.
#         The stick is opened read-only and nothing on it is mounted. It needs about 7.3 GiB free.
#   [3/4] Whether ~/mpfree-backups already holds the same backup (the same SHA256SUMS, byte for byte).
#   [4/4] compare-emmc-backups.py: the copy against the backup made over SSH in the 16 Nov round's Q4
#         (or, without it, the newest other backup in ~/mpfree-backups), region by region. The partition
#         table, boot areas, U-Boot, splashes and rootfs must be the same: a backup of this MPC on 3.10.1.
#
# It ends with one RESULT line. Exit code 0: this is the round's stick, and its backup is good and this
# MPC's. 1: it is not (the lines above say why). 2: it could not check (no stick, more than one, Q0 not run).
#
# What it writes: only the new stick-check folder. When ~/mpfree-backups has the same backup already, the
# copy is removed at the end; otherwise it is kept, as the only copy of that backup on this computer.
# Undo: rm -rf ~/mpfree-work/stick-check-<UTC> (the folder named at the end), if it was kept.
set -u
CLONE=/home/jesse/code/mpfree-hardware-survey
BACKUPS=/home/jesse/mpfree-backups
WORK=/home/jesse/mpfree-work
STICK=rust/target/release/mpfree-stick
# The backups this round's notes name: the 16 Nov round's stick and the 17 Nov Windows creator's stick.
EXPECTED="mpc-one-emmc-20261116T014409Z mpc-one-emmc-20261117T173519Z"
# The backup made over SSH in the 16 Nov round's Q4, on 3.10.1.
Q4_PREFIX=mpc-one-emmc-20261116T122336Z

cd "$CLONE" || { echo "STOP: no clone at $CLONE"; exit 2; }
if [ ! -x "$STICK" ]; then
  echo "STOP: $CLONE/$STICK is not built yet: run block Q0 first."
  exit 2
fi
log=$(mktemp)
trap 'rm -f "$log"' EXIT

echo "[1/4] The stick (read-only)"
sudo "$STICK" inspect --device mpfree > "$log" 2>&1
rc=$?
sed 's/^/  /' "$log"
if [ $rc -ne 0 ]; then
  echo "RESULT: could not read an MPFree stick (above). Put only the stick to check in, and run this again."
  exit 2
fi
problems=()
grep -Eq '^partition 4: mpfree-test-data +[0-9]+ MiB at 1024 MiB$' "$log" ||
  problems+=("partition 4 is missing or does not start at 1024 MiB")
grep -qx 'partition 4: MPFREE-DATA ext4' "$log" ||
  problems+=("partition 4 is not MPFREE-DATA ext4")
backups=$(sed -n 's/^backup: //p' "$log")
if [ -z "$backups" ]; then
  problems+=("there is no backup on partition 4")
fi
if [ ${#problems[@]} -gt 0 ]; then
  for p in "${problems[@]}"; do echo "  PROBLEM: $p"; done
  echo "RESULT: NOT the round's stick: ${problems[0]}. Nothing was copied."
  exit 1
fi
newest=$(printf '%s\n' $backups | grep -v '\.part$' | sort | tail -n 1)
echo "  newest backup on the stick: $newest"
named=no
for e in $EXPECTED; do [ "$newest" = "$e" ] && named=yes; done

echo
echo "[2/4] The newest backup, copied off and checked against its SHA256SUMS (read-only on the stick)"
free=$(df --output=avail -B1 "$WORK" 2>/dev/null | tail -n 1 | tr -d ' ')
if [ -n "$free" ] && [ "$free" -lt $((9 << 30)) ]; then
  echo "  STOP: $WORK has $((free >> 30)) GiB free; the copy needs about 7.3 GiB. Nothing was copied."
  echo "RESULT: could not check the backup: not enough free space in $WORK."
  exit 2
fi
check="$WORK/stick-check-$(date -u +%Y%m%dT%H%M%SZ)"
echo "  copying into $check (about 7.3 GiB from the stick: 10 to 15 minutes)"
sudo "$STICK" copy-backup --device mpfree --to "$check" 2>&1 | tee "$log" | grep --line-buffered -v '^progress ' | sed 's/^/  /'
rc=${PIPESTATUS[0]}
copy="$check/$newest"
if [ $rc -ne 0 ] || [ ! -f "$copy/SHA256SUMS" ]; then
  echo "RESULT: NOT a good backup: copy-backup stopped (above). What it copied is in $check; delete it with: rm -rf $check"
  exit 1
fi

echo
echo "[3/4] The same backup on this computer already?"
same=""
if [ -f "$BACKUPS/$newest/SHA256SUMS" ]; then
  if cmp -s "$BACKUPS/$newest/SHA256SUMS" "$copy/SHA256SUMS"; then
    same="$BACKUPS/$newest"
    echo "  yes: $same has the same SHA256SUMS, byte for byte"
  else
    echo "  PROBLEM: $BACKUPS/$newest has the same name but different SHA256SUMS"
    echo "RESULT: NOT the round's stick: its backup has the name of the one on this computer but not its contents. The copy is in $check."
    exit 1
  fi
else
  echo "  no: $BACKUPS has no $newest (the copy just made is then the only one on this computer)"
fi

echo
echo "[4/4] Is it a backup of this MPC? Compared with an earlier backup, region by region (read-only)"
ref=""
for d in "$BACKUPS"/"$Q4_PREFIX"*; do
  [ -f "$d/SHA256SUMS" ] && ref="$d"
done
if [ -z "$ref" ]; then
  for d in "$BACKUPS"/mpc-one-emmc-*; do
    case "$d" in *.part|*.partial) continue ;; esac
    [ "$(basename "$d")" = "$newest" ] && continue
    [ -f "$d/SHA256SUMS" ] && [ -f "$d/mmcblk0.img" ] && ref="$d"
  done
fi
if [ -z "$ref" ]; then
  echo "  STOP: $BACKUPS has no other backup to compare with."
  echo "RESULT: the stick's backup is intact (SHA256SUMS), but there was nothing to compare it with, so it is not shown to be this MPC's. The copy is in $check."
  exit 1
fi
python3 tools/mpc-one/compare-emmc-backups.py "$ref" "$copy" 2>&1 | sed 's/^RESULT: /compared: /; s/^/  /'
crc=${PIPESTATUS[0]}
echo
if [ "$crc" -ne 0 ]; then
  echo "RESULT: NOT this MPC's backup, or a damaged one: it differs from $(basename "$ref") where it must be the same (above). The copy is in $check."
  exit 1
fi
if [ -n "$same" ]; then
  rm -rf "$check"
  kept="The check's copy was removed: $same is the same backup."
else
  kept="The copy is kept in $copy, the only copy of it on this computer (delete it with: rm -rf $check)."
fi
if [ "$named" = yes ]; then
  echo "RESULT: this is the round's stick. Its backup $newest is intact, and it is this MPC's. $kept"
  exit 0
fi
echo "RESULT: NOT the stick the round's notes name (its newest backup is $newest, not one of: $EXPECTED), but its backup is intact and this MPC's, so the round can keep it. $kept"
exit 1
