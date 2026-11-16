#!/usr/bin/env bash
# Runs the Open MPC catalog plugins listed in catalog-check.txt in MPFree's
# plugin sandbox, as their real 32-bit ARM releases: downloads each release
# zip (checked against its SHA-256), unpacks it, builds mpfree-plughost and
# rack_check for ARMv7 and runs rack_check under qemu-arm on the folder. The
# rack finds the catalog's release folders as a person would unpack them, each
# plugin runs in its own process, and each must play, give finite sound and
# hand its state to a second copy (see rust/mpfree-plughost/examples/rack_check.rs).
#
# Builds and runs on the workstation or in CI only. Copies nothing to an MPC.
# Needs curl, python3 (to unzip), qemu-arm, the armhf cross compiler and its libc, and the
# armv7-unknown-linux-gnueabihf Rust target. Downloads are kept in
# $MPFREE_CATALOG_CACHE (default ~/.cache/mpfree-catalog) by their SHA-256;
# if that folder can't be made, in this run's own temporary folder.
#
# Usage: catalog-check.sh [LIST]   (default: catalog-check.txt beside this script)
set -Eeuo pipefail
repo="$(cd -- "$(dirname -- "$0")/../.." && pwd -P)"
list="${1:-$repo/tools/mpc-plugins/catalog-check.txt}"
cache="${MPFREE_CATALOG_CACHE:-$HOME/.cache/mpfree-catalog}"
target=armv7-unknown-linux-gnueabihf
sysroot=/usr/arm-linux-gnueabihf
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/plugins"
if ! mkdir -p "$cache" 2>/dev/null; then
  echo "note: can't make $cache; downloading into this run's temporary folder" >&2
  cache="$work/cache"
  mkdir -p "$cache"
fi

n=0
while read -r id kind sha url; do
  case "$id" in ''|'#'*) continue ;; esac
  zip="$cache/$sha.zip"
  if ! echo "$sha  $zip" | sha256sum -c --status - 2>/dev/null; then
    curl -fsSL --retry 3 -o "$zip.part" "$url"
    echo "$sha  $zip.part" | sha256sum -c --status - ||
      { echo "FAIL $id: the download doesn't match its SHA-256" >&2; rm -f "$zip.part"; exit 1; }
    mv "$zip.part" "$zip"
  fi
  mkdir -p "$work/unpack/$id"
  # Python's zipfile, not unzip: the self-hosted runner has no unzip.
  python3 -m zipfile -e "$zip" "$work/unpack/$id"
  # The release folder (holding portable/), moved in whole, as a person
  # would copy it.
  for rel in "$work/unpack/$id"/*/; do mv "$rel" "$work/plugins/"; done
  echo "$id ($kind): $(basename "$url")"
  n=$((n + 1))
done < "$list"

env CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path "$repo/rust/Cargo.toml" --release --locked -q \
    --target "$target" -p mpfree-plughost --bins --example rack_check
bin="${CARGO_TARGET_DIR:-$repo/rust/target}/$target/release"
out="$work/report.txt"
timeout 900 qemu-arm -L "$sysroot" "$bin/examples/rack_check" "$work/plugins" \
  --exe qemu-arm -L "$sysroot" "$bin/mpfree-plughost" -- | tee "$out"
found="$(grep -c '^ok \|^FAIL ' "$out" || true)"
[[ "$found" == "$n" ]] || { echo "FAIL: $n plugins listed, $found checked" >&2; exit 1; }
echo "PASS: $n Open MPC catalog plugins ran in the plugin sandbox (ARMv7 under qemu)"
