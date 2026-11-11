#!/usr/bin/env bash
# One command: bash tools/ghidra/run_wide_bootchain_audit.sh
# READ-ONLY OFFLINE: existing eMMC backup and saved Ghidra projects only.
# Never communicates with the MPC, enumerates USB, mounts live disks,
# executes bootloader commands, writes partition tables, or programs OTP.
set -Eeuo pipefail
umask 077

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$HOME/mpfree-work"
PROJECTS="$WORK/ghidra/projects"
BACKUP="${MPFREE_EMMC_BACKUP:-$HOME/mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img}"
GHIDRA_AUDIT="$ROOT/tools/ghidra/MPFreeBootChainWideAudit.py"
ARTIFACTS="$ROOT/tools/mpc-one/offline-boot-artifacts-wide.py"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)_$$"
REPORT_DIR="$WORK/bootchain-wide-$STAMP"

[[ -f "$BACKUP" && ! -b "$BACKUP" ]] || {
    echo "ERROR: expected existing regular-file eMMC backup, never a live block device: $BACKUP" >&2
    exit 1
}
[[ -f "$GHIDRA_AUDIT" && -f "$ARTIFACTS" ]] || {
    echo "ERROR: missing audit scripts; update the MPFree repository" >&2
    exit 1
}
[[ -d "$PROJECTS" ]] || {
    echo "ERROR: missing saved Ghidra project directory: $PROJECTS" >&2
    echo "Run the existing p1 and p3 Ghidra import scripts first." >&2
    exit 1
}
if command -v snap >/dev/null 2>&1 && snap list ghidra >/dev/null 2>&1; then
    GHIDRA=(snap run ghidra.ghidra -H)
elif [[ -n "${GHIDRA_HOME:-}" && -x "$GHIDRA_HOME/support/pyghidraRun" ]]; then
    GHIDRA=("$GHIDRA_HOME/support/pyghidraRun" -H)
else
    echo "ERROR: no PyGhidra-capable Ghidra launcher found (Snap or GHIDRA_HOME)." >&2
    exit 1
fi

project_name() {
    local kind="$1" explicit="$2" pattern="$3" newest
    if [[ -n "$explicit" ]]; then
        [[ -f "$PROJECTS/$explicit.gpr" ]] || {
            echo "ERROR: no Ghidra project $PROJECTS/$explicit.gpr" >&2
            return 1
        }
        printf '%s\n' "$explicit"
        return
    fi
    newest="$(find "$PROJECTS" -maxdepth 1 -type f -name "$pattern" \
                -printf '%T@ %f\n' | sort -nr | sed -n '1p')"
    [[ -n "$newest" ]] || {
        echo "ERROR: no analyzed $kind Ghidra project matches $pattern" >&2
        return 1
    }
    newest="${newest#* }"
    printf '%s\n' "${newest%.gpr}"
}

P1_PROJECT="$(project_name p1 "${MPFREE_P1_PROJECT:-}" 'MPFree_P1_SRAM_*.gpr')"
P3_PROJECT="$(project_name p3 "${MPFREE_P3_PROJECT:-}" 'MPFree_P3_Uboot_*.gpr')"

mkdir -p "$REPORT_DIR"
REPORT_P1="$REPORT_DIR/p1-sram-ghidra.json"
REPORT_P3="$REPORT_DIR/p3-uboot-ghidra.json"
REPORT_ART="$REPORT_DIR/storage-env-boot-dtb.json"

echo "================================================================="
echo "MPFree wide boot-chain investigation — OFFLINE and READ-ONLY"
echo "Backup: $BACKUP"
echo "Existing P1 Ghidra project: $P1_PROJECT"
echo "Existing P3 Ghidra project: $P3_PROJECT"
echo "PRIVATE REPORT DIRECTORY: $REPORT_DIR"
echo "No MPC or hardware access; no writes to any project or boot medium."
echo "================================================================="

echo
echo "[1/3] Validating offline backup, U-Boot environment, p6 boot script and DTBs..."
OPT=()
if [[ "${MPFREE_SKIP_ROOTFS:-0}" == "1" ]]; then OPT=(--skip-rootfs); fi
python3 "$ARTIFACTS" --backup "$BACKUP" --report "$REPORT_ART" "${OPT[@]}"

run_ghidra() {
    local kind="$1" project="$2" filename="$3" report="$4"
    local logfile="$REPORT_DIR/$kind-ghidra.log"
    echo
    echo "Inspecting $kind in Ghidra (read-only)..."
    "${GHIDRA[@]}" "$PROJECTS" "$project" \
        -process "$filename" \
        -readOnly -noanalysis \
        -scriptPath "$ROOT/tools/ghidra" \
        -postScript MPFreeBootChainWideAudit.py "$kind" "$report" \
        -log "$logfile" \
        2>&1 | tee "$REPORT_DIR/$kind-console.txt"
    [[ -s "$report" ]] || {
        echo "ERROR: no $kind report. See $logfile and $REPORT_DIR/$kind-console.txt" >&2
        return 1
    }
    python3 -m json.tool "$report" >/dev/null
}
echo "[2/3] P1 SRAM SPL"
run_ghidra p1 "$P1_PROJECT" "p1-rk3288-sram-stage.bin" "$REPORT_P1"
echo "[3/3] P3 U-Boot"
run_ghidra p3 "$P3_PROJECT" "p3-uimage-payload.bin" "$REPORT_P3"

python3 - "$REPORT_DIR" "$REPORT_P1" "$REPORT_P3" "$REPORT_ART" <<'PY'
import json
from pathlib import Path
import sys
import hashlib
folder = Path(sys.argv[1])
paths = [Path(x) for x in sys.argv[2:]]
docs = [json.loads(path.read_text(encoding="utf-8")) for path in paths]
manifest = {
    "schema": "mpfree-wide-bootchain-manifest-v1",
    "source_mode": "offline, verified owner backup and saved analyzed Ghidra projects",
    "reports": [{"filename": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                for path in paths],
    "p1_functions": docs[0]["program_statistics"]["functions_known_to_ghidra"],
    "p3_functions": docs[1]["program_statistics"]["functions_known_to_ghidra"],
    "p1_string_hits": len(docs[0]["strings"]),
    "p3_string_hits": len(docs[1]["strings"]),
    "rootfs": docs[2]["rootfs"]["status"],
    "warnings": [
        "Static findings cannot prove MaskROM entry, runtime unlocking or unsigned-loader acceptance.",
        "Keep these files private; they include decompiled proprietary functions.",
    ],
}
(folder / "manifest.json").write_text(json.dumps(manifest, indent=2)+"\n", encoding="utf-8")
print("\nFinished: ", folder)
print(json.dumps(manifest, indent=2))
PY
echo
echo "NEXT: share manifest.json and the three JSON reports privately here."
echo "DO NOT commit the reports to public GitHub."
