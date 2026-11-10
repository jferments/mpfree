#!/usr/bin/env bash
# OFFLINE, workstation only: survey an MPC root filesystem image read-only,
# to see what a firmware version would mount, start and flash before it is
# ever run on the MPC.
#
# Two sources:
#   --image FILE    an ext2/3/4 rootfs image, such as the rootfs unpacked from
#                   an Akai update by offline-update-extract.py. Default: the
#                   single *rootfs*.img in the newest ~/mpfree-work/akai-update-*/
#   --from-backup   the 3.6.0 rootfs (p6) cut from the newest verified eMMC
#                   backup and checked against the boot map, for comparison
#
# It never contacts the MPC and never writes the image or the backup: debugfs
# opens images read-only unless given -w, which this script never passes.
# Output goes to a new folder under ~/mpfree-work. It quotes Akai's files,
# so keep it private and commit only findings.
set -Eeuo pipefail
umask 077
PATH="$PATH:/usr/sbin:/sbin"

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"
BASELINE="$SCRIPT_DIR/baselines/mpc-one-3.6.0-reference.txt"
BACKUP_ROOT="${MPC_BACKUP_DIR:-$HOME/mpfree-backups}"
WORK_ROOT="${MPFREE_WORK_DIR:-$HOME/mpfree-work}"
SOURCE=""
IMAGE_ARG=""

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

while (( $# )); do
  case "$1" in
    --image) SOURCE=image; IMAGE_ARG="${2:?}"; shift 2 ;;
    --from-backup) SOURCE=backup; shift ;;
    --baseline) BASELINE="${2:?}"; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done
: "${SOURCE:=image}"

for tool in debugfs file strings sha256sum dd awk sort realpath python3; do
  command -v "$tool" >/dev/null 2>&1 ||
    die "missing '$tool'. Install e2fsprogs, file, binutils and python3 with your package manager."
done

