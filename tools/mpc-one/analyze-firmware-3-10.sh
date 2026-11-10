#!/usr/bin/env bash
# MPFree: one-command, HOST-ONLY 3.10.0 Gen1 firmware inspection and Ghidra prep.
# No MPC writes, no image patching/repacking, no flashing.
set -Eeuo pipefail
export LC_ALL=C

FW="/home/jesse/mpfree-work/akai-update-files/MPC-3.10.0-Gen1-update.img"
OUT="/home/jesse/mpfree-work/firmware-analysis/MPC-3.10.0-Gen1-update"
REPO="/home/jesse/mpfree-work/tools/MPC-LiveXplore"
ROOTFS="$OUT/rootfs.ext4"
EXTRACTED="$OUT/extracted-3.10"
BASE="$OUT/extracted-3.6"
REPORTS="$OUT/reports"
LOG="$OUT/run.log"
mkdir -p "$OUT" "$EXTRACTED" "$BASE" "$REPORTS"
exec > >(tee -a "$LOG") 2>&1

die() { printf '\nERROR: %s\n' "$*" >&2; exit 1; }
section() { printf '\n===== %s =====\n' "$*"; }
trap 'printf "\nFAILED at line %s. See %s\n" "$LINENO" "$LOG" >&2' ERR

[[ -r "$FW" && -f "$FW" ]] || die "Firmware not found: $FW"
section 'Dependencies'
need=()
for name in git file readelf strings debugfs unsquashfs sha256sum od xz; do
  command -v "$name" >/dev/null 2>&1 || need+=("$name")
