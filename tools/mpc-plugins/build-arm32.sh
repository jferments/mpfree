#!/usr/bin/env bash
# Builds the example MPC plugins (rust/mpfree-mpcplug/examples) for the
# MPC One's 32-bit ARM and checks each .so against the MPC plugin rules in
# the inMusic SDK's docs/plugin-folder-format.md: ELF32 ARM hard-float
# shared object, exports only VSTPluginMain, needs nothing beyond libc,
# libm and the loader, glibc symbol versions no newer than 2.39.
#
# Builds on the workstation or in CI only. Copies nothing to an MPC.
# Needs: rustup target armv7-unknown-linux-gnueabihf, gcc-arm-linux-gnueabihf.
# Prints the folder holding the .so files.
set -Eeuo pipefail
repo="$(cd -- "$(dirname -- "$0")/../.." && pwd -P)"
target=armv7-unknown-linux-gnueabihf
out="${CARGO_TARGET_DIR:-$repo/rust/target}/$target/release/examples"

# Rust's std asks for libgcc_s.so.1 by name. Point that name at GCC's static
# unwinder (libgcc_eh.a) so the plugin carries its own copy, as the SDK's
# C++ builds do with -static-libgcc.
eh="$(arm-linux-gnueabihf-gcc -print-file-name=libgcc_eh.a)"
[[ -f "$eh" ]] || { echo "libgcc_eh.a not found for arm-linux-gnueabihf" >&2; exit 1; }
shim="$(mktemp -d)"
trap 'rm -rf "$shim"' EXIT
ln -s "$eh" "$shim/libgcc_s.a"

CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
RUSTFLAGS="-L native=$shim -C link-arg=-static-libgcc -C link-arg=-Wl,--exclude-libs,ALL" \
  cargo build --manifest-path "$repo/rust/Cargo.toml" --release --locked \
    --target "$target" -p mpfree-mpcplug --examples >&2

fail=0
for so in "$out"/libmpfree_*.so; do
  name="$(basename "$so")"
  case "$name" in *-*.so) continue ;; esac   # cargo's hashed copies
  hdr="$(arm-linux-gnueabihf-readelf -h "$so")"
  grep -Eq 'Class: *ELF32' <<<"$hdr" && grep -Eq 'Type: *DYN' <<<"$hdr" &&
    grep -Eq 'Flags:.*hard-float ABI' <<<"$hdr" || { echo "FAIL $name: not an ELF32 ARM hard-float shared object" >&2; fail=1; }
  needed="$(arm-linux-gnueabihf-readelf -d "$so" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p')"
  for lib in $needed; do
    case "$lib" in libc.so.6|libm.so.6|libdl.so.2|libpthread.so.0|ld-linux-armhf.so.3) ;;
      *) echo "FAIL $name: needs $lib" >&2; fail=1 ;; esac
  done
  exports="$(arm-linux-gnueabihf-readelf --dyn-syms -W "$so" | awk '$7!="UND" && ($5=="GLOBAL" || $5=="WEAK") {print $8}' | sort -u)"
  [[ "$exports" == "VSTPluginMain" ]] || { echo "FAIL $name: exports $(echo $exports)" >&2; fail=1; }
  newest="$(arm-linux-gnueabihf-objdump -T "$so" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -n 1)"
  if [[ -n "$newest" ]] && [[ "$(printf '%s\n2.39\n' "$newest" | sort -V | tail -n 1)" != "2.39" ]]; then
    echo "FAIL $name: needs GLIBC_$newest (the MPC has 2.39)" >&2; fail=1
  fi
  if arm-linux-gnueabihf-objdump -T "$so" | grep -Eq 'GLIBCXX_|GCC_[0-9]'; then
    echo "FAIL $name: needs libstdc++ or libgcc_s symbols" >&2; fail=1
  fi
  [[ "$fail" == 0 ]] && echo "PASS $name: ELF32 ARM hard-float, needs $(echo $needed), glibc <= ${newest:-none}, exports VSTPluginMain only" >&2
done
[[ "$fail" == 0 ]] || exit 1
echo "$out"
