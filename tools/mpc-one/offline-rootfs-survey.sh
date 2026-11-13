#!/usr/bin/env bash
# OFFLINE, workstation only: survey an MPC root filesystem image read-only,
# to see what a firmware version would mount, start and flash before it is
# ever run on the MPC.
#
# Two sources:
#   --image FILE    an ext2/3/4 rootfs image, such as the rootfs unpacked from
#                   an Akai update by offline-update-extract.py. Default: the
#                   single *rootfs*.img in the newest ~/mpfree-work/akai-unpacked-<UTC>-*/
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

for tool in debugfs file strings sha256sum dd awk sort realpath python3 cmp cut; do
  command -v "$tool" >/dev/null 2>&1 ||
    die "missing '$tool'. Install e2fsprogs, file, binutils, diffutils and python3 with your package manager."
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
  folders=("$work_abs"/akai-unpacked-[0-9]*/)
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
  trap 'rm -f -- "$ROOTFS"' EXIT  # the copy is 500 MiB; remove it on every exit, also on errors
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
SAFE_PATH='^[A-Za-z0-9@._+/-]+$'
spec_of() {  # how to name $1 to debugfs: the path, or <inode> for a name debugfs would split
  local path="$1" dir name ino
  if [[ "$path" =~ $SAFE_PATH ]]; then printf '%s' "$path"; return 0; fi
  dir="${path%/*}" name="${path##*/}"
  [[ -n "$dir" && "$dir" =~ $SAFE_PATH ]] || return 1
  # The name goes through the environment: awk -v would expand escapes such as \x2d.
  ino="$(dfs "ls -p $dir" | NAME="$name" awk -F/ 'NF >= 7 && $6 == ENVIRON["NAME"] { print $2; exit }')"
  [[ "$ino" =~ ^[0-9]+$ ]] || return 1
  printf '<%s>' "$ino"
}
rootfs_type() {
  local spec
  spec="$(spec_of "$1")" || return 0
  dfs "stat $spec" | sed -n 's/^Inode: [0-9]*  *Type: \([a-z]*\).*/\1/p' | head -n 1
}
link_target() {  # a symlink's target: fast links are stored in the inode, slow ones in a block
  local target spec
  spec="$(spec_of "$1")" || { printf '(target unreadable)'; return 0; }
  target="$(dfs "stat $spec" | sed -n 's/^Fast link dest: "\(.*\)"$/\1/p' | head -n 1)"
  if [[ -z "$target" ]]; then
    rm -f -- "$WORK/link.tmp"
    dfs "dump $spec $WORK/link.tmp"
    target="$(head -c 4096 -- "$WORK/link.tmp" 2>/dev/null | tr -d '\0')"
    rm -f -- "$WORK/link.tmp"
  fi
  printf '%s' "${target:-(target unreadable)}"
}
names_in() {  # $1 = directory; prints "type name" per entry, skipping . and ..
  dfs "ls -p $1" | awk -F/ 'NF >= 7 && $6 != "." && $6 != ".." {
    t = substr($3, 1, 2); kind = (t == "04") ? "dir" : (t == "12") ? "link" : "file"; print kind, $6 }'
}
STRINGS_PATTERN='(/dev/|/sys/|/proc/|/etc/|/usr/|/opt/|/data|/media|mmc|boot|spl|sig|key|flash|dfu|firmware|fw|update|upgrade|partition|i2c|loadpin|kexec|reboot|systemctl|exec|[.]sh|[.]bin|[.]hex|erase|write|verify|version)'
show_file() {  # $1 = path in the image; prints a small text file, or a binary's sha256 and relevant strings
  local path="$1" tmp="$WORK/show.tmp" type spec kind size
  type="$(rootfs_type "$path")"
  case "$type" in
    regular) ;;
    "") echo "($path: not in the image)"; return 0 ;;
    symlink) echo "--- $path -> $(link_target "$path")"; return 0 ;;
    *) echo "($path: $type, not a regular file)"; return 0 ;;
  esac
  spec="$(spec_of "$path")"
  rm -f -- "$tmp"
  dfs "dump $spec $tmp"
  kind="$(file -b -- "$tmp")" size="$(stat -c %s -- "$tmp")"
  if [[ "$kind" == *text* ]] && (( size <= 65536 )); then
    echo "--- $path"; cat -- "$tmp"
  else
    echo "--- $path: $kind ($size bytes), sha256 $(sha256sum -- "$tmp" | awk '{print $1}')"
    if [[ "$kind" == ELF* ]]; then
      strings -a -n 6 -- "$tmp" | grep -E -i -- "$STRINGS_PATTERN" | head -n 200 | sed 's/^/    /'
    fi
  fi
  rm -f -- "$tmp"
}
unique_dirs() {  # the given folders that exist, each real folder once (/lib is often a link to /usr/lib)
  local d ino seen=" "
  for d in "$@"; do
    [[ "$(rootfs_type "$d")" == directory ]] || continue
    ino="$(dfs "stat $d" | sed -n 's/^Inode: \([0-9][0-9]*\) .*/\1/p' | head -n 1)"
    [[ -n "$ino" && "$seen" != *" $ino "* ]] || continue
    seen+="$ino "
    printf '%s\n' "$d"
  done
}
file_sha() {  # sha256 of a regular file in the image
  local tmp="$WORK/sha.tmp" spec
  spec="$(spec_of "$1")" || return 0
  rm -f -- "$tmp"
  dfs "dump $spec $tmp"
  [[ -f "$tmp" ]] && sha256sum -- "$tmp" | awk '{print $1}'
  rm -f -- "$tmp"
}

