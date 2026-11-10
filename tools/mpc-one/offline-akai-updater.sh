#!/usr/bin/env bash
# OFFLINE, workstation only: extract Akai's on-device updater from a verified
# eMMC backup (collect-over-ssh.sh --backup) and write a text report of what
# it contains, so we can read which partitions an official update writes.
#
# It never contacts the MPC: no ssh, no network. It never writes to the
# backup. It copies the rootfs region (p6) out of mmcblk0.img into a new work
# folder and only reads that copy. debugfs opens an image read-only unless
# given -w, which this script never passes.
#
# Output goes to a new folder under ~/mpfree-work, outside the git clone,
# because the extracted files are Akai's proprietary software. Never commit
# or share them. Share only the findings.
#
# Usage: bash tools/mpc-one/offline-akai-updater.sh [--backup DIR]
#          [--baseline FILE] [--updater-sha256 HEX]
# Defaults: the newest verified backup in ~/mpfree-backups (MPC_BACKUP_DIR),
# the reference baseline, and the /opt/ramfs-update.squashfs hash from the
# 2026-11-09 boot map. MPFREE_WORK_DIR overrides ~/mpfree-work.
set -Eeuo pipefail
umask 077
# debugfs lives in /usr/sbin, which some distributions leave out of a normal user's PATH.
PATH="$PATH:/usr/sbin:/sbin"

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"
BASELINE="$SCRIPT_DIR/baselines/mpc-one-3.6.0-reference.txt"
UPDATER_SHA256="39db4a08bc988d8cb8ce24fb40e29ed6289c1484753e07c52c842906ad24fadb"
BACKUP=""
BACKUP_ROOT="${MPC_BACKUP_DIR:-$HOME/mpfree-backups}"
WORK_ROOT="${MPFREE_WORK_DIR:-$HOME/mpfree-work}"
MAX_TEXT_BYTES=262144

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

while (( $# )); do
  case "$1" in
    --backup) BACKUP="${2:?}"; shift 2 ;;
    --baseline) BASELINE="${2:?}"; shift 2 ;;
    --updater-sha256) UPDATER_SHA256="${2:?}"; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done

for tool in debugfs unsquashfs file strings sha256sum dd awk sort realpath python3; do
  command -v "$tool" >/dev/null 2>&1 ||
    die "missing '$tool'. Install e2fsprogs, squashfs-tools, file, binutils and python3 with your package manager."
done
[[ -r "$BASELINE" ]] || die "missing baseline: $BASELINE"
[[ "$UPDATER_SHA256" =~ ^[0-9a-f]{64}$ ]] || die "--updater-sha256 must be 64 lowercase hex digits"

