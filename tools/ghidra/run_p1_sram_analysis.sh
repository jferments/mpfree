#!/usr/bin/env bash
# MPFree: analyze ORIGINAL 32-KiB RK3288 p1 SRAM stage in Ghidra.
# Workstation-only. Reads private backup; writes only to a NEW local
# Ghidra project and private workdir. NEVER contacts MPC/eMMC/USB/OTP.
set -Eeuo pipefail
umask 077

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$HOME/mpfree-work"
PROJECTS="$WORK/ghidra/projects"
BACKUP="${MPFREE_EMMC_BACKUP:-$HOME/mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img}"

if [[ ! -f "$ROOT/tools/mpc-one/extract-p1-sram-for-ghidra.py" ||
      ! -f "$ROOT/tools/ghidra/MPFreeSPLAudit.py" ]]; then
    echo "Missing SPL research files; git pull --ff-only origin main" >&2
    exit 1
fi

if command -v snap >/dev/null 2>&1 && snap list ghidra >/dev/null 2>&1; then
    GHIDRA=(snap run ghidra.ghidra -H)
elif [[ -n "${GHIDRA_HOME:-}" && -f "$GHIDRA_HOME/support/pyghidraRun" ]]; then
    GHIDRA=("$GHIDRA_HOME/support/pyghidraRun" -H)
else
    echo "Cannot find Python-enabled Ghidra. Requires Ghidra Snap or GHIDRA_HOME." >&2
    exit 1
fi

TAG="$(date -u +%Y%m%dT%H%M%SZ)_$$"
PRIVATE="$WORK/ghidra/sram-analysis-$TAG"
mkdir -p "$PRIVATE" "$PROJECTS"
STAGE="$PRIVATE/p1-rk3288-sram-stage.bin"
LOG="$PRIVATE/ghidra.log"
CONSOLE="$PRIVATE/console.txt"
NAME="MPFree_P1_SRAM_$TAG"
STAMPFILE="$PRIVATE/run-start"

python3 "$ROOT/tools/mpc-one/extract-p1-sram-for-ghidra.py" \
  "$BACKUP" --output "$STAGE"

echo
echo "Ghidra launcher: ${GHIDRA[*]}"
echo "New local project: $PROJECTS/$NAME"
echo "Raw image mapping: ARM:LE:32:v7 @ 0xff704000"
echo "Candidate entry: 0xff704004 (ARM state is hypothesis)"
echo "Private stage: $STAGE"
echo "No MPC connection or firmware modification."
echo

export MPFREE_REPORT_DIR="$WORK"
touch "$STAMPFILE"

"${GHIDRA[@]}" "$PROJECTS" "$NAME" \
  -import "$STAGE" \
  -loader BinaryLoader \
  -loader-baseAddr ff704000 \
  -processor ARM:LE:32:v7 \
  -cspec default \
  -scriptPath "$ROOT/tools/ghidra" \
  -preScript MPFreeSPLAudit.py pre \
  -postScript MPFreeSPLAudit.py post \
  -max-cpu 8 \
  -log "$LOG" \
  2>&1 | tee "$CONSOLE"

mapfile -t RESULTS < <(
  find "$WORK" -maxdepth 1 -type f \
    -name 'mpfree-ghidra-p1-sram-*.json' -newer "$STAMPFILE" -print
)
if (( ${#RESULTS[@]} != 1 )); then
    echo "ERROR: Expected exactly one fresh SPL report, found ${#RESULTS[@]}." >&2
    echo "Review $CONSOLE and $LOG" >&2
    exit 2
fi

python3 -m json.tool "${RESULTS[0]}" >/dev/null
echo
echo "MPFree p1 SRAM Ghidra analysis report: ${RESULTS[0]}"
echo "New Ghidra project: $PROJECTS/$NAME"
echo "Ghidra stdout: $CONSOLE"
echo "The original eMMC backup was not modified."
