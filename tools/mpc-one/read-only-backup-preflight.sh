#!/bin/sh
# Read-only helper for the eMMC backup (collect-over-ssh.sh --backup).
# Streamed to the MPC over SSH as `sh -s hash` (before the copy) or
# `sh -s stat` (after it). Prints machine-readable lines for host_emmc_backup.py.
#
# It reads procfs and sysfs, reads block devices only through
# `dd if=... | sha256sum` pipelines (no output file), and reads the eMMC
# EXT_CSD only if debugfs is already mounted. It mounts nothing, writes
# nothing, and changes no service or setting. Uses only BusyBox 1.36 syntax
# that the boot-map survey already exercised on az0x.
#
# MPFREE_TEST_ROOT is for the host-side test suite only: it prefixes every
# path so the script can run against a fake sysfs/dev tree. It is unset on
# the MPC, where every path is the real one.
set -u
R="${MPFREE_TEST_ROOT:-}"
mode="${1:-}"
case "$mode" in
  hash|stat) ;;
  *) echo "ERROR usage: sh -s hash|stat"; exit 2 ;;
esac

DEVICES='mmcblk0 mmcblk0p1 mmcblk0p2 mmcblk0p3 mmcblk0p4 mmcblk0p5 mmcblk0p6 mmcblk0p7 mmcblk0p8 mmcblk0boot0 mmcblk0boot1'
HASHED='mmcblk0p1 mmcblk0p2 mmcblk0p3 mmcblk0p4 mmcblk0p5 mmcblk0p6 mmcblk0boot0 mmcblk0boot1'

boot_id="$(cat "$R/proc/sys/kernel/random/boot_id" 2>/dev/null)"
echo "MODE $mode"
echo "BOOT_ID ${boot_id:--}"

# Write counters come first, so that the host's before/after comparison
# covers the hashing below as well as the copy itself.
for name in $DEVICES; do
  d="$R/sys/class/block/$name"
  if [ ! -r "$d/stat" ]; then
    echo "ABSENT $name"
    continue
  fi
  # shellcheck disable=SC2046 # word splitting collapses sysfs padding on purpose
  echo "STAT $name" $(cat "$d/stat")
done

if [ "$mode" = hash ]; then
  for name in $DEVICES; do
    d="$R/sys/class/block/$name"
    [ -r "$d/size" ] || continue
    start="$(cat "$d/start" 2>/dev/null)"
    pname="$(sed -n 's/^PARTNAME=//p' "$d/uevent" 2>/dev/null)"
    echo "GEOM $name ${start:--} $(cat "$d/size") ${pname:--}"
  done
  emmc="$(cat "$R/sys/block/mmcblk0/device/name" 2>/dev/null)"
  echo "EMMC ${emmc:--}"
  echo "KERNEL $(uname -r)"
  # Sectors 0-63: protective MBR, primary GPT and the gap before p1.
  # Read from offset 0 only, so no seek is needed.
  if [ -r "$R/dev/mmcblk0" ]; then
    echo "HASH gpt-head $(dd if="$R/dev/mmcblk0" bs=512 count=64 2>/dev/null | sha256sum | awk '{print $1}')"
  else
    echo "HASH gpt-head -"
  fi
  for name in $HASHED; do
    if [ -r "$R/dev/$name" ]; then
      echo "HASH $name $(dd if="$R/dev/$name" bs=1048576 2>/dev/null | sha256sum | awk '{print $1}')"
    else
      echo "HASH $name -"
    fi
  done
  # EXT_CSD holds the eMMC boot configuration, which no partition image
  # contains. Read it only if debugfs is already mounted; never mount it.
  card="$(readlink "$R/sys/block/mmcblk0/device" 2>/dev/null)"
  card="${card##*/}"
  host="${card%%:*}"
  ext="$R/sys/kernel/debug/$host/$card/ext_csd"
  if [ -n "$card" ] && [ -r "$ext" ]; then
    echo "EXT_CSD $(head -n 1 "$ext" 2>/dev/null)"
  else
    echo "EXT_CSD -"
  fi
fi
echo "END $mode"
