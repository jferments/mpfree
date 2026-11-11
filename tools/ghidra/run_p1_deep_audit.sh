#!/usr/bin/env bash
# MPFree: inspect the already-analyzed SRAM SPL with Python 3 / PyGhidra.
# No new import, no source backup access, no project writes, no MPC contact.
set -Eeuo pipefail
umask 077

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
PROJECTS="$HOME/mpfree-work/ghidra/projects"
WORK="$HOME/mpfree-work"
REPORT_SCRIPT="$ROOT/tools/ghidra/MPFreeSPLDeepAudit.py"
[[ -f "$REPORT_SCRIPT" ]] || {
    echo "Missing deep-audit Python script: git pull first" >&2
    exit 1
}
python3 -m py_compile "$REPORT_SCRIPT"

[[ -d "$PROJECTS" ]] || {
    echo "Missing Ghidra projects: $PROJECTS" >&2
    exit 1
}

if command -v snap >/dev/null 2>&1 && snap list ghidra >/dev/null 2>&1; then
    GHIDRA=(snap run ghidra.ghidra -H)
elif [[ -n "${GHIDRA_HOME:-}" && -f "$GHIDRA_HOME/support/pyghidraRun" ]]; then
    GHIDRA=("$GHIDRA_HOME/support/pyghidraRun" -H)
else
    echo "Python-enabled Ghidra not found (Ghidra Snap or GHIDRA_HOME required)" >&2
    exit 1
fi

# Prefer explicit name for full reproducibility; otherwise select newest P1
# project. Never silently replace/overwrite the old analyzed project.
NAME="${MPFREE_P1_PROJECT:-}"
if [[ -z "$NAME" ]]; then
    newest="$(find "$PROJECTS" -maxdepth 1 -type f \
        -name 'MPFree_P1_SRAM_*.gpr' -printf '%T@ %f\n' |
        sort -rn | sed -n '1p')"
    [[ -n "$newest" ]] || {
        echo "No previously analyzed P1 SRAM .gpr project in $PROJECTS" >&2
        exit 1
    }
    NAME="${newest#* }"
    NAME="${NAME%.gpr}"
fi
[[ -f "$PROJECTS/$NAME.gpr" ]] || {
    echo "Ghidra project does not exist: $PROJECTS/$NAME.gpr" >&2
    exit 1
}
mkdir -p "$WORK/ghidra"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)_$$"
REPORT="$WORK/mpfree-ghidra-p1-deep-$STAMP.json"
CONSOLE="$WORK/ghidra/p1-deep-$STAMP.console.txt"
LOG="$WORK/ghidra/p1-deep-$STAMP.ghidra.log"

echo "Project (read-only): $PROJECTS/$NAME"
echo "Expected SRAM hash: 6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"
echo "Saving private report to: $REPORT"
echo "No MPC communication and no writes to Ghidra project."
"${GHIDRA[@]}" "$PROJECTS" "$NAME" \
    -process p1-rk3288-sram-stage.bin \
    -noanalysis \
    -readOnly \
    -scriptPath "$ROOT/tools/ghidra" \
    -postScript MPFreeSPLDeepAudit.py "$REPORT" \
    -log "$LOG" \
    2>&1 | tee "$CONSOLE"

[[ -s "$REPORT" ]] || {
    echo "No report generated; see $CONSOLE and $LOG" >&2
    exit 2
}
python3 -m json.tool "$REPORT" >/dev/null
echo "MPFree SPL deep audit complete: $REPORT"
echo "Log: $CONSOLE"