echo "=== 1. Identity ==="
for f in /etc/issue /etc/os-release /etc/version /etc/timestamp; do show_file "$f"; done
echo "--- ls -l /boot"
dfs "ls -l /boot"
echo "--- sha256 of the files in /boot"
while read -r kind name; do
  [[ "$kind" == file ]] && echo "$(file_sha "/boot/$name") $name"
done < <(names_in /boot | sort -k2)
while read -r d; do
  echo "--- kernel module folders in $d"; names_in "$d"
done < <(unique_dirs /usr/lib/modules /lib/modules)
echo

echo "=== 2. What it mounts ==="
show_file /etc/fstab
mapfile -t UNIT_DIRS < <(unique_dirs /usr/lib/systemd/system /lib/systemd/system /etc/systemd/system)
for d in "${UNIT_DIRS[@]}"; do
  while read -r kind name; do
    [[ "$kind" == file && "$name" == *.mount ]] && show_file "$d/$name"
  done < <(names_in "$d")
done
echo

echo "=== 3. Units ==="
UNIT_PATTERN='(update|upd|flash|fw|dfu|firmware|xmos|touch|ilitek|resize|grow|format|factory|repart|overlay|recover|loader|mmc|emmc|partition|data|content|^az0|^acvs|^media-|^storage|^secure|loadpin|sig|webserver)'
for d in "${UNIT_DIRS[@]}"; do
  echo "--- all entries in $d"
  names_in "$d" | sort -k2
done
for d in "${UNIT_DIRS[@]}"; do
  while read -r kind name; do
    [[ "$kind" == dir ]] && continue
    [[ "$name" =~ $UNIT_PATTERN ]] && show_file "$d/$name"
  done < <(names_in "$d" | sort -k2)
done
echo "--- started at boot (.wants and .requires folders)"
for d in "${UNIT_DIRS[@]}"; do
  while read -r kind name; do
    [[ "$kind" == dir && "$name" =~ ^[A-Za-z0-9@._+-]+\.(wants|requires)$ ]] || continue
    names_in "$d/$name" | awk -v d="$d/$name" '{ print d ": " $2 }' | sort
  done < <(names_in "$d" | sort -k2)
done
echo

