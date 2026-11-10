#!/usr/bin/env bash
# Read-only inspection of an official Akai MPC Gen1 3.x update image.
# Works entirely on the workstation; never connects to or writes to the MPC.
# Does NOT repack firmware or modify the original update image.
set -Eeuo pipefail

usage() {
  cat <<'HELP'
Usage:
  bash tools/mpc-one/inspect-update-image.sh /absolute/path/to/update.img [--extract]

By default, identify the update image and save a header/hash report.
--extract also runs the independently installed TheKikGen mpcimg2 -r extractor,
mounts the resulting ext filesystem READ-ONLY on the workstation, and copies a
small set of binaries and boot metadata for Ghidra analysis.

Environment:
  MPFREE_FIRMWARE_WORKDIR  Override analysis output directory.
  MPCIMG2                  Path to reviewed mpcimg2 executable, needed for --extract.

The script never executes any binary extracted from the proprietary firmware.
HELP
}

if [[ $# -lt 1 || $# -gt 2 ]]; then usage >&2; exit 2; fi
FW=$1
MODE=${2:---probe}
[[ $MODE == --probe || $MODE == --extract ]] || { usage >&2; exit 2; }
[[ -f $FW && -r $FW ]] || { printf 'Firmware not readable: %s\n' "$FW" >&2; exit 1; }
FW=$(realpath -- "$FW")
BASE=$(basename -- "$FW")
BASE=${BASE%.img}
OUT=${MPFREE_FIRMWARE_WORKDIR:-$HOME/mpfree-work/firmware-analysis/$BASE}
mkdir -p -- "$OUT"
OUT=$(realpath -- "$OUT")
[[ $OUT != / && $OUT != "$(dirname -- "$FW")" ]] || {
  echo 'Choose an analysis output directory distinct from the firmware directory.' >&2
  exit 1
}
REPORT="$OUT/inspection.txt"
MAGIC=$(od -An -tx1 -N4 -- "$FW" | tr -d '[:space:]')
{
  echo 'MPFree offline Akai MPC firmware survey (workstation only)'
  printf 'File: %s\n' "$FW"
  printf 'Bytes: '; stat -c '%s' -- "$FW"
  printf 'SHA-256: '; sha256sum -- "$FW"
  printf 'file: '; file -- "$FW"
  printf 'Magic hex: %s\n' "$MAGIC"
  echo 'First 128 bytes:'
  od -Ax -tx1 -N128 -- "$FW"
  echo 'End header'
} | tee "$REPORT"

if [[ $MAGIC != 415a3031 ]]; then
  echo 'WARNING: Image does not start with AZ01. Do not assume the MPC 3.x extractor applies.' | tee -a "$REPORT"
  [[ $MODE != --extract ]] || exit 1
fi

[[ $MODE != --extract ]] && { echo "Report: $REPORT"; exit 0; }

if [[ -n ${MPCIMG2:-} ]]; then
  EXTRACTOR=$MPCIMG2
elif command -v mpcimg2 >/dev/null 2>&1; then
  EXTRACTOR=$(command -v mpcimg2)
else
  cat >&2 <<'ERROR'
MPCIMG2 is not set and mpcimg2 was not found in PATH.
Review/install TheKikGen MPC-LiveXplore's mpcimg2 locally, then run:
  MPCIMG2=/absolute/path/to/mpcimg2 bash tools/mpc-one/inspect-update-image.sh /absolute/path/to/update.img --extract
ERROR
  exit 1
fi
[[ -f $EXTRACTOR && -x $EXTRACTOR ]] || { echo "Extractor missing or non-executable: $EXTRACTOR" >&2; exit 1; }
ROOTFS="$OUT/rootfs.ext4"
if [[ ! -f $ROOTFS ]]; then
  echo "Extracting rootfs to $ROOTFS ..."
  "$EXTRACTOR" -r "$FW" "$ROOTFS"
else
  echo "Already exists: $ROOTFS (not overwriting)."
fi
[[ -s $ROOTFS ]] || { echo "Rootfs extraction failed: $ROOTFS" >&2; exit 1; }
ROOTFS_MAGIC=$(od -An -tx1 -j1080 -N2 -- "$ROOTFS" | tr -d '[:space:]')
{
  echo 'Extracted filesystem:'
  printf 'file: '; file -- "$ROOTFS"
  printf 'Bytes: '; stat -c '%s' -- "$ROOTFS"
  printf 'SHA-256: '; sha256sum -- "$ROOTFS"
  printf 'Ext superblock magic at 0x438: %s\n' "$ROOTFS_MAGIC"
} | tee -a "$REPORT"
[[ $ROOTFS_MAGIC == 53ef ]] || {
  echo 'Not a recognized ext2/3/4 filesystem; stop before trying to mount.' >&2
  exit 1
}

MNT="$OUT/mount-readonly"
mkdir -p -- "$MNT"
if mountpoint -q -- "$MNT"; then
  echo "Refusing to use an already-mounted directory: $MNT" >&2
  exit 1
fi
mounted=0
cleanup() {
  if [[ $mounted -eq 1 ]]; then
    sudo umount -- "$MNT" || echo "WARNING: could not unmount $MNT" >&2
  fi
}
trap cleanup EXIT
# ext4's 'noload' prevents journal replay; 'ro' prevents writes even if journaled.
sudo mount -t ext4 -o loop,ro,noload -- "$ROOTFS" "$MNT"
mounted=1
EXTRACTED="$OUT/extracted"
mkdir -p -- "$EXTRACTED"
COPY_PATHS=(
  usr/bin/MPC
  usr/bin/az01-script-runner
  usr/bin/az01-launch-MPC
  usr/bin/az01-update
  boot/boot.scr.uimg
  opt/ramfs-update.squashfs
  usr/lib/systemd/system/acvs.service
  usr/lib/systemd/system/az01-script-runner.service
  usr/lib/systemd/system/az01-update.service
  etc/os-release
  usr/lib/os-release
  etc/az01-release
)
for rel in "${COPY_PATHS[@]}"; do
  if [[ -f "$MNT/$rel" ]]; then
    mkdir -p -- "$EXTRACTED/$(dirname -- "$rel")"
    cp -L -- "$MNT/$rel" "$EXTRACTED/$rel"
  fi
done
{
  echo 'Relevant executables in firmware (paths only):'
  if [[ -d "$MNT/usr/bin" ]]; then
    find "$MNT/usr/bin" -maxdepth 1 -type f -printf '%f\n' | grep -Ei '(az01|mpc|autorun|script|update|launch)' || :
  fi
  echo 'Relevant services (paths only):'
  for d in "$MNT/usr/lib/systemd/system" "$MNT/lib/systemd/system" "$MNT/etc/systemd/system"; do
    [[ -d $d ]] || continue
    find "$d" -maxdepth 1 -type f -printf '%f\n' | grep -Ei '(acvs|mpc|autorun|script|update|mount|usb)' || :
  done
  echo 'Boot directory listing:'
  [[ ! -d "$MNT/boot" ]] || find "$MNT/boot" -maxdepth 1 -type f -printf '%f\n'
} | tee -a "$REPORT"
{
  echo 'COPIED FILES (local analysis):'
  find "$EXTRACTED" -type f -print0 | sort -z | xargs -0 -r sha256sum
  echo 'ELF headers:'
  for f in "$EXTRACTED/usr/bin/"*; do
    [[ -f $f ]] || continue
    printf '\n%s\n' "$f"
    file -- "$f"
    readelf -h -- "$f" 2>/dev/null | grep -E 'Class:|Data:|Machine:|Entry point address:|Flags:' || :
  done
} | tee -a "$REPORT"
echo "Done. Ghidra-ready files: $EXTRACTED"
echo "Report: $REPORT"
