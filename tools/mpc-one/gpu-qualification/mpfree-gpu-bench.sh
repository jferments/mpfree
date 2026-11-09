#!/usr/bin/env bash
# MPFree GPU qualification stage 2; run on the workstation, never on the MPC.
# MIT License — Copyright (c) 2026 MPFree contributors.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CC="${MPFREE_CROSS_CC:-arm-linux-gnueabihf-gcc}"
HOST="${MPFREE_MPC_HOST:-root@10.42.0.141}"
PORT="${MPFREE_MPC_PORT:-2222}"
KEY="${MPFREE_MPC_KEY:-$HOME/.ssh/mpc_one_autorun_ed25519}"
DATE="$(date +%Y%m%d-%H%M%S)"
LOG="$HERE/gpu-bench-$DATE.log"
SNAPSHOT="$HERE/gpu-bench-$DATE.ppm"
SRC="$HERE/mpfree-gpu-bench.c"
BINARY="$HERE/mpfree-gpu-bench-armhf"
SSH=(ssh -T -o BatchMode=yes -o ConnectTimeout=5 -o IdentitiesOnly=yes \
  -o StrictHostKeyChecking=yes -i "$KEY" -p "$PORT" "$HOST")
[[ -r "$SRC" ]] || { echo "Missing $SRC" >&2; exit 2; }
command -v "$CC" >/dev/null || { echo "Missing $CC; install gcc-arm-linux-gnueabihf on workstation" >&2; exit 2; }
[[ -r "$KEY" ]] || { echo "Missing SSH key $KEY" >&2; exit 2; }
echo '=== Cross-compiling GPU benchmark for MPC ARMv7 ==='
"$CC" -std=c11 -O2 -Wall -Wextra -Werror \
  -Wl,--dynamic-linker=/usr/lib/ld-linux-armhf.so.3 \
  "$SRC" -o "$BINARY" -ldl -lm
file "$BINARY"
echo '=== Transferring binary into MPC RAM-only /tmp ==='
"${SSH[@]}" 'cat > /tmp/mpfree-gpu-bench && chmod 700 /tmp/mpfree-gpu-bench' < "$BINARY"
echo '=== Offscreen scene, no DRM modesetting ==='
set +e
"${SSH[@]}" 'MESA_SHADER_CACHE_DISABLE=true; export MESA_SHADER_CACHE_DISABLE; if command -v timeout >/dev/null 2>&1; then timeout 25 /tmp/mpfree-gpu-bench; else /tmp/mpfree-gpu-bench; fi' \
  2>&1 | tee "$LOG"
RC=${PIPESTATUS[0]}
set -e
echo "GPU benchmark exit code: $RC"
if [[ "$RC" -eq 0 ]]; then
  echo '=== Downloading offscreen preview ==='
  "${SSH[@]}" 'cat /tmp/mpfree-gpu-bench.ppm' > "$SNAPSHOT"
  echo "Saved preview: $SNAPSHOT"
  echo "Saved report:  $LOG"
  echo 'Convert preview on workstation, if desired: python3 -c '\''from PIL import Image; Image.open("'"$SNAPSHOT"'").save("'"${SNAPSHOT%.ppm}.png"'")'\'''
else
  echo "Benchmark did not pass; report saved: $LOG"
fi
exit "$RC"