echo "=== 4. Programs whose names suggest flashing or storage work ==="
PROG_PATTERN='(fw|dfu|flash|upd|firmware|xmos|ilitek|boot|loader|mmc|emmc|prog|recover|factory|sshd|dropbear)'
mapfile -t PROG_DIRS < <(unique_dirs /usr/bin /usr/sbin /usr/libexec /bin /sbin)
for d in "${PROG_DIRS[@]}"; do
  names_in "$d" | awk -v d="$d" -v re="$PROG_PATTERN" '$2 ~ re { print d "/" $2 " (" $1 ")" }'
done
echo

echo "=== 5. Firmware blobs ==="
while read -r d; do
  echo "--- $d"
  while read -r kind name; do
    if [[ "$kind" == file ]]; then
      echo "file $name sha256 $(file_sha "$d/$name")"
    else
      echo "$kind $name"
    fi
    if [[ "$kind" == dir && "$name" =~ ^[A-Za-z0-9@._+-]+$ ]]; then
      while read -r kind2 name2; do
        if [[ "$kind2" == file ]]; then
          echo "    file $name2 sha256 $(file_sha "$d/$name/$name2")"
        else
          echo "    $kind2 $name2"
        fi
      done < <(names_in "$d/$name" | sort -k2)
    fi
  done < <(names_in "$d" | sort -k2)
done < <(unique_dirs /usr/lib/firmware /lib/firmware)
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
mentions() {  # each argument is a needle; prints "needle: path" for every file whose data holds it
  local needle blocks owners unowned inodes ino path
  for needle in "$@"; do
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
}
mentions ramfs-update erase-data-partition fastboot mmcblk0boot /dev/mmcblk0 xmosdfu touch-fw-upd dfu-util \
         update-bootloader flash-bootloader uboot-spl reboot-argument=
echo

echo "=== 7. Akai's own programs, scripts and firmware updaters ==="
AKAI_PATTERN='^(az0|acvs|touch-fw|xmosdfu|dfu-util|fw_setenv|fw_printenv)'
for d in "${PROG_DIRS[@]}"; do
  while read -r kind name; do
    [[ "$kind" != dir && "$name" =~ $AKAI_PATTERN ]] && show_file "$d/$name"
  done < <(names_in "$d" | sort -k2)
done
for d in /usr/lib/systemd/system-preset /etc/az01; do
  [[ "$(rootfs_type "$d")" == directory ]] || continue
  while read -r kind name; do
    [[ "$kind" == file ]] && show_file "$d/$name"
  done < <(names_in "$d" | sort -k2)
done
if [[ "$(rootfs_type /usr/share/Akai)" == directory ]]; then
  while read -r kind name; do
    [[ "$kind" == dir && "$name" =~ ^[A-Za-z0-9@._+-]+$ ]] || continue
    fw="/usr/share/Akai/$name/Firmware"
    [[ "$(rootfs_type "$fw")" == directory ]] || continue
    while read -r kind2 name2; do
      [[ "$kind2" == file ]] || { echo "($fw/$name2: $kind2)"; continue; }
      show_file "$fw/$name2"
    done < <(names_in "$fw" | sort -k2)
  done < <(names_in /usr/share/Akai | sort -k2)
fi
echo

echo "=== 8. Kernel and device trees, for a kexec boot ==="
# Copies /boot out of the image into ~/mpfree-work/boot-files/<kernel release>/,
# so later steps can find the kernel and device trees by version. An existing
# copy is checked against the image and never overwritten.
BOARD_COMPATIBLE="inmusic,acva"
MOD_PATTERN='(usb-storage|/uas|sd_mod|scsi_mod|/ext4|jbd2|mbcache|dwc2|ehci|ohci|xhci|phy-rockchip-usb|usbcore|usb-common|/vfat|/fat\.|overlay|dw_mmc|mmc_block|mmc_core|squashfs|dm-verity|dm-mod|/loop|stmmac|dwmac|configs)'
kernels=()
while read -r kind name; do
  [[ "$kind" == file && "$name" == zImage-* ]] && kernels+=("$name")
