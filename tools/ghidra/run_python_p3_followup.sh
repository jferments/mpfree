#!/usr/bin/env bash
# Run an OPTIONAL CPython3 read-only follow-up on the latest saved p3 project.
# Does not replace the proven Java bootstrap analyzer.
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "$0")/../.." && pwd)"
PROJECTS="$HOME/mpfree-work/ghidra/projects"
OUT="$HOME/mpfree-work"
SCRIPT="$ROOT/tools/ghidra/MPFreePythonFollowup.py"

[[ -f "$SCRIPT" ]] || { echo "Missing Python audit: git pull first" >&2; exit 1; }
command -v snap >/dev/null || { echo "Snap is required for this launcher" >&2; exit 1; }
snap list ghidra >/dev/null || { echo "Ghidra Snap not installed" >&2; exit 1; }
mkdir -p "$OUT"

# Optional explicit project name: MPFREE_P3_PROJECT=name_without_gpr
project="${MPFREE_P3_PROJECT:-}"
if [[ -z "$project" ]]; then
    latest="$(find "$PROJECTS" -maxdepth 1 -type f -name 'MPFree_P3_Uboot_*.gpr' -printf '%T@ %p\n' | sort -rn | sed -n '1p')"
    [[ -n "$latest" ]] || {
        echo "Could not find a Ghidra .gpr project under $PROJECTS" >&2
        exit 1
    }
    project="${latest#* }"
    project="${project##*/}"
    project="${project%.gpr}"
fi
[[ -f "$PROJECTS/$project.gpr" ]] || {
    echo "Project not found: $PROJECTS/$project.gpr" >&2; exit 1
}
STAMP="$(date +%Y%m%dT%H%M%S)"
REPORT="$OUT/mpfree-ghidra-p3-python-$STAMP.json"
LOG="$OUT/ghidra/python-followup-$STAMP.log"
export MPFREE_REPORT_DIR="$OUT"

echo "PyGhidra launcher: snap run ghidra.ghidra -H"
echo "Saved project: $PROJECTS/$project"
echo "Read-only: yes"
echo "Output: $REPORT"
echo "On first launch, PyGhidra may offer to create a Python venv."
snap run ghidra.ghidra -H \
    "$PROJECTS" "$project" \
    -process p3-uimage-payload.bin \
    -noanalysis \
    -readOnly \
    -scriptPath "$ROOT/tools/ghidra" \
    -postScript MPFreePythonFollowup.py "$REPORT" \
    -log "$LOG"

if [[ ! -s "$REPORT" ]]; then
    echo "ERROR: Python script did not create report; see $LOG" >&2
    exit 2
fi
python3 -m json.tool "$REPORT" >/dev/null
echo "Python follow-up succeeded: $REPORT"
