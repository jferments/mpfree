#!/usr/bin/env bash
# MPFree MPC One Gen1 3.10.1 OFFLINE firmware lab (host only).
# No target-device access, no mount, no firmware execution, no flashing.
set -Eeuo pipefail
umask 077
export LC_ALL=C

usage() {
  cat <<'HELP'
Usage: bash tools/mpc-one/setup-firmware-3-10-1-lab.sh [options]

  --image FILE       Firmware image (default: ~/Downloads/MPC-3.10.1-Gen1-update.img)
  --baseline FILE    Optional 3.10.0 comparison image (default below, if present)
  --no-baseline      Skip 3.10.0 image extraction and comparison
  --ghidra           Also import extracted ELF files into a NEW headless Ghidra project
  --no-qemu          Do not prepare the QEMU copy-on-write disk
  --qemu-kernel FILE Show a QEMU 'virt' boot command for this GENERIC ARM32 kernel
  -h, --help         Show this help

Output: ~/mpfree-work/firmware-labs/mpc-gen1-3101-<unique>/
The firmware images and any cloned backup are NEVER modified.
No physical MPC, SSH, USB, root mounts, updater, or untrusted executable runs.
QEMU guest boot is NOT started. Ghidra imports are optional.
HELP
}

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
TOOLS="$ROOT/tools/mpc-one"
IMAGE="$HOME/Downloads/MPC-3.10.1-Gen1-update.img"
BASELINE="$HOME/mpfree-work/akai-update-files/MPC-3.10.0-Gen1-update.img"
USE_BASELINE=1
RUN_GHIDRA=0
USE_QEMU=1
QEMU_KERNEL=""
while (($#)); do
  case "$1" in
    --image|--baseline|--qemu-kernel)
      (($# >= 2)) || { echo "Missing argument for $1" >&2; exit 2; }
      case "$1" in
        --image) IMAGE="$2" ;;
        --baseline) BASELINE="$2" ;;
        --qemu-kernel) QEMU_KERNEL="$2" ;;
      esac
      shift 2 ;;
    --no-baseline) USE_BASELINE=0; shift ;;
    --ghidra) RUN_GHIDRA=1; shift ;;
    --no-qemu) USE_QEMU=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

fail() { printf '\nERROR: %s\n' "$*" >&2; exit 1; }
source_file() {
  local label="$1" value="$2"
  [[ -f "$value" && -r "$value" && ! -L "$value" && ! -b "$value" && ! -c "$value" ]] ||
    fail "$label must be a readable regular file (not a link/device): $value"
  realpath -- "$value"
}
for s in offline-update-image.py offline-update-extract.py offline-rootfs-survey.sh qemu_arm_lab_preflight.py; do
  [[ -f "$TOOLS/$s" ]] || fail "MPFree source checkout lacks $s: $ROOT"
done
for x in python3 git file sha256sum stat od debugfs strings readelf cmp diff df realpath; do
  command -v "$x" >/dev/null 2>&1 || fail "Missing $x. Suggested packages: sudo apt-get install -y python3 git file coreutils e2fsprogs binutils diffutils qemu-utils qemu-system-arm zstd lz4"
done
IMAGE="$(source_file '3.10.1 image' "$IMAGE")"
if ((USE_BASELINE)); then
  if [[ -e "$BASELINE" ]]; then BASELINE="$(source_file '3.10.0 baseline' "$BASELINE")"
  else printf 'INFO: No baseline at %s; continuing without it.\n' "$BASELINE"; USE_BASELINE=0; fi
fi
[[ "$IMAGE" != "$BASELINE" || "$USE_BASELINE" -eq 0 ]] || fail "3.10.1 image and 3.10.0 baseline must be different files"
if [[ -n "$QEMU_KERNEL" ]]; then QEMU_KERNEL="$(source_file 'generic ARM32 QEMU kernel' "$QEMU_KERNEL")"; fi
MAGIC="$(od -An -tx1 -N4 -- "$IMAGE" | tr -d '[:space:]')"
[[ "$MAGIC" == 415a3031 || "$MAGIC" == d00dfeed ]] || fail "Unrecognized update container header ($MAGIC); refusing extraction"
if ((USE_BASELINE)); then
  OLD_MAGIC="$(od -An -tx1 -N4 -- "$BASELINE" | tr -d '[:space:]')"
  [[ "$OLD_MAGIC" == 415a3031 || "$OLD_MAGIC" == d00dfeed ]] || fail "Baseline is not a recognized update container"
