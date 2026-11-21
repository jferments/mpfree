#!/usr/bin/env bash
# Builds the MPFree USB creator for this Linux computer, with this clone's
# MPFree for the MPC inside it, the way a release builds it:
#
#   tools/build-usb-creator.sh
#
# It ends with the creator as one file, rust/target/usb-creator/mpfree-creator,
# and the line to open it. Run it again after a `git pull` to build a newer one.
# See docs/dev/guides/build-usb-creator.md.
#
# It builds MPFree's three programs for the MPC (static, as the device rounds and
# releases build them), the download helper and the plugin process (linked
# dynamically against a glibc no newer than the MPC's 2.39: with cargo-zigbuild
# when it is installed, else with the ARM cross compiler, and checked either
# way), and the free drum kits (about 15 MB fetched once, then kept in
# ~/.cache/mpfree-free-kits); then the creator, with all of them inside it.
# It writes only under rust/target and that cache, and needs no sudo.
set -euo pipefail

usage() {
  sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
}
case "${1:-}" in
  "") ;;
  -h | --help) usage; exit 0 ;;
  *) usage >&2; exit 2 ;;
esac

stop() {
  printf '\nSTOP: %s\n' "$*" >&2
  exit 1
}
step() { printf '\n== %s\n' "$*"; }

cd "$(dirname "$0")/.."
root=$PWD
arm=armv7-unknown-linux-gnueabihf
out=rust/target/usb-creator
programs=$out/stick-programs
helpers_target=rust/target/helpers
# The builds below choose their own flags and folders; settings for other
# builds in this shell would change what they make, or where.
unset RUSTFLAGS CARGO_BUILD_RUSTFLAGS CARGO_TARGET_DIR CARGO_BUILD_TARGET

step "What it builds"
git log --oneline -1 2>/dev/null || echo "(not a git clone)"

step "The tools"
[ "$(uname -s)" = Linux ] || stop "this builds the creator on Linux only (MPFree's programs for the MPC are built with a Linux ARM cross compiler)."
command -v cargo >/dev/null || stop "Rust isn't installed. Install it from https://rustup.rs, open a new terminal, and run this again."
command -v python3 >/dev/null || stop "python3 isn't installed: install your distribution's python3 package, and run this again."
command -v arm-linux-gnueabihf-gcc >/dev/null || stop "the ARM cross compiler (arm-linux-gnueabihf-gcc) isn't installed. On Debian or Ubuntu: sudo apt install gcc-arm-linux-gnueabihf libc6-dev-armhf-cross   Then run this again."
if command -v arm-linux-gnueabihf-readelf >/dev/null; then
  readelf=arm-linux-gnueabihf-readelf
elif command -v readelf >/dev/null; then
  readelf=readelf
else
  stop "readelf isn't installed: install your distribution's binutils package, and run this again."
fi
if command -v rustup >/dev/null; then
  rustup target list --installed | grep -qx "$arm" || rustup target add "$arm"
fi
# The helpers: cargo-zigbuild links them against glibc 2.34's symbols whatever
# this computer's cross compiler has. It finds zig on PATH, or in the private
# Python folder the device runbooks' block G1a installs it into.
zig_python="$HOME/.local/share/mpfree-zig/bin/python3"
use_zig=no
if cargo zigbuild --help >/dev/null 2>&1; then
  if command -v zig >/dev/null; then
    use_zig=yes
  elif [ -x "$zig_python" ]; then
    export CARGO_ZIGBUILD_PYTHON_PATH="$zig_python"
    use_zig=yes
  fi
fi
echo "cargo $(cargo --version | cut -d' ' -f2), cross compiler $(arm-linux-gnueabihf-gcc -dumpversion), helpers linked with $([ $use_zig = yes ] && echo cargo-zigbuild || echo the cross compiler)"

step "MPFree's programs for the MPC"
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target "$arm" -p mpfree-app -p mpfree-startup -p mpfree-screen-log

step "The download helper and the plugin process"
# Their own target folder, so neither build undoes the other's flags.
if [ $use_zig = yes ]; then
  cargo zigbuild --manifest-path rust/Cargo.toml --release --locked \
    --target-dir "$helpers_target" --target "$arm.2.34" \
    -p mpfree-mpcstore --bin mpfree-fetch -p mpfree-plughost --bin mpfree-plughost
else
  CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
    cargo build --manifest-path rust/Cargo.toml --release --locked \
    --target-dir "$helpers_target" --target "$arm" \
    -p mpfree-mpcstore --bin mpfree-fetch -p mpfree-plughost --bin mpfree-plughost