work_abs="$(realpath -m -- "$WORK_ROOT")"
case "$work_abs/" in
  "$REPO_ROOT"/*)
    git -C "$REPO_ROOT" check-ignore -q -- "$work_abs/probe" 2>/dev/null ||
      die "MPFREE_WORK_DIR is inside the git clone and not git-ignored: $work_abs" ;;
esac
[[ "$work_abs" != *[[:space:]\"\']* ]] || die "MPFREE_WORK_DIR must not contain spaces or quotes: $work_abs"

if [[ "$SOURCE" == image && -z "$IMAGE_ARG" ]]; then
  shopt -s nullglob
  folders=("$work_abs"/akai-update-*/)
  (( ${#folders[@]} )) || die "no unpacked update under $work_abs (run offline-update-extract.py first) or pass --image"
  found=("${folders[-1]}"*rootfs*.img)
  shopt -u nullglob
  (( ${#found[@]} == 1 )) || die "expected one *rootfs*.img in ${folders[-1]}, found ${#found[@]}; pass --image"
  IMAGE_ARG="${found[0]}"
fi
if [[ "$SOURCE" == image ]]; then
  [[ -r "$IMAGE_ARG" && -f "$IMAGE_ARG" ]] || die "cannot read $IMAGE_ARG"
  IMAGE_ARG="$(realpath -- "$IMAGE_ARG")"
  [[ "$IMAGE_ARG" != *[[:space:]\"\']* ]] || die "the image path must not contain spaces or quotes: $IMAGE_ARG"
fi

mkdir -p -- "$work_abs"
WORK="$(mktemp -d "$work_abs/rootfs-survey-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
REPORT="$WORK/report.txt"
exec > >(tee "$REPORT") 2>&1
echo "MPFree offline rootfs survey (no MPC contact)"
echo "work folder: $WORK"

if [[ "$SOURCE" == backup ]]; then
  [[ -r "$BASELINE" ]] || die "missing baseline: $BASELINE"
  shopt -s nullglob
  verified=("$BACKUP_ROOT"/mpc-one-emmc-*/SHA256SUMS)
  shopt -u nullglob
  (( ${#verified[@]} )) || die "no verified backup (a folder with SHA256SUMS) under $BACKUP_ROOT"
  backup="${verified[-1]%/SHA256SUMS}"
  read -r p6_start p6_sectors p6_sha < <(awk '$1 == "part" && $2 == "mmcblk0p6" { print $4, $5, $6 }' "$BASELINE")
  [[ "$p6_start" =~ ^[0-9]+$ && "$p6_sectors" =~ ^[0-9]+$ && "$p6_sha" =~ ^[0-9a-f]{64}$ ]] ||
    die "baseline has no usable mmcblk0p6 line"
  ROOTFS="$WORK/p6-rootfs.img"
  echo "source: p6 of $backup"
  dd if="$backup/mmcblk0.img" of="$ROOTFS" bs=1048576 iflag=skip_bytes,count_bytes \
     skip=$(( p6_start * 512 )) count=$(( p6_sectors * 512 )) status=none
  [[ "$(sha256sum "$ROOTFS" | awk '{print $1}')" == "$p6_sha" ]] ||
    die "the backup's rootfs does not match the boot-map hash; stopping"
  echo "p6 matches the boot map"
else
  ROOTFS="$IMAGE_ARG"
  echo "source: $ROOTFS"
fi
echo "sha256 $(sha256sum "$ROOTFS" | awk '{print $1}')"
echo

dfs() { debugfs -R "$1" "$ROOTFS" 2>/dev/null; }
[[ -n "$(dfs 'ls -p /')" ]] || die "debugfs could not read $ROOTFS (not ext2/3/4, or e2fsprogs too old?)"
rootfs_type() { dfs "stat $1" | sed -n 's/^Inode: [0-9]*  *Type: \([a-z]*\).*/\1/p' | head -n 1; }
link_target() {  # a symlink's target: fast links are stored in the inode, slow ones in a block
  local target
  target="$(dfs "stat $1" | sed -n 's/^Fast link dest: "\(.*\)"$/\1/p' | head -n 1)"
  if [[ -z "$target" ]]; then
    rm -f -- "$WORK/link.tmp"
    dfs "dump $1 $WORK/link.tmp"
    target="$(head -c 4096 -- "$WORK/link.tmp" 2>/dev/null | tr -d '\0')"
    rm -f -- "$WORK/link.tmp"
  fi
  printf '%s' "${target:-(target unreadable)}"
}
names_in() {  # $1 = directory; prints "type name" per entry, skipping . and ..
  dfs "ls -p $1" | awk -F/ 'NF >= 7 && $6 != "." && $6 != ".." {
    t = substr($3, 1, 2); kind = (t == "04") ? "dir" : (t == "12") ? "link" : "file"; print kind, $6 }'
}
show_file() {  # $1 = path in the image; prints it if it's a small text file
  local path="$1" tmp="$WORK/show.tmp" type
  [[ "$path" =~ ^[A-Za-z0-9@._+/-]+$ ]] || { echo "($path: unusual name, skipped)"; return 0; }
  type="$(rootfs_type "$path")"
  case "$type" in
    regular) ;;
    "") echo "($path: not in the image)"; return 0 ;;
    symlink) echo "--- $path -> $(link_target "$path")"; return 0 ;;
    *) echo "($path: $type, not a regular file)"; return 0 ;;
  esac
  rm -f -- "$tmp"
  dfs "dump $path $tmp"
  if [[ "$(file -b -- "$tmp")" == *text* ]] && (( $(stat -c %s -- "$tmp") <= 65536 )); then
    echo "--- $path"; cat -- "$tmp"
  else
    echo "--- $path: $(file -b -- "$tmp") ($(stat -c %s -- "$tmp") bytes)"
  fi
  rm -f -- "$tmp"
}

echo "=== 1. Identity ==="
for f in /etc/issue /etc/os-release /etc/version /etc/timestamp; do show_file "$f"; done
echo "--- ls -l /boot"
dfs "ls -l /boot"
for d in /usr/lib/modules /lib/modules; do
  [[ "$(rootfs_type "$d")" == directory ]] && { echo "--- kernel module folders in $d"; names_in "$d"; }
done
echo

echo "=== 2. What it mounts ==="
show_file /etc/fstab
UNIT_DIRS=(/usr/lib/systemd/system /lib/systemd/system /etc/systemd/system)
for d in "${UNIT_DIRS[@]}"; do
  [[ "$(rootfs_type "$d")" == directory ]] || continue
  while read -r kind name; do
    [[ "$kind" == file && "$name" == *.mount ]] && show_file "$d/$name"
  done < <(names_in "$d")
done
echo

echo "=== 3. Units ==="
UNIT_PATTERN='(update|upd|flash|fw|dfu|firmware|xmos|touch|ilitek|resize|grow|format|factory|repart|overlay|recover|loader|mmc|emmc|partition|data|content)'
for d in "${UNIT_DIRS[@]}"; do
  [[ "$(rootfs_type "$d")" == directory ]] || continue
  echo "--- all entries in $d"
  names_in "$d" | sort -k2
done
for d in "${UNIT_DIRS[@]}"; do
  [[ "$(rootfs_type "$d")" == directory ]] || continue
  while read -r kind name; do
    [[ "$kind" == dir ]] && continue
    [[ "$name" =~ $UNIT_PATTERN ]] && show_file "$d/$name"
  done < <(names_in "$d" | sort -k2)
done
echo "--- started at boot (.wants and .requires folders)"
for d in "${UNIT_DIRS[@]}"; do
  [[ "$(rootfs_type "$d")" == directory ]] || continue
  while read -r kind name; do
    [[ "$kind" == dir && "$name" =~ ^[A-Za-z0-9@._+-]+\.(wants|requires)$ ]] || continue
    names_in "$d/$name" | awk -v d="$d/$name" '{ print d ": " $2 }' | sort
  done < <(names_in "$d" | sort -k2)
done
echo

echo "=== 4. Programs whose names suggest flashing or storage work ==="
PROG_PATTERN='(fw|dfu|flash|upd|firmware|xmos|ilitek|boot|loader|mmc|emmc|prog|recover|factory)'
for d in /usr/bin /usr/sbin /usr/libexec /bin /sbin; do
  [[ "$(rootfs_type "$d")" == directory ]] || continue
  names_in "$d" | awk -v d="$d" -v re="$PROG_PATTERN" '$2 ~ re { print d "/" $2 " (" $1 ")" }'
done
echo

echo "=== 5. Firmware blobs ==="
for d in /usr/lib/firmware /lib/firmware; do
  [[ "$(rootfs_type "$d")" == directory ]] || continue
  echo "--- $d"
  while read -r kind name; do
    echo "$kind $name"
    if [[ "$kind" == dir && "$name" =~ ^[A-Za-z0-9@._+-]+$ ]]; then
      names_in "$d/$name" | sed 's/^/    /'
    fi
  done < <(names_in "$d" | sort -k2)
done
echo

echo "=== 6. Files that mention flashing or the eMMC ==="
match_offsets() {  # $1 = needle; byte offset of every match (exact, low memory)
  python3 -I -c '
import mmap, sys
with open(sys.argv[1], "rb") as fh, mmap.mmap(fh.fileno(), 0, access=mmap.ACCESS_READ) as m:
    needle, count = sys.argv[2].encode(), 0
    i = m.find(needle)
    while i != -1 and count < 5000:
        print(i)
        count += 1
        i = m.find(needle, i + 1)
' "$ROOTFS" "$1"
}
BLOCK_SIZE="$(dfs stats | sed -n 's/^Block size: *\([0-9][0-9]*\)$/\1/p')"
[[ "$BLOCK_SIZE" =~ ^[0-9]+$ ]] || die "debugfs did not report the block size"
NEEDLES=(ramfs-update erase-data-partition fastboot mmcblk0boot /dev/mmcblk0 xmosdfu touch-fw-upd dfu-util
         update-bootloader flash-bootloader uboot-spl reboot-argument=)
for needle in "${NEEDLES[@]}"; do
  blocks="$(match_offsets "$needle" |
    awk -v bs="$BLOCK_SIZE" '{ print int($1 / bs) }' | sort -un | head -n 500 | tr '\n' ' ')"
  if [[ -z "${blocks// /}" ]]; then echo "$needle: no match"; continue; fi
  owners="$(dfs "icheck $blocks")"
  unowned="$(awk 'NR > 1 && /block not found/' <<<"$owners" | wc -l)"
  inodes="$(awk 'NR > 1 && $2 ~ /^[0-9]+$/ { print $2 }' <<<"$owners" | sort -un | tr '\n' ' ')"
  (( unowned == 0 )) || echo "$needle: $unowned match(es) outside file data (free space or metadata)"
  [[ -n "${inodes// /}" ]] || continue
  while IFS=$'\t' read -r ino path; do
    [[ "$ino" =~ ^[0-9]+$ ]] || continue
    while [[ "$path" == *//* ]]; do path="${path//\/\//\/}"; done
    if [[ "$(rootfs_type "$path")" == directory ]]; then
      echo "$needle: $path (a file name in this folder)"
    else
      echo "$needle: $path"
    fi
  done < <(dfs "ncheck $inodes")
done
echo

if [[ "$SOURCE" == backup ]]; then rm -f -- "$ROOTFS"; fi
echo "DONE. Report: $REPORT"
echo "It quotes Akai's files: keep it private and commit only findings."
