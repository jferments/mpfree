#!/usr/bin/env bash
# Run from ML-tower: bash tools/ghidra/run_full_p3_analysis.sh
# No MPC connection. Reads validated backup, creates NEW Ghidra project.
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "$0")/../.." && pwd)"
SCRIPT_DIR="$REPO_ROOT/tools/ghidra"
WORKDIR="$HOME/mpfree-work/ghidra"
BACKUP="$HOME/mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img"
if [[ -n "$MPFREE_EMMC_BACKUP" ]]; then BACKUP="$MPFREE_EMMC_BACKUP"; fi
PAYLOAD="$WORKDIR/p3-uimage-payload.bin"
SHA='b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'

find_headless() {
    if command -v analyzeHeadless >/dev/null 2>&1; then
        command -v analyzeHeadless
        return
    fi
    if [[ -n "$GHIDRA_HOME" && -f "$GHIDRA_HOME/support/analyzeHeadless" ]]; then
        printf '%s\n' "$GHIDRA_HOME/support/analyzeHeadless"
        return
    fi
    local item
    shopt -s nullglob
    for item in \
        /opt/ghidra*/support/analyzeHeadless \
        /usr/local/ghidra*/support/analyzeHeadless \
        /usr/share/ghidra/support/analyzeHeadless \
        "$HOME"/ghidra*/support/analyzeHeadless \
        "$HOME"/Downloads/ghidra*/support/analyzeHeadless \
        "$HOME"/tools/ghidra*/support/analyzeHeadless \
        "$HOME"/apps/ghidra*/support/analyzeHeadless; do
        if [[ -f "$item" ]]; then
            printf '%s\n' "$item"
            return
        fi
    done
    return 1
}

[[ -r "$BACKUP" ]] || { echo "Missing eMMC backup: $BACKUP" >&2; exit 1; }
[[ -f "$SCRIPT_DIR/MPFreeFinishBootChain.py" ]] || {
    echo "Missing Ghidra script; git pull --ff-only origin main first" >&2
    exit 1
}
GHIDRA="$(find_headless)" || {
    echo "Could not locate analyzeHeadless." >&2
    echo "Set GHIDRA_HOME=/path/to/ghidra installation and retry." >&2
    exit 1
}

mkdir -p "$WORKDIR" "$WORKDIR/projects" "$HOME/mpfree-work"

echo "Extracting only p3 payload from the verified eMMC backup..."
python3 - "$BACKUP" "$PAYLOAD" "$SHA" <<'PY'
import hashlib
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2])
expected_sha = sys.argv[3]
start = 256 * 512
size = 363649

with source.open("rb") as handle:
    handle.seek(start)
    data = handle.read(64 + size)

if len(data) != 64 + size or data[:4] != bytes.fromhex("27051956"):
    raise SystemExit("STOP: p3 U-Boot header/length not recognized")

payload = data[64:]
actual = hashlib.sha256(payload).hexdigest()
if actual != expected_sha:
    raise SystemExit("STOP: p3 SHA256 mismatch: %s" % actual)

target.write_bytes(payload)
print("Validated p3 SHA256:", actual)
print("Payload:", target, "bytes:", len(payload))
PY

PROJECT_NAME="MPFree_P3_Uboot_$(date +%Y%m%dT%H%M%S)_$$"
LOG="$WORKDIR/$PROJECT_NAME.log"

echo
echo "Ghidra headless executable: $GHIDRA"
echo "New, separately saved project: $WORKDIR/projects/$PROJECT_NAME"
echo "Import base: 0x00100000 (passed to raw BinaryLoader as 100000)"
echo "Prior base-zero Ghidra program will not be modified."
echo

# Ghidra BinaryLoader -loader-baseAddr intentionally has NO 0x prefix.
# Source: official analyzeHeadlessREADME.md.
"$GHIDRA" \
    "$WORKDIR/projects" "$PROJECT_NAME" \
    -import "$PAYLOAD" \
    -loader BinaryLoader \
    -loader-baseAddr 100000 \
    -processor ARM:LE:32:v7 \
    -cspec default \
    -scriptPath "$SCRIPT_DIR" \
    -postScript MPFreeFinishBootChain.py \
    -max-cpu 8 \
    -log "$LOG" \
    2>&1 | tee "$WORKDIR/$PROJECT_NAME.console.txt"

echo
echo "Ghidra static analysis complete."
echo "Project: $WORKDIR/projects/$PROJECT_NAME"
echo "Ghidra application log: $LOG"
echo "MPFree report(s):"
find "$HOME/mpfree-work" -maxdepth 1 -type f -name 'mpfree-ghidra-p3-final-*.txt' -printf '%TY-%Tm-%Td %TH:%TM %p\n' | sort -r | head -5
echo "No MPC, USB, firmware, or eMMC changes were made."
