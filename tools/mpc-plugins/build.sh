#!/usr/bin/env bash
# Builds the MPFree MPC plugins (rust/mpfree-mpcplug/examples) for one MPC
# CPU family and checks each .so against the plugin rules in the inMusic
# SDK's docs/plugin-folder-format.md: an ELF shared object for the right
# CPU (32-bit: ARMv7 hard-float; 64-bit: AArch64), exporting only
# VSTPluginMain, needing nothing beyond libc, libm and the loader, with
# glibc symbol versions no newer than 2.39.
#
#   build.sh arm32   MPC X, Live, Live II, One, One+, Key 61, Key 37, Force
#   build.sh arm64   MPC Live III, MPC XL and later
#
# Builds on the workstation or in CI only. Copies nothing to an MPC.
# Needs the Rust target and the GNU cross compiler for that family.
# Prints the folder holding the .so files.
set -Eeuo pipefail
repo="$(cd -- "$(dirname -- "$0")/../.." && pwd -P)"
case "${1:-}" in
  arm32) target=armv7-unknown-linux-gnueabihf; cc=arm-linux-gnueabihf; loader=ld-linux-armhf.so.3
         class='ELF32'; machine='ARM'; flags='hard-float ABI'
         linker_var=CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER ;;
  arm64) target=aarch64-unknown-linux-gnu; cc=aarch64-linux-gnu; loader=ld-linux-aarch64.so.1
         class='ELF64'; machine='AArch64'; flags=''
         linker_var=CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER ;;
  *) echo "usage: build.sh arm32|arm64" >&2; exit 2 ;;
esac
out="${CARGO_TARGET_DIR:-$repo/rust/target}/$target/release/examples"

# Rust's std asks for libgcc_s.so.1 by name. Point that name at GCC's static
# unwinder (libgcc_eh.a) so the plugin carries its own copy, as the SDK's
# C++ builds do with -static-libgcc.
eh="$("$cc-gcc" -print-file-name=libgcc_eh.a)"
[[ -f "$eh" ]] || { echo "libgcc_eh.a not found for $cc" >&2; exit 1; }
shim="$(mktemp -d)"
trap 'rm -rf "$shim"' EXIT
ln -s "$eh" "$shim/libgcc_s.a"

env "$linker_var=$cc-gcc" \
  RUSTFLAGS="-L native=$shim -C link-arg=-static-libgcc -C link-arg=-Wl,--exclude-libs,ALL" \
  cargo build --manifest-path "$repo/rust/Cargo.toml" --release --locked \
    --target "$target" -p mpfree-mpcplug --example mpfree_delay --example mpfree_squelch >&2

fail=0
for so in "$out"/libmpfree_*.so; do
  name="$(basename "$so")"
  case "$name" in *-*.so) continue ;; esac   # cargo's hashed copies
  bad=0
  hdr="$("$cc-readelf" -h "$so")"
  { grep -Eq "Class: *$class" <<<"$hdr" && grep -Eq "Machine: *$machine" <<<"$hdr" &&
    grep -Eq 'Type: *DYN' <<<"$hdr" && grep -Eq "Flags:.*$flags" <<<"$hdr"; } ||
    { echo "FAIL $name: not a $class $machine shared object" >&2; bad=1; }
  needed="$("$cc-readelf" -d "$so" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p')"
  for lib in $needed; do
    case "$lib" in libc.so.6|libm.so.6|libdl.so.2|libpthread.so.0|"$loader") ;;
      *) echo "FAIL $name: needs $lib" >&2; bad=1 ;; esac
  done
  exports="$("$cc-readelf" --dyn-syms -W "$so" | awk '$7!="UND" && ($5=="GLOBAL" || $5=="WEAK") {print $8}' | sort -u)"
  [[ "$exports" == "VSTPluginMain" ]] || { echo "FAIL $name: exports $(echo $exports)" >&2; bad=1; }
  newest="$("$cc-objdump" -T "$so" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -n 1)"
  if [[ -n "$newest" ]] && [[ "$(printf '%s\n2.39\n' "$newest" | sort -V | tail -n 1)" != "2.39" ]]; then
    echo "FAIL $name: needs GLIBC_$newest (the MPC has 2.39)" >&2; bad=1
  fi
  if "$cc-objdump" -T "$so" | grep -Eq 'GLIBCXX_|GCC_[0-9]'; then
    echo "FAIL $name: needs libstdc++ or libgcc_s symbols" >&2; bad=1
  fi
  [[ "$bad" == 0 ]] && echo "PASS $1 $name: needs $(echo $needed), glibc <= ${newest:-none}, exports VSTPluginMain only" >&2
  fail=$((fail | bad))
done
[[ "$fail" == 0 ]] || exit 1
echo "$out"
