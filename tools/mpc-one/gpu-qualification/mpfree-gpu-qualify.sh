#!/usr/bin/env bash
# MPFree GPU qualification stage 1 — inventory + offscreen GLES2 smoke test.
# MIT License. Copyright (c) 2026 MPFree contributors.
# Run on the Linux workstation, not the MPC.
set -euo pipefail

HOST="${MPFREE_MPC_HOST:-root@10.42.0.141}"
PORT="${MPFREE_MPC_PORT:-2222}"
KEY="${MPFREE_MPC_KEY:-$HOME/.ssh/mpc_one_autorun_ed25519}"
CROSS="${MPFREE_CROSS_CC:-arm-linux-gnueabihf-gcc}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$HERE/mpfree-egl-smoke.c"
OUTPUT_DIR="${MPFREE_GPU_OUTPUT_DIR:-$HERE}"
mkdir -p "$OUTPUT_DIR"
REPORT="$OUTPUT_DIR/gpu-qualification-$(date +%Y%m%d-%H%M%S).log"
BINARY="$OUTPUT_DIR/mpfree-egl-smoke-armhf"

SSH=(ssh -T -o BatchMode=yes -o ConnectTimeout=5 -o IdentitiesOnly=yes \
   -o StrictHostKeyChecking=yes -i "$KEY" -p "$PORT" "$HOST")

echo "=== MPFree GPU qualification: read-only hardware inventory ===" | tee "$REPORT"
"${SSH[@]}" 'sh -s' <<'REMOTE' 2>&1 | tee -a "$REPORT"
echo '=== Platform ==='
uname -a
printf 'Model: '
if [ -r /proc/device-tree/model ]; then tr '\000' '\n' < /proc/device-tree/model; else echo '(unavailable)'; fi
printf 'SoC compatible strings:\n'
if [ -r /proc/device-tree/compatible ]; then tr '\000' '\n' < /proc/device-tree/compatible; else echo '(unavailable)'; fi

echo '=== GPU drivers ==='
for dev in /sys/class/drm/card0 /sys/class/drm/card1 /sys/class/drm/renderD128; do
    if [ -e "$dev" ]; then
        echo "$dev:"
        readlink -f "$dev/device/driver" 2>/dev/null || true
    fi
done
ls -l /dev/dri /usr/lib/dri/panfrost_dri.so 2>/dev/null || true

echo '=== Display information (READ ONLY) ==='
for f in /sys/class/graphics/fb0/virtual_size /sys/class/graphics/fb0/bits_per_pixel; do
    if [ -r "$f" ]; then echo "$f:"; cat "$f"; fi
done
for f in /sys/class/drm/card0-*/modes; do
    if [ -r "$f" ]; then echo "$f:"; cat "$f"; fi
done

echo '=== Graphics libraries ==='
ls -l /usr/lib/libEGL.so.1 /usr/lib/libGLESv2.so.2 \
    /usr/lib/libgbm.so.1 /usr/lib/ld-linux-armhf.so.3 2>/dev/null || true

echo '=== RAM/CMA ==='
grep -E '^(MemTotal|MemAvailable|CmaTotal|CmaFree|SwapTotal):' /proc/meminfo || true

echo '=== Compile/runtime tools on MPC ==='
for cmd in gcc cc clang python3 ldd getconf; do
    command -v "$cmd" || true
done
if command -v getconf >/dev/null 2>&1; then getconf GNU_LIBC_VERSION 2>/dev/null || true; fi
REMOTE

echo | tee -a "$REPORT"
echo '=== Workstation cross-compilation ===' | tee -a "$REPORT"
if [ ! -f "$SRC" ]; then
    echo "ERROR: Cannot find $SRC; save the C file alongside this script." | tee -a "$REPORT"
    exit 2
fi
if ! command -v "$CROSS" >/dev/null 2>&1; then
    echo "Cross-compiler not found: $CROSS" | tee -a "$REPORT"
    echo 'On Ubuntu/Kubuntu, install with:' | tee -a "$REPORT"
    echo '  sudo apt update && sudo apt install gcc-arm-linux-gnueabihf' | tee -a "$REPORT"
    echo "Inventory is saved in $REPORT. Re-run this script after installing." | tee -a "$REPORT"
    exit 2
fi

# MPC's binaries use /usr/lib/ld-linux-armhf.so.3 rather than the normal /lib path.
"$CROSS" -std=c11 -O2 -Wall -Wextra -Werror \
   -Wl,--dynamic-linker=/usr/lib/ld-linux-armhf.so.3 \
   "$SRC" -o "$BINARY" -ldl
file "$BINARY" | tee -a "$REPORT"
echo

echo '=== Uploading probe to MPC /tmp (RAM, not permanent storage) ===' | tee -a "$REPORT"
"${SSH[@]}" 'cat > /tmp/mpfree-egl-smoke && chmod 700 /tmp/mpfree-egl-smoke' < "$BINARY"

echo '=== Running OFFSCREEN 256x256 GLES2 smoke test ===' | tee -a "$REPORT"
set +e
"${SSH[@]}" 'if command -v timeout >/dev/null 2>&1; then timeout 20 /tmp/mpfree-egl-smoke; else /tmp/mpfree-egl-smoke; fi' \
    2>&1 | tee -a "$REPORT"
RC=${PIPESTATUS[0]}
set -e

echo | tee -a "$REPORT"
echo "Probe exit code: $RC" | tee -a "$REPORT"
echo "Full report: $REPORT" | tee -a "$REPORT"
if [ "$RC" -eq 0 ]; then
    echo 'PASS: Hardware EGL/GLES2 offscreen rendering verified.' | tee -a "$REPORT"
else
    echo 'NOT YET QUALIFIED: See GPU/EGL diagnostic output above; do not assume acceleration.' | tee -a "$REPORT"
fi
exit "$RC"