done < <(names_in /boot | sort -k2)
release=""
if (( ${#kernels[@]} == 1 )) && [[ "${kernels[0]#zImage-}" =~ ^[A-Za-z0-9._+-]+$ ]]; then
  release="${kernels[0]#zImage-}"
else
  echo "(expected one /boot/zImage-<release> file, found ${#kernels[@]}; skipping this section)"
fi
if [[ -n "$release" ]]; then
  BOOT_COPY="$work_abs/boot-files/$release"
  echo "kernel release: $release"
  fresh="$WORK/boot-files.tmp"
  mkdir -p -- "$fresh"
  while read -r kind name; do
    [[ "$kind" == file && "$name" =~ ^[A-Za-z0-9._+-]+$ ]] || continue
    dfs "dump /boot/$name $fresh/$name"
  done < <(names_in /boot | sort -k2)
  (cd -- "$fresh" && sha256sum -- * > SHA256SUMS)
  if [[ -d "$BOOT_COPY" ]]; then
    if cmp -s -- "$fresh/SHA256SUMS" "$BOOT_COPY/SHA256SUMS"; then
      echo "boot files: $BOOT_COPY (already there, same hashes)"
    else
      echo "WARNING: $BOOT_COPY exists with different files; left unchanged. The checks below use this image's copy."
      BOOT_COPY="$fresh"
    fi
  else
    mkdir -p -- "$work_abs/boot-files"
    cp -a -- "$fresh" "$BOOT_COPY"
    echo "boot files copied to $BOOT_COPY"
  fi
  modules_dir=""
  while read -r d; do
    [[ "$(rootfs_type "$d/$release")" == directory ]] && { modules_dir="$d/$release"; break; }
  done < <(unique_dirs /usr/lib/modules /lib/modules)
  for list in modules.builtin modules.dep; do
    if [[ -z "$modules_dir" || "$(rootfs_type "$modules_dir/$list")" != regular ]]; then
      echo "--- $list: not in the image"
      continue
    fi
    if [[ "$list" == modules.builtin ]]; then
      echo "--- built into the kernel ($modules_dir/$list), USB storage, filesystems, MMC and Ethernet"
    else
      echo "--- loadable modules ($modules_dir/$list), same names"
    fi
    rm -f -- "$WORK/list.tmp"
    dfs "dump $modules_dir/$list $WORK/list.tmp"
    { cut -d: -f1 -- "$WORK/list.tmp" | grep -E -- "$MOD_PATTERN" || echo "(none)"; } | sed 's/^/    /'
    echo "    ($(wc -l < "$WORK/list.tmp") entries in all)"
    rm -f -- "$WORK/list.tmp"
  done
  echo "--- kernel config embedded in zImage-$release (symbols that matter for a kexec boot)"
  python3 -I "$SCRIPT_DIR/offline-kernel-config.py" "$BOOT_COPY/zImage-$release" --select ||
    echo "(could not read the embedded config)"
  shopt -s nullglob
  dtbs=("$BOOT_COPY"/*.dtb)
  shopt -u nullglob
  if (( ${#dtbs[@]} )); then
    echo "--- device trees: board of each file"
    python3 -I "$SCRIPT_DIR/offline-dtb.py" boards "${dtbs[@]}"
    # Shortest name first, so the plain board file is compared with its variants.
    mapfile -t ours < <(python3 -I "$SCRIPT_DIR/offline-dtb.py" match "$BOARD_COMPATIBLE" "${dtbs[@]}" |
                        awk '{ print length($0), $0 }' | sort -k1,1n -k2 | cut -d' ' -f2-)
    echo "--- device trees for this board ($BOARD_COMPATIBLE): ${#ours[@]}"
    if (( ${#ours[@]} )); then
      python3 -I "$SCRIPT_DIR/offline-dtb.py" summary "${ours[@]}"
      for other in "${ours[@]:1}"; do
        python3 -I "$SCRIPT_DIR/offline-dtb.py" diff "${ours[0]}" "$other" --values
      done
    fi
  fi
  rm -rf -- "$fresh"
fi
echo

echo "=== 9. Developer mode: SSH and the script runner ==="
# MPC OS 3.10.1 adds a developer mode: a shell with the app's rights, an SSH
# server and a script runner that runs autorun.sh from any drive (Akai's
# release notes). This shows the pieces an image ships for it, so the stick
# test and the tools can be planned before the version runs anywhere.
DEV_PATTERN='(ssh|dropbear|script|runner|autorun|devel|shell|terminal|remote)'
for d in "${UNIT_DIRS[@]}"; do
  while read -r kind name; do
    [[ "$kind" == dir ]] && continue
    [[ "$name" =~ $DEV_PATTERN ]] && show_file "$d/$name"
  done < <(names_in "$d" | sort -k2)
done
for f in /etc/ssh/sshd_config /etc/default/dropbear /etc/pam.d/sshd; do show_file "$f"; done
if [[ "$(rootfs_type /etc/ssh/sshd_config.d)" == directory ]]; then
  while read -r kind name; do
    [[ "$kind" != dir ]] && show_file "/etc/ssh/sshd_config.d/$name"
  done < <(names_in /etc/ssh/sshd_config.d | sort -k2)
fi
echo "--- root's password field in /etc/shadow (what kind it is, never its value)"
if [[ "$(rootfs_type /etc/shadow)" == regular ]]; then
  rm -f -- "$WORK/shadow.tmp"
  dfs "dump /etc/shadow $WORK/shadow.tmp"
  awk -F: '$1 == "root" { found = 1
    if ($2 == "") print "root: empty (no password asked)"
    else if ($2 ~ /^[!*]/) print "root: locked for password logins (starts with " substr($2, 1, 1) ")"
    else print "root: a password hash is set" }
    END { if (!found) print "root: no line" }' "$WORK/shadow.tmp"
  rm -f -- "$WORK/shadow.tmp"
else
  echo "(/etc/shadow: not in the image)"
fi
echo "--- files that mention developer mode, autorun.sh or SSH login options"
mentions autorun.sh script-runner "Developer mode" "developer mode" dropbear PermitRootLogin \
         PermitEmptyPasswords
echo

echo "=== 10. Peripheral firmware files (sha256; offline-firmware-compare.py compares two reports) ==="
# One "fw <sha256> <path>" line per file that Akai's updaters or app could flash
# into a chip outside the eMMC: everything under the firmware folders (3 levels)
# and in /usr/share/Akai/*/Firmware. A name debugfs can't take as a path, or a
# folder too deep, gets an "fw-skipped" line instead, so a comparison still
# shows it.
FW_NAME='^[A-Za-z0-9@._+-]+$'
fw_walk() {  # $1 = folder, $2 = levels left
  local kind name
  while read -r kind name; do
    if [[ ! "$name" =~ $FW_NAME ]]; then echo "fw-skipped $1/ (a $kind whose name has spaces or other characters)"; continue; fi
    case "$kind" in
      file) echo "fw $(file_sha "$1/$name") $1/$name" ;;
      dir) if (( $2 > 1 )); then fw_walk "$1/$name" $(( $2 - 1 )); else echo "fw-skipped $1/$name/ (too deep)"; fi ;;
      link) echo "fw-link $1/$name -> $(link_target "$1/$name")" ;;
    esac
  done < <(names_in "$1" | sort -k2)
}
while read -r d; do fw_walk "$d" 3; done < <(unique_dirs /usr/lib/firmware /lib/firmware)
if [[ "$(rootfs_type /usr/share/Akai)" == directory ]]; then
  while read -r kind name; do
    [[ "$kind" == dir && "$name" =~ $FW_NAME ]] || continue
    [[ "$(rootfs_type "/usr/share/Akai/$name/Firmware")" == directory ]] && fw_walk "/usr/share/Akai/$name/Firmware" 2
  done < <(names_in /usr/share/Akai | sort -k2)
fi
echo

echo "DONE. Report: $REPORT"
echo "It quotes Akai's files: keep it private and commit only findings."
