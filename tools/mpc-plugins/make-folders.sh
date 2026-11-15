#!/usr/bin/env bash
# Makes the complete MPC plugin folders for the MPFree plugins:
#
#   <out>/MPFree - VST - <Name>/
#     plugin.json  version.xml          (from our code: make_folders)
#     VST/<slug>-arm32.so  -arm64.so    (build.sh arm32 / arm64)
#     Plugin Skins/TUI.json Q-Links.json *.png
#                                        (the SDK's gen_ui.py, from our ui.json
#                                         and the parameters the binary reports)
#
# then runs the SDK's validate.py --strict on each folder.
#
# The inMusic MPC Plugin Builder SDK (MIT) is fetched at a pinned commit into
# <out>/.sdk; only its host simulator, gen_ui.py and validate.py are used.
# Builds on the workstation or in CI only. Copies nothing to an MPC.
#
# Usage: make-folders.sh [OUT_DIR]   (default rust/target/mpc-plugins)
set -Eeuo pipefail
repo="$(cd -- "$(dirname -- "$0")/../.." && pwd -P)"
target_dir="${CARGO_TARGET_DIR:-$repo/rust/target}"
out="${1:-$target_dir/mpc-plugins}"
SDK_URL=https://github.com/inmusic-software/mpc-plugin-builder-sdk
SDK_COMMIT=febd2ca5bd9eb16c4a412837196e145566bc981b
sdk="$out/.sdk"

mkdir -p "$out"
if [[ "$(git -C "$sdk" rev-parse HEAD 2>/dev/null || :)" != "$SDK_COMMIT" ]]; then
  rm -rf "$sdk"
  git init -q "$sdk"
  git -C "$sdk" fetch -q --depth 1 "$SDK_URL" "$SDK_COMMIT"
  git -C "$sdk" checkout -q FETCH_HEAD
fi
host_sim="$out/.host_sim"
[[ -x "$host_sim" && "$host_sim" -nt "$sdk/tools/host-sim/host_sim.cpp" ]] ||
  c++ -std=c++17 -O2 -o "$host_sim" "$sdk/tools/host-sim/host_sim.cpp" -ldl

arm32="$(bash "$repo/tools/mpc-plugins/build.sh" arm32)"
arm64="$(bash "$repo/tools/mpc-plugins/build.sh" arm64)"
cargo build --manifest-path "$repo/rust/Cargo.toml" --locked -q -p mpfree-mpcplug \
  --example mpfree_delay --example mpfree_squelch --example make_folders
native="$target_dir/debug/examples"

"$native/make_folders" "$out" | while IFS=$'\t' read -r folder lib slug; do
  name="$(basename "$folder")"
  build="$out/build/$lib"
  cp "$arm32/lib$lib.so" "$folder/VST/$slug-arm32.so"
  cp "$arm64/lib$lib.so" "$folder/VST/$slug-arm64.so"
  # The parameters as the MPC will see them, from a native build of the same source.
  "$host_sim" "$native/lib$lib.so" dump > "$build/params.json"
  # host_sim test is the SDK's check of the MPC call order; keep its report.
  "$host_sim" "$native/lib$lib.so" test --notes 36,48 > "$build/host_sim.txt" 2>&1 ||
    { cat "$build/host_sim.txt" >&2; echo "FAIL $name: host_sim test" >&2; exit 1; }
  rm -rf "$folder/Plugin Skins"
  python3 "$sdk/tools/gen_ui.py" --params "$build/params.json" --ui "$build/ui.json" \
    --out "$folder/Plugin Skins" --kit-cache "$out/build" --quiet
  python3 "$sdk/tools/validate.py" "$folder" --params "$build/params.json" --strict > "$build/validate.txt" 2>&1 ||
    { cat "$build/validate.txt" >&2; echo "FAIL $name: validate.py" >&2; exit 1; }
  echo "PASS $name: host_sim $(grep -o 'Result: PASS.*' "$build/host_sim.txt"), validate.py --strict clean" >&2
done
echo "$out"