fi

mkdir -p "$HOME/mpfree-work/firmware-labs"
LAB="$(mktemp -d "$HOME/mpfree-work/firmware-labs/mpc-gen1-3101-XXXXXXXX")"
mkdir -p "$LAB/reports" "$LAB/extracted" "$LAB/artifacts" "$LAB/ghidra-input"
REPORTS="$LAB/reports"
SESSION="$REPORTS/setup.log"
exec > >(tee -a "$SESSION") 2>&1
printf 'MPFree 3.10.1 host-only firmware lab: %s\n' "$LAB"
printf '3.10.1: %s\nBaseline 3.10.0: %s\n' "$IMAGE" "$([[ $USE_BASELINE == 1 ]] && echo "$BASELINE" || echo '(not available)')"
free_gib="$(df -Pk "$LAB" | awk 'NR==2{printf "%.1f",$4/1048576}')"
printf 'Free space: %s GiB (full extraction may consume several GiB per image).\n' "$free_gib"
[[ "$(df -Pk "$LAB" | awk 'NR==2{print $4}')" -gt 4194304 ]] || fail 'Less than 4 GiB free; stopping before extraction.'
printf 'Git revision: ' ; git -C "$ROOT" rev-parse HEAD || true
printf '\nNever install this image or connect the device during this run.\n'

inventory() {
  local version="$1" source="$2"
  printf '\n===== %s firmware container inventory =====\n' "$version"
  file -- "$source" | tee "$REPORTS/$version-file.txt"
  stat -c 'Bytes: %s' -- "$source" | tee "$REPORTS/$version-size.txt"
  sha256sum -- "$source" | tee "$REPORTS/$version-image.sha256"
  od -Ax -tx1 -N128 -- "$source" > "$REPORTS/$version-header.hex"
  python3 "$TOOLS/offline-update-image.py" "$source" > "$REPORTS/$version-partitions.txt"
  grep -Ei 'PART|rootfs|kernel|u-boot|spl|flash|sha|offset|length|size|compression' "$REPORTS/$version-partitions.txt" | head -n 55 || true
}