fi

step "The free drum kits"
kits_cache="${XDG_CACHE_HOME:-$HOME/.cache}/mpfree-free-kits"
mkdir -p "$out"
echo "(the first time, this builds the kit maker and fetches the samples: a few minutes)"
# It lists every pad of every kit; that goes to a log, and only its last line here.
if ! cargo run --manifest-path rust/Cargo.toml --release --locked -p mpfree-factory --bin mpfree-free-kits -- \
  --cache "$kits_cache" --out "$out/free-kits" >"$out/free-kits.log" 2>&1; then
  tail -n 20 "$out/free-kits.log" >&2
  stop "the free drum kits couldn't be made: the lines above say why, and $out/free-kits.log has them all."
fi
tail -n 1 "$out/free-kits.log"

step "Collecting them for the creator"
rm -rf "$programs"
mkdir -p "$programs"
built="rust/target/$arm/release"
# The names the creator needs, read from its source, so a program added there
# can't be left out here.
names=$(sed -n 's/^pub const PROGRAM_FILES: \[&str; [0-9]*\] = \[\(.*\)\];$/\1/p' rust/mpfree-stick/src/stick.rs | grep -o '"[a-z0-9-]*"' | tr -d '"' | tr '\n' ' ')
helpers=$(sed -n 's/^pub const \(FETCH_FILE\|PLUGHOST_FILE\): &str = "\([a-z0-9-]*\)";$/\2/p' rust/mpfree-stick/src/stick.rs | tr '\n' ' ')
[ -n "$names" ] || stop "couldn't read PROGRAM_FILES from rust/mpfree-stick/src/stick.rs."
[ "$(echo $helpers | wc -w)" = 2 ] || stop "couldn't read FETCH_FILE and PLUGHOST_FILE from rust/mpfree-stick/src/stick.rs."
is_arm() {
  "$readelf" -h "$1" | grep -Eq 'Machine: *ARM' &&
    "$readelf" -h "$1" | grep -Eq 'Flags:.*Version5 EABI.*hard-float ABI'
}
for n in $names; do
  f="$built/$n"
  [ -f "$f" ] || stop "the creator needs $n, which this script doesn't build: add it to the programs' cargo build line."
  is_arm "$f" || stop "$f isn't an ARM hard-float program."
  cp "$f" "$programs/$n"
done
for n in $helpers; do
  f="$helpers_target/$arm/release/$n"
  [ -f "$f" ] || stop "$n wasn't built."
  is_arm "$f" || stop "$f isn't an ARM hard-float program."
  "$readelf" -l "$f" | grep -q 'Requesting program interpreter: /lib/ld-linux-armhf.so.3' ||
    stop "$n isn't linked dynamically against the MPC's C library."
  newest=$("$readelf" -W --version-info "$f" | grep -o 'GLIBC_2\.[0-9]*' | sort -t. -k2 -n | tail -n 1)
  if [ "${newest#GLIBC_2.}" -gt 39 ]; then
    stop "$n asks for $newest, newer than the MPC's glibc 2.39: this computer's cross compiler is too new for it. Install cargo-zigbuild and zig once (no sudo; about 5 minutes):
  cargo install --locked cargo-zigbuild && python3 -m venv $HOME/.local/share/mpfree-zig && $HOME/.local/share/mpfree-zig/bin/pip install ziglang==0.17.1
then run this again: it uses them by itself."
  fi
  cp "$f" "$programs/$n"
done
cp "$out/free-kits" "$programs/free-kits"
(cd "$programs" && sha256sum $names $helpers free-kits)

step "The USB creator, with them inside"
MPFREE_CREATOR_PROGRAMS="$root/$programs" \
  cargo build --manifest-path rust/mpfree-creator/Cargo.toml --release --locked
exe=rust/mpfree-creator/target/release/mpfree-creator
python3 - "$exe" "$programs" <<'PY'
import os, sys
exe = open(sys.argv[1], "rb").read()
missing = [n for n in sorted(os.listdir(sys.argv[2]))
           if open(os.path.join(sys.argv[2], n), "rb").read() not in exe]
if missing:
    sys.exit("STOP: not inside the creator: " + ", ".join(missing))
PY
cp "$exe" "$out/mpfree-creator"

printf '\nDONE: the USB creator is ready, with MPFree %s inside:\n  %s\nOpen it with:\n  %s\n' \
  "$(git log -1 --format=%h 2>/dev/null || echo '(this folder)')" "$root/$out/mpfree-creator" "$out/mpfree-creator"
