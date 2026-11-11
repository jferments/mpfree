#!/usr/bin/env bash
# Renders the startup screen with a title font, so a typeface can be tried
# in under a minute. Desktop only: it touches no device.
#
#   rust/mpfree-splash/preview-font.sh FONT.ttf|FONT.otf [TRACKING]
#
# TRACKING is extra space between letters in cap heights (default 0; 0.15
# suits fonts whose letters crowd each other, like Cynatar). Writes the
# framebuffer version's frames (exactly what the MPC One would show) and,
# if the SDL3/GLES version builds here, its frames too, into one folder
# plus a zip of it. Font files are not copied into the output.
set -euo pipefail

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "usage: $0 FONT.ttf|FONT.otf [TRACKING]" >&2
    exit 2
fi
font=$(realpath "$1")
tracking=${2:-0}
rust=$(cd "$(dirname "$0")/.." && pwd)
name=$(basename "${font%.*}" | tr -c 'A-Za-z0-9_.\n-' '_')
out=${OUT:-$PWD/splash-font-$name-$(date -u +%Y%m%dT%H%M%SZ)}
mkdir -p "$out"
log=$out/log.txt
echo "font: $font  tracking: $tracking" | tee "$log"

cargo run -q --release --locked --manifest-path "$rust/Cargo.toml" -p mpfree-splash -- \
    --preview "$out/framebuffer" --frames 2 --every 3 --font "$font" --tracking "$tracking" 2>&1 | tee -a "$log"

if SDL_VIDEO_DRIVER=${SDL_VIDEO_DRIVER:-offscreen} cargo run -q --release --locked \
    --manifest-path "$rust/mpfree-splash-gl/Cargo.toml" -- \
    --screenshot "$out/gl" --frames 2 --every 3 --font "$font" --tracking "$tracking" >>"$log" 2>&1; then
    grep '^wrote' "$log" | tail -2
else
    echo "SDL3/GLES version not rendered (see $log); the framebuffer frames are still there." | tee -a "$log"
fi

(cd "$(dirname "$out")" && zip -qr "$(basename "$out").zip" "$(basename "$out")")
echo "done: $out and $out.zip"