extract() {
  local version="$1" source="$2" target="$LAB/artifacts/$1" log="$REPORTS/$1-extraction.log"
  mkdir -p "$target"
  printf '\n===== %s: verified payload extraction (host output ONLY) =====\n' "$version"
  if ! MPFREE_WORK_DIR="$target" python3 "$TOOLS/offline-update-extract.py" "$source" > "$log" 2>&1; then
    tail -n 100 "$log" || true
    fail "$version extraction failed; see $log. No original image was written."
  fi
  tail -n 35 "$log"
  local -a directories=()
  mapfile -d '' -t directories < <(find "$target" -mindepth 1 -maxdepth 1 -type d -name 'akai-unpacked-*' -print0)
  ((${#directories[@]}==1)) || fail "$version extractor output folder ambiguous (found ${#directories[@]})"
  printf '%s\n' "${directories[0]}" > "$REPORTS/$version-payload-directory.txt"
  local -a fs=()
  mapfile -d '' -t fs < <(find "${directories[0]}" -maxdepth 1 -type f \( -iname '*rootfs*.img' -o -iname '*rootfs*.ext4' \) -print0)
  if ((${#fs[@]} == 1)); then
    local magic
    magic="$(od -An -tx1 -j1080 -N2 -- "${fs[0]}" | tr -d '[:space:]')"
    if [[ "$magic" == 53ef ]]; then
      printf '%s\n' "${fs[0]}" > "$REPORTS/$version-rootfs-path.txt"
      sha256sum -- "${fs[0]}" | tee "$REPORTS/$version-rootfs.sha256"
      printf '%s\n' "$version rootfs: ${fs[0]}"
    else
      printf 'WARNING: %s candidate rootfs lacks ext4 superblock (got %s)\n' "$version" "$magic"
    fi
  else
    printf 'WARNING: %s has %d *rootfs* images, cannot choose automatically\n' "$version" "${#fs[@]}"
  fi
}

inventory '3.10.1' "$IMAGE"
if ((USE_BASELINE)); then inventory '3.10.0' "$BASELINE"; fi
extract '3.10.1' "$IMAGE"
if ((USE_BASELINE)); then extract '3.10.0' "$BASELINE"; fi

# Never mount or execute any extracted Akai file. Use debugfs in default read-only mode.
TARGETS=(
  usr/bin/az01-update usr/bin/az01-script-runner usr/bin/az01-launch-MPC
  usr/bin/MPC usr/bin/ssh usr/sbin/sshd
  boot/boot.scr.uimg boot/zImage boot/rk3288-az01.dtb
  opt/ramfs-update.squashfs etc/os-release usr/lib/os-release etc/az01-release
  etc/fstab etc/ssh/sshd_config etc/fw_env.config
  usr/lib/systemd/system/acvs.service usr/lib/systemd/system/sshd.service
  usr/lib/systemd/system/az01-update.service
  usr/lib/systemd/system/az01-script-runner.service
  lib/systemd/system/acvs.service lib/systemd/system/sshd.service
  etc/systemd/system/sshd.service
)

collect_rootfs() {
  local version="$1" fs="$2" dest="$LAB/extracted/$1" log="$REPORTS/$1-debugfs.log" rel info
  mkdir -p "$dest"
  printf '\n===== %s read-only rootfs survey =====\n' "$version"
  if ! MPFREE_WORK_DIR="$LAB/artifacts/$version" bash "$TOOLS/offline-rootfs-survey.sh" --image "$fs" > "$REPORTS/$version-rootfs-survey-console.log" 2>&1; then
    printf 'WARNING: full rootfs survey failed; see %s\n' "$REPORTS/$version-rootfs-survey-console.log"
  else
    tail -n 8 "$REPORTS/$version-rootfs-survey-console.log"
  fi
  printf 'debugfs inventory: %s\n' "$REPORTS/$version-rootfs-dirs.txt"
  for d in / /boot /etc /usr/bin /usr/sbin /usr/lib/systemd/system /etc/systemd/system /opt; do
    printf '\n--- %s ---\n' "$d"
    debugfs -R "ls -p $d" "$fs" 2>/dev/null | head -n 100 || true
  done > "$REPORTS/$version-rootfs-dirs.txt"
  : > "$log"
  for rel in "${TARGETS[@]}"; do
    info="$(debugfs -R "stat /$rel" "$fs" 2>/dev/null || true)"
    [[ "$info" == *'Type: regular'* ]] || continue
    mkdir -p "$dest/$(dirname -- "$rel")"
    if debugfs -R "dump /$rel $dest/$rel" "$fs" >> "$log" 2>&1 && [[ -s "$dest/$rel" ]]; then
      printf '%s\n' "$rel" >> "$REPORTS/$version-extracted-paths.txt"
    else
      rm -f -- "$dest/$rel"
    fi
  done
  if [[ -d "$dest" ]]; then
    find "$dest" -type f -print0 | sort -z | xargs -0 -r sha256sum > "$REPORTS/$version-extracted-hashes.txt"
    for bin in az01-update az01-script-runner az01-launch-MPC MPC; do
      [[ -f "$dest/usr/bin/$bin" ]] || continue
      file -- "$dest/usr/bin/$bin" > "$REPORTS/$version-$bin-elf.txt"
      readelf -h -d -W -- "$dest/usr/bin/$bin" >> "$REPORTS/$version-$bin-elf.txt" 2>&1 || true
      strings -a -n 5 -- "$dest/usr/bin/$bin" | grep -Ei 'kexec|kernel|verify|signature|boot|ssh|autorun|developer|shell|update|recovery|secure|rollback|fastboot|plugin|terminal' | sort -u | head -n 600 \
        > "$REPORTS/$version-$bin-selected-strings.txt" || true
    done
  fi
}

if [[ -f "$REPORTS/3.10.1-rootfs-path.txt" ]]; then
  NEW_FS="$(cat "$REPORTS/3.10.1-rootfs-path.txt")"
  collect_rootfs '3.10.1' "$NEW_FS"
else
  NEW_FS=""
  printf 'WARNING: 3.10.1 rootfs not identified; no filesystem exploration or QEMU preparation.\n'
fi
if ((USE_BASELINE)) && [[ -f "$REPORTS/3.10.0-rootfs-path.txt" ]]; then
  OLD_FS="$(cat "$REPORTS/3.10.0-rootfs-path.txt")"
  collect_rootfs '3.10.0' "$OLD_FS"
fi

# Look for embedded kernel build configs; failure is explicitly UNKNOWN, not disabled.
for version in 3.10.1 3.10.0; do
  if [[ "$version" == 3.10.0 && "$USE_BASELINE" -eq 0 ]]; then continue; fi
  candidates=()
  [[ ! -f "$LAB/extracted/$version/boot/zImage" ]] || candidates+=("$LAB/extracted/$version/boot/zImage")
  if [[ -f "$REPORTS/$version-payload-directory.txt" ]]; then
    payload_dir="$(cat "$REPORTS/$version-payload-directory.txt")"
    while IFS= read -r -d '' candidate; do
      candidates+=("$candidate")
    done < <(find "$payload_dir" -maxdepth 1 -type f \( -iname '*zimage*' -o -iname '*kernel*' \) -print0)
  fi
  if ((${#candidates[@]}==0)); then
    printf '%s: no standalone kernel image identified in this update. Kernel boot settings remain UNKNOWN.\n' "$version" \
      > "$REPORTS/$version-kernel-config.txt"
  else
    : > "$REPORTS/$version-kernel-config.txt"
    for candidate in "${candidates[@]}"; do
      printf '\n--- Candidate: %s ---\n' "$candidate" >> "$REPORTS/$version-kernel-config.txt"
      python3 "$TOOLS/offline-kernel-config.py" "$candidate" --select \
        >> "$REPORTS/$version-kernel-config.txt" 2>&1 || \
        printf 'No embedded config recovered. This does NOT prove CONFIG_KEXEC is disabled.\n' \
          >> "$REPORTS/$version-kernel-config.txt"
    done
  fi
done

if ((USE_BASELINE)); then
  printf '\n===== New vs old comparison =====\n'
  diff -u "$REPORTS/3.10.0-partitions.txt" "$REPORTS/3.10.1-partitions.txt" > "$REPORTS/partition-diff.txt" || true
  {
    printf 'MPFree boot-related file digest comparison (3.10.0 vs 3.10.1)\n'
    for rel in "${TARGETS[@]}"; do
      old="$LAB/extracted/3.10.0/$rel"
      new="$LAB/extracted/3.10.1/$rel"
      if [[ -f "$old" && -f "$new" ]]; then
        if cmp -s -- "$old" "$new"; then state=SAME; else state=CHANGED; fi
      elif [[ -f "$new" ]]; then state=NEW
      elif [[ -f "$old" ]]; then state=REMOVED
      else state=NOT_FOUND; fi
      printf '%-11s %s\n' "$state" "/$rel"
    done
  } | tee "$REPORTS/boot-files-diff.txt"
  for rel in etc/os-release etc/az01-release etc/fstab etc/fw_env.config \
    usr/lib/systemd/system/acvs.service usr/lib/systemd/system/sshd.service \
    usr/lib/systemd/system/az01-script-runner.service etc/ssh/sshd_config; do
    old="$LAB/extracted/3.10.0/$rel"; new="$LAB/extracted/3.10.1/$rel"
    [[ -f "$old" && -f "$new" ]] || continue
    # Textual artifacts may contain hostnames/settings; keep reports PRIVATE.
    diff -u --label "3.10.0/$rel" --label "3.10.1/$rel" "$old" "$new" \
      > "$REPORTS/diff-$(basename "$rel").txt" || true
  done
fi

if ((USE_QEMU)) && [[ -n "$NEW_FS" ]]; then
  printf '\n===== QEMU ARM virt copy-on-write preparation (NO BOOT) =====\n'
  if command -v qemu-img >/dev/null 2>&1; then
    cmd=(python3 "$TOOLS/qemu_arm_lab_preflight.py" --rootfs "$NEW_FS" \
      --workspace "$LAB/qemu" --name mpc-3101 --create-overlay)
    [[ -z "$QEMU_KERNEL" ]] || cmd+=(--kernel "$QEMU_KERNEL")
    if ! "${cmd[@]}" > "$REPORTS/qemu-preflight.json.txt" 2>&1; then
      printf 'WARNING: QEMU disk preparation failed; see %s\n' "$REPORTS/qemu-preflight.json.txt"
    else
      cat "$REPORTS/qemu-preflight.json.txt"
    fi
  else
    printf 'QEMU not installed. Install qemu-utils/qemu-system-arm and rerun; no guest boot attempted.\n' | tee "$REPORTS/qemu-preflight.txt"
  fi
fi

if ((RUN_GHIDRA)); then
  printf '\n===== Optional Ghidra ELF imports (new, isolated project) =====\n'
  HEADLESS="${GHIDRA_HEADLESS:-}"
  if [[ -z "$HEADLESS" ]]; then HEADLESS="$(command -v analyzeHeadless 2>/dev/null || true)"; fi
  if [[ -z "$HEADLESS" && -n "${GHIDRA_HOME:-}" ]]; then HEADLESS="$GHIDRA_HOME/support/analyzeHeadless"; fi
  [[ -n "$HEADLESS" && -x "$HEADLESS" ]] || fail 'Ghidra requested but analyzeHeadless not found; set GHIDRA_HEADLESS or GHIDRA_HOME'
  mkdir -p "$LAB/ghidra"
  for bin in az01-update az01-script-runner az01-launch-MPC MPC; do
    ELF="$LAB/extracted/3.10.1/usr/bin/$bin"
    [[ -s "$ELF" ]] || continue
    if ! "$HEADLESS" "$LAB/ghidra" MPFree_MPC_3_10_1 \
      -import "$ELF" -analysisTimeoutPerFile 600 > "$REPORTS/ghidra-$bin.log" 2>&1; then
      printf 'WARNING: Ghidra import failed for %s; see log.\n' "$bin"
    fi
  done
fi

cat > "$REPORTS/README.txt" <<EOF
MPFree Gen1 firmware 3.10.1 host-only offline lab
New image: $IMAGE
Baseline: $([[ $USE_BASELINE == 1 ]] && echo "$BASELINE" || echo 'none')
Lab: $LAB

Start with boot-files-diff.txt, partition-diff.txt, and 3.10.1-rootfs-survey-console.log.
Extracted payloads and rootfs: artifacts/ (PRIVATE, proprietary; do not git commit).
Selected ELF and configuration files: extracted/ (PRIVATE, proprietary).
QEMU: qemu/mpc-3101.qcow2; RAW backing rootfs is never written.
QEMU runs a generic ARM 'virt' board, NOT RK3288 or its boot ROM.
A guest boot requires a separate generic ARM32 virt-compatible kernel.
Ghidra project (if requested): ghidra/MPFree_MPC_3_10_1.gpr.

No physical MPC/SSH/USB, no mounts, no flashing, no vendor executables run.
The update container's claimed hashes can be checked by extraction,
but proprietary signing-key signature verification is NOT established.

Useful follow-on checks: CONFIG_KEXEC, lockdown, CAP_SYS_BOOT, boot scripts,
firmware signatures, updater, SSH/dev mode, kernel/DTB and bootloader partitions.
Do not infer a safe upgrade from this offline setup alone.
EOF

# A single private ZIP of small reports; never include extracted binaries, rootfs or firmware.
python3 - "$REPORTS" "$LAB/mpc-gen1-3101-reports.zip" <<'PY'
import hashlib, json, pathlib, sys, zipfile
src, dst = map(pathlib.Path, sys.argv[1:])
with zipfile.ZipFile(dst, 'x', zipfile.ZIP_DEFLATED, compresslevel=6) as out:
    rows = []
    for p in sorted(src.rglob('*')):
        if p.is_symlink() or not p.is_file():
            continue
        if p.stat().st_size > 32 * 1024 * 1024:
            continue
        name = p.relative_to(src).as_posix()
        out.write(p, name)
        rows.append({'path':name,'bytes':p.stat().st_size, 'sha256':hashlib.sha256(p.read_bytes()).hexdigest()})
    out.writestr('MANIFEST.json', json.dumps({'private':True,'files':rows},indent=2))
with zipfile.ZipFile(dst) as z:
    assert z.testzip() is None
print('Private report ZIP:', dst)
PY

printf '\nDONE. Read %s\n' "$REPORTS/README.txt"
printf 'LAB DIRECTORY: %s\n' "$LAB"
printf 'PRIVATE REPORT ZIP: %s\n' "$LAB/mpc-gen1-3101-reports.zip"
printf 'The MPC and the original firmware images were not modified.\n'