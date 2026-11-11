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

# Even if a later optional pass fails, provide one ZIP with what succeeded.
package_reports_on_exit() {
    local exit_code=$?
    trap - EXIT
    if [[ -d "$REPORT_DIR" ]] && find "$REPORT_DIR" -type f -print -quit | grep -q .; then
        echo
        echo "Packaging ALL available private reports and logs into ONE ZIP..."
        python3 "$ROOT/tools/mpc-one/package-private-boot-audit.py" "$REPORT_DIR" ||
            echo "WARNING: private ZIP packaging failed; reports remain in $REPORT_DIR" >&2
    fi
    exit "$exit_code"
}
trap package_reports_on_exit EXIT


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

# Preserve the ROM-image header interpretation and inspect SPL's
# embedded boot-critical device tree (especially SDRAM/DDR parameters).
echo
echo "[supplement] RK3288 ID block and embedded SPL FDT/DDR scan..."
python3 "$ROOT/tools/mpc-one/offline-rk3288-rom.py" idblock "$BACKUP" \
    >"$REPORT_DIR/p1-rockchip-idblock.txt" 2>&1 ||
    echo "WARNING: RK3288 ID block inspection failed; see ZIP output." >&2
python3 "$ROOT/tools/mpc-one/offline-spl-fdt.py" \
    --backup "$BACKUP" --report "$REPORT_DIR/p1-embedded-spl-dtb.json" \
    >"$REPORT_DIR/p1-embedded-spl-dtb-console.txt" 2>&1 ||
    echo "WARNING: embedded SPL FDT inspection failed; see ZIP output." >&2


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
# Optional official 3.10 image: inspect the update container, never execute it.
# Important because signed/update files may differ from installed P1/P3.
UPDATE_IMAGE="${MPFREE_UPDATE_IMAGE:-$HOME/mpfree-work/akai-update-files/MPC-3.10.0-Gen1-update.img}"
if [[ -f "$UPDATE_IMAGE" && ! -b "$UPDATE_IMAGE" ]]; then
    echo
    echo "[optional] Cataloging 3.10 firmware PART records (offline only)..."
    python3 "$ROOT/tools/mpc-one/offline-update-image.py" "$UPDATE_IMAGE" \
        > "$REPORT_DIR/firmware-3.10-container.txt"
else
    echo "[optional] No official 3.10 image found; skipping firmware container audit."
fi

echo "[2/3] P1 SRAM SPL"
run_ghidra p1 "$P1_PROJECT" "p1-rk3288-sram-stage.bin" "$REPORT_P1"
echo "[3/3] P3 U-Boot"
run_ghidra p3 "$P3_PROJECT" "p3-uimage-payload.bin" "$REPORT_P3"


# Wide second pass: all major unresolved questions in both projects.
# Optional by design; errors are logged, not allowed to discard successful
# primary reports. Each pass remains read-only, hash-verified, and offline.
run_forensics() {
    local kind="$1" project="$2" filename="$3" output="$4"
    local log="$REPORT_DIR/$kind-forensics-pass2-ghidra.log"
    echo
    echo "Forensics pass 2: $kind (reboot/ROM, recovery, boot, security, RAM...)"
    if ! "${GHIDRA[@]}" "$PROJECTS" "$project" \
        -process "$filename" \
        -readOnly -noanalysis \
        -scriptPath "$ROOT/tools/ghidra" \
        -postScript MPFreeBootChainForensicsPass2.py "$kind" "$output" \
        -log "$log" \
        >"$REPORT_DIR/$kind-forensics-pass2-console.txt" 2>&1; then
        echo "WARNING: $kind follow-up failed; details are in the ZIP." >&2
        return 1
    fi
    if [[ ! -s "$output" ]] || ! python3 -m json.tool "$output" >/dev/null; then
        echo "WARNING: $kind follow-up missing/invalid; details are in the ZIP." >&2
        return 1
    fi
    echo "$kind forensics complete: $output"
}
run_forensics p1 "$P1_PROJECT" "p1-rk3288-sram-stage.bin" \
    "$REPORT_DIR/p1-forensics-pass2.json" || true
run_forensics p3 "$P3_PROJECT" "p3-uimage-payload.bin" \
    "$REPORT_DIR/p3-forensics-pass2.json" || true

# Cross-check newer Akai userspace, boot scripts, and DTBs if the existing
# extracted 3.10 rootfs image is on the workstation. Do not extract/run update.
ROOTFS_310="${MPFREE_310_ROOTFS:-$HOME/mpfree-work/firmware-analysis/MPC-3.10.0-Gen1-update/rootfs.ext4}"
if [[ -f "$ROOTFS_310" && ! -b "$ROOTFS_310" ]]; then
    echo
    echo "Comparing 3.10 rootfs boot, updater and reboot-mode metadata..."
    python3 "$ROOT/tools/mpc-one/offline-boot-version-compare.py" \
        --rootfs-310 "$ROOTFS_310" \
        --baseline-report "$REPORT_ART" \
        --report "$REPORT_DIR/firmware-3.10-boot-compare.json" \
        >"$REPORT_DIR/firmware-3.10-boot-compare-console.txt" 2>&1 ||
        echo "WARNING: optional 3.10 rootfs comparison failed; see ZIP logs." >&2
else
    printf 'Optional 3.10 rootfs not found at %s; skipped comparison.\n' "$ROOTFS_310" |
        tee "$REPORT_DIR/firmware-3.10-boot-compare-skipped.txt"
fi


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
echo "NEXT: upload the single ZIP generated at exit, not individual reports."
echo "DO NOT commit the ZIP or proprietary reports to public GitHub."