if [[ -z "$BACKUP" ]]; then
  # Folder names start with the UTC time, so the last one in sorted order is the newest.
  shopt -s nullglob
  verified=("$BACKUP_ROOT"/mpc-one-emmc-*/SHA256SUMS)
  shopt -u nullglob
  (( ${#verified[@]} )) || die "no verified backup (a folder with SHA256SUMS) under $BACKUP_ROOT"
  BACKUP="${verified[-1]%/SHA256SUMS}"
fi
IMAGE="$BACKUP/mmcblk0.img"
[[ -r "$IMAGE" ]] || die "cannot read $IMAGE"

read -r P6_START P6_SECTORS P6_SHA256 < <(
  awk '$1 == "part" && $2 == "mmcblk0p6" { print $4, $5, $6 }' "$BASELINE")
[[ "$P6_START" =~ ^[0-9]+$ && "$P6_SECTORS" =~ ^[0-9]+$ && "$P6_SHA256" =~ ^[0-9a-f]{64}$ ]] ||
  die "baseline has no usable mmcblk0p6 line"

# Every check on the work folder happens before anything is created.
work_abs="$(realpath -m -- "$WORK_ROOT")"
backup_abs="$(realpath -- "$BACKUP")"
case "$work_abs/" in
  "$backup_abs"/*) die "MPFREE_WORK_DIR must not be inside the backup: $work_abs" ;;
  "$REPO_ROOT"/*)
    git -C "$REPO_ROOT" check-ignore -q -- "$work_abs/probe" 2>/dev/null ||
      die "MPFREE_WORK_DIR is inside the git clone and not git-ignored: $work_abs" ;;
esac
# debugfs splits its -R command on spaces, so the output path must not contain any.
[[ "$work_abs" != *[[:space:]\"\']* ]] || die "MPFREE_WORK_DIR must not contain spaces or quotes: $work_abs"

mkdir -p -- "$work_abs"
WORK="$(mktemp -d "$work_abs/akai-updater-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
REPORT="$WORK/report.txt"
ROOTFS="$WORK/p6-rootfs.img"
exec > >(tee "$REPORT") 2>&1

echo "MPFree offline Akai updater report (no MPC contact)"
echo "backup: $backup_abs"
echo "work folder: $WORK"
echo

echo "=== 1. Copy the rootfs (p6) out of the backup and check it ==="
dd if="$IMAGE" of="$ROOTFS" bs=1048576 iflag=skip_bytes,count_bytes \
   skip=$(( P6_START * 512 )) count=$(( P6_SECTORS * 512 )) status=none
got="$(sha256sum "$ROOTFS" | awk '{print $1}')"
echo "p6 sha256 $got"
[[ "$got" == "$P6_SHA256" ]] ||
  die "the backup's rootfs does not match the boot-map hash ($P6_SHA256); stopping"
echo "matches the boot map"
echo

dfs() { debugfs -R "$1" "$ROOTFS" 2>/dev/null; }
# Prints regular, symlink, directory, ... for a path in the rootfs, or nothing if absent.
rootfs_type() { dfs "stat $1" | sed -n 's/^Inode: [0-9]*  *Type: \([a-z]*\).*/\1/p' | head -n 1; }
rootfs_link() {  # prints a symlink's target (short targets live in the inode)
  local target
  target="$(dfs "stat $1" | sed -n 's/^Fast link dest: "\(.*\)"$/\1/p')"
  if [[ -z "$target" ]]; then
    dfs "dump $1 $WORK/link-target.tmp"
    target="$(cat -- "$WORK/link-target.tmp" 2>/dev/null || true)"
    rm -f -- "$WORK/link-target.tmp"
  fi
  printf '%s\n' "$target"
}
dump_file() {  # $1 = path in the rootfs, $2 = output file; follows up to 3 symlinks
  local path="$1" type="" target hop
  for hop in 1 2 3 4; do
    # debugfs splits commands on spaces, so only plain paths are passed to it.
    [[ "$path" =~ ^[A-Za-z0-9@._+/-]+$ ]] || { echo "$path: unusual name, not copied"; return 0; }
    type="$(rootfs_type "$path")"
    [[ "$type" == symlink && $hop -lt 4 ]] || break
    target="$(rootfs_link "$path")"
    echo "$path -> $target"
    case "$target" in /*) path="$target" ;; *) path="${path%/*}/$target" ;; esac
  done
  case "$type" in
    regular) dfs "dump $path $2"; echo "copied $path" ;;
    "") echo "$path: not found" ;;
    *) echo "$path: $type, not copied" ;;
  esac
}
[[ -n "$(dfs 'ls -p /')" ]] || die "debugfs could not read the rootfs copy (e2fsprogs too old?)"

echo "=== 2. Update-related files in the rootfs (debugfs, read-only) ==="
echo "--- ls -l /opt ---"
dfs "ls -l /opt"
dump_file /opt/ramfs-update.squashfs "$WORK/ramfs-update.squashfs"
[[ -s "$WORK/ramfs-update.squashfs" ]] || die "/opt/ramfs-update.squashfs not found in the rootfs"
got="$(sha256sum "$WORK/ramfs-update.squashfs" | awk '{print $1}')"
echo "ramfs-update.squashfs sha256 $got"
[[ "$got" == "$UPDATER_SHA256" ]] ||
  die "ramfs-update.squashfs does not match the boot-map hash ($UPDATER_SHA256); stopping"
echo "matches the boot map"

mkdir -p "$WORK/rootfs-files"
echo "--- ls -l /boot ---"
dfs "ls -l /boot"
dump_file /boot/boot.scr.uimg "$WORK/rootfs-files/boot.scr.uimg"
echo "--- systemd units whose names mention update, upd, flash or fw ---"
units="$(dfs "ls -p /usr/lib/systemd/system" |
  awk -F/ 'NF >= 7 && $6 ~ /(update|upd|flash|fw)/ && $6 ~ /^[A-Za-z0-9@._-]+$/ { print $6 }' |
  sort -u)"
for unit in $units; do
  dump_file "/usr/lib/systemd/system/$unit" "$WORK/rootfs-files/$unit"
done
[[ -n "$units" ]] || echo "(none)"
echo

echo "=== 3. Contents of ramfs-update.squashfs ==="
unsquashfs -lls "$WORK/ramfs-update.squashfs" | sed '/^Parallel unsquashfs/d'
# Unpacking as a normal user cannot create device nodes or set owners.
# Those entries are skipped (they are listed above), and every regular file
# is kept. -ignore-errors only exists in newer squashfs-tools, so it is added
# only when offered.
unsq_flags=(-no-xattrs)
if unsquashfs -h 2>&1 | grep -q -- '-ignore-errors'; then unsq_flags+=(-ignore-errors); fi
unsquashfs "${unsq_flags[@]}" -d "$WORK/ramfs-update" "$WORK/ramfs-update.squashfs" \
  >/dev/null 2>"$WORK/unsquashfs-warnings.txt" || true
[[ -d "$WORK/ramfs-update" ]] || die "unsquashfs could not unpack the updater"
echo

describe() {  # $1 = file, $2 = label
  local f="$1" label="$2" kind
  kind="$(file -b -- "$f")"
  echo "----- $label: $kind"
  case "$kind" in
    *ELF*)
      echo "(binary: strings that mention storage, partitions or update steps)"
      { strings -n 5 -- "$f" |
          grep -E -i 'mmcblk|/dev/|fastboot|partition|rootfs|uboot|u-boot|spl|env|flash|mkfs|mount|squashfs|update|dd |gpt|sgdisk|sfdisk|bootloader|recovery|verify|sha|md5|signature|version' |
          sort -u | head -n 300; } || true ;;
    *u-boot*|*uImage*)
      echo "(U-Boot image: printable strings)"
      { strings -n 3 -- "$f" | head -n 200; } || true ;;
    *text*|*script*)
      if (( $(stat -c %s -- "$f") <= MAX_TEXT_BYTES )); then
        cat -- "$f"
      else
        echo "(text longer than $MAX_TEXT_BYTES bytes; first 400 lines)"
        sed -n '1,400p' -- "$f"
      fi ;;
    *) ;;
  esac
  echo
}

echo "=== 4. Files in the updater (scripts in full, binaries summarised) ==="
while IFS= read -r -d '' f; do
  describe "$f" "ramfs-update${f#"$WORK/ramfs-update"}"
done < <(find "$WORK/ramfs-update" -type f -print0 | sort -z)

echo "=== 5. Update files from the rootfs ==="
while IFS= read -r -d '' f; do
  describe "$f" "rootfs:${f##*/}"
done < <(find "$WORK/rootfs-files" -type f -print0 | sort -z)

# What starts the RAM updater is not in any update-named unit, so search the
# whole rootfs copy: find each needle's byte offsets, turn them into
# filesystem blocks, and ask debugfs which inode owns each block (icheck) and
# what its path is (ncheck). A directory owns a block when a file *name*
# matches. A match in a block that no file owns is leftover data in free space.
NEEDLES=(ramfs-update erase-data-partition)
mention_context() {  # $1 = file, $2 = label
  local f="$1" kind
  kind="$(file -b -- "$f")"
  echo "----- $2: $kind"
  case "$kind" in
    *ELF*) ;;
    *text*|*script*)
      { grep -n -F -C 8 -e "${NEEDLES[0]}" -e "${NEEDLES[1]}" -- "$f" | head -n 200; } || true
      echo
      return 0 ;;
  esac
  echo "(printable strings around each match)"
  { strings -n 4 -- "$f" | grep -n -F -C 12 -e "${NEEDLES[0]}" -e "${NEEDLES[1]}" | head -n 300; } || true
  echo
}

# Byte offset of every match in the p6 copy. Python's mmap keeps memory low and
# the offsets exact; GNU grep -b needs the image's longest newline-free run in
# memory, and with -z it reported wrong offsets (grep 3.11, 2026-11-10).
match_offsets() {  # $1 = needle
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

echo "=== 6. Files in the rootfs that mention the updater ==="
BLOCK_SIZE="$(dfs stats | sed -n 's/^Block size: *\([0-9][0-9]*\)$/\1/p')"
[[ "$BLOCK_SIZE" =~ ^[0-9]+$ ]] || die "debugfs did not report the rootfs block size"
echo "searching for: ${NEEDLES[*]} (block size $BLOCK_SIZE)"
mkdir -p "$WORK/mentions"
declare -A copied=()
for needle in "${NEEDLES[@]}"; do
  blocks="$(match_offsets "$needle" |
    awk -v bs="$BLOCK_SIZE" '{ print int($1 / bs) }' | sort -un | head -n 500 | tr '\n' ' ')"
  if [[ -z "${blocks// /}" ]]; then
    echo "$needle: no match"
    continue
  fi
  owners="$(dfs "icheck $blocks")"
  # A block no file owns is either free (deleted data) or filesystem metadata,
  # such as an inode holding a short symlink target or a tiny inline file.
  free_blocks=0 meta_blocks=()
  while read -r b; do
    [[ "$b" =~ ^[0-9]+$ ]] || continue
    if dfs "testb $b" | grep -q 'marked in use'; then meta_blocks+=("$b"); else free_blocks=$((free_blocks + 1)); fi
  done < <(awk 'NR > 1 && /block not found/ { print $1 }' <<<"$owners")
  (( free_blocks == 0 )) || echo "$needle: $free_blocks match(es) in free space (deleted data, no file)"
  (( ${#meta_blocks[@]} == 0 )) ||
    echo "$needle: match(es) in filesystem metadata, block(s) ${meta_blocks[*]} (for example a short symlink target)"
  inodes="$(awk 'NR > 1 && $2 ~ /^[0-9]+$/ { print $2 }' <<<"$owners" | sort -un | tr '\n' ' ')"
  [[ -n "${inodes// /}" ]] || continue
  while IFS=$'\t' read -r ino path; do
    [[ "$ino" =~ ^[0-9]+$ ]] || continue
    while [[ "$path" == *//* ]]; do path="${path//\/\//\/}"; done  # ncheck prints //opt for /opt
    echo "$needle: $path"
    [[ -z "${copied[$ino]:-}" ]] || continue
    copied[$ino]=1
    dump_file "$path" "$WORK/mentions/$ino-${path##*/}"
  done < <(dfs "ncheck $inodes")
done
echo
while IFS= read -r -d '' f; do
  mention_context "$f" "mentions:${f##*/}"
done < <(find "$WORK/mentions" -type f -print0 | sort -z)

if [[ -s "$WORK/unsquashfs-warnings.txt" ]]; then
  echo "=== unsquashfs warnings (device nodes and owners can't be created without root) ==="
  head -n 50 "$WORK/unsquashfs-warnings.txt"
  echo
fi

rm -f -- "$ROOTFS"
echo "DONE. Report: $REPORT"
echo "Akai's files are in $WORK. They are proprietary: never commit or share them."
