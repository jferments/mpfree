#!/usr/bin/env bash
# Builds the MPFree USB creator for this computer, with this clone's MPFree
# for the MPC inside it. The same as running, in the clone's top folder:
#
#   cargo usb-creator
#
# which works the same way on Linux, macOS and Windows (rust/mpfree-creator-build).
# It ends with the creator as one file in rust/target/usb-creator and the line
# to open it. See docs/dev/guides/build-usb-creator.md.
set -euo pipefail
cd "$(dirname "$0")/.."
if ! command -v cargo >/dev/null; then
  printf '\nSTOP: %s\n' "Rust isn't installed. Install it from https://rustup.rs, open a new terminal, and run this again." >&2
  exit 1
fi
exec cargo usb-creator "$@"