done
if (( ${#need[@]} )); then
  echo "Installing missing workstation tools: ${need[*]}"
  command -v apt-get >/dev/null || die 'Install the missing dependencies with your package manager.'
  sudo apt-get update
  sudo apt-get install -y git file binutils e2fsprogs squashfs-tools xz-utils coreutils
fi
for name in git file readelf strings debugfs unsquashfs sha256sum od xz; do
  command -v "$name" >/dev/null || die "Missing prerequisite: $name"
done

section 'Verify original firmware, without modifying it'
MAGIC=$(od -An -tx1 -N4 "$FW" | tr -d '[:space:]')
echo "File: $FW"
file "$FW"
stat -c 'Size: %s bytes' "$FW"
sha256sum "$FW" | tee "$REPORTS/original-image.sha256"
od -Ax -tx1 -N128 "$FW" | tee "$REPORTS/container-header.hex"
echo "Header magic: $MAGIC"
if [[ "$MAGIC" != 415a3031 ]]; then
  die 'Not an AZ01 Gen1 firmware image. Refusing to apply the AZ01 extractor.'
fi
dd if="$FW" bs=1048576 count=1 status=none |
  strings -a -n 5 | head -n 120 > "$REPORTS/container-first-megabyte-strings.txt" || true

section 'Get the upstream MPC image extractor'
if [[ -n "${MPCIMG2:-}" ]]; then
  TOOL="$MPCIMG2"
else
  TOOL="$REPO/imgmaker/bin/mpcimg2"
  if [[ ! -f "$TOOL" ]]; then
    mkdir -p "$(dirname "$REPO")"
    [[ ! -e "$REPO" ]] || die "Missing $TOOL, but $REPO exists. Supply MPCIMG2=/path/to/mpcimg2."
    git clone --depth 1 https://github.com/TheKikGen/MPC-LiveXplore.git "$REPO"
  fi
  [[ ! -d "$REPO/.git" ]] || git -C "$REPO" rev-parse HEAD | tee "$REPORTS/mpc-livexplore-commit.txt"
fi
[[ -f "$TOOL" ]] || die "Extractor absent: $TOOL"
file "$TOOL"
echo "Extractor SHA-256:"
sha256sum "$TOOL" | tee "$REPORTS/mpcimg2.sha256"
[[ $(uname -m) == x86_64 ]] || echo 'WARNING: upstream prebuilt extractor may need a different architecture.'
chmod u+x "$TOOL"

section 'Extract the compressed rootfs into the host analysis directory'
if [[ -s "$ROOTFS" ]]; then
  echo "Reusing existing extracted rootfs: $ROOTFS"
else
  TMP_ROOTFS="$OUT/rootfs.ext4.incomplete"
  rm -f -- "$TMP_ROOTFS"
  # -r is TheKikGen mpcimg2 rootfs extraction mode.
  "$TOOL" -r "$FW" "$TMP_ROOTFS" || die 'mpcimg2 extraction failed; original image unchanged.'
  [[ -s "$TMP_ROOTFS" ]] || die 'Extractor did not produce a filesystem.'
  mv -- "$TMP_ROOTFS" "$ROOTFS"
fi
file "$ROOTFS"
sha256sum "$ROOTFS" | tee "$REPORTS/rootfs.sha256"
EXT_MAGIC=$(od -An -tx1 -j1080 -N2 "$ROOTFS" | tr -d '[:space:]')
[[ "$EXT_MAGIC" == 53ef ]] || die "Expected ext4 superblock magic 53ef, got $EXT_MAGIC. No filesystem reads attempted."

section 'Read ext4 directly with debugfs (no root mount, no journal replay)'
for dir in / /usr/bin /usr/lib/systemd/system /lib/systemd/system /boot /opt /etc /etc/ssh; do
  echo "--- $dir"
  debugfs -R "ls -p $dir" "$ROOTFS" 2>/dev/null | head -n 100 || true
done > "$REPORTS/rootfs-directory-inventory.txt"

extract_file() {
  local rel="$1" dest="$2" stat_info
  stat_info=$(debugfs -R "stat /$rel" "$ROOTFS" 2>/dev/null || true)
  [[ "$stat_info" == *"Type: regular"* ]] || return 0
  mkdir -p "$(dirname "$dest")"
  debugfs -R "dump /$rel $dest" "$ROOTFS" >/dev/null 2>&1 || true
  [[ -s "$dest" ]] || { rm -f -- "$dest"; return 0; }
  echo "Extracted $rel"
}
FILES=(
  usr/bin/az01-script-runner
  usr/bin/az01-launch-MPC
  usr/bin/az01-update
  usr/bin/MPC
  usr/bin/ssh
  usr/sbin/sshd
  boot/boot.scr.uimg
  opt/ramfs-update.squashfs
  usr/lib/systemd/system/acvs.service
  usr/lib/systemd/system/sshd.service
  usr/lib/systemd/system/az01-script-runner.service
  usr/lib/systemd/system/az01-update.service
  lib/systemd/system/acvs.service
  lib/systemd/system/sshd.service
  etc/ssh/sshd_config
  etc/os-release
  usr/lib/os-release
  etc/fstab
)
for rel in "${FILES[@]}"; do extract_file "$rel" "$EXTRACTED/$rel"; done

section 'Unpack update RAM filesystem if present'
SQ="$EXTRACTED/opt/ramfs-update.squashfs"
if [[ -s "$SQ" ]]; then
  unsquashfs -ll "$SQ" > "$REPORTS/update-squashfs-contents.txt" 2>&1 || true
  if [[ ! -e "$OUT/update-ramfs-extracted/.mpfree-extraction-done" ]]; then
    mkdir -p "$OUT/update-ramfs-extracted"
    if unsquashfs -f -d "$OUT/update-ramfs-extracted" "$SQ"; then
      touch "$OUT/update-ramfs-extracted/.mpfree-extraction-done"
    fi
  fi
fi

section 'Collect optional, strictly read-only 3.6.0 baseline over SSH'
KEY="/home/jesse/.ssh/mpc_one_autorun_ed25519"
if [[ -r "$KEY" ]] && timeout 8 ssh -n -o BatchMode=yes -o IdentitiesOnly=yes \
  -o StrictHostKeyChecking=yes -o ConnectTimeout=5 \
  -i "$KEY" -p 2222 root@10.42.0.141 true >/dev/null 2>&1; then
  for binary in az01-script-runner az01-launch-MPC az01-update; do
    target="$BASE/$binary"
    if [[ ! -s "$target" ]]; then
      ssh -n -o BatchMode=yes -o IdentitiesOnly=yes \
        -o StrictHostKeyChecking=yes -o ConnectTimeout=8 \
        -i "$KEY" -p 2222 root@10.42.0.141 \
        "cat /usr/bin/$binary" > "$target" || rm -f -- "$target"
      [[ -s "$target" ]] || rm -f -- "$target"
    fi
  done
else
  echo '3.6 reference SSH is unavailable/noninteractive: skipping baseline download.'
  echo 'You can rerun after ssh-add; no host-key checks are bypassed.'
fi

section 'Build offline ELF, strings, and version-difference reports'
PATTERN='autorun|script|update|fastboot|verify|signature|checksum|sha1|sha256|openssl|systemd|execve|execv|spawn|usb|mount|boot|firmware|acvs|sshd|rootfs|dorecovery'
for ver in 3.10 3.6; do
  if [[ "$ver" == 3.10 ]]; then src="$EXTRACTED/usr/bin"; else src="$BASE"; fi
  [[ -d "$src" ]] || continue
  for bin in az01-script-runner az01-launch-MPC az01-update; do
    f="$src/$bin"
    [[ -s "$f" ]] || continue
    { echo "Source: $f"; file "$f"; sha256sum "$f"
      readelf -h "$f" 2>&1 || true
      readelf -d "$f" 2>&1 || true
      readelf -Ws "$f" 2>&1 | grep -Ei "$PATTERN" | head -n 150 || true
    } > "$REPORTS/$ver-$bin.elf.txt"
    strings -a -n 5 "$f" | grep -Ei "$PATTERN" | sort -u \
      > "$REPORTS/$ver-$bin.interesting-strings.txt" || true
  done
done
for bin in az01-script-runner az01-launch-MPC az01-update; do
  old="$BASE/$bin"
  new="$EXTRACTED/usr/bin/$bin"
  if [[ -s "$old" && -s "$new" ]]; then
    echo "Comparison: $bin"
    sha256sum "$old" "$new"
    if cmp -s "$old" "$new"; then
      echo "Identical binary"
    else
      echo "Binary differs"
    fi
    comm -3 "$REPORTS/3.6-$bin.interesting-strings.txt" \
      "$REPORTS/3.10-$bin.interesting-strings.txt" \
      > "$REPORTS/diff-$bin-strings.txt" || true
  fi
done

section 'Locate Ghidra and prepare automatic headless analysis'
if [[ -n "${GHIDRA_HEADLESS:-}" ]]; then
  HEADLESS="$GHIDRA_HEADLESS"
elif command -v analyzeHeadless >/dev/null 2>&1; then
  HEADLESS=$(command -v analyzeHeadless)
else
  HEADLESS=$(find /opt /usr/local "$HOME/Downloads" "$HOME/opt" "$HOME/apps" "$HOME/tools" \
    -maxdepth 6 -type f -name analyzeHeadless -print -quit 2>/dev/null || true)
fi
if [[ -z "$HEADLESS" || ! -f "$HEADLESS" ]]; then
  echo 'No Ghidra analyzeHeadless found. Extraction and all ELF/string reports are ready.'
  echo 'Set GHIDRA_HEADLESS=/path/to/ghidra/support/analyzeHeadless and rerun.'
else
  chmod u+x "$HEADLESS" || true
  PROJECTS="$OUT/ghidra-projects"
  mkdir -p "$PROJECTS" "$OUT/ghidra-logs"
  for ver in 3.6 3.10; do
    if [[ "$ver" == 3.10 ]]; then src="$EXTRACTED/usr/bin"; else src="$BASE"; fi
    for bin in az01-script-runner az01-launch-MPC az01-update; do
      f="$src/$bin"
      [[ -s "$f" ]] || continue
      echo "Ghidra import/analyze: $ver / $bin"
      "$HEADLESS" "$PROJECTS" "MPFree_3_10_Analysis/v$ver" \
        -import "$f" -overwrite -analysisTimeoutPerFile 240 \
        > "$OUT/ghidra-logs/$ver-$bin.log" 2>&1 ||
        echo "WARNING: Ghidra reported an issue for $ver / $bin. Review its log."
    done
  done
  echo "Open Ghidra project: $PROJECTS/MPFree_3_10_Analysis.gpr"
fi

section 'Final summary'
{
  echo "Original firmware: $FW"
  echo "Container magic: $MAGIC"
  echo "Extracted rootfs: $ROOTFS"
  echo 'Extracted analysis files:'
  find "$EXTRACTED" -type f -printf '  %P (%s bytes)\n' | sort
  echo 'Version comparison files:'
  find "$REPORTS" -maxdepth 1 -type f -printf '  %f\n' | sort
  echo "Ghidra project (if Ghidra was available): $OUT/ghidra-projects/MPFree_3_10_Analysis.gpr"
  echo 'No writes were made to the MPC, its partitions, or the official firmware file.'
  echo 'This script does not determine whether firmware flashing or alternative boot is safe.'
} | tee "$OUT/SUMMARY.txt"
echo "Complete. Report: $OUT/SUMMARY.txt"
